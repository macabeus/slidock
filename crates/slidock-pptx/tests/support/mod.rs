//! What the PPTX tests share: a minimal PPTX writer (a package with a presentation, masters with their themes and
//! layouts, and slides, from XML snippets, every part what PowerPoint and Google Slides write, cut down to what a
//! test needs), the synthetic presentations each exercising one area, and the harness that lowers or compiles
//! them.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code: each test file uses some of it"
)]

pub mod fixtures;
pub mod mutate;

use indexmap::IndexMap;
use serde::Serialize;
use slidock_core::{FrontendRegistry, PassRegistry, Pipeline, Session, SessionOptions};
use slidock_core_api::{
    Analysis, ContainerOrder, Deadline, Frontend, FrontendCtx, FrontendError, Input, Limits,
    LowerOptions, LowerResult, Sink, Unverified, sort_diagnostics,
};
use slidock_html::{EmitOptions, Emitted, emit_documents};
use slidock_io::BytesReader;
use slidock_ir::diagnostics::{Diagnostic, Severity, Stage};
use slidock_ir::ids::SlideId;
use slidock_ir::{resolved, source};
use slidock_pptx::PptxFrontend;

pub const NS: &str = concat!(
    "xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" ",
    "xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" ",
    "xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" ",
    "xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" ",
    "xmlns:p14=\"http://schemas.microsoft.com/office/powerpoint/2010/main\""
);

const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>";

#[derive(Clone, Debug, Default)]
pub struct Rel {
    pub id: String,
    pub kind: String,
    pub target: String,
    pub external: bool,
}

pub fn rel(id: &str, kind: &str, target: &str) -> Rel {
    Rel {
        id: id.to_owned(),
        kind: kind.to_owned(),
        target: target.to_owned(),
        external: false,
    }
}

pub fn external(id: &str, kind: &str, target: &str) -> Rel {
    Rel {
        external: true,
        ..rel(id, kind, target)
    }
}

#[derive(Clone, Debug, Default)]
pub struct LayoutSpec {
    pub id: Option<u64>,
    pub kind: Option<String>,
    pub name: Option<String>,
    pub show_master_sp: Option<bool>,
    pub tree: String,
    pub bg: Option<String>,
    pub clr_map_ovr: Option<String>,
    pub rels: Vec<Rel>,
}

#[derive(Clone, Debug, Default)]
pub struct MasterSpec {
    pub id: Option<u64>,
    pub clr_map: Option<String>,
    pub tx_styles: Option<String>,
    pub tree: String,
    pub bg: Option<String>,
    pub layouts: Vec<LayoutSpec>,
    /// The theme's inner XML; the default theme when absent.
    pub theme: Option<String>,
    pub rels: Vec<Rel>,
}

