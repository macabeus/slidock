//! The ODF style model, resolved inside the frontend: ODF style inheritance is internal to the format, so the
//! presentation receives explicit values. A style is looked up by family and name in its part's automatic styles, then
//! in the common styles of styles.xml; its properties are its family's default style, then each parent from the root
//! down (depth and cycles checked), then its own. Relative font sizes resolve against the size inherited so far.
//! Graphic properties read alike whichever namespace writes them (`loext:graphic-properties` in the families ODF gives
//! none), and a cell's standard properties (`style:table-cell-properties`) read in the graphic terms LibreOffice writes
//! for cells: background as area fill, padding, vertical alignment; its borders, wherever written, are the cell's.
//! Named resources (markers, dashes, gradients, hatches, fill images, list styles, table templates, font faces, page
//! layouts) are looked up here too.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use slidock_canon::{js_number, to_fixed};

use crate::names::{QName, qn};
use crate::odf::{Element, Ns, decode_style_name};
use crate::units::{percent, raw_length};

/// Attribute values by canonical qualified name, as written.
pub(crate) type PropertySet = BTreeMap<String, String>;

/// A property set's value.
pub(crate) fn get(set: &PropertySet, key: QName) -> Option<&str> {
    set.get(key.as_str()).map(String::as_str)
}

/// What a chain of styles amounts to, still in ODF's own terms.
#[derive(Clone, Debug, Default)]
pub(crate) struct Props<'a> {
    pub graphic: Rc<PropertySet>,
    pub paragraph: Rc<PropertySet>,
    pub text: Rc<PropertySet>,
    pub drawing_page: Rc<PropertySet>,
    /// A cell's borders and other table-cell properties outside the graphic terms.
    pub cell: Rc<PropertySet>,
    /// `style:table-column-properties`.
    pub column: Rc<PropertySet>,
    /// `style:table-row-properties`.
    pub row: Rc<PropertySet>,
    /// The list style a graphic or presentation style embeds, or names (`style:list-style-name`).
    pub list_style: Option<&'a Element>,
    /// The last `style:tab-stops` declared.
    pub tab_stops: Option<&'a Element>,
    /// The column count of a text area (`style:columns`).
    pub columns: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Family {
    Graphic,
    Presentation,
    Paragraph,
    Text,
    DrawingPage,
    TableCell,
    TableColumn,
    TableRow,
}

impl Family {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Family::Graphic => "graphic",
            Family::Presentation => "presentation",
            Family::Paragraph => "paragraph",
            Family::Text => "text",
            Family::DrawingPage => "drawing-page",
            Family::TableCell => "table-cell",
            Family::TableColumn => "table-column",
            Family::TableRow => "table-row",
        }
    }
}

/// Style chains deeper than this are cut, so a cyclic or hostile chain ends.
const MAX_CHAIN: usize = 32;

/// A style element; `common` when it is one of styles.xml's common styles rather than a part's automatic ones.
#[derive(Clone, Copy, Debug)]
struct StyleDef<'a> {
    name: &'a str,
    parent: Option<&'a str>,
    element: &'a Element,
    common: bool,
}

#[derive(Debug, Default)]
struct Index<'a> {
    styles: BTreeMap<(&'a str, &'a str), StyleDef<'a>>,
    list_styles: BTreeMap<&'a str, &'a Element>,
}

fn index_styles<'a>(container: Option<&'a Element>, into: &mut Index<'a>, common: bool) {
    for element in container.into_iter().flat_map(Element::kids) {
        if element.is(Ns::Style, "style") {
            let (Some(family), Some(name)) = (
                element.attr(qn!("style:family")).filter(|s| !s.is_empty()),
                element.attr(qn!("style:name")).filter(|s| !s.is_empty()),
            ) else {
                continue;
            };
            into.styles.entry((family, name)).or_insert(StyleDef {
                name,
                parent: element
                    .attr(qn!("style:parent-style-name"))
                    .filter(|s| !s.is_empty()),
                element,
                common,
            });
        } else if element.is(Ns::Text, "list-style")
            && let Some(name) = element.attr(qn!("style:name")).filter(|s| !s.is_empty())
        {
            into.list_styles.entry(name).or_insert(element);
        }
    }
}

/// The elements `local` of `ns` in a container, by the first of the name attributes each states.
fn named<'a>(
    container: Option<&'a Element>,
    ns: Ns,
    local: &str,
    name_attrs: &[QName],
) -> BTreeMap<&'a str, &'a Element> {
    let mut out = BTreeMap::new();
    for element in container.into_iter().flat_map(|c| c.children_of(ns, local)) {
        let name = name_attrs
            .iter()
            .find_map(|name| element.attr(*name).filter(|s| !s.is_empty()));
        if let Some(name) = name {
            // A later element of the same name replaces an earlier one, as a Map built from entries does.
            out.insert(name, element);
        }
    }
    out
}

