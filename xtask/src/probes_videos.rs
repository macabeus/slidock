//! The video probe deck (`probes/out/video-probe.pptx`): one 960 × 540 pt slide of movies, for the download's video
//! compression (`slidock::videos`). The movies are our own (`probes/videos/make-media.sh`), each with its first frame
//! as its poster; each names what it pins:
//!
//! - "Clip with sound": a 1280 × 720 H.264 movie with an AAC tone, drawn at 240 × 135 pt: re-encoded at 960 × 540
//!   with its sound carried over as it is.
//! - "Turned clip": stored 960 × 540 with a quarter-turn matrix (it shows 540 × 960), drawn at 90 × 160 pt:
//!   re-encoded upright at 360 × 640.
//! - "Tiny clip": 320 × 180 at about 46 kbit/s, drawn at 160 × 90 pt: already small, so it stays as it is.
//! - "PCM clip" and "PCM clip again": one QuickTime movie with 16-bit PCM sound, drawn at 192 × 108 pt and, trimmed
//!   (`p14:trim`), at 128 × 72 pt. The larger use sets its size (768 × 432); its sound becomes AAC; it keeps its
//!   whole length.
//! - "Clip with alpha": HEVC with an alpha layer: it stays as it is.
//! - "Clip at its own size": 640 × 360 at a high bitrate, its colour unstated, drawn at 320 × 180 pt (larger than it
//!   is): re-encoded at its own size, its frames as decoded.
//! - "Muted clip": 640 × 360 at a low bitrate with an AAC tone, its media node muted (`mute="1"`): its sound is never
//!   heard, and at its own size under the bitrate its video is copied as it is without it.
//! - "Clip with sound, muted": the clip with sound again, muted: its other use is heard, so it keeps its sound.
//! - "Quiet clip": 960 × 540 with an AAC tone, its media node at a volume of 0 (`vol="0"`): re-encoded without sound.

use std::fmt::Write as _;
use std::path::Path;

use crate::probes_pictures::pptx_package;
use crate::probes_pictures::probes_zip::{Entry, sha256_hex, write_zip};

const USAGE: &str = "usage: cargo xtask probes-videos [--check]";

/// EMU per point.
const EMU: u32 = 12_700;

const NS_P14: &str = "http://schemas.microsoft.com/office/powerpoint/2010/main";

/// The relationship type of an embedded movie (`p14:media`).
const MEDIA_REL: &str = "http://schemas.microsoft.com/office/2007/relationships/media";

const SOUND: &[u8] = include_bytes!("../../probes/videos/media/sound.mp4");
const TURNED: &[u8] = include_bytes!("../../probes/videos/media/turned.mp4");
const TINY: &[u8] = include_bytes!("../../probes/videos/media/tiny.mp4");
const PCM: &[u8] = include_bytes!("../../probes/videos/media/pcm.mov");
const ALPHA: &[u8] = include_bytes!("../../probes/videos/media/alpha.mov");
const OWN: &[u8] = include_bytes!("../../probes/videos/media/own.mp4");
const MUTED: &[u8] = include_bytes!("../../probes/videos/media/muted.mp4");
const QUIET: &[u8] = include_bytes!("../../probes/videos/media/quiet.mp4");

const SOUND_POSTER: &[u8] = include_bytes!("../../probes/videos/media/sound.png");
const TURNED_POSTER: &[u8] = include_bytes!("../../probes/videos/media/turned.png");
const TINY_POSTER: &[u8] = include_bytes!("../../probes/videos/media/tiny.png");
const PCM_POSTER: &[u8] = include_bytes!("../../probes/videos/media/pcm.png");
const ALPHA_POSTER: &[u8] = include_bytes!("../../probes/videos/media/alpha.png");
const OWN_POSTER: &[u8] = include_bytes!("../../probes/videos/media/own.png");
const MUTED_POSTER: &[u8] = include_bytes!("../../probes/videos/media/muted.png");
const QUIET_POSTER: &[u8] = include_bytes!("../../probes/videos/media/quiet.png");

