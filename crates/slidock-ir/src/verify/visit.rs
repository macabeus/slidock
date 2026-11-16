//! A typed walk over a container's objects. It visits objects depth-first, each before its fields, and fields in the
//! declaration order of the zod shapes (which is the key order of a parsed presentation), skipping `ext` bags and
//! Foreign payloads. That order decides the order of the diagnostics the graph rules and `theme.resolved` report.

use crate::color::{Color, ThemeColor};
use crate::de::Nullable;
use crate::ids::{NodeId, Sha256};
use crate::paint::{CellFill, Fill, GradientStop, StrokePaint};
use crate::path::{IrPath, PathSeg};
use crate::shadow::SourceShadow;
use crate::source::{Container, Element};
use crate::table::{Border, CellBorders};
use crate::text::{
    Bullet, FontRef, Inline, LevelStyles, Link, OrText, PartialParaProps, PartialRunProps, TextBody,
};

/// What the walk reports. Every method has an empty default.
pub(crate) trait ObjectVisitor {
    /// An object's `media` or `poster` hash field, reported when the object is visited.
    fn media(&mut self, _path: &IrPath, _field: &'static str, _sha: &Sha256) {}
    /// An object under a `link` key.
    fn link(&mut self, _path: &IrPath, _link: &Link) {}
    /// A theme colour reference.
    fn theme_color(&mut self, _path: &IrPath, _color: &ThemeColor) {}
    /// A theme font reference.
    fn theme_font(&mut self, _path: &IrPath) {}
}

pub(crate) struct Walk<'v, V: ObjectVisitor> {
    visitor: &'v mut V,
    path: IrPath,
    notes: bool,
}

impl<'v, V: ObjectVisitor> Walk<'v, V> {
    /// Walks `container`, whose path is `path`; `notes` says whether a slide's notes are walked.
    pub(crate) fn container(
        visitor: &'v mut V,
        container: Container<'_>,
        path: &IrPath,
        notes: bool,
    ) {
        let mut walk = Walk {
            visitor,
            path: path.clone(),
            notes,
        };
        walk.walk_container(container);
    }

    fn at(&mut self, seg: impl Into<PathSeg>, f: impl FnOnce(&mut Self)) {
        self.path.push(seg);
        f(self);
        self.path.pop();
    }

    fn walk_container(&mut self, container: Container<'_>) {
        match container {
            Container::Master(master) => {
                if let Some(styles) = &master.text_styles {
                    self.at("textStyles", |w| {
                        for (key, levels) in [
                            ("title", &styles.title),
                            ("body", &styles.body),
                            ("other", &styles.other),
                        ] {
                            if let Some(levels) = levels {
                                w.at(key, |w| w.level_styles(levels));
                            }
                        }
                    });
                }
                self.background(master.background.as_ref());
                self.elements(&master.elements);
            }
            Container::Layout(layout) => {
                self.background(layout.background.as_ref());
                self.elements(&layout.elements);
            }
            Container::Slide(slide) => {
                if self.notes
                    && let Some(notes) = &slide.notes
                {
                    self.at("notes", |w| w.text_body(notes));
                }
                self.background(slide.background.as_ref());
                self.elements(&slide.elements);
            }
        }
    }

    fn background(&mut self, fill: Option<&Fill>) {
        if let Some(fill) = fill {
            self.at("background", |w| w.fill(fill));
        }
    }

    fn elements(&mut self, elements: &[Element]) {
        self.at("elements", |w| {
            for (i, element) in elements.iter().enumerate() {
                w.at(i, |w| w.element(element));
            }
        });
    }

    fn element(&mut self, element: &Element) {
        match element {
            Element::Picture(picture) => {
                self.visitor.media(&self.path, "media", &picture.media);
                if let Some(sha) = &picture.fallback {
                    self.visitor.media(&self.path, "fallback", sha);
                }
            }
            Element::Media(media) => {
                if let Some(sha) = &media.media {
                    self.visitor.media(&self.path, "media", sha);
                }
                if let Some(sha) = &media.poster {
                    self.visitor.media(&self.path, "poster", sha);
                }
            }
            _ => {}
        }
        if let Some(link) = element.link() {
            self.at("link", |w| w.visitor.link(&w.path, link));
        }
        match element {
            Element::Shape(shape) => {
                if let Some(fill) = &shape.fill {
                    self.at("fill", |w| w.fill(fill));
                }
                if let Some(Some(stroke)) = &shape.stroke {
                    self.at("stroke", |w| w.stroke_paint(&stroke.fill));
                }
                if let Some(text) = &shape.text {
                    self.at("text", |w| w.text_body(text));
                }
                self.shadow(&shape.shadow);
            }
            Element::Picture(picture) => {
                if let Some(Some(stroke)) = &picture.stroke {
                    self.at("stroke", |w| w.stroke_paint(&stroke.fill));
                }
                self.shadow(&picture.shadow);
            }
            Element::Table(table) => self.at("rows", |w| {
                for (r, row) in table.rows.iter().enumerate() {
                    w.at(r, |w| {
                        w.at("cells", |w| {
                            for (c, cell) in row.cells.iter().enumerate() {
                                let Some(cell) = cell else { continue };
                                w.at(c, |w| {
                                    if let Some(fill) = &cell.fill {
                                        w.at("fill", |w| w.cell_fill(fill));
                                    }
                                    if let Some(borders) = &cell.borders {
                                        w.at("borders", |w| w.borders(borders));
                                    }
                                    w.at("text", |w| w.text_body(&cell.text));
                                });
                            }
                        });
                    });
                }
            }),
            Element::Group(group) => {
                self.at("children", |w| {
                    for (i, child) in group.children.iter().enumerate() {
                        w.at(i, |w| w.element(child));
                    }
                });
                self.shadow(&group.shadow);
            }
            Element::Foreign(foreign) => {
                if let Some(fallback) = &foreign.fallback {
                    self.at("fallback", |w| {
                        if let Some(sha) = &fallback.media {
                            w.visitor.media(&w.path, "media", sha);
                        }
                        if let Some(children) = &fallback.elements {
                            w.at("elements", |w| {
                                for (i, child) in children.iter().enumerate() {
                                    w.at(i, |w| w.element(child));
                                }
                            });
                        }
                    });
                }
            }
            Element::Media(media) => self.shadow(&media.shadow),
        }
    }

