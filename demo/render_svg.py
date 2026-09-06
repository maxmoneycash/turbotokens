#!/usr/bin/env python3
"""Render the final frame of an asciicast v2 file to a crisp SVG.

Box-drawing characters are emitted as vector lines/arcs so borders connect
perfectly at any scale; text runs are tspans in a monospace font stack.
Usage: render_svg.py in.cast out.svg [cols rows font_size trim]
"""
import json, sys
import pyte
from glass import backdrop

CAST, OUT = sys.argv[1], sys.argv[2]
COLS = int(sys.argv[3]) if len(sys.argv) > 3 else 100
ROWS = int(sys.argv[4]) if len(sys.argv) > 4 else 32
FONT = int(sys.argv[5]) if len(sys.argv) > 5 else 15
TRIM = len(sys.argv) > 6 and sys.argv[6] == "trim"

BG = "#edf3fa"
FG = "#172b4d"
BASE16 = {
    "black": "#172b4d", "red": "#b83250", "green": "#087c51",
    "yellow": "#99611b", "brown": "#99611b", "blue": "#0865ce", "magenta": "#7254bb",
    "cyan": "#177897", "white": "#334d6d",
    "brightblack": "#516681", "brightred": "#b83250",
    "brightgreen": "#087c51", "brightyellow": "#99611b",
    "brightblue": "#0865ce", "brightmagenta": "#7254bb",
    "brightcyan": "#177897", "brightwhite": "#172b4d",
}
FONT_STACK = "Menlo, 'SF Mono', 'Cascadia Code', Consolas, monospace"

def xterm256(n):
    if n < 16:
        return list(BASE16.values())[n]
    if n < 232:
        n -= 16
        r, g, b = n // 36, (n % 36) // 6, n % 6
        conv = lambda v: 55 + v * 40 if v else 0
        return "#%02x%02x%02x" % (conv(r), conv(g), conv(b))
    v = 8 + (n - 232) * 10
    return "#%02x%02x%02x" % (v, v, v)

def resolve(c, default):
    if c in ("default", None):
        return default
    if c in BASE16:
        return BASE16[c]
    if isinstance(c, str) and c.isdigit():
        return xterm256(int(c))
    if isinstance(c, str) and len(c) == 6:
        try:
            int(c, 16)
            channels = [int(c[i:i+2], 16) for i in (0, 2, 4)]
            luminance = sum(v * w for v, w in zip(channels, (.2126, .7152, .0722)))
            if luminance > 145:
                channels = [round(v * 120 / luminance) for v in channels]
            return "#%02x%02x%02x" % tuple(channels)
        except ValueError:
            pass
    return default

# --- compose final frame ---
screen = pyte.Screen(COLS, ROWS)
stream = pyte.Stream(screen)
with open(CAST) as f:
    json.loads(f.readline())  # header
    for line in f:
        try:
            ev = json.loads(line)
        except ValueError:
            continue
        if len(sys.argv) > 7 and ev[0] > float(sys.argv[7]):
            break
        if len(ev) >= 3 and ev[1] == "o":
            stream.feed(ev[2])

disp = screen.display
nrows = 33 if len(sys.argv) > 6 and sys.argv[6] == "fixed" else ROWS
if TRIM:
    nrows = max((i for i, row in enumerate(disp) if row.strip()), default=0) + 1

# geometry: Menlo advance is 0.6em; rows get 1.33em pitch
CW = FONT * 0.6
LH = round(FONT * 1.34)
PADX, PADY = 28, 62
W = round(COLS * CW) + PADX * 2
H = nrows * LH + PADY + 28
LINE_W = max(1.2, FONT * 0.09)

def esc(s):
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace('"', "&quot;")

parts = []
parts.append(f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" '
             f'viewBox="0 0 {W} {H}" font-family="{FONT_STACK}" font-size="{FONT}">')
parts.append('<title>turbotokens live dashboard, synthetic usage</title>')
parts.append(backdrop())
parts.append('<text x="28" y="34" fill="#516681" font-size="14">$ turbotokens live --offline</text>')

