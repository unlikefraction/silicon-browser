# Browser Use: proxy metering reported for proxy-disabled incognito

Observed 2026-09-05 against the real Browser Use v3 service. The tested session was confirmed remotely stopped. Credentials, signed endpoint URLs, and session identifiers are omitted.

## Documented contract

Browser Use's create-browser documentation gives `proxyCountryCode` a US default and explicitly says: “Set to null to disable proxy.” A US default when the field is omitted therefore does not explain an explicitly null request. [Official create-browser documentation](https://docs.browser-use.com/cloud/api-v3/browsers/create-browser-session)

The browser lookup contract describes `proxyUsedMb` as proxy data usage and `proxyCost` as its USD cost. [Official get-browser documentation](https://docs.browser-use.com/cloud/api-v3/browsers/get-browser-session)

## Request and observation

The audit created an incognito session through `sb`, without a profile or selected proxy location. `StartBrowserBody.proxy_country_code` deliberately has no skip-if-none serialization attribute. Its HTTP fixture test asserts that an incognito create serializes `"proxyCountryCode": null`. This is source and captured mock-request evidence; the audit did not retain a raw production HTTP request capture.

A read-only provider GET after shutdown returned:

| Field | Exact observed value |
| --- | --- |
| `status` | `stopped` |
| `proxyUsedMb` | `1.1601905822753905812` |
| `proxyCost` | `0.00022659972310066222289062500` |
| `browserCost` | `0.0003333333333333333333333333333` |
| `startedAt` | `2026-09-05T08:59:45.741840Z` |
| `finishedAt` | `2026-09-05T09:00:43.660575Z` |

The observed duration is 57.918735 seconds. These counters conflict with the expected zero proxy metering for a proxy-disabled session. They do not independently prove which network path the browser used: this may be routing, metering, billing, or provider contract behavior. Actual exit-IP routing was not measured, and the upstream implementation is not established by the public schema.

## Local accounting defect fixed

Silicon Browser previously forced incognito proxy bytes to zero and its store rejected any nonzero incognito proxy counters, while still preserving the provider's proxy cost. That hid measured traffic and produced a cost/traffic discrepancy.

Accounting now preserves the provider's combined traffic counter for every session in `proxy_bytes_unclassified`, without inventing incoming/outgoing amounts. Nonzero incognito measurements also produce a server warning that the provider reported traffic despite a request to disable the proxy. Tests cover both store persistence and the complete incognito end/usage route using the observed decimal values.

With the existing decimal-MB conversion and rounding policy, the observed sample becomes 1,160,191 unclassified bytes, 227 proxy cost micro-units, and 560 total USD micro-units. This faithfully records the provider's report; it does not certify the provider's invoice or prove that proxy disabling worked. Previously finalized records whose bytes were discarded are not retroactively repaired by this change.

## Reproduction for the provider

Create one short-lived standalone browser with explicit `proxyCountryCode: null`, connect to its returned CDP endpoint, navigate a small public page, explicitly stop it, then compare the final `proxyUsedMb` and `proxyCost` against the documented disabled-proxy behavior. Retain a sanitized request capture and measure exit IP if routing verification is needed. No additional paid session was created for this investigation.

Until this discrepancy is resolved, the product's intended “incognito without proxy” behavior is requested from the provider but is not verified end to end. The local accounting correction must not be presented as resolving that upstream behavior.

## Automatic-delivery walkthrough reproduced the discrepancy

Two additional explicitly proxy-disabled incognito sessions on 2026-09-05 also returned nonzero counters at end. Carbon: 152,328 unclassified proxy bytes and 30 USD micro-units of proxy cost. Silicon: 40,775 bytes and 8 micro-units. Both remote sessions stopped; their native recordings were delivered and verified. This reproduces the metering discrepancy without establishing the actual network route. See [the live walkthrough](AUTOMATIC_RECORDING_LIVE_RETEST.md).
