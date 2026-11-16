//! `slidock compile <input> -o deck.zip` (or `-o <folder>`): the product output, the bundled deck, written as a zip
//! or as the folder a zip unpacks to (`index.html`, `media/`, `LICENSES.txt`).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Component, Path, PathBuf};

use slidock::{BundleError, BundleSink};

/// The entries of a deck folder the writer owns; they are replaced and the rest kept.
const OWNED: [&str; 3] = ["index.html", "media", "LICENSES.txt"];

/// How the deck is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// One zip file.
    Zip(PathBuf),
    /// The files of the deck in a folder.
    Folder(PathBuf),
}

impl Target {
    /// A path ending in `.zip` (any case) is a zip; any other path a folder.
    pub fn of(path: &Path) -> Target {
        let zip = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"));
        if zip {
            Target::Zip(path.to_path_buf())
        } else {
            Target::Folder(path.to_path_buf())
        }
    }
}

fn sink_error(path: &Path) -> impl Fn(std::io::Error) -> BundleError + '_ {
    move |error| BundleError::Sink(format!("{}: {error}", path.display()))
}

/// The zip, written to `<path>.part` and renamed over `path` once whole, so a failed bundle leaves no zip.
pub struct ZipFile {
    path: PathBuf,
    part: PathBuf,
    out: Option<BufWriter<File>>,
}

impl ZipFile {
    pub fn create(path: &Path) -> Result<ZipFile, String> {
        let mut part = path.as_os_str().to_owned();
        part.push(".part");
        let part = PathBuf::from(part);
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        let file = File::create(&part).map_err(|error| format!("{}: {error}", part.display()))?;
        Ok(ZipFile {
            path: path.to_path_buf(),
            part,
            out: Some(BufWriter::new(file)),
        })
    }

    /// Moves the whole zip into place.
    pub fn commit(mut self) -> Result<(), String> {
        let at = |path: &Path, error: std::io::Error| format!("{}: {error}", path.display());
        if let Some(mut out) = self.out.take() {
            out.flush().map_err(|error| at(&self.part, error))?;
        }
        std::fs::rename(&self.part, &self.path).map_err(|error| {
            let _ = std::fs::remove_file(&self.part);
            at(&self.path, error)
        })
    }
}

impl Drop for ZipFile {
    /// A zip never committed is removed.
    fn drop(&mut self) {
        if self.out.take().is_some() {
            let _ = std::fs::remove_file(&self.part);
        }
    }
}

impl BundleSink for ZipFile {
    fn write(&mut self, _path: &str, bytes: &[u8]) -> Result<(), BundleError> {
        let out = self
            .out
            .as_mut()
            .ok_or_else(|| BundleError::Sink("the zip is closed".to_owned()))?;
        out.write_all(bytes).map_err(sink_error(&self.part))
    }
}

/// The staging folder inside the deck folder, moved into place on commit.
const STAGING: &str = ".slidock.part";

/// The deck's files in a folder: written into a staging folder inside it as their bytes arrive and moved into place
/// once whole, so a failed bundle leaves the folder as it was.
pub struct Folder {
    root: PathBuf,
    staging: PathBuf,
    /// The folders `create` made, the root first, which a failed bundle removes again while they are empty.
    made: Vec<PathBuf>,
    current: Option<(String, BufWriter<File>)>,
    pending: bool,
}

fn at(path: &Path) -> impl Fn(std::io::Error) -> String + '_ {
    move |error| format!("{}: {error}", path.display())
}

