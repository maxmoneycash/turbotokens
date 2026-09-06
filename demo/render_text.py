#!/usr/bin/env python3
"""Render a plain-text terminal capture, with continuous vector box borders.

Usage: render_text.py capture.txt out.svg
"""
from html import escape
from pathlib import Path
import sys
from glass import backdrop

lines = Path(sys.argv[1]).read_text().strip('\n').splitlines()
cell, pitch, padding = 8.4, 21, 28
width = round(max(map(len, lines)) * cell + padding * 2)
height = len(lines) * pitch + padding * 2 + 38
svg = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">', '<title>turbotokens daily report, sample data</title>', backdrop(), f'<text x="{padding}" y="32" font-family="Menlo,monospace" font-size="14" fill="#516681">$ turbotokens daily --last 3</text>']
# Directions from the center of a terminal cell: left, up, right, down.
boxes = {'─': 'lr', '│': 'ud', '┌': 'rd', '╭': 'rd', '┐': 'ld', '╮': 'ld', '└': 'ur', '╰': 'ur', '┘': 'lu', '╯': 'lu', '├': 'urd', '┤': 'lud', '┬': 'lrd', '┴': 'lur', '┼': 'lurd'}
for row, line in enumerate(lines):
    top = padding + 38 + row * pitch
    color = '#0865ce' if 'Total ' in line else '#172b4d'
    run, start = '', 0

    def flush():
        if run.strip():
            svg.append(f'<text x="{padding+start*cell:.1f}" y="{top+15}" font-family="Menlo,monospace" font-size="14" fill="{color}" xml:space="preserve" textLength="{len(run)*cell:.1f}" lengthAdjust="spacingAndGlyphs">{escape(run)}</text>')

    for column, char in enumerate(line):
        if char in boxes:
            flush(); run = ''
            x, y = padding + (column + .5) * cell, top + pitch / 2
            for direction in boxes[char]:
                dx, dy = {'l': (-cell/2, 0), 'r': (cell/2, 0), 'u': (0, -pitch/2), 'd': (0, pitch/2)}[direction]
                svg.append(f'<path d="M{x:.1f},{y:.1f} l{dx:.1f},{dy:.1f}" stroke="#9eb6d0" stroke-width=".8" fill="none"/>')
        else:
            if not run: start = column
            run += char
    flush()
svg.append('</svg>')
Path(sys.argv[2]).write_text('\n'.join(svg)+'\n')
