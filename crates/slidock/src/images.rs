//! Compressing a deck's pictures for a download: the compiler plans, a host encodes, the compiler checks, the bundle
//! swaps (videos follow the same steps: `crate::videos`). [`drawn_sizes`] reads the largest size each medium (a
//! picture, a poster, a video) is drawn at from the resolved presentation; [`plan_images`]
//! lists the raster files of a printed deck a host may re-encode, each with the pixel size to encode it at;
//! [`check_image`] takes a host's file for one of them; [`CompressedMedia`] serves the taken files to the bundle,
//! which writes them in place of the originals ([`slidock_bundle::BundleOptions::replaced`]).
//!
//! The size rule: a medium keeps at most [`DENSITY`] pixels per CSS pixel of the largest box it is ever drawn in when
//! the slide fills a [`SHOWN`] screen, and is never upscaled. A picture draws its whole image at its frame divided by
//! the share its crop leaves visible; an image fill, a background, a poster and a Foreign fallback draw it at their
//! box. Every layout, master and slide counts, each use of a medium (the largest wins, per axis), a held scale-up of a
//! build (grow/shrink, pulse, a scale track) and the boxes a Morph or Magic Move carries a matched element through.
//! Every output draws a medium stretched to its box, so a smaller file changes no layout.
//!
//! What may be re-encoded: still PNG, JPEG, WebP and GIF files. SVG, animated GIF, APNG and WebP, AVIF and video stay
//! as they are. A host's file is WebP, JPEG or PNG (by its bytes), still, of the planned size, stored as it shows (no
//! EXIF turn) and smaller than the original; its name is the original's with the type's extension. A JPEG's EXIF
//! orientation is applied by the browser that draws it, so its planned size is the size it shows at.

use std::collections::BTreeMap;
use std::sync::Arc;

use slidock_bundle::{BundleError, DeckMedia, ReplacedMedia};
use slidock_core_api::{MediaSource, MediaStore};
use slidock_html::{Documents, MediaAction, MediaPlanRef};
use slidock_io::{
    BytesReader, Mime, RangeError, RangeReader, SubReader, jpeg_orientation, read_head,
    read_image_size, sniff_mime,
};
use slidock_ir::effects::{BuildEffect, Keyframes, TransitionEffect};
use slidock_ir::elements::MediaKind;
use slidock_ir::frame::Frame;
use slidock_ir::ids::Sha256;
use slidock_ir::paint::{Fill, ResolvedCellFill, ResolvedFill};
use slidock_ir::resolved::{Body, Element, Presentation, Slide};
use slidock_ir::table::table_cells;
use thiserror::Error;

use crate::media::write_media_file;
use crate::videos::CompressedVideo;

/// The screen a slide fills, in CSS pixels: a full-HD display.
pub const SHOWN: (f64, f64) = (1920.0, 1080.0);

/// Image pixels kept per CSS pixel of a box on the [`SHOWN`] screen.
pub const DENSITY: f64 = 2.0;

/// CSS pixels per point.
const PX_PER_PT: f64 = 4.0 / 3.0;

/// The share of an image a crop leaves visible, at least: the printer's floor.
const MIN_VISIBLE: f64 = 0.005;

/// How much of a file is read for its size and orientation before the size reader reads on.
const HEAD_BYTES: usize = 64 * 1024;

/// The largest box each medium is drawn in, in CSS pixels of the slide at its own size, and the slide's size.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawnSizes {
    /// The slide's width and height in CSS pixels.
    pub slide: (f64, f64),
    /// By medium: the widest and the tallest it is drawn (each axis on its own).
    pub media: BTreeMap<Sha256, (f64, f64)>,
    /// By movie: whether every use plays it muted (a muted element, or a volume of 0 as its frontend reads it), so its
    /// sound is never heard. One use that may be heard is enough to keep it.
    pub muted: BTreeMap<Sha256, bool>,
}

