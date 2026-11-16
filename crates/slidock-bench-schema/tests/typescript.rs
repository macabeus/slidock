//! `apps/bench-web/src/schema` holds exactly the TypeScript declarations `slidock_bench_schema::typescript`
//! generates. `UPDATE_GOLDEN=1 cargo test -p slidock-bench-schema --test typescript` rewrites them.
#![allow(clippy::unwrap_used, clippy::panic, reason = "test code")]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use slidock_bench_schema::typescript::{SCHEMA_DIR, files};

fn schema_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(SCHEMA_DIR)
}

/// Every file under `dir`, by name.
fn committed(dir: &Path) -> BTreeMap<String, String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return BTreeMap::new();
    };
    entries
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .map(|path| {
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            (name, std::fs::read_to_string(&path).unwrap())
        })
        .collect()
}

#[test]
fn the_committed_declarations_are_the_generated_ones() {
    let generated = files().unwrap();
    for name in [
        "index.ts",
        "CorpusManifest.ts",
        "Results.ts",
        "RowRecord.ts",
        "RunsIndex.ts",
        "ParityReport.ts",
        "RowDetail.ts",
        "Feature.ts",
        "RowId.ts",
    ] {
        assert!(generated.contains_key(name), "{name}");
    }
    let dir = schema_dir();
    let on_disk = committed(&dir);
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::create_dir_all(&dir).unwrap();
        for name in on_disk.keys().filter(|name| !generated.contains_key(*name)) {
            std::fs::remove_file(dir.join(name)).unwrap();
        }
        for (name, text) in &generated {
            std::fs::write(dir.join(name), text).unwrap();
        }
        return;
    }
    let names: BTreeSet<&String> = generated.keys().chain(on_disk.keys()).collect();
    let drift: Vec<&String> = names
        .into_iter()
        .filter(|name| generated.get(*name) != on_disk.get(*name))
        .collect();
    assert!(
        drift.is_empty(),
        "{SCHEMA_DIR} drifted from the Rust types in {drift:?}: UPDATE_GOLDEN=1 cargo test -p \
         slidock-bench-schema --test typescript rewrites it"
    );
}

#[test]
fn declarations_print_the_wire_shapes() {
    let generated = files().unwrap();
    let file = |name: &str| generated.get(name).unwrap().as_str();
    assert!(file("RowId.ts").contains("export type RowId = string;"));
    assert!(file("Sha256.ts").contains("export type Sha256 = string;"));
    assert!(file("Feature.ts").contains("\"text.run\""));
    let record = file("RowRecord.ts");
    assert!(record.contains("reason?: Reason"), "{record}");
    assert!(record.contains("reference?: true"), "{record}");
    assert!(record.contains("knownLoss?: Array<LossKind>"), "{record}");
    let source = file("Source.ts");
    assert!(source.contains("sha256: Sha256 | null"), "{source}");
    assert!(source.contains("rowCount?: number | null"), "{source}");
    assert!(
        file("Oracle.ts").contains("\"kind\": \"pdf-file\""),
        "{}",
        file("Oracle.ts")
    );
    assert!(file("CorpusManifest.ts").contains("version: 1"));
    assert!(file("ResultsSchema.ts").contains("\"slidock-bench-results@1\""));
    assert!(file("RowOutcome.ts").contains("Outcome | MotionOutcome"));
    assert!(file("FrontendSummary.ts").contains("\"render-error\": number"));
}
