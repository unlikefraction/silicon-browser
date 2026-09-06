import test from 'node:test';
import assert from 'node:assert/strict';
import { BrowserApi } from '../src/api';
import { TabSession } from '../src/session';
import type { AuthSession } from '../src/types';

const origin = 'https://backend.browser.teamofsilicons.com';
const initial: AuthSession = { access_token: 'oat_original', refresh_token: 'ort_original', expires_at: '2020-01-01T00:00:00Z', identity: { id: 'owner', name: 'Owner', kind: 'carbon' }, org: { id: 'tos', name: 'tos' } };
const renewed: AuthSession = { ...initial, access_token: 'oat_renewed', refresh_token: 'ort_renewed', expires_at: '2099-01-01T00:00:00Z' };
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

test('terminal renewal rejection clears saved login; network and service failures preserve it', async () => {
  for (const status of [401, 403, 500, 0]) {
    const { saved } = fixture(); saved.save(initial);
    const api = new BrowserApi(origin, (async () => { if (!status) throw new Error('offline'); return reply({}, status); }) as typeof fetch, value => saved.save(value));
    api.setSession(saved.load());
    await assert.rejects(api.call('/sessions'));
    assert.deepEqual(saved.load(), status === 401 || status === 403 ? null : initial);
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
  for (const bad of ['{', 'null', '{}', JSON.stringify({ ...initial, refresh_token: 'oac_code' }), JSON.stringify({ ...initial, expires_at: 'bad' })]) {
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
