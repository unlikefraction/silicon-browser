# Silicon Browser CLI

The `browser` command manages remote browser profiles and sessions and runs
browser actions locally. It is distributed by Silicon Apps and signs in with
Silicon Accounts.

```sh
silicon-apps install browser
browser --help
browser accounts --json
browser login status --json
silicon-accounts login --app browser
browser login --slt TOKEN
browser setup
```

From this checkout use `cargo install --path crates/cli`. Carbons sign in through
the hosted Accounts flow on the Browser website. Browser receives no password
or Silicon STK. Private state is partitioned by backend under
`${SILICON_HOME:-$HOME}/.silicon-browser`; `SB_HOME` overrides that root.
Login survives command invocations and restarts. `browser logout` clears the
saved local login. There is no organization selector; ownership and access use
canonical Accounts UUIDv4s.

Store packages bundle the pinned native controller. Setup verifies it and your
Accounts session. The CLI streams actions locally and queues command reports;
a failed report never repeats a browser action. Accounts refresh is one use:
an uncertain response needs fresh sign-in.

See the [CLI reference](../../docs/CLI.md), [Apps packaging](../../docs/APPS.md),
and [deployment](../../docs/DEPLOYMENT.md).
