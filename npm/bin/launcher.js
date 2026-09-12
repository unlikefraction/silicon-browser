#!/usr/bin/env node

import { existsSync, chmodSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const dir = dirname(fileURLToPath(import.meta.url));
const platform = process.platform;
const arch = process.arch;
let name;

if (platform === 'darwin' && (arch === 'arm64' || arch === 'x64')) name = `sb-darwin-${arch}`;
else if (platform === 'linux' && (arch === 'arm64' || arch === 'x64')) name = `sb-linux-${arch}`;
else {
  console.error(`Silicon Browser does not support ${platform}-${arch}; supported targets are macOS and Linux on x64 or arm64.`);
  process.exit(1);
}

if (platform === 'linux') {
  const version = process.report?.getReport()?.header?.glibcVersionRuntime;
  const [major, minor] = (version || '').split('.').map(Number);
  if (!version || major < 2 || (major === 2 && minor < 34)) {
    console.error('Silicon Browser requires glibc 2.34 or later on Linux. Alpine/musl is unsupported.');
    process.exit(1);
  }
}

const binary = join(dir, name);
if (!existsSync(binary)) {
  console.error(`Silicon Browser binary is missing for ${platform}-${arch}. Reinstall silicon-browser.`);
  process.exit(1);
}
if (platform !== 'win32') chmodSync(binary, 0o755);

const result = spawnSync(binary, process.argv.slice(2), { stdio: 'inherit' });
if (result.error) {
  console.error(`Could not start Silicon Browser: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status ?? 1);
