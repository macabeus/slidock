// Reads a movie's tracks and sample tables from its ISO base media boxes (ISO/IEC 14496-12; QuickTime's movie atoms
// are the same boxes), for a re-encode: the top-level box headers by `Blob` slices, then the whole `moov` (a few MiB
// at most), never the media data. Each track lists its samples in decode order with their byte range, decode and
// composition times and whether they are sync samples, its edit list, and its first sample description: the format,
// the decoder configuration (`avcC`, `hvcC`) of a picture, the channels, rate and `esds` of a sound.

/** A `moov` larger than this is not read: a movie's metadata is kilobytes to a few MiB. */
export const MAX_MOOV_BYTES = 32 * 1024 * 1024;

const MAX_TOP_LEVEL_BOXES = 4096;

/** One sample: its bytes in the file and its times in the track's timescale. */
export interface Sample {
  offset: number;
  size: number;
  /** Decode time. */
  dts: number;
  /** Composition (presentation) time, before the edit list. */
  cts: number;
  /** Until the next sample's decode time. */
  duration: number;
  sync: boolean;
}

/** An edit list entry: `duration` in the movie's timescale, `mediaTime` in the track's (−1: an empty edit). */
export interface Edit {
  duration: number;
  mediaTime: number;
  rate: number;
}

export interface AudioDescription {
  channels: number;
  sampleSize: number;
  sampleRate: number;
  /** `esds`' object type (0x40: MPEG-4 audio). */
  objectType: number | null;
  /** `esds`' decoder specific information (an AAC AudioSpecificConfig). */
  specific: Uint8Array | null;
  /** QuickTime's `lpcm` flags (version 2): float, big-endian, signed. */
  pcmFlags: number | null;
  /** The bytes of one frame (a sample of every channel) of uncompressed sound; `null` for a compressed one. */
  frameBytes: number | null;
}

export interface VisualDescription {
  width: number;
  height: number;
  depth: number;
  /** The decoder configuration: its type (`avcC`, `hvcC`, …) and body. */
  config: { kind: string; bytes: Uint8Array } | null;
  /** The whole sample description box, as the file writes it: what a copy of the track writes again. */
  entry: Uint8Array;
}

export interface MovieTrack {
  id: number;
  /** `vide`, `soun`, … */
  handler: string;
  timescale: number;
  /** In the track's timescale. */
  duration: number;
  edits: Edit[];
  /** The first sample description's format (`avc1`, `mp4a`, `sowt`, …). */
  format: string;
  descriptions: number;
  visual: VisualDescription | null;
  audio: AudioDescription | null;
  samples: Sample[];
  /** `tkhd`'s matrix (a, b, u, c, d, v, x, y, w) and presentation size (in pixels, 16.16 read as a number). */
  matrix: number[];
  width: number;
  height: number;
}

export interface Movie {
  /** `mvhd`'s timescale and duration. */
  timescale: number;
  duration: number;
  tracks: MovieTrack[];
}

export class MovieError extends Error {
  override name = 'MovieError';
}

interface Atom {
  kind: string;
  start: number;
  end: number;
}

function fourcc(view: DataView, at: number): string {
  return String.fromCharCode(view.getUint8(at), view.getUint8(at + 1), view.getUint8(at + 2), view.getUint8(at + 3));
}

function u64(view: DataView, at: number): number {
  return view.getUint32(at) * 2 ** 32 + view.getUint32(at + 4);
}

/** The boxes directly inside `[start, end)`; a box that runs past `end` ends the list. */
function atoms(view: DataView, start: number, end: number): Atom[] {
  const found: Atom[] = [];
  let at = start;
  while (at + 8 <= end) {
    let size = view.getUint32(at);
    const kind = fourcc(view, at + 4);
    let header = 8;
    if (size === 1) {
      if (at + 16 > end) {
        break;
      }
      size = u64(view, at + 8);
      header = 16;
    } else if (size === 0) {
      size = end - at;
    }
    if (size < header || at + size > end) {
      break;
    }
    found.push({ kind, start: at + header, end: at + size });
    at += size;
  }
  return found;
}

function child(view: DataView, within: Atom, kind: string): Atom | null {
  return atoms(view, within.start, within.end).find((atom) => atom.kind === kind) ?? null;
}

function path(view: DataView, within: Atom, kinds: string[]): Atom | null {
  let at: Atom | null = within;
  for (const kind of kinds) {
    at = at && child(view, at, kind);
  }
  return at;
}

