//! EOT header parsing and unwrapping.
#![no_main]

use libfuzzer_sys::fuzz_target;
use slidock_fonts::container::{ContainerLimits, eot};

fuzz_target!(|data: &[u8]| {
    let limits = ContainerLimits {
        max_output_bytes: 8 << 20,
        ..ContainerLimits::default()
    };
    let _ = eot::parse(data);
    if let Ok(font) = eot::decode(data, &limits) {
        assert!(font.sfnt.len() as u64 <= limits.max_output_bytes);
    }
});
