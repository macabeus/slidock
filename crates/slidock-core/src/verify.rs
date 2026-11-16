//! verify-source with isolation, and the invariant checks the pass manager makes.
//!
//! Containers the verifier attributes errors to fail (as bare skeletons when their kept fields are what failed),
//! dependents fail with them, and the rest verifies again. Until the presentation verifies, it is JSON: a container
//! that does not even read as IR is failed in place, in the raw object.

use std::collections::BTreeSet;

use serde_json::{Map, Value};
use slidock_core_api::failure::{
    Causes, FailOptions, FailureCause, fail_containers, with_dependents,
};
use slidock_core_api::{PresentationFacts, Unverified, has_errors};
use slidock_ir::diagnostics::{
    DataValue, Diagnostic, DiagnosticSite, Severity, Stage, codes, make_diagnostic,
};
use slidock_ir::ids::NodeId;
use slidock_ir::presentation::Failed;
use slidock_ir::source::Presentation;
use slidock_ir::verify::{
    CheckOptions, GraphOptions, Invariant, VerifyOptions, check_invariant, parse_presentation,
    validate_presentation, verify_source, verify_source_json,
};

pub const VERIFY_SOURCE: &str = "verify-source";
pub const VERIFY_RESOLVED: &str = "verify-resolved";

/// IR0907: the presentation failed as a whole, in `pass`.
pub(crate) fn presentation_failed(stage: Stage, pass: &str, reason: &str) -> Diagnostic {
    make_diagnostic(
        codes::IR0907,
        stage,
        pass,
        DiagnosticSite::new()
            .data("pass", pass)
            .data("reason", reason),
    )
}

/// The IR0003 kinds each graph invariant other than `source.valid` holds, in the order they are reported.
const GRAPH_KINDS: [(Invariant, &[&str]); 2] = [
    (
        Invariant::PlaceholdersBound,
        &["inherits", "placeholder-target"],
    ),
    (Invariant::TimelineNormalized, &["timeline-target"]),
];

/// Whether an invariant is held by the graph rules rather than a checker of its own.
pub(crate) fn is_graph(invariant: Invariant) -> bool {
    matches!(
        invariant,
        Invariant::SourceValid | Invariant::PlaceholdersBound | Invariant::TimelineNormalized
    )
}

/// The invariant a verifier diagnostic breaks, among those provided so far.
pub(crate) fn invariant_of(diagnostic: &Diagnostic, provided: &[Invariant]) -> Invariant {
    if diagnostic.code == codes::IR0003 {
        let kind = diagnostic
            .data
            .as_ref()
            .and_then(|data| data.get("kind"))
            .map_or_else(|| "undefined".to_owned(), DataValue::to_js_string);
        for (invariant, kinds) in GRAPH_KINDS {
            if kinds.contains(&kind.as_str()) && provided.contains(&invariant) {
                return invariant;
            }
        }
    }
    provided
        .iter()
        .rev()
        .find(|invariant| invariant.rules().contains(&diagnostic.code))
        .copied()
        .unwrap_or(Invariant::SourceValid)
}

/// The theme a schema diagnostic's path is under, if any (`themes.x…` or `themes["x"]…`): its masters fail bare
/// and the theme is dropped.
pub(crate) fn theme_of_path(path: &str) -> Option<String> {
    let rest = path.strip_prefix("themes")?;
    if let Some(rest) = rest.strip_prefix('.') {
        let mut chars = rest.char_indices();
        let first = chars.next()?.1;
        if !(first.is_ascii_alphabetic() || first == '_' || first == '$') {
            return None;
        }
        let end = chars
            .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_' || *c == '$'))
            .map_or(rest.len(), |(i, _)| i);
        return rest.get(..end).map(str::to_owned);
    }
    let quoted = rest.strip_prefix('[')?;
    if !quoted.starts_with('"') {
        return None;
    }
    let mut escaped = false;
    for (i, c) in quoted.char_indices().skip(1) {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            let literal = quoted.get(..=i)?;
            if !quoted.get(i + 1..)?.starts_with(']') {
                return None;
            }
            return serde_json::from_str(literal).ok();
        }
    }
    None
}

