we are making silicon-browser. it is a cli (agent-browser by vercel) with cloakbrowser as the engine to do things.

we will replace the cli (using alias) `agent-browser` to `silicon-browser` so we can use it as the name for all commands.

what i want from it at the end:
1. `silicon-browser` cli with things like --profile {profile_name} or --incognito or someway to start a profiled or incognito browser instance.
2. a way of replication of this project so multiple browser sessions can be run on a single server and served as a product. with something like docker, idk yet.
3. remote viewing & take-over for a browser session running.
4. fingerprinting for profiles and maintainance of it.
5. since the browser itself will remian the same accross different users, what makes a profile a profile are files, cookies, fingerprints, etc etc. we need a way to inject them in. this should be instant and browser start time should be near 0. idk how yet.