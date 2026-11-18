#!/usr/bin/env bash
# Scan tracked (or staged) files for personal data patterns listed in .claude/hygiene-patterns.local
# Usage: scripts/check-personal-data.sh [--staged]
set -u
cd "$(git rev-parse --show-toplevel)" || exit 2
P=".claude/hygiene-patterns.local"
[ -f "$P" ] || { echo "Missing $P (one grep -E pattern per line, # for comments)"; exit 2; }
PATS=$(grep -v -E '^\s*(#|!|@|$)' "$P" | paste -sd'|' -)
# Lines starting with "!" are allowed substrings (for example the repository URL itself); they are removed before matching.
ALLOW=$(grep -E '^!' "$P" | cut -c2- | paste -sd'|' -)
# Lines starting with "@" name files to skip entirely (for example the funding file, which must hold the maintainer's handles).
SKIP=$(grep -E '^@' "$P" | cut -c2-)
[ -n "$PATS" ] || { echo "No patterns in $P"; exit 2; }
if [ "${1:-}" = "--staged" ]; then FILES=$(git diff --cached --name-only --diff-filter=ACMR); else FILES=$(git ls-files); fi
hits=0
for f in $FILES; do
  [ "$f" = "$P" ] && continue
  for skipped in $SKIP; do [ "$f" = "$skipped" ] && continue 2; done
  [ -f "$f" ] || continue
  if [ -n "$ALLOW" ]; then body=$(sed -E "s#($ALLOW)##g" "$f"); else body=$(cat "$f"); fi
  if m=$(printf '%s\n' "$body" | grep -n -i -E -- "$PATS" 2>/dev/null | head -5) && [ -n "$m" ]; then
    echo "== $f"; echo "$m" | cut -c1-140; hits=$((hits+1))
  fi
done
if [ "$hits" -gt 0 ]; then echo; echo "FOUND personal data in $hits file(s)."; exit 1; fi
echo "Clean: no personal data patterns found."