#[derive(Clone, Debug, Default)]
pub struct SlideSpec {
    pub id: Option<u64>,
    /// The index of the layout among every master's layouts, in order.
    pub layout: Option<usize>,
    pub show: Option<bool>,
    pub show_master_sp: Option<bool>,
    pub tree: String,
    pub bg: Option<String>,
    pub clr_map_ovr: Option<String>,
    /// Children of the slide root after `p:clrMapOvr` (transition, timing).
    pub tail: String,
    pub rels: Vec<Rel>,
    /// The notes body's paragraphs (`<a:p>…`), in a notes slide.
    pub notes: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct PptxSpec {
    pub size: Option<(i64, i64)>,
    pub default_text_style: Option<String>,
    pub embedded_fonts: Option<String>,
    /// One master with one layout when `None`.
    pub masters: Option<Vec<MasterSpec>>,
    pub slides: Vec<SlideSpec>,
    /// The show order as slide indexes, when it differs from the parts' numbering.
    pub order: Option<Vec<usize>>,
    /// The table styles part's inner XML (`a:tblStyle` elements).
    pub table_styles: Option<String>,
    /// More parts, by path (media, documents).
    pub parts: Vec<(String, Vec<u8>)>,
}

pub fn slide(tree: impl Into<String>) -> SlideSpec {
    SlideSpec {
        tree: tree.into(),
        ..SlideSpec::default()
    }
}

pub const DEFAULT_THEME: &str = concat!(
    "<a:themeElements>\n",
    "<a:clrScheme name=\"Test\"><a:dk1><a:sysClr val=\"windowText\" lastClr=\"000000\"/></a:dk1><a:lt1><a:sysClr val=\"window\" lastClr=\"FFFFFF\"/></a:lt1><a:dk2><a:srgbClr val=\"44546A\"/></a:dk2><a:lt2><a:srgbClr val=\"E7E6E6\"/></a:lt2><a:accent1><a:srgbClr val=\"4472C4\"/></a:accent1><a:accent2><a:srgbClr val=\"ED7D31\"/></a:accent2><a:accent3><a:srgbClr val=\"A5A5A5\"/></a:accent3><a:accent4><a:srgbClr val=\"FFC000\"/></a:accent4><a:accent5><a:srgbClr val=\"5B9BD5\"/></a:accent5><a:accent6><a:srgbClr val=\"70AD47\"/></a:accent6><a:hlink><a:srgbClr val=\"0563C1\"/></a:hlink><a:folHlink><a:srgbClr val=\"954F72\"/></a:folHlink></a:clrScheme>\n",
    "<a:fontScheme name=\"Test\"><a:majorFont><a:latin typeface=\"Liberation Serif\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:majorFont><a:minorFont><a:latin typeface=\"Liberation Sans\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:minorFont></a:fontScheme>\n",
    "<a:fmtScheme name=\"Test\"><a:fillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:gradFill rotWithShape=\"1\"><a:gsLst><a:gs pos=\"0\"><a:schemeClr val=\"phClr\"><a:lumMod val=\"110000\"/></a:schemeClr></a:gs><a:gs pos=\"100000\"><a:schemeClr val=\"phClr\"><a:shade val=\"78000\"/></a:schemeClr></a:gs></a:gsLst><a:lin ang=\"5400000\" scaled=\"0\"/></a:gradFill><a:solidFill><a:schemeClr val=\"phClr\"><a:tint val=\"50000\"/></a:schemeClr></a:solidFill></a:fillStyleLst><a:lnStyleLst><a:ln w=\"6350\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/><a:miter lim=\"800000\"/></a:ln><a:ln w=\"12700\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/></a:ln><a:ln w=\"19050\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"dash\"/></a:ln></a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst><a:outerShdw blurRad=\"57150\" dist=\"19050\" dir=\"5400000\"><a:srgbClr val=\"000000\"><a:alpha val=\"63000\"/></a:srgbClr></a:outerShdw></a:effectLst></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"><a:tint val=\"95000\"/></a:schemeClr></a:solidFill><a:gradFill rotWithShape=\"1\"><a:gsLst><a:gs pos=\"0\"><a:schemeClr val=\"phClr\"/></a:gs><a:gs pos=\"100000\"><a:schemeClr val=\"phClr\"><a:shade val=\"50000\"/></a:schemeClr></a:gs></a:gsLst><a:lin ang=\"5400000\" scaled=\"0\"/></a:gradFill></a:bgFillStyleLst></a:fmtScheme>\n",
    "</a:themeElements>"
);

const CLR_MAP: &str = concat!(
    "bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" ",
    "accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\""
);

fn rels_xml(rels: &[Rel]) -> String {
    let items: String = rels
        .iter()
        .map(|rel| {
            let kind = if rel.kind.contains('/') {
                rel.kind.clone()
            } else {
                format!("{REL}/{}", rel.kind)
            };
            let mode = if rel.external {
                " TargetMode=\"External\""
            } else {
                ""
            };
            format!(
                "<Relationship Id=\"{}\" Type=\"{kind}\" Target=\"{}\"{mode}/>",
                rel.id, rel.target
            )
        })
        .collect();
    format!(
        "{XML}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{items}</Relationships>"
    )
}

fn tree(content: &str) -> String {
    format!(
        "<p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{content}</p:spTree>"
    )
}

fn c_sld(name: Option<&str>, bg: Option<&str>, content: &str) -> String {
    let name = name
        .map(|name| format!(" name=\"{name}\""))
        .unwrap_or_default();
    let bg = bg
        .map(|bg| format!("<p:bg>{bg}</p:bg>"))
        .unwrap_or_default();
    format!("<p:cSld{name}>{bg}{}</p:cSld>", tree(content))
}

fn flag(name: &str, value: Option<bool>) -> String {
    value
        .map(|value| format!(" {name}=\"{}\"", u8::from(value)))
        .unwrap_or_default()
}

/// The bytes of a package built from `spec`.
pub fn pptx(spec: PptxSpec) -> Vec<u8> {
    let mut files: IndexMap<String, Vec<u8>> = IndexMap::new();
    let put = |files: &mut IndexMap<String, Vec<u8>>, path: String, text: String| {
        files.insert(path, text.into_bytes());
    };
    let mut presentation_rels: Vec<Rel> = Vec::new();
    let masters = spec.masters.clone().unwrap_or_else(|| {
        vec![MasterSpec {
            layouts: vec![LayoutSpec::default()],
            ..MasterSpec::default()
        }]
    });
    let mut layout_parts: Vec<String> = Vec::new();
    let mut next_layout_id: u64 = 2_147_483_649;
    let mut layout_number = 0;

    let mut master_ids = Vec::new();
    for (index, master) in masters.iter().enumerate() {
        let n = index + 1;
        let id = master.id.unwrap_or(2_147_483_648 + index as u64 * 1000);
        let mut rels = vec![rel("rIdTheme", "theme", &format!("../theme/theme{n}.xml"))];
        rels.extend(master.rels.iter().cloned());
        let mut layout_ids = Vec::new();
        for (li, layout) in master.layouts.iter().enumerate() {
            layout_number += 1;
            let part = format!("ppt/slideLayouts/slideLayout{layout_number}.xml");
            layout_parts.push(part.clone());
            rels.push(rel(
                &format!("rIdL{}", li + 1),
                "slideLayout",
                &format!("../slideLayouts/slideLayout{layout_number}.xml"),
            ));
            let kind = layout
                .kind
                .as_ref()
                .map(|kind| format!(" type=\"{kind}\""))
                .unwrap_or_default();
            put(
                &mut files,
                part,
                format!(
                    "{XML}<p:sldLayout {NS}{kind}{}>{}<p:clrMapOvr>{}</p:clrMapOvr></p:sldLayout>",
                    flag("showMasterSp", layout.show_master_sp),
                    c_sld(layout.name.as_deref(), layout.bg.as_deref(), &layout.tree),
                    layout
                        .clr_map_ovr
                        .as_deref()
                        .unwrap_or("<a:masterClrMapping/>")
                ),
            );
            let mut layout_rels = vec![rel(
                "rIdM",
                "slideMaster",
                &format!("../slideMasters/slideMaster{n}.xml"),
            )];
            layout_rels.extend(layout.rels.iter().cloned());
            put(
                &mut files,
                format!("ppt/slideLayouts/_rels/slideLayout{layout_number}.xml.rels"),
                rels_xml(&layout_rels),
            );
            let layout_id = layout.id.unwrap_or_else(|| {
                let id = next_layout_id;
                next_layout_id += 1;
                id
            });
            layout_ids.push(format!(
                "<p:sldLayoutId id=\"{layout_id}\" r:id=\"rIdL{}\"/>",
                li + 1
            ));
        }
        put(
            &mut files,
            format!("ppt/theme/theme{n}.xml"),
            format!(
                "{XML}<a:theme {NS} name=\"Theme {n}\">{}</a:theme>",
                master.theme.as_deref().unwrap_or(DEFAULT_THEME)
            ),
        );
        let tx_styles = master
            .tx_styles
            .as_ref()
            .map(|styles| format!("<p:txStyles>{styles}</p:txStyles>"))
            .unwrap_or_default();
        put(
            &mut files,
            format!("ppt/slideMasters/slideMaster{n}.xml"),
            format!(
                "{XML}<p:sldMaster {NS}>{}<p:clrMap {}/><p:sldLayoutIdLst>{}</p:sldLayoutIdLst>{tx_styles}</p:sldMaster>",
                c_sld(None, master.bg.as_deref(), &master.tree),
                master.clr_map.as_deref().unwrap_or(CLR_MAP),
                layout_ids.join("")
            ),
        );
        put(
            &mut files,
            format!("ppt/slideMasters/_rels/slideMaster{n}.xml.rels"),
            rels_xml(&rels),
        );
        presentation_rels.push(rel(
            &format!("rIdM{n}"),
            "slideMaster",
            &format!("slideMasters/slideMaster{n}.xml"),
        ));
        master_ids.push(format!("<p:sldMasterId id=\"{id}\" r:id=\"rIdM{n}\"/>"));
    }

    let mut slide_ids = Vec::new();
    for (index, slide) in spec.slides.iter().enumerate() {
        let n = index + 1;
        let mut rels: Vec<Rel> = slide.rels.clone();
        if slide.layout.is_some() || !layout_parts.is_empty() {
            let layout = layout_parts[slide.layout.unwrap_or(0)].replacen("ppt/", "../", 1);
            rels.insert(0, rel("rIdLayout", "slideLayout", &layout));
        }
        if let Some(notes) = &slide.notes {
            rels.push(rel(
                "rIdNotes",
                "notesSlide",
                &format!("../notesSlides/notesSlide{n}.xml"),
            ));
            let body = format!(
                "<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Notes\"/><p:cNvSpPr/><p:nvPr><p:ph type=\"body\" idx=\"1\"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/>{notes}</p:txBody></p:sp>"
            );
            put(
                &mut files,
                format!("ppt/notesSlides/notesSlide{n}.xml"),
                format!("{XML}<p:notes {NS}>{}</p:notes>", c_sld(None, None, &body)),
            );
        }
        put(
            &mut files,
            format!("ppt/slides/slide{n}.xml"),
            format!(
                "{XML}<p:sld {NS}{}{}>{}<p:clrMapOvr>{}</p:clrMapOvr>{}</p:sld>",
                flag("show", slide.show),
                flag("showMasterSp", slide.show_master_sp),
                c_sld(None, slide.bg.as_deref(), &slide.tree),
                slide
                    .clr_map_ovr
                    .as_deref()
                    .unwrap_or("<a:masterClrMapping/>"),
                slide.tail
            ),
        );
        put(
            &mut files,
            format!("ppt/slides/_rels/slide{n}.xml.rels"),
            rels_xml(&rels),
        );
        presentation_rels.push(rel(
            &format!("rIdS{n}"),
            "slide",
            &format!("slides/slide{n}.xml"),
        ));
        let id = slide.id.unwrap_or(255 + n as u64);
        slide_ids.push(format!("<p:sldId id=\"{id}\" r:id=\"rIdS{n}\"/>"));
    }

    let (cx, cy) = spec.size.unwrap_or((9_144_000, 5_143_500));
    put(
        &mut files,
        "[Content_Types].xml".to_owned(),
        format!(
            "{XML}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"xml\" ContentType=\"application/xml\"/></Types>"
        ),
    );
    put(
        &mut files,
        "_rels/.rels".to_owned(),
        rels_xml(&[rel("rId1", "officeDocument", "ppt/presentation.xml")]),
    );
    let order: Vec<usize> = spec
        .order
        .clone()
        .unwrap_or_else(|| (0..slide_ids.len()).collect());
    let shown: String = order.iter().map(|&i| slide_ids[i].as_str()).collect();
    let fonts = spec
        .embedded_fonts
        .as_ref()
        .map(|fonts| format!("<p:embeddedFontLst>{fonts}</p:embeddedFontLst>"))
        .unwrap_or_default();
    let defaults = spec
        .default_text_style
        .as_ref()
        .map(|style| format!("<p:defaultTextStyle>{style}</p:defaultTextStyle>"))
        .unwrap_or_default();
    put(
        &mut files,
        "ppt/presentation.xml".to_owned(),
        format!(
            "{XML}<p:presentation {NS}><p:sldMasterIdLst>{}</p:sldMasterIdLst><p:sldIdLst>{shown}</p:sldIdLst><p:sldSz cx=\"{cx}\" cy=\"{cy}\"/>{fonts}{defaults}</p:presentation>",
            master_ids.join("")
        ),
    );
    if let Some(styles) = &spec.table_styles {
        presentation_rels.push(rel("rIdTableStyles", "tableStyles", "tableStyles.xml"));
        put(
            &mut files,
            "ppt/tableStyles.xml".to_owned(),
            format!("{XML}<a:tblStyleLst {NS}>{styles}</a:tblStyleLst>"),
        );
    }
    put(
        &mut files,
        "ppt/_rels/presentation.xml.rels".to_owned(),
        rels_xml(&presentation_rels),
    );
    for (path, bytes) in spec.parts {
        files.insert(path, bytes);
    }
    zip(&files, 6)
}

/// `Math.round(pt * 12700)`.
fn emu(pt: f64) -> i64 {
    slidock_canon::js_round(pt * 12_700.0) as i64
}

/// A shape (`p:sp`) with its non-visual properties and the given properties and body.
#[derive(Clone, Debug, Default)]
pub struct Sp {
    pub id: u64,
    pub name: Option<String>,
    pub ph: String,
    pub sp_pr: String,
    pub tx_body: Option<String>,
    pub style: Option<String>,
    /// More attributes of `p:cNvPr`.
    pub c_nv_pr: String,
    /// Children of `p:cNvPr` (a hyperlink).
    pub link: Option<String>,
}

pub fn sp(options: Sp) -> String {
    let name = options
        .name
        .clone()
        .unwrap_or_else(|| format!("Shape {}", options.id));
    let c_nv_pr = format!(
        "<p:cNvPr id=\"{}\" name=\"{name}\"{}",
        options.id, options.c_nv_pr
    );
    let c_nv_pr = match &options.link {
        Some(link) => format!("{c_nv_pr}>{link}</p:cNvPr>"),
        None => format!("{c_nv_pr}/>"),
    };
    let style = options
        .style
        .as_ref()
        .map(|style| format!("<p:style>{style}</p:style>"))
        .unwrap_or_default();
    let tx_body = options
        .tx_body
        .as_ref()
        .map(|body| format!("<p:txBody>{body}</p:txBody>"))
        .unwrap_or_default();
    format!(
        "<p:sp><p:nvSpPr>{c_nv_pr}<p:cNvSpPr/><p:nvPr>{}</p:nvPr></p:nvSpPr><p:spPr>{}</p:spPr>{style}{tx_body}</p:sp>",
        options.ph, options.sp_pr
    )
}

/// An `a:xfrm` in points.
pub fn xfrm(x: f64, y: f64, w: f64, h: f64, extra: &str) -> String {
    format!(
        "<a:xfrm{extra}><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>",
        emu(x),
        emu(y),
        emu(w),
        emu(h)
    )
}

/// One table row: its height in points and its cells.
pub struct Row {
    pub height: f64,
    pub cells: Vec<String>,
}

/// A graphic frame holding a table: `a:tblPr`'s attributes and children, the column widths in points, the rows.
pub struct TableSpec {
    pub id: u64,
    pub frame: String,
    pub tbl_pr: Option<String>,
    pub columns: Vec<f64>,
    pub rows: Vec<Row>,
    pub ph: String,
}

pub fn table(options: TableSpec) -> String {
    let columns: String = options
        .columns
        .iter()
        .map(|w| format!("<a:gridCol w=\"{}\"/>", emu(*w)))
        .collect();
    let rows: String = options
        .rows
        .iter()
        .map(|row| {
            format!(
                "<a:tr h=\"{}\">{}</a:tr>",
                emu(row.height),
                row.cells.join("")
            )
        })
        .collect();
    format!(
        "<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"{id}\" name=\"Table {id}\"/><p:cNvGraphicFramePr/><p:nvPr>{ph}</p:nvPr></p:nvGraphicFramePr>{frame}<a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/table\"><a:tbl>{tbl_pr}<a:tblGrid>{columns}</a:tblGrid>{rows}</a:tbl></a:graphicData></a:graphic></p:graphicFrame>",
        id = options.id,
        ph = options.ph,
        frame = options.frame.replace("a:xfrm", "p:xfrm"),
        tbl_pr = options.tbl_pr.as_deref().unwrap_or("<a:tblPr/>"),
    )
}

/// A table cell: its text (one paragraph per string; no text body when `None`), its `a:tcPr` and attributes.
pub fn tc(text: Option<&[&str]>, tc_pr: &str, attrs: &str) -> String {
    let tx_body = text
        .map(|paragraphs| format!("<a:txBody>{}</a:txBody>", body(paragraphs, "<a:bodyPr/>")))
        .unwrap_or_default();
    let attrs = if attrs.is_empty() {
        String::new()
    } else {
        format!(" {attrs}")
    };
    format!("<a:tc{attrs}>{tx_body}{tc_pr}</a:tc>")
}

/// A one-paragraph cell.
pub fn tc1(text: &str) -> String {
    tc(Some(&[text]), "", "")
}

/// A text body with one paragraph per string.
pub fn body(paragraphs: &[&str], body_pr: &str) -> String {
    let paragraphs: String = paragraphs
        .iter()
        .map(|text| format!("<a:p><a:r><a:rPr lang=\"en-US\"/><a:t>{text}</a:t></a:r></a:p>"))
        .collect();
    format!("{body_pr}<a:lstStyle/>{paragraphs}")
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                0xedb8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A zip of `files` in order, every entry STORED (`level` 0) or DEFLATE at `level`.
pub fn zip(files: &IndexMap<String, Vec<u8>>, level: u8) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, content) in files {
        let (method, data): (u16, Vec<u8>) = if level == 0 {
            (0, content.clone())
        } else {
            (8, miniz_oxide::deflate::compress_to_vec(content, level))
        };
        let offset = out.len() as u32;
        let crc = crc32(content);
        let common = |header: &mut Vec<u8>| {
            header.extend(20u16.to_le_bytes());
            header.extend(0u16.to_le_bytes());
            header.extend(method.to_le_bytes());
            header.extend(0u16.to_le_bytes());
            header.extend(0x21u16.to_le_bytes());
            header.extend(crc.to_le_bytes());
            header.extend((data.len() as u32).to_le_bytes());
            header.extend((content.len() as u32).to_le_bytes());
            header.extend((name.len() as u16).to_le_bytes());
            header.extend(0u16.to_le_bytes());
        };
        out.extend(0x0403_4b50u32.to_le_bytes());
        common(&mut out);
        out.extend(name.as_bytes());
        out.extend(&data);
        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend(20u16.to_le_bytes());
        common(&mut central);
        // Comment length, disk number, internal and external attributes.
        central.extend([0u8; 10]);
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let directory_offset = out.len() as u32;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0; 4]);
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(directory_offset.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

/// Every entry of a zip, inflated, in directory order.
pub fn unzip(bytes: &[u8]) -> IndexMap<String, Vec<u8>> {
    let reader = BytesReader::new(bytes);
    let archive = slidock_io::ZipArchive::open(&reader, slidock_io::ZipLimits::default()).unwrap();
    archive
        .entries()
        .iter()
        .map(|entry| {
            (
                entry.name.clone(),
                archive.read(entry).unwrap().into_owned(),
            )
        })
        .collect()
}

/// The frontend's analysis and lowering of a presentation, and its diagnostics in output order.
pub struct Lowered {
    pub analysis: Analysis,
    pub lowered: LowerResult,
    pub diagnostics: Vec<Diagnostic>,
}

impl Lowered {
    pub fn presentation(&self) -> &source::Presentation {
        match &self.lowered.presentation {
            Unverified::Typed(presentation) => presentation,
            Unverified::Json(_) => panic!("typed IR unless a custom path coordinate is NaN"),
        }
    }

    /// The codes reported on `node`, in output order.
    pub fn codes(&self, node: &str) -> Vec<String> {
        self.diagnostics
            .iter()
            .filter(|d| d.node.as_ref().is_some_and(|n| n.as_str() == node))
            .map(|d| d.code.code().to_owned())
            .collect()
    }

    pub fn find(&self, code: &str) -> Option<&Diagnostic> {
        self.diagnostics.iter().find(|d| d.code.code() == code)
    }
}

pub fn input<'a>(bytes: &'a BytesReader<&'a [u8]>, name: &'a str) -> Input<'a> {
    Input::new(name, bytes)
}

