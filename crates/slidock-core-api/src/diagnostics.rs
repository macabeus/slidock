//! The diagnostics sink and the output order of diagnostics.

use indexmap::IndexMap;
use serde::Serialize;
use slidock_canon::utf16;
use slidock_ir::diagnostics::{
    Diagnostic, DiagnosticCode, DiagnosticData, DiagnosticSink, DiagnosticSite, Severity, Stage,
    make_diagnostic,
};
use slidock_ir::ids::NodeId;
use slidock_ir::{resolved, source};

/// A sink that appends to `out`, stamping every diagnostic with a stage and an emitter.
#[derive(Debug)]
pub struct Sink<'a> {
    out: &'a mut Vec<Diagnostic>,
    stage: Stage,
    emitter: &'a str,
}

impl<'a> Sink<'a> {
    pub fn new(out: &'a mut Vec<Diagnostic>, stage: Stage, emitter: &'a str) -> Self {
        Sink {
            out,
            stage,
            emitter,
        }
    }
}

impl DiagnosticSink for Sink<'_> {
    fn report(&mut self, code: DiagnosticCode, site: DiagnosticSite) {
        self.out
            .push(make_diagnostic(code, self.stage, self.emitter, site));
    }
}

/// Each container's place in the output order: masters, then layouts, then slides in show order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContainerOrder(IndexMap<NodeId, usize>);

impl ContainerOrder {
    /// The order of these ids; a repeated id keeps its last place.
    pub fn new<'a>(ids: impl IntoIterator<Item = &'a NodeId>) -> Self {
        let mut order = IndexMap::new();
        for (index, id) in ids.into_iter().enumerate() {
            order.insert(id.clone(), index);
        }
        ContainerOrder(order)
    }

    /// The order of a source presentation's containers.
    pub fn of(presentation: Option<&source::Presentation>) -> Self {
        presentation.map_or_else(ContainerOrder::default, |p| {
            ContainerOrder::new(p.containers().map(|c| c.id()))
        })
    }

    /// The order of a resolved presentation's containers.
    pub fn of_resolved(presentation: &resolved::Presentation) -> Self {
        ContainerOrder::new(presentation.container_ids())
    }

    pub fn get(&self, id: &NodeId) -> Option<usize> {
        self.0.get(id).copied()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The compact canonical JSON of a value, or `""` for one it cannot print (a non-finite number in data).
fn canonical<T: Serialize + ?Sized>(value: &T) -> String {
    slidock_canon::to_canonical_json(value, slidock_canon::Layout::Compact).unwrap_or_default()
}

/// What makes two diagnostics the same one: code, container, node and data.
fn merge_key(d: &Diagnostic) -> String {
    #[derive(Serialize)]
    struct Key<'a>(
        DiagnosticCode,
        Option<&'a NodeId>,
        Option<&'a NodeId>,
        Option<&'a DiagnosticData>,
    );
    canonical(&Key(
        d.code,
        d.container.as_ref(),
        d.node.as_ref(),
        d.data.as_ref(),
    ))
}

fn id_str(id: Option<&NodeId>) -> &str {
    id.map_or("", NodeId::as_str)
}

/// Diagnostics in their output order: presentation-level first, then by container (masters, layouts, slides), code,
/// node and data; identical (code, container, node, data) merge into one with a `count`. Strings compare by UTF-16
/// code unit, so the order depends only on the diagnostics and the presentation.
pub fn sort_diagnostics(diagnostics: Vec<Diagnostic>, order: &ContainerOrder) -> Vec<Diagnostic> {
    // Presentation-level diagnostics rank before every container, unknown containers after them.
    let rank = |d: &Diagnostic| {
        d.container
            .as_ref()
            .map_or(0, |id| order.get(id).unwrap_or(order.len()) + 1)
    };
    let mut keyed: Vec<(usize, String, String, Diagnostic)> = diagnostics
        .into_iter()
        .map(|d| (rank(&d), merge_key(&d), canonical(&d), d))
        .collect();
    keyed.sort_by(|(rank_a, key_a, full_a, a), (rank_b, key_b, full_b, b)| {
        rank_a
            .cmp(rank_b)
            .then_with(|| utf16::cmp(id_str(a.container.as_ref()), id_str(b.container.as_ref())))
            .then_with(|| a.code.code().cmp(b.code.code()))
            .then_with(|| utf16::cmp(id_str(a.node.as_ref()), id_str(b.node.as_ref())))
            .then_with(|| utf16::cmp(key_a, key_b))
            .then_with(|| utf16::cmp(full_a, full_b))
    });
    let mut out: Vec<Diagnostic> = Vec::with_capacity(keyed.len());
    let mut previous: Option<String> = None;
    for (_, key, _, d) in keyed {
        match out.last_mut() {
            Some(last) if previous.as_deref() == Some(key.as_str()) => {
                let count = last.count.unwrap_or(1).saturating_add(d.count.unwrap_or(1));
                last.count = Some(count);
            }
            _ => out.push(d),
        }
        previous = Some(key);
    }
    out
}

pub fn has_errors(diagnostics: &[Diagnostic]) -> bool {
    diagnostics.iter().any(|d| d.severity == Severity::Error)
}

/// A diagnostic's data as site entries, in order, for a diagnostic derived from it.
pub fn site_data(data: Option<&DiagnosticData>) -> DiagnosticSite {
    let mut site = DiagnosticSite::new();
    for (key, value) in data.into_iter().flatten() {
        site = site.data(key.clone(), value.clone());
    }
    site
}
