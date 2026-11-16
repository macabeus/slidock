//! Video compression for a download (`slidock::videos`): the movies planned for re-encoding and their target sizes
//! (every use, the slides a clip plays across, Magic Move matches, a trimmed use, a quarter-turn matrix), the movies
//! kept as they are and why, the checks on a host's file, and the bundle that writes a taken file in place of the
//! original, renamed everywhere the deck names it, and leaves the deck byte for byte as it was without one.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};
use slidock::images::{CompressedMedia, drawn_sizes};
use slidock::videos::{
    CompressedVideo, VideoKeep, VideoPlan, VideoRefusal, check_video, plan_videos,
    video_replacements, video_target_size,
};
use slidock::{
    BundleError, BundleOptions, BundleSink, CompiledDeck, Frontends, Input, Session, SessionMedia,
    SessionOptions, Sha256, StoreMedia, bundle, finish_deck, pipeline, slidock_generator,
};
use slidock_io::zip::{ZipArchive, ZipLimits};
use slidock_io::{BytesReader, RgbaImage, encode_png};
use support::*;

const PROBE: &[u8] = include_bytes!("../../../probes/out/video-probe.pptx");

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

    /// The deck's video plans.
    fn plans(&self) -> Vec<VideoPlan> {
        let mut session = self.session();
        let finished = session.finish().unwrap();
        let drawn = drawn_sizes(finished.presentation.as_ref().unwrap());
        let printed = finish_deck(&session, &finished, slidock_generator()).unwrap();
        plan_videos(
            &printed.documents,
            &drawn,
            &SessionMedia(&session),
            &self.reader,
            50_000_000,
        )
    }
}

/// The probe's movies by name (`probes/videos/make-media.sh`).
fn probe_sha(name: &str) -> Sha256 {
    let bytes: &[u8] = match name {
        "sound" => include_bytes!("../../../probes/videos/media/sound.mp4"),
        "turned" => include_bytes!("../../../probes/videos/media/turned.mp4"),
        "tiny" => include_bytes!("../../../probes/videos/media/tiny.mp4"),
        "pcm" => include_bytes!("../../../probes/videos/media/pcm.mov"),
        "alpha" => include_bytes!("../../../probes/videos/media/alpha.mov"),
        "own" => include_bytes!("../../../probes/videos/media/own.mp4"),
        "muted" => include_bytes!("../../../probes/videos/media/muted.mp4"),
        "quiet" => include_bytes!("../../../probes/videos/media/quiet.mp4"),
        _ => panic!("{name}"),
    };
    Sha256::of(bytes)
}

/// A plan as the probe test reads it: codec, size as shown, turn, target size, sound, length, why it is kept.
type Planned = (
    String,
    (u32, u32),
    u16,
    (u32, u32),
    Option<(String, bool)>,
    u64,
    Option<VideoKeep>,
);

fn planned(plan: &VideoPlan) -> Planned {
    (
        plan.codec.clone(),
        (plan.width, plan.height),
        plan.rotation,
        (plan.target_width, plan.target_height),
        plan.audio
            .as_ref()
            .map(|audio| (audio.codec.clone(), audio.aac)),
        plan.duration_ms.round() as u64,
        plan.keep.clone(),
    )
}

#[test]
fn plans_the_probe_s_movies_at_twice_their_largest_box_on_a_full_hd_screen() {
    let probe = Printed::new("video-probe.pptx", PROBE.to_vec(), Frontends::default());
    let plans = probe.plans();
    let found: BTreeMap<Sha256, Planned> = plans
        .iter()
        .map(|plan| (plan.sha256, planned(plan)))
        .collect();
    let sound = |codec: &str, aac: bool| Some((codec.to_owned(), aac));
    // 960 × 540 pt is 1280 × 720 px; a full-HD screen shows it 1.5 times, so 3 pixels per CSS pixel.
    let expected: BTreeMap<Sha256, Planned> = [
        // 240 × 135 pt is 320 × 180 px: 960 × 540.
        (
            "sound",
            (
                "avc1".to_owned(),
                (1280, 720),
                0,
                (960, 540),
                sound("mp4a", true),
                2000,
                None,
            ),
        ),
        // Stored 960 × 540 under a quarter turn counterclockwise: it shows 540 × 960, at 90 × 160 pt.
        (
            "turned",
            (
                "avc1".to_owned(),
                (540, 960),
                270,
                (360, 640),
                None,
                2000,
                None,
            ),
        ),
        // Drawn larger than it is: its own size.
        (
            "tiny",
            (
                "avc1".to_owned(),
                (320, 180),
                0,
                (320, 180),
                None,
                2000,
                None,
            ),
        ),
        // Drawn at 192 × 108 and, trimmed, at 128 × 72 pt: the larger use; the trim keeps its whole length.
        (
            "pcm",
            (
                "avc1".to_owned(),
                (1280, 720),
                0,
                (768, 432),
                sound("sowt", false),
                1600,
                None,
            ),
        ),
        (
            "alpha",
            (
                "hvc1".to_owned(),
                (320, 240),
                0,
                (320, 240),
                None,
                1000,
                Some(VideoKeep::Alpha),
            ),
        ),
        // Drawn at 320 × 180 pt, larger than it is: its own size, its colour unstated.
        (
            "own",
            (
                "avc1".to_owned(),
                (640, 360),
                0,
                (640, 360),
                None,
                2000,
                None,
            ),
        ),
        // Drawn at 160 × 90 pt: its own size.
        (
            "muted",
            (
                "avc1".to_owned(),
                (640, 360),
                0,
                (640, 360),
                sound("mp4a", true),
                2000,
                None,
            ),
        ),
        // Drawn at 192 × 108 pt: 768 × 432.
        (
            "quiet",
            (
                "avc1".to_owned(),
                (960, 540),
                0,
                (768, 432),
                sound("mp4a", true),
                2000,
                None,
            ),
        ),
    ]
    .into_iter()
    .map(|(name, plan)| (probe_sha(name), plan))
    .collect();
    assert_eq!(found, expected);
    // Each plan names the file's bytes; the PCM movie is one file for its two uses.
    assert_eq!(plans.len(), 8);
    // Muted in its only use, and at a volume of 0: never heard. The clip with sound is muted in one use and heard in
    // the other; the others' uses are heard (no media node mutes them).
    let silent: Vec<Sha256> = plans
        .iter()
        .filter(|plan| plan.silent)
        .map(|plan| plan.sha256)
        .collect();
    let mut expected_silent = vec![probe_sha("muted"), probe_sha("quiet")];
    expected_silent.sort();
    assert_eq!(silent, expected_silent);
    let muted = plans
        .iter()
        .find(|plan| plan.sha256 == probe_sha("muted"))
        .unwrap();
    // 128 kbit/s of AAC for 2 s.
    let bytes = muted.audio.as_ref().unwrap().bytes;
    assert!((30_000..34_000).contains(&bytes), "{bytes}");
    let own = plans
        .iter()
        .find(|plan| plan.sha256 == probe_sha("own"))
        .unwrap();
    assert_eq!(own.colour, slidock::videos::VideoColour::default());
    let pcm = plans
        .iter()
        .find(|plan| plan.sha256 == probe_sha("pcm"))
        .unwrap();
    assert_eq!(pcm.bytes, 916_765);
    assert_eq!(pcm.mime, "video/quicktime");
    assert_eq!(pcm.audio.as_ref().unwrap().channels, 2);
    assert_eq!(pcm.audio.as_ref().unwrap().sample_rate, 44_100);
    assert_eq!(pcm.frames, 40);
    // 1.6 s of 16-bit stereo at 44.1 kHz: its frames times 4 bytes, whatever size stsz states for them.
    assert_eq!(pcm.audio.as_ref().unwrap().bytes, 70_560 * 4);
}

