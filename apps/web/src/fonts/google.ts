// Google Fonts as a font source, in the browser: for each catalog family the
// worker planned, fetch its `css2` stylesheet, keep the per-script chunks whose `unicode-range` holds a character the
// deck draws, fetch those WOFF2 files and hand them to the worker, which subsets them. Requests carry font names and
// chunk URLs only: a URL with `text=` (which would send the deck's characters) is never fetched.
import type { GooglePlan } from '@slidock/types';

import { CSS2_ENDPOINT, type Css2Face, isGstaticUrl, parseCss2 } from './css2';

/** Parallel downloads of chunk files. */
const PARALLEL = 6;
/** A chunk file larger than this is not read (Google's largest CJK chunks are a few hundred KB). */
export const MAX_CHUNK_BYTES = 8 << 20;

export type Fetch = (url: string, init: RequestInit) => Promise<Response>;

/** The request options of every Google fetch: no cookies, no referrer. */
export const GOOGLE_INIT: RequestInit = { credentials: 'omit', referrerPolicy: 'no-referrer', mode: 'cors' };
/** A Google request (its body included) that takes longer than this is given up. */
export const GOOGLE_TIMEOUT_MS = 15_000;

export class GoogleRefusal extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'GoogleRefusal';
  }
}

/** Refuses any stylesheet URL but a `css2` request without `text`. */
export function checkCss2Url(url: string): URL {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    throw new GoogleRefusal(`not a URL: ${url}`);
  }
  if (`${parsed.origin}${parsed.pathname}` !== CSS2_ENDPOINT) {
    throw new GoogleRefusal(`not a css2 request: ${url}`);
  }
  if (parsed.searchParams.has('text')) {
    throw new GoogleRefusal('a css2 request with text= would send the deck’s characters');
  }
  return parsed;
}

/** A chunk file to add: its bytes and the part of its face it holds. */
export interface GoogleFile {
  family: string;
  url: string;
  bytes: ArrayBuffer;
  /** `null` for a whole face. */
  unicodeRange: string | null;
}

export interface GoogleProblem {
  family: string;
  message: string;
}

export interface GoogleResult {
  /** Families whose stylesheet was read. */
  families: string[];
  files: number;
  problems: GoogleProblem[];
}

export interface GoogleFetchOptions {
  fetch: Fetch;
  /** The code points the deck draws, sorted. */
  codepoints: readonly number[];
  /** Hands a file to the worker; may reject (the worker could not read it). */
  add(file: GoogleFile): Promise<void>;
  /** Stops before the next request once it returns true (the toggle went off, the deck closed). */
  stopped?(): boolean;
  /** Per request; `GOOGLE_TIMEOUT_MS` by default. */
  timeoutMs?: number;
}

/** The chunks worth fetching: those holding a drawn character, each URL once (a variable file serves many weights). */
export function chunksToFetch(faces: readonly Css2Face[], codepoints: readonly number[]): Css2Face[] {
  const seen = new Set<string>();
  return faces.filter((face) => {
    if (seen.has(face.url) || (face.unicodeRange !== null && !face.unicodeRange.intersects(codepoints))) {
      return false;
    }
    seen.add(face.url);
    return true;
  });
}

async function readBytes(response: Response, limit: number): Promise<ArrayBuffer> {
  const declared = Number(response.headers.get('content-length') ?? '0');
  if (declared > limit) {
    throw new GoogleRefusal(`the file is larger than ${limit} bytes`);
  }
  const bytes = await response.arrayBuffer();
  if (bytes.byteLength > limit) {
    throw new GoogleRefusal(`the file is larger than ${limit} bytes`);
  }
  return bytes;
}

function describe(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** Runs `tasks` at most `limit` at a time. */
async function pool(tasks: (() => Promise<void>)[], limit: number): Promise<void> {
  let next = 0;
  const run = async () => {
    while (next < tasks.length) {
      const task = tasks[next++];
      await task?.();
    }
  };
  await Promise.all(Array.from({ length: Math.min(limit, tasks.length) }, run));
}

/** Fetches `url` and reads its body with `read`, giving up (and aborting the request) after `timeoutMs`. */
async function fetchWithin<T>(
  fetch: Fetch,
  url: string,
  timeoutMs: number,
  read: (response: Response) => Promise<T>,
): Promise<T> {
  const controller = new AbortController();
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => {
      controller.abort();
      reject(new GoogleRefusal(`no answer within ${timeoutMs / 1000} s`));
    }, timeoutMs);
  });
  try {
    return await Promise.race([fetch(url, { ...GOOGLE_INIT, signal: controller.signal }).then(read), timeout]);
  } finally {
    clearTimeout(timer);
  }
}

/** Fetches every plan's chunks and adds them; a family that fails is reported and the others go on. */
export async function fetchGoogleFonts(
  plans: readonly GooglePlan[],
  options: GoogleFetchOptions,
): Promise<GoogleResult> {
  const result: GoogleResult = { families: [], files: 0, problems: [] };
  const stopped = options.stopped ?? (() => false);
  const timeoutMs = options.timeoutMs ?? GOOGLE_TIMEOUT_MS;
  const fail = (family: string, error: unknown) => result.problems.push({ family, message: describe(error) });
  for (const plan of plans) {
    if (stopped()) {
      break;
    }
    let faces: Css2Face[];
    try {
      const url = checkCss2Url(plan.url);
      const css = await fetchWithin(options.fetch, url.href, timeoutMs, (response) => {
        if (!response.ok) {
          throw new GoogleRefusal(`Google Fonts answered HTTP ${response.status}`);
        }
        return response.text();
      });
      faces = parseCss2(css);
    } catch (error) {
      fail(plan.family, error);
      continue;
    }
    result.families.push(plan.family);
    const tasks = chunksToFetch(faces, options.codepoints).map((face) => async () => {
      if (stopped()) {
        return;
      }
      try {
        if (!isGstaticUrl(face.url)) {
          throw new GoogleRefusal(`not a Google Fonts file: ${face.url}`);
        }
        const bytes = await fetchWithin(options.fetch, face.url, timeoutMs, (response) => {
          if (!response.ok) {
            throw new GoogleRefusal(`fonts.gstatic.com answered HTTP ${response.status}`);
          }
          return readBytes(response, MAX_CHUNK_BYTES);
        });
        if (stopped()) {
          return;
        }
        await options.add({
          family: plan.family,
          url: face.url,
          bytes,
          unicodeRange: face.unicodeRange?.toString() ?? null,
        });
        result.files += 1;
      } catch (error) {
        fail(plan.family, error);
      }
    });
    await pool(tasks, PARALLEL);
  }
  return result;
}
