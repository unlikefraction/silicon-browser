#!/usr/bin/env python3
"""Back up the configured Accounts database to the private service bucket."""
from contextlib import closing
import datetime
import os
from pathlib import Path
import re
import sqlite3
import subprocess
import tempfile
from urllib.parse import parse_qsl, unquote


def database_source():
    value = os.environ.get('SB_DATABASE_URL', 'sqlite://silicon-browser-accounts.db?mode=rwc')
    if not value.startswith('sqlite:'):
        raise ValueError('SB_DATABASE_URL must be a file SQLite URL')
    filename, _, query = value.removeprefix('sqlite://').removeprefix('sqlite:').partition('?')
    filename = unquote(filename, errors='strict')
    allowed = {'mode': {'ro', 'rw', 'rwc'}, 'cache': {'private', 'shared'},
               'immutable': {'true', 'false', '1', '0'}}
    if (not filename or filename == ':memory:' or filename.startswith('file:') or '\x00' in filename
            or any(value not in allowed.get(key, ()) for key, value in parse_qsl(query, keep_blank_values=True))):
        raise ValueError('SB_DATABASE_URL must name a file SQLite database without a custom VFS')
    return Path(filename)


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
    return [(destination, '.db')]


def main():
    os.umask(0o077)
    source = database_source()
    bucket = Path('/etc/silicon-browser/artifact-bucket').read_text().strip()
    region_file = Path('/etc/silicon-browser/artifact-region')
    region = region_file.read_text().strip() if region_file.is_file() else 'us-east-1'
    if not re.fullmatch(r'[a-z0-9-]+', region):
        raise SystemExit('Invalid artifact bucket region')
    timestamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y/%m/%d/%Y%m%dT%H%M%SZ')
    with tempfile.TemporaryDirectory(prefix='browser-backup-') as directory:
        files = snapshots(source, Path(directory))
        for destination, suffix in files:
            subprocess.run(['aws', 's3', 'cp', str(destination), f's3://{bucket}/backups/{timestamp}{suffix}',
                            '--region', region, '--only-show-errors', '--sse', 'AES256'], check=True)
    print(f'{len(files)} SQLite backups uploaded after integrity verification')


if __name__ == '__main__':
    main()
