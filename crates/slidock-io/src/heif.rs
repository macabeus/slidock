//! A bounded HEIF reader (ISO/IEC 23008-12) for HEIC photos a host decodes: it reads the file's `meta` box and plans
//! the decode of its primary image, which is one `hvc1` item or a `grid` of `hvc1` tiles. It decodes no pixel: the
//! plan names each tile's HEVC configuration (the `hvcC` record, with its RFC 6381 codec string from ISO/IEC
//! 14496-15 Annex E), the byte ranges of its coded data in the file, its place in the grid, the image's transforms in
//! the order its properties list them (clean aperture, rotation, mirror) and its colour (`nclx`, ICC, and what each
//! configuration's SPS states: [`crate::hevc`]).
//!
//! The reader walks the fixed shape of the boxes it reads (`meta` → `iinf`/`iloc`/`iref`/`iprp`/`idat`, `iprp` →
//! `ipco`/`ipma`), never a box tree of the file's choosing, so the box depth is at most four. Every count is checked
//! against [`HeifLimits`] before anything is allocated for it, and every read against its box. Anything outside the
//! subset (another codec, an overlay or identity item, an alpha plane, an essential property it does not know, data
//! in another file or item) is refused with [`HeifError::Unsupported`], naming the reason.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::hevc::{SPS_NAL_TYPE, VideoSignal, sps_video_signal};
use crate::range::{RangeError, RangeReader, check_range};

/// What the reader accepts, besides the subset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeifLimits {
    /// The `meta` box (its item and property tables, and the `idat` data they hold).
    pub meta_bytes: u64,
    /// Items in `iinf` and `iloc`, properties in `ipco`, entries in `ipma` and references in `iref`.
    pub items: u32,
    /// Tiles of a grid.
    pub tiles: u32,
    /// Distinct HEVC configurations of an image's tiles: each one is a decoder a host configures in turn.
    pub configs: u32,
    /// Pixels decoded: the tiles' (a grid's tiles may overhang its output), and the output's.
    pub pixels: u64,
}

impl HeifLimits {
    /// The default tables (4 MiB of `meta`, 4096 items, 1024 tiles of at most 16 configurations) with this pixel
    /// limit.
    pub const fn with_pixels(pixels: u64) -> Self {
        HeifLimits {
            meta_bytes: 4 * 1024 * 1024,
            items: 4096,
            tiles: 1024,
            configs: 16,
            pixels,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum HeifError {
    /// The bytes break the format.
    #[error("the HEIF file is malformed: {0}")]
    Malformed(String),
    /// Valid HEIF outside the subset the reader plans.
    #[error("{0}")]
    Unsupported(String),
    #[error("the HEIF image has {pixels} pixels, more than the {limit} allowed")]
    TooLarge { pixels: u64, limit: u64 },
    #[error(transparent)]
    Read(#[from] RangeError),
}

fn malformed<T>(message: impl Into<String>) -> Result<T, HeifError> {
    Err(HeifError::Malformed(message.into()))
}

fn unsupported<T>(message: impl Into<String>) -> Result<T, HeifError> {
    Err(HeifError::Unsupported(message.into()))
}

/// A byte range of the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    pub offset: u64,
    pub len: u64,
}

/// An HEVC decoder configuration: the `hvcC` record as WebCodecs takes it (`description`) and what it says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HevcConfig {
    /// The RFC 6381 codec string (`hvc1.1.6.L93.B0`).
    pub codec: String,
    /// The `hvcC` box's payload: the `HEVCDecoderConfigurationRecord`.
    pub description: Vec<u8>,
    /// `chroma_format_idc`: 0 grey, 1 4:2:0, 2 4:2:2, 3 4:4:4.
    pub chroma_format: u8,
    pub bit_depth_luma: u8,
    pub bit_depth_chroma: u8,
    /// What the record's first SPS states of its colour (its VUI's video signal type).
    pub signal: VideoSignal,
}

/// One coded picture of the image, at its place in the coded output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeifTile {
    /// Into [`HeifImage::configs`].
    pub config: usize,
    /// Its left and top edge in the coded image.
    pub x: u32,
    pub y: u32,
    /// The coded data (length-prefixed NAL units), in order: concatenated, one sample.
    pub extents: Vec<Extent>,
}

/// A rectangle of the image as it stands when the transform applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// A transformative property, applied in the order [`HeifImage::transforms`] lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transform {
    /// `clap`: keep this rectangle.
    Crop(Crop),
    /// `irot`: turn anticlockwise by this many quarter turns (1–3).
    Rotate(u8),
    /// `imir`: mirror about the vertical axis (left and right swap: its axis field 1) or the horizontal one (top and
    /// bottom swap: 0).
    Mirror(MirrorAxis),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MirrorAxis {
    Vertical,
    Horizontal,
}

/// A `colr` box of type `nclx` (ISO/IEC 23091-2 code points).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Nclx {
    pub primaries: u16,
    pub transfer: u16,
    pub matrix: u16,
    pub full_range: bool,
}

/// What an ICC profile's colourants and tone curve match, as ISO/IEC 23091-2 code points: primaries 1 (sRGB and
/// BT.709), 12 (Display P3) or 9 (BT.2020); transfer 13 (the sRGB curve) or 1 (BT.709's).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IccColour {
    pub primaries: Option<u16>,
    pub transfer: Option<u16>,
}

/// The red, green and blue colourants (`rXYZ`, `gXYZ`, `bXYZ`, D50) of the primaries [`IccColour`] names.
const ICC_PRIMARIES: [(u16, [[f64; 3]; 3]); 3] = [
    (
        1,
        [
            [0.4361, 0.2225, 0.0139],
            [0.3851, 0.7169, 0.0971],
            [0.1431, 0.0606, 0.7141],
        ],
    ),
    (
        12,
        [
            [0.5151, 0.2412, -0.0011],
            [0.2920, 0.6922, 0.0419],
            [0.1571, 0.0666, 0.7841],
        ],
    ),
    (
        9,
        [
            [0.6735, 0.2790, -0.0019],
            [0.1657, 0.6753, 0.0300],
            [0.1250, 0.0456, 0.7968],
        ],
    ),
];

