//! collect-glyphs: the code points each bound face draws, which the font emitter subsets that
//! face to. It reads the resolved presentation after a11y and the catalog bind-fonts bound against, and writes
//! nothing back: the IR holds no glyph sets.
//!
//! A run, field, styled break or paragraph mark draws with the face CSS matches in its binding's family at its
//! drawn weight and style. Every built container counts (masters, layouts, slides, hidden slides too: the player
//! can still show them); failed containers draw nothing. Per face it adds:
//!
//! - the text's code points as stored;
//! - for `caps: all` and `caps: small`, the full uppercase mapping too (browsers synthesize small caps from capitals
//!   when the font has no `smcp`);
//! - for slide-number and slide-count fields, the digits (the player renumbers when slides are hidden or skipped);
//! - for a paragraph's bullet label, the label's code points with the label's family, at 400 upright and at every
//!   weight and style the paragraph's runs draw (the backend sets the label in the paragraph's line face);
//! - always: [`ALWAYS_DRAWN`](slidock_fonts::glyphs::ALWAYS_DRAWN).
//!
//! A family bound to the `emoji` generic draws with the viewer's emoji font: it has no face to collect for.

use std::collections::{BTreeMap, BTreeSet};

use slidock_core_api::PassError;
use slidock_fonts::glyphs::ALWAYS_DRAWN;
use slidock_fonts::{CodepointSet, FamilyKey, FontCatalog};
use slidock_ir::fonts::{FaceStyle, GenericFamily, font_key};
use slidock_ir::ids::Sha256;
use slidock_ir::resolved::{Element, Presentation};
use slidock_ir::text::{
    Caps, FieldKind, Inline, ResolvedBullet, ResolvedRunProps, ResolvedTextBody,
};

/// A face a deck draws with: the catalog face CSS matches for a drawn slot.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GlyphFace {
    /// The family the binding ships, as the catalog spells it.
    pub family: String,
    pub weight: u32,
    pub italic: bool,
    pub postscript_name: String,
    /// The file the face comes from; `None` for a built-in face, which has no file to ship.
    pub sha256: Option<Sha256>,
}

impl GlyphFace {
    pub fn style(&self) -> FaceStyle {
        if self.italic {
            FaceStyle::Italic
        } else {
            FaceStyle::Normal
        }
    }
}

/// What a deck draws with one face.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FaceGlyphs {
    pub codepoints: CodepointSet,
    /// The drawn weights this face renders (a variable face renders several).
    pub weights: BTreeSet<u32>,
    /// Small caps are drawn with it.
    pub small_caps: bool,
    /// The families the slides name that draw with it, as the requests spell them.
    pub requested: BTreeSet<String>,
    /// The code points of `codepoints` the face keeps whatever the text draws, and the text does not draw.
    pub forced: CodepointSet,
}

/// A drawn slot: the requested family's key, the drawn weight and style.
type Slot = (String, u32, bool);

#[derive(Default)]
struct Drawn {
    slots: BTreeMap<Slot, FaceGlyphs>,
}

impl Drawn {
    fn slot(&mut self, family: &str, weight: u32, italic: bool) -> &mut FaceGlyphs {
        self.slots
            .entry((family.to_owned(), weight, italic))
            .or_default()
    }

    fn text(&mut self, props: &ResolvedRunProps, text: &str) {
        let slot = self.slot(&props.font.family, props.weight, props.italic);
        match props.caps {
            Caps::None => slot.codepoints.insert_text(text),
            Caps::All => slot.codepoints.insert_text_upper(text),
            Caps::Small => {
                slot.codepoints.insert_text_upper(text);
                slot.small_caps = true;
            }
        }
    }