/// Schema issues and invariant checker diagnostics, each with the invariant it breaks: bounds (all `source.valid`),
/// then the graph rules, then the checker of every non-graph invariant provided so far.
pub(crate) fn invariant_violations(
    presentation: &Presentation,
    provided: &[Invariant],
    emitter: &str,
    linkable_slides: &[NodeId],
) -> Vec<(Invariant, Diagnostic)> {
    let bounds = validate_presentation(presentation, emitter);
    if !bounds.is_empty() {
        return bounds
            .into_iter()
            .map(|diagnostic| (Invariant::SourceValid, diagnostic))
            .collect();
    }
    let options = CheckOptions {
        graph: GraphOptions {
            emitter: Some(emitter.to_owned()),
            linkable_slides: linkable_slides.to_vec(),
        },
        containers: None,
    };
    let graph = check_invariant(Invariant::SourceValid, presentation, &options)
        .into_iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|diagnostic| (invariant_of(&diagnostic, provided), diagnostic));
    let checked = provided
        .iter()
        .filter(|invariant| !is_graph(**invariant))
        .flat_map(|invariant| {
            check_invariant(*invariant, presentation, &options)
                .into_iter()
                .map(|diagnostic| (*invariant, diagnostic))
        });
    graph.chain(checked).collect()
}

/// What verify-source hands the passes: the verified presentation (failed containers marked), or none when the
/// presentation failed as a whole; and every diagnostic on the way.
#[derive(Clone, Debug, PartialEq)]
pub struct VerifiedSource {
    pub presentation: Option<Presentation>,
    pub diagnostics: Vec<Diagnostic>,
}

/// verify-source, then, unless `native_motion`, the native motion data left out of what verified
/// ([`slidock_ir::effects::strip_native_motion`]): the parity path refuses what the deck path refuses, and prints
/// what verified without it.
pub(crate) fn verify_source_motion(
    input: &Unverified,
    facts: Option<&PresentationFacts>,
    native_motion: bool,
) -> VerifiedSource {
    let mut verified = verify_source_presentation(input, facts);
    if !native_motion && let Some(presentation) = &mut verified.presentation {
        slidock_ir::effects::strip_native_motion(presentation);
    }
    verified
}

