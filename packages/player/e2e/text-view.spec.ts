// The deck page with its text view below the stage (docs/format.md §2), from file:// with this build's player: the
// stage is the first screen and the view follows it, and the slides draw as on a page without a view; `#slide-<n>` deep
// links start there and follow the show; a heading of the view plays its slide; keys scroll the page while the view
// is in sight, and Enter follows the focused Text link; the presenter window holds no text view; and slide elements'
// `data-link` (URL, slide, action) work. The link deck is shrink-probe's first slide with links of every kind, as IR.
import { type Page, expect, test } from '@playwright/test';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { OUT, REPO, cli, compile, embedPlayer } from './support/cli';

test.describe.configure({ timeout: 5 * 60_000 });

let motion = '';
let bare = '';
let links = '';
let shown = 0;
let size = { w: 0, h: 0 };

/** A compiled deck's page with this build's player; returns its file URL. */
function deckPage(dir: string): string {
  const path = join(dir, 'index.html');
  writeFileSync(path, embedPlayer(readFileSync(path, 'utf8')));
  return pathToFileURL(path).href;
}

/** Shrink-probe's first slide, then two copies: on the first, run links (URL, slide, action) and element links. */
function linkDeck(binary: string): string {
  const dir = mkdtempSync(join(OUT, 'links-ir-'));
  const run = spawnSync(binary, ['compile', join(REPO, 'probes/out/shrink-probe.pptx'), '--emit', 'ir', '-o', dir], {
    encoding: 'utf8',
    maxBuffer: 64 * 1024 * 1024,
  });
  if (run.status !== 0) {
    throw new Error(`slidock compile --emit ir exited ${run.status}: ${run.stderr.slice(-2000)}`);
  }
  const path = join(dir, 'presentation-ir.json');
  type Inline = { kind: string; props: Record<string, unknown> };
  type Element = { id: string; link?: unknown; text?: { paragraphs: { inlines: Inline[] }[] } };
  type Slide = { id: string; elements: Element[] };
  const ir = JSON.parse(readFileSync(path, 'utf8')) as { slides: Slide[] };
  const first = ir.slides[0];
  if (first === undefined) {
    throw new Error('shrink-probe has no slide');
  }
  for (const id of ['pp-257', 'pp-258']) {
    const copy = JSON.parse(JSON.stringify(first)) as Slide;
    copy.id = id;
    copy.elements.forEach((element, i) => (element.id = `${id}.${i + 2}`));
    ir.slides.push(copy);
  }
  const kinds: [string, Record<string, string>][] = [
    ['run', { kind: 'url', url: 'https://example.com/run-link' }],
    ['run', { kind: 'slide', slide: 'pp-258' }],
    ['run', { kind: 'action', action: 'last' }],
    ['element', { kind: 'url', url: 'https://example.com/element-link' }],
    ['element', { kind: 'slide', slide: 'pp-258' }],
  ];
  kinds.forEach(([where, link], i) => {
    const element = first.elements[i];
    if (element === undefined) {
      throw new Error('shrink-probe has too few elements');
    }
    if (where === 'element') {
      element.link = link;
    } else {
      const run = element.text?.paragraphs.flatMap((p) => p.inlines).find((inline) => inline.kind === 'run');
      if (run === undefined) {
        throw new Error(`${element.id} has no run`);
      }
      run.props.link = link;
    }
  });
  writeFileSync(path, JSON.stringify(ir));
  return compile(binary, relative(REPO, path), 'links');
}

