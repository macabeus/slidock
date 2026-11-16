//! WOFF 2.0 (W3C Recommendation, 2024) decoding: the header, the table directory with its known-tag flags, the
//! collection directory, one Brotli stream holding every table, and the reversal of the `glyf`/`loca` and `hmtx`
//! transforms. The output is laid out as the reference decoder (google/woff2 `woff2_dec.cc`) lays it out: glyphs
//! padded to four bytes, simple glyphs written with repeat runs and short coordinates, `loca` right after `glyf`.

use std::collections::BTreeMap;
use std::io::Read;
use std::ops::Range;

use super::assemble::{self, Face, Table};
use super::cursor::Cursor;
use super::glyf::{self, PointEncoding, SimpleGlyph};
use super::triplet::{self, TripletError};
use super::{
    ContainerError, ContainerLimits, Format, GlyfStream, Part, check_expansion, check_output,
    is_sfnt_version,
};
use crate::sfnt::TableTag;

const HEADER: usize = 48;
const TTCF: u32 = u32::from_be_bytes(*b"ttcf");
const GLYF: [u8; 4] = *b"glyf";
const LOCA: [u8; 4] = *b"loca";
const HMTX: [u8; 4] = *b"hmtx";
const HHEA: [u8; 4] = *b"hhea";

/// The tags a table directory entry can name by index (WOFF2 §5.1).
const KNOWN_TAGS: [&[u8; 4]; 63] = [
    b"cmap", b"head", b"hhea", b"hmtx", b"maxp", b"name", b"OS/2", b"post", b"cvt ", b"fpgm",
    b"glyf", b"loca", b"prep", b"CFF ", b"VORG", b"EBDT", b"EBLC", b"gasp", b"hdmx", b"kern",
    b"LTSH", b"PCLT", b"VDMX", b"vhea", b"vmtx", b"BASE", b"GDEF", b"GPOS", b"GSUB", b"EBSC",
    b"JSTF", b"MATH", b"CBDT", b"CBLC", b"COLR", b"CPAL", b"SVG ", b"sbix", b"acnt", b"avar",
    b"bdat", b"bloc", b"bsln", b"cvar", b"fdsc", b"feat", b"fmtx", b"fvar", b"gvar", b"hsty",
    b"just", b"lcar", b"mort", b"morx", b"opbd", b"prop", b"trak", b"Zapf", b"Silf", b"Glat",
    b"Gloc", b"Feat", b"Sill",
];
const ARBITRARY_TAG: u8 = 0x3f;

// Composite glyph flags.
const ARG_1_AND_2_ARE_WORDS: u16 = 1 << 0;
const WE_HAVE_A_SCALE: u16 = 1 << 3;
const MORE_COMPONENTS: u16 = 1 << 5;
const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 1 << 6;
const WE_HAVE_A_TWO_BY_TWO: u16 = 1 << 7;
const WE_HAVE_INSTRUCTIONS: u16 = 1 << 8;

/// Transformed `glyf` option flag: an overlap bitmap follows the streams.
const OVERLAP_SIMPLE_BITMAP: u16 = 1 << 0;

/// Why a WOFF2 file can't be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Woff2Error {
    #[error("The WOFF2 header's length {declared} is not the file's {actual} bytes")]
    Length { declared: u32, actual: usize },
    #[error("A WOFF2 variable-length number is malformed")]
    Base128,
    #[error("The {tag} table has unknown transform version {version}")]
    Transform { tag: TableTag, version: u8 },
    #[error("The transformed loca table has a length")]
    LocaTransformLength,
    #[error("The compressed stream, metadata and private data are not laid out as WOFF2 requires")]
    Layout,
    #[error("The Brotli stream is invalid or not its declared length")]
    Brotli,
    #[error("A face has only one of glyf and loca, or transforms only one of them")]
    GlyfLocaPair,
    #[error("The loca table's length does not match the transformed glyf table")]
    LocaLength,
    #[error("The transformed glyf table has index format {0}")]
    IndexFormat(u16),
    #[error("Glyph {0} is composite without a bounding box")]
    CompositeWithoutBbox(u16),
    #[error("Glyph {0} is empty with a bounding box")]
    EmptyWithBbox(u16),
    #[error("The transformed hmtx table is invalid: {0}")]
    Hmtx(&'static str),
    #[error("WOFF2 collection version {0:#010x} is not 1.0 or 2.0")]
    CollectionVersion(u32),
    #[error("A collection face names table {index} of {count}")]
    TableIndex { index: u16, count: usize },
}

