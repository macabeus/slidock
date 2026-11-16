//! Keynote's movie exports as the motion lane's transition oracles.
//!
//! **Movie.** Keynote 14.5's `export … as QuickTime movie` (native size, H.264, 60 fps) shows each slide for
//! [`HOLD_MS`] and then plays the transition into the next shown slide over its own duration, on its own curve. A
//! transition's span is its first frame (progress 0) and its length in frames; it is recorded per slide it plays into
//! (`oracle --movie --record`, proposed from that timing and checked against where the movie changes). A deck with
//! builds is not timed so, and is refused.
//!
//! **Identity.** The record pins the movie file's sha256 and, under a decoder key (the decoder's version and the
//! arguments it decodes with), the sha256 of each scored frame's RGBA raster: a movie that is not the recorded file
//! is the oracle only when every frame it gives is the pinned one (`oracle-identity` otherwise, and the deck is not
//! scored).
//!
//! **Scores.** Each transition row `#t@<pm>` is the player's frame at `pm` ‰ of the transition, and its oracle is the
//! movie frame `start + pm · frames / 1000`, rounded: both on the transition's normalised time, so the player's curve
//! and Keynote's meet where they agree (Keynote 14.5's Magic Move is `cubic-bezier(0.42, 0, 0.58, 1)`, measured on
//! the probe's movie to 0.01 pt). The pair is scored with the static lane's SSIM and ΔE00 at the metric size, and
//! classed with `classifier@2`'s SSIM and colour bounds (the text bounds do not apply: a frame mid-move has no word
//! boxes to align), which is reported. What a deck's frames can reach is its static fidelity (its faces, its
//! antialiasing, the movie's compression): the transition's two ends, scored the same against the movie's first and
//! last frames of the transition, set the row's ceiling ([`Ceiling`]), and a row that falls short of it by more than
//! [`SHORT_SSIM`] or [`SHORT_DELTA_E`] moves differently from Keynote: it is `motion-mismatch`, with the check
//! `movie`.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use slidock_bench_metrics::{HostLibm, Rgba, color_difference, metric_pair, quantise, ssim};
use slidock_bench_schema::Sha256;
use slidock_bench_schema::corpus::KeynoteExporter;
use slidock_bench_schema::results::MotionOutcome;

use super::deck::IrSlide;
use crate::error::{BenchError, Result};
use crate::exec::{ExecOptions, exec};

/// The movie records, beside the corpus manifest: `bench/corpus/movies.json`.
pub const RECORD_FILE: &str = "movies.json";
/// The movie scores of a motion run, per pair: `runs/<key>/movies.json`.
pub const MOVIES_FILE: &str = "movies.json";
/// How long Keynote 14.5's movie export shows each slide before its transition.
pub const HOLD_MS: u32 = 5_000;
/// The decoder's arguments after the input: the frames asked for, as 8-bit RGBA from BT.709 video range.
const DECODE_FILTER: &str = "scale=in_color_matrix=bt709:in_range=tv:out_range=pc:flags=accurate_rnd+full_chroma_int,format=rgba";
/// A decode's wall-clock limit.
const DECODE_TIMEOUT: Duration = Duration::from_secs(600);
/// A frame pixel moves when a channel moves by more than this (the movie's own blips stay under it).
const MOVE_CHANNEL: i16 = 8;

/// `movies.json`: each presentation's movie oracle, by presentation id.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MovieRecords {
    pub movies: BTreeMap<String, MovieOracle>,
}

/// A Keynote movie export of one presentation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MovieOracle {
    pub exporter: KeynoteExporter,
    /// The file's name when recorded: a hint only, the movie is found by the source's sha256.
    pub file: String,
    pub sha256: Sha256,
    pub fps: u32,
    pub width: u32,
    pub height: u32,
    /// Each transition's span, by the slide it plays into.
    pub transitions: BTreeMap<String, Span>,
    /// The scored frames' RGBA sha256, by decoder key, then by row (`<slide>#t@<pm>`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub frame_hashes: BTreeMap<String, BTreeMap<String, Sha256>>,
}

/// A transition in the movie: its first frame (progress 0) and its length in frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Span {
    pub start: u32,
    pub frames: u32,
}