#[test]
fn even_sides_never_upscale_and_never_exceed_the_video() {
    // 1001 × 563 drawn at 400 × 225 px, twice: the tighter axis meets 450, the other comes to 801, made even.
    assert_eq!(
        video_target_size(1001, 563, (400.0, 225.0), 2.0),
        (802, 450)
    );
    // An odd side within the video rounds up; one at the video's own odd size rounds down.
    assert_eq!(
        video_target_size(1000, 563, (250.0, 140.6), 2.0),
        (500, 282)
    );
    assert_eq!(
        video_target_size(1001, 563, (2000.0, 2000.0), 2.0),
        (1000, 562)
    );
    assert_eq!(video_target_size(3, 3, (0.5, 0.5), 2.0), (2, 2));
}

// Synthetic movies: the boxes the planner and the checker read, with an `mdat` of zeros.

fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

fn full(kind: &[u8; 4], version_flags: u32, body: &[u8]) -> Vec<u8> {
    atom(kind, &[&version_flags.to_be_bytes()[..], body].concat())
}

const ONE: i32 = 0x1_0000;
const IDENTITY: [i32; 9] = [ONE, 0, 0, 0, ONE, 0, 0, 0, 0x4000_0000];

#[derive(Clone)]
struct Track {
    handler: &'static [u8; 4],
    format: &'static [u8; 4],
    size: (u16, u16),
    depth: u16,
    config: Option<(&'static [u8; 4], Vec<u8>)>,
    colr: Option<(u16, u16, u16)>,
    pasp: Option<(u32, u32)>,
    matrix: [i32; 9],
    /// (duration in the movie's timescale, media time).
    edits: Vec<(u32, i32)>,
    /// `esds`' object type of a sound.
    object_type: u8,
    samples: u32,
    entries: usize,
    /// The media's length in milliseconds, when not the movie's.
    length: Option<u32>,
}

fn avcc(profile: u8) -> Option<(&'static [u8; 4], Vec<u8>)> {
    Some((b"avcC", vec![1, profile, 0, 40, 0xff, 0xe0, 0]))
}

/// An `hvcC` with one VPS stating `layers` layers.
fn hvcc(layers: u8) -> Option<(&'static [u8; 4], Vec<u8>)> {
    let mut config = vec![1];
    config.extend_from_slice(&[0; 21]);
    let vps = [
        0x40,
        0x01,
        0x0c | ((layers - 1) >> 4),
        ((layers - 1) << 4) | 0x01,
    ];
    config.extend_from_slice(&[1, 32, 0, 1, 0, vps.len() as u8]);
    config.extend_from_slice(&vps);
    Some((b"hvcC", config))
}

fn video_track() -> Track {
    Track {
        handler: b"vide",
        format: b"avc1",
        size: (1280, 720),
        depth: 24,
        config: avcc(100),
        colr: None,
        pasp: None,
        matrix: IDENTITY,
        edits: Vec::new(),
        object_type: 0,
        samples: 30,
        entries: 1,
        length: None,
    }
}

fn aac_track() -> Track {
    Track {
        handler: b"soun",
        format: b"mp4a",
        size: (0, 0),
        object_type: 0x40,
        samples: 47,
        ..video_track()
    }
}

#[derive(Clone)]
struct Movie {
    brand: &'static [u8; 4],
    tracks: Vec<Track>,
    /// In milliseconds (the movie's timescale is 1000).
    duration: u32,
    moov_first: bool,
    fragmented: bool,
    mdat: usize,
}

fn movie(tracks: Vec<Track>) -> Movie {
    Movie {
        brand: b"isom",
        tracks,
        duration: 1000,
        moov_first: true,
        fragmented: false,
        mdat: 4000,
    }
}

fn sample_entry(track: &Track) -> Vec<u8> {
    let mut body = vec![0u8; 6];
    body.extend_from_slice(&1u16.to_be_bytes());
    if track.handler == b"soun" {
        body.extend_from_slice(&[0; 8]);
        body.extend_from_slice(&2u16.to_be_bytes());
        body.extend_from_slice(&16u16.to_be_bytes());
        body.extend_from_slice(&[0; 4]);
        body.extend_from_slice(&(48_000u32 << 16).to_be_bytes());
        if track.format == b"mp4a" {
            let specific = [5u8, 2, 0x11, 0x90];
            let mut config = vec![4, 13 + specific.len() as u8, track.object_type, 0x15];
            config.extend_from_slice(&[0; 11]);
            config.extend_from_slice(&specific);
            let mut es = vec![3, 3 + config.len() as u8 + 3, 0, 1, 0];
            es.extend_from_slice(&config);
            es.extend_from_slice(&[6, 1, 2]);
            body.extend_from_slice(&full(b"esds", 0, &es));
        }
    } else {
        body.extend_from_slice(&[0; 16]);
        body.extend_from_slice(&track.size.0.to_be_bytes());
        body.extend_from_slice(&track.size.1.to_be_bytes());
        body.extend_from_slice(&[0, 0x48, 0, 0, 0, 0x48, 0, 0, 0, 0, 0, 0, 0, 1]);
        body.extend_from_slice(&[0; 32]);
        body.extend_from_slice(&track.depth.to_be_bytes());
        body.extend_from_slice(&[0xff, 0xff]);
        if let Some((kind, config)) = &track.config {
            body.extend_from_slice(&atom(kind, config));
        }
        if let Some((primaries, transfer, matrix)) = track.colr {
            let mut colr = b"nclx".to_vec();
            for code in [primaries, transfer, matrix] {
                colr.extend_from_slice(&code.to_be_bytes());
            }
            colr.push(0);
            body.extend_from_slice(&atom(b"colr", &colr));
        }
        if let Some((h, v)) = track.pasp {
            body.extend_from_slice(&atom(b"pasp", &[h.to_be_bytes(), v.to_be_bytes()].concat()));
        }
    }
    atom(track.format, &body)
}

fn trak(track: &Track, id: u32, duration: u32) -> Vec<u8> {
    let mut tkhd = vec![0u8; 8];
    tkhd.extend_from_slice(&id.to_be_bytes());
    tkhd.extend_from_slice(&[0; 4]);
    tkhd.extend_from_slice(&duration.to_be_bytes());
    tkhd.extend_from_slice(&[0; 16]);
    for value in track.matrix {
        tkhd.extend_from_slice(&value.to_be_bytes());
    }
    tkhd.extend_from_slice(&(u32::from(track.size.0) << 16).to_be_bytes());
    tkhd.extend_from_slice(&(u32::from(track.size.1) << 16).to_be_bytes());
    let mut out = full(b"tkhd", 3, &tkhd);
    if !track.edits.is_empty() {
        let mut elst = (track.edits.len() as u32).to_be_bytes().to_vec();
        for (length, time) in &track.edits {
            elst.extend_from_slice(&length.to_be_bytes());
            elst.extend_from_slice(&time.to_be_bytes());
            elst.extend_from_slice(&ONE.to_be_bytes());
        }
        out.extend_from_slice(&atom(b"edts", &full(b"elst", 0, &elst)));
    }
    let mut mdhd = vec![0u8; 8];
    mdhd.extend_from_slice(&1000u32.to_be_bytes());
    mdhd.extend_from_slice(&duration.to_be_bytes());
    mdhd.extend_from_slice(&[0x55, 0xc4, 0, 0]);
    let mut hdlr = vec![0u8; 4];
    hdlr.extend_from_slice(track.handler);
    hdlr.extend_from_slice(&[0; 13]);
    let mut stsd = (track.entries as u32).to_be_bytes().to_vec();
    for _ in 0..track.entries {
        stsd.extend_from_slice(&sample_entry(track));
    }
    let mut stsz = 0u32.to_be_bytes().to_vec();
    stsz.extend_from_slice(&track.samples.to_be_bytes());
    for _ in 0..track.samples {
        stsz.extend_from_slice(&100u32.to_be_bytes());
    }
    let stbl = atom(
        b"stbl",
        &[full(b"stsd", 0, &stsd), full(b"stsz", 0, &stsz)].concat(),
    );
    let mdia = atom(
        b"mdia",
        &[
            full(b"mdhd", 0, &mdhd),
            full(b"hdlr", 0, &hdlr),
            atom(b"minf", &stbl),
        ]
        .concat(),
    );
    out.extend_from_slice(&mdia);
    atom(b"trak", &out)
}

impl Movie {
    fn bytes(&self) -> Vec<u8> {
        let mut ftyp = self.brand.to_vec();
        ftyp.extend_from_slice(&[0, 0, 2, 0]);
        ftyp.extend_from_slice(b"isomiso2avc1mp41");
        let mut mvhd = vec![0u8; 8];
        mvhd.extend_from_slice(&1000u32.to_be_bytes());
        mvhd.extend_from_slice(&self.duration.to_be_bytes());
        mvhd.extend_from_slice(&[0; 80]);
        let mut moov = full(b"mvhd", 0, &mvhd);
        for (index, track) in self.tracks.iter().enumerate() {
            let length = track.length.unwrap_or(self.duration);
            moov.extend_from_slice(&trak(track, index as u32 + 1, length));
        }
        if self.fragmented {
            moov.extend_from_slice(&atom(b"mvex", &[]));
        }
        let moov = atom(b"moov", &moov);
        let mdat = atom(b"mdat", &vec![0; self.mdat]);
        let mut out = atom(b"ftyp", &ftyp);
        if self.moov_first {
            out.extend_from_slice(&moov);
            out.extend_from_slice(&mdat);
        } else {
            out.extend_from_slice(&mdat);
            out.extend_from_slice(&moov);
        }
        out
    }
}

/// The poster every IR video shows: a 2 × 2 PNG.
fn poster() -> Vec<u8> {
    encode_png(&RgbaImage {
        width: 2,
        height: 2,
        data: vec![90; 16],
    })
    .unwrap()
}

/// A video element with the poster; a video without one prints its marker and no movie.
fn video(id: &str, at: Value, sha: &str, extra: Value) -> Value {
    let mut element = json!({
        "id": id, "origin": agent(), "kind": "media", "frame": at, "mediaKind": "video", "media": sha,
        "poster": sha256(&poster()), "loop": false, "muted": false, "autoplay": false
    });
    for (key, value) in extra.as_object().unwrap() {
        element[key] = value.clone();
    }
    element
}

/// An IR deck (720 × 405 pt) of these movies (H.264, as the frontend states their codec), each drawn by the elements
/// `uses` makes of its hash; with the plans.
fn ir_plans(
    movies: &[Vec<u8>],
    uses: impl Fn(&[String]) -> Value,
) -> (Vec<String>, Vec<VideoPlan>) {
    let mut p = presentation(vec![]);
    let sha: Vec<String> = movies
        .iter()
        .map(|bytes| {
            let sha = add_media(&mut p, bytes, "video/mp4", 1280, 720);
            p["media"][&sha]["codec"] = json!("avc1");
            sha
        })
        .collect();
    let poster = poster();
    add_media(&mut p, &poster, "image/png", 2, 2);
    p["slides"] = uses(&sha);
    let mut files: Vec<(&[u8], &str)> = movies
        .iter()
        .map(|bytes| (bytes.as_slice(), "mp4"))
        .collect();
    files.push((&poster, "png"));
    let media = media_files(&files);
    let deck = Printed::new(NAME, bytes(&p), Frontends::default().with_ir_media(media));
    let plans = deck.plans();
    (sha, plans)
}

fn plan_of<'a>(plans: &'a [VideoPlan], sha: &str) -> &'a VideoPlan {
    plans
        .iter()
        .find(|plan| plan.sha256.to_hex() == sha)
        .unwrap_or_else(|| panic!("no plan for {sha}"))
}

