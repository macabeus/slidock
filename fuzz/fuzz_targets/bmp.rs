//! A BMP as plan-media's transcode reads it: decoded within a pixel cap straight into RGBA, then written as PNG,
//! whose header gives back the decoded size.
#![no_main]

use libfuzzer_sys::fuzz_target;
use slidock_fuzz_support::{Counting, measured};
use slidock_io::{Mime, decode_bmp, encode_png, image_size, transcode_to_png};

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const MAX_PIXELS: u64 = 1 << 20;

fuzz_target!(|data: &[u8]| {
    let decoded = measured(|| decode_bmp(data, MAX_PIXELS));
    let input = data.len() as u64;
    let Ok(image) = decoded.value else {
        assert!(decoded.peak <= 4 * MAX_PIXELS + 2 * input + (64 << 10));
        return;
    };
    let pixels = u64::from(image.width) * u64::from(image.height);
    assert!(pixels > 0 && pixels <= MAX_PIXELS);
    assert_eq!(image.data.len() as u64, 4 * pixels);
    assert!(
        decoded.peak <= 4 * pixels + 2 * input + (64 << 10),
        "decoding {pixels} pixels held {} bytes",
        decoded.peak
    );
    let png = encode_png(&image).expect("a decoded image encodes");
    let size = image_size(&png, Mime::Png).expect("the PNG has a size");
    assert_eq!((size.width, size.height), (image.width, image.height));
    assert_eq!(transcode_to_png(data, MAX_PIXELS).ok(), Some(png));
});
