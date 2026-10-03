import test from 'node:test';
import assert from 'node:assert/strict';
import { loginUrl, readEntry, requireLiveEnvironment, matchingCallback, completeCallback, signInPopup, PendingLoginStore, IAM_AUTH_ORIGIN, type LoginAttempt } from '../src/auth';
const attempt: LoginAttempt = { attempt_id: '11111111-1111-4111-8111-111111111111', state: 'a'.repeat(64), identity_kind: 'carbon', expires_at: '2099-01-01T00:00:00Z' };
const origin = 'https://browser.teamofsilicons.com';
const callback = { attempt_id: attempt.attempt_id, state: attempt.state, token: 'oac_oneuse' };
function store() {
  const values = new Map<string, string>();
  const storage = { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); }, removeItem: (key: string) => { values.delete(key); } } as Storage;
  return { values, pending: new PendingLoginStore(origin, () => storage), storage };
}
test('IAM login locks the backend account kind and callback binding, with optional popup display', () => {
  for (const kind of ['carbon', 'silicon'] as const) for (const popup of [true, false]) {
    const url = new URL(loginUrl(origin, { ...attempt, identity_kind: kind }, popup));
    assert.equal(url.origin, IAM_AUTH_ORIGIN); assert.equal(url.pathname, '/login'); assert.equal(url.searchParams.get('app_id'), 'browser'); assert.equal(url.searchParams.get('org_id'), null);
    assert.equal(url.searchParams.get('identity_kind'), kind); assert.equal(url.searchParams.get('display'), popup ? 'popup' : null);
    const destination = new URL(url.searchParams.get('redirect_uri')!);
    assert.equal(destination.origin, origin); assert.equal(destination.pathname, '/auth/callback'); assert.equal(destination.searchParams.get('attempt_id'), attempt.attempt_id); assert.equal(destination.searchParams.get('state'), attempt.state);
  }
});
test('callback credentials and live grant are stripped from displayed entry URL', () => {
  const callback = readEntry(new URL(`https://browser.teamofsilicons.com/auth/callback?attempt_id=${attempt.attempt_id}&state=${attempt.state}&slt=oac_private#extra`));
  assert.equal(callback.cleanPath, '/auth/callback'); assert.deepEqual(callback.callback, { attempt_id: attempt.attempt_id, state: attempt.state, token: 'oac_private' });
  const live = readEntry(new URL('https://browser.teamofsilicons.com/sessions/session%201/live#grant=private'));
  assert.deepEqual(live.pending, {id:'session 1',grant:'private'}); assert.equal(live.cleanPath, '/sessions/session%201/live');
});
test('unrelated and malformed routes cannot create a live handoff', () => {
  for (const path of ['/other#grant=x', '/sessions/%zz/live#grant=x', '/sessions/id/live']) assert.equal(readEntry(new URL(`https://browser.teamofsilicons.com${path}`)).pending, null);
});
test('test live invitations retain their environment and cannot cross production or another environment', () => {
  const environment = '11111111-1111-4111-8111-111111111111';
  const entry = readEntry(new URL(`https://browser.teamofsilicons.com/sessions/session-1/live#grant=private&test=${environment}`));
  assert.deepEqual(entry.pending, { id: 'session-1', grant: 'private', testEnvironmentId: environment });
  assert.equal(entry.cleanPath, '/sessions/session-1/live');
  assert.throws(() => requireLiveEnvironment(entry.pending!), /requires testing environment/);
  assert.throws(() => requireLiveEnvironment(entry.pending!, '22222222-2222-4222-8222-222222222222'), /requires testing environment/);
  assert.doesNotThrow(() => requireLiveEnvironment(entry.pending!, environment));
  assert.throws(() => requireLiveEnvironment({ id: 'session-1', grant: 'production' }, environment), /belongs to production/);
  for (const selector of ['', '../production', `${environment}&test=${environment}`]) {
    const invalid = readEntry(new URL(`https://browser.teamofsilicons.com/sessions/session-1/live#grant=private&test=${selector}`));
    assert.equal(invalid.pending, null); assert.equal(invalid.liveId, null); assert.match(invalid.error, /invalid/);
  }
});