/// Analyzes and lowers `bytes` (only the slides `only` names, when set) under `limits`.
pub fn lower_with(
    bytes: &[u8],
    only: Option<&[SlideId]>,
    limits: &Limits,
) -> Result<Lowered, FrontendError> {
    lower_motion(bytes, only, limits, false)
}

/// Lowers `bytes` with native motion (the deck path).
pub fn lower_native(bytes: &[u8]) -> Lowered {
    lower_motion(bytes, None, &Limits::default(), true).unwrap()
}

/// Analyzes and lowers `bytes`, reading motion natively when `native_motion`.
pub fn lower_motion(
    bytes: &[u8],
    only: Option<&[SlideId]>,
    limits: &Limits,
    native_motion: bool,
) -> Result<Lowered, FrontendError> {
    let frontend = PptxFrontend;
    let reader = BytesReader::new(bytes);
    let input = Input::new("presentation.pptx", &reader);
    let mut out = Vec::new();
    let mut sink = Sink::new(&mut out, Stage::Frontend, "pptx");
    let mut ctx = FrontendCtx {
        limits,
        diag: &mut sink,
        deadline: Deadline::never(),
        spent: slidock_core_api::Output::default(),
    };
    let analysis = frontend.analyze(&input, &mut ctx)?;
    let facts = analysis.facts();
    let lowered = frontend.lower(
        &input,
        &mut ctx,
        &LowerOptions {
            facts: &facts,
            only,
            native_motion,
        },
    )?;
    let order = match &lowered.presentation {
        Unverified::Typed(presentation) => ContainerOrder::of(Some(presentation)),
        Unverified::Json(_) => ContainerOrder::default(),
    };
    Ok(Lowered {
        analysis,
        lowered,
        diagnostics: sort_diagnostics(out, &order),
    })
}

