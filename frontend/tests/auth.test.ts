import test from 'node:test';
import assert from 'node:assert/strict';
import { loginUrl, readEntry, matchingCallback, matchingRedirect, signInPopup, ACCOUNTS_ORIGIN } from '../src/auth';

const origin = 'https://browser.teamofsilicons.com';
function attempt() {
  const attempt_id = '11111111-1111-4111-8111-111111111111', state = `${attempt_id}.${'a'.repeat(64)}`;
  const url = new URL('/authorize', ACCOUNTS_ORIGIN);
  url.search = new URLSearchParams({ app_id:'browser', redirect_uri:`${origin}/auth/callback`, response_type:'code', state, code_challenge:'b'.repeat(43), code_challenge_method:'S256' }).toString();
  return { attempt_id, state, authorize_url:url.href };
}
test('Carbon sign-in uses the backend PKCE attempt and only the Accounts authorize page', () => {
  const valid = attempt();
  assert.equal(loginUrl(origin, valid), valid.authorize_url);
  for (const change of [
    (url: URL) => { url.hostname = 'evil.example'; },
    (url: URL) => { url.pathname = '/login'; },
    (url: URL) => { url.searchParams.set('state', 'another'); },
    (url: URL) => { url.searchParams.set('code_challenge_method', 'plain'); },
    (url: URL) => { url.searchParams.set('redirect_uri', `${origin}/auth/callback?attempt_id=another`); },
    (url: URL) => { url.searchParams.set('redirect_uri', 'https://evil.example/auth/callback?attempt_id=request-id'); },
  ]) {
    const url = new URL(valid.authorize_url); change(url);
    assert.throws(() => loginUrl(origin, {...valid, authorize_url:url.href}));
  }
});
test('callback codes and live grants are stripped from the displayed entry URL', () => {
  const request = attempt();
  const callback = readEntry(new URL(`${origin}/auth/callback?state=${request.state}&code=sac_private#extra`));
  assert.equal(callback.callback?.attempt, request.attempt_id);
  assert.equal(callback.cleanPath, '/auth/callback'); assert.equal(callback.callback?.code, 'sac_private');
  const live = readEntry(new URL(`${origin}/sessions/session%201/live#grant=private&org=browser`));
  assert.deepEqual(live.pending, {id:'session 1',grant:'private'}); assert.equal(live.cleanPath, '/sessions/session%201/live');
});
test('retired organization and testing invitations cannot be redeemed in Browser', () => {
  for (const selector of ['org=tos', 'org=browser&org=browser', 'test=11111111-1111-4111-8111-111111111111']) {
    const invalid = readEntry(new URL(`${origin}/sessions/session-1/live#grant=private&${selector}`));
    assert.equal(invalid.pending, null); assert.match(invalid.error, /older workspace/);
  }
});
test('unrelated and malformed routes cannot create a live handoff', () => {
  for (const path of ['/other#grant=x', '/sessions/%zz/live#grant=x', '/sessions/id/live']) assert.equal(readEntry(new URL(`${origin}${path}`)).pending, null);
});
test('popup handoff requires exact origin, source window, state, attempt and authorization code', () => {
  const popup = {} as Window, request = attempt();
  const valid = {origin,source:popup,data:{type:'silicon-browser:sign-in',nonce:request.state,attempt:request.attempt_id,code:'sac_fresh'}};
  assert.equal(matchingCallback(valid,origin,popup,request),'sac_fresh');
  for (const event of [ {...valid, origin:'https://evil.example'}, {...valid, source:{} as Window}, {...valid, data:{...valid.data,nonce:'different'}}, {...valid, data:{...valid.data,attempt:'different'}}, {...valid, data:{...valid.data,type:'other'}}, {...valid, data:{...valid.data,code:'sar_wrong'}}, {...valid, data:{...valid.data,code:'sac_bad\nvalue'}} ]) assert.equal(matchingCallback(event,origin,popup,request),null);
});
test('popup is reserved before the backend attempt and resolves only its exact code', async () => {
  const originalWindow = globalThis.window, originalLocation = globalThis.location;
  let receive!: (event: unknown) => void; const calls: string[] = [];
  const popup = { closed: false, location: { href: '' }, focus() {}, close() { this.closed = true; } };
  globalThis.window = { open() { calls.push('popup'); return popup; }, addEventListener(_type: string, listener: typeof receive) { receive = listener; }, removeEventListener() {} } as unknown as Window & typeof globalThis;
  globalThis.location = { origin } as Location;
  try {
    const request = attempt();
    const pending = signInPopup(async () => { calls.push('backend'); return request; });
    await Promise.resolve();
    assert.deepEqual(calls, ['popup', 'backend']);
    receive({ origin, source: popup, data: {type:'silicon-browser:sign-in',nonce:request.state,attempt:request.attempt_id,code:'sac_once'} });
    assert.deepEqual(await pending, {attempt_id:request.attempt_id,state:request.state,code:'sac_once'});
    assert.equal(popup.closed, true);
  } finally { globalThis.window = originalWindow; globalThis.location = originalLocation; }
});

test('full-page fallback requires its tab-bound attempt and rejects expired state', () => {
  const original = globalThis.sessionStorage, request = attempt();
  let saved: unknown = { attempt_id:request.attempt_id, state:request.state, created:Date.now() };
  globalThis.sessionStorage = { getItem() { return JSON.stringify(saved); } } as unknown as Storage;
  const callback = {nonce:request.state,attempt:request.attempt_id,code:'sac_once'};
  try {
    assert.equal(matchingRedirect(callback), true);
    assert.equal(matchingRedirect({...callback, nonce:'another'}), false);
    assert.equal(matchingRedirect({...callback, code:'sar_wrong'}), false);
    saved = {attempt_id:request.attempt_id, state:request.state, created:Date.now()-600001};
    assert.equal(matchingRedirect(callback), false);
  } finally { globalThis.sessionStorage = original; }
});
