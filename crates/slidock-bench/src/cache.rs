//! The cache's layout and its write discipline: every key is content-derived, and every write goes to a temporary
//! sibling and is renamed into place, so a killed run never leaves a half-written file behind a valid name.
//!
//! ```text
//! <cache>/hashes.json              file hashes by (path, size, mtime, inode)
//! <cache>/fonts/<sha>.<ext>        the pinned font set (mounted read-only into the renderer, `--fonts` of the compiler)
//! <cache>/font-archives/<sha(url)>.<ext>
//! <cache>/oracle/{src,raster,text} everything the renderer container may touch (mounted at /work)
//! <cache>/gif-frames/<sha>.png     first frames of animated GIFs
//! <cache>/runs/<runKey>/, runs/latest
//! ```

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use serde::de::DeserializeOwned;
use slidock_canon::Layout;

use crate::error::{BenchError, Result};

#[derive(Clone, Debug)]
pub struct CacheLayout {
    pub root: PathBuf,
    pub hashes: PathBuf,
    pub fonts: PathBuf,
    pub font_archives: PathBuf,
    /// Mounted at /work in the renderer.
    pub oracle: PathBuf,
    pub gif_frames: PathBuf,
    pub runs: PathBuf,
}

impl CacheLayout {
    pub fn new(root: &Path) -> CacheLayout {
        CacheLayout {
            root: root.to_path_buf(),
            hashes: root.join("hashes.json"),
            fonts: root.join("fonts"),
            font_archives: root.join("font-archives"),
            oracle: root.join("oracle"),
            gif_frames: root.join("gif-frames"),
            runs: root.join("runs"),
        }
    }
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A temporary sibling of `path`, unique within and across processes.
pub fn temp_sibling(path: &Path) -> PathBuf {
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!("{name}.{}.{n}.tmp", std::process::id()))
}

/// Writes through a temporary sibling and a rename.
pub fn write_atomic(path: &Path, data: impl AsRef<[u8]>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(BenchError::io(parent))?;
    }
    let temp = temp_sibling(path);
    std::fs::write(&temp, data).map_err(BenchError::io(&temp))?;
    std::fs::rename(&temp, path).map_err(|source| {
        let _ = std::fs::remove_file(&temp);
        BenchError::Io {
            path: path.to_path_buf(),
            source,
        }
    })
}

/// Writes a value as canonical JSON (two-space indent, trailing newline), atomically.
pub fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    write_atomic(
        path,
        slidock_canon::to_canonical_json(value, Layout::Indented)?,
    )
}

/// A JSON file's value, or `None` when the file is missing.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|error| BenchError::failed(format!("{}: {error}", path.display()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(BenchError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Removes a file or folder; a missing one is fine.
pub fn remove(path: &Path) -> Result<()> {
    let result = match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => Err(error),
    };
    result.map_err(BenchError::io(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_atomically_and_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b/value.json");
        write_json(&path, &serde_json::json!({"b": 1, "a": [1.5, 2]})).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\n  \"a\": [\n    1.5,\n    2\n  ],\n  \"b\": 1\n}\n"
        );
        let back: Option<serde_json::Value> = read_json(&path).unwrap();
        assert_eq!(back.unwrap()["b"], 1);
        // No temporary file is left beside it.
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().collect();
        assert_eq!(names.len(), 1);
        assert!(
            read_json::<serde_json::Value>(&dir.path().join("missing"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn temp_siblings_are_unique() {
        let path = Path::new("/x/file.json");
        assert_ne!(temp_sibling(path), temp_sibling(path));
        assert_eq!(temp_sibling(path).parent(), path.parent());
    }
}
