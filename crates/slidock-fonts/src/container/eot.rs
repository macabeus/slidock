//! Embedded OpenType (W3C EOT submission, versions 1.0, 2.1 and 2.2): the little-endian header, then the font data,
//! optionally XORed with 0x50 and optionally MicroType Express compressed. PowerPoint and Google Slides write a
//! PPTX's `ppt/fonts/*.fntdata` parts this way.

use super::cursor::Cursor;
use super::mtx;
use super::{
    ContainerError, ContainerLimits, ContainerWarning, Format, HeaderNames, Part, check_output,
};

/// `MagicNumber`, at byte 34.
pub const MAGIC: u16 = 0x504c;
/// `TTEMBED_SUBSET`: the font holds only the glyphs its document uses.
pub const FLAG_SUBSET: u32 = 0x0000_0001;
/// `TTEMBED_TTCOMPRESSED`: the font data is MicroType Express compressed.
pub const FLAG_COMPRESSED: u32 = 0x0000_0004;
/// `TTEMBED_XORENCRYPTDATA`: every font data byte is XORed with 0x50.
pub const FLAG_XOR: u32 = 0x1000_0000;

const XOR_KEY: u8 = 0x50;
const VERSION_1_0: u32 = 0x0001_0000;
const VERSION_2_1: u32 = 0x0002_0001;
const VERSION_2_2: u32 = 0x0002_0002;
/// The fixed fields, up to and including `Padding1`.
const FIXED_HEADER: usize = 82;
const MAX_NAME_BYTES: u16 = 512;

/// Why an EOT can't be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EotError {
    #[error("EOTSize {eot_size} is more than the {length} bytes of the file")]
    Size { eot_size: u32, length: usize },
    #[error("FontDataSize {font_data_size} is more than EOTSize {eot_size}")]
    FontDataSize { font_data_size: u32, eot_size: u32 },
    #[error("EOT version {0:#010x} is not 1.0, 2.1 or 2.2")]
    Version(u32),
    #[error("The EOT magic number is missing")]
    Magic,
    #[error("The EOT reserved fields are not zero")]
    Reserved,
    #[error("An EOT name string of {0} bytes is odd or longer than 512 bytes")]
    NameSize(u16),
}

/// The fields of an EOT header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EotHeader {
    pub version: u32,
    pub flags: u32,
    pub panose: [u8; 10],
    pub charset: u8,
    pub italic: bool,
    /// As `usWeightClass`.
    pub weight: u32,
    pub fs_type: u16,
    pub unicode_range: [u32; 4],
    pub code_page_range: [u32; 2],
    pub checksum_adjustment: u32,
    pub family_name: String,
    pub style_name: String,
    pub version_name: String,
    pub full_name: String,
    /// The sites allowed to use the font (versions 2.1 and 2.2), as written.
    pub root_string: String,
    /// Where the header's variable part ends.
    pub header_end: u64,
    /// Where the font data starts: `EOTSize - FontDataSize`.
    pub data_start: u64,
}

impl EotHeader {
    pub fn subset(&self) -> bool {
        self.flags & FLAG_SUBSET != 0
    }

    pub fn compressed(&self) -> bool {
        self.flags & FLAG_COMPRESSED != 0
    }

    pub fn xor(&self) -> bool {
        self.flags & FLAG_XOR != 0
    }

    pub(super) fn names(&self) -> HeaderNames {
        HeaderNames {
            family: self.family_name.clone(),
            style: self.style_name.clone(),
            version: self.version_name.clone(),
            full_name: self.full_name.clone(),
            weight: self.weight,
            italic: self.italic,
        }
    }
}

/// An EOT's header and its font data, still XORed and compressed as the flags say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Eot<'a> {
    pub header: EotHeader,
    pub data: &'a [u8],
    pub warnings: Vec<ContainerWarning>,
}

/// An EOT's header and its unwrapped font.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EotFont {
    pub header: EotHeader,
    pub sfnt: Vec<u8>,
    pub warnings: Vec<ContainerWarning>,
}

fn utf16le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    String::from_utf16_lossy(&units)
}

/// A `Padding` field, then a name size and its UTF-16LE string.
fn name(cursor: &mut Cursor<'_>) -> Result<String, ContainerError> {
    cursor.u16_le()?;
    let size = cursor.u16_le()?;
    if size % 2 != 0 || size > MAX_NAME_BYTES {
        return Err(EotError::NameSize(size).into());
    }
    Ok(utf16le(cursor.bytes(usize::from(size))?))
}