/// The parametric curves (ICC `para` type 3: g, a, b, c, d) of the transfers [`IccColour`] names.
const ICC_CURVES: [(u16, [f64; 5]); 2] = [
    (13, [2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045]),
    (
        1,
        [1.0 / 0.45, 1.0 / 1.099, 0.099 / 1.099, 1.0 / 4.5, 0.081],
    ),
];

/// The sRGB curve at `x` in 0–1.
fn srgb_curve(x: f64) -> f64 {
    if x <= 0.04045 {
        x / 12.92
    } else {
        slidock_math::exp(2.4 * slidock_math::log((x + 0.055) / 1.055))
    }
}

/// What `icc`'s colourants and red tone curve match ([`IccColour`]); unknown or unread values are `None`.
pub fn icc_colour(icc: &[u8]) -> IccColour {
    let mut tags = BTreeMap::new();
    let mut cursor = Cursor::new(icc, "ICC");
    if cursor.take(128).is_err() {
        return IccColour::default();
    }
    let count = cursor.u32().unwrap_or(0).min(256);
    for _ in 0..count {
        let (Ok(signature), Ok(offset), Ok(size)) =
            (cursor.array::<4>(), cursor.u32(), cursor.u32())
        else {
            break;
        };
        let end = (offset as usize).checked_add(size as usize);
        if let Some(data) = end.and_then(|end| icc.get(offset as usize..end)) {
            tags.entry(signature).or_insert(data);
        }
    }
    let fixed = |bytes: &[u8]| -> Option<f64> {
        let raw = i32::from_be_bytes(bytes.try_into().ok()?);
        Some(f64::from(raw) / 65_536.0)
    };
    let xyz = |signature: &[u8; 4]| -> Option<[f64; 3]> {
        let data = tags.get(signature)?;
        if data.get(..4)? != b"XYZ " {
            return None;
        }
        Some([
            fixed(data.get(8..12)?)?,
            fixed(data.get(12..16)?)?,
            fixed(data.get(16..20)?)?,
        ])
    };
    let colourants = [xyz(b"rXYZ"), xyz(b"gXYZ"), xyz(b"bXYZ")];
    let primaries = ICC_PRIMARIES.iter().find_map(|(code, expected)| {
        let matches = colourants.iter().zip(expected).all(|(found, expected)| {
            found.is_some_and(|found| {
                found
                    .iter()
                    .zip(expected)
                    .all(|(a, b)| (a - b).abs() < 0.002)
            })
        });
        matches.then_some(*code)
    });
    let transfer = tags.get(b"rTRC").and_then(|curve| match curve.get(..4)? {
        b"para" => {
            if curve.get(8..10)? != [0, 3] {
                return None;
            }
            let params: Option<Vec<f64>> = (0..5)
                .map(|index| fixed(curve.get(12 + 4 * index..16 + 4 * index)?))
                .collect();
            let params = params?;
            ICC_CURVES.iter().find_map(|(code, expected)| {
                let matches = params
                    .iter()
                    .zip(expected)
                    .all(|(a, b)| (a - b).abs() < 0.001);
                matches.then_some(*code)
            })
        }
        b"curv" => {
            // A sampled curve is the sRGB curve when 16 of its samples are, within 0.1 % (gamma 2.2 is 0.3 % off).
            let entries = u32::from_be_bytes(curve.get(8..12)?.try_into().ok()?) as usize;
            if entries < 16 {
                return None;
            }
            let sample = |index: usize| -> Option<f64> {
                let at = 12 + 2 * index;
                Some(
                    f64::from(u16::from_be_bytes(curve.get(at..at + 2)?.try_into().ok()?))
                        / 65_535.0,
                )
            };
            let step = (entries / 16).max(1);
            (0..entries)
                .step_by(step)
                .all(|index| {
                    let x = index as f64 / (entries - 1) as f64;
                    sample(index).is_some_and(|y| (y - srgb_curve(x)).abs() < 0.001)
                })
                .then_some(13)
        }
        _ => None,
    });
    IccColour {
        primaries,
        transfer,
    }
}

/// The decode plan of a HEIF file's primary image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeifImage {
    /// The coded image's size (a grid's output, a single item's `ispe`), before the transforms.
    pub coded_width: u32,
    pub coded_height: u32,
    /// Every tile's size (`ispe`); a single item is one tile of the image's size.
    pub tile_width: u32,
    pub tile_height: u32,
    pub columns: u32,
    pub rows: u32,
    /// The configurations the tiles use, each once, in first-use order.
    pub configs: Vec<HevcConfig>,
    /// Row-major.
    pub tiles: Vec<HeifTile>,
    pub transforms: Vec<Transform>,
    pub nclx: Option<Nclx>,
    /// An ICC profile (`colr` of type `prof` or `rICC`).
    pub icc: Option<Vec<u8>>,
    /// Bits per channel (`pixi`), when stated.
    pub bits: Option<Vec<u8>>,
}

impl HeifImage {
    /// The image's size once every transform applies: what a decoder of this plan outputs.
    pub fn size(&self) -> (u32, u32) {
        let mut size = (self.coded_width, self.coded_height);
        for transform in &self.transforms {
            match transform {
                Transform::Crop(crop) => size = (crop.width, crop.height),
                Transform::Rotate(turns) if turns % 2 == 1 => size = (size.1, size.0),
                Transform::Rotate(_) | Transform::Mirror(_) => {}
            }
        }
        size
    }
}

