# Briefcase 0.1.3 retest

## CLI and published source compatibility

Reviewed published `briefcase-cli 0.1.3` and `briefcase-client 0.1.3` crate source. CLI release provenance: `d22424caed0509b10d2b2ebd86cf78b26fba7679`.

The published CLI source now includes the features missing from installed 0.1.1 during the earlier audit:

- `env` lifecycle commands, including create and key retrieval.
- A `--test` selector using the Briefcase public environment UUID.
- `login --slt-stdin`, with explicit rejection of an access-token override as a login method.
- Separately stored production/test rotating sessions and test roots, bound to canonical deployment origin and organization.

The installed executable now reports `briefcase 0.1.3`. Real `env create --help`, `login --help`, and `env current --help` all exited 0; creation exposes IAM pairing flags and login exposes `--slt-stdin`. The earlier published-CLI compatibility gap is fixed. Authenticated command results follow below.

## Safe private CLI bootstrap from an existing paired fixture

Use a new private `BRIEFCASE_HOME` directory (0700), not the user's default state. Write any secret-bearing JSON as 0600. The production Briefcase auth pair, IAM root, Briefcase root, and imported Application secrets are different credentials and must not be interchanged.

Flow exercised using the paired Briefcase UUID/root:

1. Seed a named profile with the exact `https://backend.briefcase.teamofsilicons.com/api/v1/` URL and `tos` organization.
2. Seed only its Briefcase root and explicit scope binding, then obtain a fresh test IAM SLT for `tos>briefcase` and use real `briefcase --test <Briefcase-UUID> login --org tos --slt-stdin`. This exercises session validation and lets the CLI calculate expiry at the actual exchange time.
3. Use separate named profiles for Carbon and Silicon so their session state cannot overwrite each other. Both can refer to the same paired root if their organization and origin match.
4. Verify `env current` (root selection), then `ls` (member authentication and current permissions). Root self-description alone is not evidence that member access works.

Reviewed 0.1.3 state schema for an already-owned root (placeholders only):

```json
{
  "testing_environment_keys": {
    "carbon": {"<Briefcase-UUID>": "<Briefcase-root>"}
  },
  "testing_environment_scopes": {
    "carbon": {
      "<Briefcase-UUID>": {
        "deployment_origin": "https://backend.briefcase.teamofsilicons.com/",
        "organization": "tos"
      }
    }
  }
}
```

Configuration contains `current_profile`, `auto_update: false`, and `profiles.carbon` with `url` and `org`. The key is a serialized string. Scope binding is mandatory before forwarding stored roots to an endpoint. Preserve any pending one-use login/refresh idempotency state if a request's outcome is uncertain.

If a complete session is seeded instead, the schema requires `access_token`, `refresh_token`, RFC3339 `expires_at`, `actor`, and `org_id`; derive expiry from the real acquisition timestamp, never from the time an old fixture is copied. Fresh SLT login is simpler and provides stronger test evidence.

## Live outcome

The main runner replayed the original environment-creation intent and received **201**, creating Briefcase environment `01a07249-f2ad-7922-87bc-ca984d0d54cc`, paired with IAM environment `01a07025-85ea-7413-8045-24a897b86f88`. This confirms that the earlier environment-create rejection no longer blocks this exact setup.

The real 0.1.3 CLI was exercised from isolated private home `/tmp/sb-briefcase-audit/cli013`:

- Carbon `sbauditfive`: fresh IAM test SLT passed through stdin; CLI login succeeded and stored a rotating session. `env current`, `ls`, and `status` succeeded. Status reported authenticated; root listing showed Private and Public.
- Silicon `browser-audit-reader:tos`: created as a separate test persona in the test `tos` organization. Authentication used the test-only Silicon credential without putting it in argv. A fresh Briefcase SLT was exchanged through real CLI login into the separate `silicon` profile.
- Silicon Private listing revealed only `private/browser-audit-reader:tos`, not the Carbon's private root.

The main runner's successful raw OBO diagnostic created entry `01a0724c-5c2d-7403-8cee-013f5da987b4` at `private/sbauditfive/apps/tos>browser/obo-diagnostic013-892391dd.txt` (48 bytes).

**Owner readback:** Carbon CLI `stat` and `cat` succeeded. Downloaded bytes matched the uploaded source exactly; SHA-256 `73df6b48f7c4d22100367c339efa24762a45c8de05cdf66f93dc3fd53dc2c404`.

