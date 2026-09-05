# Configured browser-provider capacity — 2026-09-06

The configured provider account currently reports **three concurrent sessions**, not 500. This is an operational capacity gap even though local metadata concurrency tests exercise 500 users and browser traffic bypasses the backend.

A read-only request to the official, documented `GET https://api.browser-use.com/api/v3/billing/account`, authenticated with the existing configured provider key, returned HTTP 200 and these fields:

```json
{
  "concurrentSessionLimit": 3,
  "rateLimit": 3,
  "isFreeTier": true,
  "planInfo": {
    "planName": "Unknown Plan Name",
    "subscriptionStatus": null
  }
}
```

The same configured key was used for the preceding production-session lookup. No credits were purchased, limits changed or new sessions created during this investigation. Account identity, balance, project/API-key identifiers and credentials are excluded from this report.

The supported [billing endpoint documentation](https://docs.browser-use.com/cloud/api-v3/billing/get-account-billing) exposes account information. Its current rendered schema documents `rateLimit` generically; the live response additionally supplies the unambiguous `concurrentSessionLimit` field above. No interpretation of `rateLimit` as requests per minute is assumed.

The live official [pricing page](https://browser-use.com/pricing), fetched directly rather than relying on an older search snippet, describes concurrency tiers based on lifetime payments net of refunds. Its published 500-session tier requires $5,000 of qualifying lifetime payments. These are advertised tiers, not proof that this account has that entitlement, and signup/granted credits do not count toward those payment thresholds.

The observed account limit of three is the current deployment constraint, irrespective of the public tier table. Supporting 500 simultaneous users of shared metadata/live views is different from supplying 500 separate remote browsers. The latter requires a provider-confirmed capacity increase or another provider allocation before it can be claimed. No purchase or spending-limit increase was authorized or performed by this check.

## Current capacity API

Authenticated clients can read `GET /api/v1/usage/limits` with their ordinary bearer token and `X-Org-ID`. The shared `UsageLimits` response is wrapped in the standard success envelope:

```json
{
  "data": {
    "concurrent_browser_limit": 3,
    "rate_limit": 3,
    "checked_at": "2026-09-05T20:00:00Z"
  }
}
```

This example is illustrative; the endpoint reads `concurrentSessionLimit` from the configured account on demand. A process-wide single-entry cache retains successful checks for 60 seconds and coalesces concurrent refreshes, so a later account upgrade appears on the next check after expiry. `checked_at` identifies the actual successful check and remains unchanged on cache hits. `rate_limit` is nullable, and its value implies no undocumented interval.

The capacity is shared by the service account, not allocated separately to each organization. The response exposes no balances, payment or plan details, account identifiers, or active-session counts across organizations. Responses use `Cache-Control: no-store`. A failed or timed-out check returns `503 usage_limits_unavailable` with a five-second retry delay; expired successful values are not returned as current. Each upstream check has a ten-second deadline and a 64 KiB response limit. This endpoint reports capacity; the provider still owns admission and entitlement enforcement.