/** `mvhd`'s and `mdhd`'s timescale and duration. */
function times(view: DataView, atom: Atom): [number, number] {
  const wide = view.getUint8(atom.start) === 1;
  const at = atom.start + (wide ? 20 : 12);
  return [view.getUint32(at), wide ? u64(view, at + 4) : view.getUint32(at + 4)];
}

function readEdits(view: DataView, elst: Atom | null): Edit[] {
  if (!elst) {
    return [];
  }
  const wide = view.getUint8(elst.start) === 1;
  const count = view.getUint32(elst.start + 4);
  const size = wide ? 20 : 12;
  const edits: Edit[] = [];
  for (let index = 0; index < count && elst.start + 8 + (index + 1) * size <= elst.end; index += 1) {
    const at = elst.start + 8 + index * size;
    const duration = wide ? u64(view, at) : view.getUint32(at);
    const mediaTime = wide ? view.getInt32(at + 8) * 2 ** 32 + view.getUint32(at + 12) : view.getInt32(at + 4);
    edits.push({ duration, mediaTime, rate: view.getInt32(at + (wide ? 16 : 8)) / 0x10000 });
  }
  return edits;
}

/** A descriptor's tag and its body's range: the tag, then the size in up to 4 bytes of 7 bits each. */
function descriptor(view: DataView, at: number, end: number): { tag: number; start: number; end: number } | null {
  if (at >= end) {
    return null;
  }
  const tag = view.getUint8(at);
  let size = 0;
  let cursor = at + 1;
  for (let index = 0; index < 4 && cursor < end; index += 1) {
    const byte = view.getUint8(cursor);
    cursor += 1;
    size = (size << 7) | (byte & 0x7f);
    if ((byte & 0x80) === 0) {
      break;
    }
  }
  return cursor + size <= end ? { tag, start: cursor, end: cursor + size } : null;
}

/** `esds` (ISO/IEC 14496-1 §7.2.6): the decoder config's object type and decoder specific information. */
function readEsds(view: DataView, esds: Atom): { objectType: number | null; specific: Uint8Array | null } {
  const none = { objectType: null, specific: null };
  const es = descriptor(view, esds.start + 4, esds.end);
  if (!es || es.tag !== 3 || es.start + 3 > es.end) {
    return none;
  }
  const flags = view.getUint8(es.start + 2);
  let at = es.start + 3;
  if (flags & 0x80) {
    at += 2;
  }
  if (flags & 0x40) {
    at += 1 + view.getUint8(at);
  }
  if (flags & 0x20) {
    at += 2;
  }
  const config = descriptor(view, at, es.end);
  if (!config || config.tag !== 4) {
    return none;
  }
  const specific = descriptor(view, config.start + 13, config.end);
  return {
    objectType: view.getUint8(config.start),
    specific:
      specific && specific.tag === 5
        ? new Uint8Array(view.buffer, view.byteOffset + specific.start, specific.end - specific.start).slice()
        : null,
  };
}

const VISUAL_FORMATS = new Set(['avc1', 'avc3', 'hvc1', 'hev1', 'vp09', 'av01', 'mp4v', 'jpeg', 'ap4h', 'apch']);

function readVisual(view: DataView, entry: Atom): VisualDescription {
  let config: VisualDescription['config'] = null;
  for (const atom of atoms(view, entry.start + 78, entry.end)) {
    if (config === null && ['avcC', 'hvcC', 'vpcC', 'av1C'].includes(atom.kind)) {
      config = {
        kind: atom.kind,
        bytes: new Uint8Array(view.buffer, view.byteOffset + atom.start, atom.end - atom.start).slice(),
      };
    }
  }
  return {
    width: view.getUint16(entry.start + 24),
    height: view.getUint16(entry.start + 26),
    depth: view.getUint16(entry.start + 74),
    config,
    // A sample description's box has a 32-bit size.
    entry: new Uint8Array(view.buffer, view.byteOffset + entry.start - 8, entry.end - entry.start + 8).slice(),
  };
}

