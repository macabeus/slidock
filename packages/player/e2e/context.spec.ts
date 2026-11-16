// Motion in context: the readiness gate before a transition, auto-advance, media steps, Magic
// Move between two slides (FLIP frames mid-way, unmatched elements fading), and the presenter view's preview of the
// next click.
import { type Page, type Route, expect, test } from '@playwright/test';
import type { DeckStep, DeckTransition } from '@slidock/types';

import { HEIGHT, WIDTH, deckSlide, fixture, slideHtml, step, transition } from './support/deck';
import { type Harness, diff, disallowedKeys, open, pauseOnStart, settle, shot, staticShot } from './support/harness';

const MEDIA = 'https://media.slidock.test/';
const AA = 96;
const SPECKS = 256;

const fields = (f: Record<string, unknown>) => f as Partial<DeckStep>;
const withAdvance = (t: DeckTransition, onClick: boolean, afterMs: number | null): DeckTransition => ({
  ...t,
  advance: { onClick, afterMs },
});

/** A 960 x 540 slide in the fixture's markup: a background and absolutely placed boxes (pt). */
function boxes(id: string, background: string, items: string[]): string {
  return (
    `<main data-slide="${id}" style="position:relative;width:720pt;height:405pt;overflow:hidden">` +
    `<div data-sd-bg style="position:absolute;inset:0;background:${background}"></div>${items.join('')}</main>`
  );
}
function box(id: string, x: number, y: number, w: number, h: number, color: string, extra = ''): string {
  return (
    `<div data-el="${id}" data-kind="shape" ${extra} style="position:absolute;left:${x}pt;top:${y}pt;width:${w}pt;` +
    `height:${h}pt;background:${color}"></div>`
  );
}

test.describe('readiness gate', () => {
  const picture = (id: string) =>
    slideHtml(id, 'b').replace(
      '</main>',
      `<img data-el="${id}.pic" src="media/pic.svg" style="position:absolute;left:500pt;top:40pt;width:120pt;height:80pt"></main>`,
    );
  const SVG =
    '<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80"><rect width="120" height="80" fill="#7c3aed"/></svg>';

  test("a transition starts once the incoming slide's images decode", async ({ page }) => {
    let release: () => void = () => undefined;
    const held = new Promise<void>((resolve) => (release = resolve));
    await page.route(`${MEDIA}**`, async (route: Route) => {
      await held;
      await route.fulfill({ contentType: 'image/svg+xml', body: SVG });
    });
    const data = fixture(
      [deckSlide('s1'), deckSlide('s2', { transition: transition({ effect: 'fade', params: { through: 'none' } }) })],
      {
        s2: picture('s2'),
      },
    );
    await open(page, { ...data, mediaBase: MEDIA });
    await page.evaluate(() => (window as unknown as Harness).sd.player.next());
    await page.waitForTimeout(600);
    const waiting = await page.evaluate(() => ({
      running: (window as unknown as Harness).sd.count(),
      current: document.querySelector('.sd-layer[data-sd-current]')?.getAttribute('data-sd-layer'),
    }));
    expect(waiting).toEqual({ running: 0, current: 's1' });
    release();
    await page.waitForFunction(() => (window as unknown as Harness).sd.count() > 0);
    await settle(page);
    const codes = await page.evaluate(() => (window as unknown as Harness).sd.diagnostics.map((d) => d.code));
    expect(codes).not.toContain('RT0001');
    expect(await diff(page, await shot(page), await staticShot(page, 1, 0))).toBe(0);
  });

  test('a transition starts at the 1.5 s cap when an image never loads, with RT0001', async ({ page }) => {
    await page.route(`${MEDIA}**`, () => new Promise(() => undefined));
    const data = fixture(
      [deckSlide('s1'), deckSlide('s2', { transition: transition({ effect: 'fade', params: { through: 'none' } }) })],
      {
        s2: picture('s2'),
      },
    );
    await open(page, { ...data, mediaBase: MEDIA });
    const waited = await page.evaluate(async () => {
      const { sd } = window as unknown as Harness;
      const start = performance.now();
      sd.player.next();
      while (sd.count() === 0) {
        await new Promise((resolve) => requestAnimationFrame(resolve));
      }
      return performance.now() - start;
    });
    expect(waited).toBeGreaterThanOrEqual(1450);
    expect(waited).toBeLessThan(2500);
    await settle(page);
    const codes = await page.evaluate(() => (window as unknown as Harness).sd.diagnostics.map((d) => d.code));
    expect(codes).toContain('RT0001');
  });
});

