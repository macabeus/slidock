// Mask reveals: conic masks for wheel, clock and wedge animate the registered angle `--sd-a`; a
// soft wipe animates the registered percentage `--sd-p` inside a linear gradient whose edge is feathered over 8% of
// the travel. The mask images live in stylesheet rules keyed by `data-sd-mask` (Chromium resolves a `var()` inside
// an animated mask image once, so the image cannot be a keyframe value); the runner sets the attribute for the
// effect's duration and the slide state clears it. Only the custom property is animated.
import type { ClockParams, Dir, WheelParams } from '@slidock/types';

const FEATHER = 8;

export interface MaskFrames {
  /** The `data-sd-mask` value that selects the mask image. */
  readonly mask: string;
  readonly keyframes: Keyframe[];
}

const HOURS: Readonly<Record<ClockParams['from'], number>> = { '12': 0, '3': 90, '6': 180, '9': 270 };

function clockImage(from: number, cw: boolean): string {
  return cw
    ? `conic-gradient(from ${from}deg, #000 var(--sd-a), transparent 0)`
    : `conic-gradient(from ${from}deg, transparent calc(360deg - var(--sd-a)), #000 0)`;
}

function wheelImage(spokes: number, cw: boolean): string {
  const sector = 360 / spokes;
  return cw
    ? `repeating-conic-gradient(#000 0deg var(--sd-a), transparent 0deg ${sector}deg)`
    : `repeating-conic-gradient(transparent 0deg calc(${sector}deg - var(--sd-a)), #000 0deg ${sector}deg)`;
}

const WEDGE_IMAGE = 'conic-gradient(#000 calc(var(--sd-a) / 2), transparent 0 calc(360deg - var(--sd-a) / 2), #000 0)';

const SIDES: Readonly<Record<Dir, string>> = {
  r: 'right',
  l: 'left',
  d: 'bottom',
  u: 'top',
  rd: 'bottom right',
  ld: 'bottom left',
  ru: 'top right',
  lu: 'top left',
};

function softImage(dir: Dir): string {
  return `linear-gradient(to ${SIDES[dir]}, #000 calc(var(--sd-p) - ${FEATHER}%), transparent var(--sd-p))`;
}

/** Every mask rule, for the slide roots and the stage (`selector` scopes them). */
export function maskCss(selector: string): string {
  const rules: string[] = [];
  const rule = (name: string, image: string) =>
    rules.push(`${selector}[data-sd-mask="${name}"]{-webkit-mask-image:${image};mask-image:${image}}`);
  for (let spokes = 1; spokes <= 8; spokes++) {
    rule(`wheel-${spokes}-cw`, wheelImage(spokes, true));
    rule(`wheel-${spokes}-ccw`, wheelImage(spokes, false));
  }
  for (const [hour, from] of Object.entries(HOURS)) {
    rule(`clock-${hour}-cw`, clockImage(from, true));
    rule(`clock-${hour}-ccw`, clockImage(from, false));
  }
  rule('wedge', WEDGE_IMAGE);
  for (const dir of Object.keys(SIDES) as Dir[]) {
    rule(`soft-${dir}`, softImage(dir));
  }
  return rules.join('');
}

function sweep(mask: string, property: '--sd-a' | '--sd-p', from: string, to: string): MaskFrames {
  return {
    mask,
    keyframes: [
      { offset: 0, [property]: from },
      { offset: 1, [property]: to },
    ],
  };
}

/** A single sweep from an hour of the clock face. */
export function clockMask(params: ClockParams): MaskFrames {
  return sweep(`clock-${params.from}-${params.cw ? 'cw' : 'ccw'}`, '--sd-a', '0deg', '360deg');
}

/** A wheel of 1 to 8 spokes sweeping together. */
export function wheelMask(params: WheelParams): MaskFrames {
  return sweep(`wheel-${params.spokes}-${params.cw ? 'cw' : 'ccw'}`, '--sd-a', '0deg', `${360 / params.spokes}deg`);
}

/** Two sweeps mirrored from the top centre. */
export function wedgeMask(): MaskFrames {
  return sweep('wedge', '--sd-a', '0deg', '360deg');
}

/** A wipe with a feathered leading edge moving along `dir`. */
export function softWipeMask(dir: Dir): MaskFrames {
  return sweep(`soft-${dir}`, '--sd-p', '0%', `${100 + FEATHER}%`);
}
