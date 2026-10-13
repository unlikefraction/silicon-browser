import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { runInNewContext } from 'node:vm';
import { backendOrigin } from '../backend-origin.mjs';

const CSP = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; font-src 'self'; connect-src https://backend.browser.teamofsilicons.com http://127.0.0.1:8091; img-src 'self' data:; frame-src https:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";
type HeaderBlock = { source: string; headers: { key: string; value: string }[] };
type Rewrite = { source: string; destination: string };
const readJson = async (relative: string) => JSON.parse(await readFile(new URL(relative, import.meta.url), 'utf8'));

test('build backend configuration rejects visitor-controlled or credential-bearing URLs', () => {
  assert.equal(backendOrigin(), 'https://backend.browser.teamofsilicons.com'); assert.equal(backendOrigin('http://127.0.0.1:8091/'), 'http://127.0.0.1:8091');
  for (const value of ['https://evil.example', 'https://a:b@backend.browser.teamofsilicons.com', 'http://backend.browser.teamofsilicons.com', 'https://backend.browser.teamofsilicons.com/api', 'https://backend.browser.teamofsilicons.com?backend=x', 'https://backend.browser.teamofsilicons.com#x']) assert.throws(() => backendOrigin(value));
});

test('Next.js builds a static export with no server routes, headers, or source maps', async () => {
  const { default: config } = await import('../next.config.mjs');
  assert.equal(config.output, 'export');
  assert.equal(config.trailingSlash, false);
  assert.equal(config.reactStrictMode, true);
  assert.equal(config.productionBrowserSourceMaps, false);
  assert.equal(config.poweredByHeader, false);
  assert.deepEqual(config.images, { unoptimized: true });
  for (const hook of ['rewrites', 'headers', 'redirects', 'middleware', 'basePath', 'assetPrefix']) assert.equal((config as Record<string, unknown>)[hook], undefined, hook);
  assert.equal(config.env?.SB_BACKEND_ORIGIN, backendOrigin(process.env.SB_PUBLIC_BACKEND_URL));
  const pkg = await readJson('../package.json');
  assert.match(pkg.scripts.build, /tsc --noEmit && .*next build --webpack && node publish-docs\.mjs && node scripts\/check-export\.mjs$/);
  assert.match(pkg.scripts.build, /NEXT_TELEMETRY_DISABLED=1/);
  for (const [name, version] of Object.entries({ ...pkg.dependencies, ...pkg.devDependencies })) assert.match(String(version), /^\d+\.\d+\.\d+$/, `${name} must be pinned exactly`);
  assert.equal(pkg.dependencies.next, '16.3.6');
});

test('vercel.json routes the app shell and docs statically, never proxies APIs, and sends the strict policy', async () => {
  const config = await readJson('../vercel.json');
  const { packageManager } = await readJson('../package.json');
  assert.equal(config.framework, 'nextjs');
  assert.equal(config.outputDirectory, undefined);
  assert.equal(config.buildCommand, `npx --yes ${packageManager} run build`);
  assert.equal(config.installCommand, `npx --yes ${packageManager} install --frozen-lockfile`);
  assert.deepEqual(config.rewrites.map((route: Rewrite) => [route.source, route.destination]).sort(), [
    ['/auth/callback', '/index.html'], ['/docs', '/docs/index.html'], ['/sessions/:id/live', '/index.html'],
  ]);
  assert(!config.rewrites.some((route: Rewrite) => route.source.includes('/auth/obo/') || route.source.includes('api') || route.destination.includes('api')));
  assert.equal(config.redirects, undefined);
  const block = (source: string) => Object.fromEntries((config.headers as HeaderBlock[]).find(item => item.source === source)!.headers.map(header => [header.key, header.value]));
  const headers = block('/(.*)');
  assert.equal(headers['Content-Security-Policy'], CSP);
  const directives = Object.fromEntries(CSP.split('; ').map(directive => { const [name, ...values] = directive.split(' '); return [name, values]; }));
  assert.deepEqual(directives['script-src'], ["'self'"]);
  assert.deepEqual(directives['style-src'], ["'self'", "'unsafe-inline'"]);
  assert(!headers['Content-Security-Policy'].includes('unsafe-eval'));
  assert.equal(headers['Cache-Control'], 'no-store');
  assert.equal(headers['X-Content-Type-Options'], 'nosniff');
  assert.equal(headers['Referrer-Policy'], 'no-referrer');
  assert.equal(headers['Cross-Origin-Opener-Policy'], undefined, 'COOP would cut window.opener and break popup sign-in');
  assert.deepEqual(block('/_next/static/(.*)'), { 'Cache-Control': 'public, max-age=31536000, immutable' });
});

