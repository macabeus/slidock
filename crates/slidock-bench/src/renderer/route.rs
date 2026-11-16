//! The renderer's network. `http://presentation.test/…` is served from a compiled presentation's folder: a slide URL
//! gets the slide composed with its layout (`slidock-format`'s composer), media is read from disk, anything else is a
//! 404, and every request to another origin is aborted. Two rewrites make a static render deterministic: an animated
//! GIF is served as its first frame, and each video's poster is shown as an image in the video's box, so waiting for
//! images covers posters too. A row's `html` sha256 is that of its composed document as served.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use html5ever::{LocalName, QualName, ns};
use slidock_format::dom::{Attr, Document, NodeId};
use slidock_format::manifest::{Manifest, read_manifest};
use slidock_format::paths::file_id;
use slidock_format::{MediaUrls, PresentationFiles, compose_slide};

use crate::cache::write_atomic;
use crate::error::{BenchError, Result};
use crate::renderer::session::{Response, Router};

pub const ORIGIN: &str = "http://presentation.test";

pub fn slide_url(file_id: &str) -> String {
    format!("{ORIGIN}/slides/{file_id}.html")
}

fn content_type(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "png" => "image/png",
        "jpg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "svg" => "image/svg+xml",
        _ => return None,
    })
}

/// A compiled presentation's file set as the composer reads it: documents as text, media by name only.
pub fn read_compiled(dir: &Path) -> Result<PresentationFiles> {
    let mut files = PresentationFiles::new();
    let manifest = dir.join("presentation.json");
    files.insert(
        "presentation.json".to_owned(),
        Some(std::fs::read_to_string(&manifest).map_err(BenchError::io(&manifest))?),
    );
    for folder in ["slides", "layouts", "media"] {
        let path = dir.join(folder);
        if !path.is_dir() {
            continue;
        }
        let mut names: Vec<String> = std::fs::read_dir(&path)
            .map_err(BenchError::io(&path))?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        for name in names {
            let key = format!("{folder}/{name}");
            if folder == "media" {
                files.insert(key, None);
            } else {
                let file = path.join(&name);
                files.insert(
                    key,
                    Some(std::fs::read_to_string(&file).map_err(BenchError::io(&file))?),
                );
            }
        }
    }
    Ok(files)
}

/// The manifest of a file set.
pub fn manifest_of(files: &PresentationFiles) -> Result<Manifest> {
    let text = files
        .get("presentation.json")
        .and_then(Option::as_deref)
        .ok_or_else(|| BenchError::failed("the file set has no presentation.json"))?;
    read_manifest(text).map_err(|error| BenchError::failed(format!("presentation.json: {error}")))
}

fn attr(name: &str, value: String) -> Attr {
    Attr {
        name: QualName::new(None, ns!(), LocalName::from(name)),
        value,
    }
}

/// Replaces every `<video poster>` with an `<img>` of the poster in the same box.
pub fn poster_images(html: &str) -> String {
    let mut document = Document::parse(html);
    rewrite_videos(&mut document, NodeId::DOCUMENT);
    document.serialize(NodeId::DOCUMENT)
}

/// Walks child nodes as parse5's tree does: a `<template>`'s content is not among its children.
fn rewrite_videos(document: &mut Document, node: NodeId) {
    let children: Vec<NodeId> = document.children(node).to_vec();
    for child in children {
        if document.is_element(child) && document.tag_name(child) == "video" {
            let Some(poster) = document.attribute(child, "poster").map(str::to_owned) else {
                continue;
            };
            let style = document.attribute(child, "style").unwrap_or("").to_owned();
            document.set_tag_name(child, "img");
            if let Some(attrs) = document.attrs_mut(child) {
                *attrs = vec![
                    attr("src", poster),
                    attr("alt", String::new()),
                    attr("style", format!("{style};display:block")),
                ];
            }
            document.set_children(child, Vec::new());
        } else {
            rewrite_videos(document, child);
        }
    }
}

/// Every slide's composed document, by file id: slide and layout composed, with the poster rewrite applied.
pub fn composed_documents(files: &PresentationFiles) -> Result<BTreeMap<String, String>> {
    let manifest = manifest_of(files)?;
    let mut documents = BTreeMap::new();
    for slide in &manifest.slides {
        let composed = compose_slide(files, &slide.id, &MediaUrls::Unchanged)
            .map_err(|error| BenchError::failed(format!("compose {}: {error}", slide.id)))?;
        documents.insert(file_id(&slide.id), poster_images(&composed));
    }
    Ok(documents)
}