/// How a table is stored in the stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Transform {
    None,
    Glyf,
    Loca,
    Hmtx,
}

struct Entry {
    tag: [u8; 4],
    transform: Transform,
    orig_length: u32,
    /// Where the table lies in the decompressed stream.
    src: Range<usize>,
}

fn read_tag(cursor: &mut Cursor<'_>, flags: u8) -> Result<[u8; 4], ContainerError> {
    let index = flags & 0x3f;
    if index == ARBITRARY_TAG {
        return cursor.tag();
    }
    KNOWN_TAGS
        .get(usize::from(index))
        .map(|tag| **tag)
        .ok_or(ContainerError::Truncated(Part::TableDirectory))
}

fn base128(cursor: &mut Cursor<'_>) -> Result<u32, ContainerError> {
    cursor.base128()?.ok_or_else(|| Woff2Error::Base128.into())
}

fn read_directory(
    cursor: &mut Cursor<'_>,
    count: u16,
) -> Result<(Vec<Entry>, usize), ContainerError> {
    let mut entries = Vec::with_capacity(usize::from(count));
    let mut offset: usize = 0;
    for _ in 0..count {
        let flags = cursor.u8()?;
        let tag = read_tag(cursor, flags)?;
        let version = flags >> 6;
        let transform = match (tag, version) {
            (GLYF, 0) => Transform::Glyf,
            (LOCA, 0) => Transform::Loca,
            (GLYF | LOCA, 3) => Transform::None,
            (GLYF | LOCA, _) => {
                return Err(Woff2Error::Transform {
                    tag: TableTag(tag),
                    version,
                }
                .into());
            }
            (HMTX, 1) => Transform::Hmtx,
            (_, 0) => Transform::None,
            _ => {
                return Err(Woff2Error::Transform {
                    tag: TableTag(tag),
                    version,
                }
                .into());
            }
        };
        let orig_length = base128(cursor)?;
        let stored_length = if transform == Transform::None {
            orig_length
        } else {
            base128(cursor)?
        };
        if transform == Transform::Loca && stored_length != 0 {
            return Err(Woff2Error::LocaTransformLength.into());
        }
        let end = offset
            .checked_add(stored_length as usize)
            .ok_or(ContainerError::TooLarge {
                limit: u64::from(u32::MAX),
            })?;
        entries.push(Entry {
            tag,
            transform,
            orig_length,
            src: offset..end,
        });
        offset = end;
    }
    Ok((entries, offset))
}

/// The collection directory: its version and each face's flavor and table indices.
fn read_collection(
    cursor: &mut Cursor<'_>,
    entries: &[Entry],
    limits: &ContainerLimits,
) -> Result<(u32, Vec<Face>), ContainerError> {
    let version = cursor.u32_be()?;
    if version != 0x0001_0000 && version != 0x0002_0000 {
        return Err(Woff2Error::CollectionVersion(version).into());
    }
    let count = cursor.u255()?;
    if count == 0 || u32::from(count) > limits.max_faces {
        return Err(ContainerError::FaceCount {
            count: count.into(),
            limit: limits.max_faces,
        });
    }
    let mut faces = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let tables = cursor.u255()?;
        if tables == 0 || tables > limits.max_tables {
            return Err(ContainerError::TableCount {
                count: tables.into(),
                limit: limits.max_tables,
            });
        }
        let flavor = cursor.u32_be()?;
        if !is_sfnt_version(flavor) {
            return Err(ContainerError::NotSfnt(Format::Woff2));
        }
        let mut indices = Vec::with_capacity(usize::from(tables));
        for _ in 0..tables {
            let index = cursor.u255()?;
            if usize::from(index) >= entries.len() {
                return Err(Woff2Error::TableIndex {
                    index,
                    count: entries.len(),
                }
                .into());
            }
            indices.push(usize::from(index));
        }
        faces.push(Face {
            flavor,
            tables: indices,
        });
    }
    Ok((version, faces))
}

/// The Brotli stream inflated to exactly `length` bytes.
fn decompress(stream: &[u8], length: usize) -> Result<Vec<u8>, ContainerError> {
    let mut out = Vec::with_capacity(length);
    let decoder = brotli_decompressor::Decompressor::new(stream, 4096);
    decoder
        .take(length as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|_| Woff2Error::Brotli)?;
    if out.len() != length {
        return Err(Woff2Error::Brotli.into());
    }
    Ok(out)
}

