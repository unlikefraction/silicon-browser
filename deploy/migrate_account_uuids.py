#!/usr/bin/env python3
"""Offline Accounts UUID cutover with a private SQLite/key backup and exact replay.

Load the existing runtime environment and stop silicon-browser before --apply.
This never stops/restarts services and never contacts Accounts or providers.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

from backup import database_source, snapshot


def execute(binary, mapping, apply=False):
    command = [str(binary), '--migrate-account-uuids', str(mapping)]
    if apply:
        command.extend(['--apply', '--writers-stopped'])
    subprocess.run(command, check=True)


def migrate(binary, mapping, apply=False, writers_stopped=False, backup_dir=None):
    os.umask(0o077)
    binary, mapping = Path(binary).resolve(strict=True), Path(mapping).resolve(strict=True)
    source = database_source().resolve(strict=True)
    if apply:
        if not writers_stopped or backup_dir is None:
            raise ValueError('--apply requires --writers-stopped and a new --backup-dir')
        state = subprocess.run(['systemctl', 'show', '--property=ActiveState', '--value', 'silicon-browser'],
                               capture_output=True, text=True, check=True).stdout.strip()
        if state not in ('inactive', 'failed'):
            raise ValueError('silicon-browser must be stopped before backup and apply')
    # Reject missing maps, wrong keys, IAM stores and collisions before backing up.
    execute(binary, mapping)
    if not apply:
        return
    destination = Path(backup_dir)
    destination.mkdir(mode=0o700, parents=False, exist_ok=False)
    snapshot(source, destination / 'accounts.db')
    key = os.environ['SB_ENCRYPTION_KEY']
    if len(key) != 64 or any(character not in '0123456789abcdefABCDEF' for character in key):
        raise ValueError('SB_ENCRYPTION_KEY must be 64 hexadecimal characters')
    (destination / 'encryption-key').write_text(key + '\n')
    shutil.copyfile(mapping, destination / 'account-uuid-map.csv')
    manifest = {
        'database': str(source),
        'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
        'map_sha256': hashlib.sha256(mapping.read_bytes()).hexdigest(),
        'backup_sha256': hashlib.sha256((destination / 'accounts.db').read_bytes()).hexdigest(),
    }
    (destination / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    execute(binary, mapping, apply=True)
    execute(binary, mapping, apply=True)
    print(json.dumps({'backup': str(destination), 'map_sha256': manifest['map_sha256'], 'replay_verified': True}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--map', required=True, type=Path, dest='mapping')
    parser.add_argument('--apply', action='store_true')
    parser.add_argument('--writers-stopped', action='store_true')
    parser.add_argument('--backup-dir', type=Path)
    args = parser.parse_args()
    migrate(args.binary, args.mapping, args.apply, args.writers_stopped, args.backup_dir)


if __name__ == '__main__':
    main()
