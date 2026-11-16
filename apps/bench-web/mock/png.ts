// Synthetic slide images for the dev mock and the sample report: an "oracle" drawn from a seed, a "render" of the
// same slide moved and recoloured by how far its class is from a match, and a heatmap in the bench's colour ramp.
// They carry no corpus content.
import { createHash } from 'node:crypto';
import { crc32, deflateSync } from 'node:zlib';

export const WIDTH = 640;
export const HEIGHT = 360;

type Rgb = readonly [number, number, number];

class Canvas {
  readonly data: Uint8Array;
  readonly width: number;
  readonly height: number;

  constructor(width: number, height: number, background: Rgb) {
    this.width = width;
    this.height = height;
    this.data = new Uint8Array(width * height * 4);
    this.rect(0, 0, width, height, background);
  }

  rect(x: number, y: number, w: number, h: number, color: Rgb) {
    const x0 = Math.max(0, Math.round(x));
    const y0 = Math.max(0, Math.round(y));
    const x1 = Math.min(this.width, Math.round(x + w));
    const y1 = Math.min(this.height, Math.round(y + h));
    for (let py = y0; py < y1; py += 1) {
      for (let px = x0; px < x1; px += 1) {
        const i = (py * this.width + px) * 4;
        this.data[i] = color[0];
        this.data[i + 1] = color[1];
        this.data[i + 2] = color[2];
        this.data[i + 3] = 255;
      }
    }
  }
}

function chunk(type: string, body: Uint8Array): Buffer {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(body.length);
  const tagged = Buffer.concat([Buffer.from(type, 'ascii'), body]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(tagged));
  return Buffer.concat([length, tagged, crc]);
}

/** An 8-bit RGBA PNG. */
export function encodePng(width: number, height: number, rgba: Uint8Array): Buffer {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header[8] = 8;
  header[9] = 6;
  const raw = Buffer.alloc((width * 4 + 1) * height);
  for (let y = 0; y < height; y += 1) {
    raw[y * (width * 4 + 1)] = 0;
    raw.set(rgba.subarray(y * width * 4, (y + 1) * width * 4), y * (width * 4 + 1) + 1);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', header),
    chunk('IDAT', deflateSync(raw)),
    chunk('IEND', new Uint8Array()),
  ]);
}

/** How far a render strays from its oracle, by class. */
const DRIFT: Record<string, number> = {
  match: 0.5,
  'motion-match': 0.5,
  close: 3,
  'motion-close': 3,
  'motion-fallback': 5,
  mismatch: 9,
  'motion-mismatch': 9,
};

interface Scene {
  background: Rgb;
  accent: Rgb;
  block: Rgb;
  lines: number;
  layout: number;
}

function scene(seed: string): Scene {
  const bytes = createHash('sha256').update(seed).digest();
  const at = (i: number) => bytes[i] ?? 0;
  const pastel = (i: number): Rgb => [215 + (at(i) % 40), 215 + (at(i + 1) % 40), 215 + (at(i + 2) % 40)];
  const strong = (i: number): Rgb => [40 + (at(i) % 180), 40 + (at(i + 1) % 180), 60 + (at(i + 2) % 180)];
  return { background: pastel(0), accent: strong(3), block: strong(6), lines: 3 + (at(9) % 3), layout: at(10) % 2 };
}

