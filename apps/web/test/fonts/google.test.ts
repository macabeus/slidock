import type { GooglePlan } from '@slidock/types';
import { describe, expect, it } from 'vitest';

import {
  type Fetch,
  type GoogleFile,
  GoogleRefusal,
  MAX_CHUNK_BYTES,
  checkCss2Url,
  fetchGoogleFonts,
} from '../../src/fonts/google';
import { ROBOTO_PLAN } from '../../src/testing/sample-fonts';
import { answer, points, recorded } from './recorded';

const OSWALD_PLAN: GooglePlan = {
  family: 'Oswald',
  licence: 'OFL-1.1',
  faces: [
    { weight: 400, style: 'normal' },
    { weight: 700, style: 'normal' },
  ],
  url: 'https://fonts.googleapis.com/css2?family=Oswald:wght@400;700&display=block',
};

/** A network answering the recorded `css2` responses and a small body for each gstatic file, logging requests. */
function recordedNetwork(overrides: Record<string, () => Response> = {}) {
  const requests: { url: string; init: RequestInit }[] = [];
  let active = 0;
  let peak = 0;
  const fetch: Fetch = async (url, init) => {
    requests.push({ url, init });
    active += 1;
    peak = Math.max(peak, active);
    await new Promise((resolve) => setTimeout(resolve, 1));
    active -= 1;
    const override = overrides[url];
    if (override) {
      return override();
    }
    if (url === ROBOTO_PLAN.url) {
      return answer(recorded('roboto-browser'));
    }
    if (url === OSWALD_PLAN.url) {
      return answer(recorded('oswald-browser'));
    }
    if (url.startsWith('https://fonts.gstatic.com/')) {
      return answer(new TextEncoder().encode(`wOF2 ${url}`));
    }
    return answer('not found', { status: 404 });
  };
  return {
    fetch,
    requests,
    get peak() {
      return peak;
    },
  };
}

describe('checkCss2Url', () => {
  it('takes only a css2 request without text=', () => {
    expect(checkCss2Url(ROBOTO_PLAN.url).searchParams.get('family')).toBe('Roboto:ital,wght@0,400;0,700;1,400');
    expect(() => checkCss2Url(`${ROBOTO_PLAN.url}&text=Hello`)).toThrow(GoogleRefusal);
    expect(() => checkCss2Url('https://fonts.googleapis.com/css2?text=a&family=Roboto')).toThrow(/text=/);
    expect(() => checkCss2Url('https://fonts.googleapis.com/css?family=Roboto')).toThrow(/not a css2/);
    expect(() => checkCss2Url('https://evil.example/css2?family=Roboto')).toThrow(/not a css2/);
    expect(() => checkCss2Url('not a url')).toThrow(/not a URL/);
  });
});

