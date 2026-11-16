import { type ReactNode, createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';

import { focusRing } from './components/Button';
import { Icon } from './components/Icon';

/** The theme the user picked: follow the system, or force one. */
export type ThemeChoice = 'system' | 'light' | 'dark';

/** The theme in effect. */
export type ResolvedTheme = 'light' | 'dark';

const STORAGE_KEY = 'slidock.theme';
const LIGHT_QUERY = '(prefers-color-scheme: light)';

function readChoice(): ThemeChoice {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    return stored === 'light' || stored === 'dark' ? stored : 'system';
  } catch {
    return 'system';
  }
}

function writeChoice(choice: ThemeChoice) {
  try {
    if (choice === 'system') {
      localStorage.removeItem(STORAGE_KEY);
    } else {
      localStorage.setItem(STORAGE_KEY, choice);
    }
  } catch {
    // Storage can be unavailable (private windows, blocked site data); the choice then lasts for this page only.
  }
}

function systemTheme(): ResolvedTheme {
  return typeof matchMedia === 'function' && matchMedia(LIGHT_QUERY).matches ? 'light' : 'dark';
}

/** Sets `data-theme` on <html> for a forced theme and removes it for `system`, which the tokens' media query follows. */
export function applyTheme(choice: ThemeChoice, root: HTMLElement = document.documentElement) {
  if (choice === 'system') {
    root.removeAttribute('data-theme');
  } else {
    root.setAttribute('data-theme', choice);
  }
}

interface ThemeContextValue {
  choice: ThemeChoice;
  resolved: ResolvedTheme;
  setChoice(choice: ThemeChoice): void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [choice, setChoiceState] = useState<ThemeChoice>(readChoice);
  const [system, setSystem] = useState<ResolvedTheme>(systemTheme);

  useEffect(() => {
    applyTheme(choice);
  }, [choice]);

  useEffect(() => {
    if (typeof matchMedia !== 'function') {
      return;
    }
    const query = matchMedia(LIGHT_QUERY);
    const onChange = () => setSystem(query.matches ? 'light' : 'dark');
    query.addEventListener('change', onChange);
    return () => query.removeEventListener('change', onChange);
  }, []);

  const setChoice = useCallback((next: ThemeChoice) => {
    writeChoice(next);
    setChoiceState(next);
  }, []);

  const value = useMemo(
    () => ({ choice, resolved: choice === 'system' ? system : choice, setChoice }),
    [choice, system, setChoice],
  );
  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeContextValue {
  const value = useContext(ThemeContext);
  if (!value) {
    throw new Error('useTheme needs a <ThemeProvider>');
  }
  return value;
}

const NEXT: Record<ThemeChoice, ThemeChoice> = { system: 'light', light: 'dark', dark: 'system' };
const LABEL: Record<ThemeChoice, string> = { system: 'System theme', light: 'Light theme', dark: 'Dark theme' };
const ICON = { system: 'desktop', light: 'sun', dark: 'moon' } as const;

/** One button that cycles system → light → dark; its label names the theme in effect and the next one. */
export function ThemeToggle({ className = '' }: { className?: string }) {
  const { choice, setChoice } = useTheme();
  const next = NEXT[choice];
  return (
    <button
      type="button"
      onClick={() => setChoice(next)}
      className={`inline-flex size-9 items-center justify-center rounded-md text-muted transition-colors hover:bg-elevated hover:text-fg ${focusRing} ${className}`}
      aria-label={`${LABEL[choice]}. Switch to ${LABEL[next].toLowerCase()}`}
      title={`${LABEL[choice]} (switch to ${LABEL[next].toLowerCase()})`}
    >
      <Icon name={ICON[choice]} className="size-5" />
    </button>
  );
}