impl Span {
    /// The movie frame at `pm` ‰ of the span, and how far (‰ of the span) it lies from that time.
    pub fn frame(&self, pm: u32) -> (u32, f64) {
        let exact = f64::from(pm) * f64::from(self.frames) / 1000.0;
        let at = exact.round();
        let off = if self.frames == 0 {
            0.0
        } else {
            (at - exact) * 1000.0 / f64::from(self.frames)
        };
        (self.start + at as u32, quantise(off))
    }
}

/// The records file beside a manifest.
pub fn record_path(manifest: &Path) -> PathBuf {
    manifest.with_file_name(RECORD_FILE)
}

pub fn load_records(path: &Path) -> Result<MovieRecords> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| BenchError::failed(format!("{}: {e}", path.display()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(MovieRecords::default()),
        Err(error) => Err(BenchError::io(path)(error)),
    }
}

/// Writes the records canonically: sorted keys, two-space indent, a final newline.
pub fn save_records(path: &Path, records: &MovieRecords) -> Result<()> {
    let text = slidock_bench_schema::to_canonical(records)
        .map_err(|e| BenchError::failed(e.to_string()))?;
    crate::cache::write_atomic(path, text)
}

/// The movie export of a source in `dir/<sourceSha>/`: its last `.m4v` or `.mov` by name.
pub fn movie_of(dir: &Path, source_sha: &str) -> Option<PathBuf> {
    let mut movies: Vec<PathBuf> = std::fs::read_dir(dir.join(source_sha))
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "m4v" || x == "mov"))
        .collect();
    movies.sort();
    movies.pop()
}

/// What a deck's IR gives each transition the movie plays: the slide it plays into and its length in ms, in show
/// order; `Err` when the deck is not timed as [`HOLD_MS`] says (a shown slide with builds).
pub fn transitions_of(slides: &[IrSlide]) -> std::result::Result<Vec<(String, u32)>, String> {
    let shown: Vec<&IrSlide> = slides.iter().filter(|s| !s.hidden).collect();
    if let Some(slide) = shown.iter().find(|s| !s.timeline.is_empty()) {
        return Err(format!(
            "{} has builds, which the movie's timing does not model",
            slide.id
        ));
    }
    Ok(shown
        .iter()
        .skip(1)
        .filter_map(|slide| {
            let ms = slide.transition.as_ref()?.duration_ms.as_f64()?;
            (ms > 0.0).then(|| (slide.id.clone(), ms.round() as u32))
        })
        .collect())
}

/// The spans Keynote's export timing gives: every shown slide held [`HOLD_MS`], each transition after its outgoing
/// slide's hold, all in frames at `fps`. Only slides with a transition of their own are listed; a cut takes no time.
pub fn proposed_spans(transitions: &[(String, u32)], fps: u32) -> BTreeMap<String, Span> {
    let frames = |ms: u32| (u64::from(ms) * u64::from(fps)).div_ceil(1000) as u32;
    let mut at = 0u32;
    let mut out = BTreeMap::new();
    for (slide, ms) in transitions {
        at += frames(HOLD_MS);
        let span = Span {
            start: at,
            frames: frames(*ms),
        };
        at += span.frames;
        out.insert(slide.clone(), span);
    }
    out
}

/// The movie decoder: `SLIDOCK_FFMPEG`, else `ffmpeg` on the path.
#[derive(Clone, Debug)]
pub struct Decoder {
    pub binary: OsString,
    /// Its `-version` first line.
    pub version: String,
}

