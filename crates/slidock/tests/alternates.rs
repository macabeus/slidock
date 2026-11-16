//! The WebM copy of a video with alpha (`slidock::alternates`): the probe's HEVC-with-alpha movie gets one and no
//! other movie does, the checks on a host's copy (each rule with a file that breaks it), and the bundle that writes a
//! taken copy beside the original, offers both from every `<video>` that plays it (the original first) and lists the
//! copy in the deck JSON, and leaves the deck byte for byte as it was without one.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};
use slidock::alternates::{
    AlternateMedia, AlternateRefusal, AlternateVideo, MAX_GROWTH, alternate_sources,
    check_alternate, needs_alternate, plan_alternates,
};
use slidock::images::drawn_sizes;
use slidock::videos::{PlannedAudio, Span, VideoKeep, VideoPlan, plan_videos};
use slidock::{
    BundleError, BundleOptions, BundleSink, CompiledDeck, DeckMedia, Frontends, Input,
    ReplacedMedia, Session, SessionMedia, SessionOptions, Sha256, StoreMedia, bundle, finish_deck,
    pipeline, slidock_generator,
};
use slidock_io::BytesReader;
use slidock_io::zip::{ZipArchive, ZipLimits};

const PROBE: &[u8] = include_bytes!("../../../probes/out/video-probe.pptx");
const ALPHA: &[u8] = include_bytes!("../../../probes/videos/media/alpha.mov");

#[derive(Default)]
struct Collect(Vec<u8>);

impl BundleSink for Collect {
    fn write(&mut self, _: &str, bytes: &[u8]) -> Result<(), BundleError> {
        self.0.extend_from_slice(bytes);
        Ok(())
    }
}

fn entries(zip: &[u8]) -> BTreeMap<String, Vec<u8>> {
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

/// The text view in an `index.html`.
fn text_view(index: &str) -> &str {
    let start = index.find("<article id=\"sd-text\"").unwrap();
    let end = start + index[start..].find("</article>").unwrap();
    &index[start..end]
}

fn session<'a>(
    registry: &'a slidock::FrontendRegistry,
    pipeline: &'a slidock::Pipeline,
    reader: &'a BytesReader<Vec<u8>>,
) -> Session<'a> {
    let options = SessionOptions {
        native_motion: true,
        ..SessionOptions::new(registry, pipeline)
    };
    Session::open(&Input::new("video-probe.pptx", reader), options).unwrap()
}

#[test]
fn only_the_movie_kept_for_its_alpha_gets_a_copy_at_its_planned_size() {
    let registry = Frontends::default().registry().unwrap();
    let pipeline = pipeline().unwrap();
    let reader = BytesReader::new(PROBE.to_vec());
    let mut session = session(&registry, &pipeline, &reader);
    let finished = session.finish().unwrap();
    let drawn = drawn_sizes(finished.presentation.as_ref().unwrap());
    let printed = finish_deck(&session, &finished, slidock_generator()).unwrap();
    let plans = plan_videos(
        &printed.documents,
        &drawn,
        &SessionMedia(&session),
        &reader,
        50_000_000,
    );
    assert_eq!(plans.len(), 8);
    let copies = plan_alternates(&plans);
    let [copy] = copies.as_slice() else {
        panic!("one copy: {copies:?}");
    };
    assert_eq!(copy.sha256, Sha256::of(ALPHA));
    assert_eq!(copy.keep, Some(VideoKeep::Alpha));
    assert_eq!(copy.codec, "hvc1");
    // 320 × 240, drawn larger than it is: its own size.
    assert_eq!((copy.target_width, copy.target_height), (320, 240));
    // Negative control: a video kept for another reason, or none, gets no copy.
    for keep in [None, Some(VideoKeep::Hdr(16)), Some(VideoKeep::Fragmented)] {
        assert!(!needs_alternate(&VideoPlan {
            keep,
            ..copy.clone()
        }));
    }
}

// A WebM built element by element, with every size in eight bytes.

fn element(id: &[u8], body: &[u8]) -> Vec<u8> {
    let mut out = id.to_vec();
    out.push(0x01);
    out.extend_from_slice(&(body.len() as u64).to_be_bytes()[1..]);
    out.extend_from_slice(body);
    out
}

