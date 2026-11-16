//! Hostile WOFF and WOFF2 files: each lie a decoder must refuse, the limits, transformed glyph data built in code,
//! and a deterministic corpus of mutated fixtures that must never panic or pass the output limit.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod container_support;

use container_support::*;
use slidock_fonts::container::{
    ContainerError, ContainerLimits, Format, GlyfStream, Hint, Part, Woff2Error, WoffError, eot,
    odttf, unwrap, unwrap_with,
};
use slidock_fonts::read_sfnt;
use slidock_fonts::sfnt::TableTag;

const TRUETYPE: u32 = 0x0001_0000;

fn decode(bytes: &[u8]) -> Result<Vec<u8>, ContainerError> {
    unwrap(bytes, &Hint::default()).map(|u| u.sfnt)
}

fn tag(name: &[u8; 4]) -> TableTag {
    TableTag(*name)
}

/// How a test font stores `hmtx`.
enum Hmtx {
    /// One advance per glyph, every left side bearing stored.
    Plain,
    /// The transformed table, as given.
    Transformed(Vec<u8>),
}

/// A WOFF2 font with a transformed `glyf` (and `loca`) of `glyphs`, plus `head`, `hhea` and `hmtx`.
fn glyph_font(glyphs: &[TGlyph], index_format: u16, hmtx: Hmtx, overlap: Option<&[u8]>) -> Vec<u8> {
    let glyf = glyf_transform(index_format, glyphs, overlap);
    let n = glyphs.len() as u32;
    let loca_length = if index_format == 1 { 4 } else { 2 } * (n + 1);
    let (hmtx_version, hmtx_data) = match hmtx {
        Hmtx::Plain => (0, vec![0u8; 4 * glyphs.len()]),
        Hmtx::Transformed(data) => (1, data),
    };
    let hmtx_entry = if hmtx_version == 1 {
        (1, *b"hmtx", 4 * n, Some(hmtx_data.len() as u32))
    } else {
        (0, *b"hmtx", hmtx_data.len() as u32, None)
    };
    let entries = [
        (0, *b"glyf", 0, Some(glyf.len() as u32)),
        (0, *b"head", 54, None),
        (0, *b"hhea", 36, None),
        hmtx_entry,
        (0, *b"loca", loca_length, Some(0)),
    ];
    let stream = [
        glyf,
        head(index_format as i16),
        hhea(glyphs.len().max(1) as u16),
        hmtx_data,
    ]
    .concat();
    woff2_raw(TRUETYPE, &entries, &stream, &[])
}

fn square(size: i32) -> TGlyph {
    TGlyph::Simple {
        contours: vec![4],
        points: vec![
            (0, 0, true),
            (size, 0, true),
            (size, size, true),
            (0, size, false),
        ],
        instructions: vec![0xb0, 0x01],
        bbox: None,
    }
}

/// A composite record: one component, glyph 0 at (dx, dy) as words.
fn component(dx: i16, dy: i16, more: bool, instructions: bool) -> Vec<u8> {
    let flags: u16 =
        0x0001 | 0x0002 | if more { 0x20 } else { 0 } | if instructions { 0x100 } else { 0 };
    [
        flags.to_be_bytes(),
        0u16.to_be_bytes(),
        dx.to_be_bytes(),
        dy.to_be_bytes(),
    ]
    .concat()
}

