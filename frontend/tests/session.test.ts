import test from 'node:test';
import assert from 'node:assert/strict';
import { BrowserApi } from '../src/api';
import { BrowserSession } from '../src/session';
import type { AuthSession } from '../src/types';

const origin = 'https://backend.browser.teamofsilicons.com';
const initial: AuthSession = { access_token: 'eyJheader.eyJpayload.original', refresh_token: 'sar_original', expires_at: '2020-01-01T00:00:00Z', identity: { id: 'Ab1', name: 'Owner', kind: 'carbon' }, org: { id: 'browser', name: 'Browser' } };
const renewed: AuthSession = { ...initial, access_token: 'eyJheader.eyJpayload.renewed', refresh_token: 'sar_renewed', expires_at: '2099-01-01T00:00:00Z' };
function fixture() {
  const values = new Map<string, string>();
  const storage = { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); }, removeItem: (key: string) => { values.delete(key); } } as Storage;
  const lock = (_name: string, task: () => Promise<AuthSession>) => task();
  return { values, saved: new BrowserSession(origin, () => storage, lock), storage, lock };
}
const reply = (data: unknown, status = 200) => new Response(JSON.stringify(status === 200 ? { data } : { error: { message: 'Unavailable' } }), { status });

test('reload restores the interactive session and saves the rotated pair before the next request', async () => {
  const { saved, storage, lock } = fixture(); saved.save(initial);
  const reloaded = new BrowserSession(origin, () => storage, lock);
  const calls: string[] = [];
  const api = new BrowserApi(origin, (async (url, options) => {
    calls.push(String(url));
    if (String(url).endsWith('/auth/refresh')) {
      assert.equal(JSON.parse(String(options?.body)).refresh_token, initial.refresh_token);
      assert.deepEqual(reloaded.load(), initial);
      return reply(renewed);
    }
    assert.equal((options?.headers as Record<string, string>).Authorization, `Bearer ${renewed.access_token}`);
    assert.deepEqual(reloaded.load(), renewed);
    return reply([]);
  }) as typeof fetch, (value, previous) => reloaded.save(value, previous), (session, exchange) => reloaded.refresh(session, exchange));
  api.setSession(reloaded.load());
  await api.call('/sessions');
  assert.equal(calls.length, 2);
  assert.deepEqual(new BrowserSession(origin, () => storage, lock).load(), renewed);
  api.setSession(null);
  assert.equal(new BrowserSession(origin, () => storage, lock).load(), null);
});

test('any failed one-use refresh clears saved login before it can be reused', async () => {
  for (const status of [401, 403, 500, 0]) {
    const { saved } = fixture(); saved.save(initial);
    const api = new BrowserApi(origin, (async () => { if (!status) throw new Error('offline'); return reply({}, status); }) as typeof fetch, (value, previous) => saved.save(value, previous), (session, exchange) => saved.refresh(session, exchange));
    api.setSession(saved.load());
    await assert.rejects(api.call('/sessions'));
    assert.equal(saved.load(), null);
  }
});

test('sign-out during renewal cannot repopulate saved credentials', async () => {
  const { saved } = fixture();
  let finish!: (response: Response) => void;
  const api = new BrowserApi(origin, (() => new Promise(resolve => { finish = resolve; })) as typeof fetch, (value, previous) => saved.save(value, previous), (session, exchange) => saved.refresh(session, exchange));
  api.setSession(initial);
  const pending = api.call('/sessions');
  api.setSession(null);
  finish(reply(renewed));
  await assert.rejects(pending, /Sign-in changed/);
  assert.equal(saved.load(), null);
});

test('malformed saved state is removed, and another API origin cannot load the session', () => {
  const { saved, storage, values } = fixture();
  saved.save(initial);
  assert.equal(new BrowserSession('http://127.0.0.1:8091', () => storage).load(), null);
  const key = [...values.keys()][0];
  for (const bad of ['{', 'null', '{}', JSON.stringify({ ...initial, refresh_token: 'sac_code' }), JSON.stringify({ ...initial, expires_at: 'bad' })]) {
    values.set(key, bad);
    assert.equal(saved.load(), null);
    assert.deepEqual(saved.all(), []);
  }
});

