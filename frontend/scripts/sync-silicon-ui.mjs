// Install Silicon UI registry items verbatim from registry-lock.json.
//
//   node scripts/sync-silicon-ui.mjs           download, verify sha256, write files, remove stale files
//   node scripts/sync-silicon-ui.mjs --check   download, verify sha256, fail if any file on disk differs
//   node scripts/sync-silicon-ui.mjs --update  download and record fresh sha256 hashes, then write files
//
// Every registry file targets `@components/silicon-ui/...`, which maps to `src/components/silicon-ui/...`.
// The registry already uses relative imports between its own files, so content is written byte-for-byte.
import { createHash } from 'node:crypto';
import { mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const REGISTRY = 'https://ui.teamofsilicons.com/r/';
const TARGET_PREFIX = '@components/silicon-ui/';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const uiDir = path.join(root, 'src/components/silicon-ui');
const lockPath = path.join(uiDir, 'registry-lock.json');
const kept = new Set(['registry-lock.json', 'LICENSE']);

const mode = process.argv.includes('--update') ? 'update' : process.argv.includes('--check') ? 'check' : 'sync';
const lock = JSON.parse(await readFile(lockPath, 'utf8'));
if (!Array.isArray(lock) || !lock.every(entry => typeof entry?.url === 'string' && entry.url.startsWith(REGISTRY) && entry.url.endsWith('.json'))) {
  throw new Error(`${path.relative(root, lockPath)} must be an array of {url, sha256} entries under ${REGISTRY}`);
}

const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const files = new Map();
const failures = [];
const locked = new Set(lock.map(entry => entry.url));

for (const entry of lock) {
  const response = await fetch(entry.url, { redirect: 'error', cache: 'no-store' });
  if (!response.ok) throw new Error(`${entry.url}: HTTP ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  const digest = sha256(bytes);
  if (mode === 'update') entry.sha256 = digest;
  else if (digest !== entry.sha256) { failures.push(`${entry.url}: sha256 ${digest} does not match the lock (${entry.sha256}). Review the change, then run with --update.`); continue; }

  const item = JSON.parse(bytes.toString('utf8'));
  for (const dependency of item.registryDependencies ?? []) {
    if (!locked.has(dependency)) failures.push(`${entry.url}: registry dependency ${dependency} is missing from the lock.`);
  }
  for (const file of item.files ?? []) {
    const target = String(file.target ?? '');
    const relative = target.slice(TARGET_PREFIX.length);
    if (!target.startsWith(TARGET_PREFIX) || !relative || relative.split('/').some(part => !part || part === '.' || part === '..') || kept.has(relative)) {
      throw new Error(`${entry.url}: refusing to write unexpected target ${JSON.stringify(target)}`);
    }
    if (typeof file.content !== 'string') throw new Error(`${entry.url}: ${target} has no content`);
    const previous = files.get(relative);
    if (previous !== undefined && previous.content !== file.content) failures.push(`${relative} differs between ${previous.url} and ${entry.url}.`);
    files.set(relative, { content: file.content, url: entry.url });
  }
}

async function listFiles(directory, prefix = '') {
  const found = [];
  for (const dirent of await readdir(directory, { withFileTypes: true }).catch(() => [])) {
    const relative = prefix ? `${prefix}/${dirent.name}` : dirent.name;
    if (dirent.isDirectory()) found.push(...await listFiles(path.join(directory, dirent.name), relative));
    else found.push(relative);
  }
  return found;
}
const stale = (await listFiles(uiDir)).filter(relative => !kept.has(relative) && !files.has(relative));

if (mode === 'check') {
  for (const [relative, { content }] of files) {
    const current = await readFile(path.join(uiDir, relative), 'utf8').catch(() => null);
    if (current !== content) failures.push(`${relative} is ${current === null ? 'missing' : 'not byte-identical to the registry'}.`);
  }
  for (const relative of stale) failures.push(`${relative} is not part of any locked registry item.`);
}
if (failures.length) {
  console.error(failures.join('\n'));
  process.exit(1);
}
if (mode !== 'check') {
  for (const [relative, { content }] of files) {
    const destination = path.join(uiDir, relative);
    await mkdir(path.dirname(destination), { recursive: true });
    await writeFile(destination, content);
  }
  for (const relative of stale) await rm(path.join(uiDir, relative));
  if (mode === 'update') await writeFile(lockPath, `${JSON.stringify(lock, null, 2)}\n`);
}
console.log(`${mode === 'check' ? 'Verified' : 'Installed'} ${files.size} files from ${lock.length} Silicon UI registry items${stale.length && mode !== 'check' ? `; removed ${stale.join(', ')}` : ''}.`);
