//! WOFF 1.0 decoding.
#![no_main]

use libfuzzer_sys::fuzz_target;
use slidock_fonts::container::{ContainerLimits, woff};

fuzz_target!(|data: &[u8]| {
    let limits = ContainerLimits {
        max_output_bytes: 8 << 20,
        ..ContainerLimits::default()
    };
    if let Ok(font) = woff::decode(data, &limits) {
        assert!(font.sfnt.len() as u64 <= limits.max_output_bytes);
    }
});
