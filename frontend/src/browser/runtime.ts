// Page-level singletons, created once when the client-only app first loads (never during the static export).
// This mirrors the module scope of the former src/main.tsx line for line.
import { BrowserApi } from '@/lib/api';
import { readEntry } from '@/lib/auth';
import { takeEntryHref } from '@/lib/entry';
import { BrowserSession } from '@/lib/session';

/** The callback, live invitation, or entry error parsed from the URL this page was opened with. */
export const entry = readEntry(new URL(takeEntryHref()));
// public/boot.js already removed one-use credentials and live grants; repeat it in case the page was opened another way.
if (location.hash || location.search) history.replaceState(null, '', entry.cleanPath);

export const savedSession = new BrowserSession(process.env.SB_BACKEND_ORIGIN);
export const productionApi = new BrowserApi(
  process.env.SB_BACKEND_ORIGIN,
  undefined,
  (value, previous) => savedSession.save(value, previous),
  (session, exchange) => savedSession.refresh(session, exchange),
);
// The popup only hands off its one-use code; it must not restore the opener's session.
export const restored = entry.callback ? null : savedSession.load();
if (restored) productionApi.setSession(restored, false);
