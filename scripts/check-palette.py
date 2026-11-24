#!/usr/bin/env python3
"""Checks the Nodivara colour tokens (web/src/tokens.css): WCAG 2.2 contrast (AAA 7:1 for every text token on every surface, 3:1 for UI boundaries)
in the light and dark themes, and that the two dark blocks are identical. Exit 1 on failure."""
import re
import sys
from pathlib import Path

CSS = Path(__file__).resolve().parent.parent / "web" / "src" / "tokens.css"
TEXT = 7.0  # WCAG AAA, normal text
UI = 3.0  # non-text UI: focus ring, control borders, button fill (WCAG 2.2 1.4.11)

SURFACES = ["bg", "surface", "surface-2"]
TEXT_TOKENS = ["text", "text-muted", "link", "success", "warning", "danger", "info"]
UI_TOKENS = ["primary", "primary-hover", "focus", "border-strong"]
PAIRS = (
    [(t, s, TEXT) for t in TEXT_TOKENS for s in SURFACES]
    + [("on-primary", "primary", TEXT), ("on-primary", "primary-hover", TEXT),
       ("on-accent", "accent", TEXT)]
    + [(u, s, UI) for u in UI_TOKENS for s in SURFACES]
)


def lum(hexcolor):
    channels = [int(hexcolor[i:i + 2], 16) / 255 for i in (1, 3, 5)]
    linear = [c / 12.92 if c <= 0.03928 else ((c + 0.055) / 1.055) ** 2.4 for c in channels]
    return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2]


def ratio(a, b):
    hi, lo = sorted((lum(a), lum(b)), reverse=True)
    return (hi + 0.05) / (lo + 0.05)


def blocks(text):
    found = re.findall(r"\{([^{}]*)\}", text)
    return [dict(re.findall(r"--nv-([a-z0-9-]+):\s*(#[0-9a-fA-F]{6})", body)) for body in found]


def main():
    themes = [b for b in blocks(CSS.read_text()) if b]
    if len(themes) != 3:
        print(f"expected 3 token blocks (light, dark media, dark attribute), found {len(themes)}")
        return 1
    light, dark_media, dark_attr = themes
    failed = 0
    if dark_media != dark_attr:
        print("the two dark blocks differ")
        failed += 1
    for name, theme in (("light", light), ("dark", dark_attr)):
        for fg, bg, need in PAIRS:
            if fg not in theme or bg not in theme:
                print(f"{name}: missing token {fg} or {bg}")
                failed += 1
                continue
            got = ratio(theme[fg], theme[bg])
            if got < need:
                print(f"{name}: {fg} {theme[fg]} on {bg} {theme[bg]} = {got:.2f} (needs {need})")
                failed += 1
    if failed:
        print(f"palette check failed: {failed} problem(s)")
        return 1
    print(f"palette ok: {len(PAIRS)} pairs x 2 themes")
    return 0


sys.exit(main())
