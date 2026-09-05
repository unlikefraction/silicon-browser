// Preview the static deployment with Vercel's headers and routes. No API proxy.
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
const root = path.dirname(fileURLToPath(import.meta.url));
const config = JSON.parse(await readFile(path.join(root, 'vercel.json'), 'utf8'));
const headers = Object.fromEntries(config.headers[0].headers.map(header => [header.key, header.value]));
const types = { '.svg': 'image/svg+xml', '.js': 'text/javascript', '.css': 'text/css', '.woff': 'font/woff', '.woff2': 'font/woff2' };
const server = createServer(async (request, response) => {
  const pathname = new URL(request.url, 'http://localhost').pathname;
  const page = pathname === '/' || pathname === '/auth/callback' || /^\/sessions\/[^/]+\/live\/?$/.test(pathname);
  const asset = /^\/assets\/[A-Za-z0-9_.-]+$/.test(pathname) && types[path.extname(pathname)];
  if (!['GET', 'HEAD'].includes(request.method) || (!page && !asset)) {
    response.writeHead(404, headers); response.end('Not found'); return;
  }
  try {
    const body = await readFile(path.join(root, 'dist', page ? 'index.html' : pathname.slice(1)));
    response.writeHead(200, { ...headers, ...(asset ? { 'Cache-Control': 'public, max-age=31536000, immutable' } : {}), 'Content-Type': page ? 'text/html; charset=utf-8' : asset });
    response.end(request.method === 'HEAD' ? undefined : body);
  } catch { response.writeHead(503, headers); response.end('Build the frontend first.'); }
});
server.listen(Number(process.env.SB_FRONTEND_PORT || 8092), '127.0.0.1', () => console.log(`Frontend preview: http://127.0.0.1:${server.address().port}`));
