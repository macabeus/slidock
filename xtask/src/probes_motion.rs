//! `cargo xtask probes-motion`: writes the spec-authored motion probe decks (`probes/out/motion-probe.pptx` and
//! `.odp`), their expectations (`motion-probe.expect.json`) and one manifest per deck. The generator is
//! deterministic (fixed zip timestamps and entry order), so regenerating is a no-op; `--check` compares instead of
//! writing. See `probes/motion/README.md`.

#[path = "probes_catalog.rs"]
mod probes_catalog;
#[path = "probes_json.rs"]
mod probes_json;
#[path = "probes_libreoffice.rs"]
mod probes_libreoffice;
#[path = "probes_odp.rs"]
mod probes_odp;
#[path = "probes_pptx.rs"]
mod probes_pptx;
#[path = "probes_zip.rs"]
mod probes_zip;

use std::path::{Path, PathBuf};

use probes_catalog::{
    After, Class, Fmt, Interval, OAdv, Repeat, Slide, Step, Trans, Unit, Vocab, layout, offsets,
};
use probes_json::Json;

const USAGE: &str = "usage: cargo xtask probes-motion [--check] [--check-catalog [<dir>]]

  Writes probes/out/motion-probe.{pptx,odp,expect.json} and motion-probe.{pptx,odp}.manifest.json.
  --check            compare with the committed files instead of writing (fails on any difference)
  --check-catalog    compare the preset, clip, transition and filter tables with LibreOffice's
                     sd/xml/effects.xml, transitions.xml, transitions-ogl.xml and
                     oox/source/ppt/pptfilterhelpers.cxx in <dir> (default
                     $SLIDOCK_PRIVATE/tools/libreoffice-bbb07447, files named with '/' as '_')";

/// The LibreOffice commit (tag libreoffice-25.2.3.2, the preset tables' pin) the catalog lists were taken from, and
/// the files' sha256.
const LIBREOFFICE_COMMIT: &str = crate::libreoffice_presets::COMMIT;
const LIBREOFFICE_FILES: &[(&str, &str)] = &[
    (
        "sd/xml/effects.xml",
        "ce150ba36c14559abc48aa2d61561ffef9386e8acd1a419037cfa04c6d9f6103",
    ),
    (
        "sd/xml/transitions.xml",
        "6385fb101f409fdeb2625440d6cce8edd604c604dd28eb628b21fea6c0adf67d",
    ),
    (
        "sd/xml/transitions-ogl.xml",
        "3c2dff3766ccfd6816ced260f94ebd44cf49191435424140d04831adc73630f1",
    ),
    (
        "oox/source/ppt/pptfilterhelpers.cxx",
        "495a68c8243c803d445e7db079cdebb156f69de1586872ad71687dbce77d75da",
    ),
];

const CLIP: &[u8] = include_bytes!("../../probes/motion/media/clip.mp4");
const POSTER: &[u8] = include_bytes!("../../probes/motion/media/poster.png");

pub fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        None => write_all(&repo_root()),
        Some("--check") => check_all(&repo_root()),
        Some("--check-catalog") => {
            let dir = match args.get(1) {
                Some(dir) => PathBuf::from(dir),
                None => private_dir().join("tools/libreoffice-bbb07447"),
            };
            check_catalog(&dir)
        }
        _ => Err(USAGE.to_owned()),
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn private_dir() -> PathBuf {
    match std::env::var_os("SLIDOCK_PRIVATE") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".slidock"),
    }
}

/// Every generated file: (path under `probes/out/`, bytes).
pub fn generate() -> Vec<(String, Vec<u8>)> {
    let slides = probes_catalog::deck();
    let pptx = probes_zip::write_zip(&probes_pptx::write(&slides, CLIP, POSTER));
    let odp = probes_zip::write_zip(&probes_odp::write(&slides, CLIP, POSTER));
    let expect = expectations(&slides).pretty_until(6).into_bytes();
    let expect_sha = probes_zip::sha256_hex(&expect);
    let pptx_manifest = manifest(&slides, Fmt::Pptx, &pptx, &expect_sha)
        .pretty()
        .into_bytes();
    let odp_manifest = manifest(&slides, Fmt::Odp, &odp, &expect_sha)
        .pretty()
        .into_bytes();
    vec![
        ("motion-probe.pptx".to_owned(), pptx),
        ("motion-probe.odp".to_owned(), odp),
        ("motion-probe.expect.json".to_owned(), expect),
        ("motion-probe.pptx.manifest.json".to_owned(), pptx_manifest),
        ("motion-probe.odp.manifest.json".to_owned(), odp_manifest),
    ]
}

fn write_all(root: &Path) -> Result<(), String> {
    let out = root.join("probes/out");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    for (name, bytes) in generate() {
        let path = out.join(&name);
        std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        println!(
            "wrote probes/out/{name} ({} bytes, sha256 {})",
            bytes.len(),
            probes_zip::sha256_hex(&bytes)
        );
    }
    Ok(())
}

fn check_all(root: &Path) -> Result<(), String> {
    let mut stale = Vec::new();
    for (name, bytes) in generate() {
        let path = root.join("probes/out").join(&name);
        if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
            stale.push(name);
        }
    }
    if stale.is_empty() {
        println!("probes/out motion files are up to date");
        Ok(())
    } else {
        Err(format!(
            "stale (run cargo xtask probes-motion): {}",
            stale.join(", ")
        ))
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Expectations.

fn format_name(fmt: Fmt) -> &'static str {
    match fmt {
        Fmt::Pptx => "motion-probe.pptx",
        Fmt::Odp => "motion-probe.odp",
    }
}

fn diagnostic(code: &str, name: &str) -> Json {
    Json::obj().with("code", code).with("name", name)
}

fn effect_approximated(fmt: Fmt) -> Json {
    match fmt {
        Fmt::Pptx => diagnostic("PX2413", "pptx-effect-approximated"),
        Fmt::Odp => diagnostic("OD9413", "odp-effect-approximated"),
    }
}

fn timing_approximated(fmt: Fmt) -> Json {
    match fmt {
        Fmt::Pptx => diagnostic("PX2411", "timing-approximated"),
        Fmt::Odp => diagnostic("OD9411", "odp-timing-approximated"),
    }
}

/// `slidock_ir::units::Ms::MAX` (one hour): a longer duration, delay or advance time clamps to it.
const MS_MAX: u32 = 3_600_000;
/// The most repeats of a step (`REPEAT_COUNT`, the IR's effect limit).
const REPEAT_MAX: u32 = 1_000;

/// How a frontend reads a native timing value: as written, clamped to its limit, or invalid (NaN, negative,
/// unparsable), which fails closed to the declared default.
enum TimeRead {
    Ms(u32),
    Clamped(String),
    Invalid(String),
}

impl TimeRead {
    /// A value the generator wrote from its own number of milliseconds.
    fn ms(ms: u32) -> TimeRead {
        if ms > MS_MAX {
            TimeRead::Clamped(ms.to_string())
        } else {
            TimeRead::Ms(ms)
        }
    }

    /// A value written as given: PPTX milliseconds, or an ODF clock value (`0.5s`) or ISO duration.
    fn raw(text: &str, fmt: Fmt) -> TimeRead {
        let ms = match fmt {
            Fmt::Pptx => text.parse::<f64>().ok(),
            Fmt::Odp => text
                .strip_suffix('s')
                .and_then(|n| n.parse::<f64>().ok())
                .map(|secs| secs * 1000.0),
        };
        match ms {
            Some(ms) if ms.is_finite() && ms >= 0.0 && ms > f64::from(MS_MAX) => {
                TimeRead::Clamped(text.to_owned())
            }
            Some(ms) if ms.is_finite() && ms >= 0.0 => TimeRead::Ms(ms.round() as u32),
            _ => TimeRead::Invalid(text.to_owned()),
        }
    }

    fn value(&self) -> Json {
        match self {
            TimeRead::Ms(ms) => Json::from(*ms),
            TimeRead::Clamped(_) => Json::from(MS_MAX),
            TimeRead::Invalid(_) => Json::Null,
        }
    }

    fn millis(&self) -> Option<u32> {
        match self {
            TimeRead::Ms(ms) => Some(*ms),
            TimeRead::Clamped(_) => Some(MS_MAX),
            TimeRead::Invalid(_) => None,
        }
    }

    /// The `hostileTiming` entry: what the file says and how it reads.
    fn note(&self) -> Option<Json> {
        let (native, read, limit) = match self {
            TimeRead::Ms(_) => return None,
            TimeRead::Clamped(native) => {
                (native, "clamped", Json::from("slidock_ir::units::Ms::MAX"))
            }
            TimeRead::Invalid(native) => (native, "invalid", Json::Null),
        };
        Some(
            Json::obj()
                .with("native", native.as_str())
                .with("read", read)
                .with("limit", limit),
        )
    }
}

