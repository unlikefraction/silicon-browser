# Earlier IAM issues retested on 1.2.1 — 2026-09-05

Both the installed CLI and Browser's pinned SDK are 1.2.1. This retest targets the
previous failures, with instrumented local regression tests and real Carbon/Silicon
terminal workflows in the existing authorized testing environment.

| Earlier issue | Current result |
| --- | --- |
| Credential-write corruption/lost successful updates | No corruption or lost successful logout observed across 288 commands. One command failed to open its lock; see below. |
| Unsafe credential/config/lock/home symlinks | Rejected in all three suites; victim files preserved. Hardlink and FIFO cases also passed. |
| IPv6 loopback rejected by client | IPv6 and both IPv4 loopback checks passed in all three suites. |
| Invalid Carbon ID rejected after OTP work | All four malformed IDs rejected locally before network/OTP in each suite. |
| Ambiguous empty-tag confirmation | Fixed: actual CLI tag removal printed `Tags cleared.` |
| GitHub main behind published source | Fixed: main and both released crates identify `d4b34c93df107e14116da856e27883b12e42302a`, version 1.2.1. Matching v1.2.0/v1.2.1 tags now exist. |
| Old IAM home mode 0755 rejected | Still intentionally requires private permissions. Isolated check verified chmod guidance and successful operation after mode 0700. |
| Hosted application loopback URL rejected | Still explicitly documented as an ingress restriction. No registration mutation repeated; local URL acceptance is a different check. |

## Remaining failure

**287/288 concurrent logouts succeeded; 47/48 offline test groups passed.** One
command exited 2 because opening `credentials.lock` returned `ENOENT`. Its target
remained stored, and no successful command's target remained. This gives a concrete
error for an intermittent failure; it does not prove the exact cause of the older
undiagnosed failure. Details, source investigation, and reproduction are in
[IAM_1_2_1_EXTERNAL_BUGS.md](IAM_1_2_1_EXTERNAL_BUGS.md).

## Live verification

- Carbon: **27/27** Browser auth checks passed.
- Silicon: **27/27** Browser auth checks passed.
- IAM application discovery, saved-session app login, SLT exchange/idempotent retry,
  consumed-SLT rejection, authoritative introspection, wrong-org rejection,
  refresh/rotation retry, OBO verification/replay rejection, and Silicon exchange passed.
- Explicit token revocation made IAM introspection inactive and Browser returned
  HTTP 401 immediately.
- Through normal CLI commands, Carbon granted the test Silicon a tag. Silicon
  signed in, ran Browser setup, and saw the tag in its identity. Carbon then cleared
  the tag; Silicon's saved application token immediately failed. Fresh login/setup
  worked and returned an identity without the tag. The test membership was restored
  to its original empty tag set.

No new paid browser sessions were created for this IAM-focused retest. The temporary
backend was stopped afterward. The previous upgrade already passed all232 Rust
workspace tests and a full build; this turn changes the audit harness and reports,
not Browser runtime code. Provider proxy metering and deferred Briefcase recording
are separate issues, not claimed resolved by IAM 1.2.1.
