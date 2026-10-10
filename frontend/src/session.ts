import { acceptAuth } from './api';
import type { AuthSession } from './types';

export const contextKey = (session: AuthSession) => session.identity.id;
interface SavedContexts { active: string | null; sessions: AuthSession[]; refreshing?: Record<string, string> }
type SessionLock = (name: string, task: () => Promise<AuthSession>) => Promise<AuthSession>;
const empty = (): SavedContexts => ({ active: null, sessions: [] });
const nativeLock: SessionLock = async (name, task) => {
  if (typeof navigator === 'undefined' || !navigator.locks) return Promise.reject(new Error('This browser cannot safely renew a shared sign-in. Sign in again.'));
  return await navigator.locks.request(name, task);
};

// Accounts share rotating credentials across tabs; renewal consumes each token once.
export class BrowserSession {
  readonly storageKey: string;
  private readonly legacyKey: string;
  private readonly lockName: string;
  constructor(origin: string, private storage: () => Storage = () => window.localStorage, private lock: SessionLock = nativeLock, private legacyStorage: () => Storage = () => window.sessionStorage) {
    this.storageKey = `silicon-browser:session:v4:${origin}`;
    this.legacyKey = `silicon-browser:session:v3:${origin}`;
    this.lockName = `silicon-browser:refresh:${origin}`;
  }
  private parse(raw: string): SavedContexts {
    const value = JSON.parse(raw) as SavedContexts;
    if (!Array.isArray(value?.sessions) || (value.active !== null && typeof value.active !== 'string')) throw new Error('Invalid saved contexts');
    if (value.refreshing !== undefined && (!value.refreshing || typeof value.refreshing !== 'object' || Array.isArray(value.refreshing) || Object.values(value.refreshing).some(token => typeof token !== 'string'))) throw new Error('Invalid pending renewal');
    const normalizeKey = (key: string | null) => key?.endsWith('/browser') ? key.slice(0, -8) : key;
    const sessions = value.sessions.map(session => {
      const legacy = session as AuthSession & { org?: { id: string } };
      if (legacy.org !== undefined && legacy.org?.id !== 'browser') throw new Error('Invalid legacy workspace');
      return acceptAuth(session);
    });
    return { active: normalizeKey(value.active), sessions, ...(value.refreshing ? { refreshing: Object.fromEntries(Object.entries(value.refreshing).map(([key, token]) => [normalizeKey(key), token])) } : {}) };
  }
  private clearLegacy() { try { this.legacyStorage().removeItem(this.legacyKey); } catch { /* Tab storage can be unavailable. */ } }
  private read(): SavedContexts {
    try {
      const raw = this.storage().getItem(this.storageKey);
      if (raw !== null) {
        this.clearLegacy();
        try {
          const value = this.parse(raw);
          if (JSON.stringify(value) !== raw) this.write(value);
          return value;
        } catch { this.write(empty()); return empty(); }
      }
      let legacy: string | null = null;
      try { legacy = this.legacyStorage().getItem(this.legacyKey); } catch { /* No legacy tab storage. */ }
      if (legacy === null) return empty();
      let value: SavedContexts;
      try { value = this.parse(legacy); } catch { value = empty(); }
      this.write(value);
      this.clearLegacy();
      return value;
    } catch { return empty(); }
  }
  private write(value: SavedContexts) {
    try { this.storage().setItem(this.storageKey, JSON.stringify(value)); }
    catch { throw new Error('Browser could not save your sign-in. Allow website storage and try again.'); }
  }
  load(): AuthSession | null {
    const value = this.read();
    return value.sessions.find(session => contextKey(session) === value.active) ?? null;
  }
  all(): AuthSession[] { return this.read().sessions; }
  save(session: AuthSession | null, previous?: AuthSession | null) {
    if (!session && previous === null) return;
    const value = this.read();
    const key = session ? contextKey(session) : previous ? contextKey(previous) : value.active;
    const current = value.sessions.find(item => contextKey(item) === key);
    if (!session && previous && current?.refresh_token !== previous.refresh_token) return;
    value.sessions = value.sessions.filter(item => contextKey(item) !== key);
    if (key && value.refreshing && (!session || current?.refresh_token !== session.refresh_token)) delete value.refreshing[key];
    if (session) {
      value.sessions.push(acceptAuth(session));
      value.active = key;
    } else if (value.active === key) value.active = null;
    this.write(value);
    this.clearLegacy();
  }
  forget(session: AuthSession) {
    const value = this.read(), key = contextKey(session);
    value.sessions = value.sessions.filter(item => contextKey(item) !== key);
    if (value.refreshing) delete value.refreshing[key];
    if (value.active === key) value.active = null;
    this.write(value);
    this.clearLegacy();
  }
  refresh(session: AuthSession, renew: (latest: AuthSession) => Promise<AuthSession>): Promise<AuthSession> {
    return this.lock(this.lockName, async () => {
      const value = this.read(), key = contextKey(session);
      const latest = value.sessions.find(item => contextKey(item) === key);
      if (value.active !== key || !latest) throw new Error('Sign-in changed. Please try again.');
      if (value.refreshing?.[key] === latest.refresh_token) {
        this.save(null, latest);
        throw new Error('Your sign-in renewal was interrupted. Sign in again.');
      }
      if (latest.refresh_token !== session.refresh_token) return latest;
      value.refreshing ??= {};
      value.refreshing[key] = latest.refresh_token;
      this.write(value);
      try {
        const renewed = acceptAuth(await renew(latest), latest.identity.id, latest.identity.kind);
        const current = this.read();
        const stored = current.sessions.find(item => contextKey(item) === key);
        if (stored?.refresh_token !== latest.refresh_token || current.refreshing?.[key] !== latest.refresh_token) throw new Error('Sign-in changed during renewal.');
        current.sessions = current.sessions.map(item => contextKey(item) === key ? renewed : item);
        delete current.refreshing![key];
        this.write(current);
        return renewed;
      } catch (error) {
        this.save(null, latest);
        throw error;
      }
    });
  }
}
