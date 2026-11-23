#!/usr/bin/env bash
# Smoke test of the real binary: start `infra serve` on a throwaway database and check that the
# embedded UI and the API guard answer as documented. Run after scripts/build-web.sh and
# `cargo build -p cli` (CI does both). Needs only curl and a POSIX shell.
#
# Only the server process this script started is ever signalled, by its own PID. It binds
# 127.0.0.1 on a free port and uses a temporary directory that is removed on exit.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INFRA="${INFRA:-$ROOT/target/debug/infra}"
[[ -x "$INFRA" ]] || { echo "missing $INFRA (cargo build -p cli)" >&2; exit 2; }

WORK="$(mktemp -d)"
PID=""
cleanup() {
  [[ -n "$PID" ]] && kill -TERM "$PID" 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

export NODIVARA_DB="$WORK/smoke.db"
"$INFRA" init --name "Smoke Org" --slug smoke --email owner@example.test >/dev/null
"$INFRA" serve --port 0 >"$WORK/out" 2>"$WORK/err" &
PID=$!

URL=""
for _ in $(seq 1 50); do
  URL="$(grep -o 'http://127\.0\.0\.1:[0-9]*' "$WORK/out" | head -1 || true)"
  [[ -n "$URL" ]] && break
  sleep 0.2
done
[[ -n "$URL" ]] || { echo "server did not start:" >&2; cat "$WORK/err" >&2; exit 1; }

fail() { echo "SMOKE FAIL: $*" >&2; exit 1; }
code() { curl -s -o /dev/null -w '%{http_code}' "$@"; }

[[ "$(code "$URL/healthz")" == "200" ]] || fail "/healthz is not 200"
page="$(curl -s -H 'Accept: text/html' "$URL/")"
grep -q '<div id="app"' <<<"$page" || fail "the embedded UI is not served at / (is web/dist built before cargo build?)"
grep -q 'scripts/build-web.sh' <<<"$page" && fail "the placeholder page is served: the UI was not embedded"
[[ "$(code "$URL/api/session")" == "401" ]] || fail "/api/session must need a session"
[[ "$(code -H 'Host: evil.example.test' "$URL/api/i18n/en")" == "403" ]] || fail "a foreign Host must be refused"
csp="$(curl -s -D - -o /dev/null "$URL/" | tr -d '\r' | grep -i '^content-security-policy:' || true)"
[[ -n "$csp" ]] || fail "no Content-Security-Policy header"
catalog="$(curl -s "$URL/api/i18n/en")"
grep -q '"ui\.' <<<"$catalog" || fail "the public catalog has no ui.* keys"
echo "smoke ok: $URL"
