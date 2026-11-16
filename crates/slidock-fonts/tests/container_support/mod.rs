//! Fixture loading, sfnt table access and small container builders for the container tests.

#![allow(dead_code, reason = "each test file uses some of the helpers")]

use std::collections::BTreeMap;
use std::path::PathBuf;

pub fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/container")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

pub fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_be_bytes(bytes[at..at + 2].try_into().unwrap())
}

pub fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap())
}

/// The table directory offsets of every face of a font or collection.
pub fn face_offsets(sfnt: &[u8]) -> Vec<usize> {
    if &sfnt[..4] == b"ttcf" {
        (0..u32_at(sfnt, 8) as usize)
            .map(|i| u32_at(sfnt, 12 + 4 * i) as usize)
            .collect()
    } else {
        vec![0]
    }
}

/// One face's tables by tag, with each record's checksum.
pub fn tables(sfnt: &[u8], face: usize) -> BTreeMap<[u8; 4], (&[u8], u32)> {
    let count = u16_at(sfnt, face + 4) as usize;
    (0..count)
        .map(|i| {
            let record = face + 12 + 16 * i;
            let tag: [u8; 4] = sfnt[record..record + 4].try_into().unwrap();
            let offset = u32_at(sfnt, record + 8) as usize;
            let length = u32_at(sfnt, record + 12) as usize;
            (
                tag,
                (&sfnt[offset..offset + length], u32_at(sfnt, record + 4)),
            )
        })
        .collect()
}

pub fn table<'a>(sfnt: &'a [u8], face: usize, tag: &[u8; 4]) -> &'a [u8] {
    tables(sfnt, face)[tag].0
}

/// A glyph as a decoder must reproduce it, whatever bytes encode it.
#[derive(Debug, PartialEq)]
pub enum GlyphModel {
    Empty,
    Simple {
        bbox: [i16; 4],
        end_points: Vec<u16>,
        instructions: Vec<u8>,
        /// x, y, on curve, overlap.
        points: Vec<(i32, i32, bool, bool)>,
    },
    Composite {
        bbox: [i16; 4],
        /// Flags without the encoding bits, glyph, arguments, transform.
        components: Vec<(u16, u16, i32, i32, Vec<i16>)>,
        instructions: Vec<u8>,
    },
}

pub fn glyph_model(glyph: &[u8]) -> GlyphModel {
    if glyph.is_empty() {
        return GlyphModel::Empty;
    }
    let contours = u16_at(glyph, 0) as i16;
    let bbox = [2, 4, 6, 8].map(|at| u16_at(glyph, at) as i16);
    if contours < 0 {
        let mut at = 10;
        let mut components = Vec::new();
        let mut instructions = false;
        loop {
            let flags = u16_at(glyph, at);
            let index = u16_at(glyph, at + 2);
            at += 4;
            instructions |= flags & 0x100 != 0;
            let xy = flags & 2 != 0;
            let (a, b) = if flags & 1 != 0 {
                let (a, b) = (u16_at(glyph, at), u16_at(glyph, at + 2));
                at += 4;
                if xy {
                    (a as i16 as i32, b as i16 as i32)
                } else {
                    (a as i32, b as i32)
                }
            } else {
                let (a, b) = (glyph[at], glyph[at + 1]);
                at += 2;
                if xy {
                    (a as i8 as i32, b as i8 as i32)
                } else {
                    (a as i32, b as i32)
                }
            };
            let count = if flags & 8 != 0 {
                1
            } else if flags & 0x40 != 0 {
                2
            } else if flags & 0x80 != 0 {
                4
            } else {
                0
            };
            let transform = (0..count)
                .map(|i| u16_at(glyph, at + 2 * i) as i16)
                .collect();
            at += 2 * count;
            // Keep ARGS_ARE_XY_VALUES, ROUND_XY_TO_GRID, USE_MY_METRICS, OVERLAP_COMPOUND and the scaled-offset bits.
            components.push((flags & 0x1e06, index, a, b, transform));
            if flags & 0x20 == 0 {
                break;
            }
        }
        let instructions = if instructions {
            let length = u16_at(glyph, at) as usize;
            glyph[at + 2..at + 2 + length].to_vec()
        } else {
            Vec::new()
        };
        return GlyphModel::Composite {
            bbox,
            components,
            instructions,
        };
    }
    let contours = contours as usize;
    let end_points: Vec<u16> = (0..contours).map(|i| u16_at(glyph, 10 + 2 * i)).collect();
    let points = end_points.last().map_or(0, |&e| e as usize + 1);
    let mut at = 10 + 2 * contours;
    let instruction_length = u16_at(glyph, at) as usize;
    let instructions = glyph[at + 2..at + 2 + instruction_length].to_vec();
    at += 2 + instruction_length;
    let mut flags = Vec::with_capacity(points);
    while flags.len() < points {
        let flag = glyph[at];
        at += 1;
        let repeat = if flag & 8 != 0 {
            at += 1;
            glyph[at - 1] as usize
        } else {
            0
        };
        for _ in 0..=repeat {
            flags.push(flag);
        }
    }
    let mut read = |short: u8, same: u8, flags: &[u8]| -> Vec<i32> {
        let mut value = 0;
        flags
            .iter()
            .map(|&flag| {
                if flag & short != 0 {
                    let delta = glyph[at] as i32;
                    at += 1;
                    value += if flag & same != 0 { delta } else { -delta };
                } else if flag & same == 0 {
                    value += u16_at(glyph, at) as i16 as i32;
                    at += 2;
                }
                value
            })
            .collect()
    };
    let xs = read(2, 0x10, &flags);
    let ys = read(4, 0x20, &flags);
    let points = flags
        .iter()
        .zip(xs.into_iter().zip(ys))
        .map(|(&flag, (x, y))| (x, y, flag & 1 != 0, flag & 0x40 != 0))
        .collect();
    GlyphModel::Simple {
        bbox,
        end_points,
        instructions,
        points,
    }
}

