//! The bytes of an agent-written presentation's media: a set of `<sha256>.<ext>` files (the CLI reads the `media/`
//! folder beside the IR file). The names are claims; the frontend hashes every file it uses.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use indexmap::IndexMap;
use slidock_core_api::{MediaHandle, MediaSource, MediaStore};
use slidock_io::{BytesReader, RangeReader};
use slidock_ir::ids::Sha256;

/// Whether `name` is a media file name: 64 lowercase hex digits, a dot and an extension of 1 to 8 ASCII letters and
/// digits.
pub fn is_media_file_name(name: &str) -> bool {
    media_file_sha(name).is_some()
}

/// The hash a media file name claims.
fn media_file_sha(name: &str) -> Option<&str> {
    let (sha, ext) = name.split_once('.')?;
    let hex = sha.len() == 64
        && sha
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    let ext_ok = (1..=8).contains(&ext.len()) && ext.bytes().all(|b| b.is_ascii_alphanumeric());
    (hex && ext_ok).then_some(sha)
}

/// A presentation's media files by name, each read by range. A hash named by several files resolves to the first
/// name in byte order.
#[derive(Clone, Default)]
pub struct MediaFiles {
    files: BTreeMap<String, Arc<dyn RangeReader>>,
}

impl fmt::Debug for MediaFiles {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.files.keys()).finish()
    }
}

impl MediaFiles {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the file `name`; returns `false`, adding nothing, when the name is not a media file name.
    pub fn insert(&mut self, name: &str, reader: Arc<dyn RangeReader>) -> bool {
        if !is_media_file_name(name) {
            return false;
        }
        self.files.insert(name.to_owned(), reader);
        true
    }

    /// [`MediaFiles::insert`] for bytes in memory.
    pub fn insert_bytes(&mut self, name: &str, bytes: impl Into<Arc<[u8]>>) -> bool {
        let bytes: Arc<[u8]> = bytes.into();
        self.insert(name, Arc::new(BytesReader::new(bytes)))
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// The file whose name claims `sha256`, if any.
    pub fn open(&self, sha256: &str) -> Option<&Arc<dyn RangeReader>> {
        let prefix = format!("{sha256}.");
        self.files
            .range(prefix.clone()..)
            .next()
            .filter(|(name, _)| name.starts_with(&prefix))
            .map(|(_, reader)| reader)
    }
}

/// The media whose bytes hashed to their name.
#[derive(Default)]
pub(crate) struct VerifiedMedia(pub(crate) IndexMap<Sha256, Arc<dyn RangeReader>>);

impl MediaStore for VerifiedMedia {
    fn open(&self, sha256: &Sha256) -> Option<MediaHandle> {
        self.0.get(sha256).map(|reader| MediaHandle {
            bytes: reader.len(),
            source: MediaSource::Reader(reader.clone()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(c: char) -> String {
        c.to_string().repeat(64)
    }

    #[test]
    fn accepts_only_media_file_names() {
        assert!(is_media_file_name(&format!("{}.png", sha('a'))));
        assert!(is_media_file_name(&format!("{}.Mp4", sha('0'))));
        for name in [
            format!("{}.png", sha('A')),
            format!("{}.png", &sha('a')[1..]),
            format!("{}.", sha('a')),
            format!("{}.toolongext", sha('a')),
            format!("{}.p-g", sha('a')),
            format!("{}.tar.gz", sha('a')),
            "notes.txt".to_owned(),
        ] {
            assert!(!is_media_file_name(&name), "{name}");
        }
        let mut files = MediaFiles::new();
        assert!(!files.insert_bytes("notes.txt", b"x".to_vec()));
        assert!(files.is_empty());
    }

    #[test]
    fn resolves_a_hash_to_its_first_name_in_byte_order() {
        let mut files = MediaFiles::new();
        files.insert_bytes(&format!("{}.png", sha('a')), b"png".to_vec());
        files.insert_bytes(&format!("{}.gif", sha('a')), b"gif".to_vec());
        files.insert_bytes(&format!("{}.png", sha('b')), b"b".to_vec());
        let read = |sha: &str| {
            files.open(sha).map(|reader| {
                reader
                    .read_at(0, reader.len() as usize)
                    .unwrap()
                    .into_owned()
            })
        };
        assert_eq!(read(&sha('a')).as_deref(), Some(&b"gif"[..]));
        assert_eq!(read(&sha('b')).as_deref(), Some(&b"b"[..]));
        assert_eq!(read(&sha('c')), None);
        // A prefix of a hash is not that hash.
        assert_eq!(read(&sha('a')[..63]), None);
    }
}
