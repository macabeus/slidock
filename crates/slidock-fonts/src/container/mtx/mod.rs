//! MicroType Express (W3C MTX submission, 2008): the compression of TTEMBED_TTCOMPRESSED EOT font data. A 10-byte
//! header gives three LZCOMP blocks (`lzcomp`), which hold the font in Compact Table Format (`ctf`).
//!
//! The rebuilt font matches libeot's (`umanwizard/libeot`) table for table, except that `hdmx` and `VDMX`, which
//! libeot drops, are decoded.

mod ctf;
mod lzcomp;

#[cfg(test)]
mod encode;
#[cfg(test)]
mod tests;

use std::fmt;

use super::cursor::Cursor;
use super::{ContainerError, ContainerLimits, ContainerWarning, Part, assemble};
use crate::sfnt::TableTag;

/// The MTX version this decoder reads.
pub const VERSION: u8 = 3;
const HEADER: usize = 10;
const TRUETYPE: u32 = 0x0001_0000;

/// Why MicroType Express data can't be decoded.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MtxError {
    #[error("MicroType Express version {0} is not 3")]
    Version(u8),
    #[error(
        "The MicroType Express block offsets {second} and {third} do not fit its {length} bytes"
    )]
    Offsets {
        second: u32,
        third: u32,
        length: usize,
    },
    #[error("LZCOMP block {block} is not valid")]
    Lzcomp { block: u8 },
    #[error("The compact font has no {0} table")]
    MissingTable(TableTag),
    #[error("The compact font's {0} table lies outside its block")]
    TableBounds(TableTag),
    #[error("A push data hop code has no value two back or overruns the glyph's push count")]
    HopCode,
}

/// The streams of an MTX font, for [`Part::Mtx`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MtxStream {
    Header,
    /// The bitstream of LZCOMP block 1, 2 or 3.
    Lzcomp(u8),
    /// The compact glyph records in block 1's `glyf` table.
    Glyphs,
    /// Block 2.
    PushData,
    /// Block 3.
    Instructions,
}

impl fmt::Display for MtxStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MtxStream::Header => f.write_str("header"),
            MtxStream::Lzcomp(block) => write!(f, "LZCOMP block {block}"),
            MtxStream::Glyphs => f.write_str("glyph data"),
            MtxStream::PushData => f.write_str("push data"),
            MtxStream::Instructions => f.write_str("instruction data"),
        }
    }
}

/// A decoded MTX font.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MtxFont {
    pub sfnt: Vec<u8>,
    pub warnings: Vec<ContainerWarning>,
}

/// The three LZCOMP blocks of an MTX font, decompressed: the compact font, its push data and its instructions.
pub fn decompress(data: &[u8], limits: &ContainerLimits) -> Result<[Vec<u8>; 3], ContainerError> {
    let mut cursor = Cursor::new(data, Part::Mtx(MtxStream::Header));
    let version = cursor.u8()?;
    if version != VERSION {
        return Err(MtxError::Version(version).into());
    }
    // The copy limit bounds the encoder's search; decoding does not need it.
    cursor.skip(3)?;
    let mut offset = || -> Result<u32, ContainerError> {
        let bytes = cursor.bytes(3)?;
        Ok(bytes
            .iter()
            .fold(0u32, |value, &byte| value << 8 | u32::from(byte)))
    };
    let (second, third) = (offset()?, offset()?);
    let bad_offsets = || MtxError::Offsets {
        second,
        third,
        length: data.len(),
    };
    let (start2, start3) = (second as usize, third as usize);
    if start2 < HEADER || start3 < start2 || start3 > data.len() {
        return Err(bad_offsets().into());
    }
    let mut spent = 0;
    let mut decompress = |block: u8, range: std::ops::Range<usize>| {
        let bytes = data.get(range).ok_or_else(bad_offsets)?;
        lzcomp::decompress(bytes, block, &mut spent, data.len(), limits)
    };
    Ok([
        decompress(1, HEADER..start2)?,
        decompress(2, start2..start3)?,
        decompress(3, start3..data.len())?,
    ])
}

/// Decodes MTX data into a TrueType font: table data in the compact font's directory order, records in tag order,
/// each table padded to four bytes, fresh checksums and `head.checkSumAdjustment`.
pub fn decode(data: &[u8], limits: &ContainerLimits) -> Result<MtxFont, ContainerError> {
    let [font, push, code] = decompress(data, limits)?;
    decode_ctf([&font, &push, &code], limits)
}

/// Decodes a Compact Table Format font from its three decompressed blocks, as [`decode`] does.
pub fn decode_ctf(blocks: [&[u8]; 3], limits: &ContainerLimits) -> Result<MtxFont, ContainerError> {
    let (tables, warnings) = ctf::tables(blocks, limits)?;
    let sfnt = assemble::font(TRUETYPE, &tables, limits)?;
    Ok(MtxFont { sfnt, warnings })
}
