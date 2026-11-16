// The theme toggle's rule, the one the web app's `ThemeToggle` (`@slidock/ui`) follows: one button cycles system →
// light → dark, the choice is remembered in `localStorage` under `slidock.theme` (system removes it), and a forced
// theme is `data-theme` on <html>, which the presenter window's tokens follow, as the system preference when unset.

export type ThemeChoice = 'system' | 'light' | 'dark';

export const THEME_STORAGE_KEY = 'slidock.theme';

export const NEXT_THEME: Readonly<Record<ThemeChoice, ThemeChoice>> = {
  system: 'light',
  light: 'dark',
  dark: 'system',
};

export const THEME_LABEL: Readonly<Record<ThemeChoice, string>> = {
  system: 'System theme',
  light: 'Light theme',
  dark: 'Dark theme',
};

/** The remembered choice; `system` when there is none or storage is unavailable. */
export function readTheme(view: Window): ThemeChoice {
  try {
    const stored = view.localStorage.getItem(THEME_STORAGE_KEY);
    return stored === 'light' || stored === 'dark' ? stored : 'system';
  } catch {
    return 'system';
  }
}

/** Remembers `choice` (for this page only when storage is unavailable) and applies it to <html>. */
export function writeTheme(view: Window, choice: ThemeChoice): void {
  try {
    if (choice === 'system') {
      view.localStorage.removeItem(THEME_STORAGE_KEY);
    } else {
      view.localStorage.setItem(THEME_STORAGE_KEY, choice);
    }
  } catch {
    // Private windows and blocked site data have no storage.
  }
  applyTheme(view.document.documentElement, choice);
}

export function applyTheme(root: HTMLElement, choice: ThemeChoice): void {
  if (choice === 'system') {
    root.removeAttribute('data-theme');
  } else {
    root.setAttribute('data-theme', choice);
  }
}

/** The toggle's accessible name and its tooltip: the theme in effect and the next one. */
export function themeLabels(choice: ThemeChoice): { label: string; title: string } {
  const next = THEME_LABEL[NEXT_THEME[choice]].toLowerCase();
  return { label: `${THEME_LABEL[choice]}. Switch to ${next}`, title: `${THEME_LABEL[choice]} (switch to ${next})` };
}
