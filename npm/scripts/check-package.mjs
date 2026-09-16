import { existsSync, readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
for (const file of ['package.json', 'README.md', 'LICENSE', 'bin/launcher.js', 'bin/browser-darwin-arm64', 'bin/browser-darwin-x64', 'bin/browser-linux-arm64', 'bin/browser-linux-x64']) {
  if (!existsSync(join(root, file))) throw new Error(`missing ${file}`);
}
const result = spawnSync(process.execPath, [join(root, 'bin/launcher.js'), '--version'], { encoding: 'utf8' });
if ((process.platform === 'darwin' || process.platform === 'linux') && result.status !== 0) throw new Error(result.stderr || 'native --version failed');
const version = readFileSync(join(root, '../Cargo.toml'), 'utf8').match(/\[workspace\.package\][\s\S]*?\nversion = "([^"]+)"/)[1];
if (result.stdout.trim() !== `browser ${version}`) throw new Error('native command name/version does not match this release');
console.log('package files and native launcher check passed');