test('unavailable storage does not crash initial rendering and explains a failed save', () => {
  const saved = new BrowserSession(origin, () => { throw new Error('blocked'); });
  assert.equal(saved.load(), null);
  assert.throws(() => saved.save(initial), /Allow website storage/);
});

test('legacy cached identities cannot be saved as Accounts logins', () => {
  const { saved } = fixture();
  for (const identity of [{ id: 'c:owner', name: 'Owner', kind: 'carbon' as const }, { id: 'worker:tos', name: 'Worker', kind: 'silicon' as const }]) {
    assert.throws(() => saved.save({ ...initial, expires_at: '2099-01-01T00:00:00Z', identity }));
    assert.equal(saved.load(), null);
    assert.deepEqual(saved.all(), []);
  }
});


test('accounts retain separate rotating credential families in one Browser workspace', () => {
  const { saved } = fixture();
  const second = { ...renewed, access_token: 'eyJheader.eyJpayload.second', refresh_token: 'sar_second', identity: {id:'Cd2', name:'Other', kind:'carbon' as const} };
  const otherActor = { ...renewed, access_token: 'eyJheader.eyJpayload.other', refresh_token: 'sar_other', identity: { id: 'Ef3', kind: 'silicon' as const, name: 'Worker' } };
  saved.save(initial); saved.save(second); saved.save(otherActor);
  assert.equal(saved.all().length, 3);
  saved.save(renewed);
  assert.equal(saved.load()?.access_token, 'eyJheader.eyJpayload.renewed');
  assert.deepEqual(saved.all().find(item => item.identity.id === 'Cd2'), second);
  saved.save(null);
  assert.equal(saved.load(), null);
  assert.equal(saved.all().length, 2);
  saved.save(second);
  assert.equal(saved.load()?.refresh_token, 'sar_second');
});

test('a changed public display handle keeps the same account context', () => {
  const { saved } = fixture();
  saved.save(initial);
  saved.save({ ...renewed, identity: { ...renewed.identity, name: 'c:renamed' } });
  assert.equal(saved.all().length, 1);
  assert.equal(saved.load()?.identity.id, initial.identity.id);
  assert.equal(saved.load()?.identity.name, 'c:renamed');
});

test('two tabs share one rotation and both use its new access token', async () => {
  const { storage, saved } = fixture(); saved.save(initial);
  let tail = Promise.resolve();
  const lock = (_name: string, task: () => Promise<AuthSession>) => {
    const operation = tail.then(task);
    tail = operation.then(() => {}, () => {});
    return operation;
  };
  let refreshes = 0;
  const fetcher = (async (url, options) => {
    if (String(url).endsWith('/auth/refresh')) { refreshes++; await Promise.resolve(); return reply(renewed); }
    assert.equal((options?.headers as Record<string, string>).Authorization, `Bearer ${renewed.access_token}`);
    return reply([]);
  }) as typeof fetch;
  const clients = [1, 2].map(() => {
    const store = new BrowserSession(origin, () => storage, lock);
    const api = new BrowserApi(origin, fetcher, (value, previous) => store.save(value, previous), (session, exchange) => store.refresh(session, exchange));
    api.setSession(store.load(), false);
    return api;
  });
  await Promise.all(clients.map(api => api.call('/sessions')));
  assert.equal(refreshes, 1);
  for (const api of clients) assert.deepEqual(api.currentSession(), renewed);
  assert.deepEqual(saved.load(), renewed);
});

test('a late refresh cannot overwrite another tab’s replacement login or sign it out', async () => {
  const { saved } = fixture(); saved.save(initial);
  let finish!: (response: Response) => void;
  const api = new BrowserApi(origin, (() => new Promise(resolve => { finish = resolve; })) as typeof fetch, (value, previous) => saved.save(value, previous), (session, exchange) => saved.refresh(session, exchange));
  api.setSession(initial, false);
  const pending = api.call('/sessions');
  const replacement = { ...renewed, refresh_token: 'sar_replacement' };
  saved.save(replacement);
  finish(reply(renewed));
  await assert.rejects(pending);
  assert.deepEqual(saved.load(), replacement);
});

