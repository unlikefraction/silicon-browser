# Browser 1.1.0 — Silicon Accounts and Silicon Apps

Date: October 10, 2026.

## Live application

- Website: https://browser.teamofsilicons.com
- API: https://backend.browser.teamofsilicons.com
- Accounts app: `browser`, owned by Silicon `si:tos`.
- Initial production frontend deployment: `dpl_GTzNGdZ6VC1WPNqhzDaJXRmtdB7P`.
- Automatic main deployment verified: `dpl_HRdfzhthcjUjeprxmWucQiYJuAZY` at source `f6552bdd6455c72e12c0a92b4e58a2e078b839c0`.
- Backend release: `20261010-09ae21e79194bbf5`.
- Backend SHA-256: `202569f100c4e39c225a29d0ddf8ec820b663a4ef947c23c67e3d733943f54be`.

Accounts provides hosted Carbon PKCE sign-in and Silicon application-bound SLTs.
The callback is `https://browser.teamofsilicons.com/auth/callback`; the allowed
frontend origin is `https://browser.teamofsilicons.com`. The Accounts webhook is
`https://backend.browser.teamofsilicons.com/webhooks/accounts`. Application and
webhook secrets are held in private runtime configuration.

## Build evidence

Native binaries were built from source revision
`24e8337e30b8acbe51046ce0364a5237a29804dc`.

- [CI](https://github.com/unlikefraction/silicon-browser/actions/runs/38034874461): passed Rust tests, formatting, Clippy, rustdoc, crate packaging, frontend tests/build, installer and backup contracts.
- [Native Apps packages](https://github.com/unlikefraction/silicon-browser/actions/runs/38034874484): all six Linux, macOS and Windows architecture jobs plus package assembly passed.
- [Production backend](https://github.com/unlikefraction/silicon-browser/actions/runs/38034903005): ARM64 build for glibc 2.34 passed.
- Official Apps archive validation passed without errors.
- Apps archive SHA-256: `d792a6264374bd99f185a3ce8454766f1037812f26361e8809a18af8ab2875ca`.

The release commit also contains the prepared npm binary copies and this report;
those do not change the compiled Rust sources above.

## Production verification

Verified live Silicon SLT exchange as `si:tos`, permanent UUID ownership,
authenticated account/services/usage limits, and rotating refresh. A native
1.1.0 CLI created an incognito provider session, opened `https://example.com`,
read its title, reported both commands, and ended the session. The smoke
credentials were revoked afterward; access was rejected and Accounts delivered
the signed sign-out webhook.

Hosted Carbon sign-in also completed through the Accounts popup, consent and
PKCE callback into the live Browser workspace. The Carbon workspace did not
inherit the Silicon-owned smoke session.

The frontend production build and canonical aliases were verified, including
callback/live rewrites, documentation, CSP and cache headers. CORS grants the
canonical frontend origin. No nonterminal browser session remained after the
provider smoke.

## Retention and backup

The former resource database remains separate and unmodified, with its original
migrations 1–13. Its integrity, frozen snapshot checksum, separate restore copy,
old release, runtime configuration and encryption key were verified before the
cutover. The previous secret version remains retained. A root-only recovery
script restores the paired old runtime and release without overwriting the
Accounts database.

The Accounts database passed integrity checks with migrations 1–14. The backup
service reads its database path from the same runtime environment as the API;
a fresh encrypted cloud backup completed successfully after the live smoke.

Historical resources were not automatically imported. Ownership adoption needs
a verified old-principal-to-Accounts-UUID mapping. The retired IAM testing worlds
and Briefcase OBO delivery flow are not part of this release.

## Platform availability

Apps currently validates Browser's Linux x86-64 and ARM64 packages. macOS and
Windows validation workers are unavailable on the Apps service. macOS and Linux
also have npm binaries and standalone archives; the complete Apps archive retains
all six native target payloads. Linux binaries reference glibc 2.34 or earlier.

## Published distribution

Silicon Apps publicly lists [Silicon Browser](https://apps.teamofsilicons.com/apps/browser)
with Silicon `si:tos` as its author. Production version `1.1.0` resolves anonymously
to signed packages under release `06eacae9-cbdc-4e97-b6dd-83c3a6264b2a`:

- Linux x86-64: `d18370db-b980-421d-b2ca-19b8e158d133`.
- Linux ARM64: `145899a7-0c07-4835-8e32-21ac4b1d85ff`.

Both packages passed the hosted discovery checks. The development release was
verified with the official Apps CLI before promotion; the production signature
was verified anonymously afterward.

The Rust crates `silicon-browser-shared`, `silicon-browser` and
`silicon-browser-cli` were published to crates.io at `1.1.0` in dependency order.

The [GitHub release](https://github.com/unlikefraction/silicon-browser/releases/tag/managed-v1.1.0)
is public at tag `managed-v1.1.0`, targeting `f6552bdd6455c72e12c0a92b4e58a2e078b839c0`.
All twelve uploaded assets match their local SHA-256 values, including the
standalone archives, complete Apps archive, checksums and provenance.

[npm silicon-browser](https://www.npmjs.com/package/silicon-browser/v/1.1.0)
was published at `1.1.0`, and the public `latest` tag resolves to this version.
The downloaded archive matches the prepared package byte for byte:
`963e62cac32a492b8a68ff3e0fe378bdc7d8ac23f4b2eea45a5d01a673f71b60`.
A fresh temporary npm installation passed version, help, Accounts discovery and
signed-out status checks with an isolated configuration directory.

The [main CI run](https://github.com/unlikefraction/silicon-browser/actions/runs/38036428593)
passed after the release branch was merged and pushed normally. Canonical
production documentation was verified against the committed source after
Vercel's automatic main deployment.
