import importlib.util
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
import zipfile

SPEC = importlib.util.spec_from_file_location('release', Path(__file__).with_name('release.py'))
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)
REPO = Path(__file__).resolve().parents[2]


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / 'Cargo.toml').write_text('[workspace.package]\nversion = "0.1.9-nailington.1"\n')
        (self.root / 'sweetpad-vscode').mkdir()
        self.pkg = self.root / 'sweetpad-vscode/package.json'
        self.pkg.write_text(json.dumps({'name': 'sweetpad-nailington', 'publisher': 'nailington', 'version': '0.2.16'}))
        self.dist = self.root / 'dist'
        self.dist.mkdir()

    def test_matching_cli_tag_publishes_with_fork_version(self):
        meta = release.metadata('cli', self.root, {'GITHUB_REF': 'refs/tags/cli-v0.1.9-nailington.1', 'GITHUB_EVENT_NAME': 'push'})
        self.assertEqual(meta['publish'], 'true')
        self.assertEqual(meta['asset_version'], '0.1.9-nailington.1')

    def test_upstream_and_mismatching_tags_are_rejected(self):
        for kind, tag in [('cli', 'cli-v0.1.9'), ('cli', 'cli-v0.1.9-nailington.2'), ('extension', 'v0.2.16'), ('extension', 'v0.2.17-nailington')]:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                release.metadata(kind, self.root, {'GITHUB_REF': f'refs/tags/{tag}', 'GITHUB_EVENT_NAME': 'push'})

    def test_manual_dispatch_never_publishes_even_on_a_tag(self):
        for kind, tag in [('cli', 'cli-v0.1.9-nailington.1'), ('extension', 'v0.2.16-nailington')]:
            meta = release.metadata(kind, self.root, {'GITHUB_REF': f'refs/tags/{tag}', 'GITHUB_EVENT_NAME': 'workflow_dispatch', 'GITHUB_RUN_NUMBER': '42'})
            self.assertEqual(meta['publish'], 'false')
            self.assertTrue(meta['asset_version'].endswith('-dryrun.42'))

    def test_extension_identity_is_isolated_and_version_is_numeric(self):
        meta = release.metadata('extension', self.root, {'GITHUB_REF': 'refs/tags/v0.2.16-nailington', 'GITHUB_EVENT_NAME': 'push'})
        self.assertEqual(meta['version'], '0.2.16')
        self.assertEqual(meta['asset_version'], '0.2.16-nailington')
        self.assertEqual(meta['publish'], 'true')
        original = json.loads(self.pkg.read_text())
        for updates in [{'publisher': 'sweetpad'}, {'name': 'sweetpad'}, {'version': '0.2.16-nailington'}]:
            pkg = dict(original)
            pkg.update(updates)
            self.pkg.write_text(json.dumps(pkg))
            with self.assertRaises(ValueError):
                release.metadata('extension', self.root, {})

    def test_binary_archives_preserve_contents_and_unix_executable_mode(self):
        binary = self.root / 'binary'
        binary.write_bytes(b'fixture executable')
        binary.chmod(0o755)
        for platform in ['macos-universal', 'linux-x64', 'windows-x64']:
            archive = release.package(binary, platform, '0.1.9-nailington.1', self.dist)
            if platform == 'windows-x64':
                with zipfile.ZipFile(archive) as content:
                    self.assertEqual(content.namelist(), ['sweetpad.exe'])
                    self.assertEqual(content.read('sweetpad.exe'), binary.read_bytes())
            else:
                with tarfile.open(archive) as content:
                    self.assertEqual(content.getnames(), ['sweetpad'])
                    self.assertTrue(content.getmember('sweetpad').mode & 0o111)
                    self.assertEqual(content.extractfile('sweetpad').read(), binary.read_bytes())

    def test_formula_references_exact_uploaded_archives_and_digests(self):
        for platform in ['macos-universal', 'linux-x64']:
            (self.dist / f'sweetpad-cli-0.1.9-nailington.1-{platform}.tar.gz').write_bytes(platform.encode())
        result = release.formula('0.1.9-nailington.1', 'Nailington/sweetpad', self.dist, REPO / '.github/homebrew/sweetpad-nailington.rb.tmpl')
        for archive in self.dist.iterdir():
            self.assertIn(release.sha256(archive), result)
            self.assertIn(f'/Nailington/sweetpad/releases/download/cli-v0.1.9-nailington.1/{archive.name}', result)
        self.assertNotIn('{{', result)
        with self.assertRaises(ValueError):
            release.formula('0.1.9-nailington.1', 'sweetpad-dev/sweetpad', self.dist, REPO / '.github/homebrew/sweetpad-nailington.rb.tmpl')

    def test_skills_archive_includes_supporting_files_and_instructions(self):
        skill = self.root / 'skills/sweetpad'
        (skill / 'references').mkdir(parents=True)
        (skill / 'SKILL.md').write_text('skill fixture')
        (skill / 'references/remote.md').write_text('reference fixture')
        archive = release.skills_archive('0.1.9-nailington.1', self.root, self.dist)
        with zipfile.ZipFile(archive) as content:
            self.assertEqual(content.read('skills/sweetpad/SKILL.md'), b'skill fixture')
            self.assertEqual(content.read('skills/sweetpad/references/remote.md'), b'reference fixture')
            self.assertIn(b'npx skills add ./skills', content.read('INSTALL.md'))
        release.checksums(self.dist)
        self.assertIn(f'{release.sha256(archive)}  {archive.name}', (self.dist / 'SHA256SUMS').read_text())

    def test_empty_skills_and_external_symlinks_fail(self):
        with self.assertRaises(ValueError):
            release.skills_archive('0.2.16-nailington', self.root, self.dist)
        skill = self.root / 'skills/sweetpad'
        skill.mkdir(parents=True)
        (skill / 'SKILL.md').write_text('fixture')
        (skill / 'outside').symlink_to(self.pkg)
        with self.assertRaises(ValueError):
            release.skills_archive('0.2.16-nailington', self.root, self.dist)

    def test_checksums_cover_all_download_types_but_exclude_notes(self):
        for name in ['cli.tar.gz', 'cli.zip', 'extension.vsix', 'formula.rb', 'notes.md']:
            (self.dist / name).write_text(name)
        release.checksums(self.dist)
        sums = (self.dist / 'SHA256SUMS').read_text()
        self.assertEqual(len(sums.splitlines()), 4)
        self.assertNotIn('notes.md', sums)
        before = sums
        release.checksums(self.dist)
        self.assertEqual((self.dist / 'SHA256SUMS').read_text(), before)

    def test_extension_notes_include_only_the_requested_release(self):
        (self.root / 'sweetpad-vscode/CHANGELOG.md').write_text('# Changes\n\n## [0.2.19]\n\n- Newer fix\n\n## [0.2.18] - date\n\n- This release\n\n## [0.2.17]\n\n- Older fix\n')
        self.assertEqual(release.extension_notes('0.2.18', self.root), '- This release')
        with self.assertRaises(ValueError):
            release.extension_notes('0.2.20', self.root)


if __name__ == '__main__':
    unittest.main()
