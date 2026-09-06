#!/usr/bin/env python3
"""Render the measured cache benchmark: plot_scaling.py results.json out.png."""
import json
from pathlib import Path
import sys
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
from matplotlib.ticker import MaxNLocator


def main():
    result = json.loads(Path(sys.argv[1]).read_text())
    rows = result['datasets']
    ink, muted, rule, green = '#f0f5ef', '#a2b2a8', '#33403a', '#b9f582'
    plt.rcParams.update({'font.family': 'DejaVu Sans', 'font.size': 12, 'text.color': ink, 'axes.labelcolor': muted, 'xtick.color': muted, 'ytick.color': muted, 'figure.facecolor': '#0c1413', 'axes.facecolor': '#0c1413', 'svg.fonttype': 'none'})
    fig = plt.figure(figsize=(12, 6.4), dpi=160)
    fig.text(.065, .91, 'Reports over growing log histories', fontsize=24, weight='bold')
    fig.text(.065, .854, f"turbotokens {result['version'].split()[-1]}  /  Claude daily report  /  median of {result['runs']} runs", fontsize=12, color=muted)
    ax = fig.add_axes([.08, .245, .51, .50])
    sizes = [row['bytes'] / 1e9 for row in rows]
    for mode, color, label in [('uncached', '#91b9ee', 'Cache disabled'), ('cached', green, 'Cache enabled')]:
        values = [row['median_seconds'][mode] for row in rows]
        ax.plot(sizes, values, color=color, linewidth=2.5, marker='o', markersize=6, label=label)
    ax.set_xlim(0, max(sizes) * 1.06)
    ax.set_ylim(0, max(row['median_seconds']['uncached'] for row in rows) * 1.15)
    ax.yaxis.set_major_locator(MaxNLocator(4, min_n_ticks=4))
    ax.set_xlabel('Log size (GB)', labelpad=12)
    ax.set_ylabel('Report time (seconds)', labelpad=10)
    ax.spines[['top', 'right', 'left']].set_visible(False)
    ax.spines['bottom'].set_color(rule)
    ax.tick_params(length=0, pad=10)
    ax.grid(axis='y', color=rule, linewidth=.7)
    ax.set_axisbelow(True)
    ax.legend(loc='upper left', frameon=False, fontsize=11, handlelength=2)
    # A small table keeps the cached values legible near the zero baseline.
    table = fig.add_axes([.66, .245, .28, .5])
    table.axis('off')
    for x, label in [(0, 'Log size'), (.60, 'Disabled'), (1, 'Enabled')]:
        table.text(x, .96, label, ha='left' if x == 0 else 'right', fontsize=11, color=muted, transform=table.transAxes)
    for index, row in enumerate(rows):
        y = .80 - index * .145
        table.plot([0, 1], [y + .09, y + .09], color=rule, linewidth=.65, transform=table.transAxes)
        table.text(0, y, f"{row['bytes'] / 1e9:.2f} GB", fontsize=12, transform=table.transAxes)
        for x, mode, color in [(.60, 'uncached', ink), (1, 'cached', green)]:
            seconds = row['median_seconds'][mode]
            label = f'{seconds * 1000:.0f} ms' if seconds < 1 else f'{seconds:.2f} s'
            table.text(x, y, label, ha='right', fontsize=12, color=color, weight='bold' if mode == 'cached' else 'normal', transform=table.transAxes)
    fig.text(.065, .10, f"{result['processor']} · synthetic JSONL · warm OS file cache · offline pricing", fontsize=11, color=muted)
    fig.text(.065, .056, 'Every measured report matched byte for byte. Raw samples and reproduction steps: rust/bench/', fontsize=10, color=muted)
    fig.savefig(sys.argv[2], facecolor='#0c1413')
    plt.close(fig)


if __name__ == '__main__':
    main()