/// What both parts share: the common styles, default styles, named resources and font faces.
#[derive(Debug, Default)]
pub(crate) struct CommonStyles<'a> {
    index: Index<'a>,
    defaults: BTreeMap<&'a str, &'a Element>,
    pub markers: BTreeMap<&'a str, &'a Element>,
    pub dashes: BTreeMap<&'a str, &'a Element>,
    pub gradients: BTreeMap<&'a str, &'a Element>,
    /// Transparency gradients (`draw:opacity`).
    pub opacities: BTreeMap<&'a str, &'a Element>,
    /// Whether a unitless gradient `draw:angle` is in tenths of a degree (gradient.rs).
    pub unitless_angle_tenths: bool,
    pub hatches: BTreeMap<&'a str, &'a Element>,
    pub fill_images: BTreeMap<&'a str, &'a Element>,
    pub page_layouts: BTreeMap<&'a str, &'a Element>,
    /// Table templates (`table:table-template`), named as ODF 1.3 or as ODF 1.2 does.
    pub table_templates: BTreeMap<&'a str, &'a Element>,
    font_faces: BTreeMap<&'a str, String>,
}

impl<'a> CommonStyles<'a> {
    pub(crate) fn new(styles_root: Option<&'a Element>, content_root: Option<&'a Element>) -> Self {
        let common = styles_root.and_then(|root| root.child(Ns::Office, "styles"));
        let mut index = Index::default();
        index_styles(common, &mut index, true);
        let mut defaults = BTreeMap::new();
        for element in common
            .into_iter()
            .flat_map(|c| c.children_of(Ns::Style, "default-style"))
        {
            if let Some(family) = element.attr(qn!("style:family")).filter(|s| !s.is_empty()) {
                defaults.entry(family).or_insert(element);
            }
        }
        let mut font_faces = BTreeMap::new();
        for root in [content_root, styles_root].into_iter().flatten() {
            let decls = root.child(Ns::Office, "font-face-decls");
            for face in decls
                .into_iter()
                .flat_map(|d| d.children_of(Ns::Style, "font-face"))
            {
                let Some(name) = face.attr(qn!("style:name")).filter(|s| !s.is_empty()) else {
                    continue;
                };
                let family = Some(unquote(face.attr(qn!("svg:font-family")).unwrap_or("")))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| name.to_owned());
                font_faces.entry(name).or_insert(family);
            }
        }
        let automatic = styles_root.and_then(|root| root.child(Ns::Office, "automatic-styles"));
        CommonStyles {
            index,
            defaults,
            markers: named(common, Ns::Draw, "marker", &[qn!("draw:name")]),
            dashes: named(common, Ns::Draw, "stroke-dash", &[qn!("draw:name")]),
            gradients: named(common, Ns::Draw, "gradient", &[qn!("draw:name")]),
            opacities: named(common, Ns::Draw, "opacity", &[qn!("draw:name")]),
            unitless_angle_tenths: false,
            hatches: named(common, Ns::Draw, "hatch", &[qn!("draw:name")]),
            fill_images: named(common, Ns::Draw, "fill-image", &[qn!("draw:name")]),
            table_templates: named(
                common,
                Ns::Table,
                "table-template",
                &[qn!("table:name"), qn!("text:style-name")],
            ),
            page_layouts: named(automatic, Ns::Style, "page-layout", &[qn!("style:name")]),
            font_faces,
        }
    }

    /// The family a `style:font-name` names, through its font-face declaration.
    pub(crate) fn font_family(&self, name: &str) -> String {
        self.font_faces
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_owned())
    }
}

/// `'Liberation Sans'`, `"Barlow Condensed", serif` → the first family, unquoted.
pub(crate) fn unquote(families: &str) -> String {
    let rest = slidock_canon::js_trim_start(families);
    let first = if let Some(quoted) = rest.strip_prefix('\'') {
        quoted.split('\'').next().filter(|_| quoted.contains('\''))
    } else if let Some(quoted) = rest.strip_prefix('"') {
        quoted.split('"').next().filter(|_| quoted.contains('"'))
    } else {
        None
    };
    let first = first.unwrap_or_else(|| {
        // An unterminated quote is not a quoted family: the text up to the first comma.
        families.split(',').next().unwrap_or_default()
    });
    slidock_canon::js_trim(first).to_owned()
}

/// Why a style chain was cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ChainCut {
    Cycle,
    Depth,
}

impl ChainCut {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            ChainCut::Cycle => "cycle",
            ChainCut::Depth => "depth",
        }
    }
}

/// A style chain cut: the style as displayed, why, and whether the style is a common one of styles.xml.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CutChain {
    pub style: String,
    pub reason: ChainCut,
    pub common: bool,
}

/// The property names a style's chain declares, by kind: what a paragraph or span sets on its own text.
#[derive(Clone, Debug, Default)]
pub(crate) struct Declared {
    pub paragraph: BTreeSet<String>,
    pub text: BTreeSet<String>,
}

