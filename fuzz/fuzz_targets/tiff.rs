//! A TIFF as plan-media's transcode reads it: decoded within a pixel cap, then written as PNG, whose header gives
//! back the decoded size. The decode holds the RGBA image (4 bytes a pixel) and one strip of samples (at most 8
//! bytes a pixel, 16-bit RGBA), never the samples whole beside the image.
#![no_main]

use libfuzzer_sys::fuzz_target;
use slidock_fuzz_support::{Counting, measured};
use slidock_io::{Mime, decode_tiff, encode_png, image_size};

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const MAX_PIXELS: u64 = 1 << 20;

fuzz_target!(|data: &[u8]| {
    let decoded = measured(|| decode_tiff(data, MAX_PIXELS));
    let input = data.len() as u64;
    let Ok(image) = decoded.value else {
        assert!(decoded.peak <= 12 * MAX_PIXELS + 8 * input + (1 << 20));
        return;
    };
    let pixels = u64::from(image.width) * u64::from(image.height);
    assert!(pixels > 0 && pixels <= MAX_PIXELS);
    assert_eq!(image.data.len() as u64, 4 * pixels);
    assert!(
        decoded.peak <= 12 * pixels + 8 * input + (1 << 20),
        "decoding {pixels} pixels held {} bytes",
        decoded.peak
    );
    let png = encode_png(&image).expect("a decoded image encodes");
    let size = image_size(&png, Mime::Png).expect("the PNG has a size");
    assert_eq!((size.width, size.height), (image.width, image.height));
});
