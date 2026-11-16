// The colour an H.264 stream states, in its sequence parameter sets' VUI (ITU-T H.264 7.3.2.1.1, E.1.1): a re-encode
// of an untagged video states none of it, as the original. A browser's encoder states its own guess of the frames'
// colour (Chromium's WebCodecs say BT.709 for an untagged stream, where Chromium plays the untagged original of a
// standard-definition size as BT.601), so the stated primaries, transfer and matrix are rewritten to "unspecified"
// (2): every bit keeps its place, so only the parameter set's bytes change.

/** NAL unit type of a sequence parameter set. */
const SPS = 7;

/** "Unspecified" (ISO/IEC 23091-2). */
const UNSPECIFIED = 2;

/** The NAL unit's payload without its emulation prevention bytes, and where each payload byte came from. */
function unescape(nal: Uint8Array): { rbsp: Uint8Array; at: number[] } {
  const rbsp: number[] = [];
  const at: number[] = [];
  let zeros = 0;
  for (let index = 0; index < nal.length; index++) {
    const byte = nal[index]!;
    if (zeros >= 2 && byte === 3) {
      zeros = 0;
      continue;
    }
    zeros = byte === 0 ? zeros + 1 : 0;
    rbsp.push(byte);
    at.push(index);
  }
  return { rbsp: Uint8Array.from(rbsp), at };
}

/** The NAL unit of `rbsp`, emulation prevention bytes added. */
function escape(rbsp: Uint8Array): Uint8Array {
  const out: number[] = [];
  let zeros = 0;
  for (const byte of rbsp) {
    if (zeros >= 2 && byte <= 3) {
      out.push(3);
      zeros = 0;
    }
    zeros = byte === 0 ? zeros + 1 : 0;
    out.push(byte);
  }
  return Uint8Array.from(out);
}

class Bits {
  bit = 0;
  constructor(readonly bytes: Uint8Array) {}

  u(count: number): number {
    let value = 0;
    for (let index = 0; index < count; index++) {
      const byte = this.bytes[this.bit >> 3];
      if (byte === undefined) {
        throw new RangeError('the parameter set ends early');
      }
      value = value * 2 + ((byte >> (7 - (this.bit & 7))) & 1);
      this.bit += 1;
    }
    return value;
  }

  ue(): number {
    let zeros = 0;
    while (this.u(1) === 0) {
      zeros += 1;
      if (zeros > 31) {
        throw new RangeError('an Exp-Golomb code past 32 bits');
      }
    }
    return 2 ** zeros - 1 + this.u(zeros);
  }

  se(): number {
    const code = this.ue();
    return code % 2 === 1 ? (code + 1) / 2 : -(code / 2);
  }
}

function skipScalingList(bits: Bits, size: number) {
  let last = 8;
  let next = 8;
  for (let index = 0; index < size; index++) {
    if (next !== 0) {
      next = (last + bits.se() + 256) % 256;
    }
    last = next === 0 ? last : next;
  }
}

/** The bit (of the RBSP, its header byte included) where an SPS states its colour primaries, or `null`. */
export function colourDescriptionBit(rbsp: Uint8Array): number | null {
  const bits = new Bits(rbsp);
  if ((bits.u(8) & 0x1f) !== SPS) {
    return null;
  }
  const profile = bits.u(8);
  bits.u(16);
  bits.ue();
  if ([100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135].includes(profile)) {
    const chroma = bits.ue();
    if (chroma === 3) {
      bits.u(1);
    }
    bits.ue();
    bits.ue();
    bits.u(1);
    if (bits.u(1) === 1) {
      for (let index = 0; index < (chroma === 3 ? 12 : 8); index++) {
        if (bits.u(1) === 1) {
          skipScalingList(bits, index < 6 ? 16 : 64);
        }
      }
    }
  }
  bits.ue();
  const order = bits.ue();
  if (order === 0) {
    bits.ue();
  } else if (order === 1) {
    bits.u(1);
    bits.se();
    bits.se();
    const cycle = bits.ue();
    for (let index = 0; index < cycle; index++) {
      bits.se();
    }
  }
  bits.ue();
  bits.u(1);
  bits.ue();
  bits.ue();
  if (bits.u(1) === 0) {
    bits.u(1);
  }
  bits.u(1);
  if (bits.u(1) === 1) {
    bits.ue();
    bits.ue();
    bits.ue();
    bits.ue();
  }
  if (bits.u(1) === 0) {
    return null;
  }
  // VUI: aspect ratio, overscan, then the video signal type.
  if (bits.u(1) === 1 && bits.u(8) === 255) {
    bits.u(32);
  }
  if (bits.u(1) === 1) {
    bits.u(1);
  }
  if (bits.u(1) === 0) {
    return null;
  }
  bits.u(4);
  return bits.u(1) === 1 ? bits.bit : null;
}

/** `nal` (an SPS) stating unspecified primaries, transfer and matrix, or itself when it states none. */
export function unstatedSps(nal: Uint8Array): Uint8Array {
  const { rbsp } = unescape(nal);
  let at: number | null;
  try {
    at = colourDescriptionBit(rbsp);
  } catch {
    return nal;
  }
  if (at === null) {
    return nal;
  }
  const out = rbsp.slice();
  for (let bit = 0; bit < 24; bit++) {
    const value = (UNSPECIFIED >> (7 - (bit % 8))) & 1;
    const index = (at + bit) >> 3;
    const mask = 1 << (7 - ((at + bit) & 7));
    out[index] = value === 1 ? out[index]! | mask : out[index]! & ~mask;
  }
  return escape(out);
}

/** An `avcC` whose sequence parameter sets state no colour (`unstatedSps`). */
export function unstatedAvcC(avcC: Uint8Array): Uint8Array {
  if (avcC.length < 7) {
    return avcC;
  }
  const out: number[] = Array.from(avcC.subarray(0, 5));
  let at = 5;
  const sets = avcC[at]! & 0x1f;
  out.push(avcC[at]!);
  at += 1;
  for (let index = 0; index < sets; index++) {
    const length = (avcC[at]! << 8) | avcC[at + 1]!;
    const nal = unstatedSps(avcC.subarray(at + 2, at + 2 + length));
    out.push(nal.length >> 8, nal.length & 0xff, ...nal);
    at += 2 + length;
  }
  out.push(...avcC.subarray(at));
  return Uint8Array.from(out);
}

/** A sample (4-byte NAL lengths) whose in-band sequence parameter sets state no colour. */
export function unstatedSample(sample: Uint8Array): Uint8Array {
  const parts: Uint8Array[] = [];
  let changed = false;
  for (let at = 0; at + 4 <= sample.length;) {
    const length = ((sample[at]! << 24) | (sample[at + 1]! << 16) | (sample[at + 2]! << 8) | sample[at + 3]!) >>> 0;
    const nal = sample.subarray(at + 4, at + 4 + length);
    const next = (nal[0]! & 0x1f) === SPS ? unstatedSps(nal) : nal;
    changed ||= next !== nal;
    parts.push(
      Uint8Array.of(next.length >>> 24, (next.length >> 16) & 0xff, (next.length >> 8) & 0xff, next.length & 0xff),
      next,
    );
    at += 4 + length;
  }
  if (!changed) {
    return sample;
  }
  const out = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}
