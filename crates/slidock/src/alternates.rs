//! A second file for a video that only some browsers play, offered after the original: the deck's `<video>` then
//! holds a `<source>` of each, the original first, typed with its bare sample format (`video/quicktime;
//! codecs="hvc1"`), a type WebKit takes and Chromium and Firefox refuse (Firefox decodes HEVC, but without its alpha
//! layer), so they play the copy (`docs/format.md` §1). A video kept as it is for its alpha
//! ([`VideoKeep::Alpha`]: HEVC with an alpha layer plays in Safari and in Chrome on macOS; a 32-bit QuickTime codec
//! in Safari alone) gets a WebM copy with WebM's alpha, which Chromium and Firefox play everywhere. Like a compressed
//! video, the compiler plans it ([`plan_alternates`]: the size is the video's planned one, `crate::videos`), a host
//! makes it, the compiler checks it ([`check_alternate`]) and the bundle writes it beside the original
//! ([`slidock_bundle::BundleOptions::alternates`]).
//!
//! A host's file is a WebM (by its bytes and its `DocType`) read to its end, with exactly one video track, VP9 or
//! VP8 of the planned size with `AlphaMode` 1 and every block carrying its alpha (`BlockAddID` 1), a key frame
//! first, its cues before its first cluster (so a player seeks before it has read the media), the original's frames
//! (at most 1 % or two fewer), a sound track (Opus or Vorbis) when the original has one some use may play and none
//! otherwise (a [`VideoPlan::silent`] sound is never heard, so it is left out, as a compressed video leaves it), no
//! other track, starting and ending as the original's video and lasting as long as the original (each within
//! [`DURATION_TOLERANCE_MS`]), and at most [`MAX_GROWTH`] times the original's size. Its name is the original's with
//! `.webm`.

use std::collections::BTreeMap;
use std::sync::Arc;

use slidock_bundle::{AlternateSource, BundleError, DeckMedia};
use slidock_io::{Mime, RangeError, RangeReader, read_head, read_webm, sniff_mime};
use thiserror::Error;

use crate::videos::{DURATION_TOLERANCE_MS, VideoKeep, VideoPlan};

/// A copy holds at most this many times the original's bytes.
pub const MAX_GROWTH: u64 = 2;

/// Matroska's track types.
const VIDEO_TRACK: u64 = 1;
const AUDIO_TRACK: u64 = 2;

/// Whether `plan`'s video gets a copy: it is kept for its alpha and shows at a size.
pub fn needs_alternate(plan: &VideoPlan) -> bool {
    plan.keep == Some(VideoKeep::Alpha) && plan.target_width >= 2 && plan.target_height >= 2
}

/// The videos of `plans` that get a copy, in their order.
pub fn plan_alternates(plans: &[VideoPlan]) -> Vec<VideoPlan> {
    plans
        .iter()
        .filter(|plan| needs_alternate(plan))
        .cloned()
        .collect()
}

/// Why a host's file is not taken as a video's copy.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum AlternateRefusal {
    #[error("{0} is no video that gets a copy")]
    NotPlanned(String),
    #[error("the file is {0}, not WebM")]
    Type(String),
    #[error("the file cannot be read as WebM")]
    Unreadable,
    #[error("the file ends before its segment does")]
    Incomplete,
    #[error("the file has {0} video tracks, not one")]
    VideoTracks(usize),
    #[error("the video is {0}, not VP9 or VP8")]
    Codec(String),
    #[error("the video track carries no alpha (AlphaMode {0})")]
    NoAlpha(u64),
    #[error("{with} of the video's {blocks} frames carry their alpha")]
    AlphaBlocks { with: usize, blocks: usize },
    #[error(
        "the video is {width} × {height}, not the planned {expected_width} × {expected_height}"
    )]
    Size {
        width: u64,
        height: u64,
        expected_width: u32,
        expected_height: u32,
    },
    #[error("the video does not start with a key frame")]
    NotKeyFirst,
    #[error("the file's cues do not come before its media")]
    Cues,
    #[error("the file has {found} sound tracks, the original {expected}")]
    AudioTracks { found: usize, expected: usize },
    #[error("the sound is {0}, not Opus or Vorbis")]
    AudioCodec(String),
    #[error("the file has a track of type {0}")]
    OtherTrack(u64),
    #[error("the file has {found} frames, the original {expected}")]
    Frames { found: usize, expected: u32 },
    #[error(
        "the video plays from {start_ms} ms to {end_ms} ms, not the original's {expected_start_ms} ms to {expected_end_ms} ms"
    )]
    Span {
        start_ms: f64,
        end_ms: f64,
        expected_start_ms: f64,
        expected_end_ms: f64,
    },
    #[error("the file lasts {ms} ms, not the original's {expected} ms")]
    Duration { ms: f64, expected: f64 },
    #[error("the file has {bytes} bytes, more than {limit} ({MAX_GROWTH} times the original's)")]
    TooLarge { bytes: u64, limit: u64 },
}

