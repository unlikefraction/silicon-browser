import type { AuthSession } from './types';

export class ApiError extends Error {
  constructor(message: string, public authRejected = false, public status?: number) { super(message); }
}
export function publicError(value: unknown): string {
  const message = value instanceof Error ? value.message : 'Something went wrong. Please try again.';
  return message.replace(/browser[ -]?use|tiny[ -]?fish/gi, 'browser service');
}
export function acceptAuth(value: AuthSession, org?: string, identity?: string, kind?: AuthSession['identity']['kind']): AuthSession {
  if (!/^[-A-Za-z0-9_]+\.[-A-Za-z0-9_]+\.[-A-Za-z0-9_]+$/.test(value?.access_token || '') || value.access_token.length > 16384 ||
      !/^sar_[A-Za-z0-9_-]{1,16380}$/.test(value?.refresh_token || '') ||
      value.org?.id !== 'browser' || (org !== undefined && value.org.id !== org) || !validIdentity(value.identity) || (identity && value.identity.id !== identity) || (kind && value.identity.kind !== kind) ||
      !Number.isFinite(Date.parse(value.expires_at))) throw new Error('Sign-in did not match your Browser account. Please sign in again.');
  return value;
}
function validIdentity(value: AuthSession['identity']): boolean {
  return typeof value?.name === 'string' && typeof value.id === 'string' && /^[a-zA-Z0-9]{3,128}$/.test(value.id) && ['carbon', 'silicon'].includes(value.kind);
}
export class BrowserApi {
  private session: AuthSession | null = null;
  private generation = 0;
  private refreshing: Promise<void> | null = null;
  private closed = false;
  constructor(readonly origin: string, private fetcher: typeof fetch = fetch.bind(globalThis), private persist?: (session: AuthSession | null, previous: AuthSession | null) => void, private renew?: (session: AuthSession, exchange: (latest: AuthSession) => Promise<AuthSession>) => Promise<AuthSession>) {}
  close() { this.closed = true; this.session = null; this.generation++; this.refreshing = null; }
  setSession(session: AuthSession | null, persist = true) {
    if (persist) this.persist?.(session, this.session);
    this.session = session; this.generation++;
    this.refreshing = null;
  }
  currentSession() { return this.session; }
  async request<T>(path: string, method = 'GET', body?: unknown, session = this.session, idempotencyKey?: string): Promise<T> {
    if (this.closed) throw new ApiError('This workspace has been closed.');
    const endpoint = `${this.origin}/api/v1${path}`;
    const headers: Record<string, string> = { Accept: 'application/json' };
    if (session) { headers.Authorization = `Bearer ${session.access_token}`; headers['X-Org-Id'] = session.org.id; }
    if (idempotencyKey) headers['Idempotency-Key'] = idempotencyKey;
    if (body !== undefined) headers['Content-Type'] = 'application/json';
    let response: Response;
    try { response = await this.fetcher(endpoint, { method, headers, body: body === undefined ? undefined : JSON.stringify(body), credentials: 'omit', cache: 'no-store', redirect: 'error' }); }
    catch { throw new ApiError('Connection interrupted. A submitted change may have completed. Refresh its status before trying again.'); }
    let envelope;
    try { envelope = await response.json(); } catch { throw new ApiError(`The server returned an unreadable response (${response.status}). Refresh status before repeating a change.`); }
    if (this.closed) throw new ApiError('This workspace has been closed.');
    if (!response.ok) {
      const error = envelope.error || {};
      const fields = Array.isArray(error.fields) ? error.fields.map((field: {field: string; message: string}) => `${field.field}: ${field.message}`).join(' · ') : '';
      throw new ApiError(publicError(new Error(`${error.message || 'Request failed'}${fields ? ` — ${fields}` : ''}${error.request_id ? ` (Request ${error.request_id})` : ''}`)), response.status === 401 && response.headers.get('x-sb-auth-rejected') === '1', response.status);
    }
    return envelope.data as T;
  }
  private async refresh(session: AuthSession, generation: number) {
    if (!this.session || generation !== this.generation) throw new ApiError('Sign-in changed. Please try again.');
    if (session !== this.session) return;
    if (!this.refreshing) {
      const exchange = async (latest: AuthSession) => {
        if (this.session !== session || generation !== this.generation) throw new ApiError('Sign-in changed during renewal.');
        const result = await this.request<AuthSession>('/auth/refresh', 'POST', { refresh_token: latest.refresh_token, org_id: latest.org.id }, null);
        if (!this.renew && (this.session !== session || generation !== this.generation)) throw new ApiError('Sign-in changed during renewal.');
        return acceptAuth(result, session.org.id, session.identity.id);
      };
      // Shared browser storage coordinates rotation across every open tab.
      if (!this.renew) this.persist?.(null, session);
      const operation = (this.renew ? this.renew(session, exchange) : exchange(session))
        .then(result => {
          if (this.session !== session || generation !== this.generation) throw new ApiError('Sign-in changed during renewal.');
          const renewed = acceptAuth(result, session.org.id, session.identity.id);
          if (!this.renew) this.persist?.(renewed, session);
          this.session = renewed;
        }).catch(error => {
          if (this.session === session && generation === this.generation) {
            if (this.renew) this.setSession(null, false);
            else this.setSession(null);
            throw new ApiError('Your sign-in could not be renewed. Sign in again.');
          }
          throw error;
        });
      this.refreshing = operation;
      // A lost Accounts refresh response must not resend a possibly spent token.
      void operation.then(() => { if (this.refreshing === operation) this.refreshing = null; }, () => {});
    }
    await this.refreshing;
  }
  async call<T>(path: string, method = 'GET', body?: unknown, idempotencyKey?: string): Promise<T> {
    const session = this.session, generation = this.generation;
    if (!session) throw new ApiError('Sign in to continue.');
    if (Date.parse(session.expires_at) <= Date.now() + 60000) await this.refresh(session, generation);
    if (!this.session || generation !== this.generation) throw new ApiError('Sign-in changed. Please try again.');
    const sent = this.session;
    try {
      const result = await this.request<T>(path, method, body, sent, idempotencyKey);
      if (generation !== this.generation) throw new ApiError('Sign-in changed. Please try again.');
      return result;
    }
    catch (error) {
      // Only a verified pre-handler auth rejection permits one replay of a mutation.
      if (!(error instanceof ApiError) || !error.authRejected) throw error;
      await this.refresh(sent, generation);
      if (!this.session || generation !== this.generation) throw new ApiError('Sign-in changed. Please try again.');
      const result = await this.request<T>(path, method, body, this.session, idempotencyKey);
      if (generation !== this.generation) throw new ApiError('Sign-in changed. Please try again.');
      return result;
    }
  }
}
export function safeHttps(value?: string, disallowedOrigin?: string): string | null {
  if (!value) return null;
  try { const url = new URL(value); return url.protocol === 'https:' && !url.username && !url.password && url.origin !== disallowedOrigin ? url.href : null; } catch { return null; }
}
export const segment = encodeURIComponent;
export const shellQuote = (value: string) => `'${value.replaceAll("'", "'\"'\"'")}'`;
export function dateForApi(value: string) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value) || !Number.isFinite(Date.parse(`${value}T00:00:00Z`)) || new Date(`${value}T00:00:00Z`).toISOString().slice(0, 10) !== value) throw new Error('Choose a valid date.');
  return value;
}
