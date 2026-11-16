//! The codec of a movie's video track and the movie's length, read from its ISO media boxes by range (`moov` → `trak`
//! → `mdia` → `hdlr` says "vide" → `minf` → `stbl` → `stsd`, whose first sample entry is named by the codec's
//! four-character code; `moov` → `mvhd` for the timescale and duration). Box sizes are big-endian. Only top-level box
//! headers and `moov` are read, wherever `moov` sits.

use slidock_canon::js_round;

use crate::range::{RangeError, RangeReader};

const BOX_HEADER: u64 = 8;
const LARGE_SIZE: u64 = 1;
/// A movie's metadata is kilobytes to a few MiB; this probe skips anything larger.
const MAX_MOOV_BYTES: u64 = 32 * 1024 * 1024;
const MAX_TOP_LEVEL_BOXES: usize = 1024;

/// What a movie's metadata says, each when readable.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VideoFacts {
    /// The video codec: `avc1`, `hvc1`, `ap4h`, …
    pub codec: Option<String>,
    /// The movie's length in milliseconds, rounded as JavaScript's `Math.round`.
    pub duration_ms: Option<f64>,
}

pub fn probe_video<R: RangeReader + ?Sized>(reader: &R) -> Result<VideoFacts, RangeError> {
    let Some((offset, size)) = find_top_level_box(reader, b"moov")? else {
        return Ok(VideoFacts::default());
    };
    if size > MAX_MOOV_BYTES {
        return Ok(VideoFacts::default());
    }
    // At most MAX_MOOV_BYTES, so it fits in usize.
    let bytes = reader.read_at(offset, size as usize)?;
    Ok(VideoFacts {
        codec: codec_of(&bytes),
        duration_ms: duration_of(&bytes),
    })
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

fn fourcc(bytes: &[u8], at: usize) -> Option<String> {
    let code = bytes.get(at..at.checked_add(4)?)?;
    Some(code.iter().map(|&byte| char::from(byte)).collect())
}

fn codec_of(bytes: &[u8]) -> Option<String> {
    for trak in children(bytes, 0, bytes.len(), b"trak") {
        let Some(mdia) = children(bytes, trak.start, trak.end, b"mdia")
            .into_iter()
            .next()
        else {
            continue;
        };
        let hdlr = children(bytes, mdia.start, mdia.end, b"hdlr")
            .into_iter()
            .next();
        // Full box: version and flags (4), pre_defined (4), then the handler type.
        let is_video = hdlr.is_some_and(|hdlr| {
            hdlr.start + 12 <= hdlr.end
                && bytes.get(hdlr.start + 8..hdlr.start + 12) == Some(b"vide")
        });
        if !is_video {
            continue;
        }
        let first = |within: Option<BoxRange>, kind: &[u8; 4]| {
            within.and_then(|within| {
                children(bytes, within.start, within.end, kind)
                    .into_iter()
                    .next()
            })
        };
        let minf = first(Some(mdia), b"minf");
        let stbl = first(minf, b"stbl");
        let stsd = first(stbl, b"stsd");
        // Full box (4), entry count (4), then the first entry's size (4) and format (4).
        if let Some(stsd) = stsd.filter(|stsd| stsd.start + 16 <= stsd.end) {
            return fourcc(bytes, stsd.start + 12);
        }
    }
    None
}

/// `mvhd`: after its version and flags, creation and modification times (4 bytes each, 8 in version 1), the timescale
/// and the duration (4 bytes, 8 in version 1).
fn duration_of(bytes: &[u8]) -> Option<f64> {
    let mvhd = children(bytes, 0, bytes.len(), b"mvhd")
        .into_iter()
        .next()?;
    let wide = bytes.get(mvhd.start) == Some(&1);
    let timescale_at = mvhd.start + if wide { 20 } else { 12 };
    if timescale_at + 4 + if wide { 8 } else { 4 } > mvhd.end {
        return None;
    }
    let timescale = be32(bytes, timescale_at)?;
    let duration = if wide {
        be64(bytes, timescale_at + 4)?
    } else {
        u64::from(be32(bytes, timescale_at + 4)?)
    };
    // All ones: the length is unknown.
    if timescale == 0 || duration == 0 || duration == 0xffff_ffff {
        return None;
    }
    let ms = js_round(duration as f64 / f64::from(timescale) * 1000.0);
    (ms.is_finite() && ms > 0.0).then_some(ms)
}

#[derive(Clone, Copy, Debug)]
struct BoxRange {
    start: usize,
    end: usize,
}

/// The offset and size of the first top-level box of this type's body.
fn find_top_level_box<R: RangeReader + ?Sized>(
    reader: &R,
    kind: &[u8; 4],
) -> Result<Option<(u64, u64)>, RangeError> {
    let total = reader.len();
    let mut offset: u64 = 0;
    for _ in 0..MAX_TOP_LEVEL_BOXES {
        if offset + BOX_HEADER > total {
            break;
        }
        let header = reader.read_at(offset, (total - offset).min(16) as usize)?;
        let mut size = u64::from(be32(&header, 0).unwrap_or(0));
        let mut header_size = BOX_HEADER;
        if size == LARGE_SIZE {
            let Some(large) = be64(&header, 8) else {
                return Ok(None);
            };
            size = large;
            header_size = 16;
        } else if size == 0 {
            size = total - offset;
        }
        let Some(end) = offset
            .checked_add(size)
            .filter(|&end| size >= header_size && end <= total)
        else {
            return Ok(None);
        };
        if header.get(4..8) == Some(kind) {
            return Ok(Some((offset + header_size, size - header_size)));
        }
        offset = end;
    }
    Ok(None)
}

/// The boxes of one type directly inside `[start, end)` of `bytes`.
fn children(bytes: &[u8], start: usize, end: usize, kind: &[u8; 4]) -> Vec<BoxRange> {
    let mut found = Vec::new();
    let mut at = start;
    while at + BOX_HEADER as usize <= end {
        let mut size = be32(bytes, at).map_or(0, |size| size as usize);
        let mut header_size = BOX_HEADER as usize;
        if size == LARGE_SIZE as usize {
            if at + 16 > end {
                break;
            }
            let Some(large) = be64(bytes, at + 8).and_then(|large| usize::try_from(large).ok())
            else {
                break;
            };
            size = large;
            header_size = 16;
        } else if size == 0 {
            size = end - at;
        }
        let Some(box_end) = at
            .checked_add(size)
            .filter(|&box_end| size >= header_size && box_end <= end)
        else {
            break;
        };
        if bytes.get(at + 4..at + 8) == Some(kind) {
            found.push(BoxRange {
                start: at + header_size,
                end: box_end,
            });
        }
        at = box_end;
    }
    found
}
