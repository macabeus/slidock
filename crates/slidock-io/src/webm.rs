//! A WebM file's structure, read by range (Matroska's EBML elements, RFC 8794 and RFC 9559; WebM's subset): the EBML
//! header's document type, then the segment's info (`TimestampScale`, `Duration`), its tracks (number, type, codec,
//! a picture's size and `AlphaMode`), its cues and, walking every cluster, each track's blocks: how many, how many key
//! frames, how many carry a `BlockAdditional` of `BlockAddID` 1 (WebM's alpha), and the first and last block's
//! times. Block bodies are skipped: only element headers and a block's first bytes are read, through a window of
//! [`WINDOW_BYTES`]. An element of unknown size (a live stream) ends the walk, as does anything past [`MAX_BLOCKS`].

use std::borrow::Cow;

use crate::range::{RangeError, RangeReader};

/// Blocks walked at most.
pub const MAX_BLOCKS: usize = 4_194_304;
/// The info, tracks and cues are read whole up to this size.
const MAX_INDEX_BYTES: u64 = 16 * 1024 * 1024;
/// How much is read at a time while walking the clusters.
const WINDOW_BYTES: u64 = 1024 * 1024;
/// Top-level elements of the segment walked at most.
const MAX_TOP_LEVEL: usize = 1_048_576;

const EBML: u32 = 0x1a45_dfa3;
const DOC_TYPE: u32 = 0x4282;
const SEGMENT: u32 = 0x1853_8067;
const INFO: u32 = 0x1549_a966;
const TIMESTAMP_SCALE: u32 = 0x2a_d7b1;
const DURATION: u32 = 0x4489;
const TRACKS: u32 = 0x1654_ae6b;
const TRACK_ENTRY: u32 = 0xae;
const TRACK_NUMBER: u32 = 0xd7;
const TRACK_TYPE: u32 = 0x83;
const CODEC_ID: u32 = 0x86;
const VIDEO: u32 = 0xe0;
const PIXEL_WIDTH: u32 = 0xb0;
const PIXEL_HEIGHT: u32 = 0xba;
const ALPHA_MODE: u32 = 0x53c0;
const CUES: u32 = 0x1c53_bb6b;
const CUE_POINT: u32 = 0xbb;
const CLUSTER: u32 = 0x1f43_b675;
const TIMESTAMP: u32 = 0xe7;
const SIMPLE_BLOCK: u32 = 0xa3;
const BLOCK_GROUP: u32 = 0xa0;
const BLOCK: u32 = 0xa1;
const BLOCK_DURATION: u32 = 0x9b;
const REFERENCE_BLOCK: u32 = 0xfb;
const BLOCK_ADDITIONS: u32 = 0x75a1;
const BLOCK_MORE: u32 = 0xa6;
const BLOCK_ADD_ID: u32 = 0xee;

/// A track of a WebM file and what its blocks hold.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WebmTrack {
    pub number: u64,
    /// 1 video, 2 audio (Matroska's `TrackType`).
    pub kind: u64,
    /// `V_VP9`, `V_VP8`, `A_OPUS`, …
    pub codec: String,
    pub width: Option<u64>,
    pub height: Option<u64>,
    /// 1 when its blocks may carry alpha.
    pub alpha_mode: u64,
    pub blocks: usize,
    pub key_blocks: usize,
    /// Blocks with a `BlockAdditional` of `BlockAddID` 1.
    pub alpha_blocks: usize,
    /// Whether its first block is a key frame.
    pub starts_with_key: bool,
    /// Its first and last block's times, in ticks of the timestamp scale, and the last one's `BlockDuration`.
    pub first: Option<i64>,
    pub last: Option<i64>,
    pub last_duration: Option<u64>,
}

