#!/usr/bin/env python3
"""Rehearse or apply the typed public-ID namespace upgrade without starting APIs or workers.

Apply requires a stopped Browser service. A failed offline conversion restores
all databases from the new snapshot while the service remains stopped. Once the
new daemon has run, do not restore a pre-upgrade database without reconciling any
new external token rotations or deliveries.
"""
import argparse
from contextlib import closing
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import sqlite3
import subprocess

from backup import snapshots

ROOT = Path('/var/lib/silicon-browser/browser.db')
ENVIRONMENT = Path('/etc/silicon-browser/runtime.env')
MUTABLE = {
    'identity_projection': {'principal_id', 'public_id', 'updated_at'},
    'profiles': {'owner_id', 'access_json'},
    'sessions': {'started_by', 'delivery_principal_id', 'delivery_membership_id'},
    'session_participants': {'actor_id'},
    'commands': {'actor_id', 'command_enc', 'delivery_actor_id'},
    'command_reports': {'principal_id', 'actor_id'},
    'recordings': {'owner_id'},
    'discovery_log': {'actor_id'},
    'delivery_credentials': {'actor_id', 'principal_id', 'membership_id', 'encrypted_payload'},
}


def service_stopped():
    return subprocess.run(['systemctl', 'is-active', '--quiet', 'silicon-browser.service']).returncode != 0


def environment():
    result = os.environ.copy()
    for line in ENVIRONMENT.read_text().splitlines():
        if not line.strip() or line.lstrip().startswith('#'):
            continue
        fields = shlex.split(line)
        if len(fields) != 1 or '=' not in fields[0]:
            raise ValueError('unsupported runtime environment line')
        key, value = fields[0].split('=', 1)
        result[key] = value
    return result


def retained_rows(database):
    with closing(sqlite3.connect(database.resolve().as_uri() + '?mode=ro', uri=True)) as db:
        if db.execute('PRAGMA integrity_check').fetchone() != ('ok',):
            raise ValueError('SQLite integrity check failed')
        result = {}
        for (table,) in db.execute("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"):
            if table.startswith('sqlite_') or table in ('_sqlx_migrations', 'public_identifier_schema'):
                continue
            columns = [row[1] for row in db.execute('PRAGMA table_info("' + table + '")')
                       if row[1] not in MUTABLE.get(table, set())]
            names = ','.join('"' + name + '"' for name in columns)
            rows = db.execute('SELECT ' + names + ' FROM "' + table + '"').fetchall()
            # Preserve every non-identity ledger value, including family IDs,
            # retry digests, mutation keys, leases and completed receipts.
            result[table] = sorted(rows, key=repr)
        return result


def convert(binary, database, expected, runtime, mapping):
    env = runtime.copy()
    env['SB_DATABASE_URL'] = 'sqlite://' + str(database.resolve()) + '?mode=rw'
    subprocess.run([str(binary), '--migrate-public-identifiers', str(mapping)], env=env, check=True)
    if retained_rows(database) != expected:
        raise ValueError('conversion changed a retained ledger value: ' + database.name)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['rehearse', 'apply'])
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--expected-binary-sha256', required=True)
    parser.add_argument('--mapping', required=True, type=Path, help='Reviewed production IAM mapping for exactly the local projections')
    parser.add_argument('--snapshot-directory', required=True, type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    binary = args.binary.resolve(strict=True)
    if hashlib.sha256(binary.read_bytes()).hexdigest() != args.expected_binary_sha256:
        raise SystemExit('Binary does not match the reviewed release artifact')
    mapping = args.mapping.resolve(strict=True)
    if not isinstance(json.loads(mapping.read_text()), list):
        raise SystemExit('Mapping must be a JSON array')
    with open('/var/lock/silicon-browser-deploy.lock', 'w') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if args.action == 'apply' and not service_stopped():
            raise SystemExit('stop silicon-browser.service before applying the identity upgrade')
        directory = args.snapshot_directory.absolute()
        directory.mkdir(mode=0o700, parents=False, exist_ok=False)
        backups = directory / 'backups'
        backups.mkdir(mode=0o700)
        files = snapshots(ROOT, backups)
        # This entry point is production-only. Test stores require their own
        # explicit IAM-world mapping and --scope-key offline command.
        if len(files) != 1 or files[0][1] != '.db':
            raise SystemExit('Testing databases require separate scoped mappings; refusing the production map')
        with closing(sqlite3.connect(backups.joinpath(files[0][0].name))) as db:
            if db.execute('SELECT count(*) FROM testing_environments').fetchone()[0]:
                raise SystemExit('Testing registrations must be handled with separate scoped mappings')
        saved_mapping = directory / 'mapping.json'
        shutil.copyfile(mapping, saved_mapping)
        shutil.copyfile(ENVIRONMENT, directory / 'runtime.env')
        runtime = environment()
        for key, old, new in [('IAM_APP_ID', 'tos>browser', 'browser'),
                              ('BRIEFCASE_APP_ID', 'tos>briefcase', 'briefcase')]:
            if runtime.get(key) == old:
                runtime[key] = new
            elif runtime.get(key) not in (None, new):
                raise SystemExit('Unexpected typed application configuration: ' + key)
        manifest = {
            'action': args.action,
            'previous_release': str(Path('/opt/silicon-browser/current').resolve()),
            'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
            'mapping_sha256': hashlib.sha256(saved_mapping.read_bytes()).hexdigest(),
            'runtime_sha256': hashlib.sha256(ENVIRONMENT.read_bytes()).hexdigest(),
            'databases': [],
        }
        pairs = []
        for snapshot, suffix in files:
            live = ROOT if suffix == '.db' else ROOT.with_name(ROOT.name + '.testing') / snapshot.name
            expected = retained_rows(snapshot)
            rehearsal = directory / snapshot.name
            shutil.copy2(snapshot, rehearsal)
            convert(binary, rehearsal, expected, runtime, saved_mapping)
            pairs.append((snapshot, live, expected))
            manifest['databases'].append({'source': str(live), 'backup': str(snapshot),
                'sha256': hashlib.sha256(snapshot.read_bytes()).hexdigest(),
                'row_counts': {table: len(rows) for table, rows in expected.items()}})
        manifest['rehearsal_verified'] = True
        (directory / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
        if args.action == 'apply':
            try:
                if not service_stopped():
                    raise RuntimeError('Browser started during the offline conversion')
                for _, live, expected in pairs:
                    convert(binary, live, expected, runtime, saved_mapping)
                if hashlib.sha256(ENVIRONMENT.read_bytes()).hexdigest() != manifest['runtime_sha256']:
                    raise RuntimeError('runtime configuration changed during conversion')
            except Exception:
                if not service_stopped():
                    raise RuntimeError('service is running; retain converted data for operator recovery') from None
                for snapshot, live, _ in pairs:
                    metadata = live.stat()
                    for suffix in ('-wal', '-shm'):
                        live.with_name(live.name + suffix).unlink(missing_ok=True)
                    shutil.copyfile(snapshot, live)
                    os.chown(live, metadata.st_uid, metadata.st_gid)
                    os.chmod(live, metadata.st_mode & 0o777)
                raise
            manifest['applied'] = True
            (directory / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
        print(json.dumps({'action': args.action, 'databases': len(pairs), 'verified': True,
                          'snapshot_directory': str(directory)}))


if __name__ == '__main__':
    main()