test.describe('auto-advance', () => {
  const dissolve = transition({ effect: 'fade', params: { through: 'none' }, durationMs: 500 });

  test('a slide advances afterMs after its transition ends', async ({ page }) => {
    const data = fixture([
      deckSlide('s1'),
      deckSlide('s2', { transition: withAdvance(dissolve, true, 700) }),
      deckSlide('s3', { transition: dissolve }),
    ]);
    await open(page, data);
    const start = await page.evaluate(() => {
      const { sd } = window as unknown as Harness;
      sd.events.length = 0;
      sd.player.next();
      return performance.now();
    });
    await page.waitForFunction(() => (window as unknown as Harness).sd.player.position.slide === 2, null, {
      timeout: 5000,
    });
    const events = await page.evaluate(() => (window as unknown as Harness).sd.events);
    const left = events.find((e) => e.type === 'position' && e.slide === 2);
    // The readiness gate opens at once (no fonts, no images): 500 ms of transition, then 700 ms on the slide.
    expect((left?.t ?? 0) - start).toBeGreaterThanOrEqual(1200);
    expect((left?.t ?? 0) - start).toBeLessThan(1700);
    await settle(page);
    // The last slide has no timer.
    await page.waitForTimeout(1200);
    expect(await page.evaluate(() => (window as unknown as Harness).sd.player.position.slide)).toBe(2);
    expect(await diff(page, await shot(page), await staticShot(page, 2, 0))).toBe(0);
  });

  test('a slide that ignores clicks plays its builds on its own, and advances after the last one', async ({ page }) => {
    const timeline = [
      step('s2.box', fields({ effect: 'fade', durationMs: 600 })),
      step('s2.text', fields({ effect: 'wipe', params: { dir: 'r', soft: false }, durationMs: 400 })),
    ];
    const data = fixture([
      deckSlide('s1'),
      deckSlide('s2', { timeline, transition: withAdvance(dissolve, false, 300) }),
      deckSlide('s3', { transition: dissolve }),
    ]);
    await open(page, data);
    const start = await page.evaluate(() => {
      const { sd } = window as unknown as Harness;
      sd.events.length = 0;
      sd.player.next();
      return performance.now();
    });
    await page.waitForFunction(() => (window as unknown as Harness).sd.player.position.slide === 2, null, {
      timeout: 5000,
    });
    const moves = await page.evaluate(() =>
      (window as unknown as Harness).sd.events.filter((e) => e.type === 'position'),
    );
    expect(moves.map((e) => `${e.slide}.${e.step}`)).toEqual(['1.0', '1.1', '1.2', '2.0']);
    const at = (key: string) => (moves.find((e) => `${e.slide}.${e.step}` === key)?.t ?? 0) - start;
    // Transition 500 ms; the builds follow at once (600 ms, then 400 ms); the 300 ms timer ran out meanwhile.
    expect(at('1.1')).toBeGreaterThanOrEqual(480);
    expect(at('1.2')).toBeGreaterThanOrEqual(1080);
    expect(at('2.0')).toBeGreaterThanOrEqual(1480);
    expect(at('2.0')).toBeLessThan(2100);
  });

  test('a click forward does not advance a slide that ignores clicks', async ({ page }) => {
    const data = fixture([deckSlide('s1', { transition: withAdvance(dissolve, false, null) }), deckSlide('s2')]);
    await open(page, data, { pointer: 'always' });
    await page.mouse.click(WIDTH * 0.8, HEIGHT / 2);
    await page.waitForTimeout(300);
    expect(await page.evaluate(() => (window as unknown as Harness).sd.player.position.slide)).toBe(0);
  });
});

test.describe('media steps', () => {
  /** A mono 8 kHz WAV of `seconds` of a quiet tone. */
  function wav(seconds: number): Buffer {
    const rate = 8000;
    const samples = rate * seconds;
    const buffer = Buffer.alloc(44 + samples * 2);
    buffer.write('RIFF', 0);
    buffer.writeUInt32LE(36 + samples * 2, 4);
    buffer.write('WAVEfmt ', 8);
    buffer.writeUInt32LE(16, 16);
    buffer.writeUInt16LE(1, 20);
    buffer.writeUInt16LE(1, 22);
    buffer.writeUInt32LE(rate, 24);
    buffer.writeUInt32LE(rate * 2, 28);
    buffer.writeUInt16LE(2, 32);
    buffer.writeUInt16LE(16, 34);
    buffer.write('data', 36);
    buffer.writeUInt32LE(samples * 2, 40);
    for (let i = 0; i < samples; i++) {
      buffer.writeInt16LE(Math.round(Math.sin((i / rate) * 2 * Math.PI * 440) * 800), 44 + i * 2);
    }
    return buffer;
  }

  test('play, pause, stop and toggle steps drive their medium; leaving the slide stops it', async ({ page }) => {
    const sound = wav(6);
    await page.route(`${MEDIA}**`, (route) => route.fulfill({ contentType: 'audio/wav', body: sound }));
    const media = (kind: string, extra: Record<string, unknown> = {}) =>
      step('s1.snd', fields({ kind: 'media', fallback: 'none', native: `test:${kind}`, effect: kind, ...extra }));
    const timeline = [media('play', { params: { from: 1000 } }), media('pause'), media('toggle'), media('stop')];
    const html = slideHtml('s1').replace(
      '</main>',
      '<audio data-el="s1.snd" data-kind="media" src="media/tone.wav" data-muted preload="none"></audio></main>',
    );
    await open(page, { ...fixture([deckSlide('s1', { timeline }), deckSlide('s2')], { s1: html }), mediaBase: MEDIA });
    const state = () =>
      page.evaluate(() => {
        const audio = document
          .querySelector('.sd-layer[data-sd-layer="s1"]')
          ?.shadowRoot?.querySelector('audio') as HTMLAudioElement | null;
        return audio === null ? null : { paused: audio.paused, time: audio.currentTime, muted: audio.muted };
      });
    const next = async () => {
      await page.evaluate(() => (window as unknown as Harness).sd.player.next());
      await settle(page);
    };
    expect(await state()).toMatchObject({ paused: true, time: 0 });
    const played = Date.now();
    await next();
    await expect.poll(async () => (await state())?.time ?? 0).toBeGreaterThan(1.1);
    expect(await state()).toMatchObject({ paused: false, muted: true });
    await next();
    const elapsed = (Date.now() - played) / 1000;
    // The engine settles on where the medium stopped (WebKit's playing clock runs ahead and corrects itself after a
    // pause), then holds there; that position lies between the play step's start and the wall time since it.
    let paused = await state();
    expect(paused?.paused).toBe(true);
    await expect
      .poll(
        async () => {
          const before = paused?.time;
          await page.waitForTimeout(150);
          paused = await state();
          return paused?.time === before;
        },
        { intervals: [0] },
      )
      .toBe(true);
    expect(paused?.time ?? 0).toBeGreaterThanOrEqual(1);
    expect(paused?.time ?? 0).toBeLessThanOrEqual(1 + elapsed);
    await page.waitForTimeout(300);
    expect((await state())?.time).toBe(paused?.time);
    await next();
    await expect.poll(async () => (await state())?.time ?? 0).toBeGreaterThan((paused?.time ?? 0) + 0.1);
    expect((await state())?.paused).toBe(false);
    await next();
    expect(await state()).toMatchObject({ paused: true, time: 0 });
    // Play again, then leave: the medium pauses and rewinds.
    await page.evaluate(() => (window as unknown as Harness).sd.player.goTo(0, 0));
    await settle(page);
    await next();
    await expect.poll(async () => (await state())?.paused).toBe(false);
    await page.evaluate(() => (window as unknown as Harness).sd.player.goTo(1));
    await settle(page);
    expect(await state()).toMatchObject({ paused: true, time: 0 });
  });
});

