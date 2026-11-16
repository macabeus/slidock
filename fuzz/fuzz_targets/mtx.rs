//! MicroType Express decoding: the LZCOMP blocks, the compact tables and glyphs, and the rebuilt font.
#![no_main]

use libfuzzer_sys::fuzz_target;
use slidock_fonts::container::{ContainerLimits, mtx};
use slidock_fonts::read_sfnt;

fuzz_target!(|data: &[u8]| {
    let limits = ContainerLimits {
        max_output_bytes: 8 << 20,
        ..ContainerLimits::default()
    };
    if let Ok(font) = mtx::decode(data, &limits) {
        assert!(font.sfnt.len() as u64 <= limits.max_output_bytes);
        let _ = read_sfnt(&font.sfnt);
    }
});