describe('fetchGoogleFonts', () => {
  it('fetches only the chunks that hold a drawn character, never with text=, without cookies or referrer', async () => {
    const network = recordedNetwork();
    const added: GoogleFile[] = [];
    const result = await fetchGoogleFonts([ROBOTO_PLAN], {
      fetch: network.fetch,
      codepoints: points('Quarterly review 2026'),
      add: async (file) => void added.push(file),
    });
    expect(result).toEqual({ families: ['Roboto'], files: 2, problems: [] });
    // The latin chunks (the italic file, and the upright file 400 and 700 share): no Cyrillic, Greek, Vietnamese or
    // latin-ext chunk is fetched.
    expect(added.map((file) => file.unicodeRange?.startsWith('U+0000-00FF'))).toEqual([true, true]);
    expect(network.requests.map((request) => request.url)).toEqual([ROBOTO_PLAN.url, ...added.map((file) => file.url)]);
    for (const { url, init } of network.requests) {
      expect(new URL(url).searchParams.has('text')).toBe(false);
      expect(url).not.toContain('Quarterly');
      expect(init).toMatchObject({ credentials: 'omit', referrerPolicy: 'no-referrer' });
    }
  });

  it('fetches each chunk of every script the text draws, and a shared variable file once', async () => {
    const network = recordedNetwork();
    const added: GoogleFile[] = [];
    await fetchGoogleFonts([OSWALD_PLAN, ROBOTO_PLAN], {
      fetch: network.fetch,
      codepoints: points('Hello Привет Γειά'),
      add: async (file) => void added.push(file),
    });
    const oswald = added.filter((file) => file.family === 'Oswald');
    // Oswald serves 400 and 700 from one variable file per script: latin and cyrillic (it has no Greek).
    expect(oswald).toHaveLength(2);
    expect(new Set(oswald.map((file) => file.url)).size).toBe(2);
    // Roboto: the latin, cyrillic, greek and math (which holds Greek letters) chunks of the italic and the upright
    // file (400 and 700 share it).
    const roboto = added.filter((file) => file.family === 'Roboto');
    expect(new Set(roboto.map((file) => file.url)).size).toBe(roboto.length);
    expect(roboto).toHaveLength(8);
    expect(network.peak).toBeLessThanOrEqual(6);
  });

  it('refuses a plan URL with text= before any request', async () => {
    const network = recordedNetwork();
    const result = await fetchGoogleFonts([{ ...ROBOTO_PLAN, url: `${ROBOTO_PLAN.url}&text=Secret` }], {
      fetch: network.fetch,
      codepoints: points('Secret'),
      add: async () => {},
    });
    expect(network.requests).toEqual([]);
    expect(result.files).toBe(0);
    expect(result.problems[0]?.message).toMatch(/text=/);
  });

  it('gives up on a stylesheet or a chunk that never answers, aborting it, and goes on', async () => {
    const network = recordedNetwork();
    const aborted: string[] = [];
    let hungChunk: string | null = null;
    // Oswald's stylesheet and Roboto's first chunk never answer.
    const fetch: Fetch = (url, init) => {
      const chunk = url.startsWith('https://fonts.gstatic.com/');
      if (url === OSWALD_PLAN.url || (chunk && (hungChunk ??= url) === url)) {
        init.signal?.addEventListener('abort', () => aborted.push(url));
        return new Promise<Response>(() => {});
      }
      return network.fetch(url, init);
    };
    const result = await fetchGoogleFonts([OSWALD_PLAN, ROBOTO_PLAN], {
      fetch,
      codepoints: points('A'),
      add: async () => {},
      timeoutMs: 20,
    });
    expect(result.families).toEqual(['Roboto']);
    expect(result.files).toBe(1);
    expect(result.problems).toEqual([
      { family: 'Oswald', message: 'no answer within 0.02 s' },
      { family: 'Roboto', message: 'no answer within 0.02 s' },
    ]);
    expect(aborted).toEqual([OSWALD_PLAN.url, hungChunk]);
  });

  it('reports a family that fails and goes on with the next', async () => {
    const network = recordedNetwork({ [OSWALD_PLAN.url]: () => answer('<html>', { status: 400 }) });
    const result = await fetchGoogleFonts([OSWALD_PLAN, ROBOTO_PLAN], {
      fetch: network.fetch,
      codepoints: points('A'),
      add: async () => {},
    });
    expect(result.families).toEqual(['Roboto']);
    expect(result.files).toBe(2);
    expect(result.problems).toEqual([{ family: 'Oswald', message: 'Google Fonts answered HTTP 400' }]);
  });

  it('skips a chunk that is too large or that the worker refuses, keeping the others', async () => {
    const faces = recorded('roboto-browser');
    const latinUrls = [...faces.matchAll(/\/\* latin \*\/[^}]*url\(([^)]+)\)/g)].map((match) => match[1] ?? '');
    const network = recordedNetwork({
      [latinUrls[0] ?? '']: () =>
        answer(new Uint8Array(4), { headers: { 'content-length': String(MAX_CHUNK_BYTES + 1) } }),
    });
    let calls = 0;
    const result = await fetchGoogleFonts([ROBOTO_PLAN], {
      fetch: network.fetch,
      // Latin and latin-ext chunks of the italic and the upright file.
      codepoints: points('AĀ'),
      add: async () => {
        calls += 1;
        if (calls === 1) {
          throw new Error('not a font');
        }
      },
    });
    expect(result.files).toBe(2);
    expect(result.problems.map((problem) => problem.message).sort()).toEqual([
      'not a font',
      `the file is larger than ${MAX_CHUNK_BYTES} bytes`,
    ]);
  });

  it('stops before the next request once stopped', async () => {
    const network = recordedNetwork();
    let stop = false;
    const result = await fetchGoogleFonts([ROBOTO_PLAN, OSWALD_PLAN], {
      fetch: network.fetch,
      codepoints: points('A'),
      add: async () => {
        stop = true;
      },
      stopped: () => stop,
    });
    expect(network.requests.some((request) => request.url === OSWALD_PLAN.url)).toBe(false);
    expect(result.families).toEqual(['Roboto']);
  });
});
