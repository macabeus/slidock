//! Builders for the io tests' inputs: zips (plain, ZIP64, patched), TIFF and BMP files, ISO movies, and the seeded
//! mutations of the fuzz tests.

#![allow(dead_code, reason = "each test file uses some of the builders")]

pub fn concat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                0xedb8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A zip of `files` in order, every entry STORED (`level` 0) or DEFLATE at `level`.
pub fn zip(files: &[(&str, &[u8])], level: u8) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, content) in files {
        let (method, data): (u16, Vec<u8>) = if level == 0 {
            (0, content.to_vec())
        } else {
            (8, miniz_oxide::deflate::compress_to_vec(content, level))
        };
        let offset = out.len() as u32;
        let crc = crc32(content);
        let common = |header: &mut Vec<u8>| {
            header.extend(20u16.to_le_bytes()); // version needed
            header.extend(0u16.to_le_bytes()); // flags
            header.extend(method.to_le_bytes());
            header.extend(0u16.to_le_bytes()); // time
            header.extend(0x21u16.to_le_bytes()); // date: 1980-01-01
            header.extend(crc.to_le_bytes());
            header.extend((data.len() as u32).to_le_bytes());
            header.extend((content.len() as u32).to_le_bytes());
            header.extend((name.len() as u16).to_le_bytes());
            header.extend(0u16.to_le_bytes()); // extra field
        };
        out.extend(0x0403_4b50u32.to_le_bytes());
        common(&mut out);
        out.extend(name.as_bytes());
        out.extend(&data);

        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend(20u16.to_le_bytes()); // made by
        common(&mut central);
        central.extend(0u16.to_le_bytes()); // comment
        central.extend(0u16.to_le_bytes()); // disk
        central.extend(0u16.to_le_bytes()); // internal attributes
        central.extend(0u32.to_le_bytes()); // external attributes
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let directory_offset = out.len() as u32;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0; 4]);
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(directory_offset.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

pub fn stored(files: &[(&str, &[u8])]) -> Vec<u8> {
    zip(files, 0)
}

pub fn deflated(files: &[(&str, &[u8])]) -> Vec<u8> {
    zip(files, 6)
}

/// A ZIP64 archive: every size and offset 0xFFFFFFFF in the headers, the values in ZIP64 extra fields and end records.
pub fn zip64(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut parts = Vec::new();
    let mut central = Vec::new();
    let mut offset = 0u64;
    for (name, data) in files {
        let mut local = vec![0u8; 30];
        local[0..4].copy_from_slice(&0x0403_4b50u32.to_le_bytes());
        local[4..6].copy_from_slice(&45u16.to_le_bytes());
        local[18..22].copy_from_slice(&(data.len() as u32).to_le_bytes());
        local[22..26].copy_from_slice(&(data.len() as u32).to_le_bytes());
        local[26..28].copy_from_slice(&(name.len() as u16).to_le_bytes());
        parts.extend(local);
        parts.extend(name.as_bytes());
        parts.extend(*data);

        let mut extra = vec![0u8; 28];
        extra[0..2].copy_from_slice(&1u16.to_le_bytes());
        extra[2..4].copy_from_slice(&24u16.to_le_bytes());
        extra[4..12].copy_from_slice(&(data.len() as u64).to_le_bytes());
        extra[12..20].copy_from_slice(&(data.len() as u64).to_le_bytes());
        extra[20..28].copy_from_slice(&offset.to_le_bytes());
        let mut header = vec![0u8; 46];
        header[0..4].copy_from_slice(&0x0201_4b50u32.to_le_bytes());
        header[4..6].copy_from_slice(&45u16.to_le_bytes());
        header[6..8].copy_from_slice(&45u16.to_le_bytes());
        header[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
        header[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
        header[28..30].copy_from_slice(&(name.len() as u16).to_le_bytes());
        header[30..32].copy_from_slice(&28u16.to_le_bytes());
        header[42..46].copy_from_slice(&u32::MAX.to_le_bytes());
        central.extend(header);
        central.extend(name.as_bytes());
        central.extend(extra);
        offset += 30 + name.len() as u64 + data.len() as u64;
    }
    let count = files.len() as u64;
    let mut end64 = vec![0u8; 56];
    end64[0..4].copy_from_slice(&0x0606_4b50u32.to_le_bytes());
    end64[4..12].copy_from_slice(&44u64.to_le_bytes());
    end64[24..32].copy_from_slice(&count.to_le_bytes());
    end64[32..40].copy_from_slice(&count.to_le_bytes());
    end64[40..48].copy_from_slice(&(central.len() as u64).to_le_bytes());
    end64[48..56].copy_from_slice(&offset.to_le_bytes());
    let mut locator = vec![0u8; 20];
    locator[0..4].copy_from_slice(&0x0706_4b50u32.to_le_bytes());
    locator[8..16].copy_from_slice(&(offset + central.len() as u64).to_le_bytes());
    locator[16..20].copy_from_slice(&1u32.to_le_bytes());
    let mut end = vec![0u8; 22];
    end[0..4].copy_from_slice(&0x0605_4b50u32.to_le_bytes());
    end[8..10].copy_from_slice(&0xffffu16.to_le_bytes());
    end[10..12].copy_from_slice(&0xffffu16.to_le_bytes());
    end[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    end[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
    concat(&[&parts, &central, &end64, &locator, &end])
}

/// Patches the central directory record of entry `index` in a zip written by [`zip`]: `field` is its byte offset.
pub fn patch_central(file: &[u8], index: usize, field: usize, value: u32, width: usize) -> Vec<u8> {
    let mut out = file.to_vec();
    let len = out.len();
    let u16_at = |out: &[u8], at: usize| usize::from(u16::from_le_bytes([out[at], out[at + 1]]));
    let mut at = u32::from_le_bytes(out[len - 6..len - 2].try_into().unwrap()) as usize;
    for _ in 0..index {
        at += 46 + u16_at(&out, at + 28) + u16_at(&out, at + 30) + u16_at(&out, at + 32);
    }
    if width == 2 {
        out[at + field..at + field + 2].copy_from_slice(&(value as u16).to_le_bytes());
    } else {
        out[at + field..at + field + 4].copy_from_slice(&value.to_le_bytes());
    }
    out
}

/// Classic TIFF files for the decoder's tests: strips first, then the page's tags (as macOS writes them).
#[derive(Clone)]
pub struct TiffSpec {
    pub width: u32,
    pub height: u32,
    /// One entry per strip, already compressed.
    pub strips: Vec<Vec<u8>>,
    pub rows_per_strip: Option<u32>,
    pub bits: Vec<u16>,
    pub photometric: u16,
    pub compression: u16,
    pub predictor: Option<u16>,
    /// Tag 338: 1 associated (premultiplied), 2 unassociated.
    pub extra_samples: Option<u16>,
    pub color_map: Option<Vec<u16>>,
    /// More SHORT tags, by number.
    pub shorts: Vec<(u16, Vec<u16>)>,
    pub little: bool,
    /// Where the page's next-IFD pointer points, given the page's offset; 0 ends the chain.
    pub next: fn(u32) -> u32,
}

impl TiffSpec {
    pub fn new(width: u32, height: u32, strips: Vec<Vec<u8>>) -> Self {
        Self {
            width,
            height,
            strips,
            rows_per_strip: None,
            bits: vec![8, 8, 8],
            photometric: 2,
            compression: 1,
            predictor: None,
            extra_samples: None,
            color_map: None,
            shorts: Vec::new(),
            little: true,
            next: |_| 0,
        }
    }
}

pub fn tiff_file(spec: &TiffSpec) -> Vec<u8> {
    let little = spec.little;
    let u16b = |n: u16| {
        if little {
            n.to_le_bytes()
        } else {
            n.to_be_bytes()
        }
    };
    let u32b = |n: u32| {
        if little {
            n.to_le_bytes()
        } else {
            n.to_be_bytes()
        }
    };
    let mut strip_offsets = Vec::new();
    let mut at = 8u32;
    for strip in &spec.strips {
        strip_offsets.push(at);
        at += strip.len() as u32;
    }
    // Values too long for an entry follow the strips.
    let mut extra: Vec<u8> = Vec::new();
    enum Values {
        Short(Vec<u16>),
        Long(Vec<u32>),
    }
    let mut entries: Vec<(u16, Values)> = vec![
        (256, Values::Long(vec![spec.width])),
        (257, Values::Long(vec![spec.height])),
        (258, Values::Short(spec.bits.clone())),
        (259, Values::Short(vec![spec.compression])),
        (262, Values::Short(vec![spec.photometric])),
        (273, Values::Long(strip_offsets)),
        (277, Values::Short(vec![spec.bits.len() as u16])),
        (
            278,
            Values::Long(vec![spec.rows_per_strip.unwrap_or(spec.height)]),
        ),
        (
            279,
            Values::Long(spec.strips.iter().map(|s| s.len() as u32).collect()),
        ),
    ];
    if let Some(predictor) = spec.predictor {
        entries.push((317, Values::Short(vec![predictor])));
    }
    if let Some(map) = &spec.color_map {
        entries.push((320, Values::Short(map.clone())));
    }
    if let Some(extra_samples) = spec.extra_samples {
        entries.push((338, Values::Short(vec![extra_samples])));
    }
    for (tag, values) in &spec.shorts {
        entries.push((*tag, Values::Short(values.clone())));
    }
    entries.sort_by_key(|(tag, _)| *tag);
    let mut fields = Vec::new();
    for (tag, values) in &entries {
        let (field_type, count, bytes): (u16, usize, Vec<u8>) = match values {
            Values::Short(values) => (
                3,
                values.len(),
                values.iter().flat_map(|&v| u16b(v)).collect(),
            ),
            Values::Long(values) => (
                4,
                values.len(),
                values.iter().flat_map(|&v| u32b(v)).collect(),
            ),
        };
        fields.extend(u16b(*tag));
        fields.extend(u16b(field_type));
        fields.extend(u32b(count as u32));
        if bytes.len() <= 4 {
            let mut inline = bytes.clone();
            inline.resize(4, 0);
            fields.extend(inline);
        } else {
            fields.extend(u32b(at));
            at += bytes.len() as u32;
            extra.extend(bytes);
        }
    }
    let ifd = at;
    let next = (spec.next)(ifd);
    let mut out = Vec::new();
    out.extend(if little { *b"II" } else { *b"MM" });
    out.extend(u16b(42));
    out.extend(u32b(ifd));
    for strip in &spec.strips {
        out.extend(strip);
    }
    out.extend(extra);
    out.extend(u16b(entries.len() as u16));
    out.extend(fields);
    out.extend(u32b(next));
    out
}

/// 24 × 16 RGB pixels.
pub fn gradient_rgb() -> Vec<u8> {
    (0..24 * 16u32)
        .flat_map(|i| {
            [
                (i * 3 % 256) as u8,
                (i * 7 % 256) as u8,
                (255 - i % 256) as u8,
            ]
        })
        .collect()
}

pub fn gradient_rgba() -> Vec<u8> {
    gradient_rgb()
        .chunks(3)
        .flat_map(|p| [p[0], p[1], p[2], 255])
        .collect()
}

/// The gradient as the `tiff` crate writes it: an encoder other than the decoder under test.
pub fn tiff_encoded(
    compression: tiff::encoder::Compression,
    predictor: tiff::encoder::Predictor,
) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    tiff::encoder::TiffEncoder::new(&mut out)
        .unwrap()
        .with_compression(compression)
        .with_predictor(predictor)
        .write_image::<tiff::encoder::colortype::RGB8>(24, 16, &gradient_rgb())
        .unwrap();
    out.into_inner()
}

/// The gradient in every compression the decoder reads, LZW and Deflate also with the horizontal predictor.
pub fn tiff_encodings() -> Vec<(&'static str, Vec<u8>)> {
    use tiff::encoder::{Compression, DeflateLevel, Predictor};
    vec![
        ("lzw", tiff_encoded(Compression::Lzw, Predictor::None)),
        (
            "lzwpred",
            tiff_encoded(Compression::Lzw, Predictor::Horizontal),
        ),
        (
            "deflate",
            tiff_encoded(
                Compression::Deflate(DeflateLevel::Balanced),
                Predictor::Horizontal,
            ),
        ),
        (
            "packbits",
            tiff_encoded(Compression::Packbits, Predictor::None),
        ),
        (
            "none",
            tiff_encoded(Compression::Uncompressed, Predictor::None),
        ),
    ]
}

/// A BMP with a 40-byte header (plus masks or a palette) and `rows` of raw pixel bytes, padded to 4 bytes.
pub struct BmpSpec {
    /// 40, with the masks after the header; 108 (V4) or 124 (V5), with them inside it.
    pub header_size: u32,
    pub width: i32,
    pub height: i32,
    pub bit_count: u16,
    pub compression: u32,
    pub masks: Vec<u32>,
    pub palette: Vec<[u8; 3]>,
    pub rows: Vec<Vec<u8>>,
}

impl BmpSpec {
    pub fn new(width: i32, height: i32, bit_count: u16, rows: Vec<Vec<u8>>) -> Self {
        Self {
            header_size: 40,
            width,
            height,
            bit_count,
            compression: 0,
            masks: Vec::new(),
            palette: Vec::new(),
            rows,
        }
    }
}

pub fn bmp_file(spec: &BmpSpec) -> Vec<u8> {
    let header = spec.header_size as usize;
    let masks = if header == 40 {
        spec.masks.len() * 4
    } else {
        0
    };
    let extra = masks + spec.palette.len() * 4;
    let stride = (spec.width as usize * usize::from(spec.bit_count)).div_ceil(32) * 4;
    let offset = 14 + header + extra;
    let mut bytes = vec![0u8; offset + stride * spec.height.unsigned_abs() as usize];
    let len = bytes.len() as u32;
    bytes[0..2].copy_from_slice(b"BM");
    bytes[2..6].copy_from_slice(&len.to_le_bytes());
    bytes[10..14].copy_from_slice(&(offset as u32).to_le_bytes());
    bytes[14..18].copy_from_slice(&spec.header_size.to_le_bytes());
    bytes[18..22].copy_from_slice(&spec.width.to_le_bytes());
    bytes[22..26].copy_from_slice(&spec.height.to_le_bytes());
    bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
    bytes[28..30].copy_from_slice(&spec.bit_count.to_le_bytes());
    bytes[30..34].copy_from_slice(&spec.compression.to_le_bytes());
    bytes[46..50].copy_from_slice(&(spec.palette.len() as u32).to_le_bytes());
    let mut at = 54;
    for mask in &spec.masks {
        bytes[at..at + 4].copy_from_slice(&mask.to_le_bytes());
        at += 4;
    }
    let mut at = 14 + header + masks;
    for [r, g, b] in &spec.palette {
        bytes[at..at + 4].copy_from_slice(&[*b, *g, *r, 0]);
        at += 4;
    }
    for (index, row) in spec.rows.iter().enumerate() {
        let start = offset + index * stride;
        bytes[start..start + row.len()].copy_from_slice(row);
    }
    bytes
}

/// A 1 × 1 24-bit BMP holding one red pixel.
pub fn red_bmp() -> Vec<u8> {
    let mut bmp = vec![0u8; 58];
    bmp[0..2].copy_from_slice(b"BM");
    bmp[2..6].copy_from_slice(&58u32.to_le_bytes());
    bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
    bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
    bmp[18..22].copy_from_slice(&1i32.to_le_bytes());
    bmp[22..26].copy_from_slice(&1i32.to_le_bytes());
    bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
    bmp[28..30].copy_from_slice(&24u16.to_le_bytes());
    bmp[54..57].copy_from_slice(&[0, 0, 255]);
    bmp
}

fn iso_box(kind: &[u8; 4], children: &[&[u8]]) -> Vec<u8> {
    let body = children.concat();
    let mut out = ((8 + body.len()) as u32).to_be_bytes().to_vec();
    out.extend(kind);
    out.extend(body);
    out
}

/// A minimal ISO media file: one track with this handler and sample entry, `moov` before or after `mdat`, and a
/// movie header stating a length in milliseconds (or none).
pub fn movie(
    handler: &[u8; 4],
    codec: &[u8; 4],
    moov_first: bool,
    duration_ms: Option<u32>,
) -> Vec<u8> {
    let stsd = iso_box(
        b"stsd",
        &[&[0; 4], &[0, 0, 0, 1], &[0, 0, 0, 16], codec, &[0; 4]],
    );
    let hdlr = iso_box(b"hdlr", &[&[0; 8], handler, &[0; 12]]);
    let stbl = iso_box(b"stbl", &[&stsd]);
    let minf = iso_box(b"minf", &[&stbl]);
    let mdia = iso_box(b"mdia", &[&hdlr, &minf]);
    let tkhd = iso_box(b"tkhd", &[&[0; 8]]);
    let trak = iso_box(b"trak", &[&tkhd, &mdia]);
    let mvhd = match duration_ms {
        None => iso_box(b"mvhd", &[&[0; 8]]),
        Some(ms) => iso_box(
            b"mvhd",
            &[&[0; 12], &1000u32.to_be_bytes(), &ms.to_be_bytes()],
        ),
    };
    let moov = iso_box(b"moov", &[&mvhd, &trak]);
    let ftyp = iso_box(b"ftyp", &[b"isom", &[0; 4]]);
    let mdat = iso_box(b"mdat", &[&[0; 1000]]);
    if moov_first {
        concat(&[&ftyp, &moov, &mdat])
    } else {
        concat(&[&ftyp, &mdat, &moov])
    }
}

/// A seeded PRNG (mulberry32), so fuzz runs are reproducible from their seed.
pub struct Seeded(u32);

impl Seeded {
    pub fn new(seed: u32) -> Self {
        Self(seed)
    }

    /// A number in [0, 1).
    pub fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6d2b_79f5);
        let mut t = self.0;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() * n as f64) as usize
    }
}

/// One structure-blind mutation: a truncation, a byte flip, a run of random bytes, or a duplicated slice.
pub fn mutate(bytes: &[u8], random: &mut Seeded) -> Vec<u8> {
    let at = random.below(bytes.len());
    let mut out = bytes.to_vec();
    match random.below(4) {
        0 => out.truncate(at),
        1 => {
            let bit = random.below(8);
            if let Some(byte) = out.get_mut(at) {
                *byte ^= 1 << bit;
            }
        }
        2 => {
            for i in at..out.len().min(at + 8) {
                out[i] = random.below(256) as u8;
            }
        }
        _ => {
            let length = random.below(64);
            let copied = length.min(out.len() - at);
            let mut copy = vec![0u8; out.len() + length];
            copy[..at].copy_from_slice(&out[..at]);
            copy[at..at + copied].copy_from_slice(&out[at..at + copied]);
            copy[at + copied..at + copied + out.len() - at].copy_from_slice(&out[at..]);
            return copy;
        }
    }
    out
}

/// Fuzz iterations per test, from `FUZZ_ITERATIONS` (default 2000).
pub fn fuzz_iterations() -> usize {
    std::env::var("FUZZ_ITERATIONS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2000)
}
