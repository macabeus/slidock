//! The IR's large parts sit behind boxes, so the values a pass holds and moves stay small: a debug build compiles a
//! deck on a 1 MiB stack, a wasm worker's (`slidock`'s `stack` test runs the corpus decks on one).

use std::mem::size_of;

use slidock_ir::text::{LevelStyles, ResolvedTextBody, TextBody};
use slidock_ir::{presentation, resolved, source};

#[test]
fn keeps_elements_text_and_presentations_small() {
    assert_eq!(size_of::<source::Element>(), 2 * size_of::<usize>());
    assert_eq!(size_of::<resolved::Element>(), 2 * size_of::<usize>());
    assert!(size_of::<LevelStyles>() <= 128);
    assert!(size_of::<presentation::TextStyles>() <= 512);
    assert!(size_of::<TextBody>() <= 512);
    assert!(size_of::<ResolvedTextBody>() <= 512);
    assert!(size_of::<presentation::Defaults>() <= 512);
    assert!(size_of::<source::Presentation>() <= 1024);
    assert!(size_of::<resolved::Presentation>() <= 1024);
    assert!(size_of::<source::Master>() <= 1024);
    assert!(size_of::<resolved::Slide>() <= 1024);
}
