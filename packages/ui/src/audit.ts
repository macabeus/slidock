// The design rules every screen of the apps keeps, checked in a real browser by the
// apps' end-to-end suites: text contrast at least WCAG AA (the text form fields show too), no horizontal page scroll, a
// visible keyboard focus of at least 3:1 on every control the Tab key reaches, and a name on every control. The
// in-page parts are self-contained functions (a test runner serialises them into the page): `installAuditTools` puts
// the color and focus helpers they share on `window`. `auditPage` drives them through any Playwright-like page.

export type AuditRule = 'contrast' | 'overflow' | 'focus' | 'name';

export interface AuditFinding {
  rule: AuditRule;
  /** A short description of the element: tag, id, classes and text. */
  element: string;
  detail: string;
}

export interface ContrastOptions {
  /** Elements whose text is not the chrome's (a slide, a thumbnail, a chart canvas), with their descendants. */
  skip: readonly string[];
}

/** An sRGB color as bytes, with its alpha in 0–1. */
export type Rgba = [number, number, number, number];

/** The in-page helpers the checks share (`installAuditTools`). */
export interface AuditTools {
  /** Any CSS color resolved to sRGB, or null when the browser cannot draw it. */
  color(css: string): Rgba | null;
  over(top: Rgba, below: Rgba): Rgba;
  /** The color stops of a gradient background image; [] for anything else. */
  stops(image: string): Rgba[];
  /** The opaque colors that may be painted behind `element`'s content (its own background included). */
  backdrop(element: Element | null): Rgba[];
  /** The lowest contrast of `foreground` drawn over each of `behind`, with that pair as `#fg on #bg`. */
  worst(foreground: Rgba, behind: readonly Rgba[]): { ratio: number; pair: string };
  describe(element: Element, text?: string): string;
  /** Whether this browser reports `::placeholder` styles to `getComputedStyle`. */
  placeholderStyles: boolean;
  /** The focus look of an element: its outline and box shadow. */
  signature(element: Element): string;
  /** Records the unfocused look of every element (open shadow roots too) that has none yet, but the focused one. */
  stamp(): void;
  /** The focused element, inside open shadow roots too; null for the document itself. */
  active(): Element | null;
  /** The token of the focus walk under way (`focusFindings`): a step reads it back from the same document. */
  walk: string;
}

declare global {
  interface Window {
    __slidockAudit?: AuditTools;
  }
}