#[test]
fn a_clip_played_across_slides_magic_move_matches_and_a_trimmed_use_share_one_plan() {
    let movies = [
        movie(vec![video_track(), aac_track()]).bytes(),
        movie(vec![Track {
            size: (1920, 1080),
            ..video_track()
        }])
        .bytes(),
    ];
    let (sha, plans) = ir_plans(&movies, |sha| {
        let across = json!({ "acrossSlides": true });
        let mut moved = slide(
            "ir-s3",
            vec![video(
                "ir-s3.v",
                frame(10.0, 10.0, 360.0, 202.5),
                &sha[1],
                json!({ "matchKey": "clip" }),
            )],
        );
        moved["transition"] = json!({
            "durationMs": 1000,
            "effect": { "fallback": "dissolve", "id": "ir:magicMove",
                        "native": { "effect": "magicMove", "id": "ir:magicMove",
                                    "params": { "fadeUnmatched": true } } }
        });
        json!([
            slide(
                "ir-s1",
                vec![video(
                    "ir-s1.v",
                    frame(10.0, 10.0, 120.0, 67.5),
                    &sha[0],
                    json!({ "acrossSlides": true, "trim": { "startMs": 250, "endMs": 750 } }),
                )]
            ),
            slide(
                "ir-s2",
                vec![
                    video("ir-s2.v", frame(10.0, 10.0, 180.0, 101.25), &sha[0], across),
                    video(
                        "ir-s2.m",
                        frame(300.0, 10.0, 90.0, 50.625),
                        &sha[1],
                        json!({ "matchKey": "clip" }),
                    ),
                ]
            ),
            moved,
        ])
    });
    // 720 × 405 pt fits a full-HD screen twice: four pixels per CSS pixel. The clip plays across from a 120 pt box
    // into a 180 pt one: 240 × 135 px, four times.
    let across = plan_of(&plans, &sha[0]);
    assert_eq!((across.target_width, across.target_height), (960, 540));
    // Trimmed to 250–750 ms on one slide: the file keeps its whole length and its sound.
    assert_eq!(across.duration_ms, 1000.0);
    assert!(across.audio.as_ref().unwrap().aac);
    assert_eq!(across.keep, None);
    // Magic Move carries the 90 pt element to its 360 pt match on the next slide: the larger box, 480 × 270 px.
    let moved = plan_of(&plans, &sha[1]);
    assert_eq!((moved.target_width, moved.target_height), (1920, 1080));
}

