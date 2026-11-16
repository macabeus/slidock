// The presenter window's frame, the web app's and a deck page's alike: the slidock header card (logo, title,
// "Presenter view · <deck title>", the theme toggle, the brand strip) over a main area that holds the presenter view
// or a status (a banner, or a waiting line). Plain DOM, styled by `style.ts`.
import { el } from '../dom';
import { ensurePresenterStyle } from './style';
import { NEXT_THEME, type ThemeChoice, applyTheme, readTheme, themeLabels, writeTheme } from './theme';

/** Outline icons on a 24 × 24 grid (Heroicons v1 outline, MIT, Copyright (c) Tailwind Labs, Inc.), as `@slidock/ui`. */
const ICONS = {
  desktop: 'M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z',
  sun: 'M12 3v1m0 16v1m9-9h-1M4 12H3m15.364 6.364l-.707-.707M6.343 6.343l-.707-.707m12.728 0l-.707.707M6.343 17.657l-.707.707M16 12a4 4 0 11-8 0 4 4 0 018 0z',
  moon: 'M20.354 15.354A9 9 0 018.646 3.646 9.003 9.003 0 0012 21a9.003 9.003 0 008.354-5.646z',
  exclamation:
    'M12 9v3m0 3h.01M4.062 19h15.876c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L2.33 16c-.77 1.333.192 3 1.732 3z',
} as const;

const THEME_ICON: Readonly<Record<ThemeChoice, keyof typeof ICONS>> = { system: 'desktop', light: 'sun', dark: 'moon' };

/** The slidock mark: two stacked slides in the brand gradient (the web app's logo). */
const LOGO =
  '<svg viewBox="0 0 64 64" aria-hidden="true"><defs><linearGradient id="sd-logo" x1="0" y1="0" x2="1" y2="1">' +
  '<stop offset="0" stop-color="#f472b6"/><stop offset="0.5" stop-color="#fb7185"/><stop offset="1" stop-color="#fb923c"/>' +
  '</linearGradient></defs><rect x="14" y="8" width="42" height="30" rx="5" fill="url(#sd-logo)" opacity="0.45"/>' +
  '<rect x="8" y="18" width="42" height="30" rx="5" fill="url(#sd-logo)"/>' +
  '<path d="M24 26v14l12-7z" fill="#fff" opacity="0.92"/><rect x="20" y="52" width="18" height="4" rx="2" fill="url(#sd-logo)"/></svg>';

function icon(document: Document, name: keyof typeof ICONS): SVGSVGElement {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  for (const [attribute, value] of [
    ['viewBox', '0 0 24 24'],
    ['fill', 'none'],
    ['stroke', 'currentColor'],
    ['stroke-width', '1.75'],
    ['stroke-linecap', 'round'],
    ['stroke-linejoin', 'round'],
    ['aria-hidden', 'true'],
  ] as const) {
    svg.setAttribute(attribute, value);
  }
  const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
  path.setAttribute('d', ICONS[name]);
  svg.append(path);
  return svg;
}

/** What the main area shows instead of the presenter view. */
export type PresenterStatus =
  | { readonly kind: 'waiting'; readonly text: string }
  | { readonly kind: 'warning'; readonly title: string; readonly text: string };

export interface PresenterWindow {
  /** Where the presenter view goes. */
  readonly main: HTMLElement;
  /** The deck's title in the header's subtitle; `null` for none. */
  setTitle(title: string | null): void;
  /** A status in the main area, before the presenter view; `null` for none. */
  setStatus(status: PresenterStatus | null): void;
  destroy(): void;
}

/** Draws the presenter window's frame in `host`, and applies the remembered theme. */
export function createPresenterWindow(host: HTMLElement): PresenterWindow {
  const document = host.ownerDocument;
  const view = document.defaultView;
  ensurePresenterStyle(document);
  const root = el(document, 'div', { class: 'sd-pw' });
  const header = el(document, 'header', { class: 'sd-pw-header' });
  const bar = el(document, 'div', { class: 'sd-pw-bar' });
  const brand = el(document, 'div', { class: 'sd-pw-brand' });
  const logo = el(document, 'div', { class: 'sd-pw-logo' });
  const mark = el(document, 'div', { class: 'sd-pw-mark' });
  mark.innerHTML = LOGO;
  logo.append(el(document, 'div', { class: 'sd-pw-glow', 'aria-hidden': 'true' }), mark);
  const titles = el(document, 'div', { class: 'sd-pw-titles' });
  const subtitle = el(document, 'div', { class: 'sd-pw-subtitle' }, 'Presenter view');
  titles.append(el(document, 'p', { class: 'sd-pw-title' }, 'slidock'), subtitle);
  brand.append(logo, titles);
  const actions = el(document, 'div', { class: 'sd-pw-actions' });
  const toggle = el(document, 'button', { type: 'button', class: 'sd-pw-theme' });
  actions.append(toggle);
  bar.append(brand, actions);
  header.append(bar, el(document, 'div', { class: 'sd-pw-strip', 'aria-hidden': 'true' }));
  const main = el(document, 'main', { class: 'sd-pw-main', 'aria-label': 'Presenter view' });
  root.append(header, main);
  host.append(root);

  let choice: ThemeChoice = view === null ? 'system' : readTheme(view);
  const showTheme = (): void => {
    const { label, title } = themeLabels(choice);
    toggle.setAttribute('aria-label', label);
    toggle.setAttribute('title', title);
    toggle.replaceChildren(icon(document, THEME_ICON[choice]));
  };
  applyTheme(document.documentElement, choice);
  showTheme();
  toggle.addEventListener('click', () => {
    choice = NEXT_THEME[choice];
    if (view !== null) {
      writeTheme(view, choice);
    }
    showTheme();
  });

  let status: HTMLElement | null = null;
  return {
    main,
    setTitle: (title) => {
      subtitle.textContent = title === null ? 'Presenter view' : `Presenter view · ${title}`;
    },
    setStatus: (next) => {
      status?.remove();
      status = null;
      if (next === null) {
        return;
      }
      if (next.kind === 'waiting') {
        status = el(document, 'p', { class: 'sd-pw-waiting', role: 'status' }, next.text);
      } else {
        status = el(document, 'div', { class: 'sd-pw-banner', role: 'alert' });
        const body = el(document, 'div');
        body.append(
          el(document, 'p', { class: 'sd-pw-banner-title' }, next.title),
          el(document, 'div', { class: 'sd-pw-banner-text' }, next.text),
        );
        status.append(icon(document, 'exclamation'), body);
      }
      main.prepend(status);
    },
    destroy: () => root.remove(),
  };
}
