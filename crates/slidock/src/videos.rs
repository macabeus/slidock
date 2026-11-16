//! Compressing a deck's videos for a download, as its pictures are (`crate::images`): the compiler plans, a host
//! re-encodes, the compiler checks, the bundle swaps. [`plan_videos`] lists every video of a printed deck with the
//! size to encode it at and what its movie holds, or why it stays as it is; [`check_video`] takes a host's file for
//! one of them; [`crate::images::CompressedMedia`] serves the taken files to the bundle, which writes them in place
//! of the originals ([`slidock_bundle::BundleOptions::replaced`]).
//!
//! The size rule is the pictures' ([`crate::images::DrawnSizes`]): at most [`crate::images::DENSITY`] pixels per CSS
//! pixel of the largest box the video is drawn in on a [`crate::images::SHOWN`] screen (every use: slides, layouts,
//! Magic Move and Morph matches, the slides a clip plays across, which share its file), as it shows (its track
//! matrix's quarter turn applied), never up, each side even (H.264's 4:2:0 sampling).
//!
//! A video stays as it is when its movie cannot be re-encoded faithfully as one H.264 track: an alpha layer or
//! channel (HEVC with alpha, a 32-bit depth), HDR (a PQ or HLG transfer), a matrix that scales, skews or mirrors,
//! non-square pixels, an edit list that plays more than one piece, more than one video or sound track, a fragmented
//! or unreadable movie. It plays its whole length: a trim (`MediaElement::trim`) is the player's, so the file keeps
//! every frame and the deck's timing stays as it is.
//!
//! A host's file is an MP4 (by its brands) whose `moov` comes before its media data, not fragmented, with exactly
//! one video track, H.264 (`avc1`, a 4:2:0 8-bit profile) at the planned size and unturned, with the original's frames
//! (at most 1 % or two fewer), a sound track when the original has one (AAC) and none otherwise, no other track, as long
//! as the original, each track starting and playing as the original's (within [`DURATION_TOLERANCE_MS`]), and smaller
//! than it. Its name is the original's with `.mp4`.
//!
//! A sound every use plays muted (each element muted, or at a volume of 0, as its frontend reads it; a use that may be
//! heard, or a volume that cannot be read, keeps it) is never heard ([`VideoPlan::silent`]): the host's file leaves it
//! out. For a video it would otherwise keep, the file may be the original's video track copied as it is (its format,
//! coded size, matrix, frames and timing), the sound being all it saves, unless the movie's structure is one a copy
//! does not carry ([`VideoPlan::uncopyable`]).

use std::sync::Arc;

use slidock_bundle::{BundleError, ReplacedMedia};
use slidock_core_api::MediaStore;
use slidock_html::{Documents, MediaPlanRef};
use slidock_io::{
    Mime, Movie, MovieTrack, RangeError, RangeReader, SampleEntry, VisualEntry, read_head,
    read_movie, sniff_mime, sps_video_signal,
};
use slidock_ir::ids::Sha256;
use thiserror::Error;

use crate::images::{DrawnSizes, file_bytes, target_size};

/// How far a host's file may be from the original's length, in milliseconds: a frame's rounding of each track's end.
pub const DURATION_TOLERANCE_MS: f64 = 200.0;

/// The transfer characteristics of HDR video (ISO/IEC 23091-2): SMPTE ST 2084 (PQ) and ARIB STD-B67 (HLG).
const HDR_TRANSFERS: [u16; 2] = [16, 18];

/// The primaries a re-encode maps into BT.709 (sRGB's): BT.2020 and Display P3 (SMPTE EG 432-1).
const WIDE_PRIMARIES: [u16; 2] = [9, 12];

/// The MPEG-4 object types of AAC in `esds` (ISO/IEC 14496-1 Table 5): MPEG-4 audio and MPEG-2 AAC Main, LC, SSR.
const AAC_OBJECT_TYPES: [u8; 4] = [0x40, 0x66, 0x67, 0x68];

/// The H.264 profiles whose pictures are 4:2:0 at 8 bits (ITU-T H.264 A.2): Baseline, Main, Extended and High.
const H264_420_PROFILES: [u8; 4] = [66, 77, 88, 100];

