// The mount-time sanitizer: an allow-list walk over a slide's markup before it is adopted, behind
// the compile-time lint and the page's CSP. It keeps the lint's element vocabulary (`slidock-format`'s lint), drops
// event handlers and script URLs, keeps links to `#`, `http:`, `https:` and `mailto:`, and lets every other URL,
// in attributes and in CSS (`style`, and SVG presentation attributes), reach only the deck's own files (paths inside
// the deck's folder) and inlined `data:` media.

const HTML_ELEMENTS = new Set(
  (
    'main div span p br hr a h1 h2 h3 h4 h5 h6 section article header footer figure figcaption blockquote pre code ' +
    'b strong i em u s sub sup small mark ol ul li table caption colgroup col thead tbody tfoot tr th td img picture ' +
    'source video audio track'
  ).split(' '),
);

const SVG_ELEMENTS = new Set(
  (
    'svg g defs symbol use desc path rect circle ellipse line polyline polygon text tspan textPath image ' +
    'linearGradient radialGradient stop pattern clipPath mask marker filter feBlend feColorMatrix ' +
    'feComponentTransfer feComposite feDropShadow feFlood feFuncA feFuncB feFuncG feFuncR feGaussianBlur feMerge ' +
    'feMergeNode feOffset'
  ).split(' '),
);

const SVG_NS = 'http://www.w3.org/2000/svg';
const LINK_SCHEMES = new Set(['http', 'https', 'mailto']);
const URL_ATTRIBUTES = new Set([
  'src',
  'poster',
  'href',
  'xlink:href',
  'srcset',
  'background',
  'action',
  'formaction',
  'ping',
]);
/** URL attributes the format never writes. */
const UNSUPPORTED_URL_ATTRIBUTES = new Set(['srcset', 'action', 'formaction', 'ping']);

/** What the walk removed, for a diagnostic. */
export interface SanitizeReport {
  readonly elements: string[];
  readonly attributes: string[];
}

/** `<scheme>:` of an absolute URL, lowercase, after the characters browsers strip from URLs. */
function urlScheme(value: string): string | null {
  // eslint-disable-next-line no-control-regex
  const stripped = value.replace(/[\u0000- ]+/g, '');
  const match = /^([a-z][a-z0-9+.-]*):/i.exec(stripped);
  return match?.[1]?.toLowerCase() ?? null;
}

/** A link target the deck may hold. */
export function isSafeLink(value: string): boolean {
  const trimmed = value.trim();
  if (trimmed.startsWith('#')) {
    return true;
  }
  const scheme = urlScheme(trimmed);
  return scheme !== null && LINK_SCHEMES.has(scheme);
}

// A URL is read the way the browser reads it (backslashes, tabs and all) against two deck folders of different
// schemes, and is the deck's own when it stays inside the folder from both: whatever the folder is named and whether
// the page is served or opened from a file. Slide documents sit one level below the deck's folder, so `../media/…`
// is the deck's.
const DECK_FOLDERS = ['https://deck.invalid/a/', 'file:///b/'];

/** A resource URL the deck may load: a path inside the deck's folder, a fragment, or inlined media. */
export function isSafeResource(value: string): boolean {
  return DECK_FOLDERS.every((folder) => {
    let url: URL;
    try {
      url = new URL(value, `${folder}slides/`);
    } catch {
      return false;
    }
    if (url.protocol === 'data:') {
      return /^data:(image|video|audio)\//i.test(url.href);
    }
    return url.href.startsWith(folder);
  });
}

/** Functions that load from a string without `url()` (`slidock-format`'s lint list): the format never writes them. */
const STRING_LOADERS = new Set(['image-set', '-webkit-image-set', 'image', 'src', 'cross-fade']);
/** The functions a string may sit in: `url()` (a resource, checked) and `path()` (geometry). */
const STRING_FUNCTIONS = new Set(['url', 'path']);

/** Reads the string opening at `start`, escapes decoded; `null` for one that a newline or the end cuts. */
function readString(value: string, start: number): { text: string; next: number } | null {
  const quote = value[start];
  let text = '';
  let i = start + 1;
  while (i < value.length) {
    const c = value[i] ?? '';
    if (c === quote) {
      return { text, next: i + 1 };
    }
    if (c === '\n' || c === '\r' || c === '\f') {
      return null;
    }
    if (c === '\\') {
      const hex = /^[0-9a-f]{1,6}[ \t\n\r\f]?/i.exec(value.slice(i + 1, i + 8));
      if (hex !== null) {
        const code = Number.parseInt(hex[0], 16);
        const valid = code > 0 && code <= 0x10ffff && (code < 0xd800 || code > 0xdfff);
        text += String.fromCodePoint(valid ? code : 0xfffd);
        i += 1 + hex[0].length;
      } else {
        const next = value[i + 1];
        if (next === undefined) {
          return null;
        }
        text += next === '\n' || next === '\r' || next === '\f' ? '' : next;
        i += 2;
      }
      continue;
    }
    text += c;
    i++;
  }
  return null;
}

