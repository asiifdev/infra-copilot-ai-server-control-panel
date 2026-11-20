#!/usr/bin/env bash
# Local counterpart of the CI "Secret scan" job: run before pushing.
# Scans (1) the commits that are not on origin/main yet and (2) the files with uncommitted changes
# (modified or new, not ignored), values redacted. Committed files are covered by (1); build output
# and agent worktrees are never scanned.
# CI scans the full history; a finding in an already pushed commit needs a .gitleaksignore entry.
set -euo pipefail
cd "$(dirname "$0")/.."

if ! command -v gitleaks >/dev/null 2>&1; then
  echo "gitleaks is not installed (brew install gitleaks); CI will run it anyway." >&2
  exit 2
fi

base="origin/main"
if git rev-parse --verify --quiet "$base" >/dev/null; then
  gitleaks detect --source . --log-opts "$base..HEAD" --redact --no-banner
else
  gitleaks detect --source . --redact --no-banner
fi

# Uncommitted files: modified or new, minus ignored.
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
git ls-files -z --modified --others --exclude-standard | grep -z -v '^\.claude/worktrees/' \
  | tar --null -T - -cf - 2>/dev/null | tar -xf - -C "$tmp"
gitleaks detect --source "$tmp" --no-git --redact --no-banner
echo "No secrets found in unpushed commits or uncommitted files."
