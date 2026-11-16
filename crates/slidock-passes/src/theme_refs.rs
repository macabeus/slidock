//! Theme references against a container's theme and effective colour map: a theme colour gets its `resolved` value,
//! recomputed on every run so an edited theme re-themes the presentation, and a theme font becomes the family it names.
//! resolve-theme applies this to a whole container; cascade-text to the values it copies from the presentation defaults
//! and the master, whose references must resolve in the container's own colour map. Speaker notes, `ext` bags and
//! Foreign payloads are left as written.

use slidock_color::apply_color_mods;
use slidock_core_api::{ContainerView, OwnedContainer, PassError};
use slidock_ir::color::{
    Color, ColorMap, ColorSpace, MappedSlot, RgbColor, SchemeColors, SchemeSlot,
};
use slidock_ir::elements::ForeignFallback;
use slidock_ir::paint::{
    CellFill, EllipseLowered, Fill, GradientStop, LinearAngle, LinearAngled, LinearLowered,
    OdfGradient, PathGradient, RadialLowered, RadialSource, RectLowered, Stroke, StrokePaint,
};
use slidock_ir::presentation::{TextStyles, Theme};
use slidock_ir::shadow::Shadow;
use slidock_ir::source::{
    Cell, Element, Foreign, Group, Layout, Master, MediaElement, Picture, Shape, Slide, Table,
};
use slidock_ir::table::{Border, CellBorders, TableRow};
use slidock_ir::text::{
    Bullet, FamilyFont, FontRef, Inline, LevelStyle, LevelStyles, OrText, Paragraph,
    PartialParaProps, PartialRunProps, TextBody, ThemeFontSlot,
};
use slidock_ir::units::UnitRatio;

/// The family a container without a theme font resolves its theme fonts to.
pub const FALLBACK_FONT: &str = "Liberation Sans";

fn srgb(hex: u32) -> RgbColor {
    let channel = |shift: u32| {
        let value = f64::from((hex >> shift) & 0xff) / 255.0;
        UnitRatio::quantise(value).unwrap_or(UnitRatio::ZERO)
    };
    let one = UnitRatio::from_steps(100_000);
    RgbColor::new(ColorSpace::Srgb, channel(16), channel(8), channel(0), one)
}

/// The colours a container without a theme resolves against: Office's defaults.
fn fallback_colors() -> SchemeColors {
    SchemeColors {
        dk1: srgb(0x000000),
        lt1: srgb(0xffffff),
        dk2: srgb(0x44546a),
        lt2: srgb(0xe7e6e6),
        accent1: srgb(0x4472c4),
        accent2: srgb(0xed7d31),
        accent3: srgb(0xa5a5a5),
        accent4: srgb(0xffc000),
        accent5: srgb(0x5b9bd5),
        accent6: srgb(0x70ad47),
        hlink: srgb(0x0563c1),
        fol_hlink: srgb(0x954f72),
    }
}

/// What a container's theme references resolve against.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeContext {
    pub colors: SchemeColors,
    /// The effective colour map; none when no container of the chain has one.
    pub map: Option<ColorMap>,
    pub major: String,
    pub minor: String,
    /// What the theme lacks, for resolve-theme to report: `theme`, `major font`, `minor font`.
    pub missing: Vec<&'static str>,
}

impl ThemeContext {
    /// The scheme slot a reference shows through the colour map; `bg1`, `tx1`, `bg2` and `tx2` show `lt1`,
    /// `dk1`, `lt2` and `dk2` without one.
    pub fn scheme_slot(&self, slot: MappedSlot) -> SchemeSlot {
        if let Some(map) = &self.map {
            return map.get(slot);
        }
        match slot {
            MappedSlot::Bg1 => SchemeSlot::Lt1,
            MappedSlot::Tx1 => SchemeSlot::Dk1,
            MappedSlot::Bg2 => SchemeSlot::Lt2,
            MappedSlot::Tx2 => SchemeSlot::Dk2,
            other => other.scheme().unwrap_or(SchemeSlot::Dk1),
        }
    }

    pub fn font(&self, slot: ThemeFontSlot) -> &str {
        match slot {
            ThemeFontSlot::Major => &self.major,
            ThemeFontSlot::Minor => &self.minor,
        }
    }
}