/// A WebM file's structure (see the module doc).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Webm {
    pub doc_type: String,
    /// Nanoseconds per tick.
    pub timestamp_scale: u64,
    /// In ticks.
    pub duration: Option<f64>,
    pub tracks: Vec<WebmTrack>,
    pub cue_points: usize,
    /// The cues come before the first cluster.
    pub cues_first: bool,
    pub clusters: usize,
    /// The walk reached the segment's end.
    pub complete: bool,
}

impl Webm {
    /// A time in ticks, in milliseconds.
    pub fn ms(&self, ticks: f64) -> f64 {
        ticks * self.timestamp_scale as f64 / 1_000_000.0
    }
}

/// An element: its ID (marker bits kept), where its body starts and its size (`None`: unknown).
#[derive(Clone, Copy, Debug)]
struct Header {
    id: u32,
    body: u64,
    size: Option<u64>,
}

/// A variable-length integer at `at` of `bytes`: its value (the marker kept for an ID) and length.
fn vint(bytes: &[u8], at: usize, keep_marker: bool) -> Option<(u64, usize)> {
    let first = *bytes.get(at)?;
    let length = first.leading_zeros() as usize + 1;
    if length > 8 {
        return None;
    }
    let mut value = if keep_marker {
        u64::from(first)
    } else {
        u64::from(first) & (0xff >> length)
    };
    for byte in bytes.get(at + 1..at + length)? {
        value = (value << 8) | u64::from(*byte);
    }
    Some((value, length))
}

/// An element header from `bytes` read at `offset`.
fn header(bytes: &[u8], offset: u64) -> Option<Header> {
    let (id, id_length) = vint(bytes, 0, true)?;
    if id_length > 4 {
        return None;
    }
    let (size, size_length) = vint(bytes, id_length, false)?;
    let unknown = size == (1u64 << (7 * size_length)) - 1;
    Some(Header {
        id: u32::try_from(id).ok()?,
        body: offset + (id_length + size_length) as u64,
        size: (!unknown).then_some(size),
    })
}

fn uint(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .take(8)
        .fold(0u64, |value, byte| (value << 8) | u64::from(*byte))
}

fn float(bytes: &[u8]) -> Option<f64> {
    match bytes.len() {
        4 => Some(f64::from(f32::from_be_bytes(bytes.try_into().ok()?))),
        8 => Some(f64::from_be_bytes(bytes.try_into().ok()?)),
        _ => None,
    }
}

/// The elements directly inside `bytes` (an element's body read whole): each one's ID and body.
fn children(bytes: &[u8]) -> Vec<(u32, &[u8])> {
    let mut found = Vec::new();
    let mut at = 0usize;
    while at < bytes.len() {
        let Some(head) = bytes.get(at..).and_then(|rest| header(rest, at as u64)) else {
            break;
        };
        let (Some(size), Ok(body)) = (head.size, usize::try_from(head.body)) else {
            break;
        };
        let Some(end) = usize::try_from(size)
            .ok()
            .and_then(|size| body.checked_add(size))
            .filter(|&end| end <= bytes.len())
        else {
            break;
        };
        if let Some(slice) = bytes.get(body..end) {
            found.push((head.id, slice));
        }
        at = end;
    }
    found
}

/// Reads through a window of the file, so the cluster walk's small reads are few range reads.
struct Window<'r, R: RangeReader + ?Sized> {
    reader: &'r R,
    start: u64,
    bytes: Vec<u8>,
}