impl DrawnSizes {
    /// Image pixels per CSS pixel of the slide at its own size: [`DENSITY`] on the [`SHOWN`] screen the slide fits.
    pub fn density(&self) -> f64 {
        let (w, h) = self.slide;
        if w <= 0.0 || h <= 0.0 {
            return DENSITY;
        }
        DENSITY * f64::min(SHOWN.0 / w, SHOWN.1 / h)
    }

    fn add(&mut self, media: Sha256, (w, h): (f64, f64)) {
        let entry = self.media.entry(media).or_insert((0.0, 0.0));
        entry.0 = entry.0.max(w);
        entry.1 = entry.1.max(h);
    }
}

fn frame_px(frame: &Frame) -> (f64, f64) {
    (frame.w.to_f64() * PX_PER_PT, frame.h.to_f64() * PX_PER_PT)
}

fn scaled((w, h): (f64, f64), (sx, sy): (f64, f64)) -> (f64, f64) {
    (w * sx, h * sy)
}

/// A factor on each axis.
type Scale2 = (f64, f64);

const UNSCALED: Scale2 = (1.0, 1.0);

fn larger(a: Scale2, b: Scale2) -> Scale2 {
    (a.0.max(b.0), a.1.max(b.1))
}

/// The largest factors a build step holds or passes its element at, on each axis: a grow/shrink, a pulse, a scale
/// track.
fn step_scale(effect: &BuildEffect) -> Scale2 {
    let scale = match effect {
        BuildEffect::GrowShrink(grow) => (grow.scale.x.to_f64(), grow.scale.y.to_f64()),
        BuildEffect::Pulse(pulse) => (pulse.scale.to_f64(), pulse.scale.to_f64()),
        BuildEffect::Behaviors(tracks) => tracks
            .iter()
            .filter_map(|track| match track {
                Keyframes::Scale(track) => Some(
                    track
                        .frames
                        .iter()
                        .map(|frame| (frame.value.x.to_f64(), frame.value.y.to_f64()))
                        .fold(UNSCALED, larger),
                ),
                _ => None,
            })
            .fold(UNSCALED, larger),
        _ => UNSCALED,
    };
    larger(scale, UNSCALED)
}

/// A slide's own elements by match key, through groups and Foreign fallbacks; a key on two elements matches nothing
/// (the player's rule).
fn keyed(slide: &Slide) -> BTreeMap<&str, Option<&Element>> {
    let mut found: BTreeMap<&str, Option<&Element>> = BTreeMap::new();
    let mut stack: Vec<&Element> = slide.body.elements().iter().collect();
    while let Some(element) = stack.pop() {
        if let Some(key) = element.match_key().filter(|key| !key.is_empty()) {
            found
                .entry(key)
                .and_modify(|seen| *seen = None)
                .or_insert(Some(element));
        }
        stack.extend(children(element));
    }
    found
}