/// verify-source with isolation; see the module comment.
pub fn verify_source_presentation(
    input: &Unverified,
    facts: Option<&PresentationFacts>,
) -> VerifiedSource {
    let options = VerifyOptions {
        linkable_slides: facts.map(PresentationFacts::linkable).unwrap_or_default(),
    };
    let mut current = match input {
        Unverified::Typed(presentation) => {
            let diagnostics = verify_source(presentation, &options);
            if !has_errors(&diagnostics) {
                return VerifiedSource {
                    presentation: Some((**presentation).clone()),
                    diagnostics,
                };
            }
            match serde_json::to_value(presentation) {
                Ok(value) => value,
                Err(error) => {
                    let reason = error.to_string();
                    return VerifiedSource {
                        presentation: None,
                        diagnostics: vec![presentation_failed(
                            Stage::Verify,
                            VERIFY_SOURCE,
                            &reason,
                        )],
                    };
                }
            }
        }
        Unverified::Json(value) => value.clone(),
    };
    let mut causes = Causes::new();
    let mut bare: BTreeSet<NodeId> = BTreeSet::new();
    let mut drop_themes: BTreeSet<String> = BTreeSet::new();
    let mut explained: Vec<Diagnostic> = Vec::new();
    loop {
        let diagnostics = verify_source_json(&current, &options);
        let errors: Vec<&Diagnostic> = diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        if errors.is_empty() {
            let mut presentation = match parse_presentation(&current, VERIFY_SOURCE) {
                Ok(presentation) => presentation,
                Err(parsed) => {
                    explained.extend(parsed);
                    return VerifiedSource {
                        presentation: None,
                        diagnostics: explained,
                    };
                }
            };
            let (all, dependents) = with_dependents(&presentation, &causes);
            let fail = FailOptions {
                bare,
                drop_themes: BTreeSet::new(),
            };
            fail_containers(&mut presentation, &all, &fail);
            explained.extend(dependents);
            explained.extend(diagnostics);
            return VerifiedSource {
                presentation: Some(presentation),
                diagnostics: explained,
            };
        }
        if let Some(level) = errors.iter().find(|d| d.container.is_none()) {
            let failed = presentation_failed(Stage::Verify, VERIFY_SOURCE, &level.message);
            explained.extend(errors.into_iter().cloned());
            explained.push(failed);
            return VerifiedSource {
                presentation: None,
                diagnostics: explained,
            };
        }
        let mut progressed = false;
        for error in &errors {
            let Some(id) = &error.container else { continue };
            let theme = error
                .data
                .as_ref()
                .and_then(|data| data.get("path"))
                .and_then(|path| match path {
                    DataValue::Str(path) => theme_of_path(path),
                    _ => None,
                });
            if let Some(theme) = &theme
                && drop_themes.insert(theme.clone())
            {
                progressed = true;
            }
            if !causes.contains_key(id) {
                causes.insert(id.clone(), FailureCause::new((*error).clone()));
                if theme.is_some() {
                    bare.insert(id.clone());
                }
                progressed = true;
                explained.extend(
                    errors
                        .iter()
                        .filter(|d| d.container.as_ref() == Some(id))
                        .map(|d| (*d).clone()),
                );
            } else if bare.insert(id.clone()) {
                progressed = true;
            }
        }
        if !progressed {
            let first = errors
                .first()
                .and_then(|d| d.container.as_ref())
                .map_or("", NodeId::as_str);
            let reason = format!("{first} cannot be represented even as a failed container");
            explained.push(presentation_failed(Stage::Verify, VERIFY_SOURCE, &reason));
            return VerifiedSource {
                presentation: None,
                diagnostics: explained,
            };
        }
        fail_json(&mut current, &causes, &bare, &drop_themes);
    }
}

/// The keys a failed container of each kind keeps, unless it fails bare.
const SLIDE_KEYS: [&str; 6] = [
    "id",
    "origin",
    "layout",
    "colorMapOverride",
    "background",
    "ext",
];
const LAYOUT_KEYS: [&str; 8] = [
    "id",
    "origin",
    "name",
    "type",
    "master",
    "colorMapOverride",
    "background",
    "ext",
];
const MASTER_KEYS: [&str; 7] = [
    "id",
    "origin",
    "name",
    "theme",
    "colorMap",
    "background",
    "ext",
];

#[derive(Clone, Copy)]
enum Kind {
    Master,
    Layout,
    Slide,
}