pub fn lower(bytes: &[u8]) -> Lowered {
    lower_with(bytes, None, &Limits::default()).unwrap()
}

/// What a golden file holds: the source presentation, the frontend's diagnostics and the features it saw. The
/// input's hash is left out.
pub fn golden_bytes(lowered: &Lowered) -> String {
    #[derive(Serialize)]
    struct GoldenProvenance<'a> {
        format: &'a str,
        name: &'a str,
        bytes: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        app: Option<&'a slidock_core_api::frontend::App>,
    }
    #[derive(Serialize)]
    struct Golden<'a> {
        presentation: &'a source::Presentation,
        diagnostics: &'a [Diagnostic],
        features: &'a IndexMap<String, Vec<slidock_ir::features::FeatureId>>,
        provenance: GoldenProvenance<'a>,
    }
    let provenance = &lowered.lowered.provenance;
    slidock_canon::to_canonical_json(
        &Golden {
            presentation: lowered.presentation(),
            diagnostics: &lowered.diagnostics,
            features: &lowered.lowered.features,
            provenance: GoldenProvenance {
                format: provenance.format.as_str(),
                name: &provenance.name,
                bytes: provenance.bytes,
                app: provenance.app.as_ref(),
            },
        },
        slidock_canon::Layout::Indented,
    )
    .unwrap()
}

