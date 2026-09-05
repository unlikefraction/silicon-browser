import type { AuthSession } from './types';

export class ApiError extends Error {
  constructor(message: string, public authRejected = false) { super(message); }
}
export function publicError(value: unknown): string {
  const message = value instanceof Error ? value.message : 'Something went wrong. Please try again.';
  return message.replace(/browser[ -]?use|tiny[ -]?fish/gi, 'browser service');
}
export function acceptAuth(value: AuthSession, org: string, identity?: string): AuthSession {
  if (!value?.access_token?.startsWith('oat_') || !value?.refresh_token?.startsWith('ort_') ||
      value.org?.id !== org || !value.identity?.id || (identity && value.identity.id !== identity) ||
      !Number.isFinite(Date.parse(value.expires_at))) throw new Error('Sign-in did not match your organization or identity. Please sign in again.');
  return value;
}
export class BrowserApi {
  private session: AuthSession | null = null;
  private generation = 0;
  private refreshing: Promise<void> | null = null;
  constructor(readonly origin: string, private fetcher: typeof fetch = fetch.bind(globalThis)) {}
  setSession(session: AuthSession | null) { this.session = session; this.generation++; }
  currentSession() { return this.session; }
  async request<T>(path: string, method = 'GET', body?: unknown, session = this.session): Promise<T> {
    const headers: Record<string, string> = { Accept: 'application/json' };
    if (session) { headers.Authorization = `Bearer ${session.access_token}`; headers['X-Org-Id'] = session.org.id; }
    if (body !== undefined) headers['Content-Type'] = 'application/json';
    let response: Response;
    try { response = await this.fetcher(`${this.origin}/api/v1${path}`, { method, headers, body: body === undefined ? undefined : JSON.stringify(body), credentials: 'omit', cache: 'no-store', redirect: 'error' }); }
    catch { throw new ApiError('Connection interrupted. A submitted change may have completed. Refresh its status before trying again.'); }
    let envelope;
    try { envelope = await response.json(); } catch { throw new ApiError(`The server returned an unreadable response (${response.status}). Refresh status before repeating a change.`); }
    if (!response.ok) {
      const error = envelope.error || {};
      const fields = Array.isArray(error.fields) ? error.fields.map((field: {field: string; message: string}) => `${field.field}: ${field.message}`).join(' · ') : '';
      throw new ApiError(publicError(new Error(`${error.message || 'Request failed'}${fields ? ` — ${fields}` : ''}${error.request_id ? ` (Request ${error.request_id})` : ''}`)), response.status === 401 && response.headers.get('x-sb-auth-rejected') === '1');
    }
    return envelope.data as T;
  }
  private async refresh(session: AuthSession, generation: number) {
    if (!this.session || generation !== this.generation) throw new ApiError('Sign-in changed. Please try again.');
    if (session !== this.session) return;
    if (!this.refreshing) {
      const operation = this.request<AuthSession>('/auth/refresh', 'POST', { refresh_token: session.refresh_token, org_id: session.org.id }, null)
        .then(result => {
          if (this.session !== session || generation !== this.generation) throw new ApiError('Sign-in changed during renewal.');
          this.session = acceptAuth(result, session.org.id, session.identity.id);
        });
      this.refreshing = operation;
      void operation.finally(() => { if (this.refreshing === operation) this.refreshing = null; }).catch(() => {});
    }
    await this.refreshing;
  }
  async call<T>(path: string, method = 'GET', body?: unknown): Promise<T> {
    const session = this.session, generation = this.generation;
    if (!session) throw new ApiError('Sign in to continue.');
    if (Date.parse(session.expires_at) <= Date.now() + 60000) await this.refresh(session, generation);
    if (!this.session || generation !== this.generation) throw new ApiError('Sign-in changed. Please try again.');
    const sent = this.session;
    try { return await this.request<T>(path, method, body, sent); }
    catch (error) {
      // Only a verified pre-handler auth rejection permits one replay of a mutation.
      if (!(error instanceof ApiError) || !error.authRejected) throw error;
      await this.refresh(sent, generation);
      if (!this.session || generation !== this.generation) throw new ApiError('Sign-in changed. Please try again.');
      return this.request<T>(path, method, body, this.session);
    }
  }
}
export function safeHttps(value?: string): string | null {
  if (!value) return null;
  try { const url = new URL(value); return url.protocol === 'https:' && !url.username && !url.password ? url.href : null; } catch { return null; }
}
export const segment = encodeURIComponent;
export const shellQuote = (value: string) => `'${value.replaceAll("'", "'\"'\"'")}'`;
export function dateForApi(value: string) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value) || !Number.isFinite(Date.parse(`${value}T00:00:00Z`)) || new Date(`${value}T00:00:00Z`).toISOString().slice(0, 10) !== value) throw new Error('Choose a valid date.');
  return value;
}