test.describe('media across slides', () => {
  /** A mono 8 kHz WAV of `seconds` of a quiet tone. */
  function wav(seconds: number): Buffer {
    const rate = 8000;
    const samples = rate * seconds;
    const buffer = Buffer.alloc(44 + samples * 2);
    buffer.write('RIFF', 0);
    buffer.writeUInt32LE(36 + samples * 2, 4);
    buffer.write('WAVEfmt ', 8);
    buffer.writeUInt32LE(16, 16);
    buffer.writeUInt16LE(1, 20);
    buffer.writeUInt16LE(1, 22);
    buffer.writeUInt32LE(rate, 24);
    buffer.writeUInt32LE(rate * 2, 28);
    buffer.writeUInt16LE(2, 32);
    buffer.writeUInt16LE(16, 34);
    buffer.write('data', 36);
    buffer.writeUInt32LE(samples * 2, 40);
    for (let i = 0; i < samples; i++) {
      buffer.writeInt16LE(Math.round(Math.sin((i / rate) * 2 * Math.PI * 440) * 800), 44 + i * 2);
    }
    return buffer;
  }

  // Keynote's shared movie: the same clip on consecutive slides, each with a play step that starts with the slide.
  const play = (id: string) =>
    step(
      `${id}.snd`,
      fields({
        kind: 'media',
        trigger: 'afterPrevious',
        fallback: 'none',
        native: 'test:play',
        effect: 'play',
        params: { from: 0 },
      }),
    );
  const withClip = (id: string, file: string, across: boolean) =>
    slideHtml(id).replace(
      '</main>',
      `<audio data-el="${id}.snd" data-kind="media" src="media/${file}" data-muted preload="none"${across ? ' data-across-slides' : ''}></audio></main>`,
    );
  const clipOf = (page: Page, id: string) =>
    page.evaluate((slide) => {
      const audio = document
        .querySelector(`.sd-layer[data-sd-layer="${slide}"]`)
        ?.shadowRoot?.querySelector('audio') as HTMLAudioElement | null;
      return audio === null ? null : { paused: audio.paused, time: audio.currentTime };
    }, id);
  const next = async (page: Page) => {
    await page.evaluate(() => (window as unknown as Harness).sd.player.next());
    await settle(page);
  };

  /** Serves the clips by ranges, as a file or an HTTP server serves media: an engine seeks only in what it can fetch again. */
  const serve = (page: Page, sounds: Record<string, Buffer>) =>
    page.route(`${MEDIA}**`, (route) => {
      const body = sounds[new URL(route.request().url()).pathname.slice('/media/'.length)] ?? wav(1);
      const range = /^bytes=(\d+)-(\d*)$/.exec(route.request().headers()['range'] ?? '');
      if (range === null) {
        return route.fulfill({ contentType: 'audio/wav', headers: { 'accept-ranges': 'bytes' }, body });
      }
      const start = Number(range[1]);
      const end = range[2] === '' ? body.length - 1 : Math.min(Number(range[2]), body.length - 1);
      return route.fulfill({
        status: 206,
        contentType: 'audio/wav',
        headers: { 'accept-ranges': 'bytes', 'content-range': `bytes ${start}-${end}/${body.length}` },
        body: body.subarray(start, end + 1),
      });
    });

  test('a clip marked on both slides plays on across the move; unmarked, it starts again', async ({ page }) => {
    await serve(page, { 'long.wav': wav(6), 'short.wav': wav(1) });
    const ids = ['s0', 's1', 's2', 's3', 's4', 's5'];
    const html = {
      s1: withClip('s1', 'long.wav', true),
      s2: withClip('s2', 'long.wav', true),
      s3: withClip('s3', 'long.wav', false),
      s4: withClip('s4', 'short.wav', true),
      s5: withClip('s5', 'short.wav', true),
    };
    const slides = ids.map((id) => deckSlide(id, id === 's0' ? {} : { timeline: [play(id)] }));
    await open(page, { ...fixture(slides, html), mediaBase: MEDIA });

    await next(page);
    await expect.poll(async () => (await clipOf(page, 's1'))?.time ?? 0).toBeGreaterThan(1.2);
    await next(page);
    const carried = await clipOf(page, 's2');
    expect(carried?.paused).toBe(false);
    expect(carried?.time ?? 0, 'goes on from where the outgoing clip stood').toBeGreaterThan(1.2);
    expect(await clipOf(page, 's1'), 'the outgoing clip stops and rewinds').toMatchObject({ paused: true, time: 0 });

    // Unmarked on the incoming slide: its play step starts the clip again.
    await next(page);
    const restarted = await clipOf(page, 's3');
    expect(restarted?.paused).toBe(false);
    expect(restarted?.time ?? 99).toBeLessThan(0.8);

    // A clip that ended goes on at its end and stays there, its play step notwithstanding.
    await next(page);
    await expect.poll(async () => (await clipOf(page, 's4'))?.paused, { timeout: 5000 }).toBe(true);
    expect((await clipOf(page, 's4'))?.time ?? 0).toBeGreaterThan(0.9);
    await next(page);
    await page.waitForTimeout(500);
    const ended = await clipOf(page, 's5');
    expect(ended?.paused).toBe(true);
    expect(ended?.time ?? 0, 'holds the end').toBeGreaterThan(0.9);
  });

  test('the incoming slide shows once its clip stands where the outgoing one does, not at its first frame', async ({
    page,
  }) => {
    await serve(page, { 'long.wav': wav(6) });
    const ids = ['s0', 's1', 's2'];
    const html = { s1: withClip('s1', 'long.wav', true), s2: withClip('s2', 'long.wav', true) };
    await open(page, {
      ...fixture(
        ids.map((id) => deckSlide(id, id === 's0' ? {} : { timeline: [play(id)] })),
        html,
      ),
      mediaBase: MEDIA,
    });
    await next(page);
    await expect.poll(async () => (await clipOf(page, 's1'))?.time ?? 0).toBeGreaterThan(1.2);
    // What the incoming clip shows the moment its slide becomes the shown one: the last seek that landed.
    await page.evaluate(() => {
      const layer = document.querySelector('.sd-layer[data-sd-layer="s2"]') as HTMLElement;
      const audio = layer.shadowRoot?.querySelector('audio') as HTMLAudioElement;
      const probe: { landed: number | null; shown: number | null | undefined } = { landed: null, shown: undefined };
      (window as unknown as { probe: typeof probe }).probe = probe;
      audio.addEventListener('seeked', () => (probe.landed = audio.currentTime));
      new MutationObserver(() => {
        if (layer.hasAttribute('data-sd-current') && probe.shown === undefined) {
          probe.shown = probe.landed;
        }
      }).observe(layer, { attributes: true, attributeFilter: ['data-sd-current'] });
    });
    await next(page);
    const shown = await page.evaluate(() => (window as unknown as { probe: { shown: number | null } }).probe.shown);
    expect(shown, 'the outgoing position, sought and landed before the slide shows').toBeGreaterThan(1.2);
  });

  test('a clip playing at the move plays along through the transition and goes on from its own frame', async ({
    page,
  }) => {
    await serve(page, { 'long.wav': wav(8) });
    const fade = transition({ effect: 'fade', params: { through: 'none' }, durationMs: 2000 });
    const slides = [
      deckSlide('s0'),
      deckSlide('s1', { timeline: [play('s1')] }),
      deckSlide('s2', { timeline: [play('s2')], transition: fade }),
    ];
    const html = { s1: withClip('s1', 'long.wav', true), s2: withClip('s2', 'long.wav', true) };
    await open(page, { ...fixture(slides, html), mediaBase: MEDIA });
    await next(page);
    await expect.poll(async () => (await clipOf(page, 's1'))?.time ?? 0).toBeGreaterThan(1.2);
    // Seeks of the incoming clip once it plays along: each is a stall and a jump in what the move shows.
    await page.evaluate(() => {
      const audio = document
        .querySelector('.sd-layer[data-sd-layer="s2"]')
        ?.shadowRoot?.querySelector('audio') as HTMLAudioElement;
      const probe = { armed: false, seeks: 0 };
      (window as unknown as { probe: typeof probe }).probe = probe;
      audio.addEventListener('seeking', () => {
        if (probe.armed) {
          probe.seeks++;
        }
      });
    });
    await page.evaluate(() => (window as unknown as Harness).sd.player.next());
    await page.waitForFunction(() => (window as unknown as Harness).sd.count() > 0);
    const start = await clipOf(page, 's2');
    await page.evaluate(() => ((window as unknown as { probe: { armed: boolean } }).probe.armed = true));
    await page.waitForTimeout(1000);
    const [mid, out] = [await clipOf(page, 's2'), await clipOf(page, 's1')];
    expect(mid?.paused, 'the incoming clip plays along through the transition').toBe(false);
    expect((mid?.time ?? 0) - (start?.time ?? 0), 'not held at the frame the move began on').toBeGreaterThan(0.5);
    expect(Math.abs((mid?.time ?? 0) - (out?.time ?? 99)), 'in step with the outgoing clip').toBeLessThan(0.5);
    await settle(page);
    const after = await clipOf(page, 's2');
    expect(after?.paused).toBe(false);
    expect(after?.time ?? 0, 'goes on from its own frame, not back to an earlier one').toBeGreaterThanOrEqual(
      mid?.time ?? 99,
    );
    expect(await page.evaluate(() => (window as unknown as { probe: { seeks: number } }).probe.seeks)).toBe(0);
    expect(await clipOf(page, 's1'), 'the outgoing clip stops and rewinds').toMatchObject({ paused: true, time: 0 });
  });

  test('a forward move dropped mid-transition leaves the incoming clip at its start', async ({ page }) => {
    await serve(page, { 'long.wav': wav(6) });
    const fade = transition({ effect: 'fade', params: { through: 'none' }, durationMs: 3000 });
    const slides = [
      deckSlide('s0'),
      deckSlide('s1', { timeline: [play('s1')] }),
      deckSlide('s2', { timeline: [play('s2')], transition: fade }),
    ];
    const html = { s1: withClip('s1', 'long.wav', true), s2: withClip('s2', 'long.wav', true) };
    await open(page, { ...fixture(slides, html), mediaBase: MEDIA });
    await next(page);
    await expect.poll(async () => (await clipOf(page, 's1'))?.time ?? 0).toBeGreaterThan(1.2);
    await page.evaluate(() => (window as unknown as Harness).sd.player.next());
    await page.waitForFunction(() => (window as unknown as Harness).sd.count() > 0);
    expect((await clipOf(page, 's2'))?.time ?? 0, 'sought for the hand-over').toBeGreaterThan(1.2);
    await page.evaluate(() => (window as unknown as Harness).sd.player.previous());
    await settle(page);
    expect(await clipOf(page, 's2'), 'not shown: at its start, as on any other slide').toMatchObject({
      paused: true,
      time: 0,
    });
  });

  test("the next slide's clip loads ahead only when a clip shown can carry into it, and only while presenting", async ({
    page,
  }) => {
    await serve(page, { 'long.wav': wav(6), 'short.wav': wav(1) });
    const ids = ['s0', 's1', 's2', 's3'];
    const html = {
      s1: withClip('s1', 'long.wav', true),
      s2: withClip('s2', 'long.wav', true),
      s3: withClip('s3', 'short.wav', true),
    };
    const preloadOf = (id: string) =>
      page.evaluate(
        (slide) =>
          document.querySelector(`.sd-layer[data-sd-layer="${slide}"]`)?.shadowRoot?.querySelector('audio')?.preload,
        id,
      );
    const data = {
      ...fixture(
        ids.map((id) => deckSlide(id)),
        html,
      ),
      mediaBase: MEDIA,
    };
    for (const mode of ['present', 'view'] as const) {
      await open(page, data, { mode });
      expect(await preloadOf('s1'), `${mode}: nothing shown plays it`).toBe('none');
      await next(page);
      expect(await preloadOf('s2'), `${mode}: s1's clip can carry into it`).toBe(mode === 'present' ? 'auto' : 'none');
      await next(page);
      expect(await preloadOf('s3'), `${mode}: another clip`).toBe('none');
    }
  });
});