#[test]
fn transformed_glyphs_built_in_code_round_trip() {
    let records = [
        component(10, -20, true, true),
        component(5, 5, false, false),
    ]
    .concat();
    let built = [
        square(100),
        TGlyph::Empty { bbox: false },
        TGlyph::Composite {
            records: records.clone(),
            instructions: Some(vec![1, 2, 3]),
            bbox: Some([-5, -6, 300, 400]),
        },
        TGlyph::Simple {
            // An empty first contour ends at -1.
            contours: vec![0, 2],
            points: vec![(-300, 700, true), (-299, -32000, false)],
            instructions: vec![],
            bbox: Some([1, 2, 3, 4]),
        },
    ];
    let overlap = [0b1000_0000];
    for index_format in [0, 1] {
        let sfnt = decode(&glyph_font(
            &built,
            index_format,
            Hmtx::Plain,
            Some(&overlap),
        ))
        .unwrap();
        let models: Vec<GlyphModel> = glyphs(&sfnt, 0).iter().map(|g| glyph_model(g)).collect();
        assert_eq!(
            models,
            [
                GlyphModel::Simple {
                    bbox: [0, 0, 100, 100],
                    end_points: vec![3],
                    instructions: vec![0xb0, 0x01],
                    points: vec![
                        (0, 0, true, true),
                        (100, 0, true, false),
                        (100, 100, true, false),
                        (0, 100, false, false)
                    ],
                },
                GlyphModel::Empty,
                GlyphModel::Composite {
                    bbox: [-5, -6, 300, 400],
                    components: vec![(0x0002, 0, 10, -20, vec![]), (0x0002, 0, 5, 5, vec![])],
                    instructions: vec![1, 2, 3],
                },
                GlyphModel::Simple {
                    bbox: [1, 2, 3, 4],
                    end_points: vec![0xffff, 1],
                    instructions: vec![],
                    points: vec![(-300, 700, true, false), (-299, -32000, false, false)],
                },
            ],
            "index format {index_format}"
        );
        assert_eq!(u16_at(table(&sfnt, 0, b"head"), 50), index_format);
        assert_eq!(checksum(&sfnt), 0xb1b0_afba);
    }
}

#[test]
fn transformed_hmtx_takes_left_side_bearings_from_the_glyph_boxes() {
    // Two glyphs, numberOfHMetrics 2 (hhea of the test font): advances stored, both bearings left out.
    let glyphs = [
        square(100),
        TGlyph::Composite {
            records: component(0, 0, false, false),
            instructions: None,
            bbox: Some([-7, 0, 10, 10]),
        },
    ];
    let transformed = [
        vec![0b01],
        500u16.to_be_bytes().to_vec(),
        600u16.to_be_bytes().to_vec(),
    ]
    .concat();
    let sfnt = decode(&glyph_font(
        &glyphs,
        0,
        Hmtx::Transformed(transformed),
        None,
    ))
    .unwrap();
    let hmtx = table(&sfnt, 0, b"hmtx");
    assert_eq!(
        hmtx,
        [
            500u16.to_be_bytes(),
            0u16.to_be_bytes(),
            600u16.to_be_bytes(),
            (-7i16).to_be_bytes()
        ]
        .concat()
    );

    let bad = |flags: u8, extra: &[u8]| {
        let data = [vec![flags], vec![0; 4], extra.to_vec()].concat();
        decode(&glyph_font(&glyphs, 0, Hmtx::Transformed(data), None))
    };
    assert!(matches!(
        bad(0b100, &[]),
        Err(ContainerError::Woff2(Woff2Error::Hmtx(_)))
    ));
    assert!(matches!(
        bad(0b00, &[0; 4]),
        Err(ContainerError::Woff2(Woff2Error::Hmtx(_)))
    ));
    assert_eq!(bad(0b01, &[]).map(|_| ()), Ok(()));
    assert_eq!(
        decode(&glyph_font(
            &glyphs,
            0,
            Hmtx::Transformed(vec![0b01, 0]),
            None
        )),
        Err(ContainerError::Truncated(Part::HmtxStream))
    );
}