/// Reads the header and finds the font data. The data is the file's last `FontDataSize` bytes before `EOTSize`;
/// bytes after `EOTSize` are ignored.
pub fn parse(part: &[u8]) -> Result<Eot<'_>, ContainerError> {
    let mut cursor = Cursor::new(part, Part::Header);
    let eot_size = cursor.u32_le()?;
    let font_data_size = cursor.u32_le()?;
    let version = cursor.u32_le()?;
    let flags = cursor.u32_le()?;
    if part.len() < FIXED_HEADER {
        return Err(ContainerError::Truncated(Part::Header));
    }
    if eot_size as usize > part.len() {
        return Err(EotError::Size {
            eot_size,
            length: part.len(),
        }
        .into());
    }
    if font_data_size > eot_size {
        return Err(EotError::FontDataSize {
            font_data_size,
            eot_size,
        }
        .into());
    }
    if ![VERSION_1_0, VERSION_2_1, VERSION_2_2].contains(&version) {
        return Err(EotError::Version(version).into());
    }
    let mut panose = [0u8; 10];
    panose.copy_from_slice(cursor.bytes(10)?);
    let charset = cursor.u8()?;
    let italic = cursor.u8()? != 0;
    let weight = cursor.u32_le()?;
    let fs_type = cursor.u16_le()?;
    if cursor.u16_le()? != MAGIC {
        return Err(EotError::Magic.into());
    }
    let mut unicode_range = [0u32; 4];
    for range in &mut unicode_range {
        *range = cursor.u32_le()?;
    }
    let code_page_range = [cursor.u32_le()?, cursor.u32_le()?];
    let checksum_adjustment = cursor.u32_le()?;
    for _ in 0..4 {
        if cursor.u32_le()? != 0 {
            return Err(EotError::Reserved.into());
        }
    }

    let mut strings = Cursor::new(
        part.get(..eot_size as usize).unwrap_or_default(),
        Part::EotString,
    );
    strings.skip(cursor.position())?;
    let family_name = name(&mut strings)?;
    let style_name = name(&mut strings)?;
    let version_name = name(&mut strings)?;
    let full_name = name(&mut strings)?;
    let mut root_string = String::new();
    if version != VERSION_1_0 {
        strings.u16_le()?;
        let size = strings.u16_le()?;
        root_string = utf16le(strings.bytes(usize::from(size))?);
    }
    if version == VERSION_2_2 {
        // RootStringCheckSum, EUDCCodePage, Padding6.
        strings.skip(4 + 4 + 2)?;
        let signature = strings.u16_le()?;
        strings.skip(usize::from(signature))?;
        // EUDCFlags.
        strings.u32_le()?;
        let eudc = strings.u32_le()?;
        strings.skip(eudc as usize)?;
    }

    let header_end = strings.position() as u64;
    let data_start = u64::from(eot_size - font_data_size);
    let mut warnings = Vec::new();
    if header_end != data_start {
        warnings.push(ContainerWarning::EotHeaderEnd {
            header_end,
            data_start,
        });
    }
    let data = part
        .get((eot_size - font_data_size) as usize..eot_size as usize)
        .ok_or(ContainerError::Truncated(Part::Header))?;
    Ok(Eot {
        header: EotHeader {
            version,
            flags,
            panose,
            charset,
            italic,
            weight,
            fs_type,
            unicode_range,
            code_page_range,
            checksum_adjustment,
            family_name,
            style_name,
            version_name,
            full_name,
            root_string,
            header_end,
            data_start,
        },
        data,
        warnings,
    })
}

/// The EOT's font: its data with the XOR undone, then MicroType Express decoded. It must be a single TrueType or
/// OpenType face.
pub fn decode(part: &[u8], limits: &ContainerLimits) -> Result<EotFont, ContainerError> {
    let eot = parse(part)?;
    check_output(eot.data.len() as u64, limits)?;
    let mut data = eot.data.to_vec();
    if eot.header.xor() {
        data.iter_mut().for_each(|byte| *byte ^= XOR_KEY);
    }
    let mut warnings = eot.warnings;
    if eot.header.compressed() {
        let font = mtx::decode(&data, limits)?;
        data = font.sfnt;
        warnings.extend(font.warnings);
    }
    if !super::is_single_sfnt(&data) {
        return Err(ContainerError::NotSfnt(Format::Eot));
    }
    Ok(EotFont {
        header: eot.header,
        sfnt: data,
        warnings,
    })
}
