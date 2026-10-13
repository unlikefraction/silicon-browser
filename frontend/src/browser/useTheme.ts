import { useEffect, useSyncExternalStore } from 'react';
import { THEME_KEY, applyTheme, readThemePreference, saveThemePreference, watchSystemTheme, type ThemePreference } from '@/lib/theme';

// One preference per page, shared by every component that shows the theme switch.
let preference: ThemePreference | null = null;
const listeners = new Set<() => void>();
const current = () => preference ??= readThemePreference();
const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };

function setThemePreference(next: ThemePreference) {
  saveThemePreference(next);
  preference = next;
  applyTheme(next);
  for (const listener of listeners) listener();
}

/** The viewer's theme preference (light, dark, or system) and a setter that stores and applies it. */
export function useTheme() {
  const value = useSyncExternalStore(subscribe, current, () => 'system' as const);
  useEffect(() => {
    if (value !== 'system') return;
    return watchSystemTheme(() => applyTheme('system'));
  }, [value]);
  useEffect(() => {
    // Another tab changed the stored choice.
    const onStorage = (event: StorageEvent) => {
      if (event.key !== THEME_KEY && event.key !== null) return;
      preference = readThemePreference();
      applyTheme(preference);
      for (const listener of listeners) listener();
    };
    window.addEventListener('storage', onStorage);
    return () => window.removeEventListener('storage', onStorage);
  }, []);
  return { preference: value, setPreference: setThemePreference };
}
