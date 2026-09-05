# Briefcase slow OBO upload retest

Two live requests (one slow test and one fast control) were made against the authorized Briefcase test environment `01a07249-f2ad-7922-87bc-ca984d0d54cc` on 2026-09-05. The test obtained a fresh Browser application session through SLT exchange; it did not rotate the shared test session or upload production content. No failed request was retried; the control used a new filename and proof.

## Observed result

- File: `slow-proof-4aaf462888bc.bin`, 71,680 deterministic bytes (70 KiB).
- SHA-256: `7eef8781c29227527dcb70af4df77b29e23ca318775a4562e91eb19b793c500b`.
- Proof ID: `01a0726d-3dfb-73a3-bf54-e0accf052fdc`.
- Upload began at `2026-09-05T16:35:57.165477+00:00`, approximately 59.886 seconds before proof expiry.
- The request sent 1 KiB chunks at one-second intervals, with a Content-Length and descriptive User-Agent.
- Final body bytes were sent after 69.004 seconds, approximately 9.118 seconds after expiry.
- Response: **HTTP 401**, error `unauthenticated`, received after 69.245 seconds.
- Briefcase request ID: `01a0726d-5a47-7231-8152-fc79661f334b`.
- No entry ID was returned. Subsequent Carbon CLI listing confirmed the slow
  filename absent and the fast control entry present.

This reproduces the timing constraint with a small payload: beginning an upload while a proof is valid does not reserve its authorization through the end of the stream. The response is consistent with proof expiry; the public error deliberately does not name the internal rejection reason.

## Fast control

A second request sent the identical 71,680 bytes and SHA-256 without throttling, using the same application, actor, testing environment, and metadata shape, with a new filename and proof. It returned **HTTP 201** after 1.654 seconds; the body finished after 0.486 seconds. Entry ID: `01a0726f-40f2-7f72-ab79-92799912bc63`. Request ID: `01a0726f-3de9-7461-8660-b8153b7e6f52`. Filename: `fast-proof-fba5ec1573ec.bin`.

The slow harness retained its fresh application credentials only in memory. The control therefore used another fresh SLT exchange for the same actor/application rather than the exact same refresh family. This distinction limits the comparison, but the successful identical payload and binding shape strengthens the timing explanation for the slow request's rejection.

The Carbon CLI independently reconciled the app directory after both requests:
the failed slow filename is absent, and the control entry is present. Final
sandbox storage is **314,654,044 / 2,147,483,648 bytes**, including the separate
300 MiB test artifact. The increase from the previous 314,582,364-byte checkpoint
is exactly the successful 71,680-byte control. No extra file was committed by the
slow request. Sanitized evidence: `/tmp/sb-briefcase-audit/slow-control-reconciled.json`.

## Source explanation and scope

At Briefcase commit `5f27dd915c88d2cb839de368f273436f9254f9bf`, [the OBO handler](https://github.com/teamofsilicons/silicon-briefcase/blob/5f27dd915c88d2cb839de368f273436f9254f9bf/src/api/handlers/obo.rs#L70) stages and hashes the complete inbound body before calling IAM verification. At IAM commit `bab75c0a909481ad2d5dca5bd7d52df08476eaf3`, [proof lifetime and verification](https://github.com/teamofsilicons/silicon-iam/blob/bab75c0a909481ad2d5dca5bd7d52df08476eaf3/src/features/applications/obo.rs#L43) use a 60-second lifetime and require the proof still to be unexpired at verification (line 956). Entry creation follows successful verification.

Browser should stage/hash bytes before minting a proof and begin streaming immediately. The deadline concerns Browser-to-Briefcase transfer plus verification, not subsequent Briefcase-to-storage multipart processing. The successful 300 MiB test exercises the multipart path under its measured throughput; it does not establish that a 300 GiB inbound transfer fits this authorization window. The 2 GiB sandbox storage quota is independent of this timing behavior.

The automatic application directory and absence of delegated deletion are intended constraints, not failures. Backend-owned application refresh can obtain a fresh OAT before minting a proof; it does not extend an already-issued proof. This test does not establish a durable recording worker or exactly-once upload recovery.
