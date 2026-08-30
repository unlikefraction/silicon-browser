Silicon Browser:
We're building a managed auth & access layer on top of browser-use; a cli alias of agent-browser to run with `sb ...`. and a setup command that sets up everything.

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
it reads an env variable (SB_AUTHTOKEN) and passes that everytime to sb backend.

commands:
same grammar as si - `sb {service} {verb} [{target}] [{content}] [--flags]`, verb always second.
same seven verbs: ls, show, new, set, send, end, rm. nothing is deleted, only ended.
same `--filter "stage -> stage"`, each service listing its own is: and has: below.
`sb` on its own prints who you are, the org you are in, and the services you can reach.
`sb {service}` prints that service's verbs. `--help` on anything prints the long form.

`sb setup` the only command with no service. installs the browser, reads or prompts for SB_AUTHTOKEN, picks the org, and prints whatever is still missing. safe to run twice.


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
`sb session live {sessionid}` a link to watch or take over a session already running.
`sb session logs {sessionid} --date DD-MM-YYYY` every sb command that ran in it, in order. kept as `{sessionid}-{DD-MM-YYYY}`. defaults to today.
`sb session end {sessionid} --note "..."` stops and stores it. one that hits its ttl ends itself with
the note `ttl reached`.
is: active, ended, expired, incognito, mine | for: @{} | name:market* | description: ^research


[use session]

`sb run {sessionid} "{any agent-browser command}" [--flags]` the whole of agent-browser, one command. everything inside the quotes is passed through untouched, which is what makes sb a superset of it. `sb run --help` reveals agent-browser's own documentation, as needed. a session id is needed to use run. it informs when a `sb run` command is run and less than 1min is left to ttl.

for documentation, replace "agent-browser" with "sb run {sessionid}"

[recording]

a visual recording per session, written to the private briefcase of whoever started it (`private/{siliconid}/sb/{sessionid}`). if a silicon started it, the sb command log sits beside it.

`sb recording ls --filter "..."` prints session names & ids. contains: matches name and description.
`sb recording show {sessionid}` briefcase link, duration, size.
`sb recording rm {sessionid}` to the briefcase trash. 45 days, then gone.
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

`sb search ...` & `sb fetch ...`

expose all options available via the api over in the documentation.


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


give a similar overview for search & fetch as well.