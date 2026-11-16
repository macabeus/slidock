// Colour and font changes on the stage. While a step plays, `data-sd-paint` links what it changes
// to its registered property (`properties.ts`), which the animation interpolates (colours) or switches (fonts); once
// it settles, `data-sd-hold` holds the value it left as a plain declaration. Text colour needs nothing from the markup: the target and everything under it take `--sd-anim-color`, each
// run animated from its own colour. Fill, line and font changes play through the deck's paint hooks: the compiler
// declares the hook with its static value on a targeted element (`--sd-fill:#4472c4`) and writes the paint under it
// as `var(--sd-fill, <static>)` (`--sd-line`, `--sd-bold`, `--sd-italic`, `--sd-underline`), and the link sets the
// hook to the registered property, over that declaration. Neither linked nor held, the markup shows its static value.
import { type Paint, fontValue, paintHook, paintProperty } from './effects/properties';

type Hooked = Exclude<Paint, 'text'>;

const HOOKED: readonly Hooked[] = ['fill', 'line', 'bold', 'italic', 'underline'];

const PAINTS: readonly Paint[] = ['text', ...HOOKED];

/** The unregistered variable a settled state's paint is written in. */
const held = (paint: Paint) => `--sd-held-${paint}`;

/** The elements a link or a hold of `paint` makes read its registered property (`under`: the ones under them). */
const reading = (paint: Paint, under = '') => `[data-sd-paint~=${paint}]${under},[data-sd-hold~=${paint}]${under}`;

/**
 * The slide rules of colour and font changes. A paint that plays is linked (`data-sd-paint`) to its animated property;
 * a paint a settled state shows is held (`data-sd-hold`): a plain declaration of its value, read the same way, so a
 * settled step leaves no animation, no inline registered value and no stacking context of its own behind.
 */
export const PAINT_CSS =
  PAINTS.map(
    (paint) =>
      `[data-sd-hold~=${paint}]{${paintProperty(paint)}:var(${held(paint)})}` +
      (paint === 'text' ? `${reading(paint)},${reading(paint, ' *')}{color` : `${reading(paint)}{${paintHook(paint)}`) +
      `:var(${paintProperty(paint)})!important}`,
  ).join('') +
  // A font switch repaints only its text's own box in Chromium, which leaves the outset ink of a synthetic bold or
  // italic unpainted until something else repaints the slide: as its own stacking context, a target repaints whole.
  '[data-sd-paint]{isolation:isolate}';

/** Elements under a target read for their own colour at most, and how many start apart. */
const RUN_SCAN = 4096;
const MAX_RUNS = 512;
/** The style text read for a hook's property at most. */
const STYLE_SCAN = 4096;

function selector(paint: Hooked): string {
  return `[style*="var(${paintHook(paint)}"]`;
}

/** An element a change animates, and the value it starts from when the keyframes have no first frame. */
export interface Run {
  readonly run: Element;
  readonly from: string;
}

/**
 * The element and the elements under it whose text colour differs from their parent's, with that colour (the rest
 * inherit theirs). The scan is bounded; elements past it inherit.
 */
function colorRuns(element: Element): Run[] {
  const view = element.ownerDocument.defaultView;
  const colorOf = (e: Element) => view?.getComputedStyle(e).color || 'black';
  const own = colorOf(element);
  const runs: Run[] = [{ run: element, from: own }];
  const stack: [Element, string][] = [[element, own]];
  let scanned = 0;
  for (let next = stack.pop(); next !== undefined && runs.length < MAX_RUNS; next = stack.pop()) {
    const [parent, parentColor] = next;
    for (const child of Array.from(parent.children)) {
      if (++scanned > RUN_SCAN) {
        return runs;
      }
      const color = colorOf(child);
      if (color !== parentColor) {
        runs.push({ run: child, from: color });
      }
      stack.push([child, color]);
    }
  }
  return runs;
}

/**
 * The property of the first declaration in `style` whose value reads `hook`, or `null`. A value holding a `;` inside
 * a string or a `url()` before the hook can name the wrong property, whose computed value is then not a colour.
 */