fn uint(id: &[u8], value: u64) -> Vec<u8> {
    element(id, &value.to_be_bytes())
}

/// What a test copy holds.
#[derive(Clone)]
struct Copy {
    doc_type: &'static str,
    codec: &'static str,
    size: (u64, u64),
    alpha_mode: u64,
    /// Frames, one every 40 ms, a key frame every 25.
    frames: usize,
    /// The frames whose alpha addition is left out.
    no_alpha: Vec<usize>,
    first_key: bool,
    cues_first: bool,
    /// Sound tracks of this codec.
    sounds: Vec<&'static str>,
    other_track: bool,
    duration: f64,
    /// Bytes of each frame's payload.
    payload: usize,
    /// The file is cut this many bytes short.
    cut: usize,
}

impl Default for Copy {
    fn default() -> Self {
        Copy {
            doc_type: "webm",
            codec: "V_VP9",
            size: (320, 240),
            alpha_mode: 1,
            frames: 25,
            no_alpha: Vec::new(),
            first_key: true,
            cues_first: true,
            sounds: Vec::new(),
            other_track: false,
            duration: 1000.0,
            payload: 64,
            cut: 0,
        }
    }
}

impl Copy {
    fn bytes(&self) -> Vec<u8> {
        let header = element(
            &[0x1a, 0x45, 0xdf, 0xa3],
            &element(&[0x42, 0x82], self.doc_type.as_bytes()),
        );
        let info = element(
            &[0x15, 0x49, 0xa9, 0x66],
            &[
                uint(&[0x2a, 0xd7, 0xb1], 1_000_000),
                element(&[0x44, 0x89], &self.duration.to_be_bytes()),
            ]
            .concat(),
        );
        let video = element(
            &[0xe0],
            &[
                uint(&[0xb0], self.size.0),
                uint(&[0xba], self.size.1),
                uint(&[0x53, 0xc0], self.alpha_mode),
            ]
            .concat(),
        );
        let mut entries = element(
            &[0xae],
            &[
                uint(&[0xd7], 1),
                uint(&[0x83], 1),
                element(&[0x86], self.codec.as_bytes()),
                video,
            ]
            .concat(),
        );
        for (index, codec) in self.sounds.iter().enumerate() {
            entries.extend(element(
                &[0xae],
                &[
                    uint(&[0xd7], 2 + index as u64),
                    uint(&[0x83], 2),
                    element(&[0x86], codec.as_bytes()),
                ]
                .concat(),
            ));
        }
        if self.other_track {
            entries.extend(element(
                &[0xae],
                &[
                    uint(&[0xd7], 9),
                    uint(&[0x83], 17),
                    element(&[0x86], b"S_TEXT/WEBVTT"),
                ]
                .concat(),
            ));
        }
        let tracks = element(&[0x16, 0x54, 0xae, 0x6b], &entries);
        let mut clusters = Vec::new();
        let mut cluster = Vec::new();
        let mut cluster_start = 0u64;
        for index in 0..self.frames {
            let time = index as u64 * 40;
            let key = index % 25 == 0 && (index > 0 || self.first_key);
            if index % 25 == 0 {
                if !cluster.is_empty() {
                    clusters.push(element(&[0x1f, 0x43, 0xb6, 0x75], &cluster));
                }
                cluster = uint(&[0xe7], time);
                cluster_start = time;
            }
            let relative = (time - cluster_start) as i16;
            let mut block = vec![0x81];
            block.extend_from_slice(&relative.to_be_bytes());
            block.push(0);
            block.extend(vec![7u8; self.payload]);
            let mut group = element(&[0xa1], &block);
            if !self.no_alpha.contains(&index) {
                group.extend(element(
                    &[0x75, 0xa1],
                    &element(
                        &[0xa6],
                        &[uint(&[0xee], 1), element(&[0xa5], &[9u8; 16])].concat(),
                    ),
                ));
            }
            if index + 1 == self.frames {
                group.extend(uint(&[0x9b], 40));
            }
            if !key {
                group.extend(element(&[0xfb], &[0xd8]));
            }
            cluster.extend(element(&[0xa0], &group));
        }
        clusters.push(element(&[0x1f, 0x43, 0xb6, 0x75], &cluster));
        let cues = element(
            &[0x1c, 0x53, 0xbb, 0x6b],
            &(0..clusters.len())
                .flat_map(|index| element(&[0xbb], &uint(&[0xb3], index as u64 * 1000)))
                .collect::<Vec<u8>>(),
        );
        let body = if self.cues_first {
            [info, tracks, cues, clusters.concat()].concat()
        } else {
            [info, tracks, clusters.concat(), cues].concat()
        };
        let mut file = [header, element(&[0x18, 0x53, 0x80, 0x67], &body)].concat();
        file.truncate(file.len() - self.cut);
        file
    }
}

