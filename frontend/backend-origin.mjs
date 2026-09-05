export const DEFAULT_BACKEND_ORIGIN = 'https://backend.browser.teamofsilicons.com';

export function backendOrigin(value = DEFAULT_BACKEND_ORIGIN) {
  const parsed = new URL(value);
  if (parsed.username || parsed.password || parsed.search || parsed.hash || parsed.pathname !== '/') {
    throw new Error('Backend URL must be an origin without credentials, path, query, or fragment.');
  }
  // These two deployment targets also match the checked-in Content Security Policy.
  if (parsed.origin !== DEFAULT_BACKEND_ORIGIN && parsed.origin !== 'http://127.0.0.1:8091') {
    throw new Error('Backend origin must be the dedicated production backend or http://127.0.0.1:8091 for local testing.');
  }
  return parsed.origin;
}
