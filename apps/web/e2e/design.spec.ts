// The design rules (packages/ui/src/audit.ts) on every screen of the app (support/screens.ts, on the demo page), in
// both themes at phone (375 px) and desktop width: text contrast at least WCAG AA (form fields' values and placeholders
// too), no horizontal page scroll, a visible focus of at least 3:1 on every control the Tab key reaches, and a name on
// every control. The audit itself is checked first: it must report a page that breaks each rule.
import { type Page, expect, test } from '@playwright/test';

import { FIREFOX_IN_DOCKER } from '../../../packages/player/e2e/support/browserServer';
import { type AuditFinding, type AuditPage, auditPage, tabKeyFor } from '../../../packages/ui/src/audit';
import { DEMO_PORT } from './support/decks';
import { SCREENS, demoDefaults } from './support/screens';

const DEMO = `http://localhost:${DEMO_PORT}/demo.html`;

/** The slides and their thumbnails are the deck's content, not the app's chrome. */
const SKIP = ['[data-testid="stage"]', '[data-thumbnail]', '.sd-frame'];

const VIEWPORTS = [
  { width: 375, height: 812 },
  { width: 1280, height: 800 },
] as const;

/** The project's browser runs in the pinned image (Linux), not on this machine. */
const inDocker = (project: string) => project === 'webkit-docker' || (project === 'firefox' && FIREFOX_IN_DOCKER);

const summary = (findings: readonly AuditFinding[]) => findings.map((f) => `${f.rule}: ${f.element}: ${f.detail}`);

const PAGE = (
  style: string,
  body: string,
) => `<!doctype html><html><head><meta name="viewport" content="width=device-width">
  <style>
    body { margin: 0; padding: 0 16px; background: #0f172a; color: #f1f5f9; font: 14px system-ui; }
    input, select { background: #0f172a; color: #f1f5f9; }
    button:focus-visible, input:focus-visible, select:focus-visible { outline: 2px solid #f472b6; outline-offset: 2px; }
    ${style}
  </style></head><body>${body}</body></html>`;

/** `page` as the audit sees it, with `onPress` run before each key press of the focus walk. */
function pressing(page: Page, onPress: (count: number) => Promise<void>): AuditPage {
  const audited: AuditPage = page;
  let count = 0;
  return {
    evaluate: audited.evaluate.bind(audited),
    locator: audited.locator.bind(audited),
    url: audited.url.bind(audited),
    keyboard: {
      press: async (key) => {
        await onPress(++count);
        await page.keyboard.press(key);
      },
    },
  };
}

