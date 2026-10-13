// Preview the static export (out/) with the routes and headers in vercel.json, which production nginx mirrors. No API proxy.
// Only GET and HEAD; anything that is not an app route, docs page, licence, root asset, or Next static file is a 404.
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = path.dirname(fileURLToPath(import.meta.url));
const out = path.join(root, 'out');
const config = JSON.parse(await readFile(path.join(root, 'vercel.json'), 'utf8'));
const headerBlock = source => Object.fromEntries((config.headers.find(block => block.source === source)?.headers ?? []).map(header => [header.key, header.value]));
const headers = headerBlock('/(.*)');
const immutable = headerBlock('/_next/static/(.*)');
const types = {
  '.html': 'text/html; charset=utf-8', '.css': 'text/css; charset=utf-8', '.md': 'text/markdown; charset=utf-8', '.txt': 'text/plain; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8', '.json': 'application/json', '.svg': 'image/svg+xml', '.woff2': 'font/woff2', '.woff': 'font/woff', '.png': 'image/png',
};
const safe = '[A-Za-z0-9_-][A-Za-z0-9_.-]*';
const routes = [
  // The app shell: the client decides between the callback page and the workspace.
  { match: pathname => pathname === '/' || pathname === '/auth/callback' || /^\/sessions\/[^/]+\/live$/.test(pathname), file: () => 'index.html', shell: true },
  { match: pathname => pathname === '/docs' || pathname === '/docs/', file: () => 'docs/index.html' },
  { match: pathname => new RegExp(`^/docs/(?:${safe}/)*${safe}\\.(?:html|css|md)$`).test(pathname), file: pathname => pathname.slice(1) },
  { match: pathname => pathname === '/licenses/silicon-ui.txt' || pathname === '/boot.js' || pathname === '/mark.svg', file: pathname => pathname.slice(1) },
  { match: pathname => new RegExp(`^/_next/static/(?:${safe}/)*${safe}$`).test(pathname), file: pathname => pathname.slice(1), cache: immutable },
];

async function notFound(response, status = 404) {
  const page = await readFile(path.join(out, '404.html')).catch(() => null);
  response.writeHead(status, { ...headers, 'Content-Type': page ? types['.html'] : types['.txt'] });
  response.end(page ?? 'Not found');
}

const server = createServer(async (request, response) => {
  let pathname;
  try { pathname = new URL(request.url, 'http://localhost').pathname; } catch { pathname = ''; }
  const route = ['GET', 'HEAD'].includes(request.method) && !pathname.split('/').some(part => part === '..' || part.startsWith('.')) ? routes.find(candidate => candidate.match(pathname)) : undefined;
  if (!route) { await notFound(response); return; }
  const file = route.file(pathname);
  const type = types[path.extname(file)];
  let body;
  try { body = type ? await readFile(path.join(out, file)) : null; } catch { body = null; }
  if (!body) {
    if (route.shell) { response.writeHead(503, { ...headers, 'Content-Type': types['.txt'] }); response.end('Build the frontend first.'); }
    else await notFound(response);
    return;
  }
  response.writeHead(200, { ...headers, ...(route.cache ?? {}), 'Content-Type': type });
  response.end(request.method === 'HEAD' ? undefined : body);
});
server.listen(Number(process.env.SB_FRONTEND_PORT || 8092), '127.0.0.1', () => console.log(`Frontend preview: http://127.0.0.1:${server.address().port}`));
