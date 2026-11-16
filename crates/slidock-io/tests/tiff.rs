//! The bounded TIFF decoder: the forms it reads, matched against another encoder (the `tiff` crate), and the hostile
//! files it ends.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "test code")]

mod support;

use slidock_io::{BytesReader, ImageSize, Mime, decode_tiff, read_image_size};
use support::{TiffSpec, gradient_rgba, tiff_encoded, tiff_encodings, tiff_file};
use tiff::encoder::{Compression, Predictor};

fn pixels(spec: &TiffSpec, max: u64) -> Vec<u8> {
    decode_tiff(&tiff_file(spec), max).unwrap().data
}

fn message(spec: &TiffSpec, max: u64) -> String {
    decode_tiff(&tiff_file(spec), max).unwrap_err().0
}

#[test]
fn decodes_what_another_encoder_wrote() {
    for (name, file) in tiff_encodings() {
        let image = decode_tiff(&file, 1_000).unwrap();
        assert_eq!((image.width, image.height), (24, 16), "{name}");
        assert_eq!(image.data, gradient_rgba(), "{name}");
    }
}

#[test]
fn reads_uncompressed_and_packbits_strips_in_either_byte_order() {
    let plain = TiffSpec {
        little: false,
        ..TiffSpec::new(2, 1, vec![vec![1, 2, 3, 4, 5, 6]])
    };
    assert_eq!(pixels(&plain, 10), [1, 2, 3, 255, 4, 5, 6, 255]);
    // A literal run of 3, then 3 repeats of 9.
    let packed = TiffSpec {
        compression: 32773,
        ..TiffSpec::new(2, 1, vec![vec![2, 1, 2, 3, 0xfe, 9]])
    };
    assert_eq!(pixels(&packed, 10), [1, 2, 3, 255, 9, 9, 9, 255]);
}

#[test]
fn reads_strips_of_several_rows() {
    let spec = TiffSpec {
        rows_per_strip: Some(2),
        bits: vec![8],
        photometric: 1,
        ..TiffSpec::new(1, 3, vec![vec![1, 2], vec![3]])
    };
    assert_eq!(pixels(&spec, 3), [1, 1, 1, 255, 2, 2, 2, 255, 3, 3, 3, 255]);
    let short = TiffSpec {
        rows_per_strip: Some(2),
        ..spec.clone()
    };
    let missing = TiffSpec {
        strips: vec![vec![1, 2]],
        ..short
    };
    assert_eq!(message(&missing, 3), "The TIFF lacks strips");
    let none = TiffSpec {
        rows_per_strip: Some(0),
        ..spec
    };
    assert_eq!(message(&none, 3), "The TIFF lacks strips");
}

#[test]
fn reads_grey_of_either_polarity_palettes_and_16_bit_samples() {
    let grey = |photometric| {
        pixels(
            &TiffSpec {
                bits: vec![8],
                photometric,
                ..TiffSpec::new(1, 1, vec![vec![10]])
            },
            1,
        )
    };
    assert_eq!(grey(1), [10, 10, 10, 255]);
    assert_eq!(grey(0), [245, 245, 245, 255]);
    let color_map: Vec<u16> = (0..768)
        .map(|i| match i {
            1 => 0xff00,
            257 => 0x8000,
            _ => 0,
        })
        .collect();
    let palette = TiffSpec {
        bits: vec![8],
        photometric: 3,
        color_map: Some(color_map),
        ..TiffSpec::new(1, 1, vec![vec![1]])
    };
    assert_eq!(pixels(&palette, 1), [255, 128, 0, 255]);
    let wide = TiffSpec {
        bits: vec![16, 16, 16],
        ..TiffSpec::new(1, 1, vec![vec![0, 1, 0, 2, 0xff, 3]])
    };
    // The high byte of each little-endian sample.
    assert_eq!(pixels(&wide, 1), [1, 2, 3, 255]);
    let big = TiffSpec {
        little: false,
        ..wide.clone()
    };
    assert_eq!(pixels(&big, 1), [0, 0, 0xff, 255]);
    let grey_alpha = TiffSpec {
        bits: vec![8, 8],
        photometric: 1,
        extra_samples: Some(2),
        ..TiffSpec::new(1, 1, vec![vec![7, 9]])
    };
    assert_eq!(pixels(&grey_alpha, 1), [7, 7, 7, 9]);
}

#[test]
fn refuses_a_palette_without_its_colour_map() {
    let palette = TiffSpec {
        bits: vec![8],
        photometric: 3,
        color_map: Some(vec![0; 767]),
        ..TiffSpec::new(1, 1, vec![vec![1]])
    };
    assert_eq!(
        message(&palette, 1),
        "A palette TIFF without its colour map"
    );
}