test('explicit sign-out in another tab prevents a pending refresh from restoring login', async () => {
  const { saved } = fixture(); saved.save(initial);
  let finish!: (response: AuthSession) => void;
  const pending = saved.refresh(initial, () => new Promise(resolve => { finish = resolve; }));
  saved.forget(initial);
  finish(renewed);
  await assert.rejects(pending);
  assert.equal(saved.load(), null);
});

test('a tab closing mid-refresh cannot cause another tab to resend its spent token', async () => {
  const { saved, storage, lock, values } = fixture(); saved.save(initial);
  const state = JSON.parse(values.get(saved.storageKey)!);
  state.refreshing = { 'Ab1/browser': initial.refresh_token };
  values.set(saved.storageKey, JSON.stringify(state));
  const other = new BrowserSession(origin, () => storage, lock);
  let requests = 0;
  await assert.rejects(other.refresh(initial, async () => { requests++; return renewed; }));
  assert.equal(requests, 0);
  assert.equal(other.load(), null);
});

test('existing tab login migrates once and explicit sign-out prevents stale reimport', () => {
  const { storage, lock } = fixture();
  const legacy = new Map<string, string>();
  const key = `silicon-browser:session:v3:${origin}`;
  const oldStorage = { getItem: (key: string) => legacy.get(key) ?? null, removeItem: (key: string) => { legacy.delete(key); } } as Storage;
  legacy.set(key, JSON.stringify({ active: 'Ab1/browser', sessions: [initial] }));
  const migrated = new BrowserSession(origin, () => storage, lock, () => oldStorage);
  assert.deepEqual(migrated.load(), initial);
  assert.equal(legacy.has(key), false);
  migrated.forget(initial);
  legacy.set(key, JSON.stringify({ active: 'Ab1/browser', sessions: [initial] }));
  assert.equal(new BrowserSession(origin, () => storage, lock, () => oldStorage).load(), null);
});

test('ordinary API network failures preserve the shared login', async () => {
  const { saved } = fixture(); saved.save(renewed);
  const api = new BrowserApi(origin, (async () => { throw new Error('offline'); }) as typeof fetch, (value, previous) => saved.save(value, previous), (session, exchange) => saved.refresh(session, exchange));
  api.setSession(renewed, false);
  await assert.rejects(api.call('/sessions'));
  assert.deepEqual(saved.load(), renewed);
  assert.deepEqual(api.currentSession(), renewed);
});

test('switching accounts during rotation preserves both families and the selected account', async () => {
  const { saved } = fixture(); saved.save(initial);
  let finish!: (response: Response) => void;
  const api = new BrowserApi(origin, (() => new Promise(resolve => { finish = resolve; })) as typeof fetch, (value, previous) => saved.save(value, previous), (session, exchange) => saved.refresh(session, exchange));
  api.setSession(initial, false);
  const pending = api.call('/sessions');
  const other = { ...renewed, refresh_token: 'sar_other', identity: { id: 'Cd2', name: 'Other', kind: 'carbon' as const } };
  api.setSession(other);
  finish(reply(renewed));
  await assert.rejects(pending, /Sign-in changed/);
  assert.deepEqual(saved.load(), other);
  assert.deepEqual(saved.all().find(item => item.identity.id === initial.identity.id), renewed);
  assert.deepEqual(api.currentSession(), other);
});

test('selecting the same saved account during rotation does not erase its one-use claim', async () => {
  const { saved, values } = fixture(); saved.save(initial);
  let finish!: (session: AuthSession) => void;
  const pending = saved.refresh(initial, () => new Promise(resolve => { finish = resolve; }));
  saved.save(initial);
  assert.equal(JSON.parse(values.get(saved.storageKey)!).refreshing['Ab1/browser'], initial.refresh_token);
  finish(renewed);
  await pending;
  assert.deepEqual(saved.load(), renewed);
});
