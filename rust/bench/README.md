# Report benchmarks

The README chart measures turbotokens' Claude daily report with the application cache enabled and disabled. It does not compare other tools.

## Recorded run

See [cache-scaling.json](results/cache-scaling.json) for every timing sample, the binary hash, hardware, version, and measurement timestamp.

- Released macOS arm64 binary, turbotokens 1.1.0, on an Apple M1 Max.
- Five deterministic datasets: 72 MB, 362 MB, 725 MB, 1.81 GB, and 3.63 GB (decimal units).
- Each dataset has six JSONL files with roughly 2 KB of message content per event. Their usage counters total approximately 1B, 5B, 10B, 25B, and 50B tokens.
- Five timed runs per mode, alternating between cache disabled and enabled. The chart shows the median wall time for each subprocess, including JSON output.
- Both modes use a warm operating-system file cache. Pricing is offline. The application cache is populated before measurement; uncached runs use `TURBOTOKENS_CACHE=off`.
- Every result must equal the generated token total. Every cached and uncached JSON output must match byte for byte, or the benchmark fails.

| Log size | Cache disabled | Cache enabled |
| --- | ---: | ---: |
| 72 MB | 29 ms | 9 ms |
| 362 MB | 108 ms | 9 ms |
| 725 MB | 216 ms | 9 ms |
| 1.81 GB | 510 ms | 10 ms |
| 3.63 GB | 1.10 s | 9 ms |

The token counters describe reported usage. turbotokens reads and aggregates those numbers; it does not tokenize billions of tokens of text. File count, transcript size, hardware, and background work affect performance. Synthetic data across six files does not represent every real agent history.

## Reproduce

Run from the repository root with Python 3 and a release binary. The full dataset needs about 6.6 GB of disk space, plus the cache.

```sh
python3 rust/bench/cache-scaling.py \
  --data /tmp/tt-cache-scaling \
  --binary rust/target/release/turbotokens \
  --output /tmp/cache-scaling.json

# Requires matplotlib; rendering is separate from measurement.
python3 rust/bench/plot_scaling.py /tmp/cache-scaling.json assets/scaling-chart.png
```

Use `--sizes 1` for a smaller run. Regeneration uses a fixed random seed. Run measurements after builds and other CPU-intensive work finish.

## Other checks

`warm-bench.sh` compares cached and uncached output on an existing Claude history:

```sh
CLAUDE_CONFIG_DIR=/path/to/claude rust/bench/warm-bench.sh 10
```

`latency-probe.sh` measures live-mode latency with synthetic events. `scaling-bench.sh` is an optional comparison harness for turbotokens and a pinned ccusage command. It checks token-total parity and fails on command errors. Its output is not used in the README chart.
