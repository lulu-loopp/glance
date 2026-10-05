import type { ViewPrefs } from './types';

const dark = matchMedia('(prefers-color-scheme: dark)');

/**
 * The theme the preference stands for now: a choice, or the system's. Following
 * the backdrop is settled once the backdrop is known (PanelView.dress); until
 * then, and where there is none, it is the system's.
 */
export function resolveTheme(preference: ViewPrefs['theme']): 'light' | 'dark' {
  if (preference === 'light' || preference === 'dark') return preference;
  return dark.matches ? 'dark' : 'light';
}

/** Calls `change` whenever the system switches between light and dark. */
export function onSystemTheme(change: () => void) {
  dark.addEventListener('change', change);
}
