#!/usr/bin/env python3
"""README masthead and GitHub social image. No dependencies."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MONO = "ui-monospace, SFMono-Regular, Menlo, Monaco, monospace"
BG, FG, DIM = "#0a0a0a", "#f4f4f4", "#a3a3a3"
NAME = "turbotokens"
LINE = "Know what your agents spend."


def banner(width, height, name_size, line_size, x, name_y, line_y, anchor="start"):
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">
<title>turbotokens: {LINE}</title>
<rect width="{width}" height="{height}" fill="{BG}"/>
<g font-family="{MONO}" text-anchor="{anchor}">
<text x="{x}" y="{name_y}" fill="{FG}" font-size="{name_size}" font-weight="600">{NAME}</text>
<text x="{x}" y="{line_y}" fill="{DIM}" font-size="{line_size}">{LINE}</text>
</g>
</svg>
'''


if __name__ == '__main__':
    (ROOT / 'assets/hero.svg').write_text(banner(1280, 260, 88, 30, 56, 130, 190))
    (ROOT / 'assets/hero-mobile.svg').write_text(banner(640, 240, 70, 28, 32, 116, 172))
    (ROOT / 'assets/social-card.svg').write_text(banner(1280, 640, 100, 36, 640, 306, 388, "middle"))
