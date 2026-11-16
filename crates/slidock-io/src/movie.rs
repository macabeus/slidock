//! A movie's tracks, read from its ISO base media boxes (ISO/IEC 14496-12; QuickTime's movie atoms are the same
//! boxes) by range: the top-level box headers, then the whole `moov`, wherever it sits. Each track reports what a
//! re-encode needs to know of it: its handler (`vide`, `soun`, …), `tkhd` matrix and size, `mdhd` timescale and
//! length, edit list (`elst`), sample descriptions (`stsd`: the format, a picture's coded size, depth, decoder
//! configuration, `colr` and `pasp`; a sound's channels, rate and, in `esds`, the MPEG-4 object type), and its
//! samples' count and bytes (`stsz`). Box sizes are big-endian; a `moov` larger than [`MAX_MOOV_BYTES`] is not read.

use crate::range::{RangeError, RangeReader};

/// A movie's metadata is kilobytes to a few MiB.
pub const MAX_MOOV_BYTES: u64 = 32 * 1024 * 1024;
const MAX_TOP_LEVEL_BOXES: usize = 4096;
const MAX_DEPTH: usize = 16;

/// A box: its type and its body's range in the bytes read.
#[derive(Clone, Copy, Debug)]
struct Atom {
    kind: [u8; 4],
    start: usize,
    end: usize,
}

fn be16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        bytes.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn be64(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(
        bytes.get(at..at.checked_add(8)?)?.try_into().ok()?,
    ))
}

fn fourcc(bytes: &[u8], at: usize) -> Option<[u8; 4]> {
    bytes.get(at..at.checked_add(4)?)?.try_into().ok()
}

/// The boxes directly inside `[start, end)` of `bytes`; a box that runs past `end` ends the list.
fn atoms(bytes: &[u8], start: usize, end: usize) -> Vec<Atom> {
    let mut found = Vec::new();
    let mut at = start;
    let end = end.min(bytes.len());
    while at.saturating_add(8) <= end {
        let (Some(size), Some(kind)) = (be32(bytes, at), fourcc(bytes, at + 4)) else {
            break;
        };
        let (size, header) = match size {
            1 => match be64(bytes, at + 8).and_then(|large| usize::try_from(large).ok()) {
                Some(large) => (large, 16),
                None => break,
            },
            0 => (end - at, 8),
            size => (size as usize, 8),
        };
        let Some(box_end) = at
            .checked_add(size)
            .filter(|&box_end| size >= header && box_end <= end)
        else {
            break;
        };
        found.push(Atom {
            kind,
            start: at + header,
            end: box_end,
        });
        at = box_end;
    }
    found
}

fn child(bytes: &[u8], within: Atom, kind: &[u8; 4]) -> Option<Atom> {
    atoms(bytes, within.start, within.end)
        .into_iter()
        .find(|atom| &atom.kind == kind)
}

/// The box at the end of `path` below `within`, each step its first child of that type.
fn descend(bytes: &[u8], within: Atom, path: &[&[u8; 4]]) -> Option<Atom> {
    path.iter()
        .take(MAX_DEPTH)
        .try_fold(within, |at, kind| child(bytes, at, kind))
}

/// One entry of an edit list (`elst`): `duration` in the movie's timescale, `media_time` in the track's (−1: an empty
/// edit, which shows nothing for its duration), `rate` in 16.16 fixed point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edit {
    pub duration: u64,
    pub media_time: i64,
    pub rate: i32,
}

/// The colour a sample description states (`colr`, `nclx` or QuickTime's `nclc`), as ISO/IEC 23091-2 code points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MovieColour {
    pub primaries: u16,
    pub transfer: u16,
    pub matrix: u16,
    /// `nclx` only.
    pub full_range: Option<bool>,
}

