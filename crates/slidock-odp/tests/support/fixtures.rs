//! Synthetic ODP presentations for the frontend's tests, each exercising one area. They are built from XML snippets,
//! so every feature they hold is visible here.

use super::odp::{MasterSpec, OdpSpec, PageSpec, odp, page_style, page_style_under};

/// A small opaque PNG, red and blue, with no stated resolution: the bytes of `tests/vectors/checker-<w>x<h>.png`, so
/// media hashes stay fixed.
pub fn png(width: u32, height: u32) -> Vec<u8> {
    let path = format!(
        "{}/tests/vectors/checker-{width}x{height}.png",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(&path).unwrap_or_else(|_| panic!("no checker vector {path}"))
}

fn iso_box(kind: &str, children: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = children.concat();
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&u32::try_from(8 + body.len()).unwrap().to_be_bytes());
    out.extend_from_slice(kind.as_bytes());
    out.extend_from_slice(&body);
    out
}

/// A minimal ISO media file: one track with this handler and sample entry, `moov` before or after `mdat`, and a movie
/// header stating a length in milliseconds (or none).
pub fn movie(handler: &str, codec: &str, moov_first: bool, duration_ms: Option<u32>) -> Vec<u8> {
    let zeros = |n: usize| vec![0u8; n];
    let stsd = iso_box(
        "stsd",
        &[
            zeros(4),
            vec![0, 0, 0, 1],
            vec![0, 0, 0, 16],
            codec.as_bytes().to_vec(),
            zeros(4),
        ],
    );
    let trak = iso_box(
        "trak",
        &[
            iso_box("tkhd", &[zeros(8)]),
            iso_box(
                "mdia",
                &[
                    iso_box("hdlr", &[zeros(8), handler.as_bytes().to_vec(), zeros(12)]),
                    iso_box("minf", &[iso_box("stbl", &[stsd])]),
                ],
            ),
        ],
    );
    let mvhd = match duration_ms {
        None => iso_box("mvhd", &[zeros(8)]),
        Some(ms) => iso_box(
            "mvhd",
            &[
                zeros(12),
                1000u32.to_be_bytes().to_vec(),
                ms.to_be_bytes().to_vec(),
            ],
        ),
    };
    let moov = iso_box("moov", &[mvhd, trak]);
    let ftyp = iso_box("ftyp", &[b"isom".to_vec(), zeros(4)]);
    let mdat = iso_box("mdat", &[zeros(1000)]);
    if moov_first {
        [ftyp, moov, mdat].concat()
    } else {
        [ftyp, mdat, moov].concat()
    }
}

const NAMED_RESOURCES: &str = concat!(
    r##"<draw:gradient draw:name="Sunrise" draw:style="linear" draw:start-color="#ff0000" draw:end-color="#0000ff" "##,
    r#"draw:start-intensity="100%" draw:end-intensity="50%" draw:angle="300" draw:border="20%"/>"#,
    r##"<draw:gradient draw:name="Glow" draw:style="ellipsoid" draw:cx="25%" draw:cy="75%" draw:start-color="#000000" "##,
    r##"draw:end-color="#ffffff" draw:border="0%"/>"##,
    r##"<draw:hatch draw:name="Lines" draw:style="single" draw:color="#00ff00" draw:distance="0.1cm" draw:rotation="450"/>"##,
    r#"<draw:fill-image draw:name="Paper" xlink:href="Pictures/paper.png" xlink:type="simple" xlink:show="embed"/>"#,
);

