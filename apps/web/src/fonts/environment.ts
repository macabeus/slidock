// The browser around the fonts flow: `fetch`, Local Font Access, the app's base URL and its storage.
import type { FontsEnvironment } from './controller';
import { browserLocalFonts } from './localFonts';

function storage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

export function browserFontsEnvironment(): FontsEnvironment {
  return {
    fetch: (url, init) => window.fetch(url, init),
    localFonts: browserLocalFonts(window),
    base: import.meta.env.BASE_URL,
    storage: storage(),
  };
}
