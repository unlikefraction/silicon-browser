# Briefcase 300 MiB native CLI retest — 2026-09-05

Read-only checks used the installed Briefcase 0.1.3 CLI, isolated private CLI state, explicit actor profiles, and paired Briefcase environment `01a07249-f2ad-7922-87bc-ca984d0d54cc`. Commands ran sequentially. No file upload, modification, trash, restore, or root-key change was performed by this subtask.

Target entry: `01a0726c-2830-7bf0-b254-1947c313714e`.

| Check | Observed result |
| --- | --- |
| Carbon `stat` | Exit 0; private file `browser-audit-300mib-77b8a2ae.bin`, **314,572,800 bytes = 300 MiB**, owner `sbauditfive`, originating application `tos>browser`. |
| Carbon `versions` | Exit 0; **exactly one retained version**, number 1, matching 314,572,800-byte size. Version ID `01a0726c-290c-7e02-834b-3b8727b8bae7`. |
| Carbon `history` | Exit 0; includes `entry.file_created.v1` attributed to Carbon `sbauditfive` and application `tos>browser`, plus subsequent owner content/download/metadata reads. |
| Carbon `usage` | Exit 0; storage **314,582,364 bytes used**, **2,147,483,648 bytes limit (2 GiB)**, **1,832,901,284 bytes remaining**. Used plus remaining equals the limit; usage remains below the sandbox cap. |
| Silicon `stat` of the exact known UUID | Exit **3**, `not_found`; the Carbon's private file remains invisible to `browser-audit-reader:tos`. Request `01a0726c-afe9-7dd1-b0bc-fa62fc026c1b`. |

All five command exit statuses matched expectations. Explicit assertions also checked exact 300 MiB size, one retained version, and the storage accounting/2 GiB bound.

The stored path is `private/sbauditfive/apps/tos>browser/browser-audit-300mib-77b8a2ae.bin`. Carbon metadata disclosed read/update/delete/manage-permissions access. The file is binary (`application/octet-stream`), with render mode `unsupported`; this does not prevent downloads.

The main runner separately performs complete streamed SHA-256 readback and range checks. This report does not claim those results on the strength of metadata. It verifies a **300 MiB** object, not 300 GiB, and does not establish support for arbitrary slow transfers or completion of Browser's automatic recording-delivery worker.

Sanitized local evidence: `/tmp/sb-briefcase-audit/300mib-cli-results.json`. No credentials or roots are included in this report.
