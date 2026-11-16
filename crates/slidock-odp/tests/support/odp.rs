//! A minimal ODP writer for tests: the stored `mimetype` first, then content.xml with the pages, styles.xml with the
//! common styles and master pages, meta.xml, and any other file, from XML snippets. Every part is what LibreOffice
//! and Google Slides write, cut down to what a test needs.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub const ODP_MIMETYPE: &str = "application/vnd.oasis.opendocument.presentation";

pub const NS: &str = concat!(
    r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" "#,
    r#"xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" "#,
    r#"xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" "#,
    r#"xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" "#,
    r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" "#,
    r#"xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" "#,
    r#"xmlns:xlink="http://www.w3.org/1999/xlink" "#,
    r#"xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" "#,
    r#"xmlns:presentation="urn:oasis:names:tc:opendocument:xmlns:presentation:1.0" "#,
    r#"xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" "#,
    r#"xmlns:dr3d="urn:oasis:names:tc:opendocument:xmlns:dr3d:1.0" "#,
    r#"xmlns:anim="urn:oasis:names:tc:opendocument:xmlns:animation:1.0" "#,
    r#"xmlns:smil="urn:oasis:names:tc:opendocument:xmlns:smil-compatible:1.0" "#,
    r#"xmlns:script="urn:oasis:names:tc:opendocument:xmlns:script:1.0" "#,
    r#"xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0""#,
);

const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>"#;

#[derive(Clone, Debug, Default)]
pub struct MasterSpec {
    pub name: String,
    pub display_name: Option<String>,
    /// The drawing-page style the master page uses.
    pub style: Option<String>,
    pub page_layout: Option<String>,
    /// The master page's drawables.
    pub body: String,
}

impl MasterSpec {
    pub fn named(name: &str) -> Self {
        MasterSpec {
            name: name.to_owned(),
            ..MasterSpec::default()
        }
    }

    pub fn style(mut self, style: &str) -> Self {
        self.style = Some(style.to_owned());
        self
    }

    pub fn display(mut self, name: &str) -> Self {
        self.display_name = Some(name.to_owned());
        self
    }

    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct PageSpec {
    pub name: Option<String>,
    pub master: Option<String>,
    /// The drawing-page style.
    pub style: Option<String>,
    /// More attributes of `draw:page`.
    pub attrs: String,
    /// The page's drawables (and its timing tree).
    pub body: String,
    /// The notes page's drawables (`presentation:notes`).
    pub notes: Option<String>,
}

impl PageSpec {
    pub fn new() -> Self {
        PageSpec::default()
    }

