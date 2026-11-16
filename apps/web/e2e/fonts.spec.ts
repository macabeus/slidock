// The fonts flow in the browser. Most tests run on the demo page
// (the dev server's `demo.html`: the app over a fake compile worker that tells a font story, src/testing/sample-fonts.ts),
// so the sources, the panel and the swap of the slides are exercised whatever the wasm side ships today: Google Fonts
// answers with a recorded response, Local Font Access is the browser's own or a stand-in installed before the page
// loads. The last test opens the smoke deck in the production app, with the real worker.
import { type Page, expect, test } from '@playwright/test';
import { mkdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

import { DEMO_PORT, OUT, REPO, compiledDeck, smokeCase } from './support/decks';
import { CURRENT, checkMedia, expectSlide, watch } from './support/player';
import { differentPixels } from './support/png';

const DEMO = `http://localhost:${DEMO_PORT}/demo.html`;
/** OFL (crates/slidock-fonts/tests/data/container/SOURCES.txt): a script face, unmistakable in a screenshot. */
const PACIFICO = join(REPO, 'crates/slidock-fonts/tests/data/container/pacifico.ttf');
const ROBOTO_CSS = join(REPO, 'crates/slidock-fonts/tests/data/google/roboto-browser.css');
/** Served for every fonts.gstatic.com file: a real WOFF2 (OFL), so the chunks load as fonts. */
const CHUNK = join(REPO, 'crates/slidock-substitutes/fonts/Caladea-Regular.woff2');
const SCREENS = join(OUT, 'screens');

/** Every request the page makes, with Google Fonts answered from the recorded response. */
async function recordedGoogle(page: Page): Promise<string[]> {
  const requests: string[] = [];
  page.on('request', (request) => requests.push(request.url()));
  const cors = { 'access-control-allow-origin': '*' };
  await page.route('https://fonts.googleapis.com/**', (route) =>
    route.fulfill({ body: readFileSync(ROBOTO_CSS, 'utf8'), contentType: 'text/css', headers: cors }),
  );
  await page.route('https://fonts.gstatic.com/**', (route) =>
    route.fulfill({ body: readFileSync(CHUNK), contentType: 'font/woff2', headers: cors }),
  );
  return requests;
}

function googleRequests(requests: readonly string[]): URL[] {
  return requests
    .map((url) => new URL(url))
    .filter((url) => url.hostname.endsWith('.googleapis.com') || url.hostname === 'fonts.gstatic.com');
}

/** Every Google request is the css2 stylesheet without `text` or a gstatic file, and none spells the deck's text. */
function expectNoText(requests: readonly string[]) {
  for (const url of googleRequests(requests)) {
    expect(url.searchParams.has('text'), url.href).toBe(false);
    expect(url.href).not.toMatch(/Quarterly|Highlights|Thank/);
    expect(url.pathname === '/css2' || url.hostname === 'fonts.gstatic.com', url.href).toBe(true);
  }
}

type FakeState = 'prompt' | 'granted' | 'denied' | 'error';

/**
 * Installs a stand-in for Local Font Access before the page loads: `queryLocalFonts` and the `local-fonts`
 * permission, with two installed faces whose file is Pacifico.
 */
async function fakeLocalFonts(page: Page, state: FakeState, answer: 'granted' | 'denied' = 'granted') {
  const data = readFileSync(PACIFICO).toString('base64');
  await page.addInitScript(
    ({ state, answer, data }) => {
      let current = state === 'error' ? 'prompt' : state;
      const bytes = Uint8Array.from(atob(data), (char) => char.charCodeAt(0));
      const face = (postscriptName: string, family: string) => ({
        postscriptName,
        fullName: family,
        family,
        style: 'Regular',
        blob: async () => new Blob([bytes]),
      });
      const fonts = [
        face('Arial-BoldMT', 'Arial'),
        face('HelveticaNeue', 'Helvetica Neue'),
        face('Optima-Regular', 'Optima'),
      ];
      const target = window as unknown as Record<string, unknown>;
      target.__lfaQueries = 0;
      target.queryLocalFonts = async () => {
        target.__lfaQueries = Number(target.__lfaQueries) + 1;
        if (state === 'error') {
          throw new Error('font service crashed');
        }
        if (current === 'prompt') {
          current = answer;
        }
        if (current === 'denied') {
          throw new DOMException('The user denied the permission', 'NotAllowedError');
        }
        return fonts;
      };
      const permissions = navigator.permissions as Permissions | undefined;
      const query = permissions?.query.bind(permissions);
      const answerQuery = async (descriptor: { name: string }) =>
        descriptor.name === 'local-fonts'
          ? ({ state: current } as PermissionStatus)
          : query!(descriptor as PermissionDescriptor);
      Object.defineProperty(navigator, 'permissions', { value: { query: answerQuery }, configurable: true });
    },
    { state, answer, data },
  );
}

async function openDemo(page: Page, query = '') {
  await page.goto(`${DEMO}?panel=fonts${query}`);
  const drawer = page.getByRole('dialog', { name: 'Fonts' });
  await expect(drawer).toBeVisible();
  return drawer;
}

/** The demo's sources that start on their own (Google, the substitutes) have settled, and the slides with them. */
async function settled(page: Page) {
  const drawer = page.getByRole('dialog', { name: 'Fonts' });
  await expect(drawer.getByText(/draws the faces no source has/)).toBeVisible();
  await expect(drawer.getByText('Updating the slides with the new fonts…')).toHaveCount(0);
}

function faces(page: Page, family: string) {
  return page.getByRole('dialog', { name: 'Fonts' }).getByRole('listitem', { name: family, exact: true });
}

/** Drops `files` (path, name) on the page as a drag from the desktop would. */
async function dropFiles(page: Page, target: string, files: { path: string; name: string }[]) {
  const payload = files.map((file) => ({ name: file.name, data: readFileSync(file.path).toString('base64') }));
  await page.evaluate(
    ({ target, payload }) => {
      const data = new DataTransfer();
      for (const file of payload) {
        data.items.add(new File([Uint8Array.from(atob(file.data), (char) => char.charCodeAt(0))], file.name));
      }
      const element = document.querySelector(target) ?? window;
      for (const type of ['dragenter', 'dragover', 'drop']) {
        element.dispatchEvent(new DragEvent(type, { dataTransfer: data, bubbles: true, cancelable: true }));
      }
    },
    { target, payload },
  );
}

/** The stage's current slide, as pixels. */
async function stageShot(page: Page): Promise<Buffer> {
  await page.evaluate(() => document.fonts.ready);
  return page.getByTestId('stage').screenshot({ animations: 'disabled' });
}

async function documentFont(page: Page, family: string) {
  await page.waitForFunction(
    (family) =>
      [...document.fonts].some((face) => face.family.replace(/"/g, '') === family && face.status === 'loaded'),
    family,
  );
}

test.describe('fonts: demo page', () => {
  // Tall enough for the whole panel in one screenshot.
  test.use({ viewport: { width: 1280, height: 1900 } });

  test('the panel lists every family and source, in the dark and the light theme', async ({
    page,
    browserName,
  }, testInfo) => {
    const problems = watch(page);
    const requests = await recordedGoogle(page);
    mkdirSync(SCREENS, { recursive: true });
    for (const theme of ['dark', 'light'] as const) {
      const drawer = await openDemo(page, `&theme=${theme}`);
      await settled(page);
      await expect(faces(page, 'Barlow Condensed')).toContainText('mislabeled');
      await expect(faces(page, 'Roboto')).toContainText('Google Fonts');
      await expect(faces(page, 'Brand Sans')).toContainText('Substitute');
      await expect(drawer.getByRole('switch', { name: /Fetch open fonts from Google Fonts/ })).toBeChecked();
      const path = join(SCREENS, `fonts-panel-${theme}-${browserName}.png`);
      await drawer.screenshot({ path, animations: 'disabled' });
      testInfo.annotations.push({ type: 'screenshot', description: path });
    }
    expectNoText(requests);
    expect(problems).toEqual([]);
  });

  test('Google Fonts: on by default, the css2 stylesheet and its chunks only, never text=; off is remembered', async ({
    page,
  }) => {
    const requests = await recordedGoogle(page);
    const drawer = await openDemo(page);
    await settled(page);
    await expect(drawer.getByText('2 files for Roboto.')).toBeVisible();
    await expect(faces(page, 'Roboto')).toContainText(/400\s*·\s*Google Fonts/);
    const google = googleRequests(requests);
    expect(google.filter((url) => url.pathname === '/css2')).toHaveLength(1);
    expect(google.filter((url) => url.hostname === 'fonts.gstatic.com')).toHaveLength(2);
    expectNoText(requests);

    await drawer.getByRole('switch', { name: /Fetch open fonts from Google Fonts/ }).click();
    await expect(faces(page, 'Roboto')).toContainText('Substitute');
    requests.length = 0;
    await page.reload();
    await expect(page.getByRole('dialog', { name: 'Fonts' })).toBeVisible();
    await settled(page);
    await expect(page.getByRole('switch', { name: /Fetch open fonts from Google Fonts/ })).not.toBeChecked();
    expect(googleRequests(requests)).toEqual([]);
  });

  test('a dropped font file changes the render, and the slide shown stays', async ({ page }) => {
    const problems = watch(page);
    await recordedGoogle(page);
    await page.goto(DEMO);
    await expect(page.getByRole('toolbar', { name: 'Deck' })).toBeVisible();
    // Google Fonts draws slide 1's subtitle once its chunks arrive; the title is in Brand Sans, which no source has.
    await documentFont(page, 'Roboto');
    await expect(page.getByRole('button', { name: /^Fonts/ })).toContainText('3');
    const before = await stageShot(page);

    await page.keyboard.press('ArrowRight');
    await expect(page.getByRole('toolbar', { name: 'Deck' })).toContainText('Slide 2 of 3');
    await dropFiles(page, 'body', [{ path: PACIFICO, name: 'BrandSans.ttf' }]);
    await documentFont(page, 'Brand Sans');
    await expect(page.getByRole('button', { name: /^Fonts/ })).toContainText('2');
    await expect(page.getByRole('toolbar', { name: 'Deck' })).toContainText('Slide 2 of 3');
    await expect(page.getByRole('dialog', { name: /Open/ })).toHaveCount(0);

    await page.keyboard.press('ArrowLeft');
    await expect(page.getByRole('toolbar', { name: 'Deck' })).toContainText('Slide 1 of 3');
    const after = await stageShot(page);
    expect(differentPixels(before, after)).toBeGreaterThan(500);
    expect(problems).toEqual([]);
  });

  test('font files dropped on the panel or picked are listed, and a file that is no font is not used', async ({
    page,
  }) => {
    await recordedGoogle(page);
    const drawer = await openDemo(page);
    await settled(page);
    await dropFiles(page, '[data-testid="font-drop"]', [{ path: PACIFICO, name: 'BrandSans.ttf' }]);
    await expect(faces(page, 'Brand Sans')).toContainText(/400\s*·\s*Your file/);
    await drawer
      .getByLabel('Font files', { exact: true })
      .setInputFiles({ name: 'Broken.woff2', mimeType: 'font/woff2', buffer: Buffer.from('plain text') });
    const list = drawer.getByRole('list', { name: 'Added files' });
    await expect(list.getByText('BrandSans.ttf')).toBeVisible();
    await expect(list.getByText('not a font file')).toBeVisible();
    // The family from the user's file gets the download toggle.
    await expect(faces(page, 'Brand Sans').getByRole('checkbox', { name: /Include in the download/ })).toBeChecked();
  });

  test('the substitutes come from the app itself, checked against their manifest', async ({ page }) => {
    const requests = await recordedGoogle(page);
    const drawer = await openDemo(page);
    await settled(page);
    await expect(
      drawer.getByText('Liberation Sans draws the faces no source has, with matching metrics.'),
    ).toBeVisible();
    const own = requests
      .filter((url) => url.startsWith(`http://localhost:${DEMO_PORT}/fonts/core/`))
      .map((url) => new URL(url).pathname);
    expect(own).toEqual([
      '/fonts/core/core.json',
      '/fonts/core/LiberationSans-Regular.woff2',
      '/fonts/core/LiberationSans-Bold.woff2',
    ]);
  });
});

test.describe('fonts: Local Font Access', () => {
  test('the browser’s own: Chromium offers the button, Firefox and WebKit say they cannot', async ({
    page,
    browserName,
  }) => {
    await recordedGoogle(page);
    const drawer = await openDemo(page);
    const section = drawer.locator('section[data-lfa-state]');
    if (browserName === 'chromium') {
      await expect(section).toHaveAttribute('data-lfa-state', /^(prompt|granted|denied|done)$/);
    } else {
      await expect(section).toHaveAttribute('data-lfa-state', 'unsupported');
      await expect(section).toContainText("This browser can't share the fonts installed on this computer");
    }
  });

  test('prompt: nothing is read before the click; then the picked faces draw', async ({ page }) => {
    await recordedGoogle(page);
    await fakeLocalFonts(page, 'prompt');
    const drawer = await openDemo(page);
    await settled(page);
    const section = drawer.locator('section[data-lfa-state]');
    await expect(section).toHaveAttribute('data-lfa-state', 'prompt');
    expect(await page.evaluate(() => (window as unknown as { __lfaQueries: number }).__lfaQueries)).toBe(0);
    await section.getByRole('button', { name: 'Use fonts installed on this computer' }).click();
    await expect(section).toHaveAttribute('data-lfa-state', 'done');
    await expect(section).toContainText('Added 2 font files from this computer.');
    await expect(faces(page, 'Helvetica Neue')).toContainText(/400\s*·\s*This computer/);
    await expect(faces(page, 'Optima')).toContainText('restricted licence');
    await documentFont(page, 'Helvetica Neue');
  });

  test('granted: read on open, with no click', async ({ page }) => {
    await recordedGoogle(page);
    await fakeLocalFonts(page, 'granted');
    const drawer = await openDemo(page);
    await expect(drawer.locator('section[data-lfa-state]')).toHaveAttribute('data-lfa-state', 'done');
    await expect(faces(page, 'Helvetica Neue')).toContainText(/400\s*·\s*This computer/);
  });

  test('denied before: a hint, and no prompt', async ({ page }) => {
    await recordedGoogle(page);
    await fakeLocalFonts(page, 'denied');
    const drawer = await openDemo(page);
    const section = drawer.locator('section[data-lfa-state]');
    await expect(section).toHaveAttribute('data-lfa-state', 'denied');
    await expect(section.getByRole('button')).toHaveCount(0);
    expect(await page.evaluate(() => (window as unknown as { __lfaQueries: number }).__lfaQueries)).toBe(0);
  });

  test('denied at the prompt: the same hint', async ({ page }) => {
    await recordedGoogle(page);
    await fakeLocalFonts(page, 'prompt', 'denied');
    const drawer = await openDemo(page);
    await settled(page);
    const section = drawer.locator('section[data-lfa-state]');
    await section.getByRole('button', { name: 'Use fonts installed on this computer' }).click();
    await expect(section).toHaveAttribute('data-lfa-state', 'denied');
    await expect(section).toContainText('blocked for this site');
  });

  test('error: the message and a retry', async ({ page }) => {
    await recordedGoogle(page);
    await fakeLocalFonts(page, 'error');
    const drawer = await openDemo(page);
    await settled(page);
    const section = drawer.locator('section[data-lfa-state]');
    await section.getByRole('button', { name: 'Use fonts installed on this computer' }).click();
    await expect(section).toHaveAttribute('data-lfa-state', 'error');
    await expect(section).toContainText('font service crashed');
    await expect(section.getByRole('button', { name: 'Try again' })).toBeVisible();
  });
});

test(
  'app: the fonts panel on a compiled deck, with the real worker',
  { tag: ['@smoke'] },
  async ({ page, browserName }) => {
    const problems = watch(page);
    const requests: string[] = [];
    page.on('request', (request) => requests.push(request.url()));
    await page.goto('/');
    await expect(page.getByTestId('wasm-status')).toHaveText(/^slidock-wasm \S+$/, { timeout: 60_000 });
    await page.locator('input[type="file"]').setInputFiles(smokeCase().input);
    await expect(page.getByRole('toolbar', { name: 'Deck' })).toBeVisible({ timeout: 60_000 });
    await page.getByRole('button', { name: /^Fonts/ }).click();
    const drawer = page.getByRole('dialog', { name: 'Fonts' });
    await expect(drawer.getByRole('listitem', { name: 'Liberation Sans', exact: true })).toBeVisible();
    await expect(drawer.getByRole('switch', { name: /Fetch open fonts from Google Fonts/ })).toBeChecked();
    const section = drawer.locator('section[data-lfa-state]');
    // The real worker answers selectLocal, so Chromium offers installed fonts.
    await expect(section).toHaveAttribute(
      'data-lfa-state',
      browserName === 'chromium' ? /^(prompt|granted|denied|done)$/ : 'unsupported',
    );

    await drawer.getByLabel('Font files', { exact: true }).setInputFiles(PACIFICO);
    const list = drawer.getByRole('list', { name: 'Added files' });
    await expect(list.getByText('pacifico.ttf')).toBeVisible();
    await expect(list.getByText('Added')).toBeVisible();
    await expect(drawer.getByText('Updating the slides with the new fonts…')).toHaveCount(0);
    await expect(page.getByRole('toolbar', { name: 'Deck' })).toContainText('Slide 1 of');
    expectNoText(requests);
    expect(problems).toEqual([]);
  },
);

/** The shown slide's images, and how many of them use a `blob:` URL the page has not revoked. */
function mediaServed(page: Page) {
  return page.evaluate((selector) => {
    const revoked = (window as unknown as { __sdRevoked: string[] }).__sdRevoked;
    const images = Array.from(document.querySelector(selector)?.shadowRoot?.querySelectorAll('img') ?? []);
    const served = images.filter((img) => {
      const src = img.currentSrc || img.src;
      return src.startsWith('blob:') && !revoked.includes(src);
    });
    return { images: images.length, served: served.length };
  }, CURRENT);
}

test(
  'app: a font added while presenting leaves the images of the slides still to come loading',
  { tag: ['@smoke'] },
  async ({ page }) => {
    const problems = watch(page);
    // Counts the decks the worker sends (a re-finish sends one more), and records the object URLs revoked.
    await page.addInitScript(() => {
      const counter = window as unknown as { __sdDecks: number; __sdRevoked: string[] };
      counter.__sdDecks = 0;
      counter.__sdRevoked = [];
      const revoke = URL.revokeObjectURL.bind(URL);
      URL.revokeObjectURL = (url: string) => {
        counter.__sdRevoked.push(url);
        revoke(url);
      };
      const NativeWorker = window.Worker;
      window.Worker = class extends NativeWorker {
        constructor(url: string | URL, options?: WorkerOptions) {
          super(url, options);
          this.addEventListener('message', (event: MessageEvent<{ type?: string }>) => {
            if (event.data.type === 'deck') {
              counter.__sdDecks += 1;
            }
          });
        }
      };
    });
    const decks = () => page.evaluate(() => (window as unknown as { __sdDecks: number }).__sdDecks);
    const compiled = compiledDeck(smokeCase());
    // The smoke deck's PNG is on its second slide (fixture/pptx.ts).
    const pictured = compiled.slides.findIndex((slide) => slide.title === 'A picture');
    expect(pictured).toBe(1);

    await page.goto('/');
    await expect(page.getByTestId('wasm-status')).toHaveText(/^slidock-wasm \S+$/, { timeout: 60_000 });
    await page.locator('input[type="file"]').setInputFiles(smokeCase().input);
    await expect(page.getByRole('toolbar', { name: 'Deck' })).toBeVisible({ timeout: 60_000 });
    await expect.poll(decks).toBeGreaterThan(0);
    // The sources that start on their own settle first.
    await page.waitForTimeout(1_000);
    const before = await decks();

    await page.getByRole('button', { name: 'Present', exact: true }).click();
    await expect(page).toHaveURL(/present=1/);
    await dropFiles(page, 'body', [{ path: PACIFICO, name: 'pacifico.ttf' }]);
    await expect.poll(decks).toBeGreaterThan(before);
    for (let index = 0; index < pictured; index += 1) {
      await page.keyboard.press('ArrowRight');
    }
    await expectSlide(page, compiled.slides[pictured]?.id ?? '');
    // An image the rail decoded before can still draw from the cache, so its URL must also be alive.
    expect((await checkMedia(page, compiled)).failed).toEqual([]);
    expect(await mediaServed(page)).toEqual({ images: 1, served: 1 });
    await page.keyboard.press('Escape');
    await expect(page).not.toHaveURL(/present=1/);
    expect((await checkMedia(page, compiled)).failed).toEqual([]);
    expect(await mediaServed(page)).toEqual({ images: 1, served: 1 });
    expect(problems).toEqual([]);
  },
);