/// A host's copy of a video, taken: read from `reader` when the bundle writes it.
#[derive(Clone)]
pub struct AlternateVideo {
    /// The original's path.
    pub original: String,
    /// `media/<sha256>.webm`.
    pub path: String,
    /// The codec, as a `codecs` parameter names it (`vp9`, `vp8`).
    pub codec: &'static str,
    /// The original's video sample format (`hvc1`, …), which its `<source type>` names.
    pub original_codec: String,
    pub bytes: u64,
    pub reader: Arc<dyn RangeReader>,
}

impl std::fmt::Debug for AlternateVideo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlternateVideo")
            .field("original", &self.original)
            .field("path", &self.path)
            .field("codec", &self.codec)
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}

/// How much of a copy is read at a time as the bundle writes it.
const WRITE_CHUNK: u64 = 1024 * 1024;

impl AlternateVideo {
    /// What the bundle offers after the original.
    pub fn source(&self) -> AlternateSource {
        AlternateSource {
            path: self.path.clone(),
            mime: Mime::Webm.as_str().to_owned(),
            codecs: self.codec.to_owned(),
            original_codecs: self.original_codec.clone(),
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

/// The bundle's sources for `alternates` (by original path).
pub fn alternate_sources(
    alternates: &BTreeMap<String, AlternateVideo>,
) -> BTreeMap<String, AlternateSource> {
    alternates
        .iter()
        .map(|(original, video)| (original.clone(), video.source()))
        .collect()
}

/// A deck's media with the copies beside them: a copy's path is served from its reader, every other from `media`.
pub struct AlternateMedia<'a> {
    pub media: &'a dyn DeckMedia,
    /// By original path.
    pub alternates: &'a BTreeMap<String, AlternateVideo>,
}

impl std::fmt::Debug for AlternateMedia<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlternateMedia")
            .field("alternates", &self.alternates.len())
            .finish_non_exhaustive()
    }
}

impl DeckMedia for AlternateMedia<'_> {
    fn write_media(&self, path: &str, out: &mut dyn std::io::Write) -> Result<(), BundleError> {
        match self.alternates.values().find(|video| video.path == path) {
            Some(video) => video.write(out),
            None => self.media.write_media(path, out),
        }
    }
}

