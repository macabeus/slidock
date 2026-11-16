//! Media headers as the frontends and plan-media read them: the sniffed type, the size from a head of every type
//! and from a range source (which reads on past the head), a JPEG's orientation and an ICC profile's colour. None
//! of them decodes pixels, so what they hold is a small multiple of the input whatever size the header claims.
#![no_main]

use libfuzzer_sys::fuzz_target;
use slidock_fuzz_support::{Counting, measured};
use slidock_io::{
    BytesReader, Mime, icc_colour, image_size, jpeg_orientation, read_image_size, sniff_mime,
};

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fuzz_target!(|data: &[u8]| {
    let head = data.get(..4096).unwrap_or(data);
    let source = BytesReader::new(data);
    let read = measured(|| {
        let sniffed = sniff_mime(data.get(..64).unwrap_or(data));
        for mime in Mime::ALL {
            let from_head = image_size(data, mime);
            let read = read_image_size(head, mime, &source).expect("a byte source reads");
            if let Some(size) = from_head.filter(|_| mime != Mime::Heic && mime != Mime::Svg) {
                assert!(size.width > 0 && size.height > 0, "{mime:?}");
            }
            if let Some(size) = read {
                assert!(size.width > 0 && size.height > 0, "{mime:?}");
            }
        }
        let _ = jpeg_orientation(data);
        let _ = icc_colour(data);
        sniffed
    });
    assert!(
        read.peak <= 64 * data.len() as u64 + (1 << 20),
        "reading headers held {} bytes",
        read.peak
    );
});