/// A plan of a 320 × 240 HEVC movie with alpha, 25 frames over 1 s, no sound.
fn plan() -> VideoPlan {
    VideoPlan {
        path: "media/a.mov".to_owned(),
        sha256: Sha256::of(ALPHA),
        mime: "video/quicktime".to_owned(),
        bytes: 100_000,
        codec: "hvc1".to_owned(),
        width: 640,
        height: 480,
        rotation: 0,
        duration_ms: 1000.0,
        frames: 25,
        video_bytes: 95_000,
        audio: None,
        colour: Default::default(),
        target_width: 320,
        target_height: 240,
        keep: Some(VideoKeep::Alpha),
        silent: false,
        uncopyable: None,
        matrix: [0x1_0000, 0, 0, 0, 0x1_0000, 0, 0, 0, 0x4000_0000],
        video_span: Span {
            start_ms: 0.0,
            length_ms: 1000.0,
        },
        audio_span: None,
    }
}

fn check(plan: &VideoPlan, copy: &Copy) -> Result<AlternateVideo, AlternateRefusal> {
    check_alternate(plan, Arc::new(BytesReader::new(copy.bytes())))
}

#[test]
fn takes_a_webm_of_one_vp9_or_vp8_track_with_alpha_at_the_planned_size() {
    let taken = check(&plan(), &Copy::default()).unwrap();
    assert_eq!(taken.original, "media/a.mov");
    assert_eq!(taken.path, format!("media/{}.webm", Sha256::of(ALPHA)));
    assert_eq!(taken.codec, "vp9");
    assert_eq!(taken.bytes, Copy::default().bytes().len() as u64);
    let vp8 = check(
        &plan(),
        &Copy {
            codec: "V_VP8",
            ..Copy::default()
        },
    )
    .unwrap();
    assert_eq!(vp8.codec, "vp8");
    assert_eq!(
        vp8.source().source_type(),
        "video/webm; codecs=\"vp8\"",
        "the type its <source> states"
    );
    // Two frames short of the original's is taken (an encoder's rounding); three are not (below).
    assert!(
        check(
            &plan(),
            &Copy {
                frames: 23,
                duration: 920.0,
                ..Copy::default()
            }
        )
        .is_ok()
    );
    // With sound, an Opus track is taken.
    let with_sound = VideoPlan {
        audio: Some(PlannedAudio {
            codec: "mp4a".to_owned(),
            object_type: Some(0x40),
            aac: true,
            channels: 2,
            sample_rate: 48_000,
            sample_size: 16,
            bytes: 32_000,
        }),
        ..plan()
    };
    assert!(
        check(
            &with_sound,
            &Copy {
                sounds: vec!["A_OPUS"],
                ..Copy::default()
            }
        )
        .is_ok()
    );
}

#[test]
fn a_copy_of_a_video_muted_in_every_use_carries_no_sound() {
    let sound = PlannedAudio {
        codec: "mp4a".to_owned(),
        object_type: Some(0x40),
        aac: true,
        channels: 2,
        sample_rate: 48_000,
        sample_size: 16,
        bytes: 32_000,
    };
    let silent = VideoPlan {
        audio: Some(sound.clone()),
        silent: true,
        ..plan()
    };
    // Its sound is never heard: the copy leaves it out, and one that carries it is refused.
    assert!(check(&silent, &Copy::default()).is_ok());
    assert_eq!(
        check(
            &silent,
            &Copy {
                sounds: vec!["A_OPUS"],
                ..Copy::default()
            }
        )
        .unwrap_err(),
        AlternateRefusal::AudioTracks {
            found: 1,
            expected: 0
        }
    );
    // Negative control: a sound some use may play is required.
    let heard = VideoPlan {
        silent: false,
        ..silent
    };
    assert_eq!(
        check(&heard, &Copy::default()).unwrap_err(),
        AlternateRefusal::AudioTracks {
            found: 0,
            expected: 1
        }
    );
}