fn round4(value: u64) -> u64 {
    value.next_multiple_of(4)
}

/// The decoded sfnt (or collection) of a WOFF2 file.
pub fn decode(bytes: &[u8], limits: &ContainerLimits) -> Result<Vec<u8>, ContainerError> {
    let mut header = Cursor::new(bytes, Part::Header);
    header.tag()?;
    let flavor = header.u32_be()?;
    let length = header.u32_be()?;
    let num_tables = header.u16_be()?;
    let _reserved = header.u16_be()?;
    let total_sfnt_size = header.u32_be()?;
    let compressed_length = header.u32_be()?;
    let _version = (header.u16_be()?, header.u16_be()?);
    let meta = (header.u32_be()?, header.u32_be()?, header.u32_be()?);
    let private = (header.u32_be()?, header.u32_be()?);
    if length as usize != bytes.len() {
        return Err(Woff2Error::Length {
            declared: length,
            actual: bytes.len(),
        }
        .into());
    }
    if num_tables == 0 || num_tables > limits.max_tables {
        return Err(ContainerError::TableCount {
            count: num_tables.into(),
            limit: limits.max_tables,
        });
    }
    check_output(total_sfnt_size.into(), limits)?;

    let mut directory = Cursor::new(
        bytes.get(HEADER..).unwrap_or_default(),
        Part::TableDirectory,
    );
    let (entries, uncompressed) = read_directory(&mut directory, num_tables)?;
    if flavor != TTCF && !is_sfnt_version(flavor) {
        return Err(ContainerError::NotSfnt(Format::Woff2));
    }
    let collection = if flavor == TTCF {
        let mut cursor = Cursor::new(directory.rest(), Part::CollectionDirectory);
        let collection = read_collection(&mut cursor, &entries, limits)?;
        directory.skip(cursor.position())?;
        Some(collection)
    } else {
        None
    };

    // The compressed stream, then the metadata and the private data, each 4-byte aligned, end the file.
    let file_length = bytes.len() as u64;
    let stream_start = (HEADER + directory.position()) as u64;
    let mut at = round4(stream_start + u64::from(compressed_length));
    if at > round4(file_length) || stream_start + u64::from(compressed_length) > file_length {
        return Err(Woff2Error::Layout.into());
    }
    for (offset, length) in [(meta.0, meta.1), (private.0, private.1)] {
        if offset == 0 {
            continue;
        }
        if u64::from(offset) != at || u64::from(offset) + u64::from(length) > file_length {
            return Err(Woff2Error::Layout.into());
        }
        at = round4(u64::from(offset) + u64::from(length));
    }
    if at != round4(file_length) {
        return Err(Woff2Error::Layout.into());
    }

    check_expansion(uncompressed as u64, bytes.len(), limits)?;
    let stream = bytes
        .get(stream_start as usize..(stream_start + u64::from(compressed_length)) as usize)
        .ok_or(Woff2Error::Layout)?;
    let data = decompress(stream, uncompressed)?;

    let faces = match &collection {
        Some((_, faces)) => faces.iter().map(|face| face.tables.clone()).collect(),
        None => vec![(0..entries.len()).collect::<Vec<_>>()],
    };
    let mut tables = reconstruct(&entries, &data, &faces, limits)?;
    match collection {
        Some((version, faces)) => assemble::collection(version, &faces, &tables, limits),
        None => {
            // A reconstructed loca follows its glyf.
            let transformed = |transform| {
                entries
                    .iter()
                    .position(|entry| entry.transform == transform)
            };
            if let (Some(glyf), Some(loca)) =
                (transformed(Transform::Glyf), transformed(Transform::Loca))
                && loca < tables.len()
            {
                let table = tables.remove(loca);
                let at = if loca < glyf { glyf } else { glyf + 1 };
                tables.insert(at.min(tables.len()), table);
            }
            assemble::font(flavor, &tables, limits)
        }
    }
}

/// A reconstructed `glyf`/`loca` pair, with what `hmtx` needs.
struct Glyphs {
    loca_index: usize,
    glyf: Vec<u8>,
    loca: Vec<u8>,
    x_mins: Vec<i16>,
}

