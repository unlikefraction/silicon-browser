import type { AuthSession } from './types';

export interface TestingCredentials { app_secret: string; iam_test_key?: string; briefcase_test_environment_key?: string }
export interface TestingContext { environment_id: string; app_id: string; name: string }

export class ApiError extends Error {
  constructor(message: string, public authRejected = false, public status?: number) { super(message); }
}
export function publicError(value: unknown): string {
  const message = value instanceof Error ? value.message : 'Something went wrong. Please try again.';
  return message.replace(/browser[ -]?use|tiny[ -]?fish/gi, 'browser service');
}
export function acceptAuth(value: AuthSession, org?: string, identity?: string): AuthSession {
  if (!/^oat_[^\s\x00-\x1f\x7f]{1,16380}$/.test(value?.access_token || '') ||
      !/^ort_[^\s\x00-\x1f\x7f]{1,16380}$/.test(value?.refresh_token || '') ||
      !value.org?.id || (org !== undefined && value.org.id !== org) || !value.identity?.id || (identity && value.identity.id !== identity) ||
      !Number.isFinite(Date.parse(value.expires_at))) throw new Error('Sign-in did not match your organization or identity. Please sign in again.');
  return value;
}
export class BrowserApi {
  private session: AuthSession | null = null;
  private generation = 0;
  private refreshing: Promise<void> | null = null;
  private closed = false;
  constructor(readonly origin: string, private fetcher: typeof fetch = fetch.bind(globalThis), private persist?: (session: AuthSession | null) => void, private testing?: TestingCredentials) {}
  async startTesting(credentials: TestingCredentials, token: string, org: string, expectedEnvironmentId?: string): Promise<{ api: BrowserApi; context: TestingContext }> {
    if (!/^[^\s\x00-\x1f\x7f]{1,16384}$/.test(token)) throw new ApiError('Enter an existing IAM test actor ID or an IAM-issued test short-lived token (oac_) without whitespace.');
    if (!org.trim()) throw new ApiError('Enter the organization granted to the test token.');
    for (const [name, value] of Object.entries(credentials)) {
      if (typeof value !== 'string' || !/^[^\s\x00-\x1f\x7f]{1,16384}$/.test(value)) throw new ApiError(`Enter a valid ${name.replaceAll('_', ' ')} without whitespace.`);
    }
    if (!credentials.app_secret) throw new ApiError('Browser test app secret is required.');
    const context = await this.request<TestingContext>('/testing/context', 'POST', credentials, null);
    if (context?.app_id !== 'tos>browser' || !/^[a-f\d]{8}(?:-[a-f\d]{4}){3}-[a-f\d]{12}$/i.test(context.environment_id) || typeof context.name !== 'string') throw new ApiError('The server returned an invalid Browser testing environment. Your current workspace has not changed.');
    context.environment_id = context.environment_id.toLowerCase();
    if (expectedEnvironmentId && context.environment_id.toLowerCase() !== expectedEnvironmentId.toLowerCase()) throw new ApiError(`This invitation requires testing environment ${expectedEnvironmentId}. The supplied app secret selects a different environment.`);
    // A separate client keeps test credentials and renewals out of production storage.
    const api = new BrowserApi(`${this.origin}/testing/${context.environment_id}`, this.fetcher, undefined, { ...credentials });
    try {
      api.setSession(acceptAuth(await api.request<AuthSession>('/auth/exchange', 'POST', { short_lived_token: token, org_id: org.trim() }, null), org.trim()));
      return { api, context };
    } catch (error) { api.close(); throw error; }
  }
  close() { this.closed = true; this.session = null; this.testing = undefined; this.generation++; this.refreshing = null; }
  setSession(session: AuthSession | null) {
    this.persist?.(session);
    this.session = session; this.generation++;
    this.refreshing = null;
  }
  currentSession() { return this.session; }
  async request<T>(path: string, method = 'GET', body?: unknown, session = this.session): Promise<T> {
    if (this.closed) throw new ApiError('This workspace has been closed.');
    let endpoint = `${this.origin}/api/v1${path}`;
    if (this.testing) {
      const base = new URL(this.origin), target = new URL(endpoint);
      if (target.origin !== base.origin || !target.pathname.startsWith(`${base.pathname}/api/v1/`) || /%2f|%5c/i.test(target.pathname) || target.hash) throw new ApiError('Test requests must stay inside this environment’s API path.');
      endpoint = target.href;
    }
    const headers: Record<string, string> = { Accept: 'application/json' };
    if (this.testing) {
      headers['x-sb-test-app-secret'] = this.testing.app_secret;
      if (this.testing.iam_test_key) headers['x-testing-environment-key'] = this.testing.iam_test_key;
      if (this.testing.briefcase_test_environment_key) headers['x-sb-test-briefcase-key'] = this.testing.briefcase_test_environment_key;
    }
    if (session) { headers.Authorization = `Bearer ${session.access_token}`; headers['X-Org-Id'] = session.org.id; }
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
      const operation = this.request<AuthSession>('/auth/refresh', 'POST', { refresh_token: session.refresh_token, org_id: session.org.id }, null)
        .then(result => {
          if (this.session !== session || generation !== this.generation) throw new ApiError('Sign-in changed during renewal.');
          const renewed = acceptAuth(result, session.org.id, session.identity.id);
          this.persist?.(renewed);
          this.session = renewed;
        }).catch(error => {
          if (this.session === session && generation === this.generation && error instanceof ApiError && [401, 403].includes(error.status || 0)) {
            this.setSession(null);
            throw new ApiError('Your sign-in has ended. Please sign in again.');
          }
          throw error;
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
    try {
      const result = await this.request<T>(path, method, body, sent);
      if (generation !== this.generation) throw new ApiError('Sign-in changed. Please try again.');
      return result;
    }
    catch (error) {
      // Only a verified pre-handler auth rejection permits one replay of a mutation.
      if (!(error instanceof ApiError) || !error.authRejected) throw error;
      await this.refresh(sent, generation);
      if (!this.session || generation !== this.generation) throw new ApiError('Sign-in changed. Please try again.');
      return this.request<T>(path, method, body, this.session);
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
