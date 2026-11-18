#!/usr/bin/env bash
# Performance measurement for the M0 prototype (docs/decisions/0010).
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
BIN="$ROOT/target/release/proto-server"
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
  (cd "$ROOT/ui" && npm ci --no-audit --no-fund >&2 && npm run build >&2)
  (cd "$ROOT" && cargo build --release -p proto-server >&2)
fi

[[ -x "$BIN" ]] || { echo "missing $BIN" >&2; exit 1; }

file_size() { wc -c <"$1" | tr -d ' '; }
gz_size() { gzip -9 -c "$1" | wc -c | tr -d ' '; }

# Bundle sizes: one JSON object per file under ui/dist.
bundle_json=""
total_raw=0
total_gz=0
while IFS= read -r f; do
  raw="$(file_size "$f")"
  gz="$(gz_size "$f")"
  total_raw=$((total_raw + raw))
  total_gz=$((total_gz + gz))
  rel="${f#"$ROOT/ui/dist/"}"
  bundle_json+="${bundle_json:+,}{\"file\":\"$rel\",\"raw\":$raw,\"gzip\":$gz}"
done < <(find "$ROOT/ui/dist" -type f | LC_ALL=C sort)

bin_size="$(file_size "$BIN")"

run_once() {
  python3 - "$BIN" "$PIDFILE" "$SETTLE_SECONDS" <<'PY'
import subprocess, sys, time, urllib.request, signal, json

binary, pidfile, settle = sys.argv[1], sys.argv[2], float(sys.argv[3])
t0 = time.perf_counter()
proc = subprocess.Popen([binary], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
with open(pidfile, "w") as fh:
    fh.write(str(proc.pid))
try:
    line = proc.stdout.readline().strip()
    addr = line.split(" ", 1)[1]
    url = f"http://{addr}/"
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