#[test]
fn hmtx_needs_metrics_and_a_transformed_glyf() {
    let ttf_tables = |hmtx: Vec<u8>| {
        let entries = [
            (0, *b"head", 54, None),
            (0, *b"hhea", 36, None),
            (1, *b"hmtx", 8, Some(hmtx.len() as u32)),
        ];
        let stream = [head(0), hhea(1), hmtx].concat();
        woff2_raw(TRUETYPE, &entries, &stream, &[])
    };
    assert_eq!(
        decode(&ttf_tables(vec![1, 0, 1])),
        Err(ContainerError::Woff2(Woff2Error::Hmtx(
            "the glyf table is not transformed"
        )))
    );
    // numberOfHMetrics 0.
    let glyphs = [square(10)];
    let mut font = glyph_font(&glyphs, 0, Hmtx::Transformed(vec![0b11]), None);
    let zero_metrics = {
        let entries = [
            (
                0,
                *b"glyf",
                0,
                Some(glyf_transform(0, &glyphs, None).len() as u32),
            ),
            (0, *b"head", 54, None),
            (0, *b"hhea", 36, None),
            (1, *b"hmtx", 4, Some(1)),
            (0, *b"loca", 4, Some(0)),
        ];
        let stream = [
            glyf_transform(0, &glyphs, None),
            head(0),
            hhea(0),
            vec![0b11],
        ]
        .concat();
        woff2_raw(TRUETYPE, &entries, &stream, &[])
    };
    assert_eq!(
        decode(&zero_metrics),
        Err(ContainerError::Woff2(Woff2Error::Hmtx(
            "numberOfHMetrics is out of range"
        )))
    );
    font.truncate(10);
    assert!(decode(&font).is_err());
}

#[test]
fn glyph_data_that_lies_is_refused() {
    let composite = |bbox| TGlyph::Composite {
        records: component(0, 0, false, false),
        instructions: None,
        bbox,
    };
    let font = |glyphs: &[TGlyph]| decode(&glyph_font(glyphs, 0, Hmtx::Plain, None));
    assert_eq!(
        font(&[composite(None)]),
        Err(ContainerError::Woff2(Woff2Error::CompositeWithoutBbox(0)))
    );
    assert_eq!(
        font(&[square(1), TGlyph::Empty { bbox: true }]),
        Err(ContainerError::Woff2(Woff2Error::EmptyWithBbox(1)))
    );
    let far = TGlyph::Simple {
        contours: vec![2],
        points: vec![(0, 0, true), (40_000, 0, true)],
        instructions: vec![],
        bbox: Some([0; 4]),
    };
    assert_eq!(font(&[far]), Err(ContainerError::GlyphCoordinate));
    // A delta that needs more than 16 bits, between two coordinates that fit.
    let jump = TGlyph::Simple {
        contours: vec![2],
        points: vec![(-20_000, 0, true), (20_000, 0, true)],
        instructions: vec![],
        bbox: None,
    };
    assert_eq!(font(&[jump]), Err(ContainerError::GlyphCoordinate));
    // A short loca can't reach past 128 KiB. The instructions are random, so they don't compress.
    let mut rng = Rng(7);
    let big = |_| TGlyph::Simple {
        contours: vec![1],
        points: vec![(0, 0, true)],
        instructions: (0..60_000).map(|_| rng.below(256) as u8).collect(),
        bbox: None,
    };
    let three: Vec<TGlyph> = (0..3).map(big).collect();
    assert_eq!(font(&three), Err(ContainerError::LocaOverflow));
    assert!(decode(&glyph_font(&three, 1, Hmtx::Plain, None)).is_ok());
}

/// The transformed glyf of one square, with one of its fields replaced.
fn patched_glyf(at: usize, bytes: &[u8]) -> Result<Vec<u8>, ContainerError> {
    let glyphs = [square(10)];
    let mut glyf = glyf_transform(0, &glyphs, None);
    glyf[at..at + bytes.len()].copy_from_slice(bytes);
    let entries = [
        (0, *b"glyf", 0, Some(glyf.len() as u32)),
        (0, *b"head", 54, None),
        (0, *b"loca", 4, Some(0)),
    ];
    decode(&woff2_raw(
        TRUETYPE,
        &entries,
        &[glyf, head(0)].concat(),
        &[],
    ))
}