/// Reads big-endian fields from a slice, every read checked.
#[derive(Clone, Copy)]
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
    /// What is being read, for errors.
    what: &'static str,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8], what: &'static str) -> Self {
        Cursor { bytes, at: 0, what }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], HeifError> {
        let end = self.at.checked_add(len);
        match end.and_then(|end| self.bytes.get(self.at..end)) {
            Some(slice) => {
                self.at += len;
                Ok(slice)
            }
            None => malformed(format!("the `{}` box ends early", self.what)),
        }
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], HeifError> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, HeifError> {
        Ok(u8::from_be_bytes(self.array()?))
    }

    fn u16(&mut self) -> Result<u16, HeifError> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, HeifError> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, HeifError> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    /// An unsigned field of 0, 4 or 8 bytes (`iloc`'s sizes).
    fn sized(&mut self, size: u8) -> Result<u64, HeifError> {
        match size {
            0 => Ok(0),
            4 => self.u32().map(u64::from),
            8 => self.u64(),
            other => malformed(format!("`iloc` uses a field size of {other} bytes")),
        }
    }

    /// The version and flags of a full box.
    fn full(&mut self) -> Result<(u8, u32), HeifError> {
        let word = self.u32()?;
        Ok(((word >> 24) as u8, word & 0x00ff_ffff))
    }

    fn rest(&self) -> &'a [u8] {
        self.bytes.get(self.at..).unwrap_or_default()
    }

    fn is_empty(&self) -> bool {
        self.at >= self.bytes.len()
    }
}

/// A box inside a payload: its type, its payload and where that payload starts in the file.
#[derive(Clone, Copy)]
struct Child<'a> {
    kind: [u8; 4],
    payload: &'a [u8],
    offset: u64,
}

/// The boxes of a payload that starts at `offset` in the file, in order: at most `max` of them.
fn children<'a>(
    payload: &'a [u8],
    offset: u64,
    what: &'static str,
    max: u32,
) -> Result<Vec<Child<'a>>, HeifError> {
    let mut out = Vec::new();
    let mut cursor = Cursor::new(payload, what);
    while !cursor.is_empty() {
        if out.len() as u64 >= u64::from(max) {
            return unsupported(format!("its `{what}` holds more than {max} boxes"));
        }
        let start = cursor.at;
        let size = cursor.u32()?;
        let kind = cursor.array::<4>()?;
        let size = match size {
            0 => (payload.len() - start) as u64,
            1 => cursor.u64()?,
            size => u64::from(size),
        };
        let header = cursor.at - start;
        let Some(len) = size
            .checked_sub(header as u64)
            .and_then(|len| usize::try_from(len).ok())
        else {
            return malformed(format!("a box in `{what}` is shorter than its header"));
        };
        let at = cursor.at;
        let body = cursor.take(len)?;
        out.push(Child {
            kind,
            payload: body,
            offset: offset + at as u64,
        });
    }
    Ok(out)
}

fn fourcc(kind: &[u8; 4]) -> String {
    kind.iter()
        .map(|&b| {
            if b.is_ascii_graphic() || b == b' ' {
                char::from(b)
            } else {
                '?'
            }
        })
        .collect()
}

/// At most this many boxes sit directly in `meta` or `iprp`.
const MAX_META_BOXES: u32 = 256;

/// At most this many top-level boxes are walked to find `meta`.
const MAX_TOP_BOXES: u32 = 64;

/// The `meta` box's payload and where it starts in the file.
fn read_meta<R: RangeReader + ?Sized>(
    source: &R,
    limits: &HeifLimits,
) -> Result<(Vec<u8>, u64), HeifError> {
    let size = source.len();
    let mut at = 0u64;
    for index in 0..MAX_TOP_BOXES {
        if at >= size {
            break;
        }
        let head_len = (size - at).min(16);
        let head = source.read_at(at, head_len as usize)?;
        let mut cursor = Cursor::new(&head, "file");
        let declared = cursor.u32()?;
        let kind = cursor.array::<4>()?;
        if index == 0 && &kind != b"ftyp" {
            return malformed("the file does not start with an `ftyp` box");
        }
        let total = match declared {
            0 => size - at,
            1 => cursor.u64()?,
            declared => u64::from(declared),
        };
        let header = cursor.at as u64;
        if total < header {
            return malformed("a top-level box is shorter than its header");
        }
        check_range(size, at, total)?;
        if &kind == b"meta" {
            let len = total - header;
            if len > limits.meta_bytes {
                return unsupported(format!(
                    "its `meta` box has {len} bytes, more than the {} read",
                    limits.meta_bytes
                ));
            }
            let start = at + header;
            let bytes = source.read_at(start, len as usize)?.into_owned();
            return Ok((bytes, start));
        }
        at += total;
    }
    malformed("the file has no `meta` box")
}

/// An item's location (`iloc`).
#[derive(Clone, Debug)]
struct Location {
    construction: u8,
    data_reference: u16,
    base: u64,
    extents: Vec<(u64, u64)>,
}

/// An item's entry (`infe`, version 2 or 3).
#[derive(Clone, Copy, Debug)]
struct ItemInfo {
    kind: [u8; 4],
    protected: bool,
}

