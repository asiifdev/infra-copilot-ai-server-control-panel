#!/usr/bin/env bash
set -euo pipefail

# Pre-push CI check: runs locally what GitHub Actions runs on all three OSes.
# Cross-compilation to Windows is not available without rustup, so this script
# uses heuristics to catch the most common cross-platform mistakes.

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BOLD='\033[1m'
RESET='\033[0m'

pass=0
fail=0
warn=0

ok()   { pass=$((pass + 1)); printf "${GREEN}  ✓ %s${RESET}\n" "$1"; }
err()  { fail=$((fail + 1)); printf "${RED}  ✗ %s${RESET}\n" "$1"; }
skip() { warn=$((warn + 1)); printf "${YELLOW}  ⚠ %s${RESET}\n" "$1"; }

section() { printf "\n${BOLD}── %s${RESET}\n" "$1"; }

cd "$(git rev-parse --show-toplevel)"

# ── 1. Format ──────────────────────────────────────────────────────────────
section "Format"
if cargo fmt --all --check >/dev/null 2>&1; then
    ok "cargo fmt"
else
    err "cargo fmt --check failed (run: cargo fmt --all)"
fi

# ── 2. Clippy (warnings = errors, same as CI) ─────────────────────────────
section "Clippy"
if cargo clippy --workspace --all-targets --locked -- -D warnings 2>&1; then
    ok "cargo clippy"
else
    err "cargo clippy failed"
fi

# ── 3. Tests ───────────────────────────────────────────────────────────────
section "Tests"
if cargo test --workspace --locked 2>&1; then
    ok "cargo test"
else
    err "cargo test failed"
fi

# ── 4. Web UI ──────────────────────────────────────────────────────────────
section "Web UI"
if [ -f web/package.json ]; then
    if (cd web && npm run check 2>&1) && (cd web && npm test 2>&1) && (cd web && npm run build 2>&1); then
        ok "web check + test + build"
    else
        err "web UI check/test/build failed"
    fi
else
    skip "web/package.json not found, skipping"
fi

# ── 5. Docs ────────────────────────────────────────────────────────────────
section "Docs"
if python3 scripts/check-docs.py 2>&1; then
    ok "check-docs.py"
else
    err "check-docs.py failed"
fi
if python3 scripts/check-palette.py 2>&1; then
    ok "check-palette.py"
else
    err "check-palette.py failed"
fi

# ── 6. Secrets (personal data) ────────────────────────────────────────────
section "Secrets"
if [ -f scripts/check-secrets.sh ]; then
    if bash scripts/check-secrets.sh 2>&1; then
        ok "check-secrets.sh"
    else
        err "check-secrets.sh failed"
    fi
fi
if [ -f scripts/check-personal-data.sh ]; then
    if bash scripts/check-personal-data.sh 2>&1; then
        ok "check-personal-data.sh"
    else
        err "check-personal-data.sh failed"
    fi
fi

# ── 7. Gitleaks (if installed) ─────────────────────────────────────────────
section "Gitleaks"
if command -v gitleaks >/dev/null 2>&1; then
    if gitleaks git . --redact --no-banner --verbose 2>&1; then
        ok "gitleaks"
    else
        err "gitleaks found leaks"
    fi
else
    skip "gitleaks not installed (brew install gitleaks)"
fi

# ── 8. Windows cross-platform heuristics ──────────────────────────────────
section "Windows compatibility (heuristics)"
win_issues=0

# 8a. Items used behind #[cfg(unix)] imports but referenced outside cfg(unix)
# Check for types commonly gated behind cfg(unix) that are used without the gate
while IFS= read -r file; do
    # Find types imported under #[cfg(unix)]
    unix_types=()
    while IFS= read -r line; do
        # Extract type names from use statements under cfg(unix)
        for t in OsString OsStr OwnedFd RawFd SignalKind; do
            if echo "$line" | grep -qw "$t"; then
                unix_types+=("$t")
            fi
        done
    done < <(awk '/#\[cfg\(unix\)\]/{getline; print}' "$file")

    for t in "${unix_types[@]:-}"; do
        [ -z "$t" ] && continue
        # Find uses of the type outside of cfg(unix) blocks (rough heuristic:
        # lines that reference the type but are NOT immediately preceded by cfg(unix))
        while IFS=: read -r lineno content; do
            prev_line=$((lineno - 1))
            prev=$(sed -n "${prev_line}p" "$file")
            prev2_line=$((lineno - 2))
            prev2=$(sed -n "${prev2_line}p" "$file" 2>/dev/null || true)
            if ! echo "$prev" | grep -q 'cfg(unix)' && \
               ! echo "$prev2" | grep -q 'cfg(unix)' && \
               ! echo "$content" | grep -q '#\[cfg' && \
               ! echo "$content" | grep -q '^\s*use ' && \
               ! echo "$content" | grep -q '^\s*//' ; then
                # Check if inside a #[cfg(unix)] fn (rough: look back for fn with cfg)
                in_unix_fn=$(awk -v line="$lineno" '
                    /#\[cfg\(unix\)\]/ { saw_cfg = NR }
                    /^[[:space:]]*(pub )?(async )?fn / {
                        if (saw_cfg == NR-1 || saw_cfg == NR-2) fn_start = NR
                        else fn_start = 0
                    }
                    NR == line { if (fn_start > 0) print "yes"; else print "no" }
                ' "$file")
                if [ "$in_unix_fn" != "yes" ]; then
                    printf "  ${YELLOW}  ⚠ %s:%s uses %s without #[cfg(unix)]${RESET}\n" \
                        "$file" "$lineno" "$t"
                    win_issues=$((win_issues + 1))
                fi
            fi
        done < <(grep -n "\b${t}\b" "$file" | grep -v '^\s*use ' | grep -v '^\s*//' | grep -v '#\[cfg')
    done
done < <(find crates -name '*.rs' -type f)

# 8b. Check for unix-only std APIs used without cfg gate
for pattern in 'std::os::unix' 'nix::' 'libc::' 'signal(SignalKind'; do
    while IFS=: read -r file lineno content; do
        prev_line=$((lineno - 1))
        prev=$(sed -n "${prev_line}p" "$file")
        if ! echo "$prev" | grep -q 'cfg(unix)' && \
           ! echo "$content" | grep -q 'cfg(unix)' && \
           ! echo "$content" | grep -q '^\s*//'; then
            printf "  ${YELLOW}  ⚠ %s:%s uses %s without cfg(unix)${RESET}\n" \
                "$file" "$lineno" "$pattern"
            win_issues=$((win_issues + 1))
        fi
    done < <(grep -rn "$pattern" crates/ --include='*.rs' 2>/dev/null || true)
done

if [ "$win_issues" -eq 0 ]; then
    ok "no obvious Windows issues found"
else
    skip "$win_issues potential Windows issue(s) — review above"
fi

# ── Summary ────────────────────────────────────────────────────────────────
section "Summary"
printf "  ${GREEN}passed: %d${RESET}  " "$pass"
[ "$warn" -gt 0 ] && printf "${YELLOW}warnings: %d${RESET}  " "$warn"
if [ "$fail" -gt 0 ]; then
    printf "${RED}failed: %d${RESET}\n" "$fail"
    exit 1
else
    printf "${GREEN}all checks passed${RESET}\n"
fi
