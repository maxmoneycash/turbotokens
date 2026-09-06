"""Shared vector material used by native exports and README renderers."""
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
MATERIAL = ROOT / 'rust/crates/turbotokens/src/commands/glass.svg'
INK, MUTED, BLUE, RULE = '#172b4d', '#516681', '#0865ce', '#c4d4e7'


def backdrop():
    return MATERIAL.read_text()


def panel(x, y, width, height, radius=22):
    return f'<rect x="{x}" y="{y}" width="{width}" height="{height}" rx="{radius}" fill="#fff" fill-opacity=".42" stroke="#fff" stroke-opacity=".9"/>'


def figure_material(fig):
    """Render the same SVG behind a matplotlib figure; keep plot data untouched."""
    import matplotlib.image as mpimg
    width, height = (round(v * 180) for v in fig.get_size_inches())
    with tempfile.TemporaryDirectory(prefix='tt-glass-') as work:
        svg, png = Path(work) / 'glass.svg', Path(work) / 'glass.png'
        svg.write_text(f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}">{backdrop()}</svg>')
        subprocess.run(['rsvg-convert', str(svg), '-o', str(png)], check=True)
        background = fig.add_axes([0, 0, 1, 1], zorder=-1)
        background.imshow(mpimg.imread(png), aspect='auto')
        background.axis('off')
    fig.patch.set_alpha(0)