/// Every table of the stream, with the transformed ones reconstructed.
fn reconstruct(
    entries: &[Entry],
    data: &[u8],
    faces: &[Vec<usize>],
    limits: &ContainerLimits,
) -> Result<Vec<Table>, ContainerError> {
    let stored = |index: usize| -> Result<&[u8], ContainerError> {
        entries
            .get(index)
            .and_then(|entry| data.get(entry.src.clone()))
            .ok_or(ContainerError::Truncated(Part::TableDirectory))
    };
    let mut glyphs: BTreeMap<usize, Glyphs> = BTreeMap::new();
    let mut hmtx: BTreeMap<usize, (usize, Vec<u8>)> = BTreeMap::new();
    let mut decoded_bytes: u64 = 0;

    for face in faces {
        let find = |tag: [u8; 4]| {
            face.iter()
                .copied()
                .find(|&i| entries.get(i).is_some_and(|entry| entry.tag == tag))
        };
        let mut tags = BTreeMap::new();
        for &index in face {
            if let Some(entry) = entries.get(index)
                && tags.insert(entry.tag, ()).is_some()
            {
                return Err(ContainerError::DuplicateTable(TableTag(entry.tag)));
            }
        }
        let glyf_index = find(GLYF);
        let loca_index = find(LOCA);
        let transform_of = |index: usize| entries.get(index).map(|entry| entry.transform);
        let transformed_glyf = match (glyf_index, loca_index) {
            (None, None) => None,
            (Some(g), Some(l)) => match (transform_of(g), transform_of(l)) {
                (Some(Transform::Glyf), Some(Transform::Loca)) => Some((g, l)),
                (Some(Transform::None), Some(Transform::None)) => None,
                _ => return Err(Woff2Error::GlyfLocaPair.into()),
            },
            _ => return Err(Woff2Error::GlyfLocaPair.into()),
        };
        if let Some((g, l)) = transformed_glyf {
            match glyphs.get(&g) {
                Some(done) if done.loca_index != l => return Err(Woff2Error::GlyfLocaPair.into()),
                Some(_) => {}
                None => {
                    let loca_length = entries.get(l).map_or(0, |entry| entry.orig_length);
                    let done = reconstruct_glyf(stored(g)?, l, loca_length, limits)?;
                    decoded_bytes += (done.glyf.len() + done.loca.len()) as u64;
                    check_output(decoded_bytes, limits)?;
                    glyphs.insert(g, done);
                }
            }
        }
        if let Some(h) = find(HMTX).filter(|&h| transform_of(h) == Some(Transform::Hmtx)) {
            let Some((g, _)) = transformed_glyf else {
                return Err(Woff2Error::Hmtx("the glyf table is not transformed").into());
            };
            match hmtx.get(&h) {
                Some((done, _)) if *done != g => {
                    return Err(Woff2Error::Hmtx("two faces share it with different glyphs").into());
                }
                Some(_) => {}
                None => {
                    let hhea = find(HHEA)
                        .map(stored)
                        .transpose()?
                        .ok_or(Woff2Error::Hmtx("the face has no hhea table"))?;
                    let num_h_metrics = hhea
                        .get(34..36)
                        .and_then(|b| b.try_into().ok())
                        .map(u16::from_be_bytes)
                        .ok_or(ContainerError::ShortTable(TableTag(HHEA)))?;
                    let x_mins = glyphs
                        .get(&g)
                        .map(|done| done.x_mins.as_slice())
                        .unwrap_or_default();
                    let table = reconstruct_hmtx(stored(h)?, num_h_metrics, x_mins)?;
                    decoded_bytes += table.len() as u64;
                    check_output(decoded_bytes, limits)?;
                    hmtx.insert(h, (g, table));
                }
            }
        }
    }

    let mut tables = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let data = match entry.transform {
            Transform::None => {
                let table = stored(index)?.to_vec();
                decoded_bytes += table.len() as u64;
                check_output(decoded_bytes, limits)?;
                table
            }
            Transform::Glyf => glyphs
                .get_mut(&index)
                .map(|g| std::mem::take(&mut g.glyf))
                .unwrap_or_default(),
            Transform::Loca => glyphs
                .values_mut()
                .find(|g| g.loca_index == index)
                .map(|g| std::mem::take(&mut g.loca))
                .unwrap_or_default(),
            Transform::Hmtx => hmtx
                .get_mut(&index)
                .map(|(_, t)| std::mem::take(t))
                .unwrap_or_default(),
        };
        tables.push(Table {
            tag: entry.tag,
            data,
        });
    }
    Ok(tables)
}

fn stream_error(stream: GlyfStream) -> ContainerError {
    ContainerError::Truncated(Part::GlyfStream(stream))
}

