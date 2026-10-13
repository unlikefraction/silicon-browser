// End-to-end checks of the exported site in headless Google Chrome, under the production headers (out/ served by dev.mjs).
// The backend is never reached: every API call to the local backend origin is answered by an in-memory mock.
//
//   SB_PUBLIC_BACKEND_URL=http://127.0.0.1:8091 npx --yes pnpm@10.33.0 build
//   E2E_SCREENSHOTS=/tmp/browser-screens npx --yes pnpm@10.33.0 e2e      # screenshots are optional
//
// Chrome comes from CHROME_PATH, otherwise the installed Google Chrome channel. `E2E_ONLY=<text>` runs matching scenarios.
// Every scenario fails on a Content-Security-Policy violation, an uncaught page error, or an unexpected console error.
import { spawn } from 'node:child_process';
import { mkdir, readFile, readdir } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';
import { chromium } from 'playwright-core';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const BACKEND = 'http://127.0.0.1:8091';
const STORAGE_KEY = `silicon-browser:session:v4:${BACKEND}`;
const THEME_KEY = 'silicon-browser:theme';
const REDIRECT_KEY = 'silicon-browser:accounts-attempt';
const SHOTS = process.env.E2E_SCREENSHOTS ? path.resolve(process.env.E2E_SCREENSHOTS) : null;
const ONLY = process.env.E2E_ONLY ?? '';
const vercel = JSON.parse(await readFile(path.join(root, 'vercel.json'), 'utf8'));
const CSP = vercel.headers.find(block => block.source === '/(.*)').headers.find(header => header.key === 'Content-Security-Policy').value;

// ---------- fixtures ----------
const ADA = { id: '11111111-1111-4111-8111-111111111111', name: 'Ada Lovelace', kind: 'carbon' };
const ATLAS = { id: '33333333-3333-4333-9333-333333333333', name: 'Atlas', kind: 'silicon' };
const auth = (identity, generation = 1) => ({
  access_token: `eyJhbGciOiJFUzI1NiJ9.eyJzdWIiOiIke${identity.kind}${generation}.c2lnbmF0dXJl${generation}`,
  refresh_token: `sar_${identity.kind}_${generation}`,
  expires_at: '2099-01-01T00:00:00Z',
  identity,
});
const ADA_AUTH = auth(ADA), ATLAS_AUTH = auth(ATLAS);
/** The localStorage value BrowserSession writes (historical shape with the `/browser` suffix and org). */
const saved = (...sessions) => JSON.stringify({ active: `${sessions[0].identity.id}/browser`, sessions: sessions.map(session => ({ ...session, org: { id: 'browser', name: 'Browser' } })) });
const ATTEMPT = '22222222-2222-4222-8222-222222222222';
const STATE = `${ATTEMPT}.${'5f'.repeat(32)}`;
const SLT = 'slt_e2e-Token_0123456789';

const S1 = '0192f6a4-7c1e-7b2a-9d3e-5f1a2b3c4d5e', S2 = '0192f5d1-2a3b-7c4d-8e5f-6a7b8c9d0e1f', S3 = '0192f4b2-9c8d-7e6f-a5b4-c3d2e1f0a9b8', S4 = '0192f7e3-1d2c-7b3a-8f4e-5d6c7b8a9f0e';
const usage = (seconds, bytesIn, bytesOut, unclassified, micros) => ({ browser_seconds: seconds, proxy_bytes_in: bytesIn, proxy_bytes_out: bytesOut, proxy_bytes_unclassified: unclassified, ...(micros === undefined ? {} : { cost: { total: { micros, currency: 'USD' } } }) });
const LIVE_GRANT = 'lg_9f2c41d07be35a68';
const EXTRA_ACCESS = '44444444-4444-4444-8444-444444444444';

function fixtures() {
  return {
    sessions: [
      { id: S1, name: 'Vendor research', description: 'Compare three CRM vendors on pricing, API limits, and support.', status: 'active', initiator_id: ADA.id, participant_ids: [ADA.id, ATLAS.id], started_at: '2026-10-12T09:30:00Z', expires_at: '2026-10-12T10:30:00Z', usage: usage(1260, 520_000_000, 180_000_000, 60_000_000, 12_300) },
      { id: S2, name: 'Checkout flow audit', description: 'Walk the staging checkout and note every friction point.', status: 'complete', initiator_id: ADA.id, participant_ids: [], started_at: '2026-10-11T14:05:00Z', expires_at: '2026-10-11T15:05:00Z', usage: usage(618, 300_000_000, 130_000_000, 40_000_000, 4_100) },
      { id: S3, name: 'Supplier onboarding', description: 'Fill the supplier portal forms with the shared test data.', status: 'expired', initiator_id: ADA.id, started_at: '2026-10-10T08:15:00Z', expires_at: '2026-10-10T08:30:00Z', usage: usage(120, 0, 0, 0) },
      { id: S4, name: 'Price monitor', description: 'Check competitor prices every morning.', status: 'active', initiator_id: ATLAS.id, participant_ids: [], started_at: '2026-10-12T07:00:00Z', expires_at: '2026-10-12T11:00:00Z', usage: usage(300, 1_000_000, 1_000_000, 0, 900) },
    ],
    profiles: [
      { id: 'prf_research', name: 'Research workspace', fingerprint: 'fp_7d1e', location: { code: 'de', name: 'Germany' }, access: [ATLAS.id], owner_id: ADA.id, sessions_run: 4, status: 'active' },
      { id: 'prf_retail', name: 'Retail checkout', fingerprint: 'fp_2b9a', location: { code: 'us', name: 'United States' }, access: [], owner_id: ADA.id, sessions_run: 12, status: 'retired' },
    ],
    locations: [{ code: 'us', name: 'United States' }, { code: 'de', name: 'Germany' }, { code: 'gb', name: 'United Kingdom' }, { code: 'jp', name: 'Japan' }],
    recordings: [
      { session_id: S2, owner_id: ADA.id, session_name: 'Checkout flow audit', session_description: 'Walk the staging checkout and note every friction point.', status: 'available', created_at: '2026-10-11T15:06:00Z', duration_seconds: 2580, size_bytes: 48_200_000, briefcase_link: 'https://briefcase.example/recordings/checkout-flow-audit' },
      { session_id: S1, owner_id: ADA.id, session_name: 'Vendor research', session_description: 'Compare three CRM vendors on pricing, API limits, and support.', status: 'pending', created_at: '2026-10-12T10:31:00Z', duration_seconds: 1260, size_bytes: 0 },
      { session_id: S3, owner_id: ADA.id, session_name: 'Supplier onboarding', session_description: 'Fill the supplier portal forms with the shared test data.', status: 'failed', created_at: '2026-10-10T08:31:00Z', duration_seconds: 120, size_bytes: 0, delivery_error: 'native_recording_unavailable', briefcase_link: 'http://briefcase.example/insecure' },
    ],
    usage: [
      { session_id: S1, ...usage(1260, 520_000_000, 180_000_000, 60_000_000, 12_300) },
      { session_id: S2, ...usage(618, 300_000_000, 130_000_000, 40_000_000, 4_100) },
      { session_id: S3, ...usage(120, 0, 0, 0) },
    ],
    limits: { concurrent_browser_limit: 4, rate_limit: null, checked_at: '2026-10-12T10:00:00Z' },
  };
}
const EMPTY = { sessions: [], profiles: [], recordings: [], usage: [] };

class HttpError extends Error { constructor(status, message) { super(message); this.status = status; } }

