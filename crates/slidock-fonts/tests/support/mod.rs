//! Minimal TrueType bytes with just the tables the sfnt reader reads, so tests build their fonts in code, and the
//! seeded mutations of the fuzz tests.

#![allow(dead_code, reason = "each test file uses some of the builders")]

pub fn u16be(n: u16) -> [u8; 2] {
    n.to_be_bytes()
}

pub fn u32be(n: u32) -> [u8; 4] {
    n.to_be_bytes()
}

fn utf16be(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_be_bytes).collect()
}

/// One face. `FaceSpec { weight: Some(500), ..FaceSpec::new("Roboto") }`.
#[derive(Clone, Debug, Default)]
pub struct FaceSpec {
    pub family: String,
    pub subfamily: Option<String>,
    pub postscript_name: Option<String>,
    /// Name id 16, written in addition to id 1.
    pub typographic_family: Option<String>,
    pub weight: Option<u16>,
    /// `OS/2.usWidthClass`; 5 (normal) when unset.
    pub width: Option<u16>,
    pub italic: bool,
    pub units_per_em: Option<u16>,
    pub hhea: Option<[i16; 3]>,
    pub typo: Option<[i16; 3]>,
    pub win: Option<[u16; 2]>,
    pub use_typo_metrics: bool,
    pub variable: bool,
    /// Leave the OS/2 table out.
    pub no_os2: bool,
    /// head yMin and yMax.
    pub bounds: Option<[i16; 2]>,
    /// Adds cmap, maxp, loca and glyf tables with an À glyph whose box tops out here.
    pub grave_top: Option<i16>,
    /// `OS/2.fsType`.
    pub fs_type: u16,
    /// `OS/2.fsSelection` bit 5 and `head.macStyle` bit 0: the face calls itself bold.
    pub bold_bits: bool,
    /// Adds cmap, maxp and hmtx tables mapping U+0020 onward to glyphs 1, 2, … with these advances (glyph 0 advances
    /// by 0). Not combined with `grave_top`.
    pub advances: Option<Vec<u16>>,
}

impl FaceSpec {
    pub fn new(family: &str) -> FaceSpec {
        FaceSpec {
            family: family.to_owned(),
            ..FaceSpec::default()
        }
    }
}

/// A name record: platform, encoding, language, name id and the string's bytes.
pub type NameRecord = (u16, u16, u16, u16, Vec<u8>);

/// A Windows Unicode English record.
pub fn windows_name(id: u16, value: &str) -> NameRecord {
    (3, 1, 0x409, id, utf16be(value))
}

pub fn name_table(records: &[NameRecord]) -> Vec<u8> {
    let mut strings: Vec<u8> = Vec::new();
    let mut directory = Vec::new();
    for (platform, encoding, language, id, bytes) in records {
        for field in [
            *platform,
            *encoding,
            *language,
            *id,
            bytes.len() as u16,
            strings.len() as u16,
        ] {
            directory.extend(u16be(field));
        }
        strings.extend(bytes);
    }
    let mut table = Vec::new();
    table.extend(u16be(0));
    table.extend(u16be(records.len() as u16));
    table.extend(u16be(6 + directory.len() as u16));
    table.extend(directory);
    table.extend(strings);
    table
}

fn put(table: &mut [u8], at: usize, bytes: &[u8]) {
    table[at..at + bytes.len()].copy_from_slice(bytes);
}

