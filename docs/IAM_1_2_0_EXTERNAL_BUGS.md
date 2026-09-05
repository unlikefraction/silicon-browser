# IAM 1.2.0 external findings and retest status

Historical report. See [the 1.2.1 retest](IAM_1_2_1_RETEST.md) for current status.

Retested on 2026-09-05 after confirming the executable reports `iam 1.2.0`. Both published CLI and client 1.2.0 record source commit `ec04ec92444e02c88a39c83a286dbf47b5ded458`.

The four previously reported local correctness/security bugs did not reproduce in the initial retest. A later concurrent audit recorded one unsuccessful command whose cause is unresolved; seven instrumented reruns passed. No new local security or functional bug has been confirmed. A new cosmetic CLI confirmation defect was observed during live tag removal. GitHub main documentation/source drift remains outstanding. Live service results are distinguished below from offline checks.

This report contains no real credentials, environment keys, OTPs, or personal contact details. Nothing was posted externally. The earlier [1.1.1 report](IAM_UPSTREAM_FINDINGS.md) is historical evidence, not the current defect list.

## Outstanding external issue: GitHub main does not match the release

See also [the 1.2.0 provenance audit](IAM_1_2_PROVENANCE.md). The inspected GitHub main checkout still points to `d9fa8745b28a5aff3cd041005fd8855ce10f73ca`, where the CLI and client manifests and client README describe **1.1.0**. The published **1.2.0** crates identify a different source commit. This makes the user-provided GitHub main docs unsuitable as the sole source for auditing installed behavior.

**Recommendation:** Publish matching release tags/source and version the documentation. Link the release source directly from installation/update instructions. Use downloaded 1.2.0 crate source and the running service contract when diagnosing current integration behavior.

## Unresolved intermittent observation: one concurrent logout failed

A later audit run, while repository builds/tests were also running, recorded **1 unsuccessful logout and 1 targeted session remaining** in its first 32-command concurrency group. The next two groups passed. That version of the harness recorded counts only, so the failed command's exit code and stderr are unavailable. It is not possible to tell from that log whether the cause was IAM, host resource pressure, or another environmental failure. One failed command leaving one session stored does not by itself prove a successful write was lost.

The harness now retains the failing profile, exit code, and sanitized stderr, and separately lists any targeted sessions that remain despite a successful command. **Seven complete instrumented reruns passed: 672 concurrent logout calls, no command failures, and no lost successful removals.** No additional stress runs were performed after those bounded checks.

**Current classification:** Unresolved intermittent failure, not a confirmed regression and not an all-clear. Preserve the enhanced diagnostic output if it recurs; do not silently retry and discard evidence. The original summary is retained at `/tmp/sb12-offline-final.log` in this local audit environment.

## New low-severity UX finding: clearing tags prints an empty confirmation

**Observed hands-on in 1.2.0:** Removing the last tag from a membership succeeds, but text output says exactly `Tags are now .` before normal next-step guidance. The operation completed; the defect is ambiguous confirmation wording, not failed authorization or mutation.

Generic reproduction, using a test membership that currently has a tag and a caller authorized to change it:

```sh
iam --test <ENVIRONMENT_ID> --org <ORG_ID> approval set-tags <MEMBERSHIP_ID>
```

Omit every `--tag` argument to request an empty tag set. This was already executed by the main live test; no additional mutation was performed for this report.

**Source confirmation:** Published `silicon-iam-cli-1.2.0/src/commands/approval.rs:175` formats `Tags are now {}.` by joining the returned tag list. An empty list produces an empty interpolation.

**Expected / suggested fix:** For an empty returned list, print `Tags cleared.` or `No tags remain.` Preserve the current list rendering when tags exist. Add a small text-rendering regression for empty and nonempty results; JSON output need not change.

## Previously reported defects: current status

