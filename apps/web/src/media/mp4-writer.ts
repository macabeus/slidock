// Writes an MP4 (ISO/IEC 14496-12, 14496-14) of one H.264 video track (its colour stated by the stream alone) and at
// most one AAC sound track, its movie boxes first so a player starts before it has read the whole file ("fast start"). The media data is laid out in
// chunks the caller interleaves by time; each chunk's bytes are `Blob`s (encoded frames, or ranges of the original
// file for a sound carried over as it is), so the file is assembled without holding it in memory.

/** A sample of the video track, in the track's timescale. */
export interface VideoSample {
  size: number;
  duration: number;
  sync: boolean;
  /** Its composition time less its decode time, in the track's timescale (a copied stream with B-frames). */
  offset?: number;
}

/** A sample of the sound track. */
export interface AudioSample {
  size: number;
  duration: number;
}

export interface VideoTrackOut {
  timescale: number;
  width: number;
  height: number;
  /** The H.264 decoder configuration record (`avcC`'s body). */
  avcC: Uint8Array;
  /** A copied track's own sample description box, written as it is in place of an `avc1` around `avcC`. */
  entry?: Uint8Array;
  /** A copied track's `tkhd` matrix (the identity otherwise) and presentation size (`width` × `height` otherwise). */
  matrix?: number[];
  presented?: [number, number];
  samples: VideoSample[];
  /** The edit list to write: (duration in the movie's timescale, media time in the track's; −1 an empty edit). */
  edits: { duration: number; mediaTime: number }[];
  /** The movie's length in its timescale when longer than its tracks' (the original's, its sound left out). */
  movieDuration?: number;
}

export interface AudioTrackOut {
  timescale: number;
  sampleRate: number;
  channels: number;
  /** The AudioSpecificConfig (ISO/IEC 14496-3) for `esds`. */
  specific: Uint8Array;
  /** The sound's average bitrate in bits per second, for `esds`. */
  bitrate: number;
  samples: AudioSample[];
  /** The edit list to write: (duration in the movie's timescale, media time in the track's; −1 an empty edit). */
  edits: { duration: number; mediaTime: number }[];
}

/** A run of samples of one track, laid out together in the media data. */
export interface Chunk {
  track: 'video' | 'audio';
  /** How many samples, from where the track's last chunk ended. */
  samples: number;
  bytes: Blob;
}

/** The movie's timescale. */
const MOVIE_TIMESCALE = 1000;

class Writer {
  #parts: Uint8Array[] = [];
  #length = 0;

  get length() {
    return this.#length;
  }

  bytes(bytes: ArrayLike<number>) {
    const part = Uint8Array.from(bytes);
    this.#parts.push(part);
    this.#length += part.length;
  }

  u8(value: number) {
    this.bytes([value & 0xff]);
  }

  u16(value: number) {
    this.bytes([(value >>> 8) & 0xff, value & 0xff]);
  }

  u32(value: number) {
    this.bytes([(value >>> 24) & 0xff, (value >>> 16) & 0xff, (value >>> 8) & 0xff, value & 0xff]);
  }

  i32(value: number) {
    this.u32(value >>> 0);
  }

  u64(value: number) {
    this.u32(Math.floor(value / 2 ** 32));
    this.u32(value % 2 ** 32);
  }

  text(value: string) {
    this.bytes(Array.from(value, (char) => char.charCodeAt(0)));
  }

  zeros(count: number) {
    this.bytes(new Uint8Array(count));
  }

  /** A box around what `body` writes. */
  box(kind: string, body: (w: Writer) => void) {
    const inner = new Writer();
    body(inner);
    this.u32(inner.length + 8);
    this.text(kind);
    this.#parts.push(...inner.#parts);
    this.#length += inner.length;
  }

  /** A full box: version and flags, then `body`. */
  full(kind: string, version: number, flags: number, body: (w: Writer) => void) {
    this.box(kind, (w) => {
      w.u32(((version & 0xff) << 24) | (flags & 0xffffff));
      body(w);
    });
  }