test('boot.js removes one-use secrets from the URL before the app starts and applies the stored theme', async () => {
  const source = await readFile(new URL('../public/boot.js', import.meta.url), 'utf8');
  const run = (href: string, stored: string | null, prefersDark = false, storageThrows = false) => {
    const url = new URL(href);
    const calls: unknown[][] = [];
    const window: Record<string, unknown> = { matchMedia: () => ({ matches: prefersDark }) };
    const root = { attributes: {} as Record<string, string>, setAttribute(name: string, value: string) { this.attributes[name] = value; } };
    const context = {
      window, location: { href: url.href, search: url.search, hash: url.hash, pathname: url.pathname }, history: { replaceState: (...args: unknown[]) => calls.push(args) },
      localStorage: { getItem: (key: string) => { if (storageThrows) throw new Error('blocked'); return key === 'silicon-browser:theme' ? stored : null; } },
      document: { documentElement: root }, Object,
    };
    runInNewContext(source, context);
    return { calls, window, theme: root.attributes['data-theme'] };
  };
  const callback = run('https://browser.teamofsilicons.com/auth/callback?state=s&code=sac_secret', 'dark');
  assert.deepEqual(callback.calls, [[null, '', '/auth/callback']]);
  assert.equal(callback.window.__sbEntry, 'https://browser.teamofsilicons.com/auth/callback?state=s&code=sac_secret');
  assert(!Object.keys(callback.window).includes('__sbEntry'), 'the captured entry URL is not enumerable');
  assert.equal(callback.theme, 'dark');
  const live = run('https://browser.teamofsilicons.com/sessions/s1/live#grant=secret', 'light', true);
  assert.deepEqual(live.calls, [[null, '', '/sessions/s1/live']]);
  assert.equal(live.theme, 'light');
  const plain = run('https://browser.teamofsilicons.com/', null, true);
  assert.deepEqual(plain.calls, []);
  assert.equal(plain.theme, 'dark');
  assert.equal(run('https://browser.teamofsilicons.com/', 'purple').theme, 'light');
  assert.equal(run('https://browser.teamofsilicons.com/', null, true, true).theme, 'dark');
});

