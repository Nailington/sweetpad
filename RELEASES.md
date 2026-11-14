# Releasing Nailington's SweetPad fork

Release development happens on `codex/windows-remote-testing`. Upstream tags do
not publish this fork. The workflows keep the CLI and extension on separate
release schedules, as upstream does.

## Release identities

- CLI workspace version: `0.1.12-nailington.2`; tag: `cli-v0.1.12-nailington.2`.
  Increment the final fork number for another release of the same upstream version.
  `sweetpad --version` includes `-nailington`; untagged builds also include `-dev+<commit>`.
- VS Code: **SweetPad (Nailington)**, ID `nailington.sweetpad-nailington`.
  Manifest version: `0.2.18`; tag: `v0.2.18-nailington`.
  VS Code requires numeric versions, so the fork suffix is in the name, identity,
  tag and VSIX filename. Increment the manifest version for each new VSIX.
- Homebrew formula: `sweetpad-nailington`. The command remains `sweetpad` for
  compatibility; Homebrew also installs a `sweetpad-nailington` alias. The formula
  conflicts with upstream `sweetpad` because both install that command.

Disable upstream SweetPad in VS Code when using this fork. The extensions have
separate identities, but contribute the same commands and settings.

## What CI produces

The **Release CLI (Nailington)** workflow builds:

- `sweetpad-cli-<version>-macos-universal.tar.gz` for Apple Silicon and Intel Macs,
  including the bundled hot-reload injection clients.
- `sweetpad-cli-<version>-windows-x64.zip`, containing `sweetpad.exe`.
- `sweetpad-cli-<version>-linux-x64.tar.gz` (glibc 2.35 or newer).
- `sweetpad-agent-skills-<version>.zip`, containing every repository skill and its
  supporting files from the tagged commit, plus `INSTALL.md`.
- `sweetpad-nailington.rb`, generated from the actual uploaded archive digests.
- `SHA256SUMS`, covering every download above.

The **Release VS Code extension (Nailington)** workflow builds one universal
`sweetpad-<version>-nailington.vsix`, the agent-skills ZIP, and `SHA256SUMS`.
The VSIX bundles the universal Mac native addon and the Windows/Linux remote UI.
Use **Extensions → … → Install from VSIX** to install it in VS Code or Cursor.
The CLI is distributed separately; set `sweetpad.remote.cliPath` if it is not on PATH.

Windows/Linux Apple builds need OpenSSH and a Mac with Xcode and this fork's CLI.
A local Mac installation uses Xcode directly.

## Dry runs

Run either workflow manually from the Actions tab and select
`codex/windows-remote-testing`. These runs upload downloadable Actions artifacts
with `-dryrun.<run-number>` filenames. They never publish a GitHub release, update
Homebrew or publish to a registry, even if manually run against a tag.
Both dry runs also upload the agent skills.

GitHub only lists manually dispatchable workflows once they exist on its default
branch. If this branch has not been merged there, push a suffixed release tag to
run its release workflow, or install the workflow files on the default branch
before trying a manual run. Changing the T3 checkout does not change GitHub's
repository default branch.

## Publish the current versions

Commit and push the release configuration and application changes to your fork
first. From that exact commit on `codex/windows-remote-testing`:

```bash
git tag -a cli-v0.1.12-nailington.2 -m "SweetPad CLI 0.1.12-nailington.2"
git push origin cli-v0.1.12-nailington.2

git tag -a v0.2.18-nailington -m "SweetPad VS Code 0.2.18 (Nailington)"
git push origin v0.2.18-nailington
```

Each workflow validates its tag against its manifest before building. A mismatched
version fails instead of publishing a mislabeled artifact. Regular tags such as
`cli-v0.1.12` and `v0.2.18` do not trigger these fork workflows.

For later releases, the existing helpers now target this fork's branch and tags:

```bash
# macOS with Xcode and Rust; bumps Cargo.toml/Cargo.lock, checks, commits and tags.
sweetpad-lib/ci/release.sh 0.1.12-nailington.3

# Add the next numeric version's CHANGELOG entry first; then bump, check and tag.
npm run publish-patch --workspace sweetpad-nailington
```

Both helpers confirm before pushing. Set `SWEETPAD_RELEASE_BRANCH` if the fork's
release branch changes. Do not reuse published tags; bump the relevant version.

## Homebrew setup

No second tap repository or deploy key is required. After a successful CLI release,
CI commits `Formula/sweetpad-nailington.rb` to this repository's **GitHub default
branch**, using `GITHUB_TOKEN` with `contents: write`. The release itself comes
from the tagged commit, which may be on `codex/windows-remote-testing`.
Ensure branch rules allow the GitHub Actions bot to push this formula commit.
If that push is blocked, the binary release remains available, including its
formula; fix the branch rule and rerun the Homebrew job.

After the first successful release:

```bash
brew tap Nailington/sweetpad https://github.com/Nailington/sweetpad
brew install Nailington/sweetpad/sweetpad-nailington
# Later:
brew update
brew upgrade sweetpad-nailington
```

The explicit tap URL is needed because the repository is named `sweetpad`,
instead of Homebrew's conventional `homebrew-sweetpad`.

## Optional signing and registries

GitHub downloads and Homebrew releases work without these optional settings.
Configure repository variables and secrets under **Settings → Secrets and
variables → Actions**.

To sign and notarize the macOS CLI and addon, set variable
`MACOS_SIGNING_ENABLED=true` and add these secrets:

- `MACOS_CERTIFICATE_P12`: base64 Developer ID Application certificate.
- `MACOS_CERTIFICATE_PASSWORD`: certificate password.
- `MACOS_NOTARY_KEY_P8`: base64 App Store Connect API key.
- `MACOS_NOTARY_KEY_ID` and `MACOS_NOTARY_ISSUER_ID`.

Without signing enabled, macOS artifacts are unsigned and unnotarized; Gatekeeper
may require explicit approval. Release notes indicate whether signing was enabled.

To publish the VSIX to the Visual Studio Marketplace, first create/own the
`nailington` publisher, add secret `VS_MARKETPLACE_TOKEN`, and set variable
`PUBLISH_VS_MARKETPLACE=true`. For Open VSX, own namespace `nailington`, add
`OPEN_VSX_TOKEN`, and set `PUBLISH_OPEN_VSX=true`. Both are off by default and
only run after a tagged GitHub release succeeds. Neither workflow uses upstream's
publisher or tap credentials.

Telemetry uploads are off by default. If wanted, configure your own Sentry org,
project and credentials in the build environment; do not reuse upstream's DSN.

## Install the bundled agent skills

Download `sweetpad-agent-skills-<version>.zip` from either release and extract it.
Install the CLI, then copy each complete `skills/<name>/` directory into your
agent's skill directory, or run this from the extracted directory:

```bash
npx skills add ./skills
```

Both `sweetpad` and `sweetpad-remote` are included. Their instructions point to
this fork. Supporting references, scripts or assets added to any skill are
included automatically in future releases.

## Validation

```bash
python3 -m unittest discover -s .github/scripts -p 'test_*.py' -v
python3 .github/scripts/release.py metadata cli
python3 .github/scripts/release.py metadata extension
```

The release-checks workflow runs these tests plus actionlint on release changes.
Native runners build and smoke-test the CLI before packaging; the extension
release checks types and unit tests after building its current native addon.
