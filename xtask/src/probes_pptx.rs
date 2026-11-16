//! Writes the PPTX probe deck: a minimal PresentationML package (one master, one blank layout, one theme) whose
//! slides carry the catalog's transitions and `p:timing` trees, in the shapes PowerPoint writes (ECMA-376 Part 1
//! §19.3.1.x transitions, §19.5 timing; `p14`/`p15`/`p159` inside `mc:AlternateContent` with a fallback choice).

use std::fmt::Write as _;

use super::probes_catalog::{
    After, Bhv, Class, Effect, Ext, Fmt, Interval, OUTLINE_COLOR, Repeat, Shape, Slide, Step,
    Trigger, Unit, layout,
};
use super::probes_zip::Entry;

pub const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
pub const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
pub const NS_P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const NS_MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const NS_P14: &str = "http://schemas.microsoft.com/office/powerpoint/2010/main";
const NS_P15: &str = "http://schemas.microsoft.com/office/powerpoint/2012/main";
const NS_P159: &str = "http://schemas.microsoft.com/office/powerpoint/2015/09/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

/// EMU per point.
const EMU: i64 = 12_700;

/// A probe line's width, 6 pt in EMU: its stroke and arrowhead overflow its box of zero height.
const LINE_WIDTH: i64 = 6 * EMU;

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

/// A shape's box in points: (x, y, width, height). `count` is how many shapes share the slide.
pub fn shape_box(index: usize, count: usize, big: bool) -> (i64, i64, i64, i64) {
    if big || count <= 1 {
        return (72, 96, 816, 400);
    }
    let cols = if count > 6 { 4 } else { 3 };
    let col = (index % cols) as i64;
    let row = (index / cols) as i64;
    let cell_w = 912 / cols as i64;
    (24 + col * cell_w + 8, 80 + row * 225 + 8, cell_w - 16, 209)
}

pub const TITLE_BOX: (i64, i64, i64, i64) = (36, 14, 888, 56);

/// Slide background colours, alternating so a transition shows.
pub fn background(index: usize) -> &'static str {
    if index.is_multiple_of(2) {
        "F4F1EA"
    } else {
        "E3ECF7"
    }
}

