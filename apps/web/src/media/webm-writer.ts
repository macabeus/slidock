// Writes a WebM (Matroska's WebM subset, https://www.matroska.org/technical/elements.html) of one VP8 or VP9 video
// track, its alpha carried as WebM's alpha (`AlphaMode` 1: each frame's alpha plane coded as a second stream of the
// same codec, its luma the alpha, in the frame's `BlockAdditional` of `BlockAddID` 1, as libvpx writes it), and at
// most one Opus sound track. The index comes first: the segment opens with a seek head, then the info, the tracks and
// the cues (one per cluster), so a player seeks before it has read the media. A cluster starts at each video key
// frame; sound blocks go into the cluster of their time. Times are in milliseconds (`TimestampScale` 1 000 000 ns).

/** An encoded video frame: its time from the start in milliseconds, whether it is a key frame, its alpha stream's. */
export interface WebmFrame {
  timestamp: number;
  key: boolean;
  data: Uint8Array;
  alpha: Uint8Array | null;
}

/** An encoded sound frame. */
export interface WebmSound {
  timestamp: number;
  data: Uint8Array;
}

/** ISO/IEC 23091-2 code points, written as the track's `Colour`. */
export interface WebmColour {
  matrix: number;
  transfer: number;
  primaries: number;
  fullRange: boolean;
}

export interface WebmVideoTrack {
  codec: 'V_VP8' | 'V_VP9';
  width: number;
  height: number;
  alpha: boolean;
  colour: WebmColour | null;
  /** In time order; the first a key frame. */
  frames: WebmFrame[];
}

export interface WebmSoundTrack {
  codec: 'A_OPUS';
  sampleRate: number;
  channels: number;
  /** The codec's private data (Opus: the `OpusHead`). */
  codecPrivate: Uint8Array;
  /** The samples the decoder drops at the start, in nanoseconds (Opus: its pre-skip). */
  codecDelay: number;
  /** How much to decode before a seek target, in nanoseconds (Opus: 80 ms). */
  seekPreRoll: number;
  frames: WebmSound[];
}

export interface WebmMovie {
  video: WebmVideoTrack;
  sound: WebmSoundTrack | null;
  /** The length, in milliseconds: the last video frame lasts until it. */
  duration: number;
}

const ID = {
  EBML: 0x1a45dfa3,
  EBMLVersion: 0x4286,
  EBMLReadVersion: 0x42f7,
  EBMLMaxIDLength: 0x42f2,
  EBMLMaxSizeLength: 0x42f3,
  DocType: 0x4282,
  DocTypeVersion: 0x4287,
  DocTypeReadVersion: 0x4285,
  Segment: 0x18538067,
  SeekHead: 0x114d9b74,
  Seek: 0x4dbb,
  SeekID: 0x53ab,
  SeekPosition: 0x53ac,
  Info: 0x1549a966,
  TimestampScale: 0x2ad7b1,
  Duration: 0x4489,
  MuxingApp: 0x4d80,
  WritingApp: 0x5741,
  Tracks: 0x1654ae6b,
  TrackEntry: 0xae,
  TrackNumber: 0xd7,
  TrackUID: 0x73c5,
  TrackType: 0x83,
  FlagLacing: 0x9c,
  CodecID: 0x86,
  CodecPrivate: 0x63a2,
  CodecDelay: 0x56aa,
  SeekPreRoll: 0x56bb,
  MaxBlockAdditionID: 0x55ee,
  Video: 0xe0,
  PixelWidth: 0xb0,
  PixelHeight: 0xba,
  AlphaMode: 0x53c0,
  Colour: 0x55b0,
  MatrixCoefficients: 0x55b1,
  Range: 0x55b9,
  TransferCharacteristics: 0x55ba,
  Primaries: 0x55bb,
  Audio: 0xe1,
  SamplingFrequency: 0xb5,
  Channels: 0x9f,
  Cues: 0x1c53bb6b,
  CuePoint: 0xbb,
  CueTime: 0xb3,
  CueTrackPositions: 0xb7,
  CueTrack: 0xf7,
  CueClusterPosition: 0xf1,
  Cluster: 0x1f43b675,
  Timestamp: 0xe7,
  SimpleBlock: 0xa3,
  BlockGroup: 0xa0,
  Block: 0xa1,
  BlockDuration: 0x9b,
  ReferenceBlock: 0xfb,
  BlockAdditions: 0x75a1,
  BlockMore: 0xa6,
  BlockAddID: 0xee,
  BlockAdditional: 0xa5,
} as const;

