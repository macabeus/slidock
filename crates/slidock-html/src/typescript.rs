//! The TypeScript declarations `packages/types` holds for the Rust types that cross into JavaScript (the deck JSON, the
//! IR values it carries, the worker protocol), generated with `ts-rs`: one file per type under a folder per crate,
//! and an `index.ts` per folder. Tests compare them with the committed files (and rewrite them with
//! `UPDATE_GOLDEN=1`); nothing here touches the file system.

use std::any::TypeId;
use std::collections::{BTreeMap, BTreeSet};

use ts_rs::{Config, ExportError, TS, TypeVisitor};

/// The folder under `packages/types/src` the generated files go in.
pub const GENERATED_DIR: &str = "generated";

/// Generated declarations by path under [`GENERATED_DIR`] (`deck/Deck.ts`).
#[derive(Clone, Debug, Default)]
pub struct Bindings {
    files: BTreeMap<String, String>,
    seen: BTreeSet<TypeId>,
}

/// How `ts-rs` prints: JSON numbers for 64-bit integers, imports without an extension.
fn config() -> Config {
    Config::new().with_large_int("number")
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
    pub fn new() -> Bindings {
        Bindings::default()
    }

    /// Adds the declaration of `T` and of every type it names.
    pub fn add<T: TS + 'static + ?Sized>(&mut self) -> Result<&mut Bindings, ExportError> {
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

    /// Every file: the declarations, and each folder's `index.ts` re-exporting them.
    pub fn files(&self) -> BTreeMap<String, String> {
        let mut files = self.files.clone();
        let mut folders: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for path in self.files.keys() {
            if let Some((folder, file)) = path.rsplit_once('/') {
                folders
                    .entry(folder)
                    .or_default()
                    .push(file.strip_suffix(".ts").unwrap_or(file));
            }
        }
        for (folder, names) in folders {
            let mut index = String::from(
                "// Generated from the Rust types by ts-rs (see `slidock_html::typescript`). Do not edit.\n",
            );
            for name in names {
                index.push_str(&format!("export type * from './{name}';\n"));
            }
            files.insert(format!("{folder}/index.ts"), index);
        }
        files
    }
}
