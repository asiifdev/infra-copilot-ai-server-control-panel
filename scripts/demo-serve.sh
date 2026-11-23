#!/usr/bin/env bash
# Start `infra serve` on throwaway demo data, to look at and test the web UI.
#
#   scripts/demo-serve.sh            demo organization, two nodes, a comparison, a transport and a draft plan
#   scripts/demo-serve.sh --fake-ai  also a loopback "AI provider" (scripts/fake-openai.py) set as default
#
# Everything lives in a temporary directory that is removed on exit (Ctrl-C). Nothing touches your
# real data: the database, the master key and the fake provider key are made up here, the inventories
# come from the fixture root file systems in crates/collectors/tests/fixtures, and the server binds
# 127.0.0.1 on a free port. Requires: cargo, python3, openssl. The UI must be built (web/dist) to be
# served; run scripts/build-web.sh first (without it the server shows a placeholder page).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FAKE_AI=0
[[ "${1:-}" == "--fake-ai" ]] && FAKE_AI=1

WORK="$(mktemp -d)"
PIDS=()
cleanup() {
  # Only the processes this script started, by their own PIDs.
  for pid in "${PIDS[@]:-}"; do
    [[ -n "$pid" ]] && kill -TERM "$pid" 2>/dev/null || true
  done
  rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

(cd "$ROOT" && cargo build -p cli >&2)
INFRA="$ROOT/target/debug/infra"
FIXTURES="$ROOT/crates/collectors/tests/fixtures"

export NODIVARA_DB="$WORK/demo.db"
export NODIVARA_MASTER_KEY="k1:$(openssl rand -hex 32)"

"$INFRA" init --name "Demo Org" --slug demo --email owner@example.test >&2
"$INFRA" node add --name old-server --kind vps >&2
"$INFRA" node add --name new-server --kind vps >&2
"$INFRA" collect --root "$FIXTURES/full" --os linux --arch x86_64 --save --node old-server >/dev/null
"$INFRA" collect --root "$FIXTURES/empty" --os linux --arch x86_64 --save --node new-server >/dev/null
"$INFRA" compare --save --source-node old-server --target-node new-server >/dev/null || true
"$INFRA" transport add --name offsite-demo --kind rsync --host backup.example.test --user backup --path /srv/backups >&2

# A plan can only be drafted against a verified transport; mark the demo one as verified directly.
python3 - "$NODIVARA_DB" <<'PY'
import sqlite3, sys
db = sqlite3.connect(sys.argv[1])
db.execute("UPDATE transports SET verified_at = strftime('%Y-%m-%dT%H:%M:%fZ','now'), last_check = '{\"ok\":true}'")
db.commit()
PY
"$INFRA" plan draft --source old-server --target new-server --transport offsite-demo --name demo-plan >/dev/null

if [[ "$FAKE_AI" == "1" ]]; then
  printf 'fake-%s' "$(openssl rand -hex 10)" > "$WORK/fake.key"
  python3 "$ROOT/scripts/fake-openai.py" 58123 "$WORK/fake.key" &
  PIDS+=("$!")
  sleep 1
  "$INFRA" provider add --name fake --base-url http://127.0.0.1:58123/v1 --model fake-1 \
    --params '{"allow_private_network":true}' --default --key-stdin < "$WORK/fake.key" >&2
fi

echo "Demo data ready. Press Ctrl-C to stop and delete it." >&2
"$INFRA" serve --port 0 &
PIDS+=("$!")
wait "${PIDS[${#PIDS[@]}-1]}"
