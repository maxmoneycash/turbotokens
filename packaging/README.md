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

The npm smoke test packs the package, installs it in a temporary directory, exercises the executable shim, and checks missing-binary and checksum-failure handling. It downloads release assets and works on macOS, Linux, and Windows. The shell tests use a local archive and require a Unix shell.

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

The shell installer supports `TURBOTOKENS_VERSION` (for example `v1.1.0`) and `TURBOTOKENS_INSTALL_DIR`. Test it in a temporary directory before release:

```sh
TURBOTOKENS_VERSION=v1.1.0 TURBOTOKENS_INSTALL_DIR=/tmp/turbotokens-install sh install.sh
/tmp/turbotokens-install/turbotokens --version
```
