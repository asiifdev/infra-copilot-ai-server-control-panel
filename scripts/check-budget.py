#!/usr/bin/env python3
"""Compare scripts/measure.sh output with the performance budget (docs/decisions/0005, 0010).

Usage: [BUDGET_PROFILE=infra] check-budget.py MEASURE_JSON [BUDGET_JSON]
BUDGET_PROFILE selects a named block of the budget file (e.g. "infra" for the real binary with the
embedded web UI, measured with TARGET=infra scripts/measure.sh); unset uses the M0 probe's block.
Exits 1 when any metric is over budget or missing. Prints a Markdown table, also appended to
$GITHUB_STEP_SUMMARY when that variable is set. Standard library only.
"""
import json
import os
import platform
import sys

LABELS = {
    "ui_gzip_bytes": ("UI bundle, gzip", "B"),
    "ui_js_raw_bytes": ("UI JavaScript, raw", "B"),
    "binary_bytes": ("Release binary", "B"),
    "idle_rss_kib": ("Idle RSS (5 s)", "KiB"),
    "startup_ms": ("Exec to HTTP 200 (median)", "ms"),
}


def fmt(value, unit):
    number = f"{value:,.2f}" if isinstance(value, float) and not value.is_integer() else f"{int(value):,}"
    return f"{number} {unit}"


def measured(data):
    files = data["ui_bundle"]["files"]
    return {
        "ui_gzip_bytes": data["ui_bundle"]["total_gzip_bytes"],
        "ui_js_raw_bytes": sum(f["raw"] for f in files if f["file"].endswith(".js")),
        "binary_bytes": data["binary_release_bytes"],
        "idle_rss_kib": data["idle_rss_kib_median"],
        "startup_ms": data["startup_to_200_ms_median"],
    }


def main(argv):
    if len(argv) not in (2, 3):
        print(__doc__, file=sys.stderr)
        return 2
    here = os.path.dirname(os.path.abspath(__file__))
    budget_path = argv[2] if len(argv) == 3 else os.path.join(here, "..", "docs", "perf-budget.json")
    with open(argv[1], encoding="utf-8") as fh:
        got = measured(json.load(fh))
    with open(budget_path, encoding="utf-8") as fh:
        raw = json.load(fh)
    profile = os.environ.get("BUDGET_PROFILE")
    if profile:
        raw = raw[profile]
    budget = dict(raw["default"])
    budget.update(raw.get(platform.system(), {}))

    rows, failed = [], False
    for key, (label, unit) in LABELS.items():
        limit, value = budget.get(key), got.get(key)
        if limit is None or value is None:
            rows.append(f"| {label} | {value} | missing budget or value | FAIL |")
            failed = True
            continue
        ok = value <= limit
        failed |= not ok
        rows.append(f"| {label} | {fmt(value, unit)} | {fmt(limit, unit)} | {'ok' if ok else 'OVER BUDGET'} |")

    table = "\n".join(
        [f"### Performance budget {profile or 'probe'} ({platform.system()} {platform.machine()})", "",
         "| Metric | Measured | Budget | Result |", "|---|---|---|---|", *rows]
    )
    print(table)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as fh:
            fh.write(table + "\n")
    if failed:
        print("\nPerformance budget exceeded: explain it in a decision record instead of raising the number.",
              file=sys.stderr)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
