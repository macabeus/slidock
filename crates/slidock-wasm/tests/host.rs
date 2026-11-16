//! The worker's host natively: it prints the document set and the deck the facade prints, bundles the bytes
//! `slidock::compile` writes in 1 MiB chunks, and answers stages out of order, refused inputs and bad fonts with
//! typed errors.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use serde_json::{Value, json};
use slidock::{BundleOptions, BytesReader, Compiler, FontSource, Frontends, Input, Options};
use slidock_wasm::host::{CHUNK_BYTES, Host, MediaBytes, Swaps};
use slidock_wasm::protocol::ErrorKind;

const NAME: &str = "hello.presentation-ir.json";
const SCHEMA: &str = slidock::SCHEMA_URL;

fn text_shape(id: &str, text: &str) -> Value {
    json!({
        "id": id, "origin": { "kind": "agent" }, "kind": "shape",
        "frame": { "x": 60, "y": 160, "w": 600, "h": 80, "rotation": 0, "flipH": false, "flipV": false },
        "geometry": { "kind": "preset", "name": "rect", "adjust": {} },
        "text": { "body": {}, "paragraphs": [{ "level": 0, "props": {}, "inlines": [{ "kind": "run", "text": text,
            "props": { "font": { "kind": "family", "family": "Liberation Sans" }, "size": 24,
                       "color": { "kind": "rgb", "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 1 } } }] }] }
    })
}

/// Two slides of text, as agent-written IR.
fn presentation() -> Vec<u8> {
    let level = json!({
        "para": { "align": "left", "bullet": { "kind": "none" }, "indent": 0,
                  "lineSpacing": { "basis": "size-1.2", "kind": "multiple", "value": 1 }, "marginLeft": 0,
                  "marginRight": 0, "rtl": false, "spaceAfter": { "kind": "pt", "value": 0 },
                  "spaceBefore": { "kind": "pt", "value": 0 }, "tabSize": 72 },
        "run": { "baseline": 0, "caps": "none", "color": { "kind": "theme", "mods": [], "slot": "tx1" },
                 "font": { "kind": "theme", "slot": "minor" }, "highlight": null, "italic": false,
                 "letterSpacing": 0, "link": null, "scriptScale": 1, "size": 18, "strike": "none",
                 "synthesizeBold": true, "underline": "none", "weight": 400 }
    });
    let text: serde_json::Map<String, Value> =
        (0..9).map(|i| (i.to_string(), level.clone())).collect();
    let slide = |id: &str, words: &str| {
        json!({ "id": id, "origin": { "kind": "agent" }, "hidden": false, "showLayout": true, "timeline": [],
                "elements": [text_shape(&format!("{id}.t"), words)] })
    };
    serde_json::to_vec(&json!({
        "$schema": SCHEMA, "size": { "w": 720, "h": 405 },
        "defaults": { "body": { "anchor": "top", "autofit": { "kind": "none" },
                                "insets": { "b": 3.6, "l": 7.2, "r": 7.2, "t": 3.6 }, "overflow": "spread",
                                "wrap": "square" }, "text": text },
        "themes": {}, "masters": {}, "layouts": {}, "fonts": {}, "media": {}, "phase": "source",
        "slides": [slide("ir-s1", "Hello"), slide("ir-s2", "World")]
    }))
    .unwrap()
}

fn opened(bytes: Vec<u8>) -> Host {
    let mut host = Host::new();
    host.open(NAME, Box::new(BytesReader::new(bytes)), false)
        .unwrap();
    host
}

#[test]
fn prints_the_document_set_the_facade_compiles() {
    let bytes = presentation();
    let compiler = Compiler::new(Frontends::default()).unwrap();
    let reader = BytesReader::new(bytes.clone());
    let compiled = compiler
        .compile(&Input::new(NAME, &reader), &Options::default())
        .unwrap();

    let mut host = opened(bytes);
    host.finish().unwrap();
    // The slide templates come from the bundler; the document set is kept whether or not they print.
    let _ = host.print();
    assert_eq!(Some(host.documents().unwrap()), compiled.documents());
}

/// Needs `slidock_bundle::{slide_templates, bundle}`: fails while they return `Unimplemented`.
#[test]
fn prints_the_deck_and_bundles_the_bytes_compile_writes_in_whole_chunks() {
    struct Collect(Vec<u8>);
    impl slidock::BundleSink for Collect {
        fn write(&mut self, _path: &str, bytes: &[u8]) -> Result<(), slidock::BundleError> {
            self.0.extend_from_slice(bytes);
            Ok(())
        }
    }
    let bytes = presentation();
    let mut collected = Collect(Vec::new());
    let reader = BytesReader::new(bytes.clone());
    let compiled = slidock::compile(
        &Input::new(NAME, &reader),
        Frontends::default(),
        &Options::default(),
        &BundleOptions::default(),
        &mut collected,
    )
    .unwrap();
    let expected = collected.0;

    let mut host = opened(bytes);
    host.finish().unwrap();
    let printed = host.print().unwrap();
    // The printed deck carries no player stamp: the bundle writes the one it embeds.
    assert_eq!(printed.deck.player, None);
    // The bundle titles its deck JSON with the page's title, which its id hashes too.
    assert_eq!(
        compiled.deck.title,
        slidock::page_defaults(&printed.deck).title
    );
    let bundled = slidock::deck::Deck {
        player: None,
        title: printed.deck.title.clone(),
        id: printed.deck.id.clone(),
        ..compiled.deck.clone()
    };
    assert_eq!(printed.deck, bundled);
    assert_eq!(printed.diagnostics, compiled.diagnostics);
    let ids: Vec<&str> = printed.slides.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["ir-s1", "ir-s2"]);
    assert!(
        printed
            .media
            .iter()
            .all(|media| media.bytes == MediaBytes::Written)
    );

    let mut chunks: Vec<Vec<u8>> = Vec::new();
    let report = host
        .bundle(&BundleOptions::default(), Swaps::default(), &mut |chunk| {
            chunks.push(chunk.to_vec());
            Ok(())
        })
        .unwrap();
    assert_eq!(report, compiled.report);
    assert_eq!(chunks.concat(), expected);
    let (last, whole) = chunks.split_last().unwrap();
    assert!(whole.iter().all(|chunk| chunk.len() == CHUNK_BYTES));
    assert!(!last.is_empty() && last.len() <= CHUNK_BYTES);
    // The deck stays printed: a second bundle writes the same bytes.
    let mut again = Vec::new();
    host.bundle(&BundleOptions::default(), Swaps::default(), &mut |chunk| {
        again.extend_from_slice(chunk);
        Ok(())
    })
    .unwrap();
    assert_eq!(again, expected);
}

#[test]
fn refuses_what_is_not_a_presentation_and_leaves_nothing_open() {
    let mut host = opened(presentation());
    let refused = host
        .open(
            "notes.txt",
            Box::new(BytesReader::new(b"not a deck".to_vec())),
            false,
        )
        .unwrap_err();
    assert_eq!(refused.kind, ErrorKind::Refused);
    assert_eq!(refused.diagnostics[0].code.code(), "IR0902");
    assert_eq!(host.font_needs().unwrap_err().kind, ErrorKind::Request);
}

#[test]
fn answers_stages_out_of_order_with_request_errors() {
    let mut host = opened(presentation());
    assert_eq!(host.print().unwrap_err().kind, ErrorKind::Request);
    assert_eq!(
        host.bundle(&BundleOptions::default(), Swaps::default(), &mut |_| Ok(()))
            .unwrap_err()
            .kind,
        ErrorKind::Request
    );
    assert_eq!(
        host.media_bytes("media/x.png").unwrap_err().kind,
        ErrorKind::Request
    );
    assert_eq!(host.documents().unwrap_err().kind, ErrorKind::Request);
}

#[test]
fn takes_fonts_after_a_finish_and_finishes_again() {
    let mut host = opened(presentation());
    host.finish().unwrap();
    // A finished session reopens for the font; a file that is no font is a request error.
    let bad = host
        .add_font(b"not a font".to_vec(), FontSource::User, None)
        .unwrap_err();
    assert_eq!(bad.kind, ErrorKind::Request);
    assert!(bad.message.contains("font"), "{}", bad.message);
    host.finish().unwrap();
    host.finish().unwrap();
    assert!(host.font_needs().is_ok());
}

/// The header of a baseline JPEG of `width` × `height` (SOI, SOF0, EOI): what the compiler reads of one.
fn jpeg(width: u16, height: u16) -> Vec<u8> {
    let mut out = vec![0xff, 0xd8, 0xff, 0xc0, 0, 17, 8];
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1, 0xff, 0xd9]);
    out
}

#[test]
fn lists_the_heic_photos_and_shows_the_ones_the_host_decodes() {
    let deck = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../probes/out/heic-probe.pptx"),
    )
    .unwrap();
    let mut host = Host::new();
    let opened = host
        .open(
            "heic-probe.pptx",
            Box::new(BytesReader::new(deck.clone())),
            true,
        )
        .unwrap();
    // Grid, turned grid, single, mirrored single, cropped odd photo: each a range of the input (a stored zip entry),
    // each with a plan.
    let sizes: Vec<(u32, u32)> = opened
        .media
        .iter()
        .map(|need| need.plan.as_ref().unwrap().size())
        .collect();
    assert_eq!(
        sizes,
        [(1100, 600), (600, 1100), (64, 48), (64, 48), (40, 30)]
    );
    assert_eq!(host.media_needs().unwrap(), opened.media);
    for need in &opened.media {
        let MediaBytes::Range { offset, len } = need.bytes else {
            panic!("a stored entry is a range");
        };
        let id = need.sha256.to_hex();
        assert_eq!(
            host.need_bytes(&id).unwrap(),
            deck[offset as usize..(offset + len) as usize]
        );
    }
    let [grid, turned, ..] = opened.media.as_slice() else {
        panic!("five needs");
    };
    let file = jpeg(1100, 600);
    host.add_media(&grid.sha256.to_hex(), file.clone()).unwrap();
    // The turned grid outputs 600 × 1100: its coded size is refused.
    let refused = host
        .add_media(&turned.sha256.to_hex(), jpeg(1100, 600))
        .unwrap_err();
    assert_eq!(refused.kind, ErrorKind::Request);
    assert!(
        refused.message.contains("not the 600 × 1100"),
        "{}",
        refused.message
    );
    assert_eq!(
        host.add_media("nonsense", file.clone()).unwrap_err().kind,
        ErrorKind::Request
    );

    host.finish().unwrap();
    let printed = host.print().unwrap();
    let media: Vec<(&str, &str, MediaBytes)> = printed
        .media
        .iter()
        .map(|media| (media.path.as_str(), media.mime.as_str(), media.bytes))
        .collect();
    let path = format!("media/{}.jpg", grid.sha256.to_hex());
    assert_eq!(media, [(path.as_str(), "image/jpeg", MediaBytes::Written)]);
    assert_eq!(host.media_bytes(&path).unwrap(), file);
    let codes: Vec<&str> = printed
        .diagnostics
        .iter()
        .map(|d| d.code.code())
        .filter(|code| code.starts_with("MID56"))
        .collect();
    // In output order: by code within the slide.
    assert_eq!(
        codes,
        ["MID5601", "MID5601", "MID5601", "MID5601", "MID5608"]
    );

    let mut zip = Vec::new();
    host.bundle(&BundleOptions::default(), Swaps::default(), &mut |chunk| {
        zip.extend_from_slice(chunk);
        Ok(())
    })
    .unwrap();
    let archive = slidock_io::zip::ZipArchive::open(
        BytesReader::new(zip),
        slidock_io::zip::ZipLimits::default(),
    )
    .unwrap();
    let entry = archive
        .entries()
        .iter()
        .find(|entry| entry.name == path)
        .unwrap();
    assert_eq!(archive.read(entry).unwrap().as_ref(), file.as_slice());
}

/// The head of an extended WebP (`VP8X`) of `width` × `height`: what the compiler reads of one.
fn webp(width: u32, height: u32) -> Vec<u8> {
    let mut chunk = b"VP8X\x0a\x00\x00\x00\x00\x00\x00\x00".to_vec();
    chunk.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
    chunk.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&((chunk.len() + 4) as u32).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(&chunk);
    out
}

fn zip_names(zip: &[u8]) -> Vec<String> {
    slidock_io::zip::ZipArchive::open(
        BytesReader::new(zip.to_vec()),
        slidock_io::zip::ZipLimits::default(),
    )
    .unwrap()
    .entries()
    .iter()
    .map(|entry| entry.name.clone())
    .collect()
}

#[test]
fn plans_the_pictures_takes_their_re_encodes_and_bundles_them_when_asked() {
    let deck = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../probes/out/photo-probe.pptx"),
    )
    .unwrap();
    let mut host = Host::new();
    host.open("photo-probe.pptx", Box::new(BytesReader::new(deck)), true)
        .unwrap();
    assert_eq!(host.image_plan().unwrap_err().kind, ErrorKind::Request);
    host.finish().unwrap();
    host.print().unwrap();
    let plans = host.image_plan().unwrap();
    assert_eq!(plans.len(), 6);
    let photo = plans
        .iter()
        .find(|plan| (plan.width, plan.height) == (2400, 1600))
        .unwrap();
    assert_eq!((photo.target_width, photo.target_height), (1440, 960));

    let refused = host.add_image(&photo.path, webp(1440, 961)).unwrap_err();
    assert_eq!(refused.kind, ErrorKind::Request);
    assert!(
        refused.message.contains("not the planned 1440 × 960"),
        "{}",
        refused.message
    );
    assert_eq!(
        host.add_image("media/unplanned.png", webp(1, 1))
            .unwrap_err()
            .kind,
        ErrorKind::Request
    );
    host.add_image(&photo.path, webp(1440, 960)).unwrap();
    let webp_path = format!("media/{}.webp", photo.sha256.to_hex());

    let zip = |host: &mut Host, compressed: bool| {
        let mut zip = Vec::new();
        let swaps = Swaps {
            images: compressed,
            ..Swaps::default()
        };
        host.bundle(&BundleOptions::default(), swaps, &mut |chunk| {
            zip.extend_from_slice(chunk);
            Ok(())
        })
        .unwrap();
        zip
    };
    let names = zip_names(&zip(&mut host, true));
    assert!(names.contains(&webp_path), "{names:?}");
    assert!(!names.contains(&photo.path));
    // Without compressImages, and again with it: the taken file is kept between bundles.
    let names = zip_names(&zip(&mut host, false));
    assert!(names.contains(&photo.path) && !names.contains(&webp_path));
    assert!(zip_names(&zip(&mut host, true)).contains(&webp_path));
    // A new plan forgets the taken files; so does a new print.
    host.image_plan().unwrap();
    assert!(zip_names(&zip(&mut host, true)).contains(&photo.path));
    host.add_image(&photo.path, webp(1440, 960)).unwrap();
    host.finish().unwrap();
    host.print().unwrap();
    assert!(zip_names(&zip(&mut host, true)).contains(&photo.path));
    assert_eq!(
        host.add_image(&photo.path, webp(1440, 960))
            .unwrap_err()
            .kind,
        ErrorKind::Request
    );
}

#[test]
fn plans_the_videos_and_refuses_a_file_that_is_not_the_planned_one() {
    let probes = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../probes");
    let deck = std::fs::read(probes.join("out/video-probe.pptx")).unwrap();
    let mut host = Host::new();
    host.open("video-probe.pptx", Box::new(BytesReader::new(deck)), true)
        .unwrap();
    assert_eq!(host.video_plan().unwrap_err().kind, ErrorKind::Request);
    host.finish().unwrap();
    host.print().unwrap();
    let plans = host.video_plan().unwrap();
    assert_eq!(plans.len(), 8);
    let kept: Vec<String> = plans
        .iter()
        .filter_map(|plan| plan.keep.as_ref().map(ToString::to_string))
        .collect();
    assert_eq!(
        kept,
        vec!["it has an alpha channel, which H.264 cannot carry".to_owned()]
    );
    let sound = plans
        .iter()
        .find(|plan| (plan.target_width, plan.target_height) == (960, 540))
        .unwrap();
    // The original is no file of the planned size.
    let original = std::fs::read(probes.join("videos/media/sound.mp4")).unwrap();
    let refused = host
        .add_video(&sound.path, Box::new(BytesReader::new(original)))
        .unwrap_err();
    assert_eq!(refused.kind, ErrorKind::Request);
    assert!(
        refused.message.contains("not the planned 960 × 540"),
        "{}",
        refused.message
    );
    assert_eq!(
        host.add_video(
            "media/unplanned.mp4",
            Box::new(BytesReader::new(Vec::new()))
        )
        .unwrap_err()
        .kind,
        ErrorKind::Request
    );
    let alpha = plans.iter().find(|plan| plan.keep.is_some()).unwrap();
    let refused = host
        .add_video(&alpha.path, Box::new(BytesReader::new(Vec::new())))
        .unwrap_err();
    assert!(
        refused.message.contains("stays as it is"),
        "{}",
        refused.message
    );
    // With nothing taken, a bundle with compressed videos is the plain one.
    let zip = |host: &mut Host, videos: bool| {
        let mut zip = Vec::new();
        let swaps = Swaps {
            videos,
            ..Swaps::default()
        };
        host.bundle(&BundleOptions::default(), swaps, &mut |chunk| {
            zip.extend_from_slice(chunk);
            Ok(())
        })
        .unwrap();
        zip
    };
    assert_eq!(zip(&mut host, true), zip(&mut host, false));
}