fn transition_expect(trans: &Trans, fmt: Fmt) -> Option<Json> {
    // an invalid advance time fails closed to advancing on click only
    let mut hostile = Json::obj();
    let (native, duration, advance) = match fmt {
        Fmt::Pptx => {
            let t = trans.pptx.as_ref()?;
            let after = match (t.adv_tm_raw, t.adv_tm) {
                (Some(raw), _) => Some(TimeRead::raw(raw, fmt)),
                (None, Some(ms)) => Some(TimeRead::ms(ms)),
                (None, None) => None,
            };
            let on_click =
                t.adv_click.unwrap_or(true) || matches!(after, Some(TimeRead::Invalid(_)));
            if let Some(note) = after.as_ref().and_then(TimeRead::note) {
                hostile.push("afterMs", note);
            }
            let advance = Json::obj()
                .with("onClick", on_click)
                .with("afterMs", after.map(|a| a.value()).unwrap_or(Json::Null));
            (t.native(), t.duration_ms(), advance)
        }
        Fmt::Odp => {
            let t = trans.odp.as_ref()?;
            let (on_click, after) = match (t.advance, t.duration_raw) {
                (OAdv::Manual | OAdv::SemiAutomatic, _) => (true, Json::Null),
                (OAdv::Automatic(_), Some(raw)) => {
                    let read = iso_read(raw);
                    if let Some(note) = read.note() {
                        hostile.push("afterMs", note);
                    }
                    (matches!(read, TimeRead::Invalid(_)), read.value())
                }
                (OAdv::Automatic(ms), None) => (false, Json::from(ms.unwrap_or(0))),
            };
            (
                t.native(),
                t.duration_ms(),
                Json::obj().with("onClick", on_click).with("afterMs", after),
            )
        }
    };
    let hostile = match hostile {
        Json::Obj(fields) if fields.is_empty() => None,
        other => Some(other),
    };
    let mut out = Json::obj().with("native", native);
    match &trans.vocab {
        Some((name, params, approximate)) => {
            let params = match (fmt, &trans.odp_params) {
                (Fmt::Odp, Some(odp)) => odp.clone(),
                _ => params.clone(),
            };
            out.push("effect", *name);
            out.push("params", vocab_tokens(params));
            out.push("approximate", *approximate);
            out.push(
                "diagnostic",
                if hostile.is_some() {
                    timing_approximated(fmt)
                } else if *approximate {
                    effect_approximated(fmt)
                } else {
                    Json::Null
                },
            );
        }
        None => {
            out.push("effect", Json::Null);
            out.push("params", Json::Null);
            out.push("approximate", false);
            out.push(
                "diagnostic",
                match fmt {
                    Fmt::Pptx => diagnostic("PX2402", "transition-unmapped"),
                    Fmt::Odp => diagnostic("OD9402", "transition-dropped"),
                },
            );
        }
    }
    out.push("fallback", trans.fallback);
    out.push("durationMs", duration);
    out.push("advance", advance);
    out.push("hostileTiming", hostile.unwrap_or(Json::Null));
    Some(out)
}

/// An ODF `presentation:duration` (`PT00H00M03S`, `-PT5S`).
fn iso_read(text: &str) -> TimeRead {
    let Some(body) = text.strip_prefix("PT") else {
        return TimeRead::Invalid(text.to_owned());
    };
    let mut ms = 0.0;
    let mut number = String::new();
    for c in body.chars() {
        let scale = match c {
            'H' => 3_600_000.0,
            'M' => 60_000.0,
            'S' => 1_000.0,
            _ => {
                number.push(c);
                continue;
            }
        };
        match number.parse::<f64>() {
            Ok(n) if n.is_finite() && n >= 0.0 => ms += n * scale,
            _ => return TimeRead::Invalid(text.to_owned()),
        }
        number.clear();
    }
    if !number.is_empty() {
        TimeRead::Invalid(text.to_owned())
    } else if ms > f64::from(MS_MAX) {
        TimeRead::Clamped(text.to_owned())
    } else {
        TimeRead::Ms(ms.round() as u32)
    }
}

fn step_native(step: &Step, fmt: Fmt) -> String {
    let effect = &step.effect;
    match fmt {
        Fmt::Pptx => {
            let id = effect.pptx_id.unwrap_or(0);
            let sub = if effect.pptx_sub != 0 {
                format!("/{}", effect.pptx_sub)
            } else {
                String::new()
            };
            format!("pptx:{}.{id}{sub}", effect.class.pptx())
        }
        Fmt::Odp => {
            let sub = effect
                .sub
                .as_ref()
                .map(|s| format!("/{s}"))
                .unwrap_or_default();
            format!("odp:{}{sub}", effect.ooo.as_deref().unwrap_or(""))
        }
    }
}

fn step_expect(
    slide: &Slide,
    index: usize,
    offset: Option<u32>,
    fmt: Fmt,
    page: usize,
    spids: &[Option<u32>],
) -> Json {
    let step = &slide.steps[index];
    let shape = &slide.shapes[step.shape];
    let target = match fmt {
        Fmt::Pptx => Json::obj().with("spid", spids[step.shape].unwrap_or(0)),
        Fmt::Odp => Json::obj().with("xmlId", probes_odp::shape_id(page, step.shape)),
    };
    let mut out = Json::obj()
        .with("shape", shape.label())
        .with("target", target)
        .with(
            "paragraphs",
            step.paragraphs
                .map(|(a, b)| Json::from(vec![a, b]))
                .unwrap_or(Json::Null),
        )
        .with("background", step.background)
        .with(
            "trigger",
            if step.interactive {
                "onShapeClick"
            } else {
                step.trigger.vocab()
            },
        )
        .with("kind", step.effect.class.kind())
        .with("native", step_native(step, fmt));
    let class = step.effect.class;
    let mut delivery = match (step.iterate, step.paragraphs) {
        (Some(it), _) => match it.unit {
            Unit::Word => "byWord",
            Unit::Letter => "byChar",
            Unit::Paragraph => "byParagraph",
        },
        (None, Some(_)) => "byParagraph",
        _ => "all",
    };
    let (effect, params, approximate, tier, fallback, diag, clip) = match &step.vocab {
        Vocab::Effect {
            name,
            params,
            approximate,
            delivery: implied,
        } => {
            if let Some(implied) = implied {
                delivery = implied;
            }
            let mut params = params.clone();
            if *name == "path"
                && let Some(path) = step.effect.bhv.iter().find_map(|b| match b {
                    probes_catalog::Bhv::Motion(path) => Some(path),
                    _ => None,
                })
            {
                params.push("d", probes_catalog::path_px(path));
            }
            let fallback = if matches!(*name, "appear" | "typewriter")
                && matches!(class, Class::Entr | Class::Exit)
            {
                "appear"
            } else {
                class.fallback()
            };
            let diag = if *approximate {
                effect_approximated(fmt)
            } else {
                Json::Null
            };
            (
                Json::from(*name),
                params,
                *approximate,
                Json::from(1u32),
                fallback,
                diag,
                None,
            )
        }
        Vocab::Tier2 { clip } => {
            let clip = clip.as_ref().map(|(name, params)| {
                Json::obj()
                    .with("effect", *name)
                    .with("params", vocab_tokens(params.clone()))
            });
            (
                Json::from("behaviors"),
                Json::Null,
                false,
                Json::from(2u32),
                class.fallback(),
                Json::Null,
                clip,
            )
        }
        Vocab::Fallback => {
            let diag = match fmt {
                Fmt::Pptx => diagnostic("PX2410", "preset-unmapped"),
                Fmt::Odp => diagnostic("OD9410", "odp-preset-unmapped"),
            };
            (
                Json::Null,
                Json::Null,
                false,
                Json::Null,
                class.fallback(),
                diag,
                None,
            )
        }
        Vocab::Dropped => {
            let diag = match fmt {
                Fmt::Pptx => diagnostic("PX2403", "interactive-sequence-dropped"),
                Fmt::Odp => diagnostic("OD9404", "odp-interactive-sequence-dropped"),
            };
            (Json::Null, Json::Null, false, Json::Null, "", diag, None)
        }
        Vocab::Bounded => (
            Json::Null,
            Json::Null,
            false,
            Json::Null,
            "",
            timing_approximated(fmt),
            None,
        ),
    };
    let dropped = matches!(step.vocab, Vocab::Dropped | Vocab::Bounded);
    let mut hostile = Json::obj();
    let dur = match step.raw.dur(fmt) {
        Some(raw) => TimeRead::raw(raw, fmt),
        None => TimeRead::ms(step.dur_ms),
    };
    let delay = match step.raw.delay(fmt) {
        Some(raw) => TimeRead::raw(raw, fmt),
        None => TimeRead::ms(step.delay_ms),
    };
    if let Some(note) = dur.note() {
        hostile.push("durationMs", note);
    }
    if let Some(note) = delay.note() {
        hostile.push("delayMs", note);
    }
    let repeat_count = match step.repeat {
        Some(Repeat::Count(count)) => Some(count / 1000),
        _ => None,
    };
    if let Some(count) = repeat_count.filter(|&c| c > REPEAT_MAX) {
        hostile.push(
            "repeat",
            Json::obj()
                .with("native", count.to_string())
                .with("read", "clamped")
                .with("limit", "slidock_ir::effects::limits::REPEAT_COUNT"),
        );
    }
    let hostile = match hostile {
        Json::Obj(fields) if fields.is_empty() => None,
        other => Some(other),
    };
    let diag = if diag == Json::Null && hostile.is_some() {
        timing_approximated(fmt)
    } else {
        diag
    };
    out.push("effect", effect);
    out.push("params", vocab_tokens(params));
    out.push("approximate", approximate);
    out.push("tier", tier);
    if let Some(clip) = clip {
        out.push("clip", clip);
    }
    out.push(
        "fallback",
        if dropped {
            Json::Null
        } else {
            Json::from(fallback)
        },
    );
    out.push("dropped", dropped);
    out.push("diagnostic", diag);
    out.push("delivery", delivery);
    let stagger = step
        .iterate
        .filter(|it| it.unit != Unit::Paragraph || it.interval != Interval::Ms(0))
        .map(|it| {
            let order = if it.backwards { "backward" } else { "forward" };
            match it.interval {
                Interval::Ms(ms) => Json::obj().with("perUnitMs", ms).with("order", order),
                Interval::Pct(pct) => Json::obj()
                    .with(
                        "perUnitPct",
                        Json::Num(trim_decimal(f64::from(pct) / 1000.0)),
                    )
                    .with("order", order),
            }
        });
    out.push("stagger", stagger.unwrap_or(Json::Null));
    let conditional = step.begin_on.is_some();
    out.push(
        "offsetMs",
        match (offset, delay.millis(), conditional) {
            (Some(begin), Some(delay), false) => Json::from(begin.saturating_add(delay)),
            _ => Json::Null,
        },
    );
    out.push(
        "delayMs",
        if conditional {
            Json::Null
        } else {
            delay.value()
        },
    );
    // an appear is a switch: 0 ms in both formats (the 1 ms PPTX set, the 0.001 s ODF set)
    let switch = (matches!(&step.vocab, Vocab::Effect { name: "appear", .. })
        && !step.shows_while())
        || step.is_switch();
    // an ODF play command lasts its medium's own length (the probe clip's `mvhd`), whatever its own `smil:dur`
    let command = fmt == Fmt::Odp
        && step
            .effect
            .bhv
            .iter()
            .any(|b| matches!(b, probes_catalog::Bhv::Command(_, "play")));
    out.push(
        "durationMs",
        if switch {
            Json::from(0u32)
        } else if command {
            Json::from(movie_length_ms(CLIP).unwrap_or(0))
        } else {
            dur.value()
        },
    );
    let easing = match (step.accel, step.decel) {
        (None, None) => Json::Null,
        (accel, decel) => Json::obj()
            .with(
                "accel",
                Json::Num(trim_decimal(f64::from(accel.unwrap_or(0)) / 100_000.0)),
            )
            .with(
                "decel",
                Json::Num(trim_decimal(f64::from(decel.unwrap_or(0)) / 100_000.0)),
            ),
    };
    out.push("easing", easing);
    let repeat = match step.repeat {
        None => Json::Null,
        Some(Repeat::Count(count)) => Json::obj().with("count", (count / 1000).min(REPEAT_MAX)),
        Some(Repeat::UntilEndOfSlide) => Json::from("untilEndOfSlide"),
        Some(Repeat::UntilNextClick) => Json::from("untilNextClick"),
    };
    out.push("repeat", repeat);
    out.push("autoReverse", step.auto_rev);
    out.push(
        "after",
        match step.after {
            // Flash Once hides its target when its switch ends.
            None if step.shows_while() => Json::from("hide"),
            None => Json::from("keep"),
            Some(After::Hide) => Json::from("hide"),
            Some(After::Dim(color)) => Json::obj().with("dim", format!("#{color}")),
        },
    );
    out.push("hostileTiming", hostile.unwrap_or(Json::Null));
    out
}