struct Item {
    name: &'static str,
    /// x, y, w, h in points.
    frame: (u32, u32, u32, u32),
    /// The movie's part name and bytes, then its poster's.
    movie: (&'static str, &'static [u8]),
    poster: (&'static str, &'static [u8]),
    /// `p14:trim`'s cuts from the start and the end, in milliseconds.
    trim: Option<(u32, u32)>,
    /// The attributes of its media node in the slide's timing (`p:cMediaNode`), when it has one.
    playback: Option<&'static str>,
}

const ITEMS: [Item; 10] = [
    Item {
        name: "Clip with sound",
        frame: (40, 40, 240, 135),
        movie: ("sound.mp4", SOUND),
        poster: ("sound.png", SOUND_POSTER),
        trim: None,
        playback: None,
    },
    Item {
        name: "Turned clip",
        frame: (320, 40, 90, 160),
        movie: ("turned.mp4", TURNED),
        poster: ("turned.png", TURNED_POSTER),
        trim: None,
        playback: None,
    },
    Item {
        name: "Tiny clip",
        frame: (460, 40, 160, 90),
        movie: ("tiny.mp4", TINY),
        poster: ("tiny.png", TINY_POSTER),
        trim: None,
        playback: None,
    },
    Item {
        name: "PCM clip",
        frame: (40, 260, 192, 108),
        movie: ("pcm.mov", PCM),
        poster: ("pcm.png", PCM_POSTER),
        trim: None,
        playback: None,
    },
    Item {
        name: "PCM clip again",
        frame: (260, 260, 128, 72),
        movie: ("pcm.mov", PCM),
        poster: ("pcm.png", PCM_POSTER),
        trim: Some((400, 400)),
        playback: None,
    },
    Item {
        name: "Clip with alpha",
        frame: (460, 260, 160, 120),
        movie: ("alpha.mov", ALPHA),
        poster: ("alpha.png", ALPHA_POSTER),
        trim: None,
        playback: None,
    },
    Item {
        name: "Clip at its own size",
        frame: (640, 300, 320, 180),
        movie: ("own.mp4", OWN),
        poster: ("own.png", OWN_POSTER),
        trim: None,
        playback: None,
    },
    Item {
        name: "Muted clip",
        frame: (40, 400, 160, 90),
        movie: ("muted.mp4", MUTED),
        poster: ("muted.png", MUTED_POSTER),
        trim: None,
        playback: Some(" vol=\"80000\" mute=\"1\""),
    },
    Item {
        name: "Clip with sound, muted",
        frame: (220, 400, 112, 63),
        movie: ("sound.mp4", SOUND),
        poster: ("sound.png", SOUND_POSTER),
        trim: None,
        playback: Some(" vol=\"80000\" mute=\"1\""),
    },
    Item {
        name: "Quiet clip",
        frame: (360, 400, 192, 108),
        movie: ("quiet.mp4", QUIET),
        poster: ("quiet.png", QUIET_POSTER),
        trim: None,
        playback: Some(" vol=\"0\""),
    },
];

pub fn run(args: &[String]) -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../probes/out");
    match args.first().map(String::as_str) {
        None => {
            for (name, bytes) in generate() {
                let path = root.join(&name);
                std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
                println!(
                    "wrote probes/out/{name} ({} bytes, sha256 {})",
                    bytes.len(),
                    sha256_hex(&bytes)
                );
            }
            Ok(())
        }
        Some("--check") => {
            let stale: Vec<String> = generate()
                .into_iter()
                .filter(|(name, bytes)| std::fs::read(root.join(name)).ok().as_ref() != Some(bytes))
                .map(|(name, _)| name)
                .collect();
            if stale.is_empty() {
                println!("probes/out video files are up to date");
                Ok(())
            } else {
                Err(format!(
                    "stale (run cargo xtask probes-videos): {}",
                    stale.join(", ")
                ))
            }
        }
        _ => Err(USAGE.to_owned()),
    }
}

/// Every generated file: (name under `probes/out/`, bytes).
pub fn generate() -> Vec<(String, Vec<u8>)> {
    vec![("video-probe.pptx".to_owned(), write_zip(&pptx()))]
}

fn xfrm((x, y, w, h): (u32, u32, u32, u32)) -> String {
    format!(
        "<a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>",
        x * EMU,
        y * EMU,
        w * EMU,
        h * EMU
    )
}

