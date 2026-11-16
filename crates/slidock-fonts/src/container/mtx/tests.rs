#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use super::encode::{self, Encoder, Options, Stats};
use super::lzcomp::{self, PRELOAD};
use super::*;
use crate::container::assemble::checksum;

const ROBOTO: &[u8] = include_bytes!("../../../tests/data/container/roboto.ttf");

fn inflate(
    stream: &[u8],
    input: usize,
    limits: &ContainerLimits,
) -> Result<Vec<u8>, ContainerError> {
    let mut spent = 0;
    lzcomp::decompress(stream, 1, &mut spent, input, limits)
}

fn round_trip(data: &[u8], options: Options) -> Stats {
    let (stream, stats) = encode::lzcomp(data, options);
    let out = inflate(&stream, data.len().max(1), &ContainerLimits::default()).unwrap();
    assert!(
        out == data,
        "{options:?}: {} bytes back of {}",
        out.len(),
        data.len()
    );
    stats
}

#[test]
fn lzcomp_round_trips_a_font_through_every_symbol_kind() {
    let stats = round_trip(ROBOTO, Options::default());
    assert!(stats.literals > 0, "{stats:?}");
    assert!(stats.dups.iter().all(|&count| count > 0), "{stats:?}");
    assert!(stats.copies > 0 && stats.far_copies > 0, "{stats:?}");
    assert!(
        stats.preload_copies > 0 && stats.long_lengths > 0,
        "{stats:?}"
    );
    // Literals only, and literals with one-byte copies.
    let plain = Options {
        copies: false,
        dups: false,
        ..Options::default()
    };
    assert_eq!(round_trip(&ROBOTO[..4000], plain).literals, 4000);
    let dups = Options {
        copies: false,
        ..Options::default()
    };
    round_trip(&ROBOTO[..4000], dups);
    round_trip(&[], Options::default());
    round_trip(&[7], Options::default());
}

#[test]
fn the_run_length_layer_expands_escaped_runs() {
    let mut data = Vec::new();
    for i in 0..300u32 {
        data.extend(std::iter::repeat_n((i % 7) as u8, (i % 300) as usize));
        data.extend_from_slice(&ROBOTO[i as usize * 5..i as usize * 5 + 9]);
    }
    data.extend((0..=255u8).chain(0..=255u8));
    let packed = encode::run_length_pack(&data);
    assert!(packed.len() < data.len() / 4);
    // The escape byte itself occurs in the data, so `escape, 0` is exercised.
    assert!(data.contains(&packed[0]));
    let options = Options {
        run_length: true,
        ..Options::default()
    };
    round_trip(&data, options);
    round_trip(ROBOTO, options);
    round_trip(
        &data,
        Options {
            run_length: true,
            copies: false,
            dups: false,
        },
    );
}

#[test]
fn lzcomp_streams_that_lie_are_refused() {
    let limits = ContainerLimits::default();
    let (stream, _) = encode::lzcomp(&ROBOTO[..3000], Options::default());
    for cut in [1, 3, 4, stream.len() / 2, stream.len() - 1] {
        assert_eq!(
            inflate(&stream[..cut], 3000, &limits),
            Err(ContainerError::Truncated(Part::Mtx(MtxStream::Lzcomp(1)))),
            "cut at {cut}"
        );
    }
    // A copy that reaches before the preloaded history.
    let mut encoder = Encoder::new(false, 100_000);
    encoder.copy(3, u32::try_from(PRELOAD).unwrap());
    let invalid = Err(MtxError::Lzcomp { block: 1 }.into());
    assert_eq!(inflate(&encoder.finish(), 100_000, &limits), invalid);
    // The farthest copy that stays inside it, the preload's first three bytes, decodes.
    let mut encoder = Encoder::new(false, 100_000);
    encoder.copy(3, u32::try_from(PRELOAD - 2).unwrap());
    let stream = encoder.finish();
    let mut spent = 0;
    let error = lzcomp::decompress(&stream, 1, &mut spent, 100_000, &limits).unwrap_err();
    // The copy is fine; the stream then ends early.
    assert_eq!(
        error,
        ContainerError::Truncated(Part::Mtx(MtxStream::Lzcomp(1)))
    );
    // A copy longer than what is left of the block.
    let mut encoder = Encoder::new(false, 4);
    encoder.copy(6, 1);
    assert_eq!(inflate(&encoder.finish(), 4, &limits), invalid);
    // A copy length that never stops.
    let mut encoder = Encoder::new(false, 10);
    encoder.copy_length_chunks(&[7; 40]);
    assert_eq!(inflate(&encoder.finish(), 10, &limits), invalid);
}

