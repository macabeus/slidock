//! Synthetic PPTX presentations for the frontend's tests, each exercising one area. They are built from XML
//! snippets, so every feature they hold is visible here.

use super::{
    LayoutSpec, MasterSpec, PptxSpec, Row, SlideSpec, Sp, TableSpec, body, external, pptx, rel, sp,
    table, tc, tc1, xfrm,
};

const TITLE_STYLE: &str = concat!(
    "<a:lvl1pPr algn=\"ctr\"><a:defRPr sz=\"4400\" b=\"1\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill>",
    "<a:latin typeface=\"+mj-lt\"/></a:defRPr></a:lvl1pPr>"
);

const BODY_STYLE: &str = concat!(
    "<a:defPPr><a:lnSpc><a:spcPct val=\"90000\"/></a:lnSpc></a:defPPr>",
    "<a:lvl1pPr marL=\"228600\" indent=\"-228600\"><a:spcBef><a:spcPts val=\"1000\"/></a:spcBef>",
    "<a:buFont typeface=\"Arial\"/><a:buChar char=\"•\"/><a:defRPr sz=\"2800\"><a:solidFill><a:schemeClr val=\"tx1\"/>",
    "</a:solidFill><a:latin typeface=\"+mn-lt\"/></a:defRPr></a:lvl1pPr>",
    "<a:lvl2pPr marL=\"685800\" indent=\"-228600\"><a:buFont typeface=\"Arial\"/><a:buChar char=\"–\"/>",
    "<a:defRPr sz=\"2400\"/></a:lvl2pPr>"
);

const OTHER_STYLE: &str = "<a:lvl1pPr><a:defRPr sz=\"1200\"/></a:lvl1pPr>";

fn tx_styles() -> String {
    format!(
        "<p:titleStyle>{TITLE_STYLE}</p:titleStyle><p:bodyStyle>{BODY_STYLE}</p:bodyStyle><p:otherStyle>{OTHER_STYLE}</p:otherStyle>"
    )
}

fn title(id: u64, sp_pr: &str) -> String {
    sp(Sp {
        id,
        name: Some("Title".to_owned()),
        ph: "<p:ph type=\"title\"/>".to_owned(),
        sp_pr: sp_pr.to_owned(),
        tx_body: Some(body(&[""], "<a:bodyPr/>")),
        ..Sp::default()
    })
}

fn content(id: u64, sp_pr: &str) -> String {
    sp(Sp {
        id,
        name: Some("Content".to_owned()),
        ph: "<p:ph idx=\"1\"/>".to_owned(),
        sp_pr: sp_pr.to_owned(),
        tx_body: Some(body(&[""], "<a:bodyPr/>")),
        ..Sp::default()
    })
}

fn slide_number(id: u64) -> String {
    sp(Sp {
        id,
        name: Some("Slide Number".to_owned()),
        ph: "<p:ph type=\"sldNum\" sz=\"quarter\" idx=\"12\"/>".to_owned(),
        sp_pr: xfrm(620.0, 380.0, 80.0, 20.0, ""),
        tx_body: Some(
            concat!(
                "<a:bodyPr/><a:lstStyle/><a:p><a:fld id=\"{B6F15528-21DE-4FAA-801E-634DDDAF4B2B}\" type=\"slidenum\">",
                "<a:rPr lang=\"en-US\"/><a:t>‹#›</a:t></a:fld></a:p>"
            )
            .to_owned(),
        ),
        ..Sp::default()
    })
}

/// Masters, layouts, themes, colour maps, text styles and defaults, backgrounds, order and hidden slides.
pub fn basic() -> Vec<u8> {
    pptx(PptxSpec {
        default_text_style: Some(
            concat!(
                "<a:defPPr><a:defRPr lang=\"en-US\"/></a:defPPr><a:lvl1pPr marL=\"0\" algn=\"l\" defTabSz=\"457200\">",
                "<a:defRPr sz=\"1800\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"+mn-lt\"/>",
                "</a:defRPr></a:lvl1pPr>"
            )
            .to_owned(),
        ),
        embedded_fonts: Some(
            "<p:embeddedFont><p:font typeface=\"Roboto\"/><p:regular r:id=\"rIdF1\"/><p:bold r:id=\"rIdF2\"/></p:embeddedFont>"
                .to_owned(),
        ),
        masters: Some(vec![MasterSpec {
            bg: Some("<p:bgPr><a:solidFill><a:schemeClr val=\"bg1\"/></a:solidFill><a:effectLst/></p:bgPr>".to_owned()),
            tx_styles: Some(tx_styles()),
            tree: title(2, &xfrm(36.0, 20.0, 648.0, 60.0, ""))
                + &content(3, &xfrm(36.0, 100.0, 648.0, 260.0, ""))
                + &slide_number(4),
            layouts: vec![
                LayoutSpec {
                    kind: Some("title".to_owned()),
                    name: Some("Title Slide".to_owned()),
                    tree: title(2, &xfrm(54.0, 120.0, 612.0, 100.0, "")),
                    ..LayoutSpec::default()
                },
                LayoutSpec {
                    kind: Some("obj".to_owned()),
                    name: Some("Title and Content".to_owned()),
                    show_master_sp: Some(false),
                    clr_map_ovr: Some(
                        concat!(
                            "<a:overrideClrMapping bg1=\"dk1\" tx1=\"lt1\" bg2=\"dk2\" tx2=\"lt2\" accent1=\"accent1\" accent2=\"accent2\" ",
                            "accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>"
                        )
                        .to_owned(),
                    ),
                    tree: title(2, "") + &content(3, ""),
                    ..LayoutSpec::default()
                },
            ],
            ..MasterSpec::default()
        }]),
        // Parts are numbered in another order than the show's.
        order: Some(vec![0, 2, 1]),
        slides: vec![
            SlideSpec {
                id: Some(256),
                layout: Some(0),
                tree: title(2, ""),
                ..SlideSpec::default()
            },
            SlideSpec {
                id: Some(300),
                layout: Some(1),
                show: Some(false),
                tree: title(2, "") + &content(3, ""),
                ..SlideSpec::default()
            },
            SlideSpec {
                id: Some(257),
                layout: Some(1),
                show_master_sp: Some(false),
                bg: Some(
                    concat!(
                        "<p:bgPr><a:gradFill rotWithShape=\"1\"><a:gsLst><a:gs pos=\"0\"><a:srgbClr val=\"FF0000\"/></a:gs>",
                        "<a:gs pos=\"100000\"><a:schemeClr val=\"accent1\"><a:lumMod val=\"75000\"/></a:schemeClr></a:gs></a:gsLst>",
                        "<a:lin ang=\"2700000\" scaled=\"1\"/></a:gradFill><a:effectLst/></p:bgPr>"
                    )
                    .to_owned(),
                ),
                clr_map_ovr: Some("<a:masterClrMapping/>".to_owned()),
                tree: title(2, ""),
                ..SlideSpec::default()
            },
        ],
        ..PptxSpec::default()
    })
}