#[test]
fn refuses_a_copy_that_breaks_a_rule() {
    let refused = |plan: &VideoPlan, copy: Copy| check(plan, &copy).unwrap_err();
    // The original stays without a copy unless it is kept for its alpha.
    assert_eq!(
        refused(
            &VideoPlan {
                keep: None,
                ..plan()
            },
            Copy::default()
        ),
        AlternateRefusal::NotPlanned("media/a.mov".to_owned())
    );
    assert_eq!(
        refused(
            &plan(),
            Copy {
                doc_type: "matroska",
                ..Copy::default()
            }
        ),
        AlternateRefusal::Type("video/x-matroska".to_owned())
    );
    assert_eq!(
        check_alternate(
            &plan(),
            Arc::new(BytesReader::new(
                b"\0\0\0\x18ftypisom\0\0\0\0isomavc1".to_vec()
            ))
        )
        .unwrap_err(),
        AlternateRefusal::Type("video/mp4".to_owned())
    );
    assert_eq!(
        refused(
            &plan(),
            Copy {
                cut: 10,
                ..Copy::default()
            }
        ),
        AlternateRefusal::Incomplete
    );
    assert_eq!(
        refused(
            &plan(),
            Copy {
                codec: "V_AV1",
                ..Copy::default()
            }
        ),
        AlternateRefusal::Codec("V_AV1".to_owned())
    );
    assert_eq!(
        refused(
            &plan(),
            Copy {
                alpha_mode: 0,
                ..Copy::default()
            }
        ),
        AlternateRefusal::NoAlpha(0)
    );
    assert_eq!(
        refused(
            &plan(),
            Copy {
                no_alpha: vec![7],
                ..Copy::default()
            }
        ),
        AlternateRefusal::AlphaBlocks {
            with: 24,
            blocks: 25
        }
    );
    assert_eq!(
        refused(
            &plan(),
            Copy {
                size: (640, 480),
                ..Copy::default()
            }
        ),
        AlternateRefusal::Size {
            width: 640,
            height: 480,
            expected_width: 320,
            expected_height: 240
        }
    );
    assert_eq!(
        refused(
            &plan(),
            Copy {
                first_key: false,
                ..Copy::default()
            }
        ),
        AlternateRefusal::NotKeyFirst
    );
    assert_eq!(
        refused(
            &plan(),
            Copy {
                cues_first: false,
                ..Copy::default()
            }
        ),
        AlternateRefusal::Cues
    );
    assert_eq!(
        refused(
            &plan(),
            Copy {
                sounds: vec!["A_OPUS"],
                ..Copy::default()
            }
        ),
        AlternateRefusal::AudioTracks {
            found: 1,
            expected: 0
        }
    );
    let with_sound = VideoPlan {
        audio: Some(PlannedAudio {
            codec: "mp4a".to_owned(),
            object_type: Some(0x40),
            aac: true,
            channels: 2,
            sample_rate: 48_000,
            sample_size: 16,
            bytes: 32_000,
        }),
        ..plan()
    };
    assert_eq!(
        refused(
            &with_sound,
            Copy {
                sounds: vec!["A_AAC"],
                ..Copy::default()
            }
        ),
        AlternateRefusal::AudioCodec("A_AAC".to_owned())
    );
    assert_eq!(
        refused(
            &plan(),
            Copy {
                other_track: true,
                ..Copy::default()
            }
        ),
        AlternateRefusal::OtherTrack(17)
    );
    assert_eq!(
        refused(
            &plan(),
            Copy {
                frames: 22,
                ..Copy::default()
            }
        ),
        AlternateRefusal::Frames {
            found: 22,
            expected: 25
        }
    );
    // A copy that starts late.
    assert!(matches!(
        refused(
            &VideoPlan {
                video_span: Span {
                    start_ms: 500.0,
                    length_ms: 500.0
                },
                ..plan()
            },
            Copy::default()
        ),
        AlternateRefusal::Span { .. }
    ));
    assert_eq!(
        refused(
            &plan(),
            Copy {
                duration: 2000.0,
                ..Copy::default()
            }
        ),
        AlternateRefusal::Duration {
            ms: 2000.0,
            expected: 1000.0
        }
    );
    let bytes = Copy {
        payload: 5000,
        ..Copy::default()
    };
    let size = bytes.bytes().len() as u64;
    assert_eq!(
        refused(
            &VideoPlan {
                bytes: 50_000,
                ..plan()
            },
            bytes
        ),
        AlternateRefusal::TooLarge {
            bytes: size,
            limit: 50_000 * MAX_GROWTH
        }
    );
}

