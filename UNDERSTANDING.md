Silicon Browser:
We're building a managed auth & access layer on top of browser-use; a cli that runs agent-browser underneath to run browser. it wraps it with other commands like `sb ...`. and a setup command that sets up everything.

Architecture clarification (2026-09-06): `sb` is a local usability wrapper. The Rust client obtains auth, profile/session metadata, the CDP capability and live links from our backend. The CLI runs the native controller locally and connects directly to the remote browser over CDP. Browser actions and their outputs never run on, or pass through, our backend. The frontend also connects its live iframe directly to the returned viewer. Completed command metadata is reported separately to our backend; stdout/stderr remain local. Reports are cooperative and cannot prove all direct browser activity.

The dedicated AWS daemon owns the auth/access and lifecycle control plane, usage metadata, command-log storage, and completed-recording delivery to Briefcase. Existing search/fetch requests still use its shared provider-key pool and fair queue as described below. It needs no local browser or controller runtime. The frontend is a minimal SolidJS/TypeScript app in IAM's visual style, hosted on Vercel. User-facing commands and screens use Silicon Browser branding; provider integrations remain internal.

Target scale: 500 simultaneous users, with browser action and live-view bandwidth going directly to the remote provider. Local credentials and remembered state are partitioned by normalized backend URL so simultaneous users and environments cannot borrow one another's auth. A direct CDP/live capability remains valid until its remote session ends; changing API authorization cannot retract an already-issued provider URL.

we'll use silicon iam to authenticate carbons & silicons. primary users of sb will be silicons and occasionally carbons to authenticate or pass captchas.

user flow:
login -> (after this, everything they see will be scoped to what they can see) choose org to enter
			-> create a browser profile + fix the proxy location
			-> give access to that profile (list of carbons, silicons or tags, union is the access).
			-> start a new session (with name + description) (useful for searching the recordings later) 
			-> on the UI, there will be a button to start a live session with that profile. (this can be used to authenticate into and save)
			-> store the session once it expired / ends.
			-> a recording tab where all the recordings in decending chronological order is kept. and filters of profile, name, description, or incognito, or even by silicons & carbons who ran that session.
			-> when a silicon shares a sb live session link and it is opened by a carbon, the carbon opening that page is also tagged as associated with that sesion.

