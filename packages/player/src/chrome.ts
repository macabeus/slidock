// The player's own chrome: the stage, the slide counter, the blank screen and the help overlay. Its rules are
// injected once per document (or shadow root) the player mounts in; a deck page may also carry them as
// `<style id="sd-chrome">`. The deck page's stage (`.sd-page`) is fixed to the window; the page's text view, if any,
// scrolls up over it. The presenter view's rules are its own (`presenter/style.ts`).
import { el } from './dom';
import { maskCss } from './effects/masks';

export const CHROME_CSS = `
.sd-player{position:relative;width:100%;height:100%;overflow:hidden;background:#000;outline:none;-webkit-user-select:none;user-select:none;font:14px/1.4 system-ui,sans-serif}
.sd-player.sd-page{position:fixed;inset:0;width:auto;height:auto}
.sd-player.sd-fill{position:fixed;inset:0;width:auto;height:auto;z-index:2147483647}
.sd-player.sd-idle{cursor:none}
.sd-player:focus-visible{outline:2px solid #38bdf8;outline-offset:-2px}
.sd-player.sd-fill:focus-visible{outline:none}
.sd-player:fullscreen{outline:none}
.sd-frame{position:absolute;left:0;top:0;transform-origin:0 0;overflow:hidden}
.sd-layer{position:absolute;left:0;top:0;visibility:hidden}
.sd-layer[data-sd-current]{visibility:visible;z-index:1}
.sd-layer[data-sd-incoming]{visibility:visible;z-index:2}
.sd-layer[data-sd-incoming][data-sd-under]{z-index:0}
.sd-layer[data-sd-clone]{visibility:visible;z-index:1}
.sd-layer[data-sd-clone=in]{z-index:2}
.sd-overlay{position:absolute;inset:0;z-index:3;pointer-events:none}
.sd-blank{position:absolute;inset:0;z-index:5;display:none}
.sd-blank[data-sd-blank=black]{display:block;background:#000}
.sd-blank[data-sd-blank=white]{display:block;background:#fff}
.sd-counter{position:absolute;right:12px;bottom:10px;z-index:6;padding:2px 10px;border-radius:999px;background:rgba(15,23,42,.72);color:#e2e8f0;font:12px/1.6 ui-monospace,SFMono-Regular,Menlo,monospace;opacity:0;transition:opacity .3s;pointer-events:none}
.sd-player:hover .sd-counter,.sd-counter[data-sd-active]{opacity:1}
.sd-to-text{position:absolute;left:12px;bottom:10px;z-index:6;padding:2px 10px;border-radius:999px;background:rgba(15,23,42,.72);color:#e2e8f0;font:12px/1.6 system-ui,sans-serif;text-decoration:none;opacity:0;transition:opacity .3s;pointer-events:none}
.sd-player:hover .sd-to-text,.sd-to-text[data-sd-active],.sd-to-text:focus-visible{opacity:1;pointer-events:auto}
.sd-player.sd-fill .sd-to-text,.sd-player:fullscreen .sd-to-text{display:none}
.sd-sr{position:absolute;width:1px;height:1px;overflow:hidden;clip:rect(0 0 0 0);white-space:nowrap}
.sd-help{position:absolute;inset:0;z-index:7;display:flex;align-items:center;justify-content:center;background:rgba(2,6,23,.72);color:#e2e8f0}
.sd-help[hidden]{display:none}
.sd-help:focus{outline:none}
.sd-help>div{max-width:min(520px,90%);padding:20px 24px;border-radius:14px;background:#0f172a;border:1px solid #334155}
.sd-help .sd-help-title{margin:0 0 10px;font-size:16px;font-weight:700}
.sd-help dl{display:grid;grid-template-columns:auto 1fr;gap:4px 16px;margin:0}
.sd-help dt{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;color:#fbbf24}
.sd-help dd{margin:0}
.sd-static{position:relative;overflow:hidden}
${maskCss('.sd-layer')}
`;

/** The deck page's text view of its slides, below the stage. */
export const TEXT_VIEW_ID = 'sd-text';

const injected = new WeakSet<Document | ShadowRoot>();

/** Adds the chrome rules to the document or shadow root that holds `node`, once. */
export function ensureChrome(node: Node): void {
  const root = node.getRootNode() as Document | ShadowRoot;
  const document = node.ownerDocument;
  if (document === null || injected.has(root)) {
    return;
  }
  injected.add(root);
  if (root === document && document.getElementById('sd-chrome')?.textContent?.includes('.sd-player') === true) {
    return;
  }
  const style = document.createElement('style');
  style.setAttribute('data-sd-chrome', '');
  style.textContent = CHROME_CSS;
  (root === document ? (document.head ?? document.documentElement) : root).append(style);
}

/** A row of the help overlay: the keys, and what they do. */
export type HelpRow = readonly [string, string];

export const HELP_KEYS: readonly HelpRow[] = [
  ['→ ↓ Space PgDn', 'Next build or slide'],
  ['← ↑ PgUp', 'Previous'],
  ['Home / End', 'First / last slide'],
  ['number Enter', 'Go to slide'],
  ['B / .', 'Black screen'],
  ['W / ,', 'White screen'],
  ['F', 'Full screen'],
  ['F5', 'Present'],
  ['Esc', 'Stop presenting'],
  ['?', 'This help'],
];

/** The help's row for a page with a text view below the stage. */
export const HELP_TEXT_VIEW: HelpRow = ['Scroll down', 'The slides as text'];

/** The help's rows: [`HELP_KEYS`], with the presenter view's row (if any) after full screen, and the text view's last. */
export function helpRows(presenter: HelpRow | null, textView: boolean): HelpRow[] {
  const rows = HELP_KEYS.flatMap((row) => (row[0] === 'F' && presenter !== null ? [row, presenter] : [row]));
  return textView ? [...rows, HELP_TEXT_VIEW] : rows;
}

export function helpOverlay(document: Document, presenter: HelpRow | null = null, textView = false): HTMLElement {
  const overlay = el(document, 'div', {
    class: 'sd-help',
    role: 'dialog',
    'aria-modal': 'true',
    'aria-label': 'Keyboard shortcuts',
    tabindex: '-1',
  });
  overlay.hidden = true;
  const box = el(document, 'div');
  box.append(el(document, 'p', { class: 'sd-help-title' }, 'Keyboard shortcuts'));
  const list = el(document, 'dl');
  for (const [keys, action] of helpRows(presenter, textView)) {
    list.append(el(document, 'dt', {}, keys), el(document, 'dd', {}, action));
  }
  box.append(list);
  overlay.append(box);
  return overlay;
}