    fn shadow(&mut self, shadow: &Nullable<SourceShadow>) {
        if let Some(Some(shadow)) = shadow {
            self.at("shadow", |w| w.at("color", |w| w.color(&shadow.color)));
        }
    }

    fn color(&mut self, color: &Color) {
        if let Color::Theme(theme) = color {
            self.visitor.theme_color(&self.path, theme);
        }
    }

    fn font(&mut self, font: &FontRef) {
        if let FontRef::Theme(_) = font {
            self.visitor.theme_font(&self.path);
        }
    }

    fn stops(&mut self, stops: &[GradientStop<Color>]) {
        self.at("stops", |w| {
            for (i, stop) in stops.iter().enumerate() {
                w.at(i, |w| w.at("color", |w| w.color(&stop.color)));
            }
        });
    }

    fn fill(&mut self, fill: &Fill) {
        match fill {
            Fill::None => {}
            Fill::Solid { color } => self.at("color", |w| w.color(color)),
            Fill::Image { media, .. } => self.visitor.media(&self.path, "media", media),
            Fill::LinearAngle(g) => self.stops(&g.stops),
            Fill::RadialSource(g) => self.stops(&g.stops),
            Fill::Path(g) => self.stops(&g.stops),
            Fill::Odf(g) => self.stops(&g.stops),
            Fill::Linear(g) => self.stops(&g.stops),
            Fill::Radial(g) => self.stops(&g.stops),
            Fill::Rect(g) => self.stops(&g.stops),
            Fill::Ellipse(g) => self.stops(&g.stops),
        }
    }

    fn cell_fill(&mut self, fill: &CellFill) {
        match fill {
            CellFill::Fill(fill) => self.fill(fill),
            CellFill::Angled(g) => self.stops(&g.stops),
        }
    }

    fn stroke_paint(&mut self, paint: &StrokePaint<Color>) {
        if let StrokePaint::Solid { color } = paint {
            self.at("fill", |w| w.at("color", |w| w.color(color)));
        }
    }

    fn borders(&mut self, borders: &CellBorders<Color>) {
        for (key, border) in borders.edges() {
            if let Some(Border { fill, .. }) = border {
                self.at(key, |w| w.stroke_paint(fill));
            }
        }
    }

    fn level_styles(&mut self, levels: &LevelStyles) {
        for (key, style) in levels.levels() {
            let Some(style) = style else { continue };
            self.at(key, |w| {
                if let Some(para) = &style.para {
                    w.at("para", |w| w.para_props(para));
                }
                if let Some(run) = &style.run {
                    w.at("run", |w| w.run_props(run));
                }
            });
        }
    }

    fn para_props(&mut self, props: &PartialParaProps) {
        if let Some(bullet) = &props.bullet {
            self.at("bullet", |w| w.bullet(bullet));
        }
    }

    fn bullet(&mut self, bullet: &Bullet) {
        let (font, color) = match bullet {
            Bullet::Char { font, color, .. } | Bullet::Auto { font, color, .. } => (font, color),
            Bullet::Label { color, .. } => {
                self.at("color", |w| w.color(color));
                return;
            }
            Bullet::None {} => return,
        };
        if let Some(OrText::Value(font)) = font {
            self.at("font", |w| w.font(font));
        }
        if let Some(OrText::Value(color)) = color {
            self.at("color", |w| w.color(color));
        }
    }

    fn run_props(&mut self, props: &PartialRunProps) {
        if let Some(font) = &props.font {
            self.at("font", |w| w.font(font));
        }
        if let Some(color) = &props.color {
            self.at("color", |w| w.color(color));
        }
        if let Some(Some(highlight)) = &props.highlight {
            self.at("highlight", |w| w.color(highlight));
        }
        if let Some(Some(link)) = &props.link {
            self.at("link", |w| w.visitor.link(&w.path, link));
        }
        self.shadow(&props.shadow);
    }

    fn text_body(&mut self, text: &TextBody) {
        if let Some(levels) = &text.list_style {
            self.at("listStyle", |w| w.level_styles(levels));
        }
        self.at("paragraphs", |w| {
            for (p, paragraph) in text.paragraphs.iter().enumerate() {
                w.at(p, |w| {
                    w.at("props", |w| w.para_props(&paragraph.props));
                    w.at("inlines", |w| {
                        for (i, inline) in paragraph.inlines.iter().enumerate() {
                            let props = match inline {
                                Inline::Run { props, .. } | Inline::Field { props, .. } => {
                                    Some(props)
                                }
                                Inline::Break { props } => props.as_ref(),
                                Inline::Tab {} => None,
                            };
                            if let Some(props) = props {
                                w.at(i, |w| w.at("props", |w| w.run_props(props)));
                            }
                        }
                    });
                    if let Some(end) = &paragraph.end_props {
                        w.at("endProps", |w| w.run_props(end));
                    }
                });
            }
        });
    }
}

/// A slide link's target, if `link` is one.
pub(crate) fn slide_target(link: &Link) -> Option<&NodeId> {
    match link {
        Link::Slide { slide } => Some(slide),
        _ => None,
    }
}