#[test]
fn keeps_the_movies_it_cannot_re_encode_faithfully() {
    let cases: Vec<(Movie, Option<VideoKeep>)> = vec![
        (movie(vec![video_track()]), None),
        (
            movie(vec![Track {
                format: b"hvc1",
                config: hvcc(1),
                ..video_track()
            }]),
            None,
        ),
        (
            movie(vec![Track {
                format: b"hvc1",
                config: hvcc(2),
                ..video_track()
            }]),
            Some(VideoKeep::Alpha),
        ),
        (
            movie(vec![Track {
                depth: 32,
                ..video_track()
            }]),
            Some(VideoKeep::Alpha),
        ),
        (
            movie(vec![Track {
                matrix: [-ONE, 0, 0, 0, ONE, 0, 0, 0, 0x4000_0000],
                ..video_track()
            }]),
            Some(VideoKeep::Matrix),
        ),
        (
            movie(vec![Track {
                pasp: Some((4, 3)),
                ..video_track()
            }]),
            Some(VideoKeep::PixelAspect(4, 3)),
        ),
        (
            movie(vec![Track {
                colr: Some((9, 16, 9)),
                ..video_track()
            }]),
            Some(VideoKeep::Hdr(16)),
        ),
        (
            movie(vec![Track {
                edits: vec![(400, 0), (600, 2000)],
                ..video_track()
            }]),
            Some(VideoKeep::Edits(2)),
        ),
        (
            movie(vec![Track {
                edits: vec![(100, -1), (900, 0)],
                ..video_track()
            }]),
            None,
        ),
        (
            movie(vec![video_track(), aac_track(), aac_track()]),
            Some(VideoKeep::AudioTracks(2)),
        ),
        (
            movie(vec![video_track(), video_track()]),
            Some(VideoKeep::VideoTracks(2)),
        ),
        (
            movie(vec![Track {
                entries: 2,
                ..video_track()
            }]),
            Some(VideoKeep::Formats(2)),
        ),
        (
            Movie {
                fragmented: true,
                ..movie(vec![video_track()])
            },
            Some(VideoKeep::Fragmented),
        ),
        (
            movie(vec![Track {
                samples: 0,
                ..video_track()
            }]),
            Some(VideoKeep::Empty),
        ),
    ];
    let files: Vec<Vec<u8>> = cases
        .iter()
        .enumerate()
        .map(|(index, (movie, _))| Movie {
            // Each file its own hash.
            mdat: 4000 + index,
            ..movie.clone()
        })
        .map(|movie| movie.bytes())
        .collect();
    let (sha, plans) = ir_plans(&files, |sha| {
        json!([slide(
            "ir-s1",
            sha.iter()
                .enumerate()
                .map(|(index, sha)| video(
                    &format!("ir-s1.v{index}"),
                    frame(10.0 * index as f64, 10.0, 100.0, 56.25),
                    sha,
                    json!({})
                ))
                .collect()
        )])
    });
    for (index, (_, keep)) in cases.iter().enumerate() {
        assert_eq!(&plan_of(&plans, &sha[index]).keep, keep, "case {index}");
    }
    // The wide-gamut colour of a movie is reported, not kept.
    let (sha, plans) = ir_plans(
        &[movie(vec![Track {
            colr: Some((12, 1, 1)),
            ..video_track()
        }])
        .bytes()],
        |sha| {
            json!([slide(
                "ir-s1",
                vec![video(
                    "ir-s1.v",
                    frame(0.0, 0.0, 100.0, 56.25),
                    &sha[0],
                    json!({})
                )]
            )])
        },
    );
    let wide = plan_of(&plans, &sha[0]);
    assert_eq!(wide.keep, None);
    assert!(wide.colour.wide_gamut() && !wide.colour.hdr());
}

