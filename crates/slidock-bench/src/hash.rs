//! SHA-256 of files and text. Corpus files are large (a 1 GB Keynote deck), so file hashes stream, and a file with the
//! same path, size, modification time and inode reuses its remembered hash while the cache is on.

use std::collections::BTreeMap;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use sha2::{Digest, Sha256};
use slidock_canon::Layout;

use crate::cache::{read_json, write_json};
use crate::error::{BenchError, Result};

pub fn sha256(data: impl AsRef<[u8]>) -> String {
    hex::encode(Sha256::digest(data.as_ref()))
}

/// The sha256 of a value's compact canonical JSON (`sha256Json`).
pub fn sha256_json<T: Serialize + ?Sized>(value: &T) -> Result<String> {
    Ok(sha256(slidock_canon::to_canonical_json(
        value,
        Layout::Compact,
    )?))
}

/// Streams a file through SHA-256.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path).map_err(BenchError::io(path))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 8 << 20];
    loop {
        let n = file.read(&mut buffer).map_err(BenchError::io(path))?;
        if n == 0 {
            break;
        }
        hasher.update(buffer.get(..n).unwrap_or_default());
    }
    Ok(hex::encode(hasher.finalize()))
}

/// File hashes remembered by (path, size, mtime, inode) in one JSON file.
pub struct FileHashes {
    file: PathBuf,
    use_cache: bool,
    entries: Mutex<Option<BTreeMap<String, String>>>,
}

impl FileHashes {
    pub fn new(file: &Path, use_cache: bool) -> FileHashes {
        FileHashes {
            file: file.to_path_buf(),
            use_cache,
            entries: Mutex::new(None),
        }
    }

    fn key(path: &Path) -> Result<String> {
        let meta = std::fs::metadata(path).map_err(BenchError::io(path))?;
        let mtime_ms = meta.mtime() as f64 * 1000.0 + meta.mtime_nsec() as f64 / 1e6;
        Ok(format!(
            "{}\0{}\0{}\0{}",
            path.display(),
            meta.len(),
            slidock_canon::to_js_string(mtime_ms),
            meta.ino()
        ))
    }

    /// The sha256 of the file at `path` (following symbolic links).
    pub fn of(&self, path: &Path) -> Result<String> {
        let key = FileHashes::key(path)?;
        if self.use_cache {
            let mut entries = self.lock();
            let entries = self.loaded(&mut entries)?;
            if let Some(known) = entries.get(&key) {
                return Ok(known.clone());
            }
        }
        let digest = sha256_file(path)?;
        if self.use_cache {
            let mut entries = self.lock();
            let entries = self.loaded(&mut entries)?;
            entries.insert(key, digest.clone());
            write_json(&self.file, entries)?;
        }
        Ok(digest)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<BTreeMap<String, String>>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn loaded<'a>(
        &self,
        entries: &'a mut Option<BTreeMap<String, String>>,
    ) -> Result<&'a mut BTreeMap<String, String>> {
        if entries.is_none() {
            *entries = Some(read_json(&self.file)?.unwrap_or_default());
        }
        Ok(entries.get_or_insert_with(BTreeMap::new))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_text_json_and_files() {
        assert_eq!(
            sha256("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_json(&serde_json::json!({"b": 1, "a": 2})).unwrap(),
            sha256("{\"a\":2,\"b\":1}")
        );
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, "abc").unwrap();
        assert_eq!(sha256_file(&file).unwrap(), sha256("abc"));
    }

    #[test]
    fn remembers_hashes_until_the_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, "abc").unwrap();
        let store = dir.path().join("hashes.json");
        let hashes = FileHashes::new(&store, true);
        assert_eq!(hashes.of(&file).unwrap(), sha256("abc"));
        assert!(store.exists());
        // A fresh reader of the store finds the remembered hash; a changed file is hashed again.
        let again = FileHashes::new(&store, true);
        assert_eq!(again.of(&file).unwrap(), sha256("abc"));
        std::fs::write(&file, "abcd").unwrap();
        assert_eq!(again.of(&file).unwrap(), sha256("abcd"));
        let uncached = FileHashes::new(&dir.path().join("none.json"), false);
        uncached.of(&file).unwrap();
        assert!(!dir.path().join("none.json").exists());
    }
}
