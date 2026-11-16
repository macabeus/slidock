//! Picture compression for a download (`slidock::images`): the size each medium is drawn at (every use, crops,
//! fills, layouts, backgrounds, held scale-ups, Morph matches), the files planned for re-encoding and their target
//! sizes (EXIF turns applied; animated and vector files left out), the checks on a host's file, and the bundle that
//! writes the taken files in place of the originals, renamed everywhere the deck names them.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::collections::BTreeMap;

use serde_json::{Value, json};
use slidock::images::{
    CompressedImage, CompressedMedia, DrawnSizes, ImagePlan, ImageRefusal, check_image,
    drawn_sizes, plan_images, replacements,
};
use slidock::{
    BundleError, BundleOptions, BundleSink, CompiledDeck, Frontends, Input, Session, SessionMedia,
    SessionOptions, Sha256, StoreMedia, bundle, finish_deck, pipeline, slidock_generator,
};
use slidock_io::zip::{ZipArchive, ZipLimits};
use slidock_io::{BytesReader, RgbaImage, encode_png};
use support::*;

const PROBE: &[u8] = include_bytes!("../../../probes/out/photo-probe.pptx");

#[derive(Default)]
struct Collect(Vec<u8>);

impl BundleSink for Collect {
    fn write(&mut self, _: &str, bytes: &[u8]) -> Result<(), BundleError> {
        self.0.extend_from_slice(bytes);
        Ok(())
    }
}

fn entries(zip: &[u8]) -> Vec<(String, Vec<u8>)> {
    let archive = ZipArchive::open(BytesReader::new(zip.to_vec()), ZipLimits::default()).unwrap();
    archive
        .entries()
        .iter()
        .map(|entry| {
            (
                entry.name.clone(),
                archive.read(entry).unwrap().into_owned(),
            )
        })
        .collect()
}

/// An input compiled with native motion on (as the web app compiles), finished and printed.
struct Printed {
    registry: slidock::FrontendRegistry,
    pipeline: slidock::Pipeline,
    reader: BytesReader<Vec<u8>>,
    name: &'static str,
}

impl Printed {
    fn new(name: &'static str, bytes: Vec<u8>, frontends: Frontends) -> Printed {
        Printed {
            registry: frontends.registry().unwrap(),
            pipeline: pipeline().unwrap(),
            reader: BytesReader::new(bytes),
            name,
        }
    }

    fn session(&self) -> Session<'_> {
        let options = SessionOptions {
            native_motion: true,
            ..SessionOptions::new(&self.registry, &self.pipeline)
        };
        Session::open(&Input::new(self.name, &self.reader), options).unwrap()
    }
}

/// The probe's media by what they are (`probes/photos/make-media.sh`).
fn probe_sha(name: &str) -> Sha256 {
    let bytes: &[u8] = match name {
        "photo" => include_bytes!("../../../probes/photos/media/photo.jpg"),
        "crop" => include_bytes!("../../../probes/photos/media/crop.png"),
        "logo" => include_bytes!("../../../probes/photos/media/logo.png"),
        "turned" => include_bytes!("../../../probes/photos/media/turned.jpg"),
        "fill" => include_bytes!("../../../probes/photos/media/fill.png"),
        "bg" => include_bytes!("../../../probes/photos/media/bg.jpg"),
        _ => panic!("{name}"),
    };
    Sha256::of(bytes)
}

/// A plan as the probe test reads it: type, size as shown, orientation, target size.
type Planned<'a> = (&'a str, (u32, u32), u16, (u32, u32));

