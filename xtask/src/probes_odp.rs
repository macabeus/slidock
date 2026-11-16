//! Writes the ODP probe deck: an ODF 1.3 presentation whose pages carry the catalog's transitions (drawing-page
//! style `smil:*` and `presentation:transition-*`, plus the timing root's `anim:transitionFilter` duration) and
//! `anim:*` timing trees in the layout LibreOffice writes (timing root, main sequence, click groups, time slots,
//! one `anim:par` or `anim:iterate` per effect).

use std::fmt::Write as _;

use super::probes_catalog::{
    After, Bhv, Class, Fmt, Interval, OAdv, OUTLINE_COLOR, Repeat, Slide, Step, Trigger, Unit,
    layout,
};
use super::probes_pptx::{TITLE_BOX, background, escape, shape_box};
use super::probes_zip::Entry;

const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";
const NAMESPACES: &str = concat!(
    " xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"",
    " xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\"",
    " xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"",
    " xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\"",
    " xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\"",
    " xmlns:xlink=\"http://www.w3.org/1999/xlink\"",
    " xmlns:svg=\"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\"",
    " xmlns:presentation=\"urn:oasis:names:tc:opendocument:xmlns:presentation:1.0\"",
    " xmlns:smil=\"urn:oasis:names:tc:opendocument:xmlns:smil-compatible:1.0\"",
    " xmlns:anim=\"urn:oasis:names:tc:opendocument:xmlns:animation:1.0\""
);

pub fn write(slides: &[Slide], clip: &[u8], poster: &[u8]) -> Vec<Entry> {
    let included: Vec<&Slide> = slides.iter().filter(|s| s.formats.odp).collect();
    let media = included
        .iter()
        .any(|s| s.shapes.iter().any(|sh| sh.media && sh.formats.odp));
    let mut entries = vec![Entry {
        name: "mimetype".into(),
        data: b"application/vnd.oasis.opendocument.presentation".to_vec(),
        stored: true,
    }];
    let mut manifest = String::from(DECL);
    manifest.push_str("<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.3\">");
    manifest.push_str("<manifest:file-entry manifest:full-path=\"/\" manifest:version=\"1.3\" manifest:media-type=\"application/vnd.oasis.opendocument.presentation\"/>");
    manifest.push_str("<manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/>");
    manifest.push_str(
        "<manifest:file-entry manifest:full-path=\"styles.xml\" manifest:media-type=\"text/xml\"/>",
    );
    if media {
        manifest.push_str("<manifest:file-entry manifest:full-path=\"Media/clip.mp4\" manifest:media-type=\"video/mp4\"/>");
        manifest.push_str("<manifest:file-entry manifest:full-path=\"Pictures/poster.png\" manifest:media-type=\"image/png\"/>");
    }
    manifest.push_str("</manifest:manifest>");
    entries.push(Entry {
        name: "META-INF/manifest.xml".into(),
        data: manifest.into_bytes(),
        stored: false,
    });
    entries.push(Entry {
        name: "content.xml".into(),
        data: content(&included).into_bytes(),
        stored: false,
    });
    entries.push(Entry {
        name: "styles.xml".into(),
        data: styles().into_bytes(),
        stored: false,
    });
    if media {
        entries.push(Entry {
            name: "Media/clip.mp4".into(),
            data: clip.to_vec(),
            stored: true,
        });
        entries.push(Entry {
            name: "Pictures/poster.png".into(),
            data: poster.to_vec(),
            stored: true,
        });
    }
    entries
}

fn styles() -> String {
    format!(
        "{DECL}<office:document-styles{NAMESPACES} office:version=\"1.3\"><office:styles><draw:marker draw:name=\"Arrow\" svg:viewBox=\"0 0 20 30\" svg:d=\"M10 0l-10 30h20z\"/></office:styles><office:automatic-styles><style:page-layout style:name=\"PM1\"><style:page-layout-properties fo:margin-top=\"0pt\" fo:margin-bottom=\"0pt\" fo:margin-left=\"0pt\" fo:margin-right=\"0pt\" fo:page-width=\"960pt\" fo:page-height=\"540pt\" style:print-orientation=\"landscape\"/></style:page-layout><style:style style:name=\"Mdp1\" style:family=\"drawing-page\"><style:drawing-page-properties draw:fill=\"solid\" draw:fill-color=\"#FFFFFF\"/></style:style></office:automatic-styles><office:master-styles><style:master-page style:name=\"Probe\" style:page-layout-name=\"PM1\" draw:style-name=\"Mdp1\"/></office:master-styles></office:document-styles>"
    )
}