/// A picture's sample description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisualEntry {
    /// The coded size.
    pub width: u16,
    pub height: u16,
    /// 24 for colour, 32 for colour with alpha (QuickTime).
    pub depth: u16,
    /// The decoder configuration's body (`avcC`, `hvcC`, `vpcC`, `av1C`), by its type.
    pub config: Option<([u8; 4], Vec<u8>)>,
    pub colour: Option<MovieColour>,
    /// `pasp`: horizontal and vertical spacing.
    pub pixel_aspect: Option<(u32, u32)>,
}

/// A sound's sample description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioEntry {
    pub channels: u16,
    /// Bits per sample.
    pub sample_size: u16,
    /// In hertz (the integer part of the 16.16 rate; QuickTime's version 2 states it as a double).
    pub sample_rate: u32,
    /// `esds`' object type indication (ISO/IEC 14496-1; 0x40 is MPEG-4 audio), in the entry or QuickTime's `wave`.
    pub object_type: Option<u8>,
    /// `esds`' decoder specific information (an AAC AudioSpecificConfig).
    pub specific: Option<Vec<u8>>,
    /// The bytes of one frame (a sample of every channel) of uncompressed sound, whose `stsz` QuickTime writes as one
    /// sample of a size of 1 per frame; `None` for compressed sound.
    pub frame_bytes: Option<u32>,
}

/// The formats of uncompressed sound.
const PCM_FORMATS: [&[u8; 4]; 9] = [
    b"sowt", b"twos", b"lpcm", b"in24", b"in32", b"fl32", b"fl64", b"raw ", b"NONE",
];

/// A sample description: its format and what is read of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampleEntry {
    pub format: [u8; 4],
    pub visual: Option<VisualEntry>,
    pub audio: Option<AudioEntry>,
}

impl SampleEntry {
    /// The format as text (`avc1`, `mp4a`, …).
    pub fn format_name(&self) -> String {
        self.format.iter().map(|&byte| char::from(byte)).collect()
    }
}

/// One track of a movie.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovieTrack {
    pub id: u32,
    /// `hdlr`'s handler type: `vide`, `soun`, `tmcd`, `text`, …
    pub handler: [u8; 4],
    /// `tkhd`'s enabled flag.
    pub enabled: bool,
    /// `tkhd`'s transformation matrix: a, b, u, c, d, v, x, y, w (a–d and x, y in 16.16, u, v, w in 2.30).
    pub matrix: [i32; 9],
    /// `tkhd`'s presentation size in 16.16.
    pub width: u32,
    pub height: u32,
    /// `mdhd`.
    pub timescale: u32,
    pub duration: u64,
    pub edits: Vec<Edit>,
    pub entries: Vec<SampleEntry>,
    /// `stsz`: how many samples and their bytes.
    pub samples: u32,
    pub sample_bytes: u64,
}

const IDENTITY: [i32; 9] = [0x1_0000, 0, 0, 0, 0x1_0000, 0, 0, 0, 0x4000_0000];

impl MovieTrack {
    pub fn is_video(&self) -> bool {
        &self.handler == b"vide"
    }

    pub fn is_audio(&self) -> bool {
        &self.handler == b"soun"
    }

    /// Whether `tkhd`'s matrix is the identity.
    pub fn unturned(&self) -> bool {
        self.matrix == IDENTITY
    }

    /// The clockwise quarter turns the matrix applies (0, 90, 180 or 270 degrees), its translation aside; `None` for a
    /// matrix that scales, skews or mirrors.
    pub fn rotation(&self) -> Option<u16> {
        const ONE: i32 = 0x1_0000;
        let [a, b, u, c, d, v, _, _, w] = self.matrix;
        if (u, v, w) != (0, 0, 0x4000_0000) {
            return None;
        }
        match (a, b, c, d) {
            (ONE, 0, 0, ONE) => Some(0),
            (0, ONE, n, 0) if n == -ONE => Some(90),
            (n, 0, 0, m) if n == -ONE && m == -ONE => Some(180),
            (0, n, ONE, 0) if n == -ONE => Some(270),
            _ => None,
        }
    }

