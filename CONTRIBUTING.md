# Contributing to turbotokens

Help make usage counts accurate, reports fast, and the CLI pleasant to use. Small reproducible fixes are welcome.

## Get a working build

```sh
git clone https://github.com/maxmoneycash/turbotokens.git
cd turbotokens/rust
cargo build --release --bin turbotokens --features fetch-litellm-pricing
cargo test --workspace --features fetch-litellm-pricing
cargo fmt --all --check
cargo clippy --workspace --all-targets --features fetch-litellm-pricing -- -D warnings
```

Use Rust with edition 2024 support. The pricing feature downloads a snapshot at build time. For a local file, set `TURBOTOKENS_PRICING_JSON_PATH` to a LiteLLM `model_prices_and_context_window.json` file. The binary is the product; `npm/` only installs and launches release binaries.

## Useful places to start

- **A counting bug:** add the smallest sanitized fixture that reproduces it, then fix the relevant adapter. Include expected input, output, cache, and total counts.
- **An unclear command:** improve its JSON help definition under `rust/crates/turbotokens-cli-parser/src/`, its snapshot, and the matching usage example.
- **A missing log format:** start with the [adapter guide](rust/adapters/README.md) and a real, sanitized sample of that format.
- **A slow report:** record the command, machine, file count, bytes, version, and raw timings. Use the [benchmark harnesses](rust/bench/README.md) to establish a baseline before changing code.
- **A visual problem:** include the terminal width or export dimensions. Generate examples with [isolated synthetic data](demo/README.md), not personal session histories.

## Keep changes reviewable

Explain the concrete problem, the resulting behavior, and how you checked it. Keep unrelated refactoring out of the patch.

Parsing and caching changes must preserve `--json` output byte for byte. Run `rust/bench/warm-bench.sh` on a suitable history and include the result. If fixing an incorrect count, document the affected case and the expected corrected value, with a regression fixture. Never silently change the meaning of a token category.

Hot paths stay small: no async runtime, no clap, and no new dependency without a measured reason. Keep source-specific discovery and parsing in that adapter; shared rendering and aggregation belong in the shared crates.

Benchmarks must compare equivalent work with pinned versions and matching architectures. Report caches, pricing setup, raw samples, and limitations along with the headline number.

## Report a bug

[Open a bug report](https://github.com/maxmoneycash/turbotokens/issues/new?template=bug_report.yml) with the command, binary version, OS, and expected result. `turbotokens doctor` helps identify data discovery problems. Remove credentials, prompts, private paths, and project details from anything you post; a fixture usually needs only the usage fields and IDs required to reproduce the behavior.

## Release work

The [packaging guide](packaging/README.md) covers release binaries, checksums, the Homebrew tap, and the npm wrapper. A README or benchmark claim should describe a version readers can obtain, and link to its measurement evidence.