/// Seconds as ODF/SMIL writes them: `0.5s`, `1.25s`, `0s`.
pub fn secs(ms: u32) -> String {
    let whole = ms / 1000;
    let frac = ms % 1000;
    if frac == 0 {
        return format!("{whole}s");
    }
    let text = format!("{whole}.{frac:03}");
    format!("{}s", text.trim_end_matches('0'))
}

/// An ISO-8601 duration as `xsd:duration` (`anim:iterate-interval`): `PT0.05S`.
fn iso(ms: u32) -> String {
    let s = secs(ms);
    format!("PT{}S", s.trim_end_matches('s'))
}

/// The `xml:id` of a shape: `s<page>-sh<n>`, and of its paragraphs `s<page>-sh<n>-p<k>`.
pub fn shape_id(page: usize, shape: usize) -> String {
    format!("s{page}-sh{shape}")
}

fn content(slides: &[&Slide]) -> String {
    let mut styles = String::new();
    let mut pages = String::new();
    let mut colors: Vec<&str> = Vec::new();
    for slide in slides {
        for shape in &slide.shapes {
            if !colors.contains(&shape.color) {
                colors.push(shape.color);
            }
        }
    }
    for (n, color) in colors.iter().enumerate() {
        let _ = write!(
            styles,
            "<style:style style:name=\"gr{n}\" style:family=\"graphic\"><style:graphic-properties draw:fill=\"solid\" draw:fill-color=\"#{color}\" draw:stroke=\"none\" draw:textarea-vertical-align=\"middle\" draw:textarea-horizontal-align=\"center\" fo:padding-left=\"7pt\" fo:padding-right=\"7pt\"/></style:style>"
        );
    }
    for (n, color) in colors.iter().enumerate() {
        let _ = write!(
            styles,
            "<style:style style:name=\"gr{n}L\" style:family=\"graphic\"><style:graphic-properties draw:fill=\"solid\" draw:fill-color=\"#{color}\" draw:stroke=\"solid\" svg:stroke-color=\"#{OUTLINE_COLOR}\" svg:stroke-width=\"1.5pt\" draw:textarea-vertical-align=\"middle\" draw:textarea-horizontal-align=\"center\" fo:padding-left=\"7pt\" fo:padding-right=\"7pt\"/></style:style>"
        );
    }
    for (n, color) in colors.iter().enumerate() {
        if slides
            .iter()
            .any(|slide| slide.shapes.iter().any(|s| s.line && s.color == *color))
        {
            let _ = write!(
                styles,
                "<style:style style:name=\"gr{n}Line\" style:family=\"graphic\"><style:graphic-properties draw:fill=\"none\" draw:stroke=\"solid\" svg:stroke-color=\"#{color}\" svg:stroke-width=\"6pt\" draw:marker-end=\"Arrow\" draw:marker-end-width=\"30pt\"/></style:style>"
            );
        }
    }
    styles.push_str("<style:style style:name=\"grTitle\" style:family=\"graphic\"><style:graphic-properties draw:fill=\"none\" draw:stroke=\"none\" draw:textarea-vertical-align=\"middle\"/></style:style>");
    styles.push_str("<style:style style:name=\"grMedia\" style:family=\"graphic\"><style:graphic-properties draw:fill=\"none\" draw:stroke=\"none\"/></style:style>");
    styles.push_str("<style:style style:name=\"P1\" style:family=\"paragraph\"><style:paragraph-properties fo:text-align=\"center\"/><style:text-properties fo:color=\"#FFFFFF\" fo:font-family=\"'Liberation Sans'\" fo:font-size=\"12pt\"/></style:style>");
    styles.push_str("<style:style style:name=\"P2\" style:family=\"paragraph\"><style:paragraph-properties fo:text-align=\"center\"/><style:text-properties fo:color=\"#FFFFFF\" fo:font-family=\"'Liberation Sans'\" fo:font-size=\"20pt\"/></style:style>");
    styles.push_str("<style:style style:name=\"PT\" style:family=\"paragraph\"><style:text-properties fo:color=\"#1F2937\" fo:font-family=\"'Liberation Sans'\" fo:font-size=\"20pt\"/></style:style>");
    for (index, slide) in slides.iter().enumerate() {
        let page = index + 1;
        let _ = write!(
            styles,
            "<style:style style:name=\"dp{page}\" style:family=\"drawing-page\"><style:drawing-page-properties draw:fill=\"solid\" draw:fill-color=\"#{}\"{}/></style:style>",
            background(index),
            transition_attrs(slide)
        );
        pages.push_str(&page_xml(slide, page, &colors));
    }
    format!(
        "{DECL}<office:document-content{NAMESPACES} office:version=\"1.3\"><office:automatic-styles>{styles}</office:automatic-styles><office:body><office:presentation>{pages}</office:presentation></office:body></office:document-content>"
    )
}

