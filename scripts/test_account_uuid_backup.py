#!/usr/bin/env python3
"""Exercise the guarded cutover wrapper with a real WAL-mode SQLite backup."""
from contextlib import closing
import importlib.util
import os
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'deploy'))
import migrate_account_uuids as migration


class CutoverBackupTests(unittest.TestCase):
    def test_backup_precedes_apply_and_retains_wal_state_and_key(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source, binary, mapping = root / 'accounts.db', root / 'backend', root / 'mapping.csv'
            binary.write_bytes(b'candidate')
            mapping.write_text('old_uuid,new_uuid,kind\n')
            calls = []
            with closing(sqlite3.connect(source)) as database:
                database.execute('PRAGMA journal_mode=WAL')
                database.execute('CREATE TABLE marker (value TEXT)')
                database.execute("INSERT INTO marker VALUES ('retained')")
                database.commit()
                def execute(_binary, _map, apply=False):
                    if apply:
                        with closing(sqlite3.connect(root / 'backup/accounts.db')) as saved:
                            self.assertEqual(saved.execute('SELECT value FROM marker').fetchone(), ('retained',))
                        self.assertEqual((root / 'backup/encryption-key').read_text(), 'ab' * 32 + '\n')
                    calls.append(apply)
                with patch.dict(os.environ, {'SB_DATABASE_URL': f'sqlite://{source}', 'SB_ENCRYPTION_KEY': 'ab' * 32}), patch.object(migration, 'execute', execute), patch.object(migration.subprocess, 'run') as run:
                    run.return_value.stdout = 'inactive\n'
                    migration.migrate(binary, mapping, True, True, root / 'backup')
                    self.assertEqual(calls, [False, True, True])
                    self.assertEqual((root / 'backup/encryption-key').stat().st_mode & 0o777, 0o600)
                    with self.assertRaises(FileExistsError):
                        migration.migrate(binary, mapping, True, True, root / 'backup')
                    run.return_value.stdout = 'active\n'
                    with self.assertRaises(ValueError):
                        migration.migrate(binary, mapping, True, True, root / 'another')
                    self.assertFalse((root / 'another').exists())


if __name__ == '__main__':
    unittest.main()