const VIDEO_TRACK = 1;
const SOUND_TRACK = 2;

/** The bytes of an element ID: its own length marker included. */
function idBytes(id: number): number[] {
  const bytes: number[] = [];
  for (let value = id; value > 0; value = Math.floor(value / 256)) {
    bytes.unshift(value & 0xff);
  }
  return bytes;
}

/** A size as an EBML variable-length integer, in `length` bytes (the shortest that holds it when absent). */
export function vint(value: number, length?: number): number[] {
  let n = length ?? 1;
  while (length === undefined && value >= 2 ** (7 * n) - 1) {
    n += 1;
  }
  if (n > 8 || value >= 2 ** (7 * n) - 1) {
    throw new RangeError(`${value} needs more than ${n} bytes`);
  }
  const bytes = new Array<number>(n);
  let rest = value;
  for (let i = n - 1; i >= 0; i -= 1) {
    bytes[i] = rest % 256;
    rest = Math.floor(rest / 256);
  }
  bytes[0]! |= 0x80 >> (n - 1);
  return bytes;
}

/** An unsigned integer in `length` bytes (the fewest that hold it when absent). */
function uintBytes(value: number, length?: number): number[] {
  const bytes: number[] = [];
  for (let rest = value; rest > 0; rest = Math.floor(rest / 256)) {
    bytes.unshift(rest % 256);
  }
  if (bytes.length === 0) {
    bytes.push(0);
  }
  while (length !== undefined && bytes.length < length) {
    bytes.unshift(0);
  }
  return bytes;
}

type Part = Uint8Array | number[];

/** An element: its ID, its size and its body. */
function element(id: number, body: Part[]): Part[] {
  const size = body.reduce((sum, part) => sum + part.length, 0);
  return [idBytes(id), vint(size), ...body];
}

const uint = (id: number, value: number, length?: number) => element(id, [uintBytes(value, length)]);
const text = (id: number, value: string) => element(id, [Array.from(value, (char) => char.charCodeAt(0) & 0x7f)]);
const binary = (id: number, value: Uint8Array) => element(id, [value]);

function float64(id: number, value: number): Part[] {
  const bytes = new Uint8Array(8);
  new DataView(bytes.buffer).setFloat64(0, value);
  return element(id, [bytes]);
}

function length(parts: Part[]): number {
  return parts.reduce((sum, part) => sum + part.length, 0);
}

/** A block's header: its track, its time from its cluster's, its flags. */
function blockHeader(track: number, relative: number, flags: number): number[] {
  return [...vint(track), (relative >> 8) & 0xff, relative & 0xff, flags];
}

interface Cluster {
  start: number;
  parts: Part[];
}

