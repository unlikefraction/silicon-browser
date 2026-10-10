#!/usr/bin/env python3
"""Offline installer contracts; mock Apps never downloads or publishes a release."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SOURCE = Path(__file__).with_name('install.sh').read_text()
APPS = '''#!/bin/sh
set -eu
printf '%s\\n' "$@" >> "$TEST_COMMANDS"
[ "${TEST_INSTALL_FAIL:-0}" = 0 ] || exit 1
'''


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='browser-installer-test-')
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.home = self.root / 'home with spaces'
        self.home.mkdir()
        self.mock = self.root / 'mock'
        self.mock.mkdir()
        self.temporary = self.root / 'temporary with spaces'
        self.temporary.mkdir()
        self.env = {key: value for key, value in os.environ.items()
                    if not key.startswith(('APPS_', 'SILICON_'))}
        self.env.update(HOME=str(self.home), TMPDIR=str(self.temporary),
                        PATH=str(self.mock) + ':/usr/bin:/bin',
                        TEST_COMMANDS=str(self.root / 'commands'), TEST_URL=str(self.root / 'url'))
        self.script = self.root / 'install.sh'
        self.script.write_text(SOURCE)
        self.apps_template = self.root / 'silicon-apps'
        self.apps_template.write_text(APPS)
        self.apps_template.chmod(0o755)
        self.bootstrap = self.root / 'bootstrap.sh'
        self.bootstrap.write_text('''#!/bin/sh
set -eu
[ "${TEST_BOOTSTRAP_FAIL:-0}" = 0 ] || exit 1
printf '%s\\n' "$@" > "$TEST_BOOTSTRAP_ARGS"
mkdir -p "${SILICON_HOME:-$HOME}/.apps/bin"
cp "$TEST_APPS_TEMPLATE" "${SILICON_HOME:-$HOME}/.apps/bin/silicon-apps"
''')
        self.env.update(TEST_BOOTSTRAP=str(self.bootstrap),
                        TEST_BOOTSTRAP_ARGS=str(self.root / 'bootstrap-args'),
                        TEST_APPS_TEMPLATE=str(self.apps_template))
        self.stub('curl', '''
[ "${TEST_NETWORK_FAIL:-0}" = 0 ] || exit 22
while [ "$#" -gt 0 ]; do
  case "$1" in
    https:*) printf '%s\\n' "$1" > "$TEST_URL" ;;
    --output) shift; destination=$1 ;;
  esac
  shift
done
cp "$TEST_BOOTSTRAP" "$destination"
''')

    def stub(self, name, body):
        path = self.mock / name
        path.write_text('#!/bin/sh\nset -eu\n' + body + '\n')
        path.chmod(0o755)

    def run_installer(self, *arguments):
        return subprocess.run(['sh', str(self.script), *arguments], env=self.env,
                              stdin=subprocess.DEVNULL, capture_output=True, text=True)

    def assert_installed_without_auth(self, result):
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.root / 'commands').read_text(),
                         '--server\nhttps://apps.teamofsilicons.com\ninstall\nbrowser\n')
        self.assertNotIn('login\n', (self.root / 'commands').read_text())
        self.assertFalse((self.home / '.local/bin/browser').exists())
        self.assertEqual(list(self.temporary.glob('browser-install.*')), [])

    def test_bootstrap_uses_official_apps_and_install_without_terminal(self):
        self.assert_installed_without_auth(self.run_installer())
        self.assertEqual((self.root / 'url').read_text(),
                         'https://apps.teamofsilicons.com/install.sh\n')
        self.assertEqual((self.root / 'bootstrap-args').read_text(),
                         '--server\nhttps://apps.teamofsilicons.com\n')
        self.assertNotIn('github.com/unlikefraction', SOURCE)

    def test_existing_apps_skips_download_and_keeps_no_setup_compatibility(self):
        self.stub('silicon-apps', APPS.split('set -eu\n', 1)[1])
        self.assert_installed_without_auth(self.run_installer('--no-setup'))
        self.assertFalse((self.root / 'url').exists())
        self.assertFalse((self.root / 'bootstrap-args').exists())

    def test_home_apps_path_and_custom_silicon_home(self):
        custom = self.root / 'silicon home with spaces'
        custom.mkdir()
        self.env['SILICON_HOME'] = str(custom)
        self.assert_installed_without_auth(self.run_installer())
        self.assertTrue((custom / '.apps/bin/silicon-apps').exists())
        (self.root / 'commands').unlink()
        (self.root / 'url').unlink()
        self.assert_installed_without_auth(self.run_installer())
        self.assertFalse((self.root / 'url').exists())

    def test_failures_never_install_legacy_artifact(self):
        legacy = self.home / '.local/bin/browser'
        legacy.parent.mkdir(parents=True)
        legacy.write_text('existing installation')
        for failure in ['TEST_NETWORK_FAIL', 'TEST_BOOTSTRAP_FAIL', 'TEST_INSTALL_FAIL']:
            with self.subTest(failure=failure):
                self.env[failure] = '1'
                result = self.run_installer()
                del self.env[failure]
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(legacy.read_text(), 'existing installation')
                self.assertEqual(list(self.temporary.glob('browser-install.*')), [])
        self.assertNotIn('managed-v0.3.1', SOURCE)

    def test_help_and_retired_arguments_do_not_download(self):
        result = self.run_installer('--help')
        self.assertEqual(result.returncode, 0)
        self.assertIn('Silicon Apps', result.stdout)
        result = self.run_installer('--org', 'example-org')
        self.assertEqual(result.returncode, 2)
        self.assertFalse((self.root / 'url').exists())


if __name__ == '__main__':
    unittest.main()
