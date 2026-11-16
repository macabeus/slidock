//! Media a host decodes: a HEIC photo is listed with its decode plan; a PNG or JPEG of the plan's output size is
//! taken, and the deck shows it (MID5608) with the host's bytes; any other file is refused; without one the photo is
//! omitted (MID5601) and the deck is the one `slidock::compile` writes.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde_json::Value;
use slidock::{
    BundleError, BundleOptions, BundleSink, CompiledDeck, Frontends, Input, MediaAction,
    MediaFiles, MediaRefusal, Options, Session, SessionError, SessionMedia, SessionOptions,
    StoreMedia, VerifyMode, bundle, compile, finish_deck, pipeline, slidock_generator,
};
use slidock_io::zip::{ZipArchive, ZipLimits};
use slidock_io::{BytesReader, RgbaImage, encode_png};
use support::*;

const GRID: &[u8] = include_bytes!("../../../probes/heif/grid.heic");
const SINGLE: &[u8] = include_bytes!("../../../probes/heif/single.heic");

#[derive(Default)]
struct Collect(Vec<u8>);

impl BundleSink for Collect {
    fn write(&mut self, _: &str, bytes: &[u8]) -> Result<(), BundleError> {
        self.0.extend_from_slice(bytes);
        Ok(())
    }
}

/// One slide showing `heic` twice (the need is listed once) and a slide of text.
fn deck(heic: &[u8]) -> (Value, MediaFiles, String) {
    let mut p = presentation(vec![]);
    let sha = add_media(&mut p, heic, "image/heic", 1100, 600);
    p["slides"] = serde_json::json!([
        slide(
            "ir-s1",
            vec![
                picture("ir-s1.a", frame(10.0, 10.0, 330.0, 180.0), &sha),
                picture("ir-s1.b", frame(360.0, 10.0, 330.0, 180.0), &sha),
            ]
        ),
        slide(
            "ir-s2",
            vec![text_shape(
                "ir-s2.t",
                frame(10.0, 10.0, 300.0, 60.0),
                "Text"
            )]
        ),
    ]);
    (p, media_files(&[(heic, "heic")]), sha)
}

/// The header of a baseline JPEG of `width` × `height` (SOI, SOF0, EOI): what the compiler reads of one.
fn jpeg(width: u16, height: u16) -> Vec<u8> {
    let mut out = vec![0xff, 0xd8, 0xff, 0xc0, 0, 17, 8];
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1, 0xff, 0xd9]);
    out
}

/// [`jpeg`] with an `Exif` APP1 segment stating `orientation` (tag 274) in a TIFF of either byte order.
fn oriented_jpeg(width: u16, height: u16, orientation: u16, little: bool) -> Vec<u8> {
    let u16_bytes = |value: u16| {
        if little {
            value.to_le_bytes()
        } else {
            value.to_be_bytes()
        }
    };
    let u32_bytes = |value: u32| {
        if little {
            value.to_le_bytes()
        } else {
            value.to_be_bytes()
        }
    };
    let mut tiff = if little {
        b"II".to_vec()
    } else {
        b"MM".to_vec()
    };
    tiff.extend_from_slice(&u16_bytes(42));
    tiff.extend_from_slice(&u32_bytes(8));
    // IFD0: an XResolution-like entry first, then the orientation (SHORT, one value).
    tiff.extend_from_slice(&u16_bytes(2));
    for (tag, kind, value) in [(0x011a, 5, 0), (0x0112, 3, u32::from(orientation))] {
        tiff.extend_from_slice(&u16_bytes(tag));
        tiff.extend_from_slice(&u16_bytes(kind));
        tiff.extend_from_slice(&u32_bytes(1));
        let mut field = [0; 4];
        if kind == 3 {
            field[..2].copy_from_slice(&u16_bytes(value as u16));
        }
        tiff.extend_from_slice(&field);
    }
    tiff.extend_from_slice(&u32_bytes(0));
    let mut segment = vec![0xff, 0xe1];
    segment.extend_from_slice(&((8 + tiff.len()) as u16).to_be_bytes());
    segment.extend_from_slice(b"Exif\0\0");
    segment.extend_from_slice(&tiff);
    let plain = jpeg(width, height);
    [&plain[..2], &segment, &plain[2..]].concat()
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let image = RgbaImage {
        width,
        height,
        data: vec![255; (width * height * 4) as usize],
    };
    encode_png(&image).unwrap()
}

struct Opened {
    registry: slidock::FrontendRegistry,
    pipeline: slidock::Pipeline,
    reader: BytesReader<Vec<u8>>,
}

impl Opened {
    fn new(p: &Value, media: MediaFiles) -> Opened {
        Opened {
            registry: Frontends::default()
                .with_ir_media(media)
                .registry()
                .unwrap(),
            pipeline: pipeline().unwrap(),
            reader: BytesReader::new(bytes(p)),
        }
    }

