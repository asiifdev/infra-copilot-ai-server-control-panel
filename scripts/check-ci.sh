#!/usr/bin/env bash
set -euo pipefail

# Pre-push CI check: runs locally what GitHub Actions runs on all three OSes.
# Full cross-compilation to Windows is impossible from macOS (C deps like ring,
# libsqlite3-sys need MSVC), so this script catches Windows issues via heuristics
# that match every real CI failure pattern we have seen so far.

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

# Ensure rustup cargo-fmt works when both Homebrew and rustup coexist.
if [ -x "$HOME/.cargo/bin/rustup" ]; then
    export PATH="$HOME/.cargo/bin:$PATH"
fi

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

# ── 8. Windows compatibility (heuristic linter) ───────────────────────────
# Catches every failure pattern seen in CI so far:
#   - unix-only types (OsString, OwnedFd, ...) used outside #[cfg(unix)]
#   - /dev/* paths in tests without #[cfg(unix)]
#   - unix-only std/libc APIs without cfg gate
#   - structs/enums/methods only used in unix code but not gated (dead_code on Windows)
section "Windows compatibility (heuristics)"
win_issues=0

# Helper: check whether a line number is inside a #[cfg(unix)] or #[cfg(target_os = "linux")] block.
# Recognizes file-level #![cfg(...)], mod/fn/struct/enum/impl blocks, and local #[cfg(...)] blocks.
in_unix_scope() {
    local file="$1" lineno="$2"
    awk -v line="$lineno" '
        /^[[:space:]]*#!\[cfg\((unix|target_os = "linux")\)\]/ { file_unix = 1 }
        /^[[:space:]]*#\[cfg\((unix|target_os = "linux")\)\]/ { saw_cfg = NR }
        /^[[:space:]]*(pub )?(unsafe )?(async )?(mod |fn |struct |enum |impl |let )/ {
            if (saw_cfg >= NR-3 && saw_cfg < NR) {
                block_depth = depth
                in_unix_block = 1
            } else if (!in_unix_block) {
                in_unix_block = 0
            }
            saw_cfg = 0
        }
        {
            # Track brace depth to exit mod/fn/let #[cfg(unix)] scopes accurately
            n_open = gsub(/\{/, "{", $0)
            n_close = gsub(/\}/, "}", $0)
            depth += n_open - n_close
            if (in_unix_block && depth <= block_depth && n_close > 0) {
                in_unix_block = 0
            }
        }
        /^[[:space:]]*#\[cfg\((unix|target_os = "linux")\)\]/ && NR >= line-2 && NR <= line { found_inline = 1 }
        NR == line {
            if (file_unix || in_unix_block || found_inline) print "yes"
            else print "no"
        }
    ' "$file"
}