    pub fn body(body: impl Into<String>) -> Self {
        PageSpec {
            body: body.into(),
            ..PageSpec::default()
        }
    }

    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_owned());
        self
    }

    pub fn master(mut self, master: &str) -> Self {
        self.master = Some(master.to_owned());
        self
    }

    pub fn style(mut self, style: &str) -> Self {
        self.style = Some(style.to_owned());
        self
    }

    pub fn attrs(mut self, attrs: &str) -> Self {
        self.attrs = attrs.to_owned();
        self
    }

    pub fn with_body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    pub fn notes(mut self, notes: impl Into<String>) -> Self {
        self.notes = Some(notes.into());
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct OdpSpec {
    pub pages: Vec<PageSpec>,
    pub masters: Option<Vec<MasterSpec>>,
    /// Children of styles.xml's `office:styles` (common styles, named resources).
    pub styles: String,
    /// Children of styles.xml's `office:automatic-styles` besides the page layout.
    pub master_automatic_styles: String,
    /// Children of content.xml's `office:automatic-styles`.
    pub automatic_styles: String,
    pub font_faces: String,
    /// Children of `office:presentation` after the pages (footer and date declarations).
    pub declarations: String,
    /// The page size.
    pub size: Option<(String, String)>,
    pub generator: Option<String>,
    /// More files, by path (pictures, media), or replacements of the parts written here.
    pub parts: Vec<(String, Vec<u8>)>,
    /// The `mimetype` entry's bytes.
    pub mimetype: Option<String>,
    /// More `manifest:file-entry` elements (embedded objects' document types).
    pub manifest: String,
}

impl OdpSpec {
    pub fn pages(pages: Vec<PageSpec>) -> Self {
        OdpSpec {
            pages,
            ..OdpSpec::default()
        }
    }

    pub fn part(mut self, path: &str, bytes: impl Into<Vec<u8>>) -> Self {
        self.parts.push((path.to_owned(), bytes.into()));
        self
    }
}

/// LibreOffice's graphic default style and its `standard` style, which every document carries.
pub const DEFAULT_STYLES: &str = concat!(
    r##"<style:default-style style:family="graphic"><style:graphic-properties svg:stroke-color="#3465a4" "##,
    r##"draw:fill-color="#729fcf" fo:wrap-option="no-wrap"/><style:paragraph-properties/>"##,
    r#"<style:text-properties style:font-name="Liberation Serif" fo:font-size="24pt"/></style:default-style>"#,
    r#"<style:style style:name="standard" style:family="graphic"><style:graphic-properties draw:stroke="solid" "#,
    r##"svg:stroke-width="0cm" svg:stroke-color="#3465a4" draw:fill="solid" draw:fill-color="#729fcf" "##,
    r#"fo:padding-top="0.125cm" fo:padding-bottom="0.125cm" fo:padding-left="0.25cm" fo:padding-right="0.25cm"/>"#,
    r#"<style:paragraph-properties fo:margin-left="0cm" fo:margin-right="0cm" fo:margin-top="0cm" "#,
    r#"fo:margin-bottom="0cm" fo:line-height="100%" fo:text-indent="0cm"/><style:text-properties "#,
    r##"style:font-name="Liberation Sans" fo:font-size="18pt" fo:color="#000000"/></style:style>"##,
);

const FONT_FACES: &str = concat!(
    r#"<style:font-face style:name="Liberation Sans" svg:font-family="&apos;Liberation Sans&apos;"/>"#,
    r#"<style:font-face style:name="Liberation Serif" svg:font-family="&apos;Liberation Serif&apos;"/>"#,
);

/// The bytes of a package built from `spec`.
pub fn odp(spec: &OdpSpec) -> Vec<u8> {
    zip_entries(&files(spec))
}

/// The files of a package built from `spec`, `mimetype` first.
pub fn files(spec: &OdpSpec) -> Vec<(String, Vec<u8>)> {
    let masters = spec
        .masters
        .clone()
        .unwrap_or_else(|| vec![MasterSpec::named("Default")]);
    let (w, h) = spec
        .size
        .clone()
        .unwrap_or_else(|| ("10in".to_owned(), "5.625in".to_owned()));
    let font_faces = format!(
        "<office:font-face-decls>{FONT_FACES}{}</office:font-face-decls>",
        spec.font_faces
    );
    let master_pages: String = masters
        .iter()
        .map(|master| {
            let display = master
                .display_name
                .as_ref()
                .map(|d| format!(r#" style:display-name="{d}""#))
                .unwrap_or_default();
            let style = master
                .style
                .as_ref()
                .map(|s| format!(r#" draw:style-name="{s}""#))
                .unwrap_or_default();
            format!(
                r#"<style:master-page style:name="{}"{display} style:page-layout-name="{}"{style}>{}</style:master-page>"#,
                master.name,
                master.page_layout.as_deref().unwrap_or("PM1"),
                master.body
            )
        })
        .collect();
    let styles = format!(
        concat!(
            r#"{xml}<office:document-styles {ns} office:version="1.2">{faces}"#,
            "<office:styles>{defaults}{styles}</office:styles>",
            r#"<office:automatic-styles><style:page-layout style:name="PM1"><style:page-layout-properties "#,
            r#"fo:page-width="{w}" fo:page-height="{h}" style:print-orientation="landscape"/></style:page-layout>"#,
            "{automatic}</office:automatic-styles>",
            "<office:master-styles>{masters}</office:master-styles></office:document-styles>"
        ),
        xml = XML,
        ns = NS,
        faces = font_faces,
        defaults = DEFAULT_STYLES,
        styles = spec.styles,
        w = w,
        h = h,
        automatic = spec.master_automatic_styles,
        masters = master_pages,
    );
    let first_master = masters.first().map_or("Default", |m| m.name.as_str());
    let pages: String = spec
        .pages
        .iter()
        .enumerate()
        .map(|(index, page)| {
            let name = page
                .name
                .clone()
                .unwrap_or_else(|| format!("page{}", index + 1));
            let style = page
                .style
                .as_ref()
                .map(|s| format!(r#" draw:style-name="{s}""#))
                .unwrap_or_default();
            let notes = page
                .notes
                .as_ref()
                .map(|n| format!("<presentation:notes>{n}</presentation:notes>"))
                .unwrap_or_default();
            format!(
                r#"<draw:page draw:name="{name}" draw:master-page-name="{}"{style}{}>{}{notes}</draw:page>"#,
                page.master.as_deref().unwrap_or(first_master),
                page.attrs,
                page.body
            )
        })
        .collect();
    let content = format!(
        concat!(
            r#"{xml}<office:document-content {ns} office:version="1.2">{faces}"#,
            "<office:automatic-styles>{automatic}</office:automatic-styles>",
            "<office:body><office:presentation>{pages}{declarations}</office:presentation></office:body>",
            "</office:document-content>"
        ),
        xml = XML,
        ns = NS,
        faces = font_faces,
        automatic = spec.automatic_styles,
        pages = pages,
        declarations = spec.declarations,
    );
    let meta = format!(
        concat!(
            r#"{xml}<office:document-meta {ns} office:version="1.2"><office:meta><meta:generator>"#,
            "{generator}</meta:generator></office:meta></office:document-meta>"
        ),
        xml = XML,
        ns = NS,
        generator = spec
            .generator
            .as_deref()
            .unwrap_or("LibreOffice/7.6.4.1$Linux_X86_64 LibreOffice_project/e19e193f88cd"),
    );
    let manifest = format!(
        concat!(
            r#"{xml}<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0">"#,
            r#"<manifest:file-entry manifest:full-path="/" manifest:media-type="{mime}"/>{more}"#,
            "</manifest:manifest>"
        ),
        xml = XML,
        mime = ODP_MIMETYPE,
        more = spec.manifest,
    );
    let mut files: Vec<(String, Vec<u8>)> = vec![(
        "mimetype".to_owned(),
        spec.mimetype
            .as_deref()
            .unwrap_or(ODP_MIMETYPE)
            .as_bytes()
            .to_vec(),
    )];
    let mut rest: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let ordered = [
        ("content.xml", content.into_bytes()),
        ("styles.xml", styles.into_bytes()),
        ("meta.xml", meta.into_bytes()),
        ("META-INF/manifest.xml", manifest.into_bytes()),
    ];
    let mut order: Vec<String> = Vec::new();
    for (path, bytes) in ordered {
        order.push(path.to_owned());
        rest.insert(path.to_owned(), bytes);
    }
    for (path, bytes) in &spec.parts {
        if !rest.contains_key(path) {
            order.push(path.clone());
        }
        rest.insert(path.clone(), bytes.clone());
    }
    for path in order {
        let bytes = rest.remove(&path).unwrap();
        files.push((path, bytes));
    }
    files
}

/// A zip of these files in order: the first stored, the rest deflated.
pub fn zip_entries(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (index, (path, bytes)) in files.iter().enumerate() {
        let method = if index == 0 {
            CompressionMethod::Stored
        } else {
            CompressionMethod::Deflated
        };
        let options = SimpleFileOptions::default().compression_method(method);
        writer.start_file(path.as_str(), options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// A zip of these files, each stored or deflated as given.
pub fn zip_with(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (path, bytes, deflated) in files {
        let method = if *deflated {
            CompressionMethod::Deflated
        } else {
            CompressionMethod::Stored
        };
        writer
            .start_file(
                *path,
                SimpleFileOptions::default().compression_method(method),
            )
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// The files of a zip, in order.
pub fn unzip(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..archive.len())
        .map(|i| {
            let mut file = archive.by_index(i).unwrap();
            let mut data = Vec::new();
            file.read_to_end(&mut data).unwrap();
            (file.name().unwrap().into_owned(), data)
        })
        .collect()
}

/// Replaces (or adds) one file of a file list.
pub fn replace(files: &mut Vec<(String, Vec<u8>)>, path: &str, bytes: impl Into<Vec<u8>>) {
    let bytes = bytes.into();
    match files.iter_mut().find(|(name, _)| name == path) {
        Some(file) => file.1 = bytes,
        None => files.push((path.to_owned(), bytes)),
    }
}

/// A drawing-page style.
pub fn page_style(name: &str, props: &str) -> String {
    format!(
        r#"<style:style style:name="{name}" style:family="drawing-page"><style:drawing-page-properties {props}/></style:style>"#
    )
}

/// A drawing-page style with a parent.
pub fn page_style_under(name: &str, props: &str, parent: &str) -> String {
    format!(
        r#"<style:style style:name="{name}" style:family="drawing-page" style:parent-style-name="{parent}"><style:drawing-page-properties {props}/></style:style>"#
    )
}
