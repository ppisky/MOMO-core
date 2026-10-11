"""Exercise Git identity and Cargo invalidation in isolated temporary checkouts."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parents[1] / 'crates/momo-core/build.rs'


class BuildMetadataTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='momo-build-metadata-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / 'repo'
        self.crate = self.root / 'crates/momo-core'
        (self.crate / 'src').mkdir(parents=True)
        shutil.copyfile(SOURCE, self.crate / 'build.rs')
        (self.crate / 'Cargo.toml').write_text('[package]\nname="build-probe"\nversion="0.1.0"\nedition="2024"\n', encoding='utf-8')
        (self.crate / 'src/main.rs').write_text('fn main() { println!("{}|{}|{}", env!("MOMO_GIT_REVISION"), env!("MOMO_GIT_TAG"), env!("MOMO_GIT_DIRTY")); }', encoding='utf-8')
        (self.root / '.gitignore').write_text('target/\n', encoding='utf-8')
        (self.root / 'README.md').write_text('Build probe\n', encoding='utf-8')

    def run_command(self, *args, cwd=None):
        return subprocess.check_output(args, cwd=cwd or self.root, text=True, stderr=subprocess.PIPE).strip()

    def git(self, *args):
        return self.run_command('git', *args)

    def init_git(self):
        self.git('init', '-q')
        self.git('config', 'user.name', 'ppisky')
        self.git('config', 'user.email', '280739163+ppisky@users.noreply.github.com')
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture')

    def probe(self, crate=None):
        return self.run_command('cargo', 'run', '--quiet', '--manifest-path', str((crate or self.crate) / 'Cargo.toml')).split('|')

    def test_commit_tag_dirty_and_incremental_rebuild(self):
        # Generate the lock before committing so the probe starts clean.
        self.run_command('cargo', 'generate-lockfile', '--manifest-path', str(self.crate / 'Cargo.toml'))
        self.init_git()
        first = self.git('rev-parse', 'HEAD')
        self.assertEqual(self.probe(), [first, '', 'false'])
        self.git('tag', 'v1.0.0-rc.6')
        self.assertEqual(self.probe(), [first, 'v1.0.0-rc.6', 'false'])
        (self.root / 'README.md').write_text('Changed\n', encoding='utf-8')
        self.assertEqual(self.probe(), [first, 'v1.0.0-rc.6', 'true'])
        self.git('add', 'README.md')
        self.git('commit', '-qm', 'next commit')
        second = self.git('rev-parse', 'HEAD')
        self.assertNotEqual(first, second)
        self.assertEqual(self.probe(), [second, '', 'false'])
        executable = self.crate / 'target/debug' / ('build-probe.exe' if os.name == 'nt' else 'build-probe')
        self.git('commit', '--allow-empty', '-qm', 'later commit')
        # Running the already-built binary must not query the current checkout.
        self.assertEqual(self.run_command(str(executable)).split('|'), [second, '', 'false'])
        self.assertEqual(self.probe()[0], self.git('rev-parse', 'HEAD'))

    def test_source_archive_has_no_git_identity(self):
        self.assertEqual(self.probe(), ['', '', ''])

    def test_linked_worktree(self):
        self.init_git()
        worktree = Path(self.temp.name) / 'worktree'
        self.git('worktree', 'add', '--detach', str(worktree), 'HEAD')
        self.assertEqual(self.probe(worktree / 'crates/momo-core'), [self.git('rev-parse', 'HEAD'), '', 'false'])


if __name__ == '__main__':
    unittest.main()