/// The effective colour map: a slide's override, else its layout's, else the master's map.
fn color_map_of(view: &ContainerView<'_>) -> Option<ColorMap> {
    use slidock_ir::source::Container;
    match view.container {
        Container::Master(master) => master.color_map.clone(),
        Container::Layout(layout) => layout
            .color_map_override
            .clone()
            .or_else(|| view.master.and_then(|m| m.color_map.clone())),
        Container::Slide(slide) => slide
            .color_map_override
            .clone()
            .or_else(|| view.layout.and_then(|l| l.color_map_override.clone()))
            .or_else(|| view.master.and_then(|m| m.color_map.clone())),
    }
}

/// The theme a view's references resolve against. An empty theme typeface takes the other slot's, as Office does.
pub fn theme_context(view: &ContainerView<'_>) -> ThemeContext {
    theme_context_of(view.theme, color_map_of(view))
}

fn theme_context_of(theme: Option<&Theme>, map: Option<ColorMap>) -> ThemeContext {
    let mut missing = Vec::new();
    let (colors, major, minor) = match theme {
        Some(theme) => {
            let major = theme.fonts.major.latin.clone();
            let minor = theme.fonts.minor.latin.clone();
            if major.is_empty() {
                missing.push("major font");
            }
            if minor.is_empty() {
                missing.push("minor font");
            }
            (theme.colors.clone(), major, minor)
        }
        None => {
            missing.push("theme");
            (fallback_colors(), String::new(), String::new())
        }
    };
    let pick = |first: &str, second: &str| {
        [first, second, FALLBACK_FONT]
            .into_iter()
            .find(|name| !name.is_empty())
            .unwrap_or(FALLBACK_FONT)
            .to_owned()
    };
    ThemeContext {
        colors,
        map,
        major: pick(&major, &minor),
        minor: pick(&minor, &major),
        missing,
    }
}

/// A value whose theme references resolve in place.
pub(crate) trait ThemeRefs {
    /// Resolves every theme colour and replaces every theme font by its family.
    fn resolve_refs(&mut self, theme: &ThemeContext) -> Result<(), PassError>;

    /// Whether the value holds any theme reference.
    fn has_refs(&self) -> bool;
}

impl ThemeRefs for Color {
    fn resolve_refs(&mut self, theme: &ThemeContext) -> Result<(), PassError> {
        if let Color::Theme(color) = self {
            let base = theme.colors.get(theme.scheme_slot(color.slot));
            let resolved = apply_color_mods(base, &color.mods).map_err(|error| {
                PassError::failed(format!("A theme colour does not resolve: {error}"))
            })?;
            color.resolved = Some(resolved);
        }
        Ok(())
    }

    fn has_refs(&self) -> bool {
        matches!(self, Color::Theme(_))
    }
}

impl ThemeRefs for FontRef {
    fn resolve_refs(&mut self, theme: &ThemeContext) -> Result<(), PassError> {
        if let FontRef::Theme(font) = self {
            let slot = font.slot;
            let mut family = FamilyFont::new(theme.font(slot));
            family.theme = Some(slot);
            *self = FontRef::Family(family);
        }
        Ok(())
    }

    fn has_refs(&self) -> bool {
        matches!(self, FontRef::Theme(_))
    }
}

impl<T: ThemeRefs> ThemeRefs for Box<T> {
    fn resolve_refs(&mut self, theme: &ThemeContext) -> Result<(), PassError> {
        T::resolve_refs(self, theme)
    }

    fn has_refs(&self) -> bool {
        T::has_refs(self)
    }
}

impl<T: ThemeRefs> ThemeRefs for Option<T> {
    fn resolve_refs(&mut self, theme: &ThemeContext) -> Result<(), PassError> {
        match self {
            Some(value) => value.resolve_refs(theme),
            None => Ok(()),
        }
    }

    fn has_refs(&self) -> bool {
        self.as_ref().is_some_and(ThemeRefs::has_refs)
    }
}

impl<T: ThemeRefs> ThemeRefs for Vec<T> {
    fn resolve_refs(&mut self, theme: &ThemeContext) -> Result<(), PassError> {
        self.iter_mut()
            .try_for_each(|item| item.resolve_refs(theme))
    }

    fn has_refs(&self) -> bool {
        self.iter().any(ThemeRefs::has_refs)
    }
}

impl<T: ThemeRefs> ThemeRefs for OrText<T> {
    fn resolve_refs(&mut self, theme: &ThemeContext) -> Result<(), PassError> {
        match self {
            OrText::Value(value) => value.resolve_refs(theme),
            OrText::Text => Ok(()),
        }
    }

    fn has_refs(&self) -> bool {
        matches!(self, OrText::Value(value) if value.has_refs())
    }
}