/// Each glyph's record, by `loca`, with trailing zero padding dropped.
pub fn glyphs(sfnt: &[u8], face: usize) -> Vec<Vec<u8>> {
    let long = u16_at(table(sfnt, face, b"head"), 50) == 1;
    let loca = table(sfnt, face, b"loca");
    let glyf = table(sfnt, face, b"glyf");
    let offsets: Vec<usize> = if long {
        loca.chunks(4)
            .map(|c| u32::from_be_bytes(c.try_into().unwrap()) as usize)
            .collect()
    } else {
        loca.chunks(2)
            .map(|c| 2 * u16::from_be_bytes(c.try_into().unwrap()) as usize)
            .collect()
    };
    offsets
        .windows(2)
        .map(|pair| glyph_without_padding(&glyf[pair[0]..pair[1]]))
        .collect()
}

/// A glyph record cut to the length its own fields give.
fn glyph_without_padding(glyph: &[u8]) -> Vec<u8> {
    if glyph.is_empty() {
        return Vec::new();
    }
    let contours = u16_at(glyph, 0) as i16;
    let end = if contours < 0 {
        let mut at = 10;
        let mut instructions = false;
        loop {
            let flags = u16_at(glyph, at);
            instructions |= flags & 0x100 != 0;
            at += 4 + if flags & 1 != 0 { 4 } else { 2 };
            at += if flags & 8 != 0 {
                2
            } else if flags & 0x40 != 0 {
                4
            } else if flags & 0x80 != 0 {
                8
            } else {
                0
            };
            if flags & 0x20 == 0 {
                break;
            }
        }
        if instructions {
            at += 2 + u16_at(glyph, at) as usize;
        }
        at
    } else {
        let contours = contours as usize;
        let points = u16_at(glyph, 10 + 2 * (contours - 1)) as usize + 1;
        let mut at = 10 + 2 * contours;
        at += 2 + u16_at(glyph, at) as usize;
        let (mut x_bytes, mut y_bytes, mut seen) = (0, 0, 0);
        while seen < points {
            let flag = glyph[at];
            at += 1;
            let repeat = if flag & 8 != 0 {
                at += 1;
                glyph[at - 1] as usize + 1
            } else {
                1
            };
            x_bytes += repeat
                * if flag & 2 != 0 {
                    1
                } else if flag & 0x10 != 0 {
                    0
                } else {
                    2
                };
            y_bytes += repeat
                * if flag & 4 != 0 {
                    1
                } else if flag & 0x20 != 0 {
                    0
                } else {
                    2
                };
            seen += repeat;
        }
        at + x_bytes + y_bytes
    };
    glyph[..end].to_vec()
}