/// Checks a host's copy for `plan`, read from `reader` (its element headers only): see the module doc.
pub fn check_alternate(
    plan: &VideoPlan,
    reader: Arc<dyn RangeReader>,
) -> Result<AlternateVideo, AlternateRefusal> {
    if !needs_alternate(plan) {
        return Err(AlternateRefusal::NotPlanned(plan.path.clone()));
    }
    let unreadable = |_: RangeError| AlternateRefusal::Unreadable;
    let head = read_head(reader.as_ref(), 64).map_err(unreadable)?;
    match sniff_mime(&head) {
        Some(Mime::Webm) => {}
        Some(mime) => return Err(AlternateRefusal::Type(mime.as_str().to_owned())),
        None => return Err(AlternateRefusal::Type("of no known type".to_owned())),
    }
    let webm = read_webm(reader.as_ref())
        .map_err(unreadable)?
        .filter(|webm| webm.doc_type == "webm")
        .ok_or(AlternateRefusal::Unreadable)?;
    if !webm.complete {
        return Err(AlternateRefusal::Incomplete);
    }
    let videos: Vec<_> = webm
        .tracks
        .iter()
        .filter(|track| track.kind == VIDEO_TRACK)
        .collect();
    let [video] = videos.as_slice() else {
        return Err(AlternateRefusal::VideoTracks(videos.len()));
    };
    let codec = match video.codec.as_str() {
        "V_VP9" => "vp9",
        "V_VP8" => "vp8",
        other => return Err(AlternateRefusal::Codec(other.to_owned())),
    };
    if video.alpha_mode != 1 {
        return Err(AlternateRefusal::NoAlpha(video.alpha_mode));
    }
    let (width, height) = (video.width.unwrap_or(0), video.height.unwrap_or(0));
    if (width, height) != (u64::from(plan.target_width), u64::from(plan.target_height)) {
        return Err(AlternateRefusal::Size {
            width,
            height,
            expected_width: plan.target_width,
            expected_height: plan.target_height,
        });
    }
    if video.alpha_blocks != video.blocks {
        return Err(AlternateRefusal::AlphaBlocks {
            with: video.alpha_blocks,
            blocks: video.blocks,
        });
    }
    if !video.starts_with_key {
        return Err(AlternateRefusal::NotKeyFirst);
    }
    if webm.cue_points == 0 || !webm.cues_first {
        return Err(AlternateRefusal::Cues);
    }
    let sounds: Vec<_> = webm
        .tracks
        .iter()
        .filter(|track| track.kind == AUDIO_TRACK)
        .collect();
    // A sound every use plays muted is never heard: the copy leaves it out, as a compressed video does.
    let expected = usize::from(plan.audio.is_some() && !plan.silent);
    if sounds.len() != expected {
        return Err(AlternateRefusal::AudioTracks {
            found: sounds.len(),
            expected,
        });
    }
    if let Some(sound) = sounds
        .iter()
        .find(|sound| !matches!(sound.codec.as_str(), "A_OPUS" | "A_VORBIS"))
    {
        return Err(AlternateRefusal::AudioCodec(sound.codec.clone()));
    }
    if let Some(other) = webm
        .tracks
        .iter()
        .find(|track| track.kind != VIDEO_TRACK && track.kind != AUDIO_TRACK)
    {
        return Err(AlternateRefusal::OtherTrack(other.kind));
    }
    // An encoder that drops frames leaves fewer; frames within a millisecond of each other are one.
    let allowed = usize::try_from((plan.frames / 100).max(2)).unwrap_or(usize::MAX);
    let expected_frames = usize::try_from(plan.frames).unwrap_or(usize::MAX);
    if video.blocks.saturating_add(allowed) < expected_frames || video.blocks > expected_frames {
        return Err(AlternateRefusal::Frames {
            found: video.blocks,
            expected: plan.frames,
        });
    }
    let ms = |ticks: i64| webm.ms(ticks as f64);
    let start_ms = ms(video.first.unwrap_or(0));
    let end_ms = ms(video.last.unwrap_or(0)) + webm.ms(video.last_duration.unwrap_or(0) as f64);
    let expected_end_ms = plan.video_span.start_ms + plan.video_span.length_ms;
    if (start_ms - plan.video_span.start_ms).abs() > DURATION_TOLERANCE_MS
        || (end_ms - expected_end_ms).abs() > DURATION_TOLERANCE_MS
    {
        return Err(AlternateRefusal::Span {
            start_ms,
            end_ms,
            expected_start_ms: plan.video_span.start_ms,
            expected_end_ms,
        });
    }
    let duration = webm.duration.map_or(0.0, |ticks| webm.ms(ticks));
    if (duration - plan.duration_ms).abs() > DURATION_TOLERANCE_MS {
        return Err(AlternateRefusal::Duration {
            ms: duration,
            expected: plan.duration_ms,
        });
    }
    let bytes = reader.len();
    let limit = plan.bytes.saturating_mul(MAX_GROWTH);
    if bytes > limit {
        return Err(AlternateRefusal::TooLarge { bytes, limit });
    }
    Ok(AlternateVideo {
        original: plan.path.clone(),
        path: format!("media/{}.webm", plan.sha256),
        codec,
        original_codec: plan.codec.clone(),
        bytes,
        reader,
    })
}