test.describe('the audit', () => {
  const BROKEN = PAGE(
    `.faint { color: #475569; }
    .gradient { background: linear-gradient(to right, #f472b6, #fb923c); color: #fff; }
    .wide { width: 600px; }
    button.bare { outline: none; }
    .faint-field { color: #475569; background: #0f172a; }
    .faint-placeholder::placeholder { color: #475569; }
    button.dim-ring:focus-visible { outline-color: #1e293b; }
    button.amber { background: #fbbf24; color: #0f172a; }
    button.amber:focus-visible { outline-color: #f1f5f9; outline-offset: -2px; }`,
    `<p>Readable text</p>
    <p class="faint">Faint text</p>
    <p><span class="gradient">White on the brand gradient</span></p>
    <div class="wide">A wide row</div>
    <button class="bare">No focus ring</button>
    <button></button>
    <input class="faint-placeholder" placeholder="Faint placeholder" aria-label="Search">
    <input class="faint-field" value="Faint value" aria-label="Value">
    <select class="faint-field" aria-label="Choice"><option>Faint option</option></select>
    <button class="dim-ring">Dim focus ring</button>
    <button class="amber">Light ring on amber</button>`,
  );

  test('reports a page that breaks each rule, and nothing on one that keeps them', async ({
    page,
    browserName,
  }, info) => {
    const tabKey = tabKeyFor(browserName, process.platform, inDocker(info.project.name));
    await page.setViewportSize({ width: 375, height: 600 });
    await page.setContent(BROKEN);
    const findings = summary(await auditPage(page, { skip: [], tabKey }));
    expect(findings.filter((line) => line.startsWith('contrast'))).toEqual([
      expect.stringContaining('“Faint text”'),
      expect.stringContaining('“White on the brand gradient”'),
      expect.stringMatching(/“Faint placeholder”.*\(placeholder\)$/),
      expect.stringContaining('“Faint value”'),
      expect.stringContaining('“Faint option”'),
    ]);
    expect(findings.filter((line) => line.startsWith('overflow'))).toEqual([
      expect.stringContaining('overflow: document'),
      expect.stringContaining('“A wide row”'),
    ]);
    expect(findings.filter((line) => line.startsWith('focus'))).toEqual([
      expect.stringContaining('“No focus ring”: no visible focus'),
      expect.stringMatching(/“Dim focus ring”: the focus indicator is 1\.\d+:1 \(outline/),
      expect.stringMatching(/“Light ring on amber”: the focus indicator is 1\.\d+:1 \(outline #f1f5f9 on #fbbf24\)/),
    ]);
    expect(findings.filter((line) => line.startsWith('name'))).toEqual([expect.stringContaining('button')]);

    await page.setContent(
      PAGE(
        `.muted-placeholder::placeholder { color: #94a3b8; }
        button.amber { background: #fbbf24; color: #0f172a; }
        button.amber:focus-visible { outline-color: #0f172a; outline-offset: -2px; }`,
        `<p>Readable text</p><button>Labelled</button>
        <input class="muted-placeholder" placeholder="Muted placeholder" aria-label="Search">
        <select aria-label="Choice"><option>Readable option</option></select>
        <button class="amber">Dark ring on amber</button>`,
      ),
    );
    expect(summary(await auditPage(page, { skip: [], tabKey }))).toEqual([]);
    // A key that moves the focus nowhere reaches no control: reported, not passed as "nothing unfocused".
    expect(summary(await auditPage(page, { skip: [], tabKey: 'Shift' }))).toEqual([
      'focus: document: Shift reached none of 3 controls',
    ]);
  });

  test('checks a control that appears during the walk, walks from the top every time, and reports a walk cut short', async ({
    page,
    browserName,
  }, info) => {
    const tabKey = tabKeyFor(browserName, process.platform, inDocker(info.project.name));
    // The first button's focus adds a button without a focus style after it.
    await page.setContent(
      PAGE(
        'button.bare { outline: none; }',
        `<button id="first">First</button><span id="slot"></span><button>Last</button>
        <script>
          document.getElementById('first').addEventListener('focus', () => {
            const late = document.createElement('button');
            late.className = 'bare';
            late.textContent = 'Late';
            document.getElementById('slot').replaceChildren(late);
          }, { once: true });
        </script>`,
      ),
    );
    expect(summary(await auditPage(page, { skip: [], tabKey }))).toEqual([
      'focus: <button> “Late”: no visible focus (none|none)',
    ]);
    // A second walk on the same page starts from the top too, wherever the first left the focus.
    await page.setContent(
      PAGE(
        'button.bare { outline: none; }',
        '<button class="bare">One</button><button>Two</button><button>Three</button>',
      ),
    );
    const one = 'focus: <button> “One”: no visible focus (none|none)';
    expect(summary(await auditPage(page, { skip: [], tabKey }))).toEqual([one]);
    expect(summary(await auditPage(page, { skip: [], tabKey }))).toEqual([one]);
    expect(summary(await auditPage(page, { skip: [], tabKey, limit: 2 }))).toEqual([
      one,
      'focus: document: the walk stopped after 2 presses (2 controls) before the focus came back to one',
    ]);
  });

  test('reports a page that navigates during the walk', async ({ page, browserName }, info) => {
    const tabKey = tabKeyFor(browserName, process.platform, inDocker(info.project.name));
    await page.route('http://audit.invalid/', (route) =>
      route.fulfill({
        contentType: 'text/html',
        body: PAGE(
          '',
          `<button>One</button><button id="reload">Two</button><button>Three</button>
          <script>
            document.getElementById('reload').addEventListener('focus', () => {
              if (!sessionStorage.getItem('reloaded')) {
                sessionStorage.setItem('reloaded', '1');
                location.reload();
              }
            });
          </script>`,
        ),
      }),
    );
    await page.goto('http://audit.invalid/');
    const findings = summary(await auditPage(page, { skip: [], tabKey }));
    expect(findings).toEqual([expect.stringMatching(/^focus: document: the page navigated during the focus walk/)]);
    // Once reloaded, the page stays put and passes.
    expect(summary(await auditPage(page, { skip: [], tabKey }))).toEqual([]);
  });

  test('a dropped dev-server socket reloads the demo page, which the walk reports; held, it stays', async ({
    page,
    browserName,
  }, info) => {
    const tabKey = tabKeyFor(browserName, process.platform, inDocker(info.project.name));
    const socket = await demoDefaults(page);
    await page.goto(`${DEMO}?view=landing`);
    await expect(page.getByTestId('wasm-status')).toHaveText('slidock-wasm demo');
    expect(summary(await auditPage(page, { skip: SKIP, tabKey }))).toEqual([]);
    const dropping = pressing(page, async (count) => {
      if (count === 2) {
        await socket.drop();
        await page.waitForEvent('framenavigated', { timeout: 15_000 });
      }
    });
    expect(summary(await auditPage(dropping, { skip: SKIP, tabKey }))).toEqual([
      expect.stringMatching(/^focus: document: the page navigated during the focus walk/),
    ]);
  });
});

async function audit(page: Page, tabKey: string, gutter: number): Promise<string[]> {
  return summary(await auditPage(page, { skip: SKIP, tabKey, gutter }));
}

for (const screen of SCREENS.filter((candidate) => candidate.audit)) {
  test(`${screen.name} keeps the design rules`, async ({ page, browserName }, info) => {
    const tabKey = tabKeyFor(browserName, process.platform, inDocker(info.project.name));
    test.setTimeout(240_000);
    await demoDefaults(page);
    for (const colorScheme of ['dark', 'light'] as const) {
      for (const viewport of VIEWPORTS) {
        await test.step(`${colorScheme} at ${viewport.width} px`, async () => {
          await page.emulateMedia({ colorScheme });
          await page.setViewportSize(viewport);
          const shown = await screen.open(page, DEMO);
          if (shown !== page) {
            await shown.emulateMedia({ colorScheme });
            await shown.setViewportSize(viewport);
          }
          // Let the chrome's transitions (150 ms) end before colors are read.
          await shown.waitForTimeout(250);
          expect.soft(await audit(shown, tabKey, screen.gutter)).toEqual([]);
          if (shown !== page) {
            await shown.close();
          }
          // Present mode leaves the window full screen, which cannot be resized.
          await page.evaluate(async () => {
            if (document.fullscreenElement) {
              await document.exitFullscreen();
            }
          });
        });
      }
    }
  });
}

test('the viewer letterboxes the slide on the sunken color, and presenting on black', async ({ page }) => {
  await demoDefaults(page);
  const viewer = SCREENS.find((screen) => screen.name === 'viewer');
  const colors = () =>
    page.evaluate(() => {
      const color = (selector: string) => {
        const element = document.querySelector(selector);
        return element ? getComputedStyle(element).backgroundColor : null;
      };
      return { player: color('[data-testid="stage"] .sd-player'), well: color('[data-stage-well]') };
    });
  for (const colorScheme of ['dark', 'light'] as const) {
    await page.emulateMedia({ colorScheme });
    await viewer?.open(page, DEMO);
    const { player, well } = await colors();
    expect(player, colorScheme).not.toBeNull();
    expect(player, colorScheme).toBe(well);
  }
  await page.getByRole('button', { name: 'Present', exact: true }).click();
  await expect(page).toHaveURL(/present=1/);
  await expect.poll(async () => (await colors()).player).toBe('rgb(0, 0, 0)');
});