/// The face's tables in tag order.
pub fn face_tables(spec: &FaceSpec) -> Vec<([u8; 4], Vec<u8>)> {
    let [ascender, descender, line_gap] = spec.hhea.unwrap_or([1854, -434, 67]);
    let typo = spec.typo.unwrap_or([1491, -431, 307]);
    let win = spec.win.unwrap_or([1854, 434]);
    let mut head = vec![0; 54];
    put(&mut head, 18, &u16be(spec.units_per_em.unwrap_or(2048)));
    put(
        &mut head,
        44,
        &u16be(u16::from(spec.bold_bits) | if spec.italic { 0x02 } else { 0 }),
    );
    if let Some([y_min, y_max]) = spec.bounds {
        put(&mut head, 38, &y_min.to_be_bytes());
        put(&mut head, 42, &y_max.to_be_bytes());
    }
    let mut hhea = vec![0; 36];
    put(&mut hhea, 4, &ascender.to_be_bytes());
    put(&mut hhea, 6, &descender.to_be_bytes());
    put(&mut hhea, 8, &line_gap.to_be_bytes());
    let mut os2 = vec![0; 96];
    put(&mut os2, 4, &u16be(spec.weight.unwrap_or(400)));
    put(&mut os2, 6, &u16be(spec.width.unwrap_or(5)));
    put(&mut os2, 8, &u16be(spec.fs_type));
    let fs_selection = u16::from(spec.italic)
        | if spec.bold_bits { 0x20 } else { 0 }
        | if spec.use_typo_metrics { 0x80 } else { 0 };
    put(&mut os2, 62, &u16be(fs_selection));
    put(&mut os2, 68, &typo[0].to_be_bytes());
    put(&mut os2, 70, &typo[1].to_be_bytes());
    put(&mut os2, 72, &typo[2].to_be_bytes());
    put(&mut os2, 74, &u16be(win[0]));
    put(&mut os2, 76, &u16be(win[1]));
    let mut post = vec![0; 32];
    put(
        &mut post,
        4,
        &(if spec.italic { -12 * 65536 } else { 0i32 }).to_be_bytes(),
    );
    let subfamily = spec.subfamily.as_deref();
    let mut names = vec![
        windows_name(1, &spec.family),
        windows_name(
            2,
            subfamily.unwrap_or(if spec.italic { "Italic" } else { "Regular" }),
        ),
        windows_name(
            4,
            &format!("{} {}", spec.family, subfamily.unwrap_or("Regular")),
        ),
        windows_name(
            6,
            &spec.postscript_name.clone().unwrap_or_else(|| {
                let stem: String = spec.family.chars().filter(|c| !c.is_whitespace()).collect();
                format!("{stem}-{}", subfamily.unwrap_or("Regular"))
            }),
        ),
    ];
    if let Some(typographic) = &spec.typographic_family {
        names.push(windows_name(16, typographic));
    }
    let mut tables = vec![
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"name", name_table(&names)),
        (*b"post", post),
    ];
    if !spec.no_os2 {
        tables.push((*b"OS/2", os2));
    }
    if let Some(top) = spec.grave_top {
        tables.extend(grave_glyph_tables(top));
    }
    if let Some(advances) = &spec.advances {
        let glyphs = advances.len() as u16 + 1;
        put(
            &mut tables.iter_mut().find(|(tag, _)| tag == b"hhea").unwrap().1,
            34,
            &u16be(glyphs),
        );
        tables.extend(advance_tables(advances));
    }
    if spec.variable {
        tables.push((*b"fvar", vec![0; 16]));
    }
    tables.sort_by_key(|a| a.0);
    tables
}

/// A format 4 subtable mapping U+00C0 to `glyph` through its delta.
pub fn format4_grave(glyph: u16) -> Vec<u8> {
    let mut sub = Vec::new();
    for field in [4u16, 32, 0, 4, 4, 1, 0, 0xc0, 0xffff, 0, 0xc0, 0xffff] {
        sub.extend(u16be(field));
    }
    for field in [glyph.wrapping_sub(0xc0), 1, 0, 0] {
        sub.extend(u16be(field));
    }
    sub
}

/// A cmap with one subtable record per `(platform, encoding, subtable)`.
pub fn cmap_table(subtables: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
    let mut table = Vec::new();
    table.extend(u16be(0));
    table.extend(u16be(subtables.len() as u16));
    let mut offset = 4 + 8 * subtables.len();
    for (platform, encoding, sub) in subtables {
        table.extend(u16be(*platform));
        table.extend(u16be(*encoding));
        table.extend(u32be(offset as u32));
        offset += sub.len();
    }
    for (_, _, sub) in subtables {
        table.extend(sub);
    }
    table
}

/// A format 4 cmap mapping U+0020 onward to glyphs 1, 2, … (one per advance), and the maxp and hmtx tables that
/// give each glyph its advance.
pub fn advance_tables(advances: &[u16]) -> Vec<([u8; 4], Vec<u8>)> {
    let count = advances.len() as u16;
    let last = 0x20 + count - 1;
    let mut sub = Vec::new();
    for field in [4u16, 32, 0, 4, 4, 1, 0, last, 0xffff, 0, 0x20, 0xffff] {
        sub.extend(u16be(field));
    }
    for field in [1u16.wrapping_sub(0x20), 1, 0, 0] {
        sub.extend(u16be(field));
    }
    let mut maxp = Vec::new();
    maxp.extend(u32be(0x0000_5000));
    maxp.extend(u16be(count + 1));
    let mut hmtx = Vec::new();
    for advance in std::iter::once(0).chain(advances.iter().copied()) {
        hmtx.extend(u16be(advance));
        hmtx.extend(u16be(0));
    }
    vec![
        (*b"cmap", cmap_table(&[(3, 1, sub)])),
        (*b"hmtx", hmtx),
        (*b"maxp", maxp),
    ]
}

