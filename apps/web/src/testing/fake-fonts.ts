import type { FontsEnvironment } from '../fonts/controller';
import type { LfaPermission, LocalFontData, LocalFontsApi } from '../fonts/localFonts';

/** Storage in memory, for the Google Fonts switch. */
export function memoryStorage(): Pick<Storage, 'getItem' | 'setItem' | 'removeItem'> & { values: Map<string, string> } {
  const values = new Map<string, string>();
  return {
    values,
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => void values.set(key, value),
    removeItem: (key) => void values.delete(key),
  };
}

/** A network that answers nothing: every request fails, as offline. Tests that fetch pass their own. */
export const offlineFetch: FontsEnvironment['fetch'] = (url) => Promise.reject(new TypeError(`offline: ${url}`));

/** The fonts environment of a test: offline, no Local Font Access, storage in memory, unless overridden. */
export function testFontsEnvironment(overrides: Partial<FontsEnvironment> = {}): FontsEnvironment {
  return { fetch: offlineFetch, localFonts: null, base: '/', storage: memoryStorage(), ...overrides };
}

/** The first bytes of a TrueType file: enough for a fake worker that only sniffs. */
export const TRUETYPE_HEAD = new Uint8Array([0, 1, 0, 0, 0, 10, 0, 128, 0, 3, 0, 32]);

/** An installed face for a fake Local Font Access. */
export function localFont(postscriptName: string, family: string, bytes: Uint8Array = TRUETYPE_HEAD): LocalFontData {
  return {
    postscriptName,
    fullName: postscriptName.replace(/-/g, ' '),
    family,
    style: 'Regular',
    blob: () => Promise.resolve(new Blob([bytes.slice()])),
  };
}

export interface FakeLocalFonts extends LocalFontsApi {
  /** Times `query` ran. */
  queries: number;
  state: Exclude<LfaPermission, 'unsupported'>;
}

/**
 * Local Font Access answering from `fonts`: `query` follows `state` (`prompt` asks `answer`, `denied` throws
 * `NotAllowedError`), or throws `failure` when given.
 */
export function fakeLocalFonts(
  fonts: readonly LocalFontData[],
  options: {
    state?: Exclude<LfaPermission, 'unsupported'>;
    answer?: 'granted' | 'denied';
    failure?: () => Error;
  } = {},
): FakeLocalFonts {
  const api: FakeLocalFonts = {
    queries: 0,
    state: options.state ?? 'prompt',
    permission: () => Promise.resolve(api.state),
    query() {
      api.queries += 1;
      if (options.failure) {
        return Promise.reject(options.failure());
      }
      if (api.state === 'prompt') {
        api.state = options.answer ?? 'granted';
      }
      if (api.state === 'denied') {
        return Promise.reject(new DOMException('The user denied the permission', 'NotAllowedError'));
      }
      return Promise.resolve(fonts);
    },
  };
  return api;
}
