import { existsSync, readFileSync, mkdtempSync, rmSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
for (const file of ['package.json', 'README.md', 'LICENSE', 'bin/launcher.js', 'bin/browser-darwin-arm64', 'bin/browser-darwin-x64', 'bin/browser-linux-arm64', 'bin/browser-linux-x64']) {
  if (!existsSync(join(root, file))) throw new Error(`missing ${file}`);
}
const version = readFileSync(join(root, '../Cargo.toml'), 'utf8').match(/\[workspace\.package\][\s\S]*?\nversion = "([^"]+)"/)[1];
if (JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).version !== version) throw new Error('npm version does not match Cargo.toml');
const home = mkdtempSync(join(tmpdir(), 'browser-npm-check-'));
const env = { ...Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('SB_'))), SB_HOME: home };
const run = (...args) => spawnSync(process.execPath, [join(root, 'bin/launcher.js'), ...args], { encoding: 'utf8', env });
try {
  const result = run('--version');
  if (result.status !== 0) throw new Error(result.stderr || 'native --version failed');
  if (result.stdout.trim() !== `browser ${version}`) throw new Error('native command name/version does not match this release; run npm run prepare-release with current native artifacts');
  const help = run('--help');
  if (help.status !== 0 || !help.stdout.includes('Usage: browser ') || /\bsb\s/.test(help.stdout)) throw new Error('native help does not use the browser command');
  for (const [args, field, expected] of [
    [['accounts', '--json'], 'app_id', 'browser'],
    [['login', 'status', '--json'], 'authenticated', false]
  ]) {
    const result = run(...args);
    if (result.status !== 0 || JSON.parse(result.stdout)[field] !== expected) throw new Error(`native discovery check failed: ${args.join(' ')}`);
  }
  console.log('package files and native discovery checks passed');
} finally {
  rmSync(home, { recursive: true, force: true });
}
