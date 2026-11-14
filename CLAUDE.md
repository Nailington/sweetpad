# sweetpad

A Rust workspace (`sweetpad-lib`, `sweetpad-core`, `sweetpad-cli`,
`sweetpad-vscode/native`) plus the VS Code extension. `sweetpad-cli/CLI_DESIGN.md`
is the CLI's design record — grammar, output model, and the per-version feature
sections (§9a onward), including directions that are written down but not
scheduled. `sweetpad-lib/CLAUDE.md` covers the project-file and build-settings
crate.

## Releasing this fork

Release development uses `codex/windows-remote-testing`, tracking
`origin/codex/windows-remote-testing`. See `RELEASES.md` for setup and commands.

The CLI workspace version is `X.Y.Z-nailington.N`; only the matching
`cli-vX.Y.Z-nailington.N` tag publishes this fork. Untagged binaries additionally
report `-dev+<commit>`. `sweetpad-lib/ci/release.sh` accepts the complete suffixed
version, tests, bumps, commits, tags and confirms before pushing.

The CLI workflow builds macOS universal, Windows x64 and Linux x64 binaries,
packages all repository agent skills, publishes checksums and a Homebrew formula,
and updates `Formula/sweetpad-nailington.rb` on the GitHub default branch. Signing
and notarization are opt-in. It never updates upstream's Homebrew tap.

The extension keeps a numeric VS Code version; its tag is `vX.Y.Z-nailington`,
and its identity is `nailington.sweetpad-nailington`. `publish-patch.sh` requires
the next version's CHANGELOG entry, then bumps and tags on the fork release
branch. The workflow packages a universal VSIX and all agent skills to GitHub.
Marketplace/Open VSX publishing is opt-in under the fork publisher/namespace.

Both workflows support manual artifact builds without publication. Tag versions
must match the manifests before any release build proceeds. Helpers default to
`codex/windows-remote-testing`; `SWEETPAD_RELEASE_BRANCH` overrides that choice.

## Options objects: `?` versus `| undefined`

**Do not mark an options property optional when the call sites all hold the
value.** Declare it `foo: T | undefined` instead, so a caller with nothing to
pass has to say so.

Both read the same at the use site. They differ for the next call somebody
adds: `foo?: T` lets it compile while quietly taking the default, while
`foo: T | undefined` is a type error until it is answered. `refreshBuildServer`
carries `configuration` this way, which is what makes each of its six call
sites name one instead of falling through to Debug, and
`repairStaleBuildServerConfig` carries `workspaceState` the same way.

Keep `?` for what a caller really can have no opinion on: a flag with a
default, a rarely-used override. A spec that would otherwise need a stub is not
one of those — write the stub. Loosening a signature to keep a test call short
means the tests stop exercising the shape production actually uses.

## Writing the extension CHANGELOG

`sweetpad-vscode/CHANGELOG.md` is what users read on the Marketplace, and
`publish-patch.sh` refuses to cut a release without a `## [<version>]` entry for
the version it is about to tag.

**Write each entry as a title, not a sentence.** Ten words is a good ceiling:
one clause, no colon splicing two halves together, no subordinate clause
explaining the mechanism. The CLI's notes are commit subjects, so they stay
terse by construction; the extension's are hand-written, and the failure mode is
a 60-word entry that packs the symptom, the root cause and the fix into one
breath. Say what changed from the user's side and stop. Anyone who wants the
diagnosis has the linked issue.

    too long  - Fix schemes and targets from a workspace's local Swift packages
                never reaching the Build and Testing panels: a package joins an
                `.xcworkspace` as a reference to its directory, and SweetPad
                read only the references ending in `.xcodeproj`
    a title   - Fix Swift package schemes missing from the Build and Testing
                panels

Attribution stays as it is: link the issue or PR, and thank the reporter.

    - Fix Swift package schemes missing from the Build and Testing panels
      ([#327](https://github.com/sweetpad-dev/sweetpad/issues/327),
      thanks [@rssole](https://github.com/rssole))

Entries are written one per user-visible change, newest version on top, and the
version's own entries ordered features first, then fixes.
