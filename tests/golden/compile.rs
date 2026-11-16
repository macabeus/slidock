//! A corpus pair compiled in process, from a file reader through a frontend crate's own [`Session`], against what
//! `slidock compile` recorded for it (`corpus/<pair>/`): the resolved IR as `--emit ir` dumps it, every document and
//! medium of the document set, and the report, byte for byte. Included by path next to `support.rs` (as `mod golden`)
//! by the frontend crates' corpus suites.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test support"
)]

use std::path::PathBuf;
use std::sync::Arc;

use slidock_core::{Finished, Session};
use slidock_core_api::visit::slide_shows;
use slidock_core_api::{ContainerOrder, MediaSource, sort_diagnostics};
use slidock_fonts::{CatalogFace, FaceFile, FontCatalog, builtin_catalog, read_sfnt};
use slidock_format::manifest::Generator;
use slidock_html::{EmitOptions, MediaAction, Provenance as HtmlProvenance, emit_documents};
use slidock_io::RangeReader;
use slidock_ir::ids::Sha256;

use super::golden::{Golden, Pair};

/// The font files of the static lane's set, in path order.
pub fn font_files(golden: &Golden) -> Vec<Vec<u8>> {
    let dir = golden.fonts();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            let name = path.to_string_lossy().to_ascii_lowercase();
            [".ttf", ".otf", ".ttc"]
                .iter()
                .any(|ext| name.ends_with(ext))
        })
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no fonts in {}", dir.display());
    paths
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect()
}

/// The catalog `--fonts` binds against: the built-in metrics, then every face of the static lane's set, kept with its
/// file.
pub fn font_catalog(golden: &Golden) -> FontCatalog {
    let mut faces = Vec::new();
    for bytes in font_files(golden) {
        let sha256 = Sha256::of(&bytes);
        let faces_of = read_sfnt(&bytes).unwrap();
        let file: Arc<[u8]> = Arc::from(bytes);
        for (face, index) in faces_of.iter().zip(0u32..) {
            faces.push((
                face.family.clone(),
                CatalogFace::from_sfnt(face, Some(sha256))
                    .with_file(FaceFile::new(file.clone(), index)),
            ));
        }
    }
    FontCatalog::merge([builtin_catalog(), &FontCatalog::build(faces)])
}

/// The pairs of one frontend.
pub fn pairs_of(golden: &Golden, frontend: &str) -> Vec<Pair> {
    golden
        .pairs()
        .iter()
        .filter(|pair| pair.frontend == frontend)
        .cloned()
        .collect()
}

/// A medium as a host writes it: its item's bytes, copied or transcoded to PNG.
fn media_bytes(source: &MediaSource, input: &dyn RangeReader, action: MediaAction) -> Vec<u8> {
    let bytes = match source {
        MediaSource::InputRange { offset, len } => input
            .read_at(*offset, usize::try_from(*len).unwrap())
            .unwrap()
            .into_owned(),
        MediaSource::Bytes(bytes) => bytes.to_vec(),
        MediaSource::Reader(reader) => reader
            .read_at(0, usize::try_from(reader.len()).unwrap())
            .unwrap()
            .into_owned(),
    };
    match action {
        MediaAction::Copy => bytes,
        MediaAction::Transcode => slidock_io::transcode_to_png(&bytes, 50_000_000).unwrap(),
    }
}

/// A finished session against the pair's recorded outputs; returns the differences and the files compared.
pub fn compare_pair(
    golden: &Golden,
    pair: &Pair,
    session: &Session<'_>,
    finished: &Finished,
    input: &dyn RangeReader,
) -> (Vec<String>, usize) {
    let name = &pair.name;
    let prefix = format!("corpus/{name}");
    let presentation = finished
        .presentation
        .as_ref()
        .unwrap_or_else(|| panic!("{name}: nothing resolved: {:?}", finished.diagnostics));
    let mut problems = Vec::new();
    let mut written = Vec::new();
    let mut check = |path: &str, bytes: &[u8]| {
        written.push(path.to_owned());
        problems.extend(golden.compare(&format!("{prefix}/{path}"), bytes));
    };

    // The resolved IR, as `--emit ir` dumps it.
    let ir = slidock_ir::canonical::dump_presentation(presentation).unwrap();
    check("presentation-ir.json", ir.as_bytes());

    // The document set, named as the CLI names its generator.
    let provenance = session.provenance().clone();
    let emit = EmitOptions {
        provenance: HtmlProvenance {
            format: provenance.format.as_str().to_owned(),
            name: provenance.name.clone(),
            sha256: provenance.sha256.to_hex(),
            bytes: provenance.bytes,
        },
        generator: Generator {
            name: "slidock".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        },
    };
    let emitted = emit_documents(presentation, &emit).unwrap();
    for (path, text) in &emitted.files.text {
        check(path, text.as_bytes());
    }
    for (path, plan) in &emitted.files.media {
        let handle = session.media(&plan.sha256).unwrap();
        check(path, &media_bytes(&handle.source, input, plan.action));
    }

    // The report.
    let mut diagnostics = finished.diagnostics.clone();
    diagnostics.extend(emitted.diagnostics.iter().cloned());
    let diagnostics = sort_diagnostics(diagnostics, &ContainerOrder::of_resolved(presentation));
    let report = serde_json::json!({
        "provenance": provenance,
        "diagnostics": diagnostics,
        "features": session.features(),
        "shows": slide_shows(&presentation.to_source().unwrap()),
    });
    let report =
        slidock_canon::to_canonical_json(&report, slidock_canon::Layout::Indented).unwrap();
    check("report.json", report.as_bytes());

    let compared = written.len();
    problems.extend(golden.compare_paths(&prefix, written));
    (problems, compared)
}