test.beforeAll(() => {
  test.setTimeout(30 * 60_000);
  const binary = cli();
  const dir = compile(binary, 'probes/out/motion-probe.pptx', 'text-view');
  motion = deckPage(dir);
  bare = deckPage(compile(binary, 'probes/out/motion-probe.pptx', 'no-text-view', ['--no-text-view']));
  const deck = JSON.parse(
    /<script type="application\/json" id="sd-deck">([\s\S]*?)<\/script>/.exec(
      readFileSync(join(dir, 'index.html'), 'utf8'),
    )?.[1] ?? 'null',
  ) as { slides: { hidden: boolean }[]; size: { w: number; h: number } };
  shown = deck.slides.filter((slide) => !slide.hidden).length;
  size = { w: (deck.size.w * 4) / 3, h: (deck.size.h * 4) / 3 };
  links = deckPage(linkDeck(binary));
});

/** Waits for the counter to read `label`. */
async function counter(page: Page, label: string): Promise<void> {
  await expect(page.locator('.sd-counter')).toHaveText(label);
}

/** Waits until nothing animates on the deck page (a key during a transition finishes it instead of advancing). */
async function idle(page: Page): Promise<void> {
  await page.waitForFunction(
    () =>
      [...document.querySelectorAll('.sd-layer')].every((layer) =>
        (layer.shadowRoot?.getAnimations() ?? []).every((a) => a.playState !== 'running'),
      ) &&
      // The chrome's own fades aside.
      document.getAnimations().every((a) => {
        const target = (a.effect as KeyframeEffect | null)?.target;
        return a.playState !== 'running' || target?.matches('.sd-counter, .sd-to-text') === true;
      }),
  );
}

/** The key that moves focus to the next link or button: Tab skips them in WebKit on macOS, Alt+Tab does not. */
function tabKey(): string {
  return test.info().project.name === 'webkit' && process.platform === 'darwin' ? 'Alt+Tab' : 'Tab';
}

async function open(page: Page, url: string): Promise<void> {
  await page.goto(url);
  await page.waitForSelector('.sd-layer[data-sd-current]');
}

test('the stage is the first screen and the text view follows it', async ({ page }) => {
  await page.setViewportSize({ width: 1366, height: 768 });
  await open(page, motion);
  const layout = await page.evaluate(() => {
    const frame = document.querySelector('.sd-frame')?.getBoundingClientRect();
    return {
      first: document.body.firstElementChild?.classList.contains('sd-player') ?? false,
      scrolls: document.documentElement.scrollHeight > window.innerHeight,
      frame: frame === undefined ? null : [frame.x, frame.y, frame.width, frame.height],
      viewTop: document.getElementById('sd-text')?.getBoundingClientRect().top ?? null,
      inner: [window.innerWidth, window.innerHeight],
    };
  });
  expect(layout.first).toBe(true);
  expect(layout.scrolls).toBe(true);
  // The frame letterboxed in the window, as when the stage covered the page.
  const [w, h] = layout.inner as [number, number];
  const scale = Math.min(w / size.w, h / size.h);
  const expected = [(w - size.w * scale) / 2, (h - size.h * scale) / 2, size.w * scale, size.h * scale];
  layout.frame?.forEach((value, i) => expect(value).toBeCloseTo(expected[i] ?? NaN, 3));
  expect(layout.viewTop).toBe(h);
});

test('the text view does not move when the stage mounts', async ({ page }) => {
  await page.setViewportSize({ width: 412, height: 839 });
  // The page as the browser shows it before the player's script has run (it comes last, after every template).
  const path = fileURLToPath(motion);
  const html = readFileSync(path, 'utf8');
  const boot = html.lastIndexOf('<script>');
  const unbooted = join(dirname(path), 'unbooted.html');
  writeFileSync(unbooted, html.slice(0, boot) + html.slice(html.indexOf('</script>', boot) + '</script>'.length));
  await page.goto(pathToFileURL(unbooted).href);
  const before = await page.evaluate(() => document.getElementById('sd-text')?.getBoundingClientRect().top);
  await page.addInitScript(() => {
    const shifts: number[] = [];
    (window as unknown as { shifts: number[] }).shifts = shifts;
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        shifts.push((entry as unknown as { value: number }).value);
      }
    }).observe({ type: 'layout-shift', buffered: true });
  });
  await open(page, motion);
  const after = await page.evaluate(() => document.getElementById('sd-text')?.getBoundingClientRect().top);
  expect(before).toBe(839);
  expect(after).toBe(839);
  if (test.info().project.name === 'chromium') {
    expect(await page.evaluate(() => (window as unknown as { shifts: number[] }).shifts)).toEqual([]);
  }
});