/// The byte length of a composite glyph's component records, and whether any asks for instructions.
fn composite_size(stream: &Cursor<'_>) -> Result<(usize, bool), ContainerError> {
    let mut cursor = stream.clone();
    let start = cursor.position();
    let mut instructions = false;
    let mut flags = MORE_COMPONENTS;
    while flags & MORE_COMPONENTS != 0 {
        flags = cursor.u16_be()?;
        instructions |= flags & WE_HAVE_INSTRUCTIONS != 0;
        let mut size = 2 + if flags & ARG_1_AND_2_ARE_WORDS != 0 {
            4
        } else {
            2
        };
        if flags & WE_HAVE_A_SCALE != 0 {
            size += 2;
        } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
            size += 4;
        } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
            size += 8;
        }
        cursor.skip(size)?;
    }
    Ok((cursor.position() - start, instructions))
}

fn read_bbox(bboxes: &mut Cursor<'_>) -> Result<[i16; 4], ContainerError> {
    Ok([
        bboxes.i16_be()?,
        bboxes.i16_be()?,
        bboxes.i16_be()?,
        bboxes.i16_be()?,
    ])
}

fn bit(bitmap: &[u8], index: u16) -> bool {
    bitmap
        .get(usize::from(index >> 3))
        .is_some_and(|byte| byte & (0x80 >> (index & 7)) != 0)
}