fn transition_attrs(slide: &Slide) -> String {
    let Some(trans) = slide.transition.as_ref().and_then(|t| t.odp.as_ref()) else {
        return String::new();
    };
    let mut out = String::new();
    let (ty, duration) = match trans.advance {
        OAdv::Manual => ("manual", None),
        OAdv::Automatic(ms) => ("automatic", ms),
        OAdv::SemiAutomatic => ("semi-automatic", None),
    };
    let _ = write!(out, " presentation:transition-type=\"{ty}\"");
    match (&trans.smil, trans.legacy) {
        (Some((smil_type, subtype, reverse, color)), _) => {
            let _ = write!(out, " smil:type=\"{smil_type}\"");
            if !subtype.is_empty() {
                let _ = write!(out, " smil:subtype=\"{subtype}\"");
            }
            if *reverse {
                out.push_str(" smil:direction=\"reverse\"");
            }
            if let Some(color) = color {
                let _ = write!(out, " smil:fadeColor=\"{}\"", color.to_ascii_lowercase());
            }
        }
        (None, Some(style)) => {
            let _ = write!(out, " presentation:transition-style=\"{style}\"");
        }
        _ => {}
    }
    let _ = write!(out, " presentation:transition-speed=\"{}\"", trans.speed);
    if let Some(raw) = trans.duration_raw {
        let _ = write!(out, " presentation:duration=\"{raw}\"");
    } else if let Some(ms) = duration {
        let _ = write!(out, " presentation:duration=\"PT00H00M{:02}S\"", ms / 1000);
    }
    out
}

