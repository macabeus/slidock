// The web app on the production build: each deck uploads, compiles with progress, shows every slide, presents, and
// downloads a zip byte-identical to the CLI's for the same input. Both run with Google Fonts off (the CLI fetches
// static files and a browser per-script chunks, so their Google faces differ by design): the zips then ship the same
// embedded and bundled substitute faces. A deck whose HEIC photos this browser decoded (the CLI omits them) is held
// instead to the zip the compiler writes under Node through the compile worker with the same decoded files. The
// download's copies of transparent videos are off (`COPIES_OFF`): the CLI makes none (copies.spec.ts covers them).
import { expect, test } from '@playwright/test';
import { spawnSync } from 'node:child_process';
import { join } from 'node:path';

import {
  CLI,
  CLI_FONT_ARGS,
  COPIES_OFF,
  GOOGLE_OFF,
  OUT,
  cliZip,
  compiledDeck,
  deckCases,
  timeoutFor,
} from './support/decks';
import { digest } from './support/files';
import { decodedPhotos, wasmZip } from './support/heic';
import {
  checkMedia,
  countPlayers,
  expectDrawn,
  expectMounted,
  expectSettled,
  expectSlide,
  noteUnsupported,
  watch,
} from './support/player';

interface Progress {
  panel: boolean;
  phases: string[];
}