/// A property of `ipco`, as far as the reader reads it.
#[derive(Clone, Debug)]
enum Property<'a> {
    Hvcc(&'a [u8]),
    Ispe(u32, u32),
    Irot(u8),
    Imir(MirrorAxis),
    Clap([u32; 8]),
    Nclx(Nclx),
    Icc(&'a [u8]),
    Pixi(Vec<u8>),
    /// `auxC`: an auxiliary image's type URN.
    AuxC(&'a [u8]),
    /// One the reader does not read.
    Other([u8; 4]),
}

fn property<'a>(child: &Child<'a>) -> Result<Property<'a>, HeifError> {
    let mut cursor = Cursor::new(child.payload, "ipco");
    Ok(match &child.kind {
        b"hvcC" => Property::Hvcc(child.payload),
        b"ispe" => {
            cursor.full()?;
            Property::Ispe(cursor.u32()?, cursor.u32()?)
        }
        b"irot" => Property::Irot(cursor.u8()? & 3),
        // Axis 0 swaps top and bottom, 1 left and right, as Apple's ImageIO and libheif read it.
        b"imir" => Property::Imir(if cursor.u8()? & 1 == 0 {
            MirrorAxis::Horizontal
        } else {
            MirrorAxis::Vertical
        }),
        b"clap" => {
            let mut fields = [0; 8];
            for field in &mut fields {
                *field = cursor.u32()?;
            }
            Property::Clap(fields)
        }
        b"colr" => match &cursor.array::<4>()? {
            b"nclx" => {
                let primaries = cursor.u16()?;
                let transfer = cursor.u16()?;
                let matrix = cursor.u16()?;
                let full_range = cursor.u8()? & 0x80 != 0;
                Property::Nclx(Nclx {
                    primaries,
                    transfer,
                    matrix,
                    full_range,
                })
            }
            b"prof" | b"rICC" => Property::Icc(cursor.rest()),
            _ => Property::Other(child.kind),
        },
        b"pixi" => {
            cursor.full()?;
            let channels = cursor.u8()?;
            Property::Pixi(cursor.take(usize::from(channels))?.to_vec())
        }
        b"auxC" => {
            cursor.full()?;
            let rest = cursor.rest();
            let urn = rest.split(|&b| b == 0).next().unwrap_or_default();
            Property::AuxC(urn)
        }
        _ => Property::Other(child.kind),
    })
}

/// The `auxC` types of an alpha plane (HEIF and the HEVC auxiliary id 1).
const ALPHA_URNS: [&[u8]; 2] = [
    b"urn:mpeg:mpegB:cicp:systems:auxiliary:alpha",
    b"urn:mpeg:hevc:2015:auxid:1",
];

/// The tables of the `meta` box.
struct Meta<'a> {
    primary: u32,
    items: BTreeMap<u32, ItemInfo>,
    locations: BTreeMap<u32, Location>,
    /// `(type, from) → to`, in order.
    references: Vec<([u8; 4], u32, Vec<u32>)>,
    properties: Vec<Property<'a>>,
    /// Item → `(essential, index into properties)`, in order.
    associations: BTreeMap<u32, Vec<(bool, usize)>>,
    /// The `idat` payload and where it starts in the file.
    idat: Option<(&'a [u8], u64)>,
}

fn check_count(count: u64, limits: &HeifLimits, what: &str) -> Result<(), HeifError> {
    if count > u64::from(limits.items) {
        return unsupported(format!(
            "its `{what}` lists {count} entries, more than the {} read",
            limits.items
        ));
    }
    Ok(())
}

fn parse_iinf(payload: &[u8], limits: &HeifLimits) -> Result<BTreeMap<u32, ItemInfo>, HeifError> {
    let mut cursor = Cursor::new(payload, "iinf");
    let (version, _) = cursor.full()?;
    let count = if version == 0 {
        u32::from(cursor.u16()?)
    } else {
        cursor.u32()?
    };
    check_count(u64::from(count), limits, "iinf")?;
    let entries = children(cursor.rest(), 0, "iinf", limits.items)?;
    let mut items = BTreeMap::new();
    for entry in entries.iter().filter(|entry| &entry.kind == b"infe") {
        let mut cursor = Cursor::new(entry.payload, "infe");
        let (version, _) = cursor.full()?;
        if version < 2 {
            // An entry of the first versions names no item type: it is no image.
            continue;
        }
        let id = if version == 2 {
            u32::from(cursor.u16()?)
        } else {
            cursor.u32()?
        };
        let protection = cursor.u16()?;
        let kind = cursor.array::<4>()?;
        if items
            .insert(
                id,
                ItemInfo {
                    kind,
                    protected: protection != 0,
                },
            )
            .is_some()
        {
            return malformed(format!("item {id} has two `infe` entries"));
        }
    }
    Ok(items)
}

fn parse_iloc(payload: &[u8], limits: &HeifLimits) -> Result<BTreeMap<u32, Location>, HeifError> {
    let mut cursor = Cursor::new(payload, "iloc");
    let (version, _) = cursor.full()?;
    if version > 2 {
        return unsupported(format!("its `iloc` is version {version}"));
    }
    let sizes = cursor.u8()?;
    let (offset_size, length_size) = (sizes >> 4, sizes & 15);
    let sizes = cursor.u8()?;
    let base_size = sizes >> 4;
    let index_size = if version == 0 { 0 } else { sizes & 15 };
    let count = if version < 2 {
        u32::from(cursor.u16()?)
    } else {
        cursor.u32()?
    };
    check_count(u64::from(count), limits, "iloc")?;
    let mut locations = BTreeMap::new();
    let mut extents_read = 0u64;
    for _ in 0..count {
        let id = if version < 2 {
            u32::from(cursor.u16()?)
        } else {
            cursor.u32()?
        };
        let construction = if version == 0 {
            0
        } else {
            (cursor.u16()? & 15) as u8
        };
        let data_reference = cursor.u16()?;
        let base = cursor.sized(base_size)?;
        let extent_count = cursor.u16()?;
        extents_read += u64::from(extent_count);
        check_count(extents_read, limits, "iloc")?;
        let mut extents = Vec::with_capacity(usize::from(extent_count));
        for _ in 0..extent_count {
            if index_size > 0 {
                cursor.sized(index_size)?;
            }
            let offset = cursor.sized(offset_size)?;
            let len = cursor.sized(length_size)?;
            extents.push((offset, len));
        }
        let location = Location {
            construction,
            data_reference,
            base,
            extents,
        };
        if locations.insert(id, location).is_some() {
            return malformed(format!("item {id} has two `iloc` entries"));
        }
    }
    Ok(locations)
}

type References = Vec<([u8; 4], u32, Vec<u32>)>;