fn page_xml(slide: &Slide, page: usize, colors: &[&str]) -> String {
    let mut out = format!(
        "<draw:page draw:name=\"{}\" draw:style-name=\"dp{page}\" draw:master-page-name=\"Probe\" xml:id=\"s{page}\" draw:id=\"s{page}\">",
        escape(&slide.key)
    );
    let (x, y, w, h) = TITLE_BOX;
    let _ = write!(
        out,
        "<draw:frame draw:style-name=\"grTitle\" svg:x=\"{x}pt\" svg:y=\"{y}pt\" svg:width=\"{w}pt\" svg:height=\"{h}pt\"><draw:text-box><text:p text:style-name=\"PT\">{}</text:p></draw:text-box></draw:frame>",
        escape(&slide.title)
    );
    let shown: Vec<usize> = (0..slide.shapes.len())
        .filter(|&i| slide.shapes[i].formats.odp)
        .collect();
    let big = slide.transition.is_some() || slide.shapes.iter().any(|s| s.paras.len() > 1);
    for (position, &i) in shown.iter().enumerate() {
        let shape = &slide.shapes[i];
        let id = shape_id(page, i);
        let (x, y, w, h) = shape_box(position, shown.len(), big);
        let geometry =
            format!("svg:x=\"{x}pt\" svg:y=\"{y}pt\" svg:width=\"{w}pt\" svg:height=\"{h}pt\"");
        if shape.media {
            let _ = write!(
                out,
                "<draw:frame draw:style-name=\"grMedia\" {geometry} xml:id=\"{id}\" draw:id=\"{id}\"><draw:plugin xlink:href=\"Media/clip.mp4\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\" draw:mime-type=\"video/mp4\"><draw:param draw:name=\"Loop\" draw:value=\"false\"/><draw:param draw:name=\"Mute\" draw:value=\"false\"/></draw:plugin><draw:image xlink:href=\"Pictures/poster.png\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/><svg:title>{}</svg:title></draw:frame>",
                escape(&shape.label())
            );
            continue;
        }
        let style = colors.iter().position(|c| *c == shape.color).unwrap_or(0);
        if shape.line {
            let mid = y + h / 2;
            let _ = write!(
                out,
                "<draw:line draw:style-name=\"gr{style}Line\" svg:x1=\"{x}pt\" svg:y1=\"{mid}pt\" svg:x2=\"{}pt\" svg:y2=\"{mid}pt\" xml:id=\"{id}\" draw:id=\"{id}\"><svg:title>{}</svg:title></draw:line>",
                x + w,
                escape(&shape.label())
            );
            continue;
        }
        let outline = if slide.outlined(i) { "L" } else { "" };
        let para_style = if big { "P2" } else { "P1" };
        let _ = write!(
            out,
            "<draw:rect draw:style-name=\"gr{style}{outline}\" {geometry} xml:id=\"{id}\" draw:id=\"{id}\">"
        );
        out.push_str(&paragraphs(shape, &id, para_style));
        out.push_str("</draw:rect>");
    }
    out.push_str(&timing(slide, page));
    out.push_str("</draw:page>");
    out
}

/// The shape's paragraphs; a level-2 paragraph nests in a list under the paragraph before it.
fn paragraphs(shape: &super::probes_catalog::Shape, id: &str, style: &str) -> String {
    let p = |k: usize, text: &str| {
        format!(
            "<text:p text:style-name=\"{style}\" xml:id=\"{id}-p{k}\">{}</text:p>",
            escape(text)
        )
    };
    if shape.paras.iter().all(|para| para.level == 0) {
        return shape
            .paras
            .iter()
            .enumerate()
            .map(|(k, para)| p(k, &para.text))
            .collect();
    }
    let mut out = String::from("<text:list>");
    let mut k = 0;
    while k < shape.paras.len() {
        out.push_str("<text:list-item>");
        out.push_str(&p(k, &shape.paras[k].text));
        let mut child = k + 1;
        if child < shape.paras.len() && shape.paras[child].level > 0 {
            out.push_str("<text:list>");
            while child < shape.paras.len() && shape.paras[child].level > 0 {
                let _ = write!(
                    out,
                    "<text:list-item>{}</text:list-item>",
                    p(child, &shape.paras[child].text)
                );
                child += 1;
            }
            out.push_str("</text:list>");
        }
        out.push_str("</text:list-item>");
        k = child;
    }
    out.push_str("</text:list>");
    out
}

fn target_id(step: &Step, page: usize) -> String {
    let id = shape_id(page, step.shape);
    match step.paragraphs {
        Some((first, _)) => format!("{id}-p{first}"),
        None => id,
    }
}

/// The attribute name of a colour: `fill`, `line` or the text's.
fn color_attr(property: &str) -> &'static str {
    match property {
        "fill" => "fill-color",
        "line" => "stroke-color",
        _ => "color",
    }
}

/// One leaf's common attributes: begin, duration (clock values), fill, target (left out inside an `anim:iterate`).
fn leaf(begin: Option<&str>, dur: Option<&str>, target: Option<&str>, sub_item: bool) -> String {
    let mut out = String::new();
    if let Some(begin) = begin {
        let _ = write!(out, " smil:begin=\"{begin}\"");
    }
    if let Some(dur) = dur {
        let _ = write!(out, " smil:dur=\"{dur}\"");
    }
    out.push_str(" smil:fill=\"hold\"");
    if let Some(target) = target {
        let _ = write!(out, " smil:targetElement=\"{target}\"");
    }
    if sub_item {
        out.push_str(" anim:sub-item=\"background\"");
    }
    out
}