test.describe('magic move', () => {
  // The hero moves from (60, 60) 120 x 60 pt to (420, 220) 240 x 120 pt; each slide has one unmatched box.
  const outgoing = boxes('s1', '#ffffff', [
    box('s1.hero', 60, 60, 120, 60, '#2563eb'),
    box('s1.only', 480, 280, 90, 60, '#dc2626'),
  ]);
  const incoming = (heroes = 1) =>
    boxes('s2', '#f1f5f9', [
      box('s2.hero', 420, 220, 240, 120, '#16a34a'),
      ...(heroes > 1 ? [box('s2.twin', 60, 200, 60, 60, '#0891b2')] : []),
      box('s2.only', 60, 300, 90, 60, '#f59e0b'),
    ]);
  const PT = 4 / 3;
  const move = (effect: 'magicMove' | 'morph') =>
    transition({
      ...(effect === 'morph'
        ? { effect, params: { by: 'object' as const } }
        : { effect, params: { fadeUnmatched: true } }),
      durationMs: 1000,
      easing: 'linear',
    });

  /** A slide layer's own opacity. */
  const layerOpacity = (page: Page, layer: string) =>
    page.evaluate(
      (l) => Number(getComputedStyle(document.querySelector(`.sd-layer[data-sd-layer="${l}"]`) as Element).opacity),
      layer,
    );
  /** The colour of the stage at (x, y) slide px. */
  const pixel = async (page: Page, x: number, y: number) =>
    page.evaluate(
      async ([b64, px, py]) => {
        const bitmap = await createImageBitmap(await (await fetch(`data:image/png;base64,${b64}`)).blob());
        const ctx = new OffscreenCanvas(bitmap.width, bitmap.height).getContext('2d');
        ctx?.drawImage(bitmap, 0, 0);
        const d = ctx?.getImageData(Math.round(px), Math.round(py), 1, 1).data;
        return [d?.[0] ?? -1, d?.[1] ?? -1, d?.[2] ?? -1];
      },
      [await shot(page), x, y] as const,
    );

  /** Boxes (slide px) and opacities of elements of a slide layer while paused. */
  const measure = (page: Page, layer: string, ids: string[]) =>
    page.evaluate(
      ([l, list]) => {
        const host = document.querySelector(`.sd-layer[data-sd-layer="${l}"]`) as HTMLElement;
        const frame = host.getBoundingClientRect();
        const scale = frame.width / 960;
        return Object.fromEntries(
          list.map((id) => {
            const el = [...(host.shadowRoot?.querySelectorAll('[data-el]') ?? [])].find(
              (e) => e.getAttribute('data-el') === id,
            ) as HTMLElement;
            const r = el.getBoundingClientRect();
            return [
              id,
              {
                cx: (r.left - frame.left + r.width / 2) / scale,
                cy: (r.top - frame.top + r.height / 2) / scale,
                w: r.width / scale,
                h: r.height / scale,
                opacity: Number(getComputedStyle(el).opacity),
              },
            ];
          }),
        );
      },
      [layer, ids] as const,
    );

  for (const keys of ['matchKeys', 'data-sd-match'] as const) {
    test(`FLIP frames between matched elements, unmatched ones fade (${keys})`, async ({ page }) => {
      const byKeys = keys === 'matchKeys';
      const data = fixture(
        [
          deckSlide('s1', byKeys ? { matchKeys: { 's1.hero': 'hero' } } : {}),
          deckSlide('s2', { transition: move('magicMove'), ...(byKeys ? { matchKeys: { 's2.hero': 'hero' } } : {}) }),
        ],
        byKeys
          ? { s1: outgoing, s2: incoming() }
          : {
              s1: outgoing.replace('data-el="s1.hero"', 'data-el="s1.hero" data-sd-match="hero"'),
              s2: incoming().replace('data-el="s2.hero"', 'data-el="s2.hero" data-sd-match="hero"'),
            },
      );
      await open(page, data);
      const before = await staticShot(page, 0, 0);
      const after = await staticShot(page, 1, 0);
      await page.evaluate(() => (window as unknown as Harness).sd.player.next());
      await pauseOnStart(page);
      expect(await disallowedKeys(page)).toEqual([]);
      const seek = (f: number) => page.evaluate((x) => (window as unknown as Harness).sd.seek(x), f);

      await seek(0);
      expect(await diff(page, await shot(page), before, AA), 'first frame = outgoing slide').toBeLessThanOrEqual(
        SPECKS,
      );
      const from = { cx: (60 + 60) * PT, cy: (60 + 30) * PT, w: 120 * PT, h: 60 * PT };
      const to = { cx: (420 + 120) * PT, cy: (220 + 60) * PT, w: 240 * PT, h: 120 * PT };
      for (const f of [0.25, 0.5]) {
        await seek(f);
        const out = await measure(page, 's1', ['s1.hero', 's1.only']);
        const into = await measure(page, 's2', ['s2.hero', 's2.only']);
        const hero = into['s2.hero'];
        expect(hero?.cx).toBeCloseTo(from.cx + (to.cx - from.cx) * f, 0);
        expect(hero?.cy).toBeCloseTo(from.cy + (to.cy - from.cy) * f, 0);
        expect(hero?.w).toBeCloseTo(from.w + (to.w - from.w) * f, 0);
        expect(hero?.h).toBeCloseTo(from.h + (to.h - from.h) * f, 0);
        // The outgoing hero travels with its match, over it, as the outgoing slide fades out.
        for (const side of ['cx', 'cy', 'w', 'h'] as const) {
          expect(out['s1.hero']?.[side], `the outgoing hero's ${side} at ${f}`).toBeCloseTo(
            hero?.[side] ?? Number.NaN,
            0,
          );
        }
        expect(out['s1.hero']?.opacity).toBe(1);
        expect(await layerOpacity(page, 's1'), 'the outgoing slide fades as one picture').toBeCloseTo(1 - f, 2);
        expect(hero?.opacity).toBe(1);
        // Keynote's thirds, linear here: the unmatched outgoing box is out by a third, the incoming one waits.
        expect(out['s1.only']?.opacity).toBeCloseTo(Math.max(0, 1 - 3 * f), 2);
        expect(into['s2.only']?.opacity).toBe(0);
      }
      // So the heroes mix half and half halfway, with nothing of what is behind them.
      const mixed = await pixel(page, (from.cx + to.cx) / 2, (from.cy + to.cy) / 2);
      const half = [0x25 + 0x16, 0x63 + 0xa3, 0xeb + 0x4a].map((v) => v / 2);
      for (const [i, channel] of ['red', 'green', 'blue'].entries()) {
        expect(Math.abs((mixed[i] ?? -1) - (half[i] ?? 0)), `${channel} ${mixed.join(',')}`).toBeLessThanOrEqual(3);
      }
      // Keynote's last third: the incoming unmatched box is a quarter in at 0.75.
      await seek(0.75);
      expect((await measure(page, 's2', ['s2.only']))['s2.only']?.opacity).toBeCloseTo(0.25, 2);
      await seek(1);
      expect(await diff(page, await shot(page), after, AA), 'last frame = incoming slide').toBeLessThanOrEqual(SPECKS);
      await page.evaluate(() => (window as unknown as Harness).sd.resume());
      await settle(page);
      expect(await diff(page, await shot(page), after), 'settled = renderStatic').toBe(0);
    });
  }

  // Only Keynote's Magic Move with fadeUnmatched holds the incoming slide's unmatched objects back to the last third: a
  // Morph fades them in over the whole move, a Magic Move that does not fade them shows them at once, and an element
  // on both slides (a layout's) shows throughout.
  for (const [name, effect, fade] of [
    ['a Morph', 'morph', true],
    ['a Magic Move that does not fade unmatched objects', 'magicMove', false],
    ['a Magic Move, an element on both slides', 'magicMove', true],
  ] as const) {
    test(`${name}: the incoming slide's unmatched objects come in as it says`, async ({ page }) => {
      const shared = box('layout.mark', 300, 20, 60, 30, '#7c3aed');
      const data = fixture(
        [
          deckSlide('s1', { matchKeys: { 's1.hero': 'hero' } }),
          deckSlide('s2', {
            transition: transition({
              ...(effect === 'morph'
                ? { effect, params: { by: 'object' as const } }
                : { effect, params: { fadeUnmatched: fade } }),
              durationMs: 1000,
              easing: 'linear',
            }),
            matchKeys: { 's2.hero': 'hero' },
          }),
        ],
        {
          s1: outgoing.replace('</main>', `${shared}</main>`),
          s2: incoming().replace('</main>', `${shared}</main>`),
        },
      );
      await open(page, data);
      await page.evaluate(() => (window as unknown as Harness).sd.player.next());
      await pauseOnStart(page);
      await page.evaluate(() => (window as unknown as Harness).sd.seek(0.25));
      const into = await measure(page, 's2', ['s2.hero', 's2.only', 'layout.mark']);
      expect(into['layout.mark']?.opacity, 'the element on both slides').toBe(1);
      const only = { morph: 0.25, magicMove: fade ? 0 : 1 }[effect];
      expect(into['s2.only']?.opacity, 'the unmatched incoming box').toBeCloseTo(only, 2);
      expect(into['s2.hero']?.opacity, 'the incoming hero, beneath the outgoing one').toBe(1);
      await page.evaluate(() => (window as unknown as Harness).sd.resume());
      await settle(page);
      expect(await diff(page, await shot(page), await staticShot(page, 1, 0))).toBe(0);
    });
  }

  test('a translucent match moves without doubling, its opacity on the curve, and a match over another shows nothing of it', async ({
    page,
  }) => {
    // A half-opaque blue card moves right off an unmatched yellow photo and is opaque at the end; an opaque amber
    // square over a matched green panel moves down with it.
    const card = (id: string, x: number, opacity: number) =>
      box(id, x, 20, 120, 60, '#2563eb', `data-sd-match="card"`).replace(
        'background:',
        `opacity:${opacity};background:`,
      );
    const panel = (id: string, y: number) => box(id, 300, y, 200, 120, '#16a34a', 'data-sd-match="panel"');
    const square = (id: string, y: number) => box(id, 360, y + 30, 60, 60, '#f59e0b', 'data-sd-match="square"');
    const data = fixture([deckSlide('s1'), deckSlide('s2', { transition: move('magicMove') })], {
      s1: boxes('s1', '#ffffff', [
        box('s1.photo', 20, 10, 300, 90, '#facc15'),
        card('s1.card', 40, 0.5),
        panel('s1.panel', 120),
        square('s1.square', 120),
      ]),
      s2: boxes('s2', '#ffffff', [card('s2.card', 440, 1), panel('s2.panel', 260), square('s2.square', 260)]),
    });
    await open(page, data);
    const before = await staticShot(page, 0, 0);
    await page.evaluate(() => (window as unknown as Harness).sd.player.next());
    await pauseOnStart(page);
    const seek = (f: number) => page.evaluate((x) => (window as unknown as Harness).sd.seek(x), f);
    await seek(0);
    expect(await diff(page, await shot(page), before, AA), 'first frame = outgoing slide').toBeLessThanOrEqual(SPECKS);
    await seek(0.5);
    expect((await measure(page, 's2', ['s2.card']))['s2.card']?.cx).toBeCloseTo((40 + 440 + 120) * 0.5 * PT, 0);
    // The card at 0.75 on white: no second card.
    const cardAt = await pixel(page, (40 + 440 + 120) * 0.5 * PT, 50 * PT);
    const square0 = await pixel(page, 390 * PT, (190 + 140 / 2) * PT);
    const blue = [0x25, 0x63, 0xeb].map((v) => 0.75 * v + 0.25 * 0xff);
    const amber = [0xf5, 0x9e, 0x0b];
    for (const [i, channel] of ['red', 'green', 'blue'].entries()) {
      expect(Math.abs((cardAt[i] ?? -1) - (blue[i] ?? 0)), `card ${channel} ${cardAt.join(',')}`).toBeLessThanOrEqual(
        3,
      );
      expect(
        Math.abs((square0[i] ?? -1) - (amber[i] ?? 0)),
        `square ${channel} ${square0.join(',')}`,
      ).toBeLessThanOrEqual(3);
    }
    await page.evaluate(() => (window as unknown as Harness).sd.resume());
    await settle(page);
    expect(await diff(page, await shot(page), await staticShot(page, 1, 0))).toBe(0);
  });

  test('a key on two elements matches nothing: the outgoing element fades where it is', async ({ page }) => {
    const data = fixture(
      [
        deckSlide('s1', { matchKeys: { 's1.hero': 'hero' } }),
        deckSlide('s2', { transition: move('morph'), matchKeys: { 's2.hero': 'hero', 's2.twin': 'hero' } }),
      ],
      { s1: outgoing, s2: incoming(2) },
    );
    await open(page, data);
    await page.evaluate(() => (window as unknown as Harness).sd.player.next());
    await pauseOnStart(page);
    await page.evaluate(() => (window as unknown as Harness).sd.seek(0.5));
    const out = await measure(page, 's1', ['s1.hero']);
    expect(out['s1.hero']?.cx).toBeCloseTo(120 * PT, 0);
    expect(out['s1.hero']?.w).toBeCloseTo(120 * PT, 0);
    expect(out['s1.hero']?.opacity).toBeCloseTo(0.5, 2);
    await page.evaluate(() => (window as unknown as Harness).sd.resume());
    await settle(page);
    expect(await diff(page, await shot(page), await staticShot(page, 1, 0))).toBe(0);
  });

  test('reduced motion plays the 150 ms fade instead', async ({ page }) => {
    const data = fixture(
      [
        deckSlide('s1', { matchKeys: { 's1.hero': 'hero' } }),
        deckSlide('s2', { transition: move('magicMove'), matchKeys: { 's2.hero': 'hero' } }),
      ],
      { s1: outgoing, s2: incoming() },
    );
    await open(page, data, { reducedMotion: 'always' });
    await page.evaluate(() => (window as unknown as Harness).sd.player.next());
    // Paused as it starts: on a loaded machine the 150 ms fade could otherwise end before it is read.
    await pauseOnStart(page);
    const played = await page.evaluate(() =>
      document.getAnimations().map((a) => ({
        duration: (a.effect as KeyframeEffect).getTiming().duration,
        keys: (a.effect as KeyframeEffect).getKeyframes().flatMap((k) => Object.keys(k)),
      })),
    );
    expect(played).toHaveLength(1);
    expect(played[0]?.duration).toBe(150);
    expect(played[0]?.keys).not.toContain('transform');
    await page.evaluate(() => (window as unknown as Harness).sd.resume());
    await settle(page);
    expect(await diff(page, await shot(page), await staticShot(page, 1, 0))).toBe(0);
  });
});

