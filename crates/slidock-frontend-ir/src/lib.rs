//! The IR frontend: agent-written presentation IR as an input, a source presentation in JSON (a `.presentation-ir.json`
//! file, or any canonical dump, which starts with the IR's `$schema`), with its media bytes in a [`MediaFiles`] set the
//! host provides. Every document names the IR's `$schema`; one naming none or another is refused. It lowers the JSON
//! as it came, minus the values passes compute, for verify-source to check. Used by fixtures, probes and agents.

mod json;
mod media;
mod select;
mod strip;

use std::collections::BTreeSet;

use serde::Deserialize;
use serde_json::{Map, Value};
use slidock_canon::{EXACT_STEPS, is_js_whitespace, utf16};
use slidock_core_api::frontend::SlideFacts;
use slidock_core_api::{
    Analysis, Frontend, FrontendCtx, FrontendError, Input, LowerOptions, LowerResult, Provenance,
    SniffHints, Unverified,
};
use slidock_ir::diagnostics::{
    DataValue, DiagnosticCode, DiagnosticSite, Stage, codes, make_diagnostic,
};
use slidock_ir::features::{FeatureId, census_presentation};
use slidock_ir::formats::SourceFormat;
use slidock_ir::ids::{NodeId, Sha256};
use slidock_ir::presentation::{Failed, SchemaUrl, SlideSize};
use slidock_ir::source;
use slidock_ir::units::Size;
use slidock_ir::verify::parse_presentation;

pub use media::{MediaFiles, is_media_file_name};
pub use slidock_ir::canonical::dump_presentation;

use media::VerifiedMedia;
use strip::{Decided, strip, strip_entries, strip_slide, synth_origin_path};

const FORMAT: SourceFormat = SourceFormat::Ir;
const EMITTER: &str = "ir";
/// The name an IR file ends with.
pub const IR_FILE_SUFFIX: &str = ".presentation-ir.json";

/// The schema URL every IR document's `$schema` holds.
pub const SCHEMA_URL: &str = SchemaUrl::WIRE;

fn refusal(code: DiagnosticCode, data: &[(&'static str, DataValue)]) -> FrontendError {
    let mut site = DiagnosticSite::new();
    for (key, value) in data {
        site = site.data(*key, value.clone());
    }
    FrontendError::refused(make_diagnostic(code, Stage::Frontend, EMITTER, site))
}

fn schema_invalid(path: &str, reason: impl Into<String>) -> FrontendError {
    refusal(
        codes::IR0001,
        &[("path", path.into()), ("reason", reason.into().into())],
    )
}

fn crashed(error: impl std::fmt::Display) -> FrontendError {
    FrontendError::Crashed(error.to_string())
}

/// Whether `text` starts like an IR document: an optional BOM and whitespace, `{`, then `"$schema"` holding the IR's
/// schema URL.
fn has_schema_head(text: &str) -> bool {
    fn token<'a>(text: &'a str, expected: &str) -> Option<&'a str> {
        text.trim_start_matches(is_js_whitespace)
            .strip_prefix(expected)
    }
    let quoted = format!("\"{}\"", SCHEMA_URL);
    token(text, "{")
        .and_then(|rest| token(rest, "\"$schema\""))
        .and_then(|rest| token(rest, ":"))
        .and_then(|rest| token(rest, &quoted))
        .is_some()
}

/// The input's JSON, which must be an object within the IR input limit, and its bytes.
fn read_presentation_json(
    input: &Input<'_>,
    ctx: &FrontendCtx<'_>,
) -> Result<(Map<String, Value>, Vec<u8>), FrontendError> {
    let limit = ctx.limits.ir_bytes;
    if input.size() > limit {
        return Err(refusal(
            codes::IR0903,
            &[("bytes", input.size().into()), ("limit", limit.into())],
        ));
    }
    let len = usize::try_from(input.size()).map_err(crashed)?;
    let bytes = input.reader.read_at(0, len).map_err(crashed)?.into_owned();
    // A UTF-8 BOM is not part of the text, as a decoder reads it.
    let text = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    let json: Value = std::str::from_utf8(text)
        .map_err(|error| error.to_string())
        .map_err(|error| format!("Not JSON: {error}"))
        .and_then(|text| {
            json::parse(text).map_err(|error| {
                if error.is_data() {
                    error.to_string()
                } else {
                    format!("Not JSON: {error}")
                }
            })
        })
        .map_err(|reason| schema_invalid("(presentation)", reason))?;
    let Value::Object(object) = json else {
        return Err(schema_invalid(
            "(presentation)",
            "A presentation is a JSON object",
        ));
    };
    schema_of(&object)?;
    Ok((object, bytes))
}

