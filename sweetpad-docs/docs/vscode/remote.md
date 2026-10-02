---
sidebar_label: Remote Mac
---

# Use the SweetPad sidebar from Windows or Linux

This fork's VS Code extension can run its build, run, test, clean, and simulator controls on a Mac
over SSH. The Windows or Linux extension host uses the updated SweetPad CLI instead of loading the Mac-only
native add-on. Your source files stay in the local workspace; the CLI syncs them to the Mac before
project commands. You do not have to commit or push a test build.

## Install

1. Install the updated SweetPad CLI from this fork and an OpenSSH client. Windows runs the
   native `sweetpad.exe`; Linux and macOS run `sweetpad`. All use the same SSH archive transfer and do not require rsync or WSL. Install the same CLI on
   the Mac, along with Xcode. Enable **Remote Login** on the Mac. See the [CLI remote setup](../cli/remote.md).
2. Build and install this fork's VS Code extension on Windows or Linux. In `sweetpad-vscode`, run `npm ci` and
   `npm run build`; package the extension with `npx vsce package --no-dependencies`, then install
   that VSIX in VS Code. The current Marketplace release does not contain this fork's remote UI.
3. Add a Mac with `sweetpad remote add "Studio Mac" user@mac.local`, or use **SweetPad: Add Remote
   Mac** in the Command Palette. The GUI also accepts a key path and SSH port. SSH config aliases
   and agent identities work without a saved key path.
4. Open the project folder in VS Code. Choose **SweetPad: Select Remote Mac** or click the Mac name
   in the status bar. The selection is saved in the workspace's `sweetpad.remote.mac` setting.

If the CLI is not on VS Code's `PATH`, set `sweetpad.remote.cliPath` to its absolute path. On a Mac,
set `sweetpad.remote.mac` and reload the extension window to switch from local to remote mode.

## Use the sidebar

The **Build** view lists schemes discovered on the selected Mac. The **Destinations** view lists
simulators and phones connected to that Mac. Select a destination, then use the existing **Build**,
**Build & Run**, **Run**, **Test**, and **Clean** buttons. Their output opens in a VS Code terminal;
**Stop** sends Ctrl-C to active remote commands and keeps their terminals open while the CLI
finishes and retrieves artifacts. Simulator boot, shutdown, and open actions use the
Mac. **Open Simulator** opens a window on the Mac's display; **Stream Simulator** opens an
interactive simulator in VS Code. **Refresh** reloads the scheme or destination list after changes.

Use **SweetPad: Run Remote CLI Command** for another CLI action, for example `simulator list` or
`app logs`. This accepts arguments, not shell operators. It uses the selected Mac and the current
project folder.

## Debugging and Swift language services

Use **SweetPad: Build & Debug** or a launch configuration with `"type": "sweetpad-remote"` and
`"request": "attach"`. SweetPad builds and launches the selected app waiting for the debugger,
then attaches Xcode's LLDB on the Mac. Breakpoints, stack frames, variables and expressions use
VS Code's normal debugger UI; paths inside the project map back to local source. A configuration
with an explicit `pid` attaches to that Mac process, while `request: launch` with `program` starts
a Mac executable. Automatic debugger attachment is supported for Mac processes and simulators.
Physical-device debugging requires an explicit LLDB configuration; phone deployment and attachment
have not been validated in this remote workflow. Existing `sweetpad-lldb` configurations use the remote adapter in remote mode.

SourceKit starts automatically through the CLI's build-server bridge. Hover, completion,
diagnostics and navigation use the selected Mac's SDK and the local editor's current text.
Project files map to Windows/Linux paths; SDK files and generated Swift interfaces open read-only.
Saving a Swift file syncs it to the Mac. **SweetPad: Set Up Build Server** restarts the language
service. Set `sweetpad.remote.languageServer.enabled` to false to disable automatic startup.
Set `sweetpad.remote.developerDir` when the Mac has multiple Xcode installations.

## Testing panel

Refresh **SweetPad Remote** in VS Code's Testing view to discover the selected Xcode scheme or
Swift package's tests. Run all tests or selected cases through the normal panel controls; each
test receives its own result. Xcode results come from the retained xcresult bundle. Canceling a
run requests a graceful remote interrupt. The sidebar also supports **Build for Testing** and
**Test Without Building**. Use `sweetpad.remote.xcodebuildArgs` for additional Xcode arguments,
such as a private `-derivedDataPath`; values are separate arguments, not shell expressions.

## Simulator streaming and SwiftUI previews

**Stream Simulator** tunnels an interactive simulator to a VS Code webview. Browser and copy-URL
actions use the same stream. The default helper is pinned `serve-sim@0.1.47`, downloaded with
`npx` into a cache inside the private Mac workspace. It requires Node.js/npm and an Apple Silicon
Mac. SSH must permit local port forwarding. The service binds to loopback and stops when its
panel closes or the extension deactivates. `sweetpad.remote.streamCommand` and
`sweetpad.remote.streamArgs` allow an already installed compatible helper.

The Previews view and editor CodeLens discover `#Preview` and legacy `PreviewProvider` declarations.
Run **Set Up SwiftUI Previews**, include the generated `SweetPadPreviewHost.swift` in the app
target, and call `SweetPadPreviewHost.rootView()` from the app's root as described in its header.
Rendering builds a Debug app on the selected simulator and checks that the preview host actually
rendered the requested preview. Saving Swift source rebuilds and refreshes the selected preview.
Screenshot actions download PNGs, including light/dark variants, into `sweetpad-shots`.

Preview registry reflection depends on SwiftUI's SDK internals. Xcode 26.0.1's macro previews
and legacy providers have been exercised; a host that cannot render reports an error instead
of presenting the app's fallback UI as a successful preview. Previews rebuild and relaunch,
so view state is reset on save. Simulator streaming is distinct from Xcode Canvas.

All these features live in the same SweetPad extension and use the same SweetPad CLI. Mac-local
utilities without a remote workflow show an explanatory message; **Run Remote CLI Command**
remains available for their CLI equivalents.