/// A plan for a 1280 × 720, 1000 ms movie of 50,000 bytes with AAC sound, to be encoded at 428 × 240.
fn plan() -> VideoPlan {
    let (_, plans) = ir_plans(
        &[Movie {
            mdat: 50_000,
            ..movie(vec![video_track(), aac_track()])
        }
        .bytes()],
        |sha| {
            json!([slide(
                "ir-s1",
                vec![video(
                    "ir-s1.v",
                    frame(0.0, 0.0, 80.0, 45.0),
                    &sha[0],
                    json!({})
                )]
            )])
        },
    );
    let plan = plans.into_iter().next().unwrap();
    assert_eq!((plan.target_width, plan.target_height), (428, 240));
    plan
}

fn check(plan: &VideoPlan, movie: &Movie) -> Result<CompressedVideo, VideoRefusal> {
    check_video(plan, Arc::new(BytesReader::new(movie.bytes())))
}

#[test]
fn takes_an_mp4_of_one_h264_track_of_the_planned_size_with_the_original_s_sound() {
    let plan = plan();
    let output = |video: Track| {
        movie(vec![
            Track {
                size: (428, 240),
                ..video
            },
            aac_track(),
        ])
    };
    let good = output(video_track());
    let taken = check(&plan, &good).unwrap();
    assert_eq!(taken.path, format!("media/{}.mp4", plan.sha256));
    assert_eq!(taken.bytes, good.bytes().len() as u64);
    assert_eq!(taken.replaced().mime, "video/mp4");

    let refused = |movie: Movie| check(&plan, &movie).unwrap_err();
    assert_eq!(
        refused(Movie {
            brand: b"qt  ",
            ..good.clone()
        }),
        VideoRefusal::Type("video/quicktime".to_owned())
    );
    assert_eq!(
        refused(Movie {
            moov_first: false,
            ..good.clone()
        }),
        VideoRefusal::NotFastStart
    );
    assert_eq!(
        refused(Movie {
            fragmented: true,
            ..good.clone()
        }),
        VideoRefusal::Fragmented
    );
    assert_eq!(
        refused(output(Track {
            format: b"hvc1",
            config: hvcc(1),
            ..video_track()
        })),
        VideoRefusal::Codec("hvc1".to_owned())
    );
    assert_eq!(
        refused(output(Track {
            config: avcc(244),
            ..video_track()
        })),
        VideoRefusal::Profile(244)
    );
    assert_eq!(
        refused(movie(vec![
            Track {
                size: (430, 240),
                ..video_track()
            },
            aac_track()
        ])),
        VideoRefusal::Size {
            width: 430,
            height: 240,
            expected_width: 428,
            expected_height: 240
        }
    );
    assert_eq!(
        refused(output(Track {
            matrix: [0, ONE, 0, -ONE, 0, 0, 0, 0, 0x4000_0000],
            ..video_track()
        })),
        VideoRefusal::Turned
    );
    assert_eq!(
        refused(movie(vec![Track {
            size: (428, 240),
            ..video_track()
        }])),
        VideoRefusal::AudioTracks {
            found: 0,
            expected: 1
        }
    );
    assert_eq!(
        refused(movie(vec![
            Track {
                size: (428, 240),
                ..video_track()
            },
            Track {
                format: b"sowt",
                ..aac_track()
            }
        ])),
        VideoRefusal::AudioCodec("sowt".to_owned())
    );
    // Frames an encoder dropped.
    assert_eq!(
        refused(output(Track {
            samples: 27,
            ..video_track()
        })),
        VideoRefusal::Frames {
            found: 27,
            expected: 30
        }
    );
    assert!(
        check(
            &plan,
            &output(Track {
                samples: 28,
                ..video_track()
            })
        )
        .is_ok()
    );
    let mut timecode = good.clone();
    timecode.tracks.push(Track {
        handler: b"tmcd",
        format: b"tmcd",
        ..aac_track()
    });
    assert_eq!(
        refused(timecode),
        VideoRefusal::OtherTrack("tmcd".to_owned())
    );
    // Both tracks cut to 750 ms: the video's span is refused first.
    assert!(matches!(
        refused(Movie {
            duration: 750,
            ..good.clone()
        }),
        VideoRefusal::Span {
            track: "video",
            length_ms: 750.0,
            ..
        }
    ));
    // The tracks as long as the original's, the movie's header shorter.
    let mut header = good.clone();
    header.duration = 750;
    for track in &mut header.tracks {
        track.length = Some(1000);
    }
    assert_eq!(
        refused(header),
        VideoRefusal::Duration {
            ms: 750.0,
            expected: 1000.0
        }
    );
    // Within the tolerance: a frame's rounding.
    assert!(
        check(
            &plan,
            &Movie {
                duration: 1040,
                ..good.clone()
            }
        )
        .is_ok()
    );
    let original = plan.bytes;
    let large = Movie {
        mdat: original as usize,
        ..good.clone()
    };
    assert_eq!(
        refused(large.clone()),
        VideoRefusal::NotSmaller {
            bytes: large.bytes().len() as u64,
            original
        }
    );
    assert_eq!(
        check(
            &VideoPlan {
                keep: Some(VideoKeep::Alpha),
                ..plan.clone()
            },
            &good
        )
        .unwrap_err(),
        VideoRefusal::Kept(VideoKeep::Alpha)
    );
    // A video without sound takes a file without sound only.
    let silent = VideoPlan {
        audio: None,
        ..plan.clone()
    };
    assert!(matches!(
        check(&silent, &good).unwrap_err(),
        VideoRefusal::AudioTracks {
            found: 1,
            expected: 0
        }
    ));
}