impl Decoder {
    pub async fn find() -> Result<Decoder> {
        let binary: OsString = std::env::var_os("SLIDOCK_FFMPEG")
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "ffmpeg".into());
        let result = exec(
            binary.clone(),
            &["-version".into()],
            &ExecOptions::timeout(Duration::from_secs(30)),
        )
        .await
        .map_err(|e| {
            BenchError::refused(format!(
                "no movie decoder ({e}); install ffmpeg or set SLIDOCK_FFMPEG"
            ))
        })?;
        if !result.ok() {
            return Err(BenchError::refused(
                "ffmpeg -version failed; set SLIDOCK_FFMPEG",
            ));
        }
        let version = result
            .stdout_text()
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned();
        Ok(Decoder { binary, version })
    }

    /// The decoder key: its version and its arguments, hashed.
    pub fn key(&self) -> String {
        Sha256::of(format!("{}\n{DECODE_FILTER}", self.version)).to_string()
    }

    /// The movie's frames `indices` (any order, repeats allowed), each `width` × `height` RGBA.
    pub async fn frames(
        &self,
        movie: &Path,
        (width, height): (u32, u32),
        indices: &[u32],
    ) -> Result<BTreeMap<u32, Rgba>> {
        let mut wanted: Vec<u32> = indices.to_vec();
        wanted.sort_unstable();
        wanted.dedup();
        if wanted.is_empty() {
            return Ok(BTreeMap::new());
        }
        let select: Vec<String> = wanted.iter().map(|n| format!("eq(n\\,{n})")).collect();
        let filter = format!("select='{}',{DECODE_FILTER}", select.join("+"));
        let size = (width as usize) * (height as usize) * 4;
        let arguments: Vec<OsString> = [
            "-v",
            "error",
            "-nostdin",
            "-i",
            &movie.to_string_lossy(),
            "-vf",
            &filter,
            "-fps_mode",
            "passthrough",
            "-f",
            "rawvideo",
            "-",
        ]
        .iter()
        .map(OsString::from)
        .collect();
        let options = ExecOptions {
            timeout: Some(DECODE_TIMEOUT),
            max_output_bytes: Some(size * wanted.len() + 1),
            ..ExecOptions::default()
        };
        let result = exec(self.binary.clone(), &arguments, &options).await?;
        if !result.ok() || result.stdout.len() != size * wanted.len() {
            return Err(BenchError::failed(format!(
                "{}: the decoder gave {} bytes for {} frames of {width}×{height} ({})",
                movie.display(),
                result.stdout.len(),
                wanted.len(),
                result.stderr.trim()
            )));
        }
        let mut out = BTreeMap::new();
        for (n, chunk) in wanted.iter().zip(result.stdout.chunks_exact(size)) {
            let image = Rgba::new(width as usize, height as usize, chunk.to_vec())
                .map_err(|e| BenchError::failed(e.to_string()))?;
            out.insert(*n, image);
        }
        Ok(out)
    }
}

/// The sha256 of a frame's RGBA raster.
pub fn frame_hash(image: &Rgba) -> Sha256 {
    Sha256::of(image.data())
}

/// The pixels of two frames of one movie where a channel moves by more than [`MOVE_CHANNEL`].
pub fn moved_pixels(a: &Rgba, b: &Rgba) -> usize {
    a.data()
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.data().as_chunks::<4>().0)
        .filter(|(p, q)| {
            p.iter()
                .zip(q.iter())
                .take(3)
                .any(|(x, y)| (i16::from(*x) - i16::from(*y)).abs() > MOVE_CHANNEL)
        })
        .count()
}

/// How a recorded span answers the movie: the frame before it equals the one before that (the outgoing slide is held),
/// its middle frame moved from its first, and the frame after its end equals the one after that (the incoming slide
/// is held). The frames asked for are `span_checks`'.
pub fn span_problems(span: &Span, frames: &BTreeMap<u32, Rgba>, still: usize) -> Vec<String> {
    let get = |n: u32| frames.get(&n);
    let mut out = Vec::new();
    let [before2, before, first, middle, after, after2] = span_checks(span);
    match (
        get(before2),
        get(before),
        get(first),
        get(middle),
        get(after),
        get(after2),
    ) {
        (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)) => {
            if moved_pixels(a, b) > still {
                out.push(format!(
                    "frames {before2}–{before} move: the outgoing slide is not held there"
                ));
            }
            if moved_pixels(c, d) <= still {
                out.push(format!(
                    "frames {first}–{middle} do not move: no transition there"
                ));
            }
            if moved_pixels(e, f) > still {
                out.push(format!(
                    "frames {after}–{after2} move: the incoming slide is not held there"
                ));
            }
        }
        _ => out.push("the movie has no frames there".to_owned()),
    }
    out
}

/// The frames [`span_problems`] reads.
pub fn span_checks(span: &Span) -> [u32; 6] {
    let end = span.start + span.frames;
    [
        span.start.saturating_sub(4),
        span.start.saturating_sub(1),
        span.start,
        span.start + span.frames / 2,
        end + 2,
        end + 6,
    ]
}