/// Params with an orientation or an axis in the vocabulary's spelling, OOXML's tokens (`horz`, `vert`).
fn vocab_tokens(params: Json) -> Json {
    match params {
        Json::Obj(fields) => Json::Obj(
            fields
                .into_iter()
                .map(|(key, value)| {
                    let value = match (key.as_str(), value) {
                        ("orient" | "axis", Json::Str(word)) => Json::Str(
                            match word.as_str() {
                                "horizontal" => "horz",
                                "vertical" => "vert",
                                other => other,
                            }
                            .to_owned(),
                        ),
                        (_, value) => value,
                    };
                    (key, value)
                })
                .collect(),
        ),
        other => other,
    }
}

/// A movie's length in milliseconds, from its version-0 `mvhd` box (timescale, duration).
fn movie_length_ms(movie: &[u8]) -> Option<u32> {
    let at = movie.windows(4).position(|w| w == b"mvhd")?;
    let field = |offset: usize| -> Option<u32> {
        let bytes = movie.get(at + offset..at + offset + 4)?;
        Some(u32::from_be_bytes(bytes.try_into().ok()?))
    };
    if *movie.get(at + 4)? != 0 {
        return None;
    }
    let (timescale, duration) = (field(16)?, field(20)?);
    (timescale != 0).then(|| (u64::from(duration) * 1000 / u64::from(timescale)) as u32)
}

fn trim_decimal(value: f64) -> String {
    let text = format!("{value:.4}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn deck_expect(slides: &[Slide], fmt: Fmt) -> Json {
    let mut list = Vec::new();
    let mut page = 0;
    for slide in slides.iter().filter(|s| s.formats.has(fmt)) {
        page += 1;
        let spids = probes_pptx::spids(slide);
        let groups = layout(&slide.steps, fmt);
        let offs = offsets(&groups, slide.steps.len());
        let steps: Vec<Json> = (0..slide.steps.len())
            .filter(|&i| slide.steps[i].formats.has(fmt))
            .map(|i| step_expect(slide, i, offs[i], fmt, page, &spids))
            .collect();
        let shapes: Vec<Json> = (0..slide.shapes.len())
            .filter(|&i| slide.shapes[i].formats.has(fmt))
            .map(|i| {
                let target = match fmt {
                    Fmt::Pptx => Json::obj().with("spid", spids[i].unwrap_or(0)),
                    Fmt::Odp => Json::obj().with("xmlId", probes_odp::shape_id(page, i)),
                };
                Json::obj()
                    .with("label", slide.shapes[i].label())
                    .with("target", target)
            })
            .collect();
        let click_groups = groups.iter().filter(|g| !g.auto).count();
        let entry = Json::obj()
            .with("index", page)
            .with("key", &slide.key)
            .with("title", &slide.title)
            .with(
                "transition",
                slide
                    .transition
                    .as_ref()
                    .and_then(|t| transition_expect(t, fmt))
                    .unwrap_or(Json::Null),
            )
            .with("clickGroups", click_groups)
            .with("autoGroup", groups.first().is_some_and(|g| g.auto))
            .with("shapes", Json::Arr(shapes))
            .with("steps", Json::Arr(steps))
            .with("rows", slide.rows.clone());
        list.push(entry);
    }
    Json::obj()
        .with("slides", list.len())
        .with("slideList", Json::Arr(list))
}

/// The catalog rows (`12.1/…` PowerPoint transitions, `12.2/…` build presets, `12.3/…` ODP) every probe must cover,
/// or list as not authorable with a reason.
fn required_rows() -> Vec<String> {
    let mut rows: Vec<String> = [
        "12.1/cut",
        "12.1/fade",
        "12.1/dissolve",
        "12.1/push",
        "12.1/cover",
        "12.1/pull",
        "12.1/wipe",
        "12.1/split",
        "12.1/circle-diamond-plus",
        "12.1/zoom",
        "12.1/newsflash",
        "12.1/wheel",
        "12.1/wedge",
        "12.1/blinds-checker-randomBar-comb",
        "12.1/strips",
        "12.1/random",
        "12.1/p14-flash",
        "12.1/p14-reveal",
        "12.1/p14-3d",
        "12.1/p14-doors-window",
        "12.1/p14-flythrough",
        "12.1/p14-warp",
        "12.1/p14-fallback",
        "12.1/p15-prstTrans",
        "12.1/p159-morph",
        "12.1/durations",
        "12.1/advance",
    ]
    .iter()
    .map(|r| (*r).to_owned())
    .collect();
    let entr_exit: Vec<u32> = (1..=31)
        .chain([
            34, 35, 37, 38, 39, 40, 41, 42, 43, 45, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 58,
        ])
        .collect();
    for class in ["entr", "exit"] {
        rows.extend(
            entr_exit
                .iter()
                .filter(|&&id| !(32..=33).contains(&id))
                .map(|id| format!("12.2/{class}.{id}")),
        );
    }
    let emph: &[u32] = &[
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 14, 15, 16, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 30,
        31, 32, 33, 34, 35, 36,
    ];
    rows.extend(emph.iter().map(|id| format!("12.2/emph.{id}")));
    rows.push("12.2/path".into());
    rows.extend((1..=64).map(|id| format!("12.2/path.{id}")));
    for sub in [
        1, 2, 3, 4, 5, 6, 8, 9, 10, 12, 16, 32, 36, 528, 272, 288, 21, 26, 37, 42,
    ] {
        rows.push(format!("12.2/subtype.{sub}"));
    }
    rows.push("12.2/tier2".into());
    for filter in [
        "wipe",
        "wheel",
        "strips",
        "barn",
        "randombar",
        "checkerboard",
        "plus",
        "diamond",
        "circle",
        "box",
        "wedge",
        "blinds",
        "fade",
        "dissolve",
        "slide",
    ] {
        rows.push(format!("12.2/tier2-filter.{filter}"));
    }
    for ty in [
        "barWipe",
        "pushWipe",
        "slideWipe",
        "slideWipe-reverse",
        "barnDoorWipe",
        "irisWipe",
        "ellipseWipe",
        "fourBoxWipe",
        "pinWheelWipe",
        "fanWipe",
        "clockWipe",
        "blindsWipe",
        "checkerBoardWipe",
        "randomBarWipe",
        "waterfallWipe",
        "dissolve",
        "fade",
        "zoom",
        "random",
        "snakeWipe",
        "spiralWipe",
        "zigZagWipe",
        "bowTieWipe",
        "veeWipe",
        "starWipe",
        "heart",
        "miscShapeWipe",
        "fadeColor",
        "duration-filter",
        "duration-speed",
        "advance",
        "node-type",
        "preset-class",
        "preset-id",
        "preset-sub-type",
        "iterate",
        "paragraph-target",
        "custom-tier2",
        "media-call",
    ] {
        rows.push(format!("12.3/{ty}"));
    }
    rows.push("12.4/keynote".into());
    rows
}

/// Rows no generator here can author, with the reason.
fn not_authorable(row: &str) -> Option<&'static str> {
    match row {
        "12.4/keynote" => Some(
            "Keynote builds and transitions need Keynote itself (AppleScript and the Build inspector); the attended \
             catalog step is probes/keynote/ (make-transition-catalog.applescript and RUNBOOK.md)",
        ),
        _ => None,
    }
}