/// The failed skeleton of a container as JSON, keeping the raw values of what it keeps.
fn failed_json(kind: Kind, raw: &Map<String, Value>, failed: &Failed, bare: bool) -> Value {
    let mut out = Map::new();
    if bare {
        if let Some(id) = raw.get("id") {
            out.insert("id".to_owned(), id.clone());
        }
        let pass = failed.pass.as_deref().unwrap_or("verify");
        out.insert(
            "origin".to_owned(),
            serde_json::json!({ "kind": "synth", "pass": pass }),
        );
    } else {
        let keys: &[&str] = match kind {
            Kind::Master => &MASTER_KEYS,
            Kind::Layout => &LAYOUT_KEYS,
            Kind::Slide => &SLIDE_KEYS,
        };
        for key in keys {
            if let Some(value) = raw.get(*key) {
                out.insert((*key).to_owned(), value.clone());
            }
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

fn cause_of<'a>(causes: &'a Causes, container: &Value) -> Option<&'a FailureCause> {
    let id = container.get("id")?.as_str()?;
    causes.get(&NodeId::new(id).ok()?)
}

fn fail_value(kind: Kind, container: &mut Value, causes: &Causes, bare: &BTreeSet<NodeId>) {
    let Some(cause) = cause_of(causes, container) else {
        return;
    };
    let Some(raw) = container.as_object() else {
        return;
    };
    let is_bare = raw
        .get("id")
        .and_then(Value::as_str)
        .and_then(|id| NodeId::new(id).ok())
        .is_some_and(|id| bare.contains(&id));
    *container = failed_json(kind, raw, &cause.failed(), is_bare);
}

/// [`fail_containers`] on a presentation that is still JSON.
fn fail_json(
    presentation: &mut Value,
    causes: &Causes,
    bare: &BTreeSet<NodeId>,
    drop_themes: &BTreeSet<String>,
) {
    if !drop_themes.is_empty()
        && let Some(themes) = presentation
            .get_mut("themes")
            .and_then(Value::as_object_mut)
    {
        themes.retain(|key, _| !drop_themes.contains(key));
    }
    for (key, kind) in [("masters", Kind::Master), ("layouts", Kind::Layout)] {
        if let Some(record) = presentation.get_mut(key).and_then(Value::as_object_mut) {
            for container in record.values_mut() {
                fail_value(kind, container, causes, bare);
            }
        }
    }
    if let Some(slides) = presentation.get_mut("slides").and_then(Value::as_array_mut) {
        for slide in slides {
            fail_value(Kind::Slide, slide, causes, bare);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_theme_of_a_path() {
        assert_eq!(theme_of_path("themes.t1.colors.dk1"), Some("t1".to_owned()));
        assert_eq!(theme_of_path("themes.t1"), Some("t1".to_owned()));
        assert_eq!(
            theme_of_path("themes[\"ir-theme\"].colors.dk1"),
            Some("ir-theme".to_owned())
        );
        assert_eq!(
            theme_of_path("themes[\"a\\\"b\"].x"),
            Some("a\"b".to_owned())
        );
        assert_eq!(theme_of_path("themes[\"ir-theme\"x"), None);
        assert_eq!(theme_of_path("themes.1x"), None);
        assert_eq!(theme_of_path("masters[\"ir-master\"]"), None);
        assert_eq!(theme_of_path("(presentation)"), None);
    }

    fn diagnostic(code: slidock_ir::diagnostics::DiagnosticCode, kind: Option<&str>) -> Diagnostic {
        let site = match kind {
            Some(kind) => DiagnosticSite::new().data("kind", kind),
            None => DiagnosticSite::new(),
        };
        make_diagnostic(code, Stage::Verify, "test", site)
    }

    #[test]
    fn attributes_a_diagnostic_to_the_last_invariant_owning_its_rule() {
        let all = Invariant::ALL;
        let dangling = diagnostic(codes::IR0003, Some("inherits"));
        assert_eq!(invariant_of(&dangling, &all), Invariant::PlaceholdersBound);
        let timeline = diagnostic(codes::IR0003, Some("timeline-target"));
        assert_eq!(invariant_of(&timeline, &all), Invariant::TimelineNormalized);
        assert_eq!(
            invariant_of(&timeline, &[Invariant::SourceValid]),
            Invariant::SourceValid
        );
        // IR0003 of another kind: the last provided invariant listing IR0003 owns it.
        let layout = diagnostic(codes::IR0003, Some("layout"));
        assert_eq!(invariant_of(&layout, &all), Invariant::TimelineNormalized);
        let frames = diagnostic(codes::IR0105, None);
        assert_eq!(invariant_of(&frames, &all), Invariant::GeometryLowered);
        assert_eq!(
            invariant_of(
                &frames,
                &[Invariant::SourceValid, Invariant::FramesCanonical]
            ),
            Invariant::FramesCanonical
        );
        assert_eq!(
            invariant_of(&diagnostic(codes::IR0109, None), &[Invariant::SourceValid]),
            Invariant::SourceValid
        );
    }
}