/// One transition row scored against its movie frame.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieScore {
    /// The movie frame, and how far (‰ of the transition) it lies from the row's time.
    pub frame: u32,
    pub off_pm: f64,
    pub ssim: f64,
    pub ssim_p1: f64,
    pub delta_e: f64,
    /// `classifier@2`'s SSIM and colour bounds on the scores ([`movie_class`]): reported.
    pub class: &'static str,
    /// What the deck's stills reach against Keynote's at this time ([`Ceiling`]), and how far the row falls short.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ceiling: Option<Ceiling>,
    /// The player's frame scored (`frames/<sha256>.png` of the run).
    pub render: Sha256,
}

/// How far a row may fall short of its ceiling: below it by more SSIM, or above it by more ΔE00, the motion differs.
/// Measured on the two recorded decks: rows that move as Keynote's fall short by at most 0.020 SSIM (a panel of text
/// in a substitute face crossing the slide, which the ends' interpolation does not weigh) and 0.25 ΔE; a Magic Move
/// that crossfades its matches late, or keeps a match's opacity to the end, falls short by 0.037 to 0.069 SSIM, or 4.9
/// to 8.5 ΔE.
pub const SHORT_SSIM: f64 = 0.03;
pub const SHORT_DELTA_E: f64 = 1.0;

/// The scores the transition's two ends reach against Keynote's (the frame at 0 against the movie's first frame of
/// the transition, at 1000 against its last), interpolated at the row's time: what the deck's static fidelity (its
/// faces, its antialiasing, the movie's compression) lets a frame reach, so the row is held to its motion alone.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ceiling {
    pub ssim: f64,
    pub delta_e: f64,
}

impl Ceiling {
    /// Between the ends' (ssim mean, ssim p1, ΔE) scores at `pm` ‰.
    pub fn between(first: (f64, f64, f64), last: (f64, f64, f64), pm: u32) -> Ceiling {
        let t = f64::from(pm) / 1000.0;
        Ceiling {
            ssim: quantise(first.0 + (last.0 - first.0) * t),
            delta_e: quantise(first.2 + (last.2 - first.2) * t),
        }
    }
}

impl MovieScore {
    pub fn new(
        (frame, off_pm): (u32, f64),
        (ssim, ssim_p1, delta_e): (f64, f64, f64),
        ceiling: Option<Ceiling>,
        render: Sha256,
    ) -> MovieScore {
        MovieScore {
            frame,
            off_pm,
            ssim,
            ssim_p1,
            delta_e,
            class: class_name(movie_class(ssim, ssim_p1, delta_e)),
            ceiling,
            render,
        }
    }

    /// Why the row's motion differs from Keynote's: it falls short of its ceiling by more than [`SHORT_SSIM`] or
    /// [`SHORT_DELTA_E`]; `None` when it does not, or has no ceiling.
    pub fn failure(&self) -> Option<String> {
        let ceiling = self.ceiling?;
        let short = ceiling.ssim - self.ssim;
        let over = self.delta_e - ceiling.delta_e;
        (short > SHORT_SSIM || over > SHORT_DELTA_E).then(|| {
            format!(
                "movie: ssim {} (the ends reach {}), ΔE {} ({}) against Keynote's frame {}",
                self.ssim, ceiling.ssim, self.delta_e, ceiling.delta_e, self.frame
            )
        })
    }
}

/// The class of a movie score: `classifier@2`'s SSIM and colour bounds.
pub fn movie_class(ssim_mean: f64, ssim_p1: f64, delta_e: f64) -> MotionOutcome {
    use slidock_bench_metrics::{CLOSE, MATCH};
    if ssim_mean >= MATCH.ssim_mean && ssim_p1 >= MATCH.ssim_p1 && delta_e <= MATCH.color_mean {
        MotionOutcome::MotionMatch
    } else if ssim_mean >= CLOSE.ssim_mean && delta_e <= CLOSE.color_mean {
        MotionOutcome::MotionClose
    } else {
        MotionOutcome::MotionMismatch
    }
}

pub fn class_name(class: MotionOutcome) -> &'static str {
    match class {
        MotionOutcome::MotionMatch => "motion-match",
        MotionOutcome::MotionClose => "motion-close",
        MotionOutcome::MotionMismatch => "motion-mismatch",
        MotionOutcome::MotionFallback => "motion-fallback",
    }
}

/// The worse of two motion classes.
pub fn worse(a: MotionOutcome, b: MotionOutcome) -> MotionOutcome {
    let rank = |c: MotionOutcome| match c {
        MotionOutcome::MotionMatch => 0,
        MotionOutcome::MotionFallback => 1,
        MotionOutcome::MotionClose => 2,
        MotionOutcome::MotionMismatch => 3,
    };
    if rank(b) > rank(a) { b } else { a }
}

