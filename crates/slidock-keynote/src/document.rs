//! Opening a Keynote document: the package, its object graph and the show it describes. The graph is read through a
//! counting reader, so what the index costs is measured apart from the media, which lowering hashes later (the 1 GB
//! corpus presentation reads about 3 MiB of index).

use std::fmt;

use slidock_core_api::{FrontendError, Input, Limits};
use slidock_io::{CountingReader, RangeReader};
use slidock_ir::diagnostics::{DiagnosticSink, DiagnosticSite, codes};

use crate::graph::{GraphIssue, IndexBudget, ObjectGraph, read_object_graph};
use crate::package::{KeynotePackage, refusal};
use crate::proto::kn;

/// The input as the document reads it: every read counted.
pub type IndexReader<'a> = CountingReader<&'a dyn RangeReader>;

pub struct KeynoteDocument<'a> {
    pub pkg: KeynotePackage<IndexReader<'a>>,
    pub graph: ObjectGraph,
    /// The show of the first `KN.DocumentArchive`; it has a slide tree.
    pub show: kn::ShowArchive,
}

impl fmt::Debug for KeynoteDocument<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeynoteDocument")
            .field("graph", &self.graph)
            .field("show", &self.show)
            .finish_non_exhaustive()
    }
}

impl KeynoteDocument<'_> {
    /// The bytes the package directory and the index cost.
    pub fn index_bytes_read(&self) -> u64 {
        self.pkg.reader().bytes_read()
    }
}

/// Opens the document, or refuses it: a package refusal (KN1001–KN1003), an index over its budget (KN1004) or a
/// missing slide tree (KN1003).
pub fn open_document<'a>(
    input: &Input<'a>,
    limits: &Limits,
) -> Result<KeynoteDocument<'a>, FrontendError> {
    let reader: IndexReader<'a> = CountingReader::new(input.reader);
    let pkg = KeynotePackage::open(input.name, reader, limits.zip)?;
    let graph = read_object_graph(&pkg, &IndexBudget::default())
        .map_err(|error| refusal(input.name, codes::KN1004, Some(&error.0)))?;
    let show = graph
        .of_type::<kn::DocumentArchive>()
        .next()
        .and_then(|(_, document, _)| graph.get::<kn::ShowArchive>(document.show.as_ref()))
        .filter(|show| show.slide_tree.is_some())
        .cloned()
        .ok_or_else(|| {
            refusal(
                input.name,
                codes::KN1003,
                Some("the document has no slide tree"),
            )
        })?;
    Ok(KeynoteDocument { pkg, graph, show })
}

/// What reading the index passed over, as presentation-level diagnostics.
pub fn report_index(graph: &ObjectGraph, diag: &mut dyn DiagnosticSink) {
    for issue in &graph.issues {
        match issue {
            GraphIssue::MergeUnapplied { id, file } => diag.report(
                codes::KN1005,
                DiagnosticSite::new()
                    .data("id", id.to_string())
                    .data("file", file),
            ),
            GraphIssue::FileUnreadable { file, reason } => diag.report(
                codes::KN1007,
                DiagnosticSite::new()
                    .data("file", file)
                    .data("reason", reason),
            ),
        }
    }
    let extra = graph.stats.extra_payloads;
    if extra > 0 {
        diag.report(codes::KN1006, DiagnosticSite::new().data("objects", extra));
    }
}
