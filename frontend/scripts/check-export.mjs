// Fails the build if the static export could break the production CSP (`script-src 'self'`) or is missing published files.
// Executable inline scripts, inline event handlers, and source maps are all rejected; only __NEXT_DATA__ (application/json) may be inline.
import { readFile, readdir, stat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const out = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'out');
const failures = [];

async function walk(directory) {
  const files = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const full = path.join(directory, entry.name);
    if (entry.isDirectory()) files.push(...await walk(full));
    else files.push(full);
  }
  return files;
}

for (const required of ['index.html', '404.html', 'boot.js', 'docs/index.html', 'docs/CLI.md', 'licenses/silicon-ui.txt']) {
  if (!(await stat(path.join(out, required)).catch(() => null))?.isFile()) failures.push(`missing ${required}`);
}

const attribute = (attributes, name) => attributes.match(new RegExp(`(?:^|\\s)${name}\\s*=\\s*(?:"([^"]*)"|'([^']*)'|([^\\s>]+))`, 'i'))?.slice(1).find(value => value !== undefined);
const files = await walk(out).catch(() => []);
for (const file of files) {
  const relative = path.relative(out, file);
  if (file.endsWith('.map')) failures.push(`${relative}: source map`);
  if (file.endsWith('.js') && /\/\/[#@]\s*sourceMappingURL=/.test(await readFile(file, 'utf8'))) failures.push(`${relative}: sourceMappingURL comment`);
  if (!file.endsWith('.html')) continue;
  const html = await readFile(file, 'utf8');
  for (const [, attributes = '', body = ''] of html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script\s*>/gi)) {
    const src = attribute(attributes, 'src'), type = attribute(attributes, 'type');
    if (src !== undefined) { if (body.trim()) failures.push(`${relative}: <script src> with an inline body`); continue; }
    if (type?.toLowerCase() !== 'application/json') failures.push(`${relative}: executable inline <script${attributes}>`);
  }
  for (const [tag] of html.matchAll(/<[a-z][a-z0-9-]*\b(?:[^>"']|"[^"]*"|'[^']*')*>/gi)) {
    if (/\son[a-z]+\s*=/i.test(tag)) failures.push(`${relative}: inline event handler in ${tag.slice(0, 80)}`);
    if (/\s(?:href|src|action|formaction)\s*=\s*["']?\s*javascript:/i.test(tag)) failures.push(`${relative}: javascript: URL in ${tag.slice(0, 80)}`);
  }
}

// One-use secrets must leave the URL before Next.js starts: boot.js has to be the first script, synchronous and classic.
const index = await readFile(path.join(out, 'index.html'), 'utf8').catch(() => '');
const first = index.match(/<script\b([^>]*)>/i)?.[1] ?? '';
if (attribute(first, 'src') !== '/boot.js' || /\s(?:defer|async)\b/i.test(first) || attribute(first, 'type') !== undefined) failures.push('index.html: /boot.js must be the first script in the document, without defer, async, or type');

if (failures.length) {
  console.error(`Static export check failed:\n  ${failures.join('\n  ')}`);
  process.exit(1);
}
console.log(`Static export check passed (${files.filter(file => file.endsWith('.html')).length} HTML files, no executable inline scripts).`);
