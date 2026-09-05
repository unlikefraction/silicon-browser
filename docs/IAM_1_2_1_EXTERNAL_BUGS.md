# IAM 1.2.1 external bug findings — 2026-09-05

Confirmed installed binary `iam 1.2.1`. Published CLI source records commit `d4b34c93df107e14116da856e27883b12e42302a`.

Three complete instrumented suites ran: **47/48 groups passed**. Of **288 concurrent local logout commands, 287 succeeded and 1 failed**. No session whose logout succeeded remained stored. Do not describe this result as all tests passing.

## Outstanding intermittent local failure: credential lock creation

The first suite's first concurrency group failed for synthetic `profile-0` with exit **2**:

```text
Local credential removal could not be confirmed.
Context: profile=profile-0, service=https://backend.iam.teamofsilicons.com, environment=production, organization=(none)
error: cannot access $ISOLATED_ROOT/race-0/credentials.lock: cannot open lock: No such file or directory (os error 2)
help: iam logout --help
```

That failed command's session remained stored. `successful_targets_remaining` was empty. The other eight concurrency groups passed, including every group in the second and third suites.

This is a confirmed intermittent CLI failure under concurrent local usage, with a precise captured error. It is **not evidence of a successful logout being lost or JSON corruption**. The narrower root cause remains unproven. It may explain the earlier 1.2.0 observation, but that older run lacked diagnostics, so identity of cause is not established.

The test creates an existing private home and a valid synthetic credential file, then launches 32 distinct-profile local logouts with 16 workers. It neither removes the home nor deletes the lock file while commands are running. The reported `environment=production` is the empty profile's display context; `logout --local-only` makes no IAM request and all credentials are synthetic.

Published CLI source `src/store.rs`:

- `LockedSession::forget` obtains `credentials.lock` before reading/modifying credentials.
- `StoreDirectory::lock` calls `open_file(name, true, false)` and immediately surfaces the open error.
- `open_file` uses directory-relative `openat` with `RDWR | CREATE | NOFOLLOW | CLOEXEC | NONBLOCK` on Unix.
- The lock file is documented as permanent; no normal lock unlink was found in that source.

**Working hypothesis, not a diagnosis:** The failure occurs when several fresh processes create/open the shared lock for the first time. Concurrent first-use file creation or a platform filesystem transient is therefore a more specific lead than the old unlocked credential-write bug. The source intends an atomic create-or-open operation, so inspection alone does not explain why the OS returned `ENOENT`. No reproducible harness deletion race was found.

**Recommended upstream investigation:** Reproduce concurrent first-use lock creation on macOS/APFS and capture filesystem-level evidence for the unexpected `ENOENT`. If a platform transient is confirmed, handle it with a bounded retry while retaining no-follow checks, ownership checks, and a pinned directory. Do not replace this with an unlocked fallback or silently report successful logout.

## Other earlier findings

All three suites passed credential/config/lock/home symlink rejection, credential hardlink/FIFO rejection, IPv4/IPv6 loopback acceptance, and invalid Carbon ID local validation. Victim files remained untouched.

Source inspection confirms the cosmetic empty-tag message is fixed in `silicon-iam-cli-1.2.1/src/commands/approval.rs:174`: an empty returned tag list prints `Tags cleared.`; a nonempty list retains `Tags are now ...`. The main retest also granted and cleared the test Silicon’s tag through the real CLI: clearing printed `Tags cleared.`, immediately invalidated the old Browser token, and fresh login returned an identity without that tag.

## Reproduction and captured evidence

```sh
python3 scripts/iam_offline_audit.py --expected-version 1.2.1
```

The script now defaults to expected version **1.2.1**. For historical retests, pass `--expected-version 1.2.0` while that exact binary is installed. A version mismatch aborts before testing. The script retains failing exit codes, sanitized stderr, and a separate list of successful commands whose target remained stored.

Captured local evidence:

- `/tmp/sb121-offline-1.json`: one failed concurrency group; detailed error retained.
- `/tmp/sb121-offline-2.json`: all 16 groups passed.
- `/tmp/sb121-offline-3.json`: all 16 groups passed.

The offline suites used only synthetic credentials and made no live IAM calls. The separately described tag walkthrough used the authorized testing environment. Nothing was posted externally and no production records were mutated.
