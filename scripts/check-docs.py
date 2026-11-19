#!/usr/bin/env python3
"""Documentation checks used by CI and runnable locally: python3 scripts/check-docs.py

1. Every public document has an English and an Indonesian version.
2. Each version links to the other on its first line.
3. Every relative Markdown link resolves.
4. Drift against the code: every crate under crates/ is listed in both READMEs and in the
   structure line of CLAUDE.md, decision files (docs/decisions/NNNN-*.md) are numbered without
   gaps or duplicates, and database migrations (crates/db/migrations/NNNN_*.sql) likewise.
5. No phrase from STALE_PHRASES (statements that were once true and no longer are) appears
   outside docs/decisions/.
Unset placeholders are reported as notices; they do not fail the check.
"""

import glob
import os
import re
import sys

PAIRS = [
    "README", "CONTRIBUTING", "CODE_OF_CONDUCT", "SECURITY", "LICENSING", "FUNDING",
    "docs/faq", "docs/comparison",
]
SKIP_DIRS = ("research_notes/", "reports/", "target/", ".git/", "ui/node_modules/")
failures = []


def read(path):
    with open(path, encoding="utf-8") as handle:
        return handle.read()


for base in PAIRS:
    en, idn = f"{base}.md", f"{base}.id.md"
    for path in (en, idn):
        if not os.path.exists(path):
            failures.append(f"missing {path}")
    if os.path.exists(en) and os.path.exists(idn):
        if os.path.basename(idn) not in read(en).split("\n", 1)[0]:
            failures.append(f"{en}: first line must link to {os.path.basename(idn)}")
        if os.path.basename(en) not in read(idn).split("\n", 1)[0]:
            failures.append(f"{idn}: first line must link to {os.path.basename(en)}")

markdown = [p for p in glob.glob("**/*.md", recursive=True) if not p.startswith(SKIP_DIRS)]
for path in markdown:
    for match in re.finditer(r"\]\(([^)#\s]+)(#[^)]*)?\)", read(path)):
        target = match.group(1)
        if re.match(r"^(https?:|mailto:)", target):
            continue
        resolved = os.path.normpath(os.path.join(os.path.dirname(path), target))
        if not os.path.exists(resolved):
            failures.append(f"{path}: broken link -> {target}")

# 4. Drift against the code.
crate_dirs = sorted(
    d for d in os.listdir("crates") if os.path.isfile(os.path.join("crates", d, "Cargo.toml"))
) if os.path.isdir("crates") else []
claude = read("CLAUDE.md") if os.path.exists("CLAUDE.md") else ""
structure = next((l for l in claude.splitlines() if l.startswith("crates/") and "workspace" in l), "")
for crate in crate_dirs:
    for readme in ("README.md", "README.id.md"):
        if os.path.exists(readme) and f"crates/{crate}" not in read(readme):
            failures.append(f"{readme}: crate '{crate}' is not listed (crates/{crate})")
    if not re.search(rf"(?<![\w-]){re.escape(crate)}(?![\w-])", structure):
        failures.append(f"CLAUDE.md: crate '{crate}' is missing from the Struktur crates line")


def check_sequence(label, pattern, directory):
    numbers = []
    for name in sorted(os.listdir(directory)) if os.path.isdir(directory) else []:
        match = re.match(pattern, name)
        if match:
            numbers.append(int(match.group(1)))
    for expected, found in enumerate(numbers, start=1 if numbers and numbers[0] == 1 else numbers[0] if numbers else 0):
        if found != expected:
            failures.append(f"{label}: numbering is not contiguous around {found:04d} (expected {expected:04d})")
            break


check_sequence("docs/decisions", r"^(\d{4})-.*\.md$", "docs/decisions")
check_sequence("crates/db/migrations", r"^(\d{4})_.*\.sql$", "crates/db/migrations")

# Phrases that were true once and must not come back outside the (historical) decision records.
STALE_PHRASES = [
    "Nama produk belum ditentukan",
    "product name is not decided",
    "belum ada kode",
]
for path in markdown:
    if path.startswith("docs/decisions/"):
        continue
    text = read(path)
    for phrase in STALE_PHRASES:
        if phrase.lower() in text.lower():
            failures.append(f"{path}: stale phrase '{phrase}'")

notices = []
for path in markdown:
    if re.search(r"conduct contact|kontak pelaporan", read(path)):
        notices.append(f"{path}: contact placeholder is still unset")

for line in notices:
    sys.stdout.write(f"notice: {line}\n")
if failures:
    for line in failures:
        sys.stdout.write(f"FAIL: {line}\n")
    sys.exit(1)
sys.stdout.write(f"docs ok: {len(PAIRS)} language pairs, {len(markdown)} files checked\n")