/// The edit list's rate of normal playback (16.16).
const NORMAL_RATE: i32 = 0x1_0000;

/// What a video's colour is, as its movie states it (`colr`, or an HEVC stream's VUI): ISO/IEC 23091-2 code points,
/// each `None` where nothing states it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VideoColour {
    pub primaries: Option<u16>,
    pub transfer: Option<u16>,
    pub matrix: Option<u16>,
    pub full_range: Option<bool>,
}

impl VideoColour {
    /// HDR: a PQ or HLG transfer.
    pub fn hdr(&self) -> bool {
        self.transfer
            .is_some_and(|transfer| HDR_TRANSFERS.contains(&transfer))
    }

    /// Primaries wider than BT.709's.
    pub fn wide_gamut(&self) -> bool {
        self.primaries
            .is_some_and(|primaries| WIDE_PRIMARIES.contains(&primaries))
    }
}

/// A video's sound track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedAudio {
    /// Its sample format (`mp4a`, `sowt`, `lpcm`, …).
    pub codec: String,
    /// `esds`' object type, for `mp4a`.
    pub object_type: Option<u8>,
    /// AAC, which an MP4 carries as it is.
    pub aac: bool,
    pub channels: u16,
    pub sample_rate: u32,
    /// Bits per sample, as the description states it.
    pub sample_size: u16,
    /// The bytes of its media: its samples', or, for uncompressed sound, its frames times the bytes per frame.
    pub bytes: u64,
}

/// Why a video stays as it is.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum VideoKeep {
    #[error("its movie cannot be read")]
    Unreadable,
    #[error("it is a fragmented movie")]
    Fragmented,
    #[error("it has {0} video tracks")]
    VideoTracks(usize),
    #[error("its video has {0} sample descriptions")]
    Formats(usize),
    #[error("it has an alpha channel, which H.264 cannot carry")]
    Alpha,
    #[error("its track matrix scales, skews or mirrors it")]
    Matrix,
    #[error("its pixels are not square ({0}:{1})")]
    PixelAspect(u32, u32),
    #[error("its edit list plays {0} pieces")]
    Edits(usize),
    #[error("it is HDR (transfer {0}): an SDR re-encode would change how it looks")]
    Hdr(u16),
    #[error("it has {0} sound tracks")]
    AudioTracks(usize),
    #[error("it has no frames")]
    Empty,
}

/// A video of a printed deck: what its movie holds, the size to encode it at, or why it stays as it is.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoPlan {
    /// `media/<sha256>.<ext>`, as the deck names it.
    pub path: String,
    pub sha256: Sha256,
    /// The file's type.
    pub mime: String,
    /// The file's size.
    pub bytes: u64,
    /// The video track's sample format (`avc1`, `hvc1`, …); empty when unreadable.
    pub codec: String,
    /// The size it shows at: its coded size, turned by its track matrix.
    pub width: u32,
    pub height: u32,
    /// The clockwise quarter turn its track matrix applies, in degrees.
    pub rotation: u16,
    /// The movie's length (`mvhd`).
    pub duration_ms: f64,
    /// The video track's samples and their bytes.
    pub frames: u32,
    pub video_bytes: u64,
    pub audio: Option<PlannedAudio>,
    pub colour: VideoColour,
    /// The size to encode it at: never larger than it shows, each side even.
    pub target_width: u32,
    pub target_height: u32,
    /// Why it stays as it is; `None` when a host may re-encode it.
    pub keep: Option<VideoKeep>,
    /// It has sound, and every use of it plays it muted ([`DrawnSizes::muted`]): its sound is never heard, so a host
    /// leaves it out, and copies the video as it is (unless [`Self::uncopyable`]) when it would keep it otherwise.
    pub silent: bool,
    /// Why its video track cannot be copied as it is into a file of its own (a structure a copy does not carry);
    /// `None` when it can, whatever keeps it from a re-encode.
    pub uncopyable: Option<VideoKeep>,
    /// Its video track's `tkhd` matrix, which a copy carries.
    pub matrix: [i32; 9],
    /// When the video and the sound start and how long they play ([`presented`]).
    pub video_span: Span,
    pub audio_span: Option<Span>,
}

