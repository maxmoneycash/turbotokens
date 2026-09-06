# Report benchmarks

The README uses two cross-tool comparisons and a separate cache experiment. Measurements use released binaries, deterministic synthetic Claude-format logs, and process wall time. Scripts fail on command errors or mismatched token counts.

## Recorded comparisons

- [Daily reports: raw samples](results/comparison-daily.json), turbotokens 1.1.0 and ccusage 20.0.20.
- [Monthly reports: raw samples](results/comparison.json), turbotokens 1.1.0, ccusage 20.0.20, and tokscale 4.15.1.
- Apple M1 Max, macOS 26.6, September 6, 2026. All three binaries are **native arm64**, confirmed with `file`; hashes are in the results. The timing driver is Python under x64/Rosetta, but it launches the ARM binaries directly.
- Five measured runs per tool and size. A warm-up runs each tool first; timed runs rotate tool order. Charts and tables show medians. Raw samples retain variability, including the slower 1.81 GB ccusage daily result.
- Application caches and OS file reads are warmed before measurement. Actual OS cache residency and background work are not controlled. No daemon, npm launcher, package download, or initial pricing fetch is inside the timer.
- turbotokens and ccusage use `--offline`. Tokscale has no offline report flag in this release; its pricing cache is populated during warm-up. Its configuration and message cache use an isolated `TOKSCALE_CONFIG_DIR`.
- Each subprocess must return the expected **input, output, cache creation, and cache read totals**, not only their sum. Cross-tool cost and byte parity are not asserted: pricing sources and output schemas differ.
- Six files per dataset, across three projects and two sessions. There is roughly 2 KB of synthetic message content per event. Recorded usage is cache-read-heavy. The generator uses seed 42.

| Log size | Recorded token counters | Daily: turbotokens | Daily: ccusage |
| --- | ---: | ---: | ---: |
| 72 MB | ~1B | 10.9 ms | 78.1 ms |
| 362 MB | ~5B | 11.8 ms | 277.9 ms |
| 725 MB | ~10B | 11.6 ms | 585.4 ms |
| 1.81 GB | ~25B | 16.6 ms | 4,487.9 ms |
| 3.63 GB | ~50B | 12.9 ms | 3,649.2 ms |

At the largest size, the daily elapsed-time ratio is 3.649209125 / 0.0128925 = **283.05**. This is a result for a warmed Claude daily workload, not a general speed ratio across agents or commands.

| Log size | Monthly: turbotokens | Monthly: ccusage | Monthly: tokscale |
| --- | ---: | ---: | ---: |
| 72 MB | 37.6 ms | 70.8 ms | 107.3 ms |
| 362 MB | 146.6 ms | 273.0 ms | 339.8 ms |
| 725 MB | 262.9 ms | 539.0 ms | 751.3 ms |
| 1.81 GB | 621.8 ms | 1,230.2 ms | 1,858.8 ms |
| 3.63 GB | 1,544.7 ms | 4,827.0 ms | 3,990.8 ms |

**Limits:** this tests Claude logs, not all supported agents. Six large files do not represent a history with thousands of short sessions. File count, cache state, transcript contents, memory pressure, and hardware matter. Recorded token counters are numbers in the logs; the tools are not tokenizing 50 billion tokens of text. These results do not measure interactive UI rendering, app installation, subscription APIs, or live-event latency.

## A history with more files

[comparison-many-files.json](results/comparison-many-files.json) checks the daily commands on the same 72 MB / ~1B-token-counter dataset redistributed across **1,000 files**. The recorded medians are **12.66 ms** for turbotokens and **71.94 ms** for ccusage, with the same four-category count checks and five rotating runs. This adds a different file shape; the records remain synthetic.

```sh
python3 rust/bench/shard_data.py /tmp/tt-cache-scaling/tok-1B /tmp/tt-many-files/tok-1B 1000
```

Then use `compare.py --report daily --sizes 1 --data /tmp/tt-many-files` with the same binary paths and a separate `--output`. The destination of `shard_data.py` must be new.

## Reproduce the comparisons

Use Python 3 and native binaries for the same architecture. Pin versions before measuring. Do not compare an ARM turbotokens build with a competitor running under Rosetta.