/** A sound description (QuickTime's versions 1 and 2 included): see `slidock_io::movie`. */
function readAudio(view: DataView, entry: Atom): AudioDescription {
  const version = view.getUint16(entry.start + 8);
  let channels = view.getUint16(entry.start + 16);
  let sampleSize = view.getUint16(entry.start + 18);
  let sampleRate = view.getUint32(entry.start + 24) >>> 16;
  let pcmFlags: number | null = null;
  if (version === 2) {
    sampleRate = view.getFloat64(entry.start + 32);
    channels = view.getUint32(entry.start + 40);
    sampleSize = view.getUint32(entry.start + 48);
    pcmFlags = view.getUint32(entry.start + 52);
  }
  const boxes = entry.start + 28 + (version === 1 ? 16 : version === 2 ? 36 : 0);
  const inner = atoms(view, boxes, entry.end);
  const wave = inner.find((atom) => atom.kind === 'wave');
  const esds = inner.find((atom) => atom.kind === 'esds') ?? (wave ? child(view, wave, 'esds') : null);
  const { objectType, specific } = esds ? readEsds(view, esds) : { objectType: null, specific: null };
  // QuickTime's version 1 states the bytes per frame (after the samples and bytes per packet); version 2 the bytes
  // of a packet of one frame; version 0 only the bits per sample.
  const frameBytes = !PCM_FORMATS.has(entry.kind)
    ? null
    : version === 1
      ? view.getUint32(entry.start + 36)
      : version === 2
        ? view.getUint32(entry.start + 56)
        : Math.ceil(sampleSize / 8) * channels;
  return { channels, sampleSize, sampleRate, objectType, specific, pcmFlags, frameBytes: frameBytes || null };
}

/** The formats of uncompressed sound, whose `stsz` QuickTime writes as one sample of a size of 1 per frame. */
const PCM_FORMATS = new Set(['sowt', 'twos', 'lpcm', 'in24', 'in32', 'fl32', 'fl64', 'raw ', 'NONE']);

/** At most this many samples of a track are listed one by one: a 60 fps video of 19 hours. */
export const MAX_SAMPLES = 4_194_304;

/** A table's entry count, bounded by the entries its box holds (each `size` bytes after `header` bytes). */
function tableCount(view: DataView, atom: Atom, header: number, size: number): number {
  return Math.min(
    view.getUint32(atom.start + header - 4),
    Math.max(0, Math.floor((atom.end - atom.start - header) / size)),
  );
}

/**
 * The sample tables of `stbl`, expanded: each sample's offset, size, times and sync flag. Uncompressed sound
 * (`frameBytes`) is listed a chunk at a time, each chunk one "sample" of its frames: QuickTime lists one sample per
 * frame (48,000 a second) and states their size as 1, so a chunk's size is its frames times `frameBytes`, as ffmpeg
 * reads it. Every table's count is bounded by the bytes of its box; a track listing more than `MAX_SAMPLES` samples one
 * by one is refused.
 */
