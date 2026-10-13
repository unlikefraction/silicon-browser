import type { PendingLive } from './types';
export const ACCOUNTS_ORIGIN = 'https://accounts.teamofsilicons.com';
export const ACCOUNTS_APP_ID = 'browser';
const CALLBACK_TYPE = 'silicon-browser:sign-in';
const CODE = /^sac_[A-Za-z0-9_-]{1,16380}$/;
const STATE = /^[a-f\d]{8}(?:-[a-f\d]{4}){3}-[a-f\d]{12}\.[a-f\d]{64}$/i;
export function readEntry(url: URL) {
  const match = url.pathname.match(/^\/sessions\/([^/]+)\/live\/?$/);
  let pending: PendingLive | null = null;
  let error = '';
  try {
    const fragment = new URLSearchParams(url.hash.slice(1)), grant = fragment.get('grant'), workspace = fragment.get('org');
    if (match && grant && grant.length <= 16384) {
      if (fragment.has('test') || (fragment.has('org') && (fragment.getAll('org').length !== 1 || workspace !== 'browser'))) throw new Error('This invitation belongs to an older workspace. Request a new Browser invitation.');
      pending = { id: decodeURIComponent(match[1]!), grant };
    }
  } catch (cause) { error = cause instanceof URIError ? 'The live invitation has an invalid session ID. Request a new invitation.' : (cause as Error).message; }
  const state = url.searchParams.get('state');
  const callback = url.pathname === '/auth/callback' ? { nonce: state, attempt: STATE.test(state || '') ? state!.split('.')[0]! : null, code: url.searchParams.get('code') } : null;
  if (callback && url.searchParams.has('error')) error = 'Sign-in was not completed. Start sign-in again from Browser.';
  return { pending, callback, cleanPath: url.pathname, error };
}
export interface LoginAttempt { attempt_id: string; state: string; authorize_url: string }
export interface LoginCredential { attempt_id: string; state: string; code: string }
export interface LoginCallback { nonce: string | null; attempt: string | null; code: string | null }
const REDIRECT_ATTEMPT = 'silicon-browser:accounts-attempt';
export function loginUrl(origin: string, attempt: LoginAttempt) {
  const login = new URL(attempt.authorize_url), callback = new URL(login.searchParams.get('redirect_uri') || '', origin);
  if (attempt.state.split('.')[0] !== attempt.attempt_id || !STATE.test(attempt.state) || login.origin !== ACCOUNTS_ORIGIN || login.username || login.password || login.hash || login.pathname !== '/authorize' ||
      login.searchParams.get('app_id') !== ACCOUNTS_APP_ID || login.searchParams.get('state') !== attempt.state || login.searchParams.get('response_type') !== 'code' ||
      login.searchParams.get('code_challenge_method') !== 'S256' || !/^[A-Za-z0-9_-]{43}$/.test(login.searchParams.get('code_challenge') || '') ||
      callback.origin !== origin || callback.pathname !== '/auth/callback' || callback.search || callback.hash || callback.username || callback.password) throw new Error('The sign-in address did not match this Browser attempt. Start sign-in again.');
  return login.href;
}
export function matchingCallback(event: Pick<MessageEvent, 'origin' | 'source' | 'data'>, origin: string, popup: Window, attempt: Pick<LoginAttempt, 'attempt_id' | 'state'>): string | null {
  const data = event.data;
  if (event.origin !== origin || event.source !== popup || data?.type !== CALLBACK_TYPE || data?.nonce !== attempt.state || data?.attempt !== attempt.attempt_id || typeof data.code !== 'string' || !CODE.test(data.code)) return null;
  return data.code;
}
export function matchingRedirect(callback: LoginCallback): boolean {
  try {
    const pending = JSON.parse(sessionStorage.getItem(REDIRECT_ATTEMPT) || 'null');
    return pending?.state === callback.nonce && pending?.attempt_id === callback.attempt && Number.isFinite(pending.created) && Date.now() - pending.created >= 0 && Date.now() - pending.created <= 600000 && CODE.test(callback.code || '');
  } catch { return false; }
}
export function clearRedirectAttempt() { sessionStorage.removeItem(REDIRECT_ATTEMPT); }
export function completeCallback(callback: LoginCallback): boolean {
  if (!STATE.test(callback.nonce || '') || !callback.attempt || !CODE.test(callback.code || '') || !window.opener) return false;
  window.opener.postMessage({ type: CALLBACK_TYPE, nonce: callback.nonce, attempt: callback.attempt, code: callback.code }, location.origin);
  return true;
}
export async function signInPopup(createAttempt: () => Promise<LoginAttempt>, signal?: AbortSignal): Promise<LoginCredential> {
  const popup = window.open('about:blank', `browser-sign-in-${crypto.randomUUID()}`, 'popup,width=520,height=720');
  let attempt: LoginAttempt, url: string;
  try { attempt = await createAttempt(); url = loginUrl(location.origin, attempt); } catch (error) { popup?.close(); throw error; }
  if (signal?.aborted || popup?.closed) { popup?.close(); throw new Error('Sign-in cancelled. Your workspace is unchanged.'); }
  if (!popup) {
    sessionStorage.setItem(REDIRECT_ATTEMPT, JSON.stringify({ attempt_id: attempt.attempt_id, state: attempt.state, created: Date.now() }));
    location.assign(url);
    return new Promise(() => {});
  }
  return new Promise((resolve, reject) => {
    const cleanup = () => { window.removeEventListener('message', receive); signal?.removeEventListener('abort', cancel); clearTimeout(timeout); clearInterval(closed); popup.close(); };
    const receive = (event: MessageEvent) => { const code = matchingCallback(event, location.origin, popup, attempt); if (code) { cleanup(); resolve({ attempt_id: attempt.attempt_id, state: attempt.state, code }); } };
    const cancel = () => { cleanup(); reject(new Error('Sign-in cancelled. Your workspace is unchanged.')); };
    const timeout = setTimeout(() => { cleanup(); reject(new Error('Sign-in timed out. Please try again.')); }, 10 * 60 * 1000);
    const closed = setInterval(() => { if (popup.closed) cancel(); }, 500);
    window.addEventListener('message', receive);
    signal?.addEventListener('abort', cancel, { once: true });
    if (signal?.aborted) { cancel(); return; }
    popup.location.href = url; popup.focus();
  });
}
