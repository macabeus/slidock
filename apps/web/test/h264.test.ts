import { describe, expect, it } from 'vitest';

import { colourDescriptionBit, unstatedAvcC, unstatedSample, unstatedSps } from '../src/media/h264';

const hex = (text: string) => Uint8Array.from(text.match(/../g)!.map((byte) => parseInt(byte, 16)));

/** Chromium's encoder's `avcC` for a 640 × 360 stream: its SPS states BT.601 (SMPTE 170M) primaries, transfer and
 * matrix. */
const AVCC = hex('01640c1effe1001367640c1eac18d01405ff2cd41818181e1108d401000468ce3c80fdf8f800');
const SPS = AVCC.subarray(8, 8 + 0x13);

/** The three bytes of colour an SPS states, from the bit `colourDescriptionBit` names. */
function colour(sps: Uint8Array): number[] {
  const at = colourDescriptionBit(sps)!;
  const bit = (index: number) => (sps[index >> 3]! >> (7 - (index & 7))) & 1;
  return [0, 1, 2].map((byte) =>
    Array.from({ length: 8 }, (_, i) => bit(at + byte * 8 + i)).reduce((v, b) => v * 2 + b, 0),
  );
}

describe('an SPS stating no colour', () => {
  it('finds where the SPS states its colour, and rewrites it to unspecified in place', () => {
    expect(colour(SPS)).toEqual([6, 6, 6]);
    const unstated = unstatedSps(SPS);
    expect(unstated.length).toBe(SPS.length);
    expect(colour(unstated)).toEqual([2, 2, 2]);
    // Nothing else changes.
    const differing = Array.from(SPS).filter((byte, index) => byte !== unstated[index]).length;
    expect(differing).toBeLessThanOrEqual(4);
  });

  it('leaves an SPS that states no colour, or cannot be read, as it is', () => {
    const short = hex('6764001f');
    expect(unstatedSps(short)).toBe(short);
  });

  it('rewrites the avcC’s SPS and an SPS repeated in a sample, keeping the rest', () => {
    const avcC = unstatedAvcC(AVCC);
    expect(avcC.length).toBe(AVCC.length);
    expect(colour(avcC.subarray(8, 8 + 0x13))).toEqual([2, 2, 2]);
    expect(Array.from(avcC.subarray(8 + 0x13))).toEqual(Array.from(AVCC.subarray(8 + 0x13)));
    const slice = hex('0000000465888400');
    const sample = new Uint8Array([0, 0, 0, SPS.length, ...SPS, ...slice]);
    const rewritten = unstatedSample(sample);
    expect(colour(rewritten.subarray(4, 4 + SPS.length))).toEqual([2, 2, 2]);
    expect(Array.from(rewritten.subarray(4 + SPS.length))).toEqual(Array.from(slice));
    expect(unstatedSample(slice)).toBe(slice);
  });
});