/** The clusters: one from each video key frame, the sound blocks of its time in order among the video's. */
function clusters(movie: WebmMovie): Cluster[] {
  const { video, sound } = movie;
  const out: Cluster[] = [];
  let soundAt = 0;
  const sounds = sound?.frames ?? [];
  video.frames.forEach((frame, index) => {
    if (frame.key || out.length === 0) {
      out.push({ start: frame.timestamp, parts: [] });
    }
    const cluster = out.at(-1)!;
    const next = video.frames[index + 1];
    const end = next === undefined ? Infinity : next.timestamp;
    const add = (time: number, block: Part[]) => {
      const relative = time - cluster.start;
      if (relative < -32768 || relative > 32767) {
        throw new RangeError('a block lies more than 32 s from its cluster');
      }
      cluster.parts.push(...block);
    };
    // Sound before this frame's time goes first (the first cluster takes any that starts before the video).
    while (soundAt < sounds.length && sounds[soundAt]!.timestamp < frame.timestamp) {
      const block = sounds[soundAt]!;
      add(
        block.timestamp,
        element(ID.SimpleBlock, [blockHeader(SOUND_TRACK, block.timestamp - cluster.start, 0x80), block.data]),
      );
      soundAt += 1;
    }
    const relative = frame.timestamp - cluster.start;
    const last = next === undefined;
    if (video.alpha || last) {
      // A block group: its alpha in a block addition, a reference for a delta frame, the last frame's duration.
      const body: Part[] = [...element(ID.Block, [blockHeader(VIDEO_TRACK, relative, 0), frame.data])];
      if (frame.alpha) {
        body.push(
          ...element(ID.BlockAdditions, [
            ...element(ID.BlockMore, [...uint(ID.BlockAddID, 1), ...binary(ID.BlockAdditional, frame.alpha)]),
          ]),
        );
      }
      if (last) {
        body.push(...uint(ID.BlockDuration, Math.max(1, movie.duration - frame.timestamp)));
      }
      if (!frame.key) {
        const previous = video.frames[index - 1]!;
        const reference = previous.timestamp - frame.timestamp;
        body.push(...element(ID.ReferenceBlock, [signedBytes(reference)]));
      }
      add(frame.timestamp, element(ID.BlockGroup, body));
    } else {
      add(
        frame.timestamp,
        element(ID.SimpleBlock, [blockHeader(VIDEO_TRACK, relative, frame.key ? 0x80 : 0), frame.data]),
      );
    }
    if (last) {
      while (soundAt < sounds.length && sounds[soundAt]!.timestamp < end) {
        const block = sounds[soundAt]!;
        add(
          block.timestamp,
          element(ID.SimpleBlock, [blockHeader(SOUND_TRACK, block.timestamp - cluster.start, 0x80), block.data]),
        );
        soundAt += 1;
      }
    }
  });
  return out;
}

/** A signed integer in the fewest bytes, two's complement. */
function signedBytes(value: number): number[] {
  let n = 1;
  while (value < -(2 ** (8 * n - 1)) || value >= 2 ** (8 * n - 1)) {
    n += 1;
  }
  const bytes = new Array<number>(n);
  let rest = value < 0 ? 2 ** (8 * n) + value : value;
  for (let i = n - 1; i >= 0; i -= 1) {
    bytes[i] = rest % 256;
    rest = Math.floor(rest / 256);
  }
  return bytes;
}

function tracks(movie: WebmMovie): Part[] {
  const { video, sound } = movie;
  const colour = video.colour
    ? element(ID.Colour, [
        ...uint(ID.MatrixCoefficients, video.colour.matrix),
        ...uint(ID.Range, video.colour.fullRange ? 2 : 1),
        ...uint(ID.TransferCharacteristics, video.colour.transfer),
        ...uint(ID.Primaries, video.colour.primaries),
      ])
    : [];
  const entries: Part[] = element(ID.TrackEntry, [
    ...uint(ID.TrackNumber, VIDEO_TRACK),
    ...uint(ID.TrackUID, VIDEO_TRACK),
    ...uint(ID.TrackType, 1),
    ...uint(ID.FlagLacing, 0),
    ...text(ID.CodecID, video.codec),
    ...(video.alpha ? uint(ID.MaxBlockAdditionID, 1) : []),
    ...element(ID.Video, [
      ...uint(ID.PixelWidth, video.width),
      ...uint(ID.PixelHeight, video.height),
      ...(video.alpha ? uint(ID.AlphaMode, 1) : []),
      ...colour,
    ]),
  ]);
  if (sound) {
    entries.push(
      ...element(ID.TrackEntry, [
        ...uint(ID.TrackNumber, SOUND_TRACK),
        ...uint(ID.TrackUID, SOUND_TRACK),
        ...uint(ID.TrackType, 2),
        ...uint(ID.FlagLacing, 0),
        ...text(ID.CodecID, sound.codec),
        ...binary(ID.CodecPrivate, sound.codecPrivate),
        ...uint(ID.CodecDelay, sound.codecDelay),
        ...uint(ID.SeekPreRoll, sound.seekPreRoll),
        ...element(ID.Audio, [
          ...float64(ID.SamplingFrequency, sound.sampleRate),
          ...uint(ID.Channels, sound.channels),
        ]),
      ]),
    );
  }
  return element(ID.Tracks, entries);
}