/// Removes the file, folder or link at `path`, if any.
fn remove(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

impl Folder {
    /// Opens `root` for a deck; nothing in it changes until [`Folder::commit`].
    pub fn create(root: &Path) -> Result<Folder, String> {
        let made = root
            .ancestors()
            .filter(|folder| !folder.as_os_str().is_empty())
            .take_while(|folder| !folder.exists())
            .map(Path::to_path_buf)
            .collect();
        std::fs::create_dir_all(root).map_err(at(root))?;
        let staging = root.join(STAGING);
        remove(&staging).map_err(at(&staging))?;
        std::fs::create_dir(&staging).map_err(at(&staging))?;
        Ok(Folder {
            root: root.to_path_buf(),
            staging,
            made,
            current: None,
            pending: true,
        })
    }

    fn close(&mut self) -> Result<(), BundleError> {
        if let Some((path, mut out)) = self.current.take() {
            out.flush().map_err(sink_error(&self.staging.join(path)))?;
        }
        Ok(())
    }

    /// Replaces the deck entries of the folder with the ones written, keeping everything else.
    pub fn commit(mut self) -> Result<(), String> {
        self.close().map_err(|error| error.to_string())?;
        for entry in OWNED {
            let path = self.root.join(entry);
            remove(&path).map_err(at(&path))?;
            let staged = self.staging.join(entry);
            if staged.exists() {
                std::fs::rename(&staged, &path).map_err(at(&path))?;
            }
        }
        std::fs::remove_dir(&self.staging).map_err(at(&self.staging))?;
        self.pending = false;
        Ok(())
    }

    /// `path` under the staging folder: relative, with plain names only, under one of the deck entries.
    fn target(&self, path: &str) -> Result<PathBuf, BundleError> {
        let relative = Path::new(path);
        let plain = relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
        let owned = relative
            .components()
            .next()
            .is_some_and(|first| OWNED.iter().any(|entry| first.as_os_str() == *entry));
        if !plain || !owned {
            return Err(BundleError::Sink(format!(
                "{path} is not a path in the deck"
            )));
        }
        Ok(self.staging.join(relative))
    }
}

impl Drop for Folder {
    /// A folder never committed loses the staging folder and the empty folders `create` made.
    fn drop(&mut self) {
        if self.pending {
            self.current = None;
            let _ = std::fs::remove_dir_all(&self.staging);
            for folder in &self.made {
                let _ = std::fs::remove_dir(folder);
            }
        }
    }
}

impl BundleSink for Folder {
    fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), BundleError> {
        if self.current.as_ref().map(|(open, _)| open.as_str()) != Some(path) {
            self.close()?;
            let target = self.target(path)?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(sink_error(parent))?;
            }
            let file = File::create(&target).map_err(sink_error(&target))?;
            self.current = Some((path.to_owned(), BufWriter::new(file)));
        }
        match &mut self.current {
            Some((_, out)) => out
                .write_all(bytes)
                .map_err(sink_error(&self.staging.join(path))),
            None => Err(BundleError::Sink(format!("{path} is not open"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zip_extension_in_any_case_names_a_zip_and_anything_else_a_folder() {
        assert_eq!(
            Target::of(Path::new("out/deck.zip")),
            Target::Zip("out/deck.zip".into())
        );
        assert_eq!(
            Target::of(Path::new("deck.ZIP")),
            Target::Zip("deck.ZIP".into())
        );
        assert_eq!(
            Target::of(Path::new("deck/")),
            Target::Folder("deck/".into())
        );
        assert_eq!(
            Target::of(Path::new("deck.zip.d")),
            Target::Folder("deck.zip.d".into())
        );
    }

    #[test]
    fn writes_each_file_in_turn_and_refuses_paths_outside_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("deck");
        let mut folder = Folder::create(&root).unwrap();
        folder.write("index.html", b"<!doctype").unwrap();
        folder.write("index.html", b" html>").unwrap();
        folder.write("media/a.png", b"png").unwrap();
        for bad in ["", "../x", "/x", "media/../../x", "./x", "x", "notes/a.png"] {
            assert!(folder.write(bad, b"x").is_err(), "{bad}");
        }
        assert!(!root.join("index.html").exists());
        folder.commit().unwrap();
        assert_eq!(
            std::fs::read(root.join("index.html")).unwrap(),
            b"<!doctype html>"
        );
        assert_eq!(std::fs::read(root.join("media/a.png")).unwrap(), b"png");
        assert!(!dir.path().join("x").exists());
        assert!(!root.join(STAGING).exists());
    }

    #[test]
    fn replaces_only_the_deck_entries_of_an_existing_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("media")).unwrap();
        std::fs::write(dir.path().join("media/old.png"), "old").unwrap();
        std::fs::write(dir.path().join("index.html"), "old").unwrap();
        std::fs::write(dir.path().join("keep.txt"), "mine").unwrap();
        Folder::create(dir.path()).unwrap().commit().unwrap();
        assert!(!dir.path().join("media").exists());
        assert!(!dir.path().join("index.html").exists());
        assert!(dir.path().join("keep.txt").is_file());
    }

    #[test]
    fn a_folder_changes_only_once_committed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("media")).unwrap();
        std::fs::write(dir.path().join("media/old.png"), "old").unwrap();
        std::fs::write(dir.path().join("index.html"), "old").unwrap();
        let mut folder = Folder::create(dir.path()).unwrap();
        folder.write("index.html", b"new").unwrap();
        folder.write("media/new.png", b"new").unwrap();
        drop(folder);
        assert_eq!(
            std::fs::read(dir.path().join("index.html")).unwrap(),
            b"old"
        );
        assert_eq!(
            std::fs::read(dir.path().join("media/old.png")).unwrap(),
            b"old"
        );
        assert!(!dir.path().join("media/new.png").exists());
        assert!(!dir.path().join(STAGING).exists());

        // A folder the writer made is gone again; one left by an interrupted run is replaced.
        let made = dir.path().join("made");
        drop(Folder::create(&made.join("deck")).unwrap());
        assert!(!made.exists());
        std::fs::create_dir_all(dir.path().join(STAGING).join("media")).unwrap();
        std::fs::write(dir.path().join(STAGING).join("media/stale.png"), "stale").unwrap();
        let mut folder = Folder::create(dir.path()).unwrap();
        folder.write("index.html", b"new").unwrap();
        folder.commit().unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("index.html")).unwrap(),
            b"new"
        );
        assert!(!dir.path().join("media").exists());
        assert!(!dir.path().join(STAGING).exists());
    }

    #[test]
    fn a_zip_appears_only_once_committed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out/deck.zip");
        let mut zip = ZipFile::create(&path).unwrap();
        zip.write("index.html", b"PK").unwrap();
        assert!(!path.exists());
        zip.commit().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"PK");
        assert!(!dir.path().join("out/deck.zip.part").exists());

        let failed = dir.path().join("failed.zip");
        let mut zip = ZipFile::create(&failed).unwrap();
        zip.write("index.html", b"PK").unwrap();
        drop(zip);
        assert!(!failed.exists());
        assert!(!dir.path().join("failed.zip.part").exists());
    }
}
