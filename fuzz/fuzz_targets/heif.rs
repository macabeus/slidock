//! A HEIF file as the compiler plans a host's decode of it (`read_heif`): the `meta` box, the primary item, its
//! grid and tiles, and each HEVC configuration's parameter sets (`sps_video_signal`), within the limits. The plan
//! stays within the pixel cap, and what reading it holds is a small multiple of the input.
#![no_main]

use libfuzzer_sys::fuzz_target;
use slidock_fuzz_support::{Counting, measured};
use slidock_io::{BytesReader, HeifLimits, read_heif, sps_video_signal};

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const MAX_PIXELS: u64 = 1 << 24;

fuzz_target!(|data: &[u8]| {
    let limits = HeifLimits::with_pixels(MAX_PIXELS);
    let read = measured(|| read_heif(&BytesReader::new(data), &limits));
    assert!(
        read.peak <= 64 * data.len() as u64 + (1 << 20),
        "planning held {} bytes",
        read.peak
    );
    let _ = sps_video_signal(data);
    let Ok(image) = read.value else {
        return;
    };
    let (width, height) = image.size();
    assert!(u64::from(width) * u64::from(height) <= MAX_PIXELS);
    assert!(image.tiles.len() as u64 <= u64::from(limits.tiles));
    for tile in &image.tiles {
        assert!(tile.config < image.configs.len());
    }
    for config in &image.configs {
        let _ = sps_video_signal(&config.description);
    }
});
