//! Shadows as LibreOffice 25.2 draws them (`probes/out/shadow-probe.odp`, `shadow-probe-odp.odp`): `draw:shadow` with
//! its offset, colour, opacity and `loext:shadow-blur` on shapes, pictures and placeholders; the equal-transparency
//! rule; the graphic family's base (the file's "standard", else LibreOffice's own) against the presentation family's;
//! `fo:text-shadow` as LibreOffice's fixed character shadow; and the carriers it is not drawn on (OD9104).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde_json::{Value, json};
use support::fixtures::{graphic, place};
use support::odp::{MasterSpec, OdpSpec, PageSpec, odp};
use support::{Lowered, js, lower};

fn rect(name: &str, style: &str) -> String {
    format!(
        r#"<draw:rect draw:name="{name}" draw:style-name="{style}" {}/>"#,
        place("1in", "1in", "2in", "1in")
    )
}

fn deck(automatic: &str, body: &str) -> Lowered {
    lower(&odp(&OdpSpec {
        automatic_styles: automatic.to_owned(),
        ..OdpSpec::pages(vec![PageSpec::body(body)])
    }))
}

fn codes(lowered: &Lowered, code: &str) -> Vec<(String, Value)> {
    lowered
        .diagnostics
        .iter()
        .filter(|d| d.code.code() == code)
        .map(|d| (d.node.as_ref().unwrap().as_str().to_owned(), js(&d.data)))
        .collect()
}

