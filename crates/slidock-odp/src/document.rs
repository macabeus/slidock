//! The presentation's structure: the pages of content.xml in document order (the only order source), the master pages
//! of styles.xml, the style scopes of both parts, the slide size from the page layout of the first page's master, and
//! the footer, header and date declarations pages refer to. This is everything `analyze` needs.

use std::collections::BTreeMap;

use indexmap::IndexMap;
use slidock_canon::utf16;
use slidock_core_api::FrontendError;
use slidock_ir::diagnostics::codes;
use slidock_ir::ids::{IdPart, NodeId};
use slidock_ir::presentation::SlideSize;
use slidock_ir::units::Size;

use crate::context::{DateTimeDecl, PageDeclarations};
use crate::defaults::root_props;
use crate::ids::{id_scope, master_parts, slide_key};
use crate::names::{QName, qn};
use crate::odf::{Element, Ns};
use crate::package::{OdpPackage, PartError, refusal};
use crate::styles::{CommonStyles, Family, PropertySet, StyleScope, get};
use crate::units::size_pt;

pub(crate) const CONTENT: &str = "content.xml";
pub(crate) const STYLES: &str = "styles.xml";

/// The parts a document is read from: content.xml, which every presentation has, and styles.xml, or why it could
/// not be read.
pub(crate) struct Parts {
    pub content: Element,
    pub styles: Result<Element, PartError>,
}

impl Parts {
    /// Reads content.xml (refusing the input when it cannot be read) and styles.xml.
    pub(crate) fn read(pkg: &OdpPackage<'_>, name: &str) -> Result<Parts, FrontendError> {
        let content = pkg
            .xml(CONTENT)
            .map_err(|error| refusal(name, codes::OD9001, &error.message()))?;
        if !content.is(Ns::Office, "document-content") {
            return Err(refusal(
                name,
                codes::OD9001,
                &format!("{CONTENT} is not document content"),
            ));
        }
        let styles = pkg.xml(STYLES).and_then(|styles| {
            if styles.is(Ns::Office, "document-styles") {
                Ok(styles)
            } else {
                Err(PartError::new(STYLES, "not document styles"))
            }
        });
        Ok(Parts { content, styles })
    }

    pub(crate) fn styles(&self) -> Option<&Element> {
        self.styles.as_ref().ok()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PageRef<'a> {
    pub element: &'a Element,
    pub id: NodeId,
    pub duplicate: bool,
    pub path: String,
}

#[derive(Clone, Debug)]
pub(crate) struct MasterRef<'a> {
    pub element: &'a Element,
    pub id: NodeId,
    pub duplicate: bool,
    pub path: String,
    pub name: &'a str,
}

/// A read document: its pages and master pages, the styles each part sees, its slide size and declarations.
pub(crate) struct OdpDocument<'a> {
    /// Why styles.xml could not be read: every master page a page names then fails.
    pub styles_error: Option<&'a PartError>,
    /// The styles slides see (content.xml's automatic styles) and master pages see (styles.xml's).
    pub content_styles: StyleScope<'a>,
    pub master_styles: StyleScope<'a>,
    pub pages: Vec<PageRef<'a>>,
    pub masters: IndexMap<&'a str, MasterRef<'a>>,
    pub size: SlideSize,
    /// `presentation:footer-decl`, `presentation:header-decl` and `presentation:date-time-decl` by name.
    footers: BTreeMap<&'a str, String>,
    headers: BTreeMap<&'a str, String>,
    date_times: BTreeMap<&'a str, DateTimeDecl>,
}

/// LibreOffice's default slide (28 × 15.75 cm), for a page layout missing its size.
fn default_size() -> SlideSize {
    SlideSize {
        w: Size::from_steps(793_701),
        h: Size::from_steps(446_457),
    }
}