fn children(element: &Element) -> Vec<&Element> {
    match element {
        Element::Group(group) => group.children.iter().collect(),
        Element::Foreign(foreign) => foreign
            .fallback
            .as_ref()
            .and_then(|fallback| fallback.elements.as_ref())
            .map(|elements| elements.iter().collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// How much larger than its box an element of a slide is drawn, on each axis, by slide index and element id: a held
/// or passing scale of its build steps, and a Morph or Magic Move carrying it to its match's box (into the slide,
/// from the slide before it and from the last slide shown before it). An element's factors apply to everything
/// inside it.
fn motion_scales(presentation: &Presentation) -> BTreeMap<(usize, String), Scale2> {
    let mut scales: BTreeMap<(usize, String), Scale2> = BTreeMap::new();
    let mut raise = |slide: usize, element: String, scale: Scale2| {
        if scale.0 > 1.0 || scale.1 > 1.0 {
            let entry = scales.entry((slide, element)).or_insert(UNSCALED);
            *entry = larger(*entry, scale);
        }
    };
    for (index, slide) in presentation.slides.iter().enumerate() {
        for step in &slide.timeline {
            if let Some(native) = &step.effect.native {
                raise(
                    index,
                    step.target.element.to_string(),
                    step_scale(&native.effect),
                );
            }
        }
        let morphs = slide
            .transition
            .as_ref()
            .and_then(|transition| transition.effect.native.as_ref())
            .is_some_and(|native| {
                matches!(
                    native.effect,
                    TransitionEffect::Morph(_) | TransitionEffect::MagicMove(_)
                )
            });
        if !morphs || index == 0 {
            continue;
        }
        let shown_before = presentation
            .slides
            .iter()
            .take(index)
            .rposition(|before| !before.hidden);
        let mut before = vec![index - 1];
        before.extend(shown_before.filter(|&shown| shown != index - 1));
        let into = keyed(slide);
        for from_index in before {
            let Some(from_slide) = presentation.slides.get(from_index) else {
                continue;
            };
            for (key, from) in keyed(from_slide) {
                let (Some(from), Some(Some(to))) = (from, into.get(key)) else {
                    continue;
                };
                let (fw, fh) = frame_px(from.frame());
                let (tw, th) = frame_px(to.frame());
                let ratio = |a: f64, b: f64| if b > 0.0 { a / b } else { 1.0 };
                raise(
                    from_index,
                    from.id().to_string(),
                    (ratio(tw, fw), ratio(th, fh)),
                );
                raise(index, to.id().to_string(), (ratio(fw, tw), ratio(fh, th)));
            }
        }
    }
    scales
}

/// Records the media `element` and its children draw, at `scale` times their boxes.
fn element_sizes(
    element: &Element,
    scale: Scale2,
    slide: Option<usize>,
    motion: &BTreeMap<(usize, String), Scale2>,
    out: &mut DrawnSizes,
) {
    let own = slide
        .and_then(|slide| motion.get(&(slide, element.id().to_string())))
        .copied()
        .unwrap_or(UNSCALED);
    let scale = (scale.0 * own.0, scale.1 * own.1);
    let boxed = scaled(frame_px(element.frame()), scale);
    match element {
        Element::Picture(picture) => {
            let (l, t, r, b) = picture.crop.as_ref().map_or((0.0, 0.0, 0.0, 0.0), |crop| {
                (
                    crop.l.to_f64(),
                    crop.t.to_f64(),
                    crop.r.to_f64(),
                    crop.b.to_f64(),
                )
            });
            let visible_w = f64::max(MIN_VISIBLE, 1.0 - l - r);
            let visible_h = f64::max(MIN_VISIBLE, 1.0 - t - b);
            out.add(picture.media, (boxed.0 / visible_w, boxed.1 / visible_h));
        }
        Element::Shape(shape) => {
            if let Some(ResolvedFill::Image { media, .. }) = &shape.fill {
                out.add(*media, boxed);
            }
        }
        Element::Media(media) => {
            if let Some(poster) = media.poster {
                out.add(poster, boxed);
            }
            // The movie itself, for video compression (`crate::videos`); a sound draws nothing.
            if media.media_kind == MediaKind::Video
                && let Some(movie) = media.media
            {
                out.add(movie, boxed);
                let muted = out.muted.entry(movie).or_insert(true);
                *muted &= media.muted;
            }
        }
        Element::Table(table) => {
            // A cell is never larger than its table.
            for (cell, _, _) in table_cells(&table.rows) {
                if let Some(ResolvedCellFill::Fill(ResolvedFill::Image { media, .. })) = &cell.fill
                {
                    out.add(*media, boxed);
                }
            }
        }
        Element::Foreign(foreign) => {
            if let Some(media) = foreign
                .fallback
                .as_ref()
                .and_then(|fallback| fallback.media)
            {
                out.add(media, boxed);
            }
        }
        Element::Group(_) => {}
    }
    for child in children(element) {
        element_sizes(child, scale, slide, motion, out);
    }
}

fn background_media<B>(
    body: &Body<B>,
    built: impl Fn(&B) -> Option<&ResolvedFill>,
) -> Option<Sha256> {
    match body {
        Body::Built(b) => match built(b) {
            Some(ResolvedFill::Image { media, .. }) => Some(*media),
            _ => None,
        },
        Body::Failed { background, .. } => match background {
            Some(Fill::Image { media, .. }) => Some(*media),
            _ => None,
        },
    }
}

/// The largest box each medium of `presentation` is drawn in: every master, layout and slide, each use, the motion
/// that draws an element larger than its box (see the module doc).
pub fn drawn_sizes(presentation: &Presentation) -> DrawnSizes {
    let slide = (
        presentation.size.w.to_f64() * PX_PER_PT,
        presentation.size.h.to_f64() * PX_PER_PT,
    );
    let mut out = DrawnSizes {
        slide,
        media: BTreeMap::new(),
        muted: BTreeMap::new(),
    };
    let motion = motion_scales(presentation);
    for master in presentation.masters.values() {
        if let Some(media) = background_media(&master.body, |b| b.background.as_ref()) {
            out.add(media, slide);
        }
        for element in master.body.elements() {
            element_sizes(element, UNSCALED, None, &motion, &mut out);
        }
    }
    for layout in presentation.layouts.values() {
        if let Some(media) = background_media(&layout.body, |b| b.background.as_ref()) {
            out.add(media, slide);
        }
        for element in layout.body.elements() {
            element_sizes(element, UNSCALED, None, &motion, &mut out);
        }
    }
    for (index, shown) in presentation.slides.iter().enumerate() {
        if let Some(media) = background_media(&shown.body, |b| b.background.as_ref()) {
            out.add(media, slide);
        }
        for element in shown.body.elements() {
            element_sizes(element, UNSCALED, Some(index), &motion, &mut out);
        }
    }
    out
}

/// A raster file of a printed deck a host may re-encode smaller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImagePlan {
    /// `media/<sha256>.<ext>`, as the deck names it.
    pub path: String,
    pub sha256: Sha256,
    /// The file's type.
    pub mime: String,
    /// The file's size.
    pub bytes: u64,
    /// The size it shows at: its pixels, turned by its EXIF orientation.
    pub width: u32,
    pub height: u32,
    /// Its EXIF orientation (1–8; 1 when it states none).
    pub orientation: u16,
    /// The size to encode it at: never larger than it shows.
    pub target_width: u32,
    pub target_height: u32,
}

/// The size `drawn` needs of an image that shows at `width` × `height`, at `density` pixels per CSS pixel: the
/// image scaled (keeping its proportions) until its tighter axis meets the drawn box, never up.
pub fn target_size(width: u32, height: u32, drawn: (f64, f64), density: f64) -> (u32, u32) {
    let (w, h) = (f64::from(width), f64::from(height));
    let factor = f64::max(drawn.0 * density / w, drawn.1 * density / h);
    if !factor.is_finite() || factor >= 1.0 {
        return (width, height);
    }
    let side = |pixels: f64, whole: u32| -> u32 {
        // At most `whole`, which fits.
        // A hair under a whole pixel is that pixel: `pixels * factor` carries the rounding of `factor`.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let side = (pixels * factor - 1e-6).ceil().max(1.0) as u32;
        side.min(whole)
    };
    (side(w, width), side(h, height))
}

/// What a re-encoded file may be, by type: the extension the deck names it with.
fn encoded_ext(mime: Mime) -> Option<&'static str> {
    match mime {
        Mime::Webp => Some("webp"),
        Mime::Jpeg => Some("jpg"),
        Mime::Png => Some("png"),
        _ => None,
    }
}

/// The planned files' types.
fn compressible(mime: &str) -> Option<Mime> {
    match mime {
        "image/png" => Some(Mime::Png),
        "image/jpeg" => Some(Mime::Jpeg),
        "image/webp" => Some(Mime::Webp),
        "image/gif" => Some(Mime::Gif),
        _ => None,
    }
}

/// The bytes of a deck's file: a range of the input or of a reader, or bytes in hand.
pub(crate) enum FileBytes<'a> {
    Range(SubReader<&'a dyn RangeReader>),
    Reader(Arc<dyn RangeReader>),
    Bytes(BytesReader<Arc<[u8]>>),
    Written(BytesReader<Vec<u8>>),
}

impl FileBytes<'_> {
    pub(crate) fn reader(&self) -> &dyn RangeReader {
        match self {
            FileBytes::Range(reader) => reader,
            FileBytes::Reader(reader) => reader.as_ref(),
            FileBytes::Bytes(reader) => reader,
            FileBytes::Written(reader) => reader,
        }
    }
}