/// A document is read only when its `$schema` is the IR's: one naming no schema or another is refused, IR0001 at
/// `$schema`.
fn schema_of(json: &Map<String, Value>) -> Result<SchemaUrl, FrontendError> {
    let reason = match json.get("$schema") {
        None => "missing field `$schema`".to_owned(),
        Some(value) => match SchemaUrl::deserialize(value) {
            Ok(schema) => return Ok(schema),
            Err(error) => error.to_string(),
        },
    };
    Err(schema_invalid("$schema", reason))
}

/// The show order, which must be slides with an id and a hidden flag.
fn slides_of(json: &Map<String, Value>) -> Result<Vec<SlideFacts>, FrontendError> {
    let invalid = || schema_invalid("slides", "Slides need a string id and a hidden flag");
    let slides = json
        .get("slides")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let facts = slides
        .iter()
        .map(|slide| {
            let id = slide.get("id").and_then(Value::as_str);
            let hidden = slide.get("hidden").and_then(Value::as_bool);
            let (Some(id), Some(hidden)) = (id, hidden) else {
                return Err(invalid());
            };
            Ok((id, hidden, slide.get("title").and_then(Value::as_str)))
        })
        .collect::<Result<Vec<_>, _>>()?;
    facts
        .into_iter()
        .enumerate()
        .map(|(index, (id, hidden, title))| {
            let id = NodeId::new(id).map_err(|error| {
                schema_invalid(&format!("slides[{index}].id"), error.to_string())
            })?;
            Ok(SlideFacts {
                id,
                hidden,
                title: title.map(str::to_owned),
            })
        })
        .collect()
}

/// The slide size, which must be numeric. A side off the point grid is rounded to it, and one past the grid's exact
/// range saturates: the verifier reports the value as written (IR0007, IR0008).
fn size_of(json: &Map<String, Value>) -> Result<SlideSize, FrontendError> {
    let invalid = || schema_invalid("size", "The presentation needs a numeric size");
    let size = json
        .get("size")
        .and_then(Value::as_object)
        .ok_or_else(invalid)?;
    let side = |key: &str| -> Result<Size, FrontendError> {
        let x = size.get(key).and_then(Value::as_f64).ok_or_else(invalid)?;
        Ok(Size::quantise(x).unwrap_or_else(|_| {
            let steps = EXACT_STEPS as i64;
            Size::from_steps(if x < 0.0 { -steps } else { steps })
        }))
    };
    Ok(SlideSize {
        w: side("w")?,
        h: side("h")?,
    })
}

/// The bytes the media entries declare.
fn media_bytes_of(json: &Map<String, Value>) -> u64 {
    let Some(media) = json.get("media").and_then(Value::as_object) else {
        return 0;
    };
    let total: f64 = media
        .values()
        .filter_map(|entry| entry.get("bytes")?.as_f64())
        .sum();
    // Saturating: a negative or fractional declaration rounds toward zero.
    total as u64
}

/// The source presentation the JSON is, if it is one: what the census reads.
fn parsed(json: &Value) -> Option<source::Presentation> {
    parse_presentation(json, EMITTER).ok()
}

fn analysis_features(json: &Value) -> Vec<FeatureId> {
    let Some(presentation) = parsed(json) else {
        return Vec::new();
    };
    census_presentation(&presentation)
        .into_values()
        .flatten()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// The container kinds a failed skeleton keeps different fields of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Master,
    Layout,
    Slide,
}

impl Kind {
    fn kept(self) -> &'static [&'static str] {
        match self {
            Kind::Slide => &[
                "id",
                "origin",
                "layout",
                "colorMapOverride",
                "background",
                "ext",
            ],
            Kind::Layout => &[
                "id",
                "origin",
                "name",
                "type",
                "master",
                "colorMapOverride",
                "background",
                "ext",
            ],
            Kind::Master => &[
                "id",
                "origin",
                "name",
                "theme",
                "colorMap",
                "background",
                "ext",
            ],
        }
    }
}