For the recorded macOS ARM run, turbotokens came from the v1.1.0 release asset. The competitors came from npm's `@ccusage/ccusage-darwin-arm64@20.0.20` and `@tokscale/cli-darwin-arm64@4.15.1` packages. These contain native binaries, even though the top-level npm entrypoints are JavaScript launchers. Download with `npm pack`, extract with `tar`, and make the binaries executable if necessary. Other platforms have different package names.

Generate the five datasets (about 6.6 GB total, plus application caches):

```sh
for size in 1 5 10 25 50; do
  python3 rust/bench/gen_scaling_data.py "/tmp/tt-cache-scaling/tok-${size}B" "$size"
done
```

Run the matching monthly commands:

```sh
python3 rust/bench/compare.py \
  --turbotokens /path/to/native/turbotokens \
  --ccusage /path/to/native/ccusage \
  --tokscale /path/to/native/tokscale \
  --data /tmp/tt-cache-scaling \
  --output /tmp/comparison.json
```

Repeat with `--report daily --output /tmp/comparison-daily.json` for the two-tool daily comparison. Use `--sizes 1` for a small trial. Run measurements after builds and other CPU-intensive work finish.

The exact report commands are:

```sh
turbotokens claude monthly --offline --json
ccusage claude monthly --offline --json
tokscale monthly --client claude --home /path/to/isolated/home --json --no-spinner
```

`CLAUDE_CONFIG_DIR` points the first two tools at the generated dataset. The isolated tokscale home contains a `.claude` symlink to that same dataset. For daily reports, replace `monthly` with `daily` for turbotokens and ccusage. The script records commands, versions, architecture, hashes, categories, and every timing sample.

Render the images separately; matplotlib is a documentation dependency, not a CLI dependency:

```sh
python3 rust/bench/plot_comparison.py /tmp/comparison-daily.json assets/comparison-daily.png
python3 rust/bench/plot_comparison.py /tmp/comparison.json assets/comparison-monthly.png
```

## Cache enabled versus disabled

[cache-scaling.json](results/cache-scaling.json) records a separate run with the released turbotokens 1.1.0 ARM binary on the same M1 Max.

Five alternating cache-disabled/cache-enabled runs per size; both modes use warmed OS reads and offline pricing. Cache-disabled runs set `TURBOTOKENS_CACHE=off`. Every output must match **byte for byte** and equal the generated token total.

| Log size | Parse cache disabled | Parse cache enabled |
| --- | ---: | ---: |
| 72 MB | 29 ms | 9 ms |
| 362 MB | 108 ms | 9 ms |
| 725 MB | 216 ms | 9 ms |
| 1.81 GB | 510 ms | 10 ms |
| 3.63 GB | 1.10 s | 9 ms |

```sh
python3 rust/bench/cache-scaling.py \
  --data /tmp/tt-cache-scaling \
  --binary /path/to/native/turbotokens \
  --output /tmp/cache-scaling.json
python3 rust/bench/plot_scaling.py /tmp/cache-scaling.json assets/scaling-chart.png
```

The 9 ms cache experiment and 13 ms comparative result are different runs. Neither is a hard latency guarantee. Disabling the parse cache does not flush the operating system's file cache.

## Comparison scope

Feature descriptions were checked against installed command help and these pinned upstream READMEs:

- [ccusage at 5c0de7c](https://github.com/ccusage/ccusage/blob/5c0de7cc6e05d0078b8140bbb593a9dc2b3d49a5/README.md): 18 sources, reporting commands, Claude billing-block monitoring, and status-line integration.
- [tokscale at 4664d38](https://github.com/junhoyeo/tokscale/blob/4664d3866b7000a4cc12682947dae0430be513d2/README.md): interactive views, contribution graphs, sharing, and provider integrations. Installed 4.15.1 help exposes more than 50 client filters.

Tokscale provides daily views inside its TUI and graph export, but no matching standalone `daily --json` command. We compare all three tools using their monthly JSON report instead of treating different commands as equivalent.

The feature table describes documented strengths; it is not an exhaustive absence-of-feature audit. All three versions tested use native Rust. A comparison against an old JavaScript ccusage release would tell a different story and is not the basis for these claims.

## Other checks

`warm-bench.sh` checks cached/uncached parity on an existing Claude history:

```sh
CLAUDE_CONFIG_DIR=/path/to/claude rust/bench/warm-bench.sh 10
```

The older `scaling-bench.sh` and `latency-probe.sh` are development probes. Their output is not the source of the README comparison or a live-latency claim.
