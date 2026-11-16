//! `presentation.json` from a resolved presentation: show order, titles, notes as plain text, transitions as data,
//! layouts with their themes, theme colours, and where the presentation came from. Free of timestamps, so the same
//! input writes the same bytes.

use std::collections::BTreeMap;

use slidock_canon::utf16;
use slidock_format::manifest::{
    Advance, Generator, Manifest, ManifestLayout, ManifestSize, ManifestSlide, ManifestSource,
    ManifestTheme, ManifestTransition, SchemeColors, SizeUnit, TransitionFallback,
};
use slidock_format::paths::{layout_path, slide_path};
use slidock_format::serialize_manifest;
use slidock_ir::resolved::{Presentation, Slide};
use slidock_ir::text::TextBody;
use slidock_ir::timeline::{self, Transition};

use crate::color::literal_color;
use crate::error::Result;

/// The longest title, in UTF-16 code units.
const MAX_TITLE: usize = 256;

/// The input's name without its extension: `Quarterly Review.pptx` → `Quarterly Review`,
/// `min.presentation-ir.json` → `min`.
pub fn presentation_title(name: &str) -> String {
    let base = strip_suffix_ignore_ascii_case(name, ".presentation-ir.json").unwrap_or(name);
    let base = match base.rfind('.') {
        Some(dot) => {
            let ext = base.get(dot + 1..).unwrap_or("");
            if (1..=8).contains(&ext.len()) && ext.bytes().all(|b| b.is_ascii_alphanumeric()) {
                base.get(..dot).unwrap_or(base)
            } else {
                base
            }
        }
        None => base,
    };
    let title = if base.is_empty() { name } else { base };
    utf16::clip(title, MAX_TITLE).to_owned()
}

fn strip_suffix_ignore_ascii_case<'a>(text: &'a str, suffix: &str) -> Option<&'a str> {
    let at = text.len().checked_sub(suffix.len())?;
    let tail = text.get(at..)?;
    tail.eq_ignore_ascii_case(suffix)
        .then(|| text.get(..at))
        .flatten()
}

/// Notes as plain text: paragraphs joined by newlines, breaks as newlines, tabs as tabs, fields as their cached
/// values, trailing whitespace removed.
pub fn notes_text(notes: Option<&TextBody>) -> String {
    crate::deck::notes_paragraphs(notes).join("\n")
}

fn transition_entry(transition: &Transition) -> ManifestTransition {
    let fallback = match transition.effect.fallback {
        timeline::TransitionFallback::None => TransitionFallback::None,
        timeline::TransitionFallback::Dissolve => TransitionFallback::Dissolve,
        timeline::TransitionFallback::Push => TransitionFallback::Push,
        timeline::TransitionFallback::Wipe => TransitionFallback::Wipe,
    };
    ManifestTransition {
        effect: transition.effect.id.to_string(),
        fallback,
        duration_ms: transition.duration_ms.0,
        advance: transition.advance.as_ref().map(|advance| Advance {
            on_click: advance.on_click,
            after_ms: advance.after_ms.map(|ms| ms.0),
        }),
    }
}

/// A slide's title for navigation: a11y's, or the one a failed slide kept; `""` when it has none.
pub(crate) fn slide_title(slide: &Slide) -> String {
    match slide.body.built() {
        Some(built) => built.title.clone(),
        None => slide
            .failed_a11y
            .as_ref()
            .and_then(|kept| kept.title.clone())
            .unwrap_or_default(),
    }
}

fn slide_entry(slide: &Slide) -> ManifestSlide {
    let notes = notes_text(slide.notes.as_ref());
    let title = slide_title(slide);
    ManifestSlide {
        id: slide.id.to_string(),
        file: slide_path(slide.id.as_str()),
        title,
        hidden: slide.hidden,
        layout: slide.layout.as_ref().map(ToString::to_string),
        notes: (!notes.is_empty()).then_some(notes),
        transition: slide.transition.as_ref().map(transition_entry),
        failed: slide
            .body
            .failed()
            .map(|failed| failed.code.as_str().to_owned()),
    }
}

/// Where the presentation came from, as the driver hands it over.
pub type Provenance = ManifestSource;

pub fn build_manifest(
    presentation: &Presentation,
    source: &Provenance,
    generator: &Generator,
) -> Result<String> {
    let mut layouts: Vec<ManifestLayout> = presentation
        .layouts
        .values()
        .map(|layout| {
            let theme = layout
                .master
                .as_ref()
                .and_then(|master| presentation.masters.get(master))
                .and_then(|master| master.theme.as_ref());
            ManifestLayout {
                id: layout.id.to_string(),
                file: layout_path(layout.id.as_str()),
                name: layout.name.clone().filter(|name| !name.is_empty()),
                theme: theme.map(ToString::to_string),
                failed: layout
                    .body
                    .failed()
                    .map(|failed| failed.code.as_str().to_owned()),
            }
        })
        .collect();
    layouts.sort_by(|a, b| utf16::cmp(&a.id, &b.id));
    let mut themes = BTreeMap::new();
    for theme in presentation.themes.values() {
        let c = &theme.colors;
        let colors = SchemeColors {
            dk1: literal_color(&c.dk1)?,
            lt1: literal_color(&c.lt1)?,
            dk2: literal_color(&c.dk2)?,
            lt2: literal_color(&c.lt2)?,
            accent1: literal_color(&c.accent1)?,
            accent2: literal_color(&c.accent2)?,
            accent3: literal_color(&c.accent3)?,
            accent4: literal_color(&c.accent4)?,
            accent5: literal_color(&c.accent5)?,
            accent6: literal_color(&c.accent6)?,
            hlink: literal_color(&c.hlink)?,
            fol_hlink: literal_color(&c.fol_hlink)?,
        };
        themes.insert(
            theme.id.to_string(),
            ManifestTheme {
                name: theme.name.clone().filter(|name| !name.is_empty()),
                colors,
            },
        );
    }
    let manifest = Manifest {
        title: presentation_title(&source.name),
        size: ManifestSize {
            w: presentation.size.w.to_f64(),
            h: presentation.size.h.to_f64(),
            unit: SizeUnit::Pt,
        },
        slides: presentation.slides.iter().map(slide_entry).collect(),
        layouts,
        themes,
        source: Some(source.clone()),
        generator: Some(generator.clone()),
    };
    Ok(serialize_manifest(&manifest)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_the_extension_from_the_title() {
        assert_eq!(
            presentation_title("Quarterly Review.pptx"),
            "Quarterly Review"
        );
        assert_eq!(presentation_title("min.presentation-ir.json"), "min");
        assert_eq!(presentation_title("MIN.Presentation-IR.JSON"), "MIN");
        assert_eq!(presentation_title("deck.final.key"), "deck.final");
        assert_eq!(
            presentation_title("archive.tar.toolongext"),
            "archive.tar.toolongext"
        );
        assert_eq!(presentation_title(".pptx"), ".pptx");
        assert_eq!(presentation_title("a.b-c"), "a.b-c");
        assert_eq!(utf16::len(&presentation_title(&"😀".repeat(200))), 256);
    }
}