#[test]
fn lzcomp_lengths_are_budgeted_before_allocation() {
    let encoder = Encoder::new(false, 0xff_ffff);
    let stream = encoder.finish();
    let small = ContainerLimits {
        max_output_bytes: 1 << 20,
        ..ContainerLimits::default()
    };
    assert_eq!(
        inflate(&stream, 1 << 20, &small),
        Err(ContainerError::TooLarge { limit: 1 << 20 })
    );
    assert_eq!(
        inflate(&stream, 1000, &ContainerLimits::default()),
        Err(ContainerError::Expansion { ratio: 40 })
    );
    // Run-length output counts against the same budget.
    let data = vec![9u8; 200_000];
    let (stream, _) = encode::lzcomp(
        &data,
        Options {
            run_length: true,
            ..Options::default()
        },
    );
    assert!(stream.len() * 40 < data.len());
    assert_eq!(
        inflate(&stream, stream.len(), &ContainerLimits::default()),
        Err(ContainerError::Expansion { ratio: 40 })
    );
}

#[test]
fn the_container_header_is_checked() {
    let limits = ContainerLimits::default();
    let empty = encode::lzcomp(&[], Options::default()).0;
    let good = encode::container([&empty, &empty, &empty]);
    assert!(decompress(&good, &limits).is_ok());
    let mut bad = good.clone();
    bad[0] = 2;
    assert_eq!(decompress(&bad, &limits), Err(MtxError::Version(2).into()));
    let with_offsets = |second: u32, third: u32| {
        let mut bytes = good.clone();
        bytes[4..7].copy_from_slice(&second.to_be_bytes()[1..]);
        bytes[7..10].copy_from_slice(&third.to_be_bytes()[1..]);
        decompress(&bytes, &limits)
    };
    let length = good.len();
    let lie = |second, third| {
        Err(MtxError::Offsets {
            second,
            third,
            length,
        }
        .into())
    };
    assert_eq!(with_offsets(9, 20), lie(9, 20));
    assert_eq!(with_offsets(20, 19), lie(20, 19));
    assert_eq!(with_offsets(10, 0xff_ffff), lie(10, 0xff_ffff));
    // An empty block is a truncated stream.
    assert_eq!(
        with_offsets(10, 10).unwrap_err(),
        ContainerError::Truncated(Part::Mtx(MtxStream::Lzcomp(1)))
    );
    assert_eq!(
        decompress(&good[..9], &limits),
        Err(ContainerError::Truncated(Part::Mtx(MtxStream::Header)))
    );
}

