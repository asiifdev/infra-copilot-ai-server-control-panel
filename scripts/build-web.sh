#!/usr/bin/env bash
# Builds the web UI (web/) into web/dist, which the `server` crate embeds into the `infra`
# binary at compile time (decision 0030). Usable locally and in CI; rebuild the binary afterwards
# (`cargo build --release -p cli`) so it carries the new files.
#
# Steps: npm ci (exact lockfile install), type and Svelte checks, unit tests, production build.
# Does nothing when web/ does not exist yet.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ ! -d web ]; then
  echo "web/ does not exist yet: nothing to build (the binary then serves a placeholder page)." >&2
  exit 0
fi

if ! command -v npm >/dev/null 2>&1; then
  echo "npm is not installed (Node.js is needed to build the web UI)." >&2
  exit 2
fi

cd web
npm ci
npm run check
npm test
npm run build

if [ ! -f dist/index.html ]; then
  echo "the build did not produce web/dist/index.html" >&2
  exit 1
fi
echo "web/dist is ready ($(find dist -type f | wc -l | tr -d ' ') files). Rebuild the binary to embed it."