fn coverage(slides: &[Slide]) -> Json {
    let mut out = Vec::new();
    for row in required_rows() {
        let keys = |fmt: Fmt| -> Vec<String> {
            slides
                .iter()
                .filter(|s| s.formats.has(fmt) && s.rows.contains(&row))
                .map(|s| s.key.clone())
                .collect()
        };
        let (pptx, odp) = (keys(Fmt::Pptx), keys(Fmt::Odp));
        let mut entry = Json::obj().with("row", row.as_str());
        if pptx.is_empty() && odp.is_empty() {
            entry.push("notAuthorable", not_authorable(&row).unwrap_or("MISSING"));
        } else {
            entry.push("pptx", pptx);
            entry.push("odp", odp);
        }
        out.push(entry);
    }
    Json::Arr(out)
}

fn expectations(slides: &[Slide]) -> Json {
    let conventions = Json::obj()
        .with("dir", "the direction the content moves, in OOXML tokens l r u d lu ru ld rd (docs/format.md §5.2): fly-in from the left is dir=r; an exit names where the content goes; a wipe names the direction its edge travels")
        .with("ecmaVsLibreOffice", "PPTX push/cover/pull dir follows ECMA-376 (the slide moves that way); Keynote's own PPTX export confirms it (its default Push, Move In and Reveal move the content left in its movie export and are written dir=l). LibreOffice's PPTX import maps l/r to SMIL fromLeft/fromRight, the opposite horizontal sense; the ODP deck uses SMIL's own meaning (pushWipe fromRight moves left), as LibreOffice's transitions.xml names it (push-left = fromRight)")
        .with("clips", "one rule maps a clip to the vocabulary everywhere (presets, tier 2, transitions): the SMIL filter (type, subtype, direction, mode) decides, never a preset sub-type label. A PPTX animEffect filter reads as LibreOffice reads it (pptfilterhelpers.cxx): wipe(X) starts at side X (wipe(down) = barWipe topToBottom reverse, the edge moves up), strips(downLeft) = waterfallWipe horizontalRight. blinds/randomBars/split orient is the SMIL subtype, which equals OOXML dir=horz|vert and PowerPoint's filter word; LibreOffice's effects.xml sub-type labels name the opposite: venetian-blinds and random-bars sub-type vertical play blindsWipe/randomBarWipe horizontal, so pptx entr.3/10 (LibreOffice: vertical) is blinds orient=horizontal. strips corner is the corner the bands travel toward: waterfallWipe horizontalRight = ld, horizontalRight reverse = ru, horizontalLeft = rd, horizontalLeft reverse = lu; diagonal-squares left-to-bottom plays horizontalLeft (rd) as an entrance and horizontalLeft reverse (lu) as an exit. A slide clip (slideWipe, peek) moves the content away from its from-side as it enters and toward it as it leaves (smil:mode=out); other clips name their edge whatever the mode, and a leaving split or iris (smil:mode=out, transition=out) plays the entrance its filter names backwards, as LibreOffice's engine plays it (it inverts an iris's or a barn door's sweep in mode out) and ECMA-376 reads transition=out: ooo-exit-box in (irisWipe rectangle reverse, mode out) and pptx exit.4/16 (box(in), transition out) open a hole from the centre, iris inOut=in. Each preset in the ODP deck carries LibreOffice's own clip; its other behaviours are our own")
        .with("zoom", "LibreOffice reads PPTX zoom dir=in and dir=out both as SMIL zoom default; the ODP deck writes zoom default for in and zoom default reverse for out (zoom{inOut})")
        .with("effect", "the vocabulary's names (docs/format.md §5.2); null means the vocabulary has no entry and the step or transition plays `fallback`, reported with `diagnostic`")
        .with("params", "the params the spec fixes; a frontend may add params at their defaults. Value tokens: orient horz|vert, inOut in|out, shape circle|diamond|box|plus, corner lu|ru|ld|rd, through none|black|white|color, by object|word|char, distance offSlide|slight, origin object|slideCenter, axis horz|vert|both, scale [x, y], path d in slide px relative to the element (1280x720 slide). An exit's params name the entrance it reverses: exit zoom out (shrinks to nothing) is inOut=in, exit box in (a hole opening from the centre) is iris inOut=in, exit split horizontal-in (a gap opening from the centre line) is split inOut=in. A named param is what the source app animates, never a sub-type's word: axis is the dimension LibreOffice's effects.xml animates (swivel vertical turns the width: axis horz)")
        .with("approximate", "true where the vocabulary only approximates the preset or transition (the vocabulary marks it, or a param it needs has no vocabulary slot: stretch from a side, oval iris, magnify, spiral-in/out's path, LibreOffice's turning Float). A preset's approximate entry yields to its own behaviours, which both decks carry: the step expects tier 2, and the approximation (with effect-approximated) only where the behaviours compile to nothing in tier 2 (a colour change alone). An approximate transition plays the approximation and reports effect-approximated")
        .with("tier", "1: a vocabulary effect from the preset; 2: behaviors compiled from the primitives (clip: what the step's clip plays by the clips rule)")
        .with("native", "provenance id: pptx:<class>.<presetID>[/<presetSubtype>] (subtype left out when 0), pptx:<transition element> (p15: pptx:prstTrans/<prst>), odp:<preset-id>[/<preset-sub-type>], odp:<smil:type>[/<smil:subtype>][/reverse], odp:legacy/<transition-style>")
        .with("dropped", "true: the step is not in the IR (an interactive sequence, or hostile timing: a start condition that names its own node or closes a cycle, nesting past the timing depth limit); no fallback plays (fallback null) and the element shows fully built")
        .with("offsetMs", "absolute start inside the click group (time slot begin + the effect's own delay); null where the start is a condition on another node or the delay is invalid")
        .with("durationMs", "one iteration, repeat and auto-reverse excluded; an appear is a switch: 0 ms in both formats (the 1 ms PPTX set and the 0.001 s ODF set; colored lettering's colour pass is the approximation), except Flash Once, whose switch lasts the step and does not hold (after: hide); a behaviour that reverses on its own runs over half the step and back; an ODF play command lasts its medium's own length (the movie's mvhd) whatever its own smil:dur (LibreOffice writes 0.001 s on every command), as PowerPoint writes the clip's length on its playFrom node, while stop and pause are instants (PowerPoint's 1 ms, LibreOffice's 0.001 s); null where the native duration is invalid")
        .with("hostileTiming", "null, or per field (durationMs, delayMs, repeat, afterMs) the value as written and how it reads: clamped to the named limit (slidock_ir::units::Ms::MAX = 3600000 ms; REPEAT_COUNT = 1000, the IR's effect limit), the field holding the clamped value; or invalid (NaN, negative, unparsable), the field null: the step plays with the frontend's declared default duration and delay, and an invalid advance time advances on click only. Either reports animation-timing-approximated")
        .with("crossFormat", "a slide key shared by both decks agrees on transition effect, params, approximate and advance, and on each step's effect, params, approximate, tier, clip, trigger, kind, delivery, stagger, paragraphs, offset, delay, duration, repeat, autoReverse, after, dropped and hostileTiming read, except: transition params.soft (PowerPoint's wipe is feathered, ODF's barWipe is not), the transition durationMs of tm-speed-* (spd and transition-speed scales differ), and the diagnostic codes (PX vs OD)")
        .with("diagnosticCodes", "the registered codes (slidock_ir::diagnostics::registry): PX2402 transition-unmapped and OD9402 transition-dropped (OD9402 only for what has no native mapping); PX2403/OD9404 an effect of an interactive sequence, left out; PX2410/OD9410 preset-unmapped (a preset with no entry, a behaviour that can't compile; a behaviour's own reversal plays, folded into its track); PX2411/OD9411 timing-approximated (dropped, clamped or invalid timing); PX2413/OD9413 effect-approximated (an approximation plays). Every fallback and every approximation is reported exactly once. Frontends compare codes; names are informational");
    Json::obj()
        .with("format", "slidock-motion-probe-expect@1")
        .with(
            "generator",
            "cargo xtask probes-motion (xtask/src/probes_motion.rs)",
        )
        .with(
            "spec",
            "docs/format.md §5 (the timeline and the effect vocabulary); ECMA-376 Part 1 transitions and timing; ODF 1.3 smil:* and anim:*",
        )
        .with("conventions", conventions)
        .with(
            "decks",
            Json::obj()
                .with(format_name(Fmt::Pptx), deck_expect(slides, Fmt::Pptx))
                .with(format_name(Fmt::Odp), deck_expect(slides, Fmt::Odp)),
        )
        .with("coverage", coverage(slides))
}