test('popup handoff requires exact origin, source window, attempt, state and bounded SLT', () => {
  const popup = {} as Window;
  const valid = { origin, source: popup, data: { type: 'silicon-browser:sign-in', ...callback } };
  assert.equal(matchingCallback(valid, origin, popup, attempt), 'oac_oneuse');
  for (const event of [ {...valid, origin:'https://evil.example'}, {...valid, source:{} as Window}, {...valid, data:{...valid.data,state:'different'}}, {...valid, data:{...valid.data,attempt_id:'different'}}, {...valid, data:{...valid.data,type:'other'}}, {...valid, data:{...valid.data,token:'ort_wrong'}}, {...valid, data:{...valid.data,token:'oac_bad\nvalue'}} ]) assert.equal(matchingCallback(event,origin,popup,attempt),null);
});
test('full-page attempt storage excludes tokens and rejects mismatches, expiry and external destinations', () => {
  const { pending, values, storage } = store();
  pending.save({ ...attempt, token: 'oac_private', grant: 'private' } as LoginAttempt, '/sessions/s1/live');
  assert.deepEqual(pending.load(callback), { ...attempt, return_path: '/sessions/s1/live' });
  assert(![...values.values()][0].includes('private'));
  assert.throws(() => pending.load({ ...callback, state: 'b'.repeat(64) }), /does not match/);
  assert.throws(() => new PendingLoginStore('https://other.example', () => storage).load(callback), /does not match/);
  for (const path of ['//evil.example', '/?token=secret', '/#grant=secret', '/other', '/\\evil.example']) assert.throws(() => pending.save(attempt, path));
  assert.throws(() => pending.save({ ...attempt, expires_at: '2020-01-01T00:00:00Z' }, '/'), /expired/);
  pending.clear(); assert.equal(values.size, 0);
});
function browser(t: { after: (fn: () => void) => void }, blocked = false) {
  const listeners = new Map<string, (event: MessageEvent) => unknown>(), messages: unknown[] = [];
  let closed = false, assigned = '';
  const popup = { location: { href: '' }, focus() {}, close() { closed = true; }, get closed() { return closed; }, postMessage(message: unknown) { messages.push(message); } } as unknown as Window;
  const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window'), originalLocation = Object.getOwnPropertyDescriptor(globalThis, 'location');
  Object.defineProperty(globalThis, 'window', { configurable: true, value: { open: () => blocked ? null : popup, addEventListener: (name: string, listener: (event: MessageEvent) => unknown) => listeners.set(name, listener), removeEventListener: (name: string) => listeners.delete(name) } });
  Object.defineProperty(globalThis, 'location', { configurable: true, value: { origin, assign: (url: string) => { assigned = url; } } });
  t.after(() => { for (const [name, descriptor] of [['window', originalWindow], ['location', originalLocation]] as const) { if (descriptor) Object.defineProperty(globalThis, name, descriptor); else Reflect.deleteProperty(globalThis, name); } });
  return { popup, listeners, messages, closed: () => closed, assigned: () => assigned };
}
test('popup waits for backend verification before acknowledgement or closing', async t => {
  const ui = browser(t), { pending } = store();
  let finish!: () => void;
  const verified = new Promise<void>(resolve => { finish = resolve; });
  const operation = signInPopup('carbon', async () => attempt, async () => verified, pending, '/');
  await new Promise(resolve => setImmediate(resolve));
  const receive = ui.listeners.get('message')!;
  const completing = receive({ origin, source: ui.popup, data: { type: 'silicon-browser:sign-in', ...callback } } as unknown as MessageEvent);
  assert.equal(ui.closed(), false); assert.deepEqual(ui.messages, []);
  finish(); await completing; assert.equal(await operation, true);
  assert.equal(ui.closed(), true); assert.deepEqual(ui.messages, [{ type: 'silicon-browser:sign-in:complete', attempt_id: attempt.attempt_id, state: attempt.state }]);
});
test('blocked popup preserves the bound attempt and local action before full-page navigation', async t => {
  const ui = browser(t, true), { pending } = store(); let exchanges = 0;
  assert.equal(await signInPopup('carbon', async () => attempt, async () => { exchanges++; }, pending, '/sessions/s1/live'), false);
  assert.equal(exchanges, 0); assert.equal(pending.load(callback).return_path, '/sessions/s1/live');
  const url = new URL(ui.assigned()); assert.equal(url.origin, IAM_AUTH_ORIGIN); assert.equal(url.searchParams.get('display'), null);
});
test('callback remains open until its exact opener acknowledges the same attempt', t => {
  const ui = browser(t); let closed = false;
  Object.assign(window, { opener: ui.popup, close() { closed = true; } });
  assert.equal(completeCallback(callback, () => {}), true); assert.equal(closed, false);
  const receive = ui.listeners.get('message')!;
  const data = { type: 'silicon-browser:sign-in:complete', attempt_id: attempt.attempt_id, state: attempt.state };
  receive({ origin: 'https://evil.example', source: ui.popup, data } as unknown as MessageEvent); assert.equal(closed, false);
  receive({ origin, source: {} as Window, data } as unknown as MessageEvent); assert.equal(closed, false);
  receive({ origin, source: ui.popup, data } as unknown as MessageEvent); assert.equal(closed, true);
});

test('closing the popup during verification cancels without acknowledging a late success', async t => {
  const ui = browser(t), { pending } = store();
  let finish!: () => void;
  const verified = new Promise<void>(resolve => { finish = resolve; });
  const operation = signInPopup('carbon', async () => attempt, async () => verified, pending, '/');
  const rejected = assert.rejects(operation, /Sign-in cancelled/);
  await new Promise(resolve => setImmediate(resolve));
  const completing = ui.listeners.get('message')!({ origin, source: ui.popup, data: { type: 'silicon-browser:sign-in', ...callback } } as unknown as MessageEvent);
  ui.popup.close(); finish(); await completing; await rejected;
  assert(!ui.messages.some(message => (message as {type:string}).type.endsWith(':complete')));
});
