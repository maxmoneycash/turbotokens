"""Exercise install.sh using a local release archive and a curl stub."""
import io
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest

INSTALLER = Path(__file__).resolve().parents[2] / 'install.sh'


class ShellInstallerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='turbotokens install ')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.destination = self.root / 'installed bin'
        self.commands = self.root / 'commands'
        self.commands.mkdir()
        archive = self.root / 'release.tar.gz'
        with tarfile.open(archive, 'w:gz') as output:
            content = b'#!/bin/sh\nprintf "turbotokens 1.1.0\\n"\n'
            member = tarfile.TarInfo('turbotokens')
            member.size, member.mode = len(content), 0o755
            output.addfile(member, io.BytesIO(content))
        curl = self.commands / 'curl'
        curl.write_text('#!/bin/sh\n[ "${TEST_DOWNLOAD_FAIL:-0}" = 1 ] && exit 22\nwhile [ "$1" != "-o" ]; do shift; done\ncp "$TEST_ARCHIVE" "$2"\n')
        curl.chmod(0o755)
        self.env = dict(os.environ, PATH=f'{self.commands}{os.pathsep}{os.environ["PATH"]}', TURBOTOKENS_INSTALL_DIR=str(self.destination), TURBOTOKENS_VERSION='v1.1.0', TEST_ARCHIVE=str(archive))

    def install(self, **env):
        return subprocess.run(['sh', str(INSTALLER)], env=dict(self.env, **env), capture_output=True, text=True)

    def test_install_and_reinstall_leave_an_executable(self):
        for _ in range(2):
            result = self.install()
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            binary = self.destination / 'turbotokens'
            self.assertFalse(binary.is_symlink())
            self.assertEqual(subprocess.check_output([str(binary), '--version'], text=True).strip(), 'turbotokens 1.1.0')

    def test_repairs_a_self_referencing_symlink(self):
        self.destination.mkdir()
        (self.destination / 'turbotokens').symlink_to('turbotokens')
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse((self.destination / 'turbotokens').is_symlink())

    def test_failed_download_preserves_existing_binary(self):
        self.assertEqual(self.install().returncode, 0)
        binary = self.destination / 'turbotokens'
        previous = binary.read_bytes()
        result = self.install(TEST_DOWNLOAD_FAIL='1')
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(binary.read_bytes(), previous)
        self.assertIn('download failed', result.stderr)


if __name__ == '__main__':
    unittest.main()