/// A WOFF 1.0 file of `tables`, each zlib compressed when that is smaller.
pub fn woff1(flavor: u32, tables: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut directory = Vec::new();
    let mut data = Vec::new();
    let data_start = 44 + 20 * tables.len();
    for (tag, table) in tables {
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(table, 9);
        let stored = if compressed.len() < table.len() {
            compressed
        } else {
            table.clone()
        };
        directory.extend_from_slice(tag);
        directory.extend_from_slice(&((data_start + data.len()) as u32).to_be_bytes());
        directory.extend_from_slice(&(stored.len() as u32).to_be_bytes());
        directory.extend_from_slice(&(table.len() as u32).to_be_bytes());
        directory.extend_from_slice(&checksum(table).to_be_bytes());
        data.extend_from_slice(&stored);
        data.resize(data.len().next_multiple_of(4), 0);
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"wOFF");
    out.extend_from_slice(&flavor.to_be_bytes());
    out.extend_from_slice(&((data_start + data.len()) as u32).to_be_bytes());
    out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
    out.extend_from_slice(&[0; 2]);
    out.extend_from_slice(&[0; 28]);
    out.extend_from_slice(&directory);
    out.extend_from_slice(&data);
    out
}

pub fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

/// A WOFF2 file of untransformed `tables` (tags by known index or written out), Brotli compressed.
pub fn woff2_plain(flavor: u32, tables: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let entries: Vec<(u8, [u8; 4], u32, Option<u32>)> = tables
        .iter()
        .map(|(tag, data)| (0, *tag, data.len() as u32, None))
        .collect();
    let stream: Vec<u8> = tables.iter().flat_map(|(_, data)| data.clone()).collect();
    woff2_raw(flavor, &entries, &stream, &[])
}

/// A WOFF2 file from directory entries `(transform version, tag, origLength, transformLength)`, the stream before
/// compression, and the collection directory bytes.
pub fn woff2_raw(
    flavor: u32,
    entries: &[(u8, [u8; 4], u32, Option<u32>)],
    stream: &[u8],
    collection: &[u8],
) -> Vec<u8> {
    let mut directory = Vec::new();
    for (version, tag, orig, transformed) in entries {
        let known = KNOWN.iter().position(|t| *t == tag);
        directory.push((version << 6) | known.map_or(0x3f, |i| i as u8));
        if known.is_none() {
            directory.extend_from_slice(tag);
        }
        directory.extend_from_slice(&base128(*orig));
        if let Some(length) = transformed {
            directory.extend_from_slice(&base128(*length));
        }
    }
    let mut compressed = Vec::new();
    {
        let mut writer = brotli::CompressorWriter::new(&mut compressed, 4096, 11, 22);
        std::io::Write::write_all(&mut writer, stream).unwrap();
    }
    let header_and_directories = 48 + directory.len() + collection.len();
    let total = (header_and_directories + compressed.len()).next_multiple_of(4);
    let mut out = Vec::new();
    out.extend_from_slice(b"wOF2");
    out.extend_from_slice(&flavor.to_be_bytes());
    out.extend_from_slice(&(total as u32).to_be_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_be_bytes());
    out.extend_from_slice(&[0; 2]);
    out.extend_from_slice(&(stream.len() as u32 + 1024).to_be_bytes());
    out.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&[0; 20]);
    out.extend_from_slice(&directory);
    out.extend_from_slice(collection);
    out.extend_from_slice(&compressed);
    out.resize(total, 0);
    out
}

const KNOWN: [&[u8; 4]; 13] = [
    b"cmap", b"head", b"hhea", b"hmtx", b"maxp", b"name", b"OS/2", b"post", b"cvt ", b"fpgm",
    b"glyf", b"loca", b"prep",
];

pub fn base128(mut value: u32) -> Vec<u8> {
    let mut bytes = vec![(value & 0x7f) as u8];
    value >>= 7;
    while value != 0 {
        bytes.insert(0, (value & 0x7f) as u8 | 0x80);
        value >>= 7;
    }
    bytes
}

/// A deterministic xorshift generator for the mutation tests.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// WOFF2's `255UInt16`.
pub fn u255(value: u16) -> Vec<u8> {
    match value {
        0..253 => vec![value as u8],
        253..506 => vec![255, (value - 253) as u8],
        506..762 => vec![254, (value - 506) as u8],
        _ => vec![253, (value >> 8) as u8, value as u8],
    }
}