function readSamples(view: DataView, stbl: Atom, frameBytes: number | null): Sample[] {
  const table = (kind: string) => child(view, stbl, kind);
  const stsz = table('stsz');
  const stz2 = table('stz2');
  let fixed = 0;
  let count = 0;
  const sizes: number[] = [];
  if (stsz) {
    fixed = view.getUint32(stsz.start + 4);
    count = fixed !== 0 ? view.getUint32(stsz.start + 8) : tableCount(view, stsz, 12, 4);
    for (let index = 0; fixed === 0 && index < count; index += 1) {
      sizes.push(view.getUint32(stsz.start + 12 + 4 * index));
    }
  } else if (stz2) {
    const field = view.getUint8(stz2.start + 7);
    count = Math.min(
      view.getUint32(stz2.start + 8),
      Math.floor(((stz2.end - stz2.start - 12) * 8) / Math.max(4, field)),
    );
    for (let index = 0; index < count; index += 1) {
      const at = stz2.start + 12;
      sizes.push(
        field === 16
          ? view.getUint16(at + 2 * index)
          : field === 8
            ? view.getUint8(at + index)
            : (view.getUint8(at + (index >> 1)) >> (index % 2 === 0 ? 4 : 0)) & 0x0f,
      );
    }
  }
  // Chunk offsets.
  const stco = table('stco');
  const co64 = table('co64');
  const chunks: number[] = [];
  if (stco) {
    const n = tableCount(view, stco, 8, 4);
    for (let index = 0; index < n; index += 1) {
      chunks.push(view.getUint32(stco.start + 8 + 4 * index));
    }
  } else if (co64) {
    const n = tableCount(view, co64, 8, 8);
    for (let index = 0; index < n; index += 1) {
      chunks.push(u64(view, co64.start + 8 + 8 * index));
    }
  }
  // Samples per chunk: runs of (first chunk, samples per chunk).
  const stsc = table('stsc');
  const runs: [number, number][] = [];
  if (stsc) {
    const n = tableCount(view, stsc, 8, 12);
    for (let index = 0; index < n; index += 1) {
      const at = stsc.start + 8 + 12 * index;
      runs.push([view.getUint32(at), view.getUint32(at + 4)]);
    }
  }
  // Decode deltas: runs of (count, delta).
  const stts = table('stts');
  const deltaRuns: [number, number][] = [];
  if (stts) {
    const n = tableCount(view, stts, 8, 8);
    for (let index = 0; index < n; index += 1) {
      deltaRuns.push([view.getUint32(stts.start + 8 + 8 * index), view.getUint32(stts.start + 12 + 8 * index)]);
    }
  }
  if (frameBytes !== null && fixed !== 0) {
    // Uncompressed sound, a chunk at a time: every frame lasts the first delta.
    const delta = deltaRuns[0]?.[1] ?? 1;
    const samples: Sample[] = [];
    let done = 0;
    for (let run = 0; run < runs.length && done < count; run += 1) {
      const [first, perChunk] = runs[run]!;
      const last = run + 1 < runs.length ? runs[run + 1]![0] - 1 : chunks.length;
      for (let chunk = first; chunk <= last && done < count; chunk += 1) {
        const offset = chunks[chunk - 1];
        if (offset === undefined) {
          throw new MovieError(`chunk ${chunk} has no offset`);
        }
        const frames = Math.min(perChunk, count - done);
        samples.push({
          offset,
          size: frames * frameBytes,
          dts: done * delta,
          cts: done * delta,
          duration: frames * delta,
          sync: true,
        });
        done += frames;
      }
    }
    if (done < count) {
      throw new MovieError(`${count - done} frames have no chunk`);
    }
    return samples;
  }
  if (count > MAX_SAMPLES) {
    throw new MovieError(`it lists ${count} samples`);
  }
  const sizeOf = (index: number) => (fixed !== 0 ? fixed : sizes[index]!);
  const offsets: number[] = [];
  for (let run = 0; run < runs.length && offsets.length < count; run += 1) {
    const [first, perChunk] = runs[run]!;
    const last = run + 1 < runs.length ? runs[run + 1]![0] - 1 : chunks.length;
    for (let chunk = first; chunk <= last && offsets.length < count; chunk += 1) {
      let offset = chunks[chunk - 1];
      if (offset === undefined) {
        throw new MovieError(`chunk ${chunk} has no offset`);
      }
      for (let index = 0; index < perChunk && offsets.length < count; index += 1) {
        offsets.push(offset);
        offset += sizeOf(offsets.length - 1);
      }
    }
  }
  if (offsets.length < count) {
    throw new MovieError(`${count - offsets.length} samples have no chunk`);
  }
  // Decode times.
  const deltas: number[] = [];
  for (const [repeat, delta] of deltaRuns) {
    for (let k = 0; k < repeat && deltas.length < count; k += 1) {
      deltas.push(delta);
    }
  }
  while (deltas.length < count) {
    deltas.push(deltas.at(-1) ?? 0);
  }
  // Composition offsets (signed in version 1, and as QuickTime writes them).
  const ctts = table('ctts');
  const composition: number[] = [];
  if (ctts) {
    const n = tableCount(view, ctts, 8, 8);
    for (let index = 0; index < n && composition.length < count; index += 1) {
      const at = ctts.start + 8 + 8 * index;
      const repeat = view.getUint32(at);
      const offset = view.getInt32(at + 4);
      for (let k = 0; k < repeat && composition.length < count; k += 1) {
        composition.push(offset);
      }
    }
  }
  // Sync samples; none listed means every sample is one.
  const stss = table('stss');
  let sync: Set<number> | null = null;
  if (stss) {
    sync = new Set();
    const n = tableCount(view, stss, 8, 4);
    for (let index = 0; index < n; index += 1) {
      sync.add(view.getUint32(stss.start + 8 + 4 * index));
    }
  }
  const samples: Sample[] = [];
  let dts = 0;
  for (let index = 0; index < count; index += 1) {
    samples.push({
      offset: offsets[index]!,
      size: sizeOf(index),
      dts,
      cts: dts + (composition[index] ?? 0),
      duration: deltas[index]!,
      sync: sync === null || sync.has(index + 1),
    });
    dts += deltas[index]!;
  }
  return samples;
}

