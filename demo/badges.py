#!/usr/bin/env python3
"""Generate the README badge SVGs, themed to match the terminal assets."""
import os
import re
from pathlib import Path

OUT_DIR = "assets/badges"
FONT_STACK = "Menlo, 'SF Mono', 'Cascadia Code', Consolas, monospace"
LABEL_BG = "#f6f8fa"
LABEL_FG = "#57606a"
VALUE_FG = "#ffffff"
BORDER = "#d0d7de"
H, FS, PAD = 20, 11, 6
CW = FS * 0.6  # Menlo advance

def badge(name, label, value, accent):
    lw = round(len(label) * CW) + PAD * 2
    vw = round(len(value) * CW) + PAD * 2
    w = lw + vw
    y = 14  # baseline
    svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{H}" viewBox="0 0 {w} {H}" font-family="{FONT_STACK}" font-size="{FS}">
  <clipPath id="r"><rect width="{w}" height="{H}" rx="3"/></clipPath>
  <g clip-path="url(#r)">
    <rect width="{lw}" height="{H}" fill="{LABEL_BG}"/>
    <rect x="{lw}" width="{vw}" height="{H}" fill="{accent}"/>
  </g>
  <rect x="0.5" y="0.5" width="{w - 1}" height="{H - 1}" rx="3" fill="none" stroke="{BORDER}"/>
  <text x="{lw / 2:.1f}" y="{y}" text-anchor="middle" fill="{LABEL_FG}">{label}</text>
  <text x="{lw + vw / 2:.1f}" y="{y}" text-anchor="middle" fill="{VALUE_FG}" font-weight="bold">{value}</text>
</svg>
'''
    path = os.path.join(OUT_DIR, name)
    with open(path, "w") as f:
        f.write(svg)
    print(f"wrote {path} {w}x{H}")

os.makedirs(OUT_DIR, exist_ok=True)
badge("version.svg", "version", 'v' + re.search(r'^version = "([^"]+)"', Path('rust/Cargo.toml').read_text(), re.M).group(1), "#0969da")
badge("agents.svg", "agents", "18", "#1a7f37")
badge("speed.svg", "Claude repeat*", "13 ms", "#0969da")
badge("rust.svg", "built with", "Rust", "#8250df")
