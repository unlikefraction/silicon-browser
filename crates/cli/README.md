# sb

`sb` is the stateful command-line shell over the stateless `silicon-browser` Rust package.
Run `sb --help` to choose between a managed remote browser and fast search/fetch.


`sb setup --org <id>` authenticates, checks recording delivery, and installs the native local controller when needed. It uses a private installation under `$SB_HOME/bin`, verifies the pinned release SHA-256, and reuses an already correctly pinned PATH installation. No Node/npm or local Chromium installation is required.

```sh
sb session new --incognito --name "Research" --description "Read the product page" --ttl 15m
sb run SESSION "open https://example.com"
sb run SESSION "snapshot -i"
sb run SESSION "screenshot ./capture.png"
sb session live SESSION
sb session end SESSION --note "Finished"
```

Browser actions run locally and connect directly to the managed remote browser. Standard output, screenshots, downloads, and uploads stay on the user's direct path. Only completed command text, flags, timestamps, and outcome metadata are reported to Browser. A report failure leaves a private durable queue and does not change a successful action's exit status. `sb session sync SESSION` retries logs without repeating actions. Ending a session first attempts to flush queued logs; once the immutable command archive starts, new late reports can no longer be added.

Credentials and refresh locks are partitioned by the normalized backend URL under `$SB_HOME/backends`. `--backend` / `SB_BACKEND_URL` chooses the issuer before reading credentials or refreshing. A matching legacy state migrates once; tokens from a different backend are never reused. Connection credentials are cached for at most 60 seconds, bounded by session expiry, and separated by organization and local login generation. Repeated local commands reuse a daemon isolated by backend, organization, immutable principal, and session.

`SB_CONTROLLER_BIN` explicitly selects an existing controller binary for development. `sb run --help` provides version-matched command help. Connection overrides and managed-session lifecycle commands are handled by `sb`; local file operations keep their normal native behavior.
