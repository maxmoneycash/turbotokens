# Release packaging

The Rust version in `rust/Cargo.toml` is the binary's source of truth. The npm wrapper and Homebrew formula must match the published release they download.

## Before publishing

1. Run the workspace tests, Clippy, and formatting checks under `rust/`.
2. Push the version tag to build all six release assets in `.github/workflows/release.yml`.
3. Download the release assets and verify their SHA-256 digests against GitHub's release metadata.
4. Update all four checksums in `packaging/turbotokens.rb` and all six in `npm/checksums.json`. Set the formula and npm package versions to the release version.
5. Run the packaging tests from the repository root:

```sh
python3 -m unittest discover -s packaging/tests -v
node packaging/smoke-npm.cjs
```

The npm smoke test packs the package, installs it in a temporary directory, exercises the executable shim, and checks missing-binary and checksum-failure handling. It downloads the published GitHub release binary, so help assertions must match that release, not unreleased commands on `main`. It works on macOS, Linux, and Windows. The shell tests use a local archive and require a Unix shell.

## Homebrew

The public formula lives in [maxmoneycash/homebrew-tap](https://github.com/maxmoneycash/homebrew-tap) at `Formula/turbotokens.rb`. Copy the verified repo formula there, test it, commit, and push.

```sh
brew install maxmoneycash/tap/turbotokens
brew test maxmoneycash/tap/turbotokens
```

The formula includes macOS and Linux builds for arm64 and x64. Existing users can run `brew update && brew upgrade turbotokens`.

## npm

The package is a downloader and a small Node launcher. Native binaries go in `vendor/` during installation and are excluded from the published tarball. Commit the release's checksums, README, and MIT license before packing.

```sh
cd npm
npm whoami
npm pack --dry-run
npm publish --access public
```

After publishing, verify `npm view turbotokens version` and run `npx --yes turbotokens@<version> --version` from outside the repository.

## Direct downloads

GitHub Releases contains `.tar.gz` archives for macOS/Linux and `.zip` archives for Windows, for both x64 and arm64. Each includes the binary, README, and license. Windows users can extract the archive and place `turbotokens.exe` on their PATH.

The archive's short README comes from `RELEASE_README.md`. Packaging replaces
`@REVISION@` with the built commit so its guide links match the binary. The
repository README's images and relative links are intended for GitHub.

The shell installer supports `TURBOTOKENS_VERSION` (for example `v1.1.2`) and `TURBOTOKENS_INSTALL_DIR`. Test it in a temporary directory before release:

```sh
TURBOTOKENS_VERSION=v1.1.2 TURBOTOKENS_INSTALL_DIR=/tmp/turbotokens-install sh install.sh
/tmp/turbotokens-install/turbotokens --version
```

## Longer native checks

The release workflow tests synthetic reports and live log changes on all six
native targets. A manual run can also stress each target for 30 minutes, one
hour, or five hours using `soak_seconds`. Pull requests and version tags keep the
short checks. Manual runs retain archives and JSON evidence without publishing.

```sh
gh workflow run release.yml --repo maxmoneycash/turbotokens \
  --ref main -f soak_seconds=3600
```

Choose a reviewed commit or version tag with `--ref` for repeatable evidence.
Each target runs the deterministic [Claude mutation harness](../rust/bench/stress-claude.py)
with isolated synthetic logs, checks all four token categories and recorded cost,
and requires cached/uncached JSON parity. This does not cover every agent or
application integration.
