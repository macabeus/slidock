// What effects may animate: opacity, transforms, clip-path, filter, masks and the player's registered
// custom properties. Conic and gradient masks animate a registered `<angle>` (`--sd-a`) or `<percentage>`
// (`--sd-p`) inside the mask image. Colour and font emphases animate the registered `--sd-anim-*` properties: text
// colour through the recolouring rule (`slide.ts`), and the deck's paint hooks through links. On the deck path the
// compiler writes a targeted element's paint as `var(--sd-fill, <static>)` (likewise `--sd-line`, `--sd-color`,
// `--sd-bold`, `--sd-italic`, `--sd-underline`); those hook names stay unregistered, because a registered property
// always has a value and its `var()` fallback (the static render) would never apply. While a step plays or holds
// (`paint.ts`), the slide rules link a hook to its registered property (`--sd-fill: var(--sd-anim-fill)`). The colours inherit, so
// a fill or line held on a target reaches the elements under it that paint the hook. Registration is per window;
// where it is unavailable the effects that need it play their fallback.

/** Keyframe keys an effect may use: the properties above, and WAAPI's own keys. */
export const ALLOWED_KEYFRAME_KEYS: ReadonlySet<string> = new Set([
  'offset',
  'easing',
  'composite',
  'opacity',
  'transform',
  'translate',
  'rotate',
  'scale',
  'clipPath',
  'filter',
  'maskImage',
  'maskSize',
  'maskPosition',
  'maskRepeat',
  '--sd-a',
  '--sd-p',
  '--sd-anim-color',
  '--sd-anim-fill',
  '--sd-anim-line',
  '--sd-anim-bold',
  '--sd-anim-italic',
  '--sd-anim-underline',
]);

export const PROPERTIES = [
  { name: '--sd-a', syntax: '<angle>', inherits: false, initialValue: '0deg' },
  { name: '--sd-p', syntax: '<percentage>', inherits: false, initialValue: '0%' },
  { name: '--sd-anim-color', syntax: '<color>', inherits: true, initialValue: 'black' },
  { name: '--sd-anim-fill', syntax: '<color>', inherits: true, initialValue: 'transparent' },
  { name: '--sd-anim-line', syntax: '<color>', inherits: true, initialValue: 'transparent' },
  { name: '--sd-anim-bold', syntax: 'normal | bold', inherits: false, initialValue: 'normal' },
  { name: '--sd-anim-italic', syntax: 'normal | italic', inherits: false, initialValue: 'normal' },
  { name: '--sd-anim-underline', syntax: 'none | underline', inherits: false, initialValue: 'none' },
] as const;

/** A colour or font a step can change: text, fill and line colours (interpolated), bold, italic, underline (discrete). */
export type Paint = 'text' | 'fill' | 'line' | 'bold' | 'italic' | 'underline';

/** The registered property a paint animates (`--sd-anim-color` for text, `--sd-anim-<paint>` for the others). */
export function paintProperty(paint: Paint): string {
  return `--sd-anim-${paint === 'text' ? 'color' : paint}`;
}

/** The deck's hook (the unregistered variable the compiler's markup reads) of a paint but text: `--sd-<paint>`. */
export function paintHook(paint: Exclude<Paint, 'text'>): string {
  return `--sd-${paint}`;
}

/** The value a font switch takes on or off. */
export function fontValue(paint: 'bold' | 'italic' | 'underline', on: boolean): string {
  if (paint === 'underline') {
    return on ? 'underline' : 'none';
  }
  return on ? paint : 'normal';
}

const registered = new WeakMap<object, boolean>();

interface CssWithRegister {
  registerProperty?: (definition: { name: string; syntax: string; inherits: boolean; initialValue: string }) => void;
}

/** Registers the player's custom properties in `view`; whether they are usable there. */
export function ensureProperties(view: (Window & typeof globalThis) | null): boolean {
  if (view === null) {
    return false;
  }
  const known = registered.get(view);
  if (known !== undefined) {
    return known;
  }
  const css = (view as unknown as { CSS?: CssWithRegister }).CSS;
  let ok = typeof css?.registerProperty === 'function';
  if (ok) {
    for (const property of PROPERTIES) {
      try {
        css?.registerProperty?.({ ...property });
      } catch (error) {
        // Already registered (another player in the page) is fine; anything else is not.
        if (!(error instanceof Error && error.name === 'InvalidModificationError')) {
          ok = false;
        }
      }
    }
  }
  registered.set(view, ok);
  return ok;
}

/** Whether every key of every keyframe is one effects may animate. */
export function allowedKeyframes(keyframes: readonly Keyframe[]): boolean {
  return keyframes.every((frame) => Object.keys(frame).every((key) => ALLOWED_KEYFRAME_KEYS.has(key)));
}
