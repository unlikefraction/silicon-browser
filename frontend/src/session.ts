import { acceptAuth } from './api';
import type { AuthSession } from './types';

// sessionStorage survives reloads and is scoped to this frontend origin and tab.
// Codes and live grants are never saved. Keep development and production separate.
export class TabSession {
  private readonly key: string;
  constructor(origin: string, private storage: () => Storage = () => window.sessionStorage) {
    this.key = `silicon-browser:session:v1:${origin}`;
  }
  load(): AuthSession | null {
    try {
      const raw = this.storage().getItem(this.key);
      if (!raw) return null;
      const value = JSON.parse(raw);
      if (typeof value?.org?.id !== 'string' || !value.org.id ||
          typeof value?.identity?.name !== 'string' || !['carbon', 'silicon'].includes(value?.identity?.kind)) throw new Error('Invalid saved session');
      // An expired access token can still have a valid refresh token.
      return acceptAuth(value, value.org.id);
    } catch {
      try { this.save(null); } catch { /* Storage may be disabled entirely. */ }
      return null;
    }
  }
  save(session: AuthSession | null) {
    try {
      if (session) this.storage().setItem(this.key, JSON.stringify(session));
      else this.storage().removeItem(this.key);
    } catch {
      throw new Error('Browser could not save your sign-in. Allow website storage and try again.');
    }
  }
}
