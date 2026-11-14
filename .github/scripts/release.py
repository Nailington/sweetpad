#!/usr/bin/env python3
"""Fork release metadata, archives, checksums, and Homebrew formula generation."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import tarfile
import tomllib
import zipfile


def metadata(kind, root=Path("."), env=os.environ):
    if kind == "cli":
        version = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]
        if not re.fullmatch(r"\d+\.\d+\.\d+-nailington\.\d+", version):
            raise ValueError("CLI version must be X.Y.Z-nailington.N in Cargo.toml")
        tag = f"cli-v{version}"
    else:
        pkg = json.loads((root / "sweetpad-vscode/package.json").read_text())
        version = pkg["version"]
        if not re.fullmatch(r"\d+\.\d+\.\d+", version):
            raise ValueError("VS Code requires a numeric X.Y.Z version")
        if (pkg["publisher"], pkg["name"]) != ("nailington", "sweetpad-nailington"):
            raise ValueError("Refusing to release under an unexpected VS Code extension identity")
        tag = f"v{version}-nailington"
    ref = env.get("GITHUB_REF", "")
    if ref.startswith("refs/tags/") and ref != f"refs/tags/{tag}":
        raise ValueError(f"Tag {ref} does not match manifest; expected {tag}")
    publish = env.get("GITHUB_EVENT_NAME") == "push" and ref == f"refs/tags/{tag}"
    asset_version = version if kind == "cli" else f"{version}-nailington"
    if not publish:
        asset_version += f"-dryrun.{env.get('GITHUB_RUN_NUMBER', 'local')}"
    return {"version": version, "tag": tag, "asset_version": asset_version, "publish": str(publish).lower()}


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def extension_notes(version, root=Path(".")):
    lines = (root / "sweetpad-vscode/CHANGELOG.md").read_text().splitlines()
    entry = []
    capture = False
    for line in lines:
        if line.startswith("## ["):
            if capture:
                break
            capture = line.startswith(f"## [{version}]")
        elif capture:
            entry.append(line)
    notes = "\n".join(entry).strip()
    if not notes:
        raise ValueError(f"No nonempty CHANGELOG entry for extension {version}")
    return notes


def package(binary, platform, version, dist):
    if not re.fullmatch(r"\d+\.\d+\.\d+-nailington\.\d+(?:-dryrun\.(?:\d+|local))?", version):
        raise ValueError("Invalid fork archive version")
    dist.mkdir(parents=True, exist_ok=True)
    stem = f"sweetpad-cli-{version}-{platform}"
    if platform == "windows-x64":
        archive = dist / f"{stem}.zip"
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as output:
            output.write(binary, "sweetpad.exe")
    else:
        archive = dist / f"{stem}.tar.gz"
        with tarfile.open(archive, "w:gz") as output:
            output.add(binary, arcname="sweetpad")
    return archive


def formula(version, repository, dist, template):
    if not re.fullmatch(r"\d+\.\d+\.\d+-nailington\.\d+", version):
        raise ValueError("Homebrew requires a tagged fork release version")
    if repository.lower() != "nailington/sweetpad":
        raise ValueError("This release configuration belongs to Nailington/sweetpad")
    values = {"VERSION": version}
    for key, platform in (("MACOS", "macos-universal"), ("LINUX", "linux-x64")):
        name = f"sweetpad-cli-{version}-{platform}.tar.gz"
        values[f"{key}_URL"] = f"https://github.com/{repository}/releases/download/cli-v{version}/{name}"
        values[f"{key}_SHA256"] = sha256(dist / name)
    result = template.read_text()
    for key, value in values.items():
        result = result.replace("{{" + key + "}}", value)
    if "{{" in result:
        raise ValueError("Unresolved Homebrew template variable")
    return result


def checksums(dist):
    assets = sorted(p for p in dist.iterdir() if p.suffix in (".gz", ".zip", ".vsix", ".rb"))
    if not assets:
        raise ValueError("No release assets found")
    (dist / "SHA256SUMS").write_text("".join(f"{sha256(p)}  {p.name}\n" for p in assets))


def skills_archive(version, root, dist):
    if not re.fullmatch(r"\d+\.\d+\.\d+-nailington(?:\.\d+)?(?:-dryrun\.(?:\d+|local))?", version):
        raise ValueError("Invalid fork skills archive version")
    skills = root / "skills"
    if not list(skills.glob("*/SKILL.md")):
        raise ValueError("No skills/*/SKILL.md files found")
    # Package the whole skills tree, including any future scripts/references/assets.
    # Fail on symlinks so the release cannot depend on files outside that tree.
    paths = sorted(skills.rglob("*"))
    for path in paths:
        if path.is_symlink():
            raise ValueError(f"Skill symlinks are not supported: {path}")
    dist.mkdir(parents=True, exist_ok=True)
    archive = dist / f"sweetpad-agent-skills-{version}.zip"
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as output:
        for path in paths:
            if path.is_file():
                output.write(path, path.relative_to(root).as_posix())
        output.writestr("INSTALL.md", """# SweetPad agent skills (Nailington)

These skills accompany the SweetPad fork release containing this archive.
Install that release's CLI first. Extract this ZIP; each skills/<name>/ directory
contains a SKILL.md plus any supporting files. Copy the complete skill directory
into your agent's skill directory (for example .agents/skills/ or .claude/skills/
in a project). Consult your agent's settings for global installation locations.

To install from the extracted directory using the skills installer instead:

    npx skills add ./skills

The CLI remains named sweetpad (sweetpad.exe on Windows). Windows/Linux Apple
builds need OpenSSH and a Mac with this fork's CLI and Xcode.
""")
    return archive


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    meta = commands.add_parser("metadata")
    meta.add_argument("kind", choices=("cli", "extension"))
    archive = commands.add_parser("package")
    archive.add_argument("--binary", type=Path, required=True)
    archive.add_argument("--platform", choices=("macos-universal", "linux-x64", "windows-x64"), required=True)
    archive.add_argument("--version", required=True)
    archive.add_argument("--dist", type=Path, default=Path("dist"))
    brew = commands.add_parser("formula")
    brew.add_argument("--version", required=True)
    brew.add_argument("--repository", required=True)
    brew.add_argument("--dist", type=Path, default=Path("dist"))
    sums = commands.add_parser("checksums")
    sums.add_argument("--dist", type=Path, default=Path("dist"))
    skills = commands.add_parser("skills")
    skills.add_argument("--version", required=True)
    skills.add_argument("--dist", type=Path, default=Path("dist"))
    notes = commands.add_parser("extension-notes")
    notes.add_argument("--version", required=True)
    args = parser.parse_args()
    if args.command == "metadata":
        values = metadata(args.kind)
        if output := os.environ.get("GITHUB_OUTPUT"):
            with open(output, "a") as file:
                file.writelines(f"{key}={value}\n" for key, value in values.items())
        print(json.dumps(values))
    elif args.command == "package":
        print(package(args.binary, args.platform, args.version, args.dist))
    elif args.command == "formula":
        template = Path(".github/homebrew/sweetpad-nailington.rb.tmpl")
        (args.dist / "sweetpad-nailington.rb").write_text(formula(args.version, args.repository, args.dist, template))
    elif args.command == "extension-notes":
        print(extension_notes(args.version))
    elif args.command == "skills":
        print(skills_archive(args.version, Path("."), args.dist))
    else:
        checksums(args.dist)


if __name__ == "__main__":
    main()
