"""Offline operator invariants; the Rust migration suite tests cryptographic conversion."""
import importlib.util
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
from unittest.mock import patch

DEPLOY = Path(__file__).resolve().parents[1] / 'deploy'
sys.path.insert(0, str(DEPLOY))
spec = importlib.util.spec_from_file_location('public_id_cutover', DEPLOY / 'public-id-cutover.py')
operator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(operator)


class RetentionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.database = Path(self.temp.name) / 'browser.db'
        with sqlite3.connect(self.database) as db:
            db.executescript("""
              CREATE TABLE identity_projection(org_id TEXT,principal_id TEXT,public_id TEXT,kind TEXT,updated_at INT);
              INSERT INTO identity_projection VALUES('tos','saket','saket','carbon',1);
              CREATE TABLE commands(id TEXT,actor_id TEXT,command_enc TEXT,delivery_actor_id TEXT,artifact BLOB,receipt TEXT);
              INSERT INTO commands VALUES('command','saket','cipher-before','saket',X'010203','receipt-before');
            """)

    def test_identity_and_authenticated_ciphertext_can_change(self):
        before = operator.retained_rows(self.database)
        with sqlite3.connect(self.database) as db:
            db.executescript("""
              UPDATE identity_projection SET principal_id='c:saket',public_id='c:saket',updated_at=2;
              UPDATE commands SET actor_id='c:saket',delivery_actor_id='c:saket',command_enc='cipher-after';
            """)
        self.assertEqual(before, operator.retained_rows(self.database))

    def test_frozen_artifact_or_receipt_changes_are_detected(self):
        before = operator.retained_rows(self.database)
        for column in ('artifact', 'receipt'):
            with self.subTest(column=column):
                with sqlite3.connect(self.database) as db:
                    db.execute('UPDATE commands SET ' + column + "='changed'")
                self.assertNotEqual(before, operator.retained_rows(self.database))

    def test_deleted_identity_is_detected(self):
        before = operator.retained_rows(self.database)
        with sqlite3.connect(self.database) as db:
            db.execute('DELETE FROM identity_projection')
        self.assertNotEqual(before, operator.retained_rows(self.database))

    def test_offline_invocation_pins_selected_database_and_mapping(self):
        mapping = Path(self.temp.name) / 'mapping.json'
        before = operator.retained_rows(self.database)
        with patch.object(operator.subprocess, 'run') as run:
            operator.convert(Path('/candidate/browser'), self.database, before, {'SB_ENCRYPTION_KEY':'retained'}, mapping)
        args, kwargs = run.call_args
        self.assertEqual(args[0], ['/candidate/browser','--migrate-public-identifiers',str(mapping)])
        self.assertEqual(kwargs['env']['SB_DATABASE_URL'], 'sqlite://' + str(self.database) + '?mode=rw')
        self.assertEqual(kwargs['env']['SB_ENCRYPTION_KEY'], 'retained')
        self.assertTrue(kwargs['check'])


if __name__ == '__main__':
    unittest.main()
