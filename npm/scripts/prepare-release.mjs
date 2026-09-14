import { readFileSync, existsSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';

const root = new URL('..', import.meta.url).pathname;
const hashes = {
  'bin/sb-darwin-arm64': 'f36d9aaccfd4da296b87e6250485eccc7088c94fbe1ffbe769aa369c9eba94cd',
  'bin/sb-darwin-x64': '1d95b76f1ee94eabf0cfc47b132eb73611bb4e2ccc42307a3fb52411fed0483a',
  'bin/sb-linux-arm64': '3956f15bbebe6023759840dae382333b43acae96e3876d1a72a339aaf137526e',
  'bin/sb-linux-x64': '1e8cf30129212d5d333feaa91a2b2135ba1d4d309392b7bf2f310ae0b6c6c798'
};

for (const [file, expected] of Object.entries(hashes)) {
  const path = join(root, file);
  if (!existsSync(path)) throw new Error(`missing ${file}; download managed-v0.2.1 assets first`);
  const actual = createHash('sha256').update(readFileSync(path)).digest('hex');
  if (actual !== expected) throw new Error(`checksum mismatch: ${file}`);
}

const pack = spawnSync('npm', ['pack', '--dry-run', '--json'], { cwd: root, encoding: 'utf8' });
if (pack.status !== 0) throw new Error(pack.stderr || 'npm pack failed');
console.log(`release package ready: ${JSON.parse(pack.stdout)[0].filename}`);
