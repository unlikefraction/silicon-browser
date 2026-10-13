# Frontend migration to Next.js and native Silicon UI

Verification date: 2026-10-13 (IST).

This migration replaces the Solid/Vite frontend with Next.js 16.3.6
and React 19.3.0. Next.js builds a Pages Router static export; nginx serves the
export on the existing AWS host. The browser continues to call the dedicated
Browser API directly. No Next.js server or API proxy is required.

## Implementation

The application imports native React components from the
[official Silicon UI registry](https://ui.teamofsilicons.com). All 53 installed
source files from 26 locked registry items were verified byte for byte against
their SHA-256-pinned registry responses. The former Solid-to-React button
adapter is removed.

The API client, saved-session coordination, recording helpers and API types
are unchanged from the preceding release. Authentication helpers have only
TypeScript annotation changes. The React workspace preserves single-flight
mutations, account-switch guards, refresh rotation, sign-out, live invitation
redemption and recording polling.

A synchronous external bootstrap script removes callback credentials and live
grants from the address bar before Next.js starts. Callback pages do not
restore saved workspace sessions. The export retains the strict script policy,
safe external links and live iframe restrictions. Next.js telemetry is disabled
during development and builds with `NEXT_TELEMETRY_DISABLED=1`.

## Pre-deployment validation

- All 49 unit and contract tests passed, covering API authorization, one-use
  refresh and callback handling, account storage, recording status, static
  routes, headers and URL cleanup.
- Strict TypeScript checking, the Next.js production export, documentation
  publication and export checks passed. Export checks reject executable inline
  scripts, inline event handlers, JavaScript URLs and source maps.
- All 38 browser scenarios passed in headless Chrome against an isolated build
  and mocked API under the production headers. They cover both sign-in paths,
  callback and live-grant cleanup, account switching, every workspace screen,
  mutation request bodies, recording polling, theme, navigation and 404s.
- The browser runs captured 178 screenshots at 1440, 1024, 768 and 390 pixels
  in light and dark themes. These checks detected no horizontal page scrolling,
  Content Security Policy violations, uncaught page errors or unexpected
  console errors.
- The registry check passed for all 53 files from 26 official registry items.

Local evidence is retained under `target/frontend-next-2026-10-13/`, including
`functional-audit.json`, unit and registry logs, and the browser suite's
`e2e/validation-receipt.json`, source snapshots, logs and screenshots.

## Deployment evidence and limits

The [frontend release](https://github.com/unlikefraction/silicon-browser/releases/tag/frontend-v0.2.0)
records the final source revision, archive and file hashes, production release
identifier, public route and asset verification, and rollback target after
live verification. The prior frontend release and its hashed assets are
retained so already-open tabs can continue loading their original assets.

The browser suite uses synthetic credentials and a mocked API. It does not
establish live hosted Accounts sign-in, provider session creation or recording
delivery. This frontend migration does not republish or change the Browser
3.0.0 native API, CLI, Rust crates or npm packages. See the
[Browser 3.0.0 release record](browser-3.0.0-2026-10-12.md) for their evidence.
