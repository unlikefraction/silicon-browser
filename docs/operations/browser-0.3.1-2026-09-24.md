# Browser 0.3.1 release — 24 September 2026

All release channels are live. Deployment completed on 23 September UTC / 24 September Asia/Kolkata.

## Source and validation

Backend and all six native clients were built from `b72250d4779e9691bb825f5192ce385032d01922`. Release tag [`managed-v0.3.1`](https://github.com/unlikefraction/silicon-browser/releases/tag/managed-v0.3.1) points to `5b6cdfabca61f346102db8f98dce6381b2c53afc`, which adds verified packaged binaries, installer checksums, and documentation. Cargo manifests, lockfile, native source, Honeycomb manifest, and native packaging inputs are identical between these commits.

[CI](https://github.com/unlikefraction/silicon-browser/actions/runs/35912185227), [all six native builds and package validation](https://github.com/unlikefraction/silicon-browser/actions/runs/35912185113), and [the production ARM64 backend build](https://github.com/unlikefraction/silicon-browser/actions/runs/35912184292) passed. Local workspace tests, Clippy, rustdoc, formatting, 35 frontend tests, frontend build, migration/operator checks, and installer tests also passed.

## Production backend and retained data

Production selects `/opt/silicon-browser/releases/20260924-b72250d-0.3.1`; executable SHA256 is `3e69526fa412cd7ec5baa196ea9cb0e0f03029dee6735299210e6cd59d5e11a4`. The service is active with zero restarts, and the backup timer is active. Public `/healthz` and `/api/v1/iam` returned 200, with app ID `browser`; unauthenticated profile access returned 401.

The existing database already contained canonical identifiers. The network-free verifier preserved deployed migration 0008 exactly, applied marker-only migration 0009, and bound the production world. It did not replay identity conversion or rotate secrets. Rehearsal and stopped-state activation compared full per-table row hashes before and after migration and candidate startup, including ciphertext, frozen command identities, receipts, and artifact states. All existing rows matched: 55 sessions, 2,082 commands, 2,102 reports, and 102 artifacts. The 96 failed and six completed artifacts retained their exact states. There were no testing stores or pending outbox operations.

The sole API ingress permission was temporarily removed at 20:00:54 UTC and restored exactly at 20:03:55 UTC after local health and backup verification. Rehearsal SSM `257021ee-303a-4d75-a36b-aa407e109c67`, activation `ee65c434-dfc2-4f3c-b236-3f19962ee893`, and final verification `d81dbcfd-09ec-4be8-b550-8c5c27a06440` succeeded. The stopped rollback snapshot, original environment, units, and prior release were retained. Private S3 archives were verified for AES256 encryption, length, SHA256, and complete download roundtrip. The activation archive SHA256 is `8fb3d3a5a9bfbf7e2844375fc4b7407790d1538d1ae18c79373620e36bf1b30d`.

Fresh IAM 4 authentication and the release CLI passed production login status and existing-profile reads before and after deployment. No paid browser-provider sessions were created.

## Published clients and frontend

- Honeycomb production release `baf7ddfd-04ea-48aa-ac4c-49bf465afa3e` is accepted, with publication state `published`; application revision remains 2. Archive SHA256: `e8d8a29d423d9d2baa99543fbf2259a826ab6cb93ddac0e7a1a8b7d8cf512158`. An anonymous clean-home install selected 0.3.1 and matched both archive and native-binary hashes.
- The GitHub release contains ten verified assets: four standalone archives and their checksums, plus the six-platform Honeycomb archive and checksum. Every uploaded asset's SHA256 and size matched locally. The pinned public installer matched its source bytes and installed `browser 0.3.1` in a fresh home.
- Crates.io now publishes `silicon-browser-shared`, `silicon-browser`, and `silicon-browser-cli` at 0.3.1. Each package was verified and published in dependency order from the clean source commit.
- npm `silicon-browser@1.0.3` is the public `latest` and embeds the same four macOS/Linux Browser 0.3.1 binaries. The downloaded registry tarball matched the upload byte-for-byte, SHA1 `5d724847c447520ee38086448544571272d154f4`, SHA512 integrity, and all four native receipts. Its fresh launcher reported `browser 0.3.1`.
- Vercel production deployment `dpl_5QANn5ky9e1u1tFv8PXWyda9D8XL` serves source `5b6cdfa` on the existing [Browser domain](https://browser.teamofsilicons.com). The live bundle is `index-5HlmELtf.js`; [live docs](https://browser.teamofsilicons.com/docs) contain the published 0.3.1 installer and crates.io instructions. No domain transfer or DNS changes were needed.

Existing published archives remain immutable. No account, webhook, credential, provider configuration, or user installation was replaced during release verification.
