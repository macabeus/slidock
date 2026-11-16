//! The bundled substitutes: Liberation 2.1.5, DejaVu Sans 2.37, Carlito and Caladea, the floor
//! under every font source, as the WOFF2 files under `fonts/` that its manifest, `fonts/core.json`, lists. The CLI
//! builds them in (feature `embed`, `build.rs`) and the web app serves the same files at `fonts/core/`, so both hosts
//! ship the same bytes. Each family's licence sits beside its files.

/// A face's style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Normal,
    Italic,
}

/// A face of the manifest, with its file's bytes.
#[derive(Clone, Copy, Debug)]
pub struct Face {
    pub family: &'static str,
    pub weight: u32,
    pub style: Style,
    /// The file's name beside the manifest.
    pub file: &'static str,
    /// Of `bytes`, as the manifest records it.
    pub sha256: &'static str,
    /// SPDX id.
    pub licence: &'static str,
    pub bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/faces.rs"));