pub(crate) fn file_bytes<'a>(
    plan: &MediaPlanRef,
    store: &dyn MediaStore,
    input: &'a dyn RangeReader,
    max_pixels: u64,
) -> Option<FileBytes<'a>> {
    let handle = match plan.action {
        MediaAction::Copy => store.open(&plan.sha256)?,
        MediaAction::Transcode => match store.decoded(&plan.sha256) {
            Some(decoded) => decoded,
            None => {
                let mut written = Vec::new();
                write_media_file(plan, store, input, max_pixels, &mut written).ok()?;
                return Some(FileBytes::Written(BytesReader::new(written)));
            }
        },
    };
    Some(match handle.source {
        MediaSource::InputRange { offset, len } => {
            FileBytes::Range(SubReader::new(input, offset, len).ok()?)
        }
        MediaSource::Bytes(bytes) => FileBytes::Bytes(BytesReader::new(bytes)),
        MediaSource::Reader(reader) => FileBytes::Reader(reader),
    })
}

/// One file's plan, or `None` for a file that stays as it is (animated, unreadable, or drawn nowhere).
fn plan_file(
    path: &str,
    plan: &MediaPlanRef,
    drawn: &DrawnSizes,
    store: &dyn MediaStore,
    input: &dyn RangeReader,
    max_pixels: u64,
) -> Result<Option<ImagePlan>, RangeError> {
    let Some(mime) = compressible(plan.mime.as_str()) else {
        return Ok(None);
    };
    let Some(&box_px) = drawn.media.get(&plan.sha256) else {
        return Ok(None);
    };
    let Some(file) = file_bytes(plan, store, input, max_pixels) else {
        return Ok(None);
    };
    let reader = file.reader();
    let bytes = reader.len();
    // A GIF's and a WebP's frames are counted in the bytes read, so those are read whole.
    let head_len = match mime {
        Mime::Gif | Mime::Webp => usize::try_from(bytes).unwrap_or(usize::MAX),
        _ => HEAD_BYTES,
    };
    let head = read_head(reader, head_len)?;
    if sniff_mime(head.get(..64).unwrap_or(&head)) != Some(mime) {
        return Ok(None);
    }
    let Some(size) = read_image_size(&head, mime, reader)? else {
        return Ok(None);
    };
    if size.frames.is_some_and(|frames| frames > 1) {
        return Ok(None);
    }
    let orientation = match mime {
        Mime::Jpeg => jpeg_orientation(&head).unwrap_or(1),
        _ => 1,
    };
    let (width, height) = if (5..=8).contains(&orientation) {
        (size.height, size.width)
    } else {
        (size.width, size.height)
    };
    let (target_width, target_height) = target_size(width, height, box_px, drawn.density());
    Ok(Some(ImagePlan {
        path: path.to_owned(),
        sha256: plan.sha256,
        mime: plan.mime.as_str().to_owned(),
        bytes,
        width,
        height,
        orientation,
        target_width,
        target_height,
    }))
}

