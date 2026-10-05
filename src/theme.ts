import type { ViewPrefs } from './types';

const dark = matchMedia('(prefers-color-scheme: dark)');

/** The theme the preference stands for now: a choice, or the system's. */
export function resolveTheme(preference: ViewPrefs['theme']): 'light' | 'dark' {
  if (preference !== 'system') return preference;
  return dark.matches ? 'dark' : 'light';
}

/** Calls `change` whenever the system switches between light and dark. */
export function onSystemTheme(change: () => void) {
  dark.addEventListener('change', change);
}