| Earlier finding | 1.2.0 result | Evidence |
| --- | --- | --- |
| Concurrent credential writes lose updates/read truncated JSON | Original corruption not reproduced; intermittent failure unresolved | Initial96-call retest passed. A later run recorded1 failed command/1 retained target without diagnostic details; seven instrumented reruns then passed672 calls. See unresolved observation above. |
| Credential writes follow symlinks and clobber targets | Fixed in tested cases | Credential, config, lock, and home symlinks rejected with exit 2; victim files unchanged. Added credential hardlink and FIFO tests also rejected safely. |
| IPv6 loopback URL rejected despite help | Fixed | `--url http://[::1]:9 config show` succeeds. `127.0.0.1` and `127.0.0.2` also accepted. This exercises real URL construction without requiring a listening server. |
| Invalid Carbon ID rejected only after both OTP ceremonies | Fixed | `carbon0`, `UPPER`, a two-character ID, and a 31-character ID all fail with exit 2 during argument parsing, before network access. Help now explicitly documents digits 1–9 and excludes 0. |
| Public edge rejects documented HTTP loopback Application base URL | Documentation mismatch fixed; known hosted restriction | Current bundled `iam docs authorization` and Application update flag help explicitly describe hosted loopback rejection and require public HTTPS examples. A fresh raw-response check returned HTTP 403 at the hosted edge; this agrees with the documented restriction and is not a new defect. |
| GitHub main source/docs lag published crate | Still open | Main checkout identifies 1.1.0; released CLI/client are 1.2.0 with different recorded source provenance. |

Source review supports the observed fixes: CLI `store.rs` now uses a shared store lock, separate session-transition locks, pinned directory handles, no-follow file opens, hardlink checks, and exclusive temporary files followed by sync/atomic rename. Client `client.rs` validates loopback addresses through typed `url::Host` variants. CLI Carbon ID parsing calls the client validation function before dispatch.

These are bounded regression checks, not a claim that every possible race, operating system, or filesystem has been verified. All filesystem cases ran on the current macOS host.

## Upgrade behavior: old 0755 home requires a one-time permission repair

A real home created under 1.1.1 had mode **0755** while its credential file was **0600**. After upgrading, `whoami`, login, and system commands refused that directory and explicitly instructed `chmod 700` on the IAM home. The main test runner checked ownership, applied that permission correction, and reached the interactive login prompt.

This is an intentional stronger security requirement with clear recovery, not evidence that symlink protection or credential persistence is broken. Release notes should call out the migration. The update warning and command error can repeat the same permission problem; consolidating that wording would improve recovery clarity.

## Realistic fresh-user and agent usage checks

In a temporary, empty home with automatic updates disabled, exercised the CLI directly without credentials or a terminal:

- Bare `iam` and `iam org` expose available commands and usage.
- `iam whoami` reports the selected profile/service/environment and gives Carbon and Silicon login commands; it explains that an Application token does not replace a direct IAM session.
- `iam login` without an identity prints the missing identity requirement and relevant help.
- `iam silicon-login` without arguments promptly explains `--sid` and `--stk`, mentions reuse of an existing session with `--app-id`, and says piped input was not read. It does not silently hang waiting for credentials.
- `iam login --app-id browser` while signed out gives the sign-in recovery path.
- `iam config show` displays the resolved empty profile cleanly.
- `iam docs --help` exposes offline documentation and search.
- `iam signup ... --carbon-id browser0` immediately explains the format mistake. `iam signup --help` now documents the same rule and the production verification requirement.

These direct usage checks complement the automated filesystem/concurrency regressions. The main integration run separately exercises real Carbon/Silicon interactive and application-token workflows in the authorized test environment.

## Reproduce the offline checks

Run from the Browser repository:

```sh
python3 scripts/iam_offline_audit.py --expected-version 1.2.0
```

The script requires the installed version to be exactly **1.2.0**, creates temporary private IAM homes, supplies only synthetic credentials, disables IAM automatic updates, and makes no live IAM requests. URL acceptance uses `config show`; malformed signup requests stop at local argument validation. It runs 16 grouped checks and exits nonzero if a check fails. Latest instrumented result: **16/16 groups passed**. The separate intermittent failed run remains unresolved as described above.