/// The first frame of a GIF as PNG: the frame drawn on a transparent canvas of the GIF's size.
pub fn first_frame_png(gif_bytes: &[u8]) -> Result<Vec<u8>> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options
        .read_info(gif_bytes)
        .map_err(|error| BenchError::failed(format!("gif: {error}")))?;
    let (width, height) = (usize::from(decoder.width()), usize::from(decoder.height()));
    let mut canvas = vec![0u8; width * height * 4];
    if let Some(frame) = decoder
        .read_next_frame()
        .map_err(|error| BenchError::failed(format!("gif: {error}")))?
    {
        let (left, top) = (usize::from(frame.left), usize::from(frame.top));
        let frame_width = usize::from(frame.width);
        for (row, line) in frame.buffer.chunks(frame_width * 4).enumerate() {
            for (column, pixel) in line.chunks(4).enumerate() {
                let (x, y) = (left + column, top + row);
                if x >= width || y >= height || pixel.get(3) == Some(&0) {
                    continue;
                }
                let at = (y * width + x) * 4;
                if let Some(target) = canvas.get_mut(at..at + 4) {
                    target.copy_from_slice(pixel);
                }
            }
        }
    }
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|error| BenchError::failed(format!("png: {error}")))?;
        writer
            .write_image_data(&canvas)
            .map_err(|error| BenchError::failed(format!("png: {error}")))?;
    }
    Ok(png)
}

/// Serves one compiled presentation: `documents` maps slide file ids to their composed text.
pub struct PresentationRouter {
    pub dir: PathBuf,
    pub documents: BTreeMap<String, String>,
    pub gif_frames: PathBuf,
}