fn parse_iref(payload: &[u8], limits: &HeifLimits) -> Result<References, HeifError> {
    let mut cursor = Cursor::new(payload, "iref");
    let (version, _) = cursor.full()?;
    let mut references = Vec::new();
    let mut total = 0u64;
    for child in children(cursor.rest(), 0, "iref", limits.items)? {
        let mut cursor = Cursor::new(child.payload, "iref");
        let id = |cursor: &mut Cursor<'_>| -> Result<u32, HeifError> {
            if version == 0 {
                cursor.u16().map(u32::from)
            } else {
                cursor.u32()
            }
        };
        let from = id(&mut cursor)?;
        let count = cursor.u16()?;
        total += u64::from(count) + 1;
        check_count(total, limits, "iref")?;
        let mut to = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            to.push(id(&mut cursor)?);
        }
        references.push((child.kind, from, to));
    }
    Ok(references)
}

type Associations = BTreeMap<u32, Vec<(bool, usize)>>;

fn parse_ipma(
    payload: &[u8],
    limits: &HeifLimits,
    into: &mut Associations,
) -> Result<(), HeifError> {
    let mut cursor = Cursor::new(payload, "ipma");
    let (version, flags) = cursor.full()?;
    let count = cursor.u32()?;
    check_count(u64::from(count) + into.len() as u64, limits, "ipma")?;
    for _ in 0..count {
        let id = if version < 1 {
            u32::from(cursor.u16()?)
        } else {
            cursor.u32()?
        };
        let associations = cursor.u8()?;
        let mut list = Vec::with_capacity(usize::from(associations));
        for _ in 0..associations {
            let (essential, index) = if flags & 1 == 1 {
                let value = cursor.u16()?;
                (value & 0x8000 != 0, usize::from(value & 0x7fff))
            } else {
                let value = cursor.u8()?;
                (value & 0x80 != 0, usize::from(value & 0x7f))
            };
            list.push((essential, index));
        }
        if into.insert(id, list).is_some() {
            return malformed(format!("item {id} has two `ipma` entries"));
        }
    }
    Ok(())
}

fn parse_meta<'a>(
    payload: &'a [u8],
    offset: u64,
    limits: &HeifLimits,
) -> Result<Meta<'a>, HeifError> {
    let mut cursor = Cursor::new(payload, "meta");
    cursor.full()?;
    let boxes = children(cursor.rest(), offset + 4, "meta", MAX_META_BOXES)?;
    let find = |kind: &[u8; 4]| boxes.iter().find(|child| &child.kind == kind);
    let handler =
        find(b"hdlr").ok_or_else(|| HeifError::Malformed("the `meta` box has no `hdlr`".into()))?;
    let mut hdlr = Cursor::new(handler.payload, "hdlr");
    hdlr.full()?;
    hdlr.u32()?;
    let handler_type = hdlr.array::<4>()?;
    if &handler_type != b"pict" {
        return unsupported(format!(
            "its `meta` box is for `{}`, not pictures",
            fourcc(&handler_type)
        ));
    }
    let pitm = find(b"pitm")
        .ok_or_else(|| HeifError::Malformed("the file names no primary item".into()))?;
    let mut pitm = Cursor::new(pitm.payload, "pitm");
    let (version, _) = pitm.full()?;
    let primary = if version == 0 {
        u32::from(pitm.u16()?)
    } else {
        pitm.u32()?
    };
    let items = match find(b"iinf") {
        Some(iinf) => parse_iinf(iinf.payload, limits)?,
        None => return malformed("the `meta` box has no `iinf`"),
    };
    let locations = match find(b"iloc") {
        Some(iloc) => parse_iloc(iloc.payload, limits)?,
        None => return malformed("the `meta` box has no `iloc`"),
    };
    let references = match find(b"iref") {
        Some(iref) => parse_iref(iref.payload, limits)?,
        None => Vec::new(),
    };
    let mut properties = Vec::new();
    let mut associations = BTreeMap::new();
    if let Some(iprp) = find(b"iprp") {
        let inner = children(iprp.payload, iprp.offset, "iprp", MAX_META_BOXES)?;
        if let Some(ipco) = inner.iter().find(|child| &child.kind == b"ipco") {
            let list = children(ipco.payload, ipco.offset, "ipco", limits.items)?;
            check_count(list.len() as u64, limits, "ipco")?;
            for child in &list {
                properties.push(property(child)?);
            }
        }
        for ipma in inner.iter().filter(|child| &child.kind == b"ipma") {
            parse_ipma(ipma.payload, limits, &mut associations)?;
        }
    }
    let idat = find(b"idat").map(|idat| (idat.payload, idat.offset));
    Ok(Meta {
        primary,
        items,
        locations,
        references,
        properties,
        associations,
        idat,
    })
}

/// The properties associated with an item, essential flag first, in association order.
fn properties_of<'m, 'a>(
    meta: &'m Meta<'a>,
    id: u32,
) -> Result<Vec<(bool, &'m Property<'a>)>, HeifError> {
    let mut out = Vec::new();
    for &(essential, index) in meta.associations.get(&id).into_iter().flatten() {
        if index == 0 {
            continue;
        }
        match meta.properties.get(index - 1) {
            Some(property) => out.push((essential, property)),
            None => {
                return malformed(format!(
                    "item {id} is associated with a property `ipco` lacks"
                ));
            }
        }
    }
    Ok(out)
}

/// What one coded `hvc1` item holds.
struct Coded<'m> {
    hvcc: &'m [u8],
    width: u32,
    height: u32,
}

fn is_transform(property: &Property<'_>) -> bool {
    matches!(
        property,
        Property::Irot(_) | Property::Imir(_) | Property::Clap(_)
    )
}

/// An `hvc1` item's configuration and size; refuses an essential property the reader does not read.
fn coded<'m>(meta: &'m Meta<'_>, id: u32, tile: bool) -> Result<Coded<'m>, HeifError> {
    let mut hvcc = None;
    let mut size = None;
    for (essential, property) in properties_of(meta, id)? {
        match property {
            Property::Hvcc(bytes) => hvcc = hvcc.or(Some(*bytes)),
            Property::Ispe(width, height) => size = size.or(Some((*width, *height))),
            Property::Other(kind) if essential => {
                return unsupported(format!(
                    "it has an essential `{}` property, which is not read",
                    fourcc(kind)
                ));
            }
            property if tile && is_transform(property) => {
                return unsupported("a tile of its grid is transformed on its own");
            }
            _ => {}
        }
    }
    let Some(hvcc) = hvcc else {
        return malformed(format!("item {id} has no `hvcC` configuration"));
    };
    let Some((width, height)) = size else {
        return malformed(format!("item {id} has no `ispe` size"));
    };
    if width == 0 || height == 0 {
        return malformed(format!("item {id} is {width} × {height}"));
    }
    Ok(Coded {
        hvcc,
        width,
        height,
    })
}

