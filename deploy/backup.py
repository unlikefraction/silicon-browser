#!/usr/bin/env python3
"""Create and check an online SQLite backup, then upload to the private service bucket."""
from contextlib import closing
import datetime
import os
from pathlib import Path
import re
import sqlite3
import subprocess
import tempfile

os.umask(0o077)
source = Path('/var/lib/silicon-browser/browser.db')
bucket = Path('/etc/silicon-browser/artifact-bucket').read_text().strip()
region_file = Path('/etc/silicon-browser/artifact-region')
region = region_file.read_text().strip() if region_file.is_file() else 'us-east-1'
if not re.fullmatch(r'[a-z0-9-]+', region):
    raise SystemExit('Invalid artifact bucket region')
if not source.is_file():
    raise SystemExit('Database does not exist; refusing to create an empty backup')
with tempfile.TemporaryDirectory(prefix='browser-backup-') as directory:
    destination = Path(directory) / 'browser.db'
    with closing(sqlite3.connect(f'file:{source}?mode=ro', uri=True)) as live:
        with closing(sqlite3.connect(destination)) as backup:
            live.backup(backup, pages=128, sleep=0.1)
    with closing(sqlite3.connect(f'file:{destination}?mode=ro', uri=True)) as restored:
        if restored.execute('PRAGMA integrity_check').fetchone() != ('ok',):
            raise SystemExit('Backup integrity check failed')
    timestamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y/%m/%d/%Y%m%dT%H%M%SZ')
    subprocess.run(['aws', 's3', 'cp', str(destination), f's3://{bucket}/backups/{timestamp}.db',
                    '--region', region, '--only-show-errors', '--sse', 'AES256'], check=True)
print('SQLite backup uploaded after integrity verification')
