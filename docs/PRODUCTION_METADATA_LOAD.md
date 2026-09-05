# Production metadata burst — 2026-09-05 19:54 UTC

A bounded read-only smoke/load check exercised the deployed HTTPS API at `backend.browser.teamofsilicons.com`. The workload used one legitimate Carbon in organization `tos`, with its authorization cache warmed. It did not create sessions, fetch CDP connections, execute browser actions, upload files or intentionally mutate application state.

This was **500 concurrent HTTPS requests from one principal and one client machine**, not 500 distinct users or 500 remote browsers. Each request used one of `/api/v1/me`, `/api/v1/sessions` and `/api/v1/usage`. The session/usage dataset was the small production walkthrough dataset, not a large historical organization.

## Observed result

Initial health and one request to each authenticated endpoint returned HTTP 200 with valid JSON envelopes. The concurrent burst then produced:

| Outcome | Count |
| --- | ---: |
| HTTP 200 | 498 |
| Client connection timeout | 1 |
| Pending request canceled by fail-fast policy | 1 |
| Observed HTTP 5xx | 0 |
| Invalid successful JSON envelopes | 0 |

The run stopped after the first transport failure at **10.20 seconds**. Across its 499 observed outcomes, p50 latency was **2.43 seconds**, p95 **7.96 seconds** and maximum **10.20 seconds**; that maximum is the timeout. Successful responses completed within approximately **8.24 seconds**. The following health check returned HTTP 200 in **224 ms**.

An initial attempt had already been aborted by the load generator's inherited 256-file-descriptor limit. Only the Python test process's limit was raised to 2,048 before the measured attempt. No operating-system-wide limits or production settings were changed. The repeat retained the 500-connection workload and used a 10-second connection timeout / 15-second overall timeout. No further 500-request repeats were run.

## AWS runtime observations

The deployed host was a `t4g.medium`. Read-only SSM checks before and after the workload found the same backend PID, service `active/running`, **zero automatic restarts**, eight threads and a 65,536 open-file limit. Process RSS changed from **21,536 KiB** to **21,836 KiB**. The service cgroup reported approximately 12 MiB current memory; this and process RSS are different accounting measures and neither is a sampled peak.

Backend cumulative CPU time increased from 0.468 to 0.829 seconds between the two audits. Those audits span more than the measured burst and can include background work; this is not a CPU-utilization or peak-CPU measurement. Kernel listen-overflow, listen-drop, backlog-drop and SYN-retransmission counters were all zero at the later observation.

`node`, `npm`, `chromium`, `chromium-browser`, `google-chrome` and `agent-browser` were absent from the checked system PATH. An exact process-name audit found no Node, Chrome/Chromium or agent-browser process. The AWS backend executes its native release binary, not a browser-controller runtime.

## Interpretation

The service stayed healthy and returned 498 successful responses, but **this was not a complete 500/500 pass**. A cold-connection tail of roughly eight seconds and one connection timeout remain a measured performance concern. The evidence does not isolate the failure to the application, ingress, client-side TLS scheduling or network path; it should not be reported as a proven upstream vendor bug or server crash.

The earlier local test covered 500 distinct synthetic identities with fake IAM/provider adapters and a real SQLite WAL database. That test and this production burst exercise different boundaries. Neither alone proves sustained production capacity for 500 simultaneous real users or a browser-provider concurrency quota. Credentials and response bodies were kept out of the report.
