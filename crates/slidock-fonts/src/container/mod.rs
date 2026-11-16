//! Font containers: sniffing a file's format and unwrapping WOFF, WOFF2, EOT (with MicroType Express) and `.odttf`
//! into a plain TrueType/OpenType font (or collection), under [`ContainerLimits`].
//!
//! Every input is untrusted. Each decoder reads through bounds-checked cursors, allocates only what the limits
//! allow, and returns a [`ContainerError`] for anything it cannot use.

mod assemble;
mod cursor;
pub mod eot;
mod glyf;
pub mod mtx;
pub mod odttf;
pub mod triplet;
pub mod woff;
pub mod woff2;

use std::fmt;

use crate::sfnt::TableTag;

pub use eot::{EotError, EotHeader};
pub use mtx::{MtxError, MtxStream};
pub use odttf::OdttfError;
pub use woff::WoffError;
pub use woff2::Woff2Error;

const MIB: u64 = 1024 * 1024;

/// The sfnt versions of a single TrueType or OpenType face: 1.0, `OTTO` (CFF) and `true` (Apple).
const SFNT_VERSIONS: [[u8; 4]; 3] = [[0, 1, 0, 0], *b"OTTO", *b"true"];

/// The content type OOXML gives an obfuscated font part.
const ODTTF_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.obfuscatedFont";

/// Budgets for one unwrap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerLimits {
    /// The decoded font, and every intermediate buffer.
    pub max_output_bytes: u64,
    /// Decompressed bytes per input byte, over the whole file.
    pub max_expansion: u64,
    /// Tables of one face, or table entries of a WOFF2 collection.
    pub max_tables: u16,
    /// Glyphs of a transformed `glyf` table.
    pub max_glyphs: u16,
    /// Faces of a collection.
    pub max_faces: u32,
}

impl Default for ContainerLimits {
    fn default() -> Self {
        ContainerLimits {
            max_output_bytes: 64 * MIB,
            max_expansion: 40,
            max_tables: 512,
            max_glyphs: u16::MAX,
            max_faces: 256,
        }
    }
}

/// What the file around the bytes says about them: an OOXML part's content type and name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hint<'a> {
    pub content_type: Option<&'a str>,
    /// The part or file name, such as `word/fonts/{0A1B…}.odttf`.
    pub part_name: Option<&'a str>,
}

impl Hint<'_> {
    fn is_odttf(&self) -> bool {
        self.content_type
            .is_some_and(|content_type| content_type.eq_ignore_ascii_case(ODTTF_CONTENT_TYPE))
            || self.part_name.is_some_and(|name| {
                name.rsplit_once('.')
                    .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("odttf"))
            })
    }
}

/// A font file's container format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// One TrueType or OpenType face.
    Sfnt,
    /// A TrueType/OpenType collection (`ttcf`).
    Collection,
    Woff,
    Woff2,
    /// Embedded OpenType.
    Eot,
    /// An OOXML font whose first 32 bytes are XORed with a GUID.
    Odttf,
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Format::Sfnt => "TrueType/OpenType",
            Format::Collection => "font collection",
            Format::Woff => "WOFF",
            Format::Woff2 => "WOFF2",
            Format::Eot => "EOT",
            Format::Odttf => "obfuscated OOXML font",
        })
    }
}

/// The container format of `bytes`, by its magic numbers. An `.odttf` part has none, so its hint decides: it is
/// [`Format::Odttf`] when nothing else matches and the hint names an obfuscated font.
pub fn sniff(bytes: &[u8], hint: &Hint<'_>) -> Option<Format> {
    let magic = bytes.first_chunk::<4>();
    let format = match magic {
        Some(magic) if SFNT_VERSIONS.contains(magic) => Some(Format::Sfnt),
        Some(b"ttcf") => Some(Format::Collection),
        Some(b"wOFF") => Some(Format::Woff),
        Some(b"wOF2") => Some(Format::Woff2),
        _ if bytes.get(34..36) == Some(&eot::MAGIC.to_le_bytes()) => Some(Format::Eot),
        _ => None,
    };
    format.or_else(|| hint.is_odttf().then_some(Format::Odttf))
}

/// What an EOT header says about its face (the font's own tables may disagree).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderNames {
    pub family: String,
    pub style: String,
    pub version: String,
    pub full_name: String,
    /// The header's weight, as `usWeightClass`.
    pub weight: u32,
    pub italic: bool,
}

/// Something a decoder accepted although the file is not quite right.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContainerWarning {
    /// A WOFF table whose `origChecksum` is not the checksum of its data.
    ChecksumMismatch(TableTag),
    /// An EOT whose header strings end elsewhere than where its font data starts; the font data is taken from the
    /// end of the file.
    EotHeaderEnd { header_end: u64, data_start: u64 },
    /// A MicroType Express `hdmx` or `VDMX` table that does not decode; the font is returned without it.
    DroppedTable(TableTag),
}

/// An unwrapped font.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unwrapped {
    /// A TrueType/OpenType font or collection. Decoded containers are rebuilt with table records in tag order, data
    /// in the container's order, 4-byte padding, fresh table checksums and `head.checkSumAdjustment`; a plain sfnt
    /// is returned as it came.
    pub sfnt: Vec<u8>,
    /// The container it came in.
    pub kind: Format,
    /// An EOT made of only the glyphs its document uses (`TTEMBED_SUBSET`).
    pub subset: bool,
    /// The EOT header's `fsType`.
    pub fs_type: Option<u16>,
    pub header_names: Option<HeaderNames>,
    pub warnings: Vec<ContainerWarning>,
}

/// The structure a read ran past the end of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Header,
    TableDirectory,
    CollectionDirectory,
    Table(TableTag),
    GlyfStream(GlyfStream),
    HmtxStream,
    EotString,
    Mtx(MtxStream),
}