/** An in-memory Browser API (`/api/v1`) with the response envelope, CORS, and bearer checks of the real one. */
function createBackend(origin, { data = {}, failLimits = false } = {}) {
  const db = { ...fixtures(), ...structuredClone(data) };
  const tokens = new Map([[ADA_AUTH.access_token, ADA], [ATLAS_AUTH.access_token, ATLAS]]);
  const requests = [], unauthorized = [], gates = new Map(), failures = new Map();
  let recordingCalls = 0, created = 0;
  const visible = identity => db.sessions.filter(item => item.initiator_id === identity.id || item.participant_ids?.includes(identity.id));
  const session = (identity, id) => visible(identity).find(item => item.id === id) ?? (() => { throw new HttpError(404, 'Session not found.'); })();
  const profile = id => db.profiles.find(item => item.id === id) ?? (() => { throw new HttpError(404, 'Profile not found.'); })();
  const routes = [
    ['POST', /^\/auth\/exchange$/, ({ body }) => { if (body.short_lived_token !== SLT) throw new HttpError(400, 'The short-lived token is invalid or expired.'); return ATLAS_AUTH; }, false],
    ['POST', /^\/auth\/attempts$/, () => ({
      attempt_id: ATTEMPT, state: STATE,
      authorize_url: `https://accounts.teamofsilicons.com/authorize?${new URLSearchParams({ app_id: 'browser', state: STATE, response_type: 'code', code_challenge_method: 'S256', code_challenge: 'E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM', redirect_uri: `${origin}/auth/callback` })}`,
    }), false],
    ['POST', /^\/auth\/attempts\/([^/]+)\/exchange$/, ({ params: [id], body }) => { if (id !== ATTEMPT || body.state !== STATE || !/^sac_/.test(body.code)) throw new HttpError(400, 'Sign-in could not be completed.'); return ADA_AUTH; }, false],
    ['GET', /^\/sessions$/, ({ identity }) => visible(identity)],
    ['POST', /^\/sessions$/, ({ identity, body }) => {
      const item = { id: `0192f8a0-0000-7000-8000-00000000000${++created}`, name: body.name, description: body.description, status: 'active', initiator_id: identity.id, participant_ids: [], started_at: '2026-10-12T11:00:00Z', expires_at: '2026-10-12T11:30:00Z', usage: usage(0, 0, 0, 0), ...(body.profile_id ? { profile_id: body.profile_id } : {}) };
      db.sessions.unshift(item);
      return item;
    }],
    ['GET', /^\/sessions\/([^/]+)$/, ({ identity, params: [id] }) => session(identity, id)],
    ['POST', /^\/sessions\/([^/]+)\/end$/, ({ identity, params: [id] }) => { const item = session(identity, id); item.status = 'complete'; return item; }],
    ['POST', /^\/sessions\/([^/]+)\/live$/, ({ identity, params: [id] }) => { session(identity, id); return { url: `${origin}/sessions/${encodeURIComponent(id)}/live#grant=${LIVE_GRANT}` }; }],
    ['POST', /^\/sessions\/([^/]+)\/live\/redeem$/, ({ identity, params: [id], body }) => { session(identity, id); if (!body?.grant) throw new HttpError(403, 'This live link has expired.'); return { url: `https://live.example/view/${id}` }; }],
    ['GET', /^\/profiles$/, () => db.profiles],
    ['POST', /^\/profiles$/, ({ identity, body }) => { const item = { id: `prf_new_${db.profiles.length}`, name: body.name, fingerprint: 'fp_new', location: db.locations.find(place => place.code === body.location), access: body.access, owner_id: identity.id, sessions_run: 0, status: 'active' }; db.profiles.push(item); return item; }],
    ['PATCH', /^\/profiles\/([^/]+)$/, ({ params: [id], body }) => Object.assign(profile(id), body)],
    ['POST', /^\/profiles\/([^/]+)\/end$/, ({ params: [id] }) => Object.assign(profile(id), { status: 'retired' })],
    ['GET', /^\/proxy-locations$/, () => db.locations],
    ['GET', /^\/recordings$/, () => {
      // The pending recording becomes available on the next refresh (the 10 s background poll in the recordings scenario).
      if (++recordingCalls > 1) for (const item of db.recordings) if (item.status === 'pending') Object.assign(item, { status: 'available', size_bytes: 31_500_000, briefcase_link: 'https://briefcase.example/recordings/vendor-research' });
      return db.recordings;
    }],
    ['POST', /^\/recordings\/([^/]+)\/trash$/, ({ params: [id] }) => { db.recordings = db.recordings.filter(item => item.session_id !== id); return { session_id: id, status: 'trashed' }; }],
    ['GET', /^\/usage$/, () => db.usage],
    ['GET', /^\/usage\/limits$/, () => { if (failLimits) throw new HttpError(503, 'Capacity check unavailable.'); return db.limits; }],
  ];

  async function handle(route) {
    const request = route.request();
    const cors = { 'Access-Control-Allow-Origin': origin, 'Access-Control-Allow-Headers': 'accept, authorization, content-type, idempotency-key', 'Access-Control-Allow-Methods': 'GET, POST, PATCH', 'Access-Control-Expose-Headers': 'x-sb-auth-rejected, x-request-id', Vary: 'Origin' };
    if (request.method() === 'OPTIONS') return route.fulfill({ status: 204, headers: cors });
    const url = new URL(request.url());
    const pathname = url.pathname.replace(/^\/api\/v1(?=\/)/, '');
    const raw = request.postData();
    const entry = { method: request.method(), pathname, authorization: request.headers().authorization, body: raw ? JSON.parse(raw) : undefined };
    requests.push(entry);
    const key = `${entry.method} ${pathname}`;
    let status = 200, payload;
    try {
      if (!url.pathname.startsWith('/api/v1/')) throw new HttpError(404, 'Not found.');
      if (gates.has(key)) await gates.get(key).promise;
      if (failures.has(key)) throw failures.get(key);
      const match = routes.map(([method, pattern, run, signedIn = true]) => ({ method, found: pattern.exec(pathname), run, signedIn })).find(item => item.method === entry.method && item.found);
      if (!match) throw new HttpError(404, 'Not found.');
      let identity = null;
      if (match.signedIn) {
        identity = tokens.get(entry.authorization?.replace(/^Bearer /, '')) ?? null;
        if (!identity) { unauthorized.push(key); throw new HttpError(401, 'Sign in to continue.'); }
      } else if (entry.authorization) unauthorized.push(`${key} (sent a token to a sign-in endpoint)`);
      payload = { data: structuredClone(match.run({ identity, body: entry.body, params: match.found.slice(1).map(decodeURIComponent) })) };
    } catch (error) {
      if (!(error instanceof HttpError)) throw error;
      status = error.status;
      payload = { error: { message: error.message, request_id: '0192f8b1-e2e0-7000-8000-000000000001' } };
    }
    return route.fulfill({ status, headers: { ...cors, 'Content-Type': 'application/json' }, body: JSON.stringify(payload) });
  }

  return {
    db, requests, unauthorized, handle,
    /** Requests other than CORS preflights, optionally for one `METHOD /path`. */
    sent: key => key ? requests.filter(item => `${item.method} ${item.pathname}` === key) : requests,
    /** Holds every `METHOD /path` response until the returned release function runs. */
    hold(key) { let release; const promise = new Promise(resolve => { release = resolve; }); gates.set(key, { promise }); return () => { gates.delete(key); release(); }; },
    fail(key, status, message) { failures.set(key, new HttpError(status, message)); },
    restore(key) { failures.delete(key); },
  };
}

// ---------- harness ----------
const chunks = path.join(root, 'out/_next/static/chunks');
const bundle = (await Promise.all((await readdir(chunks, { recursive: true }).catch(() => [])).filter(file => file.endsWith('.js')).map(file => readFile(path.join(chunks, file), 'utf8')))).join('\n');
if (!bundle) throw new Error('Build the frontend first (out/ is missing).');
if (!bundle.includes(BACKEND)) throw new Error(`out/ was not built for ${BACKEND}. Rebuild with SB_PUBLIC_BACKEND_URL=${BACKEND}.`);
if (SHOTS) await mkdir(SHOTS, { recursive: true });

const preview = spawn(process.execPath, ['dev.mjs'], { cwd: root, env: { ...process.env, SB_FRONTEND_PORT: '0' }, stdio: ['ignore', 'pipe', 'inherit'] });
const origin = await new Promise((resolve, reject) => {
  const timeout = setTimeout(() => reject(new Error('Preview did not start')), 5000);
  preview.stdout.once('data', chunk => { clearTimeout(timeout); const match = String(chunk).match(/http:\/\/127\.0\.0\.1:\d+/); match ? resolve(match[0]) : reject(new Error('Missing preview URL')); });
});
const browser = await chromium.launch(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : { channel: 'chrome' });
const results = [], screenshots = [];