**Cross-actor isolation:** Silicon CLI `stat` and `cat` of that same known entry both returned exit **3**, `not_found`. Correlation IDs: metadata `01a0724d-4d59-71d0-9e74-820e8a23cc73`; content `01a0724d-5137-7e41-83bb-802fe21d24e9`. This verifies denial even with the exact entry UUID known.

An earlier Rust-provider upload attempt returned **403**. Owner CLI listing immediately afterwards showed the Carbon's private folder empty, and the intended app folder/file returned 404. The main runner isolated the cause: public `GET /api/version` without a User-Agent returned **403 HTML** from `awselb`; with a descriptive User-Agent it returned **200 JSON**. The official Briefcase client sets this header; Browser's custom adapter omitted it. The adapter fix adds a descriptive User-Agent. The corrected adapter then returned HTTP 201 for real text and MP4 uploads; the earlier 403 is not attributed to application permission checks.

**External HTTP documentation gap:** Document the hosted edge's User-Agent requirement for direct HTTP integrations. Otherwise a public/read-only call can fail with an HTML 403 before the API's normal structured contract. The public-edge rejection and the Browser adapter omission should be distinguished from IAM/OBO authorization failures.

One initial manual CLI-state seed omitted the canonical origin's trailing slash and was rejected locally before forwarding credentials. The fixture and example above were corrected to match `Config::origin().as_str()`. This was an audit-fixture formatting mistake, not an upstream defect.

No environment cleanup, deletion, key rotation, or re-pairing was performed by this subtask. Existing `sbaudit` fixtures and the user's default CLI profiles were preserved.


## Native CLI owner-file lifecycle

Exercised only synthetic diagnostic entry `01a0724c-5c2d-7403-8cee-013f5da987b4`, as the Carbon owner. All other files, including the main runner's separate uploads, were left alone.

1. Read metadata and its initial retained version.
2. `briefcase put` replaced the same filename in its app folder. Entry UUID stayed unchanged, retained version count grew from **1 to 2**, and `cat` returned the exact replacement bytes.
3. `briefcase mv` renamed the entry within the same parent to `cli013-lifecycle-restored.txt`; the UUID remained unchanged.
4. `briefcase rm` moved it to the recoverable bin. A normal `stat` correctly returned exit **3** / `not_found`; bin listing included that exact entry.
5. `briefcase bin restore` restored the same UUID and renamed path. Final `stat` and `cat` succeeded, and content exactly matched the replacement.

Final path: `private/sbauditfive/apps/tos>browser/cli013-lifecycle-restored.txt`. Final content: **51 bytes**, SHA-256 `65d6f84c67973bfb1d9bbbf7a652bb0dd9677b60f24dc5dcf632c6e25bf3adc1`. All **14 CLI operation exit statuses** matched expectations, including the deliberate not-found check while trashed. No hard deletion occurred; the diagnostic file is restored and readable.

Private local harness: `/tmp/sb-briefcase-audit/cli013-lifecycle.py`; sanitized operation results: `/tmp/sb-briefcase-audit/cli013-lifecycle-results.json`. No new defect was observed in this lifecycle.

## Corrected backend round trips

Using the actual Browser Rust IAM proof issuer and Briefcase adapter:

| Artifact | Entry ID | Bytes | Validation |
| --- | --- | --- | --- |
| Synthetic command-log text | `01a0724e-6878-7581-955b-14aba0f65750` | 110 | HTTP 201, owner download matched every byte |
| Synthetic two-second MP4 | `01a0724e-fbfa-7d80-b237-bdda36fbad7b` | 9,355 | HTTP 201, owner download matched every byte; range 16–47 returned HTTP 206 and the exact 32 requested bytes |

Text SHA-256: `75457d295f8b838cf2880b67b352b747590a021795b0f202f7e1365b4b7ebaee`.
MP4 SHA-256: `6a4931e8a75f42eb3cec40a0838999322400c898f4c0b1e53d83251b124f6a14`.
The MP4 was generated locally as a small test fixture. It is not a real browser
session capture, and this check does not establish automatic session-end delivery
or large-upload behavior.

At this initial checkpoint the full workspace/all-target suite passed **240 tests** (18 client,153 backend,
23 CLI unit,10 CLI integration,36 shared). Formatting, Clippy with warnings denied,
documentation with warnings denied, and the backend example build passed.

