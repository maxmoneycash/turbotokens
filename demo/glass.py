"""Shared page fill used by native exports and README renderers."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MATERIAL = ROOT / 'rust/crates/turbotokens/src/commands/glass.svg'
INK, MUTED, BLUE, RULE = '#111111', '#666666', '#111111', '#e8e8e8'


def backdrop():
    return MATERIAL.read_text()


def panel(x, y, width, height, radius=0):
    return ''


def figure_material(fig):
    fig.patch.set_facecolor('#ffffff')