/// A one-glyph-plus-notdef font: a format 4 cmap mapping U+00C0 to glyph 1, short loca and a glyph header.
pub fn grave_glyph_tables(top: i16) -> Vec<([u8; 4], Vec<u8>)> {
    let cmap = cmap_table(&[(3, 1, format4_grave(1))]);
    let mut maxp = Vec::new();
    maxp.extend(u32be(0x0000_5000));
    maxp.extend(u16be(2));
    let mut glyph = Vec::new();
    for field in [1i16, 0, 0, 1000, top] {
        glyph.extend(field.to_be_bytes());
    }
    let mut loca = Vec::new();
    for field in [0u16, 0, (glyph.len() / 2) as u16] {
        loca.extend(u16be(field));
    }
    vec![
        (*b"cmap", cmap),
        (*b"glyf", glyph),
        (*b"loca", loca),
        (*b"maxp", maxp),
    ]
}

/// A table directory whose tables start at `data_start`, and the padded table data.
fn layout(tables: &[([u8; 4], Vec<u8>)], data_start: usize) -> (Vec<u8>, Vec<u8>) {
    let mut directory = Vec::new();
    directory.extend(u32be(0x0001_0000));
    directory.extend(u16be(tables.len() as u16));
    directory.extend([0; 6]);
    let mut data = Vec::new();
    for (tag, bytes) in tables {
        directory.extend(tag);
        directory.extend(u32be(0));
        directory.extend(u32be((data_start + data.len()) as u32));
        directory.extend(u32be(bytes.len() as u32));
        data.extend(bytes);
        data.resize(data.len().next_multiple_of(4), 0);
    }
    (directory, data)
}

/// A font file of these tables, in the order given.
pub fn assemble(tables: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let (mut directory, data) = layout(tables, 12 + tables.len() * 16);
    directory.extend(data);
    directory
}

pub fn sfnt(spec: &FaceSpec) -> Vec<u8> {
    assemble(&face_tables(spec))
}

pub fn ttc(specs: &[FaceSpec]) -> Vec<u8> {
    let header = 12 + specs.len() * 4;
    let faces: Vec<_> = specs.iter().map(face_tables).collect();
    let directory_sizes: Vec<usize> = faces.iter().map(|tables| 12 + tables.len() * 16).collect();
    let mut data_start = header + directory_sizes.iter().sum::<usize>();
    let mut directories = Vec::new();
    let mut datas = Vec::new();
    let mut offsets = Vec::new();
    let mut directory_at = header;
    for (tables, size) in faces.iter().zip(&directory_sizes) {
        let (directory, data) = layout(tables, data_start);
        offsets.push(directory_at as u32);
        directory_at += size;
        directories.extend(directory);
        data_start += data.len();
        datas.extend(data);
    }
    let mut out = Vec::new();
    out.extend(b"ttcf");
    out.extend(u32be(0x0001_0000));
    out.extend(u32be(specs.len() as u32));
    for offset in offsets {
        out.extend(u32be(offset));
    }
    out.extend(directories);
    out.extend(datas);
    out
}

/// The offset and length of a table in a single font file.
pub fn table_range(font: &[u8], tag: &[u8; 4]) -> Option<(usize, usize)> {
    let count = u16::from_be_bytes([font[4], font[5]]) as usize;
    (0..count).find_map(|i| {
        let record = 12 + i * 16;
        (&font[record..record + 4] == tag).then(|| {
            let at = |o: usize| u32::from_be_bytes(font[o..o + 4].try_into().unwrap()) as usize;
            (at(record + 8), at(record + 12))
        })
    })
}

/// `font` with the big-endian u16 at `offset` of table `tag` set to `value` (checksums are left stale: no reader
/// here checks them).
pub fn with_u16(font: &[u8], tag: &[u8; 4], offset: usize, value: u16) -> Vec<u8> {
    let (start, _) = table_range(font, tag).expect("the table");
    let mut out = font.to_vec();
    out[start + offset..start + offset + 2].copy_from_slice(&value.to_be_bytes());
    out
}

/// `font` with `OS/2.fsType` set.
pub fn with_fs_type(font: &[u8], fs_type: u16) -> Vec<u8> {
    with_u16(font, b"OS/2", 8, fs_type)
}