/// An `hvcC` record's codec string and formats (ISO/IEC 14496-15 §8.3.3.1, Annex E).
fn hevc_config(record: &[u8]) -> Result<HevcConfig, HeifError> {
    let mut cursor = Cursor::new(record, "hvcC");
    let version = cursor.u8()?;
    if version != 1 {
        return unsupported(format!("its `hvcC` is version {version}"));
    }
    let profile = cursor.u8()?;
    let space = profile >> 6;
    let tier = (profile >> 5) & 1;
    let idc = profile & 31;
    let compatibility = cursor.u32()?;
    let constraints = cursor.array::<6>()?;
    let level = cursor.u8()?;
    cursor.u16()?; // min_spatial_segmentation_idc
    cursor.u8()?; // parallelismType
    let chroma_format = cursor.u8()? & 3;
    let bit_depth_luma = (cursor.u8()? & 7) + 8;
    let bit_depth_chroma = (cursor.u8()? & 7) + 8;
    cursor.u16()?; // avgFrameRate
    cursor.u8()?; // constantFrameRate, numTemporalLayers, temporalIdNested, lengthSizeMinusOne
    let arrays = cursor.u8()?;
    let mut signal = None;
    for _ in 0..arrays {
        let kind = cursor.u8()? & 0x3f;
        let units = cursor.u16()?;
        for _ in 0..units {
            let len = cursor.u16()?;
            let unit = cursor.take(usize::from(len))?;
            if kind == SPS_NAL_TYPE && signal.is_none() {
                signal = Some(sps_video_signal(unit));
            }
        }
    }
    let space = ["", "A", "B", "C"]
        .get(usize::from(space))
        .copied()
        .unwrap_or("");
    let tier = if tier == 1 { 'H' } else { 'L' };
    let mut codec = format!(
        "hvc1.{space}{idc}.{:X}.{tier}{level}",
        compatibility.reverse_bits()
    );
    let kept = constraints
        .iter()
        .rposition(|&byte| byte != 0)
        .map_or(0, |last| last + 1);
    for byte in constraints.iter().take(kept) {
        codec.push_str(&format!(".{byte:X}"));
    }
    Ok(HevcConfig {
        codec,
        description: record.to_vec(),
        chroma_format,
        bit_depth_luma,
        bit_depth_chroma,
        signal: signal.unwrap_or_default(),
    })
}

/// The file ranges of an item's data; refuses data in another file or item.
fn extents<R: RangeReader + ?Sized>(
    meta: &Meta<'_>,
    source: &R,
    id: u32,
) -> Result<Vec<Extent>, HeifError> {
    let Some(location) = meta.locations.get(&id) else {
        return malformed(format!("item {id} has no `iloc` entry"));
    };
    if location.data_reference != 0 {
        return unsupported("an image's data is in another file");
    }
    if location.extents.is_empty() {
        return malformed(format!("item {id} has no data"));
    }
    // Each extent lies in the file, and together they read no more than it holds: an item cannot list one range
    // over and over.
    let total = location
        .extents
        .iter()
        .map(|&(_, len)| len)
        .fold(0, u64::saturating_add);
    if total > source.len() {
        return malformed(format!(
            "item {id}'s data adds up to {total} bytes, more than the file's {}",
            source.len()
        ));
    }
    let mut out = Vec::with_capacity(location.extents.len());
    for &(offset, len) in &location.extents {
        if len == 0 {
            return unsupported(
                "an image's data runs to the end of its container (an extent of no length)",
            );
        }
        let start = location
            .base
            .checked_add(offset)
            .ok_or_else(|| HeifError::Malformed(format!("item {id}'s data offset overflows")))?;
        let absolute = match location.construction {
            0 => {
                check_range(source.len(), start, len)?;
                start
            }
            1 => {
                let Some((idat, idat_offset)) = meta.idat else {
                    return malformed(format!("item {id} is in `idat`, which the file lacks"));
                };
                check_range(idat.len() as u64, start, len).map_err(|_| {
                    HeifError::Malformed(format!("item {id}'s data runs past `idat`"))
                })?;
                idat_offset + start
            }
            other => {
                return unsupported(format!(
                    "an image's data is built by construction method {other}"
                ));
            }
        };
        out.push(Extent {
            offset: absolute,
            len,
        });
    }
    Ok(out)
}

/// The bytes of a small item (a grid descriptor), at most `limit`.
fn item_data<R: RangeReader + ?Sized>(
    meta: &Meta<'_>,
    source: &R,
    id: u32,
    limit: u64,
) -> Result<Vec<u8>, HeifError> {
    let ranges = extents(meta, source, id)?;
    let total = ranges
        .iter()
        .map(|extent| extent.len)
        .fold(0, u64::saturating_add);
    if total > limit {
        return malformed(format!(
            "item {id} has {total} bytes, more than its {limit}"
        ));
    }
    let mut out = Vec::with_capacity(total as usize);
    for extent in ranges {
        let in_idat = meta.idat.filter(|(idat, offset)| {
            extent.offset >= *offset && extent.offset - offset < idat.len() as u64
        });
        match in_idat {
            Some((idat, offset)) => {
                let start = (extent.offset - offset) as usize;
                let bytes = idat
                    .get(start..start + extent.len as usize)
                    .ok_or_else(|| {
                        HeifError::Malformed(format!("item {id}'s data runs past `idat`"))
                    })?;
                out.extend_from_slice(bytes);
            }
            None => out.extend_from_slice(&source.read_at(extent.offset, extent.len as usize)?),
        }
    }
    Ok(out)
}