/** Installs the shared helpers on `window` (once per document); false when the page cannot resolve colors. */
export function installAuditTools(): boolean {
  if (window.__slidockAudit) {
    return true;
  }
  const canvas = document.createElement('canvas');
  canvas.width = 1;
  canvas.height = 1;
  const context = canvas.getContext('2d', { willReadFrequently: true });
  if (!context) {
    return false;
  }
  const cache = new Map<string, Rgba | null>();
  // Any CSS color (rgb, color(srgb …), oklab, color-mix results) resolved to sRGB bytes by drawing it. A value the
  // canvas refuses leaves either sentinel in place, so the two readings disagree.
  const color = (css: string): Rgba | null => {
    const known = cache.get(css);
    if (known !== undefined) {
      return known;
    }
    context.fillStyle = '#000';
    context.fillStyle = css;
    const first = context.fillStyle;
    context.fillStyle = '#fff';
    context.fillStyle = css;
    let value: Rgba | null = null;
    if (first === context.fillStyle) {
      context.clearRect(0, 0, 1, 1);
      context.fillRect(0, 0, 1, 1);
      const [r = 0, g = 0, b = 0, a = 0] = context.getImageData(0, 0, 1, 1).data;
      value = [r, g, b, a / 255];
    }
    cache.set(css, value);
    return value;
  };
  const COLOR = /(?:rgba?|hsla?|color|oklab|oklch|lab|lch|hwb)\([^()]*\)|#[0-9a-f]{3,8}\b/gi;
  const stops = (image: string): Rgba[] =>
    image.includes('gradient(')
      ? [...image.matchAll(COLOR)].flatMap((match) => {
          const stop = color(match[0]);
          return stop ? [stop] : [];
        })
      : [];
  const over = (top: Rgba, below: Rgba): Rgba => {
    const a = top[3];
    return [top[0] * a + below[0] * (1 - a), top[1] * a + below[1] * (1 - a), top[2] * a + below[2] * (1 - a), 1];
  };
  const luminance = ([r, g, b]: Rgba) => {
    const channel = (value: number) => {
      const c = value / 255;
      return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
    };
    return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
  };
  const ratio = (a: Rgba, b: Rgba) => {
    const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
    return (hi + 0.05) / (lo + 0.05);
  };
  // The few extremes of a set of colors are enough: contrast is monotonic in luminance.
  const extremes = (colors: Rgba[]): Rgba[] => {
    if (colors.length <= 2) {
      return colors;
    }
    const sorted = [...colors].sort((a, b) => luminance(a) - luminance(b));
    return [sorted[0] as Rgba, sorted[sorted.length - 1] as Rgba];
  };
  /** The element's parent, or the host of the shadow root it sits in. */
  const parent = (node: Element): Element | null => {
    const root = node.getRootNode();
    return node.parentElement ?? (root instanceof ShadowRoot ? root.host : null);
  };
  const backdrop = (element: Element | null): Rgba[] => {
    const layers: Rgba[][] = [];
    for (let node = element; node; node = parent(node)) {
      const style = getComputedStyle(node);
      const clipText = style.backgroundClip === 'text' || style.getPropertyValue('-webkit-background-clip') === 'text';
      const gradient = clipText ? [] : stops(style.backgroundImage);
      const fill = color(style.backgroundColor);
      const layer = [...(fill && fill[3] > 0 ? [fill] : []), ...gradient];
      if (layer.length > 0) {
        layers.push(layer);
        if (layer.every((c) => c[3] >= 1)) {
          break;
        }
      }
    }
    // Composite from the lowest layer up; the canvas default under everything is white.
    let base: Rgba[] = [[255, 255, 255, 1]];
    for (const layer of layers.reverse()) {
      base = extremes(layer.flatMap((top) => base.map((below) => over(top, below))));
    }
    return base;
  };
  const hex = ([r, g, b]: Rgba) => `#${[r, g, b].map((v) => Math.round(v).toString(16).padStart(2, '0')).join('')}`;
  const worst = (foreground: Rgba, behind: readonly Rgba[]) => {
    let lowest = { ratio: Infinity, pair: '' };
    for (const bg of behind) {
      const shown = over(foreground, bg);
      const value = ratio(shown, bg);
      if (value < lowest.ratio) {
        lowest = { ratio: value, pair: `${hex(shown)} on ${hex(bg)}` };
      }
    }
    return lowest;
  };
  const describe = (element: Element, text?: string) => {
    const id = element.id ? `#${element.id}` : '';
    const classes = typeof element.className === 'string' ? element.className.split(/\s+/).slice(0, 4).join('.') : '';
    const shown = (text ?? element.textContent ?? '').trim().replace(/\s+/g, ' ').slice(0, 60);
    return `<${element.tagName.toLowerCase()}${id}${classes ? `.${classes}` : ''}> “${shown}”`;
  };

  // A field with a known placeholder color: the browser reports it, or every placeholder check fails closed.
  const probe = document.createElement('input');
  probe.className = 'slidock-audit-probe';
  probe.placeholder = 'probe';
  probe.style.position = 'fixed';
  probe.style.left = '-9999px';
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.slidock-audit-probe::placeholder { color: rgb(1, 2, 3); }');
  document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
  document.body.append(probe);
  const placeholderStyles = getComputedStyle(probe, '::placeholder').color === 'rgb(1, 2, 3)';
  probe.remove();
  document.adoptedStyleSheets = document.adoptedStyleSheets.filter((candidate) => candidate !== sheet);

  // Every element: the browser decides what the Tab key reaches (Chromium's scrolling containers, for one).
  const elements = () => {
    const found: Element[] = [];
    const visit = (root: Document | ShadowRoot) => {
      for (const element of root.querySelectorAll('*')) {
        found.push(element);
        if (element.shadowRoot) {
          visit(element.shadowRoot);
        }
      }
    };
    visit(document);
    return found;
  };
  const signature = (element: Element) => {
    const style = getComputedStyle(element);
    const outline =
      style.outlineStyle === 'none' || Number.parseFloat(style.outlineWidth) === 0
        ? 'none'
        : `${style.outlineStyle} ${style.outlineWidth} ${style.outlineColor} ${style.outlineOffset}`;
    return `${outline}|${style.boxShadow}`;
  };
  const active = () => {
    let element = document.activeElement;
    while (element?.shadowRoot?.activeElement) {
      element = element.shadowRoot.activeElement;
    }
    return element === document.body || element === document.documentElement ? null : element;
  };
  let next = 0;
  const stamp = () => {
    const focused = active();
    for (const element of elements()) {
      if (element !== focused && !element.hasAttribute('data-audit-rest')) {
        element.setAttribute('data-audit-focus', String(next++));
        element.setAttribute('data-audit-rest', signature(element));
      }
    }
  };

  window.__slidockAudit = {
    color,
    over,
    stops,
    backdrop,
    worst,
    describe,
    placeholderStyles,
    signature,
    stamp,
    active,
    walk: '',
  };
  return true;
}