/// When a track starts and how long it plays, in milliseconds of the movie.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Span {
    pub start_ms: f64,
    pub length_ms: f64,
}

/// A track's presented span: its leading empty edits delay its start; it plays its other edits' lengths, or its
/// whole media without one (an edit of no length plays the media from its time to the end).
pub fn presented(track: &MovieTrack, movie_timescale: u32) -> Span {
    let movie_ms = |duration: u64| {
        if movie_timescale == 0 {
            0.0
        } else {
            duration as f64 * 1000.0 / f64::from(movie_timescale)
        }
    };
    let media_ms = |duration: f64| {
        if track.timescale == 0 {
            0.0
        } else {
            duration * 1000.0 / f64::from(track.timescale)
        }
    };
    let leading = track
        .edits
        .iter()
        .take_while(|edit| edit.media_time < 0)
        .map(|edit| movie_ms(edit.duration))
        .sum();
    let played: Vec<f64> = track
        .edits
        .iter()
        .skip_while(|edit| edit.media_time < 0)
        .filter(|edit| edit.media_time >= 0)
        .map(|edit| {
            if edit.duration == 0 {
                media_ms(track.duration as f64 - edit.media_time as f64)
            } else {
                movie_ms(edit.duration)
            }
        })
        .collect();
    Span {
        start_ms: leading,
        length_ms: if played.is_empty() {
            media_ms(track.duration as f64)
        } else {
            played.iter().sum()
        },
    }
}

/// The movie types a deck's video may have.
fn is_movie(mime: &str) -> bool {
    matches!(mime, "video/mp4" | "video/quicktime" | "video/x-m4v")
}

/// The size of an encode of a video that shows at `width` × `height`, drawn at `drawn`: [`target_size`], each side
/// then made even (up when that stays within the video, else down), at least 2.
pub fn video_target_size(width: u32, height: u32, drawn: (f64, f64), density: f64) -> (u32, u32) {
    let (w, h) = target_size(width, height, drawn, density);
    let even = |side: u32, whole: u32| -> u32 {
        if side.is_multiple_of(2) {
            side
        } else if side < whole {
            side + 1
        } else {
            side - 1
        }
        .max(2)
    };
    (even(w, width), even(h, height))
}

/// The edits that play something: an empty edit (`media_time` −1) only delays the start.
fn played_edits(track: &MovieTrack) -> usize {
    track
        .edits
        .iter()
        .filter(|edit| edit.media_time >= 0)
        .count()
}

/// Whether a track's edit list is one a re-encode keeps: at most one leading empty edit, then at most one edit at
/// normal rate.
fn plain_edits(track: &MovieTrack) -> bool {
    let mut edits = track.edits.iter().peekable();
    if edits.peek().is_some_and(|edit| edit.media_time < 0) {
        edits.next();
    }
    let rest: Vec<_> = edits.collect();
    rest.len() <= 1
        && rest
            .iter()
            .all(|edit| edit.media_time >= 0 && edit.rate == NORMAL_RATE)
}

/// What a video track's description and stream state of its colour: `colr` first, then an HEVC SPS's VUI.
fn colour_of(track: &MovieTrack) -> VideoColour {
    let Some(visual) = track
        .entries
        .first()
        .and_then(|entry| entry.visual.as_ref())
    else {
        return VideoColour::default();
    };
    // 2 is "unspecified".
    let stated = |code: u16| (code != 2).then_some(code);
    if let Some(colr) = visual.colour {
        return VideoColour {
            primaries: stated(colr.primaries),
            transfer: stated(colr.transfer),
            matrix: stated(colr.matrix),
            full_range: colr.full_range,
        };
    }
    match config_of(visual, b"hvcC") {
        Some(config) => {
            let signal = hvcc_nals(config)
                .iter()
                .find(|nal| nal.nal_type == 33 && nal.layer == 0)
                .map(|nal| sps_video_signal(nal.bytes))
                .unwrap_or_default();
            VideoColour {
                primaries: signal.primaries,
                transfer: signal.transfer,
                matrix: signal.matrix,
                full_range: signal.full_range,
            }
        }
        None => VideoColour::default(),
    }
}

