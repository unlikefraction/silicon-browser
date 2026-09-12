import { existsSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { join } from 'node:path';

const root = new URL('..', import.meta.url).pathname;
for (const file of ['package.json', 'README.md', 'LICENSE', 'bin/launcher.js', 'bin/sb-darwin-arm64', 'bin/sb-darwin-x64', 'bin/sb-linux-arm64', 'bin/sb-linux-x64']) {
  if (!existsSync(join(root, file))) throw new Error(`missing ${file}`);
}
const result = spawnSync(process.execPath, [join(root, 'bin/launcher.js'), '--version'], { encoding: 'utf8' });
if ((process.platform === 'darwin' || process.platform === 'linux') && result.status !== 0) throw new Error(result.stderr || 'native --version failed');
console.log('package files and native launcher check passed');
