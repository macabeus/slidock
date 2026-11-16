//! The HEIF reader on Apple-encoded fixtures (`probes/heif/make-media.sh`) and on files built here from their coded
//! tiles: each property the plan reads, each refusal, and truncated, mutated and oversized tables, which return errors.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use slidock_io::{
    BytesReader, Crop, Extent, HeifError, HeifImage, HeifLimits, IccColour, Mime, MirrorAxis,
    Transform, icc_colour, read_heif, read_image_size,
};
use support::{Seeded, mutate};

const SINGLE: &[u8] = include_bytes!("../../../probes/heif/single.heic");
const GRID: &[u8] = include_bytes!("../../../probes/heif/grid.heic");
const ODD: &[u8] = include_bytes!("../../../probes/heif/odd.heic");
const LIMITS: HeifLimits = HeifLimits::with_pixels(50_000_000);

fn plan(bytes: &[u8]) -> Result<HeifImage, HeifError> {
    read_heif(&BytesReader::new(bytes), &LIMITS)
}

fn data(bytes: &[u8], extents: &[Extent]) -> Vec<u8> {
    extents
        .iter()
        .flat_map(|extent| &bytes[extent.offset as usize..(extent.offset + extent.len) as usize])
        .copied()
        .collect()
}

fn unsupported(result: Result<HeifImage, HeifError>) -> String {
    match result {
        Err(HeifError::Unsupported(reason)) => reason,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn malformed(result: Result<HeifImage, HeifError>) -> String {
    match result {
        Err(HeifError::Malformed(reason)) => reason,
        other => panic!("expected a malformed file, got {other:?}"),
    }
}

#[test]
fn plans_a_single_coded_image() {
    let image = plan(SINGLE).unwrap();
    assert_eq!((image.coded_width, image.coded_height), (64, 48));
    assert_eq!((image.columns, image.rows), (1, 1));
    assert_eq!((image.tile_width, image.tile_height), (64, 48));
    assert_eq!(image.size(), (64, 48));
    assert_eq!(image.transforms, []);
    assert_eq!(image.configs.len(), 1);
    let config = &image.configs[0];
    // HEVC Main Still Picture, 4:2:0, 8 bits, as an iPhone writes its photos.
    assert!(config.codec.starts_with("hvc1.3.E.L"), "{}", config.codec);
    assert_eq!(
        (
            config.chroma_format,
            config.bit_depth_luma,
            config.bit_depth_chroma
        ),
        (1, 8, 8)
    );
    assert_eq!(config.description[0], 1);
    let nclx = image.nclx.unwrap();
    assert!(nclx.full_range);
    assert_eq!(image.icc, None);
    assert_eq!(image.bits.as_deref(), Some(&[8, 8, 8][..]));
    let [tile] = image.tiles.as_slice() else {
        panic!("one tile");
    };
    assert_eq!((tile.config, tile.x, tile.y), (0, 0, 0));
    // The coded data sit in `mdat` and start with a NAL unit's 4-byte length.
    let coded = data(SINGLE, &tile.extents);
    let first = u32::from_be_bytes(coded[..4].try_into().unwrap()) as usize;
    assert!(first + 4 <= coded.len());
}

#[test]
fn plans_a_grid_of_tiles_in_row_major_order() {
    let image = plan(GRID).unwrap();
    assert_eq!((image.coded_width, image.coded_height), (1100, 600));
    assert_eq!((image.columns, image.rows), (3, 2));
    assert_eq!((image.tile_width, image.tile_height), (512, 512));
    assert_eq!(image.size(), (1100, 600));
    let places: Vec<(u32, u32)> = image.tiles.iter().map(|tile| (tile.x, tile.y)).collect();
    assert_eq!(
        places,
        [
            (0, 0),
            (512, 0),
            (1024, 0),
            (0, 512),
            (512, 512),
            (1024, 512)
        ]
    );
    // Every tile shares one configuration.
    assert_eq!(image.configs.len(), 1);
    assert!(image.tiles.iter().all(|tile| tile.config == 0));
    // Tagged Display P3: an ICC profile (`acsp` at byte 36), no nclx.
    let icc = image.icc.unwrap();
    assert_eq!(&icc[36..40], b"acsp");
    assert_eq!(image.nclx, None);
    // Each tile's data lies in the file and differs from its neighbours'.
    let tiles: Vec<Vec<u8>> = image
        .tiles
        .iter()
        .map(|tile| data(GRID, &tile.extents))
        .collect();
    assert!(tiles.iter().all(|tile| tile.len() > 8));
    assert_ne!(tiles[0], tiles[1]);
}

#[test]
fn builds_the_rfc_6381_codec_string_from_the_configuration() {
    // ISO/IEC 14496-15 Annex E: profile space and idc, the compatibility flags in reverse bit order, tier and level,
    // the constraint bytes without the trailing zero ones.
    let mut built = Built::single(&[]);
    built.hvcc = hvcc_record(0b0110_0001, 0x6000_0000, [0xb0, 0, 0, 0, 0, 0], 93);
    assert_eq!(
        plan(&built.bytes()).unwrap().configs[0].codec,
        "hvc1.A1.6.H93.B0"
    );
    built.hvcc = hvcc_record(0b0000_0010, 0x2000_0000, [0; 6], 120);
    assert_eq!(
        plan(&built.bytes()).unwrap().configs[0].codec,
        "hvc1.2.4.L120"
    );
    built.hvcc = hvcc_record(0b1100_0011, 0x7000_0000, [0x90, 0, 0x01, 0, 0, 0], 90);
    assert_eq!(
        plan(&built.bytes()).unwrap().configs[0].codec,
        "hvc1.C3.E.L90.90.0.1"
    );
}

#[test]
fn rebuilt_fixtures_plan_as_the_originals() {
    // The builder below writes what the encoder wrote: the same plan, with the data moved.
    let original = plan(SINGLE).unwrap();
    let built = Built::single(&[]).bytes();
    let rebuilt = plan(&built).unwrap();
    assert_eq!(rebuilt.size(), original.size());
    assert_eq!(rebuilt.configs, original.configs);
    assert_eq!(
        data(&built, &rebuilt.tiles[0].extents),
        data(SINGLE, &original.tiles[0].extents)
    );
    let original = plan(GRID).unwrap();
    let built = Built::grid(&[]).bytes();
    let rebuilt = plan(&built).unwrap();
    assert_eq!(rebuilt.size(), original.size());
    assert_eq!(rebuilt.tiles.len(), 6);
    for (a, b) in rebuilt.tiles.iter().zip(&original.tiles) {
        assert_eq!((a.x, a.y), (b.x, b.y));
        assert_eq!(data(&built, &a.extents), data(GRID, &b.extents));
    }
}

#[test]
fn reads_rotation_mirror_and_clean_aperture_in_their_order() {
    let rotated = plan(&Built::single(&[irot(1)]).bytes()).unwrap();
    assert_eq!(rotated.transforms, [Transform::Rotate(1)]);
    assert_eq!(rotated.size(), (48, 64));
    // No rotation is no transform.
    assert_eq!(
        plan(&Built::single(&[irot(0)]).bytes()).unwrap().transforms,
        []
    );
    // `imir` axis 0 swaps top and bottom, 1 left and right: what WebKit's decoder (ImageIO) and libheif 1.23 draw
    // of these files.
    let flipped = plan(&Built::single(&[imir(0)]).bytes()).unwrap();
    assert_eq!(
        flipped.transforms,
        [Transform::Mirror(MirrorAxis::Horizontal)]
    );
    let mirrored = plan(&Built::single(&[imir(1)]).bytes()).unwrap();
    assert_eq!(
        mirrored.transforms,
        [Transform::Mirror(MirrorAxis::Vertical)]
    );

    // A centred 32 × 24 aperture of the 64 × 48 image, then a quarter turn, then a mirror.
    let all = plan(&Built::single(&[clap(32, 24, 0, 0), irot(3), imir(1)]).bytes()).unwrap();
    assert_eq!(
        all.transforms,
        [
            Transform::Crop(Crop {
                x: 16,
                y: 12,
                width: 32,
                height: 24
            }),
            Transform::Rotate(3),
            Transform::Mirror(MirrorAxis::Vertical),
        ]
    );
    assert_eq!(all.size(), (24, 32));
    // An aperture listed after the rotation is a rectangle of the rotated image (48 × 64), offset right and up.
    let after = plan(&Built::single(&[irot(1), clap(16, 16, 8, -4)]).bytes()).unwrap();
    assert_eq!(
        after.transforms,
        [
            Transform::Rotate(1),
            Transform::Crop(Crop {
                x: 24,
                y: 20,
                width: 16,
                height: 16
            }),
        ]
    );
    assert_eq!(after.size(), (16, 16));
    // The grid's own rotation turns its output.
    let grid = plan(&Built::grid(&[irot(1)]).bytes()).unwrap();
    assert_eq!(grid.size(), (600, 1100));
}

/// Apple's encoder codes a 61 × 45 picture as 62 × 46 and cuts it back with a clean aperture of half-pixel offsets.
#[test]
fn plans_the_encoders_own_clean_aperture() {
    let image = plan(ODD).unwrap();
    assert_eq!((image.coded_width, image.coded_height), (62, 46));
    assert_eq!(
        image.transforms,
        [Transform::Crop(Crop {
            x: 0,
            y: 0,
            width: 61,
            height: 45
        })]
    );
    assert_eq!(image.size(), (61, 45));
}

#[test]
fn refuses_an_aperture_outside_the_image_and_repeated_transforms() {
    assert!(malformed(plan(&Built::single(&[clap(80, 24, 0, 0)]).bytes())).contains("clap"));
    assert!(malformed(plan(&Built::single(&[clap(32, 24, 40, 0)]).bytes())).contains("clap"));
    let mut zero = clap(32, 24, 0, 0);
    zero[12..16].copy_from_slice(&0u32.to_be_bytes());
    assert!(malformed(plan(&Built::single(&[zero]).bytes())).contains("zero"));
    assert!(unsupported(plan(&Built::single(&[irot(1), irot(2)]).bytes())).contains("repeats"));
}

#[test]
fn refuses_what_is_outside_the_subset_with_its_reason() {
    let mut av1 = Built::single(&[]);
    av1.primary_kind = *b"av01";
    assert_eq!(
        unsupported(plan(&av1.bytes())),
        "its primary image is a `av01` item"
    );
    let mut overlay = Built::grid(&[]);
    overlay.primary_kind = *b"iovl";
    assert_eq!(
        unsupported(plan(&overlay.bytes())),
        "its primary image is a `iovl` item"
    );
    let essential = Built::single(&[property(b"a1op", &[0])]);
    assert_eq!(
        unsupported(plan(&essential.bytes())),
        "it has an essential `a1op` property, which is not read"
    );
    // The same property, not essential, is ignored.
    let mut optional = Built::single(&[property(b"a1op", &[0])]);
    optional.essential = false;
    assert!(plan(&optional.bytes()).is_ok());

    let mut alpha = Built::single(&[]);
    alpha.alpha = Some(b"urn:mpeg:mpegB:cicp:systems:auxiliary:alpha".to_vec());
    assert_eq!(
        unsupported(plan(&alpha.bytes())),
        "its primary image has an alpha plane"
    );
    // A depth map or a gain map is no alpha: it is left out.
    alpha.alpha = Some(b"urn:com:apple:photo:2020:aux:hdrgainmap".to_vec());
    assert!(plan(&alpha.bytes()).is_ok());

    let mut rotated_tile = Built::grid(&[]);
    rotated_tile.tile_extra = vec![irot(1)];
    assert_eq!(
        unsupported(plan(&rotated_tile.bytes())),
        "a tile of its grid is transformed on its own"
    );
    let mut elsewhere = Built::single(&[]);
    elsewhere.data_reference = 1;
    assert_eq!(
        unsupported(plan(&elsewhere.bytes())),
        "an image's data is in another file"
    );
    let mut constructed = Built::single(&[]);
    constructed.construction = 2;
    assert_eq!(
        unsupported(plan(&constructed.bytes())),
        "an image's data is built by construction method 2"
    );
    let mut protected = Built::single(&[]);
    protected.protection = 1;
    assert_eq!(
        unsupported(plan(&protected.bytes())),
        "its primary image is protected"
    );
}

#[test]
fn refuses_grids_that_do_not_add_up() {
    let mut short = Built::grid(&[]);
    short.drop_last_tile = true;
    assert_eq!(
        malformed(plan(&short.bytes())),
        "its 3 × 2 grid names 5 tiles"
    );
    let mut wide = Built::grid(&[]);
    wide.grid_size = (1600, 600);
    assert_eq!(
        malformed(plan(&wide.bytes())),
        "its grid is 1600 × 600 but its `ispe` says 1100 × 600"
    );
    wide.ispe = (1600, 600);
    assert_eq!(
        malformed(plan(&wide.bytes())),
        "the tiles of its grid do not cover it"
    );
    let mut missing = Built::single(&[]);
    missing.hvcc = Vec::new();
    assert_eq!(
        malformed(plan(&missing.bytes())),
        "item 1 has no `hvcC` configuration"
    );
}

#[test]
fn refuses_images_and_tables_over_the_limits() {
    let pixels = HeifLimits::with_pixels(1100 * 600 - 1);
    assert!(matches!(
        read_heif(&BytesReader::new(GRID), &pixels),
        Err(HeifError::TooLarge { .. })
    ));
    // The tiles' pixels count, overhang included: 6 × 512 × 512.
    let tiles = HeifLimits::with_pixels(6 * 512 * 512 - 1);
    assert!(matches!(
        read_heif(&BytesReader::new(GRID), &tiles),
        Err(HeifError::TooLarge { pixels, .. }) if pixels == 6 * 512 * 512
    ));
    let few_tiles = HeifLimits { tiles: 5, ..LIMITS };
    assert_eq!(
        unsupported(read_heif(&BytesReader::new(GRID), &few_tiles)),
        "its grid has 6 tiles, more than the 5 read"
    );
    let small_meta = HeifLimits {
        meta_bytes: 100,
        ..LIMITS
    };
    assert!(unsupported(read_heif(&BytesReader::new(GRID), &small_meta)).contains("`meta` box"));
    // Each configuration is a decoder a host configures in turn.
    let mut configured = Built::grid(&[]);
    configured.tile_configs = true;
    let image = plan(&configured.bytes()).unwrap();
    assert_eq!(image.configs.len(), 6);
    let few_configs = HeifLimits {
        configs: 5,
        ..LIMITS
    };
    assert_eq!(
        unsupported(read_heif(
            &BytesReader::new(&configured.bytes()),
            &few_configs
        )),
        "its tiles use more than 5 HEVC configurations"
    );
    let few_items = HeifLimits { items: 3, ..LIMITS };
    assert!(unsupported(read_heif(&BytesReader::new(GRID), &few_items)).contains("more than"));
}

#[test]
fn refuses_huge_counts_before_allocating_for_them() {
    // An `iinf`, `iloc` and `ipma` claiming 2^32 − 1 entries, and a 256 × 256 grid: each is refused, none allocated.
    let mut iinf = Built::single(&[]);
    iinf.iinf_count = Some(u32::MAX);
    assert!(unsupported(plan(&iinf.bytes())).contains("`iinf` lists 4294967295"));
    let mut iloc = Built::single(&[]);
    iloc.iloc_count = Some(u32::MAX);
    assert!(unsupported(plan(&iloc.bytes())).contains("`iloc` lists 4294967295"));
    let mut ipma = Built::single(&[]);
    ipma.ipma_count = Some(u32::MAX);
    assert!(unsupported(plan(&ipma.bytes())).contains("`ipma` lists"));
    let mut grid = Built::grid(&[]);
    grid.grid_shape = Some((255, 255));
    assert_eq!(
        unsupported(plan(&grid.bytes())),
        "its grid has 65536 tiles, more than the 1024 read"
    );
}

#[test]
fn refuses_a_grid_whose_tiles_span_past_32_bits() {
    // Three columns of 2^31-pixel-wide tiles: the third would start at 2^32.
    let mut wide = Built::grid(&[]);
    wide.tile_size = (2_147_483_648, 482);
    assert_eq!(
        malformed(plan(&wide.bytes())),
        "its 3 × 2 grid of 2147483648 × 482 tiles spans 6442450944 × 964 pixels"
    );
    // Within 32 bits, the tiles' pixels are refused before any is placed.
    wide.tile_size = (1_431_655_765, 482);
    let limited = HeifLimits::with_pixels(u64::MAX);
    assert!(read_heif(&BytesReader::new(&wide.bytes()), &limited).is_ok());
    assert!(matches!(
        plan(&wide.bytes()),
        Err(HeifError::TooLarge { pixels, .. }) if pixels == 3 * 1_431_655_765 * 964
    ));
}

#[test]
fn reads_no_more_coded_data_than_the_file_holds() {
    // One item listing its range over and over adds up to more than the file.
    let mut repeated = Built::single(&[]);
    repeated.extent_repeats = 4096;
    let bytes = repeated.bytes();
    let reason = malformed(plan(&bytes));
    let len = repeated.tiles[0].len() as u64;
    assert_eq!(
        reason,
        format!(
            "item 1's data adds up to {} bytes, more than the file's {}",
            4096 * len,
            bytes.len()
        )
    );
    // Each tile within the file, the tiles together past it.
    let mut tiles = Built::grid(&[]);
    let longest = tiles.tiles.iter().map(Vec::len).max().unwrap();
    let file = tiles.bytes().len();
    let repeats = (file / longest) as u16;
    tiles.extent_repeats = repeats;
    let bytes = tiles.bytes();
    let total: usize = tiles.tiles.iter().map(Vec::len).sum::<usize>() * usize::from(repeats);
    assert!(longest * usize::from(repeats) <= bytes.len() && total > bytes.len());
    assert_eq!(
        malformed(plan(&bytes)),
        format!(
            "its tiles' data add up to {total} bytes, more than the file's {}",
            bytes.len()
        )
    );
    // Listed twice within the file, a range is read twice.
    let mut twice = Built::single(&[]);
    twice.extent_repeats = 2;
    let image = plan(&twice.bytes()).unwrap();
    assert_eq!(image.tiles[0].extents.len(), 2);
}

/// A HEIC is sized as its primary image shows: cropped and turned, with no pixel limit (sizing decodes nothing).
#[test]
fn sizes_a_heic_as_its_image_shows() {
    let size = |bytes: &[u8]| {
        read_image_size(
            &bytes[..bytes.len().min(16)],
            Mime::Heic,
            &BytesReader::new(bytes),
        )
        .unwrap()
        .map(|size| (size.width, size.height))
    };
    assert_eq!(size(SINGLE), Some((64, 48)));
    assert_eq!(size(GRID), Some((1100, 600)));
    assert_eq!(
        size(&Built::single(&[clap(32, 24, 0, 0), irot(1)]).bytes()),
        Some((24, 32))
    );
    let mut alpha = Built::single(&[]);
    alpha.alpha = Some(b"urn:mpeg:hevc:2015:auxid:1".to_vec());
    assert_eq!(size(&alpha.bytes()), None);
    assert_eq!(size(&SINGLE[..SINGLE.len() / 2]), None);
}

#[test]
fn every_truncation_is_an_error() {
    for fixture in [SINGLE, GRID] {
        for len in 0..fixture.len() {
            assert!(
                plan(&fixture[..len]).is_err(),
                "{len} of {} bytes",
                fixture.len()
            );
        }
    }
}

#[test]
fn mutations_return_a_result() {
    let mut random = Seeded::new(0x4e1f);
    let iterations = support::fuzz_iterations();
    assert!(iterations > 0);
    let bases = [
        SINGLE.to_vec(),
        GRID.to_vec(),
        Built::single(&[clap(32, 24, 0, 0), irot(3), imir(1)]).bytes(),
    ];
    for i in 0..iterations {
        let mut bytes = bases[i % bases.len()].clone();
        for _ in 0..1 + random.below(3) {
            bytes = mutate(&bytes, &mut random);
        }
        let _ = plan(&bytes);
    }
}

/// A size field of the builder's: values at the edges of 16 and 32 bits, and random ones.
fn size_field(random: &mut Seeded) -> u32 {
    const EDGES: [u32; 12] = [
        0,
        1,
        2,
        511,
        512,
        65_535,
        65_536,
        1 << 31,
        (1 << 31) + 1,
        u32::MAX / 3 + 1,
        u32::MAX - 1,
        u32::MAX,
    ];
    if random.below(2) == 0 {
        EDGES[random.below(EDGES.len())]
    } else {
        (random.next() * 4_294_967_296.0) as u32
    }
}

#[test]
fn structured_sizes_return_a_result() {
    // The fields a byte mutation rarely lands on whole: tile and image sizes, grid shapes (each naming the builder's
    // six tiles), clean apertures and repeated extents, in files that are otherwise well formed.
    let mut random = Seeded::new(0x51ce);
    let iterations = support::fuzz_iterations();
    assert!(iterations > 0);
    let shapes = [(1, 2), (2, 1), (0, 5), (5, 0)];
    let unlimited = HeifLimits::with_pixels(u64::MAX);
    for i in 0..iterations {
        let mut built = if i % 2 == 0 {
            let mut grid = Built::grid(&[]);
            grid.grid_shape = Some(shapes[random.below(shapes.len())]);
            let size = (
                size_field(&mut random) & 0xffff,
                size_field(&mut random) & 0xffff,
            );
            grid.grid_size = size;
            grid.ispe = if random.below(2) == 0 {
                size
            } else {
                (size_field(&mut random), size_field(&mut random))
            };
            grid
        } else {
            let clap_box = clap(
                size_field(&mut random),
                size_field(&mut random),
                size_field(&mut random) as i32,
                size_field(&mut random) as i32,
            );
            let mut single = Built::single(&[clap_box, irot(random.below(4) as u8)]);
            single.ispe = (size_field(&mut random), size_field(&mut random));
            single
        };
        built.tile_size = (size_field(&mut random), size_field(&mut random));
        built.extent_repeats = [1, 2, 3, 4096][random.below(4)];
        let bytes = built.bytes();
        for limits in [&LIMITS, &unlimited] {
            if let Ok(image) = read_heif(&BytesReader::new(&bytes), limits) {
                let (width, height) = image.size();
                assert!(width > 0 && height > 0);
            }
        }
    }
}

// The builder: an `ftyp`, a `meta` box of the given items and properties, and an `mdat` of the coded data, taken from
// the fixtures (so a browser decodes what it writes).

fn full_box(kind: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut payload = (u32::from(version) << 24 | flags).to_be_bytes().to_vec();
    payload.extend_from_slice(body);
    plain_box(kind, &payload)
}

fn plain_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out
}

fn property(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    plain_box(kind, payload)
}

fn irot(turns: u8) -> Vec<u8> {
    plain_box(b"irot", &[turns])
}

fn imir(axis: u8) -> Vec<u8> {
    plain_box(b"imir", &[axis])
}

/// A clean aperture of `width` × `height` whose centre is `dx`, `dy` from the image's.
fn clap(width: u32, height: u32, dx: i32, dy: i32) -> Vec<u8> {
    let mut payload = Vec::new();
    for value in [width, 1, height, 1, dx as u32, 1, dy as u32, 1] {
        payload.extend_from_slice(&value.to_be_bytes());
    }
    plain_box(b"clap", &payload)
}

/// An `hvcC` record with no parameter sets: what the reader reads of one (a decoder would need them).
fn hvcc_record(profile: u8, compatibility: u32, constraints: [u8; 6], level: u8) -> Vec<u8> {
    let mut out = vec![1, profile];
    out.extend_from_slice(&compatibility.to_be_bytes());
    out.extend_from_slice(&constraints);
    out.extend_from_slice(&[level, 0xf0, 0, 0xfc, 0xfd, 0xf8, 0xf8, 0, 0, 0x0f, 0]);
    out
}

/// The `hvcC` payload and each tile's coded data of a fixture.
fn coded(fixture: &[u8]) -> (Vec<u8>, Vec<Vec<u8>>) {
    let image = plan(fixture).unwrap();
    let tiles = image
        .tiles
        .iter()
        .map(|tile| data(fixture, &tile.extents))
        .collect();
    (image.configs[0].description.clone(), tiles)
}

struct Built {
    hvcc: Vec<u8>,
    tiles: Vec<Vec<u8>>,
    tile_size: (u32, u32),
    /// `Some` for a grid: its output size.
    grid_size: (u32, u32),
    grid: bool,
    ispe: (u32, u32),
    primary_kind: [u8; 4],
    /// The image's properties beyond its size and configuration.
    extra: Vec<Vec<u8>>,
    essential: bool,
    tile_extra: Vec<Vec<u8>>,
    alpha: Option<Vec<u8>>,
    data_reference: u16,
    construction: u8,
    protection: u16,
    drop_last_tile: bool,
    grid_shape: Option<(u8, u8)>,
    iinf_count: Option<u32>,
    iloc_count: Option<u32>,
    ipma_count: Option<u32>,
    /// How many times each coded item's `iloc` entry lists its range.
    extent_repeats: u16,
    /// Each tile of a grid has an `hvcC` of its own (its level differs).
    tile_configs: bool,
}

impl Built {
    fn single(extra: &[Vec<u8>]) -> Built {
        let (hvcc, tiles) = coded(SINGLE);
        Built {
            hvcc,
            tiles,
            tile_size: (64, 48),
            grid_size: (64, 48),
            grid: false,
            ispe: (64, 48),
            primary_kind: *b"hvc1",
            extra: extra.to_vec(),
            essential: true,
            tile_extra: Vec::new(),
            alpha: None,
            data_reference: 0,
            construction: 0,
            protection: 0,
            drop_last_tile: false,
            grid_shape: None,
            iinf_count: None,
            iloc_count: None,
            ipma_count: None,
            extent_repeats: 1,
            tile_configs: false,
        }
    }

    fn grid(extra: &[Vec<u8>]) -> Built {
        let (hvcc, tiles) = coded(GRID);
        Built {
            tile_size: (512, 512),
            grid_size: (1100, 600),
            grid: true,
            ispe: (1100, 600),
            primary_kind: *b"grid",
            hvcc,
            tiles,
            ..Built::single(extra)
        }
    }

    /// The file: `ftyp`, `meta`, `mdat`.
    fn bytes(&self) -> Vec<u8> {
        let ftyp = plain_box(b"ftyp", b"heic\0\0\0\0mif1heic");
        let draft = self.meta(0);
        let mdat_data = ftyp.len() + draft.len() + 8;
        let meta = self.meta(mdat_data as u32);
        assert_eq!(meta.len(), draft.len());
        let mut out = ftyp;
        out.extend_from_slice(&meta);
        out.extend_from_slice(&plain_box(b"mdat", &self.tiles.concat()));
        out
    }

    fn meta(&self, mdat_data: u32) -> Vec<u8> {
        // Items: the tiles of a grid first (1…n), then the image, then an alpha plane.
        let tiles = if self.drop_last_tile {
            self.tiles.len() - 1
        } else {
            self.tiles.len()
        };
        let primary = if self.grid { tiles as u16 + 1 } else { 1 };
        let alpha_id = primary + 1;
        let mut items: Vec<(u16, [u8; 4], u16)> = Vec::new();
        if self.grid {
            for id in 1..=tiles as u16 {
                items.push((id, *b"hvc1", 0));
            }
        }
        items.push((primary, self.primary_kind, self.protection));
        if self.alpha.is_some() {
            items.push((alpha_id, *b"hvc1", 0));
        }

        let mut infe = Vec::new();
        for (id, kind, protection) in &items {
            let mut body = id.to_be_bytes().to_vec();
            body.extend_from_slice(&protection.to_be_bytes());
            body.extend_from_slice(kind);
            body.push(0);
            // Tiles and auxiliary images are hidden, as encoders write them.
            let hidden = u32::from(*id != primary);
            infe.extend_from_slice(&full_box(b"infe", 2, hidden, &body));
        }
        let mut iinf_body = (self.iinf_count.unwrap_or(items.len() as u32))
            .to_be_bytes()
            .to_vec();
        iinf_body.extend_from_slice(&infe);

        // Locations: the tiles (or the single image) in mdat; a grid's descriptor in idat.
        let (rows, columns) = self.grid_shape.unwrap_or((1, 2));
        let descriptor = [
            &[0u8, 0, rows, columns][..],
            &(self.grid_size.0 as u16).to_be_bytes(),
            &(self.grid_size.1 as u16).to_be_bytes(),
        ]
        .concat();
        let mut locations: Vec<(u16, u8, u32, u32)> = Vec::new();
        let mut at = mdat_data;
        for (index, tile) in self.tiles.iter().enumerate() {
            let id = if self.grid { index as u16 + 1 } else { 1 };
            if index < tiles {
                locations.push((id, self.construction, at, tile.len() as u32));
            }
            at += tile.len() as u32;
        }
        if self.grid {
            locations.push((primary, 1, 0, descriptor.len() as u32));
        }
        if self.alpha.is_some() {
            locations.push((alpha_id, 0, mdat_data, self.tiles[0].len() as u32));
        }
        let mut iloc_body = vec![0x44, 0x00];
        iloc_body
            .extend_from_slice(&(self.iloc_count.unwrap_or(locations.len() as u32)).to_be_bytes());
        for (id, construction, offset, len) in &locations {
            iloc_body.extend_from_slice(&u32::from(*id).to_be_bytes());
            iloc_body.extend_from_slice(&u16::from(*construction).to_be_bytes());
            let reference = if *id == primary || !self.grid {
                self.data_reference
            } else {
                0
            };
            iloc_body.extend_from_slice(&reference.to_be_bytes());
            let repeats = if *construction == 1 && *id == primary && self.grid {
                1
            } else {
                self.extent_repeats
            };
            iloc_body.extend_from_slice(&repeats.to_be_bytes());
            for _ in 0..repeats {
                iloc_body.extend_from_slice(&offset.to_be_bytes());
                iloc_body.extend_from_slice(&len.to_be_bytes());
            }
        }

        // Properties: 1 hvcC, 2 tile ispe, 3 image ispe, then the extras, the tile extras and the alpha's auxC.
        let ispe = |(w, h): (u32, u32)| {
            full_box(b"ispe", 0, 0, &[w.to_be_bytes(), h.to_be_bytes()].concat())
        };
        let mut ipco = Vec::new();
        if !self.hvcc.is_empty() {
            ipco.push(plain_box(b"hvcC", &self.hvcc));
        } else {
            ipco.push(plain_box(b"free", &[]));
        }
        ipco.push(ispe(self.tile_size));
        ipco.push(ispe(self.ispe));
        let extra_from = ipco.len() + 1;
        ipco.extend(self.extra.iter().cloned());
        let tile_extra_from = ipco.len() + 1;
        ipco.extend(self.tile_extra.iter().cloned());
        let tile_config_from = ipco.len() + 1;
        if self.tile_configs {
            for tile in 0..tiles {
                let mut record = self.hvcc.clone();
                record[12] = record[12].wrapping_add(tile as u8 + 1);
                ipco.push(plain_box(b"hvcC", &record));
            }
        }
        let aux_index = ipco.len() + 1;
        if let Some(urn) = &self.alpha {
            let mut body = urn.clone();
            body.push(0);
            ipco.push(full_box(b"auxC", 0, 0, &body));
        }
        let essential = |index: usize, yes: bool| (index as u8) | if yes { 0x80 } else { 0 };
        let mut associations: Vec<(u16, Vec<u8>)> = Vec::new();
        let extras: Vec<u8> = (extra_from..tile_extra_from)
            .map(|index| essential(index, self.essential))
            .collect();
        if self.grid {
            let tile_extras: Vec<u8> = (tile_extra_from..aux_index)
                .map(|index| essential(index, true))
                .collect();
            for id in 1..=tiles as u16 {
                let hvcc = if self.tile_configs {
                    essential(tile_config_from + usize::from(id) - 1, true)
                } else {
                    0x81
                };
                associations.push((id, [vec![hvcc, 0x82], tile_extras.clone()].concat()));
            }
            associations.push((primary, [vec![0x83], extras].concat()));
        } else {
            let hvcc = if self.hvcc.is_empty() { 0x01 } else { 0x81 };
            associations.push((primary, [vec![hvcc, 0x83], extras].concat()));
        }
        if self.alpha.is_some() {
            associations.push((alpha_id, vec![0x81, 0x83, essential(aux_index, true)]));
        }
        let mut ipma_body = (self.ipma_count.unwrap_or(associations.len() as u32))
            .to_be_bytes()
            .to_vec();
        for (id, list) in &associations {
            ipma_body.extend_from_slice(&id.to_be_bytes());
            ipma_body.push(list.len() as u8);
            ipma_body.extend_from_slice(list);
        }
        let iprp = plain_box(
            b"iprp",
            &[
                plain_box(b"ipco", &ipco.concat()),
                full_box(b"ipma", 0, 0, &ipma_body),
            ]
            .concat(),
        );

        let mut references = Vec::new();
        if self.grid {
            let mut body = primary.to_be_bytes().to_vec();
            body.extend_from_slice(&(tiles as u16).to_be_bytes());
            for id in 1..=tiles as u16 {
                body.extend_from_slice(&id.to_be_bytes());
            }
            references.push(plain_box(b"dimg", &body));
        }
        if self.alpha.is_some() {
            let body = [
                alpha_id.to_be_bytes(),
                1u16.to_be_bytes(),
                primary.to_be_bytes(),
            ]
            .concat();
            references.push(plain_box(b"auxl", &body));
        }

        let mut body = Vec::new();
        body.extend_from_slice(&full_box(
            b"hdlr",
            0,
            0,
            b"\0\0\0\0pict\0\0\0\0\0\0\0\0\0\0\0\0\0",
        ));
        body.extend_from_slice(&full_box(b"pitm", 0, 0, &primary.to_be_bytes()));
        body.extend_from_slice(&full_box(b"iinf", 1, 0, &iinf_body));
        if !references.is_empty() {
            body.extend_from_slice(&full_box(b"iref", 0, 0, &references.concat()));
        }
        body.extend_from_slice(&iprp);
        if self.grid {
            body.extend_from_slice(&plain_box(b"idat", &descriptor));
        }
        body.extend_from_slice(&full_box(b"iloc", 2, 0, &iloc_body));
        full_box(b"meta", 0, 0, &body)
    }
}

/// An ICC profile of these colourants and red tone curve: a zero header, a tag table, the tags.
fn icc(colourants: [[f64; 3]; 3], curve: Vec<u8>) -> Vec<u8> {
    let fixed = |value: f64| ((value * 65_536.0).round() as i32).to_be_bytes();
    let mut tags: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    for (signature, xyz) in [*b"rXYZ", *b"gXYZ", *b"bXYZ"].into_iter().zip(colourants) {
        let mut data = b"XYZ \0\0\0\0".to_vec();
        for value in xyz {
            data.extend_from_slice(&fixed(value));
        }
        tags.push((signature, data));
    }
    tags.push((*b"rTRC", curve));
    let mut out = vec![0u8; 128];
    out.extend_from_slice(&(tags.len() as u32).to_be_bytes());
    let mut offset = 128 + 4 + 12 * tags.len();
    let mut data = Vec::new();
    for (signature, tag) in &tags {
        out.extend_from_slice(signature);
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(tag.len() as u32).to_be_bytes());
        offset += tag.len();
        data.extend_from_slice(tag);
    }
    out.extend_from_slice(&data);
    out
}