#[test]
fn transformed_glyf_headers_and_streams_that_lie_are_refused() {
    assert!(patched_glyf(0, &[0, 0]).is_ok());
    // indexFormat.
    assert_eq!(
        patched_glyf(6, &[0, 2]),
        Err(ContainerError::Woff2(Woff2Error::IndexFormat(2)))
    );
    // numGlyphs 2 needs a 6-byte loca.
    assert_eq!(
        patched_glyf(4, &[0, 2]),
        Err(ContainerError::Woff2(Woff2Error::LocaLength))
    );
    // nContours stream size past the table.
    assert_eq!(
        patched_glyf(8, &[0, 1, 0, 0]),
        Err(ContainerError::Truncated(Part::GlyfStream(
            GlyfStream::Contours
        )))
    );
    // The flag stream one byte short of the four points.
    assert_eq!(
        patched_glyf(16, &[0, 0, 0, 3]),
        Err(ContainerError::Truncated(Part::GlyfStream(
            GlyfStream::Flags
        )))
    );
    // The glyph stream cut inside the last triplet.
    assert_eq!(
        patched_glyf(20, &[0, 0, 0, 14]),
        Err(ContainerError::Truncated(Part::GlyfStream(
            GlyfStream::Glyphs
        )))
    );
    // The overlap bitmap flag with no bitmap.
    assert_eq!(
        patched_glyf(2, &[0, 1]),
        Err(ContainerError::Truncated(Part::GlyfStream(
            GlyfStream::OverlapBitmap
        )))
    );
    let limits = ContainerLimits {
        max_glyphs: 0,
        ..ContainerLimits::default()
    };
    let font = glyph_font(&[square(1)], 0, Hmtx::Plain, None);
    assert_eq!(
        unwrap_with(&font, &Hint::default(), &limits),
        Err(ContainerError::GlyphCount { count: 1, limit: 0 })
    );
}

#[test]
fn woff2_directories_that_lie_are_refused() {
    let plain = |entries: &[(u8, [u8; 4], u32, Option<u32>)], stream: &[u8]| {
        decode(&woff2_raw(TRUETYPE, entries, stream, &[]))
    };
    let tables = [(0, *b"head", 54, None)];
    assert!(plain(&tables, &head(0)).is_ok());
    let glyf_v1 = [(1, *b"glyf", 4, Some(4)), (1, *b"loca", 4, Some(0))];
    assert_eq!(
        plain(&glyf_v1, &[0; 4]),
        Err(ContainerError::Woff2(Woff2Error::Transform {
            tag: tag(b"glyf"),
            version: 1
        }))
    );
    assert_eq!(
        plain(&[(1, *b"cmap", 4, Some(4))], &[0; 4]),
        Err(ContainerError::Woff2(Woff2Error::Transform {
            tag: tag(b"cmap"),
            version: 1
        }))
    );
    assert_eq!(
        plain(&[(2, *b"hmtx", 4, Some(4))], &[0; 4]),
        Err(ContainerError::Woff2(Woff2Error::Transform {
            tag: tag(b"hmtx"),
            version: 2
        }))
    );
    assert_eq!(
        plain(
            &[(0, *b"glyf", 4, Some(4)), (0, *b"loca", 4, Some(4))],
            &[0; 8]
        ),
        Err(ContainerError::Woff2(Woff2Error::LocaTransformLength))
    );
    // glyf without loca, and glyf transformed with loca stored.
    assert_eq!(
        plain(&[(3, *b"glyf", 4, None)], &[0; 4]),
        Err(ContainerError::Woff2(Woff2Error::GlyfLocaPair))
    );
    assert_eq!(
        plain(
            &[(0, *b"glyf", 4, Some(4)), (3, *b"loca", 4, None)],
            &[0; 8]
        ),
        Err(ContainerError::Woff2(Woff2Error::GlyfLocaPair))
    );
    assert_eq!(
        plain(
            &[(0, *b"head", 54, None), (0, *b"head", 54, None)],
            &[head(0), head(0)].concat()
        ),
        Err(ContainerError::DuplicateTable(tag(b"head")))
    );
    assert_eq!(
        plain(&[], &[0]),
        Err(ContainerError::TableCount {
            count: 0,
            limit: 512
        })
    );
    let other_flavor = woff2_plain(u32::from_be_bytes(*b"typ1"), &[(*b"head", head(0))]);
    assert_eq!(
        decode(&other_flavor),
        Err(ContainerError::NotSfnt(Format::Woff2))
    );
    let limits = ContainerLimits {
        max_tables: 1,
        ..ContainerLimits::default()
    };
    let two = woff2_plain(TRUETYPE, &[(*b"head", head(0)), (*b"cmap", vec![0; 4])]);
    assert_eq!(
        unwrap_with(&two, &Hint::default(), &limits),
        Err(ContainerError::TableCount { count: 2, limit: 1 })
    );
    // A UIntBase128 with a leading zero byte: origLength 0x80 0x01.
    let mut leading = woff2_plain(TRUETYPE, &[(*b"head", head(0))]);
    let at = 48 + 1;
    leading.splice(at..at + 1, [0x80, 0x36]);
    let length = leading.len() as u32;
    leading[8..12].copy_from_slice(&length.to_be_bytes());
    assert_eq!(
        decode(&leading),
        Err(ContainerError::Woff2(Woff2Error::Base128))
    );
}