/// A collection of single font files: their directories after the header, table offsets moved by where each
/// file's data lands.
pub fn collection(fonts: &[&[u8]]) -> Vec<u8> {
    let header = 12 + fonts.len() * 4;
    let mut out = Vec::new();
    out.extend(b"ttcf");
    out.extend(u32be(0x0001_0000));
    out.extend(u32be(fonts.len() as u32));
    let mut placed = header;
    let mut bodies = Vec::new();
    for font in fonts {
        out.extend(u32be(placed as u32));
        bodies.push(placed);
        placed += font.len().next_multiple_of(4);
    }
    for (font, base) in fonts.iter().zip(bodies) {
        let mut copy = font.to_vec();
        let count = u16::from_be_bytes([copy[4], copy[5]]) as usize;
        for i in 0..count {
            let at = 12 + i * 16 + 8;
            let offset = u32::from_be_bytes(copy[at..at + 4].try_into().unwrap());
            copy[at..at + 4].copy_from_slice(&u32be(offset + base as u32));
        }
        copy.resize(copy.len().next_multiple_of(4), 0);
        out.extend(copy);
    }
    out
}

/// `font` rebuilt with the tables `edits` names replaced (`Some`) or removed (`None`); a tag it lacks is added.
pub fn with_tables(font: &[u8], edits: &[(&[u8; 4], Option<Vec<u8>>)]) -> Vec<u8> {
    use read_fonts::types::Tag;
    let source = read_fonts::FontRef::new(font).unwrap();
    let mut builder = write_fonts::FontBuilder::new();
    for record in source.table_directory().table_records() {
        let tag = record.tag();
        if edits.iter().all(|(edited, _)| Tag::new(edited) != tag) {
            builder.add_raw(tag, source.table_data(tag).unwrap().as_bytes().to_vec());
        }
    }
    for (tag, data) in edits {
        if let Some(data) = data {
            builder.add_raw(Tag::new(tag), data.clone());
        }
    }
    builder.build()
}

/// A version 0 `kern` table of one horizontal format 0 subtable holding `pairs` (left glyph, right glyph, value),
/// which must be sorted.
pub fn kern_table(pairs: &[(u16, u16, i16)]) -> Vec<u8> {
    let count = pairs.len() as u16;
    let power = if count == 0 {
        0
    } else {
        1u16 << (15 - count.leading_zeros())
    };
    let mut subtable = Vec::new();
    subtable.extend(u16be(0));
    subtable.extend(u16be((14 + 6 * pairs.len()) as u16));
    subtable.extend(u16be(0x0001));
    subtable.extend(u16be(count));
    subtable.extend(u16be(power * 6));
    subtable.extend(u16be(power.max(1).trailing_zeros() as u16));
    subtable.extend(u16be((count - power) * 6));
    for &(left, right, value) in pairs {
        subtable.extend(u16be(left));
        subtable.extend(u16be(right));
        subtable.extend(value.to_be_bytes());
    }
    let mut table = Vec::new();
    table.extend(u16be(0));
    table.extend(u16be(1));
    table.extend(subtable);
    table
}

/// The glyph `font` maps `c` to.
pub fn glyph_of(font: &[u8], c: char) -> u32 {
    use skrifa::MetadataProvider;
    let font = read_fonts::FontRef::new(font).unwrap();
    font.charmap().map(c).unwrap().to_u32()
}

/// `font` kerned only by a legacy `kern` table: its `GPOS` dropped, one A–V pair of -120 added.
pub fn kern_only(font: &[u8]) -> Vec<u8> {
    let (a, v) = (glyph_of(font, 'A') as u16, glyph_of(font, 'V') as u16);
    let pairs = if a < v {
        [(a, v, -120)]
    } else {
        [(v, a, -120)]
    };
    with_tables(
        font,
        &[(b"GPOS", None), (b"kern", Some(kern_table(&pairs)))],
    )
}

/// `font` with its `name` table replaced by Macintosh-platform (Roman, English) copies of its Windows English
/// records that are ASCII.
pub fn mac_names_only(font: &[u8]) -> Vec<u8> {
    use read_fonts::TableProvider;
    let source = read_fonts::FontRef::new(font).unwrap();
    let name = source.name().unwrap();
    let records: Vec<NameRecord> = name
        .name_record()
        .iter()
        .filter(|record| record.platform_id() == 3 && record.language_id() == 0x409)
        .filter_map(|record| {
            let text: String = record.string(name.string_data()).ok()?.chars().collect();
            text.is_ascii()
                .then(|| (1, 0, 0, record.name_id().to_u16(), text.into_bytes()))
        })
        .collect();
    assert!(!records.is_empty());
    with_tables(font, &[(b"name", Some(name_table(&records)))])
}