/// A visual sample description's decoder configuration of type `kind`.
fn config_of<'a>(visual: &'a VisualEntry, kind: &[u8; 4]) -> Option<&'a [u8]> {
    visual
        .config
        .as_ref()
        .filter(|(found, _)| found == kind)
        .map(|(_, config)| config.as_slice())
}

fn byte(bytes: &[u8], at: usize) -> u8 {
    bytes.get(at).copied().unwrap_or(0)
}

fn be16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

/// A parameter set of an `hvcC` body: its NAL unit type, layer id and bytes.
struct HevcNal<'a> {
    nal_type: u8,
    layer: u8,
    bytes: &'a [u8],
}

/// The parameter sets of an `hvcC` body (ISO/IEC 14496-15 §8.3.3.1): after 22 bytes, the number of arrays, each its
/// NAL unit type, a count and that many length-prefixed NAL units.
fn hvcc_nals(config: &[u8]) -> Vec<HevcNal<'_>> {
    let mut found = Vec::new();
    let mut at = 23usize;
    for _ in 0..byte(config, 22) {
        let nal_type = byte(config, at) & 0x3f;
        let Some(count) = be16(config, at + 1) else {
            break;
        };
        at += 3;
        for _ in 0..count {
            let Some(bytes) =
                be16(config, at).and_then(|len| config.get(at + 2..at + 2 + usize::from(len)))
            else {
                return found;
            };
            // The NAL header: a forbidden bit, the type (6 bits), the layer id (6 bits), the temporal id (3 bits).
            let layer = ((byte(bytes, 0) & 1) << 5) | (byte(bytes, 1) >> 3);
            found.push(HevcNal {
                nal_type,
                layer,
                bytes,
            });
            at += 2 + bytes.len();
        }
    }
    found
}

/// Whether an HEVC configuration describes more than one layer: Apple's HEVC with alpha codes the alpha as a second
/// layer (its VPS has `vps_max_layers_minus1` > 0, its parameter sets a layer id above 0).
fn hevc_layers(config: &[u8]) -> bool {
    hvcc_nals(config).iter().any(|nal| {
        // The VPS after its 2-byte header: its id (4 bits), two flags, then `vps_max_layers_minus1` (6 bits).
        let max_layers_minus1 = ((byte(nal.bytes, 2) & 0x03) << 4) | (byte(nal.bytes, 3) >> 4);
        (nal.nal_type == 32 && max_layers_minus1 > 0) || nal.layer > 0
    })
}

/// Whether a video track carries alpha: a second HEVC layer, a 32-bit depth, ProRes 4444.
fn has_alpha(track: &MovieTrack) -> bool {
    track.entries.iter().any(|entry| {
        let Some(visual) = &entry.visual else {
            return false;
        };
        visual.depth == 32 || config_of(visual, b"hvcC").is_some_and(hevc_layers)
    })
}

impl VideoKeep {
    /// Whether the video track can still be copied as it is (its samples, description, matrix and timing): a reason
    /// that holds against a re-encode only.
    pub fn copyable(&self) -> bool {
        matches!(
            self,
            VideoKeep::Alpha | VideoKeep::Matrix | VideoKeep::PixelAspect(..) | VideoKeep::Hdr(_)
        )
    }
}

