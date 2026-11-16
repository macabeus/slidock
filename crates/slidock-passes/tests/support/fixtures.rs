//! The private fixtures of the golden files (tests/golden), and the IR fixtures lowered as the IR frontend lowers them:
//! resolved-only values dropped (IR0012, once per kind per container), and the media of the `media/` folder beside the
//! fixture verified by hash (IR0013). The suites skip when the fixtures are absent.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use indexmap::IndexMap;
use serde_json::Value;
use slidock_core::{
    CompileOptions, CompileResult, FrontendChoice, FrontendRegistry, PassManager, Phase,
    VerifyMode, compile,
};
use slidock_core_api::frontend::SlideFacts;
use slidock_core_api::{
    Analysis, Clock, Frontend, FrontendCtx, FrontendError, Input, LowerOptions, LowerResult,
    MediaHandle, MediaSource, MediaStore, Provenance, SniffHints, Unverified,
};
use slidock_io::BytesReader;
use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::features::{FeatureId, census_presentation};
use slidock_ir::formats::SourceFormat;
use slidock_ir::ids::{NodeId, Sha256};
use slidock_ir::presentation::SlideSize;
use slidock_ir::source::Presentation;

use super::Applied;
use super::golden::Golden;

/// The golden files, opened once; `None` when they are absent.
pub fn golden() -> Option<&'static Golden> {
    static GOLDEN: OnceLock<Option<Golden>> = OnceLock::new();
    GOLDEN
        .get_or_init(|| Golden::open("slidock-passes"))
        .as_ref()
}

fn opened() -> &'static Golden {
    golden().unwrap()
}

/// The IR fixture names, sorted.
pub fn ir_fixture_names() -> Vec<String> {
    let names = opened().ir_fixtures();
    assert!(!names.is_empty(), "no IR fixtures");
    names
}

pub fn ir_fixture_path(name: &str) -> PathBuf {
    opened().fixture(&format!("ir/{name}.presentation-ir.json"))
}

