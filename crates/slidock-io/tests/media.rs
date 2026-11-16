//! The BMP decoder, the transcode to PNG, and the video probe.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "test code")]

mod support;

use slidock_io::{
    BytesReader, TranscodeError, VideoFacts, decode_bmp, probe_video, sha256_range,
    transcode_to_png,
};
use support::{BmpSpec, bmp_file, gradient_rgba, movie, red_bmp, tiff_encodings};

const PIXELS: u64 = 50_000_000;

fn bmp_pixels(spec: &BmpSpec, max: u64) -> Vec<u8> {
    decode_bmp(&bmp_file(spec), max).unwrap().data
}

fn bmp_error(spec: &BmpSpec, max: u64) -> String {
    decode_bmp(&bmp_file(spec), max).unwrap_err().0
}

#[test]
fn reads_24_bit_rows_bottom_up_as_bgr() {
    // Stored bottom row first: blue, then red over it.
    let spec = BmpSpec::new(1, 2, 24, vec![vec![255, 0, 0], vec![0, 0, 255]]);
    let image = decode_bmp(&bmp_file(&spec), 100).unwrap();
    assert_eq!((image.width, image.height), (1, 2));
    assert_eq!(image.data, [255, 0, 0, 255, 0, 0, 255, 255]);
}

#[test]
fn reads_top_down_rows_when_the_height_is_negative() {
    let spec = BmpSpec::new(1, -2, 24, vec![vec![255, 0, 0], vec![0, 0, 255]]);
    assert_eq!(bmp_pixels(&spec, 100), [0, 0, 255, 255, 255, 0, 0, 255]);
}

#[test]
fn reads_32_bit_bit_fields_with_alpha_and_paletted_rows() {
    let fields = BmpSpec {
        compression: 3,
        masks: vec![0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0xff00_0000],
        ..BmpSpec::new(1, 1, 32, vec![vec![0x30, 0x20, 0x10, 0x80]])
    };
    // A 40-byte header with BI_BITFIELDS carries three masks; the alpha mask needs a V4 header.
    assert_eq!(bmp_pixels(&fields, 100), [0x10, 0x20, 0x30, 255]);
    // BI_ALPHABITFIELDS carries the fourth.
    let alpha = BmpSpec {
        compression: 6,
        ..fields
    };
    assert_eq!(bmp_pixels(&alpha, 100), [0x10, 0x20, 0x30, 0x80]);
    // In a V4 or V5 header the masks sit at the same offset, inside it, with the alpha mask.
    for header_size in [108, 124] {
        let inside = BmpSpec {
            header_size,
            compression: 3,
            masks: vec![0x0000_00ff, 0x0000_ff00, 0x00ff_0000, 0xff00_0000],
            ..BmpSpec::new(1, 1, 32, vec![vec![0x30, 0x20, 0x10, 0x80]])
        };
        assert_eq!(bmp_pixels(&inside, 100), [0x30, 0x20, 0x10, 0x80]);
    }
    // A mask with gaps scales its bits past the lowest run too, past 255 and modulo 256.
    let gapped = BmpSpec {
        compression: 3,
        masks: vec![0b1011, 0xff00, 0xff_0000],
        ..BmpSpec::new(1, 1, 32, vec![vec![0b1011, 0, 0, 0]])
    };
    assert_eq!(bmp_pixels(&gapped, 100), [167, 0, 0, 255]);
    let paletted = BmpSpec {
        palette: vec![[10, 20, 30], [40, 50, 60]],
        ..BmpSpec::new(3, 1, 8, vec![vec![1, 0, 1]])
    };
    assert_eq!(
        bmp_pixels(&paletted, 100),
        [40, 50, 60, 255, 10, 20, 30, 255, 40, 50, 60, 255]
    );
    let bits = BmpSpec {
        palette: vec![[0, 0, 0], [255, 255, 255]],
        ..BmpSpec::new(3, 1, 1, vec![vec![0b1010_0000]])
    };
    let reds: Vec<u8> = bmp_pixels(&bits, 100).iter().step_by(4).copied().collect();
    assert_eq!(reds, [255, 0, 255]);
    let nibbles = BmpSpec {
        palette: vec![[1, 1, 1], [2, 2, 2], [3, 3, 3]],
        ..BmpSpec::new(2, 1, 4, vec![vec![0x21]])
    };
    assert_eq!(bmp_pixels(&nibbles, 100), [3, 3, 3, 255, 2, 2, 2, 255]);
    let outside = BmpSpec {
        palette: vec![[1, 1, 1]],
        ..BmpSpec::new(1, 1, 8, vec![vec![1]])
    };
    assert_eq!(bmp_error(&outside, 100), "BMP palette index out of range");
}