/// The `glyf` and `loca` tables of a transformed `glyf` table (its `loca` is entry `loca_index`, of
/// `loca_length` bytes), and each glyph's `xMin`.
fn reconstruct_glyf(
    data: &[u8],
    loca_index: usize,
    loca_length: u32,
    limits: &ContainerLimits,
) -> Result<Glyphs, ContainerError> {
    let mut header = Cursor::new(data, Part::GlyfStream(GlyfStream::Header));
    let _version = header.u16_be()?;
    let options = header.u16_be()?;
    let num_glyphs = header.u16_be()?;
    let index_format = header.u16_be()?;
    if num_glyphs > limits.max_glyphs {
        return Err(ContainerError::GlyphCount {
            count: num_glyphs.into(),
            limit: limits.max_glyphs,
        });
    }
    let long = match index_format {
        0 => false,
        1 => true,
        other => return Err(Woff2Error::IndexFormat(other).into()),
    };
    let expected_loca = (if long { 4 } else { 2 }) * (u32::from(num_glyphs) + 1);
    if loca_length != expected_loca {
        return Err(Woff2Error::LocaLength.into());
    }

    let kinds = [
        GlyfStream::Contours,
        GlyfStream::Points,
        GlyfStream::Flags,
        GlyfStream::Glyphs,
        GlyfStream::Composites,
        GlyfStream::Bboxes,
        GlyfStream::Instructions,
    ];
    let mut sizes = [0usize; 7];
    for size in &mut sizes {
        *size = header.u32_be()? as usize;
    }
    let mut body = Cursor::new(header.rest(), Part::GlyfStream(GlyfStream::Header));
    let mut streams = Vec::with_capacity(kinds.len());
    for (kind, size) in kinds.into_iter().zip(sizes) {
        let bytes = body.bytes(size).map_err(|_| stream_error(kind))?;
        streams.push(Cursor::new(bytes, Part::GlyfStream(kind)));
    }
    let overlap_bitmap = if options & OVERLAP_SIMPLE_BITMAP != 0 {
        body.bytes(usize::from(num_glyphs).div_ceil(8))
            .map_err(|_| stream_error(GlyfStream::OverlapBitmap))?
    } else {
        &[]
    };
    let [
        contours,
        points,
        flags,
        glyph_stream,
        composites,
        bboxes,
        instructions,
    ] = <[Cursor<'_>; 7]>::try_from(streams).map_err(|_| stream_error(GlyfStream::Header))?;
    let (mut contours, mut points, mut flags, mut glyph_stream) =
        (contours, points, flags, glyph_stream);
    let (mut composites, mut bboxes, mut instructions) = (composites, bboxes, instructions);
    let bbox_bitmap = bboxes.bytes((usize::from(num_glyphs).div_ceil(32)) * 4)?;

    let mut glyf = Vec::new();
    let mut offsets = Vec::with_capacity(usize::from(num_glyphs) + 1);
    let mut x_mins = Vec::with_capacity(usize::from(num_glyphs));
    let mut contour_points = Vec::new();
    let mut decoded = Vec::new();
    for glyph in 0..num_glyphs {
        offsets.push(u32::try_from(glyf.len()).map_err(|_| ContainerError::LocaOverflow)?);
        let has_bbox = bit(bbox_bitmap, glyph);
        let n_contours = contours.u16_be()?;
        let x_min = match n_contours {
            0xffff => {
                if !has_bbox {
                    return Err(Woff2Error::CompositeWithoutBbox(glyph).into());
                }
                let bbox = read_bbox(&mut bboxes)?;
                let (size, has_instructions) = composite_size(&composites)?;
                let instruction_length = if has_instructions {
                    glyph_stream.u255()?
                } else {
                    0
                };
                glyf.extend_from_slice(&(-1i16).to_be_bytes());
                bbox.iter()
                    .for_each(|v| glyf.extend_from_slice(&v.to_be_bytes()));
                glyf.extend_from_slice(composites.bytes(size)?);
                if has_instructions {
                    glyf.extend_from_slice(&instruction_length.to_be_bytes());
                    glyf.extend_from_slice(instructions.bytes(usize::from(instruction_length))?);
                }
                bbox[0]
            }
            0 => {
                if has_bbox {
                    return Err(Woff2Error::EmptyWithBbox(glyph).into());
                }
                0
            }
            _ => {
                contour_points.clear();
                // Each count reads at least one byte, which bounds the allocation by the stream.
                contour_points.reserve(usize::from(n_contours).min(points.remaining()));
                let mut total: usize = 0;
                for _ in 0..n_contours {
                    let count = points.u255()?;
                    total += usize::from(count);
                    contour_points.push(count);
                }
                let point_flags = flags.bytes(total)?;
                decoded.clear();
                let read = triplet::decode_into(point_flags, glyph_stream.rest(), &mut decoded)
                    .map_err(|error| match error {
                        TripletError::Truncated => stream_error(GlyfStream::Glyphs),
                        TripletError::Overflow => ContainerError::GlyphCoordinate,
                    })?;
                glyph_stream.skip(read)?;
                let instruction_length = glyph_stream.u255()?;
                let bbox = if has_bbox {
                    Some(read_bbox(&mut bboxes)?)
                } else {
                    None
                };
                glyf::write_simple(
                    &mut glyf,
                    &SimpleGlyph {
                        contour_points: &contour_points,
                        points: &decoded,
                        instructions: instructions.bytes(usize::from(instruction_length))?,
                        bbox,
                        overlap: bit(overlap_bitmap, glyph),
                    },
                    PointEncoding::Woff2,
                )?
            }
        };
        glyf::pad4(&mut glyf);
        check_output(glyf.len() as u64, limits)?;
        x_mins.push(x_min);
    }
    offsets.push(u32::try_from(glyf.len()).map_err(|_| ContainerError::LocaOverflow)?);
    let loca = glyf::loca(&offsets, long)?;
    Ok(Glyphs {
        loca_index,
        glyf,
        loca,
        x_mins,
    })
}

/// The `hmtx` table of a transformed one: advances, then each left side bearing either stored or the glyph's
/// `xMin`.
fn reconstruct_hmtx(
    data: &[u8],
    num_h_metrics: u16,
    x_mins: &[i16],
) -> Result<Vec<u8>, ContainerError> {
    let mut cursor = Cursor::new(data, Part::HmtxStream);
    let flags = cursor.u8()?;
    if flags & 0xfc != 0 {
        return Err(Woff2Error::Hmtx("reserved flag bits are set").into());
    }
    let proportional_stored = flags & 1 == 0;
    let monospace_stored = flags & 2 == 0;
    if proportional_stored && monospace_stored {
        return Err(Woff2Error::Hmtx("no left side bearings are left out").into());
    }
    let num_glyphs = x_mins.len();
    let metrics = usize::from(num_h_metrics);
    if metrics == 0 || metrics > num_glyphs {
        return Err(Woff2Error::Hmtx("numberOfHMetrics is out of range").into());
    }
    let mut advances = Vec::with_capacity(metrics);
    for _ in 0..metrics {
        advances.push(cursor.u16_be()?);
    }
    let mut out = Vec::with_capacity(2 * num_glyphs + 2 * metrics);
    for (glyph, &x_min) in x_mins.iter().enumerate() {
        let stored = if glyph < metrics {
            proportional_stored
        } else {
            monospace_stored
        };
        let lsb = if stored { cursor.i16_be()? } else { x_min };
        if let Some(advance) = advances.get(glyph) {
            out.extend_from_slice(&advance.to_be_bytes());
        }
        out.extend_from_slice(&lsb.to_be_bytes());
    }
    Ok(out)
}
