// Every view and state of the bench UI over the dev server's mock API (the schema crate's synthetic bench): the
// review screenshots (screenshots.ts) shoot them and the design suite (design.e2e.ts) audits them.
import type { Page } from '@playwright/test';

export const ROW = encodeURIComponent('demo-basics:od-p901:odp');
const FAILED_ROW = encodeURIComponent('demo-basics:od-p904:odp');
const STAGE_ROW = encodeURIComponent('demo-keynote:kn-1003:keynote#s1');

export interface BenchView {
  name: string;
  /** The URL fragment that opens it. */
  hash: string;
  /** A row drawer: shot as the viewport, then scrolled to its end. */
  drawer?: boolean;
}

/** The views, with the latest motion and fonts runs of the runs index (the mock's). */
export function benchViews(index: { runs: readonly { key: string; lane: string }[] }): BenchView[] {
  const motion = index.runs.findLast((run) => run.lane === 'motion')?.key ?? '';
  const fonts = index.runs.findLast((run) => run.lane === 'fonts')?.key ?? '';
  return [
    { name: 'overview', hash: '' },
    { name: 'overview-baseline', hash: '#run=baseline' },
    { name: 'explorer', hash: '#tab=explorer' },
    { name: 'explorer-filtered', hash: '#tab=explorer&fe=odp,pptx&oc=close,mismatch&ref=1&sort=ssim&dir=desc' },
    { name: 'explorer-base', hash: '#tab=explorer&base=baseline&ref=1' },
    { name: 'explorer-feature', hash: '#tab=explorer&feat=element.chart&code=PX4001' },
    { name: 'explorer-motion', hash: `#tab=explorer&run=${motion}` },
    { name: 'explorer-fonts', hash: `#tab=explorer&run=${fonts}&ref=1` },
    { name: 'drawer-side', hash: `#tab=explorer&row=${ROW}`, drawer: true },
    { name: 'drawer-slider', hash: `#tab=explorer&row=${ROW}&img=slider`, drawer: true },
    { name: 'drawer-onion', hash: `#tab=explorer&row=${ROW}&img=onion`, drawer: true },
    { name: 'drawer-difference', hash: `#tab=explorer&row=${ROW}&img=difference`, drawer: true },
    { name: 'drawer-heatmap', hash: `#tab=explorer&row=${ROW}&img=heatmap`, drawer: true },
    { name: 'drawer-failed', hash: `#tab=explorer&row=${FAILED_ROW}&ref=1`, drawer: true },
    { name: 'drawer-stage', hash: `#tab=explorer&run=${motion}&row=${STAGE_ROW}`, drawer: true },
    { name: 'compare', hash: '#tab=compare' },
    { name: 'compare-drawer', hash: `#tab=compare&row=${ROW}`, drawer: true },
    { name: 'parity', hash: '#tab=parity' },
    { name: 'parity-filtered', hash: '#tab=parity&ps=regressed,drift-better' },
    { name: 'fonts', hash: '#tab=fonts' },
    { name: 'fonts-run', hash: `#tab=fonts&run=${fonts}` },
    { name: 'effects', hash: '#tab=effects' },
    { name: 'effects-run', hash: `#tab=effects&run=${motion}` },
    { name: 'methodology', hash: '#tab=methodology' },
    { name: 'row-missing', hash: '#tab=explorer&row=demo-basics%3Anope%3Aodp' },
  ];
}

/** Waits until the view has loaded: the shell is up, nothing shows a loading state, and every image has decoded. */
export async function settle(page: Page) {
  await page.waitForFunction(
    () => document.querySelector('main') !== null && document.querySelector('[aria-busy="true"]') === null,
  );
  // A full-page shot shows the images below the fold too: lazy ones load now (a lazy image's decode waits for it).
  await page.evaluate(() =>
    Promise.all(
      [...document.images].map((image) => {
        image.loading = 'eager';
        return image.decode().catch(() => undefined);
      }),
    ),
  );
  await page.waitForTimeout(250);
}
