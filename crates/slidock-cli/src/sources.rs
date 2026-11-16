//! The deck's font sources, resolved per face in the default order: fonts embedded in the input
//! (on unless `--no-embedded`), the computer's installed fonts (`--system-fonts`), Google Fonts (on unless
//! `--no-google` or `--offline`: static TrueType files fetched with a non-browser `User-Agent` and cached), then the
//! files of `--fonts` folders. The bundled substitutes are the floor: the faces the finished deck draws from them are
//! added from the files built in ([`crate::bundled`]) and the deck finishes again, until it draws no substitute face
//! it lacks, as the web app does. The deck ships every face it draws from them (subset, WOFF2). `--emit`, and a
//! deck with `--fonts-off`, take none of them and ship nothing. What each source added prints at `-v`.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use slidock::{
    BundleOptions, BundleSink, Bundled, CompileError, CompiledDeck, FinishedDeck, FontSource,
    Frontends, Input, Options, Session, SessionMedia, SessionOptions, SetupError, Sha256,
    StoreMedia, SubstituteFace, finish_deck, pipeline,
};
use slidock_canon::utf16;
use slidock_fonts::google::{MAX_CSS2_BYTES, parse_css2};
use slidock_fonts::resolve::{LocalBudget, LocalFontMeta, local_meta_of_name_table};

use crate::bundled;
use crate::fonts::font_files;
use crate::say;

/// Sent to Google Fonts: a non-browser agent gets one static TrueType file per face.
const USER_AGENT: &str = concat!("slidock/", env!("CARGO_PKG_VERSION"), " (font fetch)");
/// The most bytes a font file may have.
const MAX_FONT_BYTES: u64 = 48 * 1024 * 1024;
/// The most faces a system scan reads metadata for.
const MAX_SCANNED_FACES: usize = 20_000;
const HTTP_TIMEOUT: Duration = Duration::from_secs(60);

/// Which sources a deck compile takes fonts from, and whether it ships the faces it draws.
#[derive(Clone, Debug, Default)]
pub struct FontSources {
    pub embedded: bool,
    pub system: bool,
    pub google: bool,
    /// Ship the faces the deck draws from the fonts added.
    pub ship: bool,
    /// Add the bundled substitute faces the deck draws.
    pub bundled: bool,
    /// `--fonts` folders, resolved.
    pub dropped: Vec<PathBuf>,
    /// The cache for Google Fonts downloads; nothing is cached without one.
    pub cache: Option<PathBuf>,
}

/// The user's cache folder for slidock: `$SLIDOCK_CACHE_DIR`, else the platform's cache folder.
pub fn cache_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("SLIDOCK_CACHE_DIR") {
        return Some(PathBuf::from(dir));
    }
    if let Some(dir) = std::env::var_os("XDG_CACHE_HOME") {
        return Some(PathBuf::from(dir).join("slidock"));
    }
    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA")
            .map(|dir| PathBuf::from(dir).join("slidock").join("cache"));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    Some(if cfg!(target_os = "macos") {
        home.join("Library/Caches/slidock")
    } else {
        home.join(".cache/slidock")
    })
}

/// The folders the operating system installs fonts in.
pub fn system_font_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut dirs = Vec::new();
    if cfg!(target_os = "macos") {
        dirs.extend(["/System/Library/Fonts", "/Library/Fonts"].map(PathBuf::from));
        dirs.extend(home.map(|home| home.join("Library/Fonts")));
    } else if cfg!(windows) {
        dirs.extend(std::env::var_os("WINDIR").map(|dir| PathBuf::from(dir).join("Fonts")));
        dirs.extend(std::env::var_os("LOCALAPPDATA").map(|dir| {
            PathBuf::from(dir)
                .join("Microsoft")
                .join("Windows")
                .join("Fonts")
        }));
    } else {
        dirs.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(PathBuf::from));
        if let Some(home) = home {
            dirs.push(home.join(".local/share/fonts"));
            dirs.push(home.join(".fonts"));
        }
    }
    dirs.retain(|dir| dir.is_dir());
    dirs
}