/// The deck parts that break their schema on purpose: PPTX slides with malformed timing values (`ST_TLTime` is an
/// unsigned int, so NaN, negative and overflowing values can't validate). The ODP probe's values stay
/// schema-valid (`smil:dur` and `smil:begin` are strings, `-PT5S` is an `xsd:duration`).
fn schema_invalid_parts(slides: &[Slide], fmt: Fmt) -> Vec<String> {
    if fmt == Fmt::Odp {
        return Vec::new();
    }
    slides
        .iter()
        .filter(|s| s.formats.pptx)
        .enumerate()
        .filter(|(_, s)| {
            s.steps
                .iter()
                .any(|st| st.raw.pptx_dur.is_some() || st.raw.pptx_delay.is_some())
                || s.transition
                    .as_ref()
                    .and_then(|t| t.pptx.as_ref())
                    .is_some_and(|t| t.adv_tm_raw.is_some())
        })
        .map(|(i, _)| format!("ppt/slides/slide{}.xml", i + 1))
        .collect()
}

fn manifest(slides: &[Slide], fmt: Fmt, deck: &[u8], expect_sha: &str) -> Json {
    let count = slides.iter().filter(|s| s.formats.has(fmt)).count();
    let shared = slides
        .iter()
        .filter(|s| s.formats.pptx && s.formats.odp)
        .count();
    let inputs = Json::Arr(vec![
        Json::obj()
            .with("file", "probes/motion/media/clip.mp4")
            .with("sha256", probes_zip::sha256_hex(CLIP))
            .with("made", "probes/motion/make-media.sh"),
        Json::obj()
            .with("file", "probes/motion/media/poster.png")
            .with("sha256", probes_zip::sha256_hex(POSTER))
            .with("made", "probes/motion/make-media.sh"),
    ]);
    let catalog = Json::obj()
        .with("libreofficeCommit", LIBREOFFICE_COMMIT)
        .with(
            "files",
            Json::Arr(
                LIBREOFFICE_FILES
                    .iter()
                    .map(|(f, sha)| Json::obj().with("file", *f).with("sha256", *sha))
                    .collect(),
            ),
        );
    Json::obj()
        .with(
            "generator",
            "cargo xtask probes-motion (xtask/src/probes_motion.rs)",
        )
        .with(
            "spec",
            "docs/format.md §5.2 (the effect vocabulary and its direction convention); catalog rows 12.1 PowerPoint transitions, 12.2 PowerPoint build presets, 12.3 ODP",
        )
        .with("inputs", inputs)
        .with("catalog", catalog)
        .with(
            "expectations",
            Json::obj()
                .with("file", "motion-probe.expect.json")
                .with("sha256", expect_sha),
        )
        .with(
            "deck",
            Json::obj()
                .with("file", format_name(fmt))
                .with("sha256", probes_zip::sha256_hex(deck))
                .with("bytes", deck.len())
                .with("slides", count)
                .with("slidesSharedWithOtherDeck", shared)
                .with(
                    "schemaInvalidByDesign",
                    schema_invalid_parts(slides, fmt).join(" "),
                ),
        )
}

// ---------------------------------------------------------------------------------------------------------------
// The catalog check against LibreOffice's files.

