// The fonts flow of one open deck: the sources in the user's order,
// each adding its files to the worker, then one re-finish that swaps the slides in place. Embedded fonts are read by
// the worker on open. Then the bundled substitutes as the floor, at once, and beside them: installed fonts when the
// permission is already granted (otherwise only from a click), then Google Fonts unless it is off; the user's files
// whenever they arrive. The worker ranks faces by source, so the order files arrive in does not change the deck.
import type {
  AddedFontSource,
  FontNeed,
  GooglePlan,
  LocalFontMeta,
  LocalPick,
  SlideTemplate,
  SubstituteFace,
} from '@slidock/types';

import { CORE_MANIFEST, type CoreManifest, loadFace, loadManifest, substituteFile } from './bundled';
import { deckCodepoints } from './codepoints';
import { type DroppedFile, MAX_FONT_FILE_BYTES, MAX_USER_BYTES, MAX_USER_FILES } from './dropped';
import { type Fetch, type GoogleResult, fetchGoogleFonts } from './google';
import {
  type LfaFailure,
  type LfaPermission,
  LocalBudget,
  type LocalFontsApi,
  type LocalReadResult,
  NoLocalMatching,
  lfaFailure,
  readLocalFonts,
} from './localFonts';

/** What the controller asks of the open deck's session. */
export interface FontsHost {
  /** Adds a font file (its bytes move to the worker); resolves with what is still missing, rejects for no font. */
  addFont(bytes: ArrayBuffer, source: AddedFontSource, unicodeRange: string | null): Promise<FontNeed[]>;
  /** The installed faces worth reading; `null` when the worker cannot match installed fonts. */
  selectLocal(fonts: LocalFontMeta[]): Promise<LocalPick[] | null>;
  /** Re-runs the passes from bind-fonts and swaps the deck in place, at the current slide. */
  refinish(needs: readonly FontNeed[]): Promise<void>;
  /** Opens the input again with no font added; resolves with its needs. */
  reopen(): Promise<FontNeed[]>;
  /** The slides shown now. */
  slides(): readonly SlideTemplate[];
  /** The bundled substitute faces the deck shown draws with no file yet. */
  substitutes(): readonly SubstituteFace[];
}

/** The browser around the controller, so tests can stand in for it. */
export interface FontsEnvironment {
  fetch: Fetch;
  /** `null` where the browser has no Local Font Access. */
  localFonts: LocalFontsApi | null;
  /** The app's base URL, ending in `/` (the substitutes are under it). */
  base: string;
  /** Remembers the Google Fonts switch; `null` when storage is unavailable. */
  storage: Pick<Storage, 'getItem' | 'setItem' | 'removeItem'> | null;
}

export const GOOGLE_PREFERENCE = 'slidock.googleFonts';

export type LocalStatus =
  | { state: 'idle' }
  | { state: 'reading' }
  | { state: 'done'; result: LocalReadResult }
  | { state: 'failed'; failure: LfaFailure };

export type GoogleStatus =
  | { state: 'idle' }
  | { state: 'fetching'; families: string[] }
  | { state: 'done'; result: GoogleResult }
  /** Turned off after its faces arrived: the deck opens again without them. */
  | { state: 'removing' };

export type SubstituteStatus =
  { state: 'idle' } | { state: 'loading' } | { state: 'done'; families: string[]; problems: string[] };

export interface FontsState {
  /** The faces no font provides yet, as of the last change. */
  needs: readonly FontNeed[];
  /** `null` until the browser answered. */
  permission: LfaPermission | null;
  /** Whether the worker picks installed faces by their names; `null` until it answered. */
  localMatching: boolean | null;
  local: LocalStatus;
  googleEnabled: boolean;
  google: GoogleStatus;
  files: readonly DroppedFile[];
  substitutes: SubstituteStatus;
  /** Families left out of the download. */
  leaveOut: readonly string[];
  /** Families whose licence restricts embedding them that the user includes in the download all the same. */
  include: readonly string[];
  /** The sources that need no click (`start`) have not all settled yet. */
  settling: boolean;
  /** A re-finish runs. */
  refreshing: boolean;
  /** The last re-finish failed. */
  refreshError: string | null;
}

interface KeptFile {
  /** The user's `File`, an installed face's file, or a substitute's bytes: read again when the deck reopens. */
  file: Blob;
  source: AddedFontSource;
}

function readPreference(storage: FontsEnvironment['storage']): boolean {
  try {
    return storage?.getItem(GOOGLE_PREFERENCE) !== 'off';
  } catch {
    return true;
  }
}