test.describe('presenter view', () => {
  test('the next-click preview shows the next build step, then the next slide', async ({ page }) => {
    const timeline = [
      step('s1.box', fields({ effect: 'flyIn', params: { dir: 'u', distance: 'offSlide' } })),
      step('s1.text', fields({ effect: 'fade' })),
    ];
    const data = fixture([deckSlide('s1', { timeline }), deckSlide('s2', { transition: transition({}) })]);
    await page.setViewportSize({ width: 1400, height: 900 });
    await open(page, data);
    // The presenter view takes the page; the harness's own player stays for the still renders.
    await page.evaluate(() => {
      const { sd } = window as unknown as Harness;
      const host = document.createElement('div');
      host.id = 'presenter';
      document.body.append(host);
      const view = (
        window as unknown as { SlidockPresenter: { mount(h: HTMLElement, s: unknown, o: unknown): unknown } }
      ).SlidockPresenter.mount(host, sd.source, { session: 'e2e', reducedMotion: 'always' });
      (window as unknown as { view: unknown }).view = view;
    });
    const label = page.locator('.sd-presenter .sd-next .sd-label').first();
    const preview = page.locator('.sd-presenter .sd-next > div');
    /** The preview, and a still render of (slide, step) drawn at the preview's size beside it. */
    const compare = async (slide: number, step: number) => {
      await page.evaluate(() => document.getElementById('expected')?.remove());
      await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
      const box = await preview.boundingBox();
      if (box === null) {
        throw new Error('no preview');
      }
      const a = (await page.screenshot({ clip: box })).toString('base64');
      // Drawn over the preview at the same place, so text rasterizes on the same pixel grid.
      await page.evaluate(
        ([s, t, at]) => {
          const host = document.createElement('div');
          host.id = 'expected';
          host.style.cssText = `position:fixed;left:${at.x}px;top:${at.y}px;width:${at.width}px;height:${at.height}px;background:#000;z-index:20`;
          document.body.append(host);
          (
            window as unknown as { sd: { player: { renderStatic(a: number, b: number, h: HTMLElement): unknown } } }
          ).sd.player.renderStatic(s, t, host);
        },
        [slide, step, box] as const,
      );
      await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
      const b = (await page.screenshot({ clip: box })).toString('base64');
      await page.evaluate(() => document.getElementById('expected')?.remove());
      // Edge antialiasing of the scaled-down rotated shape may differ by a few levels between two layers.
      return diff(page, a, b, 16);
    };
    const presenterNext = () =>
      page.evaluate(() => (window as unknown as { view: { player: { next(): void } } }).view.player.next());
    await expect(label).toHaveText('Next: build 1 of 2');
    await expect(page.locator('.sd-next')).toHaveAttribute('data-sd-next', 'build');
    expect(await compare(0, 1)).toBe(0);
    expect(await compare(0, 0), 'not the current step').toBeGreaterThan(0);
    await presenterNext();
    await expect(label).toHaveText('Next: build 2 of 2');
    expect(await compare(0, 2)).toBe(0);
    await presenterNext();
    await expect(label).toHaveText('Next: slide 2');
    await expect(page.locator('.sd-next')).toHaveAttribute('data-sd-next', 'slide');
    expect(await compare(1, 0)).toBe(0);
    await presenterNext();
    await expect(page.locator('.sd-next')).toHaveAttribute('data-sd-next', 'end');
    await expect(page.locator('.sd-next')).toContainText('End of presentation');
  });
});