/**
 * Text contrast against what is painted behind it, per WCAG 2 (4.5:1, 3:1 for large text). Each element that shows
 * text of its own is checked: its color, or for gradient-clipped text each gradient stop, over the layers of
 * background colors and gradient stops of the element and its ancestors, composited down to the first opaque one, with
 * every ancestor's opacity applied. A form field's text is what it shows: its value, or when empty its placeholder (in
 * the `::placeholder` color and opacity), and a drop-down's selected option. Disabled controls, hidden and
 * screen-reader-only text, and glyph separators are exempt (WCAG 1.4.3). Needs `installAuditTools`.
 */
export function contrastFindings(options: ContrastOptions): AuditFinding[] {
  const tools = window.__slidockAudit;
  if (!tools) {
    return [{ rule: 'contrast', element: 'document', detail: 'the audit tools are not installed (installAuditTools)' }];
  }
  // Inputs that show no text of their own: a box, a knob, a swatch, a picture or the browser's own file button.
  const TEXTLESS_INPUTS = new Set(['checkbox', 'radio', 'range', 'color', 'file', 'hidden', 'image']);
  /** The text an element shows and the CSS color it shows it in; null when it shows none of its own. */
  const shown = (element: Element, style: CSSStyleDeclaration) => {
    if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
      if (element instanceof HTMLInputElement && TEXTLESS_INPUTS.has(element.type)) {
        return null;
      }
      if (element.value !== '') {
        return { text: element.value, css: style.color, opacity: 1, placeholder: false };
      }
      if (element.placeholder.trim() === '') {
        return null;
      }
      const pseudo = getComputedStyle(element, '::placeholder');
      const opacity = Number.parseFloat(pseudo.opacity);
      return {
        text: element.placeholder,
        css: pseudo.color,
        opacity: Number.isFinite(opacity) ? opacity : 1,
        placeholder: true,
      };
    }
    if (element instanceof HTMLSelectElement) {
      // A list box shows its options, which are checked as elements.
      return element.multiple || element.size > 1
        ? null
        : { text: element.selectedOptions[0]?.text ?? '', css: style.color, opacity: 1, placeholder: false };
    }
    const text = [...element.childNodes]
      .filter((node) => node.nodeType === Node.TEXT_NODE)
      .map((node) => node.textContent ?? '')
      .join('');
    return { text, css: style.color, opacity: 1, placeholder: false };
  };

  const skip = options.skip.join(',');
  const findings: AuditFinding[] = [];
  for (const element of document.body.querySelectorAll('*')) {
    if ((skip && element.closest(skip)) || element.closest('[disabled], [aria-disabled="true"], svg, script, style')) {
      continue;
    }
    // A drop-down's options show only in the browser's own menu.
    const option = element.closest('option');
    const list = option?.closest('select');
    if (option && !(list && (list.multiple || list.size > 1))) {
      continue;
    }
    if (element instanceof HTMLLabelElement && element.querySelector('input:disabled')) {
      continue;
    }
    const style = getComputedStyle(element);
    const field = shown(element, style);
    // Glyph separators (`·`, `—`) carry no information; they may be dim (`--text-dim`).
    if (!field || /^[\s·•—–/|]*$/.test(field.text)) {
      continue;
    }
    const rect = element.getBoundingClientRect();
    if (rect.width <= 1 || rect.height <= 1 || !element.checkVisibility({ visibilityProperty: true })) {
      continue;
    }
    const described = tools.describe(element, field.text);
    if (field.placeholder && !tools.placeholderStyles) {
      findings.push({
        rule: 'contrast',
        element: described,
        detail: 'this browser reports no ::placeholder style, so the placeholder is unchecked',
      });
      continue;
    }
    const resolved = tools.color(field.css);
    if (!resolved) {
      findings.push({ rule: 'contrast', element: described, detail: `the text color ${field.css} does not resolve` });
      continue;
    }
    const own: Rgba = [resolved[0], resolved[1], resolved[2], resolved[3] * field.opacity];
    const size = Number.parseFloat(style.fontSize);
    const bold = Number.parseInt(style.fontWeight, 10) >= 700;
    const needed = size >= 24 || (bold && size >= 18.66) ? 3 : 4.5;
    const clipText = style.backgroundClip === 'text' || style.getPropertyValue('-webkit-background-clip') === 'text';
    const behind = tools.backdrop(clipText ? element.parentElement : element);
    const foreground = clipText ? tools.stops(style.backgroundImage) : [own];
    // Opacity of the element and its ancestors fades both the text and its backdrop into what lies behind them.
    let opacity = 1;
    let faded: Element | null = null;
    for (let node: Element | null = element; node; node = node.parentElement) {
      const value = Number.parseFloat(getComputedStyle(node).opacity);
      if (value < 1) {
        opacity *= value;
        faded = node;
      }
    }
    const outside = faded ? tools.backdrop(faded.parentElement) : [];
    let worst = { ratio: Infinity, pair: '' };
    for (const bg of behind) {
      for (const fg of foreground) {
        const text = tools.over(fg, bg);
        for (const far of faded ? outside : [bg]) {
          const shownText = faded ? tools.over([text[0], text[1], text[2], opacity], far) : text;
          const shownBg = faded ? tools.over([bg[0], bg[1], bg[2], opacity], far) : bg;
          const pair = tools.worst(shownText, [shownBg]);
          if (pair.ratio < worst.ratio) {
            worst = pair;
          }
        }
      }
    }
    if (worst.ratio < needed) {
      findings.push({
        rule: 'contrast',
        element: described,
        detail: `${worst.ratio.toFixed(2)}:1 (${worst.pair}), needs ${needed}:1 at ${size}px${bold ? ' bold' : ''}${field.placeholder ? ' (placeholder)' : ''}`,
      });
    }
  }
  return findings;
}