pub fn write(slides: &[Slide], clip: &[u8], poster: &[u8]) -> Vec<Entry> {
    let included: Vec<&Slide> = slides.iter().filter(|s| s.formats.pptx).collect();
    let mut entries = Vec::new();
    let mut types = String::from(DECL);
    types
        .push_str("<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">");
    types.push_str("<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>");
    types.push_str("<Default Extension=\"xml\" ContentType=\"application/xml\"/>");
    types.push_str("<Default Extension=\"mp4\" ContentType=\"video/mp4\"/>");
    types.push_str("<Default Extension=\"png\" ContentType=\"image/png\"/>");
    let ct = "application/vnd.openxmlformats-officedocument.presentationml";
    let _ = write!(
        types,
        "<Override PartName=\"/ppt/presentation.xml\" ContentType=\"{ct}.presentation.main+xml\"/>"
    );
    let _ = write!(
        types,
        "<Override PartName=\"/ppt/presProps.xml\" ContentType=\"{ct}.presProps+xml\"/>"
    );
    let _ = write!(
        types,
        "<Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"{ct}.slideMaster+xml\"/>"
    );
    let _ = write!(
        types,
        "<Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"{ct}.slideLayout+xml\"/>"
    );
    types.push_str("<Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>");
    for n in 1..=included.len() {
        let _ = write!(
            types,
            "<Override PartName=\"/ppt/slides/slide{n}.xml\" ContentType=\"{ct}.slide+xml\"/>"
        );
    }
    types.push_str("</Types>");
    entries.push(xml_entry("[Content_Types].xml", types));
    entries.push(xml_entry(
        "_rels/.rels",
        rels(&[("rId1", "officeDocument", "ppt/presentation.xml")]),
    ));
    let mut pres = String::from(DECL);
    let _ = write!(
        pres,
        "<p:presentation xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\">"
    );
    pres.push_str("<p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst><p:sldIdLst>");
    for n in 0..included.len() {
        let _ = write!(pres, "<p:sldId id=\"{}\" r:id=\"rId{}\"/>", 256 + n, 4 + n);
    }
    pres.push_str("</p:sldIdLst><p:sldSz cx=\"12192000\" cy=\"6858000\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>");
    entries.push(xml_entry("ppt/presentation.xml", pres));
    let mut pres_rels: Vec<(String, &str, String)> = vec![
        (
            "rId1".into(),
            "slideMaster",
            "slideMasters/slideMaster1.xml".into(),
        ),
        ("rId2".into(), "theme", "theme/theme1.xml".into()),
        ("rId3".into(), "presProps", "presProps.xml".into()),
    ];
    for n in 0..included.len() {
        pres_rels.push((
            format!("rId{}", 4 + n),
            "slide",
            format!("slides/slide{}.xml", n + 1),
        ));
    }
    let pres_rels: Vec<(&str, &str, &str)> = pres_rels
        .iter()
        .map(|(a, b, c)| (a.as_str(), *b, c.as_str()))
        .collect();
    entries.push(xml_entry(
        "ppt/_rels/presentation.xml.rels",
        rels(&pres_rels),
    ));
    entries.push(xml_entry(
        "ppt/presProps.xml",
        format!(
            "{DECL}<p:presentationPr xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\"/>"
        ),
    ));
    entries.push(xml_entry("ppt/slideMasters/slideMaster1.xml", master()));
    entries.push(xml_entry(
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        rels(&[
            ("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml"),
            ("rId2", "theme", "../theme/theme1.xml"),
        ]),
    ));
    entries.push(xml_entry(
        "ppt/slideLayouts/slideLayout1.xml",
        format!(
            "{DECL}<p:sldLayout xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\" type=\"blank\" preserve=\"1\"><p:cSld name=\"Blank\"><p:spTree>{}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>",
            GROUP
        ),
    ));
    entries.push(xml_entry(
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        rels(&[("rId1", "slideMaster", "../slideMasters/slideMaster1.xml")]),
    ));
    entries.push(xml_entry("ppt/theme/theme1.xml", theme()));
    let mut media = false;
    for (n, slide) in included.iter().enumerate() {
        let has_media = slide.shapes.iter().any(|s| s.media && s.formats.pptx);
        media |= has_media;
        entries.push(xml_entry(
            &format!("ppt/slides/slide{}.xml", n + 1),
            slide_xml(slide, n),
        ));
        let mut slide_rels = vec![("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml")];
        if has_media {
            slide_rels.push(("rId2", "media", "../media/clip.mp4"));
            slide_rels.push(("rId3", "video", "../media/clip.mp4"));
            slide_rels.push(("rId4", "image", "../media/poster.png"));
        }
        entries.push(xml_entry(
            &format!("ppt/slides/_rels/slide{}.xml.rels", n + 1),
            rels(&slide_rels),
        ));
    }
    if media {
        entries.push(Entry {
            name: "ppt/media/clip.mp4".into(),
            data: clip.to_vec(),
            stored: true,
        });
        entries.push(Entry {
            name: "ppt/media/poster.png".into(),
            data: poster.to_vec(),
            stored: true,
        });
    }
    entries
}

fn xml_entry(name: &str, text: String) -> Entry {
    Entry {
        name: name.to_owned(),
        data: text.into_bytes(),
        stored: false,
    }
}

fn rels(list: &[(&str, &str, &str)]) -> String {
    let mut out = String::from(DECL);
    out.push_str(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for (id, kind, target) in list {
        let ty = match *kind {
            "media" => "http://schemas.microsoft.com/office/2007/relationships/media".to_owned(),
            other => format!("{REL}/{other}"),
        };
        let _ = write!(
            out,
            "<Relationship Id=\"{id}\" Type=\"{ty}\" Target=\"{target}\"/>"
        );
    }
    out.push_str("</Relationships>");
    out
}

const GROUP: &str = "<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>";

fn master() -> String {
    format!(
        "{DECL}<p:sldMaster xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\"><p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill><a:effectLst/></p:bgPr></p:bg><p:spTree>{GROUP}</p:spTree></p:cSld><p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/><p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst></p:sldMaster>"
    )
}

fn theme() -> String {
    let mut out = format!(
        "{DECL}<a:theme xmlns:a=\"{NS_A}\" name=\"Probe\"><a:themeElements><a:clrScheme name=\"Probe\">"
    );
    for (name, value) in [
        ("dk1", "000000"),
        ("lt1", "FFFFFF"),
        ("dk2", "1F2937"),
        ("lt2", "F4F1EA"),
        ("accent1", "2F6FDF"),
        ("accent2", "1E9E6A"),
        ("accent3", "C2410C"),
        ("accent4", "7C3AED"),
        ("accent5", "B91C1C"),
        ("accent6", "0E7490"),
        ("hlink", "2F6FDF"),
        ("folHlink", "7C3AED"),
    ] {
        let _ = write!(out, "<a:{name}><a:srgbClr val=\"{value}\"/></a:{name}>");
    }
    out.push_str("</a:clrScheme><a:fontScheme name=\"Probe\">");
    for font in ["majorFont", "minorFont"] {
        let _ = write!(
            out,
            "<a:{font}><a:latin typeface=\"Liberation Sans\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:{font}>"
        );
    }
    out.push_str("</a:fontScheme><a:fmtScheme name=\"Probe\"><a:fillStyleLst>");
    let fill = "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>";
    out.push_str(&fill.repeat(3));
    out.push_str("</a:fillStyleLst><a:lnStyleLst>");
    out.push_str(&format!("<a:ln w=\"9525\">{fill}</a:ln>").repeat(3));
    out.push_str("</a:lnStyleLst><a:effectStyleLst>");
    out.push_str(&"<a:effectStyle><a:effectLst/></a:effectStyle>".repeat(3));
    out.push_str("</a:effectStyleLst><a:bgFillStyleLst>");
    out.push_str(&fill.repeat(3));
    out.push_str("</a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>");
    out
}

fn text_body(shape: &Shape, size: u32, color: &str, center: bool) -> String {
    let mut out = String::from(
        "<p:txBody><a:bodyPr wrap=\"square\" lIns=\"91440\" tIns=\"45720\" rIns=\"91440\" bIns=\"45720\" anchor=\"ctr\"><a:normAutofit/></a:bodyPr><a:lstStyle/>",
    );
    for para in &shape.paras {
        out.push_str("<a:p>");
        let algn = if center { " algn=\"ctr\"" } else { "" };
        if para.level > 0 {
            let _ = write!(out, "<a:pPr lvl=\"{}\"{algn}/>", para.level);
        } else if center {
            let _ = write!(out, "<a:pPr{algn}/>");
        }
        let rpr = format!(
            "lang=\"en-US\" sz=\"{size}\" dirty=\"0\"><a:solidFill><a:srgbClr val=\"{color}\"/></a:solidFill><a:latin typeface=\"Liberation Sans\"/>"
        );
        if para.text.is_empty() {
            let _ = write!(out, "<a:endParaRPr {rpr}</a:endParaRPr>");
        } else {
            let _ = write!(
                out,
                "<a:r><a:rPr {rpr}</a:rPr><a:t>{}</a:t></a:r>",
                escape(&para.text)
            );
        }
        out.push_str("</a:p>");
    }
    out.push_str("</p:txBody>");
    out
}

fn xfrm((x, y, w, h): (i64, i64, i64, i64)) -> String {
    format!(
        "<a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>",
        x * EMU,
        y * EMU,
        w * EMU,
        h * EMU
    )
}

fn slide_xml(slide: &Slide, index: usize) -> String {
    let mut out = String::from(DECL);
    let _ = write!(
        out,
        "<p:sld xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\">"
    );
    let _ = write!(
        out,
        "<p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill><a:effectLst/></p:bgPr></p:bg><p:spTree>{GROUP}",
        background(index)
    );
    let title = Shape {
        paras: vec![super::probes_catalog::Para {
            text: slide.title.clone(),
            level: 0,
        }],
        media: false,
        line: false,
        color: "",
        formats: slide.formats,
    };
    let _ = write!(
        out,
        "<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Title\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr><p:spPr>{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/></p:spPr>{}</p:sp>",
        xfrm(TITLE_BOX),
        text_body(&title, 2000, "1F2937", false)
    );
    let spids = spids(slide);
    let shown: Vec<usize> = (0..slide.shapes.len())
        .filter(|&i| slide.shapes[i].formats.pptx)
        .collect();
    let big = slide.transition.is_some() || slide.shapes.iter().any(|s| s.paras.len() > 1);
    for (position, &i) in shown.iter().enumerate() {
        let shape = &slide.shapes[i];
        let spid = spids[i].unwrap_or(0);
        let frame = xfrm(shape_box(position, shown.len(), big));
        if shape.line {
            let (x, y, w, h) = shape_box(position, shown.len(), big);
            let _ = write!(
                out,
                "<p:cxnSp><p:nvCxnSpPr><p:cNvPr id=\"{spid}\" name=\"Probe line\" descr=\"{}\"/><p:cNvCxnSpPr/><p:nvPr/></p:nvCxnSpPr><p:spPr>{}<a:prstGeom prst=\"line\"><a:avLst/></a:prstGeom><a:ln w=\"{LINE_WIDTH}\"><a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill><a:tailEnd type=\"triangle\" w=\"lg\" len=\"lg\"/></a:ln></p:spPr></p:cxnSp>",
                escape(&shape.label()),
                xfrm((x, y + h / 2, w, 0)),
                shape.color
            );
        } else if shape.media {
            let _ = write!(
                out,
                "<p:pic><p:nvPicPr><p:cNvPr id=\"{spid}\" name=\"Probe clip\" descr=\"{}\"/><p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr><p:nvPr><a:videoFile r:link=\"rId3\"/><p:extLst><p:ext uri=\"{{DAA4B4D4-6D71-4841-9C94-3DE7FCFB9230}}\"><p14:media xmlns:p14=\"{NS_P14}\" r:embed=\"rId2\"/></p:ext></p:extLst></p:nvPr></p:nvPicPr><p:blipFill><a:blip r:embed=\"rId4\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr>{frame}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>",
                escape(&shape.label())
            );
        } else {
            let _ = write!(
                out,
                "<p:sp><p:nvSpPr><p:cNvPr id=\"{spid}\" name=\"Probe {position}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr>{frame}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill>{}</p:spPr>{}</p:sp>",
                shape.color,
                if slide.outlined(i) {
                    format!(
                        "<a:ln w=\"19050\"><a:solidFill><a:srgbClr val=\"{OUTLINE_COLOR}\"/></a:solidFill></a:ln>"
                    )
                } else {
                    "<a:ln><a:noFill/></a:ln>".to_owned()
                },
                text_body(shape, if big { 2000 } else { 1200 }, "FFFFFF", true)
            );
        }
    }
    out.push_str("</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>");
    if let Some(trans) = slide.transition.as_ref().and_then(|t| t.pptx.as_ref()) {
        out.push_str(&transition_xml(trans));
    }
    out.push_str(&timing_xml(slide, &spids));
    out.push_str("</p:sld>");
    out
}

/// The `cNvPr@id` of each shape in this deck (`None` for shapes PPTX leaves out); the title is 2.
pub fn spids(slide: &Slide) -> Vec<Option<u32>> {
    let mut next = 3;
    slide
        .shapes
        .iter()
        .map(|shape| {
            shape.formats.pptx.then(|| {
                next += 1;
                next - 1
            })
        })
        .collect()
}

fn transition_xml(trans: &super::probes_catalog::PTrans) -> String {
    let mut attrs = String::new();
    if let Some(spd) = trans.spd {
        let _ = write!(attrs, " spd=\"{spd}\"");
    }
    if let Some(click) = trans.adv_click {
        let _ = write!(attrs, " advClick=\"{}\"", if click { 1 } else { 0 });
    }
    if let Some(tm) = trans.adv_tm_raw {
        let _ = write!(attrs, " advTm=\"{tm}\"");
    } else if let Some(tm) = trans.adv_tm {
        let _ = write!(attrs, " advTm=\"{tm}\"");
    }
    let mut child_attrs = String::new();
    for (key, value) in &trans.attrs {
        let _ = write!(child_attrs, " {key}=\"{value}\"");
    }
    let prefix = match trans.ext {
        Ext::Ecma => "p",
        Ext::P14 => "p14",
        Ext::P15 => "p15",
        Ext::P159 => "p159",
    };
    let child = format!("<{prefix}:{}{child_attrs}/>", trans.elem);
    if trans.ext == Ext::Ecma && !trans.alternate {
        return format!("<p:transition{attrs}>{child}</p:transition>");
    }
    let (requires, decls) = match trans.ext {
        Ext::Ecma | Ext::P14 => ("p14", format!(" xmlns:p14=\"{NS_P14}\"")),
        Ext::P15 => (
            "p15",
            format!(" xmlns:p14=\"{NS_P14}\" xmlns:p15=\"{NS_P15}\""),
        ),
        Ext::P159 => (
            "p159",
            format!(" xmlns:p14=\"{NS_P14}\" xmlns:p159=\"{NS_P159}\""),
        ),
    };
    let dur = trans
        .p14_dur
        .map(|d| format!(" p14:dur=\"{d}\""))
        .unwrap_or_default();
    let fallback_child = if trans.ext == Ext::Ecma {
        child.clone()
    } else {
        "<p:fade/>".to_owned()
    };
    format!(
        "<mc:AlternateContent xmlns:mc=\"{NS_MC}\"><mc:Choice{decls} Requires=\"{requires}\"><p:transition{attrs}{dur}>{child}</p:transition></mc:Choice><mc:Fallback><p:transition{attrs}>{fallback_child}</p:transition></mc:Fallback></mc:AlternateContent>"
    )
}

/// Time node ids, allocated in document order, and each step's effect node id (for conditions that name it).
struct Ids {
    last: u32,
    steps: Vec<(usize, u32)>,
}

impl Ids {
    fn next(&mut self) -> u32 {
        self.last += 1;
        self.last
    }
}

/// A condition's placeholder for the effect node of step `index`, replaced once every id is known.
fn step_ref(index: usize) -> String {
    format!("@tn{index}@")
}

fn target(step: &Step, spid: u32) -> String {
    if step.background {
        return format!("<p:tgtEl><p:spTgt spid=\"{spid}\"><p:bg/></p:spTgt></p:tgtEl>");
    }
    match step.paragraphs {
        Some((first, last)) => format!(
            "<p:tgtEl><p:spTgt spid=\"{spid}\"><p:txEl><p:pRg st=\"{first}\" end=\"{last}\"/></p:txEl></p:spTgt></p:tgtEl>"
        ),
        None => format!("<p:tgtEl><p:spTgt spid=\"{spid}\"/></p:tgtEl>"),
    }
}

fn cbhvr(
    ids: &mut Ids,
    dur: &str,
    delay: Option<u32>,
    tgt: &str,
    attrs: &[&str],
    extra: &str,
) -> String {
    let id = ids.next();
    let cond = delay
        .map(|d| format!("<p:stCondLst><p:cond delay=\"{d}\"/></p:stCondLst>"))
        .unwrap_or_default();
    let ctn = if cond.is_empty() {
        format!("<p:cTn id=\"{id}\" dur=\"{dur}\" fill=\"hold\"/>")
    } else {
        format!("<p:cTn id=\"{id}\" dur=\"{dur}\" fill=\"hold\">{cond}</p:cTn>")
    };
    let names = if attrs.is_empty() {
        String::new()
    } else {
        let list: String = attrs
            .iter()
            .map(|a| format!("<p:attrName>{a}</p:attrName>"))
            .collect();
        format!("<p:attrNameLst>{list}</p:attrNameLst>")
    };
    format!("<p:cBhvr{extra}>{ctn}{tgt}{names}</p:cBhvr>")
}

fn set(ids: &mut Ids, dur: &str, delay: Option<u32>, tgt: &str, attr: &str, to: &str) -> String {
    format!(
        "<p:set>{}<p:to>{to}</p:to></p:set>",
        cbhvr(ids, dur, delay, tgt, &[attr], "")
    )
}

fn str_val(value: &str) -> String {
    format!("<p:strVal val=\"{}\"/>", escape(value))
}

fn anim(ids: &mut Ids, dur: &str, tgt: &str, attr: &str, from: &str, to: &str) -> String {
    format!(
        "<p:anim calcmode=\"lin\" valueType=\"num\">{}<p:tavLst><p:tav tm=\"0\"><p:val>{}</p:val></p:tav><p:tav tm=\"100000\"><p:val>{}</p:val></p:tav></p:tavLst></p:anim>",
        cbhvr(ids, dur, None, tgt, &[attr], " additive=\"base\""),
        str_val(from),
        str_val(to)
    )
}

/// The attribute name of a colour: `fill`, `line` or the text's.
fn color_attr(property: &str) -> &'static str {
    match property {
        "fill" => "fillcolor",
        "line" => "stroke.color",
        _ => "style.color",
    }
}

/// `dur` is the behaviours' `dur` attribute; `ms` the same duration as a number, for the exit's hide.
fn behaviours(ids: &mut Ids, effect: &Effect, dur: &str, ms: u32, tgt: &str) -> String {
    let entering = matches!(effect.class, Class::Entr | Class::Custom);
    let exiting = effect.class == Class::Exit;
    let mut out = String::new();
    let half = (ms / 2).to_string();
    // A behaviour that reverses on its own over half the step (`autoRev`), back at rest by its end.
    let reversing =
        |cbhvr: String| cbhvr.replacen(" fill=\"hold\"", " autoRev=\"1\" fill=\"hold\"", 1);
    if entering {
        let shows_while = effect.bhv.iter().any(|b| matches!(b, Bhv::ShowWhile));
        let shown = if shows_while {
            // Flash Once: the switch lasts the step and does not hold.
            set(
                ids,
                dur,
                Some(0),
                tgt,
                "style.visibility",
                &str_val("visible"),
            )
            .replacen(" fill=\"hold\"", "", 1)
        } else {
            set(
                ids,
                "1",
                Some(0),
                tgt,
                "style.visibility",
                &str_val("visible"),
            )
        };
        out.push_str(&shown);
    }
    for bhv in &effect.bhv {
        match bhv {
            Bhv::ShowWhile => {}
            Bhv::Filter(clip) | Bhv::FilterOver(clip, _) => {
                if let Some(filter) = clip.pptx {
                    let transition = if clip.out { "out" } else { "in" };
                    let mut behaviour = cbhvr(ids, dur, None, tgt, &[], "");
                    if let Bhv::FilterOver(_, time_filter) = bhv {
                        behaviour = behaviour.replacen(
                            " fill=\"hold\"",
                            &format!(" tmFilter=\"{time_filter}\" fill=\"hold\""),
                            1,
                        );
                    }
                    let _ = write!(
                        out,
                        "<p:animEffect transition=\"{transition}\" filter=\"{}\">{behaviour}</p:animEffect>",
                        escape(filter),
                    );
                }
            }
            Bhv::Pulse(x, y) => {
                let _ = write!(
                    out,
                    "<p:animScale>{}<p:by x=\"{x}\" y=\"{y}\"/></p:animScale>",
                    reversing(cbhvr(ids, &half, None, tgt, &[], ""))
                );
            }
            Bhv::ColorFlash(property, value) => {
                let _ = write!(
                    out,
                    "<p:animClr clrSpc=\"rgb\" dir=\"cw\">{}<p:to><a:srgbClr val=\"{value}\"/></p:to></p:animClr>",
                    reversing(cbhvr(
                        ids,
                        &half,
                        None,
                        tgt,
                        &[color_attr(property)],
                        " override=\"childStyle\""
                    ))
                );
            }
            Bhv::Move { pptx, .. } => {
                out.push_str(&anim(ids, dur, tgt, "ppt_x", &pptx[0], &pptx[2]));
                out.push_str(&anim(ids, dur, tgt, "ppt_y", &pptx[1], &pptx[3]));
            }
            Bhv::GrowFromZero => {
                let (w, h) = if exiting {
                    (("#ppt_w", "0"), ("#ppt_h", "0"))
                } else {
                    (("0", "#ppt_w"), ("0", "#ppt_h"))
                };
                out.push_str(&anim(ids, dur, tgt, "ppt_w", w.0, w.1));
                out.push_str(&anim(ids, dur, tgt, "ppt_h", h.0, h.1));
            }
            Bhv::Scale(x, y) => {
                let _ = write!(
                    out,
                    "<p:animScale>{}<p:by x=\"{x}\" y=\"{y}\"/></p:animScale>",
                    cbhvr(ids, dur, None, tgt, &[], "")
                );
            }
            Bhv::Rotate(deg) => {
                let _ = write!(
                    out,
                    "<p:animRot by=\"{}\">{}</p:animRot>",
                    i64::from(*deg) * 60_000,
                    cbhvr(ids, dur, None, tgt, &["r"], "")
                );
            }
            Bhv::Opacity(value) => {
                out.push_str(&set(ids, dur, None, tgt, "style.opacity", &str_val(value)))
            }
            Bhv::Color(property, value) => {
                let _ = write!(
                    out,
                    "<p:animClr clrSpc=\"rgb\" dir=\"cw\">{}<p:to><a:srgbClr val=\"{value}\"/></p:to></p:animClr>",
                    cbhvr(
                        ids,
                        dur,
                        None,
                        tgt,
                        &[color_attr(property)],
                        " override=\"childStyle\""
                    )
                );
            }
            Bhv::ColorBy(property, h, sat, light) => {
                let _ = write!(
                    out,
                    "<p:animClr clrSpc=\"hsl\" dir=\"cw\">{}<p:by><p:hsl h=\"{}\" s=\"{}\" l=\"{}\"/></p:by></p:animClr>",
                    cbhvr(
                        ids,
                        dur,
                        None,
                        tgt,
                        &[color_attr(property)],
                        " override=\"childStyle\""
                    ),
                    i64::from(*h) * 60_000,
                    i64::from(*sat) * 1_000,
                    i64::from(*light) * 1_000
                );
            }
            Bhv::SetColor(property, value, reverse) => {
                let set = set(
                    ids,
                    if *reverse { &half } else { dur },
                    None,
                    tgt,
                    color_attr(property),
                    &format!("<p:clrVal><a:srgbClr val=\"{value}\"/></p:clrVal>"),
                );
                out.push_str(&if *reverse { reversing(set) } else { set });
            }
            Bhv::PaintOn(property) => {
                let (attr, value) = if *property == "line" {
                    ("stroke.on", "true")
                } else {
                    ("fill.type", "solid")
                };
                out.push_str(&set(ids, dur, None, tgt, attr, &str_val(value)));
            }
            Bhv::Font(switch, on) => {
                let (attr, value) = match (*switch, *on) {
                    ("bold", on) => ("style.fontWeight", if on { "bold" } else { "normal" }),
                    ("italic", on) => ("style.fontStyle", if on { "italic" } else { "normal" }),
                    (_, on) => (
                        "style.textDecorationUnderline",
                        if on { "true" } else { "false" },
                    ),
                };
                out.push_str(&set(ids, dur, None, tgt, attr, &str_val(value)));
            }
            Bhv::BoldPulse => {
                let tav = |tm: u32, value: &str| {
                    format!(
                        "<p:tav tm=\"{tm}\"><p:val>{}</p:val></p:tav>",
                        str_val(value)
                    )
                };
                let _ = write!(
                    out,
                    "<p:anim calcmode=\"discrete\" valueType=\"str\">{}<p:tavLst>{}{}{}{}</p:tavLst></p:anim>",
                    cbhvr(ids, dur, None, tgt, &["style.fontWeight"], ""),
                    tav(0, "normal"),
                    tav(50_000, "bold"),
                    tav(60_000, "normal"),
                    tav(100_000, "normal")
                );
            }
            Bhv::FontFamily(family) => out.push_str(&set(
                ids,
                dur,
                None,
                tgt,
                "style.fontFamily",
                &str_val(family),
            )),
            Bhv::FontSize => {
                let _ = write!(
                    out,
                    "<p:anim by=\"1.5\" calcmode=\"lin\" valueType=\"num\">{}</p:anim>",
                    cbhvr(ids, dur, None, tgt, &["style.fontSize"], "")
                );
            }
            Bhv::Motion(path) => {
                let _ = write!(
                    out,
                    "<p:animMotion origin=\"layout\" path=\"{} E\" pathEditMode=\"relative\" rAng=\"0\">{}</p:animMotion>",
                    path,
                    cbhvr(ids, dur, None, tgt, &["ppt_x", "ppt_y"], "")
                );
            }
            Bhv::Command(cmd, _) => {
                let _ = write!(
                    out,
                    "<p:cmd type=\"call\" cmd=\"{cmd}\">{}</p:cmd>",
                    cbhvr(ids, dur, None, tgt, &[], "")
                );
            }
            Bhv::Unknown => out.push_str(&set(ids, dur, None, tgt, "xshear", &str_val("0.3"))),
            Bhv::Formula { pptx, .. } => {
                let _ = write!(
                    out,
                    "<p:anim calcmode=\"lin\" valueType=\"num\">{}<p:tavLst><p:tav tm=\"0\" fmla=\"{}\"><p:val><p:fltVal val=\"0\"/></p:val></p:tav><p:tav tm=\"100000\"><p:val><p:fltVal val=\"1\"/></p:val></p:tav></p:tavLst></p:anim>",
                    cbhvr(ids, dur, None, tgt, &["ppt_x"], " additive=\"base\""),
                    escape(pptx)
                );
            }
            Bhv::DeepFormula(depth) => {
                let fmla = format!("{}$/2{}", "(".repeat(*depth), ")".repeat(*depth));
                let _ = write!(
                    out,
                    "<p:anim calcmode=\"lin\" valueType=\"num\">{}<p:tavLst><p:tav tm=\"0\" fmla=\"{fmla}\"><p:val><p:fltVal val=\"0\"/></p:val></p:tav><p:tav tm=\"100000\"><p:val><p:fltVal val=\"1\"/></p:val></p:tav></p:tavLst></p:anim>",
                    cbhvr(ids, dur, None, tgt, &["ppt_y"], " additive=\"base\"")
                );
            }
        }
    }
    if exiting {
        let delay = ms.saturating_sub(1);
        out.push_str(&set(
            ids,
            "1",
            Some(delay),
            tgt,
            "style.visibility",
            &str_val("hidden"),
        ));
    }
    out
}

fn effect_par(
    ids: &mut Ids,
    index: usize,
    step: &Step,
    spid: u32,
    grp: u32,
    node_type: &str,
) -> String {
    let wrappers: Vec<u32> = (0..step.nest).map(|_| ids.next()).collect();
    let id = ids.next();
    ids.steps.push((index, id));
    let effect = &step.effect;
    let mut attrs = format!(" id=\"{id}\"");
    if let Some(preset) = effect.pptx_id {
        let _ = write!(
            attrs,
            " presetID=\"{preset}\" presetClass=\"{}\" presetSubtype=\"{}\"",
            effect.class.pptx(),
            effect.pptx_sub
        );
    }
    match step.repeat {
        Some(Repeat::Count(count)) => {
            let _ = write!(attrs, " repeatCount=\"{count}\"");
        }
        Some(Repeat::UntilEndOfSlide | Repeat::UntilNextClick) => {
            attrs.push_str(" repeatCount=\"indefinite\"")
        }
        None => {}
    }
    if let Some(accel) = step.accel {
        let _ = write!(attrs, " accel=\"{accel}\"");
    }
    if let Some(decel) = step.decel {
        let _ = write!(attrs, " decel=\"{decel}\"");
    }
    if step.auto_rev {
        attrs.push_str(" autoRev=\"1\"");
    }
    let _ = write!(
        attrs,
        " fill=\"hold\" grpId=\"{grp}\" nodeType=\"{node_type}\""
    );
    let mut body = String::new();
    if let Some((target, end)) = step.begin_on {
        let _ = write!(
            body,
            "<p:stCondLst><p:cond evt=\"{}\" delay=\"0\"><p:tn val=\"{}\"/></p:cond></p:stCondLst>",
            if end { "onEnd" } else { "onBegin" },
            step_ref(target)
        );
    } else {
        let delay = step
            .raw
            .pptx_delay
            .map(str::to_owned)
            .unwrap_or_else(|| step.delay_ms.to_string());
        let _ = write!(
            body,
            "<p:stCondLst><p:cond delay=\"{delay}\"/></p:stCondLst>"
        );
    }
    if step.repeat == Some(Repeat::UntilNextClick) {
        body.push_str("<p:endCondLst><p:cond evt=\"onNext\" delay=\"0\"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:endCondLst>");
    }
    if let Some(iterate) = step.iterate {
        let ty = match iterate.unit {
            Unit::Word => "wd",
            Unit::Letter => "lt",
            Unit::Paragraph => "el",
        };
        let back = if iterate.backwards {
            " backwards=\"1\""
        } else {
            ""
        };
        let interval = match iterate.interval {
            Interval::Ms(ms) => format!("<p:tmAbs val=\"{ms}\"/>"),
            Interval::Pct(pct) => format!("<p:tmPct val=\"{pct}\"/>"),
        };
        let _ = write!(
            body,
            "<p:iterate type=\"{ty}\"{back}>{interval}</p:iterate>"
        );
    }
    let tgt = target(step, spid);
    let dur = step
        .raw
        .pptx_dur
        .map(str::to_owned)
        .unwrap_or_else(|| step.dur_ms.to_string());
    let _ = write!(
        body,
        "<p:childTnLst>{}</p:childTnLst>",
        behaviours(ids, effect, &dur, step.dur_ms, &tgt)
    );
    let mut out = format!("<p:par><p:cTn{attrs}>{body}</p:cTn></p:par>");
    if let Some(after) = step.after {
        let after_id = ids.next();
        let start = step.delay_ms.saturating_add(step.span_ms());
        let tgt = target(step, spid);
        let children = match after {
            After::Hide => set(
                ids,
                "1",
                Some(0),
                &tgt,
                "style.visibility",
                &str_val("hidden"),
            ),
            After::Dim(color) => {
                let clr = format!("<p:clrVal><a:srgbClr val=\"{color}\"/></p:clrVal>");
                let mut both = set(ids, "1", Some(0), &tgt, "fillcolor", &clr);
                both.push_str(&set(ids, "1", Some(0), &tgt, "style.color", &clr));
                both
            }
        };
        let _ = write!(
            out,
            "<p:par><p:cTn id=\"{after_id}\" fill=\"hold\" afterEffect=\"1\"><p:stCondLst><p:cond delay=\"{start}\"/></p:stCondLst><p:childTnLst>{children}</p:childTnLst></p:cTn></p:par>"
        );
    }
    for wrapper in wrappers.iter().rev() {
        out = format!(
            "<p:par><p:cTn id=\"{wrapper}\" fill=\"hold\"><p:stCondLst><p:cond delay=\"0\"/></p:stCondLst><p:childTnLst>{out}</p:childTnLst></p:cTn></p:par>"
        );
    }
    out
}

/// `grpId` per step and the build list: paragraph and background steps of a shape share one build (`build="p"`);
/// every other step on the shape is its own build.
fn builds(slide: &Slide, spids: &[Option<u32>]) -> (Vec<u32>, String) {
    let mut grp = vec![0u32; slide.steps.len()];
    let mut list = String::new();
    for (shape_index, shape) in slide.shapes.iter().enumerate() {
        let Some(spid) = spids[shape_index] else {
            continue;
        };
        let steps: Vec<usize> = (0..slide.steps.len())
            .filter(|&i| slide.steps[i].shape == shape_index && slide.steps[i].formats.pptx)
            .collect();
        if steps.is_empty() {
            continue;
        }
        let text_build: Vec<usize> = steps
            .iter()
            .copied()
            .filter(|&i| slide.steps[i].paragraphs.is_some() || slide.steps[i].background)
            .collect();
        let mut next = 0;
        if !text_build.is_empty() {
            let anim_bg = text_build.iter().any(|&i| slide.steps[i].background);
            if !shape.media && !shape.line {
                let _ = write!(
                    list,
                    "<p:bldP spid=\"{spid}\" grpId=\"0\" build=\"p\"{}/>",
                    if anim_bg { " animBg=\"1\"" } else { "" }
                );
            }
            next = 1;
        }
        for &i in &steps {
            if text_build.contains(&i) {
                grp[i] = 0;
                continue;
            }
            grp[i] = next;
            if !shape.media && !shape.line {
                let _ = write!(
                    list,
                    "<p:bldP spid=\"{spid}\" grpId=\"{next}\" animBg=\"1\"/>"
                );
            }
            next += 1;
        }
    }
    (grp, list)
}

fn timing_xml(slide: &Slide, spids: &[Option<u32>]) -> String {
    let groups = layout(&slide.steps, Fmt::Pptx);
    let interactive: Vec<usize> = (0..slide.steps.len())
        .filter(|&i| slide.steps[i].interactive && slide.steps[i].formats.pptx)
        .collect();
    let media_shape = slide.shapes.iter().position(|s| s.media && s.formats.pptx);
    if groups.is_empty() && interactive.is_empty() && media_shape.is_none() {
        return String::new();
    }
    let (grp, build_list) = builds(slide, spids);
    let mut ids = Ids {
        last: 0,
        steps: Vec::new(),
    };
    let root = ids.next();
    let mut out = format!(
        "<p:timing><p:tnLst><p:par><p:cTn id=\"{root}\" dur=\"indefinite\" restart=\"never\" nodeType=\"tmRoot\"><p:childTnLst>"
    );
    if !groups.is_empty() {
        let main = ids.next();
        let _ = write!(
            out,
            "<p:seq concurrent=\"1\" nextAc=\"seek\"><p:cTn id=\"{main}\" dur=\"indefinite\" nodeType=\"mainSeq\"><p:childTnLst>"
        );
        for group in &groups {
            let gid = ids.next();
            let begin = if group.auto {
                format!(
                    "<p:cond delay=\"indefinite\"/><p:cond evt=\"onBegin\" delay=\"0\"><p:tn val=\"{main}\"/></p:cond>"
                )
            } else {
                "<p:cond delay=\"indefinite\"/>".to_owned()
            };
            let _ = write!(
                out,
                "<p:par><p:cTn id=\"{gid}\" fill=\"hold\"><p:stCondLst>{begin}</p:stCondLst><p:childTnLst>"
            );
            for (slot_index, slot) in group.slots.iter().enumerate() {
                let sid = ids.next();
                let _ = write!(
                    out,
                    "<p:par><p:cTn id=\"{sid}\" fill=\"hold\"><p:stCondLst><p:cond delay=\"{}\"/></p:stCondLst><p:childTnLst>",
                    slot.begin_ms
                );
                for (n, &index) in slot.steps.iter().enumerate() {
                    let step = &slide.steps[index];
                    let node_type = match step.trigger {
                        Trigger::Click => "clickEffect",
                        Trigger::With => "withEffect",
                        Trigger::After if n == 0 && slot_index > 0 => "afterEffect",
                        Trigger::After if group.auto && slot_index == 0 && n == 0 => "afterEffect",
                        Trigger::After => "withEffect",
                    };
                    let spid = spids[step.shape].unwrap_or(0);
                    out.push_str(&effect_par(
                        &mut ids, index, step, spid, grp[index], node_type,
                    ));
                }
                out.push_str("</p:childTnLst></p:cTn></p:par>");
            }
            out.push_str("</p:childTnLst></p:cTn></p:par>");
        }
        out.push_str("</p:childTnLst></p:cTn><p:prevCondLst><p:cond evt=\"onPrev\" delay=\"0\"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:prevCondLst><p:nextCondLst><p:cond evt=\"onNext\" delay=\"0\"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:nextCondLst></p:seq>");
    }
    if !interactive.is_empty() {
        let trigger = slide.trigger_shape.and_then(|i| spids[i]).unwrap_or(0);
        let seq = ids.next();
        let _ = write!(
            out,
            "<p:seq concurrent=\"1\" nextAc=\"seek\"><p:cTn id=\"{seq}\" restart=\"whenNotActive\" fill=\"hold\" evtFilter=\"cancelBubble\" nodeType=\"interactiveSeq\"><p:stCondLst><p:cond evt=\"onClick\" delay=\"0\"><p:tgtEl><p:spTgt spid=\"{trigger}\"/></p:tgtEl></p:cond></p:stCondLst><p:endSync evt=\"end\" delay=\"0\"><p:rtn val=\"all\"/></p:endSync><p:childTnLst>"
        );
        for &index in &interactive {
            let (g, s) = (ids.next(), ids.next());
            let _ = write!(
                out,
                "<p:par><p:cTn id=\"{g}\" fill=\"hold\"><p:stCondLst><p:cond delay=\"0\"/></p:stCondLst><p:childTnLst><p:par><p:cTn id=\"{s}\" fill=\"hold\"><p:stCondLst><p:cond delay=\"0\"/></p:stCondLst><p:childTnLst>"
            );
            let step = &slide.steps[index];
            out.push_str(&effect_par(
                &mut ids,
                index,
                step,
                spids[step.shape].unwrap_or(0),
                grp[index],
                "clickEffect",
            ));
            out.push_str("</p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par>");
        }
        let _ = write!(
            out,
            "</p:childTnLst></p:cTn><p:nextCondLst><p:cond evt=\"onClick\" delay=\"0\"><p:tgtEl><p:spTgt spid=\"{trigger}\"/></p:tgtEl></p:cond></p:nextCondLst></p:seq>"
        );
    }
    if let Some(shape) = media_shape {
        let id = ids.next();
        let _ = write!(
            out,
            "<p:video><p:cMediaNode vol=\"80000\"><p:cTn id=\"{id}\" fill=\"hold\" display=\"0\"><p:stCondLst><p:cond delay=\"indefinite\"/></p:stCondLst></p:cTn><p:tgtEl><p:spTgt spid=\"{}\"/></p:tgtEl></p:cMediaNode></p:video>",
            spids[shape].unwrap_or(0)
        );
    }
    out.push_str("</p:childTnLst></p:cTn></p:par></p:tnLst>");
    if !build_list.is_empty() {
        let _ = write!(out, "<p:bldLst>{build_list}</p:bldLst>");
    }
    out.push_str("</p:timing>");
    for (index, id) in &ids.steps {
        out = out.replace(&step_ref(*index), &id.to_string());
    }
    out
}
