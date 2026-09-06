# turbotokens

Token usage and estimated cost reports for AI coding agents, including Claude Code, Codex, and OpenCode.

```sh
npx turbotokens
npx turbotokens codex daily
npx turbotokens live
```

For a persistent install:

```sh
npm install -g turbotokens
turbotokens --help
```

This package downloads a native Rust binary from GitHub Releases during installation and verifies its SHA-256 checksum. It supports macOS, Linux, and Windows on x64 and arm64. Installation scripts must be enabled; Windows requires the `tar` command included in current Windows installations.

Reports read local agent data. Costs are estimates and may differ from your provider bill or subscription allowance.

See the [full documentation](https://github.com/maxmoneycash/turbotokens#readme) for commands, supported agents, and direct binary downloads.

MIT. Derived from ccusage; attribution is preserved in LICENSE.
