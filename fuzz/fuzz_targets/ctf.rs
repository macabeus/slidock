//! Compact Table Format decoding without the LZCOMP layer: the input is the three decompressed blocks, each of the
//! first two after its 3-byte big-endian length.
#![no_main]

use libfuzzer_sys::fuzz_target;
use slidock_fonts::container::{ContainerLimits, mtx};
use slidock_fonts::read_sfnt;

fn split(data: &[u8]) -> Option<(&[u8], &[u8])> {
    let (length, rest) = data.split_first_chunk::<3>()?;
    let length = length
        .iter()
        .fold(0usize, |value, &byte| value << 8 | usize::from(byte));
    (length <= rest.len()).then(|| rest.split_at(length))
}

fuzz_target!(|data: &[u8]| {
    let Some((font, rest)) = split(data) else {
        return;
    };
    let Some((push, code)) = split(rest) else {
        return;
    };
    let limits = ContainerLimits {
        max_output_bytes: 8 << 20,
        ..ContainerLimits::default()
    };
    if let Ok(font) = mtx::decode_ctf([font, push, code], &limits) {
        assert!(font.sfnt.len() as u64 <= limits.max_output_bytes);
        let _ = read_sfnt(&font.sfnt);
    }
});
