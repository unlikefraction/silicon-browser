import test from 'node:test';
import assert from 'node:assert/strict';
import { BrowserApi, acceptAuth, dateForApi, publicError, safeHttps, shellQuote } from '../src/api';
import type { AuthSession } from '../src/types';
const session = (access = 'oat_initial'): AuthSession => ({ access_token: access, refresh_token: 'ort_refresh', expires_at: new Date(Date.now() + 900000).toISOString(), identity: { id: 'c:tester', name: 'Tester', kind: 'carbon' }, org: { id: 'tos', name: 'tos' } });
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
test('refresh response must preserve organization and identity when scoped', () => {
  assert.doesNotThrow(() => acceptAuth(session()));
  assert.throws(()=>acceptAuth(session(), 'other')); assert.throws(()=>acceptAuth(session(), 'tos', '@someone-else'));
  assert.throws(()=>acceptAuth({...session(), access_token:'secret'}, 'tos')); assert.throws(()=>acceptAuth({...session(), expires_at:'invalid'}, 'tos'));
  assert.throws(()=>acceptAuth({...session(), access_token:'oat_'}, 'tos')); assert.throws(()=>acceptAuth({...session(), refresh_token:'ort_bad\nheader'}, 'tos'));
  for (const identity of [{id:'tester', name:'Tester', kind:'carbon'}, {id:'worker:tos', name:'Worker', kind:'silicon'}, {id:'c:tester', name:'Tester', kind:'silicon'}]) {
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

const testContext = { environment_id: '11111111-1111-4111-8111-111111111111', app_id: 'browser', name: 'Browser integration tests' };
const testCredentials = { app_secret: 'ask_test-secret', iam_test_key: 'iam_test-root', briefcase_test_environment_key: 'briefcase_test-root' };
test('test app secret alone selects the environment and accepts existing test actor IDs', async () => {
  for (const actor of ['c:alice', 'si:worker']) {
    const production = new BrowserApi('https://backend.browser.teamofsilicons.com', (async (input, options) => {
      if (String(input).endsWith('/testing/context')) {
        assert.deepEqual(JSON.parse(String(options?.body)), { app_secret: testCredentials.app_secret });
        return response(testContext);
      }
      assert.equal((options?.headers as Record<string, string>)['x-testing-environment-key'], undefined);
      assert.equal((options?.headers as Record<string, string>)['x-sb-test-briefcase-key'], undefined);
      assert.equal(JSON.parse(String(options?.body)).short_lived_token, actor);
      return response({ ...session('oat_test'), identity: { id: actor, name: 'Test actor', kind: actor.startsWith('c:') ? 'carbon' : 'silicon' } });
    }) as typeof fetch);
    const result = await production.startTesting({ app_secret: testCredentials.app_secret }, actor, 'tos');
    assert.equal(result.api.currentSession()?.access_token, 'oat_test');
    assert.equal(production.currentSession(), null);
    result.api.close();
  }
});
test('test context, exchange, renewal and mutations stay isolated without persisting test credentials', async () => {
  const saved: (AuthSession | null)[] = [], calls: string[] = [];
  const origin = 'https://backend.browser.teamofsilicons.com';
  const production = new BrowserApi(origin, (async (input, options) => {
    const url = String(input), headers = options?.headers as Record<string, string>; calls.push(url);
    if (url === `${origin}/api/v1/testing/context`) {
      assert.deepEqual(JSON.parse(String(options?.body)), testCredentials);
      assert.equal(headers.Authorization, undefined);
      assert.equal(headers['x-sb-test-app-secret'], undefined);
      return response(testContext);
    }
    assert(url.startsWith(`${origin}/testing/${testContext.environment_id}/api/v1/`));
    assert.equal(headers['x-sb-test-app-secret'], testCredentials.app_secret);
    assert.equal(headers['x-testing-environment-key'], testCredentials.iam_test_key);
    assert.equal(headers['x-sb-test-briefcase-key'], testCredentials.briefcase_test_environment_key);
    if (url.endsWith('/auth/exchange')) {
      assert.deepEqual(JSON.parse(String(options?.body)), { short_lived_token: 'oac_test', org_id: 'tos' });
      assert.equal(headers.Authorization, undefined);
      return response(session('oat_test'));
    }
    if (url.endsWith('/auth/refresh')) return response(session('oat_test_renewed'));
    if (headers.Authorization === 'Bearer oat_test') return response({}, 401, {'x-sb-auth-rejected': '1'});
    assert.equal(headers.Authorization, 'Bearer oat_test_renewed');
    return response({ id: 'test-profile' });
  }) as typeof fetch, value => saved.push(value));
  production.setSession(session());
  const result = await production.startTesting(testCredentials, 'oac_test', 'tos');
  assert.deepEqual(result.context, testContext);
  assert.deepEqual(await result.api.call('/profiles', 'POST', { name: 'test' }), { id: 'test-profile' });
  assert.equal(calls.length, 5);
  assert.equal(production.currentSession()?.access_token, 'oat_initial');
  assert.deepEqual(saved, [production.currentSession()]);
  result.api.close();
  await assert.rejects(result.api.request('/sessions'), /closed/);
  assert.equal(result.api.currentSession(), null);
  assert.equal(calls.length, 5);
  assert.deepEqual(saved, [production.currentSession()]);
});
test('invalid or rejected test contexts never exchange credentials or alter production auth', async () => {
  for (const context of [null, { ...testContext, app_id: 'tos>other' }, { ...testContext, environment_id: '../api' }, { ...testContext, name: null }, 'rejected']) {
    let calls = 0, saves = 0;
    const production = new BrowserApi('https://backend.browser.teamofsilicons.com', (async () => { calls++; return context === 'rejected' ? response({}, 403) : response(context); }) as typeof fetch, () => { saves++; });
    production.setSession(session());
    await assert.rejects(production.startTesting(testCredentials, 'oac_test', 'tos'));
    assert.equal(calls, 1); assert.equal(saves, 1); assert.equal(production.currentSession()?.access_token, 'oat_initial');
  }
  let calls = 0;
  const production = new BrowserApi('https://backend.browser.teamofsilicons.com', (async () => { calls++; return response({}); }) as typeof fetch);
  await assert.rejects(production.startTesting(testCredentials, 'invalid token', 'tos'), /oac_/);
  for (const actor of ['alice', 'worker:tos', 'c:ab', 'si:worker:tos', 'oat_private']) await assert.rejects(production.startTesting(testCredentials, actor, 'tos'), /Migrate old selectors/);
  await assert.rejects(production.startTesting({ ...testCredentials, iam_test_key: 'bad\nheader' }, 'oac_test', 'tos'), /whitespace/);
  assert.equal(calls, 0);
});
test('failed test sign-in preserves production and closing during renewal never replays to production', async () => {
  let failExchange = true, finish!: (value: Response) => void, started!: () => void;
  const begun = new Promise<void>(resolve => { started = resolve; }), calls: string[] = [];
  const production = new BrowserApi('https://backend.browser.teamofsilicons.com', (async input => {
    const url = String(input); calls.push(url);
    if (url.endsWith('/testing/context')) return response(testContext);
    if (url.endsWith('/auth/exchange')) return failExchange ? response({}, 401) : response(session('oat_test'));
    if (url.endsWith('/auth/refresh')) { started(); return new Promise(resolve => { finish = resolve; }); }
    return response({}, 401, {'x-sb-auth-rejected': '1'});
  }) as typeof fetch);
  production.setSession(session());
  await assert.rejects(production.startTesting(testCredentials, 'oac_test', 'tos'));
  assert.equal(production.currentSession()?.access_token, 'oat_initial');
  assert.equal(calls.length, 2);
  failExchange = false;
  const { api } = await production.startTesting(testCredentials, 'oac_test_again', 'tos');
  const pending = api.call('/sessions', 'POST', {});
  await begun; api.close(); finish(response(session('oat_renewed')));
  await assert.rejects(pending, /closed|Sign-in changed/);
  assert.equal(calls.length, 6);
  assert.equal(api.currentSession(), null);
  assert.equal(production.currentSession()?.access_token, 'oat_initial');
});
test('test actor selectors reject an authenticated response for another account', async () => {
  const production = new BrowserApi('https://backend.browser.teamofsilicons.com', (async input => response(String(input).endsWith('/testing/context') ? testContext : session('oat_other'))) as typeof fetch);
  production.setSession(session());
  await assert.rejects(production.startTesting(testCredentials, 'si:worker', 'tos'), /identity.*sign in again/);
  assert.equal(production.currentSession()?.access_token, 'oat_initial');
});
test('test invitations require matching verified context before exchange and test paths cannot escape to production', async () => {
  let calls = 0;
  const production = new BrowserApi('https://backend.browser.teamofsilicons.com', (async input => {
    calls++;
    return response(String(input).endsWith('/testing/context') ? testContext : { ...session('oat_test'), identity: { id: 'c:alice', name: 'Alice', kind: 'carbon' } });
  }) as typeof fetch);
  await assert.rejects(production.startTesting(testCredentials, 'c:alice', 'tos', '22222222-2222-4222-8222-222222222222'), /different environment/);
  assert.equal(calls, 1);
  const { api } = await production.startTesting(testCredentials, 'c:alice', 'tos', testContext.environment_id);
  assert.equal(calls, 3);
  for (const path of ['/../../../../api/v1/me', '/%2e%2e/%2e%2e/%2e%2e/%2e%2e/api/v1/me', '/sessions/%2f..%2f..', '/sessions/%5c..', '/sessions#ignored', '\\..\\..\\..\\..\\api\\v1\\me']) {
    await assert.rejects(api.request(path), /inside this environment/);
  }
  assert.equal(calls, 3);
  api.close();
});