/** Positions inside the segment are written in this many bytes, so the index's size does not depend on them. */
const POSITION_BYTES = 8;

/** The WebM of `movie`. */
export function writeWebm(movie: WebmMovie): Blob {
  if (movie.video.frames.length === 0 || !movie.video.frames[0]!.key) {
    throw new Error('the video must start with a key frame');
  }
  const header = element(ID.EBML, [
    ...uint(ID.EBMLVersion, 1),
    ...uint(ID.EBMLReadVersion, 1),
    ...uint(ID.EBMLMaxIDLength, 4),
    ...uint(ID.EBMLMaxSizeLength, 8),
    ...text(ID.DocType, 'webm'),
    ...uint(ID.DocTypeVersion, 4),
    ...uint(ID.DocTypeReadVersion, 2),
  ]);
  const info = element(ID.Info, [
    ...uint(ID.TimestampScale, 1_000_000),
    ...text(ID.MuxingApp, 'slidock'),
    ...text(ID.WritingApp, 'slidock'),
    ...float64(ID.Duration, movie.duration),
  ]);
  const trackList = tracks(movie);
  const listed = clusters(movie);
  const times = listed.map((cluster) => cluster.start);
  const built = listed.map((cluster) => element(ID.Cluster, [...uint(ID.Timestamp, cluster.start), ...cluster.parts]));
  const cues = (positions: number[]) =>
    element(
      ID.Cues,
      positions.flatMap((position, index) =>
        element(ID.CuePoint, [
          ...uint(ID.CueTime, times[index]!),
          ...element(ID.CueTrackPositions, [
            ...uint(ID.CueTrack, VIDEO_TRACK),
            ...uint(ID.CueClusterPosition, position, POSITION_BYTES),
          ]),
        ]),
      ),
    );
  const seekHead = (at: { info: number; tracks: number; cues: number }) =>
    element(ID.SeekHead, [
      ...element(ID.Seek, [
        ...binary(ID.SeekID, Uint8Array.from(idBytes(ID.Info))),
        ...uint(ID.SeekPosition, at.info, POSITION_BYTES),
      ]),
      ...element(ID.Seek, [
        ...binary(ID.SeekID, Uint8Array.from(idBytes(ID.Tracks))),
        ...uint(ID.SeekPosition, at.tracks, POSITION_BYTES),
      ]),
      ...element(ID.Seek, [
        ...binary(ID.SeekID, Uint8Array.from(idBytes(ID.Cues))),
        ...uint(ID.SeekPosition, at.cues, POSITION_BYTES),
      ]),
    ]);
  // The sizes of the seek head and the cues do not depend on the positions they hold.
  const seekLength = length(seekHead({ info: 0, tracks: 0, cues: 0 }));
  const cuesLength = length(cues(built.map(() => 0)));
  const infoAt = seekLength;
  const tracksAt = infoAt + length(info);
  const cuesAt = tracksAt + length(trackList);
  let position = cuesAt + cuesLength;
  const positions = built.map((cluster) => {
    const at = position;
    position += length(cluster);
    return at;
  });
  const body: Part[] = [
    ...seekHead({ info: infoAt, tracks: tracksAt, cues: cuesAt }),
    ...info,
    ...trackList,
    ...cues(positions),
    ...built.flat(),
  ];
  const segment = [...idBytes(ID.Segment), ...vint(length(body), 8)];
  return new Blob(
    [...header, segment, ...body].map((part) =>
      part instanceof Uint8Array ? part : Uint8Array.from(part),
    ) as BlobPart[],
    { type: 'video/webm' },
  );
}