/// The failed skeleton of a container as JSON: its id, place, links and background, no elements.
fn failed_skeleton(kind: Kind, raw: &Map<String, Value>, failed: &Failed) -> Value {
    let mut out = Map::new();
    for key in kind.kept() {
        if let Some(value) = raw.get(*key) {
            out.insert((*key).to_owned(), value.clone());
        }
    }
    let flag = |key: &str, fallback: bool| {
        Value::Bool(raw.get(key).and_then(Value::as_bool).unwrap_or(fallback))
    };
    match kind {
        Kind::Slide => {
            out.insert("hidden".to_owned(), flag("hidden", false));
            out.insert("showLayout".to_owned(), flag("showLayout", true));
            out.insert("timeline".to_owned(), Value::Array(Vec::new()));
        }
        Kind::Layout => {
            out.insert("showMaster".to_owned(), flag("showMaster", true));
        }
        Kind::Master => {}
    }
    out.insert("elements".to_owned(), Value::Array(Vec::new()));
    out.insert(
        "failed".to_owned(),
        serde_json::to_value(failed).unwrap_or(Value::Null),
    );
    Value::Object(out)
}

/// The id a diagnostic names a container by, when it is one.
fn container_id(id: Option<&str>) -> Option<NodeId> {
    id.and_then(|id| NodeId::new(id).ok())
}

/// The per-container bookkeeping of a lower: each dropped kind reported once per container (IR0012).
struct Lowering<'c, 'a> {
    ctx: &'c mut FrontendCtx<'a>,
    noted: BTreeSet<(String, &'static str)>,
}

impl Lowering<'_, '_> {
    /// Strips `value` with `strip`, reporting each kind it drops once for `container`.
    fn strip_with(
        &mut self,
        container: Option<&str>,
        value: &mut Value,
        how: fn(&mut Value, &mut strip::Note<'_>),
    ) {
        let mut fields = Vec::new();
        how(value, &mut |field| fields.push(field));
        self.note(container, fields);
    }

    fn note(&mut self, container: Option<&str>, fields: Vec<&'static str>) {
        for field in fields {
            let key = (container.unwrap_or_default().to_owned(), field);
            if !self.noted.insert(key) {
                continue;
            }
            self.ctx.diag.report(
                codes::IR0012,
                DiagnosticSite::new()
                    .container(container_id(container))
                    .data("field", field),
            );
        }
    }

    /// A container holding an origin a pass created fails (IR0011): authored IR describes sources only.
    fn fail_synth(&mut self, kind: Kind, container: &mut Value) {
        let Value::Object(raw) = &*container else {
            return;
        };
        let Some(id) = raw.get("id").and_then(Value::as_str) else {
            return;
        };
        let Some(path) = synth_origin_path(container) else {
            return;
        };
        self.ctx.diag.report(
            codes::IR0011,
            DiagnosticSite::new()
                .container(container_id(Some(id)))
                .data("path", path),
        );
        let failed = failed_skeleton(kind, raw, &Failed::new(codes::IR0011, None));
        *container = failed;
    }

    /// Strips and checks each container of a `masters` or `layouts` record.
    fn containers(&mut self, record: Option<&mut Value>, kind: Kind) {
        let Some(Value::Object(entries)) = record else {
            return;
        };
        for (key, container) in entries.iter_mut() {
            self.strip_with(Some(key), container, strip);
            self.fail_synth(kind, container);
        }
    }

    fn entries(&mut self, record: Option<&mut Value>, decided: Decided) {
        let Some(record) = record else { return };
        let mut fields = Vec::new();
        strip_entries(record, decided, &mut |field| fields.push(field));
        self.note(None, fields);
    }
}

/// The media hashes of a presentation, sorted by UTF-16 code unit.
fn media_keys(presentation: &Map<String, Value>) -> Vec<String> {
    let mut keys: Vec<String> = presentation
        .get("media")
        .and_then(Value::as_object)
        .map(|media| media.keys().cloned().collect())
        .unwrap_or_default();
    keys.sort_by(|a, b| utf16::cmp(a, b));
    keys
}

/// Agent-written presentation IR as an input.
#[derive(Clone, Debug, Default)]
pub struct IrFrontend {
    media: MediaFiles,
}

impl IrFrontend {
    /// The frontend with the presentation's media files; without them every media item is reported missing
    /// (IR0013).
    pub fn new(media: MediaFiles) -> Self {
        IrFrontend { media }
    }
}

impl Frontend for IrFrontend {
    fn format(&self) -> SourceFormat {
        FORMAT
    }

    fn version(&self) -> &str {
        "1"
    }

    fn sniff(&self, head: &[u8], hints: &SniffHints<'_>) -> bool {
        hints.name.ends_with(IR_FILE_SUFFIX) || has_schema_head(&String::from_utf8_lossy(head))
    }