/// The styles one part sees: its automatic styles, then the common ones. Resolved chains are cached; each chain
/// problem is recorded once.
#[derive(Debug)]
pub(crate) struct StyleScope<'a> {
    pub common: &'a CommonStyles<'a>,
    automatic: Index<'a>,
    root: Props<'a>,
    resolved: RefCell<BTreeMap<(Family, String), Rc<Props<'a>>>>,
    reported: RefCell<BTreeSet<String>>,
    cuts: RefCell<Vec<CutChain>>,
}

impl<'a> StyleScope<'a> {
    /// `part` is the part whose automatic styles this scope sees; `root` the layer under every chain (the
    /// frontend's defaults, in ODF's terms).
    pub(crate) fn new(
        common: &'a CommonStyles<'a>,
        part: Option<&'a Element>,
        root: Props<'a>,
    ) -> Self {
        let mut automatic = Index::default();
        index_styles(
            part.and_then(|part| part.child(Ns::Office, "automatic-styles")),
            &mut automatic,
            false,
        );
        StyleScope {
            common,
            automatic,
            root,
            resolved: RefCell::new(BTreeMap::new()),
            reported: RefCell::new(BTreeSet::new()),
            cuts: RefCell::new(Vec::new()),
        }
    }

    /// The chain cuts found since the last call.
    pub(crate) fn take_cuts(&self) -> Vec<CutChain> {
        std::mem::take(&mut *self.cuts.borrow_mut())
    }

    fn find(&self, family: &str, name: &str) -> Option<StyleDef<'a>> {
        self.automatic
            .styles
            .get(&(family, name))
            .or_else(|| self.common.index.styles.get(&(family, name)))
            .copied()
    }

    pub(crate) fn list_style(&self, name: Option<&str>) -> Option<&'a Element> {
        let name = name.filter(|s| !s.is_empty())?;
        self.automatic
            .list_styles
            .get(name)
            .or_else(|| self.common.index.list_styles.get(name))
            .copied()
    }

    pub(crate) fn has(&self, family: Family, name: &str) -> bool {
        self.find(family.as_str(), name).is_some()
    }

    /// The chain from the root ancestor down to the style, cut where it loops or runs too deep.
    fn chain(&self, family: Family, name: &str) -> Vec<StyleDef<'a>> {
        let mut out: Vec<StyleDef<'a>> = Vec::new();
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut at = self.find(family.as_str(), name);
        while let Some(def) = at {
            if seen.contains(def.name) || out.len() >= MAX_CHAIN {
                let reason = if seen.contains(def.name) {
                    ChainCut::Cycle
                } else {
                    ChainCut::Depth
                };
                if self.reported.borrow_mut().insert(name.to_owned()) {
                    self.cuts.borrow_mut().push(CutChain {
                        style: decode_style_name(name),
                        reason,
                        common: def.common,
                    });
                }
                break;
            }
            seen.insert(def.name);
            out.insert(0, def);
            at = def
                .parent
                .and_then(|parent| self.find(family.as_str(), parent));
        }
        out
    }

    fn chain_of(&self, family: Family, name: Option<&str>) -> Vec<StyleDef<'a>> {
        match name.filter(|s| !s.is_empty()) {
            Some(name) => self.chain(family, name),
            None => Vec::new(),
        }
    }

    /// The default style of a family; presentation styles stand on the graphic defaults.
    fn default_style(&self, family: Family) -> Option<&'a Element> {
        self.common
            .defaults
            .get(family.as_str())
            .or_else(|| {
                (family == Family::Presentation)
                    .then(|| self.common.defaults.get(Family::Graphic.as_str()))
                    .flatten()
            })
            .copied()
    }

    /// The frontend's defaults, the family's default style, the style's ancestors from the root down, the style.
    /// A graphic style chain that starts at an automatic style, at none, or at "standard" stands on "standard": as
    /// LibreOffice gives a drawable whose automatic style has no parent (or that names none) its default style
    /// sheet, the file's "standard", or LibreOffice's own when the file has none. A chain that starts at another
    /// common style stands on the defaults alone.
    pub(crate) fn resolve(&self, family: Family, name: Option<&str>) -> Rc<Props<'a>> {
        let cache_key = (family, name.unwrap_or("").to_owned());
        if let Some(cached) = self.resolved.borrow().get(&cache_key) {
            return Rc::clone(cached);
        }
        let mut props = apply_style(&self.root, self.default_style(family), self);
        let chain = self.chain_of(family, name);
        let first = chain.first();
        if family == Family::Graphic && first.is_none_or(|def| !def.common || def.name == STANDARD)
        {
            let standard = self.chain(Family::Graphic, STANDARD);
            if standard.is_empty() {
                props = with_graphic(&props, BUILT_IN_STANDARD);
            } else if first.is_none_or(|def| def.name != STANDARD) {
                for def in standard {
                    props = apply_style(&props, Some(def.element), self);
                }
            }
        }
        for def in chain {
            props = apply_style(&props, Some(def.element), self);
        }
        let props = Rc::new(props);
        self.resolved
            .borrow_mut()
            .insert(cache_key, Rc::clone(&props));
        props
    }

    /// A style's chain alone over `base`: a paragraph or text style over a shape's text.
    pub(crate) fn over(&self, family: Family, name: Option<&str>, base: &Props<'a>) -> Props<'a> {
        let mut props = base.clone();
        for def in self.chain_of(family, name) {
            props = apply_style(&props, Some(def.element), self);
        }
        props
    }

    /// The property names a style's chain declares, by kind.
    pub(crate) fn declared(&self, family: Family, name: Option<&str>) -> Declared {
        let mut out = Declared::default();
        for def in self.chain_of(family, name) {
            if let Some(paragraph) = def.element.child(Ns::Style, "paragraph-properties") {
                out.paragraph.extend(paragraph.attrs.keys().cloned());
            }
            if let Some(text) = def.element.child(Ns::Style, "text-properties") {
                out.text.extend(text.attrs.keys().cloned());
            }
        }
        out
    }

    /// A style's own properties over `base`, skipping its family default and ancestors.
    pub(crate) fn own(&self, family: Family, name: Option<&str>, base: &Props<'a>) -> Props<'a> {
        let def = name
            .filter(|s| !s.is_empty())
            .and_then(|name| self.find(family.as_str(), name));
        match def {
            Some(def) => apply_style(base, Some(def.element), self),
            None => base.clone(),
        }
    }
}

/// The graphic style LibreOffice gives a drawable without a style sheet of its own.
const STANDARD: &str = "standard";

/// What LibreOffice's built-in "standard" sets, of what the frontend reads (`SdDrawDocument::CreateLayoutTemplates`):
/// a grey shadow 0.2 cm right and down. It stands only where the file has no "standard"; a file's own "standard"
/// leaves what it does not state at the pool defaults (measured, LibreOffice 25.2).
const BUILT_IN_STANDARD: &[(QName, &str)] = &[
    (qn!("draw:shadow-color"), "#808080"),
    (qn!("draw:shadow-offset-x"), "0.2cm"),
    (qn!("draw:shadow-offset-y"), "0.2cm"),
];

/// Graphic properties over a style layer.
fn with_graphic<'a>(base: &Props<'a>, pairs: &[(QName, &str)]) -> Props<'a> {
    let mut graphic = (*base.graphic).clone();
    graphic.extend(
        pairs
            .iter()
            .map(|(k, v)| (k.as_str().to_owned(), (*v).to_owned())),
    );
    Props {
        graphic: Rc::new(graphic),
        ..base.clone()
    }
}

