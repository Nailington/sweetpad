---
sidebar_label: Remote Mac
---

# Build on a Mac from Windows, Linux, or another Mac

`--remote` runs the standalone SweetPad CLI on a Mac over SSH. Commands that use project files
copy the local working tree first, including uncommitted changes and untracked files, so you do
not need to push test code. Xcode, signing, simulators, and the iPhone connection stay on the Mac.

## Set up the Mac

1. Install Xcode and SweetPad on the Mac. Run `sweetpad doctor` there and open Xcode once to accept
   its license and install components.
2. Enable **Remote Login** in macOS System Settings → General → Sharing. Allow SSH access for your
   macOS user.
3. From your development machine, arrange SSH key access and verify `ssh mac-mini.local 'command -v sweetpad'` works.
   Use the Mac's Bonjour name, IP address, or an alias in `~/.ssh/config`.
4. Pair and trust the iPhone with the Mac. For a wired phone, connect it to the Mac. For a wireless
   phone, enable **Connect via network** in Xcode's Devices and Simulators window while it is paired;
   keep the phone and Mac on the same network. Confirm the phone appears with `sweetpad devices` on
   the Mac. Configure the project's signing team on the Mac as usual.

Build this SweetPad CLI on your development machine with the Rust toolchain in
`rust-toolchain.toml`:

```bash
cargo build -p sweetpad-cli --release
# On Windows use target/release/sweetpad.exe; on Unix use target/release/sweetpad.
```

On Windows the CLI is a native executable named `sweetpad.exe`; on Linux and macOS it is named
`sweetpad`. All three use the same SSH archive transfer code, without rsync or rclone. Windows
needs its OpenSSH client (`ssh.exe`), and the Unix systems need an SSH client. The receiving Mac
needs the updated SweetPad CLI from this fork and its built-in `tar`. The SSH user needs permission to create
`~/.sweetpad/remote/` on the Mac.
For a private installation, set `SWEETPAD_REMOTE_EXECUTABLE` on the client to the absolute
path of the Mac's CLI (or a launcher script). Set `SWEETPAD_REMOTE_ROOT` to choose a dedicated
workspace directory on the Mac. These settings also work when inherited by VS Code.
The `.exe` is Windows' executable format for the same CLI, not an extra service. You can invoke it
as `sweetpad` in PowerShell when its directory is on `PATH`.
On Windows, check `ssh -V` in PowerShell. If it is unavailable, install the Windows
**OpenSSH Client** optional feature before using `--remote`.

## Use it

You can save several Macs under names that are easier to remember:

```bash
sweetpad remote add "Studio Mac" dev@studio-mac.local
sweetpad remote add "Build Mac" build@192.168.1.10 --identity-file ~/.ssh/build_mac_ed25519 --port 2222
sweetpad remote list
sweetpad remote show "Studio Mac"
sweetpad build --remote "Studio Mac"
sweetpad simulator open --remote "Build Mac"
sweetpad remote add "Studio Mac" dev@new-mac.local --replace
sweetpad remote remove "Studio Mac"
```

`sweetpad remote` lists saved Macs by default. The registry lives at
`$XDG_CONFIG_HOME/sweetpad/remotes.toml`, or `~/.config/sweetpad/remotes.toml` when
`XDG_CONFIG_HOME` is unset. On Windows it lives at `%APPDATA%\sweetpad\remotes.toml`. It stores SSH targets and optional key **paths**, not passwords or
private key contents. The key file stays in its original location. Without `--identity-file`,
SweetPad uses your normal SSH config, identities, and agent. Your `~/.ssh/known_hosts` verifies
the Mac's host key; it does not authenticate your user account. On Windows, OpenSSH uses your
Windows user profile's `.ssh` directory. Password prompts still work when
SSH permits them, but passwords are not saved. For unattended commands, set up a key or SSH agent.
`--non-interactive` disables SSH authentication prompts with `BatchMode=yes`, so missing
credentials fail promptly instead of waiting for input.
`--replace` replaces the whole saved entry, including its optional key path and port.

Raw SSH hosts and aliases continue to work with `--remote`, even if you never save them.

Run from the project directory on your development machine:

```bash
sweetpad build --remote mac-mini.local
sweetpad run --remote mac-mini.local --on device
sweetpad run --remote mac-mini.local --on "My iPhone"
sweetpad simulator boot "iPhone 16 Pro" --remote mac-mini.local
sweetpad simulator open --remote mac-mini.local
sweetpad devices --remote mac-mini.local
sweetpad doctor --remote mac-mini.local
```

`--remote` works across the standalone CLI, including `app`, `test`, `archive`, `settings`,
`simulator`, `device`, `doctor`, and `open`. An SSH alias can stand in for the name, for example
`--remote build-mac`. The first run may ask for a scheme or destination; the Mac remembers that
choice in its own SweetPad state. Use `--scheme` and `--on` to make it explicit. `--on device`
selects a physical device visible to the Mac, and `--on <UDID>` selects an exact phone.