function writePreference(storage: FontsEnvironment['storage'], enabled: boolean) {
  try {
    if (enabled) {
      storage?.removeItem(GOOGLE_PREFERENCE);
    } else {
      storage?.setItem(GOOGLE_PREFERENCE, 'off');
    }
  } catch {
    // Storage unavailable: the switch holds for this tab only.
  }
}

function describe(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** The Google requests of `needs`, each URL once. */
export function googlePlans(needs: readonly FontNeed[]): GooglePlan[] {
  const plans = new Map<string, GooglePlan>();
  for (const need of needs) {
    if (need.google && need.faces.length > 0 && !plans.has(need.google.url)) {
      plans.set(need.google.url, need.google);
    }
  }
  return [...plans.values()];
}

/**
 * The fonts flow as an external store (`subscribe`/`getSnapshot`). Font files are added one request at a time in
 * the order the sources settle; each change ends with a re-finish, and changes that arrive while one runs share the
 * next.
 */
export class FontsController {
  readonly #host: FontsHost;
  readonly #env: FontsEnvironment;
  #state: FontsState;
  readonly #listeners = new Set<() => void>();
  /** Adds and reopens, in order. */
  #queue: Promise<void> = Promise.resolve();
  #refresh: Promise<void> | null = null;
  #dirty = false;
  #disposed = false;
  /** Every file added except Google's, to add again when the deck reopens. */
  #kept: KeptFile[] = [];
  #googleFiles = 0;
  /** Bumped when Google turns off: a fetch of an earlier round stops. */
  #googleRound = 0;
  #manifest: Promise<CoreManifest> | null = null;
  /** Substitute files added, by file name. */
  #substitutes = new Set<string>();
  /** Substitute loads, one after another. */
  #substituting: Promise<void> = Promise.resolve();
  /** The installed files read for this deck. */
  readonly #localBudget = new LocalBudget();
  /** The user's files taken for this deck. */
  #userFiles = 0;
  #userBytes = 0;

  constructor(host: FontsHost, env: FontsEnvironment, needs: readonly FontNeed[]) {
    this.#host = host;
    this.#env = env;
    this.#state = {
      needs,
      permission: env.localFonts ? null : 'unsupported',
      localMatching: null,
      local: { state: 'idle' },
      googleEnabled: readPreference(env.storage),
      google: { state: 'idle' },
      files: [],
      substitutes: { state: 'idle' },
      leaveOut: [],
      include: [],
      settling: true,
      refreshing: false,
      refreshError: null,
    };
  }

  subscribe = (listener: () => void) => {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  };

  getSnapshot = (): FontsState => this.#state;

  #set(patch: Partial<FontsState>) {
    if (this.#disposed) {
      return;
    }
    this.#state = { ...this.#state, ...patch };
    for (const listener of this.#listeners) {
      listener();
    }
  }

  /** Runs `task` after the queued ones; its failure does not stop later ones. */
  #enqueue<T>(task: () => Promise<T>): Promise<T> {
    const run = this.#queue.then(task);
    this.#queue = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  }

  /**
   * Adds a file in queue order, reading a `Blob` only when its turn comes (one file in memory at a time); resolves
   * when the worker took it, rejects when it did not.
   */
  #add(data: Blob | ArrayBuffer, source: AddedFontSource, unicodeRange: string | null): Promise<void> {
    // Google's files are not kept: turning Google off removes them, and on fetches them again.
    const kept = source === 'google' ? null : { file: data instanceof Blob ? data : new Blob([data]), source };
    const round = this.#googleRound;
    return this.#enqueue(async () => {
      // A Google file of a round that was turned off stays out of the reopened deck.
      if (this.#disposed || (kept === null && round !== this.#googleRound)) {
        return;
      }
      const bytes = data instanceof Blob ? await data.arrayBuffer() : data;
      const needs = await this.#host.addFont(bytes, source, unicodeRange);
      if (kept) {
        this.#kept.push(kept);
      } else {
        this.#googleFiles += 1;
      }
      this.#set({ needs });
    });
  }

  /** Re-finishes once the queued adds are done; changes that arrive meanwhile share the next re-finish. */
  #refinish(): Promise<void> {
    this.#dirty = true;
    if (this.#refresh) {
      return this.#refresh;
    }
    this.#refresh = (async () => {
      this.#set({ refreshing: true });
      try {
        while (this.#dirty && !this.#disposed) {
          this.#dirty = false;
          await this.#queue;
          await this.#host.refinish(this.#state.needs);
          this.#set({ refreshError: null });
        }
      } catch (error) {
        this.#set({ refreshError: describe(error) });
      } finally {
        this.#refresh = null;
        this.#set({ refreshing: false });
      }
      // The new deck may draw substitute faces the last one did not (a source turned off, a new fallback).
      void this.#loadSubstitutes();
    })();
    return this.#refresh;
  }

  /**
   * The sources that need no click; call once the deck shows. The substitutes load at once, beside installed fonts
   * (when already granted) and then Google, so the floor never waits on the network.
   */
  async start(): Promise<void> {
    const substitutes = this.#loadSubstitutes();
    const sources = (async () => {
      const api = this.#env.localFonts;
      if (api) {
        const [permission, matching] = await Promise.all([api.permission(), this.#localMatching()]);
        this.#set({ permission });
        if (matching && permission === 'granted' && this.#state.needs.some((need) => need.faces.length > 0)) {
          await this.useLocalFonts();
        }
      }
      if (this.#state.googleEnabled) {
        await this.#fetchGoogle();
      }
    })();
    await Promise.all([substitutes, sources]).finally(() => this.#set({ settling: false }));
  }

  /** Asks the worker, with no font named, whether it picks installed faces (an older worker answers nothing). */
  async #localMatching(): Promise<boolean> {
    let matching = false;
    try {
      matching = (await this.#enqueue(() => this.#host.selectLocal([]))) !== null;
    } catch {
      // A worker that refuses the request does not match installed fonts either.
    }
    this.#set({ localMatching: matching });
    return matching;
  }

  /**
   * Reads the installed fonts the worker picks. Call it from a click while the permission is `prompt`: the browser's
   * prompt needs the click, so the listing starts before anything is awaited.
   */
  async useLocalFonts(): Promise<void> {
    const api = this.#env.localFonts;
    if (!api || this.#state.local.state === 'reading' || this.#state.localMatching === false) {
      return;
    }
    this.#set({ local: { state: 'reading' } });
    try {
      const result = await readLocalFonts(api, {
        select: (fonts) => this.#enqueue(() => this.#host.selectLocal(fonts)),
        add: (file) => this.#add(file, 'local', null),
        budget: this.#localBudget,
      });
      this.#set({ local: { state: 'done', result }, permission: 'granted' });
      if (result.added > 0) {
        await this.#refinish();
      }
    } catch (error) {
      if (error instanceof NoLocalMatching) {
        this.#set({ local: { state: 'idle' }, localMatching: false });
        return;
      }
      const failure = lfaFailure(error);
      this.#set({
        local: { state: 'failed', failure },
        ...(failure.kind === 'denied' ? { permission: 'denied' as const } : {}),
      });
    }
  }

  async #fetchGoogle(): Promise<void> {
    const plans = googlePlans(this.#state.needs);
    if (plans.length === 0) {
      this.#set({ google: { state: 'done', result: { families: [], files: 0, problems: [] } } });
      return;
    }
    const round = this.#googleRound;
    const stopped = () => this.#disposed || round !== this.#googleRound || !this.#state.googleEnabled;
    this.#set({ google: { state: 'fetching', families: plans.map((plan) => plan.family) } });
    const result = await fetchGoogleFonts(plans, {
      fetch: this.#env.fetch,
      codepoints: deckCodepoints(this.#host.slides()),
      add: (file) => this.#add(file.bytes, 'google', file.unicodeRange),
      stopped,
    });
    if (stopped()) {
      return;
    }
    this.#set({ google: { state: 'done', result } });
    if (result.files > 0) {
      await this.#refinish();
    }
  }

  /** Turns Google Fonts on or off, and remembers it. Off after its faces arrived opens the deck again without them. */
  async setGoogle(enabled: boolean): Promise<void> {
    if (enabled === this.#state.googleEnabled) {
      return;
    }
    writePreference(this.#env.storage, enabled);
    this.#set({ googleEnabled: enabled });
    if (enabled) {
      await Promise.all([this.#fetchGoogle(), this.#loadSubstitutes()]);
      return;
    }
    this.#googleRound += 1;
    if (this.#googleFiles === 0) {
      this.#set({ google: { state: 'idle' } });
      return;
    }
    this.#set({ google: { state: 'removing' } });
    await this.#enqueue(async () => {
      let needs = await this.#host.reopen();
      this.#googleFiles = 0;
      const kept = this.#kept;
      this.#kept = [];
      for (const file of kept) {
        try {
          needs = await this.#host.addFont(await file.file.arrayBuffer(), file.source, null);
          this.#kept.push(file);
        } catch {
          // A file the worker took before takes again; one that fails now is dropped with it.
        }
      }
      this.#set({ needs });
    });
    this.#set({ google: { state: 'idle' } });
    await this.#loadSubstitutes();
    await this.#refinish();
  }

  /** Why the user's `file` is not taken, within the deck's caps; `null` takes it. */
  #userRefusal(file: File): string | null {
    if (file.size > MAX_FONT_FILE_BYTES) {
      return `larger than ${MAX_FONT_FILE_BYTES >> 20} MB`;
    }
    if (this.#userFiles >= MAX_USER_FILES) {
      return `over the ${MAX_USER_FILES} files a deck takes`;
    }
    if (this.#userBytes + file.size > MAX_USER_BYTES) {
      return `over the ${MAX_USER_BYTES >> 20} MB of files a deck takes`;
    }
    return null;
  }

  /**
   * Adds the user's font files, then re-finishes. Each is read only when its turn in the queue comes, within the
   * deck's caps on files and bytes; a file the worker refuses gives its share back.
   */
  async addFiles(files: readonly File[]): Promise<void> {
    if (files.length === 0) {
      return;
    }
    const start = this.#state.files.length;
    this.#set({
      files: [...this.#state.files, ...files.map((file) => ({ name: file.name, state: 'reading' as const }))],
    });
    const settle = (index: number, entry: DroppedFile) => {
      const list = [...this.#state.files];
      list[start + index] = entry;
      this.#set({ files: list });
    };
    let added = 0;
    await Promise.all(
      files.map(async (file, index) => {
        const refusal = this.#userRefusal(file);
        if (refusal !== null) {
          settle(index, { name: file.name, state: 'rejected', reason: refusal });
          return;
        }
        this.#userFiles += 1;
        this.#userBytes += file.size;
        try {
          await this.#add(file, 'user', null);
          added += 1;
          settle(index, { name: file.name, state: 'added' });
        } catch (error) {
          this.#userFiles -= 1;
          this.#userBytes -= file.size;
          settle(index, { name: file.name, state: 'rejected', reason: describe(error) });
        }
      }),
    );
    if (added > 0) {
      await this.#refinish();
    }
  }

  /** Leaves a family out of the download, or puts it back. */
  setLeaveOut(family: string, leftOut: boolean) {
    const rest = this.#state.leaveOut.filter((name) => name !== family);
    this.#set({ leaveOut: leftOut ? [...rest, family] : rest });
  }

  /**
   * Includes a family whose licence restricts embedding it in the download (the user's call), or leaves it out again,
   * as it is by default.
   */
  setInclude(family: string, included: boolean) {
    const rest = this.#state.include.filter((name) => name !== family);
    this.#set(
      included
        ? { include: [...rest, family], leaveOut: this.#state.leaveOut.filter((name) => name !== family) }
        : { include: rest },
    );
  }

  /**
   * Adds the files of the substitute faces the deck shown draws with no file yet, then re-finishes; after the loads
   * queued before it.
   */
  #loadSubstitutes(): Promise<void> {
    const run = this.#substituting.then(() => this.#addSubstitutes());
    this.#substituting = run.catch(() => undefined);
    return run;
  }

  async #addSubstitutes(): Promise<void> {
    const wanted = this.#host.substitutes();
    if (wanted.length === 0 || this.#disposed) {
      return;
    }
    const previous = this.#state.substitutes;
    const families = new Set(previous.state === 'done' ? previous.families : []);
    const problems: string[] = [];
    let added = 0;
    this.#set({ substitutes: { state: 'loading' } });
    try {
      this.#manifest ??= loadManifest(this.#env.fetch, this.#env.base);
      const manifest = await this.#manifest;
      for (const wantedFace of wanted) {
        families.add(wantedFace.family);
        const face = substituteFile(manifest, wantedFace);
        if (!face) {
          problems.push(`${wantedFace.postscriptName}: not among the bundled files`);
          continue;
        }
        if (this.#substitutes.has(face.file)) {
          continue;
        }
        try {
          const bytes = await loadFace(this.#env.fetch, this.#env.base, face);
          await this.#add(bytes, 'bundled', null);
          this.#substitutes.add(face.file);
          added += 1;
        } catch (error) {
          problems.push(`${face.file}: ${describe(error)}`);
        }
      }
    } catch (error) {
      this.#manifest = null;
      problems.push(`${CORE_MANIFEST}: ${describe(error)}`);
    }
    this.#set({ substitutes: { state: 'done', families: [...families], problems } });
    if (added > 0) {
      await this.#refinish();
    }
  }

  /** Stops every source; nothing changes the deck after this. */
  dispose() {
    this.#disposed = true;
    this.#listeners.clear();
  }
}
