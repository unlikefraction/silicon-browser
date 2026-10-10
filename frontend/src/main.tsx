import { render } from 'solid-js/web';
import { createSignal, For, Show, onMount, onCleanup, type JSX } from 'solid-js';
import '@fontsource/ibm-plex-sans/latin-400.css';
import '@fontsource/ibm-plex-sans/latin-500.css';
import '@fontsource/ibm-plex-sans/latin-600.css';
import '@fontsource/ibm-plex-mono/latin-400.css';
import './styles.css';
import brandMark from './assets/mark.svg';
import { BrowserApi, acceptAuth, publicError, safeHttps, segment, shellQuote, dateForApi } from './api';
import { readEntry, completeCallback, signInPopup, matchingRedirect, clearRedirectAttempt, type LoginAttempt } from './auth';
import { recordingStatusMessage } from './recordings';
import { BrowserSession, contextKey } from './session';
import type { AuthSession, Profile, Session, Recording, Usage, UsageLimits, Location, SessionLog } from './types';

const entry = readEntry(new URL(location.href));
// Remove one-use credentials and live grants before rendering, login, or API requests.
if (location.hash || location.search) history.replaceState(null, '', entry.cleanPath);
const savedSession = new BrowserSession(import.meta.env.SB_BACKEND_ORIGIN);
const productionApi = new BrowserApi(import.meta.env.SB_BACKEND_ORIGIN, undefined, (value, previous) => savedSession.save(value, previous), (session, exchange) => savedSession.refresh(session, exchange));
// The popup only hands off its one-use code; it must not restore the opener's session.
const restored = entry.callback ? null : savedSession.load();
if (restored) productionApi.setSession(restored, false);
const date = (value: string) => new Date(value).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
const cost = (value?: Usage) => value?.cost?.total ? `${(value.cost.total.micros / 1e6).toFixed(4)} ${value.cost.total.currency}` : 'Pending';
const bytes = (value: number) => value >= 1e9 ? `${(value / 1e9).toFixed(2)} GB` : `${(value / 1e6).toFixed(1)} MB`;
const splitAccess = (value: string) => value.split(',').map(item => item.trim()).filter(Boolean);
const tabs = ['sessions', 'profiles', 'recordings', 'usage', 'settings'] as const;
type Tab = typeof tabs[number];
type View = Tab | 'new-session' | 'new-profile' | 'edit-profile' | 'detail' | 'live' | 'logs';

