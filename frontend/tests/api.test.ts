import test from 'node:test';
import assert from 'node:assert/strict';
import { BrowserApi, acceptAuth, dateForApi, publicError, safeHttps, shellQuote } from '../src/api';
import type { AuthSession } from '../src/types';
const session = (access = 'oat_initial'): AuthSession => ({ access_token: access, refresh_token: 'ort_refresh', expires_at: new Date(Date.now() + 900000).toISOString(), identity: { id: '@tester', name: 'Tester', kind: 'carbon' }, org: { id: 'tos', name: 'tos' } });
const response = (data: unknown, status = 200, headers: Record<string, string> = {}) => new Response(JSON.stringify(status === 200 ? {data} : {error: {message: 'Unauthorized'}}), {status, headers});
function client(handler: (url: string, options: RequestInit) => Promise<Response>) { const api = new BrowserApi('https://backend.browser.teamofsilicons.com', ((url, options) => handler(String(url), options || {})) as typeof fetch); api.setSession(session()); return api; }

test('API sends authorization directly to configured backend, without cookies or redirects', async () => {
  const api = client(async (url, options) => { assert.equal(url, 'https://backend.browser.teamofsilicons.com/api/v1/profiles'); assert.equal(options.credentials, 'omit'); assert.equal(options.redirect, 'error'); assert.equal(options.cache, 'no-store'); assert.equal((options.headers as Record<string, string>).Authorization, 'Bearer oat_initial'); assert.equal((options.headers as Record<string, string>)['X-Org-Id'], 'tos'); return response([]); });
  assert.deepEqual(await api.call('/profiles'), []);
});
test('pre-handler 401 refreshes and replays a mutation exactly once', async () => {
  const calls: string[] = [];
  const api = client(async (url, options) => { calls.push(url); if (url.endsWith('/auth/refresh')) { assert.equal((options.headers as Record<string, string>).Authorization, undefined); return response(session('oat_new')); } if ((options.headers as Record<string, string>).Authorization === 'Bearer oat_initial') return response({}, 401, {'x-sb-auth-rejected':'1'}); assert.equal(options.method, 'POST'); assert.equal(options.body, '{"name":"test"}'); return response({id:'p1'}); });
  assert.deepEqual(await api.call('/profiles', 'POST', {name:'test'}), {id:'p1'}); assert.equal(calls.length, 3);
});
test('handler 401 and ambiguous network failures never replay a mutation', async () => {
  for (const network of [false, true]) { let calls = 0; const api = client(async () => { calls++; if (network) throw new Error('offline'); return response({}, 401); }); await assert.rejects(api.call('/sessions', 'POST', {})); assert.equal(calls, 1); }
});
test('concurrent authentication rejection uses one refresh operation', async () => {
  let refreshes = 0; const api = client(async (url, options) => { if (url.endsWith('/auth/refresh')) { refreshes++; await new Promise(resolve=>setTimeout(resolve, 5)); return response(session('oat_new')); } return (options.headers as Record<string, string>).Authorization === 'Bearer oat_initial' ? response({}, 401, {'x-sb-auth-rejected':'1'}) : response([]); });
  await Promise.all([api.call('/sessions'), api.call('/profiles')]); assert.equal(refreshes, 1);
});
test('sign-out during refresh cannot restore tokens or retry the pending mutation', async () => {
  let finish!: (value: Response) => void; let started!: () => void; const begun = new Promise<void>(resolve=>{started=resolve;}); let calls = 0;
  const api = client(async url => { calls++; if (url.endsWith('/auth/refresh')) { started(); return new Promise(resolve=>{finish=resolve;}); } return response({}, 401, {'x-sb-auth-rejected':'1'}); });
  const pending = api.call('/sessions', 'POST', {}); await begun; api.setSession(null); finish(response(session('oat_new'))); await assert.rejects(pending, /Sign-in changed/); assert.equal(api.currentSession(), null); assert.equal(calls, 2);
});
test('refresh response must preserve organization and identity', () => {
  assert.throws(()=>acceptAuth(session(), 'other')); assert.throws(()=>acceptAuth(session(), 'tos', '@someone-else'));
  assert.throws(()=>acceptAuth({...session(), access_token:'secret'}, 'tos')); assert.throws(()=>acceptAuth({...session(), expires_at:'invalid'}, 'tos'));
});
test('viewer links reject script, plaintext and embedded credentials', () => {
  for (const value of ['javascript:alert(1)', 'http://example.com', 'https://user:secret@example.com', 'not a url']) assert.equal(safeHttps(value), null);
  assert.equal(safeHttps('https://viewer.example/session?a=b'), 'https://viewer.example/session?a=b');
});
test('command export quotes shell literals and validates real calendar dates', () => {
  assert.equal(shellQuote("a'b $HOME"), "'a'\"'\"'b $HOME'"); assert.equal(dateForApi('2026-09-06'), '2026-09-06');
  for (const value of ['2026-02-30', '06-09-2026', '2026-9-6', 'bad']) assert.throws(()=>dateForApi(value));
});
test('provider names do not leak through displayed service errors', () => { assert.equal(publicError(new Error('Browser Use timed out; TinyFish unavailable')), 'browser service timed out; browser service unavailable'); });
test('default fetch preserves the native browser global receiver', async () => {
  const original = globalThis.fetch;
  try {
    globalThis.fetch = (function(this: unknown) { assert.equal(this, globalThis); return Promise.resolve(response([])); }) as typeof fetch;
    const api = new BrowserApi('https://backend.browser.teamofsilicons.com'); api.setSession(session());
    assert.deepEqual(await api.call('/sessions'), []);
  } finally { globalThis.fetch = original; }
});