function readTrack(view: DataView, trak: Atom): MovieTrack | null {
  const tkhd = child(view, trak, 'tkhd');
  const mdia = child(view, trak, 'mdia');
  const mdhd = mdia && child(view, mdia, 'mdhd');
  const hdlr = mdia && child(view, mdia, 'hdlr');
  const stbl = mdia && path(view, mdia, ['minf', 'stbl']);
  if (!tkhd || !mdia || !mdhd || !hdlr || !stbl) {
    return null;
  }
  const wide = view.getUint8(tkhd.start) === 1;
  // Version and flags, times, id, reserved and duration (24 bytes, 36 in version 1), reserved (8), layer, alternate
  // group, volume and reserved (8), then the matrix (36) and the size (8).
  const matrixAt = tkhd.start + (wide ? 36 : 24) + 16;
  const matrix = Array.from({ length: 9 }, (_, index) => view.getInt32(matrixAt + 4 * index));
  const [timescale, duration] = times(view, mdhd);
  const stsd = child(view, stbl, 'stsd');
  const entries = stsd ? atoms(view, stsd.start + 8, stsd.end) : [];
  const entry = entries[0];
  const handler = fourcc(view, hdlr.start + 8);
  const visual = entry && (handler === 'vide' || VISUAL_FORMATS.has(entry.kind)) ? readVisual(view, entry) : null;
  const audio = entry && handler === 'soun' ? readAudio(view, entry) : null;
  return {
    id: view.getUint32(tkhd.start + (wide ? 20 : 12)),
    handler,
    timescale,
    duration,
    edits: readEdits(view, path(view, trak, ['edts', 'elst'])),
    format: entry?.kind ?? '',
    descriptions: entries.length,
    visual,
    audio,
    samples: readSamples(view, stbl, audio?.frameBytes ?? null),
    matrix,
    width: view.getUint32(matrixAt + 36) / 0x10000,
    height: view.getUint32(matrixAt + 40) / 0x10000,
  };
}

/** Reads `file`'s movie: its `moov` (wherever it sits) and its tracks' sample tables. */
export async function readMovie(file: Blob): Promise<Movie> {
  let offset = 0;
  let moov: { at: number; size: number } | null = null;
  for (let index = 0; index < MAX_TOP_LEVEL_BOXES && offset + 8 <= file.size; index += 1) {
    const head = new DataView(await file.slice(offset, Math.min(file.size, offset + 16)).arrayBuffer());
    let size = head.getUint32(0);
    const kind = fourcc(head, 4);
    let header = 8;
    if (size === 1) {
      size = u64(head, 8);
      header = 16;
    } else if (size === 0) {
      size = file.size - offset;
    }
    if (size < header || offset + size > file.size) {
      break;
    }
    if (kind === 'moov') {
      moov = { at: offset + header, size: size - header };
      break;
    }
    offset += size;
  }
  if (!moov) {
    throw new MovieError('the file has no movie box');
  }
  if (moov.size > MAX_MOOV_BYTES) {
    throw new MovieError(`its movie box is ${moov.size} bytes`);
  }
  const view = new DataView(await file.slice(moov.at, moov.at + moov.size).arrayBuffer());
  const top = atoms(view, 0, view.byteLength);
  const mvhd = top.find((atom) => atom.kind === 'mvhd');
  if (!mvhd) {
    throw new MovieError('the movie box has no header');
  }
  const [timescale, duration] = times(view, mvhd);
  const tracks = top
    .filter((atom) => atom.kind === 'trak')
    .map((trak) => readTrack(view, trak))
    .filter((track): track is MovieTrack => track !== null);
  return { timescale, duration, tracks };
}

/**
 * The span of a track's media its edit list plays, in the track's timescale, and the delay before it in seconds: at
 * most one leading empty edit, then one edit at normal rate (the planner keeps any other list's video as it is).
 */
export function playedSpan(track: MovieTrack, movieTimescale: number): { start: number; end: number; delay: number } {
  let edits = track.edits;
  let delay = 0;
  if (edits[0] && edits[0].mediaTime < 0) {
    delay = edits[0].duration / movieTimescale;
    edits = edits.slice(1);
  }
  const edit = edits[0];
  const mediaEnd = track.samples.reduce((end, sample) => Math.max(end, sample.cts + sample.duration), 0);
  if (!edit) {
    return { start: 0, end: Math.max(mediaEnd, track.duration), delay };
  }
  const start = edit.mediaTime;
  const length = edit.duration > 0 ? (edit.duration / movieTimescale) * track.timescale : mediaEnd - start;
  return { start, end: start + length, delay };
}