test('the slides draw as on a page whose stage covers the window, at a fractional device pixel ratio', async ({
  browser,
}) => {
  // Emphasis builds' end states (motion-probe's emphasis and motion path presets), where Chromium drew softer text and
  // edges when the stage was in the page flow.
  const slides = [151, 163, 184];
  // WebKit draws the window's top-left CSS pixel of any page that scrolls differently from one that does not (a page
  // without a view made scrollable does the same, and `html{overflow:hidden}` on this one draws it as before): its
  // shots leave out the top rows. A clip edge between device pixels (412 CSS px are 1,081.5 at 2.625) leaves its edge
  // pixels part-covered, which macOS WebKit blends differently on a page that scrolls: the clip keeps to whole device
  // pixels (8 CSS px are 21).
  const webkit = test.info().project.name.startsWith('webkit');
  const clip = { x: 0, y: 8, width: 408, height: 824 };
  const shots = async (url: string, css = ''): Promise<Buffer[]> => {
    const context = await browser.newContext({
      viewport: { width: 412, height: 839 },
      deviceScaleFactor: 2.625,
      ...(test.info().project.name.startsWith('webkit') ? {} : { isMobile: true }),
      hasTouch: true,
      reducedMotion: 'reduce',
    });
    await context.addInitScript(() => {
      let value: Record<string, unknown> | undefined;
      Object.defineProperty(window, 'Slidock', {
        configurable: true,
        get: () => value,
        set: (slidock: Record<string, unknown> & { mount: (...args: unknown[]) => unknown }) => {
          value = {
            ...slidock,
            mount: (...args: unknown[]) => {
              const player = slidock.mount(...args);
              (window as unknown as { player: unknown }).player = player;
              return player;
            },
          };
        },
      });
    });
    const page = await context.newPage();
    await open(page, url);
    await page.addStyleTag({ content: `.sd-counter,.sd-to-text{display:none!important}${css}` });
    const out: Buffer[] = [];
    for (const slide of slides) {
      await page.evaluate(async (slide) => {
        (window as unknown as { player: { goTo(slide: number, step: number): void } }).player.goTo(slide, 999);
        await document.fonts.ready;
        await new Promise((resolve) => setTimeout(resolve, 400));
        await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      }, slide);
      out.push(await page.screenshot({ animations: 'disabled', ...(webkit ? { clip } : {}) }));
    }
    await context.close();
    return out;
  };
  // The reference: a page without a view, its stage covering a window with nothing to scroll.
  const covering =
    '.sd-player.sd-page{position:fixed!important;inset:0!important;width:auto!important;height:auto!important;will-change:auto!important}';
  const [withView, without] = [await shots(motion), await shots(bare, covering)];
  withView.forEach((shot, i) => expect(shot.equals(without[i] ?? Buffer.alloc(0)), `slide ${slides[i]}`).toBe(true));
});

test('Enter on the focused Text link goes to the text view', async ({ page }) => {
  await open(page, motion);
  await counter(page, `1 / ${shown}`);
  await idle(page);
  await page.keyboard.press(tabKey());
  await expect(page.locator('.sd-to-text')).toBeFocused();
  await page.keyboard.press('Enter');
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('#sd-text');
  await expect.poll(() => page.evaluate(() => document.getElementById('sd-text')?.getBoundingClientRect().top)).toBe(0);
  await counter(page, `1 / ${shown}`);
});