/// Implements [`ThemeRefs`] by visiting the listed fields.
macro_rules! refs_in_fields {
    ($ty:ty { $($field:ident),* $(,)? }) => {
        impl ThemeRefs for $ty {
            fn resolve_refs(&mut self, _theme: &ThemeContext) -> Result<(), PassError> {
                $( self.$field.resolve_refs(_theme)?; )*
                Ok(())
            }

            fn has_refs(&self) -> bool {
                false $( || self.$field.has_refs() )*
            }
        }
    };
}

refs_in_fields!(GradientStop<Color> { color });
refs_in_fields!(LinearAngle { stops });
refs_in_fields!(RadialSource { stops });
refs_in_fields!(PathGradient { stops });
refs_in_fields!(OdfGradient { stops });
refs_in_fields!(LinearLowered<Color> { stops });
refs_in_fields!(RadialLowered<Color> { stops });
refs_in_fields!(RectLowered<Color> { stops });
refs_in_fields!(EllipseLowered<Color> { stops });
refs_in_fields!(LinearAngled<Color> { stops });
refs_in_fields!(Stroke<Color> { fill });
refs_in_fields!(Border<Color> { fill });
refs_in_fields!(CellBorders<Color> { top, right, bottom, left, diagonal_down, diagonal_up });
refs_in_fields!(Shadow<Color> { color });
refs_in_fields!(PartialRunProps {
    font,
    color,
    highlight,
    shadow
});
refs_in_fields!(PartialParaProps { bullet });
refs_in_fields!(LevelStyle { para, run });
refs_in_fields!(LevelStyles {
    l0,
    l1,
    l2,
    l3,
    l4,
    l5,
    l6,
    l7,
    l8
});
refs_in_fields!(TextStyles { title, body, other });
refs_in_fields!(TextBody {
    list_style,
    paragraphs
});
refs_in_fields!(Cell {
    fill,
    borders,
    text
});
refs_in_fields!(TableRow<Cell> { cells });
refs_in_fields!(ForeignFallback<Element> { elements });
refs_in_fields!(Shape {
    fill,
    stroke,
    text,
    shadow
});
refs_in_fields!(Picture { stroke, shadow });
refs_in_fields!(MediaElement { shadow });
refs_in_fields!(Table { rows });
refs_in_fields!(Group { children, shadow });
refs_in_fields!(Foreign { fallback });
refs_in_fields!(Master {
    text_styles,
    background,
    elements
});
refs_in_fields!(Layout {
    background,
    elements
});
refs_in_fields!(Slide {
    background,
    elements
});

impl ThemeRefs for StrokePaint<Color> {
    fn resolve_refs(&mut self, theme: &ThemeContext) -> Result<(), PassError> {
        match self {
            StrokePaint::Solid { color } => color.resolve_refs(theme),
            StrokePaint::None => Ok(()),
        }
    }

    fn has_refs(&self) -> bool {
        matches!(self, StrokePaint::Solid { color } if color.has_refs())
    }
}

impl ThemeRefs for Fill {
    fn resolve_refs(&mut self, theme: &ThemeContext) -> Result<(), PassError> {
        match self {
            Fill::None | Fill::Image { .. } => Ok(()),
            Fill::Solid { color } => color.resolve_refs(theme),
            Fill::LinearAngle(g) => g.resolve_refs(theme),
            Fill::RadialSource(g) => g.resolve_refs(theme),
            Fill::Path(g) => g.resolve_refs(theme),
            Fill::Odf(g) => g.resolve_refs(theme),
            Fill::Linear(g) => g.resolve_refs(theme),
            Fill::Radial(g) => g.resolve_refs(theme),
            Fill::Rect(g) => g.resolve_refs(theme),
            Fill::Ellipse(g) => g.resolve_refs(theme),
        }
    }

    fn has_refs(&self) -> bool {
        match self {
            Fill::None | Fill::Image { .. } => false,
            Fill::Solid { color } => color.has_refs(),
            Fill::LinearAngle(g) => g.has_refs(),
            Fill::RadialSource(g) => g.has_refs(),
            Fill::Path(g) => g.has_refs(),
            Fill::Odf(g) => g.has_refs(),
            Fill::Linear(g) => g.has_refs(),
            Fill::Radial(g) => g.has_refs(),
            Fill::Rect(g) => g.has_refs(),
            Fill::Ellipse(g) => g.has_refs(),
        }
    }
}

