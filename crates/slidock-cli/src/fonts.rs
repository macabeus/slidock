//! `--fonts <dir>`: the font files of folders added to the built-in catalog bind-fonts binds against.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use slidock::{CatalogFace, FontCatalog, Sha256, builtin_catalog, read_sfnt};
use slidock_canon::utf16;
use slidock_fonts::FaceFile;

/// A font file that could not be read, reported and skipped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub file: PathBuf,
    pub message: String,
}

/// TrueType, OpenType and collection files, by extension in any case.
fn is_font_file(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [".ttf", ".otf", ".ttc"]
        .iter()
        .any(|ext| name.ends_with(ext))
}

/// `path` against `cwd` as Node's `path.resolve` takes it: absolute, with `.` and `..` resolved by name.
fn resolve(cwd: &Path, path: &Path) -> PathBuf {
    let mut resolved = PathBuf::new();
    for component in cwd.join(path).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            other => resolved.push(other),
        }
    }
    resolved
}

/// The font files under `dir`, recursively.
pub(crate) fn font_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(&next)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let path = entry.path();
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() && entry.file_name().to_str().is_some_and(is_font_file) {
                files.push(path);
            }
        }
    }
    Ok(files)
}

/// The `--fonts` folders resolved against `cwd`; a folder that cannot be listed is an error.
pub fn font_dirs(dirs: &[PathBuf], cwd: &Path) -> Result<Vec<PathBuf>, String> {
    dirs.iter()
        .map(|dir| {
            let resolved = resolve(cwd, dir);
            std::fs::read_dir(&resolved)
                .map(|_| resolved)
                .map_err(|error| format!("--fonts {}: {error}", dir.display()))
        })
        .collect()
}