/// Why a movie stays as it is, from its boxes alone; `None` when a host may re-encode it.
fn keep_of(movie: &Movie, video: &MovieTrack, colour: &VideoColour) -> Option<VideoKeep> {
    if movie.fragmented {
        return Some(VideoKeep::Fragmented);
    }
    let videos = movie.video_tracks().count();
    if videos != 1 {
        return Some(VideoKeep::VideoTracks(videos));
    }
    if video.samples == 0 || movie.duration == 0 {
        return Some(VideoKeep::Empty);
    }
    if video.entries.len() != 1 {
        return Some(VideoKeep::Formats(video.entries.len()));
    }
    if has_alpha(video) {
        return Some(VideoKeep::Alpha);
    }
    if video.rotation().is_none() {
        return Some(VideoKeep::Matrix);
    }
    if let Some((h, v)) = video
        .entries
        .first()
        .and_then(|entry| entry.visual.as_ref())
        .and_then(|visual| visual.pixel_aspect)
        .filter(|&(h, v)| h != v && h != 0 && v != 0)
    {
        return Some(VideoKeep::PixelAspect(h, v));
    }
    if let Some(transfer) = colour.transfer.filter(|_| colour.hdr()) {
        return Some(VideoKeep::Hdr(transfer));
    }
    let sounds: Vec<&MovieTrack> = movie.audio_tracks().collect();
    if sounds.len() > 1 {
        return Some(VideoKeep::AudioTracks(sounds.len()));
    }
    if let Some(track) = std::iter::once(video)
        .chain(sounds)
        .find(|track| !plain_edits(track))
    {
        return Some(VideoKeep::Edits(played_edits(track)));
    }
    None
}

fn audio_of(movie: &Movie) -> Option<PlannedAudio> {
    let track = movie.audio_tracks().next()?;
    let entry = track.entries.first()?;
    let audio = entry.audio.as_ref()?;
    let bytes = match audio.frame_bytes {
        Some(frame) => u64::from(track.samples) * u64::from(frame),
        None => track.sample_bytes,
    };
    let object_type = audio.object_type;
    Some(PlannedAudio {
        codec: entry.format_name(),
        object_type,
        aac: &entry.format == b"mp4a"
            && object_type.is_some_and(|object| AAC_OBJECT_TYPES.contains(&object)),
        channels: audio.channels,
        sample_rate: audio.sample_rate,
        sample_size: audio.sample_size,
        bytes,
    })
}

/// One file's plan, or `None` for a file that is no movie or drawn nowhere.
fn plan_file(
    path: &str,
    plan: &MediaPlanRef,
    drawn: &DrawnSizes,
    store: &dyn MediaStore,
    input: &dyn RangeReader,
    max_pixels: u64,
) -> Result<Option<VideoPlan>, RangeError> {
    if !is_movie(plan.mime.as_str()) {
        return Ok(None);
    }
    let Some(&box_px) = drawn.media.get(&plan.sha256) else {
        return Ok(None);
    };
    let Some(file) = file_bytes(plan, store, input, max_pixels) else {
        return Ok(None);
    };
    let reader = file.reader();
    let unreadable = VideoPlan {
        path: path.to_owned(),
        sha256: plan.sha256,
        mime: plan.mime.as_str().to_owned(),
        bytes: reader.len(),
        codec: String::new(),
        width: 0,
        height: 0,
        rotation: 0,
        duration_ms: 0.0,
        frames: 0,
        video_bytes: 0,
        audio: None,
        colour: VideoColour::default(),
        target_width: 0,
        target_height: 0,
        keep: Some(VideoKeep::Unreadable),
        silent: false,
        uncopyable: Some(VideoKeep::Unreadable),
        matrix: [0; 9],
        video_span: Span::default(),
        audio_span: None,
    };
    let Some(movie) = read_movie(reader)? else {
        return Ok(Some(unreadable));
    };
    let Some(video) = movie.video_tracks().next() else {
        return Ok(Some(VideoPlan {
            keep: Some(VideoKeep::VideoTracks(0)),
            ..unreadable
        }));
    };
    let Some((entry, visual)) = video
        .entries
        .first()
        .and_then(|entry| Some((entry, entry.visual.as_ref()?)))
    else {
        return Ok(Some(unreadable));
    };
    let colour = colour_of(video);
    let rotation = video.rotation().unwrap_or(0);
    let (coded_w, coded_h) = (u32::from(visual.width), u32::from(visual.height));
    let (width, height) = if rotation % 180 == 90 {
        (coded_h, coded_w)
    } else {
        (coded_w, coded_h)
    };
    let keep = if width < 2 || height < 2 {
        Some(VideoKeep::Empty)
    } else {
        keep_of(&movie, video, &colour)
    };
    let (target_width, target_height) = if width >= 2 && height >= 2 {
        video_target_size(width, height, box_px, drawn.density())
    } else {
        (width, height)
    };
    Ok(Some(VideoPlan {
        codec: entry.format_name(),
        width,
        height,
        rotation,
        duration_ms: movie.duration_ms().unwrap_or(0.0),
        frames: video.samples,
        video_bytes: video.sample_bytes,
        audio: audio_of(&movie),
        video_span: presented(video, movie.timescale),
        audio_span: movie
            .audio_tracks()
            .next()
            .map(|track| presented(track, movie.timescale)),
        colour,
        target_width,
        target_height,
        keep: keep.clone(),
        silent: drawn.muted.get(&plan.sha256) == Some(&true)
            && movie.audio_tracks().next().is_some(),
        uncopyable: keep.filter(|keep| !keep.copyable()),
        matrix: video.matrix,
        ..unreadable
    }))
}

