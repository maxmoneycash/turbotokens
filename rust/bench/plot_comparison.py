#!/usr/bin/env python3
"""Render the checked-in comparison results. Requires matplotlib."""
import json
from pathlib import Path
import sys

import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
from matplotlib.ticker import FuncFormatter, FixedLocator

ROOT = Path(__file__).resolve().parents[2]
BG, FG, MUTED, GRID = '#0c1413', '#f0f5ef', '#a2b2a8', '#33403a'
COLORS = {'turbotokens': '#b9f582', 'ccusage': '#91b9ee', 'tokscale': '#e9b877'}


def seconds(value):
    return '%g ms' % (value * 1000) if value < 1 else '%g s' % value


def render(source, output):
    data = json.loads(Path(source).read_text())
    daily = data.get('report') == 'daily'
    names = list(data['tools'])
    rows = data['datasets']
    if not daily:
        render_monthly(data, output)
        return
    xs = [row['bytes'] / 1e9 for row in rows]
    plt.rcParams.update({'font.family': 'DejaVu Sans', 'font.size': 14,
                         'text.color': FG, 'axes.labelcolor': MUTED,
                         'xtick.color': MUTED, 'ytick.color': MUTED})
    fig, ax = plt.subplots(figsize=(12, 6.6), facecolor=BG)
    ax.set_facecolor(BG)
    fig.subplots_adjust(left=.105, right=.80, top=.71, bottom=.21)
    fig.text(.055, .925, 'REPEATED REPORTS / NATIVE ARM64', color=COLORS['turbotokens'], size=12, weight='bold')
    fig.text(.055, .845, 'Daily reports, without the wait.' if daily else 'Monthly reports. Three native CLIs.', size=26, weight='bold')
    fig.text(.055, .787, 'Same Claude logs. Warm caches. Median of five runs. Lower is better.', size=13, color=MUTED)
    for name in names:
        values = [row['median_seconds'][name] for row in rows]
        ax.plot(xs, values, color=COLORS[name], linewidth=3, marker='o', markersize=7)
        # Direct labels avoid a disconnected legend; price/count parity lives in the report.
        label_y = values[-1]
        ax.annotate(name + '\n' + seconds(round(label_y, 3)),
                    xy=(xs[-1], label_y), xytext=(12, 0), textcoords='offset points',
                    va='center', color=COLORS[name], size=13, weight='bold')
    ax.set_yscale('log')
    ax.set_ylim(.005, max(row['median_seconds'][name] for row in rows for name in names) * 2)
    ax.set_xlim(0, xs[-1] * 1.03)
    ax.yaxis.set_major_locator(FixedLocator([.01, .1, 1, 10]))
    ax.yaxis.set_major_formatter(FuncFormatter(lambda value, _: seconds(value)))
    ax.yaxis.set_minor_locator(FixedLocator([]))
    ax.set_xticks([xs[0], xs[2], xs[3], xs[4]])
    ax.set_xticklabels(['72 MB', '725 MB', '1.81 GB', '3.63 GB'])
    ax.set_xlabel('Transcript size on disk (decimal bytes)', labelpad=13, size=12)
    ax.grid(axis='y', color=GRID, linewidth=.7)
    ax.tick_params(length=0, pad=10)
    for spine in ax.spines.values():
        spine.set_visible(False)
    versions = '  /  '.join(data['tools'][name]['version'] for name in names)
    fig.text(.055, .095, versions, color=MUTED, size=10)
    fig.text(.055, .051, 'M1 Max · 6 synthetic files per size · logarithmic time axis · all four token categories verified', color=MUTED, size=10)
    fig.savefig(output, dpi=180, facecolor=BG)
    plt.close(fig)


def render_monthly(data, output):
    row = data['datasets'][-1]
    names = sorted(data['tools'], key=lambda name: row['median_seconds'][name])
    values = [row['median_seconds'][name] for name in names]
    fig, ax = plt.subplots(figsize=(12, 5.8), facecolor=BG)
    ax.set_facecolor(BG)
    fig.subplots_adjust(left=.19, right=.91, top=.68, bottom=.22)
    fig.text(.055, .91, 'MONTHLY REPORTS / NATIVE ARM64', color=COLORS['turbotokens'], size=12, weight='bold')
    fig.text(.055, .82, '3.63 GB of logs. Three native CLIs.', color=FG, size=26, weight='bold')
    fig.text(.055, .755, 'Same Claude logs. Warm caches. Median of five runs. Lower is better.', color=MUTED, size=13)
    ax.barh(range(3), values, height=.48, color=[COLORS[name] for name in names])
    ax.set_yticks(range(3))
    ax.set_yticklabels(names, color=FG, size=17)
    ax.invert_yaxis()
    ax.set_xlim(0, max(values) * 1.2)
    ax.set_xticks([0, 1, 2, 3, 4, 5])
    ax.set_xticklabels(['0', '1 s', '2 s', '3 s', '4 s', '5 s'], color=MUTED, size=12)
    ax.tick_params(length=0, pad=12)
    ax.set_axisbelow(True)
    ax.grid(axis='x', color=GRID, linewidth=.7)
    for index, (name, value) in enumerate(zip(names, values)):
        ax.text(value + .09, index, '%.2f s' % value, va='center', color=COLORS[name], size=18, weight='bold')
    for spine in ax.spines.values():
        spine.set_visible(False)
    fig.text(.055, .105, 'turbotokens 1.1.0 / ccusage 20.0.20 / tokscale 4.15.1', color=MUTED, size=11)
    fig.text(.055, .052, 'M1 Max · 6 synthetic files · JSON output included · all four token categories verified', color=MUTED, size=11)
    fig.savefig(output, dpi=180, facecolor=BG)
    plt.close(fig)


if __name__ == '__main__':
    render(sys.argv[1], sys.argv[2])