#[test]
fn bundles_a_taken_copy_after_the_original_and_the_same_zip_without_one() {
    let registry = Frontends::default().registry().unwrap();
    let pipeline = pipeline().unwrap();
    let reader = BytesReader::new(PROBE.to_vec());
    let mut session = session(&registry, &pipeline, &reader);
    let finished = session.finish().unwrap();
    let drawn = drawn_sizes(finished.presentation.as_ref().unwrap());
    let printed = finish_deck(&session, &finished, slidock_generator()).unwrap();
    let store = SessionMedia(&session);
    let plans = plan_videos(&printed.documents, &drawn, &store, &reader, 50_000_000);
    let alpha = plan_alternates(&plans).remove(0);
    let copy = Copy {
        frames: usize::try_from(alpha.frames).unwrap(),
        duration: alpha.duration_ms,
        ..Copy::default()
    };
    let taken = check_alternate(&alpha, Arc::new(BytesReader::new(copy.bytes()))).unwrap();
    let alternates = BTreeMap::from([(alpha.path.clone(), taken.clone())]);
    let media = StoreMedia::new(&store, &reader, &printed.documents, 50_000_000);
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
    let empty = BTreeMap::new();
    let none = AlternateMedia {
        media: &media,
        alternates: &empty,
    };
    // With nothing taken, the zip is byte for byte the plain one.
    assert_eq!(write(&none, &BundleOptions::default()), plain);
    let served = AlternateMedia {
        media: &media,
        alternates: &alternates,
    };
    let options = BundleOptions {
        alternates: alternate_sources(&alternates),
        ..BundleOptions::default()
    };
    let zip = write(&served, &options);
    let offered = entries(&zip);
    let original = entries(&plain);
    // The copy is written right after the original, which stays.
    let names: Vec<String> = ZipArchive::open(BytesReader::new(zip.clone()), ZipLimits::default())
        .unwrap()
        .entries()
        .iter()
        .map(|entry| entry.name.clone())
        .collect();
    let at = names.iter().position(|name| name == &alpha.path).unwrap();
    assert_eq!(names[at + 1], taken.path);
    assert_eq!(offered[&taken.path], copy.bytes());
    for (name, bytes) in &original {
        if name.starts_with("media/") {
            assert_eq!(&offered[name], bytes, "{name}");
        }
    }
    // Its video offers both, the original first, and no longer names a `src` of its own.
    let index = String::from_utf8(offered["index.html"].clone()).unwrap();
    let sources = format!(
        "<source src=\"{}\" type=\"video/quicktime; codecs=&quot;hvc1&quot;\"><source src=\"{}\" type=\"video/webm; codecs=&quot;vp9&quot;\">",
        alpha.path, taken.path
    );
    assert!(index.contains(&sources), "the video offers both files");
    assert!(!index.contains(&format!("src=\"{}\" poster", alpha.path)));
    let plain_index = String::from_utf8(original["index.html"].clone()).unwrap();
    assert!(plain_index.contains(&format!("<video src=\"{}\"", alpha.path)));
    // The text view holds text and alt only: the same with the copy offered, and naming neither file.
    let view = text_view(&index);
    assert_eq!(view, text_view(&plain_index));
    assert!(!view.contains("<source") && !view.contains("<video"));
    assert!(!view.contains(&alpha.path) && !view.contains(&taken.path));
    // Every other video keeps its `src`.
    assert_eq!(
        index.matches("<video src=").count(),
        plain_index.matches("<video src=").count() - 1
    );
    let deck = deck_json(&index);
    let listed: Vec<&Value> = deck["media"].as_array().unwrap().iter().collect();
    let at = listed
        .iter()
        .position(|media| media["path"] == json!(alpha.path))
        .unwrap();
    assert_eq!(listed[at + 1]["path"], json!(taken.path));
    assert_eq!(listed[at + 1]["mime"], json!("video/webm"));
    assert_eq!(listed[at + 1]["bytes"], json!(taken.bytes));
    assert_eq!(listed[at + 1]["sha256"], listed[at]["sha256"]);
    // A single file inlines both.
    let single = entries(&write(
        &served,
        &BundleOptions {
            single_file: true,
            ..options
        },
    ));
    let index = String::from_utf8(single["index.html"].clone()).unwrap();
    assert!(index.contains("<source src=\"data:video/quicktime;base64,"));
    assert!(index.contains("<source src=\"data:video/webm;base64,"));
    assert_eq!(taken.reader.len(), taken.bytes);
}