#[test]
fn plans_the_probe_s_still_rasters_at_twice_their_largest_box_on_a_full_hd_screen() {
    let probe = Printed::new("photo-probe.pptx", PROBE.to_vec(), Frontends::default());
    let mut session = probe.session();
    let finished = session.finish().unwrap();
    let drawn = drawn_sizes(finished.presentation.as_ref().unwrap());
    // 960 × 540 pt is 1280 × 720 px; a full-HD screen shows it 1.5 times, so 3 pixels per CSS pixel.
    assert_eq!(drawn.slide, (1280.0, 720.0));
    assert_eq!(drawn.density(), 3.0);
    let printed = finish_deck(&session, &finished, slidock_generator()).unwrap();
    let store = SessionMedia(&session);
    let plans = plan_images(
        &printed.documents,
        &drawn,
        &store,
        &probe.reader,
        50_000_000,
    );
    let found: BTreeMap<Sha256, Planned<'_>> = plans
        .iter()
        .map(|plan| {
            (
                plan.sha256,
                (
                    plan.mime.as_str(),
                    (plan.width, plan.height),
                    plan.orientation,
                    (plan.target_width, plan.target_height),
                ),
            )
        })
        .collect();
    let expected: BTreeMap<Sha256, Planned<'_>> = [
        // Drawn at 240 × 160 and 360 × 240 pt: the larger use, 480 × 320 px, three times.
        ("photo", ("image/jpeg", (2400, 1600), 1, (1440, 960))),
        // A quarter cut off each side of a 100 × 75 pt frame: the whole image is drawn at 200 × 150 pt.
        ("crop", ("image/png", (1000, 750), 1, (800, 600))),
        ("logo", ("image/png", (1200, 1200), 1, (480, 480))),
        // Stored 1600 × 1200, turned a quarter by its EXIF orientation: it shows 1200 × 1600.
        ("turned", ("image/jpeg", (1200, 1600), 6, (600, 800))),
        // The ellipse's fill, at its 80 × 80 pt box.
        ("fill", ("image/png", (800, 800), 1, (320, 320))),
        // The background is smaller than the 3840 × 2160 a full slide keeps: re-encoded at its own size.
        ("bg", ("image/jpeg", (3000, 1688), 1, (3000, 1688))),
    ]
    .into_iter()
    .map(|(name, plan)| (probe_sha(name), plan))
    .collect();
    assert_eq!(found, expected);
    // The animated GIF and the SVG are left out; every planned file is the printed deck's, with its size.
    assert_eq!(printed.documents.media.len(), 8);
    for plan in &plans {
        let document = &printed.documents.media[&plan.path];
        assert_eq!(document.sha256, plan.sha256);
        let entry = printed
            .deck
            .media
            .iter()
            .find(|media| media.path == plan.path)
            .unwrap();
        assert_eq!(entry.bytes, Some(plan.bytes));
    }
}

/// A PNG of `width` × `height`, one colour.
fn png(width: u32, height: u32) -> Vec<u8> {
    encode_png(&RgbaImage {
        width,
        height,
        data: vec![200; (width * height * 4) as usize],
    })
    .unwrap()
}

fn picture_at(id: &str, at: Value, sha: &str, extra: Value) -> Value {
    let mut element = picture(id, at, sha);
    for (key, value) in extra.as_object().unwrap() {
        element[key] = value.clone();
    }
    element
}

fn emphasis(target: &str, effect: Value) -> Value {
    json!({
        "delayMs": 0, "delivery": "all", "durationMs": 500, "kind": "emphasis", "trigger": "click",
        "target": { "element": target },
        "effect": { "fallback": "none", "id": "ir:emphasis", "native": effect }
    })
}