/// A presentation compiled to HTML: the resolved IR, the documents and every diagnostic.
pub struct Compiled {
    pub presentation: Option<resolved::Presentation>,
    pub emitted: Option<Emitted>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Compiled {
    pub fn errors(&self) -> Vec<&Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect()
    }

    pub fn text(&self, path: &str) -> &str {
        &self.emitted.as_ref().unwrap().files.text[path]
    }
}

pub fn pipeline() -> Pipeline {
    let mut registry = PassRegistry::new();
    for pass in slidock_passes::passes() {
        registry.register(pass).unwrap();
    }
    Pipeline::standard(registry).unwrap()
}

/// Compiles `bytes` through the standard pipeline, verifying after every pass, and prints the documents.
pub fn compile_bytes(bytes: &[u8], name: &str) -> Compiled {
    compile_motion(bytes, name, false)
}

/// Compiles `bytes` through the session, reading motion natively when `native_motion` (the deck path).
pub fn compile_motion(bytes: &[u8], name: &str, native_motion: bool) -> Compiled {
    let registry = FrontendRegistry::new(vec![Box::new(PptxFrontend)]).unwrap();
    let pipeline = pipeline();
    let mut options = SessionOptions::new(&registry, &pipeline);
    options.verify = slidock_core::VerifyMode::Each;
    options.native_motion = native_motion;
    let reader = BytesReader::new(bytes);
    let mut session = Session::open(&Input::new(name, &reader), options).unwrap();
    let finished = session.finish().unwrap();
    let emitted = finished.presentation.as_ref().map(|presentation| {
        let provenance = session.provenance();
        let emit = EmitOptions::new(slidock_html::Provenance {
            format: provenance.format.as_str().to_owned(),
            name: provenance.name.clone(),
            sha256: provenance.sha256.to_hex(),
            bytes: provenance.bytes,
        });
        emit_documents(presentation, &emit).unwrap()
    });
    let mut diagnostics = finished.diagnostics.clone();
    if let Some(emitted) = &emitted {
        diagnostics.extend(emitted.diagnostics.iter().cloned());
    }
    Compiled {
        presentation: finished.presentation,
        emitted,
        diagnostics,
    }
}