    fn session(&self) -> Session<'_> {
        let options = SessionOptions {
            verify: VerifyMode::Each,
            ..SessionOptions::new(&self.registry, &self.pipeline)
        };
        Session::open(&Input::new(NAME, &self.reader), options).unwrap()
    }

    /// The zip of what `session` finishes, and its media diagnostics' codes.
    fn zip(&self, session: &mut Session<'_>) -> (Vec<u8>, Vec<String>) {
        let finished = session.finish().unwrap();
        let finished = finish_deck(session, &finished, slidock_generator()).unwrap();
        // The media diagnostics.
        let codes = finished
            .diagnostics
            .iter()
            .filter(|d| d.code.code().starts_with("MID56"))
            .map(|d| {
                format!(
                    "{} {}",
                    d.code.code(),
                    d.container.as_ref().unwrap().as_str()
                )
            })
            .collect();
        let store = SessionMedia(session);
        let media = StoreMedia::new(&store, &self.reader, &finished.documents, 50_000_000);
        let compiled = CompiledDeck {
            deck: finished.deck,
            documents: finished.documents,
            media: &media,
            fonts: Vec::new(),
        };
        let mut sink = Collect::default();
        bundle(&compiled, &BundleOptions::default(), &mut sink).unwrap();
        (sink.0, codes)
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

#[test]
fn lists_each_shown_heic_once_with_its_plan() {
    let (p, media, sha) = deck(GRID);
    let opened = Opened::new(&p, media);
    let session = opened.session();
    let needs = session.media_needs(&opened.reader);
    assert_eq!(needs.len(), 1);
    let need = &needs[0];
    assert_eq!(need.sha256.to_hex(), sha);
    assert_eq!(need.mime, "image/heic");
    assert_eq!(need.size(), Some((1100, 600)));
    assert_eq!(need.plan.as_ref().unwrap().tiles.len(), 6);
}

#[test]
fn without_a_decoded_file_the_photo_is_omitted_as_the_cli_omits_it() {
    let (p, media, _) = deck(GRID);
    let opened = Opened::new(&p, media.clone());
    let mut session = opened.session();
    let (zip, codes) = opened.zip(&mut session);
    assert_eq!(codes, ["MID5601 ir-s1"]);
    let names: Vec<String> = entries(&zip).into_iter().map(|(name, _)| name).collect();
    assert_eq!(names, ["index.html", "LICENSES.txt"]);

    // `slidock::compile`, the CLI's path, writes the same zip.
    let reader = BytesReader::new(bytes(&p));
    let mut sink = Collect::default();
    compile(
        &Input::new(NAME, &reader),
        Frontends::default().with_ir_media(media),
        &Options {
            verify: VerifyMode::Each,
            generator: slidock_generator(),
            ..Options::default()
        },
        &BundleOptions::default(),
        &mut sink,
    )
    .unwrap();
    assert_eq!(sink.0, zip);
}

#[test]
fn a_decoded_jpeg_of_the_plan_size_is_shown_as_the_host_wrote_it() {
    let (p, media, sha) = deck(GRID);
    let opened = Opened::new(&p, media);
    let mut session = opened.session();
    let file = jpeg(1100, 600);
    let decoded = session
        .add_media(
            &slidock::Sha256::parse(&sha).unwrap(),
            file.clone(),
            &opened.reader,
        )
        .unwrap();
    assert_eq!(
        (decoded.mime, decoded.width, decoded.height, decoded.bytes),
        ("image/jpeg", 1100, 600, file.len() as u64)
    );
    let (zip, codes) = opened.zip(&mut session);
    // Reported once per container that shows it; nothing is declined.
    assert_eq!(codes, ["MID5608 ir-s1"]);
    let files = entries(&zip);
    let path = format!("media/{sha}.jpg");
    assert_eq!(files[1].0, path);
    assert_eq!(files[1].1, file);
    let index = String::from_utf8(files[0].1.clone()).unwrap();
    assert_eq!(index.matches(&format!("src=\"{path}\"")).count(), 2);

    // The same inputs make the same deck.
    let mut again = opened.session();
    again
        .add_media(&slidock::Sha256::parse(&sha).unwrap(), file, &opened.reader)
        .unwrap();
    assert_eq!(opened.zip(&mut again).0, zip);

    // A later file replaces the first; a PNG is written as PNG.
    session
        .add_media(
            &slidock::Sha256::parse(&sha).unwrap(),
            png(1100, 600),
            &opened.reader,
        )
        .unwrap();
    assert_eq!(session.decoded_media().len(), 1);
    let finished = session.finish().unwrap();
    let printed = finish_deck(&session, &finished, slidock_generator()).unwrap();
    let plans: Vec<(&String, MediaAction, &str)> = printed
        .documents
        .media
        .iter()
        .map(|(path, plan)| (path, plan.action, plan.mime.as_str()))
        .collect();
    assert_eq!(
        plans,
        [(
            &format!("media/{sha}.png"),
            MediaAction::Transcode,
            "image/png"
        )]
    );
}

/// A deck prints with the files its own `finish` planned with: a file added after it (here a PNG in place of a JPEG)
/// takes effect at the next `finish`, never under the plans of the last one.
#[test]
fn a_deck_finished_before_a_file_is_replaced_prints_the_file_it_planned_with() {
    let (p, media, sha) = deck(GRID);
    let opened = Opened::new(&p, media);
    let mut session = opened.session();
    let sha = slidock::Sha256::parse(&sha).unwrap();
    let first = jpeg(1100, 600);
    session
        .add_media(&sha, first.clone(), &opened.reader)
        .unwrap();
    let finished = session.finish().unwrap();
    let printed = finish_deck(&session, &finished, slidock_generator()).unwrap();
    session
        .add_media(&sha, png(1100, 600), &opened.reader)
        .unwrap();
    let store = SessionMedia(&session);
    let media = StoreMedia::new(&store, &opened.reader, &printed.documents, 50_000_000);
    let compiled = CompiledDeck {
        deck: printed.deck,
        documents: printed.documents,
        media: &media,
        fonts: Vec::new(),
    };
    let mut sink = Collect::default();
    bundle(&compiled, &BundleOptions::default(), &mut sink).unwrap();
    let files = entries(&sink.0);
    assert_eq!(files[1], (format!("media/{}.jpg", sha.to_hex()), first));
    // The next finish takes the PNG.
    let (zip, _) = opened.zip(&mut session);
    assert_eq!(entries(&zip)[1].0, format!("media/{}.png", sha.to_hex()));
}

#[test]
fn plans_each_need_once_per_session() {
    let (p, media, _) = deck(GRID);
    let opened = Opened::new(&p, media);
    let session = opened.session();
    let first = session.media_needs(&opened.reader);
    assert_eq!(first.len(), 1);
    assert!(std::ptr::eq(first, session.media_needs(&opened.reader)));
}

#[test]
fn refuses_a_file_of_another_size_or_type_and_a_medium_no_need_names() {
    let (p, media, sha) = deck(GRID);
    let opened = Opened::new(&p, media);
    let mut session = opened.session();
    let sha = slidock::Sha256::parse(&sha).unwrap();
    let refused = |result: Result<_, SessionError>| match result {
        Err(SessionError::MediaRefused(refusal)) => refusal,
        other => panic!("expected a refusal, got {other:?}"),
    };
    // The coded size, not the plan's output, or another one.
    assert_eq!(
        refused(session.add_media(&sha, jpeg(600, 1100), &opened.reader)),
        MediaRefusal::Size {
            width: 600,
            height: 1100,
            expected_width: 1100,
            expected_height: 600
        }
    );
    assert!(matches!(
        refused(session.add_media(&sha, GRID.to_vec(), &opened.reader)),
        MediaRefusal::Type(mime) if mime == "image/heic"
    ));
    assert!(matches!(
        refused(session.add_media(&sha, b"GIF89a\x01\0\x01\0".to_vec(), &opened.reader)),
        MediaRefusal::Type(mime) if mime == "image/gif"
    ));
    assert!(matches!(
        refused(session.add_media(&sha, vec![0; 16], &opened.reader)),
        MediaRefusal::Type(_)
    ));
    // A JPEG a browser would turn again: its EXIF orientation must be absent or 1.
    for little in [false, true] {
        assert_eq!(
            refused(session.add_media(&sha, oriented_jpeg(1100, 600, 6, little), &opened.reader)),
            MediaRefusal::Oriented(6)
        );
    }
    assert_eq!(
        slidock_io::jpeg_orientation(&oriented_jpeg(1100, 600, 1, true)),
        Some(1)
    );
    assert_eq!(slidock_io::jpeg_orientation(&jpeg(1100, 600)), None);
    let other = slidock::Sha256::parse(&"0".repeat(64)).unwrap();
    assert!(matches!(
        refused(session.add_media(&other, jpeg(1100, 600), &opened.reader)),
        MediaRefusal::NotNeeded(_)
    ));
    // Nothing was taken: the photo stays omitted.
    assert!(session.decoded_media().is_empty());
    assert_eq!(opened.zip(&mut session).1, ["MID5601 ir-s1"]);
}

#[test]
fn a_heic_outside_the_subset_is_listed_with_why_and_takes_no_file() {
    // The single image with its `hvc1` item renamed `av01`: still HEIF, of a codec the plan does not read.
    let mut bytes = SINGLE.to_vec();
    let at = bytes
        .windows(4)
        .position(|window| window == b"hvc1")
        .unwrap();
    bytes[at..at + 4].copy_from_slice(b"av01");
    let (p, media, sha) = deck(&bytes);
    let opened = Opened::new(&p, media);
    let mut session = opened.session();
    let needs = session.media_needs(&opened.reader);
    assert_eq!(
        needs[0]
            .plan
            .as_ref()
            .err()
            .map(ToString::to_string)
            .as_deref(),
        Some("its primary image is a `av01` item")
    );
    let sha = slidock::Sha256::parse(&sha).unwrap();
    assert!(matches!(
        session.add_media(&sha, jpeg(64, 48), &opened.reader),
        Err(SessionError::MediaRefused(MediaRefusal::NoPlan(..)))
    ));
}
