//! `packages/types/src/generated` holds exactly the TypeScript declarations the worker protocol generates: its own,
//! the bundle report's, the deck JSON's and the IR values they carry, each folder with its `index.ts`; and
//! `results::DECLARATIONS` declares the wasm module's results as `ts-rs` prints them.
//! `UPDATE_GOLDEN=1 cargo test -p slidock-wasm --test typescript` rewrites both.
#![allow(clippy::unwrap_used, clippy::panic, reason = "test code")]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use slidock_html::typescript::{Bindings, GENERATED_DIR};
use slidock_wasm::protocol::{FromWorker, ToWorker};
use slidock_wasm::results::{
    DECLARATIONS, DocumentsJson, FontJson, MediaJson, MediaNeedJson, OpenedJson, PrintedJson,
    RangeJson, ZipEntryJson,
};
use ts_rs::{Config, TS, TypeVisitor};

fn generated_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/types/src")
        .join(GENERATED_DIR)
}

/// Every file under `dir`, by `/`-separated path relative to it.
fn committed(dir: &Path) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let relative = path
                .strip_prefix(dir)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            files.insert(relative, std::fs::read_to_string(&path).unwrap());
        }
    }
    files
}

#[test]
fn the_committed_declarations_are_the_generated_ones() {
    let mut bindings = Bindings::new();
    bindings
        .add::<ToWorker>()
        .unwrap()
        .add::<FromWorker>()
        .unwrap();
    let generated = bindings.files();
    for path in [
        "worker/ToWorker.ts",
        "worker/index.ts",
        "bundle/BundleReport.ts",
        "deck/Deck.ts",
        "ir/Trigger.ts",
    ] {
        assert!(generated.contains_key(path), "{path}");
    }
    let root = generated_dir();
    let on_disk = committed(&root);
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        for path in on_disk.keys() {
            std::fs::remove_file(root.join(path)).unwrap();
        }
        for (path, text) in &generated {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        return;
    }
    let drift: BTreeSet<&String> = generated
        .keys()
        .chain(on_disk.keys())
        .filter(|path| generated.get(*path) != on_disk.get(*path))
        .collect();
    assert!(
        drift.is_empty(),
        "packages/types drifted from the Rust types in {drift:?}; rerun with UPDATE_GOLDEN=1"
    );
}

/// The names of the `@slidock/types` declarations a type names.
struct Imports(BTreeSet<String>);

impl TypeVisitor for Imports {
    fn visit<T: TS + 'static + ?Sized>(&mut self) {
        if T::output_path().is_some() {
            self.0.insert(T::ident(&Config::new()));
        }
    }
}

/// One declaration of the module's `.d.ts`: the type's docs, then the type.
fn declaration<T: TS + 'static + ?Sized>(imports: &mut Imports) -> String {
    T::visit_dependencies(imports);
    let config = Config::new().with_large_int("number");
    format!(
        "{}export {}\n",
        T::docs().unwrap_or_default(),
        T::decl(&config)
    )
}

fn declarations() -> String {
    // The stages' parameters name these besides the results' fields.
    let mut imports = Imports(BTreeSet::from(["AddedFontSource".to_owned()]));
    imports.0.insert("BundleReport".to_owned());
    imports.0.insert("GoogleFace".to_owned());
    imports.0.insert("LocalPick".to_owned());
    imports.0.insert("PlannedImage".to_owned());
    let body = [
        declaration::<OpenedJson>(&mut imports),
        declaration::<MediaNeedJson>(&mut imports),
        declaration::<RangeJson>(&mut imports),
        declaration::<MediaJson>(&mut imports),
        declaration::<FontJson>(&mut imports),
        declaration::<PrintedJson<'static>>(&mut imports),
        declaration::<DocumentsJson<'static>>(&mut imports),
        declaration::<ZipEntryJson>(&mut imports),
    ];
    let names: Vec<&str> = imports
        .0
        .iter()
        .map(String::as_str)
        .filter(|name| {
            ![
                "MediaRange",
                "OpenedMediaNeed",
                "PrintedMedia",
                "PrintedFont",
            ]
            .contains(name)
        })
        .collect();
    format!(
        "\nimport type {{ {} }} from '@slidock/types';\n\n{}",
        names.join(", "),
        body.join("\n")
    )
}

#[test]
fn the_module_declares_its_results_as_ts_rs_prints_them() {
    let expected = declarations();
    assert!(
        expected.contains("export type Printed = { deck: Deck,"),
        "{expected}"
    );
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/results.rs");
        let source = std::fs::read_to_string(&path).unwrap();
        let start = "pub const DECLARATIONS: &str = r#\"";
        let (head, rest) = source.split_once(start).unwrap();
        let (_, tail) = rest.split_once("\"#;").unwrap();
        std::fs::write(&path, format!("{head}{start}{expected}\"#;{tail}")).unwrap();
        return;
    }
    assert!(
        DECLARATIONS == expected,
        "results::DECLARATIONS drifted from the Rust types; rerun with UPDATE_GOLDEN=1:\n{expected}"
    );
}