ARCS = {  # quarter ellipse inscribed in the cell: (start_angle, end_angle, sweep)
    "╭": (0, 90), "╮": (90, 180), "╯": (180, 270), "╰": (270, 360),
}
LINES = {
    **{c: (1, 0, 1, 0) for c in "─━┄┅┈┉╌╍"},          # L, R
    **{c: (0, 1, 0, 1) for c in "│┃┆┇┊┋╎╏"},          # T, B
    **{c: (0, 0, 1, 1) for c in "┌┍┎┏"},
    **{c: (1, 0, 0, 1) for c in "┐┑┒┓"},
    **{c: (0, 1, 1, 0) for c in "└┕┖┗"},
    **{c: (1, 1, 0, 0) for c in "┘┙┚┛"},
    **{c: (0, 0, 1, 1) for c in "├┝┞┟┠┡┢┣"},           # T,B,R -> fix below
    **{c: (1, 0, 1, 1) for c in "┤┥┦┧┨┩┪┫"},
    **{c: (1, 1, 0, 1) for c in "┬┭┮┯┰┱┲┳"},
    **{c: (1, 1, 1, 0) for c in "┴┵┶┷┸┹┺┻"},
    **{c: (1, 1, 1, 1) for c in "┼╀╁╂╃╄╅╆╇╈╉╊╋"},
}
# correct the tees: tuple is (L, T, R, B)
LINES.update({c: (0, 1, 1, 1) for c in "├┝┞┟┠┡┢┣"})
LINES.update({c: (1, 1, 0, 1) for c in "┤┥┦┧┨┩┪┫"})
LINES.update({c: (1, 0, 1, 1) for c in "┬┭┮┯┰┱┲┳"})
LINES.update({c: (1, 1, 1, 0) for c in "┴┵┶┷┸┹┺┻"})

def box_svg(ch, px, py, color):
    cx, cy = px + CW / 2, py + LH / 2
    out = []
    if ch in ARCS:
        st, en = ARCS[ch]
        import math
        rx, ry = CW / 2, LH / 2
        x0 = cx + rx * math.cos(math.radians(st)); y0 = cy + ry * math.sin(math.radians(st))
        x1 = cx + rx * math.cos(math.radians(en)); y1 = cy + ry * math.sin(math.radians(en))
        out.append(f'<path d="M{x0:.1f},{y0:.1f} A{rx:.1f},{ry:.1f} 0 0 1 {x1:.1f},{y1:.1f}" '
                   f'stroke="{color}" stroke-width="{LINE_W}" fill="none"/>')
        return out
    l, t, r, b = LINES[ch]
    hw = LINE_W / 2
    if l: out.append(f'<rect x="{px:.1f}" y="{cy-hw:.1f}" width="{CW/2+hw:.1f}" height="{LINE_W}" fill="{color}"/>')
    if r: out.append(f'<rect x="{cx-hw:.1f}" y="{cy-hw:.1f}" width="{CW/2+hw:.1f}" height="{LINE_W}" fill="{color}"/>')
    if t: out.append(f'<rect x="{cx-hw:.1f}" y="{py:.1f}" width="{LINE_W}" height="{LH/2+hw:.1f}" fill="{color}"/>')
    if b: out.append(f'<rect x="{cx-hw:.1f}" y="{cy-hw:.1f}" width="{LINE_W}" height="{LH/2+hw:.1f}" fill="{color}"/>')
    return out

for y in range(nrows):
    words = []    # (start_col, text, fill, bold)
    cur = ["", -1, None, False]  # text, start_col, fill, bold
    def flush():
        if cur[0]:
            words.append((cur[1], cur[0], cur[2], cur[3]))
            cur[0] = ""
    for x in range(COLS):
        ch = screen.buffer[y][x]
        px, py = PADX + x * CW, PADY + y * LH
        fg = resolve(ch.fg, FG)
        bg = resolve(ch.bg, BG)
        if ch.reverse:
            fg, bg = bg, fg
        if bg != BG:
            parts.append(f'<rect x="{px:.1f}" y="{py:.1f}" width="{CW:.1f}" height="{LH}" fill="{bg}"/>')
        c = ch.data
        if c in LINES or c in ARCS:
            flush()
            parts.extend(box_svg(c, px, py, fg))
        elif c == "█":
            flush()
            parts.append(f'<rect x="{px:.1f}" y="{py:.1f}" width="{CW:.1f}" height="{LH}" fill="{fg}"/>')
        elif c == " ":
            flush()
        else:
            bold = bool(ch.bold)
            if cur[0] and fg == cur[2] and bold == cur[3]:
                cur[0] += c
            else:
                flush()
                cur[0], cur[1], cur[2], cur[3] = c, x, fg, bold
    flush()
    if words:
        spans = ""
        for col, text, fill, bold in words:
            weight = ' font-weight="bold"' if bold else ""
            tl = f' textLength="{len(text) * CW:.1f}" lengthAdjust="spacingAndGlyphs"'
            spans += f'<tspan x="{PADX + col * CW:.1f}" fill="{fill}"{weight}{tl}>{esc(text)}</tspan>'
        baseline = PADY + y * LH + LH * 0.76
        parts.append(f'<text y="{baseline:.1f}" xml:space="preserve">{spans}</text>')

parts.append("</svg>")
with open(OUT, "w") as f:
    f.write("\n".join(parts))
print(f"wrote {OUT} {W}x{H}")
