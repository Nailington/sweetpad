---
name: sweetpad-remote
description:
  Build, run, test, and inspect Apple apps on a Mac over SSH from native Windows, Linux, or macOS using SweetPad. Use
  for remote Mac setup, saved hosts, incremental sync, remote CLI workflows, or the VS Code remote integration.
---

# Drive SweetPad on a remote Mac

Edit on the client and run Xcode, simulators, and trusted-device operations on the receiving Mac. Run from the local
project directory or use `-C <directory>`; project commands discover the nearest project root and sync it privately.

## Set up remote access

Remote workflows require the updated CLI with remote support on both the client and the receiving Mac. Do not replace a
tested checkout's CLI with a Homebrew release that lacks these commands. Confirm support with `sweetpad remote --help`
and `sweetpad bridge --help`. Windows runs the native `sweetpad.exe`; Linux and macOS run `sweetpad`. Each uses the
system OpenSSH client, without WSL, rsync, or rclone. The Mac needs Xcode, Remote Login, and its built-in `tar`.

Save an SSH target or use an existing SSH config alias:

```bash
sweetpad remote add "Build Mac" developer@mac.local
sweetpad remote list -o json --non-interactive
sweetpad doctor --remote "Build Mac" -o json --non-interactive
sweetpad devices --remote "Build Mac" -o json --non-interactive
```

`remote add` accepts `--identity-file <path>` and `--port <port>`. It saves key paths, not private key contents or
passwords. SSH config and agent identities work normally; `known_hosts` verifies the host rather than authenticating the
user. Establish trusted SSH access before noninteractive commands, which use `BatchMode=yes` and cannot answer password
or host-key prompts.

Add `--remote "Build Mac"` to a Mac command to execute it on that Mac. Project commands sync uncommitted and untracked
files into a private workspace; later uploads send changed files and deletions. Host commands such as `devices`,
`doctor`, and `simulator open` do not copy the project. Artifacts and source edits download when a synced command
finishes, with newer local edits protected. `simulator open` opens the window on the Mac's display; the extension's
Stream Simulator action provides a separate client view.

```bash
sweetpad build --remote "Build Mac" --scheme MyApp --on "<simulator UDID>" -o json --non-interactive
sweetpad run --remote "Build Mac" --scheme MyApp --on "<simulator UDID>" --detach -o json --non-interactive
sweetpad test --remote "Build Mac" --scheme MyApp --on "<simulator UDID>" -o json --non-interactive
sweetpad device info "<phone UDID>" --remote "Build Mac" -o json --non-interactive
```

Check a phone's readiness on the Mac with `device info <UDID>` before selecting it; an idle phone can appear
disconnected in `devices` while still reachable. Signing must work in the SSH session; a signing identity available in
an interactive login may be locked or inaccessible there. Do not change keychain access or signing settings without
authorization.

On Unix, merge, SwiftPM conflict resolution, project editing, and scaffolding still act on the local checkout when
`--remote` is present. Windows forwards them to the Mac. A remote command retains the receiving CLI's device support:
automatic physical-device debugging, hot reload, and simulator-only actions must not be assumed to work on a phone.

For private installations, set `SWEETPAD_REMOTE_EXECUTABLE` to the Mac's CLI path and `SWEETPAD_REMOTE_ROOT` to its
dedicated workspace directory. In VS Code, select the Mac with **SweetPad: Select Remote Mac** and set
`sweetpad.remote.cliPath` if the client CLI is not on its `PATH`. See
[`remote.md`](../../sweetpad-docs/docs/cli/remote.md) and the
[remote extension guide](../../sweetpad-docs/docs/vscode/remote.md) for setup, language services, testing, streaming
requirements, and current limitations.

## Read results and finish sessions

Use `-o json --non-interactive` for finite commands. Success is a `{schema, ok, data}` envelope on stdout; errors carry
`error.code` and `error.message` on stderr. Inspect both the process exit code and the payload. For tests, `ok: true`
means the command ran; require `data.passed: true` and check `passedTests` and `failedTests` for the expected suite.
Failed tests exit 3.

After a failed build, read `sweetpad build diagnostics --remote "Build Mac" -o json --non-interactive` with the same
scheme and destination. Do not truncate build output or rebuild just to retrieve diagnostics. After editing test code,
use `sweetpad test build` to compile the test targets; a regular build only compiles the scheme's Run targets.

Use `run --detach` or `run --no-logs` when the command must return after launch. Plain `run`, `build --watch`,
`test --watch`, and `run --hot` are long-lived sessions. Bound `app logs` with `--last`, `--until`, or `--timeout` and
use `-o ndjson` for streaming output. If testing a live session, own its timeout and cancellation. Let the CLI finalize
and download artifacts after Ctrl-C instead of killing SSH immediately.

Repeat explicit `--scheme` and `--on` selections on subsequent commands; typed flags are not remembered automatically.
Use exact device UDIDs when a Mac remembers several phones, and check `device info`'s `ready` and `reason` fields with
its bounded `--timeout` (10 seconds by default).

## Discover commands and editor features

Prefer `sweetpad <command> --help` over guessing flags. The CLI's remote support retains the Mac command's requirements
and device restrictions. The bridge owns stdout for LSP and DAP; do not parse those services as JSON envelopes.

In VS Code, build/run/stop and simulator controls use the selected Mac. The Testing panel reads individual outcomes from
xcresult, while language services and the debugger map project paths back to the client. Stream Simulator needs
Node/npm, a compatible helper (the default uses Apple Silicon), and SSH port forwarding. `simulator open` alone does not
stream a screen.

For full local command guidance, read the sibling [SweetPad skill](../sweetpad/SKILL.md) when it is available. The
remote guides linked above describe synchronization exclusions, file-level transfer, private workspace access, and
editor limitations. A changed large file transfers in full; incremental sync is not block-level deduplication.