api flow:
login -> (scoped access) -> list profiles available or use incognito (no profile, no proxy)
			-> choose profile to use with proxy (location can't be changed)
			-> get a live link to this profile to view/interact.
			-> create a new profile with this silicon as a default user (can add others or tags) + fix location
			-> start a new session with name + description + ttl
			-> list previous runs & get the recording + sb commands run in that session.

login:
carbons & silicons can login using `silicon-iam`. both the UI and CLI support auth.

recordings:
we should have a visual recording (stored in private briefcase of the silicon / carbon that initiated that session)
if a silicon started it, then also a replayable log of sb commands run in that session.
do a trusted handshake with briefcase to store files on behalf of silicon/carbon.

proxy:
for a given session, proxy can only be set during its initiation. after that it uses that location always.
proxies are only available for profiles.

usage costs:
each browser run + proxy usage is priced per minute & per GB. we store that per session that is run. this is for running analytics later and charging per usage.

profiles:
profiles have a unique fingerprint, a set proxy location and always uses proxy.

sb command:
it is a superset of agent-browser cli command. any agent-browser cli command is a valid sb command.
sb provides a reveal as needed documentation.
it reads an env variable (SB_AUTHTOKEN) for authenticated backend metadata requests. Browser commands use the authorized direct CDP connection locally; they do not send the IAM token to the browser provider.

commands:
same grammar as si - `sb {service} {verb} [{target}] [{content}] [--flags]`, verb always second.
same seven verbs: ls, show, new, set, send, end, rm. nothing is deleted, only ended.
same `--filter "stage -> stage"`, each service listing its own is: and has: below.
`sb` on its own prints who you are, the org you are in, and the services you can reach.
`sb {service}` prints that service's verbs. `--help` on anything prints the long form.

`sb setup` the only command with no service. installs the native local controller (no local Chromium), reads or prompts for SB_AUTHTOKEN, picks the org, and prints whatever is still missing. safe to run twice.


[profile]

setup a persistent browser identity: one fingerprint, one proxy location, access list. always on proxy. the location is fixed at creation and can never be changed - make a new profile instead.

`sb profile ls` prints name, id & location for all profiles.
`sb profile show {profileid}` name, fingerprint, location, access, owner, sessions run, created.
`sb profile new --name "..." --location "..." --access [@carbon,@ceo:tos,growth]` @carbon, @silicon, or tag (with no @) prints the id it made. whoever runs it is the default user; access and tags union on top of that. this creates a new browser profile, not a new session.
`sb profile set {profileid} --name "..."` / `--access [...]`
name and access are all that is editable. fingerprint and location are not.
`sb profile end {profileid} --note "..."` retires it. its recordings and usage stay. no one will be able to use this profile later.

`sb proxy ls` the locations a profile can be pinned to.


[session]

use browser. name and description exist so the recording is findable later.

`sb session new {profileid} --name "..." --description "..." --ttl 30m` start a new session and print the session id. ttl: 15m,30m,45m,60m,120m,240m. only one session can be run of a given profile. if another is asked to run, display who is running this session and when is the ttl ending.
`sb session new --incognito --name "..." --description "..."` no profile, no proxy. its still recorded and kept.

`sb session ls --filter "..."` prints ids.
`sb session show {sessionid}` profile, location, name, description, status, started, ttl left, cost so far.
`sb session live {sessionid}` a link to watch or take over a session already running. this link is on silicon browser itself. `browser.teamofsilicons.com/...`
`sb session logs {sessionid} --date DD-MM-YYYY` every sb command that ran in it, in order. kept as `{sessionid}-{DD-MM-YYYY}`. defaults to today.
`sb session end {sessionid} --note "..."` stops and stores it. one that hits its ttl ends itself with
the note `ttl reached`.
is: active, ended, expired, incognito, mine | for: @{} | name:market* | description: ^research


[use session]

`sb run {sessionid} "{any agent-browser command}" [--flags]` the whole of agent-browser, one command. everything inside the quotes is passed through untouched, which is what makes sb a superset of it. `sb run --help` reveals agent-browser's own documentation, as needed. a session id is needed to use run. it informs when a `sb run` command is run and less than 1min is left to ttl.

for documentation, replace "agent-browser" with "sb run {sessionid}"

[recording]

a visual recording per session, written to the private briefcase of whoever started it. the file stored directory is automatically handled by briefcase and not changeable. if a silicon started it, the sb command log sits beside it. this is done by connecting with briefcase over an OBO.

`sb recording ls --filter "..."` prints session names & ids. contains: matches name and description.
`sb recording show {sessionid}` briefcase link, duration, size.
`sb recording rm {sessionid}` hides the Browser recording and cancels pending delivery. Briefcase owns its directory and retention; OBO cannot delete files, so this does not delete the remote recording or promise a 45-day purge.
is: mine, shared


[usage]

browser minutes and proxy GB, stored per session, for analytics and for billing.

`sb usage show {sessionid}` minutes, GB in and out, and what each costs.
`sb usage ls --filter "between:01-08-2026=30-08-2026 -> for:@ceo:tos"` a line per session with its total.
`sb usage show --org` the org's total for that window.

[search & fetch]
this uses tiny fish's search & fetch to quickly get things instead of using a browser.
this is recommended for research when lots of pages will be read.
it has a rate limit of:
Search:30 requests / min
Fetch:150 urls / min
and both cost $0

tiny fish has their own cli as well. but we'll not use that. we'll route all the traffic via silicon browser backend. this will help us log & manage rate limits.

`sb search "{query}" --purpose "..." [--flags]`
finds the urls worth reading. returns ranked results, not pages.
  --purpose "..."                 what you are actually after, up to 2000 chars. results rank on it.
  --type web/news/research        defaults to web.
  --include-domains [...]         / --exclude-domains [...]
  --location {country} --language {code}
  --recency {minutes}             or --after DD-MM-YYYY / --before DD-MM-YYYY
  --pub-year-min / --pub-year-max research only
  --page 0-10

`sb fetch [url,url,...] --purpose "..." [--flags]`
reads them. 10 urls a call upstream, so we batch and queue anything larger rather than erroring.
  --purpose "..."                 what to keep from the page
  --format markdown/html/json     defaults to markdown
  --links / --image-links         include hrefs / img srcs. off by default.
  --ttl {seconds}                 cache freshness. 0 forces a live fetch.
  --timeout {ms}                  per url, up to 110000
  --include-selectors [...]       / --exclude-selectors [...] css, up to 20
a url that errors is not billed and does not count against the limit.

the limit is per api key, not per caller, and the api returns 429 past it. one key shared across the
org means one silicon running a sweep starves everybody else. so the backend holds a pool of keys and
queues per silicon: a burst waits its turn instead of failing. this is the main reason we route
through the backend at all, alongside logging.


[sb cli]
for cli, there will be 2 major paths:
1. Remote Browser: Interaction heavy work. for handling a real browser and doing things.
2. Search & Fetch: Read heavy work. also supports JS rendering, and is significantly faster & optimised for research and reading webpages. This is text only. Images are not supported here.

so, say the first command always run is `sb --help`
then it says prints the 2 things possible and what they are good for. then the silicon picks the branch to go into.
`sb --help {remote-browser/search-and-fetch}`
and then we show all the possible things they can do in each with the most common flow mentioned by default.

for remote-browser:
"""
`sb profile ls` <- to list all browser profiles available to use
`sb session new {profileid} --name "..." --description "..." --ttl 30m` OR `sb session new --incognito --name "..." --description "..." --ttl 15m` <- which will start a new browser session with the specified profile or incognito. it outputs the session id you can use.
then `sb run --help` to see how to use the session.
"""
^ this will make it such that the silicon will get information as needed. we'll bunch up sb run commands into categories as well so its easy to find information & doesn't fill the context.


for search-and-fetch:
"""
`sb search "{query}" --purpose "..."` <- find the urls worth reading. returns results, not pages.
`sb fetch [url,url,...] --purpose "..."` <- read them as text, in one call.
then `sb search --help` / `sb fetch --help` for filters, formats, dates and selectors.
"""
no profile, no session, no setup. this is the default branch for anything read-only. remote-browser
is for interaction.

note on grammar: `sb run`, `sb search` and `sb fetch` are top level verbs with no service in front of
them, because they are the three things a silicon actually does all day. everything else stays
`{service} {verb}`.



# codebase
structure it in modules. each part here becomes a module. nesting module is possible into submodules. define modules based on how i've seperated ideas here.
create a shared dir for shared code.
keep the code to a minimum. if it can be done in less, lets do it in less.
we are following a event/callback driven code style.
publish the libs.
follow a sync approach when its for simple tasks, event/callback driven > async for complex. async otherwise.
write test cases, mention what you're testing a test-group, and then at the end, give results.
all tools you need are installed natively and feel free to install any package.
aws cli for hosting.
the backend will be on `backend.browser.teamofsilicons.com` and frontend on `browser.teamofsilicons.com`
the backend runs as a native binary daemon on its own AWS host. the frontend is hosted separately on Vercel, not served by that backend. containerization is not required.
namecheap cli for handing DNS (https://namecheap-cli.vercel.app/)

# The Rust package & cli using that rust package are first hand client with an always running deamon if needed in the background. the UI will be a subset of the cli. make sure everything works via the CLI first, and then we'll make the UI. Everyone should be able to use the CLI/Rust Package (carbons, silicons, org, access keys, api keys, read, write, patch, delete, everything).

For how this CLI is built, rust as the programming language, but can use anything under the hood that is needed. Maybe rust, or node, or shell, as and when the work comes. That is decided by the implementor based on the work. If something requirs a UI (like graph, live, video, images etc). for that the UI has an endpoint that can be viewed/used/downloaded and the cli gives the link to that.

The primary Interface is the Rust Package. CLI is built using the Rust Package only and doesn't have any feature that the Rust package does not.

if you need a local store for auth or something else, use ~/.{appname}/ dir

Rust package itself is stateless. i.e, same code on any machine will give the same expected output.
it requires an auth object to be created and uses the auth object for all further queries. Say it doesn't authenticate everytime if locally an auth token is available. but that is a stateless optimization.

CLI will be stateful. i.e it remembers the last command run. auth is managed by the cli as a single logged in user. CLI is built on top of the stateless Rust package.

IAM Documentation (https://github.com/teamofsilicons/silicon-iam/tree/main/docs)
Never ask for credentials, always ask for short lived token.


# codebase thinking
- writing code is not just about implementation, maintainability & elegance matter as much.
- test and try things before you implement. try a simpler version to see how it works, what works what doesn't work. think in extremes.
- smaller code is reliable code. write less.
- writing once is not enough. its v0.0, iterate. make it smaller, faster, reliable, resilient, elegant, & largely maintainable.
- use pre installed libraries before you need to reach out for external onces. feel free to use them when you want.
- codebase is a form of art.
- use workflows well... not just for writing code, but thinking, evaluating, testing, researching, organizing, and critiquing yourself.
- run agents to get critiques on what you have done. what you have thought.
- don't implement more than this UNDERSTANDING.md asks you until its truely needed.