Simulator control commands such as `boot`, `shutdown`, and `open` run on the Mac without copying a
project. `simulator open` opens the Simulator window on the **Mac's display**. To see and interact
with that window from your development machine, use macOS Screen Sharing or another remote desktop tool. Screenshot
and recording commands copy their output back to your working directory. Push and media-add
transfer only the named input files:

```bash
sweetpad simulator screenshot --remote build-mac --output-file ./shots/phone.png
sweetpad simulator record --remote build-mac --output-file ./shots/demo.mp4
```

The first project command copies the project to a private directory under `~/.sweetpad/remote/`
on the Mac. Later commands scan local file sizes and modification times and archive only added or
changed paths; files deleted locally are removed from that Mac workspace. If the Mac workspace or
local sync cache is missing, the next command makes a full copy again. During `run`,
`build --watch`, and `test --watch`, the same delta sync runs every two seconds so rebuilds see
later saves. Press `q` to end an interactive run. A changed large file is still transferred in full;
this is file-level incremental sync, not block-level deduplication.
Project symlinks are preserved. Explicit push and media inputs send the target file's contents.
The scan tracks change time as well as size and modification time, including equal-size edits
that preserve modification time. Unix also tracks permissions. Windows uses native file change
time and falls back to hashing content when the filesystem cannot report it.
The local sync cache is under `$XDG_CACHE_HOME/sweetpad/sync/` (or `~/.cache/sweetpad/sync/`) on
Unix and `%LOCALAPPDATA%\sweetpad\sync\` on Windows. Removing that cache forces a full resync.
Git metadata is copied too, so build scripts that read the current commit work; nothing is pushed
to a Git server. Local build output, `node_modules`, DerivedData, `sweetpad-shots`, and `.xcresult`
bundles are excluded from uploads. Screenshots and result bundles still download after a command.
Synced commands copy files changed by the Mac command back when they finish, including archives,
screenshots, test reports, and source edits made by `format` or project commands. A newer local
file is kept if it changed while the remote command ran. The Mac workspace is private to the SSH
account; that account and any tools run there can read it.

On Linux and macOS, Git merge, SwiftPM conflict resolution, low-level project editing, and project
scaffolding act directly on the local checkout even when `--remote` is present. The native Windows
client forwards these commands to the Mac when `--remote` is supplied.

Run from the project directory or a subdirectory, or use `-C PATH` to select one. SweetPad
syncs from the nearest ancestor containing `sweetpad.toml`, `Package.swift`, `.xcodeproj`, or
`.xcworkspace`. Relative `--project` and `--workspace` paths remain relative to your working
directory; absolute paths inside the synced project are translated for the Mac. Simulator input
and output paths must be inside the working directory; use `-C` to select a different one. Project
commands refuse to sync your whole home or filesystem root. To use an iPhone, pair it with the
Mac; your development machine does not need a direct connection to the phone.

If the Mac cannot find `sweetpad` in a noninteractive SSH shell, install it in
`/opt/homebrew/bin` or `/usr/local/bin`, or add a suitable command to your SSH shell's startup
configuration. The remote transport checks those two Homebrew paths automatically.

In VS Code, add `.vscode/tasks.json` to run the remote commands from **Terminal → Run
Task**:

```json
{
  "version": "2.0.0",
  "tasks": [
    { "label": "SweetPad: remote build", "type": "shell", "command": "sweetpad build --remote build-mac" },
    { "label": "SweetPad: remote iPhone", "type": "shell", "command": "sweetpad run --remote build-mac --on device" }
  ]
}
```

Replace `build-mac` with your SSH alias. This fork's extension also has a
[remote Mac sidebar mode](../vscode/remote.md) that uses the same CLI transport.

## Editor service transport

The same executable provides `sweetpad bridge` for the extension's remote services. It syncs the
project before starting a service, keeps protocol stdout separate from progress on stderr, and
uses noninteractive SSH authentication. `sweetpad bridge --help` describes its arguments.

- `context --json --remote MAC` returns the synchronized local and Mac project roots.
- `lsp --remote MAC` prepares SweetPad's build server and transports SourceKit-LSP over stdio.
- `dap --remote MAC` transports Xcode's `lldb-dap` over stdio.
- `exec --remote MAC -- PROGRAM ARGS` runs a program in the synchronized Mac workspace.
- `stream --remote MAC -- DEVICE LOCAL_PORT REMOTE_PORT PROGRAM ARGS` runs a simulator
  streaming helper with an SSH tunnel bound to loopback on both machines.

These are services of the existing CLI, not separate CLI installations. JSON output is available
for `context`; protocol services own stdout. Set `--developer-dir` to select an Xcode installation.
Normal cancellation keeps SSH open while the remote process finalizes and outputs download.