/// The files of `documents` a host may re-encode, in path order (see the module doc), each read from `store` (whose
/// ranges are ranges of `input`). A file that cannot be read is left out: it stays as it is.
pub fn plan_images(
    documents: &Documents,
    drawn: &DrawnSizes,
    store: &dyn MediaStore,
    input: &dyn RangeReader,
    max_pixels: u64,
) -> Vec<ImagePlan> {
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

/// Why a host's file is not taken for a planned image.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ImageRefusal {
    #[error("{0} is no image the deck may re-encode")]
    NotPlanned(String),
    #[error("the file is {0}, not WebP, JPEG or PNG")]
    Type(String),
    #[error("the file's size cannot be read")]
    Unsized,
    #[error("the file is animated")]
    Animated,
    #[error("the JPEG is turned by its EXIF orientation {0}: it must be stored as it shows")]
    Oriented(u16),
    #[error("the file is {width} × {height}, not the planned {expected_width} × {expected_height}")]
    Size {
        width: u32,
        height: u32,
        expected_width: u32,
        expected_height: u32,
    },
    #[error("the file has {bytes} bytes, no fewer than the original's {original}")]
    NotSmaller { bytes: u64, original: u64 },
}

/// A host's file for a planned image, taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompressedImage {
    /// The original's path.
    pub original: String,
    /// `media/<sha256>.<ext>` of its type.
    pub path: String,
    pub mime: &'static str,
    pub bytes: Arc<[u8]>,
}

