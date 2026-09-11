#!/usr/bin/env python3
"""Offline installer contracts; fixture executables never contact IAM or providers."""
import hashlib
import io
import os
from pathlib import Path
import pty
import re
import shlex
import subprocess
import tarfile
import tempfile
import unittest


SOURCE = Path(__file__).with_name('install.sh').read_text()


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='sb-installer-test-')
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.home = self.root / 'home with spaces'
        self.home.mkdir()
        self.mock = self.root / 'mock'
        self.mock.mkdir()
        self.env = dict(os.environ, HOME=str(self.home), ZDOTDIR=str(self.home),
                        SHELL='/bin/zsh', PATH=str(self.mock) + ':' + os.environ['PATH'])
        self.binary = self.home / '.local/bin/sb'
        self.stub('uname', 'case "$1" in -s) echo "$TEST_OS";; -m) echo "$TEST_ARCH";; esac')
        self.stub('getconf', 'echo "${TEST_LIBC:-glibc 2.34}"')
        self.stub('curl', '''
[ "${TEST_NETWORK_FAIL:-0}" = 0 ] || exit 22
while [ "$#" -gt 0 ]; do
  case "$1" in
    https:*) printf '%s\\n' "$1" > "$TEST_URL" ;;
    --output) shift; destination=$1 ;;
  esac
  shift
done
cp "$TEST_ARCHIVE" "$destination"
if [ "${TEST_CORRUPT:-0}" = 1 ]; then printf bad >> "$destination"; fi
''')

    def stub(self, name, body):
        path = self.mock / name
        path.write_text('#!/bin/sh\nset -eu\n' + body + '\n')
        path.chmod(0o755)

    def prepare(self, system='Darwin', arch='arm64', target='aarch64-apple-darwin'):
        archive = self.root / 'fixture.tar.gz'
        payload = b'''#!/bin/sh
if [ "$1" = --version ]; then
  [ "${TEST_EXEC_FAIL:-0}" = 0 ] || exit 1
  echo 'sb 0.2.0'
elif [ "$1" = setup ]; then
  [ -t 0 ] || exit 42
  printf '%s\\n' "$@" > "$HOME/setup-arguments"
fi
'''
        with tarfile.open(archive, 'w:gz') as output:
            entry = tarfile.TarInfo(f'sb-v0.2.0-{target}/sb')
            entry.size = len(payload)
            entry.mode = 0o755
            output.addfile(entry, io.BytesIO(payload))
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        self.script = self.root / 'install.sh'
        self.script.write_text(re.sub(r'digest=[a-f0-9]{64}', 'digest=' + digest, SOURCE))
        self.env.update(TEST_OS=system, TEST_ARCH=arch, TEST_ARCHIVE=str(archive),
                        TEST_URL=str(self.root / 'url'))

    def run_installer(self, *arguments):
        return subprocess.run(['sh', str(self.script), *arguments], env=self.env,
                              stdin=subprocess.DEVNULL, capture_output=True, text=True,
                              start_new_session=True)

    def test_platforms_and_repeat_install(self):
        for system, arch, target in [
            ('Darwin', 'arm64', 'aarch64-apple-darwin'),
            ('Darwin', 'x86_64', 'x86_64-apple-darwin'),
            ('Linux', 'aarch64', 'aarch64-unknown-linux-gnu'),
            ('Linux', 'x86_64', 'x86_64-unknown-linux-gnu'),
        ]:
            with self.subTest(target=target):
                self.prepare(system, arch, target)
                result = self.run_installer('--no-setup')
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertTrue(os.access(self.binary, os.X_OK))
                self.assertIn(target + '.tar.gz', (self.root / 'url').read_text())
                self.assertEqual((self.home / '.zshrc').read_text().count('export PATH='), 1)
                self.assertEqual(list(self.binary.parent.glob('.sb-install.*')), [])
        command = '. "$HOME/.profile"; command -v sb'
        result = subprocess.run(['sh', '-c', command], env=self.env, capture_output=True, text=True)
        self.assertEqual(result.stdout.strip(), str(self.binary))

    def test_failures_preserve_existing_binary_and_profiles(self):
        self.prepare()
        self.binary.parent.mkdir(parents=True)
        self.binary.write_text('existing installation')
        for failure in ['TEST_NETWORK_FAIL', 'TEST_CORRUPT', 'TEST_EXEC_FAIL']:
            with self.subTest(failure=failure):
                self.env[failure] = '1'
                result = self.run_installer('--no-setup')
                del self.env[failure]
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(self.binary.read_text(), 'existing installation')
                self.assertFalse((self.home / '.zshrc').exists())
                self.assertEqual(list(self.binary.parent.glob('.sb-install.*')), [])

    def test_unsupported_platforms_fail_before_download(self):
        for system, arch, libc in [('Linux', 'x86_64', 'musl'),
                                   ('Linux', 'x86_64', 'glibc 2.31'),
                                   ('Linux', 'armv7l', 'glibc 2.34'),
                                   ('FreeBSD', 'x86_64', 'glibc 2.34')]:
            with self.subTest(system=system, arch=arch, libc=libc):
                self.prepare(system, arch)
                self.env['TEST_LIBC'] = libc
                result = self.run_installer('--no-setup')
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse((self.root / 'url').exists())

    def test_no_terminal_reports_unfinished_setup(self):
        self.prepare()
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(self.binary.exists())
        self.assertIn('CLI installed. Open a terminal', result.stderr)

    def test_piped_installer_restores_terminal_for_setup(self):
        self.prepare()
        child, terminal = pty.fork()
        if child == 0:
            command = 'cat ' + shlex.quote(str(self.script)) + ' | sh -s -- --org example-org'
            os.execve('/bin/sh', ['sh', '-c', command], self.env)
        output = bytearray()
        try:
            while True:
                try:
                    chunk = os.read(terminal, 4096)
                except OSError:
                    break
                if not chunk:
                    break
                output.extend(chunk)
        finally:
            os.close(terminal)
            _, status = os.waitpid(child, 0)
        self.assertEqual(os.waitstatus_to_exitcode(status), 0, output.decode())
        self.assertEqual((self.home / 'setup-arguments').read_text(), 'setup\n--org\nexample-org\n')


if __name__ == '__main__':
    unittest.main()
