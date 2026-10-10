# Recording integration status

Browser 0.4.0 uses Silicon Accounts. The previous IAM OBO authorization and
Briefcase delivery protocol are retired and are not mounted by the current API.
Browser sessions no longer require Briefcase credentials or OBO consent to start.

The Browser recording list retains session metadata and the remote provider's
native recording status. Local controller recording commands still save files
on the caller's machine. Automatic storage into Briefcase is not configured by
this release. Existing delivery receipts and historical database migrations are
retained; they must not be retried using a different account authority.

A future Briefcase integration must use Silicon Accounts User verification,
with the user's agreement in Browser and receiver-side proof verification.
An Accounts access token for Browser cannot be forwarded as Briefcase authority.
The [Accounts User verification contract](https://developers.teamofsilicons.com/docs/accounts/start/user-verification.md)
explains that boundary.

The previous integration design, pending migration changes and operational
notes are preserved in [the legacy document](legacy/BRIEFCASE_INTEGRATION-IAM.md).
They are historical records, not setup instructions for this release.
