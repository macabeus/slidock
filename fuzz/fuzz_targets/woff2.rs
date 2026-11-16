//! WOFF2 decoding, transforms and collections included.
#![no_main]

use libfuzzer_sys::fuzz_target;
use slidock_fonts::container::{ContainerLimits, woff2};

fuzz_target!(|data: &[u8]| {
    let limits = ContainerLimits {
        max_output_bytes: 8 << 20,
        ..ContainerLimits::default()
    };
    if let Ok(sfnt) = woff2::decode(data, &limits) {
        assert!(sfnt.len() as u64 <= limits.max_output_bytes);
    }
});