/// A private presentation fixture, `source` or `resolved`.
pub fn presentation_ir_fixture(phase: &str, name: &str) -> Value {
    let path = opened().fixture(&format!("presentation/{phase}/{name}.presentation-ir.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap()
}

/// A private source fixture, typed.
pub fn source_fixture(name: &str) -> Presentation {
    super::parse(presentation_ir_fixture("source", name))
}

const LINE_METRICS: [&str; 5] = [
    "lineHeight",
    "leadingSkew",
    "lineTailPt",
    "spaceBeforePt",
    "spaceAfterPt",
];
const ELEMENT_KINDS: [&str; 6] = ["shape", "picture", "media", "table", "group", "foreign"];
/// Raw source values, kept as written.
const OPAQUE: [&str; 2] = ["ext", "payload"];

/// `value` minus its resolved-only fields, which passes compute; `note` hears each field kind dropped.
fn strip(value: &mut Value, note: &mut dyn FnMut(&'static str)) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(|item| strip(item, note)),
        Value::Object(object) => {
            for (key, child) in object.iter_mut() {
                if !OPAQUE.contains(&key.as_str()) {
                    strip(child, note);
                }
            }
            let kind = object
                .get("kind")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let mut drops: Vec<(&str, &'static str)> = Vec::new();
            if kind.as_deref() == Some("theme") && object.contains_key("mods") {
                drops.push(("resolved", "colour resolved value"));
            }
            if kind.as_deref() == Some("shrink") {
                drops.push(("fitted", "autofit fitted flag"));
            }
            if object.get("paragraphs").is_some_and(Value::is_array) {
                drops.push(("rotation", "text rotation"));
            }
            for key in LINE_METRICS {
                drops.push((key, key));
            }
            if kind.as_deref().is_some_and(|k| ELEMENT_KINDS.contains(&k))
                && object.contains_key("origin")
            {
                drops.push(("inherits", "inheritance edge"));
            }
            for (key, field) in drops {
                if object.shift_remove(key).is_some() {
                    note(field);
                }
            }
        }
        _ => {}
    }
}

/// The IR frontend as the CLI runs it: an IR file, with its media in the `media/` folder beside it.
pub struct IrFixtureFrontend {
    media_dir: PathBuf,
}

impl IrFixtureFrontend {
    pub fn beside(path: &Path) -> Self {
        IrFixtureFrontend {
            media_dir: path.parent().unwrap().join("media"),
        }
    }
}

/// The files of a media folder by the hash their name starts with (`<sha256>.<ext>`), the first name of a hash
/// winning, as bytes.
#[derive(Clone, Debug, Default)]
struct MediaDir {
    files: IndexMap<String, PathBuf>,
}

impl MediaDir {
    fn read(dir: &Path) -> MediaDir {
        let mut names: Vec<String> = match std::fs::read_dir(dir) {
            Ok(entries) => entries
                .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
                .collect(),
            Err(_) => Vec::new(),
        };
        names.sort();
        let mut files = IndexMap::new();
        for name in names {
            let Some((sha, ext)) = name.split_once('.') else {
                continue;
            };
            let named = sha.len() == 64
                && sha
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                && (1..=8).contains(&ext.len())
                && ext.bytes().all(|b| b.is_ascii_alphanumeric());
            if named && !files.contains_key(sha) {
                files.insert(sha.to_owned(), dir.join(&name));
            }
        }
        MediaDir { files }
    }
}

/// The media whose bytes hashed to their name.
struct VerifiedMedia(IndexMap<Sha256, Arc<[u8]>>);

impl MediaStore for VerifiedMedia {
    fn open(&self, sha256: &Sha256) -> Option<MediaHandle> {
        self.0.get(sha256).map(|bytes| MediaHandle {
            bytes: bytes.len() as u64,
            source: MediaSource::Bytes(bytes.clone()),
        })
    }
}

fn crashed(error: impl std::fmt::Display) -> FrontendError {
    FrontendError::Crashed(error.to_string())
}

fn read_json(input: &Input<'_>) -> Result<(Value, Vec<u8>), FrontendError> {
    let bytes = input
        .reader
        .read_at(0, input.size() as usize)
        .map_err(crashed)?
        .into_owned();
    let json = serde_json::from_slice(&bytes).map_err(crashed)?;
    Ok((json, bytes))
}

fn features_of(json: &Value) -> IndexMap<String, Vec<FeatureId>> {
    serde_json::from_value::<Presentation>(json.clone())
        .map(|presentation| census_presentation(&presentation))
        .unwrap_or_default()
}

impl Frontend for IrFixtureFrontend {
    fn format(&self) -> SourceFormat {
        SourceFormat::Ir
    }

    fn version(&self) -> &str {
        "1"
    }

    fn sniff(&self, _head: &[u8], hints: &SniffHints<'_>) -> bool {
        hints.name.ends_with(".presentation-ir.json")
    }

    fn analyze(
        &self,
        input: &Input<'_>,
        _ctx: &mut FrontendCtx<'_>,
    ) -> Result<Analysis, FrontendError> {
        let (json, _) = read_json(input)?;
        let size: SlideSize = serde_json::from_value(json["size"].clone()).map_err(crashed)?;
        let slides = json["slides"]
            .as_array()
            .ok_or_else(|| crashed("no slides"))?
            .iter()
            .map(|slide| {
                Ok(SlideFacts {
                    id: NodeId::new(slide["id"].as_str().unwrap_or_default()).map_err(crashed)?,
                    hidden: slide["hidden"].as_bool().unwrap_or(false),
                    title: slide["title"].as_str().map(str::to_owned),
                })
            })
            .collect::<Result<Vec<_>, FrontendError>>()?;
        let mut features: Vec<FeatureId> = features_of(&json).into_values().flatten().collect();
        features.sort();
        features.dedup();
        let media_bytes = json["media"]
            .as_object()
            .map(|media| media.values().filter_map(|e| e["bytes"].as_u64()).sum())
            .unwrap_or(0);
        Ok(Analysis {
            format: SourceFormat::Ir,
            size,
            slides,
            features,
            media_bytes,
            refusal: None,
        })
    }

    fn lower(
        &self,
        input: &Input<'_>,
        ctx: &mut FrontendCtx<'_>,
        _options: &LowerOptions<'_>,
    ) -> Result<LowerResult, FrontendError> {
        let (mut json, bytes) = read_json(input)?;
        // Each dropped kind once per container: the container's record key, or none for the presentation.
        let mut notes: Vec<(Option<String>, &'static str)> = Vec::new();
        let collect = |container: Option<String>, value: &mut Value| {
            let mut fields: Vec<&'static str> = Vec::new();
            strip(value, &mut |field| fields.push(field));
            fields
                .into_iter()
                .map(|field| (container.clone(), field))
                .collect::<Vec<_>>()
        };
        for record in ["masters", "layouts"] {
            if let Some(object) = json[record].as_object_mut() {
                for (key, container) in object.iter_mut() {
                    assert!(!has_synth_origin(container), "{key} holds a pass's origin");
                    notes.extend(collect(Some(key.clone()), container));
                }
            }
        }
        if let Some(slides) = json["slides"].as_array_mut() {
            for slide in slides {
                let id = slide["id"].as_str().map(str::to_owned);
                assert!(!has_synth_origin(slide), "a slide holds a pass's origin");
                notes.extend(collect(id.clone(), slide));
                if let Some(object) = slide.as_object_mut() {
                    if object.shift_remove("title").is_some() {
                        notes.push((id.clone(), "slide title"));
                    }
                    if object.shift_remove("readingOrder").is_some() {
                        notes.push((id.clone(), "reading order"));
                    }
                }
            }
        }
        notes.extend(collect(None, &mut json["defaults"]));
        for (record, field, note) in [
            ("fonts", "binding", "font binding"),
            ("media", "plan", "media plan"),
        ] {
            if let Some(entries) = json[record].as_object_mut() {
                for entry in entries.values_mut() {
                    if entry
                        .as_object_mut()
                        .and_then(|e| e.shift_remove(field))
                        .is_some()
                    {
                        notes.push((None, note));
                    }
                }
            }
        }
        json["phase"] = Value::from("source");
        let mut reported: Vec<(Option<String>, &'static str)> = Vec::new();
        for note in notes {
            if reported.contains(&note) {
                continue;
            }
            let (container, field) = note.clone();
            let container = container.and_then(|c| NodeId::new(c).ok());
            ctx.diag.report(
                codes::IR0012,
                DiagnosticSite::new()
                    .container(container)
                    .data("field", field),
            );
            reported.push(note);
        }

        // Media bytes are trusted only after hashing them.
        let dir = MediaDir::read(&self.media_dir);
        let mut shas: Vec<String> = json["media"]
            .as_object()
            .map(|media| media.keys().cloned().collect())
            .unwrap_or_default();
        shas.sort();
        let mut verified = IndexMap::new();
        for sha in shas {
            let Some(path) = dir.files.get(&sha) else {
                ctx.diag.report(
                    codes::IR0013,
                    DiagnosticSite::new()
                        .data("sha256", sha.as_str())
                        .data("reason", "missing"),
                );
                continue;
            };
            let file: Arc<[u8]> = std::fs::read(path).map_err(crashed)?.into();
            let actual = slidock_canon::sha256_hex(&file);
            match Sha256::parse(&sha) {
                Ok(parsed) if actual == sha => {
                    verified.insert(parsed, file);
                }
                _ => ctx.diag.report(
                    codes::IR0013,
                    DiagnosticSite::new().data("sha256", sha.as_str()).data(
                        "reason",
                        format!("not the bytes hashed ({}…)", &actual[..12]),
                    ),
                ),
            }
        }
        let features = features_of(&json);
        Ok(LowerResult {
            provenance: Provenance {
                format: SourceFormat::Ir,
                name: input.name.to_owned(),
                sha256: Sha256::of(&bytes),
                bytes: input.size(),
                app: None,
            },
            presentation: Unverified::Json(json),
            media: Box::new(VerifiedMedia(verified)),
            fonts: Vec::new(),
            features,
        })
    }
}

/// Whether a value holds an origin a pass created (`{ kind: "synth", pass }`) outside raw source values.
fn has_synth_origin(value: &Value) -> bool {
    match value {
        Value::Array(items) => items.iter().any(has_synth_origin),
        Value::Object(object) => {
            (object.get("kind").and_then(Value::as_str) == Some("synth")
                && object.get("pass").is_some_and(Value::is_string))
                || object
                    .iter()
                    .any(|(key, child)| !OPAQUE.contains(&key.as_str()) && has_synth_origin(child))
        }
        _ => false,
    }
}

/// The pass manager over every slidock pass.
pub fn manager() -> PassManager {
    PassManager::new(slidock_passes::passes()).unwrap()
}

/// An IR fixture compiled as `slidock compile` compiles it: its IR frontend, every pass, the built-in fonts and the
/// default policy, verified under `verify`.
pub fn compile_fixture(name: &str, verify: VerifyMode) -> CompileResult {
    compile_fixture_clocked(name, verify, None)
}

/// `compile_fixture` with the host's clock.
pub fn compile_fixture_clocked(
    name: &str,
    verify: VerifyMode,
    clock: Option<&dyn Clock>,
) -> CompileResult {
    let path = ir_fixture_path(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let registry = FrontendRegistry::new(vec![Box::new(IrFixtureFrontend::beside(&path))]).unwrap();
    let manager = manager();
    let mut options = CompileOptions::new(&registry, Phase::Resolved(&manager));
    options.frontend = FrontendChoice::Auto;
    options.verify = verify;
    options.clock = clock;
    let reader = BytesReader::new(bytes);
    let file_name = path.file_name().unwrap().to_str().unwrap().to_owned();
    compile(&Input::new(&file_name, &reader), &options)
}

/// An IR fixture as the IR frontend lowers it and verify-source accepts it, with the diagnostics on the way.
pub fn lowered_fixture(name: &str) -> Applied {
    let path = ir_fixture_path(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let registry = FrontendRegistry::new(vec![Box::new(IrFixtureFrontend::beside(&path))]).unwrap();
    let options = CompileOptions::new(&registry, Phase::Source);
    let reader = BytesReader::new(bytes);
    let file_name = path.file_name().unwrap().to_str().unwrap().to_owned();
    let result = compile(&Input::new(&file_name, &reader), &options);
    let presentation = result
        .source
        .unwrap_or_else(|| panic!("{name} does not lower: {:?}", result.diagnostics));
    Applied {
        presentation,
        diagnostics: result.diagnostics,
    }
}