/// The deck's media with one file served from bytes: a compressed video written in its original's place.
struct WithFile<'a> {
    media: &'a dyn DeckMedia,
    path: String,
    bytes: Vec<u8>,
}

impl DeckMedia for WithFile<'_> {
    fn write_media(&self, path: &str, out: &mut dyn std::io::Write) -> Result<(), BundleError> {
        if path == self.path {
            out.write_all(&self.bytes).unwrap();
            Ok(())
        } else {
            self.media.write_media(path, out)
        }
    }
}

#[test]
fn a_copy_offers_the_file_written_in_its_original_s_place_first() {
    // "Compress videos" writes a video muted in every use as an MP4 of its video track copied as it is (HEVC with its
    // alpha layer), in the original's place: the WebM copy is still offered after it, and the MP4 is typed as the
    // original is, by its bare sample format, so WebKit alone plays it.
    let registry = Frontends::default().registry().unwrap();
    let pipeline = pipeline().unwrap();
    let reader = BytesReader::new(PROBE.to_vec());
    let mut session = session(&registry, &pipeline, &reader);
    let finished = session.finish().unwrap();
    let drawn = drawn_sizes(finished.presentation.as_ref().unwrap());
    let printed = finish_deck(&session, &finished, slidock_generator()).unwrap();
    let store = SessionMedia(&session);
    let plans = plan_videos(&printed.documents, &drawn, &store, &reader, 50_000_000);
    let alpha = plan_alternates(&plans).remove(0);
    let copy = Copy {
        frames: usize::try_from(alpha.frames).unwrap(),
        duration: alpha.duration_ms,
        ..Copy::default()
    };
    let taken = check_alternate(&alpha, Arc::new(BytesReader::new(copy.bytes()))).unwrap();
    let alternates = BTreeMap::from([(alpha.path.clone(), taken.clone())]);
    let written = format!("media/{}.mp4", alpha.sha256);
    let media = StoreMedia::new(&store, &reader, &printed.documents, 50_000_000);
    let with_file = WithFile {
        media: &media,
        path: written.clone(),
        bytes: b"mp4 bytes".to_vec(),
    };
    let served = AlternateMedia {
        media: &with_file,
        alternates: &alternates,
    };
    let options = BundleOptions {
        replaced: BTreeMap::from([(
            alpha.path.clone(),
            ReplacedMedia {
                path: written.clone(),
                mime: "video/mp4".to_owned(),
                bytes: 9,
            },
        )]),
        alternates: alternate_sources(&alternates),
        ..BundleOptions::default()
    };
    let compiled = CompiledDeck {
        deck: printed.deck.clone(),
        documents: printed.documents.clone(),
        media: &served,
        fonts: Vec::new(),
    };
    let mut sink = Collect::default();
    bundle(&compiled, &options, &mut sink).unwrap();
    let files = entries(&sink.0);
    assert!(!files.contains_key(&alpha.path));
    assert_eq!(files[&written], b"mp4 bytes");
    assert_eq!(files[&taken.path], copy.bytes());
    let index = String::from_utf8(files["index.html"].clone()).unwrap();
    assert!(
        index.contains(&format!(
            "<source src=\"{written}\" type=\"video/mp4; codecs=&quot;hvc1&quot;\"><source src=\"{}\" type=\"video/webm; codecs=&quot;vp9&quot;\">",
            taken.path
        )),
        "the written file first, typed as it is written"
    );
}
