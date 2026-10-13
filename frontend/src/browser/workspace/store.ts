// The workspace controller: a 1:1 port of the state and flows in the former src/main.tsx.
// State is read synchronously (like the Solid signals it replaces), so single-flight checks, revision tickets,
// and late-response guards behave exactly as before. React subscribes through useSyncExternalStore.
import type { FormEvent } from 'react';
import { BrowserApi, acceptAuth, publicError, safeHttps, segment } from '@/lib/api';
import { readEntry, signInPopup, type LoginAttempt } from '@/lib/auth';
import { splitAccess } from '@/lib/format';
import { BrowserSession, contextKey } from '@/lib/session';
import type { AuthSession, Location, PendingLive, Profile, Recording, Session, Usage, UsageLimits } from '@/lib/types';

export const tabs = ['sessions', 'profiles', 'recordings', 'usage', 'settings'] as const;
export type Tab = typeof tabs[number];
export type View = Tab | 'new-session' | 'new-profile' | 'edit-profile' | 'detail' | 'live';
export const tabLabels: Record<Tab, string> = { sessions: 'Sessions', profiles: 'Profiles', recordings: 'Recordings', usage: 'Usage', settings: 'Settings' };

/** The rail tab a view belongs to. */
export function activeTab(view: View): Tab {
  if (view === 'detail' || view === 'live' || view === 'new-session') return 'sessions';
  if (view === 'new-profile' || view === 'edit-profile') return 'profiles';
  return view;
}

export interface WorkspaceState {
  /** The signed-in account, or null when signed out. */
  auth: AuthSession | null;
  /** Every account saved in this browser (the rail's account list). */
  contexts: AuthSession[];
  /** A `perform` task is running. Every control except "Cancel sign-in" is disabled. */
  busy: boolean;
  /** A view is loading; the view area shows the loading skeleton instead. */
  loading: boolean;
  /** The current workspace message (errors from `perform`, entry errors). Empty when there is none. */
  notice: string;
  /** The Carbon sign-in window is open. */
  signingIn: boolean;
  /** The "Add an account" panel is open. */
  addingContext: boolean;
  /** The Silicon token form is expanded. */
  showSilicon: boolean;
  view: View;
  profiles: Profile[];
  sessions: Session[];
  /** Newest first. */
  recordings: Recording[];
  locations: Location[];
  usage: Usage[];
  /** Client-side sum of `usage` (sessions = row count). */
  total?: Usage;
  /** Undefined when /usage/limits failed. */
  limits?: UsageLimits;
  /** The session shown by detail and live views. */
  session?: Session;
  /** The profile being edited, or the profile a new session starts from. */
  profile?: Profile;
  /** The redeemed live browser URL (already checked by safeHttps), or ''. */
  liveUrl: string;
  /** The sessions/profiles search text. Cleared on every navigation. */
  filter: string;
  /** A live invitation is waiting for sign-in. */
  pendingInvitation: boolean;
}

export interface WorkspaceToast { type?: 'success' | 'info' | 'warning' | 'error'; title: string; description?: string }

export interface WorkspaceDeps {
  api: BrowserApi;
  saved: BrowserSession;
  entry: { pending: PendingLive | null; error: string };
  restored: AuthSession | null;
}

type Task = () => Promise<unknown>;

export class WorkspaceStore {
  private state: WorkspaceState;
  private readonly listeners = new Set<() => void>();
  private revision = 0;
  private pendingLive: PendingLive | null;
  private recordingRefreshes = 0;
  private signInAbort: AbortController | null = null;
  private restoreStarted = false;
  private readonly api: BrowserApi;
  private readonly saved: BrowserSession;
  private readonly restored: AuthSession | null;
  /** Shows a transient confirmation. The provider connects it to the Silicon UI toast stack. */
  toast: (toast: WorkspaceToast) => void = () => {};

  constructor({ api, saved, entry, restored }: WorkspaceDeps) {
    this.api = api;
    this.saved = saved;
    this.restored = restored;
    this.pendingLive = entry.pending;
    this.state = {
      auth: restored, contexts: saved.all(), busy: false, loading: false, notice: entry.error, signingIn: false, addingContext: false, showSilicon: false,
      view: 'sessions', profiles: [], sessions: [], recordings: [], locations: [], usage: [], liveUrl: '', filter: '', pendingInvitation: !!entry.pending,
    };
  }

