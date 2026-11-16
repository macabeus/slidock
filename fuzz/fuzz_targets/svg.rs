//! An SVG as the frontends and the media writer read it: the allow-list check (`check_svg`), read from a range
//! source as a frontend sizes it, and the picture form the writer copies, which passes the check again with the same
//! size.
#![no_main]

use libfuzzer_sys::fuzz_target;
use slidock_fuzz_support::{Counting, measured};
use slidock_io::{BytesReader, check_svg, check_svg_source, picture_svg};

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fuzz_target!(|data: &[u8]| {
    let checked = measured(|| check_svg(data));
    assert!(checked.peak <= 64 * data.len() as u64 + (1 << 20));
    let head = data.get(..4096).unwrap_or(data);
    let source = check_svg_source(head, &BytesReader::new(data)).expect("a byte source reads");
    assert_eq!(source.as_ref().ok(), checked.value.as_ref().ok());
    let Ok(facts) = checked.value else {
        return;
    };
    let picture = picture_svg(data).expect("a checked SVG has a picture form");
    let again = check_svg(&picture).expect("the picture form passes the allow-list");
    assert_eq!((again.width, again.height), (facts.width, facts.height));
});
