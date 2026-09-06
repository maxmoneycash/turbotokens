#!/usr/bin/env python3
"""Render the recorded terminal on glass, preserving the recording's timeline.

Requires pyte, Pillow, and rsvg-convert. Samples at 2 fps; no playback speedup.
"""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
cast = Path(sys.argv[1]).resolve()
out = Path(sys.argv[2]).resolve()
events = [json.loads(line) for line in cast.read_text().splitlines()[1:]]
end = max(event[0] for event in events)
frames, durations = [], []
with tempfile.TemporaryDirectory(prefix='tt-glass-live-') as work:
    work = Path(work)
    # Fixed row count avoids resizing as sessions appear during the recording.
    for index in range(int(end * 2) + 1):
        timestamp = min((index + 1) / 2, end)
        svg, png = work / 'frame.svg', work / 'frame.png'
        subprocess.run([sys.executable, str(ROOT / 'demo/render_svg.py'), str(cast), str(svg), '104', '40', '15', 'fixed', str(timestamp)], check=True, stdout=subprocess.DEVNULL)
        subprocess.run(['rsvg-convert', str(svg), '-o', str(png)], check=True)
        with Image.open(png) as source:
            frames.append(source.convert('RGB'))
        durations.append(round((timestamp - index / 2) * 1000))
    # One palette across frames prevents translucent backgrounds from flickering.
    palette = frames[-1].quantize(colors=192)
    frames = [frame.quantize(palette=palette, dither=Image.Dither.NONE) for frame in frames]
    durations[-1] += 2000
    frames[0].save(out, save_all=True, append_images=frames[1:], duration=durations, loop=0, optimize=True, disposal=1)
print(f'wrote {out}: {len(frames)} frames, {sum(durations) / 1000:g}s including a 2s final hold')
