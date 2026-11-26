# How to publish

SweetPad ships two artifacts, versioned and released independently:

- the **VSCode extension**: `v*` tags → VS Marketplace / Open VSX
- the **`sweetpad` CLI**: `cli-v*` tags → Homebrew

## VSCode extension

### Links

- https://code.visualstudio.com/api/working-with-extensions/publishing-extension#publish-an-extension
- https://marketplace.visualstudio.com/manage/publishers/sweetpad/extensions/sweetpad/hub
- https://open-vsx.org/extension/sweetpad/sweetpad

### Steps

1. Update & commit CHANGELOG.md

2. Update the version number and publish to Github (the extension lives in `sweetpad-vscode/`):

```shell
npm run publish-patch -w sweetpad
```

3. Check publishing status on github actions page:

- https://github.com/sweetpad-dev/sweetpad/actions

4. Verify the GitHub release page includes the VSIX asset and the changelog section for the tag:

- https://github.com/sweetpad-dev/sweetpad/releases

The `v*` tag triggers `.github/workflows/ci.yaml`, which builds the VSIX (Developer ID-signing + notarizing the bundled `.node` addon) and publishes to the VS Marketplace, Open VSX, and a GitHub release.

## CLI (Homebrew)

Install Nailington's CLI with `brew install nailington/tap/sweetpad-nailington`.
See [the fork's release guide](https://github.com/Nailington/sweetpad/blob/codex/windows-remote-testing/RELEASES.md)
for the release branch, version tags, and Homebrew packaging.
