# Browser session retention release — September 22, 2026

Managed CLI 0.2.5 is published on GitHub and Honeycomb from
`0680f1df13fcccd14d28256ab5d9c6ddd85ac91e`.
[Native release CI](https://github.com/unlikefraction/silicon-browser/actions/runs/35662264299)
passed all six platforms and package validation. The release includes the saved
refresh receipt and status recovery fixes in `5d0c539`.

[GitHub managed-v0.2.5](https://github.com/unlikefraction/silicon-browser/releases/tag/managed-v0.2.5)
contains the standalone Unix archives, six-platform Honeycomb archive and checksums.
All ten public assets were downloaded anonymously and checked against the release
artifacts. Honeycomb accepted release `49ea56c3-104d-4079-ab65-ad12619fcf61`, archive
SHA-256 `ec6322d02f220ba730e54fe0f2749a44f458fbdd3485c98f59e08e63dbf4d89e`.
A fresh anonymous catalog install executed the macOS ARM binary and reported 0.2.5;
its hash is `72fbdab4fb4ba64b79a25df2f2a06dca6ecb8ab19e7be117b94571a600ca8b33`.

Maharaj's managed installation updated from 0.2.4 to 0.2.5 through Honeycomb.
The existing login remained authenticated and `browser --json profile ls` read
its profile list successfully, without a new login. The owned-token transport
already handles early access rejection with one exact request retry, a locked
refresh and generation/identity/context checks; login status uses that transport.
External access-token overrides remain caller-owned.

No backend or frontend source changes were required for this release. Existing
frontend persistence remains scoped to the browser tab; reload survives, closing
the tab does not retain that tab's session.

Crates.io publication remains blocked: the configured Cargo account is not an
owner of the Browser packages (403 on `silicon-browser-shared`). No credentials
or registry ownership were changed. README installation guidance identifies the
published GitHub/Honeycomb release and a pinned source install instead. The public
catalog's preexisting desired configuration review remains pending; the accepted
0.2.5 package is publicly installable without activating that separate review.
