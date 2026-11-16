//! The marker vocabulary slidock prints into slide documents and the deck page. Every slidock-owned marker carries
//! one prefix, [`PREFIX`].

/// The prefix literal; [`PREFIX`] and every prefixed constant below are built from it.
macro_rules! prefix {
    () => {
        "sd"
    };
}

/// A `data-sd-<name>` attribute name.
macro_rules! data_attribute {
    ($name:literal) => {
        concat!("data-", prefix!(), "-", $name)
    };
}

/// The marker prefix.
pub const PREFIX: &str = prefix!();
/// The start of every prefixed attribute name.
pub const DATA_PREFIX: &str = concat!("data-", prefix!(), "-");
/// The start of every theme colour variable (`var(--sd-color-tx1,#ffffff)`).
pub const COLOR_VAR_PREFIX: &str = concat!("--", prefix!(), "-color-");

/// The background box, the first child of a document's `<main>`: compose keeps the slide's or else the layout's.
pub const BACKGROUND: &str = data_attribute!("bg");
/// A text box.
pub const TEXT: &str = data_attribute!("text");
/// A bullet label: a by-word or by-character build skips it, a by-paragraph build animates it with its paragraph.
pub const BULLET: &str = data_attribute!("bullet");
/// The marker a failed container prints, holding its diagnostic code.
pub const FAILED: &str = data_attribute!("failed");
/// An SVG geometry layer.
pub const GEOMETRY: &str = data_attribute!("geom");
/// A foreign-object marker.
pub const FOREIGN_MARKER: &str = data_attribute!("marker");
/// A picture mask.
pub const MASK: &str = data_attribute!("mask");
/// The box a shadowed picture clips in, so its root's shadow is not cut.
pub const SHADOW_CLIP: &str = data_attribute!("clip");
/// The Magic Move / Morph match key of an element.
pub const MATCH: &str = data_attribute!("match");
/// A deck page's slide template: `<template data-sd-slide="<id>">`.
pub const SLIDE_TEMPLATE: &str = data_attribute!("slide");

/// The paragraph index of a text box's paragraph block, for paragraph-range builds.
pub const PARAGRAPH: &str = "data-p";

/// The deck page's `<script type="application/json">` holding the deck data.
pub const DECK_SCRIPT_ID: &str = concat!(prefix!(), "-deck");
/// The deck page's `<style>` of `@font-face` rules.
pub const FONTS_STYLE_ID: &str = concat!(prefix!(), "-fonts");
/// The deck page's `<style>` for the player chrome.
pub const CHROME_STYLE_ID: &str = concat!(prefix!(), "-chrome");

/// Unprefixed attributes of the document format.
pub mod attr {
    /// `<main data-slide="<id>">`: a slide document's root.
    pub const SLIDE: &str = "data-slide";
    /// `<main data-layout="<id>">`: the layout a slide shows behind its own elements.
    pub const LAYOUT: &str = "data-layout";
    /// `data-show-layout="false"` on a slide's root hides its layout's elements.
    pub const SHOW_LAYOUT: &str = "data-show-layout";
    /// `<main data-layout-root="<id>">`: a layout document's root.
    pub const LAYOUT_ROOT: &str = "data-layout-root";
    /// A failed container's diagnostic code, on its root.
    pub const FAILED: &str = "data-failed";
    /// The id of a printed element.
    pub const ELEMENT: &str = "data-el";
    /// `data-field="slideNumber"`: a field compose fills.
    pub const FIELD: &str = "data-field";
    /// The field that shows the slide's show number.
    pub const SLIDE_NUMBER_FIELD: &str = "slideNumber";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_prefixed_marker_derives_from_the_prefix() {
        assert_eq!(PREFIX, "sd");
        assert_eq!(DATA_PREFIX, "data-sd-");
        assert_eq!(COLOR_VAR_PREFIX, "--sd-color-");
        for marker in [
            BACKGROUND,
            TEXT,
            BULLET,
            FAILED,
            GEOMETRY,
            FOREIGN_MARKER,
            MASK,
            SHADOW_CLIP,
            MATCH,
            SLIDE_TEMPLATE,
        ] {
            assert!(marker.starts_with(DATA_PREFIX), "{marker}");
        }
        assert_eq!(BACKGROUND, "data-sd-bg");
        assert_eq!(SHADOW_CLIP, "data-sd-clip");
        assert_eq!(DECK_SCRIPT_ID, "sd-deck");
        assert_eq!(FONTS_STYLE_ID, "sd-fonts");
    }
}
