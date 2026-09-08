# Install and upgrade

Choose [macOS](#macos), [Windows](#windows), or [Linux](#linux). Release binaries
are available for x64 and ARM64. The native install runs without Node.js; npm
downloads the same executable and launches it through Node.

**Native release: v1.1.3.** npm and Homebrew currently install v1.1.2 while their
package updates are prepared. Use the standalone macOS/Linux installer or the
Windows ZIP for v1.1.3. The npm pin below names its currently available version.

## macOS

With [Homebrew](https://brew.sh) installed:

```sh
brew install maxmoneycash/tap/turbotokens
turbotokens --version
```

Homebrew selects the Apple Silicon or Intel build. To install directly into your
home directory instead:

```sh
curl -fsSL https://raw.githubusercontent.com/maxmoneycash/turbotokens/main/install.sh | env TURBOTOKENS_INSTALL_DIR="$HOME/.local/bin" sh
"$HOME/.local/bin/turbotokens" --version
```

Then [add the directory to your shell's PATH](#shell-path) and run your
[first report](#first-report).

## Windows

### Install with npm

Install a supported [Node.js LTS release](https://nodejs.org/en/download), then
open a new **PowerShell** or **Command Prompt** window:

```text
npm install -g turbotokens
turbotokens --version
turbotokens doctor
```

Keep npm installation scripts enabled: the package's `postinstall` step downloads
the executable. If it reports a missing binary, reinstall with
`npm install -g turbotokens --ignore-scripts=false`. The installer also needs
`tar.exe`; if that command is unavailable, use the native ZIP below.

If a v1.1.2 npm install reports `tar: Cannot connect to C`, use the native ZIP
below. Git Bash or another PATH override can select an incompatible `tar`.
The v1.1.3 installer selects Windows' native extractor explicitly.

If PowerShell reports that `npm.ps1` or `turbotokens.ps1` cannot run because scripts
are disabled, use npm's Command Prompt shims directly:

```powershell
npm.cmd install -g turbotokens
turbotokens.cmd --version
turbotokens.cmd doctor
```

If the command is still missing, run `npm.cmd config get prefix`. Add the printed
directory to your user PATH using the [steps below](#windows-path), then reopen
your terminal. npm places its Windows command shims directly in that
[prefix directory](https://docs.npmjs.com/cli/v11/configuring-npm/folders/).

### Install the native ZIP

Open a [release](https://github.com/maxmoneycash/turbotokens/releases/latest) and
download **one ZIP plus `SHA256SUMS` from that same release**:

| Windows system type | Archive |
| --- | --- |
| Intel or AMD x64 | `turbotokens-windows-x64.zip` |
| ARM64 | `turbotokens-windows-arm64.zip` |

Find your system type in **Settings → System → About**. Save both downloads in
the same folder and open PowerShell there. For x64, verify the archive:

```powershell
Get-FileHash -LiteralPath .\turbotokens-windows-x64.zip -Algorithm SHA256
Select-String -Path .\SHA256SUMS -Pattern '  turbotokens-windows-x64\.zip$'
```

Compare the `Hash` value with the 64-character checksum on the matching filename's
line. Uppercase and lowercase hex digits are equivalent. Continue only when the
values match. For ARM64, replace `x64` with `arm64` in both commands. Microsoft's
[Get-FileHash reference](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.utility/get-filehash)
explains this check.

Extract the verified ZIP into a folder belonging to your account:

```powershell
Expand-Archive -LiteralPath .\turbotokens-windows-x64.zip -DestinationPath "$env:LOCALAPPDATA\turbotokens"
Set-Location "$env:LOCALAPPDATA\turbotokens"
.\turbotokens.exe --version
.\turbotokens.exe doctor
```

Use the ARM64 filename in `Expand-Archive` if that is the ZIP you downloaded.
The archive contains `turbotokens.exe`, `README.md`, and `LICENSE` directly at
its root. PowerShell requires the
[`.\` prefix](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_command_precedence)
when running an executable from the current directory.

### Windows PATH

To run `turbotokens` from any folder:

1. Search Start for **Edit environment variables for your account**.
2. Under **User variables**, select **Path → Edit → New**.
3. Add the folder containing `turbotokens.exe`, usually
   `C:\Users\YOUR_NAME\AppData\Local\turbotokens`. For an npm install, add the
   directory printed by `npm.cmd config get prefix` instead.
4. Save the dialogs and open a new terminal. Run `turbotokens --version`.

Use your actual username in the path. From the native install folder,
`(Get-Location).Path` prints the full path to copy. Until PATH is set, use
`.\turbotokens.exe` in that folder for the [report commands](#first-report).

### WSL

If your coding agent runs inside WSL, follow the [Linux instructions](#linux)
**inside that same WSL distribution**. Its Linux home directory and agent logs
are separate from your Windows user profile. The Linux release's libc
requirements also apply inside WSL.

If your agent runs on Windows, the Windows executable can read its Windows logs.
To read those logs from WSL instead, point the Linux command at the Windows
profile's mounted directories. Replace `YOUR_NAME` with your Windows username:

```sh
env CLAUDE_CONFIG_DIR="/mnt/c/Users/YOUR_NAME/.claude" turbotokens claude daily --offline
env CODEX_HOME="/mnt/c/Users/YOUR_NAME/.codex" turbotokens codex daily --offline
```

These are examples for WSL's default `C:` mount. `CLAUDE_CONFIG_DIR` should contain
`projects/`; `CODEX_HOME` should contain `sessions/`. Use the directories your
agent actually writes to. See Microsoft's
[WSL filesystem guide](https://learn.microsoft.com/en-us/windows/wsl/filesystems)
for mount paths and cross-filesystem behavior.

## Linux

**The v1.1.3 Linux downloads are static builds.** Their release checks pass on
Ubuntu 22.04, Debian 12, and Alpine 3.22 for x64 and ARM64. See
[Linux release validation](../packaging/linux.md) for the checks and build
instructions.

Older v1.1.2 Linux binaries require **glibc 2.39**, including those still downloaded
by npm or Homebrew until their package updates are published. The standalone
installer below downloads the static v1.1.3 release.

On x64 or ARM64, install into your home directory:

```sh
curl -fsSL https://raw.githubusercontent.com/maxmoneycash/turbotokens/main/install.sh | env TURBOTOKENS_INSTALL_DIR="$HOME/.local/bin" sh
"$HOME/.local/bin/turbotokens" --version
```

The installer needs `curl` and `tar` and selects your architecture. The release
page also provides `SHA256SUMS` for manual verification.
Setting `TURBOTOKENS_INSTALL_DIR` keeps the destination predictable. Without it,
the installer uses `/usr/local/bin` when writable, or `~/.local/bin` otherwise.

### Shell PATH

For **zsh**, add this line to `~/.zshrc`. For **Bash**, add it to `~/.bashrc` on
Linux, or `~/.bash_profile` for macOS Terminal's login shells:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

Open a new terminal after saving. You can also run that line directly to update
your current Bash or zsh session. If your Linux login shell uses `~/.bash_profile`,
ensure it loads `~/.bashrc`, or add the PATH line there too.

For **fish**, run this once after installation; it persists across sessions:

```fish
fish_add_path --universal "$HOME/.local/bin"
```

See [fish_add_path](https://fishshell.com/docs/current/cmds/fish_add_path.html).
Check which executable your shell finds with `command -v turbotokens`.

## First report

After installation and PATH setup, these commands work on all three platforms:

```text
turbotokens --version
turbotokens doctor
turbotokens
turbotokens claude daily --last 7
turbotokens codex daily
```

`doctor` shows the log directories and diagnostics. Plain `turbotokens` reports
across detected agents; naming `claude` or `codex` limits the source. If your logs
use a custom location, check `CLAUDE_CONFIG_DIR` or `CODEX_HOME` in the environment
where you run the command. Reports need existing usage logs from the coding agent.

For scripts and agents, request JSON explicitly:

```text
turbotokens claude daily --offline --json
turbotokens codex daily --offline --json
```

`--offline` avoids fetching pricing for these reports. Keep the source explicit
when [migrating a ccusage workflow](migrating-from-ccusage.md); combined reports
have a different JSON shape. More commands are in the [usage guide](usage.md).

## Upgrade or pin a version

Use the method you originally installed with, then check `turbotokens --version`.

| Install method | Upgrade |
| --- | --- |
| Homebrew | Run `brew update`, then `brew upgrade turbotokens`. |
| npm | Run `npm install -g turbotokens@latest` (or `npm.cmd` in PowerShell). |
| Shell installer | Run the install command again with the same `TURBOTOKENS_INSTALL_DIR`. |
| Native Windows ZIP | Download and verify the new release's ZIP and `SHA256SUMS`, then extract into the same install folder with `Expand-Archive -Force`. Close any running turbotokens process first. |

For a reproducible npm install, name the version:

```text
npm install -g turbotokens@1.1.2
```

Or try that version without a global install:

```text
npx --yes --ignore-scripts=false turbotokens@1.1.2 --version
```

On macOS or Linux, pin the downloaded binary with `TURBOTOKENS_VERSION`:

```sh
curl -fsSL https://raw.githubusercontent.com/maxmoneycash/turbotokens/main/install.sh | env TURBOTOKENS_VERSION=v1.1.3 TURBOTOKENS_INSTALL_DIR="$HOME/.local/bin" sh
```

This pins the release binary; the installer script still comes from `main`.
For a native Windows pin, keep the ZIP and `SHA256SUMS` from the chosen
[versioned release](https://github.com/maxmoneycash/turbotokens/releases/tag/v1.1.3).
Version pins retain that release's platform requirements. In particular, the
v1.1.2 Linux binaries require glibc 2.39.
