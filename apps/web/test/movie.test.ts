import { describe, expect, it } from 'vitest';

import { readMovie } from '../src/media/movie';
import { readRuns } from '../src/media/video';

function box(kind: string, ...parts: Uint8Array[]): Uint8Array {
  const length = 8 + parts.reduce((sum, part) => sum + part.length, 0);
  const out = new Uint8Array(length);
  new DataView(out.buffer).setUint32(0, length);
  out.set(
    Array.from(kind, (char) => char.charCodeAt(0)),
    4,
  );
  let at = 8;
  for (const part of parts) {
    out.set(part, at);
    at += part.length;
  }
  return out;
}

/** Big-endian 32-bit (and 16-bit, with `16`) words. */
function words(...values: (number | [number, 16])[]): Uint8Array {
  const out: number[] = [];
  for (const value of values) {
    if (Array.isArray(value)) {
      out.push((value[0] >> 8) & 0xff, value[0] & 0xff);
    } else {
      out.push((value >>> 24) & 0xff, (value >>> 16) & 0xff, (value >>> 8) & 0xff, value & 0xff);
    }
  }
  return Uint8Array.from(out);
}

/**
 * A QuickTime movie of one 16-bit stereo `sowt` sound track at 44.1 kHz as QuickTime writes it: one `stsz` sample per
 * frame of `sampleSize` bytes (1, or the frame's 4), `perChunk` frames a chunk, `frames` in all; its media data
 * follows the movie box when `data` (each frame's bytes its index), else is left out.
 */
function quickTimePcm(frames: number, perChunk: number, sampleSize: number, data: boolean): Blob {
  const chunks = Math.ceil(frames / perChunk);
  const entry = box(
    'sowt',
    new Uint8Array(6),
    words([1, 16], [0, 16], [0, 16], 0, [2, 16], [16, 16], [0, 16], [0, 16], 44_100 * 0x10000),
  );
  const sizeOfMoov = (offset: number) =>
    box(
      'moov',
      box('mvhd', new Uint8Array(4), words(0, 0, 44_100, frames), new Uint8Array(80)),
      box(
        'trak',
        box(
          'tkhd',
          words(3, 0, 0, 1, 0, frames),
          new Uint8Array(16),
          words(0x10000, 0, 0, 0, 0x10000, 0, 0, 0, 0x40000000, 0, 0),
        ),
        box(
          'mdia',
          box('mdhd', new Uint8Array(4), words(0, 0, 44_100, frames, 0)),
          box(
            'hdlr',
            new Uint8Array(4),
            words(0),
            Uint8Array.from('soun', (c) => c.charCodeAt(0)),
            new Uint8Array(13),
          ),
          box(
            'minf',
            box(
              'stbl',
              box('stsd', words(0, 1), entry),
              box('stts', words(0, 1, frames, 1)),
              box('stsc', words(0, 1, 1, perChunk, 1)),
              box('stsz', words(0, sampleSize, frames)),
              box(
                'stco',
                words(0, chunks, ...Array.from({ length: chunks }, (_, index) => offset + index * perChunk * 4)),
              ),
            ),
          ),
        ),
      ),
    );
  const ftyp = box(
    'ftyp',
    Uint8Array.from('qt  ', (c) => c.charCodeAt(0)),
    words(0),
  );
  const head = ftyp.length + sizeOfMoov(0).length + 8;
  const moov = sizeOfMoov(head);
  if (!data) {
    return new Blob([ftyp as BlobPart, moov as BlobPart]);
  }
  const pcm = new Uint8Array(frames * 4).map((_, index) => (index >> 2) & 0xff);
  return new Blob([ftyp as BlobPart, moov as BlobPart, box('mdat', pcm) as BlobPart]);
}

describe('readMovie', () => {
  it('lists uncompressed sound a chunk at a time: ten minutes are a few hundred entries, not 26 million', async () => {
    const frames = 10 * 60 * 44_100;
    const movie = await readMovie(quickTimePcm(frames, 44_100, 1, false));
    const [sound] = movie.tracks;
    expect(sound!.samples.length).toBe(600);
    expect(sound!.samples[1]).toEqual({
      offset: sound!.samples[0]!.offset + 176_400,
      size: 176_400,
      dts: 44_100,
      cts: 44_100,
      duration: 44_100,
      sync: true,
    });
    expect(sound!.samples.reduce((sum, sample) => sum + sample.duration, 0)).toBe(frames);
  });

  it('reads a PCM chunk as its frames times the bytes per frame, whatever size stsz states', async () => {
    for (const sampleSize of [1, 4]) {
      const file = quickTimePcm(70_560, 4_410, sampleSize, true);
      const [sound] = (await readMovie(file)).tracks;
      let read = 0;
      let first: number[] = [];
      await readRuns(
        file,
        sound!.samples,
        44_100 * 4,
        (start, _last, bytes) => {
          read += bytes.length;
          if (start === 0) {
            first = Array.from(bytes.subarray(0, 12));
          }
        },
        undefined,
      );
      expect(read, `stsz ${sampleSize}`).toBe(70_560 * 4);
      expect(first).toEqual([0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2]);
    }
  });

  it('refuses a table listing more samples than it may, and bounds every count by its box', async () => {
    // A fixed-size table of 2^32 - 1 video samples.
    const bytes = new Uint8Array(await quickTimePcm(8, 4, 1, false).arrayBuffer());
    const text = new TextDecoder('latin1').decode(bytes);
    const sowt = text.indexOf('sowt');
    bytes.set(
      Array.from('avc1', (c) => c.charCodeAt(0)),
      sowt,
    );
    const stsz = text.indexOf('stsz');
    new DataView(bytes.buffer).setUint32(stsz + 12, 0xffffffff);
    await expect(readMovie(new Blob([bytes]))).rejects.toThrow(/lists 4294967295 samples/);
    // An stco stating more chunks than its box holds reads only those it holds.
    const short = new Uint8Array(await quickTimePcm(8, 4, 4, false).arrayBuffer());
    const stco = new TextDecoder('latin1').decode(short).indexOf('stco');
    new DataView(short.buffer).setUint32(stco + 8, 0xffffffff);
    const [sound] = (await readMovie(new Blob([short]))).tracks;
    expect(sound!.samples).toHaveLength(2);
  });
});
