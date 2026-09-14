#!/usr/bin/env python3
"""Back up production and its registered test databases to the private service bucket."""
from contextlib import closing
import datetime
import os
from pathlib import Path
import re
import sqlite3
import subprocess
import tempfile

def snapshot(source, destination):
    if source.is_symlink() or not source.is_file():
        raise ValueError('Database must be an existing regular file')
    with closing(sqlite3.connect(source.resolve().as_uri() + '?mode=ro', uri=True)) as live:
        with closing(sqlite3.connect(destination)) as backup:
            live.backup(backup, pages=128, sleep=0.1)
    with closing(sqlite3.connect(destination.resolve().as_uri() + '?mode=ro&immutable=1', uri=True)) as restored:
        if restored.execute('PRAGMA integrity_check').fetchone() != ('ok',):
            raise ValueError('Backup integrity check failed')


def snapshots(source, directory):
    destination = directory / 'browser.db'
    snapshot(source, destination)
    files = [(destination, '.db')]
    # Read the registry from the snapshot: enrollments after this point belong
    # to the next backup, and every included row already has a test database.
    with closing(sqlite3.connect(destination.resolve().as_uri() + '?mode=ro&immutable=1', uri=True)) as registry:
        if not registry.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='testing_environments'").fetchone():
            return files
        namespaces = [row[0] for row in registry.execute('SELECT namespace FROM testing_environments')]
    test_directory = source.with_name(source.name + '.testing')
    if namespaces and (test_directory.is_symlink() or not test_directory.is_dir()):
        raise ValueError('Test database directory must be a real directory')
    for namespace in namespaces:
        if not re.fullmatch(r'[0-9a-f-]{36}-[0-9a-f]{64}', namespace):
            raise ValueError('Invalid test namespace in database registry')
        name = namespace + '.db'
        destination = directory / name
        snapshot(test_directory / name, destination)
        files.append((destination, '.testing/' + name))
    return files


def main():
    os.umask(0o077)
    source = Path('/var/lib/silicon-browser/browser.db')
    bucket = Path('/etc/silicon-browser/artifact-bucket').read_text().strip()
    region_file = Path('/etc/silicon-browser/artifact-region')
    region = region_file.read_text().strip() if region_file.is_file() else 'us-east-1'
    if not re.fullmatch(r'[a-z0-9-]+', region):
        raise SystemExit('Invalid artifact bucket region')
    timestamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y/%m/%d/%Y%m%dT%H%M%SZ')
    with tempfile.TemporaryDirectory(prefix='browser-backup-') as directory:
        files = snapshots(source, Path(directory))
        # Publish the registry last so its presence marks a complete set.
        for destination, suffix in files[1:] + files[:1]:
            subprocess.run(['aws', 's3', 'cp', str(destination), f's3://{bucket}/backups/{timestamp}{suffix}',
                            '--region', region, '--only-show-errors', '--sse', 'AES256'], check=True)
    print(f'{len(files)} SQLite backups uploaded after integrity verification')


if __name__ == '__main__':
    main()
