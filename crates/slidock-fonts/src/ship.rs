//! One face from font bytes to a shipped file: subset to what the deck draws (under neutral names when its licence
//! reserves its own, `rename`), encode as WOFF2, and describe it truthfully. Each step that fails steps down a rung,
//! with a notice: a face that cannot be subset (or renamed) ships whole (MID5418), a face that cannot be encoded ships
//! as its raw sfnt (MID5419). A face that cannot be read at all, or that browsers' font sanitizer would refuse (so they
//! would draw a fallback in its place), does not ship.

use slidock_ir::fonts::FaceStyle;

use crate::bind::{FaceDescriptor, WeightRange};
use crate::face_css::{FontDisplay, FontFaceRule, FontFormat};
use crate::glyphs::{ALWAYS_DRAWN, CodepointSet};
use crate::limits::FontLimits;
use crate::policy::{FontOrigin, FsType, LicenceEntry, stated_licence};
use crate::rename::shipped_names;
use crate::sfnt::{
    FaceDetails, SanitizerRefusal, SfntError, SfntFace, covered, face_file, read_details,
    read_sfnt, sanitizer_refusal,
};
use crate::subset::{SubsetError, SubsetOptions, subset_face};
use crate::woff2_out::{EncodeError, encode_woff2};

/// A face to ship.
#[derive(Clone, Copy, Debug)]
pub struct ShipInput<'a> {
    /// The CSS family its rule defines: the family the binding ships.
    pub family: &'a str,
    /// The font file or collection.
    pub bytes: &'a [u8],
    /// The face's index in a collection; 0 for a single font.
    pub index: u32,
    pub origin: FontOrigin,
    /// The code points the deck draws with the face.
    pub codepoints: &'a CodepointSet,
    /// The deck draws small caps with the face.
    pub small_caps: bool,
    /// The licence the source states (Google's catalog gives an SPDX id).
    pub source_licence: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShipOptions {
    pub display: FontDisplay,
    pub limits: FontLimits,
}

/// A rung the face stepped down, for the compile's diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontNotice {
    /// `fsType` forbids subsetting: the face ships whole.
    NoSubsetting,
    /// The subsetter failed: the face ships whole.
    SubsetFailed(SubsetError),
    /// The encoder failed: the face ships as its raw sfnt.
    EncodeFailed(EncodeError),
}

impl FontNotice {
    /// The diagnostic code: font-subset-failed (shipped whole) or font-encode-failed (shipped raw).
    pub const fn code(&self) -> &'static str {
        match self {
            FontNotice::NoSubsetting | FontNotice::SubsetFailed(_) => "MID5418",
            FontNotice::EncodeFailed(_) => "MID5419",
        }
    }
}

/// A face that cannot ship (MID5419, not shipped): its substitute must draw instead.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ShipError {
    #[error("The face cannot be read: {0}")]
    Unreadable(#[from] SfntError),
    /// Browsers would refuse the file and draw a fallback.
    #[error("Browsers would refuse the face: {0}")]
    Refused(#[from] SanitizerRefusal),
}

impl ShipError {
    pub const fn code(&self) -> &'static str {
        "MID5419"
    }
}

/// A shipped face.
#[derive(Clone, Debug, PartialEq)]
pub struct ShippedFont {
    pub rule: FontFaceRule,
    pub licence: LicenceEntry,
    /// The face was subset (not shipped whole).
    pub subset: bool,
    pub notices: Vec<FontNotice>,
    /// The face read from the shipped file: its names, weight, style and metrics.
    pub face: SfntFace,
}

/// The face's own descriptor: its `wght` axis range (clamped to the CSS 1–1000) for a variable face, else its
/// `usWeightClass` as the binder reads it; italic when the face says so.
pub fn true_descriptor(face: &SfntFace, details: &FaceDetails) -> FaceDescriptor {
    let weight = match details.weight_axis {
        Some(axis) => {
            let clamp = |value: f64| (value + 0.5).floor().clamp(1.0, 1000.0) as u32;
            WeightRange::between(clamp(axis.min), clamp(axis.max))
        }
        None => WeightRange::point(face.weight),
    };
    FaceDescriptor {
        weight,
        style: if face.italic {
            FaceStyle::Italic
        } else {
            FaceStyle::Normal
        },
    }
}

/// Subsets, encodes and describes one face.
pub fn ship_face(input: &ShipInput<'_>, options: &ShipOptions) -> Result<ShippedFont, ShipError> {
    let limits = &options.limits;
    let details = read_details(input.bytes, input.index)?;
    if let Some(refusal) = sanitizer_refusal(input.bytes, input.index) {
        return Err(ShipError::Refused(refusal));
    }
    let whole = face_file(input.bytes, input.index, limits)?;
    let mut notices = Vec::new();

    let subset = if FsType(details.fs_type).no_subsetting() {
        notices.push(FontNotice::NoSubsetting);
        None
    } else {
        let subset_options = SubsetOptions {
            small_caps: input.small_caps,
            limits: *limits,
        };
        let licence = stated_licence(&details, input.source_licence);
        match subset_face(input.bytes, input.index, input.codepoints, &subset_options)
            .and_then(|subset| shipped_names(subset, &details, licence.as_deref()))
        {
            Ok(subset) => Some(subset),
            Err(error) => {
                notices.push(FontNotice::SubsetFailed(error));
                None
            }
        }
    };
    let is_subset = subset.is_some();
    let sfnt: Vec<u8> = subset.unwrap_or_else(|| whole.into_owned());
    let face = read_sfnt(&sfnt)?
        .into_iter()
        .next()
        .ok_or(SfntError::FaceIndex(input.index))?;

    let (format, bytes) = match encode_woff2(&sfnt, limits) {
        Ok(woff2) => (FontFormat::Woff2, woff2),
        Err(error) => {
            notices.push(FontNotice::EncodeFailed(error));
            (FontFormat::raw(details.outlines), sfnt.clone())
        }
    };

    let mut wanted = input.codepoints.clone();
    for code_point in ALWAYS_DRAWN {
        wanted.insert(code_point);
    }
    if wanted.len() > limits.codepoints {
        wanted = CodepointSet::new();
    }
    let coverage = covered(&sfnt, 0, &wanted)?;
    let descriptor = true_descriptor(&face, &details);
    Ok(ShippedFont {
        rule: FontFaceRule {
            family: input.family.to_owned(),
            descriptor,
            unicode_range: (!coverage.is_empty()).then_some(coverage),
            format,
            display: options.display,
            bytes,
        },
        licence: LicenceEntry::new(
            input.family,
            descriptor,
            input.origin,
            &details,
            input.source_licence,
        ),
        subset: is_subset,
        notices,
        face,
    })
}