fn leaves(step: &Step, target: Option<&str>) -> String {
    let effect = &step.effect;
    let dur = step.dur_ms;
    let bg = step.background;
    let entering = matches!(effect.class, Class::Entr | Class::Custom);
    let exiting = effect.class == Class::Exit;
    let mut out = String::new();
    let dur_text = step
        .raw
        .odp_dur
        .map(str::to_owned)
        .unwrap_or_else(|| secs(dur));
    if entering {
        let shown = if effect.bhv.iter().any(|b| matches!(b, Bhv::ShowWhile)) {
            // Flash Once: the switch lasts the step and does not hold.
            leaf(Some("0s"), Some(&dur_text), target, bg).replacen(" smil:fill=\"hold\"", "", 1)
        } else {
            leaf(Some("0s"), Some("0.001s"), target, bg)
        };
        let _ = write!(
            out,
            "<anim:set{shown} smil:attributeName=\"visibility\" smil:to=\"visible\"/>"
        );
    }
    let common = leaf(None, Some(&dur_text), target, bg);
    // A behaviour that reverses on its own over half the step (`smil:autoReverse`), back at rest by its end.
    let reversing = format!(
        "{} smil:autoReverse=\"true\"",
        leaf(None, Some(&secs(dur / 2)), target, bg)
    );
    for bhv in &effect.bhv {
        match bhv {
            Bhv::ShowWhile => {}
            Bhv::Pulse(x, y) => {
                let sx = f64::from(*x) / 100_000.0;
                let sy = f64::from(*y) / 100_000.0;
                let _ = write!(
                    out,
                    "<anim:animateTransform{reversing} smil:attributeName=\"transform\" smil:by=\"{sx},{sy}\" svg:type=\"scale\"/>"
                );
            }
            Bhv::ColorFlash(property, value) => {
                let _ = write!(
                    out,
                    "<anim:animateColor{reversing} smil:attributeName=\"{}\" smil:to=\"#{}\" anim:color-interpolation=\"rgb\"/>",
                    color_attr(property),
                    value.to_ascii_lowercase()
                );
            }
            Bhv::Filter(clip) | Bhv::FilterOver(clip, _) => {
                let _ = write!(
                    out,
                    "<anim:transitionFilter{common} smil:type=\"{}\"",
                    clip.ty
                );
                if let Bhv::FilterOver(_, time_filter) = bhv {
                    let _ = write!(out, " smil:keySplines=\"{time_filter}\"");
                }
                if !clip.subtype.is_empty() {
                    let _ = write!(out, " smil:subtype=\"{}\"", clip.subtype);
                }
                if clip.reverse {
                    out.push_str(" smil:direction=\"reverse\"");
                }
                if clip.out {
                    out.push_str(" smil:mode=\"out\"");
                }
                out.push_str("/>");
            }
            Bhv::Move { odp, .. } => {
                let _ = write!(
                    out,
                    "<anim:animate{common} smil:attributeName=\"x\" smil:values=\"{};{}\" smil:keyTimes=\"0;1\"/>",
                    odp[0], odp[2]
                );
                let _ = write!(
                    out,
                    "<anim:animate{common} smil:attributeName=\"y\" smil:values=\"{};{}\" smil:keyTimes=\"0;1\"/>",
                    odp[1], odp[3]
                );
            }
            Bhv::GrowFromZero => {
                let (w, h) = if exiting {
                    ("width;0", "height;0")
                } else {
                    ("0;width", "0;height")
                };
                let _ = write!(
                    out,
                    "<anim:animate{common} smil:attributeName=\"width\" smil:values=\"{w}\" smil:keyTimes=\"0;1\"/>"
                );
                let _ = write!(
                    out,
                    "<anim:animate{common} smil:attributeName=\"height\" smil:values=\"{h}\" smil:keyTimes=\"0;1\"/>"
                );
            }
            Bhv::Scale(x, y) => {
                let sx = f64::from(*x) / 100_000.0;
                let sy = f64::from(*y) / 100_000.0;
                let _ = write!(
                    out,
                    "<anim:animateTransform{common} smil:attributeName=\"transform\" smil:by=\"{sx},{sy}\" svg:type=\"scale\"/>"
                );
            }
            Bhv::Rotate(deg) => {
                let _ = write!(
                    out,
                    "<anim:animateTransform{common} smil:attributeName=\"transform\" smil:by=\"{deg}\" svg:type=\"rotate\"/>"
                );
            }
            Bhv::Opacity(value) => {
                let _ = write!(
                    out,
                    "<anim:set{common} smil:attributeName=\"opacity\" smil:to=\"{value}\"/>"
                );
            }
            Bhv::Color(property, value) => {
                let _ = write!(
                    out,
                    "<anim:animateColor{common} smil:attributeName=\"{}\" smil:to=\"#{}\" anim:color-interpolation=\"rgb\"/>",
                    color_attr(property),
                    value.to_ascii_lowercase()
                );
            }
            Bhv::ColorBy(property, h, sat, light) => {
                let _ = write!(
                    out,
                    "<anim:animateColor{common} smil:attributeName=\"{}\" smil:by=\"hsl({h},{sat}%,{light}%)\" anim:color-interpolation=\"hsl\" anim:color-interpolation-direction=\"clockwise\"/>",
                    color_attr(property)
                );
            }
            Bhv::SetColor(property, value, reverse) => {
                let attrs = if *reverse { &reversing } else { &common };
                let _ = write!(
                    out,
                    "<anim:set{attrs} smil:attributeName=\"{}\" smil:to=\"#{}\"/>",
                    color_attr(property),
                    value.to_ascii_lowercase()
                );
            }
            Bhv::PaintOn(property) => {
                let attr = if *property == "line" {
                    "stroke"
                } else {
                    "fill"
                };
                let _ = write!(
                    out,
                    "<anim:set{common} smil:attributeName=\"{attr}\" smil:to=\"solid\"/>"
                );
            }
            Bhv::Font(switch, on) => {
                let (attr, value) = match (*switch, *on) {
                    ("bold", on) => ("font-weight", if on { "bold" } else { "normal" }),
                    ("italic", on) => ("font-style", if on { "italic" } else { "normal" }),
                    (_, on) => ("text-underline", if on { "solid" } else { "none" }),
                };
                let _ = write!(
                    out,
                    "<anim:set{common} smil:attributeName=\"{attr}\" smil:to=\"{value}\"/>"
                );
            }
            Bhv::BoldPulse => {
                let _ = write!(
                    out,
                    "<anim:animate{common} smil:attributeName=\"font-weight\" smil:values=\"normal;bold;normal;normal\" smil:keyTimes=\"0;0.5;0.6;1\" smil:calcMode=\"discrete\"/>"
                );
            }
            Bhv::FontFamily(family) => {
                let _ = write!(
                    out,
                    "<anim:set{common} smil:attributeName=\"font-family\" smil:to=\"{}\"/>",
                    escape(family)
                );
            }
            Bhv::FontSize => {
                let _ = write!(
                    out,
                    "<anim:animate{common} smil:attributeName=\"font-size\" smil:to=\"1.5pt\"/>"
                );
            }
            Bhv::Motion(path) => {
                let _ = write!(
                    out,
                    "<anim:animateMotion{common} smil:attributeName=\"xy\" svg:path=\"{path}\"/>"
                );
            }
            Bhv::Command(_, command) => {
                let target_attr = target
                    .map(|t| format!(" smil:targetElement=\"{t}\""))
                    .unwrap_or_default();
                let _ = write!(
                    out,
                    "<anim:command smil:begin=\"0s\" smil:dur=\"0.001\"{target_attr} anim:command=\"{command}\"/>"
                );
            }
            Bhv::Unknown => {
                let _ = write!(
                    out,
                    "<anim:set{common} smil:attributeName=\"xshear\" smil:to=\"0.3\"/>"
                );
            }
            Bhv::Formula { odp, .. } => {
                let _ = write!(
                    out,
                    "<anim:animate{common} smil:attributeName=\"x\" smil:values=\"x;x\" smil:keyTimes=\"0;1\" anim:formula=\"{}\"/>",
                    escape(odp)
                );
            }
            Bhv::DeepFormula(depth) => {
                let formula = format!("{}$/2{}", "(".repeat(*depth), ")".repeat(*depth));
                let _ = write!(
                    out,
                    "<anim:animate{common} smil:attributeName=\"y\" smil:values=\"y;y\" smil:keyTimes=\"0;1\" anim:formula=\"{formula}\"/>"
                );
            }
        }
    }
    if exiting {
        let begin = secs(dur.saturating_sub(1));
        let _ = write!(
            out,
            "<anim:set{} smil:attributeName=\"visibility\" smil:to=\"hidden\"/>",
            leaf(Some(&begin), Some("0.001s"), target, bg)
        );
    }
    out
}