#[test]
fn woff2_layout_and_stream_lies_are_refused() {
    let good = woff2_plain(TRUETYPE, &[(*b"head", head(0)), (*b"cmap", vec![7; 300])]);
    assert!(decode(&good).is_ok());
    let mut longer = good.clone();
    longer.extend_from_slice(&[0; 4]);
    assert_eq!(
        decode(&longer),
        Err(ContainerError::Woff2(Woff2Error::Length {
            declared: good.len() as u32,
            actual: good.len() + 4
        }))
    );
    let length = longer.len() as u32;
    longer[8..12].copy_from_slice(&length.to_be_bytes());
    assert_eq!(
        decode(&longer),
        Err(ContainerError::Woff2(Woff2Error::Layout))
    );
    // Metadata that doesn't start where the stream ends.
    let mut meta = good.clone();
    meta[28..32].copy_from_slice(&4u32.to_be_bytes());
    assert_eq!(
        decode(&meta),
        Err(ContainerError::Woff2(Woff2Error::Layout))
    );
    // totalCompressedSize past the file.
    let mut past = good.clone();
    past[20..24].copy_from_slice(&(good.len() as u32).to_be_bytes());
    assert_eq!(
        decode(&past),
        Err(ContainerError::Woff2(Woff2Error::Layout))
    );
    // A corrupt stream, and a stream longer than the tables.
    let mut corrupt = good.clone();
    let stream_at = good.len() - 8;
    corrupt[stream_at] ^= 0xff;
    corrupt[stream_at + 1] ^= 0xff;
    assert_eq!(
        decode(&corrupt),
        Err(ContainerError::Woff2(Woff2Error::Brotli))
    );
    let entries = [(0, *b"head", 54, None)];
    let longer_stream = woff2_raw(TRUETYPE, &entries, &[head(0), vec![0; 10]].concat(), &[]);
    assert_eq!(
        decode(&longer_stream),
        Err(ContainerError::Woff2(Woff2Error::Brotli))
    );
    let shorter_stream = woff2_raw(TRUETYPE, &entries, &head(0)[..50], &[]);
    assert_eq!(
        decode(&shorter_stream),
        Err(ContainerError::Woff2(Woff2Error::Brotli))
    );
}

