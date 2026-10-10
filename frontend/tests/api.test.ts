import test from 'node:test';
import assert from 'node:assert/strict';
import { BrowserApi, acceptAuth, dateForApi, publicError, safeHttps, shellQuote } from '../src/api';
import type { AuthSession } from '../src/types';
const session = (access = 'eyJheader.eyJpayload.initial'): AuthSession => ({ access_token: access, refresh_token: 'sar_refresh', expires_at: new Date(Date.now() + 900000).toISOString(), identity: { id: 'Ab1', name: 'Tester', kind: 'carbon' }, org: { id: 'browser', name: 'Browser' } });
const response = (data: unknown, status = 200, headers: Record<string, string> = {}) => new Response(JSON.stringify(status === 200 ? {data} : {error: {message: 'Unauthorized'}}), {status, headers});
function client(handler: (url: string, options: RequestInit) => Promise<Response>) { const api = new BrowserApi('https://backend.browser.teamofsilicons.com', ((url, options) => handler(String(url), options || {})) as typeof fetch); api.setSession(session()); return api; }

test('API sends authorization directly to configured backend, without cookies or redirects', async () => {
  const api = client(async (url, options) => { assert.equal(url, 'https://backend.browser.teamofsilicons.com/api/v1/profiles'); assert.equal(options.credentials, 'omit'); assert.equal(options.redirect, 'error'); assert.equal(options.cache, 'no-store'); assert.equal((options.headers as Record<string, string>).Authorization, 'Bearer eyJheader.eyJpayload.initial'); assert.equal((options.headers as Record<string, string>)['X-Org-Id'], 'browser'); return response([]); });
  assert.deepEqual(await api.call('/profiles'), []);
});
test('pre-handler 401 refreshes and replays a mutation exactly once', async () => {
  const calls: string[] = [];
  const api = client(async (url, options) => { calls.push(url); if (url.endsWith('/auth/refresh')) { assert.equal((options.headers as Record<string, string>).Authorization, undefined); return response(session('eyJheader.eyJpayload.new')); } if ((options.headers as Record<string, string>).Authorization === 'Bearer eyJheader.eyJpayload.initial') return response({}, 401, {'x-sb-auth-rejected':'1'}); assert.equal(options.method, 'POST'); assert.equal(options.body, '{"name":"test"}'); return response({id:'p1'}); });
  assert.deepEqual(await api.call('/profiles', 'POST', {name:'test'}), {id:'p1'}); assert.equal(calls.length, 3);
});
test('handler 401 and ambiguous network failures never replay a mutation', async () => {
  for (const network of [false, true]) { let calls = 0; const api = client(async () => { calls++; if (network) throw new Error('offline'); return response({}, 401); }); await assert.rejects(api.call('/sessions', 'POST', {})); assert.equal(calls, 1); }
});
test('concurrent authentication rejection uses one refresh operation', async () => {
  let refreshes = 0; const api = client(async (url, options) => { if (url.endsWith('/auth/refresh')) { refreshes++; await new Promise(resolve=>setTimeout(resolve, 5)); return response(session('eyJheader.eyJpayload.new')); } return (options.headers as Record<string, string>).Authorization === 'Bearer eyJheader.eyJpayload.initial' ? response({}, 401, {'x-sb-auth-rejected':'1'}) : response([]); });
  await Promise.all([api.call('/sessions'), api.call('/profiles')]); assert.equal(refreshes, 1);
});
test('sign-out during refresh cannot restore tokens or retry the pending mutation', async () => {
  let finish!: (value: Response) => void; let started!: () => void; const begun = new Promise<void>(resolve=>{started=resolve;}); let calls = 0;
  const api = client(async url => { calls++; if (url.endsWith('/auth/refresh')) { started(); return new Promise(resolve=>{finish=resolve;}); } return response({}, 401, {'x-sb-auth-rejected':'1'}); });
  const pending = api.call('/sessions', 'POST', {}); await begun; api.setSession(null); finish(response(session('eyJheader.eyJpayload.new'))); await assert.rejects(pending, /Sign-in changed/); assert.equal(api.currentSession(), null); assert.equal(calls, 2);
});
test('refresh response must preserve Browser workspace and stable account UUID', () => {
  assert.doesNotThrow(() => acceptAuth(session()));
  assert.throws(()=>acceptAuth(session(), 'other')); assert.throws(()=>acceptAuth(session(), 'browser', '@someone-else'));
  assert.throws(()=>acceptAuth({...session(), access_token:'secret'}, 'browser')); assert.throws(()=>acceptAuth({...session(), expires_at:'invalid'}, 'browser'));
  assert.throws(()=>acceptAuth({...session(), access_token:'eyJheader.eyJpayload.'}, 'browser')); assert.throws(()=>acceptAuth({...session(), refresh_token:'sar_bad\nheader'}, 'browser'));
  for (const identity of [{id:'ab', name:'Tester', kind:'carbon'}, {id:'worker:tos', name:'Worker', kind:'silicon'}, {id:'c:tester', name:'Tester', kind:'silicon'}]) {
    assert.throws(() => acceptAuth({...session(), identity: identity as AuthSession['identity']}), /sign in again/);
  }
});
test('viewer links reject script, plaintext and embedded credentials', () => {
  for (const value of ['javascript:alert(1)', 'http://example.com', 'https://user:secret@example.com', 'not a url']) assert.equal(safeHttps(value), null);
  assert.equal(safeHttps('https://viewer.example/session?a=b'), 'https://viewer.example/session?a=b');
  assert.equal(safeHttps('https://browser.teamofsilicons.com/session', 'https://browser.teamofsilicons.com'), null);
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


test('a failed refresh cannot resend a possibly spent Accounts refresh token', async () => {
  let refreshes = 0;
  const api = client(async url => {
    if (url.endsWith('/auth/refresh')) { refreshes++; throw new Error('response lost'); }
    return response({}, 401, {'x-sb-auth-rejected':'1'});
  });
  await assert.rejects(api.call('/profiles'));
  await assert.rejects(api.call('/profiles'));
  assert.equal(refreshes, 1);
  assert.equal(api.currentSession(), null);
});

test('late responses cannot become results in a different account', async () => {
  let finish!: (value: Response) => void;
  const api = client(async () => new Promise(resolve => { finish = resolve; }));
  const pending = api.call('/sessions');
  api.setSession({ ...session('eyJheader.eyJpayload.other'), identity: { id:'Xy2', name:'si:worker', kind:'silicon' } });
  finish(response([]));
  await assert.rejects(pending, /Sign-in changed/);
  assert.equal(api.currentSession()?.identity.id, 'Xy2');
});

test('switching accounts during a replay rejects the previous account response', async () => {
  let finish!: (value: Response) => void, started!: () => void;
  const begun = new Promise<void>(resolve => { started = resolve; });
  const api = client(async (url, options) => {
    if (url.endsWith('/auth/refresh')) return response(session('eyJheader.eyJpayload.renewed'));
    if ((options.headers as Record<string, string>).Authorization === 'Bearer eyJheader.eyJpayload.initial') return response({}, 401, { 'x-sb-auth-rejected': '1' });
    started();
    return new Promise(resolve => { finish = resolve; });
  });
  const pending = api.call('/profiles');
  await begun;
  const next = { ...session('eyJheader.eyJpayload.other'), identity: { id: 'Xy2', name: 'si:worker', kind: 'silicon' as const } };
  api.setSession(next);
  finish(response([{ id: 'previous_account_profile' }]));
  await assert.rejects(pending, /Sign-in changed/);
  assert.deepEqual(api.currentSession(), next);
});

test('canonical UUID account identities survive sign-in while malformed spellings fail closed', () => {
  const id = 'bc0f6684-df51-4d9f-b945-bbdf88e599ad';
  const auth = { ...session(), identity: { ...session().identity, id } };
  assert.equal(acceptAuth(auth).identity.id, id);
  assert.equal(acceptAuth(auth, 'browser', id).identity.id, id);
  for (const malformed of [id.toUpperCase(), '00000000-0000-0000-0000-000000000000', id.replace('-b945-', '-7945-'), `${id}\n`]) {
    assert.throws(() => acceptAuth({ ...auth, identity: { ...auth.identity, id: malformed } }));
  }
  assert.throws(() => acceptAuth(auth, 'browser', 'Ab1'));
});