/// `open` → the sources → `finish` → [`finish_deck`] → bundle, as [`slidock::compile`] does with the sources added.
/// A source that fails is a warning; the compile goes on with what it has.
pub fn compile_deck(
    input: &Input<'_>,
    frontends: Frontends,
    options: &Options<'_>,
    sources: &FontSources,
    bundle_options: &BundleOptions,
    sink: &mut dyn BundleSink,
) -> Result<Bundled, CompileError> {
    let registry = frontends.registry().map_err(SetupError::from)?;
    let pipeline = pipeline().map_err(SetupError::from)?;
    let fonts_on = slidock::fonts::with_fonts(&registry, &pipeline);
    let session_options = SessionOptions {
        frontend: options.frontend,
        only: options.only,
        verify: options.verify,
        limits: options.limits,
        fonts: options.fonts,
        policy: options.policy,
        clock: options.clock,
        trace: options.trace,
        embedded_fonts: sources.embedded,
        glyphs: fonts_on.glyphs.filter(|_| sources.ship),
        native_motion: options.native_motion,
        ..fonts_on
    };
    let mut session = Session::open(input, session_options).map_err(CompileError::Open)?;
    if sources.system {
        add_system_fonts(&mut session, &system_font_dirs());
    }
    if sources.google {
        add_google_fonts(&mut session, sources.cache.as_deref());
    }
    add_dropped_fonts(&mut session, &sources.dropped);
    let mut finished = session.finish()?;
    if sources.bundled {
        let mut tried = Vec::new();
        while add_bundled_fonts(&mut session, &finished.substitutes, &mut tried) {
            finished = session.finish()?;
        }
    }
    for issue in &finished.font_issues {
        warn(&format!("{} {issue}", issue.code()));
    }
    for face in &finished.fonts {
        let entry = &face.licence;
        say::verbose(format!(
            "font ship: {} {} {} ({}, {} bytes{}{})",
            entry.family,
            entry.weight,
            entry.style,
            face.source.as_str(),
            face.bytes,
            if face.subset { ", subset" } else { ", whole" },
            if face.demoted { ", demoted" } else { "" },
        ));
    }
    let FinishedDeck {
        deck,
        documents,
        diagnostics,
    } = finish_deck(&session, &finished, options.generator.clone())?;
    let store = SessionMedia(&session);
    let media = StoreMedia::new(
        &store,
        input.reader,
        &documents,
        options.policy.transcode_pixels,
    );
    let compiled = CompiledDeck {
        deck,
        documents,
        media: &media,
        fonts: slidock::fonts::bundled_faces(&finished.fonts),
    };
    let report = slidock::bundle_within(&compiled, bundle_options, sink, session.deadline())?;
    Ok(Bundled {
        deck: slidock::bundled_deck(&compiled, bundle_options)?,
        report,
        diagnostics,
    })
}

fn warn(message: &str) {
    say::normal(format!("warning font {message}"));
}

/// Adds a font file from a source, reporting what it holds (or why it is refused).
fn add(
    session: &mut Session<'_>,
    bytes: Result<Vec<u8>, String>,
    source: FontSource,
    origin: &str,
) {
    let added = bytes.and_then(|bytes| session.add_font(bytes, source).map_err(|e| e.to_string()));
    match added {
        Ok(_) => {
            let faces: Vec<String> = session
                .added_fonts()
                .last()
                .map(|font| {
                    font.faces
                        .iter()
                        .map(|face| face.postscript_name.clone())
                        .collect()
                })
                .unwrap_or_default();
            say::verbose(format!(
                "font {}: {} ({origin})",
                source.as_str(),
                faces.join(", ")
            ));
        }
        Err(message) => warn(&format!("{origin}: {message}")),
    }
}

/// The bundled substitutes' files of the faces the deck draws with no file, in the order drawn, each file at most once
/// (`tried` holds the files offered before); whether any was added.
fn add_bundled_fonts(
    session: &mut Session<'_>,
    faces: &[SubstituteFace],
    tried: &mut Vec<&'static str>,
) -> bool {
    let mut added = false;
    for wanted in faces {
        let Some(face) = bundled::face_for(wanted) else {
            warn(&format!(
                "{}: not among the bundled files",
                wanted.postscript_name
            ));
            continue;
        };
        if tried.contains(&face.file) {
            continue;
        }
        tried.push(face.file);
        let count = session.added_fonts().len();
        add(
            session,
            bundled::load(face),
            FontSource::Bundled,
            &format!("{} substitute", face.family),
        );
        added |= session.added_fonts().len() > count;
    }
    added
}

/// Reads at most `MAX_FONT_BYTES` of a file.
fn read_font(path: &Path) -> Result<Vec<u8>, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_FONT_BYTES {
        return Err(format!("{size} bytes, over the {MAX_FONT_BYTES} limit"));
    }
    std::fs::read(path).map_err(|e| e.to_string())
}

/// The files of the `--fonts` folders, each folder's in UTF-16 path order, as dropped files.
fn add_dropped_fonts(session: &mut Session<'_>, dirs: &[PathBuf]) {
    for dir in dirs {
        let Ok(mut files) = font_files(dir) else {
            continue;
        };
        files.sort_by(|a, b| utf16::cmp(&a.to_string_lossy(), &b.to_string_lossy()));
        for file in files {
            add(
                session,
                read_font(&file),
                FontSource::User,
                &file.display().to_string(),
            );
        }
    }
}