    /// The track's length in seconds, by its media timescale.
    pub fn seconds(&self) -> Option<f64> {
        (self.timescale > 0).then(|| self.duration as f64 / f64::from(self.timescale))
    }
}

/// A movie's metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Movie {
    /// `ftyp`'s major brand, when the file starts with one.
    pub brand: Option<[u8; 4]>,
    /// `moov` comes before the media data (`mdat`): a player starts before reading the whole file.
    pub moov_first: bool,
    /// The movie is fragmented (`mvex` in `moov`, or a `moof`): its samples are not all listed in `moov`.
    pub fragmented: bool,
    /// `mvhd`.
    pub timescale: u32,
    pub duration: u64,
    pub tracks: Vec<MovieTrack>,
}

impl Movie {
    /// The movie's length in milliseconds (`mvhd`).
    pub fn duration_ms(&self) -> Option<f64> {
        (self.timescale > 0 && self.duration != u64::from(u32::MAX))
            .then(|| self.duration as f64 * 1000.0 / f64::from(self.timescale))
    }

    pub fn video_tracks(&self) -> impl Iterator<Item = &MovieTrack> {
        self.tracks.iter().filter(|track| track.is_video())
    }

    pub fn audio_tracks(&self) -> impl Iterator<Item = &MovieTrack> {
        self.tracks.iter().filter(|track| track.is_audio())
    }
}

/// Reads `reader`'s movie: `None` when it has no readable `moov` (or one over [`MAX_MOOV_BYTES`]).
pub fn read_movie<R: RangeReader + ?Sized>(reader: &R) -> Result<Option<Movie>, RangeError> {
    let total = reader.len();
    let mut offset: u64 = 0;
    let mut brand = None;
    let mut moov: Option<(u64, u64)> = None;
    let mut mdat_before_moov = false;
    let mut fragmented = false;
    for index in 0..MAX_TOP_LEVEL_BOXES {
        if offset.saturating_add(8) > total {
            break;
        }
        let header = reader.read_at(offset, (total - offset).min(16) as usize)?;
        let (Some(size), Some(kind)) = (be32(&header, 0), fourcc(&header, 4)) else {
            break;
        };
        let (size, header_size) = match size {
            1 => match be64(&header, 8) {
                Some(large) => (large, 16),
                None => break,
            },
            0 => (total - offset, 8),
            size => (u64::from(size), 8),
        };
        let Some(end) = offset
            .checked_add(size)
            .filter(|&end| size >= header_size && end <= total)
        else {
            break;
        };
        match &kind {
            b"ftyp" if index == 0 && header_size == 8 && size >= 12 => {
                brand = fourcc(&header, 8);
            }
            b"moov" if moov.is_none() => moov = Some((offset + header_size, size - header_size)),
            b"mdat" if moov.is_none() => mdat_before_moov = true,
            b"moof" => fragmented = true,
            _ => {}
        }
        offset = end;
    }
    let Some((at, len)) = moov else {
        return Ok(None);
    };
    if len > MAX_MOOV_BYTES {
        return Ok(None);
    }
    // At most MAX_MOOV_BYTES, so it fits in usize.
    let bytes = reader.read_at(at, len as usize)?;
    Ok(parse_moov(&bytes).map(|parsed| Movie {
        brand,
        moov_first: !mdat_before_moov,
        fragmented: fragmented || parsed.fragmented,
        ..parsed
    }))
}

fn parse_moov(bytes: &[u8]) -> Option<Movie> {
    let top = atoms(bytes, 0, bytes.len());
    let mvhd = top.iter().find(|atom| &atom.kind == b"mvhd")?;
    let (timescale, duration) = header_times(bytes, *mvhd)?;
    let tracks = top
        .iter()
        .filter(|atom| &atom.kind == b"trak")
        .filter_map(|trak| parse_trak(bytes, *trak))
        .collect();
    Some(Movie {
        brand: None,
        moov_first: true,
        fragmented: top.iter().any(|atom| &atom.kind == b"mvex"),
        timescale,
        duration,
        tracks,
    })
}

