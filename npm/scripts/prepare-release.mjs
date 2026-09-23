import { readFileSync, writeFileSync, chmodSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const targets = resolve(process.argv[2] || join(root, '../target/honeycomb/targets'));
const version = readFileSync(join(root, '../Cargo.toml'), 'utf8').match(/\[workspace\.package\][\s\S]*?\nversion = "([^"]+)"/)[1];
const platforms = {
  'darwin-arm64': 'macos-aarch64',
  'darwin-x64': 'macos-x86_64',
  'linux-arm64': 'linux-aarch64',
  'linux-x64': 'linux-x86_64'
};

// Reuse the release workflow's native-tested payloads and their version/hash receipts.
const payloads = Object.entries(platforms).map(([platform, target]) => {
  const directory = join(targets, target);
  const receipt = JSON.parse(readFileSync(join(directory, 'build.json'), 'utf8'));
  const bytes = readFileSync(join(directory, 'browser'));
  if (receipt.version !== version) throw new Error(`wrong CLI version: ${target}`);
  if (createHash('sha256').update(bytes).digest('hex') !== receipt.sha256.browser) throw new Error(`checksum mismatch: ${target}`);
  return [join(root, `bin/browser-${platform}`), bytes];
});
for (const [path, bytes] of payloads) {
  writeFileSync(path, bytes);
  chmodSync(path, 0o755);
}

const check = spawnSync(process.execPath, [join(root, 'scripts/check-package.mjs')], { encoding: 'utf8' });
if (check.status !== 0) throw new Error(check.stderr || 'native package check failed');
const pack = spawnSync('npm', ['pack', '--dry-run', '--json'], { cwd: root, encoding: 'utf8' });
if (pack.status !== 0) throw new Error(pack.stderr || 'npm pack failed');
console.log(`release package ready: ${Object.values(JSON.parse(pack.stdout))[0].filename}`);
