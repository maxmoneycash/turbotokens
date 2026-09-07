#!/usr/bin/env python3
"""README masthead and GitHub social image. No dependencies."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MONO = "ui-monospace, SFMono-Regular, Menlo, Monaco, monospace"
BG, FG, DIM = "#0a0a0a", "#f4f4f4", "#8a8a8a"
NAME = "turbotokens"
LINE = "18 agents. 13 ms cached daily. rust."


def banner(width, height, name_size, line_size, x, name_y, line_y, anchor="start"):
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">
<title>turbotokens: 18 agents. 13 ms cached daily. rust.</title>
<rect width="{width}" height="{height}" fill="{BG}"/>
<g font-family="{MONO}" text-anchor="{anchor}">
<text x="{x}" y="{name_y}" fill="{FG}" font-size="{name_size}" font-weight="600">{NAME}</text>
<text x="{x}" y="{line_y}" fill="{DIM}" font-size="{line_size}">{LINE}</text>
</g>
</svg>
'''


if __name__ == '__main__':
    (ROOT / 'assets/hero.svg').write_text(banner(1280, 200, 40, 18, 56, 88, 128))
    (ROOT / 'assets/hero-mobile.svg').write_text(banner(640, 180, 32, 16, 28, 80, 116))
    (ROOT / 'assets/social-card.svg').write_text(banner(1280, 640, 56, 22, 640, 308, 360, "middle"))
