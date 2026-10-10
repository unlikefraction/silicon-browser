# Briefcase recording delivery

Browser stages the remote provider's existing MP4 and Silicon command JSONL,
then delivers them using IAM 5 consent and Briefcase 3 delegated uploads. It does
not implement a browser recorder. This migration is implemented against the
published contracts; its new hosted flow has **not been verified**. Older
[automatic recording evidence](AUTOMATIC_RECORDING_LIVE_RETEST.md) describes the
retired proof protocol and does not validate this migration.

## Authorization and destination

Configure `BRIEFCASE_URL` and `BRIEFCASE_APP_ID` together with Browser's server-side
IAM credentials. Login selects one Carbon or Silicon and one organization.
Recording access is a separate feature consent: setup/login never creates it,
and declining recording consent leaves ordinary login usable.

```sh
browser recording authorize
# Review the returned IAM URL, then supply its one-use code on stdin.
browser recording authorize --authorization-id REQUEST_UUID --code-stdin
browser recording authorization-status
browser recording disconnect
```

Use `--request-key UUID` with the first command to retain a known request identity.
Retry an uncertain completion with the same request and code. The backend retains
the IAM mutation identity, encrypts the separate OBO token families and serializes
refreshes. The CLI's ordinary login refresh token is never used for delivery.
The website offers the same feature approval through IAM.

Browser declares four Briefcase roots in
[`deploy/legacy/honeycomb-application.json`](../deploy/legacy/honeycomb-application.json):

| Endpoint | Purpose |
| --- | --- |
| `briefcase.uploads.reserve` | Reserve private staging for immutable bytes |
| `briefcase.uploads.commit` | Publish those staged bytes with current authority |
| `briefcase.uploads.status` | Reconcile an uncertain logical operation |
| `briefcase.entries.list` | Resolve the committed entry's actual metadata and link |

IAM returns a separate token family per root. All four must retain the same
selected Briefcase account, organization, testing environment and generation.
That provider destination can differ from the Browser login workspace. Jobs
remain owned by their initiating Browser principal and membership; reauthorization
or an account switch cannot silently move existing work into another destination.
Provider selection, IAM approval and explicit user consent remain required.
Identity and membership disclosure are required for resource permissions; unknown
roles or tags grant no authority. `self.tags.read` discloses optional tags.

Every control call uses `X-App-ID`, `X-IAM-OBO-Access-Token`, the selected
`X-Org-ID`, and JSON. Access tokens are opaque credentials returned by IAM;
Browser does not manufacture them or convert old `obo_` proofs. No actor bearer
or refresh token reaches Briefcase. Provider verification decides current
endpoint, account and resource authority on every control call.

Testing additionally uses the recipient application's `ask_` credential in
`X-Briefcase-App-Secret`. The credential returned for the selected OBO testing
context overrides static provider configuration. The old compatibility names
`BRIEFCASE_TEST_ENVIRONMENT_KEY`, `briefcase_test_environment_key` and
`SB_BRIEFCASE_TEST_KEY` refer to this imported application secret, **not** the
32-character IAM root key. Invalid, cleaned or mismatched test contexts never
fall back to production.

## Transfer and recovery

1. Session stop/expiry queues native video lookup. Downloads reject private
   addresses, pin public DNS results, follow no redirects, send no provider/IAM
   credentials and stream to an anonymous private file.
2. Silicon command logs freeze at archive admission. New late reports receive
   `409 report_window_closed`; exact retries retain their receipt. Carbon sessions
   have no command-log artifact. Logs are cooperative and may omit offline actions.
3. Hash the immutable staged file, retaining its size and SHA-256. Preserve one
   non-nil operation UUID and the complete manifest across worker leases/retries.
   Names remain `<session-id>.mp4` and `<session-id>-commands.jsonl`.
4. Reconcile the operation through `POST /api/v1/obo/uploads/status`. Reserve new
   work with `POST /api/v1/obo/uploads/reserve` and
   `{operation_id,parent_path:"",name,content_type,size,sha256}`. The empty parent
   selects Briefcase's private application folder; Browser does not construct it.
5. A `reserved` operation supplies a private upload capability. Transfer the same
   file handle to `PUT /api/v1/obo/uploads/{upload_id}/content` with exact
   `Content-Length`, `X-Org-ID`, `X-Briefcase-Upload-Capability`, and the same test
   selector. No `X-App-ID`, OBO token or actor bearer accompanies the bytes.
6. `staged` content needs `POST /api/v1/obo/uploads/commit` with the original
   `{operation_id,upload_id}` and a current commit token. Transfer can outlive the
   reserve token. The capability alone cannot publish.
7. `committed` returns `published_entry_id`. Resolve that UUID through
   `/api/v1/obo/entries/list`, validate organization/name/type/size, then save the
   actual provider path and authenticated link. A committed artifact is not
   uploaded again, even if another artifact is still pending.

An uncertain response is reconciled before another mutation. `receiving` and
`cleanup_pending` require later reconciliation. `cancelled` and `expired` cannot
publish; they must not be silently restarted under a fresh identity. Repeating
an unchanged logical commit does not create another version. These guarantees
come from Briefcase operation idempotency, not from reusable IAM tokens.

`SB_RECORDING_MAX_BYTES` defaults to 512 MiB per artifact; the reusable adapter
retains its separate 64 MiB default. Staging/hash memory is bounded. HTTP and job
deadlines remain bounded. The retired 60-second request-proof upload limit no
longer defines the transfer deadline; Briefcase's reservation deadline and size,
permission and quota checks still apply.

Transient failures use bounded backoff. Authorization waits do not consume the
transfer retry budget. `browser recording send SESSION_ID` retries eligible
exhausted work for its original owner while preserving completed receipts and
bound bytes. `browser recording rm` hides locally and stops new delivery work;
it does not delete or trash the remote file. Browser requests no cancellation,
read, sharing, or remote deletion scope.

## Validation and rollout

Local fixtures cover control/header isolation, exact staged bytes, immutable
operation IDs, response binding, receipt validation and denied/retired credentials.
Run the focused adapter checks without live services:

```sh
cargo test -p silicon-browser-backend providers::briefcase::tests --lib
cargo test -p silicon-browser-backend recording_delivery::tests --lib
```

The previous `briefcase_upload --proof-only` example was removed because its
single-use proof exchange and raw upload are retired. For a future hosted test,
use the actual CLI authorization flow, create/stop a Carbon and Silicon session,
and inspect both recording artifacts through the selected recipient account.
[The migration checklist](IAM_5_MIGRATION.md) records the remaining release gate.

Sources: [IAM migration](https://docs.iam.teamofsilicons.com/migrating-to-iam-5/),
[Briefcase OBO](https://docs.briefcase.teamofsilicons.com/obo/),
[delegated upload contract](https://docs.briefcase.teamofsilicons.com/api/delegated-uploads/),
[OpenAPI](https://docs.briefcase.teamofsilicons.com/openapi.yaml).