/**
 * The page never scrolls sideways, and its content keeps the side gutter (`gutter` px): every text,
 * control, image and canvas lies within it, as far as it shows (a scrolling or clipping ancestor cuts it to its own
 * box first). A wide element is a finding even while the page itself does not scroll.
 */
export function overflowFindings(gutter: number): AuditFinding[] {
  const findings: AuditFinding[] = [];
  const width = document.documentElement.clientWidth;
  const extra = document.documentElement.scrollWidth - width;
  if (extra > 0) {
    findings.push({ rule: 'overflow', element: 'document', detail: `${extra}px wider than the viewport` });
  }
  const LEAVES = new Set(['INPUT', 'SELECT', 'TEXTAREA', 'BUTTON', 'IMG', 'CANVAS', 'VIDEO']);
  for (const element of document.body.querySelectorAll('*')) {
    const ownText = [...element.childNodes].some(
      (node) => node.nodeType === Node.TEXT_NODE && (node.textContent ?? '').trim() !== '',
    );
    if (!ownText && !LEAVES.has(element.tagName)) {
      continue;
    }
    const rect = element.getBoundingClientRect();
    if (rect.width <= 1 || rect.height <= 1 || !element.checkVisibility({ visibilityProperty: true })) {
      continue;
    }
    let left = rect.left;
    let right = rect.right;
    for (let node = element.parentElement; node && node !== document.body; node = node.parentElement) {
      if (getComputedStyle(node).overflowX !== 'visible') {
        const clip = node.getBoundingClientRect();
        left = Math.max(left, clip.left);
        right = Math.min(right, clip.right);
      }
    }
    if (right <= left) {
      continue;
    }
    if (left < gutter - 0.5 || right > width - gutter + 0.5) {
      const text = (element.textContent ?? '').trim().replace(/\s+/g, ' ').slice(0, 50);
      findings.push({
        rule: 'overflow',
        element: `<${element.tagName.toLowerCase()} class="${element.getAttribute('class') ?? ''}"> “${text}”`,
        detail: `spans ${Math.round(left)}–${Math.round(right)} px, outside the ${gutter} px gutter of ${width} px`,
      });
    }
  }
  return findings;
}

