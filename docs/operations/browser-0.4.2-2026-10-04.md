# Browser 0.4.2 release — 4 October 2026

Release evidence is recorded on 3 October UTC / 4 October Asia/Kolkata.

## Source and local verification

The native CLI and controller package was built from `74e72a307a72eb95d3bffcd3757f54697d652c16`. Backend source is `2bcd26489fe5695f785188fd5f10d4f4c4bd4b6d`; its later changes bind consent links, callbacks, and state to the exact IAM request. Release commit `55100e27a69a0ca083529cd691c2edc34561a840` adds verified installer hashes and npm payloads. CLI, shared/client sources, Cargo manifests/lockfile, Honeycomb manifest, and native packaging inputs are identical across those commits.

[Source CI](https://github.com/unlikefraction/silicon-browser/actions/runs/37146023361), [all six native builds and packaging](https://github.com/unlikefraction/silicon-browser/actions/runs/37145497699), and [the production ARM64 backend build](https://github.com/unlikefraction/silicon-browser/actions/runs/37146023174) passed. Local workspace tests, strict Clippy/rustdoc, formatting, 53 frontend tests, production frontend build, six installer tests, and database migration/recording recovery checks passed. Desktop and mobile website checks covered context-bound login and recording consent.

The backend executable SHA256 is `22a3735faef2a83ce165b41320cc513c4de98b5d0569e9cff0371ed4e9fcfc4b`. The six-platform Honeycomb archive SHA256 is `2551aefed906fc9f46d7e83be226af15157fd7fa50d20762196073f3386d4a5e` (45,786,164 bytes). Public assets include per-archive checksums, twelve native executable hashes, and source/build provenance.

## Production backend and retained data

Production now runs `/opt/silicon-browser/releases/20261004-2bcd2648-0.4.2`. Cutover SSM command `b832b601-a72a-4aef-8ecd-20966437b07d` succeeded. The exact sole API ingress permission was removed during the stopped-state cutover and restored after local health and running-executable checks. Public `/healthz` and `/api/v1/iam` return 200; unauthenticated `/api/v1/me` returns 401. The service has zero restarts and its backup timer is active.

The candidate first applied embedded migrations to a copy, then to the stopped live database, preserving deployed migration checksums. Schema 13 retains 58 sessions, 2,197 commands, 2,217 reports, 58 recordings and 107 artifacts. All six completed artifact rows and hidden-recording states match exactly. The migration holds 98 uncertain legacy artifacts instead of retrying them with a new upload identity. There were no active sessions, enabled OBO grants, pending artifacts, unfinished outbox operations, or registered testing stores at cutover.

The fresh, integrity-checked stopped backup is retained at `/var/lib/silicon-browser/rollout-0.4.2-20261003T190318Z/before.db` and private S3 key `backups/iam5-042/20261003T190318Z-cutover.db`, SHA256 `235a58b1ac139fd68cf20bba22c6b1405f3997c14d8e9dbd9a14f2d792e846e2`. S3 encryption, length and digest metadata were verified. The release archive SHA256 is `dc7fdc75e0f9e60d0cef6d027a1b1f16283d76e159b490e6d2caebed76af93f0`. Rollback restores the paired stopped database, configuration and old executable before reopening ingress; the old executable cannot run against migrations it does not recognize. The activation wrapper passed seven offline success/failure-path scenarios.

## Published package verification

The `silicon-browser-shared`, `silicon-browser`, and `silicon-browser-cli` crates are published at 0.4.2. Registry checksums and downloaded archives match the exact locally published packages:

| Crate | SHA256 |
| --- | --- |
| silicon-browser-shared | `8c4e09cc960a73773bd4ba659550ae32249b66354abab2d9ebfe1d6c7d4bd9df` |
| silicon-browser | `162b033087de668e0836907f596c462aaed9353cf17e19d611653a7326a4d6b0` |
| silicon-browser-cli | `49478115871aaf2587f4296d789247e0c9ef29379293a0b8fdc83fa160aa238f` |

npm `silicon-browser@1.0.4` is the public `latest` and contains the same four macOS/Linux Browser 0.4.2 binaries. Registry SHA1 `b145850a01508737ecaf0dd03a0dbc95dd05f5f2` and SHA512 integrity match the upload. A clean registry `npm exec --yes --package=silicon-browser@1.0.4 -- browser --version` returned `browser 0.4.2`.

Honeycomb development operation `51f91c31-208d-49b3-8668-edb8131c8dbb` and production operation `7c8f0f0b-f10f-46fc-99f2-5f99eada8624` accepted the identical 0.4.2 archive. Both channels are public and published under configuration revision 4 with no pending permission approval. Anonymous clean-home macOS ARM64 installs in both channels returned `browser 0.4.2` and `agent-browser 0.36.0`, passed help checks, and matched archive executable hashes.

## IAM and Honeycomb permissions

Browser's accepted/effective Honeycomb configuration is revision 4, active in IAM as revision 46. Publication `7f9c9101-6d03-4d6c-810d-590ce505d2b2` is published. It declares exactly four Briefcase roots: `briefcase.uploads.reserve`, `briefcase.uploads.commit`, `briefcase.uploads.status`, and `briefcase.entries.list`. Existing three-root approvals require fresh explicit feature consent; ordinary login remains separate.

## Remaining shared-testing limitation

Shared test environment `7c761acc-adb5-40e9-9498-a8b3e5a86396` has an incomplete Browser participant import because its protected lifecycle transport is not configured. Production checks do not certify shared testing. The receiver and Honeycomb operator registration still need implementation/configuration; see [the integration contract](../BRIEFCASE_INTEGRATION.md#testing-and-release). Production credentials were not substituted into that testing environment.
