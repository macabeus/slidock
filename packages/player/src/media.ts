// Media on show and hide: while presenting, videos marked `data-autoplay` start when their slide
// shows; leaving a slide pauses every medium and rewinds it to its trim start. `data-trim="start-end"` (ms) bounds
// playback, looping back to the start when `data-loop` is set. A medium marked `data-across-slides` goes on into the
// next slide's same clip, also marked, on a forward move (`carryMedia`, `handOver`): the clip plays on from where it
// was, or holds its last frame once it ended, and a play step never starts it again (Keynote's "play across
// slides", `docs/format.md` §3). The move waits until the incoming clip shows that frame (`carriedFrames`), the
// incoming clip then plays along, muted, through the transition while its pair plays (`playAlong`), and a move dropped
// before it lands leaves the clip at its start (`releaseCarry`).

/**
 * How a player plays media: muted throughout (the presenter view, whose sound is the deck window's), and where a
 * medium that fails to play is reported.
 */
export interface MediaPolicy {
  readonly muted: boolean;
  readonly onError: (media: HTMLMediaElement) => void;
}

export interface Trim {
  readonly startMs: number;
  readonly endMs: number | null;
}

export function parseTrim(value: string | null): Trim | null {
  const match = value === null ? null : /^(\d+(?:\.\d+)?)-(\d+(?:\.\d+)?)?$/.exec(value);
  if (match === null) {
    return null;
  }
  return { startMs: Number(match[1]), endMs: match[2] === undefined ? null : Number(match[2]) };
}

function mediaElements(root: ParentNode): HTMLMediaElement[] {
  return Array.from(root.querySelectorAll<HTMLMediaElement>('video, audio'));
}

const watched = new WeakSet<HTMLMediaElement>();

function watchTrim(media: HTMLMediaElement): void {
  const trim = parseTrim(media.getAttribute('data-trim'));
  if (trim === null || trim.endMs === null || watched.has(media)) {
    return;
  }
  watched.add(media);
  const end = trim.endMs / 1000;
  media.addEventListener('timeupdate', () => {
    if (media.currentTime >= end) {
      if (media.hasAttribute('data-loop')) {
        media.currentTime = trim.startMs / 1000;
      } else {
        media.pause();
      }
    }
  });
}

/** `HTMLMediaElement.HAVE_METADATA`: the medium knows its duration and can seek. */
const HAVE_METADATA = 1;

/** The seek each medium waits to make until its metadata loads. */
const pendingSeeks = new WeakMap<HTMLMediaElement, number>();

function seekNow(media: HTMLMediaElement, seconds: number): void {
  try {
    media.currentTime = seconds;
  } catch {
    // Not seekable in this engine.
  }
}

/**
 * Seeks a medium, or, before its metadata loads, once it has: a seek before then only sets where playback will start,
 * and WebKit's GStreamer backend can then play on with its clock held at that position. The latest seek wins.
 */
export function seek(media: HTMLMediaElement, seconds: number): void {
  if (media.readyState >= HAVE_METADATA) {
    pendingSeeks.delete(media);
    seekNow(media, seconds);
    return;
  }
  if (!pendingSeeks.has(media)) {
    media.addEventListener(
      'loadedmetadata',
      () => {
        const at = pendingSeeks.get(media);
        pendingSeeks.delete(media);
        if (at !== undefined) {
          seekNow(media, at);
        }
      },
      { once: true },
    );
  }
  pendingSeeks.set(media, seconds);
}

/** Where a medium stands, or will once the seek it waits to make is made. */
function position(media: HTMLMediaElement): number {
  return pendingSeeks.get(media) ?? media.currentTime;
}

/** Rewinds a medium to its trim start. */
export function rewind(media: HTMLMediaElement): void {
  seek(media, (parseTrim(media.getAttribute('data-trim'))?.startMs ?? 0) / 1000);
}

/** Starts a medium from where it is when that lies inside its trim, else from the trim start. */
export function play(media: HTMLMediaElement, policy: MediaPolicy): void {
  const { onError } = policy;
  watchTrim(media);
  const trim = parseTrim(media.getAttribute('data-trim'));
  if (trim !== null) {
    const start = trim.startMs / 1000;
    const end = trim.endMs === null ? Infinity : trim.endMs / 1000;
    const at = position(media);
    if (at < start || at >= end) {
      seek(media, start);
    }
  }
  media.muted = policy.muted || media.hasAttribute('data-muted');
  if (media.preload === 'none') {
    media.preload = 'auto';
  }
  try {
    const started = media.play() as Promise<void> | undefined;
    started?.catch(() => onError(media));
  } catch {
    onError(media);
  }
}