test('static preview serves app deep links, immutable Next assets, docs and licence, and nothing else', async t => {
  const { mkdtemp, mkdir, copyFile, writeFile, rm } = await import('node:fs/promises');
  const { tmpdir } = await import('node:os'); const path = await import('node:path'); const { spawn } = await import('node:child_process');
  const dir = await mkdtemp(path.join(tmpdir(), 'sb-next-preview-'));
  await copyFile(new URL('../dev.mjs', import.meta.url), path.join(dir, 'dev.mjs')); await copyFile(new URL('../vercel.json', import.meta.url), path.join(dir, 'vercel.json'));
  const files: Record<string, string> = {
    'out/index.html': '<main>Browser</main>', 'out/404.html': '<main>Missing</main>', 'out/boot.js': '// boot', 'out/mark.svg': '<svg/>',
    'out/_next/static/chunks/main-123.js': 'export {}', 'out/_next/static/css/app-123.css': 'body{}', 'out/_next/static/media/inter-latin.woff2': 'font',
    'out/docs/index.html': '<main>Documentation</main>', 'out/docs/CLI.md': '# CLI guide', 'out/docs/operations/release.md': '# Release', 'out/docs/style.css': 'main{}',
    'out/licenses/silicon-ui.txt': 'MIT', 'out/.env': 'SECRET=1', 'out/assets/app-123.js': 'export {}',
  };
  for (const [file, content] of Object.entries(files)) { await mkdir(path.dirname(path.join(dir, file)), { recursive: true }); await writeFile(path.join(dir, file), content); }
  const child = spawn(process.execPath, ['dev.mjs'], { cwd: dir, env: { ...process.env, SB_FRONTEND_PORT: '0' }, stdio: ['ignore', 'pipe', 'pipe'] });
  t.after(async () => { if (child.exitCode === null) { const closed = new Promise<void>(resolve => child.once('close', () => resolve())); child.kill(); await closed; } await rm(dir, { recursive: true, force: true }); });
  const origin = await new Promise<string>((resolve, reject) => { const timeout = setTimeout(() => reject(new Error('Preview did not start')), 5000); child.once('error', error => { clearTimeout(timeout); reject(error); }); child.stdout.once('data', chunk => { clearTimeout(timeout); const match = String(chunk).match(/http:\/\/127\.0\.0\.1:\d+/); match ? resolve(match[0]) : reject(new Error('Missing preview URL')); }); });
  const expectSecurityHeaders = (response: Response) => {
    assert.equal(response.headers.get('content-security-policy'), CSP);
    assert.equal(response.headers.get('referrer-policy'), 'no-referrer');
    assert.equal(response.headers.get('x-content-type-options'), 'nosniff');
  };
  for (const route of ['/', '/auth/callback?state=n&code=sac_test', '/sessions/s1/live', '/sessions/any%20id/live#grant=x']) {
    const response = await fetch(origin + route);
    assert.equal(response.status, 200, route); assert.equal(await response.text(), '<main>Browser</main>');
    assert.equal(response.headers.get('content-type'), 'text/html; charset=utf-8'); assert.equal(response.headers.get('cache-control'), 'no-store'); expectSecurityHeaders(response);
  }
  for (const [route, type] of [['/_next/static/chunks/main-123.js', 'text/javascript; charset=utf-8'], ['/_next/static/css/app-123.css', 'text/css; charset=utf-8'], ['/_next/static/media/inter-latin.woff2', 'font/woff2']] as const) {
    const response = await fetch(origin + route);
    assert.equal(response.status, 200, route); assert.equal(response.headers.get('cache-control'), 'public, max-age=31536000, immutable'); assert.equal(response.headers.get('content-type'), type); expectSecurityHeaders(response);
  }
  for (const route of ['/docs', '/docs/']) { const response = await fetch(origin + route); assert.equal(response.status, 200); assert.equal(await response.text(), '<main>Documentation</main>'); assert.equal(response.headers.get('cache-control'), 'no-store'); }
  for (const [route, body] of [['/docs/CLI.md', '# CLI guide'], ['/docs/operations/release.md', '# Release']]) {
    const guide = await fetch(origin + route); assert.equal(guide.status, 200); assert.equal(guide.headers.get('content-type'), 'text/markdown; charset=utf-8'); assert.equal(await guide.text(), body);
  }
  const licence = await fetch(origin + '/licenses/silicon-ui.txt'); assert.equal(licence.status, 200); assert.equal(licence.headers.get('content-type'), 'text/plain; charset=utf-8'); assert.equal(await licence.text(), 'MIT');
  const boot = await fetch(origin + '/boot.js'); assert.equal(boot.status, 200); assert.equal(boot.headers.get('cache-control'), 'no-store');
  for (const route of ['/api/v1/me', '/auth/obo/callback', '/auth/callback/', '/sessions/s1/live/', '/sessions/live', '/.env', '/assets/app-123.js', '/assets/%2e%2e/.env', '/docs/.env', '/docs/%2e%2e/.env', '/_next/static/%2e%2e/%2e%2e/.env', '/_next/static/.hidden', '/index.html', '/out/index.html']) {
    const response = await fetch(origin + route);
    assert.equal(response.status, 404, route); assert.equal(response.headers.get('cache-control'), 'no-store'); expectSecurityHeaders(response);
  }
  assert.equal((await fetch(origin + '/', { method: 'POST' })).status, 404);
});