for (const deck of deckCases()) {
  const tag = deck.smoke ? ['@smoke'] : [];
  test(
    `app: ${deck.name} uploads, compiles, shows every slide, presents and downloads`,
    { tag },
    async ({ page, browserName }, testInfo) => {
      const compiled = compiledDeck(deck);
      const slides = compiled.slides;
      test.setTimeout(timeoutFor(deck, slides.length, 120_000));
      const problems = watch(page);
      const google: string[] = [];
      page.on('request', (request) => {
        const { hostname } = new URL(request.url());
        if (hostname.endsWith('.googleapis.com') || hostname === 'fonts.gstatic.com') {
          google.push(request.url());
        }
      });
      for (const setting of [GOOGLE_OFF, COPIES_OFF]) {
        await page.addInitScript(({ key, value }) => {
          try {
            localStorage.setItem(key, value);
          } catch {
            // A frame with an opaque origin has no storage; the app's own document does.
          }
        }, setting);
      }

      await page.goto('/');
      await expect(page.getByTestId('wasm-status')).toHaveText(/^slidock-wasm \S+$/, { timeout: 60_000 });

      await test.step('upload and compile, with progress', async () => {
        // The conversion panel can come and go between two polls on a small deck, so a mutation observer records it.
        await page.evaluate(() => {
          const seen: Progress = { panel: false, phases: [] };
          (window as unknown as { __sdProgress: Progress }).__sdProgress = seen;
          new MutationObserver(() => {
            if (document.querySelector('[aria-label="Conversion"]')) {
              seen.panel = true;
            }
            const active = document.querySelector('[aria-label="Conversion"] li[aria-current="step"]')?.textContent;
            if (active && !seen.phases.includes(active)) {
              seen.phases.push(active);
            }
          }).observe(document.body, { subtree: true, childList: true, attributes: true });
        });
        await page.locator('input[type="file"]').setInputFiles(deck.input);
        await expect(page.getByRole('toolbar', { name: 'Deck' })).toBeVisible({ timeout: testInfo.timeout });
        const progress = await page.evaluate(() => (window as unknown as { __sdProgress: Progress }).__sdProgress);
        expect(progress.panel, 'the conversion panel showed').toBe(true);
        expect(progress.phases.length, 'a compile phase showed as active').toBeGreaterThan(0);
        testInfo.annotations.push({ type: 'progress', description: progress.phases.join(' → ') });
      });

      await test.step(`view all ${slides.length} slides`, async () => {
        // The walk reads the shown slide's layer between key presses: no re-finish may replace the player meanwhile.
        await expectSettled(page, testInfo.timeout);
        const playersMade = await countPlayers(page);
        // The rail's own items: thumbnails hold slides, and role queries reach into their shadow roots.
        await expect(page.locator('nav[aria-label="Slides"] > ol > li')).toHaveCount(slides.length);
        // The player's counter and the toolbar both number shown slides only.
        const counter = page.getByTestId('stage').locator('.sd-counter');
        const toolbar = page.getByRole('toolbar', { name: 'Deck' });
        const shownCount = slides.filter((slide) => !slide.hidden).length;
        for (const [index, slide] of slides.entries()) {
          if (index > 0) {
            await page.keyboard.press('ArrowRight');
          }
          await expectSlide(page, slide.id);
          await expectMounted(page, slide.id);
          await expect(counter).toHaveText(slide.hidden ? 'hidden' : `${slide.number} / ${shownCount}`);
          await expect(toolbar).toContainText(slide.hidden ? 'Hidden slide' : `Slide ${slide.number} of ${shownCount}`);
          const media = await checkMedia(page, compiled);
          expect(media.failed, `media of ${slide.id}`).toEqual([]);
          noteUnsupported(testInfo, slide.id, media);
          await expectDrawn(page, slide.id);
        }
        expect(await playersMade(), 'players created during the walk').toBe(0);
      });

      await test.step('present', async () => {
        const shown = slides.filter((slide) => !slide.hidden);
        await page.getByRole('button', { name: 'Present', exact: true }).click();
        await expect(page).toHaveURL(/present=1/);
        await page.keyboard.press('Home');
        await expectSlide(page, shown[0]?.id ?? '');
        await page.keyboard.press('End');
        await expectSlide(page, shown.at(-1)?.id ?? '');
        await page.keyboard.press('Escape');
        await expect(page).not.toHaveURL(/present=1/);
      });

      await test.step('download the zip, byte-identical to the CLI zip (or, with decoded photos, the compiler zip)', async () => {
        await page.getByRole('button', { name: 'Download zip' }).click();
        const dialog = page.getByRole('dialog', { name: 'Download zip' });
        const download = page.waitForEvent('download', { timeout: testInfo.timeout });
        await dialog.getByRole('button', { name: 'Build and save' }).click();
        const saved = join(OUT, 'downloads', browserName, `${deck.key}.zip`);
        await (await download).saveAs(saved);
        await expect(dialog.getByText(/^Saved /)).toBeVisible();
        const [app, cli] = await Promise.all([digest(saved), digest(cliZip(deck))]);
        const work = join(OUT, 'heic', browserName, deck.key);
        const photos = decodedPhotos(saved, cliZip(deck), join(work, 'photos'));
        if (photos.length === 0) {
          expect(app, 'the app zip equals the CLI zip (Google Fonts off in both)').toEqual(cli);
        } else {
          testInfo.annotations.push({ type: 'heic', description: `${photos.length} photos decoded by this browser` });
          const wasm = await digest(wasmZip(deck.input, photos, work));
          expect(app, 'the app zip equals the compiler zip with the photos this browser decoded').toEqual(wasm);
        }
      });

      if (deck.smoke) {
        await test.step("with the page's options, still byte-identical to the CLI zip with the same flags", async () => {
          const dialog = page.getByRole('dialog', { name: 'Download zip' });
          await dialog.getByRole('textbox', { name: 'Title', exact: true }).fill('A talk "quoted" <b>');
          await dialog.getByRole('textbox', { name: /Description/ }).fill('');
          await dialog.getByRole('textbox', { name: /Public URL/ }).fill('https://example.com/talk/');
          await dialog.getByRole('checkbox', { name: /Also on the page/ }).check();
          const download = page.waitForEvent('download', { timeout: testInfo.timeout });
          await dialog.getByRole('button', { name: 'Build and save' }).click();
          const saved = join(OUT, 'downloads', browserName, `${deck.key}-page.zip`);
          await (await download).saveAs(saved);
          const zip = join(OUT, 'downloads', browserName, `${deck.key}-page-cli.zip`);
          const flags = [
            '--title',
            'A talk "quoted" <b>',
            '--description',
            '',
            '--site-url',
            'https://example.com/talk/',
          ];
          const run = spawnSync(
            CLI,
            ['compile', deck.input, '-o', zip, ...CLI_FONT_ARGS, ...flags, '--notes-on-page'],
            {
              encoding: 'utf8',
              maxBuffer: 64 * 1024 * 1024,
            },
          );
          expect(run.status, run.stderr.slice(-2000)).toBe(0);
          expect(await digest(saved), 'the app zip equals the CLI zip with the same page flags').toEqual(
            await digest(zip),
          );

          // The presenter view and the notes: each other combination the dialog offers, against the CLI's flags.
          const combinations: { name: string; set: () => Promise<void>; flags: string[] }[] = [
            {
              name: 'no-presenter-notes-on-page',
              set: () => dialog.getByRole('checkbox', { name: /^Presenter view/ }).uncheck(),
              flags: ['--no-presenter', '--notes-on-page'],
            },
            {
              name: 'no-presenter-no-notes',
              set: () => dialog.getByRole('checkbox', { name: /^Speaker notes on the page/ }).uncheck(),
              flags: ['--no-presenter', '--no-notes'],
            },
            {
              name: 'presenter-no-notes',
              set: async () => {
                await dialog.getByRole('checkbox', { name: /^Presenter view/ }).check();
                await dialog.getByRole('checkbox', { name: /^Speaker notes/ }).uncheck();
              },
              flags: ['--no-notes'],
            },
          ];
          for (const combination of combinations) {
            await combination.set();
            const download = page.waitForEvent('download', { timeout: testInfo.timeout });
            await dialog.getByRole('button', { name: 'Build and save' }).click();
            const saved = join(OUT, 'downloads', browserName, `${deck.key}-${combination.name}.zip`);
            await (await download).saveAs(saved);
            const zip = join(OUT, 'downloads', browserName, `${deck.key}-${combination.name}-cli.zip`);
            const run = spawnSync(
              CLI,
              ['compile', deck.input, '-o', zip, ...CLI_FONT_ARGS, ...flags, ...combination.flags],
              {
                encoding: 'utf8',
                maxBuffer: 64 * 1024 * 1024,
              },
            );
            expect(run.status, run.stderr.slice(-2000)).toBe(0);
            expect(await digest(saved), `the app zip equals the CLI zip with ${combination.flags.join(' ')}`).toEqual(
              await digest(zip),
            );
          }
        });
      }

      expect(google, 'requests to Google Fonts with its switch off').toEqual([]);

      expect(problems, 'console errors, page errors and CSP violations').toEqual([]);
    },
  );
}