const ACROSS = 'data-across-slides';

/** Media paired per move at most. */
const MAX_CARRIED = 64;

/** `HTMLMediaElement.NETWORK_LOADING`. */
const NETWORK_LOADING = 2;

/** The media that went on from the previous slide's clip, each with whether it stood at its end then. */
const carried = new WeakMap<HTMLMediaElement, boolean>();

/** A medium of the outgoing slide and the one of the incoming slide that goes on from it. */
export interface Carry {
  readonly from: HTMLMediaElement;
  readonly into: HTMLMediaElement;
}

function trimStart(media: HTMLMediaElement): number {
  return (parseTrim(media.getAttribute('data-trim'))?.startMs ?? 0) / 1000;
}

/** Whether a medium has played since its slide showed: playing, ended, or past its trim start. */
function started(media: HTMLMediaElement): boolean {
  return !media.paused || media.ended || position(media) > trimStart(media);
}

/** Whether a medium stands at its end: ended, or at its trim end. */
function finished(media: HTMLMediaElement): boolean {
  const end = parseTrim(media.getAttribute('data-trim'))?.endMs ?? null;
  return media.ended || (end !== null && position(media) >= end / 1000);
}

/** The file a medium plays: its `src`, or its first `<source>`'s when it offers several. */
function fileOf(media: HTMLMediaElement): string | null {
  return media.getAttribute('src') ?? media.firstElementChild?.getAttribute('src') ?? null;
}

/** The same clip: the same file and the same trim. */
function sameClip(a: HTMLMediaElement, b: HTMLMediaElement): boolean {
  const src = fileOf(a);
  return src !== null && src === fileOf(b) && a.getAttribute('data-trim') === b.getAttribute('data-trim');
}

/** Lets a medium that loads nothing until it plays (`preload="none"`) load, so a seek can show its frame. */
function load(media: HTMLMediaElement): void {
  if (media.preload === 'none') {
    media.preload = 'auto';
  }
  if (media.readyState === 0 && media.networkState !== NETWORK_LOADING) {
    try {
      media.load();
    } catch {
      // Not loadable in this engine.
    }
  }
}

/** The marked media of `into` that the same clip, also marked, on `from` can carry into: at most `MAX_CARRIED`. */
function pairable(from: ParentNode, into: ParentNode): HTMLMediaElement[] {
  const outgoing = mediaElements(from).filter((media) => media.hasAttribute(ACROSS));
  if (outgoing.length === 0) {
    return [];
  }
  return mediaElements(into)
    .filter((media) => media.hasAttribute(ACROSS) && outgoing.some((other) => sameClip(other, media)))
    .slice(0, MAX_CARRIED);
}

/**
 * While presenting, the next slide is mounted: its marked media that a marked clip of the shown slide can carry into
 * load, so a hand-over has a frame to show at once. No other medium loads before it plays.
 */
export function prepareMedia(shown: ParentNode, next: ParentNode): void {
  for (const media of pairable(shown, next)) {
    load(media);
  }
}

/**
 * A forward move from one slide to the next: pairs each medium of the incoming slide marked `data-across-slides` with
 * the first unpaired medium of the outgoing slide, also marked, that plays the same clip and has started, and seeks it
 * to where its pair stands, so the transition shows that frame.
 */
export function carryMedia(from: ParentNode, into: ParentNode): Carry[] {
  const outgoing = mediaElements(from)
    .filter((media) => media.hasAttribute(ACROSS) && started(media))
    .slice(0, MAX_CARRIED);
  const pairs: Carry[] = [];
  for (const media of mediaElements(into)) {
    if (outgoing.length === 0 || pairs.length >= MAX_CARRIED) {
      break;
    }
    if (!media.hasAttribute(ACROSS)) {
      continue;
    }
    const index = outgoing.findIndex((other) => sameClip(other, media));
    const match = index < 0 ? undefined : outgoing.splice(index, 1)[0];
    if (match === undefined) {
      continue;
    }
    load(media);
    seek(media, position(match));
    pairs.push({ from: match, into: media });
  }
  return pairs;
}

/** `HTMLMediaElement.HAVE_CURRENT_DATA`: the medium has the frame where it stands. */
const HAVE_CURRENT_DATA = 2;

/** How long a move waits for its carried media to show the frame they were sought to. */
export const CARRY_SEEK_TIMEOUT_MS = 1000;

/** Whether a medium shows the frame where it stands: its seek landed and that frame decoded, or it failed. */
function landed(media: HTMLMediaElement): boolean {
  return media.error !== null || (!pendingSeeks.has(media) && !media.seeking && media.readyState >= HAVE_CURRENT_DATA);
}