fn strip_comments(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        rest = match rest[start..].find("-->") {
            Some(end) => &rest[start + end + 3..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!(" {name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')?;
    Some(&tag[start..start + end])
}

fn tags<'a>(text: &'a str, elements: &[&str]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        let tail = &rest[start..];
        let end = tail.find('>').unwrap_or(tail.len());
        let tag = &tail[..end];
        if elements.iter().any(|element| {
            tag.strip_prefix('<')
                .and_then(|t| t.strip_prefix(element))
                .is_some_and(|t| t.starts_with(char::is_whitespace))
        }) {
            out.push(tag);
        }
        rest = &tail[end..];
    }
    out
}

fn check_catalog(dir: &Path) -> Result<(), String> {
    let read = |name: &str| -> Result<String, String> {
        let path = dir.join(name.replace('/', "_"));
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let expected = LIBREOFFICE_FILES
            .iter()
            .find(|(f, _)| *f == name)
            .map(|(_, s)| *s)
            .unwrap_or("");
        let actual = probes_zip::sha256_hex(&bytes);
        if actual != expected {
            return Err(format!(
                "{name}: sha256 {actual}, expected {expected} (pinned commit {LIBREOFFICE_COMMIT})"
            ));
        }
        Ok(strip_comments(&String::from_utf8_lossy(&bytes)))
    };
    let effects = read("sd/xml/effects.xml")?;
    let mut presets: Vec<(String, String, String)> = Vec::new();
    let mut filters: Vec<(String, String, String, String, bool, bool)> = Vec::new();
    let mut current: Option<(String, String)> = None;
    let mut flash_bulb: Option<String> = None;
    let mut in_flash_bulb = false;
    for tag in tags(
        &effects,
        &["anim:par", "anim:iterate", "anim:transitionFilter"],
    ) {
        if tag.starts_with("<anim:transitionFilter") && in_flash_bulb && flash_bulb.is_none() {
            flash_bulb = attr(tag, "smil:keySplines").map(str::to_owned);
        }
        if let (Some(class), Some(id)) =
            (attr(tag, "pres:preset-class"), attr(tag, "pres:preset-id"))
        {
            in_flash_bulb = id == "ooo-emphasis-flash-bulb";
            let sub = attr(tag, "pres:preset-sub-type").unwrap_or("");
            let row = (class.to_owned(), id.to_owned(), sub.to_owned());
            let first = !presets.contains(&row);
            if first {
                presets.push(row);
            }
            current = first.then(|| (id.to_owned(), sub.to_owned()));
        } else if tag.starts_with("<anim:transitionFilter")
            && let Some((id, sub)) = current.take()
        {
            filters.push((
                id,
                sub,
                attr(tag, "smil:type").unwrap_or("").to_owned(),
                attr(tag, "smil:subtype").unwrap_or("").to_owned(),
                attr(tag, "smil:direction") == Some("reverse"),
                attr(tag, "smil:mode") == Some("out"),
            ));
        }
    }
    if flash_bulb.as_deref() != Some(probes_libreoffice::LO_FLASH_BULB_FILTER) {
        return Err(format!(
            "the flash bulb's time filter is {flash_bulb:?} in the file, {:?} listed",
            probes_libreoffice::LO_FLASH_BULB_FILTER
        ));
    }
    let listed: Vec<(String, String, String, String, bool, bool)> =
        probes_libreoffice::ODF_PRESET_FILTERS
            .iter()
            .map(|&(a, b, c, d, e, f)| (a.into(), b.into(), c.into(), d.into(), e, f))
            .collect();
    if filters != listed {
        let first = filters
            .iter()
            .zip(&listed)
            .find(|(a, b)| a != b)
            .map(|(a, b)| format!(": first difference {a:?} in the file, {b:?} listed"))
            .unwrap_or_default();
        return Err(format!(
            "effects.xml clips differ from ODF_PRESET_FILTERS ({} in the file, {} listed){first}",
            filters.len(),
            listed.len()
        ));
    }
    let helpers = read("oox/source/ppt/pptfilterhelpers.cxx")?;
    let ppt_filters = pptx_filter_rows(&helpers);
    let listed: Vec<(String, String, String, bool)> = probes_libreoffice::PPTX_FILTERS
        .iter()
        .map(|&(a, b, c, d)| (a.into(), b.into(), c.into(), d))
        .collect();
    if ppt_filters != listed {
        return Err(format!(
            "pptfilterhelpers.cxx filters differ from PPTX_FILTERS ({} in the file, {} listed)",
            ppt_filters.len(),
            listed.len()
        ));
    }
    let listed: Vec<(String, String, String)> = probes_catalog::ODF_PRESETS
        .iter()
        .map(|(a, b, c)| ((*a).to_owned(), (*b).to_owned(), (*c).to_owned()))
        .collect();
    if presets != listed {
        return Err(format!(
            "effects.xml presets differ from ODF_PRESETS ({} in the file, {} listed)",
            presets.len(),
            listed.len()
        ));
    }
    let mut transitions = Vec::new();
    for name in ["sd/xml/transitions.xml", "sd/xml/transitions-ogl.xml"] {
        let text = read(name)?;
        let mut current: Option<String> = None;
        let mut rest = text.as_str();
        while let Some(start) = rest.find('<') {
            let tail = &rest[start..];
            let end = tail.find('>').unwrap_or(tail.len());
            let tag = &tail[..end];
            if tag.starts_with("<anim:par ") {
                current = attr(tag, "pres:preset-id").map(str::to_owned);
            } else if tag.starts_with("<anim:transitionFilter ")
                && let Some(id) = current.take()
            {
                transitions.push((
                    id,
                    attr(tag, "smil:type").unwrap_or("").to_owned(),
                    attr(tag, "smil:subtype").unwrap_or("").to_owned(),
                    attr(tag, "smil:direction") == Some("reverse"),
                    attr(tag, "smil:fadeColor").unwrap_or("").to_owned(),
                ));
            }
            rest = &tail[end..];
        }
    }
    let listed: Vec<(String, String, String, bool, String)> = probes_catalog::ODF_TRANSITIONS
        .iter()
        .map(|(a, b, c, d, e)| {
            (
                (*a).to_owned(),
                (*b).to_owned(),
                (*c).to_owned(),
                *d,
                (*e).to_owned(),
            )
        })
        .collect();
    if transitions != listed {
        return Err(format!(
            "transitions differ from ODF_TRANSITIONS ({} in the files, {} listed)",
            transitions.len(),
            listed.len()
        ));
    }
    println!(
        "catalog matches LibreOffice {LIBREOFFICE_COMMIT}: {} presets ({} with a clip), {} transitions, {} PowerPoint filters",
        presets.len(),
        filters.len(),
        transitions.len(),
        ppt_filters.len()
    );
    Ok(())
}

/// The rows of `transition::getList()`: `{ "wipe(up)", ...TransitionType::BARWIPE, ...TransitionSubType::TOPTOBOTTOM,
/// true }`, with the constants spelled as SMIL names (matched case-insensitively against the names the table uses).
fn pptx_filter_rows(source: &str) -> Vec<(String, String, String, bool)> {
    let smil = |constant: &str| -> String {
        probes_libreoffice::PPTX_FILTERS
            .iter()
            .flat_map(|&(_, ty, sub, _)| [ty, sub])
            .find(|name| name.eq_ignore_ascii_case(constant))
            .unwrap_or(constant)
            .to_owned()
    };
    let mut out = Vec::new();
    for line in source.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        let Some(rest) = line.strip_prefix("{ \"") else {
            continue;
        };
        let Some((name, rest)) = rest.split_once('"') else {
            continue;
        };
        let fields: Vec<&str> = rest
            .trim_start_matches(',')
            .trim_end_matches(',')
            .trim_end_matches('}')
            .split(',')
            .map(str::trim)
            .collect();
        let [ty, sub, forward] = fields.as_slice() else {
            continue;
        };
        let constant = |field: &str| field.rsplit("::").next().unwrap_or("").to_owned();
        out.push((
            name.to_owned(),
            smil(&constant(ty)),
            smil(&constant(sub)),
            *forward == "true",
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regenerating_is_a_no_op() {
        let root = repo_root();
        let first = generate();
        assert_eq!(first, generate(), "the generator is not deterministic");
        for (name, bytes) in &first {
            let committed = std::fs::read(root.join("probes/out").join(name)).unwrap_or_default();
            assert!(
                committed == *bytes,
                "probes/out/{name} is stale: run cargo xtask probes-motion"
            );
        }
    }

    #[test]
    fn every_catalog_row_is_covered_or_declared() {
        let coverage = coverage(&probes_catalog::deck());
        let Json::Arr(rows) = coverage else { panic!() };
        for row in &rows {
            assert_ne!(
                row.get("notAuthorable"),
                Some(&Json::from("MISSING")),
                "{row:?}"
            );
        }
        assert!(rows.len() > 300);
    }

    #[test]
    fn both_decks_stay_under_the_slide_limit_and_share_order() {
        let slides = probes_catalog::deck();
        let pptx: Vec<&str> = slides
            .iter()
            .filter(|s| s.formats.pptx)
            .map(|s| s.key.as_str())
            .collect();
        let odp: Vec<&str> = slides
            .iter()
            .filter(|s| s.formats.odp)
            .map(|s| s.key.as_str())
            .collect();
        assert!(
            pptx.len() <= 300 && odp.len() <= 300,
            "{} / {}",
            pptx.len(),
            odp.len()
        );
        let shared_in_pptx: Vec<&str> = pptx.iter().copied().filter(|k| odp.contains(k)).collect();
        let shared_in_odp: Vec<&str> = odp.iter().copied().filter(|k| pptx.contains(k)).collect();
        assert_eq!(shared_in_pptx, shared_in_odp);
        let mut keys: Vec<&str> = slides.iter().map(|s| s.key.as_str()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), slides.len(), "slide keys are unique");
    }

    #[test]
    fn expectations_cover_every_slide_and_step() {
        let slides = probes_catalog::deck();
        let expect = expectations(&slides);
        for fmt in [Fmt::Pptx, Fmt::Odp] {
            let deck = expect
                .get("decks")
                .and_then(|d| d.get(format_name(fmt)))
                .unwrap();
            let Some(Json::Arr(list)) = deck.get("slideList") else {
                panic!()
            };
            let included: Vec<&Slide> = slides.iter().filter(|s| s.formats.has(fmt)).collect();
            assert_eq!(list.len(), included.len());
            for (entry, slide) in list.iter().zip(included) {
                let Some(Json::Arr(steps)) = entry.get("steps") else {
                    panic!()
                };
                assert_eq!(
                    steps.len(),
                    slide.steps.iter().filter(|s| s.formats.has(fmt)).count(),
                    "{}",
                    slide.key
                );
                assert!(
                    slide.transition.is_some() || !steps.is_empty() || slide.key == "title",
                    "{} carries no motion",
                    slide.key
                );
            }
        }
    }

    #[test]
    fn decks_hold_the_parts_they_name() {
        for (name, bytes) in generate()
            .iter()
            .filter(|(n, _)| n.ends_with(".pptx") || n.ends_with(".odp"))
        {
            let entries = probes_zip::read_zip(bytes);
            let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
            if name.ends_with(".odp") {
                assert_eq!(names[0], "mimetype");
                assert!(names.contains(&"content.xml") && names.contains(&"Media/clip.mp4"));
            } else {
                assert_eq!(names[0], "[Content_Types].xml");
                assert!(
                    names.contains(&"ppt/presentation.xml")
                        && names.contains(&"ppt/media/clip.mp4")
                );
            }
        }
    }

    #[test]
    fn directions_follow_the_content_moves_convention() {
        use probes_catalog::preset_vocab;
        let dir = |class, ooo: &str, sub: &str| match preset_vocab(class, ooo, sub) {
            Vocab::Effect { params, .. } => params.get("dir").cloned(),
            _ => None,
        };
        assert_eq!(
            dir(Class::Entr, "ooo-entrance-fly-in", "from-left"),
            Some(Json::from("r"))
        );
        assert_eq!(
            dir(Class::Entr, "ooo-entrance-fly-in", "from-bottom"),
            Some(Json::from("u"))
        );
        assert_eq!(
            dir(Class::Entr, "ooo-entrance-fly-in", "from-top-left"),
            Some(Json::from("rd"))
        );
        assert_eq!(
            dir(Class::Exit, "ooo-exit-fly-out", "to-bottom"),
            Some(Json::from("d"))
        );
        assert_eq!(
            dir(Class::Exit, "ooo-exit-peek-out", "from-bottom"),
            Some(Json::from("d"))
        );
        assert_eq!(
            dir(Class::Entr, "ooo-entrance-wipe", "from-bottom"),
            Some(Json::from("u"))
        );
        assert_eq!(
            dir(Class::Exit, "ooo-exit-wipe", "from-bottom"),
            Some(Json::from("u"))
        );
        assert_eq!(
            dir(Class::Exit, "ooo-exit-crawl-out", "to-left"),
            Some(Json::from("l"))
        );
    }

    /// A written clip: smil:type, smil:subtype, reverse, `smil:mode="out"`.
    type WrittenClip = (String, String, bool, bool);

    /// The written ODP deck: each preset's `anim:transitionFilter`, by (preset-id, sub-type).
    fn odp_preset_clips() -> Vec<((String, String), WrittenClip)> {
        let entries = probes_odp::write(&probes_catalog::deck(), CLIP, POSTER);
        let content = entries
            .iter()
            .find(|e| e.name == "content.xml")
            .map(|e| String::from_utf8_lossy(&e.data).into_owned())
            .unwrap();
        let mut out = Vec::new();
        let mut current = None;
        for tag in tags(
            &content,
            &["anim:par", "anim:iterate", "anim:transitionFilter"],
        ) {
            if let Some(id) = attr(tag, "presentation:preset-id") {
                let sub = attr(tag, "presentation:preset-sub-type").unwrap_or("");
                current = Some((id.to_owned(), sub.to_owned()));
            } else if tag.starts_with("<anim:transitionFilter")
                && let Some(key) = current.take()
            {
                out.push((
                    key,
                    (
                        attr(tag, "smil:type").unwrap_or("").to_owned(),
                        attr(tag, "smil:subtype").unwrap_or("").to_owned(),
                        attr(tag, "smil:direction") == Some("reverse"),
                        attr(tag, "smil:mode") == Some("out"),
                    ),
                ));
            }
        }
        out
    }

    #[test]
    fn odp_presets_play_libreoffices_clip() {
        let written = odp_preset_clips();
        for &(id, sub, ty, subtype, reverse, out) in probes_libreoffice::ODF_PRESET_FILTERS {
            let found: Vec<_> = written
                .iter()
                .filter(|((i, s), _)| i == id && s == sub)
                .collect();
            assert!(!found.is_empty(), "{id}/{sub} is not in the ODP deck");
            for (_, clip) in found {
                assert_eq!(
                    clip,
                    &(ty.to_owned(), subtype.to_owned(), reverse, out),
                    "{id}/{sub}"
                );
            }
        }
        // a preset LibreOffice gives no clip carries our own fade, never another clip
        for ((id, sub), clip) in &written {
            let listed = probes_libreoffice::ODF_PRESETS
                .iter()
                .any(|r| r.1 == id && r.2 == sub);
            if listed
                && !probes_libreoffice::ODF_PRESET_FILTERS
                    .iter()
                    .any(|r| r.0 == id && r.1 == sub)
            {
                assert_eq!(
                    (clip.0.as_str(), clip.1.as_str()),
                    ("fade", "crossfade"),
                    "{id}/{sub}"
                );
            }
        }
    }

    #[test]
    fn pptx_filters_round_trip_libreoffices_reading() {
        use probes_catalog::{Bhv, Clip};
        for slide in probes_catalog::deck() {
            for step in slide.steps.iter().filter(|s| s.formats.pptx) {
                for bhv in &step.effect.bhv {
                    let Bhv::Filter(clip) = bhv else { continue };
                    let name = clip.pptx.unwrap_or_else(|| {
                        panic!("{}: no PowerPoint filter for {clip:?}", slide.key)
                    });
                    let read = Clip::pptx(name, clip.out).unwrap();
                    let norm = |s: &str| {
                        if s.is_empty() {
                            "default".to_owned()
                        } else {
                            s.to_owned()
                        }
                    };
                    assert_eq!(
                        (read.ty, norm(read.subtype), read.reverse),
                        (clip.ty, norm(clip.subtype), clip.reverse),
                        "{}: {name}",
                        slide.key
                    );
                }
            }
        }
    }

    #[test]
    fn one_clip_rule_for_presets_tier2_and_transitions() {
        use probes_catalog::{clip_vocab, lo_clip, preset_vocab};
        let params_of = |v: Vocab| match v {
            Vocab::Effect { name, params, .. } => Some((name, params)),
            _ => None,
        };
        // the cases LibreOffice's sub-type labels get backwards: the filter decides
        assert_eq!(
            params_of(preset_vocab(
                Class::Entr,
                "ooo-entrance-venetian-blinds",
                "vertical"
            )),
            Some(("blinds", Json::obj().with("orient", "horizontal")))
        );
        assert_eq!(
            params_of(preset_vocab(
                Class::Exit,
                "ooo-exit-random-bars",
                "horizontal"
            )),
            Some(("randomBars", Json::obj().with("orient", "vertical")))
        );
        assert_eq!(
            params_of(preset_vocab(
                Class::Entr,
                "ooo-entrance-diagonal-squares",
                "left-to-bottom"
            )),
            Some(("strips", Json::obj().with("corner", "rd")))
        );
        assert_eq!(
            params_of(preset_vocab(
                Class::Exit,
                "ooo-exit-diagonal-squares",
                "left-to-bottom"
            )),
            Some(("strips", Json::obj().with("corner", "lu")))
        );
        // an exit iris's or split's in/out names the entrance its filter plays backwards, as LibreOffice's engine
        // plays it: exit box `in` (irisWipe reversed, mode out) opens a hole from the centre
        for (ooo, sub, name, key, want) in [
            ("ooo-exit-box", "in", "iris", "inOut", "in"),
            ("ooo-exit-circle", "out", "iris", "inOut", "out"),
            ("ooo-exit-plus", "in", "iris", "inOut", "in"),
            ("ooo-exit-split", "horizontal-in", "split", "inOut", "in"),
            ("ooo-exit-split", "vertical-out", "split", "inOut", "out"),
            ("ooo-entrance-box", "in", "iris", "inOut", "in"),
            (
                "ooo-entrance-split",
                "vertical-out",
                "split",
                "inOut",
                "out",
            ),
        ] {
            let class = if ooo.starts_with("ooo-exit-") {
                Class::Exit
            } else {
                Class::Entr
            };
            let (got, params) = params_of(preset_vocab(class, ooo, sub)).unwrap();
            assert_eq!(got, name, "{ooo}/{sub}");
            assert_eq!(params.get(key), Some(&Json::from(want)), "{ooo}/{sub}");
        }
        let pptx_clip = |name: &str| {
            let c = probes_catalog::Clip::pptx(name, false).unwrap();
            clip_vocab(c.ty, c.subtype, c.reverse, c.out)
        };
        assert_eq!(
            pptx_clip("blinds(horizontal)"),
            Some(("blinds", Json::obj().with("orient", "horizontal")))
        );
        assert_eq!(
            pptx_clip("strips(downRight)"),
            Some(("strips", Json::obj().with("corner", "rd")))
        );
        assert_eq!(
            pptx_clip("wipe(down)"),
            Some(("wipe", Json::obj().with("dir", "u")))
        );
        // every clip preset's expectation is its clip's
        for &(class, ooo, sub) in probes_libreoffice::ODF_PRESETS {
            let Some(clip) = lo_clip(ooo, sub) else {
                continue;
            };
            let Some((clip_name, clip_params)) =
                clip_vocab(clip.ty, clip.subtype, clip.reverse, clip.out)
            else {
                continue;
            };
            let class = match class {
                "entrance" => Class::Entr,
                "exit" => Class::Exit,
                _ => continue,
            };
            if let Some((name, params)) = params_of(preset_vocab(class, ooo, sub))
                && (name == clip_name || (name == "peek" && clip_name == "flyIn"))
            {
                let Json::Obj(fields) = &params else { panic!() };
                for (key, value) in fields {
                    assert_eq!(clip_params.get(key), Some(value), "{ooo}/{sub} {key}");
                }
            }
        }
        // every transition whose SMIL filter is a clip agrees with it, in both decks
        let deck = probes_catalog::deck();
        let mut checked = 0;
        for slide in &deck {
            let Some(trans) = &slide.transition else {
                continue;
            };
            let (Some(odp), Some((name, params, _))) = (&trans.odp, &trans.vocab) else {
                continue;
            };
            let Some((ty, subtype, reverse, _)) = &odp.smil else {
                continue;
            };
            let Some((clip_name, clip_params)) = clip_vocab(ty, subtype, *reverse, false) else {
                continue;
            };
            if clip_name != *name {
                continue;
            }
            let Json::Obj(fields) = &clip_params else {
                panic!()
            };
            for (key, value) in fields {
                assert_eq!(params.get(key), Some(value), "{} {key}", slide.key);
            }
            checked += 1;
        }
        assert!(checked >= 30, "{checked} transitions checked");
        // PPTX zoom pairs with ODF zoom default, reversed for out
        for io in ["in", "out"] {
            let slide = deck
                .iter()
                .find(|s| s.key == format!("tr-zoom-{io}"))
                .unwrap();
            assert!(slide.formats.pptx && slide.formats.odp);
            let trans = slide.transition.as_ref().unwrap();
            let smil = trans.odp.as_ref().and_then(|o| o.smil.as_ref()).unwrap();
            assert_eq!(
                (smil.0.as_str(), smil.1.as_str(), smil.2),
                ("zoom", "default", io == "out")
            );
            let params = trans.vocab.as_ref().map(|v| v.1.clone()).unwrap();
            assert_eq!(params.get("inOut"), Some(&Json::from(io)));
        }
    }

    fn deck_slides(expect: &Json, fmt: Fmt) -> Vec<Json> {
        match expect
            .get("decks")
            .and_then(|d| d.get(format_name(fmt)))
            .and_then(|d| d.get("slideList"))
        {
            Some(Json::Arr(list)) => list.clone(),
            _ => panic!(),
        }
    }

    #[test]
    fn shared_slides_agree_across_formats() {
        let expect = expectations(&probes_catalog::deck());
        let pptx = deck_slides(&expect, Fmt::Pptx);
        let odp = deck_slides(&expect, Fmt::Odp);
        let mut compared = 0;
        let mut differences = Vec::new();
        for a in &pptx {
            let key = a.get("key").unwrap();
            let Some(b) = odp.iter().find(|b| b.get("key") == Some(key)) else {
                continue;
            };
            compared += 1;
            let key = format!("{key:?}");
            if let (Some(ta @ Json::Obj(_)), Some(tb @ Json::Obj(_))) =
                (a.get("transition"), b.get("transition"))
            {
                for field in ["effect", "approximate", "advance"] {
                    if ta.get(field) != tb.get(field) {
                        differences.push(format!("{key} transition.{field}"));
                    }
                }
                let strip_soft = |p: Option<&Json>| match p {
                    Some(Json::Obj(fields)) => Json::Obj(
                        fields
                            .iter()
                            .filter(|(k, _)| k != "soft")
                            .cloned()
                            .collect(),
                    ),
                    other => other.cloned().unwrap_or(Json::Null),
                };
                if strip_soft(ta.get("params")) != strip_soft(tb.get("params")) {
                    differences.push(format!("{key} transition.params"));
                }
                if !key.contains("tm-speed-") && ta.get("durationMs") != tb.get("durationMs") {
                    differences.push(format!("{key} transition.durationMs"));
                }
            }
            let (Some(Json::Arr(sa)), Some(Json::Arr(sb))) = (a.get("steps"), b.get("steps"))
            else {
                panic!()
            };
            // a step pairs with the step of the same shape and rank among that shape's steps
            let ranked = |list: &[Json]| -> Vec<(Option<Json>, usize, Json)> {
                let mut seen: Vec<Option<Json>> = Vec::new();
                list.iter()
                    .map(|x| {
                        let shape = x.get("shape").cloned();
                        let rank = seen.iter().filter(|s| **s == shape).count();
                        seen.push(shape.clone());
                        (shape, rank, x.clone())
                    })
                    .collect()
            };
            let (ra, rb) = (ranked(sa), ranked(sb));
            let pairs: Vec<(&Json, &Json)> = ra
                .iter()
                .filter_map(|(shape, rank, x)| {
                    rb.iter()
                        .find(|(s, r, _)| s == shape && r == rank)
                        .map(|(_, _, y)| (x, y))
                })
                .collect();
            for (x, y) in pairs {
                for field in [
                    "effect",
                    "params",
                    "approximate",
                    "tier",
                    "clip",
                    "trigger",
                    "kind",
                    "delivery",
                    "stagger",
                    "paragraphs",
                    "offsetMs",
                    "delayMs",
                    "durationMs",
                    "repeat",
                    "autoReverse",
                    "after",
                    "dropped",
                    "fallback",
                ] {
                    if x.get(field) != y.get(field) {
                        differences.push(format!("{key} {:?} {field}", x.get("shape")));
                    }
                }
                let reads = |h: Option<&Json>| match h {
                    Some(Json::Obj(fields)) => fields
                        .iter()
                        .map(|(k, v)| (k.clone(), v.get("read").cloned()))
                        .collect::<Vec<_>>(),
                    _ => Vec::new(),
                };
                if reads(x.get("hostileTiming")) != reads(y.get("hostileTiming")) {
                    differences.push(format!("{key} {:?} hostileTiming", x.get("shape")));
                }
            }
        }
        assert!(compared > 130, "{compared} shared slides");
        assert!(differences.is_empty(), "{differences:#?}");
    }

    #[test]
    fn unmapped_transitions_name_their_code() {
        let expect = expectations(&probes_catalog::deck());
        for (fmt, code) in [(Fmt::Pptx, "PX2402"), (Fmt::Odp, "OD9402")] {
            let mut count = 0;
            for slide in deck_slides(&expect, fmt) {
                let Some(t @ Json::Obj(_)) = slide.get("transition") else {
                    continue;
                };
                if t.get("effect") == Some(&Json::Null) {
                    count += 1;
                    assert_eq!(
                        t.get("diagnostic").and_then(|d| d.get("code")),
                        Some(&Json::from(code)),
                        "{:?}",
                        slide.get("key")
                    );
                }
            }
            assert!(count > 10, "{count}");
        }
    }

    #[test]
    fn timing_values_read_bounded() {
        let read = |text: &str, fmt| match TimeRead::raw(text, fmt) {
            TimeRead::Ms(ms) => format!("ms {ms}"),
            TimeRead::Clamped(_) => "clamped".to_owned(),
            TimeRead::Invalid(_) => "invalid".to_owned(),
        };
        assert_eq!(read("500", Fmt::Pptx), "ms 500");
        assert_eq!(read("NaN", Fmt::Pptx), "invalid");
        assert_eq!(read("-500", Fmt::Pptx), "invalid");
        assert_eq!(read("99999999999", Fmt::Pptx), "clamped");
        assert_eq!(read("0.5s", Fmt::Odp), "ms 500");
        assert_eq!(read("NaN", Fmt::Odp), "invalid");
        assert_eq!(read("NaNs", Fmt::Odp), "invalid");
        assert_eq!(read("-0.5s", Fmt::Odp), "invalid");
        assert_eq!(read("inf", Fmt::Pptx), "invalid");
        assert_eq!(read("99999999999s", Fmt::Odp), "clamped");
        assert!(matches!(iso_read("PT00H00M03S"), TimeRead::Ms(3000)));
        assert!(matches!(iso_read("-PT5S"), TimeRead::Invalid(_)));
        assert!(matches!(iso_read("PT2H"), TimeRead::Clamped(_)));
    }

    #[test]
    fn hostile_timing_expectations() {
        let expect = expectations(&probes_catalog::deck());
        for (fmt, prefix) in [(Fmt::Pptx, "PX"), (Fmt::Odp, "OD")] {
            let slides = deck_slides(&expect, fmt);
            let hostile = slides
                .iter()
                .find(|s| s.get("key") == Some(&Json::from("ft-hostile")))
                .unwrap();
            let Some(Json::Arr(steps)) = hostile.get("steps") else {
                panic!()
            };
            let code = |s: &Json| s.get("diagnostic").and_then(|d| d.get("code")).cloned();
            let field = |s: &Json, f: &str| s.get(f).cloned();
            // A, B, C: self-reference and a two-node cycle are dropped
            for step in &steps[..3] {
                assert_eq!(field(step, "dropped"), Some(Json::Bool(true)));
                assert_eq!(
                    code(step),
                    Some(Json::from(format!(
                        "{prefix}{}",
                        if prefix == "PX" { "2411" } else { "9411" }
                    )))
                );
            }
            // the huge repeat keeps the spin and clamps the count
            assert_eq!(field(&steps[3], "effect"), Some(Json::from("spin")));
            assert_eq!(
                field(&steps[3], "repeat"),
                Some(Json::obj().with("count", REPEAT_MAX))
            );
            assert_eq!(field(&steps[4], "durationMs"), Some(Json::from(MS_MAX)));
            assert_eq!(field(&steps[5], "delayMs"), Some(Json::from(MS_MAX)));
            assert_eq!(field(&steps[5], "offsetMs"), Some(Json::from(MS_MAX)));
            // the deep formula can't compile: effect-unmapped, the fallback plays
            assert_eq!(field(&steps[6], "effect"), Some(Json::Null));
            assert_eq!(field(&steps[6], "fallback"), Some(Json::from("dissolve")));
            assert_eq!(
                code(&steps[6]),
                Some(Json::from(format!(
                    "{prefix}{}",
                    if prefix == "PX" { "2410" } else { "9410" }
                )))
            );
            assert_eq!(field(&steps[7], "dropped"), Some(Json::Bool(true)));
            let malformed = slides
                .iter()
                .find(|s| s.get("key") == Some(&Json::from("ft-hostile-malformed")))
                .unwrap();
            let Some(Json::Arr(steps)) = malformed.get("steps") else {
                panic!()
            };
            assert_eq!(field(&steps[0], "durationMs"), Some(Json::Null));
            assert_eq!(field(&steps[1], "durationMs"), Some(Json::Null));
            assert_eq!(field(&steps[2], "delayMs"), Some(Json::from(MS_MAX)));
            let advance = malformed
                .get("transition")
                .and_then(|t| t.get("advance"))
                .unwrap();
            assert_eq!(advance.get("onClick"), Some(&Json::Bool(true)));
            assert_eq!(advance.get("afterMs"), Some(&Json::Null));
        }
    }

    #[test]
    fn hostile_trees_are_written_resolved_and_within_the_xml_depth() {
        let slides = probes_catalog::deck();
        let entries = probes_pptx::write(&slides, CLIP, POSTER);
        for entry in &entries {
            let text = String::from_utf8_lossy(&entry.data);
            assert!(!text.contains("@tn"), "{} keeps a placeholder", entry.name);
            // element depth stays under slidock-io's 256 levels
            let (mut depth, mut max) = (0i32, 0i32);
            for tag in text.split('<').skip(1) {
                if tag.starts_with('/') {
                    depth -= 1;
                } else if !tag.starts_with('?') && !tag.starts_with('!') {
                    let head = tag.split('>').next().unwrap_or("");
                    if !head.ends_with('/') {
                        depth += 1;
                        max = max.max(depth);
                    }
                }
            }
            assert!(max < 256, "{} nests {max} deep", entry.name);
            if text.contains("ooo-entrance-probe-deep")
                || entry.name.ends_with(".xml") && text.contains("nested in 64")
            {
                assert!(
                    max > 3 * probes_catalog::HOSTILE_NEST as i32,
                    "{} nests {max}",
                    entry.name
                );
            }
        }
    }

    /// Runs `--check-catalog` against the pinned LibreOffice files in `$SLIDOCK_PRIVATE/tools`; fails when they are
    /// missing.
    #[test]
    #[ignore = "reads LibreOffice's files from $SLIDOCK_PRIVATE/tools/libreoffice-bbb07447: cargo test -p xtask -- --ignored"]
    fn catalog_matches_libreoffice() {
        check_catalog(&private_dir().join("tools/libreoffice-bbb07447")).unwrap();
    }

    #[test]
    fn pptx_subtypes_invert_libreoffice() {
        use probes_catalog::pptx_subtype;
        assert_eq!(pptx_subtype(2, "from-bottom"), Some(4));
        assert_eq!(pptx_subtype(2, "to-top-left"), Some(9));
        assert_eq!(pptx_subtype(5, "downward"), Some(5));
        assert_eq!(pptx_subtype(5, "across"), Some(10));
        assert_eq!(pptx_subtype(18, "right-to-top"), Some(3));
        assert_eq!(pptx_subtype(23, "in-from-screen-center"), Some(528));
        assert_eq!(pptx_subtype(23, "544"), Some(544));
        assert_eq!(pptx_subtype(17, "downward"), None);
        assert_eq!(pptx_subtype(16, "horizontal-out"), Some(42));
    }

    #[test]
    fn timeline_offsets() {
        let slides = probes_catalog::deck();
        let slide = slides.iter().find(|s| s.key == "ft-triggers").unwrap();
        let groups = layout(&slide.steps, Fmt::Pptx);
        let offs = offsets(&groups, slide.steps.len());
        let starts: Vec<u32> = slide
            .steps
            .iter()
            .zip(&offs)
            .map(|(s, o)| o.unwrap() + s.delay_ms)
            .collect();
        assert_eq!(starts, vec![0, 250, 1250, 0, 500]);
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn pixel_paths() {
        assert_eq!(
            probes_catalog::path_px("M 0 0 L 0.25 -0.25 E"),
            "M 0 0 L 320 -180"
        );
        assert_eq!(
            probes_catalog::path_px(&probes_catalog::motion_path("ooo-motionpath-custom")),
            "M 0 0 C 128 0 256 72 320 180"
        );
    }
}