/// The videos of `documents` drawn somewhere, in path order (see the module doc), each read from `store` (whose
/// ranges are ranges of `input`). A file that cannot be read at all is left out: it stays as it is.
pub fn plan_videos(
    documents: &Documents,
    drawn: &DrawnSizes,
    store: &dyn MediaStore,
    input: &dyn RangeReader,
    max_pixels: u64,
) -> Vec<VideoPlan> {
    documents
        .media
        .iter()
        .filter_map(|(path, plan)| {
            plan_file(path, plan, drawn, store, input, max_pixels)
                .ok()
                .flatten()
        })
        .collect()
}

/// Why a host's file is not taken for a planned video.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum VideoRefusal {
    #[error("{0} is no video the deck may re-encode")]
    NotPlanned(String),
    #[error("the video stays as it is: {0}")]
    Kept(VideoKeep),
    #[error("the file is {0}, not MP4")]
    Type(String),
    #[error("the file's movie cannot be read")]
    Unreadable,
    #[error("the file is a fragmented movie")]
    Fragmented,
    #[error("the file's movie boxes come after its media data")]
    NotFastStart,
    #[error("the file has {0} video tracks, not one")]
    VideoTracks(usize),
    #[error("the video is {0}, not H.264 (avc1)")]
    Codec(String),
    #[error("the H.264 profile {0} is not a 4:2:0 8-bit one")]
    Profile(u8),
    #[error(
        "the video is {width} × {height}, not the planned {expected_width} × {expected_height}"
    )]
    Size {
        width: u32,
        height: u32,
        expected_width: u32,
        expected_height: u32,
    },
    #[error("the video track is turned or scaled by its matrix")]
    Turned,
    #[error("the file has {found} sound tracks, the original {expected}")]
    AudioTracks { found: usize, expected: usize },
    #[error("the sound is {0}, not AAC")]
    AudioCodec(String),
    #[error("the file has a {0} track")]
    OtherTrack(String),
    #[error("the file has {found} frames, the original {expected}")]
    Frames { found: u32, expected: u32 },
    #[error(
        "the file's {track} starts at {start_ms} ms and plays {length_ms} ms, not at the original's {expected_start_ms} ms for {expected_length_ms} ms"
    )]
    Span {
        track: &'static str,
        start_ms: f64,
        length_ms: f64,
        expected_start_ms: f64,
        expected_length_ms: f64,
    },
    #[error("the file lasts {ms} ms, not the original's {expected} ms")]
    Duration { ms: f64, expected: f64 },
    #[error("the file has {bytes} bytes, no fewer than the original's {original}")]
    NotSmaller { bytes: u64, original: u64 },
}

/// A host's file for a planned video, taken: read from `reader` when the bundle writes it.
#[derive(Clone)]
pub struct CompressedVideo {
    /// The original's path.
    pub original: String,
    /// `media/<sha256>.mp4`.
    pub path: String,
    pub bytes: u64,
    pub reader: Arc<dyn RangeReader>,
}

impl std::fmt::Debug for CompressedVideo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompressedVideo")
            .field("original", &self.original)
            .field("path", &self.path)
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}

/// How much of a compressed video is read at a time as the bundle writes it.
const WRITE_CHUNK: u64 = 1024 * 1024;