/**
 * Whether a CSS value (a `style` attribute, an SVG presentation attribute) loads only deck resources: each `url()`
 * is one, no function loads from a string, strings sit only at the top level or in `url()` and `path()`, and no
 * escape outside a string spells a name the scan cannot read. Comments are skipped; an unclosed one is refused.
 */
export function isSafeCss(value: string): boolean {
  if (/expression\s*\(|javascript:|@import|behavior\s*:/i.test(value)) {
    return false;
  }
  const functions: string[] = [];
  let i = 0;
  while (i < value.length) {
    const c = value[i];
    if (c === '\\') {
      return false;
    }
    if (c === '/' && value[i + 1] === '*') {
      const close = value.indexOf('*/', i + 2);
      if (close < 0) {
        return false;
      }
      i = close + 2;
      continue;
    }
    if (c === '"' || c === "'") {
      const string = readString(value, i);
      const fn = functions.at(-1);
      if (string === null || (fn !== undefined && !STRING_FUNCTIONS.has(fn))) {
        return false;
      }
      if (fn === 'url' && !isSafeResource(string.text)) {
        return false;
      }
      i = string.next;
      continue;
    }
    if (c === '(') {
      const name = (/[-\w]*$/.exec(value.slice(Math.max(0, i - 64), i))?.[0] ?? '').toLowerCase();
      if (STRING_LOADERS.has(name)) {
        return false;
      }
      if (name === 'url' && !/^\(\s*['"]/.test(value.slice(i, i + 64))) {
        // An unquoted URL runs to the next `)`; whitespace, quotes or brackets inside make it a bad URL.
        const close = value.indexOf(')', i);
        const url = close < 0 ? '' : value.slice(i + 1, close).trim();
        if (close < 0 || /[\s'"()\\]/.test(url) || !isSafeResource(url)) {
          return false;
        }
        i = close + 1;
        continue;
      }
      functions.push(name);
    } else if (c === ')') {
      functions.pop();
    }
    i++;
  }
  return true;
}

function allowedElement(element: Element): boolean {
  if (element.namespaceURI === SVG_NS) {
    return SVG_ELEMENTS.has(element.localName);
  }
  return HTML_ELEMENTS.has(element.localName.toLowerCase());
}

function sanitizeAttributes(element: Element, report: SanitizeReport): void {
  const isSvg = element.namespaceURI === SVG_NS;
  const isLink = element.localName.toLowerCase() === 'a' && !isSvg;
  for (const attribute of Array.from(element.attributes)) {
    const name = attribute.name.toLowerCase();
    const value = attribute.value;
    let keep: boolean;
    if (name.startsWith('on') || urlScheme(value) === 'javascript') {
      keep = false;
    } else if (name === 'href' && isLink) {
      keep = isSafeLink(value);
    } else if (URL_ATTRIBUTES.has(name)) {
      keep = !UNSUPPORTED_URL_ATTRIBUTES.has(name) && isSafeResource(value);
    } else if (name === 'style' || isSvg) {
      // Any SVG attribute may be a presentation attribute, which is CSS.
      keep = isSafeCss(value);
    } else {
      keep = true;
    }
    if (!keep) {
      element.removeAttributeNode(attribute);
      report.attributes.push(`${element.localName}@${attribute.name}`);
    }
  }
}

/** Sanitizes `root`'s subtree in place and reports what it removed. */
export function sanitize(root: ParentNode): SanitizeReport {
  const report: SanitizeReport = { elements: [], attributes: [] };
  const walk = (parent: ParentNode): void => {
    for (const child of Array.from(parent.childNodes)) {
      if (child.nodeType === 1) {
        const element = child as Element;
        if (!allowedElement(element)) {
          report.elements.push(element.localName);
          element.remove();
          continue;
        }
        sanitizeAttributes(element, report);
        walk(element);
      } else if (child.nodeType !== 3) {
        // Comments and processing instructions.
        child.parentNode?.removeChild(child);
      }
    }
  };
  walk(root);
  return report;
}