#[test]
fn woff2_budgets_hold() {
    // A megabyte of zeros compresses to a few bytes: past 40:1.
    let bomb = woff2_plain(
        TRUETYPE,
        &[(*b"head", head(0)), (*b"zero", vec![0; 1 << 20])],
    );
    assert!(bomb.len() < 1000);
    assert_eq!(decode(&bomb), Err(ContainerError::Expansion { ratio: 40 }));
    let generous = ContainerLimits {
        max_expansion: 100_000,
        ..ContainerLimits::default()
    };
    assert!(unwrap_with(&bomb, &Hint::default(), &generous).is_ok());
    let small = ContainerLimits {
        max_expansion: 100_000,
        max_output_bytes: 1 << 19,
        ..ContainerLimits::default()
    };
    assert_eq!(
        unwrap_with(&bomb, &Hint::default(), &small),
        Err(ContainerError::TooLarge { limit: 1 << 19 })
    );
    // totalSfntSize alone past the limit.
    let mut declared = fixture("barlow.woff2");
    declared[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(
        decode(&declared),
        Err(ContainerError::TooLarge { limit: 64 << 20 })
    );
    // The reconstructed glyf alone past the limit.
    let tight = ContainerLimits {
        max_output_bytes: 20_000,
        ..ContainerLimits::default()
    };
    let woff2 = fixture("barlow.woff2");
    let mut patched = woff2.clone();
    patched[16..20].copy_from_slice(&0u32.to_be_bytes());
    assert_eq!(
        unwrap_with(&patched, &Hint::default(), &tight),
        Err(ContainerError::TooLarge { limit: 20_000 })
    );
}

#[test]
fn woff2_collection_directories_that_lie_are_refused() {
    let entries = [(0, *b"head", 54, None), (0, *b"cmap", 4, None)];
    let stream = [head(0), vec![0; 4]].concat();
    let ttc = |collection: &[u8]| {
        decode(&woff2_raw(
            u32::from_be_bytes(*b"ttcf"),
            &entries,
            &stream,
            collection,
        ))
    };
    let face = |indices: &[u16]| {
        let mut bytes = u255(indices.len() as u16);
        bytes.extend_from_slice(&TRUETYPE.to_be_bytes());
        indices
            .iter()
            .for_each(|&i| bytes.extend_from_slice(&u255(i)));
        bytes
    };
    let directory = |version: u32, faces: &[Vec<u8>]| {
        let mut bytes = version.to_be_bytes().to_vec();
        bytes.extend_from_slice(&u255(faces.len() as u16));
        faces.iter().for_each(|f| bytes.extend_from_slice(f));
        bytes
    };
    let sfnt = ttc(&directory(0x0001_0000, &[face(&[0, 1]), face(&[0])])).unwrap();
    assert_eq!(face_offsets(&sfnt).len(), 2);
    assert_eq!(tables(&sfnt, face_offsets(&sfnt)[1]).len(), 1);
    assert_eq!(
        ttc(&directory(0x0003_0000, &[face(&[0])])),
        Err(ContainerError::Woff2(Woff2Error::CollectionVersion(
            0x0003_0000
        )))
    );
    assert_eq!(
        ttc(&directory(0x0002_0000, &[face(&[0, 2])])),
        Err(ContainerError::Woff2(Woff2Error::TableIndex {
            index: 2,
            count: 2
        }))
    );
    assert_eq!(
        ttc(&directory(0x0001_0000, &[])),
        Err(ContainerError::FaceCount {
            count: 0,
            limit: 256
        })
    );
    assert_eq!(
        ttc(&directory(0x0001_0000, &[face(&[1, 1])])),
        Err(ContainerError::DuplicateTable(tag(b"cmap")))
    );
    assert_eq!(
        ttc(&directory(0x0001_0000, &[face(&[])])),
        Err(ContainerError::TableCount {
            count: 0,
            limit: 512
        })
    );
}

#[test]
fn woff_files_that_lie_are_refused() {
    let tables = vec![
        (*b"cmap", vec![3; 1000]),
        (*b"head", head(0)),
        (*b"name", vec![1, 2, 3]),
    ];
    let good = woff1(TRUETYPE, &tables);
    let sfnt = decode(&good).unwrap();
    assert_eq!(table(&sfnt, 0, b"cmap"), vec![3; 1000]);
    assert_eq!(table(&sfnt, 0, b"name"), [1, 2, 3]);
    let with = |at: usize, bytes: &[u8]| {
        let mut woff = good.clone();
        woff[at..at + bytes.len()].copy_from_slice(bytes);
        decode(&woff)
    };
    let length = good.len() as u32;
    assert_eq!(
        with(8, &(length - 4).to_be_bytes()),
        Err(ContainerError::Woff(WoffError::Length {
            declared: length - 4,
            actual: good.len()
        }))
    );
    assert_eq!(
        with(14, &[0, 1]),
        Err(ContainerError::Woff(WoffError::Reserved))
    );
    assert_eq!(
        with(4, b"ttcf"),
        Err(ContainerError::Woff(WoffError::Collection))
    );
    assert_eq!(with(4, b"typ1"), Err(ContainerError::NotSfnt(Format::Woff)));
    assert_eq!(
        with(12, &[0, 0]),
        Err(ContainerError::TableCount {
            count: 0,
            limit: 512
        })
    );
    // The cmap entry: offset at 48, compLength at 52, origLength at 56, checksum at 60.
    let cmap_offset = u32_at(&good, 48);
    assert_eq!(
        with(48, &(cmap_offset + 1).to_be_bytes()),
        Err(ContainerError::Woff(WoffError::Placement(tag(b"cmap"))))
    );
    assert_eq!(
        with(48, &(cmap_offset - 4).to_be_bytes()),
        Err(ContainerError::Woff(WoffError::Placement(tag(b"cmap"))))
    );
    assert_eq!(
        with(52, &u32::MAX.to_be_bytes()),
        Err(ContainerError::Woff(WoffError::Placement(tag(b"cmap"))))
    );
    assert_eq!(
        with(56, &1u32.to_be_bytes()),
        Err(ContainerError::Woff(WoffError::CompLength(tag(b"cmap"))))
    );
    assert_eq!(
        with(56, &1001u32.to_be_bytes()),
        Err(ContainerError::Woff(WoffError::Zlib(tag(b"cmap"))))
    );
    assert_eq!(
        with(56, &999u32.to_be_bytes()),
        Err(ContainerError::Woff(WoffError::Zlib(tag(b"cmap"))))
    );
    // A broken Adler-32.
    let comp_length = u32_at(&good, 52) as usize;
    let end = cmap_offset as usize + comp_length;
    let mut adler = good.clone();
    adler[end - 1] ^= 1;
    assert_eq!(
        decode(&adler),
        Err(ContainerError::Woff(WoffError::Zlib(tag(b"cmap"))))
    );
    // A wrong checksum is only a warning.
    let mut woff = good.clone();
    woff[60] ^= 1;
    let unwrapped = unwrap(&woff, &Hint::default()).unwrap();
    assert_eq!(
        unwrapped.warnings,
        [slidock_fonts::container::ContainerWarning::ChecksumMismatch(tag(b"cmap"))]
    );
    // Expansion and output limits.
    let bomb = woff1(TRUETYPE, &[(*b"zero", vec![0; 1 << 20])]);
    assert_eq!(decode(&bomb), Err(ContainerError::Expansion { ratio: 40 }));
    let limits = ContainerLimits {
        max_output_bytes: 1000,
        ..ContainerLimits::default()
    };
    assert_eq!(
        unwrap_with(&good, &Hint::default(), &limits),
        Err(ContainerError::TooLarge { limit: 1000 })
    );
}

/// One mutation of `input`, picked by the generator.
fn mutate(input: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut bytes = input.to_vec();
    let interesting = [
        0u32,
        1,
        0x7f,
        0x80,
        0xff,
        0xffff,
        0x7fff_ffff,
        0xffff_ffff,
        input.len() as u32,
    ];
    for _ in 0..1 + rng.below(4) {
        if bytes.is_empty() {
            bytes.push(0);
        }
        let at = rng.below(bytes.len());
        match rng.below(7) {
            0 => bytes[at] ^= 1 << rng.below(8),
            1 => bytes[at] = rng.below(256) as u8,
            2 => {
                // A header-ish field set to an interesting value.
                let at = rng.below(bytes.len().min(256)) & !3;
                let value = interesting[rng.below(interesting.len())].to_be_bytes();
                let end = (at + 4).min(bytes.len());
                bytes[at..end].copy_from_slice(&value[..end - at]);
            }
            3 => bytes.truncate(at),
            4 => {
                let length = rng.below(64).min(bytes.len() - at);
                let copy = bytes[at..at + length].to_vec();
                let to = rng.below(bytes.len());
                bytes.splice(to..to, copy);
            }
            5 => {
                let length = rng.below(16);
                let to = rng.below(bytes.len());
                bytes.splice(to..to, (0..length).map(|_| rng.below(256) as u8));
            }
            _ => {
                let length = rng.below(32).min(bytes.len() - at);
                bytes.drain(at..at + length);
            }
        }
    }
    bytes
}

#[test]
fn mutated_containers_never_panic_or_pass_the_limits() {
    let ttf = fixture("barlow.ttf");
    let xored: Vec<u8> = ttf.iter().map(|b| b ^ 0x50).collect();
    let mut eot_xor = fixture("barlow-mkeot.eot");
    let data_start = eot_xor.len() - ttf.len();
    eot_xor[data_start..].copy_from_slice(&xored);
    eot_xor[12..16].copy_from_slice(&eot::FLAG_XOR.to_le_bytes());
    let guid = "{6C2E1A5B-0F3D-4C8E-9A7B-1D2E3F405162}";
    let mut obfuscated = ttf.clone();
    odttf::xor(&mut obfuscated, &odttf::parse_guid(guid).unwrap()).unwrap();
    let odttf_name = format!("{guid}.odttf");

    let mut inputs: Vec<(String, Vec<u8>, Hint<'_>)> = [
        "barlow.woff",
        "barlow-cff.woff",
        "barlow.woff2",
        "barlow-hmtx.woff2",
        "barlow-null.woff2",
        "barlow-cff.woff2",
        "roboto.woff2",
        "pair.woff2",
        "barlow-mkeot.eot",
        "barlow-mtx.eot",
        "roboto-hdmx-mtx.eot",
    ]
    .iter()
    .map(|name| (name.to_string(), fixture(name), Hint::default()))
    .collect();
    inputs.push(("eot xor".into(), eot_xor, Hint::default()));
    inputs.push((
        "odttf".into(),
        obfuscated,
        Hint {
            content_type: None,
            part_name: Some(&odttf_name),
        },
    ));
    let tight = ContainerLimits {
        max_output_bytes: 32 << 10,
        max_glyphs: 64,
        max_tables: 16,
        max_faces: 1,
        ..ContainerLimits::default()
    };
    let mut rng = Rng(0x5eed_f0e7_c0de_cafe);
    let (mut accepted, mut refused) = (0usize, 0usize);
    for (name, input, hint) in &inputs {
        assert!(
            unwrap(input, hint).is_ok(),
            "{name} must decode before mutation"
        );
        for round in 0..1500 {
            let mutated = mutate(input, &mut rng);
            let limits = if round % 3 == 0 {
                tight.clone()
            } else {
                ContainerLimits::default()
            };
            match unwrap_with(&mutated, hint, &limits) {
                Ok(unwrapped) => {
                    accepted += 1;
                    assert!(
                        unwrapped.sfnt.len() as u64 <= limits.max_output_bytes,
                        "{name} round {round}"
                    );
                    if unwrapped.kind != Format::Sfnt && unwrapped.kind != Format::Collection {
                        let magic = &unwrapped.sfnt[..4];
                        assert!(
                            [&[0, 1, 0, 0][..], b"OTTO", b"true", b"ttcf"].contains(&magic),
                            "{name} round {round}"
                        );
                    }
                    let _ = read_sfnt(&unwrapped.sfnt);
                }
                Err(_) => refused += 1,
            }
        }
    }
    assert_eq!(accepted + refused, inputs.len() * 1500);
    assert!(
        accepted > 0 && refused > 0,
        "accepted {accepted}, refused {refused}"
    );
}