const TEXT_SIZE: QName = qn!("fo:font-size");

/// `{ ...base, ...over }`.
fn spread(base: &PropertySet, over: &PropertySet) -> PropertySet {
    let mut out = base.clone();
    out.extend(over.iter().map(|(k, v)| (k.clone(), v.clone())));
    out
}

/// A text-properties set over another. A relative size resolves against the size inherited; a family or colour
/// replaces the one inherited however it is written (a font face or a family name; a colour or the automatic one).
pub(crate) fn merge_text(base: &PropertySet, over: &PropertySet) -> PropertySet {
    let mut out = spread(base, over);
    let has = |key: QName| over.contains_key(key.as_str());
    if has(qn!("style:font-name")) && !has(qn!("fo:font-family")) {
        out.remove(qn!("fo:font-family").as_str());
    }
    if has(qn!("fo:font-family")) && !has(qn!("style:font-name")) {
        out.remove(qn!("style:font-name").as_str());
    }
    if has(qn!("fo:color")) && !has(qn!("style:use-window-font-color")) {
        out.remove(qn!("style:use-window-font-color").as_str());
    }
    if get(over, qn!("style:use-window-font-color")) == Some("true") && !has(qn!("fo:color")) {
        out.remove(qn!("fo:color").as_str());
    }
    if let Some(relative) = percent(get(over, TEXT_SIZE)) {
        match raw_length(get(base, TEXT_SIZE)) {
            Some(inherited) => {
                let fixed = to_fixed(inherited * relative, 4).unwrap_or_default();
                out.insert(TEXT_SIZE.as_str().to_owned(), format!("{fixed}pt"));
            }
            None => {
                out.remove(TEXT_SIZE.as_str());
            }
        }
    }
    out
}

const SIDES: [&str; 4] = ["top", "bottom", "left", "right"];

/// A property set over another, where a shorthand (`fo:padding`) replaces the sides it stands for.
fn merge_sides(base: &PropertySet, over: &PropertySet, shorthand: QName) -> PropertySet {
    let mut out = spread(base, over);
    if over.contains_key(shorthand.as_str()) {
        for side in SIDES {
            let key = format!("{}-{side}", shorthand.as_str());
            if !over.contains_key(&key) {
                out.remove(&key);
            }
        }
    }
    out
}

