// Reads the screenshots Playwright takes (8-bit RGB or RGBA PNGs, not interlaced), to compare two renders of a slide.
import { inflateSync } from 'node:zlib';

export interface Pixels {
  width: number;
  height: number;
  /** Three bytes per pixel. */
  rgb: Uint8Array;
}

function paeth(a: number, b: number, c: number): number {
  const p = a + b - c;
  const pa = Math.abs(p - a);
  const pb = Math.abs(p - b);
  const pc = Math.abs(p - c);
  return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
}

export function decode(png: Buffer): Pixels {
  let at = 8;
  let width = 0;
  let height = 0;
  let channels = 0;
  const data: Buffer[] = [];
  while (at < png.length) {
    const length = png.readUInt32BE(at);
    const type = png.toString('latin1', at + 4, at + 8);
    const body = png.subarray(at + 8, at + 8 + length);
    if (type === 'IHDR') {
      width = body.readUInt32BE(0);
      height = body.readUInt32BE(4);
      const depth = body[8];
      const color = body[9];
      if (depth !== 8 || (color !== 2 && color !== 6) || body[12] !== 0) {
        throw new Error(`unsupported PNG: depth ${depth}, colour type ${color}`);
      }
      channels = color === 6 ? 4 : 3;
    } else if (type === 'IDAT') {
      data.push(body);
    }
    at += 12 + length;
  }
  const raw = inflateSync(Buffer.concat(data));
  const stride = width * channels;
  const rgb = new Uint8Array(width * height * 3);
  let previous = new Uint8Array(stride);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)];
    const line = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    const row = new Uint8Array(stride);
    for (let x = 0; x < stride; x++) {
      const left = x >= channels ? (row[x - channels] ?? 0) : 0;
      const up = previous[x] ?? 0;
      const upLeft = x >= channels ? (previous[x - channels] ?? 0) : 0;
      const value = line[x] ?? 0;
      const predicted =
        filter === 1
          ? left
          : filter === 2
            ? up
            : filter === 3
              ? (left + up) >> 1
              : filter === 4
                ? paeth(left, up, upLeft)
                : 0;
      row[x] = (value + predicted) & 0xff;
    }
    for (let x = 0, pixel = y * width * 3; x < stride; x += channels, pixel += 3) {
      rgb[pixel] = row[x] ?? 0;
      rgb[pixel + 1] = row[x + 1] ?? 0;
      rgb[pixel + 2] = row[x + 2] ?? 0;
    }
    previous = row;
  }
  return { width, height, rgb };
}

/** Whether some channel of the pixel at byte `at` differs by more than `tolerance` levels. */
function differs(left: Pixels, right: Pixels, at: number, tolerance: number): boolean {
  for (let channel = at; channel < at + 3; channel++) {
    if (Math.abs((left.rgb[channel] ?? 0) - (right.rgb[channel] ?? 0)) > tolerance) {
      return true;
    }
  }
  return false;
}

/**
 * The pixels whose colour differs (a channel by more than `tolerance` levels) between two screenshots of the same box,
 * leaving out the `inset` outermost rings (every pixel counts as different when the sizes differ).
 */
export function differentPixels(a: Buffer, b: Buffer, inset = 0, tolerance = 0): number {
  const left = decode(a);
  const right = decode(b);
  if (left.width !== right.width || left.height !== right.height) {
    return Math.max(left.width * left.height, right.width * right.height);
  }
  let different = 0;
  for (let y = inset; y < left.height - inset; y++) {
    for (let x = inset; x < left.width - inset; x++) {
      if (differs(left, right, (y * left.width + x) * 3, tolerance)) {
        different++;
      }
    }
  }
  return different;
}

/**
 * The smallest box, in the screenshots' own pixels, holding every pixel whose colour differs (a channel by more than
 * `tolerance` levels) between screenshots `a` and `b` of the same box, and is the same in `a` and `again` (a later
 * screenshot of what `a` shows: a pixel that changes by itself, under an animated picture, says nothing). All of them
 * when the sizes differ; `null` when no pixel does.
 */
export function differenceBox(
  a: Buffer,
  b: Buffer,
  again: Buffer,
  tolerance = 0,
): { x: number; y: number; width: number; height: number } | null {
  const left = decode(a);
  const right = decode(b);
  const steady = decode(again);
  if (left.width !== right.width || left.height !== right.height) {
    return { x: 0, y: 0, width: Math.max(left.width, right.width), height: Math.max(left.height, right.height) };
  }
  const moving = steady.width !== left.width || steady.height !== left.height;
  let x0 = Infinity;
  let y0 = Infinity;
  let x1 = -Infinity;
  let y1 = -Infinity;
  for (let y = 0; y < left.height; y++) {
    for (let x = 0; x < left.width; x++) {
      const at = (y * left.width + x) * 3;
      if (differs(left, right, at, tolerance) && !moving && !differs(left, steady, at, tolerance)) {
        x0 = Math.min(x0, x);
        y0 = Math.min(y0, y);
        x1 = Math.max(x1, x);
        y1 = Math.max(y1, y);
      }
    }
  }
  return x1 < 0 ? null : { x: x0, y: y0, width: x1 - x0 + 1, height: y1 - y0 + 1 };
}