impl<'a> OdpDocument<'a> {
    pub(crate) fn new(
        parts: &'a Parts,
        common: &'a CommonStyles<'a>,
    ) -> Result<Self, FrontendError> {
        let content = &parts.content;
        let styles = parts.styles();
        let content_styles = StyleScope::new(common, Some(content), root_props());
        let master_styles = StyleScope::new(common, styles, root_props());

        let mut ids = id_scope();
        let mut masters: IndexMap<&'a str, MasterRef<'a>> = IndexMap::new();
        let styles_name = styles.map_or("undefined", |styles| styles.name.as_str());
        let master_styles_path = format!("/{styles_name}/office:master-styles");
        let master_styles_element =
            styles.and_then(|styles| styles.child(Ns::Office, "master-styles"));
        for (element, path) in master_styles_element
            .map(|e| e.positioned(&master_styles_path))
            .unwrap_or_default()
        {
            if !element.is(Ns::Style, "master-page") {
                continue;
            }
            let Some(name) = element.attr(qn!("style:name")).filter(|s| !s.is_empty()) else {
                continue;
            };
            if masters.contains_key(name) {
                continue;
            }
            let taken = ids.take(&master_parts(name)).map_err(crashed)?;
            masters.insert(
                name,
                MasterRef {
                    element,
                    id: taken.id,
                    duplicate: taken.duplicate,
                    path,
                    name,
                },
            );
        }

        let body = content
            .child(Ns::Office, "body")
            .and_then(|body| body.child(Ns::Office, "presentation"));
        let pages_path = format!("/{}/office:body/office:presentation", content.name);
        let mut pages: Vec<PageRef<'a>> = Vec::new();
        for (element, path) in body.map(|b| b.positioned(&pages_path)).unwrap_or_default() {
            if !element.is(Ns::Draw, "page") {
                continue;
            }
            let index = pages.len() + 1;
            let key = slide_key(element, index);
            let taken = ids.take(&[IdPart::Str(&key)]).map_err(crashed)?;
            pages.push(PageRef {
                element,
                id: taken.id,
                duplicate: taken.duplicate,
                path,
            });
        }

        let footers = declared(body, "footer-decl");
        let headers = declared(body, "header-decl");
        let mut date_times = BTreeMap::new();
        for decl in body
            .into_iter()
            .flat_map(|b| b.children_of(Ns::Presentation, "date-time-decl"))
        {
            if let Some(name) = decl
                .attr(qn!("presentation:name"))
                .filter(|s| !s.is_empty())
            {
                date_times.insert(
                    name,
                    DateTimeDecl {
                        fixed: decl.attr(qn!("presentation:source")) == Some("fixed"),
                        text: utf16::clip(&decl.text_of(), 256).to_owned(),
                    },
                );
            }
        }
        let size = slide_size(common, &masters, &pages);
        Ok(OdpDocument {
            styles_error: parts.styles.as_ref().err(),
            content_styles,
            master_styles,
            pages,
            masters,
            size,
            footers,
            headers,
            date_times,
        })
    }

    /// The footer, header and date declarations a page uses.
    pub(crate) fn declarations_of(&self, page: &Element) -> PageDeclarations {
        let name = |attr: QName| page.attr(attr).unwrap_or("");
        PageDeclarations {
            footer: self
                .footers
                .get(name(qn!("presentation:use-footer-name")))
                .filter(|s| !s.is_empty())
                .cloned(),
            header: self
                .headers
                .get(name(qn!("presentation:use-header-name")))
                .filter(|s| !s.is_empty())
                .cloned(),
            date_time: self
                .date_times
                .get(name(qn!("presentation:use-date-time-name")))
                .cloned(),
        }
    }
}

/// The footer or header declarations of a presentation, by name.
fn declared<'a>(body: Option<&'a Element>, local: &str) -> BTreeMap<&'a str, String> {
    let mut out = BTreeMap::new();
    for decl in body
        .into_iter()
        .flat_map(|b| b.children_of(Ns::Presentation, local))
    {
        if let Some(name) = decl
            .attr(qn!("presentation:name"))
            .filter(|s| !s.is_empty())
        {
            out.insert(name, utf16::clip(&decl.text_of(), 4096).to_owned());
        }
    }
    out
}

fn crashed(error: slidock_ir::ids::IdError) -> FrontendError {
    FrontendError::Crashed(error.to_string())
}

/// The page layout of the first page's master page, else of the first master page.
fn slide_size(
    common: &CommonStyles<'_>,
    masters: &IndexMap<&str, MasterRef<'_>>,
    pages: &[PageRef<'_>],
) -> SlideSize {
    let first = pages.first().and_then(|page| {
        masters.get(
            page.element
                .attr(qn!("draw:master-page-name"))
                .unwrap_or(""),
        )
    });
    let master = first.or_else(|| masters.values().next());
    let layout = common.page_layouts.get(
        master
            .and_then(|master| master.element.attr(qn!("style:page-layout-name")))
            .unwrap_or(""),
    );
    let props = layout.and_then(|layout| layout.child(Ns::Style, "page-layout-properties"));
    let w = size_pt(props.and_then(|p| p.attr(qn!("fo:page-width"))));
    let h = size_pt(props.and_then(|p| p.attr(qn!("fo:page-height"))));
    match (w, h) {
        (Some(w), Some(h)) if w != 0.0 && h != 0.0 => SlideSize {
            w: crate::units::size(w),
            h: crate::units::size(h),
        },
        _ => default_size(),
    }
}

/// The drawing-page properties of a page or master page.
pub(crate) fn page_props(page: &Element, styles: &StyleScope<'_>) -> std::rc::Rc<PropertySet> {
    std::rc::Rc::clone(
        &styles
            .resolve(Family::DrawingPage, page.attr(qn!("draw:style-name")))
            .drawing_page,
    )
}

/// Whether a page is hidden from the show (`presentation:visibility`, on its drawing-page style).
pub(crate) fn is_hidden(page: &Element, styles: &StyleScope<'_>) -> bool {
    get(&page_props(page, styles), qn!("presentation:visibility")) == Some("hidden")
}

/// Every `xlink:href` a part's pictures, media and fill images name.
pub(crate) fn references(root: Option<&Element>) -> Vec<&str> {
    fn visit<'e>(element: &'e Element, out: &mut Vec<&'e str>) {
        if let Some(href) = element.attr(qn!("xlink:href")).filter(|s| !s.is_empty())
            && matches!(element.local.as_str(), "image" | "plugin" | "fill-image")
        {
            out.push(href);
        }
        for node in element.kids() {
            visit(node, out);
        }
    }
    let mut out = Vec::new();
    if let Some(root) = root {
        visit(root, &mut out);
    }
    out
}
