# README visuals

All current graphics use a shared Liquid Glass inspired SVG material: translucent layers over a soft blue background, reflective edges, and dark foreground text. The native exports embed the same self-contained material from `rust/crates/turbotokens/src/commands/glass.svg`; no fonts, images, or network resources are required at runtime.

The product screenshots and SVG exports use generated Claude Code and Codex logs. They contain no personal usage history. The chart numbers come from the separately recorded [benchmarks](../rust/bench/README.md).

## Capture the product

Build the current binary, then run:

```sh
python3 demo/showcase.py --binary rust/target/release/turbotokens
```

This creates a temporary dataset using seed 42, runs the actual daily, heatmap, and wrapped commands, and records 12 seconds of `live --offline` in a real PTY while synthetic events are appended. The child has an isolated environment. The live recording preserves elapsed time; its data feed is synthetic.

To capture the machine-readable token feed instead of the dashboard:

```sh
rust/target/release/turbotokens stream --offline --interval 100
```

Each stdout line is one JSON usage event. `live --json` is the same stream. Stop with Ctrl-C, or pipe through `head` for a few events.

`fixtures/capture.json` records the version, date, and commands. Text, JSON, SVG, and asciicast output are saved in `fixtures/`. The SVG exports in `assets/` are direct CLI output, with no palette swap or edited statistics. The yearly card may show fewer project details for adapters that do not expose them.

## Render terminal captures

The text renderer draws box borders as continuous vectors. The asciicast renderer interprets the terminal with pyte. Use Python with pyte installed for the latter.

```sh
python3 demo/render_text.py demo/fixtures/daily.txt assets/daily-report.svg
rsvg-convert -z 2 assets/daily-report.svg -o assets/daily-report.png

python3 demo/render_svg.py demo/fixtures/live.cast assets/live-dashboard.svg 104 40 15 trim
rsvg-convert -z 2 assets/live-dashboard.svg -o assets/live-dashboard.png

python3 demo/render_gif.py demo/fixtures/live.cast assets/live-demo.gif
```

The animation renderer needs pyte, Pillow, and `rsvg-convert`. It samples the original recording at two frames per second, preserves elapsed time, and adds a two-second final hold. A shared GIF palette keeps the glass background steady.

These are optional asset-generation tools. They are not runtime dependencies of turbotokens.

The benchmark plotters also use this material through `demo/glass.py`. They require matplotlib and `rsvg-convert`; the checked-in measurements remain the source of every plotted value.

## Brand and social image

```sh
python3 demo/brand.py
rsvg-convert assets/social-card.svg -o assets/social-card.png
```

The masthead uses the measured repeat-report result from v1.1.0, with its workload qualification. Update the claim and linked benchmark together if recording a new result. The social image is suitable for the repository's social preview setting.

## Older tooling

`seed.sh`, `feed.sh`, `cast_to_ansi.py`, `render_cast.py`, `light_svg.py`, `speed_chart.py`, and `social_card.py` remain available for older recordings. The current README uses the workflow above. In particular, do not palette-swap native SVGs for new product previews: render what the released CLI produces.