/** Polls a Node-side condition (request logs and the like). */
async function until(predicate, label, timeout = 5000) {
  const end = Date.now() + timeout;
  while (!(await predicate())) {
    if (Date.now() > end) throw new Error(`Timed out waiting for ${label}`);
    await new Promise(resolve => setTimeout(resolve, 50));
  }
}

/**
 * Runs one scenario in a fresh browser context: optional saved accounts, theme, and redirect attempt; the mocked backend;
 * and tracking of CSP violations, page errors, and console errors in every page of the context (popups included).
 */
async function scenario(name, options, run) {
  if (ONLY && !name.includes(ONLY)) return;
  const { accounts = [], theme = null, redirectAttempt = null, viewport = { width: 1280, height: 860 }, reducedMotion = 'no-preference', colorScheme = 'light', data, failLimits, allowConsole = [] } = options;
  const context = await browser.newContext({ viewport, reducedMotion, colorScheme });
  await context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin });
  const problems = [];
  await context.exposeBinding('__e2eViolation', ({ frame }, text) => problems.push(`CSP violation on ${frame.url()}: ${text}`));
  await context.addInitScript(([seed]) => {
    document.addEventListener('securitypolicyviolation', event => { void window.__e2eViolation(`${event.violatedDirective} blocked ${event.blockedURI || 'inline'} ${event.sample || ''}`.trim()); });
    if (location.protocol === 'http:' && !sessionStorage.getItem('e2e-seeded')) {
      sessionStorage.setItem('e2e-seeded', '1');
      if (seed.accounts) localStorage.setItem(seed.storageKey, seed.accounts);
      if (seed.theme) localStorage.setItem(seed.themeKey, seed.theme);
      if (seed.redirectAttempt) sessionStorage.setItem(seed.redirectKey, JSON.stringify({ ...seed.redirectAttempt, created: Date.now() }));
    }
  }, [{ storageKey: STORAGE_KEY, accounts: accounts.length ? saved(...accounts) : null, themeKey: THEME_KEY, theme, redirectKey: REDIRECT_KEY, redirectAttempt }]);
  const backend = createBackend(origin, { data, failLimits });
  await context.route(`${BACKEND}/**`, backend.handle);
  await context.route('https://live.example/**', route => route.fulfill({ status: 200, headers: { 'Content-Type': 'text/html' }, body: LIVE_PAGE }));
  await context.route('https://accounts.teamofsilicons.com/**', route => {
    const login = new URL(route.request().url());
    const callback = new URL(login.searchParams.get('redirect_uri'));
    callback.search = new URLSearchParams({ state: login.searchParams.get('state'), code: 'sac_popup-code_42' }).toString();
    return route.fulfill({ status: 200, headers: { 'Content-Type': 'text/html' }, body: `<!doctype html><title>Silicon Accounts</title><h1>Silicon Accounts</h1><a href="${callback.href}">Approve Browser</a>` });
  });
  const watch = page => {
    page.setDefaultTimeout(10000);
    page.on('pageerror', error => problems.push(`page error on ${page.url()}: ${error.message}`));
    page.on('console', message => {
      if (message.type() !== 'error') return;
      const text = message.text();
      if (!allowConsole.some(pattern => pattern.test(text))) problems.push(`console error on ${page.url()}: ${text}`);
    });
  };
  context.on('page', watch);
  const page = await context.newPage();
  const started = Date.now();
  try {
    await run({ page, context, backend });
    assert.deepEqual(backend.unauthorized, [], 'every workspace request carried the signed-in account token');
    assert.deepEqual(problems, [], 'no CSP violations, page errors, or console errors');
    results.push(`ok   ${name} (${((Date.now() - started) / 1000).toFixed(1)}s)`);
  } catch (error) {
    results.push(`FAIL ${name}: ${error.message.split('\n').slice(0, 12).join('\n     ')}${problems.length ? `\n     ${problems.join('\n     ')}` : ''}`);
    process.exitCode = 1;
    if (SHOTS) await page.screenshot({ path: path.join(SHOTS, `failure-${name.replace(/[^a-z0-9]+/gi, '-').slice(0, 60)}.png`), fullPage: true }).catch(() => {});
  } finally { await context.close(); }
}

const LIVE_PAGE = `<!doctype html><meta charset="utf-8"><title>Live browser</title>
<body style="margin:0;font:14px system-ui;background:#f6f6f4;color:#222"><div style="padding:10px 14px;background:#fff;border-bottom:1px solid #ddd">https://crm-vendor.example/pricing</div>
<main style="padding:32px"><h1 style="margin:0 0 8px">Pricing</h1><p>Starter, Team, and Enterprise plans.</p></main></body>`;

const workspaceNav = page => page.getByRole('complementary', { name: 'Browser' }).getByRole('navigation', { name: 'Workspace' });
const heading = (page, name) => page.getByRole('heading', { level: 1, name, exact: true });
const main = page => page.getByRole('main');
const toast = (page, title, description) => {
  const items = page.getByRole('region', { name: /Notifications/ }).getByRole('listitem');
  // Fast runs can leave two "Profile saved" toasts visible; match the saved profile too.
  return (description ? items.filter({ has: page.getByText(description, { exact: true }) }) : items).getByText(title, { exact: true });
};

/** Waits until no workspace task is running: the rail's buttons are enabled again (they are disabled while busy). */
async function idle(page) {
  await page.waitForFunction(() => {
    const button = document.querySelector('aside nav button, aside button');
    return !document.querySelector('[role="status"][aria-busy="true"]') && (!button || !button.disabled);
  });
}

/** Opens a workspace tab from the rail, or from the drawer when the rail is collapsed (below 720px). */
async function openTab(page, label) {
  const nav = workspaceNav(page);
  if (await nav.isVisible()) await nav.getByRole('button', { name: label, exact: true }).click();
  else {
    await page.getByRole('button', { name: 'Open navigation' }).click();
    const drawer = page.getByRole('dialog', { name: 'Browser' });
    await drawer.getByRole('button', { name: label, exact: true }).click();
    await drawer.waitFor({ state: 'detached' });
  }
  await idle(page);
}

async function noHorizontalScroll(page, label) {
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  assert.equal(overflow, 0, `${label}: the page scrolls sideways by ${overflow}px`);
}

/** A full-page screenshot (when E2E_SCREENSHOTS is set), after fonts load and motion settles. */
async function shoot(page, name) {
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(250);
  await noHorizontalScroll(page, name);
  if (!SHOTS) return;
  const file = path.join(SHOTS, `${name}.png`);
  await page.screenshot({ path: file, fullPage: true });
  screenshots.push(file);
}

const iframeContract = { title: 'Live browser session', referrerPolicy: 'no-referrer', sandbox: 'allow-scripts allow-same-origin allow-forms allow-popups', allow: 'clipboard-read; clipboard-write; fullscreen' };
const iframeAttributes = frame => frame.evaluate(element => ({ title: element.getAttribute('title'), src: element.getAttribute('src'), referrerPolicy: element.getAttribute('referrerpolicy'), sandbox: element.getAttribute('sandbox'), allow: element.getAttribute('allow'), attributes: element.getAttributeNames().sort() }));
async function assertLiveFrame(page, id) {
  const frame = page.locator('iframe');
  await frame.waitFor();
  assert.deepEqual(await iframeAttributes(frame), { ...iframeContract, src: `https://live.example/view/${id}`, attributes: ['allow', 'class', 'referrerpolicy', 'sandbox', 'src', 'title'] });
}
const storageSnapshot = page => page.evaluate(() => JSON.stringify({ local: { ...localStorage }, session: { ...sessionStorage } }));
const savedAccounts = async page => JSON.parse(await page.evaluate(key => localStorage.getItem(key) ?? 'null', STORAGE_KEY));