/// `decodeURIComponent` of a URL path; `None` when it is malformed.
fn decode_path(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            let hex = path.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

impl PresentationRouter {
    fn answer(&self, url: &str) -> Response {
        let not_found = Response::Fulfill {
            status: 404,
            content_type: None,
            body: Vec::new(),
        };
        let Some(rest) = url.strip_prefix(ORIGIN) else {
            return Response::Abort;
        };
        if !(rest.is_empty() || rest.starts_with('/')) {
            return Response::Abort;
        }
        let raw_path = rest.split(['?', '#']).next().unwrap_or("");
        let Some(path) = decode_path(raw_path) else {
            return not_found;
        };
        let path = path.trim_start_matches('/');
        if let Some(id) = path
            .strip_prefix("slides/")
            .and_then(|p| p.strip_suffix(".html"))
            && let Some(document) = self.documents.get(id)
        {
            return Response::Fulfill {
                status: 200,
                content_type: Some("text/html; charset=utf-8"),
                body: document.clone().into_bytes(),
            };
        }
        match path.strip_prefix("media/") {
            Some(name) => serve_media(&self.dir, &self.gif_frames, name),
            None => not_found,
        }
    }
}

/// A file of a compiled folder's `media/` (a sha256 name), an animated GIF as its first frame.
fn serve_media(dir: &Path, gif_frames: &Path, name: &str) -> Response {
    let not_found = Response::Fulfill {
        status: 404,
        content_type: None,
        body: Vec::new(),
    };
    let Some((sha, ext)) = name.split_once('.') else {
        return not_found;
    };
    let media_name = sha.len() == 64
        && sha
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && !ext.is_empty()
        && ext
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    let file = dir.join("media").join(name);
    if !media_name || !file.is_file() {
        return not_found;
    }
    if ext == "gif" {
        return match first_frame(gif_frames, &file) {
            Ok(body) => Response::Fulfill {
                status: 200,
                content_type: Some("image/png"),
                body,
            },
            Err(_) => not_found,
        };
    }
    match std::fs::read(&file) {
        Ok(body) => Response::Fulfill {
            status: 200,
            content_type: Some(content_type(ext).unwrap_or("application/octet-stream")),
            body,
        },
        Err(_) => not_found,
    }
}

/// A GIF's first frame as PNG, cached by the GIF's name.
fn first_frame(gif_frames: &Path, file: &Path) -> Result<Vec<u8>> {
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cached = gif_frames.join(format!("{stem}.png"));
    if let Ok(bytes) = std::fs::read(&cached) {
        return Ok(bytes);
    }
    let gif = std::fs::read(file).map_err(BenchError::io(file))?;
    let png = first_frame_png(&gif)?;
    write_atomic(&cached, &png)?;
    Ok(png)
}

/// The URL of a bundled deck's page.
pub fn deck_url() -> String {
    format!("{ORIGIN}/index.html")
}

/// Serves one bundled deck folder: its `index.html` and its `media/`, under the same render policy as
/// [`PresentationRouter`]; every other request is a 404, and every other origin is aborted.
pub struct DeckRouter {
    pub dir: PathBuf,
    pub gif_frames: PathBuf,
}

impl Router for DeckRouter {
    fn route(&self, url: &str) -> Response {
        let not_found = Response::Fulfill {
            status: 404,
            content_type: None,
            body: Vec::new(),
        };
        let Some(rest) = url.strip_prefix(ORIGIN) else {
            return Response::Abort;
        };
        if !(rest.is_empty() || rest.starts_with('/')) {
            return Response::Abort;
        }
        let raw_path = rest.split(['?', '#']).next().unwrap_or("");
        let Some(path) = decode_path(raw_path) else {
            return not_found;
        };
        let path = path.trim_start_matches('/');
        if path == "index.html" {
            let page = self.dir.join("index.html");
            return match std::fs::read(&page) {
                Ok(body) => Response::Fulfill {
                    status: 200,
                    content_type: Some("text/html; charset=utf-8"),
                    body,
                },
                Err(_) => not_found,
            };
        }
        match path.strip_prefix("media/") {
            Some(name) => serve_media(&self.dir, &self.gif_frames, name),
            None => not_found,
        }
    }
}

impl Router for PresentationRouter {
    fn route(&self, url: &str) -> Response {
        self.answer(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_posters_become_images() {
        let html = "<!doctype html><html><head></head><body><main><video poster=\"../media/a.png\" style=\"left:1pt\" src=\"x.mp4\"><source src=\"y\"></video><video src=\"z\"></video></main></body></html>";
        let out = poster_images(html);
        assert!(
            out.contains("<img src=\"../media/a.png\" alt=\"\" style=\"left:1pt;display:block\">"),
            "{out}"
        );
        assert!(!out.contains("<source"));
        assert!(out.contains("<video src=\"z\"></video>"));
    }

    fn router(dir: &Path) -> PresentationRouter {
        PresentationRouter {
            dir: dir.to_path_buf(),
            documents: BTreeMap::from([("s-1".to_owned(), "<p>hi</p>".to_owned())]),
            gif_frames: dir.join("frames"),
        }
    }

    fn status(response: &Response) -> Option<u16> {
        match response {
            Response::Fulfill { status, .. } => Some(*status),
            Response::Abort => None,
        }
    }

    #[test]
    fn routes_slides_media_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let sha = "a".repeat(64);
        std::fs::create_dir_all(dir.path().join("media")).unwrap();
        std::fs::write(dir.path().join(format!("media/{sha}.png")), "png").unwrap();
        let router = router(dir.path());
        let Response::Fulfill {
            status,
            content_type,
            body,
        } = router.route("http://presentation.test/slides/s-1.html")
        else {
            panic!("slide");
        };
        assert_eq!(
            (status, content_type, body.as_slice()),
            (200, Some("text/html; charset=utf-8"), &b"<p>hi</p>"[..])
        );
        assert_eq!(
            status_of(
                &router,
                &format!("http://presentation.test/media/{sha}.png")
            ),
            Some(200)
        );
        // A browser draws an SVG picture only under its own type.
        std::fs::write(dir.path().join(format!("media/{sha}.svg")), "<svg/>").unwrap();
        let Response::Fulfill { content_type, .. } =
            router.route(&format!("http://presentation.test/media/{sha}.svg"))
        else {
            panic!("svg");
        };
        assert_eq!(content_type, Some("image/svg+xml"));
        assert_eq!(
            status_of(&router, "http://presentation.test/slides/other.html"),
            Some(404)
        );
        assert_eq!(
            status_of(&router, "http://presentation.test/media/../secret.png"),
            Some(404)
        );
        assert_eq!(
            status_of(
                &router,
                &format!("http://presentation.test/media/{sha}.PNG")
            ),
            Some(404)
        );
        assert_eq!(status_of(&router, "http://example.com/x.png"), None);
        assert_eq!(status_of(&router, "http://presentation.testing/x"), None);
        assert_eq!(status_of(&router, "data:image/png;base64,AA"), None);
    }

    fn status_of(router: &PresentationRouter, url: &str) -> Option<u16> {
        status(&router.route(url))
    }

    #[test]
    fn a_gif_is_served_as_its_first_frame() {
        // A 2×1 GIF: one red pixel and one transparent.
        let mut gif = Vec::new();
        {
            let palette = [255u8, 0, 0, 0, 0, 0];
            let mut encoder = gif::Encoder::new(&mut gif, 2, 1, &palette).unwrap();
            let mut frame = gif::Frame::from_indexed_pixels(2, 1, vec![0, 1], Some(1));
            frame.left = 0;
            encoder.write_frame(&frame).unwrap();
        }
        let png = first_frame_png(&gif).unwrap();
        let decoder = png::Decoder::new(std::io::Cursor::new(png));
        let mut reader = decoder.read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut pixels).unwrap();
        assert_eq!(pixels, [255, 0, 0, 255, 0, 0, 0, 0]);
    }

    #[test]
    fn decodes_paths_as_decode_uri_component() {
        assert_eq!(
            decode_path("/slides/a%20b.html").as_deref(),
            Some("/slides/a b.html")
        );
        assert_eq!(decode_path("/bad%2"), None);
    }
}
