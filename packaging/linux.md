# Linux release validation

The release workflow builds native x64 and arm64 binaries with musl, then rejects
any executable that requires a dynamic loader or shared library. The archive names
remain `turbotokens-linux-x64.tar.gz` and `turbotokens-linux-arm64.tar.gz`, so the
npm package, Homebrew formula, and shell installer can use the same asset lookup.

The v1.1.2 Linux archives require glibc 2.39. Changing the workflow does not change
those published archives. A new release must pass the checks below before its
checksums are added to the install packages.

The [2026-09-08 validation run](https://github.com/maxmoneycash/turbotokens/actions/runs/34217300332)
passed static-linkage and container-report checks for both Linux architectures
at commit `8b137a80`, covering all six architecture/distribution combinations.

`packaging/smoke-linux.py` runs each new binary in Ubuntu 22.04, Debian 12, and
Alpine 3.22 containers. It verifies the exact version, reads a synthetic Claude
log with a duplicate event, checks all four token categories, and compares the
complete JSON report across containers. Reports use embedded pricing with
`--offline`; the containers have no network access and run as an unprivileged
user. Their log fixture is mounted read-only.

Pull requests touching Rust or the release checks run the build matrix. A manual
run of the `release` workflow also builds and retains all six archives without
publishing a release. On a version-tag push, publication waits for every build
and validation step to pass. The tag must match the version in `rust/Cargo.toml`.

## Reproduce on Linux

Use an x64 or arm64 Ubuntu machine with Rust and Docker installed. The build
uses the machine's native architecture:

```bash
sudo apt-get update
sudo apt-get install --yes --no-install-recommends musl-tools binutils
target="$(uname -m)-unknown-linux-musl"
rustup target add "$target"
export "CC_${target//-/_}=musl-gcc"
export "AR_${target//-/_}=ar"
upper_target="${target^^}"
export "CARGO_TARGET_${upper_target//-/_}_LINKER=musl-gcc"
export "CARGO_TARGET_${upper_target//-/_}_RUSTFLAGS=-C target-feature=+crt-static"
cargo build --manifest-path rust/Cargo.toml --locked --release --bin turbotokens \
  --features fetch-litellm-pricing --target "$target"
package_id="$(cargo pkgid --manifest-path rust/Cargo.toml --package turbotokens)"
version="${package_id##*#}"
version="${version##*@}"
python3 packaging/smoke-linux.py "rust/target/$target/release/turbotokens" "$version"
```

These tests cover libc compatibility and report behavior on the runner's Linux
kernel. They do not establish compatibility with every kernel or CPU. The npm
wrapper also needs a Node version that runs on the host distribution.

Rust documents [static C runtime selection](https://doc.rust-lang.org/reference/linkage.html#static-and-dynamic-c-runtimes).
Native C dependencies use the compiler and archiver selected through
[cc's target-specific environment variables](https://docs.rs/cc/latest/cc/#external-configuration-via-environment-variables).
