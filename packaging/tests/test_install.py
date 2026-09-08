"""Exercise install.sh using a local release archive and a curl stub."""
import io
import hashlib
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
        self.archive = self.root / 'release.tar.gz'
        self.manifest = self.root / 'SHA256SUMS'
        self.write_release(b'#!/bin/sh\nprintf "turbotokens 1.1.0\\n"\n')
        curl = self.commands / 'curl'
        curl.write_text('''#!/bin/sh
[ "${TEST_DOWNLOAD_FAIL:-0}" = 1 ] && exit 22
source="$TEST_ARCHIVE"
while [ "$1" != "-o" ]; do
    case "$1" in
        */SHA256SUMS)
            [ "${TEST_MANIFEST_FAIL:-0}" = 1 ] && exit 22
            source="$TEST_MANIFEST"
            ;;
    esac
    shift
done
cp "$source" "$2"
''')
        curl.chmod(0o755)
        self.env = dict(os.environ, PATH=f'{self.commands}{os.pathsep}{os.environ["PATH"]}', TURBOTOKENS_INSTALL_DIR=str(self.destination), TURBOTOKENS_VERSION='v1.1.0', TEST_ARCHIVE=str(self.archive), TEST_MANIFEST=str(self.manifest))

    def write_release(self, content):
        with tarfile.open(self.archive, 'w:gz') as output:
            member = tarfile.TarInfo('turbotokens')
            member.size, member.mode = len(content), 0o755
            output.addfile(member, io.BytesIO(content))
        digest = hashlib.sha256(self.archive.read_bytes()).hexdigest()
        self.manifest.write_text(''.join(
            f'{digest}  turbotokens-{os_name}-{arch}.tar.gz\n'
            for os_name in ('macos', 'linux') for arch in ('arm64', 'x64')))

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

    def existing_binary(self):
        self.assertEqual(self.install().returncode, 0)
        binary = self.destination / 'turbotokens'
        return binary, binary.read_bytes()

    def test_failed_checksum_download_preserves_existing_binary(self):
        binary, previous = self.existing_binary()
        result = self.install(TEST_MANIFEST_FAIL='1')
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(binary.read_bytes(), previous)
        self.assertIn('checksum download failed', result.stderr)

    def test_corrupt_archive_is_rejected_before_extraction(self):
        binary, previous = self.existing_binary()
        self.archive.write_bytes(b'not a gzip archive')
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(binary.read_bytes(), previous)
        self.assertIn('checksum mismatch', result.stderr)
        self.assertNotIn('Unpacking', result.stdout)

    def test_missing_checksum_is_rejected(self):
        binary, previous = self.existing_binary()
        self.manifest.write_text('')
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(binary.read_bytes(), previous)
        self.assertIn('missing or invalid SHA-256', result.stderr)

    def test_duplicate_checksum_is_rejected(self):
        self.manifest.write_text(self.manifest.read_text() * 2)
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('missing or invalid SHA-256', result.stderr)
        self.assertFalse(self.destination.exists())

    def test_unusable_binary_preserves_existing_installation(self):
        binary, previous = self.existing_binary()
        self.write_release(b'#!/bin/sh\nexit 1\n')
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(binary.read_bytes(), previous)
        self.assertIn('downloaded binary could not run', result.stderr)

    def test_wrong_version_preserves_existing_installation(self):
        binary, previous = self.existing_binary()
        self.write_release(b'#!/bin/sh\nprintf "turbotokens 0.0.0\\n"\n')
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(binary.read_bytes(), previous)
        self.assertIn('version does not match', result.stderr)


if __name__ == '__main__':
    unittest.main()