impl CompressedImage {
    /// What the bundle writes in place of the original.
    pub fn replaced(&self) -> ReplacedMedia {
        ReplacedMedia {
            path: self.path.clone(),
            mime: self.mime.to_owned(),
            bytes: self.bytes.len() as u64,
        }
    }
}

/// Checks a host's file for `plan`: WebP, JPEG or PNG by its bytes, still, of the planned size, stored as it shows,
/// smaller than the original.
pub fn check_image(plan: &ImagePlan, bytes: Vec<u8>) -> Result<CompressedImage, ImageRefusal> {
    let mime = match sniff_mime(bytes.get(..64).unwrap_or(&bytes)) {
        Some(mime) => mime,
        None => return Err(ImageRefusal::Type("of no known type".to_owned())),
    };
    let Some(ext) = encoded_ext(mime) else {
        return Err(ImageRefusal::Type(mime.as_str().to_owned()));
    };
    let reader = BytesReader::new(&bytes[..]);
    let size = read_image_size(&bytes, mime, &reader)
        .ok()
        .flatten()
        .ok_or(ImageRefusal::Unsized)?;
    if size.frames.is_some_and(|frames| frames > 1) {
        return Err(ImageRefusal::Animated);
    }
    if mime == Mime::Jpeg
        && let Some(orientation) = jpeg_orientation(&bytes).filter(|&value| value != 1)
    {
        return Err(ImageRefusal::Oriented(orientation));
    }
    if (size.width, size.height) != (plan.target_width, plan.target_height) {
        return Err(ImageRefusal::Size {
            width: size.width,
            height: size.height,
            expected_width: plan.target_width,
            expected_height: plan.target_height,
        });
    }
    let len = bytes.len() as u64;
    if len >= plan.bytes {
        return Err(ImageRefusal::NotSmaller {
            bytes: len,
            original: plan.bytes,
        });
    }
    Ok(CompressedImage {
        original: plan.path.clone(),
        path: format!("media/{}.{ext}", plan.sha256),
        mime: mime.as_str(),
        bytes: Arc::from(bytes),
    })
}

/// A deck's media with the compressed images and videos in place: a path a compressed image names is served from its
/// bytes, one a compressed video names from its reader, every other from `media`.
pub struct CompressedMedia<'a> {
    pub media: &'a dyn DeckMedia,
    /// By original path.
    pub images: &'a BTreeMap<String, CompressedImage>,
    /// By original path.
    pub videos: &'a BTreeMap<String, CompressedVideo>,
}

impl std::fmt::Debug for CompressedMedia<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompressedMedia")
            .field("images", &self.images.len())
            .field("videos", &self.videos.len())
            .finish_non_exhaustive()
    }
}

/// The bundle's replacements for `images` (by original path): what [`CompressedMedia`] serves.
pub fn replacements(images: &BTreeMap<String, CompressedImage>) -> BTreeMap<String, ReplacedMedia> {
    images
        .iter()
        .map(|(original, image)| (original.clone(), image.replaced()))
        .collect()
}

impl DeckMedia for CompressedMedia<'_> {
    fn write_media(&self, path: &str, out: &mut dyn std::io::Write) -> Result<(), BundleError> {
        match self.images.values().find(|image| image.path == path) {
            Some(image) => out
                .write_all(&image.bytes)
                .map_err(|error| BundleError::Media {
                    path: path.to_owned(),
                    message: error.to_string(),
                }),
            None => match self.videos.values().find(|video| video.path == path) {
                Some(video) => video.write(out),
                None => self.media.write_media(path, out),
            },
        }
    }
}