/// Drawn sizes of an IR deck (720 × 405 pt) with a layout, two uses of one medium on crossed axes, held scale-ups and
/// a Morph between two slides.
#[test]
fn every_use_held_scale_ups_and_morph_matches_set_the_largest_box() {
    let mut p = presentation(vec![]);
    let files: Vec<Vec<u8>> = (1..=7).map(|n| png(n, 1)).collect();
    let sha: Vec<String> = files
        .iter()
        .map(|bytes| add_media(&mut p, bytes, "image/png", 64, 64))
        .collect();
    let mut first = slide_on(
        "ir-s1",
        "ir-l",
        vec![
            picture("ir-s1.grow", frame(10.0, 10.0, 90.0, 60.0), &sha[0]),
            picture("ir-s1.pulse", frame(110.0, 10.0, 60.0, 60.0), &sha[1]),
            picture_at(
                "ir-s1.small",
                frame(10.0, 100.0, 30.0, 30.0),
                &sha[2],
                json!({ "matchKey": "small" }),
            ),
            picture_at(
                "ir-s1.large",
                frame(100.0, 100.0, 300.0, 150.0),
                &sha[3],
                json!({ "matchKey": "large" }),
            ),
            picture("ir-s1.wide", frame(10.0, 300.0, 300.0, 30.0), &sha[4]),
        ],
    );
    first["timeline"] = json!([
        emphasis(
            "ir-s1.grow",
            json!({ "id": "ir:growShrink", "effect": "growShrink",
                    "params": { "scale": { "x": 1.5, "y": 1.25 }, "hold": true } })
        ),
        emphasis(
            "ir-s1.pulse",
            json!({ "id": "ir:pulse", "effect": "pulse", "params": { "scale": 1.2 } })
        ),
    ]);
    let mut second = slide(
        "ir-s2",
        vec![
            picture_at(
                "ir-s2.small",
                frame(10.0, 10.0, 240.0, 120.0),
                &sha[2],
                json!({ "matchKey": "small" }),
            ),
            picture_at(
                "ir-s2.large",
                frame(10.0, 200.0, 60.0, 30.0),
                &sha[5],
                json!({ "matchKey": "large" }),
            ),
            picture("ir-s2.tall", frame(400.0, 10.0, 30.0, 300.0), &sha[4]),
        ],
    );
    second["transition"] = json!({
        "durationMs": 1000,
        "effect": { "fallback": "dissolve", "id": "ir:morph",
                    "native": { "effect": "morph", "id": "ir:morph", "params": { "by": "object" } } }
    });
    p["slides"] = json!([first, second]);
    p["masters"] = json!({ "ir-m": master("ir-m", vec![]) });
    p["layouts"] = json!({
        "ir-l": layout("ir-l", "ir-m", vec![picture("ir-l.logo", frame(600.0, 20.0, 90.0, 45.0), &sha[6])])
    });
    let media = media_files(
        &files
            .iter()
            .map(|bytes| (bytes.as_slice(), "png"))
            .collect::<Vec<_>>(),
    );
    let deck = Printed::new(NAME, bytes(&p), Frontends::default().with_ir_media(media));
    let mut session = deck.session();
    let finished = session.finish().unwrap();
    let drawn = drawn_sizes(finished.presentation.as_ref().unwrap());
    let px = |pt: f64| pt * 4.0 / 3.0;
    let at = |n: usize| drawn.media[&Sha256::parse(&sha[n]).unwrap()];
    let close = |found: (f64, f64), expected: (f64, f64)| {
        assert!(
            (found.0 - expected.0).abs() < 1e-9 && (found.1 - expected.1).abs() < 1e-9,
            "{found:?} != {expected:?}"
        );
    };
    // 720 × 405 pt fits a full-HD screen exactly twice: four pixels per CSS pixel.
    assert_eq!(drawn.density(), 4.0);
    // Grown and held, each axis by its own factor.
    close(at(0), (px(90.0) * 1.5, px(60.0) * 1.25));
    close(at(1), (px(60.0) * 1.2, px(60.0) * 1.2));
    // Morphed from 30 × 30 into its 240 × 120 match: the larger of the two, on each axis.
    close(at(2), (px(240.0), px(120.0)));
    // Morphed from 300 × 150 into a 60 × 30 match of another medium: its own box; the match grows to it.
    close(at(3), (px(300.0), px(150.0)));
    close(at(5), (px(300.0), px(150.0)));
    // Wide on one slide, tall on the other: each axis its own largest.
    close(at(4), (px(300.0), px(300.0)));
    // The layout's picture.
    close(at(6), (px(90.0), px(45.0)));
}

/// The header of a baseline JPEG of `width` × `height`.
fn jpeg(width: u16, height: u16) -> Vec<u8> {
    let mut out = vec![0xff, 0xd8, 0xff, 0xc0, 0, 17, 8];
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1, 0xff, 0xd9]);
    out
}

/// [`jpeg`] with an `Exif` APP1 segment stating `orientation`.
fn oriented_jpeg(width: u16, height: u16, orientation: u16) -> Vec<u8> {
    let mut tiff = b"MM\x00\x2a\x00\x00\x00\x08\x00\x01".to_vec();
    tiff.extend_from_slice(&[0x01, 0x12, 0x00, 0x03, 0, 0, 0, 1]);
    tiff.extend_from_slice(&orientation.to_be_bytes());
    tiff.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    let mut segment = vec![0xff, 0xe1];
    segment.extend_from_slice(&((8 + tiff.len()) as u16).to_be_bytes());
    segment.extend_from_slice(b"Exif\0\0");
    segment.extend_from_slice(&tiff);
    let plain = jpeg(width, height);
    [&plain[..2], &segment, &plain[2..]].concat()
}