/// Block 1's tables by tag.
type Tables = Vec<(&'static [u8; 4], Vec<u8>)>;

/// A compact font's block 1: a table directory, then each table.
fn ctf_block(tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    out.extend_from_slice(&u16::try_from(tables.len()).unwrap().to_be_bytes());
    out.extend_from_slice(&[0; 6]);
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in tables {
        out.extend_from_slice(*tag);
        out.extend_from_slice(&0u32.to_be_bytes());
        let (at, length) = if **tag == *b"loca" {
            (0, 0)
        } else {
            (offset, data.len())
        };
        out.extend_from_slice(&u32::try_from(at).unwrap().to_be_bytes());
        out.extend_from_slice(&u32::try_from(length).unwrap().to_be_bytes());
        if **tag != *b"loca" {
            offset += data.len();
        }
    }
    for (tag, data) in tables {
        if **tag != *b"loca" {
            out.extend_from_slice(data);
        }
    }
    out
}

/// Magnitude-dependent values, least significant bit first.
fn magnitudes(values: &[i32]) -> Vec<u8> {
    let mut bits = Vec::new();
    for &value in values {
        bits.extend(std::iter::repeat_n(true, value.unsigned_abs() as usize));
        bits.push(false);
        if value != 0 {
            bits.push(value < 0);
        }
    }
    bits.chunks(8)
        .map(|chunk| {
            chunk
                .iter()
                .enumerate()
                .fold(0u8, |byte, (i, &bit)| byte | u8::from(bit) << i)
        })
        .collect()
}

fn be16(values: &[i32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|&v| (v as i16).to_be_bytes())
        .collect()
}

fn head() -> Vec<u8> {
    let mut head = vec![0u8; 54];
    head[12..16].copy_from_slice(&0x5f0f_3cf5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head
}

/// Five glyphs: empty, simple with a computed box, simple with a given box and hop codes, composite with
/// instructions, and a given box with no contours. Advances 500, 600 and 700 (repeated).
fn sample_tables() -> (Tables, Vec<u8>, Vec<u8>) {
    let maxp = [0, 0, 0x50, 0, 0, 5].to_vec();
    let mut hhea = vec![0u8; 36];
    hhea[34..36].copy_from_slice(&3u16.to_be_bytes());
    let hmtx = be16(&[500, 0, 600, 0, 700, 0, 0, 0]);
    let mut glyf = be16(&[0]);
    // Glyph 1: one contour ending at point 2.
    glyf.extend(be16(&[1]));
    glyf.extend([2, 1, 11, 0x81, 0, 100, 50, 3, 2]);
    // Glyph 2: a given box, two points, five push values.
    glyf.extend(be16(&[0x7fff, 1, -10, -20, 110, 60]));
    glyf.extend([1, 11, 0x81, 20, 30, 5, 0]);
    // Glyph 3: two components, the second scaled with instructions.
    glyf.extend(be16(&[-1, 0, -10, 120, 70, 0x21, 1, 10, -10, 0x108, 2]));
    glyf.extend([1, 2, 0x40, 0, 1, 1]);
    // Glyph 4: a given box, no contours.
    glyf.extend(be16(&[0x7fff, 0, 1, 2, 3, 4]));
    let push = [1, 253, 1, 44, 2, 5, 7, 0xfb, 9, 250, 3].to_vec();
    let code = [0xaa, 0xbb, 0xcc].to_vec();
    let cvt = [0, 3, 10, 248, 0, 239, 1].to_vec();
    let mut hdmx = vec![0, 0, 0, 2, 0, 0, 0, 8, 12, 9, 20, 15];
    hdmx.extend(magnitudes(&[0, 1, -1, 0, 2, 0, 0, 0, -2, 3]));
    let mut vdmx = be16(&[1, 1, 1]);
    vdmx.extend([1, 1, 1, 1]);
    vdmx.extend(be16(&[12, 2, 2048, 512]));
    vdmx.extend(magnitudes(&[0, 1, -1, 1, 0, 0]));
    let tables = vec![
        (b"VDMX", vdmx),
        (b"cvt ", cvt),
        (b"glyf", glyf),
        (b"hdmx", hdmx),
        (b"head", head()),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", Vec::new()),
        (b"maxp", maxp),
    ];
    (tables, push, code)
}

fn sample_font(options: Options) -> Vec<u8> {
    let (tables, push, code) = sample_tables();
    encode::mtx([&ctf_block(&tables), &push, &code], options)
}

fn table<'a>(sfnt: &'a [u8], tag: &[u8; 4]) -> &'a [u8] {
    let count = usize::from(u16::from_be_bytes([sfnt[4], sfnt[5]]));
    for i in 0..count {
        let record = &sfnt[12 + 16 * i..28 + 16 * i];
        if record[..4] == *tag {
            let offset = u32::from_be_bytes(record[8..12].try_into().unwrap()) as usize;
            let length = u32::from_be_bytes(record[12..16].try_into().unwrap()) as usize;
            return &sfnt[offset..offset + length];
        }
    }
    panic!("no {tag:?}");
}

