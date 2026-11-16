//! The private fixtures of the golden files (`--features golden`; skips when they are absent): the TIFFs Pillow wrote
//! decode to its pixels, and every IR fixture's media sniffs as the type, and sizes as the dimensions, the fixture
//! records.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

#[path = "../../../tests/golden/support.rs"]
mod golden;

use std::path::PathBuf;

use slidock_io::{
    BytesReader, Mime, decode_tiff, image_size, sha256_range, sniff_mime, transcode_to_png,
};

fn read(path: PathBuf) -> Vec<u8> {
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn decodes_the_tiffs_pillow_wrote() {
    let Some(golden) = golden::Golden::open("slidock-io tiff") else {
        return;
    };
    // 24 × 16 RGB pixels, as RGBA.
    let rgba: Vec<u8> = read(golden.fixture("tiff/px.bin"))
        .chunks(3)
        .flat_map(|p| [p[0], p[1], p[2], 255])
        .collect();
    let mut pngs = Vec::new();
    for name in ["lzw.tif", "lzwpred.tif", "deflate.tif"] {
        let file = read(golden.fixture(&format!("tiff/{name}")));
        let image = decode_tiff(&file, 1_000).unwrap();
        assert_eq!((image.width, image.height), (24, 16), "{name}");
        assert_eq!(image.data, rgba, "{name}");
        pngs.push(transcode_to_png(&file, 50_000_000).unwrap());
    }
    assert!(pngs.windows(2).all(|pair| pair[0] == pair[1]));
}

#[test]
fn sniffs_and_sizes_every_ir_fixture_medium_as_the_fixture_records() {
    let Some(golden) = golden::Golden::open("slidock-io media") else {
        return;
    };
    let media_files = golden.fixture_names("ir/media");
    let mut checked = 0;
    for name in golden.ir_fixtures() {
        let path = golden.fixture(&format!("ir/{name}.presentation-ir.json"));
        let ir: serde_json::Value = serde_json::from_slice(&read(path)).unwrap();
        let Some(media) = ir["media"].as_object() else {
            continue;
        };
        for (sha, medium) in media {
            let kind = medium["type"].as_str().unwrap();
            let Some(file) = media_files
                .iter()
                .find(|file| file.split('.').next() == Some(sha.as_str()))
            else {
                continue;
            };
            let bytes = read(golden.fixture(&format!("ir/media/{file}")));
            let reader = BytesReader::new(&bytes);
            assert_eq!(&sha256_range(&reader, 0, bytes.len() as u64).unwrap(), sha);
            let mime = sniff_mime(&bytes);
            assert_eq!(mime.map(Mime::as_str), Some(kind), "{sha}");
            if let (Some(mime), Some(width)) = (mime, medium["width"].as_u64())
                && kind.starts_with("image/")
                && mime != Mime::Svg
            {
                let size = image_size(&bytes, mime).unwrap_or_else(|| panic!("{sha} has no size"));
                let height = medium["height"].as_u64().unwrap();
                assert_eq!(
                    (u64::from(size.width), u64::from(size.height)),
                    (width, height),
                    "{sha}"
                );
            }
            checked += 1;
        }
    }
    assert!(checked >= 11, "checked {checked} media");
}