Two Browser adapter issues were fixed: missing User-Agent rejected by the hosted
edge, and incorrectly treating preserved original creator metadata as the current
uploading app. No new core Briefcase file-operation defect was confirmed.
[Current findings](BRIEFCASE_0_1_3_EXTERNAL_BUGS.md) distinguish these corrections,
the undocumented edge requirement, and the remaining delegated-protocol gaps.

The sandbox and synthetic files remain available for follow-up verification. No
paid browser sessions were started, no environment was cleaned or retired, and
no upstream issue was posted. Product implementation order remains backend → Rust
client → CLI → frontend; the CLI walkthrough above tests Briefcase's released
client, not a prematurely added Browser product command.

## 300 MiB streaming and native provider recheck

Follow-up on 2026-09-05, with current Briefcase source
`5f27dd915c88d2cb839de368f273436f9254f9bf`. The sandbox reported a
2,147,483,648-byte limit and 9,564 bytes used before this test. One deterministic
binary file was staged locally, hashed, and uploaded through Browser's real Rust
IAM issuer and new streaming Briefcase adapter. No whole-file byte vector was used.

| Check | Result |
| --- | --- |
| Upload | HTTP 201; **314,572,800 bytes (300 MiB)** |
| Entry | `01a0726c-2830-7bf0-b254-1947c313714e` |
| Time | **43.659 seconds** for the example, including local hashing and IAM calls; not isolated network throughput |
| Peak resident memory | **18,874,368 bytes (18 MiB)** for the Rust upload process, measured by macOS `/usr/bin/time -l` |
| Complete download | HTTP 200; 314,572,800 bytes in 52.177 seconds; streamed SHA-256 matches |
| Range reads | HTTP 206 with exact bytes and Content-Range across 8, 100, 200, and 296 MiB, and at the file tail |
| Native CLI | Carbon stat/versions/history/usage pass; exactly one version; Silicon stat of the known UUID returns not_found |
| Sandbox usage after upload | **314,582,364 / 2,147,483,648 bytes**; 1,832,901,284 bytes remain |

SHA-256: `d825f0d6b616bdb3c3c17c62907ff486be39ea69830a03eab4c80af32e300dff`.
The artifact remains in Briefcase's automatically selected app directory.
See [the native CLI evidence](BRIEFCASE_300_MIB_CLI_RETEST.md).

Source precision: `src/domain/multipart.rs` makes **100 MiB the threshold**, not
a fixed chunk size. Above it, the plan rounds file_size/1000 up to a MiB and clamps
the part size between 8 MiB and 5 GiB. A 300 MiB file therefore selects 38 parts
(37 × 8 MiB plus a 4 MiB tail). This is the source-selected plan; the public upload
response does not expose S3's completed part count. The live test establishes the
300 MiB round trip. It does not establish 300 GiB transfer timing, storage capacity,
or reliability. Briefcase stages the entire incoming body before IAM verifies the
proof; multipart storage does not extend the proof lifetime.

A separate 70 KiB request deliberately stretched to 69 seconds returned 401
after proof expiry; the identical payload sent promptly with a fresh proof
returned 201. Owner CLI reconciliation found only the successful control file.
Final sandbox usage including that control is 314,654,044 bytes, below the 2 GiB
cap. See [the slow-transfer report](BRIEFCASE_SLOW_UPLOAD_RETEST.md).

Browser Use already captures the recording. New native metadata correlation and
`recordingAvailable` handling replace stale assumptions in Browser's adapter and
reconciliation. Three existing stopped browser detail responses contained recording
URLs, recording availability, metadata, and usage counters. A nonexistent exact
metadata filter returned zero matches; unfiltered listing returned twelve. Positive
create recovery is covered by mock tests, not a new paid session in this recheck.
See [the native feature review](BROWSER_USE_FEATURES.md).

Final workspace/all-target validation now passes **251 tests** (18 client, 164
backend, 23 CLI unit, 10 CLI integration, 36 shared), including a 101 MiB local
streaming socket test, native metadata matching, unavailable recording handling,
and retention of ambiguous session reservations after empty provider lookup.
Formatting, Clippy and documentation with warnings denied, and the backend example
build pass. These checks do not claim the automatic Briefcase delivery worker is
implemented.

## Automatic delivery completed

A subsequent full Browser session-end walkthrough delivered real Carbon video and Silicon video plus commands automatically. Both recordings became available; recipient CLI downloads matched all stored digests and each file had one version. See [the automatic live retest](AUTOMATIC_RECORDING_LIVE_RETEST.md). Earlier statements above about an absent worker describe the earlier checkpoint, not the current implementation.