/// One glyph of a transformed `glyf` table.
pub enum TGlyph {
    Empty {
        bbox: bool,
    },
    /// Points as absolute coordinates.
    Simple {
        contours: Vec<u16>,
        points: Vec<(i32, i32, bool)>,
        instructions: Vec<u8>,
        bbox: Option<[i16; 4]>,
    },
    Composite {
        records: Vec<u8>,
        instructions: Option<Vec<u8>>,
        bbox: Option<[i16; 4]>,
    },
}

/// A transformed `glyf` table. Points are written with four-byte triplets.
pub fn glyf_transform(index_format: u16, glyphs: &[TGlyph], overlap: Option<&[u8]>) -> Vec<u8> {
    let (mut n_contours, mut n_points, mut flags, mut glyph_stream) =
        (vec![], vec![], vec![], vec![]);
    let (mut composites, mut bboxes, mut instructions) = (vec![], vec![], vec![]);
    let mut bitmap = vec![0u8; glyphs.len().div_ceil(32) * 4];
    for (i, glyph) in glyphs.iter().enumerate() {
        let mut put_bbox = |bbox: &Option<[i16; 4]>| {
            if let Some(bbox) = bbox {
                bitmap[i / 8] |= 0x80 >> (i % 8);
                bbox.iter()
                    .for_each(|v| bboxes.extend_from_slice(&v.to_be_bytes()));
            }
        };
        match glyph {
            TGlyph::Empty { bbox } => {
                n_contours.extend_from_slice(&0u16.to_be_bytes());
                put_bbox(&bbox.then_some([0; 4]));
            }
            TGlyph::Simple {
                contours,
                points,
                instructions: code,
                bbox,
            } => {
                n_contours.extend_from_slice(&(contours.len() as u16).to_be_bytes());
                contours
                    .iter()
                    .for_each(|&c| n_points.extend_from_slice(&u255(c)));
                let (mut x, mut y) = (0, 0);
                for &(px, py, on) in points {
                    let (dx, dy): (i32, i32) = (px - x, py - y);
                    (x, y) = (px, py);
                    let flag = 124 | u8::from(dx >= 0) | (u8::from(dy >= 0) << 1);
                    flags.push(flag | if on { 0 } else { 0x80 });
                    glyph_stream.extend_from_slice(&(dx.unsigned_abs() as u16).to_be_bytes());
                    glyph_stream.extend_from_slice(&(dy.unsigned_abs() as u16).to_be_bytes());
                }
                glyph_stream.extend_from_slice(&u255(code.len() as u16));
                instructions.extend_from_slice(code);
                put_bbox(bbox);
            }
            TGlyph::Composite {
                records,
                instructions: code,
                bbox,
            } => {
                n_contours.extend_from_slice(&0xffffu16.to_be_bytes());
                composites.extend_from_slice(records);
                if let Some(code) = code {
                    glyph_stream.extend_from_slice(&u255(code.len() as u16));
                    instructions.extend_from_slice(code);
                }
                put_bbox(bbox);
            }
        }
    }
    let bbox_stream = [bitmap, bboxes].concat();
    let streams = [
        n_contours,
        n_points,
        flags,
        glyph_stream,
        composites,
        bbox_stream,
        instructions,
    ];
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&u16::from(overlap.is_some()).to_be_bytes());
    out.extend_from_slice(&(glyphs.len() as u16).to_be_bytes());
    out.extend_from_slice(&index_format.to_be_bytes());
    streams
        .iter()
        .for_each(|s| out.extend_from_slice(&(s.len() as u32).to_be_bytes()));
    streams.iter().for_each(|s| out.extend_from_slice(s));
    if let Some(overlap) = overlap {
        out.extend_from_slice(overlap);
    }
    out
}

/// A `head` table with this `indexToLocFormat`.
pub fn head(index_format: i16) -> Vec<u8> {
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5f0f_3cf5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[50..52].copy_from_slice(&index_format.to_be_bytes());
    head
}

/// An `hhea` table with this `numberOfHMetrics`.
pub fn hhea(metrics: u16) -> Vec<u8> {
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[34..36].copy_from_slice(&metrics.to_be_bytes());
    hhea
}