fn effect_node(step: &Step, index: usize, page: usize, group_id: usize, node_type: &str) -> String {
    let effect = &step.effect;
    let own = format!("s{page}-e{index}");
    let mut attrs = String::new();
    if let Some((other, end)) = step.begin_on {
        let event = if end { "end" } else { "begin" };
        let _ = write!(attrs, " smil:begin=\"s{page}-e{other}.{event}\"");
    } else {
        let begin = step
            .raw
            .odp_delay
            .map(str::to_owned)
            .unwrap_or_else(|| secs(step.delay_ms));
        let _ = write!(attrs, " smil:begin=\"{begin}\"");
    }
    attrs.push_str(" smil:fill=\"hold\"");
    match step.repeat {
        Some(Repeat::Count(count)) => {
            let _ = write!(attrs, " smil:repeatCount=\"{}\"", count / 1000);
        }
        Some(Repeat::UntilEndOfSlide) => attrs.push_str(" smil:repeatCount=\"indefinite\""),
        Some(Repeat::UntilNextClick) => {
            attrs.push_str(" smil:repeatCount=\"indefinite\" smil:end=\"next\"")
        }
        None => {}
    }
    if step.auto_rev {
        attrs.push_str(" smil:autoReverse=\"true\"");
    }
    if let Some(accel) = step.accel {
        let _ = write!(
            attrs,
            " smil:accelerate=\"{}\"",
            f64::from(accel) / 100_000.0
        );
    }
    if let Some(decel) = step.decel {
        let _ = write!(
            attrs,
            " smil:decelerate=\"{}\"",
            f64::from(decel) / 100_000.0
        );
    }
    let _ = write!(
        attrs,
        " presentation:node-type=\"{node_type}\" presentation:preset-class=\"{}\"",
        effect.class.odp()
    );
    if let Some(ooo) = &effect.ooo {
        let _ = write!(attrs, " presentation:preset-id=\"{ooo}\"");
    }
    if let Some(sub) = &effect.sub {
        let _ = write!(attrs, " presentation:preset-sub-type=\"{sub}\"");
    }
    let _ = write!(
        attrs,
        " presentation:group-id=\"{group_id}\" xml:id=\"{own}\""
    );
    let target = target_id(step, page);
    let mut out = match step.iterate {
        Some(iterate) => {
            let ty = match iterate.unit {
                Unit::Word => "by-word",
                Unit::Letter => "by-letter",
                Unit::Paragraph => "by-paragraph",
            };
            let interval = match iterate.interval {
                Interval::Ms(ms) => iso(ms),
                Interval::Pct(_) => iso(0),
            };
            format!(
                "<anim:iterate{attrs} smil:targetElement=\"{target}\" anim:iterate-type=\"{ty}\" anim:iterate-interval=\"{interval}\">{}</anim:iterate>",
                leaves(step, None)
            )
        }
        None => format!(
            "<anim:par{attrs}>{}</anim:par>",
            leaves(step, Some(&target))
        ),
    };
    if let Some(after) = step.after {
        let begin = step.delay_ms.saturating_add(step.span_ms());
        let set = match after {
            After::Hide => format!(
                "<anim:set{} smil:attributeName=\"visibility\" smil:to=\"hidden\"/>",
                leaf(Some("0s"), Some("0.001s"), Some(&target), false)
            ),
            After::Dim(color) => format!(
                "<anim:set{} smil:attributeName=\"dim\" smil:to=\"#{}\"/>",
                leaf(Some("0s"), Some("0.001s"), Some(&target), false),
                color.to_ascii_lowercase()
            ),
        };
        let _ = write!(
            out,
            "<anim:par smil:begin=\"{}\" smil:fill=\"hold\" presentation:master-element=\"{own}\">{set}</anim:par>",
            secs(begin)
        );
    }
    for _ in 0..step.nest {
        out = format!("<anim:par smil:begin=\"0s\">{out}</anim:par>");
    }
    out
}