#[test]
fn a_compact_font_decodes_every_table_kind() {
    for options in [
        Options::default(),
        Options {
            run_length: true,
            ..Options::default()
        },
    ] {
        let font = decode(&sample_font(options), &ContainerLimits::default()).unwrap();
        assert!(font.warnings.is_empty(), "{:?}", font.warnings);
        let sfnt = &font.sfnt;
        assert_eq!(checksum(sfnt), 0xb1b0_afba);

        let glyph1: Vec<u8> = [
            be16(&[1, 0, 0, 100, 50, 2, 9]),
            vec![0xb0, 1, 0xb8, 0x01, 0x2c, 0xb0, 2, 0xaa, 0xbb],
            vec![0x37, 0x33, 0x34, 0, 100, 0, 50],
        ]
        .concat();
        let glyph2: Vec<u8> = [
            be16(&[1, -10, -20, 110, 60, 1, 6]),
            vec![0xb4, 5, 7, 5, 9, 5],
            vec![0x37, 0x34, 20, 0, 30, 0],
        ]
        .concat();
        let glyph3: Vec<u8> = [
            be16(&[-1, 0, -10, 120, 70, 0x21, 1, 10, -10, 0x108, 2]),
            vec![1, 2, 0x40, 0],
            be16(&[4]),
            vec![0xb8, 0xff, 0xfd, 0xcc],
        ]
        .concat();
        let glyf = table(sfnt, b"glyf");
        assert_eq!(glyf, [glyph1, glyph2, glyph3].concat());
        assert_eq!(table(sfnt, b"loca"), be16(&[0, 0, 15, 28, 44, 44]));
        assert_eq!(table(sfnt, b"cvt "), be16(&[10, 248, 247]));

        let mut hdmx = vec![0, 0, 0, 2, 0, 0, 0, 8];
        hdmx.extend([12, 9, 6, 8, 7, 8, 10, 0]);
        hdmx.extend([20, 15, 10, 12, 14, 12, 17, 0]);
        assert_eq!(table(sfnt, b"hdmx"), hdmx);

        let mut vdmx = be16(&[1, 1, 1]);
        vdmx.extend([1, 1, 1, 1]);
        vdmx.extend(be16(&[12, 2]));
        vdmx.extend([8, 10]);
        vdmx.extend(be16(&[8, 9, -3, 10, 10, -3]));
        assert_eq!(table(sfnt, b"VDMX"), vdmx);

        let face = read_fonts::FontRef::new(sfnt).unwrap();
        use read_fonts::TableProvider;
        let loca = face.loca(None).unwrap();
        let glyf = face.glyf().unwrap();
        for gid in 0..5u32 {
            loca.get_glyf(gid.into(), &glyf).unwrap();
        }
    }
}

#[test]
fn device_metrics_that_do_not_decode_are_dropped_with_a_warning() {
    let (mut tables, push, code) = sample_tables();
    // An hdmx record size that does not fit the glyph count, and a VDMX offset that starts no group.
    tables[3].1[7] = 12;
    tables[0].1[11] = 13;
    let mtx = encode::mtx([&ctf_block(&tables), &push, &code], Options::default());
    let font = decode(&mtx, &ContainerLimits::default()).unwrap();
    assert_eq!(
        font.warnings,
        [
            ContainerWarning::DroppedTable(TableTag(*b"VDMX")),
            ContainerWarning::DroppedTable(TableTag(*b"hdmx"))
        ]
    );
    let count = u16::from_be_bytes([font.sfnt[4], font.sfnt[5]]);
    assert_eq!(count, 7);
}

#[test]
fn stored_device_metrics_get_their_version_back() {
    let (mut tables, push, code) = sample_tables();
    let raw_hdmx = [
        vec![0xff, 0xff, 0, 1, 0, 0, 0, 8],
        vec![12, 9, 6, 8, 7, 8, 10, 0],
    ]
    .concat();
    tables[3].1 = raw_hdmx.clone();
    let mut raw_vdmx = tables[0].1.clone();
    raw_vdmx[..2].copy_from_slice(&0xfffeu16.to_be_bytes());
    tables[0].1 = raw_vdmx.clone();
    let mtx = encode::mtx([&ctf_block(&tables), &push, &code], Options::default());
    let font = decode(&mtx, &ContainerLimits::default()).unwrap();
    assert!(font.warnings.is_empty());
    assert_eq!(table(&font.sfnt, b"hdmx")[..2], [0, 0]);
    assert_eq!(table(&font.sfnt, b"hdmx")[2..], raw_hdmx[2..]);
    assert_eq!(table(&font.sfnt, b"VDMX")[..2], [0, 1]);
    assert_eq!(table(&font.sfnt, b"VDMX")[2..], raw_vdmx[2..]);
}

