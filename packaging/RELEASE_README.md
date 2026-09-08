# turbotokens

Token usage and estimated cost reports for AI coding agents. This archive
contains the native executable; it needs no Node.js runtime or turbotokens
account. Reports read the usage logs already on your machine.

## Run from the extracted folder

On **macOS or Linux**, open a terminal in this folder:

```sh
./turbotokens --version
./turbotokens doctor
./turbotokens claude daily --offline
```

On **Windows**, open PowerShell in this folder:

```powershell
.\turbotokens.exe --version
.\turbotokens.exe doctor
.\turbotokens.exe claude daily --offline
```

Replace `claude` with `codex` to read Codex logs, or omit the agent and report
arguments to see daily usage across detected agents. Add `--json` for scripts.
`doctor` explains which log directories were found. Costs are estimates.

To run the command from any folder, add the extracted folder to your PATH.
The installation guide below includes steps for each platform and WSL.

## Guides for this release

- [Install, verify checksums, and configure PATH](https://github.com/maxmoneycash/turbotokens/blob/@REVISION@/docs/installation.md)
- [Migrate an existing ccusage integration](https://github.com/maxmoneycash/turbotokens/blob/@REVISION@/docs/migrating-from-ccusage.md)
- [Reports, live monitoring, and other commands](https://github.com/maxmoneycash/turbotokens/blob/@REVISION@/docs/usage.md)
- [Full README and examples](https://github.com/maxmoneycash/turbotokens/blob/@REVISION@/README.md)
- [Report a bug](https://github.com/maxmoneycash/turbotokens/issues/new?template=bug_report.yml)

MIT licensed. See the included `LICENSE`. turbotokens began as a fork of
[ccusage](https://github.com/ccusage/ccusage) by @ryoppippi; its attribution is
preserved.
