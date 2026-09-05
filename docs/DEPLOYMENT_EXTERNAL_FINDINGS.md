# Deployment integration findings — 2026-09-06

These are local operational findings, not messages posted to upstream projects. No credentials, signed capabilities or verification codes are included.

## IAM pending webhook URL replacement

The verified application `tos>browser` had a pending destination at `/webhooks/iam`. Re-proposing that same pending URL with a new signing secret returned `webhook_url_conflict` (request `01a0730b-6cb3-7980-a0ae-d2f59d54dd15`). It did not behave as an idempotent update to the same application's proposal.

Browser serves both `/webhooks/iam` and `/webhooks/iam/`. A proposal using the trailing-slash destination succeeded, resulting in pending version 3 and secret version 3. Activation remains IAM's separate `application.webhook.approve` step-up operation; proposing the alternate URL does not bypass that approval.

## Vercel CLI domain access reporting

Authenticated Vercel CLI 59.11.7 commands `domains add`, `alias set` and `inspect` reported insufficient ownership/access for the existing Silicon Browser project. The authenticated project-domain, verification and alias APIs succeeded for the same account/project. The public custom domain subsequently returned the expected production deployment and valid TLS.

This was worked around using the supported APIs, retaining the pre-existing `_vercel` TXT record for another service and adding a separate verification value for Browser. No unrelated domain was removed.

## ACM CAA inheritance during DNS migration

The frontend's previous Vercel CNAME inherited Vercel's CAA set. It did not authorize Amazon, so ACM requests for the nested backend hostname failed with `CAA_ERROR`. The frontend now uses Vercel's recommended A record; the backend uses its own ALB CNAME. After recursive DNS caches expired, a fresh DNS-validated ACM request was issued and attached to the backend HTTPS listener.

This was a DNS/certificate configuration problem, not a provider outage. Do not add CAA records alongside an existing CNAME at the same name.
