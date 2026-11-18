#!/usr/bin/env python3
"""Documentation checks used by CI and runnable locally: python3 scripts/check-docs.py

1. Every public document has an English and an Indonesian version.
2. Each version links to the other on its first line.
3. Every relative Markdown link resolves.
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
SKIP_DIRS = ("research_notes/", "reports/", "target/", ".git/")
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