impl CompressedVideo {
    /// What the bundle writes in place of the original.
    pub fn replaced(&self) -> ReplacedMedia {
        ReplacedMedia {
            path: self.path.clone(),
            mime: Mime::Mp4.as_str().to_owned(),
            bytes: self.bytes,
        }
    }

    /// Writes the file to `out`, a chunk at a time.
    pub fn write(&self, out: &mut dyn std::io::Write) -> Result<(), BundleError> {
        let failed = |message: String| BundleError::Media {
            path: self.path.clone(),
            message,
        };
        let mut offset = 0;
        while offset < self.bytes {
            // At most WRITE_CHUNK, so it fits in usize.
            let len = (self.bytes - offset).min(WRITE_CHUNK) as usize;
            let bytes = self
                .reader
                .read_at(offset, len)
                .map_err(|error| failed(error.to_string()))?;
            out.write_all(&bytes)
                .map_err(|error| failed(error.to_string()))?;
            offset += len as u64;
        }
        Ok(())
    }
}

/// The bundle's replacements for `videos` (by original path): what [`crate::images::CompressedMedia`] serves.
pub fn video_replacements(
    videos: &std::collections::BTreeMap<String, CompressedVideo>,
) -> std::collections::BTreeMap<String, ReplacedMedia> {
    videos
        .iter()
        .map(|(original, video)| (original.clone(), video.replaced()))
        .collect()
}

/// Whether a file's video track is the original's, copied: its format, coded size, matrix and frames.
fn copied_video(entry: &SampleEntry, video: &MovieTrack, plan: &VideoPlan) -> bool {
    let coded = if plan.rotation % 180 == 90 {
        (plan.height, plan.width)
    } else {
        (plan.width, plan.height)
    };
    entry.format_name() == plan.codec
        && entry
            .visual
            .as_ref()
            .is_some_and(|visual| (u32::from(visual.width), u32::from(visual.height)) == coded)
        && video.matrix == plan.matrix
        && video.samples == plan.frames
}

/// The checks on a re-encoded video track: H.264 of a 4:2:0 8-bit profile at the planned size, unturned.
fn check_encoded(
    entry: &SampleEntry,
    video: &MovieTrack,
    plan: &VideoPlan,
) -> Result<(), VideoRefusal> {
    let visual = entry
        .visual
        .as_ref()
        .filter(|_| &entry.format == b"avc1")
        .ok_or_else(|| VideoRefusal::Codec(entry.format_name()))?;
    let profile = config_of(visual, b"avcC")
        .map(|config| byte(config, 1))
        .ok_or_else(|| VideoRefusal::Codec("avc1 without avcC".to_owned()))?;
    if !H264_420_PROFILES.contains(&profile) {
        return Err(VideoRefusal::Profile(profile));
    }
    let (width, height) = (u32::from(visual.width), u32::from(visual.height));
    if (width, height) != (plan.target_width, plan.target_height) {
        return Err(VideoRefusal::Size {
            width,
            height,
            expected_width: plan.target_width,
            expected_height: plan.target_height,
        });
    }
    if !video.unturned() || (video.width >> 16, video.height >> 16) != (width, height) {
        return Err(VideoRefusal::Turned);
    }
    Ok(())
}