#[test]
fn lowers_the_offset_colour_opacity_and_blur_page_fixed() {
    let lowered = deck(
        &[
            graphic(
                "gr1",
                r##"draw:shadow="visible" draw:shadow-offset-x="20pt" draw:shadow-offset-y="-0.5cm" draw:shadow-color="#ff0000" draw:shadow-opacity="40%" loext:shadow-blur="10pt""##,
            ),
            graphic("gr2", r#"draw:shadow="hidden" draw:shadow-offset-x="1cm""#),
            graphic("gr3", r#"draw:shadow="visible" draw:shadow-opacity="0%""#),
        ]
        .concat(),
        &[rect("Shown", "gr1"), rect("Hidden", "gr2"), rect("Clear", "gr3")].concat(),
    );
    assert_eq!(
        lowered.named(0, "Shown")["shadow"],
        js(&json!({
            "dx": 20, "dy": -14.173, "sigma": 4.3, "overflowCasts": true,
            "color": { "kind": "rgb", "space": "srgb", "r": 1, "g": 0, "b": 0, "a": 0.4 }
        }))
    );
    assert_eq!(lowered.named(0, "Hidden").get("shadow"), None);
    assert_eq!(lowered.named(0, "Clear").get("shadow"), None);
    assert!(lowered.features("od-p1").contains(&"effect.shadow"));
    assert!(codes(&lowered, "OD9104").is_empty());
}

#[test]
fn draws_a_shadow_as_transparent_as_its_fill_opaque() {
    let lowered = deck(
        &[
            graphic(
                "gr1",
                r#"draw:fill="solid" draw:opacity="50%" draw:shadow="visible" draw:shadow-opacity="50%""#,
            ),
            graphic(
                "gr2",
                r#"draw:fill="solid" draw:opacity="50%" draw:shadow="visible" draw:shadow-opacity="40%""#,
            ),
        ]
        .concat(),
        &[rect("Equal", "gr1"), rect("Other", "gr2")].concat(),
    );
    let alpha = |name| lowered.named(0, name)["shadow"]["color"]["a"].clone();
    assert_eq!(alpha("Equal"), json!(1));
    assert_eq!(alpha("Other"), json!(0.4));
}

#[test]
fn stands_graphic_styles_on_the_files_standard_and_presentation_styles_on_nothing() {
    // The test package's "standard" states no shadow keys: they stay at the pool defaults, black and no offset.
    let automatic = [
        r#"<style:style style:name="gr1" style:family="graphic"><style:graphic-properties draw:shadow="visible"/></style:style>"#,
        r#"<style:style style:name="pr1" style:family="presentation" style:parent-style-name="Default-title"><style:graphic-properties draw:shadow="visible" loext:shadow-blur="0.5cm"/></style:style>"#,
        r#"<style:style style:name="pr2" style:family="presentation" style:parent-style-name="Default-title"><style:graphic-properties draw:shadow="hidden"/></style:style>"#,
    ]
    .concat();
    let title = |style: &str, name: &str| {
        format!(
            r#"<draw:frame presentation:style-name="{style}" presentation:class="title" draw:name="{name}" {}><draw:text-box><text:p>T</text:p></draw:text-box></draw:frame>"#,
            place("1in", "1in", "4in", "1in")
        )
    };
    let lowered = lower(&odp(&OdpSpec {
        styles: concat!(
            r#"<style:style style:name="Default-title" style:family="presentation"><style:graphic-properties draw:fill="none"/></style:style>"#,
            r#"<style:style style:name="Hidden-title" style:family="presentation" style:parent-style-name="Default-title"><style:graphic-properties draw:shadow="hidden"/></style:style>"#,
        )
        .to_owned(),
        automatic_styles: automatic,
        masters: Some(vec![MasterSpec::named("Default").body(title("Hidden-title", "Master title"))]),
        ..OdpSpec::pages(vec![
            PageSpec::body([rect("Unparented", "gr1"), title("pr1", "Halo")].concat()),
            PageSpec::body(title("pr2", "No shadow")),
            PageSpec::body(title("Default-title", "Silent")),
        ])
    }));
    assert_eq!(
        lowered.named(0, "Unparented")["shadow"],
        js(&json!({
            "dx": 0, "dy": 0, "sigma": 0, "overflowCasts": true,
            "color": { "kind": "rgb", "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 1 }
        }))
    );
    assert_eq!(
        lowered.named(0, "Halo")["shadow"],
        js(&json!({
            "dx": 0, "dy": 0, "sigma": 6.094, "overflowCasts": true,
            "color": { "kind": "rgb", "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 1 }
        }))
    );
    // A slide's placeholder states a hidden shadow its chain states, so its layout's is not inherited; one whose
    // chain says nothing states none, nor does a layout's placeholder, which inherits from nothing.
    assert_eq!(
        lowered.named(1, "No shadow").get("shadow"),
        Some(&Value::Null)
    );
    assert_eq!(lowered.named(2, "Silent").get("shadow"), None);
    let layout = &lowered.json["layouts"]["od-m.Default"]["elements"][0];
    assert_eq!(layout["name"], "Master title");
    assert_eq!(layout.get("shadow"), None);
}

#[test]
fn reports_the_shadow_of_a_table_group_or_media_which_libreoffice_does_not_draw_or_slidock_cannot()
{
    let shadow = r#"draw:shadow="visible" draw:fill="none""#;
    let body = [
        format!(
            r#"<draw:frame draw:name="Table" draw:style-name="gr1" {}><table:table><table:table-column/><table:table-row><table:table-cell><text:p>A</text:p></table:table-cell></table:table-row></table:table></draw:frame>"#,
            place("1in", "1in", "2in", "1in")
        ),
        format!(
            r#"<draw:g draw:name="Group" draw:style-name="gr1">{}</draw:g>"#,
            rect("Inside", "gr2")
        ),
        format!(
            r#"<draw:frame draw:name="Clip" draw:style-name="gr1" {}><draw:plugin xlink:href="https://example.com/clip" xlink:type="simple" draw:mime-type="video/mp4"/></draw:frame>"#,
            place("1in", "3in", "2in", "1in")
        ),
    ]
    .concat();
    let lowered = deck(
        &[
            graphic("gr1", shadow),
            graphic("gr2", r#"draw:shadow="hidden""#),
        ]
        .concat(),
        &body,
    );
    let reasons: Vec<Value> = codes(&lowered, "OD9104")
        .into_iter()
        .map(|(_, data)| data["reason"].clone())
        .collect();
    assert_eq!(
        reasons,
        [
            json!("table shadow"),
            json!("group shadow"),
            json!("media shadow")
        ]
    );
    assert_eq!(lowered.named(0, "Group").get("shadow"), None);
    assert_eq!(lowered.named(0, "Group")["children"][0].get("shadow"), None);
    assert_eq!(lowered.named(0, "Clip").get("shadow"), None);
}

#[test]
fn draws_fo_text_shadow_as_libreoffices_character_shadow() {
    let text = |name: &str, props: &str| {
        format!(
            r#"<style:style style:name="{name}" style:family="text"><style:text-properties {props}/></style:style>"#
        )
    };
    let automatic = [
        graphic("gr1", r#"draw:fill="none" draw:stroke="none""#),
        graphic(
            "gr2",
            r##"draw:fill="none" draw:stroke="none""##,
        )
        .replace(
            "</style:style>",
            r#"<style:text-properties fo:font-size="24pt" fo:text-shadow="1pt 1pt"/></style:style>"#,
        ),
        text("T1", r##"fo:font-size="48pt" fo:color="#000000" fo:text-shadow="1pt 1pt""##),
        text(
            "T2",
            r##"fo:font-size="48pt" fo:color="#cc0000" fo:text-shadow="5pt 5pt 3pt #00ff00""##,
        ),
        text("T3", r#"fo:text-shadow="none""#),
    ]
    .concat();
    let frame = |name: &str, style: &str, paragraph: &str| {
        format!(
            r#"<draw:frame draw:name="{name}" draw:style-name="{style}" {}><draw:text-box><text:p>{paragraph}</text:p></draw:text-box></draw:frame>"#,
            place("1in", "1in", "6in", "1in")
        )
    };
    let lowered = deck(
        &automatic,
        &[
            frame(
                "Black",
                "gr1",
                r#"<text:span text:style-name="T1">Black</text:span>"#,
            ),
            frame(
                "Red",
                "gr1",
                r#"<text:span text:style-name="T2">Red</text:span>"#,
            ),
            frame(
                "Level",
                "gr2",
                r#"Shadowed <text:span text:style-name="T3">plain</text:span>"#,
            ),
        ]
        .concat(),
    );
    let run = |name: &str, index: usize| {
        lowered.named(0, name)["text"]["paragraphs"][0]["inlines"][index]["props"]["shadow"].clone()
    };
    let shadow = |offset: f64, grey: f64| {
        js(&json!({
            "dx": offset, "dy": offset, "sigma": 0,
            "color": { "kind": "rgb", "space": "srgb", "r": grey, "g": grey, "b": grey, "a": 1 }
        }))
    };
    // 0.0451 of the size; light grey on black text, black on any other; the value's own offset and colour ignored.
    assert_eq!(run("Black", 0), shadow(2.165, 0.75294));
    assert_eq!(run("Red", 0), shadow(2.165, 0.0));
    // The level's shadow is the body's; a run that turns it off states so.
    let level = &lowered.named(0, "Level")["text"];
    assert_eq!(
        level["listStyle"]["0"]["run"]["shadow"],
        shadow(1.082, 0.75294)
    );
    let inlines = &level["paragraphs"][0]["inlines"];
    assert_eq!(inlines[0]["props"].get("shadow"), None);
    assert_eq!(inlines[1]["props"].get("shadow"), Some(&Value::Null));
    assert!(lowered.features("od-p1").contains(&"text.shadow"));
    assert!(codes(&lowered, "OD9204").is_empty());
}