impl ThemeRefs for CellFill {
    fn resolve_refs(&mut self, theme: &ThemeContext) -> Result<(), PassError> {
        match self {
            CellFill::Fill(fill) => fill.resolve_refs(theme),
            CellFill::Angled(angled) => angled.resolve_refs(theme),
        }
    }

    fn has_refs(&self) -> bool {
        match self {
            CellFill::Fill(fill) => fill.has_refs(),
            CellFill::Angled(angled) => angled.has_refs(),
        }
    }
}

impl ThemeRefs for Bullet {
    fn resolve_refs(&mut self, theme: &ThemeContext) -> Result<(), PassError> {
        match self {
            Bullet::None {} => Ok(()),
            Bullet::Char { font, color, .. } | Bullet::Auto { font, color, .. } => {
                font.resolve_refs(theme)?;
                color.resolve_refs(theme)
            }
            Bullet::Label { color, .. } => color.resolve_refs(theme),
        }
    }

    fn has_refs(&self) -> bool {
        match self {
            Bullet::None {} => false,
            Bullet::Char { font, color, .. } | Bullet::Auto { font, color, .. } => {
                font.has_refs() || color.has_refs()
            }
            Bullet::Label { color, .. } => color.has_refs(),
        }
    }
}

impl<R: ThemeRefs> ThemeRefs for Inline<R> {
    fn resolve_refs(&mut self, theme: &ThemeContext) -> Result<(), PassError> {
        match self {
            Inline::Run { props, .. } | Inline::Field { props, .. } => props.resolve_refs(theme),
            Inline::Break { props } => props.resolve_refs(theme),
            Inline::Tab {} => Ok(()),
        }
    }

    fn has_refs(&self) -> bool {
        match self {
            Inline::Run { props, .. } | Inline::Field { props, .. } => props.has_refs(),
            Inline::Break { props } => props.has_refs(),
            Inline::Tab {} => false,
        }
    }
}

impl<P: ThemeRefs, R: ThemeRefs> ThemeRefs for Paragraph<P, R> {
    fn resolve_refs(&mut self, theme: &ThemeContext) -> Result<(), PassError> {
        self.props.resolve_refs(theme)?;
        self.inlines.resolve_refs(theme)?;
        self.end_props.resolve_refs(theme)
    }

    fn has_refs(&self) -> bool {
        self.props.has_refs() || self.inlines.has_refs() || self.end_props.has_refs()
    }
}

impl ThemeRefs for Element {
    fn resolve_refs(&mut self, theme: &ThemeContext) -> Result<(), PassError> {
        match self {
            Element::Shape(e) => e.resolve_refs(theme),
            Element::Picture(e) => e.resolve_refs(theme),
            Element::Media(e) => e.resolve_refs(theme),
            Element::Table(e) => e.resolve_refs(theme),
            Element::Group(e) => e.resolve_refs(theme),
            Element::Foreign(e) => e.resolve_refs(theme),
        }
    }

    fn has_refs(&self) -> bool {
        match self {
            Element::Shape(e) => e.has_refs(),
            Element::Picture(e) => e.has_refs(),
            Element::Media(e) => e.has_refs(),
            Element::Table(e) => e.has_refs(),
            Element::Group(e) => e.has_refs(),
            Element::Foreign(e) => e.has_refs(),
        }
    }
}

impl ThemeRefs for OwnedContainer {
    fn resolve_refs(&mut self, theme: &ThemeContext) -> Result<(), PassError> {
        match self {
            OwnedContainer::Master(c) => c.resolve_refs(theme),
            OwnedContainer::Layout(c) => c.resolve_refs(theme),
            OwnedContainer::Slide(c) => c.resolve_refs(theme),
        }
    }

    fn has_refs(&self) -> bool {
        match self {
            OwnedContainer::Master(c) => c.has_refs(),
            OwnedContainer::Layout(c) => c.has_refs(),
            OwnedContainer::Slide(c) => c.has_refs(),
        }
    }
}