/// Checks a host's file for `plan`, read from `reader` (its movie boxes only): see the module doc.
pub fn check_video(
    plan: &VideoPlan,
    reader: Arc<dyn RangeReader>,
) -> Result<CompressedVideo, VideoRefusal> {
    // A sound never heard may be left out, the video then copied as it is.
    let copyable = plan.silent && plan.uncopyable.is_none();
    if let Some(keep) = plan.keep.as_ref().filter(|_| !copyable) {
        return Err(VideoRefusal::Kept(keep.clone()));
    }
    let unreadable = |_: RangeError| VideoRefusal::Unreadable;
    let head = read_head(reader.as_ref(), 64).map_err(unreadable)?;
    match sniff_mime(&head) {
        Some(Mime::Mp4) => {}
        Some(mime) => return Err(VideoRefusal::Type(mime.as_str().to_owned())),
        None => return Err(VideoRefusal::Type("of no known type".to_owned())),
    }
    let movie = read_movie(reader.as_ref())
        .map_err(unreadable)?
        .ok_or(VideoRefusal::Unreadable)?;
    if movie.fragmented {
        return Err(VideoRefusal::Fragmented);
    }
    if !movie.moov_first {
        return Err(VideoRefusal::NotFastStart);
    }
    let videos: Vec<&MovieTrack> = movie.video_tracks().collect();
    let [video] = videos.as_slice() else {
        return Err(VideoRefusal::VideoTracks(videos.len()));
    };
    let entry = match video.entries.as_slice() {
        [entry] => entry,
        _ => {
            return Err(VideoRefusal::Codec(format!(
                "{} formats",
                video.entries.len()
            )));
        }
    };
    let sounds: Vec<&MovieTrack> = movie.audio_tracks().collect();
    // A sound never heard is left out.
    let expected = usize::from(plan.audio.is_some() && !plan.silent);
    if sounds.len() != expected {
        return Err(VideoRefusal::AudioTracks {
            found: sounds.len(),
            expected,
        });
    }
    if !(copyable && sounds.is_empty() && copied_video(entry, video, plan)) {
        if let Some(keep) = &plan.keep {
            return Err(VideoRefusal::Kept(keep.clone()));
        }
        check_encoded(entry, video, plan)?;
    }
    if let Some(sound) = sounds.first() {
        let entry = sound.entries.first();
        let aac = entry.is_some_and(|entry| {
            &entry.format == b"mp4a"
                && entry
                    .audio
                    .as_ref()
                    .and_then(|audio| audio.object_type)
                    .is_some_and(|object| AAC_OBJECT_TYPES.contains(&object))
        });
        if !aac || sound.entries.len() != 1 {
            return Err(VideoRefusal::AudioCodec(
                entry.map_or_else(|| "missing".to_owned(), |entry| entry.format_name()),
            ));
        }
    }
    if let Some(other) = movie
        .tracks
        .iter()
        .find(|track| !track.is_video() && !track.is_audio())
    {
        return Err(VideoRefusal::OtherTrack(
            other.handler.iter().map(|&byte| char::from(byte)).collect(),
        ));
    }
    // Each track starts and plays as the original's: a track the movie's length does not show (it is the longest's)
    // may be shifted or cut.
    let tracks = [("video", Some(*video), Some(plan.video_span))]
        .into_iter()
        .chain(
            plan.audio_span
                .map(|span| ("sound", sounds.first().copied(), Some(span))),
        );
    for (track, found, expected) in tracks {
        let (Some(found), Some(expected)) = (found, expected) else {
            continue;
        };
        let span = presented(found, movie.timescale);
        if (span.start_ms - expected.start_ms).abs() > DURATION_TOLERANCE_MS
            || (span.length_ms - expected.length_ms).abs() > DURATION_TOLERANCE_MS
        {
            return Err(VideoRefusal::Span {
                track,
                start_ms: span.start_ms,
                length_ms: span.length_ms,
                expected_start_ms: expected.start_ms,
                expected_length_ms: expected.length_ms,
            });
        }
    }
    // An encoder that drops frames to stay under its bitrate leaves fewer; an edit list's ends may round off one or two.
    if video.samples.saturating_add((plan.frames / 100).max(2)) < plan.frames {
        return Err(VideoRefusal::Frames {
            found: video.samples,
            expected: plan.frames,
        });
    }
    let ms = movie.duration_ms().unwrap_or(0.0);
    if (ms - plan.duration_ms).abs() > DURATION_TOLERANCE_MS {
        return Err(VideoRefusal::Duration {
            ms,
            expected: plan.duration_ms,
        });
    }
    let bytes = reader.len();
    if bytes >= plan.bytes {
        return Err(VideoRefusal::NotSmaller {
            bytes,
            original: plan.bytes,
        });
    }
    Ok(CompressedVideo {
        original: plan.path.clone(),
        path: format!("media/{}.mp4", plan.sha256),
        bytes,
        reader,
    })
}
