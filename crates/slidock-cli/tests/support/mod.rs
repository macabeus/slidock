//! What the corpus tests share: the golden files (tests/golden; a suite skips when they are absent), the corpus pairs
//! they pin, and file trees read back.
#![allow(dead_code, reason = "each test file uses part of it")]

use std::path::Path;
use std::sync::OnceLock;

use serde_json::Value;

#[path = "../../../../tests/golden/support.rs"]
pub mod golden;

#[allow(unused_imports, reason = "each test file uses part of it")]
pub use golden::{Golden, Pair, read_tree};

/// The golden files, opened once; `None` when they are absent.
pub fn golden() -> Option<&'static Golden> {
    static GOLDEN: OnceLock<Option<Golden>> = OnceLock::new();
    GOLDEN.get_or_init(|| Golden::open("slidock-cli")).as_ref()
}

pub fn read_json(path: &Path) -> Value {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).unwrap()
}

/// The corpus pairs of the golden files, the repository's probes aside, by name.
pub fn corpus_pairs(golden: &Golden) -> Vec<Pair> {
    golden
        .pairs()
        .iter()
        .filter(|pair| !pair.input.starts_with("repo:"))
        .cloned()
        .collect()
}