/// The installed faces worth reading for what is still missing, chosen from a scan of each file's `name` table.
fn add_system_fonts(session: &mut Session<'_>, dirs: &[PathBuf]) {
    if session.font_needs().is_empty() {
        return;
    }
    let mut faces: Vec<(PathBuf, LocalFontMeta)> = Vec::new();
    for dir in dirs {
        let Ok(mut files) = font_files(dir) else {
            continue;
        };
        files.sort();
        for file in files {
            if faces.len() >= MAX_SCANNED_FACES {
                break;
            }
            faces.extend(
                scan_names(&file)
                    .into_iter()
                    .map(|meta| (file.clone(), meta)),
            );
        }
    }
    let metas: Vec<LocalFontMeta> = faces.iter().map(|(_, meta)| meta.clone()).collect();
    let mut read: Vec<&Path> = Vec::new();
    let mut budget = LocalBudget::default();
    for pick in session.select_local(&metas) {
        let Some((path, _)) = faces.get(pick.index) else {
            continue;
        };
        if read.contains(&path.as_path()) {
            continue;
        }
        read.push(path);
        let bytes = std::fs::metadata(path)
            .map_err(|e| e.to_string())
            .and_then(|meta| {
                budget
                    .spend(meta.len(), pick.exact_postscript)
                    .map_err(|e| e.to_string())
            })
            .and_then(|()| std::fs::read(path).map_err(|e| e.to_string()));
        add(
            session,
            bytes,
            FontSource::Local,
            &path.display().to_string(),
        );
    }
}

/// Each face's metadata from a font file's `name` tables, reading only the directory and those tables.
pub fn scan_names(path: &Path) -> Vec<LocalFontMeta> {
    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let mut read = |offset: u64, len: usize| -> Option<Vec<u8>> {
        if len > 1 << 20 {
            return None;
        }
        file.seek(SeekFrom::Start(offset)).ok()?;
        let mut buffer = vec![0; len];
        file.read_exact(&mut buffer).ok()?;
        Some(buffer)
    };
    let be32 = |bytes: &[u8], at: usize| -> Option<u32> {
        Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    };
    let Some(head) = read(0, 12) else {
        return Vec::new();
    };
    let offsets: Vec<u64> = if head.get(..4) == Some(b"ttcf") {
        let count = be32(&head, 8).unwrap_or(0).min(256) as usize;
        read(12, count * 4)
            .map(|table| {
                (0..count)
                    .filter_map(|i| be32(&table, i * 4))
                    .map(u64::from)
                    .collect()
            })
            .unwrap_or_default()
    } else {
        vec![0]
    };
    let mut metas = Vec::new();
    for offset in offsets {
        let Some(header) = read(offset, 12) else {
            continue;
        };
        let tables = header
            .get(4..6)
            .and_then(|n| n.try_into().ok())
            .map_or(0, |n: [u8; 2]| usize::from(u16::from_be_bytes(n)));
        let Some(directory) = read(offset + 12, tables.min(512) * 16) else {
            continue;
        };
        let (records, _) = directory.as_chunks::<16>();
        let name = records
            .iter()
            .find(|record| record.starts_with(b"name"))
            .and_then(|record| Some((be32(record, 8)?, be32(record, 12)?)));
        let Some((at, len)) = name else { continue };
        if let Some(meta) = read(u64::from(at), len as usize)
            .as_deref()
            .and_then(local_meta_of_name_table)
        {
            metas.push(meta);
        }
    }
    metas
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(HTTP_TIMEOUT))
        .user_agent(USER_AGENT)
        .build()
        .into()
}

/// A URL's body, from the cache when it holds it, else fetched (at most `limit` bytes) and cached.
fn fetch(
    agent: &ureq::Agent,
    url: &str,
    limit: u64,
    cache: Option<&Path>,
) -> Result<Vec<u8>, String> {
    let cached = cache.map(|dir| dir.join("google").join(Sha256::of(url.as_bytes()).to_hex()));
    if let Some(bytes) = cached.as_deref().and_then(|path| std::fs::read(path).ok()) {
        return Ok(bytes);
    }
    let mut response = agent
        .get(url)
        .call()
        .map_err(|error| format!("GET {url}: {error}"))?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|error| format!("GET {url}: {error}"))?;
    if let Some(path) = cached
        && let Some(parent) = path.parent()
        && std::fs::create_dir_all(parent).is_ok()
    {
        let partial = path.with_extension("partial");
        if std::fs::write(&partial, &bytes).is_ok() {
            let _ = std::fs::rename(&partial, &path);
        }
    }
    Ok(bytes)
}

/// The Google Fonts faces of the catalog families still missing a face: each family's `css2` request, then its files.
fn add_google_fonts(session: &mut Session<'_>, cache: Option<&Path>) {
    let mut urls: Vec<String> = Vec::new();
    for need in session.font_needs() {
        if let Some(plan) = need.google
            && !urls.contains(&plan.url)
        {
            urls.push(plan.url);
        }
    }
    if urls.is_empty() {
        return;
    }
    let agent = agent();
    let mut fetched: Vec<String> = Vec::new();
    for url in urls {
        let css = fetch(&agent, &url, MAX_CSS2_BYTES as u64, cache)
            .and_then(|bytes| String::from_utf8(bytes).map_err(|e| e.to_string()))
            .and_then(|css| parse_css2(&css).map_err(|e| format!("{url}: {e}")));
        let faces = match css {
            Ok(faces) => faces,
            Err(message) => {
                warn(&message);
                continue;
            }
        };
        for face in faces {
            if fetched.contains(&face.url) {
                continue;
            }
            fetched.push(face.url.clone());
            let bytes = fetch(&agent, &face.url, MAX_FONT_BYTES, cache);
            add(session, bytes, FontSource::Google, &face.url);
        }
    }
}
