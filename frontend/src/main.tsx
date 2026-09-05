import { render } from 'solid-js/web';
import { createSignal, For, Show, onMount, onCleanup, type JSX } from 'solid-js';
import '@fontsource/ibm-plex-sans/latin-400.css';
import '@fontsource/ibm-plex-sans/latin-500.css';
import '@fontsource/ibm-plex-sans/latin-600.css';
import '@fontsource/ibm-plex-mono/latin-400.css';
import './styles.css';
import brandMark from './assets/mark.svg';
import { BrowserApi, acceptAuth, publicError, safeHttps, segment, shellQuote, dateForApi } from './api';
import { readEntry, completeCallback, signInPopup } from './auth';
import { recordingRecovery } from './recordings';
import type { AuthSession, Profile, Session, Recording, Usage, UsageLimits, Location, Delivery, SessionLog } from './types';

const entry = readEntry(new URL(location.href));
// Remove one-use credentials and live grants before rendering, login, or API requests.
if (location.hash || location.search) history.replaceState(null, '', entry.cleanPath);
const api = new BrowserApi(import.meta.env.SB_BACKEND_ORIGIN);
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
  const [auth, setAuth] = createSignal<AuthSession | null>(null);
  const [org, setOrg] = createSignal('tos');
  const [busy, setBusy] = createSignal(false);
  const [loading, setLoading] = createSignal(false);
  const [notice, setNotice] = createSignal('');
  const [signingIn, setSigningIn] = createSignal(false);
  let signInAbort: AbortController | null = null;
  const [view, setView] = createSignal<View>('sessions');
  const [profiles, setProfiles] = createSignal<Profile[]>([]);
  const [sessions, setSessions] = createSignal<Session[]>([]);
  const [recordings, setRecordings] = createSignal<Recording[]>([]);
  const [locations, setLocations] = createSignal<Location[]>([]);
  const [usage, setUsage] = createSignal<Usage[]>([]);
  const [total, setTotal] = createSignal<Usage>();
  const [limits, setLimits] = createSignal<UsageLimits>();
  const [delivery, setDelivery] = createSignal<Delivery>();
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
  const ready = () => delivery()?.enabled && ['active', 'refreshing'].includes(delivery()?.state || '');
  async function perform(task: () => Promise<unknown>) {
    if (busy()) return;
    setBusy(true); setNotice('');
    try { await task(); } catch (error) { setNotice(publicError(error)); } finally { setBusy(false); }
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
        const [items, sum, capacity] = await Promise.all([api.call<Usage[]>('/usage'), api.call<Usage>('/usage/org'), api.call<UsageLimits>('/usage/limits').catch(() => undefined)]);
        if (ticket === revision) { setUsage(items); setTotal(sum); setLimits(capacity); }
      } else {
        const status = await api.call<Delivery>('/auth/delivery');
        if (ticket === revision) setDelivery(status);
      }
    } finally { if (ticket === revision) setLoading(false); }
  }
  async function tokenFor(org: string) {
    signInAbort = new AbortController(); setSigningIn(true);
    try { return await signInPopup(org, signInAbort.signal); } finally { setSigningIn(false); signInAbort = null; }
  }
  async function login() {
    const chosenOrg = org().trim();
    if (!chosenOrg) throw new Error('Enter your organization ID.');
    const token = await tokenFor(chosenOrg);
    const result = acceptAuth(await api.request<AuthSession>('/auth/exchange', 'POST', { short_lived_token: token, org_id: chosenOrg }, null), chosenOrg);
    api.setSession(result); setAuth(result);
    if (pendingLive) { const link = pendingLive; pendingLive = null; await openLive(link.id, link.grant); }
    else await navigate('sessions');
  }
  function logout() {
    api.setSession(null); setAuth(null); ++revision; setLiveUrl(''); setSession(undefined); pendingLive = null;
    setNotice(''); setLoading(false); setDelivery(undefined); setSessions([]); setProfiles([]); setRecordings([]); setUsage([]); setTotal(undefined); setLimits(undefined); setLogs([]);
    history.replaceState(null, '', '/');
  }
  async function authorize() {
    const current = auth(); if (!current) return;
    const token = await tokenFor(current.org.id);
    const result = await api.call<Delivery>('/auth/delivery', 'POST', { short_lived_token: token });
    setDelivery(result);
    setNotice('Recording access is ready. Your recordings will be saved after each session.');
  }
  async function refreshRecordings(ticket = revision, background = false) {
    if (background && recordingRefreshes) return;
    recordingRefreshes++;
    try {
      const result = await api.call<Recording[]>('/recordings');
      if (ticket === revision && view() === 'recordings') setRecordings(result.sort((a, b) => Date.parse(b.created_at) - Date.parse(a.created_at)));
    } finally { recordingRefreshes--; }
  }
  async function reconnectRecording() {
    await authorize();
    await refreshRecordings();
    setNotice('Recording access is ready. Pending recordings will resume delivery automatically.');
  }
  onMount(() => {
    const timer = window.setInterval(() => {
      if (auth() && view() === 'recordings' && !busy() && !loading() && recordings().some(item => ['recording', 'pending'].includes(item.status))) {
        void refreshRecordings(revision, true).catch(() => {});
      }
    }, 10000);
    onCleanup(() => window.clearInterval(timer));
  });
  async function newSession(selected?: Profile) {
    setProfile(selected); setView('new-session'); ++revision;
    setDelivery(await api.call<Delivery>('/auth/delivery'));
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
        grant = new URLSearchParams(url.hash.slice(1)).get('grant') || undefined;
      }
      if (!grant) throw new Error('The live browser link is unavailable. Refresh the session and try again.');
      const result = await api.call<{url: string}>(`/sessions/${segment(id)}/live/redeem`, 'POST', { grant });
      const url = safeHttps(result.url); if (!url) throw new Error('The live browser link is invalid.');
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
      <Show when={auth()} fallback={<p class="rail-note">A shared browser workspace for Carbons and Silicons.</p>}>
        <nav aria-label="Workspace"><For each={tabs}>{tab => <button disabled={busy()} aria-current={activeTab() === tab ? 'page' : undefined} onClick={() => void perform(() => navigate(tab))}><span>{tab === 'settings' ? 'Settings' : tab[0].toUpperCase() + tab.slice(1)}</span><span aria-hidden="true">{activeTab() === tab ? '→' : ''}</span></button>}</For></nav>
      </Show>
      <div class="rail-bottom"><a href="https://github.com/teamofsilicons/silicon-browser#readme" target="_blank" rel="noopener noreferrer">CLI & documentation ↗</a><span class="mono">TEAM OF SILICONS</span></div>
    </aside>
    <div class="main-shell">
      <header class="topbar"><span class="breadcrumb">Browser <span>/</span> {auth() ? activeTab() : 'Welcome'}</span><Show when={auth()}>{current => <div class="identity"><span>{current().identity.name} <small>{current().org.id}</small></span><button class="quiet" disabled={busy()} onClick={logout}>Sign out</button></div>}</Show></header>
      <main classList={{ 'live-main': view() === 'live' && !!auth() }}>
        <Show when={notice()}><div role="status" class="notice">{notice()}</div></Show>
        <Show when={signingIn()}><div class="notice auth-wait" role="status"><span>Complete sign-in in the IAM window.</span><button onClick={() => signInAbort?.abort()}>Cancel sign-in</button></div></Show>
        <Show when={auth()} fallback={<section class="welcome">
          <span class="eyebrow">YOUR BROWSER WORKSPACE</span><h1>A browser, ready<br/>when you are.</h1><p>Start a session. Keep your profiles. Work together across the web.</p>
          <form class="login-panel" onSubmit={event => submit(event, login)}><h2>Sign in to Browser</h2><p class="muted">Use your Silicon IAM identity to continue.</p><label>Organization ID<input name="org" autocomplete="organization" required value={org()} onInput={event => setOrg(event.currentTarget.value)} placeholder="your-organization"/></label><button class="primary wide" disabled={busy()}>{busy() ? 'Waiting for sign-in…' : 'Continue with IAM'} <span aria-hidden="true">↗</span></button><p class="fine">Sign-in opens in a separate window. Your session stays in this tab.</p></form>
          <Show when={pendingLive}><p class="hint">You have a live browser invitation. Sign in to its organization to continue.</p></Show>
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
              <label>Additional access<input name="access" value={profile()?.access.join(', ') || ''} placeholder="@person, @agent:organization, team-tag"/><span class="fine">Separate identities and team tags with commas. Your identity always keeps owner access.</span></label><button class="primary" disabled={busy() || (!profile() && !locations().length)}>Save profile</button>
            </form>
            <Show when={profile()}><form class="panel form-panel subtle" onSubmit={event => submit(event, async data => { await api.call(`/profiles/${segment(profile()!.id)}/end`, 'POST', { note: data.get('note') }); await navigate('profiles'); })}><h2>Retire profile</h2><p class="muted">Retired profiles cannot start new sessions.</p><label>Reason<textarea name="note" required maxlength={4000}/></label><button class="danger" disabled={busy()}>Retire profile</button></form></Show>
          </Show>
          <Show when={view() === 'new-session'}>
            <div class="page-heading"><div><h1>{profile() ? `Start ${profile()!.name}` : 'New session'}</h1><p class="muted">{profile() ? 'Continue with your saved browser identity.' : 'A fresh browser with no saved profile.'}</p></div>{button('Back to sessions', () => navigate('sessions'))}</div>
            <Show when={!ready()}><section class="panel form-panel"><h2>Save your recordings</h2><p>Allow Browser to save recordings in your Briefcase after the session ends. This separate authorization keeps delivery working when you close this tab.</p><Show when={delivery()?.configured} fallback={<p role="status">Recording access is being configured. Please try again shortly.</p>}>{button('Enable recording access', authorize, true)}</Show></section></Show>
            <form class="panel form-panel" onSubmit={event => submit(event, async data => { const selected = profile(); const created = await api.call<Session>('/sessions', 'POST', { name: String(data.get('name')).trim(), description: String(data.get('description')).trim(), ttl: data.get('ttl'), incognito: !selected, ...(selected ? { profile_id: selected.id } : {}) }); await detail(created.id); })}>
              <label>Session name<input name="name" required maxlength={120} placeholder="What are you working on?"/></label><label>Description<textarea name="description" required maxlength={2000} placeholder="A little context for you and your collaborators."/></label><label>Session length<select name="ttl"><For each={[15, 30, 45, 60, 120, 240]}>{minutes => <option value={`${minutes}m`}>{minutes} minutes</option>}</For></select></label><p class="fine">Usage starts when the browser opens. The session ends automatically at its time limit.</p><button class="primary" disabled={busy() || !ready()}>Start session</button>
            </form>
          </Show>
          <Show when={view() === 'detail' && session()}>{_current => <>
            <div class="page-heading"><div><span class="eyebrow">SESSION</span><h1>{session()!.name}</h1><p class="muted">{session()!.description}</p></div><Badge state={session()!.status}/></div>
            <div class="panel"><dl class="detail-grid"><dt>Session ID</dt><dd class="mono">{session()!.id}</dd><dt>Started</dt><dd>{date(session()!.started_at)}</dd><dt>Ends by</dt><dd>{date(session()!.expires_at)}</dd><dt>Usage</dt><dd>{cost(session()!.usage)}</dd><dt>Participants</dt><dd>{[session()!.initiator_id, ...(session()!.participant_ids || [])].filter((value, index, list) => list.indexOf(value) === index).join(', ')}</dd></dl><div class="actions"><Show when={session()!.status === 'active'}>{button('Open live browser', () => openLive(session()!.id), true)}</Show>{button('Refresh', () => detail(session()!.id))}{button('Command logs', loadLogs)}</div></div>
            <div class="panel subtle"><h2>Continue in your terminal</h2><p class="muted">Use the same session from the Browser CLI.</p><pre>sb run {session()!.id} snapshot</pre></div>
            <Show when={session()!.status === 'active'}><form class="panel form-panel" onSubmit={event => submit(event, async data => { await api.call(`/sessions/${segment(session()!.id)}/end`, 'POST', { note: data.get('note') }); await detail(session()!.id); })}><h2>End session</h2><label>Closing note<textarea name="note" required maxlength={4000} placeholder="What did you finish?"/></label><button class="danger" disabled={busy()}>End session</button></form></Show>
          </>}</Show>
          <Show when={view() === 'live'}><div class="page-heading"><div><h1>{session()?.name || 'Live browser'}</h1><p class="muted">Work in this browser together. The original session time limit still applies.</p></div><div class="actions"><Show when={document.fullscreenEnabled && liveUrl()}>{button('Fullscreen', async () => { if (viewerFrame) await viewerFrame.requestFullscreen(); })}</Show><Show when={session()}>{button('Session details', () => detail(session()!.id))}</Show></div></div><Show when={liveUrl()} fallback={<Empty>The live browser is unavailable. Refresh the session to check its status.</Empty>}><iframe ref={element => { viewerFrame = element; }} title="Live browser session" src={liveUrl()} referrerPolicy="no-referrer" sandbox="allow-scripts allow-same-origin allow-forms allow-popups" allow="clipboard-read; clipboard-write; fullscreen"/></Show></Show>
          <Show when={view() === 'logs'}><div class="page-heading"><div><h1>Command logs</h1><p class="muted">Commands reported by the CLI. Dates use UTC.</p></div>{button('Session details', () => detail(session()!.id))}</div><form class="toolbar" onSubmit={event => submit(event, loadLogs)}><label>Date<input type="date" required value={logDate()} onInput={event => setLogDate(event.currentTarget.value)}/></label><button disabled={busy()}>Load logs</button></form><pre>{logs().map(item => `sb run ${shellQuote(session()!.id)} ${shellQuote(item.command)}`).join('\n') || 'No commands for this date.'}</pre></Show>
          <Show when={view() === 'recordings'}>
            <div class="page-heading"><div><span class="eyebrow">YOUR ARCHIVE</span><h1>Recordings</h1><p class="muted">Revisit your sessions. Saved privately in Briefcase.</p></div>{button('Refresh', () => navigate('recordings'))}</div>
            <Show when={recordings().length} fallback={<Empty><h2>Nothing recorded yet</h2><p>Your recordings appear here after a session ends.</p></Empty>}><div class="cards"><For each={recordings()}>{item => {
              const recovery = () => recordingRecovery(item, auth()?.identity.id);
              return <article class="card">
                <Badge state={recovery().needsAuthorization ? 'access_needed' : item.status}/>
                <h2>{item.session_name}</h2><p class="muted">{item.session_description}</p>
                <p class="fine">{date(item.created_at)} · {Math.round(item.duration_seconds / 60)} min · {item.size_bytes ? bytes(item.size_bytes) : 'Size available after delivery'}</p>
                <Show when={recovery().message}><p class="hint" role="status">{recovery().message}</p></Show>
                <div class="actions">
                  <ExternalLink href={item.briefcase_link}>Open recording</ExternalLink><ExternalLink href={item.command_log_link}>Command log</ExternalLink>
                  <Show when={recovery().canReconnect}>{button('Reconnect recording access', reconnectRecording, true)}</Show>
                  <Show when={recovery().canRetry}>{button('Retry delivery', async () => { await api.call(`/recordings/${segment(item.session_id)}/retry`, 'POST'); await refreshRecordings(); })}</Show>
                  {button('Hide', async () => { await api.call(`/recordings/${segment(item.session_id)}/trash`, 'POST'); await refreshRecordings(); })}
                </div>
                <p class="fine">Hiding removes this listing. Any files already saved in Briefcase remain available.</p>
              </article>;
            }}</For></div></Show>
          </Show>
          <Show when={view() === 'usage'}><div class="page-heading"><div><span class="eyebrow">WORKSPACE ACTIVITY</span><h1>Usage</h1><p class="muted">Browser time and network usage across your organization.</p></div>{button('Refresh', () => navigate('usage'))}</div><Show when={total()}>{sum => <div class="stats"><div><span>Browser time</span><strong>{(sum().browser_seconds / 60).toFixed(1)} <small>min</small></strong></div><div><span>Proxy traffic</span><strong>{bytes(sum().proxy_bytes_in + sum().proxy_bytes_out + (sum().proxy_bytes_unclassified || 0))}</strong></div><div><span>Organization total</span><strong>{cost(sum())}</strong></div></div>}</Show><Show when={limits()} fallback={<p class="muted">Service capacity is temporarily unavailable.</p>}>{capacity => <p class="muted"><strong>{capacity().concurrent_browser_limit} concurrent browsers</strong> · Shared service limit · Checked {date(capacity().checked_at)}</p>}</Show><div class="table-wrap"><table><thead><tr><th>Session</th><th>Browser time</th><th>Usage</th></tr></thead><tbody><For each={usage()}>{item => <tr><td class="mono">{item.session_id}</td><td>{(item.browser_seconds / 60).toFixed(1)} min</td><td>{cost(item)}</td></tr>}</For></tbody></table><Show when={!usage().length}><Empty>No session usage yet.</Empty></Show></div></Show>
          <Show when={view() === 'settings'}><div class="page-heading"><div><h1>Settings</h1><p class="muted">Your identity and recording access.</p></div></div><section class="panel form-panel"><h2>Signed in</h2><dl><dt>Identity</dt><dd>{auth()?.identity.name}</dd><dt>Organization</dt><dd>{auth()?.org.id}</dd></dl><p class="fine">Sign-in is kept in this tab’s memory. Reloading requires signing in again.</p></section><section class="panel form-panel"><div class="card-top"><h2>Recording access</h2><Show when={delivery()}><Badge state={delivery()!.state}/></Show></div><p>Browser saves recordings to your private Briefcase after sessions end, even when this tab is closed.</p><div class="actions"><Show when={ready()} fallback={button('Enable recording access', authorize, true)}>{button('Disable recording access', async () => { await api.call('/auth/delivery/end', 'POST'); await navigate('settings'); })}</Show>{button('Refresh status', () => navigate('settings'))}</div></section></Show>
          </Show>
        </Show>
      </main><footer><span>Silicon Browser</span><span class="mono">BUILT FOR CARBONS & SILICONS</span></footer>
    </div>
  </div>;
}
function Callback() {
  const [sent, setSent] = createSignal(false);
  onMount(() => { if (entry.callback) setSent(completeCallback(entry.callback)); });
  return <main class="callback"><div class="brand"><img src={brandMark} alt="" width="28" height="28"/><span class="brand-wordmark">Browser</span></div><h1>{sent() ? 'Returning to Browser…' : 'Return to Browser to sign in.'}</h1><p>{sent() ? 'You can close this window and return to your workspace.' : 'This sign-in link is missing its original window. Start sign-in again from Browser.'}</p><a class="button" href="/">Open Browser</a></main>;
}
render(() => entry.callback ? <Callback/> : <App/>, document.getElementById('root')!);