function ExternalLink(props: { href?: string; children: JSX.Element }) {
  const href = () => safeHttps(props.href);
  return <Show when={href()}>{url => <a class="button" href={url()} target="_blank" rel="noopener noreferrer" referrerPolicy="no-referrer">{props.children} ↗</a>}</Show>;
}
function Badge(props: { state: string }) { return <span class={`badge ${props.state === 'active' || props.state === 'complete' ? 'positive' : ''}`}>{props.state.replaceAll('_', ' ')}</span>; }
function Empty(props: { children: JSX.Element }) { return <div class="empty">{props.children}</div>; }
function App() {
  const api = productionApi;
  const [auth, setAuth] = createSignal<AuthSession | null>(restored);
  const [showSilicon, setShowSilicon] = createSignal(false);
  const [contexts, setContexts] = createSignal<AuthSession[]>(savedSession.all());
  const [busy, setBusy] = createSignal(false);
  const [loading, setLoading] = createSignal(false);
  const [notice, setNotice] = createSignal(entry.error);
  const [signingIn, setSigningIn] = createSignal(false);
  const [addingContext, setAddingContext] = createSignal(false);
  let signInAbort: AbortController | null = null;
  const [view, setView] = createSignal<View>('sessions');
  const [profiles, setProfiles] = createSignal<Profile[]>([]);
  const [sessions, setSessions] = createSignal<Session[]>([]);
  const [recordings, setRecordings] = createSignal<Recording[]>([]);
  const [locations, setLocations] = createSignal<Location[]>([]);
  const [usage, setUsage] = createSignal<Usage[]>([]);
  const [total, setTotal] = createSignal<Usage>();
  const [limits, setLimits] = createSignal<UsageLimits>();
  const [session, setSession] = createSignal<Session>();
  const [profile, setProfile] = createSignal<Profile>();
  const [liveUrl, setLiveUrl] = createSignal('');
  const [logs, setLogs] = createSignal<SessionLog[]>([]);
  const [logDate, setLogDate] = createSignal(new Date().toISOString().slice(0, 10));
  const [filter, setFilter] = createSignal('');
  let revision = 0;
  let viewerFrame: HTMLIFrameElement | undefined;
  let pendingLive = entry.pending;
  let recordingRefreshes = 0;
  const activeTab = () => ['detail', 'live', 'logs', 'new-session'].includes(view()) ? 'sessions' : ['new-profile', 'edit-profile'].includes(view()) ? 'profiles' : view();
  async function perform(task: () => Promise<unknown>) {
    if (busy()) return;
    setBusy(true); setNotice('');
    try { await task(); } catch (error) {
      if (auth() && !api.currentSession()) { setAuth(null); clearWorkspace(); loadContexts(); }
      setNotice(publicError(error));
    } finally { setAuth(api.currentSession()); setBusy(false); }
  }
  async function navigate(next: Tab) {
    const ticket = ++revision;
    setView(next); setLoading(true); setFilter('');
    try {
      if (next === 'sessions') {
        const result = await api.call<Session[]>('/sessions');
        if (ticket === revision) setSessions(result);
      } else if (next === 'profiles') {
        const [items, places] = await Promise.all([api.call<Profile[]>('/profiles'), api.call<Location[]>('/proxy-locations')]);
        if (ticket === revision) { setProfiles(items); setLocations(places); }
      } else if (next === 'recordings') {
        await refreshRecordings(ticket);
      } else if (next === 'usage') {
        const [items, capacity] = await Promise.all([api.call<Usage[]>('/usage'), api.call<UsageLimits>('/usage/limits').catch(() => undefined)]);
        const sum: Usage = { sessions: items.length, browser_seconds: 0, proxy_bytes_in: 0, proxy_bytes_out: 0, proxy_bytes_unclassified: 0 };
        for (const item of items) {
          sum.browser_seconds += item.browser_seconds; sum.proxy_bytes_in += item.proxy_bytes_in; sum.proxy_bytes_out += item.proxy_bytes_out;
          sum.proxy_bytes_unclassified = (sum.proxy_bytes_unclassified || 0) + (item.proxy_bytes_unclassified || 0);
        }
        if (ticket === revision) { setUsage(items); setTotal(sum); setLimits(capacity); }
      }
    } finally { if (ticket === revision) setLoading(false); }
  }
  async function finishLogin(result: AuthSession) {
    api.setSession(result); setAuth(result); setAddingContext(false); setShowSilicon(false);
    const invitation = pendingLive; clearWorkspace(); pendingLive = invitation;
    await enterWorkspace();
  }
  async function login() {
    signInAbort = new AbortController(); setSigningIn(true);
    try {
      const { attempt_id, ...body } = await signInPopup(() => api.request<LoginAttempt>('/auth/attempts', 'POST', { identity_kind: 'carbon' }, null), signInAbort.signal);
      const result = acceptAuth(await api.request<AuthSession>(`/auth/attempts/${segment(attempt_id)}/exchange`, 'POST', body, null), 'browser', undefined, 'carbon');
      if (signInAbort.signal.aborted) throw new Error('Sign-in cancelled. Your workspace is unchanged.');
      await finishLogin(result);
    } finally { setSigningIn(false); signInAbort = null; }
  }
  async function siliconLogin(data: FormData) {
    const token = String(data.get('slt') || '').trim();
    if (!/^slt_[A-Za-z0-9_-]{1,16380}$/.test(token)) throw new Error('Enter the short-lived token from silicon-accounts login --app browser.');
    const result = acceptAuth(await api.request<AuthSession>('/auth/exchange', 'POST', { short_lived_token: token }, null), 'browser', undefined, 'silicon');
    await finishLogin(result);
  }
  function SignInOptions() {
    return <><div class="actions">{button('Continue as Carbon', login, true)}<button disabled={busy()} aria-expanded={showSilicon()} onClick={() => setShowSilicon(!showSilicon())}>Continue as Silicon</button></div>
      <Show when={showSilicon()}><form autocomplete="off" onSubmit={event => { event.preventDefault(); const data = new FormData(event.currentTarget); event.currentTarget.reset(); void perform(() => siliconLogin(data)); }}>
        <p>Get a short-lived token in your terminal:</p><pre>silicon-accounts login --app browser</pre>
        <label>Short-lived token<input name="slt" type="password" required maxlength={16384} placeholder="slt_…" autocomplete="off" spellcheck={false}/></label>
        <button class="primary" disabled={busy()}>Sign in as Silicon</button><p class="fine">A token works once and expires after two minutes.</p>
      </form></Show></>;
  }
  function loadContexts() {
    setContexts(savedSession.all());
  }
  async function enterWorkspace() {
    loadContexts();
    if (pendingLive) {
      const link = pendingLive;
      pendingLive = null; await openLive(link.id, link.grant);
    } else await navigate('sessions');
  }
  async function switchContext(next: AuthSession) {
    if (auth() && contextKey(auth()!) === contextKey(next)) return;
    const stored = savedSession.all().find(item => contextKey(item) === contextKey(next));
    if (!stored) throw new Error('This context has signed out. Please sign in again.');
    next = stored;
    clearWorkspace(); api.setSession(next); setAuth(next); loadContexts();
    await enterWorkspace();
  }
  async function attachAccounts() { setAddingContext(true); }
  function clearWorkspace() {
    ++revision; setLiveUrl(''); setSession(undefined); setProfile(undefined); pendingLive = null;
    setNotice(''); setLoading(false); setSessions([]); setProfiles([]); setRecordings([]); setUsage([]); setTotal(undefined); setLimits(undefined); setLogs([]);
    setLocations([]); setView('sessions'); setFilter('');
    history.replaceState(null, '', '/');
  }
  function logout() {
    const current = api.currentSession() ?? auth();
    if (current) savedSession.forget(current);
    api.setSession(null, false); setAuth(null); clearWorkspace(); loadContexts();
  }
  async function refreshRecordings(ticket = revision, background = false) {
    if (background && recordingRefreshes) return;
    recordingRefreshes++;
    try {
      const result = await api.call<Recording[]>('/recordings');
      if (ticket === revision && view() === 'recordings') setRecordings(result.sort((a, b) => Date.parse(b.created_at) - Date.parse(a.created_at)));
    } finally { recordingRefreshes--; }
  }
  onMount(() => {
    if (restored) void perform(enterWorkspace);
    const synchronize = () => {
      const next = savedSession.load(), current = api.currentSession();
      loadContexts();
      // Another tab's token rotation does not change this workspace or its requests.
      if ((!next && !current) || (next && current && contextKey(next) === contextKey(current))) return;
      clearWorkspace(); api.setSession(next, false); setAuth(next);
      if (next) void enterWorkspace().catch(error => setNotice(publicError(error)));
    };
    const storageChanged = (event: StorageEvent) => { if (event.key === savedSession.storageKey || event.key === null) synchronize(); };
    window.addEventListener('storage', storageChanged);
    window.addEventListener('focus', synchronize);
    const timer = window.setInterval(() => {
      if (auth() && view() === 'recordings' && !busy() && !loading() && recordings().some(item => ['recording', 'pending'].includes(item.status))) {
        void refreshRecordings(revision, true).catch(() => {});
      }
    }, 10000);
    onCleanup(() => {
      window.clearInterval(timer);
      window.removeEventListener('storage', storageChanged);
      window.removeEventListener('focus', synchronize);
    });
  });
  async function newSession(selected?: Profile) {
    setProfile(selected); setView('new-session'); ++revision;
  }
  async function detail(id: string) {
    const ticket = ++revision; setLoading(true); setView('detail'); setLiveUrl('');
    try { const result = await api.call<Session>(`/sessions/${segment(id)}`); if (ticket === revision) setSession(result); }
    finally { if (ticket === revision) setLoading(false); }
  }
  async function openLive(id: string, grant?: string) {
    const ticket = ++revision; setLoading(true); setView('live'); setLiveUrl('');
    try {
      if (!grant) {
        const link = await api.call<{url: string}>(`/sessions/${segment(id)}/live`, 'POST');
        const url = new URL(link.url);
        if (url.origin !== location.origin) throw new Error('The live link did not match this website.');
        const invitation = readEntry(url).pending;
        if (!invitation || invitation.id !== id) throw new Error('The live link did not match this session.');
        grant = invitation.grant;
      }
      if (!grant) throw new Error('The live browser link is unavailable. Refresh the session and try again.');
      const result = await api.call<{url: string}>(`/sessions/${segment(id)}/live/redeem`, 'POST', { grant });
      const url = safeHttps(result.url, location.origin); if (!url) throw new Error('The live browser link is invalid.');
      const current = await api.call<Session>(`/sessions/${segment(id)}`);
      if (ticket === revision) { setSession(current); setLiveUrl(url); }
    } finally { if (ticket === revision) setLoading(false); }
  }
  async function loadLogs() {
    const current = session(); if (!current) return;
    const result = await api.call<SessionLog[]>(`/sessions/${segment(current.id)}/logs?date=${dateForApi(logDate())}`);
    setLogs(result); setView('logs');
  }
  function submit(event: SubmitEvent, task: (data: FormData) => Promise<unknown>) {
    event.preventDefault(); const data = new FormData(event.currentTarget as HTMLFormElement); void perform(() => task(data));
  }
  function button(label: string, task: () => Promise<unknown>, primary = false) {
    return <button type="button" classList={{ primary }} disabled={busy()} onClick={() => void perform(task)}>{label}</button>;
  }
  const matchingSessions = () => sessions().filter(item => `${item.name} ${item.id} ${item.status}`.toLowerCase().includes(filter().toLowerCase()));
  const matchingProfiles = () => profiles().filter(item => `${item.name} ${item.id} ${item.location.name}`.toLowerCase().includes(filter().toLowerCase()));

  return <div class="shell">
    <aside class="rail">
      <a class="brand" href="/" aria-label="Browser home"><img src={brandMark} alt="" width="28" height="28"/><span class="brand-wordmark">Browser</span></a>
      <div class="rail-label">WORKSPACE</div>
      <Show when={auth() || contexts().length} fallback={<p class="rail-note">A shared browser workspace for Carbons and Silicons.</p>}>
        <div class="rail-label org-label">ACCOUNTS</div>
        <div class="org-list" aria-label="Signed-in accounts"><For each={contexts()}>{item => <button class="org-item" classList={{ selected: !!auth() && contextKey(auth()!) === contextKey(item) }} disabled={busy()} onClick={() => void perform(() => switchContext(item))}><span class="org-dot" aria-hidden="true">{item.identity.name.slice(0, 1).toUpperCase()}</span><span>{item.identity.name}<small class="subtext">{item.identity.kind}</small></span></button>}</For><button class="org-add" disabled={busy()} title="Sign in to another account" onClick={() => void perform(attachAccounts)}><span aria-hidden="true">+</span><span>Add account</span></button></div>
        <div class="rail-label workspace-label">WORKSPACE</div>
        <Show when={auth()}><nav aria-label="Workspace"><For each={tabs}>{tab => <button disabled={busy()} aria-current={activeTab() === tab ? 'page' : undefined} onClick={() => void perform(() => navigate(tab))}><span>{tab === 'settings' ? 'Settings' : tab[0].toUpperCase() + tab.slice(1)}</span><span aria-hidden="true">{activeTab() === tab ? '→' : ''}</span></button>}</For></nav></Show>
      </Show>
      <div class="rail-bottom"><a href="/docs/" target="_blank" rel="noopener noreferrer">CLI & documentation ↗</a><span class="mono">TEAM OF SILICONS</span></div>
    </aside>
    <div class="main-shell">
      <header class="topbar"><span class="breadcrumb">Browser <span>/</span> {auth() ? activeTab() : 'Welcome'}</span><div class="identity"><Show when={auth()}>{current => <><span>{current().identity.name}</span><button class="quiet" disabled={busy()} onClick={logout}>Sign out</button></>}</Show></div></header>
      <main classList={{ 'live-main': view() === 'live' && !!auth() }}>
        <Show when={notice()}><div role="status" class="notice">{notice()}</div></Show>
        <Show when={signingIn()}><div class="notice auth-wait" role="status"><span>Complete sign-in in the Silicon Accounts window.</span><button onClick={() => signInAbort?.abort()}>Cancel sign-in</button></div></Show>
        <Show when={addingContext()}><section class="panel form-panel"><h2>Add an account</h2><SignInOptions/><button disabled={busy()} onClick={() => setAddingContext(false)}>Cancel</button></section></Show>
        <Show when={auth()} fallback={<section class="welcome">
          <span class="eyebrow">YOUR BROWSER WORKSPACE</span><h1>A browser, ready<br/>when you are.</h1><p>Start a session. Keep your profiles. Work together across the web.</p>
          <section class="login-panel"><h2>Sign in to Browser</h2><p class="muted">Use your Silicon Accounts identity.</p><SignInOptions/><p class="fine">Carbon sign-in opens in a separate window, or continues in this tab when popups are blocked.</p></section>
          <Show when={pendingLive}><p class="hint">You have a live browser invitation. Sign in to continue.</p></Show>
          <div class="welcome-features"><div><span class="mono">01 / PROFILES</span><p>Keep a consistent identity across sessions.</p></div><div><span class="mono">02 / TOGETHER</span><p>Bring people and agents into the same browser.</p></div><div><span class="mono">03 / RECORDED</span><p>Return to your work when a session ends.</p></div></div>
        </section>}>
          <Show when={!loading()} fallback={<div class="loading" role="status">Loading workspace…</div>}>
          <Show when={view() === 'sessions'}>
            <div class="page-heading"><div><span class="eyebrow">WORKSPACE</span><h1>Sessions</h1><p class="muted">Your work across the web, in one place.</p></div>{button('New session', () => newSession(), true)}</div>
            <div class="toolbar"><label class="search"><span class="sr-only">Find a session</span><input type="search" placeholder="Find a session…" value={filter()} onInput={event => setFilter(event.currentTarget.value)}/></label>{button('Refresh', () => navigate('sessions'))}</div>
            <Show when={matchingSessions().length} fallback={<Empty><h2>No sessions yet</h2><p>Start with a fresh browser, or choose a saved profile.</p>{button('Start your first session', () => newSession(), true)}</Empty>}>
              <div class="table-wrap"><table><thead><tr><th>Session</th><th>Status</th><th>Started</th><th>Usage</th><th><span class="sr-only">Actions</span></th></tr></thead><tbody><For each={matchingSessions()}>{item => <tr><td><strong>{item.name}</strong><span class="subtext mono">{item.id}</span></td><td><Badge state={item.status}/></td><td>{date(item.started_at)}</td><td>{cost(item.usage)}</td><td>{button('Open', () => detail(item.id))}</td></tr>}</For></tbody></table></div>
            </Show>
          </Show>
          <Show when={view() === 'profiles'}>
            <div class="page-heading"><div><span class="eyebrow">PERSISTENT IDENTITIES</span><h1>Profiles</h1><p class="muted">A familiar browser, every time you return.</p></div><button class="primary" disabled={busy()} onClick={() => { setProfile(undefined); setView('new-profile'); }}>New profile</button></div>
            <div class="toolbar"><label class="search"><span class="sr-only">Find a profile</span><input type="search" placeholder="Find a profile…" value={filter()} onInput={event => setFilter(event.currentTarget.value)}/></label>{button('Refresh', () => navigate('profiles'))}</div>
            <Show when={matchingProfiles().length} fallback={<Empty><h2>No profiles yet</h2><p>Create a profile to keep your browser identity and location between sessions.</p></Empty>}>
              <div class="cards"><For each={matchingProfiles()}>{item => <article class="card"><div class="card-top"><span class="mono">{item.location.code.toUpperCase()}</span><Badge state={item.status}/></div><h2>{item.name}</h2><p class="muted">{item.location.name} · {item.sessions_run} sessions</p><dl><dt>Owner</dt><dd>{item.owner_id}</dd><dt>Access</dt><dd>{item.access.join(', ') || 'Owner only'}</dd></dl><div class="actions"><Show when={item.status === 'active'}>{button('Start session', () => newSession(item), true)}<button disabled={busy()} onClick={() => { setProfile(item); setView('edit-profile'); }}>Edit</button></Show></div></article>}</For></div>
            </Show>
          </Show>
          <Show when={view() === 'new-profile' || view() === 'edit-profile'}>
            <div class="page-heading"><div><h1>{view() === 'new-profile' ? 'New profile' : 'Edit profile'}</h1><p class="muted">Your profile’s location stays fixed after creation.</p></div>{button('Back to profiles', () => navigate('profiles'))}</div>
            <form class="panel form-panel" onSubmit={event => submit(event, async data => { const body = { name: String(data.get('name')).trim(), access: splitAccess(String(data.get('access') || '')) }; if (profile()) await api.call(`/profiles/${segment(profile()!.id)}`, 'PATCH', body); else await api.call('/profiles', 'POST', { ...body, location: data.get('location') }); await navigate('profiles'); })}>
              <label>Profile name<input name="name" required maxlength={100} value={profile()?.name || ''} placeholder="Research workspace"/></label>
              <Show when={!profile()} fallback={<p class="muted">Location: {profile()?.location.name}</p>}><label>Location<select name="location" required><For each={locations()}>{place => <option value={place.code}>{place.name}</option>}</For></select></label></Show>
              <label>Additional access<input name="access" value={profile()?.access.join(', ') || ''} placeholder="@Ab1, @Cd2"/><span class="fine">Separate account UUIDs with commas. Your account always keeps owner access.</span></label><button class="primary" disabled={busy() || (!profile() && !locations().length)}>Save profile</button>
            </form>
            <Show when={profile()}><form class="panel form-panel subtle" onSubmit={event => submit(event, async data => { await api.call(`/profiles/${segment(profile()!.id)}/end`, 'POST', { note: data.get('note') }); await navigate('profiles'); })}><h2>Retire profile</h2><p class="muted">Retired profiles cannot start new sessions.</p><label>Reason<textarea name="note" required maxlength={4000}/></label><button class="danger" disabled={busy()}>Retire profile</button></form></Show>
          </Show>
          <Show when={view() === 'new-session'}>
            <div class="page-heading"><div><h1>{profile() ? `Start ${profile()!.name}` : 'New session'}</h1><p class="muted">{profile() ? 'Continue with your saved browser identity.' : 'A fresh browser with no saved profile.'}</p></div>{button('Back to sessions', () => navigate('sessions'))}</div>
            <form class="panel form-panel" onSubmit={event => submit(event, async data => { const selected = profile(); const created = await api.call<Session>('/sessions', 'POST', { name: String(data.get('name')).trim(), description: String(data.get('description')).trim(), ttl: data.get('ttl'), incognito: !selected, ...(selected ? { profile_id: selected.id } : {}) }); await detail(created.id); })}>
              <label>Session name<input name="name" required maxlength={120} placeholder="What are you working on?"/></label><label>Description<textarea name="description" required maxlength={2000} placeholder="A little context for you and your collaborators."/></label><label>Session length<select name="ttl"><For each={[15, 30, 45, 60, 120, 240]}>{minutes => <option value={`${minutes}m`}>{minutes} minutes</option>}</For></select></label><p class="fine">Usage starts when the browser opens. The session ends automatically at its time limit.</p><button class="primary" disabled={busy()}>Start session</button>
            </form>
          </Show>
          <Show when={view() === 'detail' && session()}>{_current => <>
            <div class="page-heading"><div><span class="eyebrow">SESSION</span><h1>{session()!.name}</h1><p class="muted">{session()!.description}</p></div><Badge state={session()!.status}/></div>
            <div class="panel"><dl class="detail-grid"><dt>Session ID</dt><dd class="mono">{session()!.id}</dd><dt>Started</dt><dd>{date(session()!.started_at)}</dd><dt>Ends by</dt><dd>{date(session()!.expires_at)}</dd><dt>Usage</dt><dd>{cost(session()!.usage)}</dd><dt>Participants</dt><dd>{[session()!.initiator_id, ...(session()!.participant_ids || [])].filter((value, index, list) => list.indexOf(value) === index).join(', ')}</dd></dl><div class="actions"><Show when={session()!.status === 'active'}>{button('Open live browser', () => openLive(session()!.id), true)}</Show>{button('Refresh', () => detail(session()!.id))}{button('Command logs', loadLogs)}</div></div>
            <div class="panel subtle"><h2>Continue in your terminal</h2><p class="muted">Use the same session from the Browser CLI.</p><pre>browser run {session()!.id} snapshot</pre></div>
            <Show when={session()!.status === 'active'}><form class="panel form-panel" onSubmit={event => submit(event, async data => { await api.call(`/sessions/${segment(session()!.id)}/end`, 'POST', { note: data.get('note') }); await detail(session()!.id); })}><h2>End session</h2><label>Closing note<textarea name="note" required maxlength={4000} placeholder="What did you finish?"/></label><button class="danger" disabled={busy()}>End session</button></form></Show>
          </>}</Show>
          <Show when={view() === 'live'}><div class="page-heading"><div><h1>{session()?.name || 'Live browser'}</h1><p class="muted">Work in this browser together. The original session time limit still applies.</p></div><div class="actions"><Show when={document.fullscreenEnabled && liveUrl()}>{button('Fullscreen', async () => { if (viewerFrame) await viewerFrame.requestFullscreen(); })}</Show><Show when={session()}>{button('Session details', () => detail(session()!.id))}</Show></div></div><Show when={liveUrl()} fallback={<Empty>The live browser is unavailable. Refresh the session to check its status.</Empty>}><iframe ref={element => { viewerFrame = element; }} title="Live browser session" src={liveUrl()} referrerPolicy="no-referrer" sandbox="allow-scripts allow-same-origin allow-forms allow-popups" allow="clipboard-read; clipboard-write; fullscreen"/></Show></Show>
          <Show when={view() === 'logs'}><div class="page-heading"><div><h1>Command logs</h1><p class="muted">Commands reported by the CLI. Dates use UTC.</p></div>{button('Session details', () => detail(session()!.id))}</div><form class="toolbar" onSubmit={event => submit(event, loadLogs)}><label>Date<input type="date" required value={logDate()} onInput={event => setLogDate(event.currentTarget.value)}/></label><button disabled={busy()}>Load logs</button></form><pre>{logs().map(item => `browser run ${shellQuote(session()!.id)} ${shellQuote(item.command)}`).join('\n') || 'No commands for this date.'}</pre></Show>
          <Show when={view() === 'recordings'}>
            <div class="page-heading"><div><span class="eyebrow">YOUR ARCHIVE</span><h1>Recordings</h1><p class="muted">Revisit available session replays from your browser provider.</p></div>{button('Refresh', () => navigate('recordings'))}</div>
            <Show when={recordings().length} fallback={<Empty><h2>Nothing recorded yet</h2><p>Available session recordings appear here.</p></Empty>}><div class="cards"><For each={recordings()}>{item => {
              const statusMessage = () => recordingStatusMessage(item);
              return <article class="card">
                <Badge state={item.status}/>
                <h2>{item.session_name}</h2><p class="muted">{item.session_description}</p>
                <p class="fine">{date(item.created_at)} · {Math.round(item.duration_seconds / 60)} min · {item.size_bytes ? bytes(item.size_bytes) : 'Size unavailable'}</p>
                <Show when={statusMessage()}><p class="hint" role="status">{statusMessage()}</p></Show>
                <div class="actions">
                  <ExternalLink href={item.briefcase_link}>Open recording</ExternalLink><ExternalLink href={item.command_log_link}>Command log</ExternalLink>
                  {button('Hide', async () => { await api.call(`/recordings/${segment(item.session_id)}/trash`, 'POST'); await refreshRecordings(); })}
                </div>
                <p class="fine">Hiding removes this listing from Browser.</p>
              </article>;
            }}</For></div></Show>
          </Show>
          <Show when={view() === 'usage'}><div class="page-heading"><div><span class="eyebrow">WORKSPACE ACTIVITY</span><h1>Usage</h1><p class="muted">Browser time and network usage for sessions you can access.</p></div>{button('Refresh', () => navigate('usage'))}</div><Show when={total()}>{sum => <div class="stats"><div><span>Browser time</span><strong>{(sum().browser_seconds / 60).toFixed(1)} <small>min</small></strong></div><div><span>Proxy traffic</span><strong>{bytes(sum().proxy_bytes_in + sum().proxy_bytes_out + (sum().proxy_bytes_unclassified || 0))}</strong></div><div><span>Visible sessions</span><strong>{sum().sessions}</strong></div></div>}</Show><Show when={limits()} fallback={<p class="muted">Service capacity is temporarily unavailable.</p>}>{capacity => <p class="muted"><strong>{capacity().concurrent_browser_limit} concurrent browsers</strong> · Shared service limit · Checked {date(capacity().checked_at)}</p>}</Show><div class="table-wrap"><table><thead><tr><th>Session</th><th>Browser time</th><th>Usage</th></tr></thead><tbody><For each={usage()}>{item => <tr><td class="mono">{item.session_id}</td><td>{(item.browser_seconds / 60).toFixed(1)} min</td><td>{cost(item)}</td></tr>}</For></tbody></table><Show when={!usage().length}><Empty>No session usage yet.</Empty></Show></div></Show>
          <Show when={view() === 'settings'}><div class="page-heading"><div><h1>Settings</h1><p class="muted">Your Browser account.</p></div></div><section class="panel form-panel"><h2>Signed in</h2><dl><dt>Account</dt><dd>{auth()?.identity.name}</dd><dt>Account UUID</dt><dd class="mono">{auth()?.identity.id}</dd><dt>Kind</dt><dd>{auth()?.identity.kind}</dd></dl><p class="fine">You stay signed in across tabs and browser restarts. Sign out to remove this account from this browser.</p><ExternalLink href="https://accounts.teamofsilicons.com">Manage your account</ExternalLink></section></Show>
          </Show>
        </Show>
      </main><footer><span>Silicon Browser</span><span class="mono">BUILT FOR CARBONS & SILICONS</span></footer>
    </div>
  </div>;
}
function Callback() {
  const [message, setMessage] = createSignal(entry.error || 'Returning to Browser…');
  const complete = async () => {
    if (!entry.callback || entry.error) return;
    if (completeCallback(entry.callback)) { setMessage('Return to your Browser window to complete sign-in.'); return; }
    if (!matchingRedirect(entry.callback) || !entry.callback.code || !entry.callback.attempt) { setMessage('This sign-in link is missing its original attempt. Start sign-in again from Browser.'); return; }
    clearRedirectAttempt();
    try {
      const result = acceptAuth(await productionApi.request<AuthSession>(`/auth/attempts/${segment(entry.callback.attempt)}/exchange`, 'POST', { state: entry.callback.nonce, code: entry.callback.code }, null), 'browser', undefined, 'carbon');
      productionApi.setSession(result); clearRedirectAttempt(); location.replace('/');
    } catch (error) { setMessage(`${publicError(error)} Start sign-in again from Browser.`); }
  };
  onMount(() => { void complete(); });
  return <main class="callback"><div class="brand"><img src={brandMark} alt="" width="28" height="28"/><span class="brand-wordmark">Browser</span></div><h1>Sign in to Browser</h1><p>{message()}</p><a class="button" href="/">Open Browser</a></main>;
}
render(() => entry.callback ? <Callback/> : <App/>, document.getElementById('root')!);