/// A relationship id for a part name.
fn rel_id(file: &str, kind: &str) -> String {
    format!("rId{kind}{}", file.replace('.', ""))
}

fn pptx() -> Vec<Entry> {
    let mut tree = String::new();
    let mut rels: Vec<(String, &'static str, String)> = Vec::new();
    let mut media: Vec<Entry> = Vec::new();
    let mut add = |id: String,
                   kind: &'static str,
                   file: &'static str,
                   bytes: &'static [u8],
                   rels: &mut Vec<_>| {
        if !rels
            .iter()
            .any(|(known, _, _): &(String, _, _)| known == &id)
        {
            rels.push((id, kind, format!("../media/{file}")));
            if !media
                .iter()
                .any(|entry: &Entry| entry.name == format!("ppt/media/{file}"))
            {
                media.push(Entry {
                    name: format!("ppt/media/{file}"),
                    data: bytes.to_vec(),
                    stored: true,
                });
            }
        }
    };
    let mut nodes = String::new();
    for (n, item) in ITEMS.iter().enumerate() {
        let id = n + 2;
        if let Some(playback) = item.playback {
            let _ = write!(
                nodes,
                "<p:video><p:cMediaNode{playback}><p:cTn id=\"{}\" fill=\"hold\" display=\"0\"><p:stCondLst><p:cond delay=\"indefinite\"/></p:stCondLst></p:cTn><p:tgtEl><p:spTgt spid=\"{id}\"/></p:tgtEl></p:cMediaNode></p:video>",
                n + 50
            );
        }
        let (movie, movie_bytes) = item.movie;
        let (poster, poster_bytes) = item.poster;
        let media_id = rel_id(movie, "Media");
        let video_id = rel_id(movie, "Video");
        let poster_id = rel_id(poster, "Image");
        add(media_id.clone(), "media", movie, movie_bytes, &mut rels);
        add(video_id.clone(), "video", movie, movie_bytes, &mut rels);
        add(poster_id.clone(), "image", poster, poster_bytes, &mut rels);
        let trim = item
            .trim
            .map(|(st, end)| format!("<p14:trim st=\"{st}\" end=\"{end}\"/>"))
            .unwrap_or_default();
        let _ = write!(
            tree,
            "<p:pic><p:nvPicPr><p:cNvPr id=\"{id}\" name=\"{}\"/><p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr><p:nvPr><a:videoFile r:link=\"{video_id}\"/><p:extLst><p:ext uri=\"{{DAA4B4D4-6D71-4841-9C94-3DE7FCFB9230}}\"><p14:media xmlns:p14=\"{NS_P14}\" r:embed=\"{media_id}\">{trim}</p14:media></p:ext></p:extLst></p:nvPr></p:nvPicPr><p:blipFill><a:blip r:embed=\"{poster_id}\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr>{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>",
            item.name,
            xfrm(item.frame)
        );
    }
    let defaults = "<Default Extension=\"png\" ContentType=\"image/png\"/><Default Extension=\"mp4\" ContentType=\"video/mp4\"/><Default Extension=\"mov\" ContentType=\"video/quicktime\"/>";
    let mut entries = pptx_package(defaults, &tree, rels, media);
    // An embedded movie's relationship is Microsoft's own type, outside the package's namespace.
    // The media nodes, under the timing root, give the muted and silent uses their playback.
    let timing = format!(
        "<p:timing><p:tnLst><p:par><p:cTn id=\"1\" dur=\"indefinite\" restart=\"never\" nodeType=\"tmRoot\"><p:childTnLst>{nodes}</p:childTnLst></p:cTn></p:par></p:tnLst></p:timing></p:sld>"
    );
    for entry in &mut entries {
        if entry.name == "ppt/slides/slide1.xml" {
            entry.data = String::from_utf8_lossy(&entry.data)
                .replacen("</p:sld>", &timing, 1)
                .into_bytes();
        }
        if entry.name == "ppt/slides/_rels/slide1.xml.rels" {
            let text = String::from_utf8_lossy(&entry.data).replace(
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/media\"",
                &format!("{MEDIA_REL}\""),
            );
            entry.data = text.into_bytes();
        }
    }
    entries
}
