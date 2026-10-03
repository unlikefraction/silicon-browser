import { acceptAuth } from './api';
import type { AuthSession } from './types';

export const contextKey = (session: AuthSession) => `${session.identity.id}/${session.org.id}`;
interface SavedContexts { active: string | null; sessions: AuthSession[] }

// Each IAM account/organization keeps its own token family in this tab.
export class TabSession {
  private readonly key: string;
  private readonly legacyKey: string;
  constructor(origin: string, private storage: () => Storage = () => window.sessionStorage) {
    this.key = `silicon-browser:session:v2:${origin}`;
    this.legacyKey = `silicon-browser:session:v1:${origin}`;
  }
  private read(): SavedContexts {
    try {
      const storage = this.storage(), raw = storage.getItem(this.key);
      const legacy = raw === null ? storage.getItem(this.legacyKey) : null;
      if (raw === null && legacy === null) return { active: null, sessions: [] };
      const previous = legacy === null ? null : acceptAuth(JSON.parse(legacy));
      const value = previous ? { active: contextKey(previous), sessions: [previous] } : JSON.parse(raw!) as SavedContexts;
      if (!Array.isArray(value.sessions) || (value.active !== null && typeof value.active !== 'string')) throw new Error('Invalid saved workspaces');
      for (const session of value.sessions) acceptAuth(session);
      if (previous) storage.setItem(this.key, JSON.stringify(value));
      storage.removeItem(this.legacyKey);
      return value;
    } catch {
      try { this.storage().removeItem(this.key); this.storage().removeItem(this.legacyKey); } catch { /* Storage may be disabled entirely. */ }
      return { active: null, sessions: [] };
    }
  }
  load(): AuthSession | null {
    const value = this.read();
    return value.sessions.find(session => contextKey(session) === value.active) ?? null;
  }
  all(): AuthSession[] { return this.read().sessions; }
  save(session: AuthSession | null) {
    try {
      const value = this.read();
      const key = session ? contextKey(session) : value.active;
      value.sessions = value.sessions.filter(item => contextKey(item) !== key);
      if (session) value.sessions.push(session);
      value.active = session ? key : null;
      if (value.sessions.length) this.storage().setItem(this.key, JSON.stringify(value));
      else this.storage().removeItem(this.key);
    } catch {
      throw new Error('Browser could not save your sign-in. Allow website storage and try again.');
    }
  }
}