#[test]
fn reads_16_bit_rows_as_555_unless_masked() {
    let white = BmpSpec::new(1, 1, 16, vec![0x7fffu16.to_le_bytes().to_vec()]);
    assert_eq!(bmp_pixels(&white, 1), [255, 255, 255, 255]);
    // 5-bit channels scale by 255/31, rounded: 16 → 132.
    let mid = BmpSpec::new(1, 1, 16, vec![(16u16 << 10).to_le_bytes().to_vec()]);
    assert_eq!(bmp_pixels(&mid, 1), [132, 0, 0, 255]);
    let rgb565 = BmpSpec {
        compression: 3,
        masks: vec![0xf800, 0x07e0, 0x001f],
        ..BmpSpec::new(1, 1, 16, vec![0x07e0u16.to_le_bytes().to_vec()])
    };
    assert_eq!(bmp_pixels(&rgb565, 1), [0, 255, 0, 255]);
}

#[test]
fn refuses_what_it_does_not_decode_and_images_over_the_pixel_cap() {
    assert!(decode_bmp(&[0; 30], 100).is_err());
    let rle = BmpSpec {
        compression: 1,
        ..BmpSpec::new(1, 1, 8, vec![vec![0]])
    };
    assert_eq!(bmp_error(&rle, 100), "BMP compression 1 is not supported");
    let over = BmpSpec::new(20, 20, 24, vec![]);
    assert_eq!(bmp_error(&over, 100), "BMP of 400 pixels is over 100");
    let truncated = bmp_file(&BmpSpec::new(2, 2, 24, vec![]));
    assert_eq!(
        decode_bmp(&truncated[..60], 100).unwrap_err().0,
        "Truncated BMP pixels"
    );
    let fields = BmpSpec {
        compression: 3,
        ..BmpSpec::new(1, 1, 24, vec![vec![0; 3]])
    };
    assert_eq!(
        bmp_error(&fields, 1),
        "Bit fields need 16 or 32 bits per pixel"
    );
    assert_eq!(
        bmp_error(&BmpSpec::new(1, 1, 2, vec![vec![0]]), 1),
        "2-bit BMP is not supported"
    );
    assert_eq!(bmp_error(&BmpSpec::new(0, 1, 24, vec![]), 1), "Empty BMP");
}

#[test]
fn gives_the_pinned_png_for_tiff_whatever_its_compression() {
    let pngs: Vec<String> = tiff_encodings()
        .iter()
        .map(|(_, file)| {
            let png = transcode_to_png(file, PIXELS).unwrap();
            sha256_range(&BytesReader::new(&png), 0, png.len() as u64).unwrap()
        })
        .collect();
    assert!(pngs.iter().all(|sha| *sha == pngs[0]), "{pngs:?}");
    assert_eq!(
        pngs[0],
        "951f165d505fe6e7b343120b3940ed85ef92acc42311a18a9b49127b76ca7a58"
    );
    // And it holds the pixels.
    let png = transcode_to_png(&tiff_encodings()[0].1, PIXELS).unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(&png))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut pixels).unwrap();
    assert_eq!(pixels, gradient_rgba());
}

#[test]
fn gives_the_pinned_png_for_a_bmp() {
    let png = transcode_to_png(&red_bmp(), PIXELS).unwrap();
    // The pinned bytes of the deterministic PNG encoder.
    assert_eq!(
        sha256_range(&BytesReader::new(&png), 0, png.len() as u64).unwrap(),
        "29ce6573f4f98438dcd0d74d11a12e7a5177b5528659cf22e1e73741219780e3"
    );
}