#[test]
fn compact_fonts_that_lie_are_refused() {
    let limits = ContainerLimits::default();
    let decode_with = |edit: &dyn Fn(&mut Tables, &mut Vec<u8>)| {
        let (mut tables, mut push, code) = sample_tables();
        edit(&mut tables, &mut push);
        decode(
            &encode::mtx([&ctf_block(&tables), &push, &code], Options::default()),
            &limits,
        )
        .map(|_| ())
    };
    assert_eq!(decode_with(&|_, _| {}), Ok(()));
    // Too few push values for the glyphs.
    assert_eq!(
        decode_with(&|_, push| push.truncate(8)),
        Err(ContainerError::Truncated(Part::Mtx(MtxStream::PushData)))
    );
    // A hop code at the start of a glyph's values.
    assert_eq!(
        decode_with(&|_, push| push[5] = 0xfb),
        Err(MtxError::HopCode.into())
    );
    // Glyph data cut short.
    assert_eq!(
        decode_with(&|tables, _| tables[2].1.truncate(20)),
        Err(ContainerError::Truncated(Part::Mtx(MtxStream::Glyphs)))
    );
    // More glyphs than the data holds.
    assert_eq!(
        decode_with(&|tables, _| tables[8].1[5] = 6),
        Err(ContainerError::Truncated(Part::Mtx(MtxStream::Glyphs)))
    );
    assert_eq!(
        decode_with(&|tables, _| tables[8].1[4..6].copy_from_slice(&[0xff, 0xff])),
        Err(ContainerError::Truncated(Part::Mtx(MtxStream::Glyphs)))
    );
    // A negative contour count after the given-box marker.
    assert_eq!(
        decode_with(&|tables, _| tables[2].1[15..17].copy_from_slice(&[0xff, 0xfe])),
        Err(ContainerError::GlyphCoordinate)
    );
    // No head, no maxp.
    assert_eq!(
        decode_with(&|tables, _| tables[4].0 = b"heax"),
        Err(MtxError::MissingTable(TableTag(*b"head")).into())
    );
    assert_eq!(
        decode_with(&|tables, _| tables[8].0 = b"maxq"),
        Err(MtxError::MissingTable(TableTag(*b"maxp")).into())
    );
    // A loca entry without glyf.
    assert_eq!(
        decode_with(&|tables, _| tables[2].0 = b"glyg"),
        Err(MtxError::MissingTable(TableTag(*b"glyf")).into())
    );
    // A short head.
    assert_eq!(
        decode_with(&|tables, _| tables[4].1.truncate(50)),
        Err(ContainerError::ShortTable(TableTag(*b"head")))
    );
    // A cvt with fewer values than it declares.
    assert_eq!(
        decode_with(&|tables, _| tables[1].1[1] = 9),
        Err(ContainerError::Truncated(Part::Table(TableTag(*b"cvt "))))
    );
    let glyph_limit = ContainerLimits {
        max_glyphs: 4,
        ..ContainerLimits::default()
    };
    let (tables, push, code) = sample_tables();
    let mtx = encode::mtx([&ctf_block(&tables), &push, &code], Options::default());
    assert_eq!(
        decode(&mtx, &glyph_limit),
        Err(ContainerError::GlyphCount { count: 5, limit: 4 })
    );
}

#[test]
fn table_records_outside_block_1_are_refused() {
    let (tables, push, code) = sample_tables();
    let mut block = ctf_block(&tables);
    // The cvt record's length.
    let record = 12 + 16;
    block[record + 12..record + 16].copy_from_slice(&0x00ff_ffffu32.to_be_bytes());
    let mtx = encode::mtx([&block, &push, &code], Options::default());
    assert_eq!(
        decode(&mtx, &ContainerLimits::default()),
        Err(MtxError::TableBounds(TableTag(*b"cvt ")).into())
    );
    block.truncate(20);
    let mtx = encode::mtx([&block, &push, &code], Options::default());
    assert_eq!(
        decode(&mtx, &ContainerLimits::default()),
        Err(ContainerError::Truncated(Part::TableDirectory))
    );
}

#[test]
fn table_copies_are_budgeted_as_they_are_made() {
    let (mut tables, push, code) = sample_tables();
    tables.push((b"zbig", vec![7; 20_000]));
    for i in 0..40 {
        let tag: &'static [u8; 4] = Box::leak(Box::new(
            *format!("z{i:03}").as_bytes().first_chunk().unwrap(),
        ));
        tables.push((tag, Vec::new()));
    }
    let mut block = ctf_block(&tables);
    // Every z### record points at zbig's bytes.
    let big = 12 + 16 * 9;
    let (offset, length) = (
        block[big + 8..big + 12].to_vec(),
        block[big + 12..big + 16].to_vec(),
    );
    for i in 0..40 {
        let record = 12 + 16 * (10 + i);
        block[record + 8..record + 12].copy_from_slice(&offset);
        block[record + 12..record + 16].copy_from_slice(&length);
    }
    let mtx = encode::mtx([&block, &push, &code], Options::default());
    let limits = ContainerLimits {
        max_output_bytes: 200_000,
        ..ContainerLimits::default()
    };
    assert_eq!(
        decode(&mtx, &limits),
        Err(ContainerError::TooLarge { limit: 200_000 })
    );
    assert!(decode(&mtx, &ContainerLimits::default()).is_ok());
}