    fn body(&mut self, body: &ResolvedTextBody) {
        for paragraph in &body.paragraphs {
            let mut faces: BTreeSet<(u32, bool)> = BTreeSet::from([(400, false)]);
            let mut add = |drawn: &mut Drawn, props: &ResolvedRunProps, text: &str| {
                faces.insert((props.weight, props.italic));
                drawn.text(props, text);
            };
            for inline in &paragraph.inlines {
                match inline {
                    Inline::Run { text, props, .. } => add(self, props, text),
                    Inline::Field {
                        field,
                        cached,
                        props,
                        ..
                    } => {
                        add(self, props, cached);
                        if matches!(field, FieldKind::SlideNumber | FieldKind::SlideCount) {
                            add(self, props, "0123456789");
                        }
                    }
                    Inline::Break { props: Some(props) } => add(self, props, ""),
                    Inline::Break { props: None } | Inline::Tab {} => {}
                }
            }
            if let Some(end) = &paragraph.end_props {
                add(self, end, "");
            }
            if let ResolvedBullet::Label { text, font, .. } = &paragraph.props.bullet {
                for &(weight, italic) in &faces {
                    self.slot(&font.family, weight, italic)
                        .codepoints
                        .insert_text(text);
                }
            }
        }
    }

    fn elements(&mut self, elements: &[Element]) {
        let mut stack: Vec<&Element> = elements.iter().rev().collect();
        while let Some(element) = stack.pop() {
            match element {
                Element::Shape(shape) => {
                    if let Some(text) = &shape.text {
                        self.body(text);
                    }
                }
                Element::Table(table) => {
                    for cell in table.rows.iter().flat_map(|row| row.cells.iter().flatten()) {
                        self.body(&cell.text);
                    }
                }
                Element::Group(group) => stack.extend(group.children.iter().rev()),
                Element::Foreign(foreign) => {
                    if let Some(elements) =
                        foreign.fallback.as_ref().and_then(|f| f.elements.as_ref())
                    {
                        stack.extend(elements.iter().rev());
                    }
                }
                Element::Picture(_) | Element::Media(_) => {}
            }
        }
    }
}

/// The code points each face draws, by face. `catalog` must be the catalog the presentation's fonts were bound
/// against.
pub fn collect_glyphs(
    presentation: &Presentation,
    catalog: &FontCatalog,
) -> Result<BTreeMap<GlyphFace, FaceGlyphs>, PassError> {
    let mut drawn = Drawn::default();
    for master in presentation.masters.values() {
        drawn.elements(master.body.elements());
    }
    for layout in presentation.layouts.values() {
        drawn.elements(layout.body.elements());
    }
    for slide in &presentation.slides {
        drawn.elements(slide.body.elements());
    }

    let mut faces: BTreeMap<GlyphFace, FaceGlyphs> = BTreeMap::new();
    for ((family, weight, italic), glyphs) in drawn.slots {
        let Some(entry) = presentation.fonts.get(&font_key(&family)) else {
            return Err(PassError::failed(format!(
                "{family} is drawn but was not bound"
            )));
        };
        let binding = &entry.binding;
        if binding.generic == GenericFamily::Emoji {
            continue;
        }
        let bound = catalog
            .get(&FamilyKey::of(&binding.family))
            .ok_or_else(|| {
                PassError::failed(format!(
                    "The font catalog has no {}, which {family} is bound to",
                    binding.family
                ))
            })?;
        let drawn_weight = binding.drawn_weight(weight);
        let style = if italic {
            FaceStyle::Italic
        } else {
            FaceStyle::Normal
        };
        let face = bound.css_match(drawn_weight, style).ok_or_else(|| {
            PassError::failed(format!("The font catalog's {} has no faces", bound.family))
        })?;
        let key = GlyphFace {
            family: bound.family.clone(),
            weight: face.weight,
            italic: face.style == FaceStyle::Italic,
            postscript_name: face.postscript_name.clone(),
            sha256: face.sha256,
        };
        let out = faces.entry(key).or_default();
        out.codepoints.extend(&glyphs.codepoints);
        out.weights.insert(drawn_weight);
        out.small_caps |= glyphs.small_caps;
        out.requested.insert(entry.request.family.clone());
    }
    for glyphs in faces.values_mut() {
        for code_point in ALWAYS_DRAWN {
            if !glyphs.codepoints.contains(code_point) {
                glyphs.codepoints.insert(code_point);
                glyphs.forced.insert(code_point);
            }
        }
    }
    Ok(faces)
}