function draw(s: Scene, drift: number, variant: 'oracle' | 'render' | 'golden'): Canvas {
  const canvas = new Canvas(WIDTH, HEIGHT, s.background);
  const dx = variant === 'oracle' ? 0 : drift;
  const dy = variant === 'oracle' ? 0 : drift * 0.6;
  const shade = (color: Rgb, amount: number): Rgb => [
    Math.min(255, color[0] + amount),
    Math.min(255, color[1] + amount),
    Math.max(0, color[2] - amount),
  ];
  canvas.rect(0, 0, WIDTH, 48, s.accent);
  canvas.rect(36 + dx * 0.5, 72, 380, 30, [30, 41, 59]);
  const textX = s.layout === 0 ? 36 : 300;
  const blockX = s.layout === 0 ? 400 : 36;
  for (let line = 0; line < s.lines; line += 1) {
    if (variant !== 'oracle' && drift >= 9 && line === s.lines - 1) {
      continue;
    }
    canvas.rect(textX + dx, 130 + line * 34 + dy, 260 - line * 30, 14, [71, 85, 105]);
  }
  const blockColor = variant === 'oracle' ? s.block : shade(s.block, Math.round(drift * 4));
  canvas.rect(blockX + dx * 1.5, 120 + dy, 200, 150, blockColor);
  canvas.rect(36, 324, 120, 10, s.accent);
  if (variant === 'golden') {
    canvas.rect(blockX + dx * 1.5 + 20, 140 + dy, 60, 60, [255, 255, 255]);
  }
  return canvas;
}

const luma = (data: Uint8Array, i: number) =>
  0.2126 * (data[i] ?? 0) + 0.7152 * (data[i + 1] ?? 0) + 0.0722 * (data[i + 2] ?? 0);

/** Red where the two images differ, over the oracle's faded grey. */
function heatmap(oracle: Canvas, render: Canvas): Canvas {
  const out = new Canvas(WIDTH, HEIGHT, [255, 255, 255]);
  for (let i = 0; i < oracle.data.length; i += 4) {
    const grey = luma(oracle.data, i);
    const worst = Math.min(1, Math.abs(grey - luma(render.data, i)) / 96);
    const faded = 255 - (255 - grey) * 0.25;
    out.data[i] = Math.round(faded + (255 - faded) * worst);
    out.data[i + 1] = Math.round(faded * (1 - worst));
    out.data[i + 2] = Math.round(faded * (1 - worst));
    out.data[i + 3] = 255;
  }
  return out;
}

export type SyntheticKind = 'oracle' | 'render' | 'heatmap' | 'golden' | 'golden-diff';

/** One synthetic image of a row, as PNG bytes. `seed` names the slide (the row id without its frontend). */
export function syntheticImage(seed: string, outcome: string, kind: SyntheticKind): Buffer {
  const s = scene(seed);
  const drift = DRIFT[outcome] ?? 6;
  const oracle = draw(s, drift, 'oracle');
  const pick = (): Canvas => {
    switch (kind) {
      case 'oracle':
        return oracle;
      case 'render':
        return draw(s, drift, 'render');
      case 'heatmap':
        return heatmap(oracle, draw(s, drift, 'render'));
      case 'golden':
        return draw(s, drift, 'golden');
      case 'golden-diff':
        return heatmap(draw(s, drift, 'golden'), draw(s, drift, 'render'));
    }
  };
  const canvas = pick();
  return encodePng(canvas.width, canvas.height, canvas.data);
}

export type FramePhase = 'start' | 'mid' | 'end' | 'still';

/**
 * A motion row's synthetic frame: the slide's block flying in from the left, absent at the start, halfway at the
 * middle, home at the end; a mismatched row's end stops short of its still render.
 */
export function syntheticFrame(seed: string, outcome: string, phase: FramePhase): Buffer {
  const s = scene(seed);
  const canvas = new Canvas(WIDTH, HEIGHT, s.background);
  canvas.rect(0, 0, WIDTH, 48, s.accent);
  canvas.rect(36, 72, 380, 30, [30, 41, 59]);
  const home = s.layout === 0 ? 400 : 36;
  const travel = { start: null, mid: 0.5, end: outcome === 'motion-mismatch' ? 0.9 : 1, still: 1 }[phase];
  if (travel !== null) {
    canvas.rect(home - (1 - travel) * (home + 200), 120, 200, 150, s.block);
  }
  return encodePng(canvas.width, canvas.height, canvas.data);
}
