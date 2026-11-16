//! A Keynote document is a package: `Index/` (the object graph), `Data/` (media) and `Metadata/`. The single-file
//! `.key` is a zip of it, rooted at the top level or one folder down (a zipped package folder). Paths here are
//! relative to the package root. Keynote stores its entries uncompressed, so a media file is a slice of the input,
//! read in place.

use std::borrow::Cow;

use indexmap::{IndexMap, IndexSet};
use slidock_core_api::FrontendError;
use slidock_io::zip::EntryReader;
use slidock_io::{RangeReader, ZipArchive, ZipEntry, ZipError, ZipLimits};
use slidock_ir::diagnostics::{DiagnosticCode, DiagnosticSite, Stage, codes, make_diagnostic};

use crate::graph::{IndexSource, ReadMany};

/// The marker of a package's root: exactly one must exist.
pub const DOCUMENT_PATH: &str = "Index/Document.iwa";
/// Keynote 6 and older.
const LEGACY_INDEX: [&str; 3] = ["Index.zip", "index.apxl", "index.apxl.gz"];
/// iWork writes a password verifier (and optionally a hint) beside an encrypted index.
const PASSWORD_FILES: [&str; 2] = [".iwpv2", ".iwph"];

/// Who reports the frontend's diagnostics.
pub const EMITTER: &str = "keynote";

/// The whole input refused, with the code that says why.
pub fn refusal(name: &str, code: DiagnosticCode, reason: Option<&str>) -> FrontendError {
    let site = DiagnosticSite::new()
        .data("name", name)
        .data_opt("reason", reason);
    FrontendError::refused(make_diagnostic(code, Stage::Frontend, EMITTER, site))
}

/// `""` when `marker` is at the top level, `"<folder>/"` when it is one folder down, and `None` when it is neither or
/// both.
pub fn package_root<'a>(names: impl IntoIterator<Item = &'a str>, marker: &str) -> Option<&'a str> {
    let mut roots = IndexSet::new();
    for name in names {
        if name == marker {
            roots.insert("");
        } else if let Some(root) = name.strip_suffix(marker)
            && root.ends_with('/')
            && root.find('/') == Some(root.len() - 1)
        {
            roots.insert(root);
        }
    }
    match roots.len() {
        1 => roots.into_iter().next(),
        _ => None,
    }
}

/// Whether a zip's entry names hold one Keynote package.
pub fn is_keynote_package<'a>(names: impl IntoIterator<Item = &'a str>) -> bool {
    package_root(names, DOCUMENT_PATH).is_some()
}

/// A `.key` zip opened at its package root.
#[derive(Debug)]
pub struct KeynotePackage<R> {
    zip: ZipArchive<R>,
    root: String,
    /// Indexes into the zip's entries, by path relative to the package root, in directory order.
    entries: IndexMap<String, usize>,
}

impl<R: RangeReader> KeynotePackage<R> {
    /// Opens the package over `reader`. Refuses a file that is not a zip or lacks a single `Index/Document.iwa`
    /// (KN1003), was saved by Keynote 6 or older (KN1001), or is password-protected (KN1002).
    pub fn open(name: &str, reader: R, limits: ZipLimits) -> Result<Self, FrontendError> {
        let zip = ZipArchive::open(reader, limits)
            .map_err(|error| refusal(name, codes::KN1003, Some(&error.message)))?;
        let files = || {
            zip.entries()
                .iter()
                .map(|entry| entry.name.as_str())
                .filter(|name| !name.ends_with('/'))
        };
        let Some(root) = package_root(files(), DOCUMENT_PATH).map(str::to_owned) else {
            if LEGACY_INDEX
                .iter()
                .any(|marker| package_root(files(), marker).is_some())
            {
                return Err(refusal(name, codes::KN1001, None));
            }
            let reason = format!("the package has no single {DOCUMENT_PATH}");
            return Err(refusal(name, codes::KN1003, Some(&reason)));
        };
        let mut entries = IndexMap::new();
        for (index, entry) in zip.entries().iter().enumerate() {
            if entry.name.ends_with('/') {
                continue;
            }
            if let Some(path) = entry.name.strip_prefix(root.as_str()) {
                entries.insert(path.to_owned(), index);
            }
        }
        if PASSWORD_FILES
            .iter()
            .any(|path| entries.contains_key(*path))
        {
            return Err(refusal(name, codes::KN1002, None));
        }
        Ok(KeynotePackage { zip, root, entries })
    }

    /// Entries by path relative to the package root, in directory order.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &ZipEntry)> {
        self.entries.iter().filter_map(|(path, &index)| {
            self.zip
                .entries()
                .get(index)
                .map(|entry| (path.as_str(), entry))
        })
    }

    pub fn entry(&self, path: &str) -> Option<&ZipEntry> {
        self.entries
            .get(path)
            .and_then(|&index| self.zip.entries().get(index))
    }

    /// One entry's bytes in memory; `None` when the package lacks it.
    pub fn read(&self, path: &str) -> Option<Result<Cow<'_, [u8]>, ZipError>> {
        self.entry(path).map(|entry| self.zip.read(entry))
    }

    /// One entry as a reader: a slice of the input when stored.
    pub fn open_entry(&self, path: &str) -> Option<Result<EntryReader<'_, R>, ZipError>> {
        self.entry(path).map(|entry| self.zip.open_entry(entry))
    }

    /// Several entries' bytes, neighbours read as one range. Paths the package lacks are skipped; entries the zip
    /// fails to give (damaged, or over a limit) go to `failed` with their reason.
    pub fn read_many(&self, paths: &[&str]) -> ReadMany<'_> {
        let wanted: Vec<&ZipEntry> = paths.iter().filter_map(|path| self.entry(path)).collect();
        let mut out = ReadMany::default();
        match self.zip.read_many(&wanted) {
            Ok(read) => {
                for (name, bytes) in read {
                    out.read.insert(self.relative(&name).to_owned(), bytes);
                }
            }
            // One entry spoiled the batch: read each on its own to keep the others.
            Err(_) => {
                for entry in wanted {
                    let path = self.relative(&entry.name).to_owned();
                    match self.zip.read(entry) {
                        Ok(bytes) => {
                            out.read.insert(path, bytes);
                        }
                        Err(error) => {
                            out.failed.insert(path, error.message);
                        }
                    }
                }
            }
        }
        out
    }

    /// The input the package reads.
    pub fn reader(&self) -> &R {
        self.zip.reader()
    }

    fn relative<'n>(&self, name: &'n str) -> &'n str {
        name.strip_prefix(self.root.as_str()).unwrap_or(name)
    }
}

impl<R: RangeReader> IndexSource for KeynotePackage<R> {
    fn entry_sizes(&self) -> Vec<(&str, u64)> {
        self.entries()
            .map(|(path, entry)| (path, entry.size))
            .collect()
    }

    fn read_many(&self, paths: &[&str]) -> ReadMany<'_> {
        KeynotePackage::read_many(self, paths)
    }
}