/// Scores a player frame against its movie frame.
pub fn score(movie: &Rgba, render: &Rgba) -> Option<(f64, f64, f64)> {
    let (oracle, played) = metric_pair(movie, render).ok()?;
    let structure = ssim(&oracle, &played).ok()?;
    let color = color_difference::<HostLibm>(&oracle, &played).ok()?;
    Some((
        quantise(structure.mean),
        quantise(structure.p1),
        quantise(color.mean),
    ))
}

/// A pair's movie oracle in a run.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairMovie {
    /// `scored`, `pending: …` or `oracle-identity: …`.
    pub status: String,
    pub movie: String,
    pub sha256: String,
    /// `file` (the recorded movie), `frames` (another file whose frames are the pinned ones) or `unpinned`.
    pub pinned: String,
    pub decoder: String,
    pub rows: BTreeMap<String, MovieScore>,
}

impl PairMovie {
    pub fn fails(&self) -> bool {
        self.status.starts_with("oracle-identity")
    }

    pub fn line(&self, pair: &str) -> String {
        let mut classes: BTreeMap<&str, u32> = BTreeMap::new();
        for score in self.rows.values() {
            *classes.entry(score.class).or_default() += 1;
        }
        let counts: Vec<String> = classes.iter().map(|(c, n)| format!("{c} {n}")).collect();
        format!(
            "movie      {pair}: {} ({} rows{}{})",
            self.status,
            self.rows.len(),
            if counts.is_empty() { "" } else { ": " },
            counts.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ir(id: &str, hidden: bool, ms: Option<f64>, builds: bool) -> IrSlide {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "hidden": hidden,
            "timeline": if builds { serde_json::json!([{
                "target": { "element": "x" }, "trigger": "click", "kind": "entrance",
                "delayMs": 0, "durationMs": 500, "effect": { "id": "keynote:apple:dissolve", "fallback": "fade" } }]) }
                else { serde_json::json!([]) },
            "transition": ms.map(|ms| serde_json::json!({ "durationMs": ms, "effect": { "id": "keynote:apple:magic-move", "fallback": "dissolve" } })),
        }))
        .unwrap()
    }

    #[test]
    fn spans_follow_keynotes_export_timing() {
        // Shown slides held 5 s each, a hidden slide skipped, a cut taking no time: the probe's 300, 720, 1140.
        let slides = [
            ir("a", false, None, false),
            ir("b", false, Some(2000.0), false),
            ir("hidden", true, Some(2000.0), false),
            ir("c", false, Some(2000.0), false),
            ir("d", false, Some(2000.0), false),
            ir("e", false, Some(0.0), false),
        ];
        let transitions = transitions_of(&slides).unwrap();
        assert_eq!(
            transitions,
            [("b".into(), 2000), ("c".into(), 2000), ("d".into(), 2000)]
        );
        let spans = proposed_spans(&transitions, 60);
        let starts: Vec<(u32, u32)> = ["b", "c", "d"]
            .iter()
            .map(|s| (spans[*s].start, spans[*s].frames))
            .collect();
        assert_eq!(starts, [(300, 120), (720, 120), (1140, 120)]);
        assert!(transitions_of(&[ir("a", false, None, true)]).is_err());
    }

    #[test]
    fn a_row_reads_the_frame_at_its_time_on_the_span() {
        let span = Span {
            start: 300,
            frames: 120,
        };
        assert_eq!(span.frame(250), (330, 0.0));
        assert_eq!(span.frame(500), (360, 0.0));
        assert_eq!(span.frame(750), (390, 0.0));
        // 1.5 s at 60 fps: 90 frames; 250 ‰ is 22.5 frames, read at 23, 5.56 ‰ late.
        let span = Span {
            start: 0,
            frames: 90,
        };
        assert_eq!(span.frame(250), (23, quantise(500.0 / 90.0)));
    }

    #[test]
    fn a_span_is_held_before_moving_inside_and_held_after() {
        // A movie of 60 frames: the outgoing slide (grey 10) to frame 20, moving to frame 40, the incoming (grey 200) after.
        let grey = |v: u8| Rgba::new(4, 4, [v, v, v, 255].repeat(16)).unwrap();
        let movie = |n: u32| match n {
            0..=20 => grey(10),
            21..=39 => grey(10 + ((n - 20) * 9) as u8),
            _ => grey(200),
        };
        let frames_of = |span: &Span| -> BTreeMap<u32, Rgba> {
            span_checks(span).iter().map(|n| (*n, movie(*n))).collect()
        };
        let right = Span {
            start: 20,
            frames: 20,
        };
        assert!(span_problems(&right, &frames_of(&right), 0).is_empty());
        // A span placed late moves before it starts; one placed early is not held after it.
        let late = Span {
            start: 30,
            frames: 20,
        };
        assert_eq!(span_problems(&late, &frames_of(&late), 0).len(), 1);
        let early = Span {
            start: 8,
            frames: 20,
        };
        let problems = span_problems(&early, &frames_of(&early), 0);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].ends_with("no transition there"));
        assert!(problems[1].ends_with("the incoming slide is not held there"));
        // Where the movie has no frames.
        assert_eq!(
            span_problems(&right, &BTreeMap::new(), 0),
            ["the movie has no frames there"]
        );
    }

    #[test]
    fn a_row_is_held_to_the_ceiling_its_ends_reach() {
        let ceiling = Ceiling::between((0.94, 0.5, 2.0), (0.90, 0.5, 3.0), 250);
        assert_eq!(
            ceiling,
            Ceiling {
                ssim: 0.93,
                delta_e: 2.25
            }
        );
        let row = |ssim: f64, delta_e: f64, ceiling: Option<Ceiling>| {
            MovieScore::new(
                (330, 0.0),
                (ssim, 0.5, delta_e),
                ceiling,
                Sha256::of(b"frame"),
            )
        };
        // Within the bounds of the ceiling, whatever the absolute class.
        assert_eq!(row(0.905, 3.2, Some(ceiling)).failure(), None);
        assert_eq!(row(0.905, 3.2, Some(ceiling)).class, "motion-close");
        // Short by more SSIM, or over by more ΔE.
        assert!(row(0.895, 2.25, Some(ceiling)).failure().is_some());
        assert!(row(0.93, 3.3, Some(ceiling)).failure().is_some());
        // No ceiling (the transition's ends were not captured): nothing to hold the row to.
        assert_eq!(row(0.5, 30.0, None).failure(), None);
    }

    #[test]
    fn classes_take_the_static_bounds_and_the_worse_class_wins() {
        assert_eq!(movie_class(0.98, 0.7, 2.0), MotionOutcome::MotionMatch);
        assert_eq!(movie_class(0.98, 0.5, 2.0), MotionOutcome::MotionClose);
        assert_eq!(movie_class(0.95, 0.9, 2.0), MotionOutcome::MotionClose);
        assert_eq!(movie_class(0.95, 0.9, 9.0), MotionOutcome::MotionMismatch);
        assert_eq!(movie_class(0.85, 0.9, 1.0), MotionOutcome::MotionMismatch);
        assert_eq!(
            worse(MotionOutcome::MotionMatch, MotionOutcome::MotionClose),
            MotionOutcome::MotionClose
        );
        assert_eq!(
            worse(MotionOutcome::MotionMismatch, MotionOutcome::MotionClose),
            MotionOutcome::MotionMismatch
        );
    }

    #[test]
    fn records_round_trip_canonically() {
        let dir = std::env::temp_dir().join(format!("slidock-movies-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(RECORD_FILE);
        let mut records = MovieRecords::default();
        records.movies.insert(
            "probe".into(),
            MovieOracle {
                exporter: KeynoteExporter {
                    bundle_id: "com.apple.iWork.Keynote".into(),
                    version: "14.5".into(),
                    build: "7045.0.17".into(),
                },
                file: "probe.m4v".into(),
                sha256: Sha256::of(b"movie"),
                fps: 60,
                width: 1024,
                height: 768,
                transitions: BTreeMap::from([(
                    "s2".into(),
                    Span {
                        start: 300,
                        frames: 120,
                    },
                )]),
                frame_hashes: BTreeMap::new(),
            },
        );
        save_records(&path, &records).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.ends_with("}\n") && text.contains("\n  \"movies\": {"));
        assert_eq!(load_records(&path).unwrap(), records);
        assert_eq!(
            load_records(&dir.join("absent.json")).unwrap(),
            MovieRecords::default()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
