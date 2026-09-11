import type { PendingLive } from './types';
export const IAM_AUTH_ORIGIN = 'https://auth.iam.teamofsilicons.com';
export const IAM_APP_ID = 'tos>browser';
const CALLBACK_TYPE = 'silicon-browser:sign-in';
export function readEntry(url: URL) {
  const match = url.pathname.match(/^\/sessions\/([^/]+)\/live\/?$/);
  let pending: PendingLive | null = null;
  try { const grant = new URLSearchParams(url.hash.slice(1)).get('grant'); if (match && grant && grant.length <= 16384) pending = { id: decodeURIComponent(match[1]), grant }; } catch { /* malformed handoff */ }
  const callback = url.pathname === '/auth/callback' ? { nonce: url.searchParams.get('nonce'), token: url.searchParams.get('slt') } : null;
  return { pending, callback, cleanPath: url.pathname };
}
export function loginUrl(origin: string, nonce: string) {
  const callback = new URL('/auth/callback', origin); callback.searchParams.set('nonce', nonce);
  const login = new URL('/login', IAM_AUTH_ORIGIN);
  login.searchParams.set('app_id', IAM_APP_ID); login.searchParams.set('redirect_uri', callback.href);
  return login.href;
}
export function matchingCallback(event: Pick<MessageEvent, 'origin' | 'source' | 'data'>, origin: string, popup: Window, nonce: string): string | null {
  const data = event.data;
  if (event.origin !== origin || event.source !== popup || data?.type !== CALLBACK_TYPE || data?.nonce !== nonce ||
      typeof data.token !== 'string' || !/^oac_[^\s\x00-\x1f\x7f]{1,16380}$/.test(data.token)) return null;
  return data.token;
}
export function matchingBroadcast(data: unknown, nonce: string): string | null {
  const value = data as { type?: string; nonce?: string; token?: string } | null;
  if (value?.type !== CALLBACK_TYPE || value.nonce !== nonce || typeof value.token !== 'string' ||
      !/^oac_[^\s\x00-\x1f\x7f]{1,16380}$/.test(value.token)) return null;
  return value.token;
}
export function completeCallback(callback: {nonce: string | null; token: string | null}): boolean {
  if (!callback.nonce || !/^[a-f0-9-]{36}$/.test(callback.nonce) || !callback.token) return false;
  const payload = { type: CALLBACK_TYPE, nonce: callback.nonce, token: callback.token };
  if (!matchingBroadcast(payload, callback.nonce)) return false;
  // Some sign-in pages sever window.opener. A same-origin channel whose name contains
  // the initiating random nonce preserves the handoff without persisting credentials.
  const channel = new BroadcastChannel(`silicon-browser:auth:${callback.nonce}`);
  channel.postMessage(payload);
  if (window.opener) window.opener.postMessage(payload, location.origin);
  setTimeout(() => { channel.close(); window.close(); }, 100);
  return true;
}
export function signInPopup(signal?: AbortSignal): Promise<string> {
  const nonce = crypto.randomUUID();
  const channel = new BroadcastChannel(`silicon-browser:auth:${nonce}`);
  const popup = window.open('about:blank', `browser-sign-in-${nonce}`, 'popup,width=520,height=720');
  if (!popup) { channel.close(); return Promise.reject(new Error('Allow pop-ups for this website, then select Continue with IAM again.')); }
  return new Promise((resolve, reject) => {
    const cleanup = () => { window.removeEventListener('message', receive); signal?.removeEventListener('abort', cancel); channel.close(); clearTimeout(timeout); popup.close(); };
    const accept = (token: string | null) => { if (token) { cleanup(); resolve(token); } };
    const receive = (event: MessageEvent) => accept(matchingCallback(event, location.origin, popup, nonce));
    const cancel = () => { cleanup(); reject(new Error('Sign-in cancelled.')); };
    const timeout = setTimeout(() => { cleanup(); reject(new Error('Sign-in timed out. Please try again.')); }, 10 * 60 * 1000);
    channel.onmessage = event => accept(matchingBroadcast(event.data, nonce));
    window.addEventListener('message', receive);
    signal?.addEventListener('abort', cancel, { once: true });
    if (signal?.aborted) { cancel(); return; }
    popup.location.href = loginUrl(location.origin, nonce);
    popup.focus();
  });
}
