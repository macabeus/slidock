//! Schema issues as diagnostics: IR0008 for bounds with `{ what, path, limit }`, other codes with `{ path, reason }`.
//! An issue belongs to the slide, layout or master on its path (a theme's to each master using it), and names the
//! innermost element on its path. Containers and nodes are [`NodeId`]s, as the diagnostic schema declares them: an
//! issue on a slide or element whose own id is malformed names none.

use serde_json::Value;

use crate::diagnostics::{Diagnostic, DiagnosticCode, DiagnosticSite, Stage, make_diagnostic};
use crate::ids::NodeId;
use crate::parse::StructureError;
use crate::path::{IrPath, PathSeg};
use crate::source::Presentation;
use crate::validate::{Issue, IssueDetail};

/// What the containers of an issue are looked up in: the input JSON, or a typed presentation.
#[derive(Clone, Copy)]
pub(crate) enum Lookup<'a> {
    Json(&'a Value),
    Typed(&'a Presentation),
}

impl Lookup<'_> {
    fn slide_id(&self, index: usize) -> Option<NodeId> {
        match self {
            Lookup::Json(value) => value
                .get("slides")
                .and_then(|slides| slides.get(index))
                .and_then(|slide| slide.get("id"))
                .and_then(Value::as_str)
                .and_then(|id| NodeId::new(id).ok()),
            Lookup::Typed(p) => p.slides.get(index).map(|slide| slide.id.clone()),
        }
    }

    fn masters_using(&self, theme: &str) -> Vec<NodeId> {
        match self {
            Lookup::Json(value) => value
                .get("masters")
                .and_then(Value::as_object)
                .map(|masters| {
                    masters
                        .iter()
                        .filter(|(_, master)| {
                            master.get("theme").and_then(Value::as_str) == Some(theme)
                        })
                        .filter_map(|(key, _)| NodeId::new(key.clone()).ok())
                        .collect()
                })
                .unwrap_or_default(),
            Lookup::Typed(p) => p
                .masters
                .iter()
                .filter(|(_, master)| master.theme.as_ref().is_some_and(|t| t.as_str() == theme))
                .map(|(key, _)| key.clone())
                .collect(),
        }
    }

    /// The containers an issue at `path` belongs to: the slide, layout or master on it; a theme's belong to the
    /// masters using it (or to none).
    fn containers(&self, path: &IrPath) -> Vec<Option<NodeId>> {
        match path.segments() {
            [PathSeg::Key(head), PathSeg::Index(index), ..] if head == "slides" => {
                vec![self.slide_id(*index)]
            }
            [PathSeg::Key(head), PathSeg::Key(key), ..]
                if head == "layouts" || head == "masters" =>
            {
                vec![NodeId::new(key.to_string()).ok()]
            }
            [PathSeg::Key(head), PathSeg::Key(key), ..] if head == "themes" => {
                let masters = self.masters_using(key);
                if masters.is_empty() {
                    vec![None]
                } else {
                    masters.into_iter().map(Some).collect()
                }
            }
            _ => vec![None],
        }
    }
}

/// The innermost element on `path` in the input JSON: an object under `elements` or `children` below the
/// container.
fn node_of(path: &IrPath, input: &Value) -> Option<NodeId> {
    let mut node = None;
    let mut current = input;
    let segments = path.segments();
    for (index, seg) in segments.iter().enumerate() {
        let next = match seg {
            PathSeg::Key(key) => current.get(key.as_ref()),
            PathSeg::Index(i) => current.get(*i),
        };
        let Some(next) = next else { break };
        current = next;
        let parent = index.checked_sub(1).and_then(|i| segments.get(i));
        let in_elements =
            matches!(parent, Some(PathSeg::Key(key)) if key == "elements" || key == "children");
        if in_elements
            && index > 2
            && let Some(id) = current.get("id").and_then(Value::as_str)
        {
            node = NodeId::new(id).ok();
        }
    }
    node
}

fn emit(
    out: &mut Vec<Diagnostic>,
    emitter: &str,
    code: DiagnosticCode,
    path: &IrPath,
    node: Option<NodeId>,
    detail: &IssueDetail,
    lookup: Lookup<'_>,
) {
    for container in lookup.containers(path) {
        let mut site = DiagnosticSite::new()
            .container(container)
            .node(node.clone());
        site = match detail {
            IssueDetail::Bound { what, limit } => site
                .data("what", *what)
                .data("path", path.to_string())
                .data("limit", limit.as_str()),
            IssueDetail::Reason(reason) => site
                .data("path", path.to_string())
                .data("reason", reason.as_str()),
        };
        out.push(make_diagnostic(code, Stage::Verify, emitter, site));
    }
}

/// The diagnostics of a structural error in `input`.
pub(crate) fn structure_diagnostics(
    error: &StructureError,
    input: &Value,
    emitter: &str,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let node = node_of(&error.path, input);
    for issue in &error.issues {
        emit(
            &mut out,
            emitter,
            issue.code,
            &error.path,
            node.clone(),
            &issue.detail,
            Lookup::Json(input),
        );
    }
    out
}

/// The diagnostics of bound issues found on a parsed presentation.
pub(crate) fn issue_diagnostics(
    issues: &[Issue],
    presentation: &Presentation,
    emitter: &str,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for issue in issues {
        emit(
            &mut out,
            emitter,
            issue.code,
            &issue.path,
            issue.node.clone(),
            &issue.detail,
            Lookup::Typed(presentation),
        );
    }
    out
}
