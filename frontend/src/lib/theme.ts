// Per-viewer colour theme. `public/boot.js` applies the same rule before first paint; keep the two in step.
export type ThemePreference = 'light' | 'dark' | 'system';
export type Theme = 'light' | 'dark';

export const THEME_KEY = 'silicon-browser:theme';
const DARK_QUERY = '(prefers-color-scheme: dark)';

/** A stored `light` or `dark` choice; anything else (nothing stored, unreadable storage) follows the system. */
export function readThemePreference(): ThemePreference {
  try {
    const stored = localStorage.getItem(THEME_KEY);
    return stored === 'light' || stored === 'dark' ? stored : 'system';
  } catch { return 'system'; }
}

export function saveThemePreference(preference: ThemePreference) {
  try {
    if (preference === 'system') localStorage.removeItem(THEME_KEY);
    else localStorage.setItem(THEME_KEY, preference);
  } catch { /* Storage can be unavailable; the choice then lasts for this page only. */ }
}

export function resolveTheme(preference: ThemePreference): Theme {
  if (preference !== 'system') return preference;
  return typeof matchMedia === 'function' && matchMedia(DARK_QUERY).matches ? 'dark' : 'light';
}

export function applyTheme(preference: ThemePreference) {
  document.documentElement.dataset.theme = resolveTheme(preference);
}

/** Calls back when the system colour scheme changes. */
export function watchSystemTheme(onChange: () => void): () => void {
  if (typeof matchMedia !== 'function') return () => {};
  const query = matchMedia(DARK_QUERY);
  query.addEventListener('change', onChange);
  return () => query.removeEventListener('change', onChange);
}
