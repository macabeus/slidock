//! The file system side of the command: inputs and media read by range, and the document set written to a folder.

use std::borrow::Cow;
use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use slidock::{
    Documents, FileReader, MediaFiles, MediaStore, RangeError, RangeReader, check_range,
};

/// The input, read by positioned reads from one open file.
pub fn open_input(path: &Path) -> io::Result<FileReader> {
    if !std::fs::metadata(path)?.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a file"));
    }
    FileReader::open(path)
}

/// A file read by range, opened per read, so a folder of many media files holds descriptors only while reading.
#[derive(Clone, Debug)]
pub struct FileRange {
    path: PathBuf,
    len: u64,
}

impl FileRange {
    pub fn open(path: &Path) -> io::Result<FileRange> {
        let metadata = std::fs::metadata(path)?;
        if !metadata.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a file"));
        }
        Ok(FileRange {
            path: path.to_path_buf(),
            len: metadata.len(),
        })
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(buffer)
    }
}

impl RangeReader for FileRange {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<Cow<'_, [u8]>, RangeError> {
        check_range(self.len, offset, len as u64)?;
        let mut buffer = vec![0; len];
        self.read_exact_at(offset, &mut buffer)
            .map_err(|error| RangeError::Io {
                offset,
                len: len as u64,
                message: format!("{}: {error}", self.path.display()),
            })?;
        Ok(Cow::Owned(buffer))
    }
}

/// The `<sha256>.<ext>` files of a folder as a media set; other files are left out.
pub fn media_dir(dir: &Path) -> io::Result<MediaFiles> {
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .collect();
    names.sort();
    let mut media = MediaFiles::new();
    for name in names {
        if !slidock::is_media_file_name(&name) {
            continue;
        }
        let Ok(file) = FileRange::open(&dir.join(&name)) else {
            continue;
        };
        media.insert(&name, Arc::new(file));
    }
    Ok(media)
}

/// The entries of an output folder the writer owns; they are replaced and the rest kept.
const OWNED: [&str; 4] = ["presentation.json", "slides", "layouts", "media"];

/// What writing the media of a document set reads.
pub struct MediaSources<'a> {
    pub store: &'a dyn MediaStore,
    /// The input, which a store's input ranges point into.
    pub input: &'a dyn RangeReader,
    /// Transcoded rasters over this many pixels are refused.
    pub max_pixels: u64,
}

/// Replaces the document set in `dir` with `documents`, writing its media from `media` unless it is `None`.
pub fn write_documents(
    dir: &Path,
    documents: &Documents,
    media: Option<&MediaSources<'_>>,
) -> Result<(), String> {
    let at = |path: &Path| {
        let path = path.display().to_string();
        move |error: io::Error| format!("{path}: {error}")
    };
    std::fs::create_dir_all(dir).map_err(at(dir))?;
    for entry in OWNED {
        let path = dir.join(entry);
        let removed = match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(&path),
            Ok(_) => std::fs::remove_file(&path),
            Err(_) => Ok(()),
        };
        removed.map_err(at(&path))?;
    }
    for (path, text) in &documents.text {
        let target = dir.join(path);
        create_parent(&target)?;
        std::fs::write(&target, text).map_err(at(&target))?;
    }
    let Some(media) = media else { return Ok(()) };
    for (path, plan) in &documents.media {
        let target = dir.join(path);
        create_parent(&target)?;
        let file = File::create(&target).map_err(at(&target))?;
        let mut out = BufWriter::new(file);
        slidock::write_media_file(plan, media.store, media.input, media.max_pixels, &mut out)
            .map_err(|error| error.to_string())?;
        out.flush().map_err(at(&target))?;
    }
    Ok(())
}

fn create_parent(path: &Path) -> Result<(), String> {
    match path.parent() {
        Some(parent) => std::fs::create_dir_all(parent)
            .map_err(|error| format!("{}: {error}", parent.display())),
        None => Ok(()),
    }
}

/// Writes `text` to `path`, creating its folder.
pub fn write_text(path: &Path, text: &str) -> Result<(), String> {
    create_parent(path)?;
    std::fs::write(path, text).map_err(|error| format!("{}: {error}", path.display()))
}
