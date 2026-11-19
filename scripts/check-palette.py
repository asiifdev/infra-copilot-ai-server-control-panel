#!/usr/bin/env python3
"""Checks the Nodivara colour tokens (ui/src/tokens.css): WCAG 2.2 contrast for text and UI pairs
in the light and dark themes, and that the two dark blocks are identical. Exit 1 on failure."""
import re
import sys
from pathlib import Path

CSS = Path(__file__).resolve().parent.parent / "ui" / "src" / "tokens.css"
TEXT = 4.5  # normal text
UI = 3.0  # non-text UI (focus ring, control borders)

PAIRS = [
    ("text", "bg", TEXT), ("text", "surface", TEXT), ("text", "surface-2", TEXT),
    ("text-muted", "bg", TEXT), ("text-muted", "surface", TEXT), ("text-muted", "surface-2", TEXT),
    ("on-primary", "primary", TEXT), ("on-primary", "primary-hover", TEXT),
    ("on-accent", "accent", TEXT),
    ("link", "bg", TEXT), ("link", "surface", TEXT),
    ("success", "bg", TEXT), ("success", "surface", TEXT),
    ("warning", "bg", TEXT), ("warning", "surface", TEXT),
    ("danger", "bg", TEXT), ("danger", "surface", TEXT),
    ("info", "bg", TEXT), ("info", "surface", TEXT),
    ("primary", "bg", UI), ("focus", "bg", UI), ("focus", "surface", UI),
    ("border-strong", "bg", UI), ("border-strong", "surface", UI),
]


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
