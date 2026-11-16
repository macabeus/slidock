//! The TypeScript declarations the bench UI reads (`apps/bench-web/src/schema`), generated with `ts-rs`: one file
//! per type and an `index.ts` re-exporting them. The crate's `typescript` test compares them with the committed
//! files (and rewrites them with `UPDATE_GOLDEN=1`); nothing here touches the file system.

use std::any::TypeId;
use std::collections::{BTreeMap, BTreeSet};

use ts_rs::{Config, ExportError, TS, TypeVisitor};

use crate::api::{
    HistogramEntry, HoldoutView, ParityView, PublicManifest, ResultsView, RowDetail, Status,
};
use crate::compare::{DiffReport, Movement, RegressionReport};
use crate::corpus::CorpusManifest;
use crate::parity::{ParityReference, ParityReport};
use crate::results::Results;
use crate::runs::RunsIndex;

/// The folder the declarations go in, relative to the repository root.
pub const SCHEMA_DIR: &str = "apps/bench-web/src/schema";

/// How `ts-rs` prints: JSON numbers for 64-bit integers, imports without an extension.
fn config() -> Config {
    Config::new().with_large_int("number")
}

#[derive(Default)]
struct Bindings {
    files: BTreeMap<String, String>,
    seen: BTreeSet<TypeId>,
}

struct Visit<'a> {
    bindings: &'a mut Bindings,
    error: Option<ExportError>,
}

impl TypeVisitor for Visit<'_> {
    fn visit<T: TS + 'static + ?Sized>(&mut self) {
        if self.error.is_none() && T::output_path().is_some() {
            self.error = self.bindings.add::<T>().err();
        }
    }
}

impl Bindings {
    fn add<T: TS + 'static + ?Sized>(&mut self) -> Result<&mut Bindings, ExportError> {
        if !self.seen.insert(TypeId::of::<T>()) {
            return Ok(self);
        }
        let path = T::output_path()
            .ok_or(ExportError::CannotBeExported(std::any::type_name::<T>()))?
            .to_string_lossy()
            .replace('\\', "/");
        self.files.insert(path, T::export_to_string(&config())?);
        let mut visit = Visit {
            bindings: self,
            error: None,
        };
        T::visit_dependencies(&mut visit);
        match visit.error {
            Some(error) => Err(error),
            None => Ok(self),
        }
    }
}

/// Every declaration file by name (`Results.ts`), plus `index.ts`: the persisted files, the reports and the API
/// responses, and every type they name.
pub fn files() -> Result<BTreeMap<String, String>, ExportError> {
    let mut bindings = Bindings::default();
    bindings
        .add::<CorpusManifest>()?
        .add::<Results>()?
        .add::<RunsIndex>()?
        .add::<RegressionReport>()?
        .add::<DiffReport>()?
        .add::<Movement>()?
        .add::<ParityReference>()?
        .add::<ParityReport>()?
        .add::<Status>()?
        .add::<PublicManifest>()?
        .add::<ResultsView>()?
        .add::<HoldoutView>()?
        .add::<HistogramEntry>()?
        .add::<RowDetail>()?
        .add::<ParityView>()?;
    let mut files = bindings.files;
    let mut index = String::from(
        "// Generated from the Rust types by ts-rs (see `slidock_bench_schema::typescript`). Do not edit.\n",
    );
    for name in files.keys() {
        let module = name.strip_suffix(".ts").unwrap_or(name);
        index.push_str(&format!("export type * from './{module}';\n"));
    }
    files.insert("index.ts".to_owned(), index);
    Ok(files)
}
