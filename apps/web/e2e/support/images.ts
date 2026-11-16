// Pictures in the suite: the size and type of an image file from its header, the entries of a zip, and how alike two
// screenshots are (SSIM).
import { execFileSync } from 'node:child_process';
import { mkdirSync, rmSync } from 'node:fs';

import { type Pixels, decode } from './png';

/** An image file's type and size, read from its header; `null` for another file. */
export function imageHeader(bytes: Buffer): { type: string; width: number; height: number } | null {
  if (bytes.subarray(1, 4).toString('latin1') === 'PNG') {
    return { type: 'image/png', width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20) };
  }
  if (bytes.toString('latin1', 0, 4) === 'RIFF' && bytes.toString('latin1', 8, 12) === 'WEBP') {
    const chunk = bytes.toString('latin1', 12, 16);
    if (chunk === 'VP8X') {
      return { type: 'image/webp', width: bytes.readUIntLE(24, 3) + 1, height: bytes.readUIntLE(27, 3) + 1 };
    }
    if (chunk === 'VP8L') {
      const bits = bytes.readUInt32LE(21);
      return { type: 'image/webp', width: (bits & 0x3fff) + 1, height: ((bits >> 14) & 0x3fff) + 1 };
    }
    if (chunk === 'VP8 ') {
      return { type: 'image/webp', width: bytes.readUInt16LE(26) & 0x3fff, height: bytes.readUInt16LE(28) & 0x3fff };
    }
    return null;
  }
  if (bytes[0] === 0xff && bytes[1] === 0xd8) {
    let at = 2;
    while (at + 9 < bytes.length && bytes[at] === 0xff) {
      const marker = bytes[at + 1] ?? 0;
      const length = bytes.readUInt16BE(at + 2);
      if (marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc) {
        return { type: 'image/jpeg', width: bytes.readUInt16BE(at + 7), height: bytes.readUInt16BE(at + 5) };
      }
      at += 2 + length;
    }
  }
  return null;
}

/** Each entry of a zip, by name, with its bytes. */
export function zipEntries(zip: string): Map<string, Buffer> {
  const names = execFileSync('unzip', ['-Z1', zip], { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 })
    .split('\n')
    .filter((name) => name !== '');
  return new Map(
    names.map((name) => [name, execFileSync('unzip', ['-p', zip, name], { maxBuffer: 1024 * 1024 * 1024 })]),
  );
}

/** `zip` unpacked into `dir`, emptied first. */
export function unpack(zip: string, dir: string): void {
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  execFileSync('unzip', ['-q', zip, '-d', dir]);
}

function luma(pixels: Pixels): Float64Array {
  const out = new Float64Array(pixels.width * pixels.height);
  for (let i = 0; i < out.length; i++) {
    const r = pixels.rgb[i * 3] ?? 0;
    const g = pixels.rgb[i * 3 + 1] ?? 0;
    const b = pixels.rgb[i * 3 + 2] ?? 0;
    out[i] = 0.299 * r + 0.587 * g + 0.114 * b;
  }
  return out;
}

/** The mean SSIM of two screenshots of one size, over 8 × 8 windows of their luma. */
export function ssim(a: Buffer, b: Buffer): number {
  const [left, right] = [decode(a), decode(b)];
  if (left.width !== right.width || left.height !== right.height) {
    throw new Error(`${left.width} × ${left.height} against ${right.width} × ${right.height}`);
  }
  const [x, y] = [luma(left), luma(right)];
  const c1 = (0.01 * 255) ** 2;
  const c2 = (0.03 * 255) ** 2;
  const size = 8;
  let total = 0;
  let windows = 0;
  for (let top = 0; top + size <= left.height; top += size) {
    for (let start = 0; start + size <= left.width; start += size) {
      let [mx, my, sxx, syy, sxy] = [0, 0, 0, 0, 0];
      for (let row = top; row < top + size; row++) {
        for (let col = start; col < start + size; col++) {
          const at = row * left.width + col;
          mx += x[at] ?? 0;
          my += y[at] ?? 0;
        }
      }
      const n = size * size;
      mx /= n;
      my /= n;
      for (let row = top; row < top + size; row++) {
        for (let col = start; col < start + size; col++) {
          const at = row * left.width + col;
          const dx = (x[at] ?? 0) - mx;
          const dy = (y[at] ?? 0) - my;
          sxx += dx * dx;
          syy += dy * dy;
          sxy += dx * dy;
        }
      }
      sxx /= n - 1;
      syy /= n - 1;
      sxy /= n - 1;
      total += ((2 * mx * my + c1) * (2 * sxy + c2)) / ((mx * mx + my * my + c1) * (sxx + syy + c2));
      windows += 1;
    }
  }
  return windows === 0 ? 1 : total / windows;
}