/// The lint's problems with a compiled document set.
pub fn lint(compiled: &Compiled) -> Vec<String> {
    compiled
        .emitted
        .as_ref()
        .unwrap()
        .files
        .lint()
        .into_iter()
        .filter(slidock_format::Problem::is_error)
        .map(|problem| format!("{}: {}", problem.file, problem.message))
        .collect()
}

/// A value as JSON, as the IR prints it (integral numbers as integers).
pub fn json<T: Serialize + ?Sized>(value: &T) -> serde_json::Value {
    let text = slidock_canon::to_canonical_json(value, slidock_canon::Layout::Compact).unwrap();
    serde_json::from_str(&text).unwrap()
}

/// Whether `actual` holds every member `expected` names, recursively (`toMatchObject`): objects may hold more
/// members, arrays must match item by item.
pub fn matches(actual: &serde_json::Value, expected: &serde_json::Value) -> bool {
    use serde_json::Value;
    match (actual, expected) {
        (Value::Object(a), Value::Object(e)) => e
            .iter()
            .all(|(key, value)| a.get(key).is_some_and(|item| matches(item, value))),
        (Value::Array(a), Value::Array(e)) => {
            a.len() == e.len() && a.iter().zip(e).all(|(x, y)| matches(x, y))
        }
        (Value::Number(a), Value::Number(e)) => a.as_f64() == e.as_f64(),
        (a, e) => a == e,
    }
}

/// Asserts [`matches`], printing both values when it fails.
#[track_caller]
pub fn assert_matches(actual: &serde_json::Value, expected: serde_json::Value) {
    assert!(
        matches(actual, &expected),
        "\nactual:   {actual}\nexpected: {expected}"
    );
}

/// Every element of a slide, depth-first, through groups.
pub fn walk(elements: &[source::Element]) -> Vec<&source::Element> {
    slidock_core_api::visit::walk_elements(elements).collect()
}

/// The element of `slide` with id `id`, as JSON.
pub fn element(slide: &source::Slide, id: &str) -> serde_json::Value {
    let found = walk(&slide.elements)
        .into_iter()
        .find(|element| element.id().as_str() == id)
        .unwrap_or_else(|| panic!("no element {id}"));
    json(found)
}