#[test]
fn straightens_an_associated_alpha_and_keeps_an_unassociated_one() {
    let page = |extra_samples| TiffSpec {
        bits: vec![8, 8, 8, 8],
        extra_samples: Some(extra_samples),
        ..TiffSpec::new(1, 1, vec![vec![50, 100, 0, 128]])
    };
    assert_eq!(pixels(&page(1), 1), [100, 199, 0, 128]);
    assert_eq!(pixels(&page(2), 1), [50, 100, 0, 128]);
}

#[test]
fn undoes_the_horizontal_predictor_on_8_bit_samples_only() {
    let predicted = TiffSpec {
        predictor: Some(2),
        ..TiffSpec::new(2, 1, vec![vec![10, 20, 30, 1, 2, 3]])
    };
    assert_eq!(pixels(&predicted, 2), [10, 20, 30, 255, 11, 22, 33, 255]);
    let wide = TiffSpec {
        bits: vec![16, 16, 16],
        predictor: Some(2),
        ..TiffSpec::new(1, 1, vec![vec![0; 6]])
    };
    assert_eq!(message(&wide, 1), "The TIFF predictor needs 8-bit samples");
    let float = TiffSpec {
        predictor: Some(3),
        ..TiffSpec::new(1, 1, vec![vec![0; 3]])
    };
    assert_eq!(message(&float, 1), "TIFF predictor 3 is not supported");
}

#[test]
fn refuses_the_forms_it_does_not_read() {
    let page = TiffSpec::new(1, 1, vec![vec![1, 2, 3]]);
    let with = |change: fn(&mut TiffSpec)| {
        let mut spec = page.clone();
        change(&mut spec);
        spec
    };
    assert_eq!(
        message(&with(|s| s.shorts = vec![(322, vec![16])]), 1),
        "Tiled TIFFs are not supported"
    );
    assert_eq!(
        message(&with(|s| s.shorts = vec![(284, vec![2])]), 1),
        "Planar TIFFs are not supported"
    );
    assert_eq!(
        message(&with(|s| s.bits = vec![4, 4, 4]), 1),
        "4/4/4-bit TIFF samples are not supported"
    );
    assert_eq!(
        message(&with(|s| s.bits = vec![8, 16, 8]), 1),
        "8/16/8-bit TIFF samples are not supported"
    );
    assert_eq!(
        message(&with(|s| s.compression = 7), 1),
        "TIFF compression 7 is not supported"
    );
    assert_eq!(
        message(&with(|s| s.photometric = 5), 1),
        "TIFF photometric 5 with 3 samples is not supported"
    );
    assert_eq!(
        message(&with(|s| s.bits = vec![8, 8, 8, 8, 8]), 1),
        "TIFFs of 5 samples are not supported"
    );
    let big = TiffSpec::new(10, 10, vec![vec![0; 300]]);
    assert_eq!(message(&big, 99), "The TIFF's 100 pixels are over 99");
    assert!(decode_tiff(&[0; 8], 1).is_err());
    assert_eq!(decode_tiff(b"II*\0", 1).unwrap_err().0, "Not a TIFF file");
    assert_eq!(
        decode_tiff(b"II\x2b\0\x08\0\0\0", 1).unwrap_err().0,
        "Not a classic TIFF file"
    );
}

#[test]
fn decodes_the_first_of_the_largest_pages() {
    let mut out = std::io::Cursor::new(Vec::new());
    let mut encoder = tiff::encoder::TiffEncoder::new(&mut out).unwrap();
    encoder
        .write_image::<tiff::encoder::colortype::RGB8>(1, 1, &[9, 9, 9])
        .unwrap();
    encoder
        .write_image::<tiff::encoder::colortype::RGB8>(2, 1, &[1, 2, 3, 1, 2, 3])
        .unwrap();
    encoder
        .write_image::<tiff::encoder::colortype::RGB8>(1, 2, &[4, 5, 6, 4, 5, 6])
        .unwrap();
    let image = decode_tiff(&out.into_inner(), 10).unwrap();
    assert_eq!((image.width, image.height), (2, 1));
    assert_eq!(image.data, [1, 2, 3, 255, 1, 2, 3, 255]);
}

#[test]
fn ends_a_page_chain_that_loops_back_on_itself() {
    let looping = TiffSpec {
        next: |ifd| ifd,
        ..TiffSpec::new(1, 1, vec![vec![1, 2, 3]])
    };
    assert_eq!(pixels(&looping, 1), [1, 2, 3, 255]);
    // A 64-byte file whose one page points at itself and holds zero strips.
    let mut bare = vec![0u8; 64];
    bare[..34].copy_from_slice(&[
        0x49, 0x49, 42, 0, 8, 0, 0, 0, 2, 0, 0, 1, 3, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 1, 3, 0, 1, 0,
        0, 0, 1, 0, 0, 0,
    ]);
    bare[8 + 2 + 2 * 12..8 + 2 + 2 * 12 + 4].copy_from_slice(&[8, 0, 0, 0]);
    assert_eq!(
        decode_tiff(&bare, 1).unwrap_err().0,
        "1-bit TIFF samples are not supported"
    );
}