/** The key that moves through every control in a test project: `Alt+Tab` for WebKit on macOS (`focusFindings`). */
export function tabKeyFor(browserName: string, platform: string, docker: boolean): string {
  return browserName === 'webkit' && platform === 'darwin' && !docker ? 'Alt+Tab' : 'Tab';
}

/** The structural part of a Playwright `Page` the audit uses. */
export interface AuditPage {
  evaluate<R, A>(fn: (arg: A) => R | Promise<R>, arg: A): Promise<R>;
  keyboard: { press(key: string): Promise<void> };
  locator(selector: string): { ariaSnapshot(): Promise<string> };
  url(): string;
}

const NAMED_ROLES = [
  'button',
  'link',
  'textbox',
  'checkbox',
  'switch',
  'radio',
  'combobox',
  'listbox',
  'tab',
  'slider',
  'spinbutton',
  'searchbox',
  'menuitem',
  'progressbar',
  'dialog',
];

/** The controls in an ARIA snapshot (Playwright's `ariaSnapshot`) that have no accessible name. */
export function unnamedControls(snapshot: string): AuditFinding[] {
  const unnamed = new RegExp(`^\\s*- (${NAMED_ROLES.join('|')})(?:\\s*\\[[^\\]]*\\])*:?\\s*$`);
  return snapshot
    .split('\n')
    .filter((line) => unnamed.test(line))
    .map((line) => ({ rule: 'name' as const, element: line.trim(), detail: 'a control without an accessible name' }));
}

/** What one Tab press focused (`focusStep`). */
interface FocusStep {
  /** The walk's token in the document's audit tools: null or another means the page navigated. */
  walk: string | null;
  focus: {
    key: string;
    /** Its unfocused look, or null when it was never seen unfocused. */
    rest: string | null;
    now: string;
    element: string;
    /** The best contrast of a part of the indicator (an outline or box shadow that changed) against its backdrop. */
    indicator: number;
    part: string;
    /** It is the element shown full screen: it is the whole screen, and draws no indicator. */
    fullscreen: boolean;
  } | null;
}

/**
 * The focused element after a Tab press, once its focus transitions end (1 s at most), and its indicator; then the
 * unfocused look of the controls that appeared meanwhile.
 */
