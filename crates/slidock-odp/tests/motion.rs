//! Native motion: each extraction rule of the transition and timing readers on a one-page deck, the
//! fail-closed reading of unknown values, and hostile timing trees. The parity path (`LowerOptions::native_motion`
//! off) lowers none of it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde_json::{Value, json};
use slidock_ir::diagnostics::{DataValue, Diagnostic};
use support::odp::{OdpSpec, PageSpec, odp, page_style};
use support::{Lowered, assert_holds, lower, lower_motion};

/// Two text shapes, `a` (three paragraphs, the second in a list) and `b`, and a media frame `m`.
const SHAPES: &str = concat!(
    r#"<draw:rect svg:x="2cm" svg:y="2cm" svg:width="8cm" svg:height="4cm" xml:id="a">"#,
    r#"<text:p xml:id="a0">one</text:p><text:list><text:list-item><text:p xml:id="a1">two</text:p>"#,
    r#"</text:list-item></text:list><text:p xml:id="a2">three</text:p></draw:rect>"#,
    r#"<draw:rect svg:x="12cm" svg:y="2cm" svg:width="4cm" svg:height="4cm" xml:id="b"/>"#,
);

/// A one-page deck: its drawing-page properties, and a body after the shapes.
fn deck(props: &str, body: &str) -> Vec<u8> {
    odp(&OdpSpec {
        automatic_styles: page_style("dp1", props),
        ..OdpSpec::pages(vec![
            PageSpec::new()
                .style("dp1")
                .attrs(r#" xml:id="s1""#)
                .with_body(format!("{SHAPES}{body}")),
        ])
    })
}

/// A timing root holding `children` (the transition node, sequences).
fn root(children: &str) -> String {
    format!(r#"<anim:par presentation:node-type="timing-root">{children}</anim:par>"#)
}

/// A main sequence of groups.
fn main(groups: &[String]) -> String {
    root(&format!(
        r#"<anim:seq presentation:node-type="main-sequence">{}</anim:seq>"#,
        groups.concat()
    ))
}

/// A group starting at `begin` (`next` for a click) of time slots `(begin, effects)`.
fn group(begin: &str, slots: &[(&str, String)]) -> String {
    let slots: String = slots
        .iter()
        .map(|(begin, effects)| format!(r#"<anim:par smil:begin="{begin}">{effects}</anim:par>"#))
        .collect();
    format!(r#"<anim:par smil:begin="{begin}">{slots}</anim:par>"#)
}

/// An effect node with `attrs` and its behaviours.
fn effect(attrs: &str, leaves: &str) -> String {
    format!(r#"<anim:par smil:fill="hold" {attrs}>{leaves}</anim:par>"#)
}

/// An emphasis on click with LibreOffice's preset id and its behaviours.
fn emphasis(preset: &str, leaves: &str) -> String {
    effect(
        &format!(
            r#"presentation:node-type="on-click" presentation:preset-class="emphasis" presentation:preset-id="{preset}""#
        ),
        leaves,
    )
}

/// The visibility switch an entrance opens with, on `target`.
fn show(target: &str) -> String {
    format!(
        r#"<anim:set smil:begin="0s" smil:dur="0.001s" smil:fill="hold" smil:targetElement="{target}" smil:attributeName="visibility" smil:to="visible"/>"#
    )
}

fn fade(target: &str, dur: &str) -> String {
    format!(
        r#"<anim:transitionFilter smil:dur="{dur}" smil:targetElement="{target}" smil:type="fade" smil:subtype="crossfade"/>"#
    )
}

/// An on-click fade-in of `target`.
fn fade_in(target: &str) -> String {
    effect(
        r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-fade-in""#,
        &format!("{}{}", show(target), fade(target, "0.5s")),
    )
}

/// One click group holding one effect.
fn clicked(effect: String) -> String {
    group("next", &[("0s", effect)])
}

fn slide(lowered: &Lowered) -> &Value {
    &lowered.json["slides"][0]
}

fn steps(lowered: &Lowered) -> Vec<Value> {
    slide(lowered)["timeline"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// The codes reported with each effect, by native id, in order.
fn coded(lowered: &Lowered) -> Vec<(String, String)> {
    lowered
        .diagnostics
        .iter()
        .map(|d: &Diagnostic| {
            let effect = match d.data.as_ref().and_then(|data| data.get("effect")) {
                Some(DataValue::Str(s)) => s.clone(),
                _ => String::new(),
            };
            (d.code.code().to_owned(), effect)
        })
        .collect()
}

fn codes(lowered: &Lowered) -> Vec<String> {
    coded(lowered).into_iter().map(|(code, _)| code).collect()
}

// Transitions.

#[test]
fn transitions_map_through_the_smil_table_with_the_content_moves_direction() {
    for (props, effect, params) in [
        (
            r#"smil:type="pushWipe" smil:subtype="fromTop""#,
            "push",
            json!({ "dir": "d" }),
        ),
        (
            r#"smil:type="pushWipe" smil:subtype="fromRight""#,
            "push",
            json!({ "dir": "l" }),
        ),
        (
            r#"smil:type="slideWipe" smil:subtype="fromBottomLeft""#,
            "cover",
            json!({ "dir": "ru" }),
        ),
        (
            r#"smil:type="slideWipe" smil:subtype="fromLeft" smil:direction="reverse""#,
            "uncover",
            json!({ "dir": "r" }),
        ),
        (
            r#"smil:type="barWipe" smil:subtype="topToBottom" smil:direction="reverse""#,
            "wipe",
            json!({ "dir": "u", "soft": false }),
        ),
        (
            r#"smil:type="barnDoorWipe" smil:subtype="horizontal" smil:direction="reverse""#,
            "split",
            json!({ "orient": "horz", "inOut": "in" }),
        ),
        (
            r##"smil:type="fade" smil:subtype="fadeOverColor" smil:fadeColor="#ff8000""##,
            "fade",
            json!({ "through": "color", "color": "#ff8000" }),
        ),
        (
            r#"smil:type="zoom" smil:subtype="default" smil:direction="reverse""#,
            "zoom",
            json!({ "inOut": "out", "rotate": false }),
        ),
    ] {
        let lowered = lower_motion(&deck(props, ""));
        let transition = &slide(&lowered)["transition"];
        assert_eq!(transition["effect"]["native"]["effect"], effect, "{props}");
        assert_eq!(transition["effect"]["native"]["params"], params, "{props}");
        assert!(codes(&lowered).is_empty(), "{props}: {:?}", codes(&lowered));
    }
}

#[test]
fn a_transition_carries_its_native_id_and_canonical_fallback() {
    let lowered = lower_motion(&deck(r#"smil:type="pushWipe" smil:subtype="fromLeft""#, ""));
    assert_holds(
        &slide(&lowered)["transition"],
        json!({
            "effect": { "id": "odp:pushWipe/fromLeft", "fallback": "push", "native": { "id": "odp:pushWipe/fromLeft" } },
            "durationMs": 1000,
            "advance": { "onClick": true },
        }),
    );
}

#[test]
fn the_timing_roots_filter_duration_beats_the_speed() {
    let filter = root(
        r#"<anim:par smil:begin="s1.begin"><anim:transitionFilter smil:type="fade" smil:subtype="crossfade" smil:dur="1.2s"/></anim:par>"#,
    );
    for (props, body, duration) in [
        (
            r#"smil:type="fade" smil:subtype="crossfade" presentation:transition-speed="slow""#,
            "",
            2000,
        ),
        (
            r#"smil:type="fade" smil:subtype="crossfade" presentation:transition-speed="fast""#,
            "",
            500,
        ),
        (r#"smil:type="fade" smil:subtype="crossfade""#, "", 1000),
        (
            r#"smil:type="fade" smil:subtype="crossfade" presentation:transition-speed="slow""#,
            filter.as_str(),
            1200,
        ),
    ] {
        let lowered = lower_motion(&deck(props, body));
        assert_eq!(
            slide(&lowered)["transition"]["durationMs"],
            duration,
            "{props} {body}"
        );
    }
}

#[test]
fn the_advance_follows_the_transition_type() {
    let fade = r#"smil:type="fade" smil:subtype="crossfade""#;
    for (extra, advance, code) in [
        (
            r#"presentation:transition-type="manual""#,
            json!({ "onClick": true }),
            None,
        ),
        (
            r#"presentation:transition-type="semi-automatic""#,
            json!({ "onClick": true }),
            None,
        ),
        (
            r#"presentation:transition-type="automatic" presentation:duration="PT00H00M05S""#,
            json!({ "onClick": false, "afterMs": 5000 }),
            None,
        ),
        (
            r#"presentation:transition-type="automatic""#,
            json!({ "onClick": false, "afterMs": 0 }),
            None,
        ),
        (
            r#"presentation:transition-type="automatic" presentation:duration="-PT5S""#,
            json!({ "onClick": true }),
            Some("OD9411"),
        ),
        (
            r#"presentation:transition-type="automatic" presentation:duration="P2D""#,
            json!({ "onClick": false, "afterMs": 3_600_000 }),
            Some("OD9411"),
        ),
        (
            r#"presentation:transition-type="sometimes""#,
            json!({ "onClick": true }),
            Some("OD9412"),
        ),
    ] {
        let lowered = lower_motion(&deck(&format!("{fade} {extra}"), ""));
        assert_eq!(slide(&lowered)["transition"]["advance"], advance, "{extra}");
        assert_eq!(
            codes(&lowered),
            code.into_iter().collect::<Vec<_>>(),
            "{extra}"
        );
    }
    // An automatic advance alone is a transition with no effect.
    let lowered = lower_motion(&deck(r#"presentation:transition-type="automatic""#, ""));
    assert_holds(
        &slide(&lowered)["transition"],
        json!({ "effect": { "id": "odp:none", "fallback": "none", "native": { "effect": "none" } } }),
    );
}

/// An ODF 1.0 transition style plays the transition LibreOffice migrates it to (`EffectMigration.cxx`): the style's
/// `FadeEffect`, its first preset in the migration table, that preset's SMIL filter.
#[test]
fn legacy_transition_styles_play_what_libreoffice_migrates_them_to() {
    for (style, effect, params) in [
        ("move-from-top", "cover", json!({ "dir": "d" })),
        (
            "fade-from-left",
            "wipe",
            json!({ "dir": "r", "soft": false }),
        ),
        // diagonal-squares-left-down
        ("fade-from-upperright", "strips", json!({ "corner": "ld" })),
        // wheel-clockwise-1-spoke, the style's first row
        ("clockwise", "wheel", json!({ "cw": true, "spokes": 1 })),
        (
            "counterclockwise",
            "clock",
            json!({ "cw": false, "from": "12" }),
        ),
        ("uncover-to-right", "uncover", json!({ "dir": "r" })),
        // split-horizontal-out
        (
            "open-vertical",
            "split",
            json!({ "orient": "horz", "inOut": "out" }),
        ),
        (
            "close-horizontal",
            "split",
            json!({ "orient": "vert", "inOut": "in" }),
        ),
        // box-in
        (
            "fade-to-center",
            "iris",
            json!({ "shape": "box", "inOut": "in" }),
        ),
        ("vertical-stripes", "blinds", json!({ "orient": "vert" })),
        (
            "horizontal-lines",
            "randomBars",
            json!({ "orient": "horz" }),
        ),
        ("roll-from-left", "push", json!({ "dir": "r" })),
        // LibreOffice migrates stretches to wipes
        (
            "stretch-from-top",
            "wipe",
            json!({ "dir": "d", "soft": false }),
        ),
        ("vertical-checkerboard", "checker", json!({ "dir": "d" })),
        ("dissolve", "dissolve", Value::Null),
        ("random", "random", Value::Null),
    ] {
        let lowered = lower_motion(&deck(
            &format!(r#"presentation:transition-style="{style}""#),
            "",
        ));
        let transition = &slide(&lowered)["transition"];
        assert_eq!(transition["effect"]["id"], format!("odp:legacy/{style}"));
        assert_eq!(transition["effect"]["native"]["effect"], effect, "{style}");
        assert_eq!(transition["effect"]["native"]["params"], params, "{style}");
        assert!(codes(&lowered).is_empty(), "{style}");
    }
    // A style the migration has no row for, or one spelt otherwise, dissolves and is reported.
    for style in [
        "interlocking-horizontal-left",
        "fly-away",
        "FADE-FROM-LEFT",
        "fade_from_left",
    ] {
        let lowered = lower_motion(&deck(
            &format!(r#"presentation:transition-style="{style}""#),
            "",
        ));
        let transition = &slide(&lowered)["transition"];
        assert!(transition["effect"]["native"].is_null(), "{style}");
        assert_eq!(codes(&lowered), ["OD9402"], "{style}");
    }
    let lowered = lower_motion(&deck(r#"presentation:transition-style="none""#, ""));
    assert!(slide(&lowered)["transition"].is_null());
}

#[test]
fn a_transition_with_no_entry_dissolves_and_is_reported() {
    for props in [
        r#"smil:type="spiralWipe" smil:subtype="topLeftClockwise""#,
        r#"smil:type="miscShapeWipe" smil:subtype="heart""#,
        r#"smil:type="sparkleWipe""#,
        r#"presentation:transition-style="wavyline-from-left""#,
    ] {
        let lowered = lower_motion(&deck(props, ""));
        let transition = &slide(&lowered)["transition"];
        assert_eq!(transition["effect"]["fallback"], "dissolve", "{props}");
        assert!(transition["effect"]["native"].is_null(), "{props}");
        assert_eq!(codes(&lowered), ["OD9402"], "{props}");
    }
}

#[test]
fn unknown_transition_values_read_as_their_defaults_and_are_reported() {
    let lowered = lower_motion(&deck(
        r#"smil:type="pushWipe" smil:subtype="fromNowhere" smil:direction="sideways" presentation:transition-speed="warp""#,
        "",
    ));
    let transition = &slide(&lowered)["transition"];
    assert_eq!(
        transition["effect"]["native"]["params"],
        json!({ "dir": "r" })
    );
    assert_eq!(transition["durationMs"], 1000);
    assert_eq!(codes(&lowered), ["OD9412", "OD9412", "OD9412"]);
    // The SMIL `default` sub-type is the type's own default, not an unknown one.
    let lowered = lower_motion(&deck(r#"smil:type="zoom" smil:subtype="default""#, ""));
    assert!(codes(&lowered).is_empty());
}

#[test]
fn the_parity_path_lowers_no_motion() {
    let bytes = deck(
        r#"smil:type="pushWipe" smil:subtype="fromTop""#,
        &main(&[clicked(fade_in("a"))]),
    );
    let parity = lower(&bytes);
    assert!(slide(&parity)["transition"].is_null());
    assert!(
        slide(&parity)["timeline"]
            .as_array()
            .is_none_or(Vec::is_empty)
    );
    assert_eq!(codes(&parity), ["OD9401", "OD9402"]);
    let native = lower_motion(&bytes);
    assert!(codes(&native).is_empty(), "{:?}", codes(&native));
    // Without motion, both frontends lower the same presentation.
    let still = deck(r#"draw:fill="none""#, "");
    assert_eq!(lower(&still).json, lower_motion(&still).json);
}

// Build steps.

#[test]
fn groups_slots_and_node_types_give_triggers_offsets_and_delays() {
    let fly = effect(
        r#"smil:begin="0.25s" presentation:node-type="with-previous" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-fly-in" presentation:preset-sub-type="from-left""#,
        &format!(
            r#"{}<anim:animate smil:dur="0.5s" smil:targetElement="b" smil:attributeName="x" smil:values="0-width/2;x" smil:keyTimes="0;1"/>"#,
            show("b")
        ),
    );
    let after = effect(
        r#"smil:begin="0.5s" presentation:node-type="after-previous" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-fade-in""#,
        &format!("{}{}", show("a"), fade("a", "0.5s")),
    );
    let body = main(&[group(
        "next",
        &[("0s", format!("{}{fly}", fade_in("a"))), ("0.75s", after)],
    )]);
    let lowered = lower_motion(&deck("", &body));
    let got = steps(&lowered);
    assert_eq!(got.len(), 3);
    assert_holds(
        &got[0],
        json!({ "trigger": "click", "delayMs": 0, "durationMs": 500, "kind": "entrance", "timing": { "offsetMs": 0 } }),
    );
    assert_holds(
        &got[1],
        json!({ "trigger": "withPrevious", "delayMs": 250, "timing": { "offsetMs": 250 },
            "effect": { "id": "odp:ooo-entrance-fly-in/from-left", "fallback": "dissolve",
                "native": { "effect": "flyIn", "params": { "dir": "r", "distance": "offSlide" } } } }),
    );
    assert_holds(
        &got[2],
        json!({ "trigger": "afterPrevious", "delayMs": 500, "timing": { "offsetMs": 1250 } }),
    );
}

#[test]
fn a_group_that_begins_at_once_plays_without_a_click() {
    let auto = effect(
        r#"presentation:node-type="after-previous" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-fade-in""#,
        &format!("{}{}", show("a"), fade("a", "0.5s")),
    );
    let lowered = lower_motion(&deck(
        "",
        &main(&[group("0s", &[("0s", auto)]), clicked(fade_in("b"))]),
    ));
    let got = steps(&lowered);
    assert_eq!(got[0]["trigger"], "afterPrevious");
    assert_eq!(got[1]["trigger"], "click");
}

#[test]
fn a_missing_node_type_comes_from_the_position_and_an_unknown_one_is_reported() {
    let plain = |target: &str| {
        effect(
            r#"presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-fade-in""#,
            &format!("{}{}", show(target), fade(target, "0.5s")),
        )
    };
    let odd = effect(
        r#"presentation:node-type="whenever" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-fade-in""#,
        &format!("{}{}", show("a"), fade("a", "0.5s")),
    );
    let body = main(&[group(
        "next",
        &[
            ("0s", format!("{}{}", plain("a"), plain("b"))),
            ("0.5s", odd),
        ],
    )]);
    let lowered = lower_motion(&deck("", &body));
    let triggers: Vec<Value> = steps(&lowered)
        .iter()
        .map(|s| s["trigger"].clone())
        .collect();
    assert_eq!(triggers, ["click", "withPrevious", "afterPrevious"]);
    assert_eq!(codes(&lowered), ["OD9412"]);
}

#[test]
fn durations_ignore_visibility_switches_and_an_appear_is_instant() {
    let exit = effect(
        r#"presentation:node-type="on-click" presentation:preset-class="exit" presentation:preset-id="ooo-exit-fade-out""#,
        r#"<anim:transitionFilter smil:dur="0.75s" smil:targetElement="a" smil:type="fade" smil:subtype="crossfade" smil:mode="out"/><anim:set smil:begin="0.749s" smil:dur="0.001s" smil:targetElement="a" smil:attributeName="visibility" smil:to="hidden"/>"#,
    );
    let appear = effect(
        r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-appear""#,
        &show("b"),
    );
    // Colored lettering appears letter by letter and passes each letter through a colour: its behaviours play.
    let lettering = effect(
        r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-colored-lettering""#,
        &format!(
            r##"{}<anim:animateColor smil:dur="0.5s" smil:targetElement="b" smil:attributeName="color" smil:to="#e0442e"/>"##,
            show("b")
        ),
    );
    // A switch that closes after the effect does not lengthen it.
    let late = effect(
        r#"presentation:node-type="on-click" presentation:preset-class="exit" presentation:preset-id="ooo-exit-fade-out""#,
        r#"<anim:transitionFilter smil:dur="0.5s" smil:targetElement="b" smil:type="fade" smil:subtype="crossfade" smil:mode="out"/><anim:set smil:begin="0.6s" smil:dur="0.001s" smil:targetElement="b" smil:attributeName="visibility" smil:to="hidden"/>"#,
    );
    let lowered = lower_motion(&deck(
        "",
        &main(&[
            clicked(exit),
            clicked(appear),
            clicked(lettering),
            clicked(late),
        ]),
    ));
    let got = steps(&lowered);
    assert_holds(
        &got[2],
        json!({ "durationMs": 500, "delivery": "byChar", "effect": { "native": { "effect": "behaviors" } } }),
    );
    assert_eq!(got[3]["durationMs"], 500);
    assert_holds(
        &got[0],
        json!({ "kind": "exit", "durationMs": 750, "effect": { "fallback": "dissolve", "native": { "effect": "fade" } } }),
    );
    assert_holds(
        &got[1],
        json!({ "durationMs": 0, "effect": { "fallback": "appear", "native": { "effect": "appear" } } }),
    );
}

/// LibreOffice's colour and font emphases (`effects.xml`): a colour change to a colour or by an HSL amount, sets of
/// colours and font switches, a discrete colour pass that returns to the element's colour; a font family or size
/// change plays the fallback.
#[test]
fn colour_and_font_behaviours_compile_to_tracks() {
    let blend = emphasis(
        "ooo-emphasis-color-blend",
        r##"<anim:animateColor smil:dur="0.5" smil:fill="hold" smil:targetElement="a" smil:attributeName="color" smil:to="#333399"/><anim:animateColor smil:dur="0.5" smil:fill="hold" smil:targetElement="a" smil:attributeName="fill-color" smil:to="#333399"/><anim:set smil:dur="0.5" smil:fill="hold" smil:targetElement="a" smil:attributeName="fill" smil:to="solid"/>"##,
    );
    let complementary = emphasis(
        "ooo-emphasis-complementary-color",
        r#"<anim:animateColor smil:dur="0.5" smil:fill="hold" smil:targetElement="a" smil:attributeName="stroke-color" smil:by="hsl(120,0%,0%)" anim:color-interpolation="hsl"/>"#,
    );
    let darken = emphasis(
        "ooo-emphasis-darken",
        r#"<anim:animateColor smil:dur="0.5" smil:fill="hold" smil:targetElement="a" smil:attributeName="fill-color" smil:by="hsl(0,-12%,-25%)"/>"#,
    );
    let style = emphasis(
        "ooo-emphasis-style-emphasis",
        r##"<anim:set smil:dur="0.5" smil:fill="hold" smil:targetElement="a" anim:sub-item="text" smil:attributeName="color" smil:to="#333399"/><anim:set smil:dur="indefinite" smil:targetElement="a" anim:sub-item="text" smil:attributeName="font-style" smil:to="italic"/><anim:set smil:dur="0.5" smil:fill="hold" smil:targetElement="a" anim:sub-item="text" smil:attributeName="font-weight" smil:to="bold"/><anim:set smil:dur="0.5" smil:fill="hold" smil:targetElement="a" anim:sub-item="text" smil:attributeName="text-underline" smil:to="solid"/>"##,
    );
    let flash = emphasis(
        "ooo-emphasis-probe-flash",
        r#"<anim:animate smil:dur="2" smil:fill="hold" smil:targetElement="a" anim:sub-item="text" smil:attributeName="font-weight" smil:values="normal;bold;normal;normal" smil:keyTimes="0;0.5;0.6;1" smil:calcMode="discrete"/>"#,
    );
    let pass = emphasis(
        "ooo-emphasis-probe-pass",
        r##"<anim:animate smil:dur="0.08" smil:targetElement="b" smil:attributeName="fill-color" smil:values="#333399;#009999" smil:keyTimes="0;0.5" smil:calcMode="discrete"/>"##,
    );
    let font = emphasis(
        "ooo-emphasis-font",
        r#"<anim:set smil:dur="indefinite" smil:targetElement="a" anim:sub-item="text" smil:attributeName="font-family" smil:to="Times New Roman"/>"#,
    );
    let grow = emphasis(
        "ooo-emphasis-grow-with-color",
        r##"<anim:animateColor smil:dur="0.5" smil:fill="hold" smil:targetElement="a" anim:sub-item="text" smil:attributeName="color" smil:to="#333399"/><anim:animate smil:dur="0.5" smil:fill="hold" smil:targetElement="a" anim:sub-item="text" smil:attributeName="font-size" smil:to="1.5pt"/>"##,
    );
    let lowered = lower_motion(&deck(
        "",
        &main(&[
            clicked(blend),
            clicked(complementary),
            clicked(darken),
            clicked(style),
            clicked(flash),
            clicked(pass),
            clicked(font),
            clicked(grow),
        ]),
    ));
    let got = steps(&lowered);
    let params = |i: usize| got[i]["effect"]["native"]["params"].clone();
    assert_eq!(
        params(0),
        json!([
            { "property": "text", "beginMs": 0, "durationMs": 500, "frames": [
                { "offset": 0, "value": { "h": 0, "s": 0, "l": 0 } }, { "offset": 1, "value": "#333399" } ] },
            { "property": "fill", "beginMs": 0, "durationMs": 500, "frames": [
                { "offset": 0, "value": { "h": 0, "s": 0, "l": 0 } }, { "offset": 1, "value": "#333399" } ] },
        ])
    );
    // A hue turn is sampled so it travels its own way round: nine frames, 15° apart.
    let line = params(1)[0].clone();
    assert_eq!(line["property"], "line");
    assert_eq!(line["frames"].as_array().unwrap().len(), 9);
    assert_eq!(
        line["frames"][1]["value"],
        json!({ "h": 15, "s": 0, "l": 0 })
    );
    assert_eq!(
        line["frames"][8]["value"],
        json!({ "h": 120, "s": 0, "l": 0 })
    );
    assert_eq!(
        params(2)[0]["frames"],
        json!([
            { "offset": 0, "value": { "h": 0, "s": 0, "l": 0 } },
            { "offset": 1, "value": { "h": 0, "s": -0.12, "l": -0.25 } },
        ])
    );
    // A `set` is a switch at its begin, of the property it sets; an indefinite one lasts 0 ms and holds.
    let switched: Vec<(String, String)> = params(3)
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["property"].as_str().unwrap().to_owned(),
                t["to"]["property"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        switched,
        [
            ("set".to_owned(), "text".to_owned()),
            ("set".to_owned(), "italic".to_owned()),
            ("set".to_owned(), "bold".to_owned()),
            ("set".to_owned(), "underline".to_owned()),
        ]
    );
    assert_eq!(
        params(3)[1],
        json!({ "property": "set", "beginMs": 0, "durationMs": 0, "to": { "property": "italic", "value": true } })
    );
    assert_eq!(
        params(3)[0]["to"],
        json!({ "property": "text", "value": "#333399" })
    );
    assert_eq!(
        params(4)[0]["frames"],
        json!([
            { "offset": 0, "value": false }, { "offset": 0.5, "value": false }, { "offset": 0.5, "value": true },
            { "offset": 0.6, "value": true }, { "offset": 0.6, "value": false }, { "offset": 1, "value": false },
            { "offset": 1, "value": false },
        ])
    );
    // A discrete pass that does not hold returns to the element's own colour.
    assert_eq!(
        params(5)[0]["frames"],
        json!([
            { "offset": 0, "value": "#333399" }, { "offset": 0.5, "value": "#333399" },
            { "offset": 0.5, "value": "#009999" }, { "offset": 1, "value": "#009999" },
            { "offset": 1, "value": { "h": 0, "s": 0, "l": 0 } },
        ])
    );
    for i in [6, 7] {
        assert!(got[i]["effect"]["native"].is_null(), "{}", got[i]);
        assert_eq!(got[i]["effect"]["fallback"], "none");
    }
    assert_eq!(
        coded(&lowered),
        [
            ("OD9410".to_owned(), "odp:ooo-emphasis-font".to_owned()),
            (
                "OD9410".to_owned(),
                "odp:ooo-emphasis-grow-with-color".to_owned()
            ),
        ]
    );
}

/// A word or a character has no fill or line of its own: a step built by unit keeps its text's tracks only.
#[test]
fn a_step_built_by_unit_drops_its_fill_and_line_tracks() {
    let iterate = |preset: &str, leaves: &str| {
        format!(
            r#"<anim:iterate smil:fill="hold" presentation:node-type="on-click" presentation:preset-class="emphasis" presentation:preset-id="{preset}" anim:iterate-type="by-letter" anim:iterate-interval="0.05s" smil:targetElement="a">{leaves}</anim:iterate>"#
        )
    };
    let both = iterate(
        "ooo-emphasis-color-over-by-letter",
        r##"<anim:set smil:dur="0.5" smil:fill="hold" smil:attributeName="color" smil:to="#333399"/><anim:set smil:dur="0.5" smil:fill="hold" smil:attributeName="fill-color" smil:to="#333399"/>"##,
    );
    let fill = iterate(
        "ooo-emphasis-probe-fill",
        r##"<anim:set smil:dur="0.5" smil:fill="hold" smil:attributeName="fill-color" smil:to="#333399"/>"##,
    );
    let lowered = lower_motion(&deck("", &main(&[clicked(both), clicked(fill)])));
    let got = steps(&lowered);
    let tracks = got[0]["effect"]["native"]["params"].as_array().unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0]["to"]["property"], "text");
    assert_eq!(got[0]["delivery"], "byChar");
    assert!(got[1]["effect"]["native"].is_null());
    assert_eq!(codes(&lowered), ["OD9410"]);
}

/// A behaviour that reverses on its own (`smil:autoReverse` on the leaf, as LibreOffice's Color Over by Letter writes
/// its sets) plays its reversal: a switch keeps its value over twice its span, and the step lasts that long. Nothing
/// is reported. The same sets without it last their own span.
#[test]
fn a_leafs_own_reversal_plays() {
    let iterate = |reverse: &str| {
        format!(
            r##"<anim:iterate smil:fill="hold" presentation:node-type="on-click" presentation:preset-class="emphasis" presentation:preset-id="ooo-emphasis-color-over-by-letter" anim:iterate-type="by-letter" anim:iterate-interval="0.05s" smil:targetElement="a"><anim:set smil:dur="0.5" smil:fill="hold"{reverse} smil:attributeName="color" smil:to="#333399"/><anim:set smil:dur="0.5" smil:fill="hold"{reverse} smil:attributeName="fill" smil:to="solid"/></anim:iterate>"##
        )
    };
    let reversed = iterate(r#" smil:autoReverse="true""#);
    let lowered = lower_motion(&deck("", &main(&[clicked(reversed), clicked(iterate(""))])));
    let got = steps(&lowered);
    for (step, span) in got.iter().zip([1000, 500]) {
        let tracks = step["effect"]["native"]["params"].as_array().unwrap();
        assert_eq!(
            tracks,
            &[
                json!({ "property": "set", "beginMs": 0, "durationMs": span, "to": { "property": "text", "value": "#333399" } })
            ],
            "{step}"
        );
        assert_eq!(step["durationMs"], span);
    }
    assert!(coded(&lowered).is_empty(), "{:?}", coded(&lowered));
}

/// A behaviour's reversal and a time filter (LibreOffice's `smil:keySplines`, its flash bulb's fade out to half and
/// back) fold into the track: the step ends where it started. A time filter that does not read plays without it,
/// named in OD9410.
#[test]
fn a_reversal_and_a_time_filter_end_at_rest() {
    let bulb = |splines: &str| {
        emphasis(
            "ooo-emphasis-flash-bulb",
            &format!(
                r#"<anim:transitionFilter smil:dur="0.5" smil:keySplines="{splines}" smil:targetElement="a" smil:type="fade" smil:subtype="crossfade" smil:mode="out"/><anim:animateTransform smil:dur="0.25" smil:fill="hold" smil:autoReverse="true" smil:targetElement="a" smil:by="1.05,1.05" svg:type="scale"/>"#
            ),
        )
    };
    let lowered = lower_motion(&deck(
        "",
        &main(&[
            clicked(bulb("0,0;0.2,0.5;0.8,0.5;1,0")),
            clicked(bulb("0,0,1,1")),
        ]),
    ));
    let got = steps(&lowered);
    let tracks = got[0]["effect"]["native"]["params"].as_array().unwrap();
    let opacity = tracks.iter().find(|t| t["property"] == "opacity").unwrap();
    assert_eq!(
        opacity["frames"],
        json!([
            { "offset": 0, "value": 1 }, { "offset": 0.2, "value": 0.5 },
            { "offset": 0.8, "value": 0.5 }, { "offset": 1, "value": 1 },
        ])
    );
    let scale = tracks.iter().find(|t| t["property"] == "scale").unwrap();
    assert_eq!(scale["durationMs"], 500);
    assert_eq!(
        scale["frames"],
        json!([
            { "offset": 0, "value": { "x": 1, "y": 1 } }, { "offset": 0.5, "value": { "x": 1.05, "y": 1.05 } },
            { "offset": 1, "value": { "x": 1, "y": 1 } },
        ])
    );
    assert_eq!(got[0]["durationMs"], 500);
    // A SMIL bezier list is not a time filter: the fade plays without it, and the loss is named.
    let unread = got[1]["effect"]["native"]["params"].as_array().unwrap();
    let fade = unread.iter().find(|t| t["property"] == "opacity").unwrap();
    assert_eq!(fade["frames"][1]["value"], 0);
    assert_eq!(
        coded(&lowered),
        [(
            "OD9410".to_owned(),
            "odp:ooo-emphasis-flash-bulb".to_owned()
        )]
    );
}

/// A switch that does not hold (no `smil:fill` with a duration, or `remove`) gives the element its own value back when
/// it ends: a colour its own (no shift), an opacity full. A font switch has no own value in the IR: it holds, and the
/// end is named (OD9410). An indefinite switch never ends: it holds.
#[test]
fn a_switch_that_does_not_hold_switches_back() {
    let leaves = concat!(
        r##"<anim:set smil:begin="0.1" smil:dur="0.4" smil:targetElement="a" smil:attributeName="color" smil:to="#333399"/>"##,
        r#"<anim:set smil:begin="0.1" smil:dur="0.4" smil:fill="remove" smil:targetElement="a" smil:attributeName="opacity" smil:to="0.5"/>"#,
        r#"<anim:set smil:begin="0.1" smil:dur="0.4" smil:targetElement="a" smil:attributeName="font-weight" smil:to="bold"/>"#,
        r#"<anim:set smil:dur="indefinite" smil:targetElement="a" smil:attributeName="font-style" smil:to="italic"/>"#,
    );
    let lowered = lower_motion(&deck(
        "",
        &main(&[clicked(emphasis("ooo-emphasis-probe-switches", leaves))]),
    ));
    let got = steps(&lowered);
    let own = json!({ "h": 0, "s": 0, "l": 0 });
    assert_eq!(
        got[0]["effect"]["native"]["params"],
        json!([
            { "property": "set", "beginMs": 100, "durationMs": 400, "to": { "property": "text", "value": "#333399" } },
            { "property": "set", "beginMs": 500, "durationMs": 0, "to": { "property": "text", "value": own } },
            { "property": "set", "beginMs": 100, "durationMs": 400, "to": { "property": "opacity", "value": 0.5 } },
            { "property": "set", "beginMs": 500, "durationMs": 0, "to": { "property": "opacity", "value": 1 } },
            { "property": "set", "beginMs": 100, "durationMs": 400, "to": { "property": "bold", "value": true } },
            { "property": "set", "beginMs": 0, "durationMs": 0, "to": { "property": "italic", "value": true } },
        ])
    );
    assert_eq!(
        coded(&lowered),
        [(
            "OD9410".to_owned(),
            "odp:ooo-emphasis-probe-switches".to_owned()
        )]
    );
    let play = lowered.diagnostics[0]
        .data
        .as_ref()
        .and_then(|data| data.get("play"))
        .cloned();
    assert_eq!(
        play,
        Some(DataValue::Str(
            "its behaviours without the end of its font-weight switch".to_owned()
        ))
    );
}

/// LibreOffice's Flash Once is an entrance whose visibility switch does not hold: it plays as an appear that lasts the
/// switch and hides its target after. The same switch held is a plain appear.
#[test]
fn flash_once_shows_its_target_while_its_switch_lasts() {
    let entrance = |fill: &str| {
        effect(
            r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-flash-once""#,
            &format!(
                r#"<anim:set smil:begin="0" smil:dur="1"{fill} smil:targetElement="a" smil:attributeName="visibility" smil:to="visible"/>"#
            ),
        )
    };
    let lowered = lower_motion(&deck(
        "",
        &main(&[
            clicked(entrance("")),
            clicked(entrance(r#" smil:fill="hold""#)),
        ]),
    ));
    let got = steps(&lowered);
    assert_eq!(got[0]["effect"]["native"]["effect"], "appear");
    assert_eq!(got[0]["durationMs"], 1000);
    assert_eq!(got[0]["timing"]["after"], "hide");
    assert_eq!(got[1]["effect"]["native"]["effect"], "appear");
    assert_eq!(got[1]["durationMs"], 0);
    assert!(
        got[1]
            .get("timing")
            .is_none_or(|t| t.get("after").is_none()),
        "{}",
        got[1]
    );
    assert!(coded(&lowered).is_empty(), "{:?}", coded(&lowered));
}

/// A leaf's own repeat (`smil:repeatCount`, `smil:repeatDur`) plays its span, reversal included, that many times, and
/// the step lasts that long. One that does not fold (indefinite) plays once, named in OD9410.
#[test]
fn a_leafs_own_repeat_plays() {
    let pulse = |repeat: &str| {
        emphasis(
            "ooo-emphasis-flash-bulb",
            &format!(
                r#"<anim:animateTransform smil:dur="0.25" smil:fill="hold" smil:autoReverse="true"{repeat} smil:targetElement="a" smil:by="1.1,1.1" svg:type="scale"/>"#
            ),
        )
    };
    let lowered = lower_motion(&deck(
        "",
        &main(&[
            clicked(pulse(r#" smil:repeatCount="3""#)),
            clicked(pulse(r#" smil:repeatDur="1s""#)),
            clicked(pulse("")),
            clicked(pulse(r#" smil:repeatCount="indefinite""#)),
        ]),
    ));
    let got = steps(&lowered);
    let scale = |i: usize| got[i]["effect"]["native"]["params"][0].clone();
    let big = json!({ "x": 1.1, "y": 1.1 });
    assert_eq!(scale(0)["durationMs"], 1500);
    assert_eq!(got[0]["durationMs"], 1500);
    let frames = scale(0)["frames"].as_array().unwrap().clone();
    assert_eq!(frames.iter().filter(|f| f["value"] == big).count(), 3);
    assert_eq!(frames.last().unwrap()["value"], json!({ "x": 1, "y": 1 }));
    assert_eq!(scale(1)["durationMs"], 1000);
    assert_eq!(got[1]["durationMs"], 1000);
    assert_eq!(scale(2)["durationMs"], 500);
    assert_eq!(scale(3)["durationMs"], 500);
    assert_eq!(
        coded(&lowered),
        [(
            "OD9410".to_owned(),
            "odp:ooo-emphasis-flash-bulb".to_owned()
        )]
    );
    let play = lowered.diagnostics[0]
        .data
        .as_ref()
        .and_then(|data| data.get("play"))
        .cloned();
    assert_eq!(
        play,
        Some(DataValue::Str(
            "its behaviours without the repeat of its animateTransform animation".to_owned()
        ))
    );
}

/// A leaf's own `smil:accelerate`/`smil:decelerate` is its track's easing; folded with a reversal it is sampled into
/// the frames. A colour from the element's own colour folds into mixes the deck builder resolves (LibreOffice's
/// vertical highlight), so the colour plays, with nothing reported.
#[test]
fn a_leafs_own_easing_plays_and_a_colour_from_its_own_folds() {
    let highlight = |extra: &str| {
        emphasis(
            "ooo-emphasis-vertical-highlight",
            &format!(
                r##"<anim:animateColor smil:dur="0.5" smil:fill="hold" smil:accelerate="0.5"{extra} smil:targetElement="a" smil:attributeName="fill-color" smil:to="#333399"/><anim:set smil:dur="0.5" smil:fill="hold" smil:targetElement="a" smil:attributeName="fill" smil:to="solid"/>"##
            ),
        )
    };
    let lowered = lower_motion(&deck(
        "",
        &main(&[
            clicked(highlight("")),
            clicked(highlight(r#" smil:autoReverse="true""#)),
            clicked(highlight(r#" smil:keySplines="0,0;0.5,0.3;1,1""#)),
        ]),
    ));
    let got = steps(&lowered);
    let track = |i: usize| got[i]["effect"]["native"]["params"][0].clone();
    assert_eq!(
        track(0)["easing"],
        json!({ "accelDecel": { "accel": 0.5, "decel": 0 } })
    );
    assert_eq!(track(0)["frames"].as_array().unwrap().len(), 2);
    for i in [1, 2] {
        let folded = track(i);
        assert_eq!(folded["property"], "fill", "{folded}");
        assert_eq!(folded["easing"], "linear");
        let frames = folded["frames"].as_array().unwrap();
        assert!(
            frames.iter().any(|f| f["value"]["toward"] == "#333399"),
            "{folded}"
        );
    }
    assert_eq!(track(1)["durationMs"], 1000);
    assert!(coded(&lowered).is_empty(), "{:?}", coded(&lowered));
}

/// `smil:fill="transition"` holds, as in PPTX: a switch with it does not switch back.
#[test]
fn a_switch_filled_to_the_transition_holds() {
    let leaves = r##"<anim:set smil:begin="0.1" smil:dur="0.4" smil:fill="transition" smil:targetElement="a" smil:attributeName="color" smil:to="#333399"/>"##;
    let lowered = lower_motion(&deck(
        "",
        &main(&[clicked(emphasis("ooo-emphasis-probe-switches", leaves))]),
    ));
    assert_eq!(
        steps(&lowered)[0]["effect"]["native"]["params"],
        json!([
            { "property": "set", "beginMs": 100, "durationMs": 400, "to": { "property": "text", "value": "#333399" } },
        ])
    );
}

/// An entrance whose switch to visible does not hold shows its target only until the switch ends, whatever else its
/// leaves do after: a fade whose 1 ms switch has no `smil:fill` lasts 1 ms and hides its target, as in PPTX.
#[test]
fn an_entrance_shows_only_while_its_switch_lasts() {
    let entrance = |fill: &str| {
        effect(
            r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-fade-in""#,
            &format!(
                r#"<anim:set smil:begin="0" smil:dur="0.001"{fill} smil:targetElement="a" smil:attributeName="visibility" smil:to="visible"/>{}"#,
                fade("a", "0.5")
            ),
        )
    };
    let lowered = lower_motion(&deck(
        "",
        &main(&[
            clicked(entrance("")),
            clicked(entrance(r#" smil:fill="hold""#)),
        ]),
    ));
    let got = steps(&lowered);
    assert_eq!(got[0]["effect"]["native"]["effect"], "fade");
    assert_eq!(got[0]["durationMs"], 1);
    assert_eq!(got[0]["timing"]["after"], "hide");
    assert_eq!(got[1]["durationMs"], 500);
}

/// Google Slides writes its text frames without an id of their own and targets their paragraphs (`text:p xml:id`):
/// the frame is found by its path.
#[test]
fn a_paragraph_of_a_frame_without_an_id_is_a_target() {
    let frame = concat!(
        r#"<draw:frame svg:x="2cm" svg:y="8cm" svg:width="8cm" svg:height="4cm"><draw:text-box>"#,
        r#"<text:p xml:id="f0">one</text:p><text:p xml:id="f1">two</text:p></draw:text-box></draw:frame>"#,
    );
    let on = effect(
        r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-fade-in""#,
        r#"<anim:transitionFilter smil:dur="0.5s" smil:targetElement="f1" anim:sub-item="text" smil:type="fade" smil:subtype="crossfade"/>"#,
    );
    let lowered = lower_motion(&deck("", &format!("{frame}{}", main(&[clicked(on)]))));
    let got = steps(&lowered);
    assert_eq!(got.len(), 1, "{:?}", codes(&lowered));
    assert_eq!(got[0]["target"]["paragraphs"], json!([1, 1]));
    assert_eq!(got[0]["delivery"], "byParagraph");
    assert!(codes(&lowered).is_empty(), "{:?}", codes(&lowered));
}

#[test]
fn paragraph_text_and_background_targets() {
    let on = |target: &str, sub: &str| {
        effect(
            r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-fade-in""#,
            &format!(
                r#"<anim:transitionFilter smil:dur="0.5s" smil:targetElement="{target}" {sub} smil:type="fade" smil:subtype="crossfade"/>"#
            ),
        )
    };
    let body = main(&[
        clicked(on("a1", "")),
        clicked(on("a", r#"anim:sub-item="text""#)),
        clicked(on("a", r#"anim:sub-item="background""#)),
    ]);
    let lowered = lower_motion(&deck("", &body));
    let got = steps(&lowered);
    assert_holds(
        &got[0],
        json!({ "target": { "paragraphs": [1, 1] }, "delivery": "byParagraph" }),
    );
    assert_holds(
        &got[1],
        json!({ "target": { "paragraphs": [0, 2] }, "delivery": "all" }),
    );
    assert_holds(
        &got[2],
        json!({ "target": { "background": true }, "delivery": "all" }),
    );
    assert!(got[2]["target"]["paragraphs"].is_null());
}

#[test]
fn iterations_deliver_by_unit_with_a_stagger() {
    let iterate = |kind: &str, interval: &str| {
        format!(
            r#"<anim:iterate presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-fade-in" smil:targetElement="a" anim:iterate-type="{kind}" anim:iterate-interval="{interval}"><anim:transitionFilter smil:dur="0.5s" smil:type="fade" smil:subtype="crossfade"/></anim:iterate>"#
        )
    };
    let body = main(&[
        clicked(iterate("by-word", "PT0.05S")),
        clicked(iterate("by-letter", "0.025s")),
        clicked(iterate("by-paragraph", "0.1s")),
        clicked(iterate("by-sentence", "0.1s")),
    ]);
    let lowered = lower_motion(&deck("", &body));
    let got = steps(&lowered);
    let stagger = |ms: u32| json!({ "perUnit": { "ms": ms }, "order": "forward" });
    assert_holds(
        &got[0],
        json!({ "delivery": "byWord", "timing": { "stagger": stagger(50) } }),
    );
    assert_holds(
        &got[1],
        json!({ "delivery": "byChar", "timing": { "stagger": stagger(25) } }),
    );
    assert_holds(
        &got[2],
        json!({ "delivery": "byParagraph", "timing": { "stagger": stagger(100) } }),
    );
    assert_holds(&got[3], json!({ "delivery": "all" }));
    assert!(got[3]["timing"]["stagger"].is_null());
    assert_eq!(codes(&lowered), ["OD9412"]);
}

#[test]
fn repeats_reverses_easing_and_after_effects() {
    let spin = |attrs: &str| {
        effect(
            &format!(
                r#"presentation:node-type="on-click" presentation:preset-class="emphasis" presentation:preset-id="ooo-emphasis-spin" {attrs}"#
            ),
            r#"<anim:animateTransform smil:dur="0.5s" smil:targetElement="b" smil:attributeName="transform" smil:by="-90" svg:type="rotate"/>"#,
        )
    };
    let dimmed = format!(
        r##"{}<anim:par smil:begin="0.5s" presentation:master-element="e1"><anim:set smil:dur="0.001s" smil:targetElement="a" smil:attributeName="dim" smil:to="#808080"/></anim:par>"##,
        effect(
            r#"xml:id="e1" presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-fade-in""#,
            &format!("{}{}", show("a"), fade("a", "0.5s")),
        )
    );
    let hidden = format!(
        r#"{}<anim:par smil:begin="0.5s" presentation:master-element="e2"><anim:set smil:dur="0.001s" smil:targetElement="b" smil:attributeName="visibility" smil:to="hidden"/></anim:par>"#,
        effect(
            r#"xml:id="e2" presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-fade-in""#,
            &format!("{}{}", show("b"), fade("b", "0.5s")),
        )
    );
    let body = main(&[
        clicked(spin(r#"smil:repeatCount="2.5""#)),
        clicked(spin(r#"smil:repeatCount="indefinite""#)),
        clicked(spin(r#"smil:repeatCount="indefinite" smil:end="next""#)),
        clicked(spin(
            r#"smil:autoReverse="true" smil:accelerate="0.2" smil:decelerate="0.3""#,
        )),
        clicked(spin(r#"smil:accelerate="0.7" smil:decelerate="0.5""#)),
        clicked(dimmed),
        clicked(hidden),
    ]);
    let lowered = lower_motion(&deck("", &body));
    let got = steps(&lowered);
    assert_holds(
        &got[0],
        json!({ "kind": "emphasis", "effect": { "fallback": "none", "native": { "effect": "spin", "params": { "deg": 90, "cw": false } } }, "timing": { "repeat": { "count": 2.5 } } }),
    );
    assert_holds(
        &got[1],
        json!({ "timing": { "repeat": "untilEndOfSlide" } }),
    );
    assert_holds(&got[2], json!({ "timing": { "repeat": "untilNextClick" } }));
    assert_holds(
        &got[3],
        json!({ "timing": { "autoReverse": true, "easing": { "accelDecel": { "accel": 0.2, "decel": 0.3 } } } }),
    );
    assert!(got[4]["timing"]["easing"].is_null());
    assert_holds(
        &got[5],
        json!({ "timing": { "after": { "dim": "#808080" } } }),
    );
    assert_holds(&got[6], json!({ "timing": { "after": "hide" } }));
    assert_eq!(codes(&lowered), ["OD9412"]);
}

#[test]
fn a_clip_preset_plays_its_filter_not_its_sub_type_label() {
    // LibreOffice's venetian-blinds `vertical` plays blindsWipe horizontal; the filter decides, and without one the
    // presets table reads the sub-type as that filter plays it.
    let blinds = effect(
        r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-venetian-blinds" presentation:preset-sub-type="vertical""#,
        &format!(
            r#"{}<anim:transitionFilter smil:dur="0.5s" smil:targetElement="a" smil:type="blindsWipe" smil:subtype="horizontal"/>"#,
            show("a")
        ),
    );
    let label_only = effect(
        r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-venetian-blinds" presentation:preset-sub-type="vertical""#,
        &show("b"),
    );
    let squares = effect(
        r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-diagonal-squares" presentation:preset-sub-type="left-to-bottom""#,
        &format!(
            r#"{}<anim:transitionFilter smil:dur="0.5s" smil:targetElement="a" smil:type="waterfallWipe" smil:subtype="horizontalLeft"/>"#,
            show("a")
        ),
    );
    let lowered = lower_motion(&deck(
        "",
        &main(&[clicked(blinds), clicked(label_only), clicked(squares)]),
    ));
    let got = steps(&lowered);
    assert_eq!(
        got[0]["effect"]["native"]["params"],
        json!({ "orient": "horz" })
    );
    assert_eq!(
        got[1]["effect"]["native"]["params"],
        json!({ "orient": "horz" })
    );
    assert_eq!(
        got[2]["effect"]["native"]["params"],
        json!({ "corner": "rd" })
    );
}

#[test]
fn values_and_directions_come_from_the_behaviours() {
    let emphasis = |id: &str, leaf: &str| {
        effect(
            &format!(
                r#"presentation:node-type="on-click" presentation:preset-class="emphasis" presentation:preset-id="{id}""#
            ),
            leaf,
        )
    };
    let body = main(&[
        clicked(emphasis(
            "ooo-emphasis-fill-color",
            r##"<anim:animateColor smil:dur="0.5s" smil:targetElement="a" smil:attributeName="fill-color" smil:to="#e0442e"/>"##,
        )),
        clicked(emphasis(
            "ooo-emphasis-grow-and-shrink",
            r#"<anim:animateTransform smil:dur="0.5s" smil:targetElement="a" smil:attributeName="transform" smil:by="2,0.5" svg:type="scale"/>"#,
        )),
        clicked(emphasis(
            "ooo-emphasis-transparency",
            r#"<anim:set smil:dur="0.5s" smil:targetElement="a" smil:attributeName="opacity" smil:to="0.25"/>"#,
        )),
        clicked(emphasis(
            "ooo-emphasis-fill-color",
            r#"<anim:animateColor smil:dur="0.5s" smil:targetElement="a" smil:attributeName="fill-color" smil:to="crimson"/>"#,
        )),
        clicked(effect(
            r#"presentation:node-type="on-click" presentation:preset-class="motion-path" presentation:preset-id="ooo-motionpath-custom""#,
            r#"<anim:animateMotion smil:dur="0.5s" smil:targetElement="b" svg:path="m 0 0 l 0.5 0 v 0.25 z"/>"#,
        )),
        clicked(effect(
            r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-ease-in""#,
            &format!(
                r#"{}<anim:animate smil:dur="0.5s" smil:targetElement="b" smil:attributeName="x" smil:values="x-0.1;x" smil:keyTimes="0;1"/>"#,
                show("b")
            ),
        )),
        clicked(effect(
            r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-float""#,
            &format!(
                r#"{}<anim:animate smil:dur="0.5s" smil:targetElement="b" smil:attributeName="y" smil:values="y-0.4;y" smil:keyTimes="0;1"/>"#,
                show("b")
            ),
        )),
        clicked(effect(
            r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-descend""#,
            &format!(
                r#"{}<anim:animate smil:dur="0.5s" smil:targetElement="b" smil:attributeName="y" smil:values="y+0.4;y" smil:keyTimes="0;1"/>"#,
                show("b")
            ),
        )),
    ]);
    let lowered = lower_motion(&deck("", &body));
    let played: Vec<Value> = steps(&lowered)
        .iter()
        .map(|s| s["effect"]["native"]["effect"].clone())
        .collect();
    let params: Vec<Value> = steps(&lowered)
        .iter()
        .map(|s| s["effect"]["native"]["params"].clone())
        .collect();
    assert_eq!(params[0], json!({ "property": "fill", "to": "#e0442e" }));
    assert_eq!(params[1]["scale"], json!({ "x": 2, "y": 0.5 }));
    assert_eq!(params[2]["alpha"], 0.25);
    assert_eq!(params[3], json!({ "property": "fill", "to": "#000000" }));
    // The default page is 960 × 540 px.
    assert_eq!(
        params[4]["d"],
        json!([["M", 0, 0], ["L", 480, 0], ["L", 480, 135], ["Z"]])
    );
    // Ease-in and float are rows the presets table marks approximate: their own behaviours play.
    assert_eq!(played[5], "behaviors");
    assert_eq!(played[6], "behaviors");
    // Descend floats up when its behaviours move it up.
    assert_eq!(played[7], "floatIn");
    assert_eq!(params[7], json!({ "dir": "u" }));
    assert_eq!(codes(&lowered), ["OD9412"]);
}

/// An exit iris or split plays the entrance its filter reads as backwards, as LibreOffice's engine plays it:
/// `ooo-exit-box in` (`irisWipe` reversed, `smil:mode="out"`) is `iris{in}` reversed, a hole opening from the
/// centre, as is a custom exit with the same filter (a closing clip track); a split exit reads the same way.
/// Entrances, and a filter an exit plays forward (no `smil:mode="out"`), read the same.
#[test]
fn an_exits_in_out_names_the_entrance_its_filter_plays() {
    let filter = |target: &str, ty: &str, subtype: &str, reverse: bool, out: bool| {
        format!(
            r#"<anim:transitionFilter smil:dur="0.5s" smil:targetElement="{target}" smil:type="{ty}" smil:subtype="{subtype}"{}{}/>"#,
            if reverse {
                r#" smil:direction="reverse""#
            } else {
                ""
            },
            if out { r#" smil:mode="out""# } else { "" },
        )
    };
    let preset = |class: &str, id: &str, subtype: &str, leaves: String| {
        effect(
            &format!(
                r#"presentation:node-type="on-click" presentation:preset-class="{class}" presentation:preset-id="{id}" presentation:preset-sub-type="{subtype}""#
            ),
            &leaves,
        )
    };
    let lowered = lower_motion(&deck(
        "",
        &main(&[
            clicked(preset(
                "exit",
                "ooo-exit-box",
                "in",
                filter("a", "irisWipe", "rectangle", true, true),
            )),
            clicked(preset(
                "exit",
                "ooo-exit-split",
                "vertical-out",
                filter("b", "barnDoorWipe", "vertical", false, true),
            )),
            clicked(preset(
                "entrance",
                "ooo-entrance-box",
                "in",
                format!(
                    "{}{}",
                    show("a"),
                    filter("a", "irisWipe", "rectangle", true, false)
                ),
            )),
            clicked(effect(
                r#"presentation:node-type="on-click" presentation:preset-class="exit" presentation:preset-id="ooo-exit-probe-custom""#,
                &filter("b", "irisWipe", "rectangle", true, true),
            )),
            clicked(effect(
                r#"presentation:node-type="on-click" presentation:preset-class="exit" presentation:preset-id="ooo-exit-probe-custom""#,
                &filter("b", "irisWipe", "rectangle", true, false),
            )),
        ]),
    ));
    let got = steps(&lowered);
    let native = |i: usize| got[i]["effect"]["native"].clone();
    let played = |i: usize| {
        let native = native(i);
        json!({ "effect": native["effect"], "params": native["params"] })
    };
    assert_eq!(
        played(0),
        json!({ "effect": "iris", "params": { "shape": "box", "inOut": "in" } })
    );
    assert_eq!(
        played(1),
        json!({ "effect": "split", "params": { "orient": "vert", "inOut": "out" } })
    );
    assert_eq!(
        played(2),
        json!({ "effect": "iris", "params": { "shape": "box", "inOut": "in" } })
    );
    assert_eq!(native(3)["effect"], "behaviors");
    let track = &native(3)["params"][0];
    assert_eq!(track["property"], "clipPath");
    assert_eq!(track["transition"], "out");
    assert_eq!(
        track["reveal"],
        json!({ "effect": "iris", "params": { "shape": "box", "inOut": "in" } })
    );
    let track = &native(4)["params"][0];
    assert_eq!(track["transition"], "in");
    assert_eq!(
        track["reveal"],
        json!({ "effect": "iris", "params": { "shape": "box", "inOut": "in" } })
    );
}

#[test]
fn an_unknown_preset_plays_its_behaviours_as_tier_two() {
    let unknown = effect(
        r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-probe-unknown""#,
        &format!(
            r#"{}<anim:transitionFilter smil:dur="0.5s" smil:targetElement="a" smil:type="barWipe" smil:subtype="topToBottom" smil:direction="reverse"/><anim:animate smil:dur="0.5s" smil:targetElement="a" smil:attributeName="x" smil:values="x;x" smil:keyTimes="0;1" anim:formula="x+sin(2*pi*$)*0.05"/><anim:animateTransform smil:dur="0.5s" smil:targetElement="a" smil:attributeName="transform" smil:by="90" svg:type="rotate"/>"#,
            show("a")
        ),
    );
    let custom = effect(
        r#"presentation:node-type="on-click" presentation:preset-class="custom""#,
        r#"<anim:set smil:begin="0.5s" smil:dur="0.001s" smil:targetElement="b" smil:attributeName="visibility" smil:to="hidden"/><anim:animate smil:dur="0.5s" smil:targetElement="b" smil:attributeName="opacity" smil:values="1;0" smil:keyTimes="0;1"/>"#,
    );
    let lowered = lower_motion(&deck("", &main(&[clicked(unknown), clicked(custom)])));
    let got = steps(&lowered);
    let tracks = got[0]["effect"]["native"]["params"].as_array().unwrap();
    let properties: Vec<&str> = tracks
        .iter()
        .map(|t| t["property"].as_str().unwrap())
        .collect();
    assert_eq!(properties, ["clipPath", "rotate", "translate"]);
    assert_eq!(
        tracks[0]["reveal"],
        json!({ "effect": "wipe", "params": { "dir": "u", "soft": false } })
    );
    assert_eq!(tracks[2]["frames"].as_array().unwrap().len(), 9);
    assert_holds(
        &got[1],
        json!({ "kind": "exit", "effect": { "id": "odp:custom", "fallback": "dissolve", "native": { "effect": "behaviors" } } }),
    );
    assert!(codes(&lowered).is_empty(), "{:?}", codes(&lowered));
}

#[test]
fn what_tier_two_cannot_animate_is_reported_and_nothing_left_falls_back() {
    let partial = effect(
        r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-probe""#,
        &format!(
            r#"{}{}<anim:animate smil:dur="0.5s" smil:targetElement="a" smil:attributeName="skewX" smil:values="0;0.3"/>"#,
            show("a"),
            fade("a", "0.5s")
        ),
    );
    let opaque = effect(
        r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-probe-opaque""#,
        &format!(
            r#"{}<anim:set smil:dur="0.5s" smil:targetElement="b" smil:attributeName="xshear" smil:to="0.3"/>"#,
            show("b")
        ),
    );
    let lowered = lower_motion(&deck("", &main(&[clicked(partial), clicked(opaque)])));
    let got = steps(&lowered);
    assert_eq!(got[0]["effect"]["native"]["effect"], "behaviors");
    assert!(got[1]["effect"]["native"].is_null());
    assert_eq!(got[1]["effect"]["fallback"], "dissolve");
    assert_eq!(
        coded(&lowered),
        [
            ("OD9410".to_owned(), "odp:ooo-entrance-probe".to_owned()),
            (
                "OD9410".to_owned(),
                "odp:ooo-entrance-probe-opaque".to_owned()
            ),
        ]
    );
}

#[test]
fn media_calls_and_ole_actions() {
    let call = |id: &str, command: &str| {
        effect(
            &format!(
                r#"presentation:node-type="on-click" presentation:preset-class="media-call" presentation:preset-id="{id}""#
            ),
            &format!(
                r#"<anim:command smil:begin="0s" smil:targetElement="b" anim:command="{command}"/>"#
            ),
        )
    };
    let ole = effect(
        r#"presentation:node-type="on-click" presentation:preset-class="ole-action""#,
        r#"<anim:command smil:targetElement="b" anim:command="verb"/>"#,
    );
    let body = main(&[
        clicked(call("ooo-media-start", "play")),
        clicked(call("ooo-media-toggle-pause", "toggle-pause")),
        clicked(call("ooo-media-stop", "stop")),
        clicked(ole),
    ]);
    let lowered = lower_motion(&deck("", &body));
    let effects: Vec<(Value, Value)> = steps(&lowered)
        .iter()
        .map(|s| (s["kind"].clone(), s["effect"]["native"]["effect"].clone()))
        .collect();
    assert_eq!(
        effects,
        [
            (json!("media"), json!("play")),
            (json!("media"), json!("toggle")),
            (json!("media"), json!("stop"))
        ]
    );
    // A play command on a shape with no medium has no length to last: it reads as the default duration, reported.
    // Pause and stop are instants.
    let durations: Vec<Value> = steps(&lowered)
        .iter()
        .map(|s| s["durationMs"].clone())
        .collect();
    assert_eq!(durations, [json!(500), json!(0), json!(0)]);
    assert_eq!(codes(&lowered), ["OD9410", "OD9411"]);
}

/// The probe clip: a one-second MP4 (`probes/motion/media`).
const CLIP: &[u8] = include_bytes!("../../../probes/motion/media/clip.mp4");

/// A play command lasts its medium's own length, read from the movie, whatever its own `smil:dur` (LibreOffice's
/// `effects.xml` writes 0.001 s on every command); stop and toggle-pause are instants that last their own duration.
#[test]
fn a_play_command_lasts_its_mediums_length() {
    let movie = r#"<draw:frame svg:x="2cm" svg:y="8cm" svg:width="4cm" svg:height="3cm" xml:id="m"><draw:plugin xlink:href="Media/clip.mp4" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad" draw:mime-type="video/mp4"/></draw:frame>"#;
    let call = |id: &str, command: &str, extra: &str| {
        effect(
            &format!(
                r#"presentation:node-type="on-click" presentation:preset-class="media-call" presentation:preset-id="{id}""#
            ),
            &format!(
                r#"<anim:command smil:begin="0s" smil:targetElement="m" anim:command="{command}"{extra}/>"#
            ),
        )
    };
    let bytes = |clip: &[u8]| {
        odp(&OdpSpec {
            parts: vec![("Media/clip.mp4".to_owned(), clip.to_vec())],
            ..OdpSpec::pages(vec![PageSpec::new().attrs(r#" xml:id="s1""#).with_body(
                format!(
                    "{movie}{}",
                    main(&[
                        clicked(call("ooo-media-start", "play", "")),
                        clicked(call("ooo-media-stop", "stop", "")),
                        clicked(call("ooo-media-start", "play", r#" smil:dur="0.001""#)),
                        clicked(call("ooo-media-stop", "stop", r#" smil:dur="0.001""#)),
                        clicked(call(
                            "ooo-media-toggle-pause",
                            "toggle-pause",
                            r#" smil:dur="0.001""#,
                        )),
                    ])
                ),
            )])
        })
    };
    let lowered = lower_motion(&bytes(CLIP));
    let durations: Vec<Value> = steps(&lowered)
        .iter()
        .map(|s| s["durationMs"].clone())
        .collect();
    assert_eq!(
        durations,
        [json!(1000), json!(0), json!(1000), json!(1), json!(1)]
    );
    assert!(codes(&lowered).is_empty(), "{:?}", codes(&lowered));
    // A medium whose metadata does not read gives no length.
    let mut broken = CLIP.to_vec();
    let at = broken.windows(4).position(|w| w == b"mvhd").unwrap();
    broken[at..at + 4].copy_from_slice(b"xxxx");
    let lowered = lower_motion(&bytes(&broken));
    let durations: Vec<Value> = steps(&lowered)
        .iter()
        .map(|s| s["durationMs"].clone())
        .collect();
    assert_eq!(
        durations,
        [json!(500), json!(0), json!(500), json!(1), json!(1)]
    );
    // The two plays of the one medium report the same loss once.
    assert_eq!(codes(&lowered), ["OD9411"]);
}

#[test]
fn a_vocabulary_effect_reports_the_sound_it_drops() {
    let loud = effect(
        r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-fade-in""#,
        &format!(
            r#"{}{}<anim:audio smil:begin="0s" xlink:href="Media/ding.wav"/>"#,
            show("a"),
            fade("a", "0.5s")
        ),
    );
    let lowered = lower_motion(&deck("", &main(&[clicked(loud), clicked(fade_in("b"))])));
    assert_eq!(steps(&lowered)[0]["effect"]["native"]["effect"], "fade");
    assert_eq!(
        coded(&lowered),
        [("OD9410".to_owned(), "odp:ooo-entrance-fade-in".to_owned())]
    );
    assert_eq!(
        lowered.diagnostics[0].message,
        "The odp:ooo-entrance-fade-in build effect has no exact equivalent; it plays fade without its sound"
    );
}

#[test]
fn an_effect_aimed_at_nothing_is_left_out() {
    let lowered = lower_motion(&deck(
        "",
        &main(&[clicked(fade_in("ghost")), clicked(fade_in("a"))]),
    ));
    assert_eq!(steps(&lowered).len(), 1);
    assert_eq!(codes(&lowered), ["OD9411"]);
}

// Hostile timing.

#[test]
fn start_conditions_on_nodes_are_dropped_without_looping() {
    let on = |id: &str, begin: &str, target: &str| {
        effect(
            &format!(
                r#"xml:id="{id}" smil:begin="{begin}" presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-fade-in""#
            ),
            &format!("{}{}", show(target), fade(target, "0.5s")),
        )
    };
    let body = main(&[
        group("next", &[("0s", on("e0", "e0.end", "a"))]),
        group("next", &[("0s", on("e1", "e2.begin", "a"))]),
        group("next", &[("0s", on("e2", "e1.end", "b"))]),
        group("next", &[("e9.end", on("e3", "0s", "b"))]),
    ]);
    let lowered = lower_motion(&deck("", &body));
    assert!(steps(&lowered).is_empty());
    // Each effect left out, and the slide that shows fully built.
    assert_eq!(
        codes(&lowered),
        ["OD9401", "OD9411", "OD9411", "OD9411", "OD9411"]
    );
}

#[test]
fn effects_nested_past_the_depth_limit_are_dropped() {
    let mut nested = fade_in("a");
    for _ in 0..40 {
        nested = format!(r#"<anim:par smil:begin="0s">{nested}</anim:par>"#);
    }
    let lowered = lower_motion(&deck(
        "",
        &main(&[group("next", &[("0s", nested)]), clicked(fade_in("b"))]),
    ));
    let got = steps(&lowered);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["trigger"], "click");
    assert_eq!(codes(&lowered), ["OD9411"]);
}

#[test]
fn timing_values_clamp_or_read_as_the_default() {
    let timed = |begin: &str, dur: &str, extra: &str| {
        effect(
            &format!(
                r#"smil:begin="{begin}" {extra} presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-fade-in""#
            ),
            &format!("{}{}", show("a"), fade("a", dur)),
        )
    };
    let body = main(&[
        clicked(timed("0s", "4294967.295s", "")),
        clicked(timed("99999999999s", "0.5s", "")),
        clicked(timed("0s", "NaN", "")),
        clicked(timed("0s", "-0.5s", "")),
        clicked(timed("-1s", "0.5s", "")),
        clicked(timed("0s", "0.5s", r#"smil:repeatCount="4294967""#)),
        clicked(timed("0s", "0.5s", r#"smil:repeatCount="-2""#)),
    ]);
    let lowered = lower_motion(&deck("", &body));
    let got = steps(&lowered);
    assert_eq!(got.len(), 7);
    assert_eq!(got[0]["durationMs"], 3_600_000);
    assert_holds(
        &got[1],
        json!({ "delayMs": 3_600_000, "timing": { "offsetMs": 3_600_000 } }),
    );
    assert_eq!(got[2]["durationMs"], 500);
    assert_eq!(got[3]["durationMs"], 500);
    assert_eq!(got[4]["delayMs"], 0);
    assert!(got[4]["timing"]["offsetMs"].is_null());
    assert_eq!(got[5]["timing"]["repeat"], json!({ "count": 1000 }));
    assert!(got[6]["timing"]["repeat"].is_null());
    assert_eq!(codes(&lowered), ["OD9411"; 7]);
}

#[test]
fn a_formula_past_its_limits_does_not_compile() {
    let deep = format!("{}y{}", "(".repeat(400), ")".repeat(400));
    let long = "y+".repeat(600) + "y";
    let body = main(&[clicked(effect(
        r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-probe-deep""#,
        &format!(
            r#"{}<anim:animate smil:dur="0.5s" smil:targetElement="a" smil:attributeName="y" smil:values="y;y" smil:keyTimes="0;1" anim:formula="{deep}"/><anim:animate smil:dur="0.5s" smil:targetElement="a" smil:attributeName="x" smil:values="x;x" smil:keyTimes="0;1" anim:formula="{long}"/>"#,
            show("a")
        ),
    ))]);
    let lowered = lower_motion(&deck("", &body));
    let got = steps(&lowered);
    assert!(got[0]["effect"]["native"].is_null());
    assert_eq!(codes(&lowered), ["OD9410"]);
}

#[test]
fn a_slide_keeps_at_most_its_step_limit() {
    let effects: Vec<String> = (0..2001).map(|_| clicked(fade_in("a"))).collect();
    let lowered = lower_motion(&deck("", &main(&effects)));
    assert_eq!(
        steps(&lowered).len(),
        slidock_ir::effects::limits::STEPS_PER_SLIDE
    );
    assert_eq!(codes(&lowered), ["OD9411"]);
}

#[test]
fn a_timing_tree_past_its_node_limit_stops_with_a_diagnostic() {
    let leaves = show("a").repeat(20_001);
    let body = main(&[clicked(effect(
        r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-appear""#,
        &leaves,
    ))]);
    let lowered = lower_motion(&deck("", &body));
    assert_eq!(steps(&lowered).len(), 1);
    assert_eq!(codes(&lowered), ["OD9411"]);
}

#[test]
fn interactive_sequences_are_dropped() {
    let body = root(&format!(
        r#"<anim:seq presentation:node-type="interactive-sequence" smil:begin="b.click">{}</anim:seq>"#,
        clicked(fade_in("a"))
    ));
    let lowered = lower_motion(&deck("", &body));
    assert!(steps(&lowered).is_empty());
    assert_eq!(codes(&lowered), ["OD9401", "OD9404"]);
}

// Fill, missing values and losses.

/// An on-click effect node of `class` and `preset` with `smil:fill="{fill}"` and its behaviours.
fn filled(fill: &str, class: &str, preset: &str, leaves: &str) -> String {
    format!(
        r#"<anim:par smil:fill="{fill}" presentation:node-type="on-click" presentation:preset-class="{class}" presentation:preset-id="{preset}">{leaves}</anim:par>"#
    )
}

const GROW: &str = r#"<anim:animateTransform smil:dur="0.5s" smil:fill="remove" smil:targetElement="b" svg:type="scale" smil:to="1.5,1.5"/>"#;

#[test]
fn an_emphasis_holds_unless_it_rewinds() {
    let body = main(&[
        clicked(filled(
            "remove",
            "emphasis",
            "ooo-emphasis-grow-and-shrink",
            GROW,
        )),
        clicked(filled(
            "hold",
            "emphasis",
            "ooo-emphasis-grow-and-shrink",
            GROW,
        )),
        clicked(filled(
            "freeze",
            "emphasis",
            "ooo-emphasis-grow-and-shrink",
            GROW,
        )),
        clicked(filled(
            "default",
            "emphasis",
            "ooo-emphasis-grow-and-shrink",
            GROW,
        )),
    ]);
    let lowered = lower_motion(&deck("", &body));
    let holds: Vec<Value> = steps(&lowered)
        .iter()
        .map(|step| step["effect"]["native"]["params"]["hold"].clone())
        .collect();
    assert_eq!(holds, [false, true, true, true]);
    assert!(codes(&lowered).is_empty(), "{:?}", codes(&lowered));
}

#[test]
fn a_rewind_the_timeline_cannot_hold_is_reported() {
    let fade_leaves = format!("{}{}", show("a"), fade("a", "0.5s"));
    let body = main(&[
        // An entrance that hides again, and a colour change that returns: neither has a rewind.
        clicked(filled(
            "remove",
            "entrance",
            "ooo-entrance-fade-in",
            &fade_leaves,
        )),
        clicked(filled(
            "remove",
            "emphasis",
            "ooo-emphasis-fill-color",
            r##"<anim:animateColor smil:dur="0.5s" smil:targetElement="a" smil:attributeName="fill-color" smil:to="#e0442e"/>"##,
        )),
        // A teeter returns on its own.
        clicked(filled(
            "remove",
            "emphasis",
            "ooo-emphasis-teeter",
            r#"<anim:animateTransform smil:dur="0.5s" smil:targetElement="b" svg:type="rotate" smil:values="0;10;0" smil:keyTimes="0;0.5;1"/>"#,
        )),
    ]);
    let lowered = lower_motion(&deck("", &body));
    assert_eq!(steps(&lowered).len(), 3);
    assert_eq!(steps(&lowered)[2]["effect"]["native"]["effect"], "teeter");
    let mut got = coded(&lowered);
    got.sort();
    assert_eq!(
        got,
        [
            (
                "OD9411".to_owned(),
                "odp:ooo-emphasis-fill-color".to_owned()
            ),
            ("OD9411".to_owned(), "odp:ooo-entrance-fade-in".to_owned()),
        ]
    );
    let what: Vec<_> = lowered
        .diagnostics
        .iter()
        .filter_map(|d| match d.data.as_ref()?.get("what")? {
            DataValue::Str(s) => Some(s.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(what, ["fill", "fill"]);
}

#[test]
fn an_unknown_fill_reads_as_the_presets_and_is_reported() {
    let lowered = lower_motion(&deck(
        "",
        &main(&[clicked(filled(
            "sometimes",
            "emphasis",
            "ooo-emphasis-grow-and-shrink",
            GROW,
        ))]),
    ));
    assert_eq!(
        steps(&lowered)[0]["effect"]["native"]["params"]["hold"],
        true
    );
    assert_eq!(codes(&lowered), ["OD9412"]);
}

#[test]
fn a_group_whose_start_names_a_node_drops_its_effects() {
    let body = main(&[
        group("e9.end", &[("0s", fade_in("a"))]),
        format!(
            r#"<anim:par xml:id="grp" smil:begin="grp.end"><anim:par smil:begin="0s">{}</anim:par></anim:par>"#,
            fade_in("b")
        ),
        clicked(fade_in("b")),
    ]);
    let lowered = lower_motion(&deck("", &body));
    let got = steps(&lowered);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["trigger"], "click");
    assert_eq!(codes(&lowered), ["OD9411"; 2]);
}

#[test]
fn an_animation_with_no_duration_plays_the_default_and_is_reported() {
    let undated = effect(
        r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-fade-in""#,
        &format!(
            r#"{}<anim:transitionFilter smil:targetElement="b" smil:type="fade" smil:subtype="crossfade"/>"#,
            show("b")
        ),
    );
    let lowered = lower_motion(&deck("", &main(&[clicked(undated)])));
    assert_eq!(steps(&lowered)[0]["durationMs"], 500);
    assert_eq!(codes(&lowered), ["OD9411"]);
}

#[test]
fn behaviours_nested_past_the_depth_limit_are_reported() {
    let mut nested = fade("b", "0.5s");
    for _ in 0..40 {
        nested = format!(r#"<anim:par smil:begin="0s">{nested}</anim:par>"#);
    }
    let deep = effect(
        r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-fade-in""#,
        &format!("{}{nested}", show("b")),
    );
    let lowered = lower_motion(&deck("", &main(&[clicked(deep)])));
    assert_eq!(steps(&lowered).len(), 1);
    let what: Vec<_> = lowered
        .diagnostics
        .iter()
        .filter_map(|d| match d.data.as_ref()?.get("what")? {
            DataValue::Str(s) => Some(s.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(what, ["behaviours"]);
}

#[test]
fn an_unknown_auto_reverse_reads_as_false_and_is_reported() {
    let reversed = |token: &str| {
        effect(
            &format!(
                r#"smil:autoReverse="{token}" presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-fade-in""#
            ),
            &format!("{}{}", show("a"), fade("a", "0.5s")),
        )
    };
    let lowered = lower_motion(&deck(
        "",
        &main(&[clicked(reversed("yes")), clicked(reversed("true"))]),
    ));
    let got = steps(&lowered);
    assert!(got[0]["timing"]["autoReverse"].is_null());
    assert_eq!(got[1]["timing"]["autoReverse"], true);
    assert_eq!(codes(&lowered), ["OD9412"]);
}

/// A row the presets table marks approximate plays the effect's own behaviours (tier 2); only when they compile to
/// nothing does the approximation play, noted (OD9413).
#[test]
fn an_approximate_preset_plays_its_behaviours() {
    let effect_of = |preset: &str, behaviours: &str| {
        effect(
            &format!(
                r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="{preset}" presentation:preset-sub-type="across""#
            ),
            &format!("{}{behaviours}", show("b")),
        )
    };
    let width = r#"<anim:animate smil:dur="0.5s" smil:targetElement="b" smil:attributeName="width" smil:values="0;width" smil:keyTimes="0;1"/>"#;
    let colour = r#"<anim:set smil:dur="0.5s" smil:targetElement="b" smil:attributeName="xshear" smil:to="0.3"/>"#;
    let lowered = lower_motion(&deck(
        "",
        &main(&[
            clicked(effect_of("ooo-entrance-stretchy", width)),
            clicked(effect_of("ooo-entrance-unfold", width)),
            clicked(effect_of("ooo-entrance-unfold", colour)),
        ]),
    ));
    let played: Vec<_> = steps(&lowered)
        .iter()
        .map(|step| step["effect"]["native"]["effect"].clone())
        .collect();
    assert_eq!(
        played,
        [json!("stretch"), json!("behaviors"), json!("stretch")]
    );
    let notes: Vec<_> = lowered
        .diagnostics
        .iter()
        .map(|d| {
            let reason = match d.data.as_ref().and_then(|data| data.get("reason")) {
                Some(DataValue::Str(s)) => s.clone(),
                _ => String::new(),
            };
            (d.code.code().to_owned(), reason)
        })
        .collect();
    assert_eq!(
        notes,
        [(
            "OD9413".to_owned(),
            "stretch only approximates the preset, and its behaviours compile to nothing"
                .to_owned()
        )]
    );
}

#[test]
fn many_position_animations_compile_within_the_track_limit() {
    // No two axes share a span, so none join: the worst case of the join.
    let mut leaves = show("b");
    for i in 0..9_000 {
        leaves.push_str(&format!(
            r#"<anim:animate smil:dur="{}ms" smil:targetElement="b" smil:attributeName="x" smil:values="x;x+0.1" smil:keyTimes="0;1"/><anim:animate smil:dur="{}ms" smil:targetElement="b" smil:attributeName="y" smil:values="y;y+0.1" smil:keyTimes="0;1"/>"#,
            2 * i + 1,
            2 * i + 2
        ));
    }
    let wide = effect(
        r#"presentation:node-type="on-click" presentation:preset-class="entrance" presentation:preset-id="ooo-entrance-probe-wide""#,
        &leaves,
    );
    let lowered = lower_motion(&deck("", &main(&[clicked(wide)])));
    let got = steps(&lowered);
    let tracks = got[0]["effect"]["native"]["params"].as_array().unwrap();
    assert_eq!(tracks.len(), 32);
    assert!(tracks.iter().all(|t| t["property"] == "translate"));
    assert_eq!(codes(&lowered), ["OD9410"]);
}

#[test]
fn native_motion_detects_what_it_lowers_and_reports_a_tree_that_lowered_nothing() {
    let id = |lowered: &Lowered| slide(lowered)["id"].as_str().unwrap().to_owned();
    let animated = lower_motion(&deck(
        r#"smil:type="pushWipe" smil:subtype="fromTop""#,
        &main(&[clicked(fade_in("a"))]),
    ));
    let features = animated.features(&id(&animated));
    assert!(features.contains(&"slide.timeline"), "{features:?}");
    assert!(features.contains(&"slide.transition"), "{features:?}");
    assert!(!features.contains(&"animation"), "{features:?}");
    // An OLE action is left out with an info note: the slide shows fully built, which is reported.
    let ole = effect(
        r#"presentation:node-type="on-click" presentation:preset-class="ole-action""#,
        r#"<anim:command smil:targetElement="b" anim:command="verb"/>"#,
    );
    let lowered = lower_motion(&deck("", &main(&[clicked(ole)])));
    assert!(steps(&lowered).is_empty());
    assert!(lowered.features(&id(&lowered)).contains(&"animation"));
    assert_eq!(codes(&lowered), ["OD9401", "OD9410"]);
}
