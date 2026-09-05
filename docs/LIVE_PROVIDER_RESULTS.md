# Live provider verification

> Historical provider test using the former backend controller. That runner and its server execution path have been removed; see [current local execution](COMMAND_EXECUTION_GAPS.md). Reported timings, failures and fixes below describe the tested revision only.

Date: 2026-09-05. Local backend against the real Browser Use and TinyFish services, with isolated IAM test-environment identities. Provider credentials, bearer tokens, signed URLs, and session identifiers are omitted.

## Initial bounded run

| Check | Result | Elapsed |
| --- | --- | --- |
| IAM-authenticated identity | HTTP 200 | 0.271 s |
| Create incognito session, TTL 15 minutes | HTTP 200; active | 1.397 s |
| Navigate, title, snapshot | HTTP stream succeeded; underlying command exited 1 | 0.043–0.165 s |
| Issue live link | HTTP 200; link present | 0.001 s |
| Explicitly end session | HTTP 200; ended | 0.353 s |
| Read terminal session | HTTP 200; ended | 0.001 s |
| Read usage | HTTP 200; zero proxy traffic for incognito | 0.001 s |
| Read recording | HTTP 200; pending | 0.001 s |
| TinyFish search | HTTP 200; 10 results | 2.444 s |
| TinyFish fetch example.com | HTTP 200; one successful result | 4.328 s |

An additional short diagnostic session identified the command failure: the generated Unix socket path was 205 bytes; agent-browser enforces a 103-byte maximum. That session was also explicitly ended and confirmed ended.

## Fixes exercised by regression tests

- Version checks now inherit only the safe runtime environment allowlist, enabling PATH and env-shebang executable discovery without forwarding IAM credentials or Node injection settings.
- Explicit relative executable paths are anchored before the subprocess changes into its isolated working directory.
- Session sockets use short, private, distinct directories; directory ownership and symlink checks precede use, and normal cleanup removes the socket directory. A regression binds a real Unix socket with a long isolation parent.
- All 12 command-runner tests pass, including version pinning, command restrictions, timeout handling, process isolation, socket binding/cleanup, and relative executable execution.

## Scope and limitations

Only example.com was opened. No proxy/profile session was needed. Search and fetch each issued one upstream request in the initial run. Every created paid session was ended in a finally block and independently read back as ended. The issued live link was not redeemed externally.

Recording status is accurately pending: the repository still uses DeferredArtifactStore, so this does not verify a completed Briefcase OBO upload or a durable externally stored recording. HTTP 200 on usage confirms retrieval, not independently reconciled provider billing accuracy.

## Final post-fix real-provider retest

| Check | Result | Elapsed |
| --- | --- | --- |
| Create incognito session | HTTP 200; active | 1.207 s |
| Open https://example.com | Exit 0; Example Domain confirmed | 5.815 s |
| Get page title | Exit 0; Example Domain confirmed | 0.800 s |
| Snapshot | Exit 0; Example Domain confirmed | 1.793 s |
| Issue live link | HTTP 200; link present | 0.002 s |
| Explicitly end session | HTTP 200; ended | 3.367 s |
| Read command audit logs | Three logged silicon commands | Passed |
| Read usage | 333 USD micro-units ($0.000333); zero proxy bytes | Passed |
| Read recording | Pending, consistent with deferred Briefcase adapter | Passed |
| List all three audit sessions | All three ended; none active | Passed |

The two initial diagnostic sessions and final smoke session were all explicitly stopped. The final command run confirms the socket-path fix against the real Browser Use browser, not just a stub. Provider-reported cost is recorded as returned and was not reconciled to an external invoice.

A subsequent regression also covers the public runner's maximum 128-byte custom session ID: long IDs receive a deterministic shorter local daemon name, while each full ID retains its own socket directory. Backend-generated UUID session names remain unchanged.