  // ---- store plumbing ----
  getState = (): WorkspaceState => this.state;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private set(patch: Partial<WorkspaceState>) {
    this.state = { ...this.state, ...patch };
    for (const listener of this.listeners) listener();
  }
  private setPendingLive(value: PendingLive | null) {
    this.pendingLive = value;
    if (this.state.pendingInvitation !== !!value) this.set({ pendingInvitation: !!value });
  }

  // ---- core flows ----
  /** Runs one task at a time: ignored while busy, clears the notice, shows failures as the notice, and signs out locally if the API dropped the session. */
  perform = async (task: Task) => {
    if (this.state.busy) return;
    this.set({ busy: true, notice: '' });
    try { await task(); } catch (error) {
      if (this.state.auth && !this.api.currentSession()) { this.set({ auth: null, addingContext: false }); this.clearWorkspace(); this.loadContexts(); }
      this.set({ notice: publicError(error) });
    } finally { this.set({ auth: this.api.currentSession(), busy: false }); }
  };

  /** Click handler factory: `onClick={action(() => navigate('sessions'))}` runs the task through `perform`. */
  action = (task: Task) => () => { void this.perform(task); };

  /** Submit handler factory: prevents navigation, reads the form's FormData, and runs the task through `perform`. */
  submit = (task: (data: FormData) => Promise<unknown>) => (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    void this.perform(() => task(data));
  };

  navigate = async (next: Tab) => {
    const ticket = ++this.revision;
    this.set({ view: next, loading: true, filter: '' });
    try {
      if (next === 'sessions') {
        const result = await this.api.call<Session[]>('/sessions');
        if (ticket === this.revision) this.set({ sessions: result });
      } else if (next === 'profiles') {
        const [items, places] = await Promise.all([this.api.call<Profile[]>('/profiles'), this.api.call<Location[]>('/proxy-locations')]);
        if (ticket === this.revision) this.set({ profiles: items, locations: places });
      } else if (next === 'recordings') {
        await this.refreshRecordings(ticket);
      } else if (next === 'usage') {
        const [items, capacity] = await Promise.all([this.api.call<Usage[]>('/usage'), this.api.call<UsageLimits>('/usage/limits').catch(() => undefined)]);
        const sum: Usage = { sessions: items.length, browser_seconds: 0, proxy_bytes_in: 0, proxy_bytes_out: 0, proxy_bytes_unclassified: 0 };
        for (const item of items) {
          sum.browser_seconds += item.browser_seconds; sum.proxy_bytes_in += item.proxy_bytes_in; sum.proxy_bytes_out += item.proxy_bytes_out;
          sum.proxy_bytes_unclassified = (sum.proxy_bytes_unclassified || 0) + (item.proxy_bytes_unclassified || 0);
        }
        if (ticket === this.revision) this.set({ usage: items, total: sum, limits: capacity });
      }
    } finally { if (ticket === this.revision) this.set({ loading: false }); }
  };

  private finishLogin = async (result: AuthSession) => {
    this.api.setSession(result); this.set({ auth: result, addingContext: false, showSilicon: false });
    const invitation = this.pendingLive; this.clearWorkspace(); this.setPendingLive(invitation);
    await this.enterWorkspace();
  };

  /** Carbon sign-in through the Accounts popup (or a same-tab redirect when popups are blocked). Run it through `perform`. */
  login = async () => {
    const abort = new AbortController();
    this.signInAbort = abort; this.set({ signingIn: true });
    try {
      const { attempt_id, ...body } = await signInPopup(() => this.api.request<LoginAttempt>('/auth/attempts', 'POST', { identity_kind: 'carbon' }, null), abort.signal);
      const result = acceptAuth(await this.api.request<AuthSession>(`/auth/attempts/${segment(attempt_id)}/exchange`, 'POST', { ...body, contract_version: 2 }, null), undefined, 'carbon');
      if (abort.signal.aborted) throw new Error('Sign-in cancelled. Your workspace is unchanged.');
      await this.finishLogin(result);
    } finally { this.set({ signingIn: false }); this.signInAbort = null; }
  };

  /** Aborts the Carbon sign-in window. Never disabled by `busy`. */
  cancelSignIn = () => { this.signInAbort?.abort(); };