/// A small opaque PNG, red and blue pixels alternating: 4 × 2 or 8 × 4, as fflate's zlib writes it at level 6 with the
/// Sub filter, so the goldens' media hashes stay fixed.
pub fn png(width: u32, height: u32) -> Vec<u8> {
    let hex = match (width, height) {
        (4, 2) => concat!(
            "89504e470d0a1a0a0000000d49484452000000040000000208060000007fa87d630000001849444154789c63fccfc0f09f91e13f",
            "103202219044170000b6fd09ff9d7d080b0000000049454e44ae426082"
        ),
        (8, 4) => concat!(
            "89504e470d0a1a0a0000000d4948445200000008000000040806000000b3cd7ef00000001849444154789c63fccfc0f09f91e13f",
            "10320221264d07050089dd23fd9f3fa4590000000049454e44ae426082"
        ),
        _ => panic!("no {width} × {height} fixture PNG"),
    };
    (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
        .collect()
}

fn solid(hex: &str) -> String {
    format!("<a:solidFill><a:srgbClr val=\"{hex}\"/></a:solidFill>")
}

const RECT: &str = "<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>";
const NO_LINE: &str = "<a:ln><a:noFill/></a:ln>";

fn frame(id: u64, name: &str, uri: &str, data: &str, at: &str) -> String {
    format!(
        "<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"{id}\" name=\"{name}\"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>{}<a:graphic><a:graphicData uri=\"{uri}\">{data}</a:graphicData></a:graphic></p:graphicFrame>",
        at.replace("a:xfrm", "p:xfrm")
    )
}

fn connector() -> String {
    format!(
        concat!(
            "<p:cxnSp><p:nvCxnSpPr><p:cNvPr id=\"30\" name=\"Connector\"/><p:cNvCxnSpPr><a:stCxn id=\"10\" idx=\"3\"/><a:endCxn id=\"11\" idx=\"1\"/>",
            "</p:cNvCxnSpPr><p:nvPr/></p:nvCxnSpPr><p:spPr>{}",
            "<a:prstGeom prst=\"straightConnector1\"><a:avLst/></a:prstGeom><a:noFill/><a:ln w=\"38100\" cap=\"flat\">",
            "<a:solidFill><a:schemeClr val=\"accent2\"/></a:solidFill><a:prstDash val=\"solid\"/><a:round/>",
            "<a:headEnd type=\"oval\" w=\"sm\" len=\"sm\"/><a:tailEnd type=\"triangle\" w=\"med\" len=\"lg\"/></a:ln></p:spPr></p:cxnSp>"
        ),
        xfrm(100.0, 200.0, 150.0, 0.0, " rot=\"10800000\"")
    )
}

fn group() -> String {
    let inner = xfrm(200.0, 100.0, 100.0, 50.0, "").replace(
        "</a:xfrm>",
        "<a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"1270000\" cy=\"635000\"/></a:xfrm>",
    );
    format!(
        concat!(
            "<p:grpSp><p:nvGrpSpPr><p:cNvPr id=\"40\" name=\"Group\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr>",
            "<a:xfrm rot=\"1200000\"><a:off x=\"1270000\" y=\"2540000\"/><a:ext cx=\"2540000\" cy=\"1270000\"/>",
            "<a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"5080000\" cy=\"2540000\"/></a:xfrm>",
            "{}</p:grpSpPr>{}",
            "<p:grpSp><p:nvGrpSpPr><p:cNvPr id=\"42\" name=\"Inner\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr>",
            "{}</p:grpSpPr>{}</p:grpSp></p:grpSp>"
        ),
        solid("00FF00"),
        sp(Sp {
            id: 41,
            sp_pr: format!("{}{RECT}<a:grpFill/>", xfrm(0.0, 0.0, 200.0, 100.0, "")),
            ..Sp::default()
        }),
        inner,
        sp(Sp {
            id: 43,
            sp_pr: format!(
                "{}{RECT}<a:grpFill/>",
                xfrm(0.0, 0.0, 50.0, 25.0, " flipV=\"1\"")
            ),
            ..Sp::default()
        }),
    )
}

/// Geometry, fills, lines, effects, style references, groups, connectors, graphic frames and alternate content.
pub fn shapes() -> Vec<u8> {
    let tree = [
        sp(Sp {
            id: 10,
            name: Some("Rotated".to_owned()),
            sp_pr: format!(
                concat!(
                    "{}{}{}",
                    "<a:ln w=\"38100\" cap=\"rnd\"><a:solidFill><a:srgbClr val=\"000000\"><a:alpha val=\"50000\"/></a:srgbClr></a:solidFill>",
                    "<a:prstDash val=\"dot\"/><a:bevel/></a:ln>"
                ),
                xfrm(20.0, 20.0, 100.0, 50.0, " rot=\"5400000\" flipH=\"1\""),
                RECT,
                solid("FF8000")
            ),
            ..Sp::default()
        }),
        sp(Sp {
            id: 11,
            name: Some("Rounded".to_owned()),
            sp_pr: format!(
                concat!(
                    "{}<a:prstGeom prst=\"roundRect\"><a:avLst><a:gd name=\"adj\" fmla=\"val 30000\"/></a:avLst></a:prstGeom>",
                    "<a:gradFill rotWithShape=\"0\"><a:gsLst><a:gs pos=\"100000\"><a:schemeClr val=\"accent1\"/></a:gs>",
                    "<a:gs pos=\"0\"><a:schemeClr val=\"accent1\"><a:lumMod val=\"50000\"/><a:lumOff val=\"50000\"/></a:schemeClr></a:gs>",
                    "</a:gsLst><a:lin ang=\"5400000\" scaled=\"1\"/></a:gradFill>{}"
                ),
                xfrm(140.0, 20.0, 100.0, 50.0, ""),
                NO_LINE
            ),
            ..Sp::default()
        }),
        sp(Sp {
            id: 12,
            name: Some("Radial".to_owned()),
            sp_pr: format!(
                concat!(
                    "{}<a:prstGeom prst=\"ellipse\"><a:avLst/></a:prstGeom><a:gradFill><a:gsLst>",
                    "<a:gs pos=\"0\"><a:srgbClr val=\"FFFFFF\"/></a:gs><a:gs pos=\"100000\"><a:srgbClr val=\"000000\"/></a:gs></a:gsLst>",
                    "<a:path path=\"rect\"><a:fillToRect l=\"50000\" t=\"25000\" r=\"50000\" b=\"75000\"/></a:path></a:gradFill>"
                ),
                xfrm(260.0, 20.0, 80.0, 80.0, "")
            ),
            ..Sp::default()
        }),
        sp(Sp {
            id: 13,
            name: Some("Pattern and shadow".to_owned()),
            sp_pr: format!(
                concat!(
                    "{}{}<a:pattFill prst=\"dkDnDiag\"><a:fgClr><a:srgbClr val=\"C00000\"/></a:fgClr>",
                    "<a:bgClr><a:srgbClr val=\"FFFFFF\"/></a:bgClr></a:pattFill><a:effectLst><a:outerShdw blurRad=\"40000\" dist=\"20000\">",
                    "<a:srgbClr val=\"000000\"/></a:outerShdw><a:glow rad=\"10000\"><a:srgbClr val=\"FFFF00\"/></a:glow></a:effectLst>"
                ),
                xfrm(360.0, 20.0, 80.0, 40.0, ""),
                RECT
            ),
            ..Sp::default()
        }),
        sp(Sp {
            id: 14,
            name: Some("Styled".to_owned()),
            sp_pr: format!("{}{RECT}", xfrm(460.0, 20.0, 80.0, 40.0, "")),
            style: Some(
                concat!(
                    "<a:lnRef idx=\"2\"><a:schemeClr val=\"accent1\"><a:shade val=\"50000\"/></a:schemeClr></a:lnRef>",
                    "<a:fillRef idx=\"2\"><a:schemeClr val=\"accent2\"/></a:fillRef><a:effectRef idx=\"3\"><a:schemeClr val=\"accent1\"/></a:effectRef>",
                    "<a:fontRef idx=\"minor\"><a:schemeClr val=\"lt1\"/></a:fontRef>"
                )
                .to_owned(),
            ),
            ..Sp::default()
        }),
        sp(Sp {
            id: 15,
            name: Some("Custom".to_owned()),
            sp_pr: format!(
                concat!(
                    "{}<a:custGeom><a:avLst/><a:gdLst><a:gd name=\"half\" fmla=\"*/ w 1 2\"/></a:gdLst>",
                    "<a:pathLst><a:path w=\"1270000\" h=\"1270000\"><a:moveTo><a:pt x=\"0\" y=\"0\"/></a:moveTo><a:lnTo><a:pt x=\"half\" y=\"0\"/></a:lnTo>",
                    "<a:arcTo wR=\"635000\" hR=\"635000\" stAng=\"16200000\" swAng=\"5400000\"/><a:quadBezTo><a:pt x=\"0\" y=\"1270000\"/><a:pt x=\"0\" y=\"635000\"/></a:quadBezTo>",
                    "<a:close/></a:path><a:path fill=\"none\" stroke=\"0\"><a:moveTo><a:pt x=\"0\" y=\"0\"/></a:moveTo>",
                    "<a:cubicBezTo><a:pt x=\"10\" y=\"10\"/><a:pt x=\"20\" y=\"20\"/><a:pt x=\"w\" y=\"h\"/></a:cubicBezTo></a:path></a:pathLst></a:custGeom>{}"
                ),
                xfrm(20.0, 100.0, 100.0, 100.0, ""),
                solid("336699")
            ),
            ..Sp::default()
        }),
        sp(Sp {
            id: 16,
            name: Some("Unknown preset".to_owned()),
            sp_pr: format!(
                "{}<a:prstGeom prst=\"fancyStar\"><a:avLst/></a:prstGeom>",
                xfrm(140.0, 100.0, 40.0, 40.0, "")
            ),
            ..Sp::default()
        }),
        connector(),
        group(),
        frame(
            50,
            "Table",
            "http://schemas.openxmlformats.org/drawingml/2006/table",
            "<a:tbl/>",
            &xfrm(400.0, 300.0, 100.0, 60.0, ""),
        ),
        frame(
            51,
            "Chart",
            "http://schemas.openxmlformats.org/drawingml/2006/chart",
            "<c:chart xmlns:c=\"urn:c\" r:id=\"rIdChart\"/>",
            &xfrm(520.0, 300.0, 100.0, 60.0, ""),
        ),
        frame(
            52,
            "Object",
            "http://schemas.openxmlformats.org/presentationml/2006/ole",
            concat!(
                "<p:oleObj r:id=\"rIdOle\" progId=\"Excel.Sheet.12\"><p:embed/><p:pic><p:nvPicPr><p:cNvPr id=\"0\" name=\"\"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>",
                "<p:blipFill><a:blip r:embed=\"rIdImage\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr/></p:pic></p:oleObj>"
            ),
            &xfrm(400.0, 200.0, 100.0, 60.0, ""),
        ),
        format!(
            "<mc:AlternateContent><mc:Choice Requires=\"p14\">{}</mc:Choice><mc:Fallback>{}</mc:Fallback></mc:AlternateContent>",
            sp(Sp {
                id: 60,
                name: Some("Chosen".to_owned()),
                sp_pr: format!("{}{RECT}{}", xfrm(20.0, 220.0, 60.0, 30.0, ""), solid("000080")),
                ..Sp::default()
            }),
            sp(Sp {
                id: 61,
                name: Some("Not chosen".to_owned()),
                sp_pr: format!("{}{RECT}", xfrm(20.0, 220.0, 60.0, 30.0, "")),
                ..Sp::default()
            })
        ),
        format!(
            "<mc:AlternateContent xmlns:cx1=\"urn:cx1\"><mc:Choice Requires=\"cx1\">{}</mc:Choice><mc:Fallback>{}</mc:Fallback></mc:AlternateContent>",
            sp(Sp {
                id: 62,
                name: Some("Needs cx1".to_owned()),
                sp_pr: format!("{}{RECT}", xfrm(100.0, 220.0, 60.0, 30.0, "")),
                ..Sp::default()
            }),
            sp(Sp {
                id: 63,
                name: Some("Fallback".to_owned()),
                sp_pr: format!("{}{RECT}{}", xfrm(100.0, 220.0, 60.0, 30.0, ""), solid("008000")),
                ..Sp::default()
            })
        ),
        sp(Sp {
            id: 70,
            name: Some("Linked".to_owned()),
            c_nv_pr: " descr=\"A described box\" hidden=\"1\"".to_owned(),
            sp_pr: format!(
                "{}{RECT}<a:blipFill><a:blip r:embed=\"rIdImage\"/><a:stretch><a:fillRect/></a:stretch></a:blipFill>",
                xfrm(200.0, 220.0, 60.0, 30.0, "")
            ),
            link: Some("<a:hlinkClick r:id=\"rIdUrl\"/>".to_owned()),
            ..Sp::default()
        }),
        sp(Sp {
            id: 70,
            name: Some("Same id".to_owned()),
            sp_pr: format!(
                "{}{RECT}<a:blipFill><a:blip r:link=\"rIdRemote\"/><a:tile/></a:blipFill>",
                xfrm(280.0, 220.0, 60.0, 30.0, "")
            ),
            ..Sp::default()
        }),
        "<p:contentPart r:id=\"rIdInk\"/>".to_owned(),
    ]
    .join("");
    pptx(PptxSpec {
        slides: vec![
            SlideSpec {
                id: Some(256),
                tree,
                rels: vec![
                    rel("rIdImage", "image", "../media/image1.png"),
                    external("rIdUrl", "hyperlink", "https://example.com/a?b=1"),
                    external("rIdRemote", "image", "https://example.com/remote.png"),
                ],
                ..SlideSpec::default()
            },
            SlideSpec {
                id: Some(257),
                bg: Some("<p:bgRef idx=\"1003\"><a:schemeClr val=\"accent6\"/></p:bgRef>".to_owned()),
                tree: sp(Sp {
                    id: 2,
                    sp_pr: format!("{}{RECT}", xfrm(10.0, 10.0, 100.0, 100.0, "")),
                    style: Some(
                        "<a:fillRef idx=\"0\"><a:schemeClr val=\"accent1\"/></a:fillRef><a:lnRef idx=\"0\"/>".to_owned(),
                    ),
                    ..Sp::default()
                }),
                ..SlideSpec::default()
            },
        ],
        parts: vec![("ppt/media/image1.png".to_owned(), png(4, 2))],
        ..PptxSpec::default()
    })
}

fn run(text: &str, r_pr: &str) -> String {
    format!("<a:r><a:rPr lang=\"en-US\"{r_pr}</a:rPr><a:t>{text}</a:t></a:r>")
}

fn para(content: &str, p_pr: &str) -> String {
    let p_pr = if p_pr.is_empty() {
        String::new()
    } else {
        format!("<a:pPr{p_pr}</a:pPr>")
    };
    format!("<a:p>{p_pr}{content}</a:p>")
}

fn text_box(id: u64, paragraphs: &str, body_pr: &str, y: f64) -> String {
    sp(Sp {
        id,
        name: Some(format!("Text {id}")),
        sp_pr: format!("{}{RECT}", xfrm(20.0, y, 300.0, 40.0, "")),
        tx_body: Some(format!("{body_pr}<a:lstStyle/>{paragraphs}")),
        ..Sp::default()
    })
}

/// Runs, fields, breaks, tabs, links, bullets, spacing and body properties, on placeholders and text boxes.
pub fn text() -> Vec<u8> {
    let styled = [
        run("Bold", " b=\"1\" sz=\"2400\">"),
        run(" italic", " i=\"1\">"),
        run(" under", " u=\"sng\">"),
        run(" wavy", " u=\"wavy\">"),
        run(" gone", " u=\"sng\"><a:uFill><a:noFill/></a:uFill>"),
        run(" struck", " strike=\"sngStrike\">"),
        run(" up", " baseline=\"30000\">"),
        run(" caps", " cap=\"small\" spc=\"150\">"),
        run(" marked", "><a:highlight><a:srgbClr val=\"FFFF00\"/></a:highlight>"),
        run(" heading", "><a:latin typeface=\"+mj-lt\"/><a:ea typeface=\"MS Gothic\"/>"),
        run(
            " family",
            "><a:solidFill><a:schemeClr val=\"accent1\"/></a:solidFill><a:latin typeface=\"Roboto Medium\"/>",
        ),
        run(
            " fade",
            "><a:gradFill><a:gsLst><a:gs pos=\"0\"><a:srgbClr val=\"00FF00\"/></a:gs></a:gsLst></a:gradFill>",
        ),
    ]
    .join("");
    let links = [
        run("site", "><a:hlinkClick r:id=\"rIdUrl\"/>"),
        run(
            " kept",
            "><a:solidFill><a:srgbClr val=\"123456\"/></a:solidFill><a:hlinkClick r:id=\"rIdUrl\"><a:extLst><a:ext uri=\"{A12FA001-AC4F-418D-AE19-62706E023703}\"><ahyp:hlinkClr xmlns:ahyp=\"http://schemas.microsoft.com/office/drawing/2018/hyperlinkcolor\" val=\"tx\"/></a:ext></a:extLst></a:hlinkClick>",
        ),
        run(
            " jump",
            "><a:hlinkClick r:id=\"rIdSlide\" action=\"ppaction://hlinksldjump\"/>",
        ),
        run(
            " next",
            "><a:hlinkClick r:id=\"\" action=\"ppaction://hlinkshowjump?jump=nextslide\"/>",
        ),
        run(" program", "><a:hlinkClick r:id=\"\" action=\"ppaction://program\"/>"),
    ]
    .join("");
    let fields = concat!(
        "<a:fld id=\"{1}\" type=\"slidenum\"><a:rPr lang=\"en-US\"/><a:t>3</a:t></a:fld><a:r><a:rPr lang=\"en-US\"/><a:t>\tof\t</a:t></a:r>",
        "<a:fld id=\"{2}\" type=\"datetime1\"><a:rPr lang=\"en-US\"/><a:t>1/2/2026</a:t></a:fld><a:br><a:rPr lang=\"en-US\" sz=\"900\"/></a:br>",
        "<a:fld id=\"{3}\" type=\"datetime12\"><a:rPr lang=\"en-US\"/><a:t>9:41 AM</a:t></a:fld>",
        "<a:fld id=\"{4}\" type=\"footer\"><a:rPr lang=\"en-US\"/><a:t>Footer text</a:t></a:fld>"
    );
    let lists = [
        para(
            &run("First", ">"),
            " marL=\"342900\" indent=\"-342900\"><a:buClr><a:schemeClr val=\"accent2\"/></a:buClr><a:buSzPct val=\"80000\"/><a:buFont typeface=\"Wingdings\"/><a:buChar char=\"§\"/>",
        ),
        para(
            &run("Second", ">"),
            " lvl=\"1\" marL=\"685800\" indent=\"-342900\"><a:buFontTx/><a:buAutoNum type=\"alphaLcParenR\" startAt=\"3\"/>",
        ),
        para(
            &run("Third", ">"),
            "><a:buSzPts val=\"1400\"/><a:buAutoNum type=\"circleNumDbPlain\"/>",
        ),
        para(&run("Fourth", ">"), "><a:buBlip><a:blip r:embed=\"rIdImage\"/></a:buBlip>"),
        para(&run("Fifth", ">"), "><a:buClr><a:srgbClr val=\"FF0000\"/></a:buClr>"),
        para(
            &run("Sixth", ">"),
            " algn=\"dist\" rtl=\"1\"><a:lnSpc><a:spcPts val=\"2400\"/></a:lnSpc><a:spcBef><a:spcPct val=\"50000\"/></a:spcBef><a:spcAft><a:spcPts val=\"600\"/></a:spcAft><a:tabLst><a:tab pos=\"914400\" algn=\"l\"/></a:tabLst>",
        ),
        para("", "><a:lnSpc><a:spcPct val=\"150000\"/></a:lnSpc>"),
    ]
    .join("");
    let tree = [
        sp(Sp {
            id: 2,
            name: Some("Title".to_owned()),
            ph: "<p:ph type=\"title\"/>".to_owned(),
            tx_body: Some(format!(
                "<a:bodyPr><a:normAutofit fontScale=\"90000\" lnSpcReduction=\"10000\"/></a:bodyPr><a:lstStyle/>{}",
                para(&run("A title", ">"), "")
            )),
            ..Sp::default()
        }),
        sp(Sp {
            id: 3,
            name: Some("Content".to_owned()),
            ph: "<p:ph idx=\"1\"/>".to_owned(),
            tx_body: Some(format!(
                "<a:bodyPr lIns=\"0\" anchor=\"ctr\"><a:spAutoFit/></a:bodyPr><a:lstStyle><a:lvl1pPr><a:defRPr sz=\"2000\"/></a:lvl1pPr></a:lstStyle>{lists}"
            )),
            ..Sp::default()
        }),
        text_box(
            10,
            &para(&styled, ""),
            "<a:bodyPr wrap=\"none\" vert=\"vert270\" numCol=\"2\" anchorCtr=\"1\"><a:noAutofit/></a:bodyPr>",
            300.0,
        ),
        text_box(
            11,
            &(para(&links, "") + &para(fields, "")),
            "<a:bodyPr anchor=\"b\"/>",
            340.0,
        ),
        sp(Sp {
            id: 12,
            name: Some("Styled text".to_owned()),
            sp_pr: format!("{}{RECT}", xfrm(400.0, 300.0, 100.0, 40.0, "")),
            style: Some(
                "<a:lnRef idx=\"0\"/><a:fillRef idx=\"1\"><a:schemeClr val=\"accent1\"/></a:fillRef><a:effectRef idx=\"0\"/><a:fontRef idx=\"major\"><a:schemeClr val=\"lt1\"/></a:fontRef>"
                    .to_owned(),
            ),
            tx_body: Some(format!(
                "<a:bodyPr/><a:lstStyle><a:lvl1pPr><a:defRPr><a:solidFill><a:srgbClr val=\"000000\"/></a:solidFill></a:defRPr></a:lvl1pPr></a:lstStyle>{}",
                para(&run("On a style", ">"), "")
            )),
            ..Sp::default()
        }),
    ]
    .join("");
    pptx(PptxSpec {
        masters: Some(vec![MasterSpec {
            tx_styles: Some(tx_styles()),
            tree: title(2, &xfrm(36.0, 20.0, 648.0, 60.0, ""))
                + &content(3, &xfrm(36.0, 100.0, 648.0, 260.0, "")),
            layouts: vec![LayoutSpec {
                kind: Some("obj".to_owned()),
                tree: title(2, "") + &content(3, ""),
                ..LayoutSpec::default()
            }],
            ..MasterSpec::default()
        }]),
        slides: vec![
            SlideSpec {
                id: Some(256),
                tree,
                rels: vec![
                    external("rIdUrl", "hyperlink", "mailto:team@example.com"),
                    rel("rIdSlide", "slide", "slide2.xml"),
                    rel("rIdImage", "image", "../media/image1.png"),
                ],
                ..SlideSpec::default()
            },
            SlideSpec {
                id: Some(257),
                tree: text_box(
                    2,
                    &(para(&run("Empty paragraphs follow", ">"), "")
                        + "<a:p/><a:p><a:endParaRPr sz=\"4000\"/></a:p>"),
                    "<a:bodyPr/>",
                    20.0,
                ),
                ..SlideSpec::default()
            },
        ],
        parts: vec![("ppt/media/image1.png".to_owned(), png(4, 2))],
        ..PptxSpec::default()
    })
}

/// A picture (`p:pic`) with a blip fill, its properties and its non-visual properties.
#[derive(Clone, Debug, Default)]
pub struct Pic {
    pub sp_pr: Option<String>,
    pub nv_pr: String,
    pub name: Option<String>,
    pub c_nv_pr: String,
}

pub fn pic(id: u64, blip_fill: &str, options: Pic) -> String {
    let name = options.name.unwrap_or_else(|| format!("Picture {id}"));
    let sp_pr = options
        .sp_pr
        .unwrap_or_else(|| format!("{}{RECT}", xfrm(20.0, 20.0, 100.0, 60.0, "")));
    format!(
        "<p:pic><p:nvPicPr><p:cNvPr id=\"{id}\" name=\"{name}\"{}/><p:cNvPicPr/><p:nvPr>{}</p:nvPr></p:nvPicPr><p:blipFill>{blip_fill}</p:blipFill><p:spPr>{sp_pr}</p:spPr></p:pic>",
        options.c_nv_pr, options.nv_pr
    )
}

pub fn blip(rel: &str, effects: &str, after: &str) -> String {
    format!("<a:blip r:embed=\"{rel}\">{effects}</a:blip>{after}")
}

const STRETCH: &str = "<a:stretch><a:fillRect/></a:stretch>";

/// A minimal ISO media file: one track with this handler and sample entry, `moov` before or after `mdat`, and a
/// movie header stating a length in milliseconds (or none).
pub fn movie(handler: &str, codec: &str, moov_first: bool, duration_ms: Option<u32>) -> Vec<u8> {
    fn boxed(kind: &str, children: &[&[u8]]) -> Vec<u8> {
        let body = children.concat();
        let mut out = ((8 + body.len()) as u32).to_be_bytes().to_vec();
        out.extend(kind.as_bytes());
        out.extend(body);
        out
    }
    let stsd = boxed(
        "stsd",
        &[
            &[0; 4],
            &[0, 0, 0, 1],
            &[0, 0, 0, 16],
            codec.as_bytes(),
            &[0; 4],
        ],
    );
    let stbl = boxed("stbl", &[&stsd]);
    let minf = boxed("minf", &[&stbl]);
    let hdlr = boxed("hdlr", &[&[0; 8], handler.as_bytes(), &[0; 12]]);
    let mdia = boxed("mdia", &[&hdlr, &minf]);
    let tkhd = boxed("tkhd", &[&[0; 8]]);
    let trak = boxed("trak", &[&tkhd, &mdia]);
    let mvhd = match duration_ms {
        None => boxed("mvhd", &[&[0; 8]]),
        Some(ms) => boxed(
            "mvhd",
            &[&[0; 12], &1000u32.to_be_bytes(), &ms.to_be_bytes()],
        ),
    };
    let moov = boxed("moov", &[&mvhd, &trak]);
    let ftyp = boxed("ftyp", &[b"isom", &[0; 4]]);
    let mdat = boxed("mdat", &[&[0; 1000]]);
    if moov_first {
        [ftyp, moov, mdat].concat()
    } else {
        [ftyp, mdat, moov].concat()
    }
}

/// Pictures (crop, opacity, recolour, clip, outline), movies and sound, notes, transitions and animations. `trim`
/// is the movie's `p14:trim` attributes; `movie_ms` the length its header states.
pub fn media_with(trim: &str, movie_ms: Option<u32>) -> Vec<u8> {
    media_timed(trim, movie_ms, None)
}

/// [`media_with`], the movies' slide (the movie is shape 30) with `timing` in place of its own `p:timing`.
pub fn media_timed(trim: &str, movie_ms: Option<u32>, timing: Option<&str>) -> Vec<u8> {
    let pictures = [
        pic(
            10,
            &blip("rIdPng", "<a:alphaModFix amt=\"40000\"/>", STRETCH),
            Pic {
                c_nv_pr: " descr=\"Logo\"".to_owned(),
                ..Pic::default()
            },
        ),
        pic(
            11,
            &blip("rIdPng", "", STRETCH).replace(
                "<a:stretch>",
                "<a:srcRect l=\"10000\" t=\"-5000\" r=\"20000\" b=\"92000\"/><a:stretch>",
            ),
            Pic::default(),
        ),
        pic(
            12,
            &blip("rIdPng", "<a:grayscl/><a:lum bright=\"20000\"/>", STRETCH),
            Pic::default(),
        ),
        pic(
            13,
            &blip("rIdPng", "", STRETCH),
            Pic {
                sp_pr: Some(format!(
                    "{}<a:prstGeom prst=\"ellipse\"><a:avLst/></a:prstGeom><a:ln w=\"25400\"><a:solidFill><a:schemeClr val=\"accent1\"/></a:solidFill></a:ln>",
                    xfrm(140.0, 20.0, 80.0, 80.0, "")
                )),
                ..Pic::default()
            },
        ),
        pic(
            14,
            &blip("rIdPng", "", STRETCH)
                .replace("<a:stretch>", "<a:srcRect t=\"50000\" b=\"49800\"/><a:stretch>"),
            Pic::default(),
        ),
        pic(
            15,
            &format!("<a:blip r:link=\"rIdRemote\"/>{STRETCH}"),
            Pic::default(),
        ),
        pic(16, &blip("rIdGone", "", STRETCH), Pic::default()),
        pic(
            17,
            &blip("rIdPng", "", "<a:stretch><a:fillRect l=\"10000\"/></a:stretch>"),
            Pic::default(),
        ),
        pic(
            18,
            &blip("rIdPng", "", "<a:tile tx=\"0\" ty=\"0\" sx=\"100000\" sy=\"100000\"/>"),
            Pic::default(),
        ),
        pic(
            19,
            &blip("rIdPng", "", STRETCH),
            Pic {
                sp_pr: Some(String::new()),
                nv_pr: "<p:ph type=\"pic\" idx=\"13\"/>".to_owned(),
                ..Pic::default()
            },
        ),
    ]
    .join("");
    let movies = [
        pic(
            30,
            &blip("rIdPoster", "", STRETCH),
            Pic {
                name: Some("Movie".to_owned()),
                nv_pr: format!(
                    "<a:videoFile r:link=\"rIdVideo\"/><p:extLst><p:ext uri=\"{{DAA4B4D4-6727-11D9-A1E7-17B52E2B1E3B}}\"><p14:media r:embed=\"rIdMedia\"><p14:trim {trim}/></p14:media></p:ext></p:extLst>"
                ),
                sp_pr: Some(format!("{}{RECT}", xfrm(20.0, 200.0, 160.0, 90.0, ""))),
                ..Pic::default()
            },
        ),
        pic(
            31,
            &blip("rIdPoster", "", STRETCH),
            Pic {
                name: Some("Online movie".to_owned()),
                nv_pr: "<a:videoFile r:link=\"rIdOnline\"/>".to_owned(),
                sp_pr: Some(format!("{}{RECT}", xfrm(200.0, 200.0, 160.0, 90.0, ""))),
                ..Pic::default()
            },
        ),
        pic(
            32,
            &blip("rIdPng", "", STRETCH),
            Pic {
                name: Some("Sound".to_owned()),
                nv_pr: "<a:audioFile r:link=\"rIdAudio\"/>".to_owned(),
                sp_pr: Some(format!("{}{RECT}", xfrm(380.0, 200.0, 30.0, 30.0, ""))),
                ..Pic::default()
            },
        ),
    ]
    .join("");
    let fade = "<p:transition spd=\"med\" advClick=\"0\" advTm=\"3000\"><p:fade/></p:transition>";
    let prism = concat!(
        "<mc:AlternateContent><mc:Choice Requires=\"p14\"><p:transition spd=\"slow\" p14:dur=\"1600\"><p14:prism/></p:transition>",
        "</mc:Choice><mc:Fallback><p:transition spd=\"slow\"><p:fade/></p:transition></mc:Fallback></mc:AlternateContent>"
    );
    let own_timing = concat!(
        "<p:timing><p:tnLst><p:par><p:cTn id=\"1\" dur=\"indefinite\" restart=\"never\" nodeType=\"tmRoot\"><p:childTnLst><p:seq>",
        "<p:cTn id=\"2\" presetID=\"10\" presetClass=\"entr\" nodeType=\"clickEffect\"/></p:seq></p:childTnLst></p:cTn></p:par>",
        "</p:tnLst></p:timing>"
    );
    pptx(PptxSpec {
        masters: Some(vec![MasterSpec {
            layouts: vec![LayoutSpec {
                tree: pic(
                    5,
                    &blip("rIdLayoutPng", "", STRETCH),
                    Pic {
                        sp_pr: Some(format!("{}{RECT}", xfrm(600.0, 20.0, 100.0, 80.0, ""))),
                        nv_pr: "<p:ph type=\"pic\" idx=\"13\"/>".to_owned(),
                        ..Pic::default()
                    },
                ),
                rels: vec![rel("rIdLayoutPng", "image", "../media/image1.png")],
                ..LayoutSpec::default()
            }],
            ..MasterSpec::default()
        }]),
        slides: vec![
            SlideSpec {
                id: Some(256),
                tree: pictures,
                tail: fade.to_owned(),
                notes: Some(
                    "<a:p><a:r><a:rPr lang=\"en-US\"/><a:t>Say hello</a:t></a:r></a:p><a:p><a:r><a:rPr lang=\"en-US\"/><a:t>then go</a:t></a:r></a:p>"
                        .to_owned(),
                ),
                rels: vec![
                    rel("rIdPng", "image", "../media/image1.png"),
                    external("rIdRemote", "image", "https://example.com/a.png"),
                    rel("rIdGone", "image", "../media/missing.png"),
                ],
                ..SlideSpec::default()
            },
            SlideSpec {
                id: Some(257),
                tree: movies,
                tail: format!("{prism}{}", timing.unwrap_or(own_timing)),
                notes: Some("<a:p><a:endParaRPr lang=\"en-US\"/></a:p>".to_owned()),
                rels: vec![
                    rel("rIdPoster", "image", "../media/poster.png"),
                    rel("rIdPng", "image", "../media/image1.png"),
                    rel("rIdVideo", "video", "../media/movie1.mp4"),
                    rel(
                        "rIdMedia",
                        "http://schemas.microsoft.com/office/2007/relationships/media",
                        "../media/movie1.mp4",
                    ),
                    external("rIdOnline", "video", "https://example.com/watch"),
                    rel("rIdAudio", "audio", "../media/sound.wav"),
                ],
                ..SlideSpec::default()
            },
        ],
        parts: vec![
            ("ppt/media/image1.png".to_owned(), png(4, 2)),
            ("ppt/media/poster.png".to_owned(), png(8, 4)),
            (
                "ppt/media/movie1.mp4".to_owned(),
                movie("vide", "avc1", true, movie_ms),
            ),
            (
                "ppt/media/sound.wav".to_owned(),
                b"RIFF\0\0\0\0WAVEfmt ".to_vec(),
            ),
        ],
        ..PptxSpec::default()
    })
}

pub fn media() -> Vec<u8> {
    media_with("st=\"1500\"", None)
}

fn line(w: u32, color: &str, extra: &str) -> String {
    format!("<a:ln w=\"{w}\"><a:solidFill>{color}</a:solidFill>{extra}</a:ln>")
}

fn scheme(slot: &str, mods: &str) -> String {
    format!("<a:schemeClr val=\"{slot}\">{mods}</a:schemeClr>")
}

fn style_fill(color: &str) -> String {
    format!("<a:fill><a:solidFill>{color}</a:solidFill></a:fill>")
}

/// A table style with every region kind: the whole table (outer edges, inside edges by line and by reference, a
/// fill, the text's font reference and colour), banded rows, the first column, the last and first rows, two corner
/// cells, and a background from the theme's fill styles.
fn test_style() -> String {
    let outer: String = ["left", "right", "top", "bottom"]
        .iter()
        .map(|edge| {
            format!(
                "<a:{edge}>{}</a:{edge}>",
                line(12_700, &scheme("accent1", ""), "")
            )
        })
        .collect();
    [
        "<a:tblStyle styleId=\"{00000000-0000-0000-0000-00000000TEST}\" styleName=\"Test\">".to_owned(),
        format!("<a:tblBg><a:fillRef idx=\"1\">{}</a:fillRef></a:tblBg>", scheme("lt2", "")),
        format!(
            "<a:wholeTbl><a:tcTxStyle><a:fontRef idx=\"minor\"><a:prstClr val=\"black\"/></a:fontRef>{}</a:tcTxStyle>",
            scheme("dk1", "")
        ),
        "<a:tcStyle><a:tcBdr>".to_owned(),
        outer,
        format!("<a:insideH><a:lnRef idx=\"1\">{}</a:lnRef></a:insideH>", scheme("accent2", "")),
        format!(
            "<a:insideV>{}</a:insideV>",
            line(6350, "<a:srgbClr val=\"808080\"/>", "<a:prstDash val=\"dash\"/>")
        ),
        format!("</a:tcBdr>{}</a:tcStyle></a:wholeTbl>", style_fill(&scheme("lt1", ""))),
        format!(
            "<a:band1H><a:tcStyle><a:tcBdr/>{}</a:tcStyle></a:band1H>",
            style_fill(&scheme("accent1", "<a:tint val=\"40000\"/>"))
        ),
        "<a:band2H><a:tcStyle><a:tcBdr/></a:tcStyle></a:band2H>".to_owned(),
        "<a:firstCol><a:tcTxStyle b=\"on\"/><a:tcStyle><a:tcBdr/></a:tcStyle></a:firstCol>".to_owned(),
        format!(
            "<a:lastRow><a:tcTxStyle i=\"on\"/><a:tcStyle><a:tcBdr><a:top>{}</a:top></a:tcBdr>",
            line(38_100, &scheme("accent1", ""), "")
        ),
        "<a:fill><a:noFill/></a:fill></a:tcStyle></a:lastRow>".to_owned(),
        format!(
            "<a:firstRow><a:tcTxStyle b=\"on\"><a:font><a:latin typeface=\"Georgia\"/></a:font>{}</a:tcTxStyle>",
            scheme("lt1", "")
        ),
        format!(
            "<a:tcStyle><a:tcBdr><a:bottom>{}</a:bottom></a:tcBdr>{}",
            line(25_400, &scheme("dk1", ""), ""),
            style_fill(&scheme("accent1", ""))
        ),
        "</a:tcStyle></a:firstRow>".to_owned(),
        format!("<a:seCell><a:tcStyle>{}</a:tcStyle></a:seCell>", style_fill(&scheme("accent5", ""))),
        format!("<a:nwCell><a:tcStyle>{}</a:tcStyle></a:nwCell>", style_fill(&scheme("accent6", ""))),
        "</a:tblStyle>".to_owned(),
    ]
    .join("")
}

fn tc_pr(attrs: &str, children: &str) -> String {
    let attrs = if attrs.is_empty() {
        String::new()
    } else {
        format!(" {attrs}")
    };
    format!("<a:tcPr{attrs}>{children}</a:tcPr>")
}

fn row(height: f64, cells: Vec<String>) -> Row {
    Row { height, cells }
}

/// Tables: a style's regions with merges, a cell's own edges, fills, margins and anchors, backgrounds, a malformed
/// grid.
pub fn tables() -> Vec<u8> {
    let styled = table(TableSpec {
        id: 10,
        // Google Slides writes a placeholder size; the grid gives the frame.
        frame: xfrm(20.0, 20.0, 236.22, 236.22, ""),
        tbl_pr: Some(
            concat!(
                "<a:tblPr firstRow=\"1\" firstCol=\"1\" lastRow=\"1\" bandRow=\"1\">",
                "<a:tableStyleId>{00000000-0000-0000-0000-00000000TEST}</a:tableStyleId></a:tblPr>"
            )
            .to_owned(),
        ),
        columns: vec![100.0, 100.0, 100.0, 100.0],
        rows: vec![
            row(
                30.0,
                vec![
                    tc1("Corner"),
                    tc(Some(&["Spanned"]), "", "gridSpan=\"2\""),
                    tc(None, "", "hMerge=\"1\""),
                    tc1("Head"),
                ],
            ),
            row(
                30.0,
                vec![
                    tc(Some(&["Tall"]), "", "rowSpan=\"2\""),
                    tc1("B1"),
                    tc(
                        Some(&["C1"]),
                        &tc_pr(
                            "",
                            "<a:lnR><a:noFill/></a:lnR><a:solidFill><a:srgbClr val=\"FF0000\"/></a:solidFill>",
                        ),
                        "",
                    ),
                    tc1("D1"),
                ],
            ),
            row(
                30.0,
                vec![
                    tc(None, "", "vMerge=\"1\""),
                    tc(Some(&["Block"]), "", "gridSpan=\"2\" rowSpan=\"2\""),
                    tc(None, "", "hMerge=\"1\""),
                    tc1("D2"),
                ],
            ),
            row(
                30.0,
                vec![
                    tc1("A3"),
                    tc(None, "", "vMerge=\"1\""),
                    tc(None, "", "hMerge=\"1\" vMerge=\"1\""),
                    tc1("D3"),
                ],
            ),
            row(
                30.0,
                vec![
                    tc1("Total"),
                    tc1("B4"),
                    tc(Some(&["C4"]), &tc_pr("", "<a:lnT w=\"6350\"/>"), ""),
                    tc1("D4"),
                ],
            ),
        ],
        ph: String::new(),
    });
    let own = table(TableSpec {
        id: 11,
        frame: xfrm(440.0, 20.0, 280.0, 80.0, ""),
        tbl_pr: None,
        columns: vec![120.0, 80.0, 80.0],
        rows: vec![
            row(
                40.0,
                vec![
                    tc(
                        Some(&["Margins", ""]),
                        &tc_pr(
                            "marL=\"0\" marR=\"25400\" marT=\"12700\" marB=\"0\" anchor=\"b\"",
                            "",
                        ),
                        "",
                    ),
                    tc(
                        Some(&["Diagonals"]),
                        &tc_pr(
                            "anchor=\"ctr\"",
                            concat!(
                                "<a:lnTlToBr w=\"12700\"><a:solidFill><a:srgbClr val=\"FF0000\"/></a:solidFill><a:prstDash val=\"sysDot\"/></a:lnTlToBr>",
                                "<a:lnBlToTr w=\"12700\"><a:solidFill><a:srgbClr val=\"0000FF\"/></a:solidFill><a:custDash><a:ds d=\"300000\" sp=\"100000\"/></a:custDash></a:lnBlToTr>"
                            ),
                        ),
                        "",
                    ),
                    tc(None, &tc_pr("", ""), ""),
                ],
            ),
            row(
                40.0,
                vec![
                    tc(
                        Some(&["Gradient"]),
                        &tc_pr(
                            "",
                            concat!(
                                "<a:lnL w=\"25400\"><a:solidFill><a:srgbClr val=\"000000\"/></a:solidFill><a:headEnd type=\"triangle\"/></a:lnL>",
                                "<a:gradFill><a:gsLst><a:gs pos=\"0\"><a:srgbClr val=\"FFFFFF\"/></a:gs><a:gs pos=\"100000\"><a:srgbClr val=\"000000\"/></a:gs>",
                                "</a:gsLst><a:lin ang=\"5400000\" scaled=\"0\"/></a:gradFill>"
                            ),
                        ),
                        "",
                    ),
                    tc(
                        Some(&["Pattern"]),
                        &tc_pr(
                            "",
                            "<a:pattFill prst=\"pct5\"><a:fgClr><a:srgbClr val=\"00FF00\"/></a:fgClr><a:bgClr><a:srgbClr val=\"FFFFFF\"/></a:bgClr></a:pattFill>",
                        ),
                        "",
                    ),
                    tc(
                        Some(&["Vertical"]),
                        &tc_pr(
                            "vert=\"vert270\" anchorCtr=\"1\"",
                            "<a:cell3D><a:bevel w=\"12700\" h=\"12700\"/><a:lightRig rig=\"threePt\" dir=\"t\"/></a:cell3D>",
                        ),
                        "",
                    ),
                ],
            ),
        ],
        ph: String::new(),
    });
    let background = table(TableSpec {
        id: 12,
        frame: xfrm(440.0, 120.0, 120.0, 30.0, ""),
        tbl_pr: Some(
            concat!(
                "<a:tblPr rtl=\"1\"><a:gradFill><a:gsLst><a:gs pos=\"0\"><a:srgbClr val=\"FFFF00\"/></a:gs><a:gs pos=\"100000\">",
                "<a:srgbClr val=\"00FFFF\"/></a:gs></a:gsLst><a:lin ang=\"0\" scaled=\"0\"/></a:gradFill>",
                "<a:effectLst><a:outerShdw blurRad=\"40000\"><a:srgbClr val=\"000000\"/></a:outerShdw></a:effectLst>",
                "<a:tableStyleId>{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}</a:tableStyleId></a:tblPr>"
            )
            .to_owned(),
        ),
        columns: vec![60.0, 60.0],
        rows: vec![row(
            30.0,
            vec![tc(Some(&["Own"]), &tc_pr("", &solid("C0C0C0")), ""), tc1("Background")],
        )],
        ph: String::new(),
    });
    let grouped = format!(
        "<p:grpSp><p:nvGrpSpPr><p:cNvPr id=\"20\" name=\"Group\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr>{}</p:grpSpPr>{}</p:grpSp>",
        xfrm(440.0, 170.0, 200.0, 50.0, "").replace(
            "</a:xfrm>",
            "<a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"1270000\" cy=\"317500\"/></a:xfrm>"
        ),
        table(TableSpec {
            id: 21,
            frame: xfrm(0.0, 0.0, 100.0, 25.0, ""),
            tbl_pr: Some(format!("<a:tblPr>{}</a:tblPr>", solid("FFF2CC"))),
            columns: vec![50.0, 50.0],
            rows: vec![row(25.0, vec![tc1("Scaled"), tc1("Solid background")])],
            ph: String::new(),
        })
    );
    let malformed = table(TableSpec {
        id: 13,
        frame: xfrm(20.0, 200.0, 120.0, 60.0, ""),
        tbl_pr: None,
        columns: vec![40.0, 40.0, 40.0],
        rows: vec![
            // Spans past the grid, a row short of cells, and a span over a slot an earlier cell covers.
            row(20.0, vec![tc(Some(&["Wide"]), "", "gridSpan=\"5\"")]),
            row(
                20.0,
                vec![tc1("B"), tc(Some(&["Deep"]), "", "rowSpan=\"9\"")],
            ),
            row(20.0, vec![tc(Some(&["Clipped"]), "", "gridSpan=\"3\"")]),
        ],
        ph: String::new(),
    });
    let placeholder = table(TableSpec {
        id: 14,
        ph: "<p:ph type=\"tbl\" idx=\"1\"/>".to_owned(),
        frame: xfrm(200.0, 300.0, 100.0, 30.0, ""),
        tbl_pr: None,
        columns: vec![100.0],
        rows: vec![row(30.0, vec![tc1("Placeholder")])],
    });
    pptx(PptxSpec {
        table_styles: Some(test_style()),
        slides: vec![SlideSpec {
            id: Some(256),
            tree: [styled, own, background, grouped, malformed, placeholder].join(""),
            ..SlideSpec::default()
        }],
        ..PptxSpec::default()
    })
}

/// A synthetic presentation's builder.
pub type Build = fn() -> Vec<u8>;

/// Every synthetic presentation, by golden name.
pub const FIXTURES: [(&str, Build); 5] = [
    ("basic", basic),
    ("shapes", shapes),
    ("text", text),
    ("media", media),
    ("tables", tables),
];