  done(): Uint8Array<ArrayBuffer> {
    const out = new Uint8Array(this.#length);
    let at = 0;
    for (const part of this.#parts) {
      out.set(part, at);
      at += part.length;
    }
    return out;
  }
}

const IDENTITY = [0x10000, 0, 0, 0, 0x10000, 0, 0, 0, 0x40000000];

/** In the movie's timescale. */
function movieTime(ticks: number, timescale: number): number {
  return Math.round((ticks * MOVIE_TIMESCALE) / timescale);
}

function sum(values: { duration: number }[]): number {
  return values.reduce((total, value) => total + value.duration, 0);
}

/** `stts`: runs of equal durations. */
function stts(w: Writer, samples: { duration: number }[]) {
  const runs: [number, number][] = [];
  for (const { duration } of samples) {
    const last = runs.at(-1);
    if (last && last[1] === duration) {
      last[0] += 1;
    } else {
      runs.push([1, duration]);
    }
  }
  w.full('stts', 0, 0, (w) => {
    w.u32(runs.length);
    for (const [count, duration] of runs) {
      w.u32(count);
      w.u32(duration);
    }
  });
}

/** `stsc`, `stsz` and `stco`/`co64` of a track's chunks. */
function layout(w: Writer, samples: { size: number }[], chunks: { samples: number; offset: number }[], wide: boolean) {
  const runs: [number, number][] = [];
  chunks.forEach((chunk, index) => {
    if (runs.at(-1)?.[1] !== chunk.samples) {
      runs.push([index + 1, chunk.samples]);
    }
  });
  w.full('stsc', 0, 0, (w) => {
    w.u32(runs.length);
    for (const [first, count] of runs) {
      w.u32(first);
      w.u32(count);
      w.u32(1);
    }
  });
  w.full('stsz', 0, 0, (w) => {
    w.u32(0);
    w.u32(samples.length);
    for (const { size } of samples) {
      w.u32(size);
    }
  });
  w.full(wide ? 'co64' : 'stco', 0, 0, (w) => {
    w.u32(chunks.length);
    for (const { offset } of chunks) {
      if (wide) {
        w.u64(offset);
      } else {
        w.u32(offset);
      }
    }
  });
}

function tkhd(
  w: Writer,
  id: number,
  duration: number,
  audio: boolean,
  width: number,
  height: number,
  matrix: readonly number[] = IDENTITY,
) {
  w.full('tkhd', 0, 3, (w) => {
    w.zeros(8);
    w.u32(id);
    w.zeros(4);
    w.u32(duration);
    w.zeros(8);
    w.u16(0);
    w.u16(audio ? 1 : 0);
    w.u16(audio ? 0x0100 : 0);
    w.zeros(2);
    for (const value of matrix) {
      w.i32(value);
    }
    w.u32(Math.round(width * 0x10000));
    w.u32(Math.round(height * 0x10000));
  });
}

/** `ctts` (version 1, signed): runs of equal composition offsets, when any sample has one. */
function ctts(w: Writer, samples: VideoSample[]) {
  if (samples.every((sample) => (sample.offset ?? 0) === 0)) {
    return;
  }
  const runs: [number, number][] = [];
  for (const sample of samples) {
    const offset = sample.offset ?? 0;
    const last = runs.at(-1);
    if (last && last[1] === offset) {
      last[0] += 1;
    } else {
      runs.push([1, offset]);
    }
  }
  w.full('ctts', 1, 0, (w) => {
    w.u32(runs.length);
    for (const [count, offset] of runs) {
      w.u32(count);
      w.i32(offset);
    }
  });
}

function mdhd(w: Writer, timescale: number, duration: number) {
  w.full('mdhd', 0, 0, (w) => {
    w.zeros(8);
    w.u32(timescale);
    w.u32(duration);
    // `und`, packed as three 5-bit letters.
    w.u16(0x55c4);
    w.u16(0);
  });
}

function hdlr(w: Writer, handler: string, name: string) {
  w.full('hdlr', 0, 0, (w) => {
    w.zeros(4);
    w.text(handler);
    w.zeros(12);
    w.text(name);
    w.u8(0);
  });
}

function dinf(w: Writer) {
  w.box('dinf', (w) =>
    w.full('dref', 0, 0, (w) => {
      w.u32(1);
      w.full('url ', 0, 1, () => {});
    }),
  );
}

/** A descriptor (ISO/IEC 14496-1): its tag and its size in four 7-bit bytes. */
function descriptor(w: Writer, tag: number, body: (w: Writer) => void) {
  const inner = new Writer();
  body(inner);
  const size = inner.length;
  w.u8(tag);
  w.bytes([0x80 | ((size >>> 21) & 0x7f), 0x80 | ((size >>> 14) & 0x7f), 0x80 | ((size >>> 7) & 0x7f), size & 0x7f]);
  w.bytes(inner.done());
}

function videoEntry(w: Writer, video: VideoTrackOut) {
  if (video.entry) {
    w.bytes(video.entry);
    return;
  }
  w.box('avc1', (w) => {
    w.zeros(6);
    w.u16(1);
    w.zeros(16);
    w.u16(video.width);
    w.u16(video.height);
    w.u32(0x00480000);
    w.u32(0x00480000);
    w.zeros(4);
    w.u16(1);
    w.zeros(32);
    w.u16(0x18);
    w.u16(0xffff);
    w.box('avcC', (w) => w.bytes(video.avcC));
    w.box('pasp', (w) => {
      w.u32(1);
      w.u32(1);
    });
  });
}

function audioEntry(w: Writer, audio: AudioTrackOut) {
  w.box('mp4a', (w) => {
    w.zeros(6);
    w.u16(1);
    w.zeros(8);
    w.u16(audio.channels);
    w.u16(16);
    w.zeros(4);
    w.u32(Math.min(audio.sampleRate, 0xffff) * 0x10000);
    w.full('esds', 0, 0, (w) =>
      descriptor(w, 3, (w) => {
        w.u16(2);
        w.u8(0);
        descriptor(w, 4, (w) => {
          // MPEG-4 audio, an audio stream.
          w.u8(0x40);
          w.u8(0x15);
          w.bytes([0, 0, 0]);
          w.u32(audio.bitrate);
          w.u32(audio.bitrate);
          descriptor(w, 5, (w) => w.bytes(audio.specific));
        });
        descriptor(w, 6, (w) => w.u8(2));
      }),
    );
  });
}

/** A track's length in the movie's timescale: its edit list's, or its samples'. */
function presented(track: { edits: { duration: number }[]; samples: { duration: number }[]; timescale: number }) {
  return track.edits.length > 0
    ? track.edits.reduce((total, edit) => total + edit.duration, 0)
    : movieTime(sum(track.samples), track.timescale);
}

/** An edit list box, unless there are no edits. */
function edts(w: Writer, edits: { duration: number; mediaTime: number }[]) {
  if (edits.length === 0) {
    return;
  }
  w.box('edts', (w) =>
    w.full('elst', 0, 0, (w) => {
      w.u32(edits.length);
      for (const edit of edits) {
        w.u32(edit.duration);
        w.i32(edit.mediaTime);
        w.u32(0x00010000);
      }
    }),
  );
}

function moov(
  video: VideoTrackOut,
  audio: AudioTrackOut | null,
  videoChunks: { samples: number; offset: number }[],
  audioChunks: { samples: number; offset: number }[],
  wide: boolean,
): Uint8Array<ArrayBuffer> {
  const videoDuration = sum(video.samples);
  const videoMovie = presented(video);
  const audioMovie = audio ? presented(audio) : 0;
  const w = new Writer();
  w.box('moov', (w) => {
    w.full('mvhd', 0, 0, (w) => {
      w.zeros(8);
      w.u32(MOVIE_TIMESCALE);
      w.u32(Math.max(videoMovie, audioMovie, video.movieDuration ?? 0));
      w.u32(0x00010000);
      w.u16(0x0100);
      w.zeros(10);
      for (const value of IDENTITY) {
        w.i32(value);
      }
      w.zeros(24);
      w.u32(audio ? 3 : 2);
    });
    w.box('trak', (w) => {
      tkhd(w, 1, videoMovie, false, ...(video.presented ?? [video.width, video.height]), video.matrix);
      edts(w, video.edits);
      w.box('mdia', (w) => {
        mdhd(w, video.timescale, videoDuration);
        hdlr(w, 'vide', 'VideoHandler');
        w.box('minf', (w) => {
          w.full('vmhd', 0, 1, (w) => w.zeros(8));
          dinf(w);
          w.box('stbl', (w) => {
            w.full('stsd', 0, 0, (w) => {
              w.u32(1);
              videoEntry(w, video);
            });
            stts(w, video.samples);
            ctts(w, video.samples);
            if (video.samples.some((sample) => !sample.sync)) {
              const sync = video.samples.flatMap((sample, index) => (sample.sync ? [index + 1] : []));
              w.full('stss', 0, 0, (w) => {
                w.u32(sync.length);
                for (const index of sync) {
                  w.u32(index);
                }
              });
            }
            layout(w, video.samples, videoChunks, wide);
          });
        });
      });
    });
    if (audio) {
      w.box('trak', (w) => {
        tkhd(w, 2, audioMovie, true, 0, 0);
        edts(w, audio.edits);
        w.box('mdia', (w) => {
          mdhd(w, audio.timescale, sum(audio.samples));
          hdlr(w, 'soun', 'SoundHandler');
          w.box('minf', (w) => {
            w.full('smhd', 0, 0, (w) => w.zeros(4));
            dinf(w);
            w.box('stbl', (w) => {
              w.full('stsd', 0, 0, (w) => {
                w.u32(1);
                audioEntry(w, audio);
              });
              stts(w, audio.samples);
              layout(w, audio.samples, audioChunks, wide);
            });
          });
        });
      });
    }
  });
  return w.done();
}

function ftyp(): Uint8Array<ArrayBuffer> {
  const w = new Writer();
  w.box('ftyp', (w) => {
    w.text('isom');
    w.u32(0x200);
    w.text('isomiso2avc1mp41');
  });
  return w.done();
}

/** The MP4 of `video` and `audio`, their samples laid out as `chunks` say, in that order. */
export function writeMp4(video: VideoTrackOut, audio: AudioTrackOut | null, chunks: Chunk[]): Blob {
  const head = ftyp();
  const media = chunks.reduce((total, chunk) => total + chunk.bytes.size, 0);
  // A 64-bit `mdat` header and chunk offsets once the file passes 4 GiB.
  const wide = media + 64 * 1024 * 1024 > 0xffffffff;
  const mdatHeader = wide ? 16 : 8;
  const place = (moovSize: number) => {
    const videoChunks: { samples: number; offset: number }[] = [];
    const audioChunks: { samples: number; offset: number }[] = [];
    let offset = head.length + moovSize + mdatHeader;
    for (const chunk of chunks) {
      (chunk.track === 'video' ? videoChunks : audioChunks).push({ samples: chunk.samples, offset });
      offset += chunk.bytes.size;
    }
    return { videoChunks, audioChunks };
  };
  // The offsets depend on the box's size, which does not depend on the offsets' values.
  const first = place(0);
  const size = moov(video, audio, first.videoChunks, first.audioChunks, wide).length;
  const placed = place(size);
  const movie = moov(video, audio, placed.videoChunks, placed.audioChunks, wide);
  const header = new Writer();
  if (wide) {
    header.u32(1);
    header.text('mdat');
    header.u64(media + 16);
  } else {
    header.u32(media + 8);
    header.text('mdat');
  }
  return new Blob([head, movie, header.done(), ...chunks.map((chunk) => chunk.bytes)], { type: 'video/mp4' });
}
