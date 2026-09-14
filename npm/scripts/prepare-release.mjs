import { readFileSync, existsSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';

const root = new URL('..', import.meta.url).pathname;
const hashes = {
  'bin/sb-darwin-arm64': 'a10d374e3a9e84eeee6c5935a0904d2859d396d2ba53362fc2b5f4d523bf1c5f',
  'bin/sb-darwin-x64': '4e179b3b28359512ada58f962373e1be729a0eabbb5f4f3649a4a0fbbcd0a15e',
  'bin/sb-linux-arm64': 'ec87b407775ffe12e9cbd0cf60bfe8049414181b7bfa011a4ffaa0ff22bf45b6',
  'bin/sb-linux-x64': '8a447b974f3b8e324c3a0b3a166199b7ab7a52b2084e97596971ff03f5b262f4'
};

for (const [file, expected] of Object.entries(hashes)) {
  const path = join(root, file);
  if (!existsSync(path)) throw new Error(`missing ${file}; download managed-v0.2.2 assets first`);
  const actual = createHash('sha256').update(readFileSync(path)).digest('hex');
  if (actual !== expected) throw new Error(`checksum mismatch: ${file}`);
}

const pack = spawnSync('npm', ['pack', '--dry-run', '--json'], { cwd: root, encoding: 'utf8' });
if (pack.status !== 0) throw new Error(pack.stderr || 'npm pack failed');
console.log(`release package ready: ${JSON.parse(pack.stdout)[0].filename}`);