// ---------- scenarios ----------
try {
  await scenario('signed-out welcome: hero, both sign-in paths, token field contract, and the production headers', {}, async ({ page, backend }) => {
    const response = await page.goto(`${origin}/`, { waitUntil: 'load' });
    assert.equal(response.headers()['content-security-policy'], CSP);
    assert.equal(response.headers()['cache-control'], 'no-store');
    assert.equal(response.headers()['x-content-type-options'], 'nosniff');
    assert.equal(response.headers()['referrer-policy'], 'no-referrer');
    await heading(page, 'A browser, ready when you are.').waitFor();
    assert.equal(await page.evaluate(() => document.documentElement.dataset.theme), 'light');
    assert.deepEqual(await page.getByRole('navigation', { name: 'Breadcrumb' }).getByRole('listitem').allInnerTexts(), ['Browser', 'Welcome']);
    assert.equal(await page.getByRole('button', { name: /Account menu/ }).count(), 0);
    for (const feature of ['01 / Profiles', '02 / Together', '03 / Recorded']) await page.getByText(feature).waitFor();
    const silicon = page.getByRole('button', { name: 'Continue as Silicon' });
    assert.equal(await silicon.getAttribute('aria-expanded'), 'false');
    await silicon.click();
    const field = page.getByLabel('Short-lived token', { exact: true });
    await field.waitFor();
    assert.equal(await silicon.getAttribute('aria-expanded'), 'true');
    assert.deepEqual(await field.evaluate(input => [input.name, input.type, input.required, input.maxLength, input.autocomplete, input.spellcheck, input.placeholder]), ['slt', 'password', true, 16384, 'off', false, 'slt_…']);
    await page.getByText('silicon-accounts login --app browser').waitFor();
    // The form clips only while its height animates, so the keyboard outline of "Sign in as Silicon" is never cut off.
    const reveal = page.locator('form:has(input[name="slt"])').locator('xpath=..');
    await until(async () => await reveal.evaluate(node => getComputedStyle(node).overflow) === 'visible', 'the open token form to stop clipping');
    assert.deepEqual(backend.sent(), [], 'signed out, nothing is requested');
    const docs = page.getByRole('link', { name: /CLI & documentation/ });
    assert.deepEqual([await docs.getAttribute('href'), await docs.getAttribute('target'), await docs.getAttribute('rel')], ['/docs/', '_blank', 'noopener noreferrer']);
  });

  await scenario('Silicon sign-in: the token is validated, cleared before the request, sent once, and never stored', {}, async ({ page, backend }) => {
    await page.goto(`${origin}/`, { waitUntil: 'load' });
    await page.getByRole('button', { name: 'Continue as Silicon' }).click();
    const field = page.getByLabel('Short-lived token', { exact: true });
    await field.fill('not-a-token');
    await page.getByRole('button', { name: 'Sign in as Silicon' }).click();
    await page.getByText('Enter the short-lived token from silicon-accounts login --app browser.').waitFor();
    assert.equal(await field.inputValue(), '', 'the form is reset before validation');
    assert.deepEqual(backend.sent(), []);

    const release = backend.hold('POST /auth/exchange');
    await field.fill(SLT);
    await page.getByRole('button', { name: 'Sign in as Silicon' }).click();
    await until(() => backend.sent('POST /auth/exchange').length === 1, 'the token exchange');
    assert.equal(await field.inputValue(), '', 'the token leaves the field before the request finishes');
    assert.equal(await page.getByRole('button', { name: 'Continue as Carbon' }).isDisabled(), true, 'controls are disabled while busy');
    await page.getByText('Enter the short-lived token').waitFor({ state: 'detached' }); // The notice clears when a task starts.
    release();
    await heading(page, 'Sessions').waitFor();
    await idle(page);
    const [exchange] = backend.sent('POST /auth/exchange');
    assert.deepEqual(exchange.body, { short_lived_token: SLT, contract_version: 2 });
    assert.equal(exchange.authorization, undefined);
    assert.equal(backend.sent('GET /sessions')[0].authorization, `Bearer ${ATLAS_AUTH.access_token}`);
    assert(!(await storageSnapshot(page)).includes(SLT), 'the short-lived token is not kept in storage');
    const stored = await savedAccounts(page);
    assert.equal(stored.active, `${ATLAS.id}/browser`);
    assert.deepEqual(stored.sessions.map(item => item.identity), [ATLAS]);
    const account = page.getByRole('complementary', { name: 'Browser' }).getByRole('button', { name: /Atlas/ });
    assert.equal(await account.getAttribute('aria-current'), 'true');
    assert.equal(await page.getByRole('table', { name: 'Sessions' }).getByRole('row').count(), 3, 'header plus the two sessions Atlas can see');
  });

  await scenario('Carbon sign-in: the Accounts popup can be cancelled, then hands its code back to this tab', {}, async ({ page, context, backend }) => {
    await page.goto(`${origin}/`, { waitUntil: 'load' });
    let opened = context.waitForEvent('page');
    await page.getByRole('button', { name: 'Continue as Carbon' }).click();
    let popup = await opened;
    await popup.getByRole('heading', { name: 'Silicon Accounts' }).waitFor();
    const cancel = page.getByRole('button', { name: 'Cancel sign-in' });
    assert.equal(await cancel.isEnabled(), true, 'Cancel sign-in is never disabled by busy');
    assert.equal(await page.getByRole('button', { name: 'Continue as Silicon' }).isDisabled(), true);
    await cancel.click();
    await page.getByText('Sign-in cancelled.').waitFor();
    await until(() => popup.isClosed(), 'the cancelled popup to close');

    opened = context.waitForEvent('page');
    await page.getByRole('button', { name: 'Continue as Carbon' }).click();
    popup = await opened;
    const closed = popup.waitForEvent('close');
    await popup.getByRole('link', { name: 'Approve Browser' }).click();
    await closed;
    await heading(page, 'Sessions').waitFor();
    await idle(page);
    assert.deepEqual(backend.sent('POST /auth/attempts').map(item => item.body), [{ identity_kind: 'carbon' }, { identity_kind: 'carbon' }]);
    const exchanges = backend.sent(`POST /auth/attempts/${ATTEMPT}/exchange`);
    assert.equal(exchanges.length, 1, 'the popup code is exchanged once, by this tab');
    assert.deepEqual(exchanges[0].body, { state: STATE, code: 'sac_popup-code_42', contract_version: 2 });
    assert.equal((await savedAccounts(page)).active, `${ADA.id}/browser`);
    await page.getByRole('button', { name: `Account menu, ${ADA.name}` }).waitFor();
  });

  await scenario('sessions: search, sort, detail, copy, live view, end session, and new session', { accounts: [ADA_AUTH] }, async ({ page, backend }) => {
    await page.goto(`${origin}/`, { waitUntil: 'load' });
    await heading(page, 'Sessions').waitFor();
    await idle(page);
    const table = page.getByRole('table', { name: 'Sessions' });
    assert.equal(await table.getByRole('row').count(), 4);
    const search = page.getByRole('searchbox', { name: 'Find a session' });
    await search.fill('checkout');
    assert.equal(await table.getByRole('row').count(), 2);
    await search.fill('expired');
    await table.getByText('Supplier onboarding').waitFor();
    await search.fill('nothing like this');
    await page.getByRole('heading', { name: 'No sessions yet' }).waitFor();
    await page.getByRole('button', { name: 'Start your first session' }).waitFor();
    await search.fill('');
    await table.getByRole('button', { name: 'Sort by Usage' }).click();
    const firstNames = async () => (await table.locator('tbody tr').allInnerTexts()).map(text => text.split('\n')[0]);
    assert.deepEqual(await firstNames(), ['Checkout flow audit', 'Vendor research', 'Supplier onboarding'], 'cheapest first, Pending last');

    await table.getByRole('button', { name: 'Open Vendor research' }).click();
    await heading(page, 'Vendor research').waitFor();
    await idle(page);
    const details = main(page);
    await details.getByText(`${ADA.id}, ${ATLAS.id}`, { exact: true }).waitFor();
    await details.getByText('0.0123 USD').waitFor();
    await details.getByText(`browser run ${S1} snapshot`).waitFor();
    await details.getByRole('button', { name: 'Copy Session ID' }).click();
    await page.getByText('Copy Session ID: Copied').waitFor({ state: 'attached' });
    assert.equal(await page.evaluate(() => navigator.clipboard.readText()), S1);

    await details.getByRole('button', { name: 'Open live browser' }).click();
    await assertLiveFrame(page, S1);
    await heading(page, 'Vendor research').waitFor();
    assert.equal(page.url(), `${origin}/`, 'the self-issued live link never reaches the address bar');
    assert.deepEqual(backend.sent(`POST /sessions/${S1}/live`).map(item => item.body), [undefined]);
    assert.deepEqual(backend.sent(`POST /sessions/${S1}/live/redeem`).map(item => item.body), [{ grant: LIVE_GRANT }]);
    await page.getByRole('button', { name: 'Fullscreen' }).waitFor();
    await page.getByRole('button', { name: 'Session details' }).click();
    await heading(page, 'Vendor research').waitFor();
    await idle(page);

    await page.getByLabel('Closing note').fill('Compared all three vendors.');
    await page.getByRole('button', { name: 'End session' }).click();
    await toast(page, 'Session ended').waitFor();
    await idle(page);
    assert.deepEqual(backend.sent(`POST /sessions/${S1}/end`).map(item => item.body), [{ note: 'Compared all three vendors.' }]);
    await main(page).getByText('complete', { exact: true }).waitFor();
    assert.equal(await page.getByRole('button', { name: 'Open live browser' }).count(), 0);
    assert.equal(await page.getByLabel('Closing note').count(), 0);

    await openTab(page, 'Sessions');
    await page.getByRole('button', { name: 'New session' }).click();
    await heading(page, 'New session').waitFor();
    await page.getByLabel('Session name').fill('  Pricing review  ');
    await page.getByLabel('Description').fill('  Check the new pricing tiers.  ');
    await page.getByRole('combobox', { name: 'Session length' }).click();
    await page.getByRole('option', { name: /30 minutes/ }).click();
    const release = backend.hold('POST /sessions');
    await page.getByRole('button', { name: 'Start session' }).click();
    await until(() => backend.sent('POST /sessions').length === 1, 'the create request');
    for (const control of await main(page).locator('button, input:not([type="hidden"]), textarea').all()) assert.equal(await control.isDisabled(), true, `every form control is disabled while busy: ${await control.evaluate(element => element.outerHTML.slice(0, 120))}`);
    release();
    await heading(page, 'Pricing review').waitFor();
    assert.deepEqual(backend.sent('POST /sessions')[0].body, { name: 'Pricing review', description: 'Check the new pricing tiers.', ttl: '30m', incognito: true });
  });

  await scenario('profiles: cards, search, start from a profile, create, edit, and retire', { accounts: [ADA_AUTH] }, async ({ page, backend }) => {
    await page.goto(`${origin}/`, { waitUntil: 'load' });
    await heading(page, 'Sessions').waitFor();
    await openTab(page, 'Profiles');
    await heading(page, 'Profiles').waitFor();
    const grid = page.getByRole('list', { name: 'Profiles' });
    assert.equal(await grid.getByRole('listitem').count(), 2);
    const research = grid.getByRole('listitem').filter({ hasText: 'Research workspace' });
    await research.getByText('Germany · 4 sessions').waitFor();
    await research.getByText(ATLAS.id).waitFor();
    const retail = grid.getByRole('listitem').filter({ hasText: 'Retail checkout' });
    await retail.getByText('Owner only').waitFor();
    assert.equal(await retail.getByRole('button').count(), 0, 'retired profiles have no actions');
    await page.getByRole('searchbox', { name: 'Find a profile' }).fill('united states');
    assert.equal(await grid.getByRole('listitem').count(), 1);
    await page.getByRole('searchbox', { name: 'Find a profile' }).fill('');

    await research.getByRole('button', { name: 'Start session with Research workspace' }).click();
    await heading(page, 'Start Research workspace').waitFor();
    await page.getByText('Continue with your saved browser identity.').waitFor();
    await page.getByLabel('Session name').fill('Weekly research');
    await page.getByLabel('Description').fill('Continue where we left off.');
    await page.getByRole('button', { name: 'Start session' }).click();
    await heading(page, 'Weekly research').waitFor();
    assert.deepEqual(backend.sent('POST /sessions')[0].body, { name: 'Weekly research', description: 'Continue where we left off.', ttl: '15m', incognito: false, profile_id: 'prf_research' });

    await openTab(page, 'Profiles');
    await page.getByRole('button', { name: 'New profile' }).click();
    await heading(page, 'New profile').waitFor();
    await page.getByLabel('Profile name').fill('  Shopping  ');
    await page.getByRole('combobox', { name: 'Location' }).click();
    await page.getByRole('option', { name: /United Kingdom/ }).click();
    const tags = page.getByLabel('Additional access');
    await tags.fill(ATLAS.id);
    await tags.press('Enter');
    await tags.fill(ATLAS.id.toUpperCase());
    await tags.press('Enter');
    await page.getByRole('button', { name: `Remove ${ATLAS.id}` }).waitFor();
    // A UUID still being typed (no Enter) is saved too, and one ordinary press of Save is enough: committing the draft on blur
    // must not move the button between mousedown and mouseup (a 300 ms press, like a deliberate click).
    await tags.fill(` ${EXTRA_ACCESS} `);
    await page.getByRole('button', { name: 'Save profile' }).click({ delay: 300 });
    await toast(page, 'Profile saved', 'Shopping').waitFor();
    await heading(page, 'Profiles').waitFor();
    await idle(page);
    assert.deepEqual(backend.sent('POST /profiles').map(item => item.body), [{ name: 'Shopping', access: [ATLAS.id, EXTRA_ACCESS], location: 'gb' }], 'one click saves once, with the pending draft');
    assert.equal(await grid.getByRole('listitem').count(), 3);

    await research.getByRole('button', { name: 'Edit Research workspace' }).click();
    await heading(page, 'Edit profile').waitFor();
    assert.equal(await page.getByLabel('Profile name').inputValue(), 'Research workspace');
    await main(page).getByText('Germany').waitFor();
    assert.equal(await page.getByRole('combobox', { name: 'Location' }).count(), 0, 'the location is fixed after creation');
    await page.getByRole('button', { name: `Remove ${ATLAS.id}` }).click();
    await page.getByLabel('Profile name').fill('Research desk');
    await page.getByRole('button', { name: 'Save profile' }).click();
    await toast(page, 'Profile saved', 'Research desk').waitFor();
    await heading(page, 'Profiles').waitFor();
    await idle(page);
    assert.deepEqual(backend.sent('PATCH /profiles/prf_research').map(item => item.body), [{ name: 'Research desk', access: [] }]);

    await grid.getByRole('button', { name: 'Edit Research desk' }).click();
    await heading(page, 'Edit profile').waitFor();
    await page.getByLabel('Reason').fill('No longer needed.');
    await page.getByRole('button', { name: 'Retire profile' }).click();
    await toast(page, 'Profile retired').waitFor();
    await heading(page, 'Profiles').waitFor();
    await idle(page);
    assert.deepEqual(backend.sent('POST /profiles/prf_research/end').map(item => item.body), [{ note: 'No longer needed.' }]);
    assert.equal(await grid.getByRole('listitem').filter({ hasText: 'Research desk' }).getByRole('button').count(), 0);
  });

  await scenario('recordings, usage, and settings: polling, safe links, hide, totals, and account details', { accounts: [ADA_AUTH] }, async ({ page, backend }) => {
    await page.goto(`${origin}/`, { waitUntil: 'load' });
    await heading(page, 'Sessions').waitFor();
    await openTab(page, 'Recordings');
    await heading(page, 'Recordings').waitFor();
    const list = page.getByRole('list', { name: 'Recordings' });
    assert.deepEqual((await list.getByRole('heading', { level: 3 }).allInnerTexts()), ['Vendor research', 'Checkout flow audit', 'Supplier onboarding'], 'newest first');
    await list.getByText('This recording is not available yet. Refresh to check its status.').waitFor();
    await list.getByText('The browser service did not provide a recording for this session.').waitFor();
    await list.getByText(/· 21 min · Size unavailable$/).waitFor();
    const links = list.getByRole('link', { name: /Open recording/ });
    assert.equal(await links.count(), 1, 'only the credential-free https link renders');
    assert.deepEqual(await links.first().evaluate(link => [link.getAttribute('href'), link.target, link.rel, link.getAttribute('referrerpolicy')]), ['https://briefcase.example/recordings/checkout-flow-audit', '_blank', 'noopener noreferrer', 'no-referrer']);
    // The 10 s background refresh runs while a recording is pending, and picks up the now-available recording.
    await until(() => backend.sent('GET /recordings').length >= 2, 'the background recordings refresh', 13000);
    await until(async () => await links.count() === 2, 'the refreshed recording link');
    await idle(page);

    const audit = list.getByRole('listitem').filter({ hasText: 'Checkout flow audit' });
    await audit.getByRole('button', { name: 'Hide' }).click();
    await toast(page, 'Recording hidden').waitFor();
    await idle(page);
    assert.deepEqual(backend.sent(`POST /recordings/${S2}/trash`).length, 1);
    await audit.waitFor({ state: 'detached' });

    await openTab(page, 'Usage');
    await heading(page, 'Usage').waitFor();
    const metrics = main(page);
    // The counters roll digit columns; what is read out (the accessibility tree) is the formatted value.
    await until(async () => (await metrics.locator('dl').first().ariaSnapshot()).includes('definition: 33.3 min'), 'the browser time total');
    assert.deepEqual((await metrics.locator('dl').first().ariaSnapshot()).split('\n').map(line => line.replace(/^- /, '').replaceAll('"', '')), [
      'term: Browser time', 'definition: 33.3 min', 'definition: Open browser time across these sessions',
      'term: Proxy traffic', 'definition: 1.23 GB', 'definition: Data in and out through the proxy',
      'term: Visible sessions', 'definition: 3', 'definition: Sessions you can access',
    ]);
    await metrics.getByText(/4 concurrent browsers/).waitFor();
    const usageTable = page.getByRole('table', { name: 'Usage by session' });
    assert.equal(await usageTable.getByRole('row').count(), 4);
    await usageTable.getByText('Pending').waitFor();

    const release = backend.hold('GET /usage');
    await page.getByRole('button', { name: 'Refresh' }).click();
    await until(() => backend.sent('GET /usage').length === 2, 'the usage refresh');
    for (const button of await page.locator('aside button').all()) assert.equal(await button.isDisabled(), true, 'rail buttons are disabled while busy');
    release();
    await idle(page);

    await openTab(page, 'Settings');
    await heading(page, 'Settings').waitFor();
    await main(page).getByText(ADA.id, { exact: true }).waitFor();
    await main(page).getByText('Carbon', { exact: true }).waitFor();
    const manage = page.getByRole('link', { name: /Manage your account/ });
    assert.deepEqual(await manage.evaluate(link => [link.getAttribute('href'), link.target, link.rel, link.getAttribute('referrerpolicy')]), ['https://accounts.teamofsilicons.com/', '_blank', 'noopener noreferrer', 'no-referrer']);
  });

  await scenario('account menu, theme, account switching, and signing out only the current account', { accounts: [ADA_AUTH, ATLAS_AUTH] }, async ({ page, backend }) => {
    await page.goto(`${origin}/`, { waitUntil: 'load' });
    await heading(page, 'Sessions').waitFor();
    await idle(page);
    const rail = page.getByRole('complementary', { name: 'Browser' });
    assert.equal(await rail.getByRole('button', { name: /Ada Lovelace/ }).getAttribute('aria-current'), 'true');
    assert.equal(await rail.getByRole('button', { name: /Atlas/ }).getAttribute('aria-current'), null);
    assert.equal(await rail.getByRole('region', { name: 'Accounts and workspace' }).getAttribute('tabindex'), '0', 'the scrollable rail is a named tab stop');

    // Signed in, "Add an account" opens right after the page header (the h1 stays the first heading), with one set of sign-in controls.
    await rail.getByRole('button', { name: 'Add account' }).click();
    await page.getByRole('heading', { level: 2, name: 'Add an account' }).waitFor();
    assert.deepEqual(await main(page).evaluate(node => [...node.querySelectorAll('h1, h2')].slice(0, 2).map(item => `${item.tagName} ${item.textContent}`)), ['H1 Sessions', 'H2 Add an account']);
    assert.equal(await page.getByRole('button', { name: 'Continue as Carbon' }).count(), 1);
    await page.getByRole('button', { name: 'Cancel', exact: true }).click();
    await page.getByRole('heading', { name: 'Add an account' }).waitFor({ state: 'detached' });

    const menuButton = page.getByRole('button', { name: `Account menu, ${ADA.name}` });
    await menuButton.click();
    const menu = page.getByRole('menu');
    await menu.waitFor();
    await menu.getByText('Carbon account').waitFor();
    assert.equal(await menu.getByText(ADA.id).count(), 0, 'the menu never shows the account UUID');
    await menu.getByRole('menuitemradio', { name: 'Dark' }).click();
    await page.waitForFunction(() => document.documentElement.dataset.theme === 'dark');
    assert.equal(await page.evaluate(key => localStorage.getItem(key), THEME_KEY), 'dark');
    await menu.getByRole('menuitemradio', { name: 'System' }).click();
    await page.waitForFunction(() => document.documentElement.dataset.theme === 'light');
    assert.equal(await page.evaluate(key => localStorage.getItem(key), THEME_KEY), null, 'System clears the stored choice');
    await menu.getByRole('menuitemradio', { name: 'Dark' }).click();
    await menu.getByRole('menuitem', { name: 'Settings' }).click();
    await heading(page, 'Settings').waitFor();
    await idle(page);

    // The stored theme is applied by boot.js before the app (or first paint) on the next load.
    await page.reload({ waitUntil: 'domcontentloaded' });
    assert.equal(await page.evaluate(() => document.documentElement.dataset.theme), 'dark');
    await heading(page, 'Sessions').waitFor();
    await idle(page);

    await rail.getByRole('button', { name: /Atlas/ }).click();
    await page.getByRole('button', { name: `Account menu, ${ATLAS.name}` }).waitFor();
    await idle(page);
    assert.equal(await rail.getByRole('button', { name: /Atlas/ }).getAttribute('aria-current'), 'true');
    assert.equal(backend.sent('GET /sessions').at(-1).authorization, `Bearer ${ATLAS_AUTH.access_token}`);
    assert.equal(await page.getByRole('table', { name: 'Sessions' }).getByRole('row').count(), 3);

    await page.getByRole('button', { name: `Account menu, ${ATLAS.name}` }).click();
    await page.getByRole('menu').getByRole('menuitem', { name: 'Sign out' }).click();
    await heading(page, 'A browser, ready when you are.').waitFor();
    assert.equal(await page.getByRole('button', { name: /Account menu/ }).count(), 0);
    const stored = await savedAccounts(page);
    assert.equal(stored.active, null);
    assert.deepEqual(stored.sessions.map(item => item.identity.id), [ADA.id], 'only the current account signs out');
    assert.equal(await rail.getByRole('button', { name: /Atlas/ }).count(), 0);
    // Signed out, "Add account" takes you to the Welcome sign-in instead of opening a second panel of sign-in controls.
    await rail.getByRole('button', { name: 'Add account' }).click();
    await until(() => page.getByRole('button', { name: 'Continue as Carbon' }).evaluate(button => button === document.activeElement), 'focus on the Welcome sign-in');
    assert.equal(await page.getByRole('heading', { name: 'Add an account' }).count(), 0);
    assert.equal(await page.getByRole('button', { name: 'Continue as Carbon' }).count(), 1);
    await rail.getByRole('button', { name: /Ada Lovelace/ }).click();
    await heading(page, 'Sessions').waitFor();
    await page.getByRole('button', { name: `Account menu, ${ADA.name}` }).waitFor();
  });

  await scenario('callback: ?state=&code= leave the address bar at once, never return, and never restore a saved session', { accounts: [ADA_AUTH] }, async ({ page, backend }) => {
    await page.goto(`${origin}/docs`, { waitUntil: 'load' });
    await page.goto(`${origin}/auth/callback?state=${STATE}&code=sac_secret-code`, { waitUntil: 'commit' });
    await page.waitForURL(`${origin}/auth/callback`);
    await heading(page, 'Sign in to Browser').waitFor();
    assert.equal(page.url(), `${origin}/auth/callback`);
    await page.getByText('This sign-in link is missing its original attempt.').waitFor();
    assert.equal(await page.evaluate(() => '__sbEntry' in window), false, 'the captured entry URL is read once and dropped');
    assert.equal(await page.evaluate(() => window.next?.router?.asPath), '/auth/callback');
    assert(!(await page.content()).includes('sac_secret-code'));
    assert.deepEqual(backend.sent(), [], 'the callback page makes no requests with the saved session');
    assert.equal(await page.getByRole('navigation', { name: 'Workspace' }).count(), 0);
    await page.goBack({ waitUntil: 'load' });
    assert.equal(page.url(), `${origin}/docs`);
    await page.goForward({ waitUntil: 'load' });
    assert.equal(page.url(), `${origin}/auth/callback`, 'Forward does not restore the code');
    await heading(page, 'Sign in to Browser').waitFor();
    assert.deepEqual(backend.sent(), []);

    await page.goto(`${origin}/auth/callback?error=access_denied&state=${STATE}`, { waitUntil: 'load' });
    assert.equal(page.url(), `${origin}/auth/callback`);
    await page.getByText('Sign-in was not completed.').waitFor();
    assert.deepEqual(backend.sent(), []);
  });

  await scenario('callback: a same-tab (popup-blocked) sign-in exchanges its code once and opens the workspace', { redirectAttempt: { attempt_id: ATTEMPT, state: STATE } }, async ({ page, backend }) => {
    await page.goto(`${origin}/docs`, { waitUntil: 'load' });
    await page.goto(`${origin}/auth/callback?state=${STATE}&code=sac_redirect-code`, { waitUntil: 'commit' });
    await heading(page, 'Sessions').waitFor();
    await idle(page);
    assert.equal(page.url(), `${origin}/`);
    assert.deepEqual(backend.sent(`POST /auth/attempts/${ATTEMPT}/exchange`).map(item => item.body), [{ state: STATE, code: 'sac_redirect-code', contract_version: 2 }]);
    assert.equal(await page.evaluate(key => sessionStorage.getItem(key), REDIRECT_KEY), null, 'the redirect attempt is used once');
    await page.goBack({ waitUntil: 'load' });
    assert.equal(page.url(), `${origin}/docs`, 'the callback entry was replaced, so Back skips it');
    await page.goForward({ waitUntil: 'load' });
    assert(!page.url().includes('code'), 'Forward never restores the code');
  });

  await scenario('live invitation: #grant= is stripped, waits for sign-in, then is redeemed once in the exact iframe', {}, async ({ page, backend }) => {
    await page.goto(`${origin}/docs`, { waitUntil: 'load' });
    await page.goto(`${origin}/sessions/${S1}/live#grant=invite-grant-7c1e`, { waitUntil: 'commit' });
    await page.waitForURL(`${origin}/sessions/${S1}/live`);
    await heading(page, 'A browser, ready when you are.').waitFor();
    await page.getByText('You have a live browser invitation. Sign in to continue.').waitFor();
    assert.equal(page.url(), `${origin}/sessions/${S1}/live`);
    assert.deepEqual(backend.sent(), []);
    await page.getByRole('button', { name: 'Continue as Silicon' }).click();
    await page.getByLabel('Short-lived token', { exact: true }).fill(SLT);
    await page.getByRole('button', { name: 'Sign in as Silicon' }).click();
    await assertLiveFrame(page, S1);
    await heading(page, 'Vendor research').waitFor();
    await idle(page);
    assert.deepEqual(backend.sent(`POST /sessions/${S1}/live/redeem`).map(item => [item.body, item.authorization]), [[{ grant: 'invite-grant-7c1e' }, `Bearer ${ATLAS_AUTH.access_token}`]]);
    assert.equal(backend.sent(`POST /sessions/${S1}/live`).length, 0, 'an invitation is redeemed directly');
    assert(!page.url().includes('grant'));
    await page.goBack({ waitUntil: 'load' });
    assert(!page.url().includes('grant'), 'Back never restores the grant');
    await page.goForward({ waitUntil: 'load' });
    assert(!page.url().includes('grant'), 'Forward never restores the grant');
  });

  await scenario('live invitation while signed in: redeemed once with the saved account', { accounts: [ADA_AUTH] }, async ({ page, backend }) => {
    await page.goto(`${origin}/sessions/${S1}/live#grant=invite-grant-9a0b`, { waitUntil: 'commit' });
    await page.waitForURL(`${origin}/sessions/${S1}/live`);
    await assertLiveFrame(page, S1);
    await idle(page);
    assert.deepEqual(backend.sent(`POST /sessions/${S1}/live/redeem`).map(item => item.body), [{ grant: 'invite-grant-9a0b' }]);
    assert.equal(backend.sent('GET /sessions').length, 0, 'the invitation opens instead of the sessions list');
  });

  await scenario('mobile 390x844: the drawer navigates every tab without sideways scrolling', { accounts: [ADA_AUTH, ATLAS_AUTH], viewport: { width: 390, height: 844 } }, async ({ page }) => {
    await page.goto(`${origin}/`, { waitUntil: 'load' });
    await heading(page, 'Sessions').waitFor();
    await idle(page);
    assert.equal(await page.getByRole('complementary', { name: 'Browser' }).isVisible(), false, 'the rail collapses below 720px');
    await noHorizontalScroll(page, 'sessions');
    for (const [tab, title] of [['Profiles', 'Profiles'], ['Recordings', 'Recordings'], ['Usage', 'Usage'], ['Settings', 'Settings'], ['Sessions', 'Sessions']]) {
      await openTab(page, tab);
      await heading(page, title).waitFor();
      await noHorizontalScroll(page, tab);
    }
    await page.getByRole('button', { name: 'Open navigation' }).click();
    const drawer = page.getByRole('dialog', { name: 'Browser' });
    await drawer.getByRole('button', { name: /Atlas/ }).waitFor();
    assert.equal(await drawer.getByRole('button', { name: 'Sessions', exact: true }).getAttribute('aria-current'), 'page');
    await page.keyboard.press('Escape');
    await drawer.waitFor({ state: 'detached' });
    await page.getByRole('table', { name: 'Sessions' }).getByRole('button', { name: 'Open Vendor research' }).click();
    await heading(page, 'Vendor research').waitFor();
    await noHorizontalScroll(page, 'session detail');
  });

  await scenario('mobile drawer: focus outlines fit, it closes past 720px, and signed out "Add account" goes to the Welcome sign-in', { accounts: [ADA_AUTH, ATLAS_AUTH], viewport: { width: 390, height: 844 } }, async ({ page }) => {
    await page.goto(`${origin}/`, { waitUntil: 'load' });
    await heading(page, 'Sessions').waitFor();
    await idle(page);
    await page.getByRole('button', { name: 'Open navigation' }).click();
    const drawer = page.getByRole('dialog', { name: 'Browser' });
    const list = drawer.getByRole('region', { name: 'Accounts and workspace' });
    await list.waitFor();
    // The list clips at its padding box: the first and last buttons keep room for the 2px outline drawn 2px outside them.
    const room = await list.evaluate(viewport => {
      viewport.scrollTop = viewport.scrollHeight;
      const box = viewport.getBoundingClientRect(), buttons = viewport.querySelectorAll('button');
      return [buttons[0].getBoundingClientRect().top - box.top, box.bottom - buttons[buttons.length - 1].getBoundingClientRect().bottom];
    });
    assert(room.every(gap => gap >= 4), `focus outlines fit inside the drawer list (room ${room.join(', ')}px)`);

    await page.setViewportSize({ width: 1100, height: 844 });
    await drawer.waitFor({ state: 'detached' });
    await workspaceNav(page).waitFor();
    await page.setViewportSize({ width: 390, height: 844 });
    await page.getByRole('button', { name: 'Open navigation' }).waitFor();
    await page.waitForTimeout(300);
    assert.equal(await drawer.count(), 0, 'the drawer stays closed when the window narrows again');

    await page.getByRole('button', { name: `Account menu, ${ADA.name}` }).click();
    await page.getByRole('menu').getByRole('menuitem', { name: 'Sign out' }).click();
    await heading(page, 'A browser, ready when you are.').waitFor();
    await page.getByRole('button', { name: 'Open navigation' }).click();
    await drawer.getByRole('button', { name: 'Add account' }).click();
    await drawer.waitFor({ state: 'detached' });
    await until(() => page.getByRole('button', { name: 'Continue as Carbon' }).evaluate(button => button === document.activeElement), 'focus on the Welcome sign-in after the drawer closes');
    assert.equal(await page.getByRole('heading', { name: 'Add an account' }).count(), 0);
    assert.equal(await page.getByRole('button', { name: 'Continue as Carbon' }).count(), 1);
    await noHorizontalScroll(page, 'welcome after the drawer');
  });

  await scenario('unknown routes get the branded 404 with the production headers', { allowConsole: [/status of 404/] }, async ({ page }) => {
    const response = await page.goto(`${origin}/sessions/s1`, { waitUntil: 'load' });
    assert.equal(response.status(), 404);
    assert.equal(response.headers()['content-security-policy'], CSP);
    await heading(page, 'This page does not exist.').waitFor();
  });

  // Every screen at desktop, tablet (the 1000px and 720px breakpoints), and phone widths, light and dark
  // (screenshots when E2E_SCREENSHOTS is set; sideways scrolling is always checked).
  for (const [width, height, themes] of [[1440, 900, ['light', 'dark']], [390, 844, ['light', 'dark']], [1024, 768, ['light', 'dark']], [768, 1024, ['light', 'dark']]]) {
    for (const theme of themes) {
      const tag = `${width}-${theme}`;
      const common = { theme, viewport: { width, height }, reducedMotion: 'reduce', colorScheme: theme };
      await scenario(`gallery ${tag}: signed out, callback, and 404`, { ...common, allowConsole: [/status of 404/] }, async ({ page }) => {
        await page.goto(`${origin}/`, { waitUntil: 'load' });
        await heading(page, 'A browser, ready when you are.').waitFor();
        assert.equal(await page.evaluate(() => document.documentElement.dataset.theme), theme);
        await shoot(page, `welcome-${tag}`);
        await page.getByRole('button', { name: 'Continue as Silicon' }).click();
        await page.getByLabel('Short-lived token', { exact: true }).waitFor();
        await shoot(page, `welcome-silicon-${tag}`);
        await page.goto(`${origin}/auth/callback?state=${STATE}&code=sac_x`, { waitUntil: 'load' });
        await heading(page, 'Sign in to Browser').waitFor();
        await shoot(page, `callback-${tag}`);
        await page.goto(`${origin}/sessions/s1`, { waitUntil: 'load' });
        await heading(page, 'This page does not exist.').waitFor();
        await shoot(page, `not-found-${tag}`);
      });

      await scenario(`gallery ${tag}: workspace screens`, { ...common, accounts: [ADA_AUTH, ATLAS_AUTH], allowConsole: [/status of 503/] }, async ({ page, backend }) => {
        await page.goto(`${origin}/`, { waitUntil: 'load' });
        await heading(page, 'Sessions').waitFor();
        await idle(page);
        await shoot(page, `sessions-${tag}`);
        await page.getByRole('table', { name: 'Sessions' }).getByRole('button', { name: 'Open Vendor research' }).click();
        await heading(page, 'Vendor research').waitFor();
        await idle(page);
        await shoot(page, `session-detail-${tag}`);
        await page.getByRole('button', { name: 'Open live browser' }).click();
        await page.locator('iframe').waitFor();
        await idle(page);
        await shoot(page, `live-${tag}`);
        await openTab(page, 'Sessions');
        await page.getByRole('button', { name: 'New session' }).click();
        await heading(page, 'New session').waitFor();
        await shoot(page, `new-session-${tag}`);
        await page.getByRole('combobox', { name: 'Session length' }).click();
        await page.getByRole('option', { name: /30 minutes/ }).waitFor();
        await shoot(page, `new-session-select-${tag}`);
        await page.keyboard.press('Escape');
        await openTab(page, 'Profiles');
        await heading(page, 'Profiles').waitFor();
        await shoot(page, `profiles-${tag}`);
        await page.getByRole('button', { name: 'New profile' }).click();
        await heading(page, 'New profile').waitFor();
        await page.getByLabel('Additional access').fill(ATLAS.id);
        await page.getByLabel('Additional access').press('Enter');
        await shoot(page, `new-profile-${tag}`);
        await openTab(page, 'Profiles');
        await page.getByRole('button', { name: 'Edit Research workspace' }).click();
        await heading(page, 'Edit profile').waitFor();
        await shoot(page, `edit-profile-${tag}`);
        await openTab(page, 'Recordings');
        await heading(page, 'Recordings').waitFor();
        await shoot(page, `recordings-${tag}`);
        await openTab(page, 'Usage');
        await heading(page, 'Usage').waitFor();
        await main(page).getByText('33.3 min', { exact: true }).waitFor();
        await shoot(page, `usage-${tag}`);
        await openTab(page, 'Settings');
        await heading(page, 'Settings').waitFor();
        await shoot(page, `settings-${tag}`);
        await page.getByRole('button', { name: `Account menu, ${ADA.name}` }).click();
        await page.getByRole('menu').waitFor();
        await shoot(page, `account-menu-${tag}`);
        await page.keyboard.press('Escape');
        await page.getByRole('menu').waitFor({ state: 'detached' });
        if (width < 720) {
          await page.getByRole('button', { name: 'Open navigation' }).click();
          await page.getByRole('dialog', { name: 'Browser' }).waitFor();
          await shoot(page, `drawer-${tag}`);
          await page.getByRole('dialog', { name: 'Browser' }).getByRole('button', { name: 'Add account' }).click();
          await page.getByRole('dialog', { name: 'Browser' }).waitFor({ state: 'detached' });
        } else await page.getByRole('complementary', { name: 'Browser' }).getByRole('button', { name: 'Add account' }).click();
        await page.getByRole('heading', { level: 2, name: 'Add an account' }).waitFor();
        await shoot(page, `add-account-${tag}`);
        await page.getByRole('button', { name: 'Cancel', exact: true }).click();
        await openTab(page, 'Sessions');
        backend.fail('GET /sessions', 503, 'The browser service is busy. Try again in a minute.');
        await page.getByRole('button', { name: 'Refresh' }).click();
        await page.getByText('The browser service is busy.').waitFor();
        await idle(page);
        await shoot(page, `notice-${tag}`);
      });

      await scenario(`gallery ${tag}: empty states`, { ...common, accounts: [ADA_AUTH], data: EMPTY, failLimits: true, allowConsole: [/status of 503/] }, async ({ page }) => {
        await page.goto(`${origin}/`, { waitUntil: 'load' });
        await heading(page, 'Sessions').waitFor();
        await page.getByRole('heading', { name: 'No sessions yet' }).waitFor();
        await idle(page);
        await shoot(page, `sessions-empty-${tag}`);
        for (const [tab, title] of [['Profiles', 'No profiles yet'], ['Recordings', 'Nothing recorded yet'], ['Usage', 'No session usage yet']]) {
          await openTab(page, tab);
          await page.getByRole('heading', { name: title }).waitFor();
          await shoot(page, `${tab.toLowerCase()}-empty-${tag}`);
        }
        await main(page).getByText('Service capacity is temporarily unavailable.').waitFor();
      });
    }
  }
} finally {
  await browser.close();
  preview.kill();
}

console.log(results.join('\n'));
const failed = results.filter(line => line.startsWith('FAIL')).length;
console.log(`\n${results.length - failed} passed, ${failed} failed${SHOTS ? `; ${screenshots.length} screenshots in ${SHOTS}` : ''}`);