/// `font` with every `cmap` encoding record moved to the Macintosh platform, which browsers do not read.
pub fn mac_cmap_only(font: &[u8]) -> Vec<u8> {
    let (start, _) = table_range(font, b"cmap").unwrap();
    let mut out = font.to_vec();
    let count = u16::from_be_bytes([out[start + 2], out[start + 3]]) as usize;
    for i in 0..count {
        let record = start + 4 + i * 8;
        out[record..record + 4].copy_from_slice(&[0, 1, 0, 0]);
    }
    out
}

/// `font` with `extra` more directory records, each covering the whole file: tables that overlap.
pub fn with_overlapping_records(font: &[u8], extra: usize, length: u32) -> Vec<u8> {
    let count = u16::from_be_bytes([font[4], font[5]]) as usize;
    let shift = (extra * 16) as u32;
    let mut out = font[..12].to_vec();
    out[4..6].copy_from_slice(&u16be((count + extra) as u16));
    for i in 0..count {
        let record = &font[12 + i * 16..28 + i * 16];
        out.extend(&record[..8]);
        let offset = u32::from_be_bytes(record[8..12].try_into().unwrap());
        out.extend(u32be(offset + shift));
        out.extend(&record[12..]);
    }
    for i in 0..extra {
        out.extend(format!("Z{:03}", i % 1000).as_bytes());
        out.extend(u32be(0));
        out.extend(u32be(0));
        out.extend(u32be(length));
    }
    out.extend(&font[12 + count * 16..]);
    out
}

/// A seeded PRNG (mulberry32), so fuzz runs are reproducible from their seed.
pub struct Seeded(u32);

impl Seeded {
    pub fn new(seed: u32) -> Seeded {
        Seeded(seed)
    }

    /// A float in [0, 1).
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
        0 => bytes[..at].to_vec(),
        1 => {
            out[at] ^= 1 << random.below(8);
            out
        }
        2 => {
            for byte in out.iter_mut().skip(at).take(8) {
                *byte = random.below(256) as u8;
            }
            out
        }
        _ => {
            let length = random.below(64);
            let end = (at + length).min(bytes.len());
            let mut copy = bytes[..at].to_vec();
            copy.extend(&bytes[at..end]);
            copy.extend(&bytes[at..]);
            copy.resize(bytes.len() + length, 0);
            copy
        }
    }
}

pub fn fuzz_iterations() -> usize {
    std::env::var("FUZZ_ITERATIONS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(2000)
}

/// The committed OFL test fonts (`tests/fonts`, with their licences and sources).
pub mod fixtures {
    pub const OSWALD: &[u8] = include_bytes!("../fonts/oswald/Oswald-Regular.ttf");
    pub const OSWALD_CFF: &[u8] = include_bytes!("../fonts/oswald/Oswald-Regular.otf");
    pub const OSWALD_VARIABLE: &[u8] = include_bytes!("../fonts/oswald/Oswald-Variable.ttf");
    pub const OSWALD_MVAR: &[u8] = include_bytes!("../fonts/oswald/Oswald-Variable-MVAR.ttf");
    pub const BARLOW_ITALIC: &[u8] =
        include_bytes!("../fonts/barlow-condensed/BarlowCondensed-Italic.ttf");
    pub const ROBOTO_MONO: &[u8] = include_bytes!("../fonts/roboto-mono/RobotoMono-Regular.ttf");
    pub const SILKSCREEN: &[u8] = include_bytes!("../fonts/silkscreen/Silkscreen-Regular.ttf");
    pub const NOTO_HEBREW: &[u8] =
        include_bytes!("../fonts/noto-sans-hebrew/NotoSansHebrew-Variable.ttf");

    /// Every fixture, by file name.
    pub const ALL: [(&str, &[u8]); 8] = [
        ("Oswald-Regular.ttf", OSWALD),
        ("Oswald-Regular.otf", OSWALD_CFF),
        ("Oswald-Variable.ttf", OSWALD_VARIABLE),
        ("Oswald-Variable-MVAR.ttf", OSWALD_MVAR),
        ("BarlowCondensed-Italic.ttf", BARLOW_ITALIC),
        ("RobotoMono-Regular.ttf", ROBOTO_MONO),
        ("Silkscreen-Regular.ttf", SILKSCREEN),
        ("NotoSansHebrew-Variable.ttf", NOTO_HEBREW),
    ];

    /// The fixtures with TrueType outlines, which subset.
    pub fn truetype() -> impl Iterator<Item = (&'static str, &'static [u8])> {
        ALL.into_iter().filter(|(name, _)| name.ends_with(".ttf"))
    }
}