/// The seven streams of a transformed `glyf` table, plus its bitmaps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyfStream {
    Header,
    Contours,
    Points,
    Flags,
    Glyphs,
    Composites,
    Bboxes,
    Instructions,
    OverlapBitmap,
}

impl fmt::Display for Part {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Part::Header => f.write_str("header"),
            Part::TableDirectory => f.write_str("table directory"),
            Part::CollectionDirectory => f.write_str("collection directory"),
            Part::Table(tag) => write!(f, "{tag} table"),
            Part::GlyfStream(stream) => write!(f, "glyf {stream:?} stream"),
            Part::HmtxStream => f.write_str("transformed hmtx table"),
            Part::EotString => f.write_str("EOT name string"),
            Part::Mtx(stream) => write!(f, "MicroType Express {stream}"),
        }
    }
}

/// Why a font file can't be unwrapped.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ContainerError {
    #[error("Not a font format slidock reads")]
    Unknown,
    #[error("The {0} is truncated")]
    Truncated(Part),
    #[error("The decoded font would pass {limit} bytes")]
    TooLarge { limit: u64 },
    #[error("The font expands more than {ratio}:1")]
    Expansion { ratio: u64 },
    #[error("The font declares {count} tables (at most {limit})")]
    TableCount { count: u32, limit: u16 },
    #[error("The collection declares {count} faces (at most {limit})")]
    FaceCount { count: u32, limit: u32 },
    #[error("The font declares {count} glyphs (at most {limit})")]
    GlyphCount { count: u32, limit: u16 },
    #[error("The font lists the {0} table twice")]
    DuplicateTable(TableTag),
    #[error("The {0} table is too short")]
    ShortTable(TableTag),
    #[error("A glyph coordinate, point count or instruction length is out of range")]
    GlyphCoordinate,
    #[error("The glyph offsets do not fit the loca format")]
    LocaOverflow,
    #[error("The {0} payload is not a TrueType or OpenType font")]
    NotSfnt(Format),
    #[error(transparent)]
    Eot(#[from] EotError),
    #[error(transparent)]
    Mtx(#[from] MtxError),
    #[error(transparent)]
    Odttf(#[from] OdttfError),
    #[error(transparent)]
    Woff(#[from] WoffError),
    #[error(transparent)]
    Woff2(#[from] Woff2Error),
}

/// `bytes` unwrapped into an sfnt under the default limits.
pub fn unwrap(bytes: &[u8], hint: &Hint<'_>) -> Result<Unwrapped, ContainerError> {
    unwrap_with(bytes, hint, &ContainerLimits::default())
}

/// `bytes` unwrapped into an sfnt, whatever container [`sniff`] finds.
pub fn unwrap_with(
    bytes: &[u8],
    hint: &Hint<'_>,
    limits: &ContainerLimits,
) -> Result<Unwrapped, ContainerError> {
    let kind = sniff(bytes, hint).ok_or(ContainerError::Unknown)?;
    let plain = |sfnt: Vec<u8>, warnings| Unwrapped {
        sfnt,
        kind,
        subset: false,
        fs_type: None,
        header_names: None,
        warnings,
    };
    match kind {
        Format::Sfnt | Format::Collection => {
            check_output(bytes.len() as u64, limits)?;
            Ok(plain(bytes.to_vec(), Vec::new()))
        }
        Format::Woff => {
            let font = woff::decode(bytes, limits)?;
            Ok(plain(font.sfnt, font.warnings))
        }
        Format::Woff2 => Ok(plain(woff2::decode(bytes, limits)?, Vec::new())),
        Format::Odttf => {
            check_output(bytes.len() as u64, limits)?;
            let key = hint
                .part_name
                .and_then(odttf::key_from_part_name)
                .ok_or(OdttfError::NoKey)?;
            let mut sfnt = bytes.to_vec();
            odttf::deobfuscate(&mut sfnt, &key)?;
            Ok(plain(sfnt, Vec::new()))
        }
        Format::Eot => {
            let font = eot::decode(bytes, limits)?;
            Ok(Unwrapped {
                sfnt: font.sfnt,
                kind,
                subset: font.header.subset(),
                fs_type: Some(font.header.fs_type),
                header_names: Some(font.header.names()),
                warnings: font.warnings,
            })
        }
    }
}

/// Fails when `length` output bytes pass the limit.
fn check_output(length: u64, limits: &ContainerLimits) -> Result<(), ContainerError> {
    if length > limits.max_output_bytes {
        return Err(ContainerError::TooLarge {
            limit: limits.max_output_bytes,
        });
    }
    Ok(())
}

/// Fails when `decoded` bytes from an `input`-byte file pass the expansion or output limit.
fn check_expansion(
    decoded: u64,
    input: usize,
    limits: &ContainerLimits,
) -> Result<(), ContainerError> {
    if decoded > limits.max_output_bytes {
        return Err(ContainerError::TooLarge {
            limit: limits.max_output_bytes,
        });
    }
    if decoded > (input as u64).saturating_mul(limits.max_expansion) {
        return Err(ContainerError::Expansion {
            ratio: limits.max_expansion,
        });
    }
    Ok(())
}

/// Whether `flavor` is the sfnt version of one TrueType/OpenType face.
fn is_sfnt_version(flavor: u32) -> bool {
    SFNT_VERSIONS.contains(&flavor.to_be_bytes())
}

/// Whether `bytes` start like one TrueType/OpenType face.
fn is_single_sfnt(bytes: &[u8]) -> bool {
    bytes
        .first_chunk::<4>()
        .is_some_and(|magic| SFNT_VERSIONS.contains(magic))
}
