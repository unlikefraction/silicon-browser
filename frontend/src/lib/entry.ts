declare global {
  interface Window { __sbEntry?: string }
}

/**
 * The URL this page was opened with, captured by `public/boot.js` before the secrets in it were removed from the address bar.
 * It can be taken once: later callers (and anything that inspects `window`) get the already-cleaned `location.href`.
 */
export function takeEntryHref(): string {
  const href = window.__sbEntry ?? location.href;
  delete window.__sbEntry;
  return href;
}
