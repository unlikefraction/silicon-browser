import test from 'node:test';
import assert from 'node:assert/strict';
import { BrowserApi } from '../src/api';
import { TabSession } from '../src/session';
import type { AuthSession } from '../src/types';

const origin = 'https://backend.browser.teamofsilicons.com';
const initial: AuthSession = { access_token: 'eyJheader.eyJpayload.original', refresh_token: 'sar_original', expires_at: '2020-01-01T00:00:00Z', identity: { id: 'Ab1', name: 'Owner', kind: 'carbon' }, org: { id: 'browser', name: 'Browser' } };
const renewed: AuthSession = { ...initial, access_token: 'eyJheader.eyJpayload.renewed', refresh_token: 'sar_renewed', expires_at: '2099-01-01T00:00:00Z' };
function fixture() {
  const values = new Map<string, string>();
  const storage = { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); }, removeItem: (key: string) => { values.delete(key); } } as Storage;
  return { values, saved: new TabSession(origin, () => storage), storage };
}
const reply = (data: unknown, status = 200) => new Response(JSON.stringify(status === 200 ? { data } : { error: { message: 'Unavailable' } }), { status });

test('reload restores the interactive session and saves the rotated pair before the next request', async () => {
  const { saved, storage } = fixture(); saved.save(initial);
  const reloaded = new TabSession(origin, () => storage);
  const calls: string[] = [];
  const api = new BrowserApi(origin, (async (url, options) => {
    calls.push(String(url));
    if (String(url).endsWith('/auth/refresh')) {
      assert.equal(JSON.parse(String(options?.body)).refresh_token, initial.refresh_token);
      assert.equal(reloaded.load(), null);
      return reply(renewed);
    }
    assert.equal((options?.headers as Record<string, string>).Authorization, `Bearer ${renewed.access_token}`);
    assert.deepEqual(reloaded.load(), renewed);
    return reply([]);
  }) as typeof fetch, value => reloaded.save(value));
  api.setSession(reloaded.load());
  await api.call('/sessions');
  assert.equal(calls.length, 2);
  assert.deepEqual(new TabSession(origin, () => storage).load(), renewed);
  api.setSession(null);
  assert.equal(new TabSession(origin, () => storage).load(), null);
});

test('any failed one-use refresh clears saved login before it can be reused', async () => {
  for (const status of [401, 403, 500, 0]) {
    const { saved } = fixture(); saved.save(initial);
    const api = new BrowserApi(origin, (async () => { if (!status) throw new Error('offline'); return reply({}, status); }) as typeof fetch, value => saved.save(value));
    api.setSession(saved.load());
    await assert.rejects(api.call('/sessions'));
    assert.equal(saved.load(), null);
  }
});

test('sign-out during renewal cannot repopulate saved credentials', async () => {
  const { saved } = fixture();
  let finish!: (response: Response) => void;
  const api = new BrowserApi(origin, (() => new Promise(resolve => { finish = resolve; })) as typeof fetch, value => saved.save(value));
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
  assert.equal(new TabSession('http://127.0.0.1:8091', () => storage).load(), null);
  const key = [...values.keys()][0];
  for (const bad of ['{', 'null', '{}', JSON.stringify({ ...initial, refresh_token: 'sac_code' }), JSON.stringify({ ...initial, expires_at: 'bad' })]) {
    values.set(key, bad);
    assert.equal(saved.load(), null);
    assert.equal(values.size, 0);
  }
});

test('unavailable storage does not crash initial rendering and explains a failed save', () => {
  const saved = new TabSession(origin, () => { throw new Error('blocked'); });
  assert.equal(saved.load(), null);
  assert.throws(() => saved.save(initial), /Allow website storage/);
});

test('legacy cached identities require fresh sign-in before being displayed', () => {
  const { saved, values } = fixture();
  for (const identity of [{ id: 'c:owner', name: 'Owner', kind: 'carbon' as const }, { id: 'worker:tos', name: 'Worker', kind: 'silicon' as const }]) {
    saved.save({ ...initial, expires_at: '2099-01-01T00:00:00Z', identity });
    assert.equal(saved.load(), null);
    assert.equal(values.size, 0);
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