fn parametric(params: [f64; 5]) -> Vec<u8> {
    let mut out = b"para\0\0\0\0\0\x03\0\0".to_vec();
    for value in params {
        out.extend_from_slice(&((value * 65_536.0).round() as i32).to_be_bytes());
    }
    out
}

fn sampled(entries: u16, curve: impl Fn(f64) -> f64) -> Vec<u8> {
    let mut out = b"curv\0\0\0\0".to_vec();
    out.extend_from_slice(&u32::from(entries).to_be_bytes());
    for index in 0..entries {
        let y = curve(f64::from(index) / f64::from(entries - 1));
        out.extend_from_slice(&((y * 65_535.0).round() as u16).to_be_bytes());
    }
    out
}

const SRGB: [[f64; 3]; 3] = [
    [0.4361, 0.2225, 0.0139],
    [0.3851, 0.7169, 0.0971],
    [0.1431, 0.0606, 0.7141],
];
const BT2020: [[f64; 3]; 3] = [
    [0.6735, 0.2790, -0.0019],
    [0.1657, 0.6753, 0.0300],
    [0.1250, 0.0456, 0.7968],
];

#[test]
fn names_the_primaries_and_curve_an_icc_profile_matches() {
    // Apple's Display P3, as the grid fixture carries it.
    let profile = plan(GRID).unwrap().icc.unwrap();
    assert_eq!(
        icc_colour(&profile),
        IccColour {
            primaries: Some(12),
            transfer: Some(13)
        }
    );
    // sRGB with a sampled curve; BT.2020 with BT.709's parametric curve.
    let srgb = |x: f64| {
        if x <= 0.04045 {
            x / 12.92
        } else {
            slidock_math::exp(2.4 * slidock_math::log((x + 0.055) / 1.055))
        }
    };
    assert_eq!(
        icc_colour(&icc(SRGB, sampled(1024, srgb))),
        IccColour {
            primaries: Some(1),
            transfer: Some(13)
        }
    );
    assert_eq!(
        icc_colour(&icc(
            BT2020,
            parametric([2.22223, 0.90967, 0.09033, 0.22223, 0.08099])
        )),
        IccColour {
            primaries: Some(9),
            transfer: Some(1)
        }
    );
    // A plain gamma 2.2 curve and colourants of no known space name nothing.
    let gamma = sampled(1024, |x: f64| {
        if x == 0.0 {
            0.0
        } else {
            slidock_math::exp(2.2 * slidock_math::log(x))
        }
    });
    let mut odd = SRGB;
    odd[0][0] += 0.01;
    assert_eq!(icc_colour(&icc(odd, gamma)), IccColour::default());
    // A truncated profile names nothing, and never panics.
    let whole = icc(SRGB, sampled(64, srgb));
    for len in 0..whole.len() {
        let _ = icc_colour(&whole[..len]);
    }
    assert_eq!(icc_colour(&whole[..140]), IccColour::default());
}