/// A clean aperture's rectangle in an image of `width` × `height` (ISO/IEC 14496-12 §12.1.4): its centre offset from
/// the image's, its size rounded to whole pixels, its left and top edges rounded down.
fn clean_aperture(fields: [u32; 8], width: u32, height: u32) -> Result<Crop, HeifError> {
    let [
        width_n,
        width_d,
        height_n,
        height_d,
        horizontal_n,
        horizontal_d,
        vertical_n,
        vertical_d,
    ] = fields;
    if [width_d, height_d, horizontal_d, vertical_d].contains(&0) {
        return malformed("the `clap` box divides by zero");
    }
    // Offsets are signed.
    let edge = |size: u32,
                clean_n: u32,
                clean_d: u32,
                offset_n: u32,
                offset_d: u32|
     -> Option<(i128, i128)> {
        let (size, clean_n, clean_d) = (i128::from(size), i128::from(clean_n), i128::from(clean_d));
        let (offset_n, offset_d) = (i128::from(offset_n as i32), i128::from(offset_d));
        // left = offset + size / 2 − clean / 2, with one denominator.
        let denominator = 2 * offset_d * clean_d;
        let numerator = 2 * offset_n * clean_d + size * offset_d * clean_d - clean_n * offset_d;
        let start = numerator.div_euclid(denominator);
        let length = (2 * clean_n + clean_d).div_euclid(2 * clean_d);
        (start >= 0 && length >= 1 && start + length <= size).then_some((start, length))
    };
    let horizontal = edge(width, width_n, width_d, horizontal_n, horizontal_d);
    let vertical = edge(height, height_n, height_d, vertical_n, vertical_d);
    let (Some((x, w)), Some((y, h))) = (horizontal, vertical) else {
        return malformed("the `clap` rectangle leaves the image");
    };
    let fit = |value: i128| u32::try_from(value).ok();
    match (fit(x), fit(y), fit(w), fit(h)) {
        (Some(x), Some(y), Some(width), Some(height)) => Ok(Crop {
            x,
            y,
            width,
            height,
        }),
        _ => malformed("the `clap` rectangle leaves the image"),
    }
}

/// Checks a grid of `columns` × `rows` tiles of `tile` size against its output `size`: the tiles cover it, their span
/// fits the coded image's 32-bit size, and their pixels (overhang included) and the output's are within the limit.
fn check_grid(
    (columns, rows): (u32, u32),
    (tile_width, tile_height): (u32, u32),
    (width, height): (u32, u32),
    check_pixels: &impl Fn(u64) -> Result<(), HeifError>,
) -> Result<(), HeifError> {
    let span = |tiles: u32, size: u32| u64::from(tiles) * u64::from(size);
    let (across, down) = (span(columns, tile_width), span(rows, tile_height));
    if across < u64::from(width) || down < u64::from(height) {
        return malformed("the tiles of its grid do not cover it");
    }
    if across > u64::from(u32::MAX) || down > u64::from(u32::MAX) {
        return malformed(format!(
            "its {columns} × {rows} grid of {tile_width} × {tile_height} tiles spans {across} × {down} pixels"
        ));
    }
    check_pixels(across.saturating_mul(down))?;
    check_pixels(u64::from(width) * u64::from(height))
}