  /** Silicon sign-in with a short-lived token read from the form's `slt` field. Run it through `perform`; reset the form before calling. */
  siliconLogin = async (data: FormData) => {
    const token = String(data.get('slt') || '').trim();
    if (!/^slt_[A-Za-z0-9_-]{1,16380}$/.test(token)) throw new Error('Enter the short-lived token from silicon-accounts login --app browser.');
    const result = acceptAuth(await this.api.request<AuthSession>('/auth/exchange', 'POST', { short_lived_token: token, contract_version: 2 }, null), undefined, 'silicon');
    await this.finishLogin(result);
  };

  toggleSilicon = () => { this.set({ showSilicon: !this.state.showSilicon }); };

  private loadContexts() { this.set({ contexts: this.saved.all() }); }

  private enterWorkspace = async () => {
    this.loadContexts();
    if (this.pendingLive) {
      const link = this.pendingLive;
      this.setPendingLive(null); await this.openLive(link.id, link.grant);
    } else await this.navigate('sessions');
  };

  /** Switches the workspace to another saved account. Run it through `perform`. */
  switchContext = async (next: AuthSession) => {
    const auth = this.state.auth;
    if (auth && contextKey(auth) === contextKey(next)) return;
    const stored = this.saved.all().find(item => contextKey(item) === contextKey(next));
    if (!stored) throw new Error('This context has signed out. Please sign in again.');
    next = stored;
    this.clearWorkspace(); this.api.setSession(next); this.set({ auth: next }); this.loadContexts();
    await this.enterWorkspace();
  };

  /** Opens the "Add an account" panel (signed in only; signing out closes it). Run it through `perform`, like the rail button always has. */
  addAccount = async () => { this.set({ addingContext: true }); };
  closeAddAccount = () => { this.set({ addingContext: false }); };

  private clearWorkspace() {
    ++this.revision; this.setPendingLive(null);
    this.set({
      liveUrl: '', session: undefined, profile: undefined, notice: '', loading: false, sessions: [], profiles: [], recordings: [], usage: [], total: undefined, limits: undefined,
      locations: [], view: 'sessions', filter: '',
    });
    history.replaceState(null, '', '/');
  }

  /** Signs out the current account only; other saved accounts stay in the rail. */
  logout = () => {
    const current = this.api.currentSession() ?? this.state.auth;
    if (current) this.saved.forget(current);
    this.api.setSession(null, false); this.set({ auth: null, addingContext: false }); this.clearWorkspace(); this.loadContexts();
  };

  /** Reloads recordings (newest first) if `ticket` is still current and the recordings view is open. `background` refreshes are single-flight. */
  refreshRecordings = async (ticket = this.revision, background = false) => {
    if (background && this.recordingRefreshes) return;
    this.recordingRefreshes++;
    try {
      const result = await this.api.call<Recording[]>('/recordings');
      if (ticket === this.revision && this.state.view === 'recordings') this.set({ recordings: result.sort((a, b) => Date.parse(b.created_at) - Date.parse(a.created_at)) });
    } finally { this.recordingRefreshes--; }
  };

  /** Opens the new-session form, optionally starting from a saved profile. */
  newSession = async (selected?: Profile) => {
    this.set({ profile: selected, view: 'new-session' }); ++this.revision;
  };

  /** Opens the new-profile form (no fetch; it reuses the loaded locations). */
  openNewProfile = () => { this.set({ profile: undefined, view: 'new-profile' }); };
  /** Opens the edit form for a profile. */
  editProfile = (item: Profile) => { this.set({ profile: item, view: 'edit-profile' }); };

  detail = async (id: string) => {
    const ticket = ++this.revision; this.set({ loading: true, view: 'detail', liveUrl: '' });
    try { const result = await this.api.call<Session>(`/sessions/${segment(id)}`); if (ticket === this.revision) this.set({ session: result }); }
    finally { if (ticket === this.revision) this.set({ loading: false }); }
  };

  openLive = async (id: string, grant?: string) => {
    const ticket = ++this.revision; this.set({ loading: true, view: 'live', liveUrl: '' });
    try {
      if (!grant) {
        const link = await this.api.call<{ url: string }>(`/sessions/${segment(id)}/live`, 'POST');
        const url = new URL(link.url);
        if (url.origin !== location.origin) throw new Error('The live link did not match this website.');
        const invitation = readEntry(url).pending;
        if (!invitation || invitation.id !== id) throw new Error('The live link did not match this session.');
        grant = invitation.grant;
      }
      if (!grant) throw new Error('The live browser link is unavailable. Refresh the session and try again.');
      const result = await this.api.call<{ url: string }>(`/sessions/${segment(id)}/live/redeem`, 'POST', { grant });
      const url = safeHttps(result.url, location.origin); if (!url) throw new Error('The live browser link is invalid.');
      const current = await this.api.call<Session>(`/sessions/${segment(id)}`);
      if (ticket === this.revision) this.set({ session: current, liveUrl: url });
    } finally { if (ticket === this.revision) this.set({ loading: false }); }
  };