    fn analyze(
        &self,
        input: &Input<'_>,
        ctx: &mut FrontendCtx<'_>,
    ) -> Result<Analysis, FrontendError> {
        let (json, _) = read_presentation_json(input, ctx)?;
        let size = size_of(&json)?;
        let slides = slides_of(&json)?;
        let media_bytes = media_bytes_of(&json);
        Ok(Analysis {
            format: FORMAT,
            size,
            slides,
            features: analysis_features(&Value::Object(json)),
            media_bytes,
            refusal: None,
        })
    }

    fn lower(
        &self,
        input: &Input<'_>,
        ctx: &mut FrontendCtx<'_>,
        options: &LowerOptions<'_>,
    ) -> Result<LowerResult, FrontendError> {
        let (json, bytes) = read_presentation_json(input, ctx)?;
        // The same refusals as analyze, so the two agree on what they read.
        size_of(&json)?;
        slides_of(&json)?;
        let mut presentation = match options.only {
            Some(only) => {
                let only: Vec<String> = only.iter().map(|id| id.as_str().to_owned()).collect();
                select::select_slides(&json, &only)
            }
            None => json,
        };

        let mut lowering = Lowering {
            ctx,
            noted: BTreeSet::new(),
        };
        if let Some(Value::Array(slides)) = presentation.get_mut("slides") {
            for slide in slides.iter_mut() {
                let id = slide.get("id").and_then(Value::as_str).map(str::to_owned);
                lowering.strip_with(id.as_deref(), slide, strip_slide);
                lowering.fail_synth(Kind::Slide, slide);
            }
        }
        presentation.insert("phase".to_owned(), Value::from("source"));
        if let Some(defaults) = presentation.get_mut("defaults") {
            lowering.strip_with(None, defaults, strip);
        }
        lowering.containers(presentation.get_mut("masters"), Kind::Master);
        lowering.containers(presentation.get_mut("layouts"), Kind::Layout);
        lowering.entries(presentation.get_mut("fonts"), Decided::Binding);
        lowering.entries(presentation.get_mut("media"), Decided::Plan);
        let ctx = lowering.ctx;

        // Media bytes are trusted only after hashing them.
        let mut verified = VerifiedMedia::default();
        for sha256 in media_keys(&presentation) {
            ctx.deadline.check()?;
            let Some(reader) = self.media.open(&sha256) else {
                ctx.diag.report(
                    codes::IR0013,
                    DiagnosticSite::new()
                        .data("sha256", sha256.as_str())
                        .data("reason", "missing"),
                );
                continue;
            };
            let actual =
                slidock_io::sha256_range(reader.as_ref(), 0, reader.len()).map_err(crashed)?;
            match Sha256::parse(&sha256) {
                Ok(parsed) if actual == sha256 => {
                    verified.0.insert(parsed, reader.clone());
                }
                _ => {
                    let short = actual.get(..12).unwrap_or(&actual);
                    ctx.diag.report(
                        codes::IR0013,
                        DiagnosticSite::new()
                            .data("sha256", sha256.as_str())
                            .data("reason", format!("not the bytes hashed ({short}…)")),
                    );
                }
            }
        }

        let presentation = Value::Object(presentation);
        let features = parsed(&presentation)
            .map(|presentation| census_presentation(&presentation))
            .unwrap_or_default();
        Ok(LowerResult {
            provenance: Provenance {
                format: FORMAT,
                name: input.name.to_owned(),
                sha256: Sha256::of(&bytes),
                bytes: input.size(),
                app: None,
            },
            presentation: Unverified::Json(presentation),
            media: Box::new(verified),
            fonts: Vec::new(),
            features,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_the_schema_head_after_a_bom_and_whitespace() {
        let url = SCHEMA_URL;
        for head in [
            format!("{{\"$schema\":\"{url}\""),
            format!("\u{FEFF}  {{\n  \"$schema\" : \"{url}\", \"size\""),
            format!("\u{00A0}{{\u{2028}\"$schema\":\u{3000}\"{url}\""),
        ] {
            assert!(has_schema_head(&head), "{head}");
        }
        for head in [
            format!("{{\"size\":1,\"$schema\":\"{url}\""),
            format!("{{\"$schema\":\"{url}x\""),
            format!("[{{\"$schema\":\"{url}\""),
            "{\"$schema\":\"https://example.com/other.json\"".to_owned(),
        ] {
            assert!(!has_schema_head(&head), "{head}");
        }
    }
}