#[test]
fn refuses_a_chain_longer_than_its_page_cap() {
    let page = tiff_file(&TiffSpec::new(1, 1, vec![vec![1, 2, 3]]));
    let ifd = u32::from_le_bytes(page[4..8].try_into().unwrap()) as usize;
    // Many copies of the page's tags, each pointing at the next.
    let tags = &page[ifd..page.len() - 4];
    let mut chain = page[..ifd].to_vec();
    for i in 0..100 {
        let next = if i < 99 {
            (ifd + (i + 1) * (tags.len() + 4)) as u32
        } else {
            0
        };
        chain.extend(tags);
        chain.extend(next.to_le_bytes());
    }
    assert_eq!(
        decode_tiff(&chain, 1).unwrap_err().0,
        "A TIFF holds more than 64 pages"
    );
}

#[test]
fn inflates_a_strip_only_as_far_as_its_rows_reach() {
    let bomb = miniz_oxide::deflate::compress_to_vec_zlib(&vec![0; 16 * 1024 * 1024], 6);
    let page = TiffSpec {
        compression: 8,
        ..TiffSpec::new(1, 1, vec![bomb])
    };
    assert_eq!(pixels(&page, 1), [0, 0, 0, 255]);
    // A Deflate strip is inflated as fflate inflates it into a buffer of the strip's size: a stored block past the
    // rows is refused, and a strip without its checksum loses those last four bytes.
    let grey = |strip: Vec<u8>| TiffSpec {
        compression: 8,
        bits: vec![8],
        photometric: 1,
        ..TiffSpec::new(2, 1, vec![strip])
    };
    let stored = |length: u8| {
        let mut strip = vec![0x78, 0x01, 0x01, length, 0, !length, 0xff];
        strip.extend(1..=length);
        strip.extend([0; 4]);
        strip
    };
    assert_eq!(pixels(&grey(stored(2)), 2), [1, 1, 1, 255, 2, 2, 2, 255]);
    assert_eq!(
        message(&grey(stored(10)), 2),
        "A TIFF strip does not inflate: Inflated more than 2 bytes"
    );
    let unchecked = miniz_oxide::deflate::compress_to_vec_zlib(&[7, 8], 6);
    assert_eq!(
        pixels(&grey(unchecked.clone()), 2),
        [7, 7, 7, 255, 8, 8, 8, 255]
    );
    assert_eq!(
        pixels(&grey(unchecked[..unchecked.len() - 4].to_vec()), 2),
        [0, 0, 0, 255, 0, 0, 0, 255]
    );
    // An LZW strip that would write more stops at its rows too.
    let lzw = tiff_encoded(Compression::Lzw, Predictor::None);
    let image = decode_tiff(&lzw, 1_000).unwrap();
    assert_eq!(image.data.len(), 24 * 16 * 4);
}

#[test]
fn refuses_tag_values_and_strips_past_the_end_of_the_file() {
    let page = tiff_file(&TiffSpec::new(1, 1, vec![vec![1, 2, 3]]));
    assert_eq!(
        decode_tiff(&page[..9], 1).unwrap_err().0,
        "A TIFF offset lies past the file"
    );
    // A strip whose byte count reaches past the file.
    let long = TiffSpec::new(1, 1, vec![vec![1, 2, 3]]);
    let mut file = tiff_file(&long);
    let ifd = u32::from_le_bytes(file[4..8].try_into().unwrap()) as usize;
    let count = u16::from_le_bytes([file[ifd], file[ifd + 1]]) as usize;
    for entry in 0..count {
        let at = ifd + 2 + entry * 12;
        if u16::from_le_bytes([file[at], file[at + 1]]) == 279 {
            file[at + 8..at + 12].copy_from_slice(&10_000u32.to_le_bytes());
        }
    }
    assert_eq!(
        decode_tiff(&file, 1).unwrap_err().0,
        "A TIFF offset lies past the file"
    );
}

#[test]
fn reads_the_size_of_tags_that_follow_the_pixels_past_the_head() {
    let big = tiff_file(&TiffSpec::new(300, 80, vec![vec![0; 300 * 80 * 3]]));
    let head = &big[..64 * 1024];
    assert_eq!(
        read_image_size(head, Mime::Tiff, &BytesReader::new(&big)).unwrap(),
        Some(ImageSize {
            width: 300,
            height: 80,
            dpi: None,
            frames: None
        })
    );
}