/// `mvhd`'s and `mdhd`'s timescale and duration: after the version and flags, creation and modification times (4
/// bytes each, 8 in version 1), then the timescale (4) and the duration (4, 8 in version 1).
fn header_times(bytes: &[u8], atom: Atom) -> Option<(u32, u64)> {
    let wide = bytes.get(atom.start)? == &1;
    let at = atom.start + if wide { 20 } else { 12 };
    let timescale = be32(bytes, at)?;
    let duration = if wide {
        be64(bytes, at + 4)?
    } else {
        u64::from(be32(bytes, at + 4)?)
    };
    (at + if wide { 12 } else { 8 } <= atom.end).then_some((timescale, duration))
}

fn parse_trak(bytes: &[u8], trak: Atom) -> Option<MovieTrack> {
    let tkhd = child(bytes, trak, b"tkhd")?;
    let wide = bytes.get(tkhd.start)? == &1;
    let flags = be32(bytes, tkhd.start)? & 0x00ff_ffff;
    // Version, flags (4); creation, modification (4 or 8 each); id (4); reserved (4); duration (4 or 8).
    let id = be32(bytes, tkhd.start + if wide { 20 } else { 12 })?;
    // Then reserved (8), layer (2), alternate group (2), volume (2), reserved (2), the matrix (36), the size (8).
    let matrix_at = tkhd.start + if wide { 36 } else { 24 } + 8 + 8;
    let mut matrix = [0i32; 9];
    for (index, value) in matrix.iter_mut().enumerate() {
        *value = be32(bytes, matrix_at + 4 * index)? as i32;
    }
    let width = be32(bytes, matrix_at + 36)?;
    let height = be32(bytes, matrix_at + 40)?;
    let edits = descend(bytes, trak, &[b"edts", b"elst"])
        .map(|elst| parse_elst(bytes, elst))
        .unwrap_or_default();
    let mdia = child(bytes, trak, b"mdia")?;
    let (timescale, duration) = header_times(bytes, child(bytes, mdia, b"mdhd")?)?;
    let hdlr = child(bytes, mdia, b"hdlr")?;
    // Version and flags (4), pre_defined (4), then the handler type.
    let handler = fourcc(bytes, hdlr.start + 8)?;
    let stbl = descend(bytes, mdia, &[b"minf", b"stbl"]);
    let entries = stbl
        .and_then(|stbl| child(bytes, stbl, b"stsd"))
        .map(|stsd| parse_stsd(bytes, stsd))
        .unwrap_or_default();
    let (samples, sample_bytes) = stbl
        .and_then(|stbl| child(bytes, stbl, b"stsz"))
        .and_then(|stsz| parse_stsz(bytes, stsz))
        .unwrap_or((0, 0));
    Some(MovieTrack {
        id,
        handler,
        enabled: flags & 1 == 1,
        matrix,
        width,
        height,
        timescale,
        duration,
        edits,
        entries,
        samples,
        sample_bytes,
    })
}

fn parse_elst(bytes: &[u8], elst: Atom) -> Vec<Edit> {
    let wide = bytes.get(elst.start) == Some(&1);
    let count = be32(bytes, elst.start + 4).unwrap_or(0) as usize;
    let size = if wide { 20 } else { 12 };
    (0..count)
        .map_while(|index| {
            let at = elst.start + 8 + index.checked_mul(size)?;
            if at + size > elst.end {
                return None;
            }
            let (duration, media_time, rate_at) = if wide {
                (be64(bytes, at)?, be64(bytes, at + 8)? as i64, at + 16)
            } else {
                (
                    u64::from(be32(bytes, at)?),
                    i64::from(be32(bytes, at + 4)? as i32),
                    at + 8,
                )
            };
            Some(Edit {
                duration,
                media_time,
                rate: be32(bytes, rate_at)? as i32,
            })
        })
        .collect()
}