/**
 * Resolves once every carried medium shows the frame it was sought to, at most `timeoutMs` later, or when `signal`
 * aborts: until then an engine shows the frame the medium stood at before, its first frame, and the move would show the
 * clip starting again.
 */
export function carriedFrames(
  pairs: readonly Carry[],
  { timeoutMs = CARRY_SEEK_TIMEOUT_MS, signal }: { timeoutMs?: number; signal?: AbortSignal } = {},
): Promise<void> {
  const waiting = pairs.map(({ into }) => into).filter((media) => !landed(media));
  if (waiting.length === 0 || signal?.aborted === true) {
    return Promise.resolve();
  }
  return new Promise((resolve) => {
    const events = ['seeked', 'loadeddata', 'canplay', 'error'] as const;
    const check = (): void => {
      if (waiting.every(landed)) {
        finish();
      }
    };
    const finish = (): void => {
      clearTimeout(timer);
      signal?.removeEventListener('abort', finish);
      for (const media of waiting) {
        for (const type of events) {
          media.removeEventListener(type, check);
        }
      }
      resolve();
    };
    const timer = setTimeout(finish, timeoutMs);
    signal?.addEventListener('abort', finish, { once: true });
    for (const media of waiting) {
      for (const type of events) {
        media.addEventListener(type, check);
      }
    }
  });
}

/**
 * The move starts, its carried frames shown: each carried medium whose pair plays plays along with it, muted (the pair
 * has the sound), so the transition shows the clip going on rather than the frame the move began on. It starts once
 * its seek landed (WebKit lets a medium played before then hold its seek), so it trails its pair by that wait, 0.1 to
 * 0.3 s. Nothing brings it back in step: a seek or a rate change of a playing medium stalls it in WebKit for longer than
 * that. A failure to play shows at the hand-over, which plays it again.
 */
export function playAlong(pairs: readonly Carry[]): void {
  for (const { from, into } of pairs) {
    if (!from.paused) {
      play(into, { muted: true, onError: () => undefined });
    }
  }
}

/**
 * A forward move was dropped before it landed: the media it sought for a hand-over that never came go back to their
 * trim start, as on any slide that is not shown.
 */
export function releaseCarry(pairs: readonly Carry[]): void {
  for (const { into } of pairs) {
    if (!carried.has(into)) {
      try {
        into.pause();
      } catch {
        // Not playable in this engine.
      }
      rewind(into);
    }
  }
}

/**
 * The move lands, before the outgoing slide's media stop: each carried medium takes over from its pair where it now
 * stands, playing on when it played and had not reached its end. One that still plays along goes on from its own frame,
 * the one the transition ended on, so the take-over neither stalls nor jumps ahead; the stretch it trails its pair by
 * (`playAlong`) plays again.
 */
export function handOver(pairs: readonly Carry[], policy: MediaPolicy): void {
  for (const { from, into } of pairs) {
    const end = finished(from);
    const playing = !from.paused && !end;
    if (!playing) {
      into.pause();
    }
    if (into.paused) {
      seek(into, position(from));
    }
    carried.set(into, end);
    if (playing) {
      play(into, policy);
    }
  }
}

/**
 * A play step: from `fromMs` when it is after the start, else from where the medium stands (`play`). A medium that went
 * on from the previous slide is already playing its clip: it plays on from where it is, and stays at its end once it
 * reached it.
 */
export function playMedium(media: HTMLMediaElement, fromMs: number, policy: MediaPolicy): void {
  const end = carried.get(media);
  if (end !== undefined) {
    if (!end && !finished(media)) {
      play(media, policy);
    }
    return;
  }
  if (fromMs > 0) {
    seek(media, fromMs / 1000);
  }
  play(media, policy);
}

/** A stop step: the medium pauses and rewinds, and a play step after it starts it again. */
export function stopMedium(media: HTMLMediaElement): void {
  carried.delete(media);
  media.pause();
  rewind(media);
}

/** A slide shows: start its autoplay media, but for those that went on from the previous slide. */
export function showMedia(root: ParentNode, policy: MediaPolicy): void {
  for (const media of mediaElements(root)) {
    if (media.hasAttribute('data-autoplay') && !carried.has(media)) {
      rewind(media);
      play(media, policy);
    }
  }
}

/** A slide leaves: pause and rewind every medium. */
export function hideMedia(root: ParentNode): void {
  for (const media of mediaElements(root)) {
    carried.delete(media);
    try {
      media.pause();
    } catch {
      // Not playable in this engine.
    }
    rewind(media);
  }
}