  setFilter = (filter: string) => { this.set({ filter }); };
  dismissNotice = () => { this.set({ notice: '' }); };

  // ---- form and list mutations (request bodies are byte-for-byte those of the former main.tsx) ----
  /** POST /sessions from fields `name`, `description`, `ttl`, then opens the new session. */
  createSession = async (data: FormData) => {
    const selected = this.state.profile;
    const created = await this.api.call<Session>('/sessions', 'POST', { name: String(data.get('name')).trim(), description: String(data.get('description')).trim(), ttl: data.get('ttl'), incognito: !selected, ...(selected ? { profile_id: selected.id } : {}) });
    await this.detail(created.id);
  };

  /** POST /sessions/{id}/end with field `note`, then reloads the session. */
  endSession = async (data: FormData) => {
    const session = this.state.session;
    if (!session) return;
    await this.api.call(`/sessions/${segment(session.id)}/end`, 'POST', { note: data.get('note') });
    this.toast({ type: 'success', title: 'Session ended', description: session.name });
    await this.detail(session.id);
  };

  /** PATCH /profiles/{id} (editing) or POST /profiles (new, with field `location`) from fields `name` and `access` (comma-separated UUIDs), then opens profiles. */
  saveProfile = async (data: FormData) => {
    const profile = this.state.profile;
    const body = { name: String(data.get('name')).trim(), access: splitAccess(String(data.get('access') || '')) };
    if (profile) await this.api.call(`/profiles/${segment(profile.id)}`, 'PATCH', body);
    else await this.api.call('/profiles', 'POST', { ...body, location: data.get('location') });
    this.toast({ type: 'success', title: 'Profile saved', description: body.name });
    await this.navigate('profiles');
  };

  /** POST /profiles/{id}/end with field `note`, then opens profiles. */
  retireProfile = async (data: FormData) => {
    const profile = this.state.profile;
    if (!profile) return;
    await this.api.call(`/profiles/${segment(profile.id)}/end`, 'POST', { note: data.get('note') });
    this.toast({ type: 'success', title: 'Profile retired', description: profile.name });
    await this.navigate('profiles');
  };

  /** POST /recordings/{session_id}/trash, then reloads recordings. */
  hideRecording = async (item: Recording) => {
    await this.api.call(`/recordings/${segment(item.session_id)}/trash`, 'POST');
    this.toast({ type: 'success', title: 'Recording hidden', description: item.session_name });
    await this.refreshRecordings();
  };

  // ---- lifecycle ----
  /** Mount: restore the saved workspace once, follow other tabs, and poll recordings in progress. Returns the cleanup. */
  start = () => {
    // React StrictMode mounts twice in development; the restore must run once.
    if (!this.restoreStarted) { this.restoreStarted = true; if (this.restored) void this.perform(this.enterWorkspace); }
    const synchronize = () => {
      const next = this.saved.load(), current = this.api.currentSession();
      this.loadContexts();
      // Another tab's token rotation does not change this workspace or its requests.
      if ((!next && !current) || (next && current && contextKey(next) === contextKey(current))) return;
      this.clearWorkspace(); this.api.setSession(next, false); this.set({ auth: next });
      if (next) void this.enterWorkspace().catch(error => this.set({ notice: publicError(error) }));
    };
    const storageChanged = (event: StorageEvent) => { if (event.key === this.saved.storageKey || event.key === null) synchronize(); };
    window.addEventListener('storage', storageChanged);
    window.addEventListener('focus', synchronize);
    const timer = window.setInterval(() => {
      const { auth, view, busy, loading, recordings } = this.state;
      if (auth && view === 'recordings' && !busy && !loading && recordings.some(item => ['recording', 'pending'].includes(item.status))) {
        void this.refreshRecordings(this.revision, true).catch(() => {});
      }
    }, 10000);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener('storage', storageChanged);
      window.removeEventListener('focus', synchronize);
    };
  };
}
