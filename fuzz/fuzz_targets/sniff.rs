//! `sniff` with each kind of hint.
#![no_main]

use libfuzzer_sys::fuzz_target;
use slidock_fonts::container::{Hint, sniff};

fuzz_target!(|data: &[u8]| {
    let hints = [
        Hint::default(),
        Hint {
            content_type: Some("application/vnd.openxmlformats-officedocument.obfuscatedFont"),
            part_name: None,
        },
        Hint {
            content_type: None,
            part_name: Some("word/fonts/{6C2E1A5B-0F3D-4C8E-9A7B-1D2E3F405162}.odttf"),
        },
    ];
    for hint in &hints {
        let _ = sniff(data, hint);
    }
});