/// The built-in fonts plus every face in `dirs` (resolved against `cwd`), each recorded with its file's SHA-256 and
/// kept with its file, whose glyph advances fit text that shrinks.
/// Each folder's files are read in the order JavaScript sorts their paths (by UTF-16 code unit), so a later face (and
/// a later folder's) replaces an earlier one with the same family, weight and style. A folder that cannot be listed
/// is an error; a file that fails to read or parse is a problem, skipped, with its absolute path.
pub fn font_catalog(dirs: &[PathBuf], cwd: &Path) -> Result<(FontCatalog, Vec<Problem>), String> {
    if dirs.is_empty() {
        return Ok((builtin_catalog().clone(), Vec::new()));
    }
    let mut entries: Vec<(String, CatalogFace)> = Vec::new();
    let mut problems = Vec::new();
    for dir in dirs {
        let mut files = font_files(&resolve(cwd, dir))
            .map_err(|error| format!("--fonts {}: {error}", dir.display()))?;
        files.sort_by(|a, b| utf16::cmp(&a.to_string_lossy(), &b.to_string_lossy()));
        for file in files {
            let read = std::fs::read(&file)
                .map_err(|error| error.to_string())
                .and_then(|bytes| {
                    let faces = read_sfnt(&bytes).map_err(|error| error.to_string())?;
                    Ok((Sha256::of(&bytes), faces, Arc::<[u8]>::from(bytes)))
                });
            match read {
                Ok((sha256, faces, bytes)) => {
                    entries.extend(faces.iter().zip(0u32..).map(|(face, index)| {
                        (
                            face.family.clone(),
                            CatalogFace::from_sfnt(face, Some(sha256))
                                .with_file(FaceFile::new(bytes.clone(), index)),
                        )
                    }));
                }
                Err(message) => problems.push(Problem { file, message }),
            }
        }
    }
    let scanned = FontCatalog::build(entries);
    Ok((FontCatalog::merge([builtin_catalog(), &scanned]), problems))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A TrueType file of one face named `family`: the head, hhea and name tables the reader needs, then `padding`
    /// zero bytes, so two files of the same face hash apart.
    fn font(family: &str, padding: usize) -> Vec<u8> {
        let name: Vec<u8> = family.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let mut name_table = Vec::new();
        for field in [0u16, 1, 18, 3, 1, 0x409, 1, name.len() as u16, 0] {
            name_table.extend(field.to_be_bytes());
        }
        name_table.extend(name);
        let mut head = vec![0; 54];
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        let tables = [
            (*b"head", head),
            (*b"hhea", vec![0; 36]),
            (*b"name", name_table),
        ];
        let mut file = Vec::new();
        file.extend(0x0001_0000u32.to_be_bytes());
        file.extend((tables.len() as u16).to_be_bytes());
        file.extend([0; 6]);
        let mut offset = 12 + tables.len() * 16;
        for (tag, table) in &tables {
            file.extend(tag);
            file.extend(0u32.to_be_bytes());
            file.extend((offset as u32).to_be_bytes());
            file.extend((table.len() as u32).to_be_bytes());
            offset += table.len().next_multiple_of(4);
        }
        for (_, table) in &tables {
            file.extend(table);
            file.resize(file.len().next_multiple_of(4), 0);
        }
        file.resize(file.len() + padding, 0);
        file
    }

    #[test]
    fn reads_font_files_by_extension_in_any_case() {
        for name in ["a.ttf", "b.OTF", "c.TtC"] {
            assert!(is_font_file(name), "{name}");
        }
        for name in ["a.woff2", "ttf", "a.ttf.bak"] {
            assert!(!is_font_file(name), "{name}");
        }
    }

    #[test]
    fn resolves_a_folder_against_the_working_folder_as_node_does() {
        let cases = [
            ("fonts", "/work/fonts"),
            ("./fonts/", "/work/fonts"),
            ("../shared/./fonts", "/shared/fonts"),
            ("../../..", "/"),
            ("/abs/fonts/..", "/abs"),
        ];
        for (path, resolved) in cases {
            assert_eq!(
                resolve(Path::new("/work"), Path::new(path)),
                PathBuf::from(resolved),
                "{path}"
            );
        }
    }

    #[test]
    fn without_folders_the_catalog_is_the_built_in_one() {
        let (catalog, problems) = font_catalog(&[], Path::new("/")).unwrap();
        assert_eq!(&catalog, builtin_catalog());
        assert!(problems.is_empty());
    }

    #[test]
    fn a_later_file_in_utf16_order_replaces_the_same_face() {
        // UTF-16 puts the astral U+1F600 (D83D DE00) before U+FF5E; code point (and UTF-8) order puts it after.
        let dir = tempfile::tempdir().unwrap();
        let (astral, high) = (font("Sorted", 0), font("Sorted", 4));
        std::fs::write(dir.path().join("\u{1F600}.ttf"), &astral).unwrap();
        std::fs::write(dir.path().join("\u{FF5E}.ttf"), &high).unwrap();
        let (catalog, problems) =
            font_catalog(&[dir.path().to_path_buf()], Path::new("/")).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        let family = catalog.find_family("Sorted").unwrap().family;
        assert_eq!(family.faces.len(), 1);
        assert_eq!(family.faces[0].sha256, Some(Sha256::of(&high)));
    }

    #[test]
    fn skips_an_unreadable_font_as_a_problem_and_refuses_a_missing_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("nested")).unwrap();
        std::fs::write(dir.path().join("nested/broken.ttf"), b"not a font").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"ignored").unwrap();
        let (catalog, problems) = font_catalog(&[PathBuf::from(".")], dir.path()).unwrap();
        assert_eq!(&catalog, builtin_catalog());
        assert_eq!(problems.len(), 1);
        // As printed: `Path`'s equality skips `.` components.
        assert_eq!(
            problems[0].file.as_os_str(),
            dir.path().join("nested/broken.ttf").as_os_str()
        );
        assert!(font_catalog(&[PathBuf::from("missing")], dir.path()).is_err());
    }
}
