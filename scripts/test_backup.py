#!/usr/bin/env python3
"""Offline contracts for deployment backup coverage and fail-closed recovery."""
from contextlib import closing
import importlib.util
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('backup', Path(__file__).resolve().parents[1] / 'deploy/backup.py')
backup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(backup)


class BackupTests(unittest.TestCase):
    def test_backup_selects_the_configured_database_instead_of_the_old_default(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            active = root / 'accounts database.db'
            for source, marker in [(root / 'browser.db', 'old-iam'), (active, 'accounts')]:
                with closing(sqlite3.connect(source)) as database:
                    database.execute('CREATE TABLE sessions (id TEXT)')
                    database.execute('INSERT INTO sessions VALUES (?)', (marker,))
                    database.commit()
            output = root / 'output'
            output.mkdir()
            with patch.dict(backup.os.environ, {'SB_DATABASE_URL': 'sqlite://' + str(active).replace(' ', '%20') + '?mode=rwc'}):
                files = backup.snapshots(backup.database_source(), output)
            with closing(sqlite3.connect(files[0][0])) as restored:
                self.assertEqual(restored.execute('SELECT id FROM sessions').fetchall(), [('accounts',)])

    def test_database_source_uses_the_backend_default_and_file_sqlite_forms(self):
        with patch.dict(backup.os.environ, {}, clear=True):
            self.assertEqual(backup.database_source(), Path('silicon-browser-accounts.db'))
        for url, expected in [
            ('sqlite://relative.db?mode=rwc', 'relative.db'),
            ('sqlite:relative.db?mode=rw&cache=shared', 'relative.db'),
            ('sqlite:///var/lib/silicon-browser/active.db?mode=ro', '/var/lib/silicon-browser/active.db'),
            ('sqlite:/var/lib/silicon-browser/active.db?immutable=1', '/var/lib/silicon-browser/active.db'),
        ]:
            with self.subTest(url=url), patch.dict(backup.os.environ, {'SB_DATABASE_URL': url}):
                self.assertEqual(backup.database_source(), Path(expected))

    def test_memory_and_non_file_database_sources_are_refused(self):
        for url in ['', 'postgres://db/browser', 'https://db/browser', 'sqlite::memory:',
                    'sqlite://:memory:', 'sqlite://%3Amemory%3A', 'sqlite://',
                    'sqlite://active.db?mode=memory', 'sqlite://active.db?mode=rw&mode=memory',
                    'sqlite://file:active.db?mode=rw', 'sqlite://active.db?vfs=custom',
                    'sqlite://active%00.db']:
            with self.subTest(url=url), patch.dict(backup.os.environ, {'SB_DATABASE_URL': url}):
                with self.assertRaises(ValueError):
                    backup.database_source()

    def test_production_and_registered_testing_databases_are_recoverable(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / 'browser.db'
            testing = root / 'browser.db.testing'
            testing.mkdir()
            output = root / 'output'
            output.mkdir()
            namespace = '00000000-0000-4000-8000-000000000001-' + 'a' * 64
            with closing(sqlite3.connect(source)) as database:
                database.execute('CREATE TABLE testing_environments (namespace TEXT)')
                database.execute('INSERT INTO testing_environments VALUES (?)', (namespace,))
                database.commit()
            test_database = testing / (namespace + '.db')
            with closing(sqlite3.connect(test_database)) as database:
                database.execute('PRAGMA journal_mode=WAL')
                database.execute('CREATE TABLE sessions (id TEXT)')
                database.execute("INSERT INTO sessions VALUES ('test-session')")
                database.commit()
                files = backup.snapshots(source, output)
            self.assertEqual([suffix for _, suffix in files], ['.db', '.testing/' + namespace + '.db'])
            with closing(sqlite3.connect(files[1][0])) as restored:
                self.assertEqual(restored.execute('SELECT id FROM sessions').fetchall(), [('test-session',)])
                self.assertEqual(restored.execute('PRAGMA integrity_check').fetchone(), ('ok',))
            test_database.unlink()
            with self.assertRaises(ValueError):
                backup.snapshots(source, output)
            test_database.symlink_to(source)
            with self.assertRaises(ValueError):
                backup.snapshots(source, output)

    def test_pre_testing_schema_remains_supported(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / 'production.db'
            with closing(sqlite3.connect(source)) as database:
                database.execute('CREATE TABLE sessions (id TEXT)')
            self.assertEqual(backup.snapshots(source, root), [(root / 'browser.db', '.db')])


if __name__ == '__main__':
    unittest.main()