/// `presentation:group-id` per step: paragraph and background steps of a shape share one id.
fn group_ids(slide: &Slide) -> Vec<usize> {
    let mut ids = vec![0; slide.steps.len()];
    let mut next = 1;
    let mut text_group: Vec<(usize, usize)> = Vec::new();
    for (index, step) in slide.steps.iter().enumerate() {
        if step.paragraphs.is_some() || step.background {
            if let Some(&(_, id)) = text_group.iter().find(|(shape, _)| *shape == step.shape) {
                ids[index] = id;
                continue;
            }
            text_group.push((step.shape, next));
        }
        ids[index] = next;
        next += 1;
    }
    ids
}

fn timing(slide: &Slide, page: usize) -> String {
    let groups = layout(&slide.steps, Fmt::Odp);
    let interactive: Vec<usize> = (0..slide.steps.len())
        .filter(|&i| slide.steps[i].interactive && slide.steps[i].formats.odp)
        .collect();
    let filter = slide
        .transition
        .as_ref()
        .and_then(|t| t.odp.as_ref())
        .and_then(|t| Some((t.smil.as_ref()?, t.filter_dur?)));
    if groups.is_empty() && interactive.is_empty() && filter.is_none() {
        return String::new();
    }
    let ids = group_ids(slide);
    let mut out = String::from("<anim:par presentation:node-type=\"timing-root\">");
    if let Some(((ty, subtype, reverse, color), dur)) = filter {
        let _ = write!(
            out,
            "<anim:par smil:begin=\"s{page}.begin\"><anim:transitionFilter smil:type=\"{ty}\""
        );
        if !subtype.is_empty() {
            let _ = write!(out, " smil:subtype=\"{subtype}\"");
        }
        if *reverse {
            out.push_str(" smil:direction=\"reverse\"");
        }
        if let Some(color) = color {
            let _ = write!(out, " smil:fadeColor=\"{}\"", color.to_ascii_lowercase());
        }
        let _ = write!(out, " smil:dur=\"{}\"/></anim:par>", secs(dur));
    }
    if !groups.is_empty() {
        out.push_str("<anim:seq presentation:node-type=\"main-sequence\">");
        for group in &groups {
            let _ = write!(
                out,
                "<anim:par smil:begin=\"{}\">",
                if group.auto { "0s" } else { "next" }
            );
            for slot in &group.slots {
                let _ = write!(out, "<anim:par smil:begin=\"{}\">", secs(slot.begin_ms));
                for &index in &slot.steps {
                    let step = &slide.steps[index];
                    let node_type = match step.trigger {
                        Trigger::Click => "on-click",
                        Trigger::With => "with-previous",
                        Trigger::After => "after-previous",
                    };
                    out.push_str(&effect_node(step, index, page, ids[index], node_type));
                }
                out.push_str("</anim:par>");
            }
            out.push_str("</anim:par>");
        }
        out.push_str("</anim:seq>");
    }
    if !interactive.is_empty() {
        let trigger = slide
            .trigger_shape
            .map(|i| shape_id(page, i))
            .unwrap_or_default();
        let _ = write!(
            out,
            "<anim:seq presentation:node-type=\"interactive-sequence\" smil:begin=\"{trigger}.click\" smil:end=\"indefinite\">"
        );
        for &index in &interactive {
            out.push_str("<anim:par smil:begin=\"0s\"><anim:par smil:begin=\"0s\">");
            out.push_str(&effect_node(
                &slide.steps[index],
                index,
                page,
                ids[index],
                "on-click",
            ));
            out.push_str("</anim:par></anim:par>");
        }
        out.push_str("</anim:seq>");
    }
    out.push_str("</anim:par>");
    out
}