export function hookProperty(style: string, hook: string): string | null {
  return (
    new RegExp(`(?:^|;)\\s*([a-z-]+)\\s*:[^;]*var\\(${hook}\\b`, 'i')
      .exec(style.slice(0, STYLE_SCAN))?.[1]
      ?.toLowerCase() ?? null
  );
}

/**
 * The elements under a colour change that paint its hook, each with the colour it shows now (its static fallback, or
 * what a held change left): the property that reads the hook, computed. An element whose paint there is not a colour
 * (a gradient) is left out. Bounded.
 */
function hookRuns(element: Element, paint: 'fill' | 'line'): Run[] {
  const view = element.ownerDocument.defaultView;
  if (view === null) {
    return [];
  }
  const match = selector(paint);
  const candidates = [...(element.matches(match) ? [element] : []), ...Array.from(element.querySelectorAll(match))];
  const runs: Run[] = [];
  for (const run of candidates.slice(0, MAX_RUNS)) {
    const property = hookProperty(run.getAttribute('style') ?? '', paintHook(paint));
    if (property === null) {
      continue;
    }
    // A shorthand (`background`, `border`, `outline`) computes its colour in its `-color` property.
    const style = view.getComputedStyle(run);
    const from = [property, `${property}-color`]
      .map((name) => style.getPropertyValue(name).trim())
      .find((value) => value !== '' && view.CSS?.supports?.('color', value) === true);
    if (from !== undefined) {
      runs.push({ run, from });
    }
  }
  return runs;
}

/**
 * The elements a change of `paint` animates on `element`, read before any of them is linked: each text run, each
 * element painting a fill or line hook, or the element itself for a font switch where it or an element under it
 * reads the hook (the elements under it inherit). None: the change cannot play on `element`.
 */
export function paintRuns(element: Element, paint: Paint): Run[] {
  switch (paint) {
    case 'text':
      return colorRuns(element);
    case 'fill':
    case 'line':
      return hookRuns(element, paint);
    default: {
      const match = selector(paint);
      const reader = element.matches(match) ? element : element.querySelector(match);
      const style = reader === null ? null : element.ownerDocument.defaultView?.getComputedStyle(reader);
      // The switch it shows now, from the font the hook's reader computes.
      const on =
        paint === 'bold'
          ? Number(style?.fontWeight) >= 600
          : paint === 'italic'
            ? (style?.fontStyle ?? 'normal') !== 'normal'
            : (style?.textDecorationLine ?? '').includes('underline');
      return reader === null ? [] : [{ run: element, from: fontValue(paint, on) }];
    }
  }
}

type Styled = Element & ElementCSSInlineStyle;

function mark(element: Element, name: string, paint: Paint): void {
  const tokens = (element.getAttribute(name) ?? '').split(' ').filter(Boolean);
  if (!tokens.includes(paint)) {
    element.setAttribute(name, [...tokens, paint].join(' '));
  }
}

/**
 * Links `paint` on `element`: the value it shows now (`from`) becomes the registered property's own value, under every
 * animation: what shows before a change begins, or between the changes of one paint.
 */
export function linkPaint(element: Element, paint: Paint, from: string): void {
  mark(element, 'data-sd-paint', paint);
  (element as Styled).style?.setProperty(paintProperty(paint), from);
}

/** Holds `value` (a `#rrggbb` colour or a switch, as the deck's vocabulary checks them) of `paint` on `element`. */
export function holdPaint(element: Element, paint: Paint, value: string): void {
  mark(element, 'data-sd-hold', paint);
  (element as Styled).style?.setProperty(held(paint), value);
}

/**
 * Removes every link and hold, and the values they gave, from the elements under `root`, but the links `keep` names.
 */
export function unlinkPaints(
  root: ParentNode,
  keep: readonly (readonly [Element, Paint | undefined] | readonly [])[],
): void {
  for (const element of Array.from(root.querySelectorAll('[data-sd-paint],[data-sd-hold]'))) {
    element.removeAttribute('data-sd-paint');
    element.removeAttribute('data-sd-hold');
    for (const paint of PAINTS) {
      (element as Styled).style?.removeProperty(held(paint));
      if (keep.some(([e, p]) => e === element && p === paint)) {
        mark(element, 'data-sd-paint', paint);
      } else {
        (element as Styled).style?.removeProperty(paintProperty(paint));
      }
    }
  }
}