/// `stsz`: a sample size for every sample (or one for all), then the count; the sum of the sizes.
fn parse_stsz(bytes: &[u8], stsz: Atom) -> Option<(u32, u64)> {
    let fixed = be32(bytes, stsz.start + 4)?;
    let count = be32(bytes, stsz.start + 8)?;
    if fixed != 0 {
        return Some((count, u64::from(fixed) * u64::from(count)));
    }
    let mut total = 0u64;
    for index in 0..count as usize {
        total += u64::from(be32(bytes, stsz.start + 12 + 4 * index)?);
    }
    Some((count, total))
}

/// The formats a visual sample description has: video codecs and QuickTime's picture formats.
fn is_visual(format: &[u8; 4]) -> bool {
    matches!(
        format,
        b"avc1"
            | b"avc3"
            | b"hvc1"
            | b"hev1"
            | b"vp08"
            | b"vp09"
            | b"av01"
            | b"mp4v"
            | b"jpeg"
            | b"mjpa"
            | b"mjpb"
            | b"apcn"
            | b"apch"
            | b"apcs"
            | b"apco"
            | b"ap4h"
            | b"ap4x"
            | b"dvh1"
            | b"dvhe"
            | b"s263"
            | b"encv"
    )
}

fn parse_stsd(bytes: &[u8], stsd: Atom) -> Vec<SampleEntry> {
    // Version and flags (4), the entry count (4), then the entries as boxes.
    atoms(bytes, stsd.start + 8, stsd.end)
        .into_iter()
        .map(|entry| {
            let visual = if is_visual(&entry.kind) {
                parse_visual(bytes, entry)
            } else {
                None
            };
            let audio = if visual.is_none() {
                parse_audio(bytes, entry)
            } else {
                None
            };
            SampleEntry {
                format: entry.kind,
                visual,
                audio,
            }
        })
        .collect()
}

/// A visual sample entry: reserved (6), data reference (2), pre-defined and reserved (16), width and height (2 each),
/// resolutions (8), reserved (4), frame count (2), compressor name (32), depth (2), pre-defined (2), then boxes.
fn parse_visual(bytes: &[u8], entry: Atom) -> Option<VisualEntry> {
    let width = be16(bytes, entry.start + 24)?;
    let height = be16(bytes, entry.start + 26)?;
    let depth = be16(bytes, entry.start + 74)?;
    let mut visual = VisualEntry {
        width,
        height,
        depth,
        config: None,
        colour: None,
        pixel_aspect: None,
    };
    for atom in atoms(bytes, entry.start + 78, entry.end) {
        match &atom.kind {
            b"avcC" | b"hvcC" | b"vpcC" | b"av1C" if visual.config.is_none() => {
                visual.config = Some((atom.kind, bytes.get(atom.start..atom.end)?.to_vec()));
            }
            b"colr" if visual.colour.is_none() => {
                let kind = fourcc(bytes, atom.start)?;
                let code = |at: usize| be16(bytes, atom.start + at);
                visual.colour = match &kind {
                    b"nclx" | b"nclc" => Some(MovieColour {
                        primaries: code(4)?,
                        transfer: code(6)?,
                        matrix: code(8)?,
                        full_range: (&kind == b"nclx")
                            .then(|| bytes.get(atom.start + 10).map(|byte| byte & 0x80 != 0))
                            .flatten(),
                    }),
                    _ => None,
                };
            }
            b"pasp" => {
                visual.pixel_aspect =
                    Some((be32(bytes, atom.start)?, be32(bytes, atom.start + 4)?));
            }
            _ => {}
        }
    }
    Some(visual)
}