#[test]
fn loca_is_added_when_the_directory_lacks_it() {
    let (mut tables, push, code) = sample_tables();
    tables.retain(|(tag, _)| **tag != *b"loca");
    let mtx = encode::mtx([&ctf_block(&tables), &push, &code], Options::default());
    let sfnt = decode(&mtx, &ContainerLimits::default()).unwrap().sfnt;
    assert_eq!(table(&sfnt, b"loca"), be16(&[0, 0, 15, 28, 44, 44]));
    // Its data comes last.
    let count = usize::from(u16::from_be_bytes([sfnt[4], sfnt[5]]));
    let loca_record = (0..count)
        .map(|i| &sfnt[12 + 16 * i..28 + 16 * i])
        .find(|record| record[..4] == *b"loca")
        .unwrap();
    let offset = u32::from_be_bytes(loca_record[8..12].try_into().unwrap()) as usize;
    assert_eq!(offset + 12, sfnt.len());
    // A long loca when head says so.
    tables[4].1[51] = 1;
    let mtx = encode::mtx([&ctf_block(&tables), &push, &code], Options::default());
    let sfnt = decode(&mtx, &ContainerLimits::default()).unwrap().sfnt;
    let long: Vec<u8> = [0u32, 0, 30, 56, 88, 88]
        .iter()
        .flat_map(|v| v.to_be_bytes())
        .collect();
    assert_eq!(table(&sfnt, b"loca"), long);
}

/// A font of no glyphs whose only device table is a one-group `VDMX` with the given multipliers and a first size
/// error of `error`.
fn vdmx_font(multipliers: [i32; 2], error: i32) -> Vec<u8> {
    let maxp = [0, 0, 0x50, 0, 0, 0].to_vec();
    let mut vdmx = be16(&[1, 1, 1]);
    vdmx.extend([1, 1, 1, 1]);
    vdmx.extend(be16(&[12, 1, multipliers[0], multipliers[1]]));
    vdmx.extend(magnitudes(&[error, 0, 0]));
    ctf_block(&[(b"VDMX", vdmx), (b"head", head()), (b"maxp", maxp)])
}

#[test]
fn vdmx_sizes_out_of_range_drop_the_table() {
    let limits = ContainerLimits::default();
    // In range: the largest size a group can name, with the largest multipliers either way round.
    let font = decode_ctf([&vdmx_font([32767, -32768], 247), &[], &[]], &limits).unwrap();
    assert!(font.warnings.is_empty());
    let entry = &table(&font.sfnt, b"VDMX")[16..22];
    assert_eq!(entry, be16(&[255, 4080, 4079]));
    // A size whose product with a multiplier leaves i32, either way round.
    for (multipliers, error) in [
        ([32767, 32767], 70000),
        ([-32768, -32768], -70016),
        ([0, 32767], 65592),
    ] {
        let font = decode_ctf([&vdmx_font(multipliers, error), &[], &[]], &limits).unwrap();
        assert_eq!(
            font.warnings,
            [ContainerWarning::DroppedTable(TableTag(*b"VDMX"))],
            "{multipliers:?} {error}"
        );
    }
}

#[test]
fn mutated_device_tables_never_panic() {
    let (tables, push, code) = sample_tables();
    let mut state: u64 = 0x5eed_0fd3_71ce_5000;
    let mut next = |below: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % below as u64) as usize
    };
    let interesting = [0x00, 0x01, 0x7f, 0x80, 0xff];
    let limits = ContainerLimits::default();
    let (mut decoded, mut dropped) = (0, 0);
    for round in 0..20_000 {
        let mut tables = tables.clone();
        // VDMX sits first and hdmx fourth.
        let data = &mut tables[if round % 2 == 0 { 0 } else { 3 }].1;
        for _ in 0..1 + next(4) {
            let at = next(data.len());
            data[at] = match next(3) {
                0 => data[at] ^ 1 << next(8),
                1 => interesting[next(interesting.len())],
                _ => next(256) as u8,
            };
        }
        // Long runs of set bits put sizes, heights and widths far out of range.
        if round % 5 == 0 {
            let at = next(data.len());
            let run = next(12_000);
            data.splice(at..at, std::iter::repeat_n(0xff, run));
        }
        if let Ok(font) = decode_ctf([&ctf_block(&tables), &push, &code], &limits) {
            decoded += 1;
            dropped += font.warnings.len();
        }
    }
    assert!(
        decoded > 0 && dropped > 0,
        "{decoded} decoded, {dropped} dropped"
    );
}