/// The head of an extended WebP (`VP8X`) of `width` × `height`, animated or not.
fn webp(width: u32, height: u32, animated: bool) -> Vec<u8> {
    let mut vp8x = vec![if animated { 0x02 } else { 0 }, 0, 0, 0];
    vp8x.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
    vp8x.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
    let mut chunks = b"VP8X".to_vec();
    chunks.extend_from_slice(&(vp8x.len() as u32).to_le_bytes());
    chunks.extend_from_slice(&vp8x);
    if animated {
        for _ in 0..2 {
            chunks.extend_from_slice(b"ANMF");
            chunks.extend_from_slice(&16u32.to_le_bytes());
            chunks.extend_from_slice(&[0; 16]);
        }
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&((chunks.len() + 4) as u32).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(&chunks);
    out
}

fn plan(path: &str, bytes: u64, target: (u32, u32)) -> ImagePlan {
    ImagePlan {
        path: path.to_owned(),
        sha256: Sha256::of(path.as_bytes()),
        mime: "image/png".to_owned(),
        bytes,
        width: target.0 * 2,
        height: target.1 * 2,
        orientation: 1,
        target_width: target.0,
        target_height: target.1,
    }
}

#[test]
fn takes_a_smaller_still_webp_jpeg_or_png_of_the_planned_size_named_by_its_type() {
    let planned = plan("media/x.png", 10_000, (40, 30));
    let sha = planned.sha256.to_hex();
    let taken = check_image(&planned, webp(40, 30, false)).unwrap();
    assert_eq!(
        (taken.original.as_str(), taken.path.as_str(), taken.mime),
        (
            "media/x.png",
            format!("media/{sha}.webp").as_str(),
            "image/webp"
        )
    );
    assert_eq!(
        check_image(&planned, jpeg(40, 30)).unwrap().path,
        format!("media/{sha}.jpg")
    );
    assert_eq!(
        check_image(&planned, png(40, 30)).unwrap().path,
        format!("media/{sha}.png")
    );
    // An EXIF orientation of 1 is as stored.
    assert!(check_image(&planned, oriented_jpeg(40, 30, 1)).is_ok());

    let refusal = |bytes: Vec<u8>| check_image(&planned, bytes).unwrap_err();
    assert_eq!(
        refusal(webp(41, 30, false)),
        ImageRefusal::Size {
            width: 41,
            height: 30,
            expected_width: 40,
            expected_height: 30
        }
    );
    // Turned by its EXIF orientation, a browser would show it 30 × 40.
    assert_eq!(refusal(oriented_jpeg(40, 30, 6)), ImageRefusal::Oriented(6));
    assert_eq!(refusal(webp(40, 30, true)), ImageRefusal::Animated);
    let gif = b"GIF89a\x28\x00\x1e\x00\x00\x00\x00\x3b".to_vec();
    assert_eq!(refusal(gif), ImageRefusal::Type("image/gif".to_owned()));
    assert_eq!(
        refusal(b"not an image".to_vec()),
        ImageRefusal::Type("of no known type".to_owned())
    );
    let tight = plan("media/x.png", webp(40, 30, false).len() as u64, (40, 30));
    assert!(matches!(
        check_image(&tight, webp(40, 30, false)),
        Err(ImageRefusal::NotSmaller { .. })
    ));
}

/// The probe printed, with a host's file of each planned picture's target size taken, and its zip with and without
/// them.
struct Bundled {
    plain: Vec<u8>,
    compressed: Vec<u8>,
    single_file: Vec<u8>,
    images: BTreeMap<String, CompressedImage>,
}

fn bundled_probe() -> Bundled {
    let probe = Printed::new("photo-probe.pptx", PROBE.to_vec(), Frontends::default());
    let mut session = probe.session();
    let finished = session.finish().unwrap();
    let drawn: DrawnSizes = drawn_sizes(finished.presentation.as_ref().unwrap());
    let printed = finish_deck(&session, &finished, slidock_generator()).unwrap();
    let store = SessionMedia(&session);
    let plans = plan_images(
        &printed.documents,
        &drawn,
        &store,
        &probe.reader,
        50_000_000,
    );
    let images: BTreeMap<String, CompressedImage> = plans
        .iter()
        .map(|plan| {
            let file = webp(plan.target_width, plan.target_height, false);
            (plan.path.clone(), check_image(plan, file).unwrap())
        })
        .collect();
    let media = StoreMedia::new(&store, &probe.reader, &printed.documents, 50_000_000);
    let write = |media: &dyn slidock::DeckMedia, options: &BundleOptions| {
        let compiled = CompiledDeck {
            deck: printed.deck.clone(),
            documents: printed.documents.clone(),
            media,
            fonts: Vec::new(),
        };
        let mut sink = Collect::default();
        bundle(&compiled, options, &mut sink).unwrap();
        sink.0
    };
    let plain = write(&media, &BundleOptions::default());
    let swapped = CompressedMedia {
        media: &media,
        images: &images,
        videos: &BTreeMap::new(),
    };
    let options = BundleOptions {
        replaced: replacements(&images),
        ..BundleOptions::default()
    };
    let compressed = write(&swapped, &options);
    let single_file = write(
        &swapped,
        &BundleOptions {
            single_file: true,
            ..options
        },
    );
    Bundled {
        plain,
        compressed,
        single_file,
        images,
    }
}

/// The deck JSON in an `index.html`.
fn deck_json(index: &str) -> Value {
    let start = index
        .find("<script type=\"application/json\" id=\"sd-deck\">")
        .unwrap();
    let rest = &index[start..];
    let open = rest.find('>').unwrap() + 1;
    let close = rest.find("</script>").unwrap();
    serde_json::from_str(&rest[open..close]).unwrap()
}

#[test]
fn a_bundle_writes_the_taken_files_in_place_of_the_originals_and_names_them_everywhere() {
    let Bundled {
        plain,
        compressed,
        single_file,
        images,
    } = bundled_probe();
    assert_eq!(images.len(), 6);
    let plain_files = entries(&plain);
    let files = entries(&compressed);
    // The same files in the same order, each taken one under its new name with its bytes.
    assert_eq!(files.len(), plain_files.len());
    for ((name, bytes), (plain_name, plain_bytes)) in files.iter().zip(&plain_files) {
        match images.get(plain_name) {
            Some(image) => {
                assert_eq!(name, &image.path);
                assert_eq!(bytes.as_slice(), &image.bytes[..]);
                assert!(name.ends_with(".webp"), "{name}");
            }
            None if name.starts_with("media/") => {
                assert_eq!((name, bytes), (plain_name, plain_bytes));
            }
            None => {}
        }
    }
    let kept: Vec<&str> = files
        .iter()
        .map(|(name, _)| name.as_str())
        .filter(|name| name.starts_with("media/") && !name.ends_with(".webp"))
        .collect();
    assert_eq!(kept.len(), 2);
    assert!(kept.iter().any(|name| name.ends_with(".gif")));
    assert!(kept.iter().any(|name| name.ends_with(".svg")));

    let index = String::from_utf8(files[0].1.clone()).unwrap();
    let deck = deck_json(&index);
    for (original, image) in &images {
        assert!(!index.contains(original.as_str()), "{original}");
        assert!(index.contains(image.path.as_str()), "{}", image.path);
        let entry = deck["media"]
            .as_array()
            .unwrap()
            .iter()
            .find(|media| media["path"] == image.path.as_str())
            .unwrap();
        assert_eq!(entry["mime"], "image/webp");
        assert_eq!(entry["bytes"], image.bytes.len());
    }

    // Inlined, each taken file is a WebP `data:` URI and no media file is written.
    let files = entries(&single_file);
    let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["index.html", "LICENSES.txt"]);
    let index = String::from_utf8(files[0].1.clone()).unwrap();
    assert_eq!(
        index.matches("data:image/webp;base64,").count(),
        // The photo is drawn twice.
        images.len() + 1
    );
    assert!(!index.contains("src=\"media/"));
}

#[test]
fn without_replacements_the_bundle_is_the_one_it_always_was() {
    let Bundled { plain, .. } = bundled_probe();
    let reader = BytesReader::new(PROBE.to_vec());
    let mut sink = Collect::default();
    slidock::compile(
        &Input::new("photo-probe.pptx", &reader),
        Frontends::default(),
        &slidock::Options {
            native_motion: true,
            generator: slidock_generator(),
            ..slidock::Options::default()
        },
        &BundleOptions::default(),
        &mut sink,
    )
    .unwrap();
    assert_eq!(sink.0, plain);
}
