# Screens contract

How the workspace screens plug into the Browser shell. The shell, the workspace state, the shared helpers, and the Silicon UI
registry files are already built. Each screen agent replaces only the body of its own screen. Decisions in
`MIGRATION_SPEC.md` are final. The behaviour being ported is the former `src/main.tsx`; read it with `git show HEAD:frontend/src/main.tsx`.

## 1. File ownership

| Agent | Edits only these files |
|---|---|
| Sessions | `screens/Sessions.tsx`, `screens/Sessions.module.css`, `screens/NewSession.tsx`, `screens/NewSession.module.css`, `screens/SessionDetail.tsx`, `screens/SessionDetail.module.css` |
| Profiles | `screens/Profiles.tsx`, `screens/Profiles.module.css`, `screens/ProfileForm.tsx`, `screens/ProfileForm.module.css` |
| Recordings and usage | `screens/Recordings.tsx`, `screens/Recordings.module.css`, `screens/Usage.tsx`, `screens/Usage.module.css` |

All paths are under `frontend/src/browser/`. Do not edit anything else. That includes `workspace/*`, `ui/*`, `shell/*`,
`WorkspaceView.tsx`, `screens/Live.tsx`, `screens/Settings.tsx`, `src/lib/*`, `src/pages/*`, `src/components/silicon-ui/**`
(registry files are installed verbatim by `scripts/sync-silicon-ui.mjs`), `package.json`, and the lockfile.
If you need a shared change, report it in your final message instead of making it. Add no dependencies.

Keep each export's name and props exactly as they are. `WorkspaceView.tsx` renders them as follows:

| View (`state.view`) | Rendered element |
|---|---|
| `sessions` | `<Sessions />` |
| `new-session` | `<NewSession />` |
| `detail` | `<SessionDetail session={session} />` (only when `session` is set, as before) |
| `profiles` | `<Profiles />` |
| `new-profile` | `<ProfileForm />` (`profile` is undefined) |
| `edit-profile` | `<ProfileForm profile={profile} />` |
| `recordings` | `<Recordings />` |
| `usage` | `<Usage />` |