/// Master pages, page order, hidden pages, backgrounds of every kind, display flags, transitions and animations.
pub fn basic() -> Vec<u8> {
    let spec = OdpSpec {
        styles: format!(
            "{NAMED_RESOURCES}{}",
            page_style("Plain", r##"draw:fill="solid" draw:fill-color="#336699""##)
        ),
        master_automatic_styles: [
            page_style("Mdp1", r##"draw:fill="solid" draw:fill-color="#ffffff""##),
            page_style("Mdp2", r#"draw:fill="gradient" draw:fill-gradient-name="Sunrise""#),
        ]
        .concat(),
        masters: Some(vec![
            MasterSpec::named("Title_20_Slide")
                .display("Title Slide")
                .style("Mdp1"),
            MasterSpec::named("Content").style("Mdp2"),
            MasterSpec::named("Unused"),
        ]),
        automatic_styles: [
            page_style(
                "dp1",
                r#"presentation:background-visible="true" presentation:background-objects-visible="true""#,
            ),
            page_style("dp2", r#"presentation:visibility="hidden""#),
            page_style_under("dp3", r#"presentation:background-objects-visible="false""#, "Plain"),
            page_style(
                "dp4",
                r#"draw:fill="bitmap" draw:fill-image-name="Paper" style:repeat="stretch""#,
            ),
            page_style(
                "dp5",
                r##"draw:fill="hatch" draw:fill-hatch-name="Lines" draw:fill-hatch-solid="true" draw:fill-color="#eeeeee""##,
            ),
            page_style(
                "dp6",
                r#"draw:fill="gradient" draw:fill-gradient-name="Glow" smil:type="barWipe" presentation:transition-speed="slow""#,
            ),
            page_style("dp7", r#"presentation:background-visible="false""#),
        ]
        .concat(),
        pages: vec![
            PageSpec::new().name("Opening").master("Title_20_Slide").style("dp1"),
            PageSpec::new().master("Content").style("dp2"),
            PageSpec::new().master("Content").style("dp3"),
            PageSpec::new().master("Content").style("dp4"),
            PageSpec::new().master("Content").style("dp5"),
            PageSpec::new()
                .master("Content")
                .style("dp6")
                .attrs(r#" xml:id="closing""#)
                .with_body(concat!(
                    r#"<anim:par presentation:node-type="timing-root"><anim:seq presentation:node-type="main-sequence">"#,
                    r#"<anim:par smil:begin="next"><anim:par smil:begin="0s"><anim:par smil:begin="0s" smil:fill="hold" "#,
                    r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-appear">"#,
                    r#"<anim:set smil:begin="0s" smil:dur="0.001s" smil:fill="hold" smil:targetElement="id1" "#,
                    r#"smil:attributeName="visibility" smil:to="visible"/></anim:par></anim:par></anim:par></anim:seq></anim:par>"#,
                )),
            PageSpec::new().master("Missing").style("dp7"),
        ],
        ..OdpSpec::default()
    }
    .part("Pictures/paper.png", png(4, 2));
    odp(&spec)
}

pub fn graphic(name: &str, props: &str) -> String {
    graphic_under(name, props, "standard")
}

pub fn graphic_under(name: &str, props: &str, parent: &str) -> String {
    format!(
        r#"<style:style style:name="{name}" style:family="graphic" style:parent-style-name="{parent}"><style:graphic-properties {props}/></style:style>"#
    )
}

pub fn place(x: &str, y: &str, w: &str, h: &str) -> String {
    format!(r#"svg:x="{x}" svg:y="{y}" svg:width="{w}" svg:height="{h}""#)
}

fn custom(name: &str, style: &str, place: &str, geometry: &str) -> String {
    format!(
        r#"<draw:custom-shape draw:name="{name}" draw:style-name="{style}" {place}><draw:enhanced-geometry {geometry}/></draw:custom-shape>"#
    )
}

fn image(name: &str, style: &str, place: &str, href: &str, extra: &str) -> String {
    format!(
        r#"<draw:frame draw:name="{name}" draw:style-name="{style}" {place}{extra}><draw:image xlink:href="{href}" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/></draw:frame>"#
    )
}

const SHAPE_STYLES: &str = concat!(
    r#"<draw:marker draw:name="msArrowEnd_20_5" draw:display-name="msArrowEnd 5" svg:viewBox="0 0 300 300" svg:d="M150 0l150 300h-300z"/>"#,
    r#"<draw:marker draw:name="Circle" svg:viewBox="0 0 1131 1131" svg:d="M462 1118l-102-29-102-51-93-72-72-93z"/>"#,
    r#"<draw:stroke-dash draw:name="Dashed_20__28_var_29__20_4" draw:display-name="Dashed (var) 4" draw:style="rect" "#,
    r#"draw:dots1="1" draw:dots1-length="0.106cm" draw:distance="0.318cm"/>"#,
    r#"<draw:stroke-dash draw:name="Dash_20_Dot" draw:style="round" draw:dots1="1" draw:dots1-length="200%" "#,
    r#"draw:dots2="1" draw:distance="100%"/>"#,
    r##"<draw:gradient draw:name="Axis" draw:style="axial" draw:start-color="#000000" draw:end-color="#ffffff" draw:angle="0"/>"##,
    r#"<draw:fill-image draw:name="Tile" xlink:href="Pictures/tile.png" xlink:type="simple" xlink:show="embed"/>"#,
    r#"<style:style style:name="Object_20_with_20_no_20_fill_20_and_20_no_20_line" style:family="graphic" "#,
    r#"style:parent-style-name="standard"><style:graphic-properties draw:stroke="none" draw:fill="none"/></style:style>"#,
);

fn shape_automatic() -> String {
    let none = "Object_20_with_20_no_20_fill_20_and_20_no_20_line";
    [
        graphic(
            "gr1",
            r##"draw:fill="solid" draw:fill-color="#f14509" draw:opacity="50%" draw:stroke="none""##,
        ),
        graphic(
            "gr2",
            concat!(
                r#"draw:stroke="dash" draw:stroke-dash="Dashed_20__28_var_29__20_4" svg:stroke-width="0.106cm" "#,
                r##"svg:stroke-color="#595959" draw:marker-end="msArrowEnd_20_5" draw:marker-end-width="0.318cm" "##,
                r#"draw:marker-start="Circle" draw:marker-start-width="0.5cm" draw:stroke-linejoin="miter" draw:fill="none""#
            ),
        ),
        graphic(
            "gr3",
            r##"draw:stroke="solid" svg:stroke-width="0cm" svg:stroke-color="#ff0000" draw:fill="gradient" draw:fill-gradient-name="Axis" draw:shadow="visible""##,
        ),
        graphic(
            "gr4",
            r#"draw:fill="bitmap" draw:fill-image-name="Tile" style:repeat="repeat" draw:stroke="dash" draw:stroke-dash="Dash_20_Dot" svg:stroke-width="2pt" svg:stroke-opacity="50%" svg:stroke-linecap="square""#,
        ),
        graphic_under(
            "gr5",
            concat!(
                r#"draw:stroke="none" draw:fill="none" fo:clip="rect(0in, 0.25in, 0in, 0.25in)" draw:image-opacity="50%" "#,
                r#"style:mirror="horizontal" draw:color-mode="greyscale" draw:luminance="10%""#
            ),
            none,
        ),
        graphic_under("gr6", r#"fo:clip="rect(0in, 0.5in, 0in, 0.5in)""#, none),
        graphic_under("gr7", r#"fo:clip="rect(1pt, 2pt, 3pt, 4pt)""#, none),
    ]
    .concat()
}

/// Custom shapes, basic shapes, lines and connectors, groups, links, pictures, media, objects and transforms.
pub fn shapes() -> Vec<u8> {
    let tree = [
        custom("Rect", "gr1", &place("1in", "0.5in", "2in", "1in"), r#"draw:type="ooxml-rect" draw:mirror-horizontal="false""#),
        custom(
            "Rounded",
            "gr3",
            &format!(
                r#"{} draw:transform="rotate (-1.5707963267949) translate (4in 3in)""#,
                place("0cm", "0cm", "2in", "1in")
            ),
            r#"draw:type="ooxml-roundRect" draw:modifiers="30000""#,
        ),
        custom(
            "Arrow",
            "gr2",
            &place("5in", "1in", "2in", "1in"),
            r#"draw:type="mso-spt32" draw:mirror-horizontal="true" svg:viewBox="0 0 21600 21600""#,
        ),
        custom("Bent", "gr2", &place("5in", "2.5in", "2in", "1in"), r#"draw:type="ooxml-bentConnector3" draw:modifiers="25000""#),
        custom("Native", "gr4", &place("0.2in", "4in", "1in", "1in"), r#"draw:type="round-rectangle" draw:modifiers="3600""#),
        custom(
            "Freeform",
            "gr1",
            &place("1.5in", "4in", "1in", "1in"),
            r#"draw:type="non-primitive" draw:enhanced-path="M 0 0 L 10 10 Z N""#,
        ),
        format!(r#"<draw:rect draw:name="Corner" draw:style-name="gr1" {} draw:corner-radius="0.125in"/>"#, place("3in", "4in", "1in", "0.5in")),
        format!(
            r#"<draw:ellipse draw:name="Pie" draw:style-name="gr1" {} draw:kind="section" draw:start-angle="90" draw:end-angle="180"/>"#,
            place("4.5in", "4in", "1in", "1in")
        ),
        r#"<draw:line draw:name="Back" draw:style-name="gr2" svg:x1="9in" svg:y1="1in" svg:x2="8in" svg:y2="0.5in"/>"#.to_owned(),
        format!(
            r#"<draw:polygon draw:name="Tri" draw:style-name="gr1" {} svg:viewBox="100 100 1000 1000" draw:points="100,1100 600,100 1100,1100"/>"#,
            place("6in", "4in", "1in", "1in")
        ),
        format!(
            r#"<draw:path draw:name="Wave" draw:style-name="gr2" {} svg:viewBox="0 0 100 100" svg:d="M0 50 C25 0 75 100 100 50 a10 10 0 0 1 -20 0 Z"/>"#,
            place("7.5in", "4in", "1in", "1in")
        ),
        r#"<draw:connector draw:name="Routed" draw:style-name="gr2" draw:type="standard" svg:x1="1in" svg:y1="3in" svg:x2="2in" svg:y2="3.5in"/>"#.to_owned(),
        format!(
            r#"<draw:g draw:name="Pair">{}{}</draw:g>"#,
            custom("Left", "gr1", &place("6in", "2.5in", "0.5in", "0.5in"), r#"draw:type="ooxml-ellipse""#),
            custom("Right", "gr1", &place("7in", "3in", "0.5in", "0.5in"), r#"draw:type="ooxml-rect""#)
        ),
        format!(
            r#"<draw:a xlink:href="https://example.com/a?b=1">{}</draw:a>"#,
            custom("Linked", "gr1", &place("8in", "2in", "1in", "0.5in"), r#"draw:type="ooxml-rect""#)
        ),
        format!(
            concat!(
                r#"<draw:custom-shape draw:name="Next" draw:style-name="gr1" xml:id="next" {}>"#,
                r#"<office:event-listeners><presentation:event-listener script:event-name="dom:click" presentation:action="next-page"/>"#,
                r#"</office:event-listeners><svg:title>Next slide</svg:title><svg:desc>Goes on</svg:desc>"#,
                r#"<draw:enhanced-geometry draw:type="ooxml-rect"/></draw:custom-shape>"#
            ),
            place("8in", "3in", "1in", "0.5in")
        ),
        format!(
            r#"<draw:custom-shape draw:name="Again" draw:style-name="gr1" xml:id="next" draw:display="none" {}><draw:enhanced-geometry draw:type="ooxml-rect"/></draw:custom-shape>"#,
            place("8in", "4in", "1in", "0.5in")
        ),
        r#"<draw:custom-shape draw:name="Skewed" draw:style-name="gr1" svg:width="1in" svg:height="1in" draw:transform="skewX (0.3) translate (1in 1in)"><draw:enhanced-geometry draw:type="ooxml-rect"/></draw:custom-shape>"#.to_owned(),
        r#"<draw:custom-shape draw:name="Garbled" draw:style-name="gr1" svg:width="1in" svg:height="1in" draw:transform="rotate (x)"><draw:enhanced-geometry draw:type="ooxml-rect"/></draw:custom-shape>"#.to_owned(),
        r#"<dr3d:scene draw:name="Cube" svg:x="0in" svg:y="5in" svg:width="1in" svg:height="0.5in"/>"#.to_owned(),
    ]
    .concat();
    let media = [
        image("Photo", "gr5", &place("0.5in", "0.5in", "1in", "0.5in"), "Pictures/photo.png", r#" xml:id="photo""#),
        image("Remote", "gr6", &place("2in", "0.5in", "1in", "0.5in"), "https://example.com/a.png", ""),
        image("Gone", "gr6", &place("3.5in", "0.5in", "1in", "0.5in"), "Pictures/missing.png", ""),
        image("Sliver", "gr6", &place("5in", "0.5in", "1in", "0.5in"), "Pictures/photo.png", ""),
        image("Vector", "gr7", &place("6.5in", "0.5in", "1in", "0.5in"), "Pictures/drawing.svg", ""),
        concat!(
            r#"<draw:frame draw:name="Chart" draw:style-name="gr6" svg:x="0.5in" svg:y="2in" svg:width="2in" svg:height="1in">"#,
            r#"<draw:object xlink:href="./Object 1" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/>"#,
            r#"<draw:image xlink:href="./ObjectReplacements/Object 1" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/></draw:frame>"#
        )
        .to_owned(),
        concat!(
            r#"<draw:frame draw:name="Grid" draw:style-name="gr6" svg:x="3in" svg:y="2in" svg:width="2in" svg:height="1in">"#,
            r#"<table:table><table:table-row><table:table-cell/></table:table-row></table:table></draw:frame>"#
        )
        .to_owned(),
        concat!(
            r#"<draw:frame draw:name="Clip" draw:style-name="gr6" svg:x="5.5in" svg:y="2in" svg:width="2in" svg:height="1in">"#,
            r#"<draw:plugin xlink:href="Media/clip.mp4" xlink:type="simple" xlink:show="embed" draw:mime-type="video/mp4">"#,
            r#"<draw:param draw:name="Loop" draw:value="true"/><draw:param draw:name="Mute" draw:value="false"/></draw:plugin>"#,
            r#"<draw:image xlink:href="Pictures/poster.png" xlink:type="simple" xlink:show="embed"/></draw:frame>"#
        )
        .to_owned(),
        concat!(
            r#"<draw:frame draw:name="Online" draw:style-name="gr6" svg:x="5.5in" svg:y="3.5in" svg:width="2in" svg:height="1in">"#,
            r#"<draw:plugin xlink:href="https://example.com/watch" xlink:type="simple" draw:mime-type="application/vnd.sun.star.media"/>"#,
            r#"<draw:image xlink:href="Pictures/poster.png" xlink:type="simple" xlink:show="embed"/></draw:frame>"#
        )
        .to_owned(),
        concat!(
            r#"<draw:frame draw:name="Sound" draw:style-name="gr6" svg:x="8in" svg:y="3.5in" svg:width="0.5in" svg:height="0.5in">"#,
            r#"<draw:plugin xlink:href="Media/sound.wav" xlink:type="simple" draw:mime-type="audio/wav"/>"#,
            r#"<draw:image xlink:href="Pictures/photo.png" xlink:type="simple" xlink:show="embed"/></draw:frame>"#
        )
        .to_owned(),
    ]
    .concat();
    let spec = OdpSpec {
        styles: SHAPE_STYLES.to_owned(),
        automatic_styles: shape_automatic(),
        master_automatic_styles: graphic("Mgr1", r#"draw:fill="none" draw:stroke="none""#),
        masters: Some(vec![MasterSpec::named("Default").body(format!(
            concat!(
                "{}",
                r#"<draw:frame presentation:class="title" presentation:placeholder="true" svg:x="0.5in" svg:y="0.2in" "#,
                r#"svg:width="9in" svg:height="1in"><draw:text-box/></draw:frame>"#
            ),
            image("Logo", "Mgr1", &place("9in", "0.1in", "0.5in", "0.5in"), "Pictures/photo.png", "")
        ))]),
        pages: vec![PageSpec::body(tree), PageSpec::body(media)],
        manifest: concat!(
            r#"<manifest:file-entry manifest:full-path="Object 1/" manifest:version="1.2" "#,
            r#"manifest:media-type="application/vnd.oasis.opendocument.chart"/>"#
        )
        .to_owned(),
        ..OdpSpec::default()
    }
    .part("Pictures/photo.png", png(96, 48))
    .part("Pictures/tile.png", png(4, 2))
    .part("Pictures/poster.png", png(8, 4))
    // An SVG that states no size, which the output does not show (`slidock_io::check_svg`).
    .part(
        "Pictures/drawing.svg",
        r#"<svg xmlns="http://www.w3.org/2000/svg"/>"#,
    )
    .part("ObjectReplacements/Object 1", png(16, 8))
    .part("Object 1/content.xml", "<office:document-content/>")
    .part("Media/clip.mp4", movie("vide", "avc1", true, None))
    .part("Media/sound.wav", b"RIFF\0\0\0\0WAVEfmt ".to_vec());
    odp(&spec)
}

fn paragraph_style(name: &str, paragraph: &str, text: &str, parent: Option<&str>) -> String {
    let parent = parent
        .map(|p| format!(r#" style:parent-style-name="{p}""#))
        .unwrap_or_default();
    let paragraph = if paragraph.is_empty() {
        String::new()
    } else {
        format!("<style:paragraph-properties {paragraph}/>")
    };
    let text = if text.is_empty() {
        String::new()
    } else {
        format!("<style:text-properties {text}/>")
    };
    format!(
        r#"<style:style style:name="{name}" style:family="paragraph"{parent}>{paragraph}{text}</style:style>"#
    )
}

fn text_style(name: &str, text: &str) -> String {
    format!(
        r#"<style:style style:name="{name}" style:family="text"><style:text-properties {text}/></style:style>"#
    )
}

fn outline(level: u32, paragraph: &str, text: &str) -> String {
    let parent = if level > 1 {
        format!(r#" style:parent-style-name="Content-outline{}""#, level - 1)
    } else {
        String::new()
    };
    format!(
        r#"<style:style style:name="Content-outline{level}" style:family="presentation"{parent}><style:paragraph-properties {paragraph}/><style:text-properties {text}/></style:style>"#
    )
}

fn text_styles() -> String {
    [
        concat!(
            r#"<style:style style:name="Content-title" style:family="presentation"><style:graphic-properties draw:stroke="none" "#,
            r#"draw:fill="none" draw:textarea-vertical-align="middle" fo:padding-left="0.254cm" fo:padding-right="0.254cm" "#,
            r#"fo:padding-top="0.254cm" fo:padding-bottom="0.254cm"/><style:paragraph-properties fo:text-align="center"/>"#,
            r##"<style:text-properties style:font-name="Liberation Sans" fo:font-size="44pt" fo:font-weight="bold" fo:color="#1f3864"/>"##,
            "</style:style>"
        )
        .to_owned(),
        outline(
            1,
            r#"fo:margin-top="0.5cm" fo:line-height="90%""#,
            r##"fo:font-size="28pt" fo:color="#000000" style:font-name="Liberation Sans""##,
        ),
        outline(2, r#"fo:margin-top="0.4cm""#, r#"fo:font-size="80%""#),
        concat!(
            r#"<text:list-style style:name="Outline"><text:list-level-style-bullet text:level="1" text:bullet-char="●">"#,
            r#"<style:list-level-properties text:space-before="0.3cm" text:min-label-width="0.9cm"/>"#,
            r#"<style:text-properties fo:font-family="StarSymbol" style:use-window-font-color="true" fo:font-size="45%"/>"#,
            r#"</text:list-level-style-bullet><text:list-level-style-bullet text:level="2" text:bullet-char="–">"#,
            r#"<style:list-level-properties text:space-before="1.5cm" text:min-label-width="0.9cm"/>"#,
            r##"<style:text-properties fo:font-family="StarSymbol" fo:color="#ff0000" fo:font-size="12pt"/>"##,
            "</text:list-level-style-bullet></text:list-style>",
            r#"<draw:marker draw:name="unused" svg:viewBox="0 0 1 1" svg:d="M0 0"/>"#
        )
        .to_owned(),
    ]
    .concat()
}

fn text_automatic() -> String {
    [
        graphic(
            "gr1",
            concat!(
                r#"draw:stroke="none" draw:fill="none" draw:textarea-vertical-align="bottom" draw:auto-grow-height="true" "#,
                r#"fo:padding="0.1in" fo:wrap-option="no-wrap" draw:shadow="hidden""#
            ),
        ),
        graphic(
            "gr2",
            r#"draw:fill="none" draw:stroke="none" draw:auto-grow-width="true" draw:textarea-horizontal-align="center""#,
        ),
        graphic("gr3", r#"draw:fill="none" draw:stroke="none" draw:fit-to-size="shrink-to-fit""#),
        concat!(
            r#"<style:style style:name="gr4" style:family="graphic"><style:graphic-properties draw:fill="none" draw:stroke="none">"#,
            r#"<style:columns fo:column-count="2"/></style:graphic-properties></style:style>"#
        )
        .to_owned(),
        page_style(
            "dp1",
            r#"presentation:display-page-number="true" presentation:display-footer="true" presentation:display-date-time="true""#,
        ),
        page_style(
            "dp2",
            r#"presentation:display-page-number="false" presentation:display-footer="false""#,
        ),
        concat!(
            r#"<style:style style:name="pr1" style:family="presentation" style:parent-style-name="Content-title">"#,
            r#"<style:graphic-properties draw:fit-to-size="shrink-to-fit" style:shrink-to-fit="true"/></style:style>"#,
            r#"<style:style style:name="pr2" style:family="presentation" style:parent-style-name="Content-outline1" "#,
            r#"style:list-style-name="Outline"/>"#
        )
        .to_owned(),
        paragraph_style(
            "P1",
            r#"fo:text-align="end" fo:margin-left="0.5in" fo:text-indent="-0.25in" fo:margin-bottom="6pt""#,
            r#"fo:font-size="150%""#,
            None,
        ),
        paragraph_style("P2", r#"fo:line-height="0.5in" style:writing-mode="rl-tb""#, "", None),
        paragraph_style(
            "P3",
            r#"style:line-height-at-least="20pt" style:line-spacing="2pt" style:writing-mode="tb-rl""#,
            "",
            None,
        ),
        paragraph_style("P4", r#"fo:text-align="justify""#, "", Some("Base")),
        concat!(
            r#"<style:style style:name="P5" style:family="paragraph"><style:paragraph-properties><style:tab-stops>"#,
            r#"<style:tab-stop style:position="2cm"/></style:tab-stops></style:paragraph-properties></style:style>"#
        )
        .to_owned(),
        paragraph_style("Base", "", r##"fo:font-size="20pt" fo:color="#336699""##, None),
        text_style(
            "T1",
            r#"fo:font-weight="bold" fo:font-style="italic" style:text-underline-style="solid" style:text-underline-type="double""#,
        ),
        text_style(
            "T2",
            r#"style:text-line-through-style="solid" style:text-position="super 58%" fo:letter-spacing="0.05cm""#,
        ),
        text_style(
            "T3",
            r##"fo:background-color="#ffff00" fo:font-variant="small-caps" style:font-name="Brand""##,
        ),
        text_style(
            "T4",
            r#"fo:text-transform="uppercase" fo:font-family="&apos;Roboto Medium&apos;, sans-serif" fo:font-size="50%""#,
        ),
        text_style(
            "T5",
            r#"style:text-underline-style="wave" fo:text-transform="capitalize" fo:text-shadow="1pt 1pt" style:text-outline="true" style:use-window-font-color="true""#,
        ),
        concat!(
            r#"<text:list-style style:name="L1"><text:list-level-style-number text:level="1" style:num-prefix="(" "#,
            r#"style:num-suffix=")" style:num-format="a" text:start-value="3"><style:list-level-properties "#,
            r#"text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment "#,
            r#"text:label-followed-by="listtab" fo:margin-left="0.5in" fo:text-indent="-0.25in"/></style:list-level-properties>"#,
            r##"<style:text-properties fo:color="#00ff00"/></text:list-level-style-number>"##,
            r#"<text:list-level-style-number text:level="2" style:num-format="I" style:num-suffix="-">"#,
            r#"<style:list-level-properties text:space-before="0.5in" text:min-label-width="0.25in"/></text:list-level-style-number>"#,
            r#"<text:list-level-style-image text:level="3" xlink:href="Pictures/bullet.png">"#,
            r#"<style:list-level-properties text:min-label-width="0.25in"/></text:list-level-style-image></text:list-style>"#
        )
        .to_owned(),
    ]
    .concat()
}

pub fn p(content: &str, style: Option<&str>) -> String {
    let style = style
        .map(|s| format!(r#" text:style-name="{s}""#))
        .unwrap_or_default();
    format!("<text:p{style}>{content}</text:p>")
}

fn span(style: &str, content: &str) -> String {
    format!(r#"<text:span text:style-name="{style}">{content}</text:span>"#)
}

/// Paragraph and run properties through style chains, white space, fields, links, lists and outlines.
pub fn text() -> Vec<u8> {
    let runs = [
        p(
            &format!(
                "  Plain   text\n with {} and {}",
                span("T1", "bold italic"),
                span("T2", r#"struck<text:s text:c="3"/>raised"#)
            ),
            Some("P1"),
        ),
        p(
            &format!(
                "{}<text:tab/>{}<text:line-break/>{}",
                span("T3", "marked"),
                span("T4", "loud"),
                span("T5", "waved")
            ),
            Some("P2"),
        ),
        p(
            concat!(
                "Page <text:page-number>4</text:page-number> of <text:page-count>9</text:page-count>, ",
                r#"<text:date text:date-value="2026-10-13">13/10/2026</text:date> <text:title>Presentation title</text:title> "#,
                r#"<text:a xlink:href="https://example.com" xlink:type="simple">site</text:a> "#,
                r##"<text:a xlink:href="#Opening" xlink:type="simple">first</text:a>"##
            ),
            Some("P3"),
        ),
        p("", Some("P4")),
        p(r#"<text:span text:style-name="T1">Tabbed</text:span>"#, Some("P5")),
    ]
    .concat();
    let list = [
        r#"<text:list text:style-name="L1"><text:list-item>"#.to_owned(),
        p("Third", None),
        "</text:list-item><text:list-item>".to_owned(),
        p("Fourth", None),
        p("Continued", None),
        "<text:list><text:list-item>".to_owned(),
        p("Nested", None),
        "</text:list-item><text:list-item><text:list><text:list-item>".to_owned(),
        p("Picture", None),
        "</text:list-item></text:list></text:list-item></text:list></text:list-item>".to_owned(),
        r#"<text:list-item text:start-value="7">"#.to_owned(),
        p("Seventh", None),
        "</text:list-item><text:list-header>".to_owned(),
        p("Header", None),
        "</text:list-header></text:list>".to_owned(),
    ]
    .concat();
    let notes = |text: &str| {
        format!(
            r#"<draw:frame presentation:class="notes" svg:x="1in" svg:y="5in" svg:width="6in" svg:height="4in"><draw:text-box>{}</draw:text-box></draw:frame>"#,
            p(text, None)
        )
    };
    let master = [
        concat!(
            r#"<draw:frame presentation:style-name="Content-title" presentation:class="title" presentation:placeholder="true" "#,
            r#"svg:x="0.5in" svg:y="0.2in" svg:width="9in" svg:height="1in"><draw:text-box/></draw:frame>"#,
            r#"<draw:frame draw:style-name="Mgr1" presentation:class="page-number" svg:x="9in" svg:y="5.2in" "#,
            r#"svg:width="0.8in" svg:height="0.3in"><draw:text-box>"#
        )
        .to_owned(),
        p("<text:page-number>&lt;number&gt;</text:page-number>", Some("MP1")),
        concat!(
            "</draw:text-box></draw:frame>",
            r#"<draw:frame draw:style-name="Mgr1" presentation:class="footer" svg:x="0.5in" svg:y="5.2in" "#,
            r#"svg:width="4in" svg:height="0.3in"><draw:text-box>"#
        )
        .to_owned(),
        p("<presentation:footer/>", Some("MP1")),
        concat!(
            "</draw:text-box></draw:frame>",
            r#"<draw:frame draw:style-name="Mgr1" presentation:class="date-time" svg:x="5in" svg:y="5.2in" "#,
            r#"svg:width="3in" svg:height="0.3in"><draw:text-box>"#
        )
        .to_owned(),
        p("<presentation:date-time/>", Some("MP1")),
        "</draw:text-box></draw:frame>".to_owned(),
    ]
    .concat();
    let opening = [
        format!(
            r#"<draw:frame presentation:style-name="pr1" presentation:class="title" svg:x="0.5in" svg:y="0.2in" svg:width="9in" svg:height="1in"><draw:text-box>{}</draw:text-box></draw:frame>"#,
            p("A title", None)
        ),
        r#"<draw:frame presentation:style-name="pr2" presentation:class="outline" svg:x="0.5in" svg:y="1.4in" svg:width="9in" svg:height="3in"><draw:text-box>"#.to_owned(),
        "<text:list><text:list-item>".to_owned(),
        p("Point", None),
        "<text:list><text:list-item>".to_owned(),
        p("Detail", None),
        "</text:list-item></text:list></text:list-item></text:list>".to_owned(),
        p("Unbulleted", None),
        "</draw:text-box></draw:frame>".to_owned(),
    ]
    .concat();
    let second = [
        format!(
            r#"<draw:custom-shape draw:name="Runs" draw:style-name="gr1" draw:text-style-name="Base" {}>{runs}<draw:enhanced-geometry draw:type="ooxml-rect"/></draw:custom-shape>"#,
            place("0.5in", "0.5in", "6in", "2in")
        ),
        format!(
            r#"<draw:custom-shape draw:name="List" draw:style-name="gr2" {}>{list}<draw:enhanced-geometry draw:type="rectangle"/></draw:custom-shape>"#,
            place("0.5in", "3in", "4in", "2in")
        ),
        format!(
            r#"<draw:rect draw:name="Shrunk" draw:style-name="gr3" {}>{}</draw:rect>"#,
            place("5in", "3in", "2in", "1in"),
            p("Fits", None)
        ),
        format!(
            r#"<draw:rect draw:name="Columns" draw:style-name="gr4" {}>{}</draw:rect>"#,
            place("7.5in", "3in", "2in", "1in"),
            p("Two", None)
        ),
        format!(
            r#"<draw:frame draw:name="Empty" draw:style-name="gr3" {}><draw:text-box/></draw:frame>"#,
            place("5in", "4.5in", "2in", "0.5in")
        ),
    ]
    .concat();
    let spec = OdpSpec {
        styles: text_styles(),
        automatic_styles: text_automatic(),
        font_faces: r#"<style:font-face style:name="Brand" svg:font-family="&apos;Barlow Condensed&apos;"/>"#.to_owned(),
        master_automatic_styles: [
            graphic("Mgr1", r#"draw:fill="none" draw:stroke="none""#),
            paragraph_style("MP1", r#"fo:text-align="end""#, r#"fo:font-size="10pt""#, None),
        ]
        .concat(),
        masters: Some(vec![MasterSpec::named("Content").body(master)]),
        pages: vec![
            PageSpec::new()
                .name("Opening")
                .master("Content")
                .style("dp1")
                .attrs(r#" presentation:use-footer-name="ftr1" presentation:use-date-time-name="dtd1""#)
                .with_body(opening)
                .notes(notes("Say hello")),
            PageSpec::new()
                .master("Content")
                .style("dp2")
                .with_body(second)
                .notes(notes("")),
        ],
        declarations: concat!(
            r#"<presentation:footer-decl presentation:name="ftr1">Company</presentation:footer-decl>"#,
            r#"<presentation:date-time-decl presentation:name="dtd1" presentation:source="fixed">13 October</presentation:date-time-decl>"#
        )
        .to_owned(),
        ..OdpSpec::default()
    }
    .part("Pictures/bullet.png", png(4, 2));
    odp(&spec)
}

fn cell_style(name: &str, props: &str, parent: Option<&str>) -> String {
    let parent = parent
        .map(|p| format!(r#" style:parent-style-name="{p}""#))
        .unwrap_or_default();
    format!(
        r#"<style:style style:name="{name}" style:family="table-cell"{parent}>{props}</style:style>"#
    )
}

fn cell_props(props: &str) -> String {
    format!("<style:table-cell-properties {props}/>")
}

fn track_style(name: &str, family: &str, props: &str) -> String {
    format!(
        r#"<style:style style:name="{name}" style:family="table-{family}"><style:table-{family}-properties {props}/></style:style>"#
    )
}

fn table_styles() -> String {
    [
        r##"<draw:gradient draw:name="Fade" draw:style="linear" draw:start-color="#ffffff" draw:end-color="#000000" draw:angle="0"/>"##.to_owned(),
        cell_style(
            "Head",
            &format!(
                "{}{}",
                cell_props(r##"fo:background-color="#003366" fo:border="1pt solid #000000""##),
                r##"<style:text-properties fo:color="#ffffff" fo:font-weight="bold"/>"##
            ),
            None,
        ),
        cell_style("Band", &cell_props(r##"fo:background-color="#dddddd""##), None),
        cell_style("Side", &cell_props(r##"fo:background-color="#ffcc00""##), None),
        cell_style(
            "Body",
            &cell_props(r##"fo:border="0.5pt solid #999999" fo:border-top="3pt solid #999999""##),
            None,
        ),
        concat!(
            r#"<table:table-template table:name="Plain"><table:first-row table:style-name="Head"/>"#,
            r#"<table:first-column table:style-name="Side"/><table:odd-rows table:style-name="Band"/>"#,
            r#"<table:body table:style-name="Body"/></table:table-template>"#,
            r#"<table:table-template text:style-name="Older"><table:body table:style-name="Band"/></table:table-template>"#
        )
        .to_owned(),
    ]
    .concat()
}

fn table_automatic() -> String {
    [
        track_style("co1", "column", r#"style:column-width="1in""#),
        track_style("co2", "column", r#"style:rel-column-width="1*""#),
        track_style("co3", "column", r#"style:rel-column-width="3*""#),
        track_style("ro1", "row", r#"style:row-height="0.5in""#),
        track_style("ro2", "row", r#"style:min-row-height="0.25in""#),
        cell_style(
            "ce1",
            &format!(
                "{}{}",
                cell_props(concat!(
                    r##"fo:background-color="#ff0000" fo:border="0.5pt solid #000000" fo:border-left="2pt dashed #0000ff" "##,
                    r#"fo:padding="0.1in" style:vertical-align="middle""#
                )),
                r##"<style:paragraph-properties fo:text-align="center"/><style:text-properties fo:font-size="12pt" fo:color="#333333"/>"##
            ),
            None,
        ),
        cell_style(
            "ce2",
            concat!(
                r#"<loext:graphic-properties draw:fill="gradient" draw:fill-gradient-name="Fade" draw:textarea-vertical-align="bottom" "#,
                r#"fo:padding-top="0.05in" fo:padding-bottom="0.05in" fo:padding-left="0.05in" fo:padding-right="0.05in"/>"#,
                r##"<style:paragraph-properties fo:border-top="1pt dotted #00ff00" fo:border-bottom="thin fine-dashed #00ff00" "##,
                r##"fo:border-left="none" fo:border-right="1pt dash-dot #000000"/>"##
            ),
            None,
        ),
        cell_style(
            "ce3",
            &cell_props(concat!(
                r##"fo:border="1.5pt double #000000" style:diagonal-tl-br="1pt solid #ff0000" "##,
                r##"style:diagonal-bl-tr="1pt dash-dot-dot #ff0000" fo:background-color="transparent" style:vertical-align="automatic""##
            )),
            None,
        ),
        cell_style("ce4", &cell_props(r##"fo:background-color="#00ff00""##), None),
        cell_style("ce5", &cell_props(r##"fo:border="0.75pt solid #123456""##), None),
        cell_style("ce6", &cell_props(r##"fo:background-color="#abcdef""##), None),
        cell_style("ce7", &cell_props(r#"fo:border-bottom="hidden""#), Some("Head")),
        paragraph_style("TP1", r#"fo:text-align="end""#, "", None),
        text_style("TT1", r##"fo:font-weight="bold" fo:color="#ff0000""##),
    ]
    .concat()
}

fn cell(content: &str, attrs: &str) -> String {
    format!("<table:table-cell{attrs}>{content}</table:table-cell>")
}

fn covered(content: &str) -> String {
    format!("<table:covered-table-cell>{content}</table:covered-table-cell>")
}

fn row(cells: &str, attrs: &str) -> String {
    format!("<table:table-row{attrs}>{cells}</table:table-row>")
}

fn table_frame(name: &str, place: &str, table: &str, extra: &str) -> String {
    format!(
        r#"<draw:frame draw:name="{name}" draw:style-name="standard" {place}>{table}{extra}</draw:frame>"#
    )
}

/// Grids, spans, repeats and track sizes; cell styles, templates and borders; cell text; tables the IR cannot hold.
pub fn tables() -> Vec<u8> {
    let c = |text: &str| p(text, None);
    let merges = [
        concat!(
            r#"<table:table><table:table-column table:style-name="co1" table:number-columns-repeated="3" "#,
            r#"table:default-cell-style-name="ce6"/><table:table-column/><table:table-rows>"#
        )
        .to_owned(),
        row(
            &[
                cell(&c("Header"), r#" table:number-columns-spanned="4""#),
                covered(""),
                covered(""),
                covered(""),
            ]
            .concat(),
            r#" table:style-name="ro1" table:default-cell-style-name="ce5""#,
        ),
        row(
            &[
                cell(&c("Tall"), r#" table:number-rows-spanned="2""#),
                cell(&c("Block"), r#" table:number-columns-spanned="2" table:number-rows-spanned="2""#),
                covered(""),
                cell(&c("Side"), ""),
            ]
            .concat(),
            r#" table:style-name="ro1""#,
        ),
        "</table:table-rows>".to_owned(),
        row(
            &[covered(""), covered(""), covered(""), cell(&c("Under"), "")].concat(),
            r#" table:style-name="ro1""#,
        ),
        row(
            &cell(&c("Rep"), r#" table:number-columns-repeated="2""#),
            r#" table:style-name="ro2" table:number-rows-repeated="2""#,
        ),
        row(
            &[covered(&c("Orphan")), cell(&c("Wide"), r#" table:number-columns-spanned="9""#)].concat(),
            "",
        ),
        row(&cell(&c("Down"), r#" table:number-rows-spanned="5""#), ""),
        "</table:table>".to_owned(),
    ]
    .concat();
    let ro1 = r#" table:style-name="ro1""#;
    let styled = [
        concat!(
            r#"<table:table table:template-name="Plain" table:use-first-row-styles="true" "#,
            r#"table:use-first-column-styles="true" table:use-banding-rows-styles="true">"#,
            r#"<table:table-column table:style-name="co2"/><table:table-column table:style-name="co3"/>"#,
            r#"<table:table-column table:style-name="co2"/>"#
        )
        .to_owned(),
        row(&[cell(&c("Name"), ""), cell(&c("Value"), ""), cell(&c("Note"), "")].concat(), ro1),
        row(
            &[cell(&c("First"), ""), cell(&c("1"), ""), cell(&c("own fill"), r#" table:style-name="ce4""#)].concat(),
            ro1,
        ),
        row(
            &[cell(&c("Second"), ""), cell(&c("2"), ""), cell(&c("no bottom"), r#" table:style-name="ce7""#)].concat(),
            ro1,
        ),
        row(
            &[
                cell(
                    &format!(
                        "{}<text:list><text:list-item>{}</text:list-item></text:list>",
                        p(&format!("Plain {}", span("TT1", "bold red")), Some("TP1")),
                        c("Item")
                    ),
                    r#" table:style-name="ce1""#,
                ),
                cell("", r#" table:style-name="ce2""#),
                cell(&c(""), r#" table:style-name="ce3""#),
            ]
            .concat(),
            ro1,
        ),
        "</table:table>".to_owned(),
    ]
    .concat();
    let one = |attrs: &str, text: &str| {
        format!(
            "<table:table{attrs}><table:table-column/>{}</table:table>",
            row(&cell(&c(text), ""), "")
        )
    };
    let clash = [
        r#"<table:table><table:table-column table:number-columns-repeated="3"/>"#.to_owned(),
        row(
            &[
                cell(&c("a"), ""),
                cell(&c("b"), ""),
                cell(&c("A"), r#" table:number-rows-spanned="2""#),
            ]
            .concat(),
            "",
        ),
        row(
            &[
                cell(&c("B"), r#" table:number-columns-spanned="3""#),
                covered(""),
                covered(&c("hidden")),
            ]
            .concat(),
            "",
        ),
        "</table:table>".to_owned(),
    ]
    .concat();
    let replacement_image = r#"<draw:image xlink:href="Pictures/preview.png" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/>"#;
    let spec = OdpSpec {
        styles: table_styles(),
        automatic_styles: table_automatic(),
        pages: vec![
            PageSpec::body(table_frame("Merges", &place("1in", "1in", "5in", "3in"), &merges, replacement_image)),
            PageSpec::body(
                [
                    table_frame("Styled", &place("1in", "0.5in", "5in", "2in"), &styled, ""),
                    table_frame(
                        "Missing",
                        &place("1in", "3in", "2in", "0.5in"),
                        &one(r#" table:template-name="Gone""#, "none"),
                        "",
                    ),
                    table_frame(
                        "Older",
                        &place("4in", "3in", "2in", "0.5in"),
                        &one(r#" table:template-name="Older""#, "banded"),
                        "",
                    ),
                ]
                .concat(),
            ),
            PageSpec::body(
                [
                    table_frame(
                        "Huge",
                        &place("0.5in", "0.5in", "4in", "1in"),
                        &format!(
                            r#"<table:table><table:table-column table:number-columns-repeated="1001"/>{}</table:table>"#,
                            row(&cell(&c("x"), ""), "")
                        ),
                        replacement_image,
                    ),
                    table_frame("Hollow", &place("5in", "0.5in", "2in", "1in"), "<table:table/>", ""),
                    r#"<draw:g draw:name="Grouped">"#.to_owned(),
                    table_frame("Clash", &place("1in", "2in", "3in", "1in"), &clash, ""),
                    format!(
                        r#"<draw:rect draw:name="Beside" draw:style-name="standard" {}/>"#,
                        place("5in", "2in", "1in", "1in")
                    ),
                    "</draw:g>".to_owned(),
                ]
                .concat(),
            ),
        ],
        ..OdpSpec::default()
    }
    .part("Pictures/preview.png", png(4, 2));
    odp(&spec)
}

/// A fixture's builder.
pub type Build = fn() -> Vec<u8>;

/// Every fixture, by golden name.
pub fn all() -> Vec<(&'static str, Build)> {
    vec![
        ("basic", basic as Build),
        ("shapes", shapes),
        ("text", text),
        ("tables", tables),
    ]
}
