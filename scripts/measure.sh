#!/usr/bin/env bash
# Performance measurement of the infra binary with the embedded web UI (docs/decisions/0010, 0032).
# Builds the UI and the release binary, then prints one JSON document with:
# UI bundle sizes (raw + gzip), release binary size, idle RSS after 5 s and
# time from exec to "GET / answers 200" (median of RUNS runs).
# Only the server process started here is ever signalled (by its own PID).
# Requires: node/npm, cargo, python3 (stdlib only), gzip, ps.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNS="${RUNS:-5}"
SETTLE_SECONDS="${SETTLE_SECONDS:-5}"
SKIP_BUILD="${SKIP_BUILD:-0}"
# Measures the real `infra` binary with web/dist embedded, started as `infra serve` on a throwaway
# database (ADR 0032, 0036).
UI_DIR="$ROOT/web"
PKG=cli
BIN="$ROOT/target/release/infra"
TARGET=infra
WORK="$(mktemp -d)"
PIDFILE="$WORK/pid"

cleanup() {
  # Kill only the PID this script itself started (recorded by the helper).
  if [[ -f "$PIDFILE" ]]; then
    local pid
    pid="$(cat "$PIDFILE" 2>/dev/null || true)"
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
      kill -TERM "$pid" 2>/dev/null || true
    fi
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

if [[ "$SKIP_BUILD" != "1" ]]; then
  (cd "$UI_DIR" && npm ci --no-audit --no-fund >&2 && npm run build >&2)
  (cd "$ROOT" && cargo build --release -p "$PKG" >&2)
fi

[[ -x "$BIN" ]] || { echo "missing $BIN" >&2; exit 1; }

file_size() { wc -c <"$1" | tr -d ' '; }
gz_size() { gzip -9 -c "$1" | wc -c | tr -d ' '; }

# Bundle sizes: one JSON object per file under web/dist.
bundle_json=""
total_raw=0
total_gz=0
while IFS= read -r f; do
  raw="$(file_size "$f")"
  gz="$(gz_size "$f")"
  total_raw=$((total_raw + raw))
  total_gz=$((total_gz + gz))
  rel="${f#"$UI_DIR/dist/"}"
  bundle_json+="${bundle_json:+,}{\"file\":\"$rel\",\"raw\":$raw,\"gzip\":$gz}"
done < <(find "$UI_DIR/dist" -type f | LC_ALL=C sort)

bin_size="$(file_size "$BIN")"

run_once() {
  python3 - "$BIN" "$PIDFILE" "$SETTLE_SECONDS" "$TARGET" "$WORK" <<'PY'
import os, re, subprocess, sys, time, urllib.request, signal, json

binary, pidfile, settle, target, work = sys.argv[1], sys.argv[2], float(sys.argv[3]), sys.argv[4], sys.argv[5]
env = dict(os.environ)
args = [binary]
if target == "infra":
    env["NODIVARA_DB"] = os.path.join(work, "measure.db")
    if not os.path.exists(env["NODIVARA_DB"]):
        subprocess.run([binary, "init", "--name", "Measure", "--slug", "measure", "--email", "owner@example.test"],
                       env=env, check=True, stdout=subprocess.DEVNULL)
    args = [binary, "serve", "--port", "0"]
t0 = time.perf_counter()
proc = subprocess.Popen(args, env=env, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
with open(pidfile, "w") as fh:
    fh.write(str(proc.pid))
try:
    url = None
    while url is None:
        line = proc.stdout.readline()
        if not line:
            raise SystemExit("server exited before printing its address")
        m = re.search(r"http://127\.0\.0\.1:\d+", line) or re.search(r"\d+\.\d+\.\d+\.\d+:\d+", line)
        if m:
            url = (m.group(0) if m.group(0).startswith("http") else "http://" + m.group(0)) + "/"
    while True:
        try:
            with urllib.request.urlopen(url, timeout=2) as r:
                if r.status == 200:
                    break
        except Exception:
            if proc.poll() is not None:
                raise SystemExit("server exited early")
            time.sleep(0.001)
    ready_ms = (time.perf_counter() - t0) * 1000
    time.sleep(settle)
    rss = subprocess.run(["ps", "-o", "rss=", "-p", str(proc.pid)],
                         capture_output=True, text=True).stdout.strip()
    print(json.dumps({"ready_ms": round(ready_ms, 2), "rss_kib": int(rss)}))
finally:
    if proc.poll() is None:
        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
PY
  : >"$PIDFILE"
}

: >"$WORK/runs"
for _ in $(seq 1 "$RUNS"); do
  run_once >>"$WORK/runs"
done

python3 - "$WORK/runs" "$bin_size" "$total_raw" "$total_gz" "$bundle_json" <<'PY'
import json, platform, statistics, sys

runs = [json.loads(l) for l in open(sys.argv[1]) if l.strip()]
bundle = json.loads("[" + sys.argv[5] + "]")
out = {
    "machine": {"os": platform.system(), "release": platform.release(), "arch": platform.machine()},
    "runs": len(runs),
    "ui_bundle": {"files": bundle, "total_raw_bytes": int(sys.argv[3]), "total_gzip_bytes": int(sys.argv[4])},
    "binary_release_bytes": int(sys.argv[2]),
    "idle_rss_kib_median": statistics.median(r["rss_kib"] for r in runs),
    "startup_to_200_ms_median": round(statistics.median(r["ready_ms"] for r in runs), 2),
    "samples": runs,
}
print(json.dumps(out, indent=2))
PY
