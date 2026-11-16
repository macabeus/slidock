//! The whole entry point: sniff, then whichever decoder, then the sfnt reader on the result. The first byte picks
//! the hint.
#![no_main]

use libfuzzer_sys::fuzz_target;
use slidock_fonts::container::{ContainerLimits, Hint, unwrap_with};
use slidock_fonts::read_sfnt;

fuzz_target!(|data: &[u8]| {
    let Some((&choice, data)) = data.split_first() else {
        return;
    };
    let hint = if choice & 1 == 0 {
        Hint::default()
    } else {
        Hint {
            content_type: None,
            part_name: Some("word/fonts/{6C2E1A5B-0F3D-4C8E-9A7B-1D2E3F405162}.odttf"),
        }
    };
    let limits = ContainerLimits {
        max_output_bytes: 8 << 20,
        ..ContainerLimits::default()
    };
    if let Ok(unwrapped) = unwrap_with(data, &hint, &limits) {
        assert!(unwrapped.sfnt.len() as u64 <= limits.max_output_bytes);
        let _ = read_sfnt(&unwrapped.sfnt);
    }
});