/// `value` with its theme references resolved.
pub(crate) fn resolved<T: ThemeRefs>(mut value: T, theme: &ThemeContext) -> Result<T, PassError> {
    value.resolve_refs(theme)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use slidock_ir::color::{ColorMod, ColorModOp, ThemeColor, ThemeTag};
    use slidock_ir::text::ThemeFont;
    use slidock_ir::units::Ratio;

    fn hex(color: &RgbColor) -> String {
        [color.r, color.g, color.b]
            .iter()
            .map(|c| format!("{:02x}", slidock_canon::js_round(c.to_f64() * 255.0) as u32))
            .collect()
    }

    fn theme_color(slot: MappedSlot, mods: Vec<ColorMod>) -> Color {
        Color::Theme(ThemeColor {
            kind: ThemeTag,
            slot,
            mods,
            resolved: None,
        })
    }

    #[test]
    fn resolves_against_office_defaults_without_a_theme() {
        let theme = theme_context_of(None, None);
        assert_eq!(theme.missing, ["theme"]);
        assert_eq!(
            (theme.major.as_str(), theme.minor.as_str()),
            (FALLBACK_FONT, FALLBACK_FONT)
        );
        let color = resolved(theme_color(MappedSlot::Accent4, vec![]), &theme).unwrap();
        assert_eq!(hex(color.rgb().unwrap()), "ffc000");
        // tx2 shows dk2 without a colour map.
        let color = resolved(theme_color(MappedSlot::Tx2, vec![]), &theme).unwrap();
        assert_eq!(hex(color.rgb().unwrap()), "44546a");
    }

    #[test]
    fn applies_the_modifiers_and_recomputes_a_stale_value() {
        let theme = theme_context_of(None, None);
        let mods = vec![ColorMod {
            op: ColorModOp::Alpha,
            value: Ratio::exact(0.5).unwrap(),
        }];
        let mut color = theme_color(MappedSlot::Dk1, mods);
        if let Color::Theme(t) = &mut color {
            t.resolved = Some(srgb(0xff0000));
        }
        let color = resolved(color, &theme).unwrap();
        let rgb = color.rgb().unwrap();
        assert_eq!((hex(rgb).as_str(), rgb.a.to_f64()), ("000000", 0.5));
    }

    #[test]
    fn turns_theme_fonts_into_their_families() {
        let mut theme = theme_context_of(None, None);
        theme.major = "Liberation Serif".to_owned();
        let font = FontRef::Theme(ThemeFont {
            kind: ThemeTag,
            slot: ThemeFontSlot::Major,
        });
        assert!(font.has_refs());
        let font = resolved(font, &theme).unwrap();
        let FontRef::Family(family) = &font else {
            panic!("{font:?}")
        };
        assert_eq!(family.family, "Liberation Serif");
        assert_eq!(family.theme, Some(ThemeFontSlot::Major));
        assert!(!font.has_refs());
    }

    #[test]
    fn leaves_speaker_notes_as_written() {
        let theme = theme_context_of(None, None);
        let run = PartialRunProps {
            color: Some(theme_color(MappedSlot::Tx1, vec![])),
            ..Default::default()
        };
        let notes = TextBody {
            body: Default::default(),
            list_style: None,
            paragraphs: vec![Paragraph {
                level: 0,
                props: PartialParaProps::default(),
                inlines: vec![Inline::Run {
                    text: "Note".to_owned(),
                    props: run,
                    line_height: None,
                    leading_skew: None,
                }],
                end_props: None,
            }],
            flips_with_frame: false,
            rotation: None,
        };
        let mut slide: Slide = serde_json::from_value(serde_json::json!({
            "id": "ir-s1", "origin": { "kind": "agent" }, "hidden": false, "showLayout": true,
            "timeline": [], "elements": []
        }))
        .unwrap();
        slide.notes = Some(notes.clone());
        assert!(!slide.has_refs());
        slide.resolve_refs(&theme).unwrap();
        assert_eq!(slide.notes, Some(notes));
    }

    #[test]
    fn an_empty_typeface_takes_the_other_slot() {
        let theme: Theme = serde_json::from_value(serde_json::json!({
            "id": "ir-theme", "origin": { "kind": "agent" },
            "colors": {
                "dk1": rgb(), "lt1": rgb(), "dk2": rgb(), "lt2": rgb(), "accent1": rgb(), "accent2": rgb(),
                "accent3": rgb(), "accent4": rgb(), "accent5": rgb(), "accent6": rgb(), "hlink": rgb(),
                "folHlink": rgb()
            },
            "fonts": { "major": { "latin": "" }, "minor": { "latin": "Carlito" } }
        }))
        .unwrap();
        let context = theme_context_of(Some(&theme), None);
        assert_eq!(context.missing, ["major font"]);
        assert_eq!(
            (context.major.as_str(), context.minor.as_str()),
            ("Carlito", "Carlito")
        );
    }

    fn rgb() -> serde_json::Value {
        serde_json::json!({ "kind": "rgb", "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 1 })
    }
}
