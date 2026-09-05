import test from 'node:test';
import assert from 'node:assert/strict';
import { loginUrl, readEntry, matchingCallback, IAM_AUTH_ORIGIN } from '../src/auth';

test('IAM popup uses canonical application and selected organization, with a nonce in callback', () => {
  const url = new URL(loginUrl('https://browser.teamofsilicons.com', 'my org', 'random-nonce'));
  assert.equal(url.origin, IAM_AUTH_ORIGIN); assert.equal(url.pathname, '/login'); assert.equal(url.searchParams.get('app_id'), 'tos>browser'); assert.equal(url.searchParams.get('org_id'), 'my org');
  const callback = new URL(url.searchParams.get('redirect_uri')!); assert.equal(callback.origin, 'https://browser.teamofsilicons.com'); assert.equal(callback.pathname, '/auth/callback'); assert.equal(callback.searchParams.get('nonce'), 'random-nonce');
});
test('callback credentials and live grant are stripped from displayed entry URL', () => {
  const callback = readEntry(new URL('https://browser.teamofsilicons.com/auth/callback?nonce=n&slt=oac_private#extra'));
  assert.equal(callback.cleanPath, '/auth/callback'); assert.equal(callback.callback?.token, 'oac_private');
  const live = readEntry(new URL('https://browser.teamofsilicons.com/sessions/session%201/live#grant=private'));
  assert.deepEqual(live.pending, {id:'session 1',grant:'private'}); assert.equal(live.cleanPath, '/sessions/session%201/live');
});
test('unrelated and malformed routes cannot create a live handoff', () => {
  for (const path of ['/other#grant=x', '/sessions/%zz/live#grant=x', '/sessions/id/live']) assert.equal(readEntry(new URL(`https://browser.teamofsilicons.com${path}`)).pending, null);
});
test('popup handoff requires exact same origin, source window, nonce and token type', () => {
  const popup = {} as Window;
  const valid = {origin:'https://browser.teamofsilicons.com',source:popup,data:{type:'silicon-browser:sign-in',nonce:'secret-random',token:'oac_fresh'}};
  assert.equal(matchingCallback(valid,valid.origin,popup,'secret-random'),'oac_fresh');
  for (const event of [ {...valid, origin:'https://evil.example'}, {...valid, source:{} as Window}, {...valid, data:{...valid.data,nonce:'different'}}, {...valid, data:{...valid.data,type:'other'}}, {...valid, data:{...valid.data,token:'ort_wrong'}}, {...valid, data:{...valid.data,token:'oac_bad\nvalue'}} ]) assert.equal(matchingCallback(event,valid.origin,popup,'secret-random'),null);
});
test('detached-opener handoff still requires the initiating nonce and bounded SLT', async () => {
  const {matchingBroadcast} = await import('../src/auth');
  const payload = {type:'silicon-browser:sign-in',nonce:'random-secret',token:'oac_one-use'};
  assert.equal(matchingBroadcast(payload,'random-secret'),'oac_one-use');
  assert.equal(matchingBroadcast(payload,'another-popup'),null);
  assert.equal(matchingBroadcast({...payload,token:'ort_refresh'},'random-secret'),null);
  assert.equal(matchingBroadcast({...payload,token:'oac_'+'x'.repeat(16381)},'random-secret'),null);
  assert.equal(matchingBroadcast(null,'random-secret'),null);
});