/// A sound sample entry: reserved (6), data reference (2), version (2), revision and vendor (6), channels (2), sample
/// size (2), compression and packet size (4), the rate in 16.16 (4); QuickTime's version 1 adds 16 bytes, version 2
/// 36 (its rate a double and its channel count after the first 16 bytes). Then boxes: `esds`, or `wave` holding it.
fn parse_audio(bytes: &[u8], entry: Atom) -> Option<AudioEntry> {
    let version = be16(bytes, entry.start + 8)?;
    let pcm = PCM_FORMATS.contains(&&entry.kind);
    let mut channels = be16(bytes, entry.start + 16)?;
    let sample_size = be16(bytes, entry.start + 18)?;
    let mut sample_rate = be32(bytes, entry.start + 24)? >> 16;
    let boxes_at = entry.start
        + 28
        + match version {
            1 => 16,
            2 => 36,
            _ => 0,
        };
    if version == 2 {
        let rate = f64::from_bits(be64(bytes, entry.start + 32)?);
        if rate.is_finite() && rate > 0.0 && rate < f64::from(u32::MAX) {
            sample_rate = rate as u32;
        }
        channels = u16::try_from(be32(bytes, entry.start + 40)?).unwrap_or(u16::MAX);
    }
    let inner = atoms(bytes, boxes_at, entry.end);
    let esds = inner
        .iter()
        .find(|atom| &atom.kind == b"esds")
        .copied()
        .or_else(|| {
            let wave = inner.iter().find(|atom| &atom.kind == b"wave")?;
            child(bytes, *wave, b"esds")
        });
    let (object_type, specific) = esds
        .map(|esds| parse_esds(bytes, esds))
        .unwrap_or((None, None));
    // QuickTime's version 1 states the bytes per frame after the samples and bytes per packet; version 2 the bytes of
    // a packet of one frame; version 0 only the bits per sample.
    let frame_bytes = match version {
        _ if !pcm => None,
        1 => be32(bytes, entry.start + 36),
        2 => be32(bytes, entry.start + 56),
        _ => Some(u32::from(sample_size).div_ceil(8) * u32::from(channels)),
    }
    .filter(|&bytes| bytes > 0);
    Some(AudioEntry {
        channels,
        sample_size,
        sample_rate,
        object_type,
        specific,
        frame_bytes,
    })
}

/// A descriptor's tag and its body's range: the tag (1), then the size in up to 4 bytes of 7 bits each.
fn descriptor(bytes: &[u8], at: usize, end: usize) -> Option<(u8, usize, usize)> {
    let tag = *bytes.get(at)?;
    let mut size = 0usize;
    let mut cursor = at + 1;
    for _ in 0..4 {
        let byte = *bytes.get(cursor)?;
        cursor += 1;
        size = (size << 7) | usize::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            break;
        }
    }
    let body_end = cursor.checked_add(size)?;
    (body_end <= end).then_some((tag, cursor, body_end))
}

/// `esds` (ISO/IEC 14496-1 §7.2.6): after the version and flags, an ES descriptor (tag 3) holding a decoder config
/// descriptor (tag 4: the object type, then 12 bytes, then a decoder specific info, tag 5).
fn parse_esds(bytes: &[u8], esds: Atom) -> (Option<u8>, Option<Vec<u8>>) {
    let Some((3, start, end)) = descriptor(bytes, esds.start + 4, esds.end) else {
        return (None, None);
    };
    // ES_ID (2), then flags naming what follows: a stream it depends on (2), a URL (its length, then it), an OCR
    // stream (2).
    let Some(&flags) = bytes.get(start + 2) else {
        return (None, None);
    };
    let mut at = start + 3;
    if flags & 0x80 != 0 {
        at += 2;
    }
    if flags & 0x40 != 0 {
        at += 1 + bytes.get(at).map_or(0, |&len| usize::from(len));
    }
    if flags & 0x20 != 0 {
        at += 2;
    }
    let Some((4, config, config_end)) = descriptor(bytes, at, end) else {
        return (None, None);
    };
    let object_type = bytes.get(config).copied();
    let specific = descriptor(bytes, config + 13, config_end)
        .filter(|(tag, _, _)| *tag == 5)
        .and_then(|(_, start, end)| bytes.get(start..end))
        .map(<[u8]>::to_vec);
    (object_type, specific)
}