test('the presenter window holds no text view', async ({ page }) => {
  await page.goto(`${motion}#presenter=e2e`);
  await page.waitForSelector('.sd-presenter .sd-layer[data-sd-current]');
  const time = page.locator('.sd-presenter .sd-time').first();
  await expect(time).toHaveText(`Slide 1 of ${shown}`);
  expect(await page.locator('#sd-text').count()).toBe(0);
  expect(await page.evaluate(() => document.documentElement.scrollHeight)).toBe(
    await page.evaluate(() => window.innerHeight),
  );
  // Tab reaches the presenter's own controls, and the arrows still drive the show.
  await page.keyboard.press(tabKey());
  expect(await page.evaluate(() => document.activeElement?.tagName)).toBe('BUTTON');
  await page.keyboard.press('ArrowRight');
  await expect(time).toHaveText(`Slide 2 of ${shown}`);
});

test('#slide-<n> starts there and the hash follows the show', async ({ page }) => {
  await open(page, `${motion}#slide-3`);
  await counter(page, `3 / ${shown}`);
  expect(await page.evaluate(() => window.scrollY)).toBe(0);
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('#slide-3');
  await idle(page);
  await page.keyboard.press('ArrowRight');
  await counter(page, `4 / ${shown}`);
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('#slide-4');
  const length = await page.evaluate(() => history.length);
  await idle(page);
  await page.keyboard.press('ArrowRight');
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('#slide-5');
  expect(await page.evaluate(() => history.length)).toBe(length);
});

test('a heading of the text view plays its slide', async ({ page }) => {
  await open(page, motion);
  const heading = page.locator('#slide-7 h2 a');
  await heading.scrollIntoViewIfNeeded();
  expect(await page.evaluate(() => window.scrollY)).toBeGreaterThan(0);
  await heading.click();
  await counter(page, `7 / ${shown}`);
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(0);
  // The heading of the slide in the hash already: the same.
  await idle(page);
  await page.keyboard.press('ArrowRight');
  await counter(page, `8 / ${shown}`);
  await page.locator('#slide-8 h2 a').click();
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(0);
  await counter(page, `8 / ${shown}`);
});

test('Space scrolls the page while the text view is in sight', async ({ page }) => {
  await open(page, motion);
  await counter(page, `1 / ${shown}`);
  await page.evaluate(() => window.scrollTo(0, window.innerHeight));
  await page.waitForTimeout(200);
  const before = await page.evaluate(() => window.scrollY);
  await page.keyboard.press('Space');
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBeGreaterThan(before);
  await page.keyboard.press('ArrowDown');
  await page.waitForTimeout(300);
  await counter(page, `1 / ${shown}`);
  // Back at the stage, the keys are the deck's.
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.waitForTimeout(200);
  await page.keyboard.press('Space');
  await counter(page, `2 / ${shown}`);
});

test("slide elements' links open their URL, go to their slide and run their action", async ({ page }) => {
  await page.addInitScript(() => {
    const opened: unknown[][] = [];
    (window as unknown as { opened: unknown[][] }).opened = opened;
    window.open = (...args: unknown[]) => {
      opened.push(args);
      return null;
    };
  });
  const current = '.sd-layer[data-sd-current]';
  await open(page, links);
  await counter(page, '1 / 3');
  await page.locator(`${current} div[data-link^="url:"]`).click();
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { opened: unknown[][] }).opened))
    .toEqual([['https://example.com/element-link', '_blank', 'noopener,noreferrer']]);
  await page.waitForTimeout(300);
  await counter(page, '1 / 3');

  await page.locator(`${current} div[data-link^="slide:"]`).click();
  await counter(page, '3 / 3');

  await page.keyboard.press('Home');
  await counter(page, '1 / 3');
  await page.locator(`${current} a[data-link^="action:"]`).click();
  await counter(page, '3 / 3');

  // The text view links the element URL, and the slide links to its section.
  const view = await page.locator('#sd-text #slide-1').innerHTML();
  expect(view).toContain('href="https://example.com/element-link"');
  expect(view).toContain('href="#slide-3"');
});
