// The viewer's installed fonts through Local Font Access: Chromium only, asked only from
// a click (or on open when the permission is already granted), never required. The worker decides which installed
// faces are worth reading from their names; only those files are read, and their bytes never leave the tab.
import type { LocalFontMeta, LocalPick } from '@slidock/types';

/** At most this many installed files are read per session, and this many bytes. */
export const MAX_LOCAL_FILES = 48;
export const MAX_LOCAL_BYTES = 96 << 20;
/** A file larger than this is read only when its PostScript name is one the deck asks for. */
export const MAX_LOCAL_FILE_BYTES = 48 << 20;

/** The browser's answer about the permission: `unsupported` when it has no Local Font Access. */
export type LfaPermission = 'unsupported' | 'prompt' | 'granted' | 'denied';

/** `FontData`: an installed face's names and its file. */
export interface LocalFontData {
  readonly postscriptName: string;
  readonly fullName: string;
  readonly family: string;
  readonly style: string;
  blob(): Promise<Blob>;
}

/** Local Font Access as the app uses it, so tests can stand in for the browser. */
export interface LocalFontsApi {
  /** `window.queryLocalFonts()`: prompts when the permission is `prompt`, so it must run in a click handler then. */
  query(): Promise<readonly LocalFontData[]>;
  /** The `local-fonts` permission's state; `prompt` when the browser cannot say. */
  permission(): Promise<Exclude<LfaPermission, 'unsupported'>>;
}

interface LocalFontsWindow {
  queryLocalFonts?: () => Promise<LocalFontData[]>;
}

/** The browser's Local Font Access, or `null` where it has none (Firefox, Safari, an insecure context). */
export function browserLocalFonts(win: Window = window): LocalFontsApi | null {
  const query = (win as Window & LocalFontsWindow).queryLocalFonts;
  if (typeof query !== 'function') {
    return null;
  }
  return {
    query: () => query.call(win),
    async permission() {
      try {
        const status = await win.navigator.permissions.query({ name: 'local-fonts' as PermissionName });
        return status.state;
      } catch {
        return 'prompt';
      }
    },
  };
}

/** Why reading installed fonts stopped. */
export type LfaFailure =
  /** The user (or a policy) refused the permission. */
  | { kind: 'denied' }
  /** The browser wants a click first (no transient activation), even with the permission granted. */
  | { kind: 'activation' }
  | { kind: 'failed'; message: string };

export function lfaFailure(error: unknown): LfaFailure {
  const name = error instanceof Error || error instanceof DOMException ? error.name : '';
  if (name === 'NotAllowedError') {
    return { kind: 'denied' };
  }
  if (name === 'SecurityError') {
    return { kind: 'activation' };
  }
  return { kind: 'failed', message: error instanceof Error ? error.message : String(error) };
}

/** A picked installed file that was not added, and why. */
export interface SkippedFile {
  /** The face's full name, as the browser lists it. */
  name: string;
  reason: string;
}

/** A size in MiB, as the panel prints it. */
function mib(bytes: number): string {
  const value = bytes / (1 << 20);
  return `${value >= 10 ? Math.round(value) : Math.round(value * 10) / 10} MB`;
}

/** What one read of the installed fonts did. */
export interface LocalReadResult {
  /** Installed faces the browser listed. */
  listed: number;
  /** Faces the worker picked. */
  picked: number;
  /** Files read and added. */
  added: number;
  /** Picked files an earlier read of this session already added. */
  known: number;
  /** Picked files not read (over the caps) or that the worker could not use. */
  skipped: number;
  /** The picked files not added, with why, for those the browser named (`skipped` counts them all). */
  skippedFiles: SkippedFile[];
}

/** The installed files one deck has read so far: the caps hold for the whole session, and a file is read once. */
export class LocalBudget {
  files = 0;
  bytes = 0;
  /** PostScript names of the files read. */
  readonly read = new Set<string>();
}

export interface LocalReadOptions {
  /** `null` when the worker matches no installed font. */
  select(fonts: LocalFontMeta[]): Promise<LocalPick[] | null>;
  /** Adds one file; may reject (the worker could not use it). */
  add(file: Blob, pick: LocalPick): Promise<void>;
  budget: LocalBudget;
}

/** The worker cannot match installed fonts: it answered `selectLocal` with no `localPicks`. */
export class NoLocalMatching extends Error {
  constructor() {
    super('This version of the converter cannot pick installed fonts');
    this.name = 'NoLocalMatching';
  }
}

/**
 * Lists the installed faces, lets the worker pick, reads the picked files within the session's caps and adds them.
 * A file read before in the session is not read again.
 */
export async function readLocalFonts(api: LocalFontsApi, options: LocalReadOptions): Promise<LocalReadResult> {
  const all = await api.query();
  const metas: LocalFontMeta[] = all.map(({ postscriptName, fullName, family, style }) => ({
    postscriptName,
    fullName,
    family,
    style,
  }));
  const picks = metas.length === 0 ? [] : await options.select(metas);
  if (picks === null) {
    throw new NoLocalMatching();
  }
  const { budget } = options;
  const result: LocalReadResult = {
    listed: all.length,
    picked: picks.length,
    added: 0,
    known: 0,
    skipped: 0,
    skippedFiles: [],
  };
  const skip = (font: LocalFontData, reason: string) => {
    result.skipped += 1;
    result.skippedFiles.push({ name: font.fullName || font.postscriptName, reason });
  };
  const seen = new Set<number>();
  for (const pick of picks) {
    const font = all[pick.index];
    if (font === undefined || seen.has(pick.index)) {
      result.skipped += 1;
      continue;
    }
    seen.add(pick.index);
    if (budget.read.has(font.postscriptName)) {
      result.known += 1;
      continue;
    }
    if (budget.files >= MAX_LOCAL_FILES) {
      skip(font, `over the ${MAX_LOCAL_FILES} installed files read for one presentation`);
      continue;
    }
    let blob: Blob;
    try {
      blob = await font.blob();
    } catch (error) {
      skip(font, `the browser could not read the file: ${error instanceof Error ? error.message : String(error)}`);
      continue;
    }
    if (blob.size > MAX_LOCAL_FILE_BYTES && !pick.exactPostscript) {
      skip(font, `its file is ${mib(blob.size)}, over the ${mib(MAX_LOCAL_FILE_BYTES)} a font file may have`);
      continue;
    }
    if (budget.bytes + blob.size > MAX_LOCAL_BYTES) {
      const room = `the ${mib(MAX_LOCAL_BYTES)} of installed fonts one presentation may read`;
      const left = budget.bytes === 0 ? room : `the ${mib(MAX_LOCAL_BYTES - budget.bytes)} left of ${room}`;
      skip(font, `its file is ${mib(blob.size)}, over ${left}`);
      continue;
    }
    budget.files += 1;
    budget.bytes += blob.size;
    budget.read.add(font.postscriptName);
    try {
      await options.add(blob, pick);
      result.added += 1;
    } catch (error) {
      skip(font, error instanceof Error ? error.message : String(error));
    }
  }
  return result;
}