function focusStep(): Promise<FocusStep> {
  const tools = window.__slidockAudit;
  if (!tools) {
    return Promise.resolve({ walk: null, focus: null });
  }
  const walk = tools.walk;
  const settled = (element: Element | null) =>
    Promise.race([
      Promise.all((element?.getAnimations() ?? []).map((animation) => animation.finished.catch(() => undefined))),
      new Promise((resolve) => setTimeout(resolve, 1000)),
    ]);
  return new Promise<void>((resolve) => setTimeout(resolve, 30))
    .then(() => settled(tools.active()))
    .then(() => {
      const probe = document.querySelector('[data-audit-start]');
      const focused = tools.active();
      const element = focused === probe ? null : focused;
      probe?.remove();
      if (!element) {
        tools.stamp();
        return { walk, focus: null };
      }
      const style = getComputedStyle(element);
      const rest = element.getAttribute('data-audit-rest');
      const now = tools.signature(element);
      const [restOutline = '', restShadow = ''] = (rest ?? '').split('|');
      const root = element.getRootNode();
      const host = element.parentElement ?? (root instanceof ShadowRoot ? root.host : null);
      // Each part of the indicator that changed, against what it is drawn over: an inset outline or shadow over the
      // element, any other over its parent.
      const parts: { name: string; color: string; inset: boolean }[] = [];
      const outline = now.split('|')[0] ?? '';
      if (outline !== 'none' && outline !== restOutline) {
        parts.push({ name: 'outline', color: style.outlineColor, inset: Number.parseFloat(style.outlineOffset) < 0 });
      }
      if (style.boxShadow !== 'none' && style.boxShadow !== restShadow) {
        for (const shadow of style.boxShadow.split(/,(?![^()]*\))/)) {
          const css = /(?:rgba?|hsla?|color|oklab|oklch|lab|lch|hwb)\([^()]*\)|#[0-9a-f]{3,8}\b/i.exec(shadow)?.[0];
          parts.push({ name: 'box shadow', color: css ?? shadow, inset: /\binset\b/.test(shadow) });
        }
      }
      let indicator = 0;
      let part = '';
      for (const candidate of parts) {
        const color = tools.color(candidate.color);
        if (color) {
          const { ratio, pair } = tools.worst(color, tools.backdrop(candidate.inset ? element : host));
          if (ratio > indicator) {
            indicator = ratio;
            part = `${candidate.name} ${pair}`;
          }
        }
      }
      const text = (element.getAttribute('aria-label') ?? element.textContent ?? '').trim().slice(0, 50);
      const focus = {
        key: element.getAttribute('data-audit-focus') ?? `new-${element.tagName}-${text}`,
        rest,
        now,
        element: `<${element.tagName.toLowerCase()}> “${text}”`,
        indicator,
        part,
        fullscreen: element === document.fullscreenElement,
      };
      tools.stamp();
      return { walk, focus };
    });
}

/** A Playwright error that says the page's document went away under an evaluation. */
const NAVIGATED = /Execution context was destroyed|because of a navigation|Frame was detached/i;

/**
 * Tabs through the page, from its top (or into the open dialog), and reports every element the Tab key reaches whose
 * outline and box shadow do not change when it takes the focus, or whose changed parts all stay under 3:1 against
 * what they are drawn over (WCAG 1.4.11); the element shown full screen is the whole screen and exempt. Controls that
 * appear during the walk are recorded unfocused before the next press, and one first seen focused is reported. The
 * walk ends when the focus comes back to a control it reached (a wrap, or the last control keeping it, as Firefox's
 * does); a walk that reaches nothing on a page with controls, ends after `limit` presses, or loses its document to a
 * navigation is reported. `tabKey` is the key that moves through every control: `Alt+Tab` in macOS WebKit, whose Tab
 * skips buttons and links unless the system's keyboard navigation is on.
 */