#[test]
fn refuses_images_over_the_pixel_limit() {
    let tiff = &tiff_encodings()[0].1;
    assert!(matches!(
        transcode_to_png(tiff, 10),
        Err(TranscodeError::Tiff(_))
    ));
    assert!(matches!(
        transcode_to_png(&red_bmp(), 0),
        Err(TranscodeError::Bmp(_))
    ));
}

fn probe(bytes: &[u8]) -> VideoFacts {
    probe_video(&BytesReader::new(bytes)).unwrap()
}

#[test]
fn names_the_video_track_codec_wherever_moov_sits() {
    assert_eq!(
        probe(&movie(b"vide", b"avc1", true, None)).codec.as_deref(),
        Some("avc1")
    );
    assert_eq!(
        probe(&movie(b"vide", b"hvc1", false, None))
            .codec
            .as_deref(),
        Some("hvc1")
    );
}

#[test]
fn reads_the_length_the_movie_header_states_and_none_from_a_header_without_one() {
    assert_eq!(
        probe(&movie(b"vide", b"avc1", true, Some(12_345))),
        VideoFacts {
            codec: Some("avc1".into()),
            duration_ms: Some(12_345.0)
        }
    );
    assert_eq!(
        probe(&movie(b"vide", b"avc1", false, None)),
        VideoFacts {
            codec: Some("avc1".into()),
            duration_ms: None
        }
    );
}

#[test]
fn reads_an_all_ones_duration_as_unknown() {
    let duration = |ms: u32| probe(&movie(b"vide", b"avc1", true, Some(ms))).duration_ms;
    assert_eq!(duration(0xffff_ffff), None);
    assert_eq!(duration(0xffff_fffe), Some(4_294_967_294.0));
    assert_eq!(duration(0), None);
    // In a version 1 header the duration is 64 bits, and the same 32-bit all-ones value is unknown.
    let wide = |duration: u64| {
        let mut file = movie(b"vide", b"avc1", true, Some(1));
        let at = file
            .windows(4)
            .position(|window| window == b"mvhd")
            .unwrap()
            - 4;
        let mvhd = [
            &40u32.to_be_bytes()[..],
            b"mvhd",
            &[1, 0, 0, 0],
            &[0; 16],
            &1000u32.to_be_bytes(),
            &duration.to_be_bytes(),
        ]
        .concat();
        // The version 0 header is 28 bytes; the moov box grows by 12.
        file.splice(at..at + 28, mvhd);
        let moov = at - 8;
        let size = u32::from_be_bytes(file[moov..moov + 4].try_into().unwrap()) + 12;
        file[moov..moov + 4].copy_from_slice(&size.to_be_bytes());
        probe(&file).duration_ms
    };
    assert_eq!(wide(0xffff_ffff), None);
    assert_eq!(wide(0x1_0000_0000), Some(4_294_967_296.0));
}

#[test]
fn ignores_non_video_tracks_and_broken_boxes() {
    assert_eq!(probe(&movie(b"soun", b"mp4a", true, None)).codec, None);
    assert_eq!(
        probe(&movie(b"vide", b"avc1", true, None)[..40]),
        VideoFacts::default()
    );
}

#[test]
fn reads_large_sizes_and_a_last_box_to_the_end_of_the_file() {
    let ftyp = [&20u32.to_be_bytes()[..], b"ftypisom", &[0; 8]].concat();
    let inner = movie(b"vide", b"av01", true, Some(1500));
    // The movie's `moov`, rewritten with a 64-bit size, then as a box running to the end.
    let moov_at = 16;
    let moov_len = u32::from_be_bytes(inner[moov_at..moov_at + 4].try_into().unwrap()) as usize;
    let body = &inner[moov_at + 8..moov_at + moov_len];
    let large = [
        &ftyp[..],
        &1u32.to_be_bytes(),
        b"moov",
        &(16 + body.len() as u64).to_be_bytes(),
        body,
    ]
    .concat();
    assert_eq!(probe(&large).duration_ms, Some(1500.0));
    let open_ended = [&ftyp[..], &0u32.to_be_bytes(), b"moov", body].concat();
    assert_eq!(probe(&open_ended).codec.as_deref(), Some("av01"));
}