/// The table-cell properties that have graphic terms: background, padding and vertical alignment.
fn in_graphic(key: &str) -> bool {
    key == qn!("fo:background-color").as_str()
        || key == qn!("style:vertical-align").as_str()
        || key.starts_with(qn!("fo:padding").as_str())
}

/// `style:table-cell-properties` in graphic terms: its background as area fill, its padding, its vertical alignment.
fn cell_graphic(cell: &PropertySet) -> PropertySet {
    let mut out = PropertySet::new();
    match get(cell, qn!("fo:background-color")) {
        Some("transparent") => {
            out.insert(qn!("draw:fill").as_str().to_owned(), "none".to_owned());
        }
        Some(background) => {
            out.insert(qn!("draw:fill").as_str().to_owned(), "solid".to_owned());
            out.insert(
                qn!("draw:fill-color").as_str().to_owned(),
                background.to_owned(),
            );
        }
        None => {}
    }
    for (key, value) in cell {
        if key.starts_with(qn!("fo:padding").as_str()) {
            out.insert(key.clone(), value.clone());
        }
    }
    let vertical = match get(cell, qn!("style:vertical-align")) {
        Some("top" | "automatic") => Some("top"),
        Some("middle") => Some("middle"),
        Some("bottom") => Some("bottom"),
        _ => None,
    };
    if let Some(vertical) = vertical {
        out.insert(
            qn!("draw:textarea-vertical-align").as_str().to_owned(),
            vertical.to_owned(),
        );
    }
    out
}

fn is_border(key: &str) -> bool {
    key.starts_with(qn!("fo:border").as_str()) || key.starts_with("style:diagonal-")
}

/// A cell's own terms: the borders its paragraph properties write, then its other table-cell properties.
fn cell_own(cell: &PropertySet, paragraph: &PropertySet) -> PropertySet {
    let borders: PropertySet = paragraph
        .iter()
        .filter(|(key, _)| is_border(key))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let other: PropertySet = cell
        .iter()
        .filter(|(key, _)| !in_graphic(key))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    merge_sides(&borders, &other, qn!("fo:border"))
}

fn attrs_of(element: Option<&Element>) -> PropertySet {
    element.map(|e| e.attrs.clone()).unwrap_or_default()
}