/// Plans the decode of the primary image of the HEIF file `source`.
pub fn read_heif<R: RangeReader + ?Sized>(
    source: &R,
    limits: &HeifLimits,
) -> Result<HeifImage, HeifError> {
    let (payload, offset) = read_meta(source, limits)?;
    let meta = parse_meta(&payload, offset, limits)?;
    let primary = meta.primary;
    let Some(info) = meta.items.get(&primary) else {
        return malformed(format!("the primary item {primary} has no `infe` entry"));
    };
    if info.protected {
        return unsupported("its primary image is protected");
    }
    let alpha = meta
        .references
        .iter()
        .filter(|(kind, _, to)| kind == b"auxl" && to.contains(&primary))
        .any(|(_, from, _)| {
            properties_of(&meta, *from).is_ok_and(|properties| {
                properties.iter().any(|(_, property)| {
                    matches!(property, Property::AuxC(urn) if ALPHA_URNS.contains(urn))
                })
            })
        });
    if alpha {
        return unsupported("its primary image has an alpha plane");
    }

    // The image's own properties: its size, transforms (a clean aperture resolved once the image's size is known)
    // and colour.
    let mut pending: Vec<Result<Transform, [u32; 8]>> = Vec::new();
    let mut nclx = None;
    let mut icc = None;
    let mut bits = None;
    let mut size = None;
    for (essential, property) in properties_of(&meta, primary)? {
        match property {
            Property::Irot(turns) => pending.push(Ok(Transform::Rotate(*turns))),
            Property::Imir(axis) => pending.push(Ok(Transform::Mirror(*axis))),
            Property::Clap(fields) => pending.push(Err(*fields)),
            Property::Nclx(value) => nclx = nclx.or(Some(*value)),
            Property::Icc(bytes) => icc = icc.or_else(|| Some(bytes.to_vec())),
            Property::Pixi(value) => bits = bits.or_else(|| Some(value.clone())),
            Property::Ispe(width, height) => size = size.or(Some((*width, *height))),
            Property::Other(kind) if essential => {
                return unsupported(format!(
                    "it has an essential `{}` property, which is not read",
                    fourcc(kind)
                ));
            }
            _ => {}
        }
    }
    let repeated = |kind: fn(&Result<Transform, [u32; 8]>) -> bool| {
        pending.iter().filter(|t| kind(t)).count() > 1
    };
    if repeated(|t| t.is_err())
        || repeated(|t| matches!(t, Ok(Transform::Rotate(_))))
        || repeated(|t| matches!(t, Ok(Transform::Mirror(_))))
    {
        return unsupported("it repeats a transformative property");
    }

    let mut configs: Vec<HevcConfig> = Vec::new();
    let mut config_of = |record: &[u8]| -> Result<usize, HeifError> {
        if let Some(index) = configs
            .iter()
            .position(|config| config.description == record)
        {
            return Ok(index);
        }
        if configs.len() as u64 >= u64::from(limits.configs) {
            return unsupported(format!(
                "its tiles use more than {} HEVC configurations",
                limits.configs
            ));
        }
        configs.push(hevc_config(record)?);
        Ok(configs.len() - 1)
    };
    let check_pixels = |pixels: u64| -> Result<(), HeifError> {
        if pixels > limits.pixels {
            return Err(HeifError::TooLarge {
                pixels,
                limit: limits.pixels,
            });
        }
        Ok(())
    };

    let (coded_width, coded_height, tile_width, tile_height, columns, rows, tiles) = match &info
        .kind
    {
        b"hvc1" => {
            let item = coded(&meta, primary, false)?;
            check_pixels(u64::from(item.width) * u64::from(item.height))?;
            let tile = HeifTile {
                config: config_of(item.hvcc)?,
                x: 0,
                y: 0,
                extents: extents(&meta, source, primary)?,
            };
            (
                item.width,
                item.height,
                item.width,
                item.height,
                1,
                1,
                vec![tile],
            )
        }
        b"grid" => {
            let descriptor = item_data(&meta, source, primary, 64)?;
            let mut cursor = Cursor::new(&descriptor, "grid");
            let version = cursor.u8()?;
            if version != 0 {
                return unsupported(format!("its grid is version {version}"));
            }
            let flags = cursor.u8()?;
            let rows = u32::from(cursor.u8()?) + 1;
            let columns = u32::from(cursor.u8()?) + 1;
            let (width, height) = if flags & 1 == 1 {
                (cursor.u32()?, cursor.u32()?)
            } else {
                (u32::from(cursor.u16()?), u32::from(cursor.u16()?))
            };
            if width == 0 || height == 0 {
                return malformed(format!("its grid is {width} × {height}"));
            }
            if let Some((ispe_width, ispe_height)) = size
                && (ispe_width, ispe_height) != (width, height)
            {
                return malformed(format!(
                    "its grid is {width} × {height} but its `ispe` says {ispe_width} × {ispe_height}"
                ));
            }
            let count = rows * columns;
            if count > limits.tiles {
                return unsupported(format!(
                    "its grid has {count} tiles, more than the {} read",
                    limits.tiles
                ));
            }
            let Some((_, _, to)) = meta
                .references
                .iter()
                .find(|(kind, from, _)| kind == b"dimg" && *from == primary)
            else {
                return malformed("its grid names no tiles");
            };
            if to.len() as u64 != u64::from(count) {
                return malformed(format!(
                    "its {columns} × {rows} grid names {} tiles",
                    to.len()
                ));
            }
            let mut tiles = Vec::with_capacity(to.len());
            let mut tile_size = None;
            for (index, &id) in (0u32..).zip(to) {
                let Some(tile_info) = meta.items.get(&id) else {
                    return malformed(format!("tile {id} has no `infe` entry"));
                };
                if &tile_info.kind != b"hvc1" {
                    return unsupported(format!(
                        "a tile of its grid is a `{}` item",
                        fourcc(&tile_info.kind)
                    ));
                }
                if tile_info.protected {
                    return unsupported("a tile of its grid is protected");
                }
                let tile = coded(&meta, id, true)?;
                match tile_size {
                    // The first tile sizes the grid: it is checked whole before any tile is placed, so every place
                    // fits.
                    None => {
                        check_grid(
                            (columns, rows),
                            (tile.width, tile.height),
                            (width, height),
                            &check_pixels,
                        )?;
                        tile_size = Some((tile.width, tile.height));
                    }
                    Some(size) if size != (tile.width, tile.height) => {
                        return malformed("the tiles of its grid differ in size");
                    }
                    Some(_) => {}
                }
                let (column, row) = (index % columns, index / columns);
                let (Some(x), Some(y)) =
                    (column.checked_mul(tile.width), row.checked_mul(tile.height))
                else {
                    return malformed("its grid is too large to place its tiles");
                };
                tiles.push(HeifTile {
                    config: config_of(tile.hvcc)?,
                    x,
                    y,
                    extents: extents(&meta, source, id)?,
                });
            }
            let Some((tile_width, tile_height)) = tile_size else {
                return malformed("its grid names no tiles");
            };
            (width, height, tile_width, tile_height, columns, rows, tiles)
        }
        kind => {
            return unsupported(format!("its primary image is a `{}` item", fourcc(kind)));
        }
    };

    // The tiles' data together read no more than the file holds.
    let coded_bytes = tiles
        .iter()
        .flat_map(|tile| &tile.extents)
        .map(|extent| extent.len)
        .fold(0, u64::saturating_add);
    if coded_bytes > source.len() {
        return malformed(format!(
            "its tiles' data add up to {coded_bytes} bytes, more than the file's {}",
            source.len()
        ));
    }

    // Each crop is a rectangle of the image as the transforms before it left it.
    let mut transforms = Vec::with_capacity(pending.len());
    let mut current = (coded_width, coded_height);
    for transform in pending {
        let transform = match transform {
            Err(fields) => {
                let crop = clean_aperture(fields, current.0, current.1)?;
                current = (crop.width, crop.height);
                Transform::Crop(crop)
            }
            Ok(Transform::Rotate(0)) => continue,
            Ok(transform @ Transform::Rotate(turns)) => {
                if turns % 2 == 1 {
                    current = (current.1, current.0);
                }
                transform
            }
            Ok(transform) => transform,
        };
        transforms.push(transform);
    }

    Ok(HeifImage {
        coded_width,
        coded_height,
        tile_width,
        tile_height,
        columns,
        rows,
        configs,
        tiles,
        transforms,
        nclx,
        icc,
        bits,
    })
}
