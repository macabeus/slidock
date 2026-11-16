// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';

import {
  type MediaPolicy,
  carriedFrames,
  carryMedia,
  handOver,
  hideMedia,
  parseTrim,
  play,
  playAlong,
  playMedium,
  prepareMedia,
  releaseCarry,
  rewind,
  seek,
  showMedia,
  stopMedium,
} from '../src/media';

const policy: MediaPolicy = { muted: false, onError: () => undefined };

/** A video whose `play()` only records that it was called; `readyState` 1 (its metadata loaded) unless given. */
function video(trim: string | null, currentTime: number, readyState = 1): HTMLVideoElement & { plays: number } {
  const element = document.createElement('video') as HTMLVideoElement & { plays: number };
  if (trim !== null) {
    element.setAttribute('data-trim', trim);
  }
  let time = currentTime;
  Object.defineProperty(element, 'currentTime', { get: () => time, set: (value: number) => (time = value) });
  Object.defineProperty(element, 'readyState', { get: () => readyState });
  element.addEventListener('loadedmetadata', () => (readyState = 1));
  element.plays = 0;
  element.play = () => {
    element.plays++;
    return Promise.resolve();
  };
  return element;
}

describe('media', () => {
  it('reads a trim in milliseconds', () => {
    expect(parseTrim('1535-186720')).toEqual({ startMs: 1535, endMs: 186720 });
    expect(parseTrim('200-')).toEqual({ startMs: 200, endMs: null });
    expect(parseTrim('x')).toBeNull();
  });

  it('plays a trimmed medium from its trim start when it stands outside the trim', () => {
    for (const [trim, from, expected] of [
      ['1535-186720', 0, 1.535],
      ['1535-186720', 186.72, 1.535],
      ['1535-186720', 200, 1.535],
      ['1535-', 0.5, 1.535],
      // Inside the trim it resumes where it paused.
      ['1535-186720', 10, 10],
      ['1535-', 10, 10],
      [null, 3, 3],
    ] as const) {
      const media = video(trim, from);
      play(media, policy);
      expect(media.currentTime, `${trim} from ${from}`).toBe(expected);
      expect(media.plays).toBe(1);
    }
  });

  it('mutes when the policy or the medium says so', () => {
    const quiet = video(null, 0);
    play(quiet, { ...policy, muted: true });
    expect(quiet.muted).toBe(true);
    const marked = video(null, 0);
    marked.setAttribute('data-muted', '');
    play(marked, policy);
    expect(marked.muted).toBe(true);
    const loud = video(null, 0);
    loud.muted = true;
    play(loud, policy);
    expect(loud.muted).toBe(false);
  });

  it('seeks a medium once its metadata loads, the latest seek winning, and plays from where it will stand', () => {
    const media = video('1535-186720', 0, 0);
    seek(media, 1);
    expect(media.currentTime, 'no seek before the metadata').toBe(0);
    seek(media, 2);
    play(media, policy);
    expect(media.plays).toBe(1);
    media.dispatchEvent(new Event('loadedmetadata'));
    expect(media.currentTime, 'the latest seek, inside the trim').toBe(2);
    media.dispatchEvent(new Event('loadedmetadata'));
    expect(media.currentTime).toBe(2);
    // With its metadata loaded it seeks at once.
    seek(media, 3);
    expect(media.currentTime).toBe(3);
    rewind(media);
    expect(media.currentTime).toBe(1.535);
    // A trim start the medium waits for is where it will play from.
    const early = video('1535-186720', 0, 0);
    play(early, policy);
    expect(early.currentTime).toBe(0);
    early.dispatchEvent(new Event('loadedmetadata'));
    expect(early.currentTime).toBe(1.535);
  });

  describe('across slides', () => {
    interface Clip extends HTMLVideoElement {
      plays: number;
      loads: number;
      playing: boolean;
      atEnd: boolean;
      /** The engine's state, set by hand: `HAVE_*` and whether a seek is under way. */
      ready: number;
      inSeek: boolean;
    }

    /**
     * A video of `src` (a `<source>` of each file of `sources` instead, when given) on a slide root, its clock and
     * state set by hand; `across` marks it `data-across-slides`.
     */
    function clip(
      root: HTMLElement,
      src: string,
      {
        across = true,
        time = 0,
        playing = false,
        atEnd = false,
        trim = null as string | null,
        readyState = 1,
        sources = null as string[] | null,
      } = {},
    ): Clip {
      const element = document.createElement('video') as Clip;
      if (sources === null) {
        element.setAttribute('src', src);
      } else {
        for (const file of sources) {
          const source = document.createElement('source');
          source.setAttribute('src', file);
          element.append(source);
        }
      }
      if (across) {
        element.setAttribute('data-across-slides', '');
      }
      if (trim !== null) {
        element.setAttribute('data-trim', trim);
      }
      element.setAttribute('preload', 'none');
      let now = time;
      element.ready = readyState;
      element.inSeek = false;
      element.playing = playing;
      element.atEnd = atEnd;
      Object.defineProperty(element, 'currentTime', { get: () => now, set: (value: number) => (now = value) });
      Object.defineProperty(element, 'readyState', { get: () => element.ready });
      Object.defineProperty(element, 'seeking', { get: () => element.inSeek });
      Object.defineProperty(element, 'networkState', { get: () => 1 });
      Object.defineProperty(element, 'paused', { get: () => !element.playing });
      Object.defineProperty(element, 'ended', { get: () => element.atEnd });
      element.addEventListener('loadedmetadata', () => (element.ready = Math.max(element.ready, 1)));
      element.plays = 0;
      element.loads = 0;
      element.play = () => {
        element.plays++;
        element.playing = true;
        return Promise.resolve();
      };
      element.pause = () => {
        element.playing = false;
      };
      element.load = () => {
        element.loads++;
      };
      root.append(element);
      return element;
    }
    const slide = () => document.createElement('main');

    it('pairs clips offered in more than one file by their first file', () => {
      const [a, b] = [slide(), slide()];
      const files = ['media/x.mov', 'media/x.webm'];
      const from = clip(a, '', { time: 2, playing: true, sources: files });
      const into = clip(b, '', { sources: files });
      expect(carryMedia(a, b)).toEqual([{ from, into }]);
      expect(into.currentTime).toBe(2);
      // Another clip, or the same file as the copy of another, does not pair.
      const [c, d] = [slide(), slide()];
      clip(c, '', { time: 2, playing: true, sources: files });
      clip(d, '', { sources: ['media/y.mov', 'media/x.webm'] });
      expect(carryMedia(c, d)).toEqual([]);
      const [e, f] = [slide(), slide()];
      clip(e, '', { time: 2, playing: true, sources: files });
      clip(f, 'media/x.mov');
      expect(carryMedia(e, f), 'a plain src of the same file pairs too').toHaveLength(1);
    });

    it('goes on from where the playing clip of the outgoing slide stands, and plays on', () => {
      const [a, b] = [slide(), slide()];
      const from = clip(a, 'media/x.mp4', { time: 5.5, playing: true, trim: '0-20020' });
      const into = clip(b, 'media/x.mp4', { trim: '0-20020' });
      const pairs = carryMedia(a, b);
      expect(pairs).toEqual([{ from, into }]);
      expect(into.currentTime, 'the transition shows the frame the outgoing clip stands at').toBe(5.5);
      expect(into.plays, 'not before that frame shows').toBe(0);
      playAlong(pairs);
      expect([into.playing, into.muted], 'and the clip going on, muted').toEqual([true, true]);
      from.currentTime = 7.5;
      into.currentTime = 7.48;
      handOver(pairs, policy);
      hideMedia(a);
      expect(into.currentTime).toBe(7.48);
      expect([into.playing, into.muted], 'its sound on, as the outgoing clip stops').toEqual([true, false]);
      expect(from.playing).toBe(false);
      // Its own play step and autoplay do not start it again.
      into.setAttribute('data-autoplay', '');
      showMedia(b, policy);
      playMedium(into, 0, policy);
      expect(into.currentTime).toBe(7.48);
    });

    it('holds the last frame of a clip that ended on the outgoing slide, whatever its play steps ask', () => {
      const [a, b] = [slide(), slide()];
      const from = clip(a, 'media/y.mp4', { time: 18.008, atEnd: true, trim: '0-18008' });
      const into = clip(b, 'media/y.mp4', { trim: '0-18008' });
      handOver(carryMedia(a, b), policy);
      expect(into.currentTime).toBe(18.008);
      expect(into.plays).toBe(0);
      playMedium(into, 0, policy);
      playMedium(into, 1000, policy);
      expect(into.currentTime).toBe(18.008);
      expect(into.plays).toBe(0);
      // A stop step ends the hand-over: a play step after it starts the clip again.
      stopMedium(into);
      playMedium(into, 0, policy);
      expect(into.currentTime).toBe(0);
      expect(into.plays).toBe(1);
      expect(from.plays).toBe(0);
    });

    it('a clip that reached its trim end holds it without the engine saying ended', () => {
      const [a, b] = [slide(), slide()];
      clip(a, 'media/y.mp4', { time: 10.3, trim: '0-10300' });
      const into = clip(b, 'media/y.mp4', { trim: '0-10300' });
      handOver(carryMedia(a, b), policy);
      playMedium(into, 0, policy);
      expect(into.currentTime).toBe(10.3);
      expect(into.plays).toBe(0);
    });

    it('a paused clip goes on paused, and its play step resumes it where it stands', () => {
      const [a, b] = [slide(), slide()];
      clip(a, 'media/x.mp4', { time: 3 });
      const into = clip(b, 'media/x.mp4');
      handOver(carryMedia(a, b), policy);
      expect(into.plays).toBe(0);
      expect(into.currentTime).toBe(3);
      playMedium(into, 0, policy);
      expect(into.plays).toBe(1);
      expect(into.currentTime).toBe(3);
    });

    it('carries nothing when either clip is unmarked, the clips differ, or the outgoing one never played', () => {
      const cases: [string, (a: HTMLElement, b: HTMLElement) => Clip][] = [
        [
          'incoming unmarked',
          (a, b) => (clip(a, 'media/x.mp4', { time: 4, playing: true }), clip(b, 'media/x.mp4', { across: false })),
        ],
        [
          'outgoing unmarked',
          (a, b) => (clip(a, 'media/x.mp4', { time: 4, playing: true, across: false }), clip(b, 'media/x.mp4')),
        ],
        ['another source', (a, b) => (clip(a, 'media/x.mp4', { time: 4, playing: true }), clip(b, 'media/z.mp4'))],
        [
          'another trim',
          (a, b) => (
            clip(a, 'media/x.mp4', { time: 4, playing: true, trim: '0-9000' }),
            clip(b, 'media/x.mp4', { trim: '1000-9000' })
          ),
        ],
        [
          'never played',
          (a, b) => (
            clip(a, 'media/x.mp4', { trim: '1000-9000', time: 1 }),
            clip(b, 'media/x.mp4', { trim: '1000-9000' })
          ),
        ],
      ];
      for (const [name, make] of cases) {
        const [a, b] = [slide(), slide()];
        const into = make(a, b);
        const pairs = carryMedia(a, b);
        expect(pairs, name).toEqual([]);
        handOver(pairs, policy);
        hideMedia(a);
        playMedium(into, 0, policy);
        expect(into.plays, name).toBe(1);
        expect(into.currentTime, `${name}: the play step starts it from its start`).toBe(
          (parseTrim(into.getAttribute('data-trim'))?.startMs ?? 0) / 1000,
        );
      }
    });

    it('pairs each clip once, in document order', () => {
      const [a, b] = [slide(), slide()];
      const first = clip(a, 'media/x.mp4', { time: 2, playing: true });
      const second = clip(a, 'media/x.mp4', { time: 6, playing: true });
      const one = clip(b, 'media/x.mp4');
      const two = clip(b, 'media/x.mp4');
      const three = clip(b, 'media/x.mp4');
      expect(carryMedia(a, b)).toEqual([
        { from: first, into: one },
        { from: second, into: two },
      ]);
      expect([one.currentTime, two.currentTime, three.currentTime]).toEqual([2, 6, 0]);
    });

    it('leaving the slide ends the hand-over', () => {
      const [a, b] = [slide(), slide()];
      clip(a, 'media/y.mp4', { time: 9, atEnd: true });
      const into = clip(b, 'media/y.mp4');
      handOver(carryMedia(a, b), policy);
      hideMedia(b);
      playMedium(into, 0, policy);
      expect(into.plays).toBe(1);
    });

    it('loads only the marked clips of the next slide that a marked clip of the shown slide can carry into', () => {
      const [shown, next] = [slide(), slide()];
      clip(shown, 'media/x.mp4', { readyState: 0 });
      clip(shown, 'media/z.mp4', { readyState: 0, across: false });
      const paired = clip(next, 'media/x.mp4', { readyState: 0 });
      const plain = clip(next, 'media/x.mp4', { readyState: 0, across: false });
      const otherClip = clip(next, 'media/y.mp4', { readyState: 0 });
      const unmarkedPair = clip(next, 'media/z.mp4', { readyState: 0 });
      prepareMedia(shown, next);
      expect([paired.preload, paired.loads]).toEqual(['auto', 1]);
      for (const [name, media] of [
        ['unmarked', plain],
        ['nothing shown plays it', otherClip],
        ['its pair on the shown slide is unmarked', unmarkedPair],
      ] as const) {
        expect([media.preload, media.loads], name).toEqual(['none', 0]);
      }
      // A slide with no clip to carry loads nothing on the next one.
      const lone = slide();
      const after = clip(lone, 'media/x.mp4', { readyState: 0 });
      prepareMedia(slide(), lone);
      expect([after.preload, after.loads]).toEqual(['none', 0]);
    });

    it('a carried clip that has not loaded loads, and is sought once its metadata has', () => {
      const [a, b] = [slide(), slide()];
      clip(a, 'media/x.mp4', { time: 4, playing: true });
      const into = clip(b, 'media/x.mp4', { readyState: 0 });
      carryMedia(a, b);
      expect([into.preload, into.loads, into.currentTime]).toEqual(['auto', 1, 0]);
      into.dispatchEvent(new Event('loadedmetadata'));
      expect(into.currentTime).toBe(4);
    });

    describe('the frame the move shows', () => {
      const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
      /** Whether `promise` has settled after the queued tasks run. */
      async function settledYet(promise: Promise<void>): Promise<boolean> {
        let done = false;
        void promise.then(() => (done = true));
        await tick();
        return done;
      }

      it('waits until each carried clip shows the frame it was sought to', async () => {
        const [a, b] = [slide(), slide()];
        clip(a, 'media/x.mp4', { time: 6.7, playing: true });
        const into = clip(b, 'media/x.mp4', { readyState: 0 });
        const pairs = carryMedia(a, b);
        const shown = carriedFrames(pairs, { timeoutMs: 10_000 });
        expect(await settledYet(shown), 'its metadata has not loaded: the seek waits').toBe(false);
        into.dispatchEvent(new Event('loadedmetadata'));
        into.inSeek = true;
        expect(await settledYet(shown), 'sought, the frame not yet decoded: it shows its first frame').toBe(false);
        into.inSeek = false;
        into.ready = 2;
        into.dispatchEvent(new Event('seeked'));
        expect(await settledYet(shown)).toBe(true);
        expect(into.currentTime).toBe(6.7);
      });

      it('does not wait for a clip that is there already, nor past its bound, an abort or an error', async () => {
        const [a, b] = [slide(), slide()];
        clip(a, 'media/x.mp4', { time: 3, playing: true });
        clip(b, 'media/x.mp4', { readyState: 4 });
        expect(await settledYet(carriedFrames(carryMedia(a, b), { timeoutMs: 10_000 }))).toBe(true);
        expect(await settledYet(carriedFrames([], { timeoutMs: 10_000 }))).toBe(true);

        const waiting = () => {
          const [c, d] = [slide(), slide()];
          clip(c, 'media/x.mp4', { time: 3, playing: true });
          const into = clip(d, 'media/x.mp4', { readyState: 1 });
          return { pairs: carryMedia(c, d), into };
        };
        const bounded = carriedFrames(waiting().pairs, { timeoutMs: 30 });
        expect(await settledYet(bounded)).toBe(false);
        await new Promise((resolve) => setTimeout(resolve, 60));
        expect(await settledYet(bounded), 'at its bound').toBe(true);
        const abort = new AbortController();
        const aborted = carriedFrames(waiting().pairs, { timeoutMs: 10_000, signal: abort.signal });
        expect(await settledYet(aborted)).toBe(false);
        abort.abort();
        expect(await settledYet(aborted)).toBe(true);
        const failing = waiting();
        const failed = carriedFrames(failing.pairs, { timeoutMs: 10_000 });
        Object.defineProperty(failing.into, 'error', { get: () => ({ code: 4 }) });
        failing.into.dispatchEvent(new Event('error'));
        expect(await settledYet(failed)).toBe(true);
      });
    });

    it('a move dropped before its hand-over leaves the incoming clip at its start, paused', () => {
      const [a, b] = [slide(), slide()];
      clip(a, 'media/x.mp4', { time: 6.86, playing: true, trim: '500-20000' });
      const into = clip(b, 'media/x.mp4', { trim: '500-20000' });
      const pairs = carryMedia(a, b);
      expect(into.currentTime).toBe(6.86);
      releaseCarry(pairs);
      expect([into.currentTime, into.playing]).toEqual([0.5, false]);
      playMedium(into, 0, policy);
      expect(into.currentTime, 'its play step starts it from its start').toBe(0.5);

      // Once handed over, the clip is the shown slide's: a later drop leaves it playing where it is.
      const [c, d] = [slide(), slide()];
      const from = clip(c, 'media/x.mp4', { time: 4, playing: true });
      const shown = clip(d, 'media/x.mp4');
      const handed = carryMedia(c, d);
      from.currentTime = 4.2;
      handOver(handed, policy);
      releaseCarry(handed);
      expect([shown.currentTime, shown.playing]).toEqual([4.2, true]);
    });

    describe('through the transition', () => {
      it('plays along while its pair plays, and takes over from its own frame, not the outgoing clock', () => {
        const [a, b] = [slide(), slide()];
        const from = clip(a, 'media/x.mp4', { time: 3.1, playing: true });
        const into = clip(b, 'media/x.mp4');
        const pairs = carryMedia(a, b);
        playAlong(pairs);
        expect([into.currentTime, into.playing, into.muted]).toEqual([3.1, true, true]);
        // Each engine stalls on its own, so the clocks drift apart; the incoming clip shows the transition's last frame.
        from.currentTime = 4.35;
        into.currentTime = 4.59;
        handOver(pairs, policy);
        hideMedia(a);
        expect([into.currentTime, into.playing, into.muted], 'no jump back to the outgoing clock').toEqual([
          4.59,
          true,
          false,
        ]);
      });

      it('an ended or paused clip does not play along', () => {
        for (const [name, atEnd] of [
          ['ended', true],
          ['paused', false],
        ] as const) {
          const [a, b] = [slide(), slide()];
          clip(a, 'media/y.mp4', { time: atEnd ? 10.3 : 3, atEnd });
          const into = clip(b, 'media/y.mp4');
          playAlong(carryMedia(a, b));
          expect(into.plays, name).toBe(0);
        }
      });

      it('one whose pair ends during the move stops at its end', () => {
        const [a, b] = [slide(), slide()];
        const from = clip(a, 'media/y.mp4', { time: 9.9, playing: true, trim: '0-10300' });
        const into = clip(b, 'media/y.mp4', { trim: '0-10300' });
        const pairs = carryMedia(a, b);
        playAlong(pairs);
        expect(into.playing).toBe(true);
        // The outgoing clip reached its end during the move; the incoming one a frame short of it.
        from.currentTime = 10.3;
        from.playing = false;
        from.atEnd = true;
        into.currentTime = 10.27;
        handOver(pairs, policy);
        expect([into.playing, into.currentTime], 'held at the end, as a clip that ended before the move').toEqual([
          false,
          10.3,
        ]);
        playMedium(into, 0, policy);
        expect(into.playing).toBe(false);
      });

      it('a move dropped while it plays along stops it at its start', () => {
        const [a, b] = [slide(), slide()];
        clip(a, 'media/x.mp4', { time: 5, playing: true, trim: '500-20000' });
        const into = clip(b, 'media/x.mp4', { trim: '500-20000' });
        const pairs = carryMedia(a, b);
        playAlong(pairs);
        expect(into.playing).toBe(true);
        releaseCarry(pairs);
        expect([into.playing, into.currentTime]).toEqual([false, 0.5]);
      });
    });
  });
});
