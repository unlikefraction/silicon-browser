import { readFileSync, existsSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';

const root = new URL('..', import.meta.url).pathname;
const hashes = {
  'bin/sb-darwin-arm64': 'a2f11031f84f21220fe0d5ee85b4aa4add8271a8b9ca43e36657374a9d212b80',
  'bin/sb-darwin-x64': '3c0d8c84b4b3a19e8c04db24eb7c8340d493915ae4739ed65ade3b862dea4814',
  'bin/sb-linux-arm64': '2729956f40b2ab47654e5279c607d13ee06224192419c55fe987203654249e6b',
  'bin/sb-linux-x64': '3acfb7b425a0f8a460e1283289595c7f118adfc03188184d9ffb4b76738b1d35'
};

for (const [file, expected] of Object.entries(hashes)) {
  const path = join(root, file);
  if (!existsSync(path)) throw new Error(`missing ${file}; download managed-v0.2.0 assets first`);
  const actual = createHash('sha256').update(readFileSync(path)).digest('hex');
  if (actual !== expected) throw new Error(`checksum mismatch: ${file}`);
}

const pack = spawnSync('npm', ['pack', '--dry-run', '--json'], { cwd: root, encoding: 'utf8' });
if (pack.status !== 0) throw new Error(pack.stderr || 'npm pack failed');
console.log(`release package ready: ${JSON.parse(pack.stdout)[0].filename}`);
