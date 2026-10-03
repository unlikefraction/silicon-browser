import type { PendingLive } from './types';
export const IAM_AUTH_ORIGIN = 'https://auth.iam.teamofsilicons.com';
export const IAM_APP_ID = 'browser';
const CALLBACK_TYPE = 'silicon-browser:sign-in';
export function readEntry(url: URL) {
  const match = url.pathname.match(/^\/sessions\/([^/]+)\/live\/?$/);
  let pending: PendingLive | null = null;
  let liveId: string | null = null;
  let error = '';
  try {
    if (match) liveId = decodeURIComponent(match[1]);
    const fragment = new URLSearchParams(url.hash.slice(1)), grant = fragment.get('grant'), environment = fragment.get('test');
    if (match && grant && grant.length <= 16384) {
      if (fragment.has('test') && (fragment.getAll('test').length !== 1 || !/^[a-f\d]{8}(?:-[a-f\d]{4}){3}-[a-f\d]{12}$/i.test(environment || ''))) throw new Error('The live invitation has an invalid test environment. Request a new invitation.');
      pending = { id: liveId!, grant, ...(environment ? { testEnvironmentId: environment.toLowerCase() } : {}) };
    }
  } catch (cause) { error = cause instanceof URIError ? 'The live invitation has an invalid session ID. Request a new invitation.' : (cause as Error).message; }
  const callback = url.pathname === '/auth/callback' ? { attempt_id: url.searchParams.get('attempt_id'), state: url.searchParams.get('state'), token: url.searchParams.get('slt') } : null;
  return { pending, liveId: error ? null : liveId, callback, cleanPath: url.pathname, error };
}
export function requireLiveEnvironment(link: PendingLive, environmentId?: string) {
  if (link.testEnvironmentId !== environmentId) throw new Error(link.testEnvironmentId
    ? `This live invitation requires testing environment ${link.testEnvironmentId}. Open Testing environment and enroll it before continuing.`
    : 'This live invitation belongs to production. Exit test mode before opening it.');
}
export type IdentityKind = 'carbon' | 'silicon';
export interface LoginAttempt { attempt_id: string; state: string; identity_kind: IdentityKind; expires_at: string }
export interface LoginCallback { attempt_id: string | null; state: string | null; token: string | null }
export interface PendingLogin extends LoginAttempt { return_path: string }
export function acceptLoginAttempt(value: LoginAttempt): LoginAttempt {
  if (!/^[a-f\d]{8}(?:-[a-f\d]{4}){3}-[a-f\d]{12}$/i.test(value?.attempt_id || '') ||
      !/^[a-f\d]{64}$/.test(value?.state || '') || !['carbon', 'silicon'].includes(value?.identity_kind) ||
      !Number.isFinite(Date.parse(value?.expires_at)) || Date.parse(value.expires_at) <= Date.now()) throw new Error('This sign-in attempt has expired or is invalid. Start sign-in again.');
  return value;
}
/** Full-page fallback stores only the attempt binding and local destination, never an SLT or live grant. */
export class PendingLoginStore {
  private readonly key: string;
  constructor(origin: string, private storage: () => Storage = () => window.sessionStorage) { this.key = `silicon-browser:login-attempt:v1:${origin}`; }
  save(attempt: LoginAttempt, returnPath: string) {
    acceptLoginAttempt(attempt);
    const path = new URL(returnPath, 'https://browser.invalid');
    if (path.origin !== 'https://browser.invalid' || path.search || path.hash || !/^\/(?:sessions\/[^/]+\/live\/?)?$/.test(path.pathname)) throw new Error('Invalid sign-in return destination.');
    const { attempt_id, state, identity_kind, expires_at } = attempt;
    this.storage().setItem(this.key, JSON.stringify({ attempt_id, state, identity_kind, expires_at, return_path: path.pathname }));
  }
  load(callback: LoginCallback): PendingLogin {
    try {
      const pending = JSON.parse(this.storage().getItem(this.key) || 'null') as PendingLogin;
      acceptLoginAttempt(pending);
      if (pending.attempt_id !== callback.attempt_id || pending.state !== callback.state ||
          typeof pending.return_path !== 'string' || !/^\/(?:sessions\/[^/]+\/live\/?)?$/.test(pending.return_path) || /[?#\\]/.test(pending.return_path)) throw new Error();
      return pending;
    } catch { throw new Error('This sign-in link does not match an active attempt in this tab. Start sign-in again.'); }
  }
  clear() { this.storage().removeItem(this.key); }
}
export function loginUrl(origin: string, attempt: LoginAttempt, popup = true) {
  acceptLoginAttempt(attempt);
  const callback = new URL('/auth/callback', origin);
  callback.searchParams.set('attempt_id', attempt.attempt_id); callback.searchParams.set('state', attempt.state);
  const login = new URL('/login', IAM_AUTH_ORIGIN);
  login.searchParams.set('identity_kind', attempt.identity_kind);
  if (popup) login.searchParams.set('display', 'popup');
  login.searchParams.set('app_id', IAM_APP_ID); login.searchParams.set('redirect_uri', callback.href);
  return login.href;
}
export function matchingCallback(event: Pick<MessageEvent, 'origin' | 'source' | 'data'>, origin: string, popup: Window, attempt: Pick<LoginAttempt, 'attempt_id' | 'state'>): string | null {
  const data = event.data;
  if (event.origin !== origin || event.source !== popup || data?.type !== CALLBACK_TYPE ||
      data?.attempt_id !== attempt.attempt_id || data?.state !== attempt.state ||
      typeof data.token !== 'string' || !/^oac_[^\s\x00-\x1f\x7f]{1,16380}$/.test(data.token)) return null;
  return data.token;
}
/** The callback hands off its one-use token and waits for verified completion from its exact opener. */
export function completeCallback(callback: LoginCallback, failed: () => void): boolean {
  const opener = window.opener;
  if (!opener || !callback.attempt_id || !callback.state || !callback.token) return false;
  const payload = { type: CALLBACK_TYPE, ...callback };
  const binding = { attempt_id: callback.attempt_id, state: callback.state };
  if (!matchingCallback({ origin: location.origin, source: opener, data: payload }, location.origin, opener, binding)) return false;
  const receive = (event: MessageEvent) => {
    if (event.origin !== location.origin || event.source !== opener || event.data?.attempt_id !== callback.attempt_id || event.data?.state !== callback.state) return;
    if (event.data.type === CALLBACK_TYPE + ':complete') { window.removeEventListener('message', receive); window.close(); }
    if (event.data.type === CALLBACK_TYPE + ':failed') { window.removeEventListener('message', receive); failed(); }
  };
  window.addEventListener('message', receive);
  opener.postMessage(payload, location.origin);
  return true;
}
/** Reserve the popup during the click; blocked popups use the same bound attempt in this tab. */
export async function signInPopup(kind: IdentityKind, start: () => Promise<LoginAttempt>, complete: (token: string, attempt: LoginAttempt, signal: AbortSignal) => Promise<unknown>, pending: PendingLoginStore, returnPath: string, signal?: AbortSignal): Promise<boolean> {
  const popup = window.open('about:blank', `browser-sign-in-${crypto.randomUUID()}`, 'popup,width=520,height=720');
  let attempt: LoginAttempt;
  try {
    pending.clear();
    attempt = acceptLoginAttempt(await start());
    if (attempt.identity_kind !== kind) throw new Error('The sign-in account type did not match. Start again.');
    if (signal?.aborted) throw new Error('Sign-in cancelled.');
    if (!popup) {
      pending.save(attempt, returnPath);
      location.assign(loginUrl(location.origin, attempt, false));
      return false;
    }
  } catch (error) { popup?.close(); throw error; }
  return new Promise((resolve, reject) => {
    let completing = false;
    const operation = new AbortController();
    const cleanup = () => { window.removeEventListener('message', receive); signal?.removeEventListener('abort', cancel); clearTimeout(timeout); clearInterval(closed); };
    const cancel = () => { operation.abort(); cleanup(); popup.close(); reject(new Error('Sign-in cancelled.')); };
    const receive = async (event: MessageEvent) => {
      const token = matchingCallback(event, location.origin, popup, attempt);
      if (!token || completing) return;
      completing = true;
      try {
        await complete(token, attempt, operation.signal);
        if (operation.signal.aborted || popup.closed) throw new Error('Sign-in cancelled.');
        popup.postMessage({ type: CALLBACK_TYPE + ':complete', attempt_id: attempt.attempt_id, state: attempt.state }, location.origin);
        cleanup(); popup.close(); resolve(true);
      } catch (error) {
        popup.postMessage({ type: CALLBACK_TYPE + ':failed', attempt_id: attempt.attempt_id, state: attempt.state }, location.origin);
        cleanup(); reject(error);
      }
    };
    const timeout = setTimeout(cancel, Math.max(0, Math.min(10 * 60 * 1000, Date.parse(attempt.expires_at) - Date.now())));
    const closed = setInterval(() => { if (popup.closed) cancel(); }, 500);
    window.addEventListener('message', receive); signal?.addEventListener('abort', cancel, { once: true });
    if (signal?.aborted) { cancel(); return; }
    popup.location.href = loginUrl(location.origin, attempt); popup.focus();
  });
}