# 8a. Unix-only types used without cfg(unix) gate
UNIX_TYPES="OsString OsStr OwnedFd RawFd BorrowedFd SignalKind"
while IFS= read -r file; do
    # Which unix-only types does this file import under #[cfg(unix)]?
    file_unix_types=()
    for t in $UNIX_TYPES; do
        if awk '/#\[cfg\(unix\)\]/{getline; if ($0 ~ /\b'"$t"'\b/) print "yes"}' "$file" | grep -q yes; then
            file_unix_types+=("$t")
        fi
    done
    [ ${#file_unix_types[@]} -eq 0 ] && continue

    for t in "${file_unix_types[@]}"; do
        while IFS=: read -r lineno content; do
            # Skip use/import lines and comments
            echo "$content" | grep -qE '^\s*(use |//)' && continue
            scope=$(in_unix_scope "$file" "$lineno")
            if [ "$scope" != "yes" ]; then
                printf "${RED}  ✗ %s:%s uses unix-only type '%s' without #[cfg(unix)]${RESET}\n" \
                    "$file" "$lineno" "$t"
                win_issues=$((win_issues + 1))
            fi
        done < <(grep -n "\b${t}\b" "$file" | grep -vE '^\s*(use |.*//.*'"$t"')')
    done
done < <(find crates -name '*.rs' -type f)

# 8b. /dev/* paths that access the filesystem without #[cfg(unix)]
while IFS=: read -r file lineno content; do
    # Skip comments
    echo "$content" | grep -qE '^\s*//' && continue
    # Skip diff output strings ("+++ /dev/null", "--- /dev/null") — just text, not fs access
    echo "$content" | grep -qE '\+\+\+ /dev/null|--- /dev/null' && continue
    # Skip string-only data in arrays/matches (parse tests, bad-input lists)
    echo "$content" | grep -qE 'parse_socket_link|^\s*"/dev/' && continue

    # Check: is there a #[cfg(unix)] anywhere in the 20 lines before this line?
    start=$((lineno - 20))
    [ "$start" -lt 1 ] && start=1
    if sed -n "${start},${lineno}p" "$file" | grep -q '#\[cfg(unix)\]'; then
        continue
    fi
    # Also check fn/struct-level scope
    scope=$(in_unix_scope "$file" "$lineno")
    if [ "$scope" = "yes" ]; then
        continue
    fi
    printf "${RED}  ✗ %s:%s uses /dev/* path without #[cfg(unix)] — will fail on Windows${RESET}\n" \
        "$file" "$lineno"
    win_issues=$((win_issues + 1))
done < <(grep -rn '"/dev/' crates/ --include='*.rs' 2>/dev/null || true)

# 8c. Unix-only std APIs & commands without cfg gate
for pattern in 'std::os::unix' '\bnix::' '\blibc::' 'signal\(SignalKind' 'Command::new\("(kill|mkfifo|chmod|ln)"\)'; do
    while IFS=: read -r file lineno content; do
        echo "$content" | grep -qE '^\s*//' && continue
        scope=$(in_unix_scope "$file" "$lineno")
        if [ "$scope" != "yes" ]; then
            printf "${YELLOW}  ⚠ %s:%s uses '%s' without cfg(unix)${RESET}\n" \
                "$file" "$lineno" "$pattern"
            win_issues=$((win_issues + 1))
        fi
    done < <(grep -rnE "$pattern" crates/ --include='*.rs' 2>/dev/null || true)
done

# 8d. Fake secret patterns that gitleaks would flag (high-entropy API key lookalikes)
while IFS=: read -r file lineno content; do
    echo "$content" | grep -qE '^\s*//' && continue
    if echo "$content" | grep -qiE '(sk-proj-|sk-live-|ghp_|ghr_|AKIA)[A-Za-z0-9]{10,}'; then
        printf "${RED}  ✗ %s:%s has a fake key that looks real to gitleaks — use a low-entropy placeholder${RESET}\n" \
            "$file" "$lineno"
        win_issues=$((win_issues + 1))
    fi
done < <(grep -rn -E '(key|token|secret|password)\s*[=:]\s*"' crates/ --include='*.rs' 2>/dev/null || true)

if [ "$win_issues" -eq 0 ]; then
    ok "no Windows / cross-platform issues found"
else
    err "$win_issues cross-platform issue(s) found — fix before pushing"
fi

# 8e. Cross-compile check for Windows when rustup + x86_64-pc-windows-gnu are installed
if [ -x "$HOME/.cargo/bin/rustup" ] && "$HOME/.cargo/bin/rustup" target list --installed 2>/dev/null | grep -q '^x86_64-pc-windows-gnu$'; then
    section "Windows cross-check (x86_64-pc-windows-gnu)"
    if PATH="$HOME/.cargo/bin:$PATH" cargo check --workspace --all-targets --locked --target x86_64-pc-windows-gnu 2>&1; then
        ok "cargo check --target x86_64-pc-windows-gnu"
    else
        err "Windows cross-check failed"
    fi
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