/// One style element's properties over `base`.
pub(crate) fn apply_style<'a>(
    base: &Props<'a>,
    element: Option<&'a Element>,
    scope: &StyleScope<'a>,
) -> Props<'a> {
    let Some(element) = element else {
        return base.clone();
    };
    let graphics: Vec<&'a Element> = [
        element.child(Ns::Style, "graphic-properties"),
        element.child(Ns::Loext, "graphic-properties"),
    ]
    .into_iter()
    .flatten()
    .collect();
    let paragraph = element.child(Ns::Style, "paragraph-properties");
    let text = element.child(Ns::Style, "text-properties");
    let drawing_page = element.child(Ns::Style, "drawing-page-properties");
    let cell = attrs_of(element.child(Ns::Style, "table-cell-properties"));
    let column = element.child(Ns::Style, "table-column-properties");
    let row = element.child(Ns::Style, "table-row-properties");
    let mut graphic = cell_graphic(&cell);
    for props in &graphics {
        graphic.extend(props.attrs.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    let list_style = graphics
        .iter()
        .find_map(|props| props.child(Ns::Text, "list-style"))
        .or_else(|| scope.list_style(element.attr(qn!("style:list-style-name"))));
    let tab_stops = paragraph.and_then(|p| p.child(Ns::Style, "tab-stops"));
    let columns = graphics
        .iter()
        .find_map(|props| props.child(Ns::Style, "columns"))
        .and_then(|c| c.attr(qn!("fo:column-count")))
        .map_or(f64::NAN, js_number);
    let cell_set = cell_own(&cell, &attrs_of(paragraph));
    Props {
        graphic: if graphic.is_empty() {
            Rc::clone(&base.graphic)
        } else {
            Rc::new(merge_sides(&base.graphic, &graphic, qn!("fo:padding")))
        },
        paragraph: match paragraph {
            Some(p) => Rc::new(merge_sides(&base.paragraph, &p.attrs, qn!("fo:margin"))),
            None => Rc::clone(&base.paragraph),
        },
        text: match text {
            Some(t) => Rc::new(merge_text(&base.text, &t.attrs)),
            None => Rc::clone(&base.text),
        },
        drawing_page: match drawing_page {
            Some(d) => Rc::new(spread(&base.drawing_page, &d.attrs)),
            None => Rc::clone(&base.drawing_page),
        },
        cell: if cell_set.is_empty() {
            Rc::clone(&base.cell)
        } else {
            Rc::new(merge_sides(&base.cell, &cell_set, qn!("fo:border")))
        },
        column: match column {
            Some(c) => Rc::new(spread(&base.column, &c.attrs)),
            None => Rc::clone(&base.column),
        },
        row: match row {
            Some(r) => Rc::new(spread(&base.row, &r.attrs)),
            None => Rc::clone(&base.row),
        },
        list_style: list_style.or(base.list_style),
        tab_stops: tab_stops.or(base.tab_stops),
        columns: if columns.fract() == 0.0 && columns.is_finite() && columns > 0.0 {
            Some(columns)
        } else {
            base.columns
        },
    }
}

/// Plain properties as a style layer (the frontend's own defaults).
pub(crate) fn props_of<'a>(
    graphic: &[(QName, &str)],
    paragraph: &[(QName, &str)],
    text: &[(QName, &str)],
) -> Props<'a> {
    let set = |pairs: &[(QName, &str)]| -> Rc<PropertySet> {
        Rc::new(
            pairs
                .iter()
                .map(|(k, v)| (k.as_str().to_owned(), (*v).to_owned()))
                .collect(),
        )
    };
    Props {
        graphic: set(graphic),
        paragraph: set(paragraph),
        text: set(text),
        ..Props::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::odf::parse;

    const NS: &str = concat!(
        r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" "#,
        r#"xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" "#,
        r#"xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" "#,
        r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" "#,
        r#"xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" "#,
        r#"xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#,
    );

    fn styles(common: &str, automatic: &str) -> Element {
        parse(&format!(
            concat!(
                "<office:document-styles {}><office:font-face-decls><style:font-face style:name=\"Brand\" ",
                "svg:font-family=\"&apos;Brand Sans&apos;, sans-serif\"/></office:font-face-decls>",
                "<office:styles>{}</office:styles><office:automatic-styles>{}</office:automatic-styles>",
                "</office:document-styles>"
            ),
            NS, common, automatic
        ))
    }

    fn style(name: &str, family: &str, props: &str, parent: Option<&str>) -> String {
        let parent = parent
            .map(|p| format!(r#" style:parent-style-name="{p}""#))
            .unwrap_or_default();
        format!(
            r#"<style:style style:name="{name}" style:family="{family}"{parent}>{props}</style:style>"#
        )
    }

    fn set(pairs: &[(&str, &str)]) -> PropertySet {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    /// Runs `check` on a scope over these styles.
    fn with_scope(common: &str, automatic: &str, check: impl FnOnce(&StyleScope<'_>)) {
        let root = styles(common, automatic);
        let shared = CommonStyles::new(Some(&root), None);
        let scope = StyleScope::new(&shared, Some(&root), Props::default());
        check(&scope);
    }

    #[test]
    fn resolves_the_family_default_then_each_parent_from_the_root_down_then_the_style() {
        let common = [
            concat!(
                r#"<style:default-style style:family="graphic"><style:graphic-properties draw:fill="solid" "#,
                r##"draw:fill-color="#000000" svg:stroke-width="1pt"/><style:text-properties fo:font-size="20pt"/></style:default-style>"##
            )
            .to_owned(),
            style(
                "base",
                "graphic",
                r##"<style:graphic-properties draw:fill-color="#111111"/><style:text-properties fo:font-size="150%"/>"##,
                None,
            ),
            style("middle", "graphic", r#"<style:graphic-properties draw:stroke="dash"/>"#, Some("base")),
        ]
        .concat();
        let automatic = style(
            "gr1",
            "graphic",
            r##"<style:graphic-properties draw:fill-color="#222222"/><style:text-properties fo:font-size="50%"/>"##,
            Some("middle"),
        );
        with_scope(&common, &automatic, |scope| {
            let props = scope.resolve(Family::Graphic, Some("gr1"));
            assert_eq!(
                *props.graphic,
                set(&[
                    ("draw:fill", "solid"),
                    ("draw:fill-color", "#222222"),
                    ("svg:stroke-width", "1pt"),
                    ("draw:stroke", "dash"),
                ])
            );
            assert_eq!(get(&props.text, qn!("fo:font-size")), Some("15.0000pt"));
            let unknown = scope.resolve(Family::Graphic, Some("unknown"));
            assert_eq!(
                get(&unknown.graphic, qn!("draw:fill-color")),
                Some("#000000")
            );
        });
    }

    #[test]
    fn stands_presentation_styles_on_the_graphic_defaults_and_finds_embedded_and_named_list_styles()
    {
        let common = [
            r#"<style:default-style style:family="graphic"><style:graphic-properties draw:stroke="none"/></style:default-style>"#.to_owned(),
            style(
                "title",
                "presentation",
                r#"<style:graphic-properties><text:list-style style:name="title"/></style:graphic-properties>"#,
                None,
            ),
            r#"<text:list-style style:name="L1"><text:list-level-style-bullet text:level="1" text:bullet-char="•"/></text:list-style>"#.to_owned(),
        ]
        .concat();
        let automatic = r#"<style:style style:name="pr1" style:family="presentation" style:parent-style-name="title" style:list-style-name="L1"/>"#;
        with_scope(&common, automatic, |scope| {
            let title = scope.resolve(Family::Presentation, Some("title"));
            assert_eq!(get(&title.graphic, qn!("draw:stroke")), Some("none"));
            assert_eq!(
                title.list_style.and_then(|l| l.attr(qn!("style:name"))),
                Some("title")
            );
            let pr1 = scope.resolve(Family::Presentation, Some("pr1"));
            assert_eq!(
                pr1.list_style.and_then(|l| l.attr(qn!("style:name"))),
                Some("L1")
            );
        });
    }

    #[test]
    fn cuts_a_chain_at_a_cycle_or_past_32_parents_once_per_style() {
        let deep: String = (0..40)
            .map(|i| {
                let parent = (i > 0).then(|| format!("s{}", i - 1));
                style(&format!("s{i}"), "graphic", "", parent.as_deref())
            })
            .collect();
        let common = format!(
            "{}{}{deep}",
            style("a", "graphic", "", Some("b")),
            style("b", "graphic", "", Some("a"))
        );
        with_scope(&common, "", |scope| {
            scope.resolve(Family::Graphic, Some("a"));
            scope.resolve(Family::Graphic, Some("s39"));
            scope.resolve(Family::Graphic, Some("s39"));
            let cuts: Vec<(String, ChainCut, bool)> = scope
                .take_cuts()
                .into_iter()
                .map(|c| (c.style, c.reason, c.common))
                .collect();
            assert_eq!(
                cuts,
                [
                    ("a".to_owned(), ChainCut::Cycle, true),
                    ("s39".to_owned(), ChainCut::Depth, true)
                ]
            );
        });
    }

    #[test]
    fn keeps_a_chain_of_32_styles_whole_and_cuts_a_longer_one_at_its_root() {
        // s0 is the root and the only style that sets a fill; s(n) has n parents.
        let chain: String = (0..33)
            .map(|i| {
                let parent = (i > 0).then(|| format!("s{}", i - 1));
                let props = if i == 0 {
                    r#"<style:graphic-properties draw:fill="solid"/>"#
                } else {
                    ""
                };
                style(&format!("s{i}"), "graphic", props, parent.as_deref())
            })
            .collect();
        with_scope(&chain, "", |scope| {
            let fill = |name| {
                scope
                    .resolve(Family::Graphic, Some(name))
                    .graphic
                    .get("draw:fill")
                    .cloned()
            };
            assert_eq!(fill("s31"), Some("solid".to_owned()));
            assert!(scope.take_cuts().is_empty());
            assert_eq!(fill("s32"), None);
            let cuts: Vec<(String, ChainCut)> = scope
                .take_cuts()
                .into_iter()
                .map(|c| (c.style, c.reason))
                .collect();
            assert_eq!(cuts, [("s32".to_owned(), ChainCut::Depth)]);
        });
    }

    #[test]
    fn stands_an_automatic_unparented_or_absent_graphic_style_on_standard_as_libreoffice_does() {
        // LibreOffice 25.2 (probe, PDF): a drawable whose automatic style has no parent, or that names none, takes
        // the file's "standard", whose unstated shadow keys stay at the pool defaults (a red shadow 0.5 cm right and
        // none down); a common style without a parent stands on neither.
        let common = [
            style(
                "standard",
                "graphic",
                r##"<style:graphic-properties draw:fill="solid" draw:fill-color="#ffffff" draw:shadow-offset-x="0.5cm" draw:shadow-color="#ff0000"/>"##,
                None,
            ),
            style(
                "plain",
                "graphic",
                r#"<style:graphic-properties draw:shadow="visible"/>"#,
                None,
            ),
        ]
        .concat();
        let automatic = [
            style(
                "gr1",
                "graphic",
                r#"<style:graphic-properties draw:shadow="visible"/>"#,
                None,
            ),
            style(
                "gr2",
                "graphic",
                r#"<style:graphic-properties draw:stroke="none"/>"#,
                Some("plain"),
            ),
        ]
        .concat();
        with_scope(&common, &automatic, |scope| {
            let shadow = |name: Option<&str>| {
                let props = scope.resolve(Family::Graphic, name);
                [
                    qn!("draw:fill-color"),
                    qn!("draw:shadow-offset-x"),
                    qn!("draw:shadow-offset-y"),
                    qn!("draw:shadow-color"),
                ]
                .map(|key| get(&props.graphic, key).map(str::to_owned))
            };
            let on_standard = [
                Some("#ffffff".to_owned()),
                Some("0.5cm".to_owned()),
                None,
                Some("#ff0000".to_owned()),
            ];
            assert_eq!(shadow(Some("gr1")), on_standard);
            assert_eq!(shadow(None), on_standard);
            assert_eq!(shadow(Some("missing")), on_standard);
            assert_eq!(shadow(Some("standard")), on_standard);
            assert_eq!(shadow(Some("plain")), [None, None, None, None]);
            assert_eq!(shadow(Some("gr2")), [None, None, None, None]);
            assert_eq!(
                scope
                    .resolve(Family::Presentation, Some("plain"))
                    .graphic
                    .get("draw:shadow-offset-x"),
                None
            );
        });
    }

    #[test]
    fn stands_a_file_without_standard_on_libreoffices_own() {
        // LibreOffice 25.2 (probe, PDF): with no "standard" in the file, an unparented automatic style's visible
        // shadow is LibreOffice's built-in one, grey, 0.2 cm right and down.
        let automatic = style(
            "gr1",
            "graphic",
            r#"<style:graphic-properties draw:shadow="visible"/>"#,
            None,
        );
        with_scope("", &automatic, |scope| {
            for name in [Some("gr1"), None, Some("standard")] {
                let props = scope.resolve(Family::Graphic, name);
                assert_eq!(
                    [
                        qn!("draw:shadow-offset-x"),
                        qn!("draw:shadow-offset-y"),
                        qn!("draw:shadow-color"),
                    ]
                    .map(|key| get(&props.graphic, key)),
                    [Some("0.2cm"), Some("0.2cm"), Some("#808080")]
                );
            }
        });
    }

    #[test]
    fn lets_a_padding_or_margin_shorthand_replace_the_sides_it_inherits() {
        let common = [
            style(
                "base",
                "graphic",
                r#"<style:graphic-properties fo:padding-left="1cm" fo:padding-top="2cm"/>"#,
                None,
            ),
            style(
                "child",
                "graphic",
                r#"<style:graphic-properties fo:padding="3mm" fo:padding-top="4mm"/>"#,
                Some("base"),
            ),
            style(
                "p",
                "paragraph",
                r#"<style:paragraph-properties fo:margin-left="1cm"/>"#,
                None,
            ),
            style(
                "q",
                "paragraph",
                r#"<style:paragraph-properties fo:margin="2mm"/>"#,
                Some("p"),
            ),
        ]
        .concat();
        with_scope(&common, "", |scope| {
            assert_eq!(
                *scope.resolve(Family::Graphic, Some("child")).graphic,
                set(&[("fo:padding", "3mm"), ("fo:padding-top", "4mm")])
            );
            assert_eq!(
                *scope.resolve(Family::Paragraph, Some("q")).paragraph,
                set(&[("fo:margin", "2mm")])
            );
        });
    }

    #[test]
    fn lets_a_cells_own_border_shorthand_replace_the_sides_its_paragraph_properties_write() {
        let common = style(
            "cell",
            "table-cell",
            concat!(
                r##"<style:paragraph-properties fo:border-top="3pt solid #ff0000" fo:border-left="2pt solid #00ff00"/>"##,
                r##"<style:table-cell-properties fo:border="1pt solid #000000" fo:border-left="0.5pt solid #0000ff"/>"##
            ),
            None,
        );
        with_scope(&common, "", |scope| {
            assert_eq!(
                *scope.resolve(Family::TableCell, Some("cell")).cell,
                set(&[
                    ("fo:border", "1pt solid #000000"),
                    ("fo:border-left", "0.5pt solid #0000ff")
                ])
            );
        });
    }

    #[test]
    fn lets_the_later_of_two_spellings_of_a_family_or_colour_win() {
        assert_eq!(
            merge_text(
                &set(&[("style:font-name", "A")]),
                &set(&[("fo:font-family", "B")])
            ),
            set(&[("fo:font-family", "B")])
        );
        assert_eq!(
            merge_text(
                &set(&[("fo:font-family", "B")]),
                &set(&[("style:font-name", "A")])
            ),
            set(&[("style:font-name", "A")])
        );
        assert_eq!(
            merge_text(
                &set(&[("fo:color", "#ff0000")]),
                &set(&[("style:use-window-font-color", "true")])
            ),
            set(&[("style:use-window-font-color", "true")])
        );
        assert_eq!(
            merge_text(
                &set(&[("style:use-window-font-color", "true")]),
                &set(&[("fo:color", "#ff0000")])
            ),
            set(&[("fo:color", "#ff0000")])
        );
    }

    #[test]
    fn reads_font_faces_and_family_lists() {
        let root = styles("", "");
        let common = CommonStyles::new(Some(&root), None);
        assert_eq!(common.font_family("Brand"), "Brand Sans");
        assert_eq!(common.font_family("Other"), "Other");
        assert_eq!(unquote(r#""Barlow Condensed", serif"#), "Barlow Condensed");
        assert_eq!(unquote(" 'Liberation Sans'"), "Liberation Sans");
        assert_eq!(unquote("'Open, x"), "'Open");
        assert_eq!(unquote("''"), "");
    }
}