impl<R: RangeReader + ?Sized> Window<'_, R> {
    /// Up to `len` bytes from `offset` (fewer at the file's end).
    fn read(&mut self, offset: u64, len: usize) -> Result<Cow<'_, [u8]>, RangeError> {
        let available = self.reader.len().saturating_sub(offset);
        if available == 0 {
            return Ok(Cow::Borrowed(&[]));
        }
        let len = (len as u64).min(available);
        let end = offset + len;
        if offset < self.start || end > self.start + self.bytes.len() as u64 {
            let take = WINDOW_BYTES.max(len).min(available);
            self.bytes = self
                .reader
                .read_at(offset, usize::try_from(take).unwrap_or(usize::MAX))?
                .into_owned();
            self.start = offset;
        }
        let from = usize::try_from(offset - self.start).unwrap_or(usize::MAX);
        let to = usize::try_from(len).map_or(usize::MAX, |len| from.saturating_add(len));
        Ok(Cow::Borrowed(self.bytes.get(from..to).unwrap_or(&[])))
    }

    fn header(&mut self, offset: u64) -> Result<Option<Header>, RangeError> {
        let bytes = self.read(offset, 12)?;
        Ok(header(&bytes, offset))
    }
}

fn track_of(entry: &[u8]) -> WebmTrack {
    let mut track = WebmTrack::default();
    for (id, body) in children(entry) {
        match id {
            TRACK_NUMBER => track.number = uint(body),
            TRACK_TYPE => track.kind = uint(body),
            CODEC_ID => {
                track.codec = body
                    .iter()
                    .take_while(|&&byte| byte != 0)
                    .map(|&byte| char::from(byte))
                    .collect();
            }
            VIDEO => {
                for (id, body) in children(body) {
                    match id {
                        PIXEL_WIDTH => track.width = Some(uint(body)),
                        PIXEL_HEIGHT => track.height = Some(uint(body)),
                        ALPHA_MODE => track.alpha_mode = uint(body),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    track
}

/// A block's track and time relative to its cluster, from its first bytes.
fn block_head(bytes: &[u8]) -> Option<(u64, i16, u8)> {
    let (track, length) = vint(bytes, 0, false)?;
    let relative = i16::from_be_bytes(bytes.get(length..length + 2)?.try_into().ok()?);
    Some((track, relative, *bytes.get(length + 2)?))
}

/// What one block says: its track, absolute time, key-ness, alpha and duration.
struct BlockFacts {
    track: u64,
    time: i64,
    key: bool,
    alpha: bool,
    duration: Option<u64>,
}

impl Webm {
    fn note(&mut self, block: BlockFacts) {
        let Some(track) = self
            .tracks
            .iter_mut()
            .find(|track| track.number == block.track)
        else {
            return;
        };
        if track.blocks == 0 {
            track.starts_with_key = block.key;
        }
        track.blocks += 1;
        track.key_blocks += usize::from(block.key);
        track.alpha_blocks += usize::from(block.alpha);
        track.first = Some(
            track
                .first
                .map_or(block.time, |first| first.min(block.time)),
        );
        if track.last.is_none_or(|last| block.time >= last) {
            track.last = Some(block.time);
            track.last_duration = block.duration;
        }
    }
}

/// A block group's facts from its children, read by header.
fn block_group<R: RangeReader + ?Sized>(
    window: &mut Window<'_, R>,
    start: u64,
    end: u64,
    cluster_time: i64,
) -> Result<Option<BlockFacts>, RangeError> {
    let mut facts: Option<BlockFacts> = None;
    let mut referenced = false;
    let mut alpha = false;
    let mut duration = None;
    let mut at = start;
    while at < end {
        let Some(head) = window.header(at)? else {
            break;
        };
        let Some(size) = head.size else {
            break;
        };
        let body_end = head.body.saturating_add(size).min(end);
        match head.id {
            BLOCK => {
                let bytes = window.read(head.body, 12)?;
                if let Some((track, relative, _)) = block_head(&bytes) {
                    facts = Some(BlockFacts {
                        track,
                        time: cluster_time + i64::from(relative),
                        key: false,
                        alpha: false,
                        duration: None,
                    });
                }
            }
            REFERENCE_BLOCK => referenced = true,
            BLOCK_DURATION => {
                let bytes = window.read(head.body, usize::try_from(size.min(8)).unwrap_or(8))?;
                duration = Some(uint(&bytes));
            }
            BLOCK_ADDITIONS if size <= MAX_INDEX_BYTES => {
                // A block addition's ID comes before its (large) payload: walk its headers.
                let mut more = head.body;
                while more < body_end {
                    let Some(entry) = window.header(more)? else {
                        break;
                    };
                    let Some(entry_size) = entry.size else {
                        break;
                    };
                    if entry.id == BLOCK_MORE {
                        let mut inner = entry.body;
                        let inner_end = entry.body.saturating_add(entry_size).min(body_end);
                        let mut id = 1;
                        while inner < inner_end {
                            let Some(field) = window.header(inner)? else {
                                break;
                            };
                            let Some(field_size) = field.size else {
                                break;
                            };
                            if field.id == BLOCK_ADD_ID {
                                let bytes = window.read(
                                    field.body,
                                    usize::try_from(field_size.min(8)).unwrap_or(8),
                                )?;
                                id = uint(&bytes);
                            }
                            inner = field.body.saturating_add(field_size);
                        }
                        alpha |= id == 1;
                    }
                    more = entry.body.saturating_add(entry_size);
                }
            }
            _ => {}
        }
        at = body_end;
    }
    Ok(facts.map(|facts| BlockFacts {
        key: !referenced,
        alpha,
        duration,
        ..facts
    }))
}

/// Reads `reader` as WebM; `None` when it is no EBML file or has no segment.
pub fn read_webm<R: RangeReader + ?Sized>(reader: &R) -> Result<Option<Webm>, RangeError> {
    let total = reader.len();
    let mut window = Window {
        reader,
        start: 0,
        bytes: Vec::new(),
    };
    let Some(ebml) = window.header(0)?.filter(|head| head.id == EBML) else {
        return Ok(None);
    };
    let Some(ebml_size) = ebml.size.filter(|&size| size <= 4096) else {
        return Ok(None);
    };
    let ebml_body = window
        .read(ebml.body, usize::try_from(ebml_size).unwrap_or(0))?
        .into_owned();
    let mut webm = Webm {
        timestamp_scale: 1_000_000,
        ..Webm::default()
    };
    for (id, body) in children(&ebml_body) {
        if id == DOC_TYPE {
            webm.doc_type = body
                .iter()
                .take_while(|&&byte| byte != 0)
                .map(|&byte| char::from(byte))
                .collect();
        }
    }
    let Some(segment) = window
        .header(ebml.body + ebml_size)?
        .filter(|head| head.id == SEGMENT)
    else {
        return Ok(None);
    };
    let segment_end = segment
        .size
        .map_or(total, |size| segment.body.saturating_add(size).min(total));
    let mut at = segment.body;
    let mut blocks = 0usize;
    for _ in 0..MAX_TOP_LEVEL {
        if at >= segment_end {
            webm.complete = true;
            break;
        }
        let Some(head) = window.header(at)? else {
            break;
        };
        let Some(size) = head.size else {
            break;
        };
        let end = head.body.saturating_add(size);
        if end > segment_end {
            break;
        }
        match head.id {
            INFO | TRACKS | CUES if size <= MAX_INDEX_BYTES => {
                let body = reader.read_at(head.body, usize::try_from(size).unwrap_or(0))?;
                read_index(&mut webm, head.id, &body);
            }
            CLUSTER => {
                webm.clusters += 1;
                let mut cluster_time = 0i64;
                let mut inner = head.body;
                while inner < end {
                    let Some(child) = window.header(inner)? else {
                        break;
                    };
                    let Some(child_size) = child.size else {
                        break;
                    };
                    let child_end = child.body.saturating_add(child_size).min(end);
                    match child.id {
                        TIMESTAMP => {
                            let bytes = window.read(
                                child.body,
                                usize::try_from(child_size.min(8)).unwrap_or(8),
                            )?;
                            cluster_time = i64::try_from(uint(&bytes)).unwrap_or(i64::MAX);
                        }
                        SIMPLE_BLOCK => {
                            let bytes = window.read(child.body, 12)?;
                            if let Some((track, relative, flags)) = block_head(&bytes) {
                                webm.note(BlockFacts {
                                    track,
                                    time: cluster_time + i64::from(relative),
                                    key: flags & 0x80 != 0,
                                    alpha: false,
                                    duration: None,
                                });
                            }
                            blocks += 1;
                        }
                        BLOCK_GROUP => {
                            if let Some(facts) =
                                block_group(&mut window, child.body, child_end, cluster_time)?
                            {
                                webm.note(facts);
                            }
                            blocks += 1;
                        }
                        _ => {}
                    }
                    if blocks > MAX_BLOCKS {
                        return Ok(Some(webm));
                    }
                    inner = child_end;
                }
            }
            _ => {}
        }
        at = end;
    }
    Ok(Some(webm))
}

fn read_index(webm: &mut Webm, id: u32, body: &[u8]) {
    match id {
        INFO => {
            for (id, value) in children(body) {
                match id {
                    TIMESTAMP_SCALE => webm.timestamp_scale = uint(value),
                    DURATION => webm.duration = float(value),
                    _ => {}
                }
            }
        }
        TRACKS => {
            webm.tracks = children(body)
                .into_iter()
                .filter(|(id, _)| *id == TRACK_ENTRY)
                .map(|(_, entry)| track_of(entry))
                .collect();
        }
        CUES => {
            webm.cue_points = children(body)
                .iter()
                .filter(|(id, _)| *id == CUE_POINT)
                .count();
            webm.cues_first = webm.clusters == 0;
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::range::BytesReader;

    /// A segment that claims more bytes than the file holds, whose walk reads past the end: no bytes there.
    #[test]
    fn reads_nothing_past_the_end_of_the_file() {
        let hex = [
            "1a45dfa301000000000000e7bd7dfeff0000000000047765626d428701000000000008408f4000000000001654ae6b01",
            "00000000000048ae010000000000003fd701000000000000010183010000000000000101860100000000000005565f56",
            "5039e00100000000000014b0010000000000000110ba0100000000000001101f43b6750100000000000019e701000000",
            "0000000100a30100000000000006810000800000",
        ]
        .concat();
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
            .collect();
        assert!(read_webm(&BytesReader::new(bytes)).is_ok());
    }

    fn size(len: usize) -> Vec<u8> {
        // Two-byte sizes keep the fixtures simple.
        vec![0x40 | ((len >> 8) as u8), len as u8]
    }

    fn element(id: &[u8], body: &[u8]) -> Vec<u8> {
        [id, &size(body.len()), body].concat()
    }

    fn uint_element(id: &[u8], value: u64) -> Vec<u8> {
        element(id, &value.to_be_bytes()[4..])
    }

    fn block(track: u8, relative: i16, flags: u8, payload: &[u8]) -> Vec<u8> {
        [
            &[0x80 | track][..],
            &relative.to_be_bytes(),
            &[flags],
            payload,
        ]
        .concat()
    }

    /// A WebM of a VP9 track with alpha: a key frame and a delta frame in one cluster, the second with
    /// `with_alpha` an addition, its cues before or after the cluster.
    fn fixture(alpha_on_second: bool, cues_first: bool) -> Vec<u8> {
        let header = element(&[0x1a, 0x45, 0xdf, 0xa3], &element(&[0x42, 0x82], b"webm"));
        let info = element(
            &[0x15, 0x49, 0xa9, 0x66],
            &[
                uint_element(&[0x2a, 0xd7, 0xb1], 1_000_000),
                element(&[0x44, 0x89], &1000.0f64.to_be_bytes()),
            ]
            .concat(),
        );
        let video = element(
            &[0xe0],
            &[
                uint_element(&[0xb0], 320),
                uint_element(&[0xba], 240),
                uint_element(&[0x53, 0xc0], 1),
            ]
            .concat(),
        );
        let tracks = element(
            &[0x16, 0x54, 0xae, 0x6b],
            &element(
                &[0xae],
                &[
                    uint_element(&[0xd7], 1),
                    uint_element(&[0x83], 1),
                    element(&[0x86], b"V_VP9"),
                    video,
                ]
                .concat(),
            ),
        );
        let addition = |payload: &[u8]| {
            element(
                &[0x75, 0xa1],
                &element(
                    &[0xa6],
                    &[uint_element(&[0xee], 1), element(&[0xa5], payload)].concat(),
                ),
            )
        };
        let first = element(
            &[0xa0],
            &[element(&[0xa1], &block(1, 0, 0, b"key")), addition(b"a1")].concat(),
        );
        let mut second_body = element(&[0xa1], &block(1, 500, 0, b"delta"));
        if alpha_on_second {
            second_body.extend(addition(b"a2"));
        }
        second_body.extend(uint_element(&[0x9b], 500));
        second_body.extend(element(&[0xfb], &[0xfe, 0x0c]));
        let cluster = element(
            &[0x1f, 0x43, 0xb6, 0x75],
            &[
                uint_element(&[0xe7], 0),
                first,
                element(&[0xa0], &second_body),
            ]
            .concat(),
        );
        let cues = element(
            &[0x1c, 0x53, 0xbb, 0x6b],
            &element(&[0xbb], &uint_element(&[0xb3], 0)),
        );
        let body = if cues_first {
            [info, tracks, cues, cluster].concat()
        } else {
            [info, tracks, cluster, cues].concat()
        };
        // The segment's size in eight bytes, as the app's writer writes it.
        let mut segment = vec![0x18, 0x53, 0x80, 0x67, 0x01];
        segment.extend(&(body.len() as u64).to_be_bytes()[1..]);
        [header, segment, body].concat()
    }

    #[test]
    fn reads_the_tracks_blocks_alpha_and_cues() {
        let bytes = fixture(true, true);
        let webm = read_webm(&BytesReader::new(&bytes)).unwrap().unwrap();
        assert_eq!(webm.doc_type, "webm");
        assert_eq!(webm.timestamp_scale, 1_000_000);
        assert_eq!(webm.duration, Some(1000.0));
        assert!(webm.complete);
        assert_eq!(
            (webm.cue_points, webm.cues_first, webm.clusters),
            (1, true, 1)
        );
        let [track] = webm.tracks.as_slice() else {
            panic!("one track: {:?}", webm.tracks);
        };
        assert_eq!(track.codec, "V_VP9");
        assert_eq!(
            (track.width, track.height, track.alpha_mode),
            (Some(320), Some(240), 1)
        );
        assert_eq!(
            (track.blocks, track.key_blocks, track.alpha_blocks),
            (2, 1, 2)
        );
        assert!(track.starts_with_key);
        assert_eq!(
            (track.first, track.last, track.last_duration),
            (Some(0), Some(500), Some(500))
        );
    }

    #[test]
    fn a_block_without_its_addition_and_cues_after_the_media_are_seen() {
        let bytes = fixture(false, false);
        let webm = read_webm(&BytesReader::new(&bytes)).unwrap().unwrap();
        assert!(!webm.cues_first);
        assert_eq!(webm.tracks[0].alpha_blocks, 1);
    }

    #[test]
    fn a_file_that_is_not_ebml_is_none() {
        let reader = BytesReader::new(b"\0\0\0\x18ftypisom");
        assert_eq!(read_webm(&reader).unwrap(), None);
    }

    #[test]
    fn a_truncated_file_is_incomplete() {
        let bytes = fixture(true, true);
        let cut = &bytes[..bytes.len() - 10];
        let webm = read_webm(&BytesReader::new(cut)).unwrap().unwrap();
        assert!(!webm.complete);
    }
}