export async function focusFindings(page: AuditPage, tabKey = 'Tab', limit = 400): Promise<AuditFinding[]> {
  if (!(await page.evaluate(installAuditTools, null))) {
    return [{ rule: 'focus', element: 'document', detail: 'the page cannot resolve colors (no 2D canvas)' }];
  }
  const start = await page.evaluate(() => {
    const focused = window.__slidockAudit?.active();
    if (focused instanceof HTMLElement || focused instanceof SVGElement) {
      focused.blur();
    }
    window.__slidockAudit?.stamp();
    // The walk starts from the top of the page, whatever focus came before: browsers start a Tab from where the
    // last focus was, and Firefox's focus stops at the last control instead of wrapping. The first step removes it.
    const start = document.createElement('span');
    start.tabIndex = -1;
    start.setAttribute('data-audit-start', '');
    document.body.prepend(start);
    start.focus({ preventScroll: true });
    const controls = [...document.querySelectorAll('button:not([disabled]), a[href], input:not([disabled])')].filter(
      (element) => element.getBoundingClientRect().width > 1 && element.checkVisibility({ visibilityProperty: true }),
    ).length;
    const walk = Math.random().toString(36).slice(2);
    if (window.__slidockAudit) {
      window.__slidockAudit.walk = walk;
    }
    return { controls, walk };
  }, null);
  const navigated = (why: string): AuditFinding => ({
    rule: 'focus',
    element: 'document',
    detail: `the page navigated during the focus walk (${why}; now ${page.url()}), so its focus is unchecked`,
  });
  const findings: AuditFinding[] = [];
  const seen = new Set<string>();
  let cycled = false;
  for (let step = 0; step < limit && !cycled; step += 1) {
    await page.keyboard.press(tabKey);
    let result: FocusStep;
    try {
      result = await page.evaluate(focusStep, null);
    } catch (error) {
      if (error instanceof Error && NAVIGATED.test(error.message)) {
        return [...findings, navigated(error.message.split('\n')[0] ?? '')];
      }
      throw error;
    }
    if (result.walk !== start.walk) {
      return [...findings, navigated('another document')];
    }
    const focus = result.focus;
    if (!focus) {
      continue;
    }
    if (seen.has(focus.key)) {
      cycled = true;
      continue;
    }
    seen.add(focus.key);
    if (focus.fullscreen) {
      continue;
    }
    if (focus.rest === null) {
      findings.push({
        rule: 'focus',
        element: focus.element,
        detail: 'first seen focused: its unfocused look is unknown',
      });
    } else if (focus.now === focus.rest) {
      findings.push({ rule: 'focus', element: focus.element, detail: `no visible focus (${focus.now})` });
    } else if (focus.indicator < 3) {
      findings.push({
        rule: 'focus',
        element: focus.element,
        detail: focus.part
          ? `the focus indicator is ${focus.indicator.toFixed(2)}:1 (${focus.part}), needs 3:1`
          : `the focus indicator's color does not resolve (${focus.now})`,
      });
    }
  }
  if (seen.size === 0 && start.controls > 0) {
    findings.push({
      rule: 'focus',
      element: 'document',
      detail: `${tabKey} reached none of ${start.controls} controls`,
    });
  } else if (seen.size > 0 && !cycled) {
    findings.push({
      rule: 'focus',
      element: 'document',
      detail: `the walk stopped after ${limit} presses (${seen.size} controls) before the focus came back to one`,
    });
  }
  return findings;
}

/** The structural part of a Playwright `BrowserContext` that `holdDevServerSocket` uses. */
export interface SocketRoutingContext {
  routeWebSocket(url: (url: URL) => boolean, handler: (socket: { close(): Promise<void> }) => void): Promise<void>;
}

/**
 * Answers the Vite dev server's HMR socket (the one whose URL carries its `token`) in every page of `context` with a
 * stub that stays open, so no page under audit reloads: Vite's client reloads the page when its socket drops (a
 * browser in Docker reaches the server through a proxied connection) and on a dependency re-optimization. `drop`
 * closes the stubs, as a dropped connection would.
 */
export async function holdDevServerSocket(context: SocketRoutingContext): Promise<{ drop(): Promise<void> }> {
  const sockets: { close(): Promise<void> }[] = [];
  await context.routeWebSocket(
    (url) => url.searchParams.has('token'),
    (socket) => {
      sockets.push(socket);
    },
  );
  return {
    drop: async () => {
      await Promise.all(sockets.splice(0).map((socket) => socket.close()));
    },
  };
}

/**
 * Every rule on the page as it is now: contrast, overflow and the gutter (16 px unless `gutter` says), names; then the
 * focus walk (it moves the focus), of at most `limit` presses.
 */
export async function auditPage(
  page: AuditPage,
  options: ContrastOptions & { focus?: boolean; gutter?: number; tabKey?: string; limit?: number },
): Promise<AuditFinding[]> {
  if (!(await page.evaluate(installAuditTools, null))) {
    return [{ rule: 'contrast', element: 'document', detail: 'the page cannot resolve colors (no 2D canvas)' }];
  }
  const findings = [
    ...(await page.evaluate(contrastFindings, { skip: options.skip })),
    ...(await page.evaluate(overflowFindings, options.gutter ?? 16)),
    ...unnamedControls(await page.locator('body').ariaSnapshot()),
  ];
  return options.focus === false
    ? findings
    : [...findings, ...(await focusFindings(page, options.tabKey, options.limit))];
}