The shell already renders the rail, top bar, breadcrumb, notices (errors from `perform`), the loading
skeleton (a screen is never rendered while `loading` is true), the footer, and the toast viewport. A screen renders only its
own content, starting with a `PageHeader` (the page's only `<h1>`). Use `<h2>` for sections inside it. While "Add an account"
is open, `PageHeader` renders that panel right after itself (WorkspaceView provides it), so the `<h1>` stays the first heading.

## 2. The workspace hook

```ts
import { useWorkspace } from '../workspace/WorkspaceContext';
const { busy, sessions, action, submit, navigate, /* … */ } = useWorkspace();
```

`useWorkspace()` returns `WorkspaceState & WorkspaceActions & WorkspaceDerived`. It re-renders on every workspace change. State
is read synchronously inside actions, so single-flight checks, revision tickets, and late-response guards behave exactly as in
`main.tsx`. Screens never call the API directly; every request a screen needs is an action below.

### State (read-only)

| Field | Type | Meaning |
|---|---|---|
| `auth` | `AuthSession \| null` | Signed-in account. It is always set while a screen is shown. |
| `busy` | `boolean` | A `perform` task is running. **Every control a screen renders is disabled while busy.** |
| `loading` | `boolean` | A view is loading. The shell shows the skeleton instead of the screen. |
| `view` | `View` | The current view (see the table above). |
| `sessions` | `Session[]` | From `GET /sessions` (loaded by `navigate('sessions')`). |
| `profiles` | `Profile[]` | From `GET /profiles`. |
| `locations` | `Location[]` | From `GET /proxy-locations` (loaded with profiles; the new-profile form reuses them). |
| `recordings` | `Recording[]` | Newest first. They refresh in the background every 10 s while any item is `recording` or `pending`. |
| `usage` | `Usage[]` | Per-session usage rows. |
| `total` | `Usage \| undefined` | Client-side sum: `browser_seconds`, `proxy_bytes_in/out/unclassified`, `sessions` (= row count). |
| `limits` | `UsageLimits \| undefined` | Undefined when `/usage/limits` failed. |
| `session` | `Session \| undefined` | The session for detail and live. |
| `profile` | `Profile \| undefined` | The profile being edited, or the profile a new session starts from. |
| `filter` | `string` | Search text for sessions and profiles. Cleared on every navigation. |
| `contexts`, `notice`, `signingIn`, `addingContext`, `showSilicon`, `liveUrl`, `pendingInvitation` | | Shell-owned. Screens ignore them. |

### Derived

| Field | Meaning |
|---|---|
| `activeTab` | Rail tab for the view. |
| `matchingSessions` | `sessions` whose `` `${name} ${id} ${status}` `` contains `filter` (case-insensitive). Use this for the table. |
| `matchingProfiles` | `profiles` whose `` `${name} ${id} ${location.name}` `` contains `filter` (case-insensitive). Use this for the grid. |

### Actions

All actions are stable functions.

**Running tasks**
- `perform(task)` is single-flight. It is ignored while busy. It clears the notice, shows a failure as the notice, and signs out locally if the API dropped the session. Every API-calling action must run through it.
- `action(task)` returns a click handler `() => void perform(task)`. For example: `onClick={action(() => navigate('sessions'))}`.
- `submit(task)` returns a form submit handler. It calls `preventDefault`, reads the form's `FormData`, and passes it to `perform(() => task(data))`. For example: `<FormStack onSubmit={submit(createSession)}>`.

**Navigation.** Each of these returns a promise; run it through `perform`, `action`, or `TaskButton`.
- `navigate(tab)` opens and loads a tab: `'sessions' | 'profiles' | 'recordings' | 'usage' | 'settings'`. Use it for Refresh and Back buttons.
- `detail(id)` loads and opens a session.
- `openLive(id)` asks for a self-issued live link, checks it, redeems it, and opens the live view.
- `newSession(profile?)` opens the new-session form, optionally starting from a profile.
- `refreshRecordings()` reloads recordings (the polling already uses it).

**Synchronous view changes.** These are not tasks: call them directly from an `onClick`, with `disabled={busy}` on the control.
- `openNewProfile()`
- `editProfile(profile)`
- `setFilter(text)`

**Mutations.** Each takes the form's `FormData` and is passed to `submit(...)`. Request bodies are byte-for-byte those of `main.tsx`. On success they show a toast, so do not add your own.
- `createSession(data)` reads fields `name`, `description`, and `ttl`, and uses `profile` from state. It sends `POST /sessions {name, description, ttl, incognito: !profile, profile_id?}`, then calls `detail(created.id)`.
- `endSession(data)` reads field `note` and uses `session` from state. It sends `POST /sessions/{id}/end {note}`, then reloads the detail. Toast: "Session ended".
- `saveProfile(data)` reads fields `name`, `access`, and `location` (new profiles only). `access` is a comma-separated string of UUIDs, which the action parses with `splitAccess`. Editing sends `PATCH /profiles/{id} {name, access}`; a new profile sends `POST /profiles {name, access, location}`. Then it calls `navigate('profiles')`. Toast: "Profile saved".
- `retireProfile(data)` reads field `note`. It sends `POST /profiles/{id}/end {note}`, then calls `navigate('profiles')`. Toast: "Profile retired".
- `hideRecording(recording)` is a task, not a submit handler: `action(() => hideRecording(item))`. It sends `POST /recordings/{session_id}/trash`, then refreshes. Toast: "Recording hidden".

**Other**
- `toast({ type?, title, description? })` shows a transient confirmation. `type` is `'success' | 'info' | 'warning' | 'error'`. You rarely need it.

Shell-only actions: `login`, `cancelSignIn`, `siliconLogin`, `toggleSilicon`, `switchContext`, `addAccount`, `closeAddAccount`, `logout`, `dismissNotice`.

### Formatters and helpers (unchanged from `main.tsx`)

`import { date, cost, bytes, minutes, statusLabel, splitAccess } from '@/lib/format';`

| Helper | What it returns |
|---|---|
| `date(iso)` | `toLocaleString`, medium date and short time. |
| `cost(usage?)` | `"0.0123 USD"`, or `"Pending"`. |
| `bytes(n)` | `"1.23 GB"` at 1e9 and above, otherwise `"12.3 MB"`. |
| `minutes(seconds)` | `(seconds / 60).toFixed(1)`. |
| `statusLabel(s)` | `_` → space. |

`recordingStatusMessage(recording)` comes from `@/lib/recordings`. Types come from `@/lib/types`.

## 3. Shared helpers

Import from `'../ui'`.

| Helper | Use |
|---|---|
| `PageHeader({ eyebrow?, title, subtitle?, actions? })` | Every screen's heading. `actions` holds the header buttons or a status badge. |
| `Toolbar({ search?, children? })` | Row above a list. `search` grows (put the `SearchField` there); `children` are end-aligned actions. |
| `Panel({ title?, description?, tone?: 'default' \| 'subtle', narrow?, ...sectionProps })` | Bordered surface section with an optional `<h2>`. Use `narrow` for forms (about 36rem) and `tone="subtle"` for secondary or destructive sections. Adjacent panels are spaced automatically. |
| `FormStack(formProps)` | A `<form>` laid out as an evenly spaced column. Put it inside a `Panel`. |
| `ButtonRow(divProps)` | A row of buttons that wraps. |
| `FinePrint(pProps)` | Small secondary hint text (`--text-secondary`; keep `--text-muted` for decoration only, it is under 4.5:1 in light). |
| `TaskButton({ task, variant = 'secondary', ...ButtonProps })` | A `type="button"` Silicon Button that runs `task` through `perform` and is disabled while busy. It replaces `main.tsx`'s `button(label, task, primary)`: `<TaskButton task={() => navigate('sessions')}>Refresh</TaskButton>`, or `variant="primary"`. |
| `SubmitButton({ variant = 'primary', ...ButtonProps })` | A `type="submit"` Silicon Button, disabled while busy (and when you pass `disabled`). |
| `ListSection({ title, children })` | A region with a visually hidden `<h2>`; wrap a card grid, table, or empty state in it so headings stay in order (`h1` → `h2` → the `h3` from Card/EmptyState). |
| `ButtonLink(anchorProps & { variant?, size? })` | A same-site link styled as a button ("Open Browser"). |
| `ExternalButtonLink({ href, children, variant?, size? })` | An external link styled as a button. It renders only if `safeHttps(href)` passes, opens with `target="_blank" rel="noopener noreferrer" referrerPolicy="no-referrer"`, and appends ↗. **External links must only ever use this.** |
| `DefinitionList({ items: { term, value, mono?, copy? }[] })` | Term/value grid. `copy` adds an icon-only `CopyButton` labelled "Copy {term}". |
| `StatusBadge({ status, size = 'sm' })` | Silicon `Badge`: tone `success` for `active`/`complete`, otherwise `neutral`; label from `statusLabel`. Use it for every session, profile, and recording status. |

## 4. Silicon UI usage

Import from `@/components/silicon-ui/<name>/<name>`, for example `import { SearchField } from '@/components/silicon-ui/search-field/search-field';`.
Read the component source for full props. Points that are easy to get wrong:

- **Button** (`button/button`): the default variant is `primary`, and there is **no default `type`**. Always pass `type`. `loading` keeps the button focusable and swallows clicks. There is no `href` or `asChild`. Prefer `TaskButton` and `SubmitButton`.
- **SearchField**: controlled only. Use `value={filter}`, `onValueChange={setFilter}`, a required `label` (for example "Find a session"), and an optional `placeholder` ("Find a session…").
- **Input / Textarea**: `label` is required. They are uncontrolled, so pass native `name`, `required`, `maxLength`, `placeholder`, and `defaultValue` for prefill.
- **MorphSelect**: `name` writes a hidden input, so `FormData` works. It has **no `required` prop**. The old native selects always had their first option selected, so pass `defaultValue` (`'15m'` for ttl; `locations[0]?.code` for location) to keep that behaviour.
- **TagInput**: has **no `name`**. Keep the tags in state and add `<input type="hidden" name="access" value={tags.join(',')} />` to the form. `saveProfile` then parses it exactly as before. Prefill with `defaultValue={profile?.access ?? []}`.
- **SortableDataTable**: `rows` must be object-literal typed; pass `rows={matchingSessions.map(row => ({ ...row }))}` (interfaces such as `Session` do not satisfy `Record<string, unknown>`). Other props: `rowKey="id"`, a `caption`, and `columns[{ key, label, sortable?, numeric?, render?(value, row) }]`. Sorting is client-side, and the table stacks into cards at 620px and below.
- **Card**: `title` is required. Props include `description`, `meta`, `status` (polite, announced), `action`, and `children`. Do not pass `details`, because the quick-look dialog is not used here.
- **MetricCard**: `value` is a **number**, and `label` and `context` are required. Use `suffix` for units.
- **EmptyState**: `title` and `description` are both required. Optional props are `action` and `icon` (a lucide icon, `size={20} strokeWidth={1.75}`).
- **CodeBlock**: `code`, `filename`, `language`. **CopyButton**: `value`, `label`, `iconOnly`.
- Icons come from `lucide-react` (pinned 1.48.0). Mark decorative icons `aria-hidden="true"`.

## 5. Per-screen requirements

Eyebrows are written in sentence case; `PageHeader` shows them in capitals with CSS, so screen readers read words rather than letters. Every screen has one (New session: "Workspace"; profile forms: "Persistent identities"; Live: "Live session"; Settings: "Account").

Keep the copy below unless the spec says otherwise. Wording may be tightened, but keep the meaning, and leave error strings from `lib` unchanged.

**Sessions** (`Sessions.tsx`)
- `PageHeader` with eyebrow "Workspace", title "Sessions", and subtitle "Your work across the web, in one place.". Action: primary "New session" → `newSession()`.
- `Toolbar`: a `SearchField` labelled "Find a session" with placeholder "Find a session…", and a "Refresh" `TaskButton` → `navigate('sessions')`.
- `SortableDataTable` over `matchingSessions`. Columns:
  - Session: bold name plus the mono id.
  - Status: `StatusBadge`.
  - Started: `date(started_at)`.
  - Usage: `cost(usage)`.
  - Actions: a hidden "Actions" label and a "Open" `TaskButton` → `detail(id)`.
- When nothing matches, show `EmptyState` with title "No sessions yet", description "Start with a fresh browser, or choose a saved profile.", and action primary "Start your first session" → `newSession()`.

**New session** (`NewSession.tsx`)
- Title: `profile ? \`Start ${profile.name}\` : 'New session'`. Subtitle: `profile ? 'Continue with your saved browser identity.' : 'A fresh browser with no saved profile.'`. Action: "Back to sessions" → `navigate('sessions')`.
- Form (`submit(createSession)`):
  - `Input` name `name`, label "Session name", required, maxLength 120, placeholder "What are you working on?".
  - `Textarea` name `description`, label "Description", required, maxLength 2000, placeholder "A little context for you and your collaborators.".
  - `MorphSelect` name `ttl`, label "Session length", items 15/30/45/60/120/240 as `{ value: '15m', label: '15 minutes' }`, `defaultValue="15m"`, `searchable={false}`.
  - `FinePrint`: "Usage starts when the browser opens. The session ends automatically at its time limit."
  - Submit: "Start session".

**Session detail** (`SessionDetail.tsx`, prop `session`)
- Header: eyebrow "Session", title `session.name`, subtitle `session.description`, and a `StatusBadge` in the actions.
- `Panel` with a `DefinitionList`:
  - Session ID: mono, with `copy`.
  - Started: `date(started_at)`.
  - Ends by: `date(expires_at)`.
  - Usage: `cost(usage)`.
  - Participants: `[initiator_id, ...(participant_ids || [])]` de-duplicated and joined with ", ".
- Actions: primary "Open live browser" → `openLive(session.id)` (only when `status === 'active'`), and "Refresh" → `detail(session.id)`.
- `Panel tone="subtle"` titled "Continue in your terminal", description "Use the same session from the Browser CLI.", containing `<CodeBlock filename="Terminal" language="bash" code={\`browser run ${session.id} snapshot\`} />`.
- When active, a `Panel` titled "End session" with a form (`submit(endSession)`): `Textarea` name `note`, label "Closing note", required, maxLength 4000, placeholder "What did you finish?"; then a `SubmitButton variant="danger"` "End session".

**Profiles** (`Profiles.tsx`)
- `PageHeader` with eyebrow "Persistent identities", title "Profiles", and subtitle "A familiar browser, every time you return.". Action: primary "New profile", a plain `Button` with `disabled={busy}` and `onClick={openNewProfile}`.
- `Toolbar`: a `SearchField` labelled "Find a profile" with placeholder "Find a profile…", and "Refresh" → `navigate('profiles')`.
- A grid of `Card`s over `matchingProfiles`:
  - title `name`;
  - description `` `${location.name} · ${sessions_run} sessions` ``;
  - the location code uppercased;
  - a `StatusBadge`;
  - Owner `owner_id`, and Access `access.join(', ') || 'Owner only'`;
  - for active profiles only: primary "Start session" → `newSession(item)`, and "Edit" → `editProfile(item)` (direct, `disabled={busy}`).
- Empty: `EmptyState` with title "No profiles yet" and description "Create a profile to keep your browser identity and location between sessions.".

**New/Edit profile** (`ProfileForm.tsx`, prop `profile?`)
- Title: `profile ? 'Edit profile' : 'New profile'`. Subtitle: "Your profile’s location stays fixed after creation." Action: "Back to profiles" → `navigate('profiles')`.
- Form (`submit(saveProfile)`):
  - `Input` name `name`, label "Profile name", required, maxLength 100, `defaultValue={profile?.name ?? ''}`, placeholder "Research workspace".
  - New profiles: `MorphSelect` name `location`, label "Location", items from `locations` (`{ value: code, label: name }`), `searchable="auto"`, `defaultValue={locations[0]?.code}`.
  - Editing: read-only text "Location: {profile.location.name}".
  - `TagInput` labelled "Additional access" with placeholder "Account UUID", plus the hidden `access` input; reject empty tags.
  - `FinePrint`: "Your account always keeps owner access."
  - `SubmitButton` "Save profile", `disabled={!profile && !locations.length}`.
- When editing, a `Panel tone="subtle"` titled "Retire profile" with description "Retired profiles cannot start new sessions.". Its form (`submit(retireProfile)`) has a `Textarea` name `note`, label "Reason", required, maxLength 4000, and a `SubmitButton variant="danger"` "Retire profile".

**Recordings** (`Recordings.tsx`)
- `PageHeader` with eyebrow "Your archive", title "Recordings", and subtitle "Revisit available session replays from your browser provider.". Action: "Refresh" → `navigate('recordings')`.
- A `Card` per recording:
  - a `StatusBadge`;
  - title `session_name`, description `session_description`;
  - the line `` `${date(created_at)} · ${Math.round(duration_seconds / 60)} min · ${size_bytes ? bytes(size_bytes) : 'Size unavailable'}` ``;
  - `recordingStatusMessage(item)` when it is non-empty, in an element with `role="status"`;
  - actions: `<ExternalButtonLink href={item.briefcase_link}>Open recording</ExternalButtonLink>`, and "Hide" → `action(() => hideRecording(item))`;
  - `FinePrint`: "Hiding removes this listing from Browser."
- Empty: `EmptyState` with title "Nothing recorded yet" and description "Available session recordings appear here.".

**Usage** (`Usage.tsx`)
- `PageHeader` with eyebrow "Workspace activity", title "Usage", and subtitle "Browser time and network usage for sessions you can access.". Action: "Refresh" → `navigate('usage')`.
- When `total` is set, three `MetricCard`s, each with a short `context`:
  - Browser time: `Number(minutes(total.browser_seconds))`, suffix " min", one decimal.
  - Proxy traffic: `in + out + (unclassified || 0)`, shown as GB or MB the way `bytes()` chooses, as a number plus a suffix.
  - Visible sessions: `total.sessions`.
- The capacity line: `` `${limits.concurrent_browser_limit} concurrent browsers · Shared service limit · Checked ${date(limits.checked_at)}` ``, or "Service capacity is temporarily unavailable." when `limits` is undefined.
- A `SortableDataTable` with columns Session (mono `session_id`), Browser time (`` `${minutes(browser_seconds)} min` ``), and Usage (`cost(row)`). When there are no rows, show `EmptyState` "No session usage yet.".

## 6. Rules

- **Busy:** every button, link-button, and form control you render is disabled while `busy`; `TaskButton` and `SubmitButton` do this for you. Never block "Cancel sign-in" (it lives in the shell).
- **Field names, limits, validation, and request bodies** stay exactly as in section 5. The mutations already send the right bodies, so do not reshape `FormData`.
- **Never read the URL** (`location`, `router.query`, `asPath`). Never use `dangerouslySetInnerHTML`, inline `<script>`, or `<style>` elements. Prefer CSS modules over `style={}`.
- External links only through `ExternalButtonLink`. Every API path id already goes through `segment()` inside the actions.
- **Styling:** use CSS modules with foundation tokens only, such as `var(--space-group)`, `var(--text-secondary)`, `var(--border)`, `var(--radius-container)`, and `var(--font-display)`; no hard-coded colours. Both themes must work (`data-theme="dark"` is on `<html>`). The breakpoints are 1000px, 720px (the rail collapses into a drawer), and 620px (tables stack). There must be no horizontal page scroll at 390px.
- **Accessibility:** label every control, keep headings in order (`h1` in `PageHeader`, `h2` for sections, `h3` comes from `Card`), make icon-only buttons carry an `aria-label`, and keep focus visible (the foundation already handles this).

## 7. Verify before you finish

From `frontend/`:

```sh
npx --yes pnpm@10.33.0 build    # tsc + next build --webpack + publish-docs + check-export
npx --yes pnpm@10.33.0 test     # 49 unit and contract tests
SB_PUBLIC_BACKEND_URL=http://127.0.0.1:8091 npx --yes pnpm@10.33.0 build && npx --yes pnpm@10.33.0 e2e   # headless Chrome, production CSP, mocked backend
npx --yes pnpm@10.33.0 preview  # http://127.0.0.1:8092 (static out/ with production headers)
```

`scripts/e2e.mjs` mocks the backend through `context.route`. You can copy its scenario pattern to check your screen locally. Do not commit edits to `e2e.mjs` unless you were asked to. Rebuild without `SB_PUBLIC_BACKEND_URL` before handing off.