/// The probe printed, with a host's file of each re-encodable movie's target size taken, and its zip with and
/// without them.
#[test]
fn bundles_a_taken_file_in_place_of_the_original_and_the_same_zip_without_one() {
    let probe = Printed::new("video-probe.pptx", PROBE.to_vec(), Frontends::default());
    let mut session = probe.session();
    let finished = session.finish().unwrap();
    let drawn = drawn_sizes(finished.presentation.as_ref().unwrap());
    let printed = finish_deck(&session, &finished, slidock_generator()).unwrap();
    let store = SessionMedia(&session);
    let plans = plan_videos(
        &printed.documents,
        &drawn,
        &store,
        &probe.reader,
        50_000_000,
    );
    // The QuickTime movie: its file is renamed `.mp4`.
    let sound = plans
        .iter()
        .find(|plan| plan.sha256 == probe_sha("pcm"))
        .unwrap();
    let file = movie(vec![
        Track {
            size: (768, 432),
            samples: 40,
            ..video_track()
        },
        aac_track(),
    ]);
    let file = Movie {
        duration: 1600,
        ..file
    };
    let taken = check(sound, &file).unwrap();
    let videos = BTreeMap::from([(sound.path.clone(), taken.clone())]);
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
    let empty = BTreeMap::new();
    let none = CompressedMedia {
        media: &media,
        images: &empty,
        videos: &BTreeMap::new(),
    };
    // With nothing taken, the zip is byte for byte the plain one.
    assert_eq!(write(&none, &BundleOptions::default()), plain);
    let swapped = CompressedMedia {
        media: &media,
        images: &empty,
        videos: &videos,
    };
    let options = BundleOptions {
        replaced: video_replacements(&videos),
        ..BundleOptions::default()
    };
    let compressed = entries(&write(&swapped, &options));
    let original = entries(&plain);
    assert!(original.contains_key(&sound.path));
    assert!(!compressed.contains_key(&sound.path));
    assert_eq!(compressed[&taken.path], file.bytes());
    // Every other medium is as it was.
    for (name, bytes) in &original {
        if name.starts_with("media/") && name != &sound.path {
            assert_eq!(&compressed[name], bytes, "{name}");
        }
    }
    let index = String::from_utf8(compressed["index.html"].clone()).unwrap();
    assert!(!index.contains(&sound.path));
    assert!(
        index.contains(&format!("src=\"{}\"", taken.path)),
        "the slide names it"
    );
    let deck = deck_json(&index);
    let entry = deck["media"]
        .as_array()
        .unwrap()
        .iter()
        .find(|media| media["path"] == json!(taken.path))
        .unwrap();
    assert_eq!(entry["mime"], json!("video/mp4"));
    assert_eq!(entry["bytes"], json!(taken.bytes));
    // A single file inlines it with its own type.
    let single = entries(&write(
        &swapped,
        &BundleOptions {
            single_file: true,
            ..options
        },
    ));
    let index = String::from_utf8(single["index.html"].clone()).unwrap();
    assert!(index.contains("src=\"data:video/mp4;base64,"));
    // The taken file's reader is read as the bundle reaches it.
    assert_eq!(taken.reader.len(), taken.bytes);
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
fn refuses_a_track_that_starts_or_ends_elsewhere_than_the_original_s() {
    // Both tracks of the original start 500 ms late and play 1000 ms.
    let delayed = |track: Track| Track {
        edits: vec![(500, -1), (1000, 0)],
        ..track
    };
    let (_, plans) = ir_plans(
        &[Movie {
            mdat: 50_000,
            duration: 1500,
            ..movie(vec![delayed(video_track()), delayed(aac_track())])
        }
        .bytes()],
        |sha| {
            json!([slide(
                "ir-s1",
                vec![video(
                    "ir-s1.v",
                    frame(0.0, 0.0, 80.0, 45.0),
                    &sha[0],
                    json!({})
                )]
            )])
        },
    );
    let plan = plans.into_iter().next().unwrap();
    let output = |video: Track, sound: Track| Movie {
        duration: 1500,
        ..movie(vec![
            Track {
                size: (428, 240),
                ..video
            },
            sound,
        ])
    };
    assert!(check(&plan, &output(delayed(video_track()), delayed(aac_track()))).is_ok());
    // The video without its delay: it would start 500 ms ahead of its sound.
    assert!(
        check(
            &plan,
            &output(
                Track {
                    edits: vec![(1000, 0)],
                    ..video_track()
                },
                delayed(aac_track())
            )
        )
        .is_err()
    );
    // A sound cut to 400 ms (a PCM chunk read short).
    let short = Track {
        edits: vec![(500, -1), (400, 0)],
        length: Some(400),
        ..aac_track()
    };
    assert!(matches!(
        check(&plan, &output(delayed(video_track()), short)),
        Err(VideoRefusal::Span {
            track: "sound",
            length_ms: 400.0,
            expected_length_ms: 1000.0,
            ..
        })
    ));
}

#[test]
fn a_sound_is_never_heard_only_when_every_use_plays_it_muted() {
    let files: Vec<Vec<u8>> = (0..4)
        .map(|index| {
            Movie {
                mdat: 4000 + index,
                ..movie(vec![video_track(), aac_track()])
            }
            .bytes()
        })
        .collect();
    let muted = json!({ "muted": true });
    let (sha, plans) = ir_plans(&files, |sha| {
        let mut moved = slide(
            "ir-s3",
            vec![video(
                "ir-s3.c",
                frame(10.0, 10.0, 200.0, 112.5),
                &sha[2],
                json!({ "muted": true, "matchKey": "c" }),
            )],
        );
        moved["transition"] = json!({
            "durationMs": 1000,
            "effect": { "fallback": "dissolve", "id": "ir:magicMove",
                        "native": { "effect": "magicMove", "id": "ir:magicMove",
                                    "params": { "fadeUnmatched": true } } }
        });
        let mut hidden = slide(
            "ir-s4",
            vec![video(
                "ir-s4.d",
                frame(10.0, 10.0, 100.0, 56.25),
                &sha[3],
                json!({}),
            )],
        );
        hidden["hidden"] = json!(true);
        json!([
            slide(
                "ir-s1",
                vec![
                    // A clip played across two slides, muted on both.
                    video(
                        "ir-s1.a",
                        frame(10.0, 10.0, 100.0, 56.25),
                        &sha[0],
                        json!({ "muted": true, "acrossSlides": true })
                    ),
                    // Muted here, heard where its other use plays it.
                    video(
                        "ir-s1.b",
                        frame(200.0, 10.0, 100.0, 56.25),
                        &sha[1],
                        muted.clone()
                    ),
                    video(
                        "ir-s1.d",
                        frame(400.0, 10.0, 100.0, 56.25),
                        &sha[3],
                        muted.clone()
                    ),
                ]
            ),
            slide(
                "ir-s2",
                vec![
                    video(
                        "ir-s2.a",
                        frame(10.0, 10.0, 100.0, 56.25),
                        &sha[0],
                        json!({ "muted": true, "acrossSlides": true })
                    ),
                    video(
                        "ir-s2.b",
                        frame(200.0, 10.0, 100.0, 56.25),
                        &sha[1],
                        json!({})
                    ),
                    // Magic Move carries this muted element to its muted match on the next slide.
                    video(
                        "ir-s2.c",
                        frame(300.0, 10.0, 100.0, 56.25),
                        &sha[2],
                        json!({ "muted": true, "matchKey": "c" })
                    ),
                ]
            ),
            moved,
            // A hidden slide's use counts: it may be shown.
            hidden,
        ])
    });
    let silent = |index: usize| plan_of(&plans, &sha[index]).silent;
    assert!(silent(0), "muted on both slides it plays across");
    assert!(!silent(1), "heard on its second slide");
    assert!(silent(2), "muted on both sides of a Magic Move");
    assert!(!silent(3), "heard on a hidden slide");
}

#[test]
fn takes_a_file_without_the_sound_only_when_it_is_never_heard() {
    let plan = plan();
    let silent = VideoPlan {
        silent: true,
        ..plan.clone()
    };
    // The original's video track copied as it is, its sound left out.
    let copy = movie(vec![video_track()]);
    assert!(check(&silent, &copy).is_ok());
    assert_eq!(
        check(&plan, &copy).unwrap_err(),
        VideoRefusal::AudioTracks {
            found: 0,
            expected: 1
        }
    );
    // A re-encode without its sound.
    let encoded = movie(vec![Track {
        size: (428, 240),
        ..video_track()
    }]);
    assert!(check(&silent, &encoded).is_ok());
    assert!(matches!(
        check(&plan, &encoded).unwrap_err(),
        VideoRefusal::AudioTracks { found: 0, .. }
    ));
    // A sound never heard is left out.
    let heard = movie(vec![
        Track {
            size: (428, 240),
            ..video_track()
        },
        aac_track(),
    ]);
    assert!(check(&plan, &heard).is_ok());
    assert_eq!(
        check(&silent, &heard).unwrap_err(),
        VideoRefusal::AudioTracks {
            found: 1,
            expected: 0
        }
    );
    // A copy is the original's track (its frames, its matrix); anything else is held to a re-encode's rules.
    let short = movie(vec![Track {
        samples: 20,
        ..video_track()
    }]);
    assert!(matches!(
        check(&silent, &short).unwrap_err(),
        VideoRefusal::Size { width: 1280, .. }
    ));
    let mut flipped = IDENTITY;
    flipped[0] = -flipped[0];
    let turned = movie(vec![Track {
        matrix: flipped,
        ..video_track()
    }]);
    assert!(matches!(
        check(&silent, &turned).unwrap_err(),
        VideoRefusal::Size { width: 1280, .. }
    ));
    // A video kept from a re-encode for what a copy carries as it is (its alpha, its matrix, …) is copied; one kept
    // for a structure a copy does not carry stays as it is.
    let alpha = VideoPlan {
        keep: Some(VideoKeep::Alpha),
        ..silent.clone()
    };
    assert!(check(&alpha, &copy).is_ok());
    assert_eq!(
        check(&alpha, &encoded).unwrap_err(),
        VideoRefusal::Kept(VideoKeep::Alpha)
    );
    assert_eq!(
        check(
            &VideoPlan {
                silent: false,
                ..alpha.clone()
            },
            &copy
        )
        .unwrap_err(),
        VideoRefusal::Kept(VideoKeep::Alpha)
    );
    let edits = VideoPlan {
        keep: Some(VideoKeep::Edits(2)),
        uncopyable: Some(VideoKeep::Edits(2)),
        ..silent.clone()
    };
    assert_eq!(
        check(&edits, &copy).unwrap_err(),
        VideoRefusal::Kept(VideoKeep::Edits(2))
    );
}

#[test]
fn a_kept_video_states_whether_a_copy_carries_it() {
    let files: Vec<Vec<u8>> = vec![
        // Its matrix mirrors it: a copy carries the matrix.
        movie(vec![
            Track {
                matrix: {
                    let mut flipped = IDENTITY;
                    flipped[0] = -flipped[0];
                    flipped
                },
                ..video_track()
            },
            aac_track(),
        ])
        .bytes(),
        // Two pieces of an edit list: a copy does not carry them.
        movie(vec![
            Track {
                edits: vec![(400, 0), (400, 600)],
                ..video_track()
            },
            aac_track(),
        ])
        .bytes(),
        // Fragmented.
        Movie {
            fragmented: true,
            ..movie(vec![video_track(), aac_track()])
        }
        .bytes(),
    ];
    let (sha, plans) = ir_plans(&files, |sha| {
        json!([slide(
            "ir-s1",
            sha.iter()
                .enumerate()
                .map(|(index, sha)| video(
                    &format!("ir-s1.v{index}"),
                    frame(10.0 + 100.0 * index as f64, 10.0, 80.0, 45.0),
                    sha,
                    json!({ "muted": true })
                ))
                .collect()
        )])
    });
    let of = |index: usize| plan_of(&plans, &sha[index]);
    assert_eq!(of(0).keep, Some(VideoKeep::Matrix));
    assert_eq!(of(0).uncopyable, None);
    assert!(of(0).silent);
    assert_eq!(of(1).keep, Some(VideoKeep::Edits(2)));
    assert_eq!(of(1).uncopyable, Some(VideoKeep::Edits(2)));
    assert_eq!(of(2).uncopyable, Some(VideoKeep::Fragmented));
    // A movie without sound has none to leave out.
    let (sha, plans) = ir_plans(&[movie(vec![video_track()]).bytes()], |sha| {
        json!([slide(
            "ir-s1",
            vec![video(
                "ir-s1.v",
                frame(10.0, 10.0, 80.0, 45.0),
                &sha[0],
                json!({ "muted": true })
            )]
        )])
    });
    assert!(!plan_of(&plans, &sha[0]).silent);
}

#[test]
fn an_uncompressed_sound_s_bytes_are_its_frames_times_the_bytes_of_one() {
    // QuickTime states each frame of PCM as one sample, whatever size stsz gives it (100 here): 16-bit stereo is 4.
    let pcm = movie(vec![
        video_track(),
        Track {
            format: b"sowt",
            samples: 1000,
            ..aac_track()
        },
    ]);
    let (sha, plans) = ir_plans(&[pcm.bytes()], |sha| {
        json!([slide(
            "ir-s1",
            vec![video(
                "ir-s1.v",
                frame(10.0, 10.0, 80.0, 45.0),
                &sha[0],
                json!({})
            )]
        )])
    });
    assert_eq!(plan_of(&plans, &sha[0]).audio.as_ref().unwrap().bytes, 4000);
    // AAC's are its samples'.
    assert_eq!(plan().audio.unwrap().bytes, 47 * 100);
}
