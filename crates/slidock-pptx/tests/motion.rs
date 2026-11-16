//! Native motion, rule by rule: the parity path, transitions (alternate content, options, advance, sound) and the
//! timing extraction (click groups, triggers, offsets, durations, targets, delivery, repeats, after effects, tier 1
//! and tier 2, interactive sequences) with the bounds that keep a hostile tree in check.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde_json::{Value, json};
use slidock_ir::effects::limits::{STEPS_PER_SLIDE, TIMING_DEPTH, TIMING_NODES};
use slidock_ir::source::Slide;
use slidock_ir::timeline::{Delivery, StepFallback, Trigger};
use support::{PptxSpec, Sp, body, compile_motion, lower, lower_native, pptx, slide, sp, xfrm};

/// A slide of four text shapes (ids 3 to 6, three paragraphs each) and `tail` after its tree.
fn deck(tail: &str) -> Vec<u8> {
    let shapes: String = (3..=6)
        .map(|id| {
            sp(Sp {
                id,
                sp_pr: xfrm(72.0 * id as f64, 72.0, 144.0, 72.0, ""),
                tx_body: Some(body(&["one", "two", "three"], "<a:bodyPr/>")),
                ..Sp::default()
            })
        })
        .collect();
    let mut spec = slide(shapes);
    spec.tail = tail.to_owned();
    pptx(PptxSpec {
        slides: vec![spec],
        ..PptxSpec::default()
    })
}

fn target(spid: u32) -> String {
    format!("<p:tgtEl><p:spTgt spid=\"{spid}\"/></p:tgtEl>")
}

fn cbhvr(dur: &str, tgt: &str, attr: &str) -> String {
    let names = if attr.is_empty() {
        String::new()
    } else {
        format!("<p:attrNameLst><p:attrName>{attr}</p:attrName></p:attrNameLst>")
    };
    format!("<p:cBhvr><p:cTn id=\"0\" dur=\"{dur}\" fill=\"hold\"/>{tgt}{names}</p:cBhvr>")
}

fn show(tgt: &str) -> String {
    format!(
        "<p:set>{}<p:to><p:strVal val=\"visible\"/></p:to></p:set>",
        cbhvr("1", tgt, "style.visibility")
    )
}

fn fade_in(tgt: &str, dur: &str) -> String {
    format!(
        "<p:animEffect transition=\"in\" filter=\"fade\">{}</p:animEffect>",
        cbhvr(dur, tgt, "")
    )
}

/// A fade entrance on `spid`: the preset's set and filter.
fn fade(spid: u32) -> String {
    let tgt = target(spid);
    format!("{}{}", show(&tgt), fade_in(&tgt, "500"))
}

/// An effect node (`p:par`) with its attributes, start delay and behaviours.
fn effect(attrs: &str, delay: &str, behaviours: &str) -> String {
    format!(
        "<p:par><p:cTn id=\"9\" fill=\"hold\" {attrs}><p:stCondLst><p:cond delay=\"{delay}\"/></p:stCondLst><p:childTnLst>{behaviours}</p:childTnLst></p:cTn></p:par>"
    )
}

fn slot(begin: u32, effects: &str) -> String {
    format!(
        "<p:par><p:cTn id=\"8\" fill=\"hold\"><p:stCondLst><p:cond delay=\"{begin}\"/></p:stCondLst><p:childTnLst>{effects}</p:childTnLst></p:cTn></p:par>"
    )
}

fn click_group(slots: &str) -> String {
    format!(
        "<p:par><p:cTn id=\"7\" fill=\"hold\"><p:stCondLst><p:cond delay=\"indefinite\"/></p:stCondLst><p:childTnLst>{slots}</p:childTnLst></p:cTn></p:par>"
    )
}

fn auto_group(slots: &str) -> String {
    format!(
        "<p:par><p:cTn id=\"7\" fill=\"hold\"><p:stCondLst><p:cond delay=\"indefinite\"/><p:cond evt=\"onBegin\" delay=\"0\"><p:tn val=\"2\"/></p:cond></p:stCondLst><p:childTnLst>{slots}</p:childTnLst></p:cTn></p:par>"
    )
}

/// A timing tree whose main sequence holds `groups`, with `rest` after it in the root and the build list `bld`.
fn timing_with(groups: &str, rest: &str, bld: &str) -> String {
    let bld = if bld.is_empty() {
        String::new()
    } else {
        format!("<p:bldLst>{bld}</p:bldLst>")
    };
    format!(
        "<p:timing><p:tnLst><p:par><p:cTn id=\"1\" dur=\"indefinite\" restart=\"never\" nodeType=\"tmRoot\"><p:childTnLst><p:seq concurrent=\"1\" nextAc=\"seek\"><p:cTn id=\"2\" dur=\"indefinite\" nodeType=\"mainSeq\"><p:childTnLst>{groups}</p:childTnLst></p:cTn></p:seq>{rest}</p:childTnLst></p:cTn></p:par></p:tnLst>{bld}</p:timing>"
    )
}

fn timing(groups: &str) -> String {
    timing_with(groups, "", "")
}

const ENTR_FADE: &str = "presetID=\"10\" presetClass=\"entr\" presetSubtype=\"0\" grpId=\"0\"";

fn native(bytes: &[u8]) -> support::Lowered {
    lower_native(bytes)
}

fn first_slide(lowered: &support::Lowered) -> &Slide {
    &lowered.presentation().slides[0]
}

fn steps(lowered: &support::Lowered) -> Vec<Value> {
    first_slide(lowered)
        .timeline
        .iter()
        .map(|step| serde_json::to_value(step).unwrap())
        .collect()
}

/// The data of each diagnostic with `code`.
fn reported(lowered: &support::Lowered, code: &str) -> Vec<Value> {
    lowered
        .diagnostics
        .iter()
        .filter(|d| d.code.code() == code)
        .map(|d| serde_json::to_value(d).unwrap()["data"].clone())
        .collect()
}

#[test]
fn the_parity_path_lowers_no_motion() {
    let tail = format!(
        "<p:transition spd=\"med\"><p:cover dir=\"r\"/></p:transition>{}",
        timing(&click_group(&slot(0, &effect(ENTR_FADE, "0", &fade(3)))))
    );
    let bytes = deck(&tail);
    let parity = lower(&bytes);
    let slide = first_slide(&parity);
    assert!(slide.timeline.is_empty());
    let transition = slide.transition.as_ref().unwrap();
    assert!(transition.effect.native.is_none());
    assert_eq!(transition.effect.id.to_string(), "pptx:cover");
    let mut codes: Vec<&str> = parity.diagnostics.iter().map(|d| d.code.code()).collect();
    codes.sort_unstable();
    assert_eq!(codes, ["PX2401", "PX2402"]);
    let deck_path = native(&bytes);
    let codes: Vec<&str> = deck_path
        .diagnostics
        .iter()
        .map(|d| d.code.code())
        .collect();
    assert_eq!(codes, Vec::<&str>::new());
    assert_eq!(first_slide(&deck_path).timeline.len(), 1);
    let transition = first_slide(&deck_path).transition.as_ref().unwrap();
    // The IR fallback keeps the parity path's rule: a cover is a dissolve there, though the vocabulary's is push.
    assert_eq!(
        serde_json::to_value(transition).unwrap(),
        json!({
            "effect": { "id": "pptx:cover", "fallback": "dissolve",
                "native": { "id": "pptx:cover", "effect": "cover", "params": { "dir": "r" } } },
            "durationMs": 750, "advance": { "onClick": true }
        })
    );
}

#[test]
fn transitions_keep_the_parity_paths_fallbacks_per_native_id() {
    for (child, fallback) in [
        ("<p:fade/>", "dissolve"),
        ("<p:dissolve/>", "dissolve"),
        ("<p:push dir=\"u\"/>", "push"),
        ("<p:wipe/>", "wipe"),
        ("<p:cover/>", "dissolve"),
        ("<p:pull/>", "dissolve"),
        ("<p:split/>", "dissolve"),
        ("<p:circle/>", "dissolve"),
        ("<p:cut/>", "dissolve"),
        ("<p:zoom/>", "dissolve"),
        ("<p:randomBar/>", "dissolve"),
        ("<p14:vortex dir=\"l\"/>", "dissolve"),
        ("", "none"),
    ] {
        let lowered = native(&deck(&format!(
            "<p:transition xmlns:p14=\"http://schemas.microsoft.com/office/powerpoint/2010/main\">{child}</p:transition>"
        )));
        let transition = first_slide(&lowered).transition.as_ref().unwrap();
        assert_eq!(
            serde_json::to_value(transition.effect.fallback).unwrap(),
            json!(fallback),
            "{child}"
        );
    }
}

fn alternate(requires: &str, choice: &str, fallback: &str) -> String {
    format!(
        "<mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><mc:Choice xmlns:p14=\"http://schemas.microsoft.com/office/powerpoint/2010/main\" xmlns:p15=\"http://schemas.microsoft.com/office/powerpoint/2012/main\" xmlns:p159=\"http://schemas.microsoft.com/office/powerpoint/2015/09/main\" xmlns:p99=\"urn:example:p99\" Requires=\"{requires}\"><p:transition spd=\"med\" p14:dur=\"1600\">{choice}</p:transition></mc:Choice><mc:Fallback><p:transition spd=\"med\">{fallback}</p:transition></mc:Fallback></mc:AlternateContent>"
    )
}

fn native_transition(tail: &str) -> (Value, Vec<String>) {
    let lowered = native(&deck(tail));
    let transition =
        serde_json::to_value(first_slide(&lowered).transition.as_ref().unwrap()).unwrap();
    let codes = lowered
        .diagnostics
        .iter()
        .map(|d| d.code.code().to_owned())
        .collect();
    (transition, codes)
}

#[test]
fn transitions_read_the_richest_choice_they_understand() {
    // p159 morph: the parity path reads the fallback (PX2005); native motion reads the choice.
    let tail = alternate("p159", "<p159:morph option=\"byWord\"/>", "<p:fade/>");
    let (transition, codes) = native_transition(&tail);
    assert_eq!(
        transition["effect"]["native"],
        json!({ "id": "pptx:morph", "effect": "morph", "params": { "by": "word" } })
    );
    assert_eq!(transition["effect"]["id"], "pptx:morph");
    assert_eq!(transition["durationMs"], 1600);
    assert!(codes.is_empty(), "{codes:?}");
    let parity = lower(&deck(&tail));
    let effect = &first_slide(&parity).transition.as_ref().unwrap().effect;
    assert_eq!(effect.id.to_string(), "pptx:fade");
    assert_eq!(parity.diagnostics[0].code.code(), "PX2005");
    // p15 presets name their preset; none has a recipe.
    let (transition, codes) = native_transition(&alternate(
        "p15",
        "<p15:prstTrans prst=\"curtains\"/>",
        "<p:fade/>",
    ));
    assert_eq!(transition["effect"]["id"], "pptx:prstTrans/curtains");
    assert!(transition["effect"].get("native").is_none());
    assert_eq!(codes, ["PX2402"]);
    // A choice whose requirements are not understood reads the fallback.
    let (transition, _) = native_transition(&alternate("p99", "<p99:x/>", "<p:push dir=\"d\"/>"));
    assert_eq!(
        transition["effect"]["native"]["params"],
        json!({ "dir": "d" })
    );
    assert_eq!(transition["durationMs"], 750);
}

#[test]
fn transition_options_fail_closed() {
    let (transition, codes) =
        native_transition("<p:transition><p:cover dir=\"sideways\"/></p:transition>");
    assert_eq!(
        transition["effect"]["native"]["params"],
        json!({ "dir": "l" })
    );
    assert_eq!(codes, ["PX2412"]);
    let (transition, codes) =
        native_transition("<p:transition advTm=\"-5\" advClick=\"0\"><p:fade/></p:transition>");
    assert_eq!(transition["advance"], json!({ "onClick": false }));
    assert_eq!(codes, ["PX2411"]);
    let (transition, codes) =
        native_transition("<p:transition advTm=\"99999999\"><p:fade/></p:transition>");
    assert_eq!(
        transition["advance"],
        json!({ "onClick": true, "afterMs": 3_600_000 })
    );
    assert_eq!(codes, ["PX2411"]);
    let (_, codes) = native_transition(
        "<p:transition><p:fade/><p:sndAc><p:stSnd><p:snd r:embed=\"rId9\" name=\"x\"/></p:stSnd></p:sndAc></p:transition>",
    );
    assert_eq!(codes, ["PX2411"]);
    let (transition, codes) = native_transition("<p:transition><p:unknownEffect/></p:transition>");
    assert!(transition["effect"].get("native").is_none());
    assert_eq!(transition["effect"]["fallback"], "dissolve");
    assert_eq!(codes, ["PX2402"]);
}

/// A transition the vocabulary only approximates is reported once (PX2413); one without an effect only advances,
/// so it cuts, with nothing to report.
#[test]
fn approximate_and_effectless_transitions() {
    let tail = alternate("p14", "<p14:warp dir=\"in\"/>", "<p:fade/>");
    let (transition, codes) = native_transition(&tail);
    assert_eq!(transition["effect"]["native"]["effect"], "zoom");
    assert_eq!(codes, ["PX2413"]);
    let (transition, codes) = native_transition("<p:transition spd=\"med\" advTm=\"2000\"/>");
    assert_eq!(
        transition["effect"]["native"],
        json!({ "id": "pptx:none", "effect": "none" })
    );
    assert_eq!(transition["effect"]["fallback"], "none");
    assert!(codes.is_empty(), "{codes:?}");
    // The parity path keeps no native effect.
    let parity = lower(&deck("<p:transition spd=\"med\" advTm=\"2000\"/>"));
    let transition = first_slide(&parity).transition.as_ref().unwrap();
    assert!(transition.effect.native.is_none());
}

#[test]
fn click_groups_and_node_types_decide_triggers_and_offsets() {
    let first = slot(
        0,
        &format!(
            "{}{}",
            effect(
                &format!("{ENTR_FADE} nodeType=\"clickEffect\""),
                "0",
                &fade(3)
            ),
            effect(
                &format!("{ENTR_FADE} nodeType=\"withEffect\""),
                "250",
                &fade(4)
            ),
        ),
    );
    let second = slot(
        750,
        &effect(
            &format!("{ENTR_FADE} nodeType=\"afterEffect\""),
            "500",
            &fade(5),
        ),
    );
    let auto = auto_group(&slot(
        0,
        &effect(
            &format!("{ENTR_FADE} nodeType=\"afterEffect\""),
            "0",
            &fade(6),
        ),
    ));
    let lowered = native(&deck(&timing(&format!(
        "{auto}{}",
        click_group(&format!("{first}{second}"))
    ))));
    let steps = steps(&lowered);
    let triggers: Vec<&str> = steps
        .iter()
        .map(|s| s["trigger"].as_str().unwrap())
        .collect();
    assert_eq!(
        triggers,
        ["afterPrevious", "click", "withPrevious", "afterPrevious"]
    );
    let offsets: Vec<&Value> = steps.iter().map(|s| &s["timing"]["offsetMs"]).collect();
    assert_eq!(offsets, [&json!(0), &json!(0), &json!(250), &json!(1250)]);
    let delays: Vec<&Value> = steps.iter().map(|s| &s["delayMs"]).collect();
    assert_eq!(delays, [&json!(0), &json!(0), &json!(250), &json!(500)]);
    // A step's first effect in a click group takes the click whatever its nodeType says.
    let lowered = native(&deck(&timing(&click_group(&slot(
        0,
        &effect(
            &format!("{ENTR_FADE} nodeType=\"withEffect\""),
            "0",
            &fade(3),
        ),
    )))));
    assert_eq!(first_slide(&lowered).timeline[0].trigger, Trigger::Click);
    // An unknown nodeType reads by place.
    let lowered = native(&deck(&timing(&click_group(&slot(
        0,
        &format!(
            "{}{}",
            effect(ENTR_FADE, "0", &fade(3)),
            effect(
                &format!("{ENTR_FADE} nodeType=\"sometimes\""),
                "0",
                &fade(4)
            )
        ),
    )))));
    assert_eq!(
        first_slide(&lowered).timeline[1].trigger,
        Trigger::WithPrevious
    );
    assert_eq!(reported(&lowered, "PX2412")[0]["option"], "nodeType");
}

#[test]
fn durations_come_from_the_behaviours() {
    let tgt = target(3);
    let staggered = format!(
        "{}<p:animEffect transition=\"in\" filter=\"fade\"><p:cBhvr><p:cTn id=\"0\" dur=\"400\"><p:stCondLst><p:cond delay=\"300\"/></p:stCondLst></p:cTn>{tgt}</p:cBhvr></p:animEffect>",
        show(&tgt)
    );
    let appear = show(&target(4));
    let invalid = format!("{}{}", show(&target(5)), fade_in(&target(5), "NaN"));
    let clamped = format!("{}{}", show(&target(6)), fade_in(&target(6), "4294967295"));
    let groups: String = [
        effect(ENTR_FADE, "0", &staggered),
        effect(
            "presetID=\"1\" presetClass=\"entr\" presetSubtype=\"0\"",
            "0",
            &appear,
        ),
        effect(ENTR_FADE, "0", &invalid),
        effect(ENTR_FADE, "0", &clamped),
    ]
    .iter()
    .map(|e| click_group(&slot(0, e)))
    .collect();
    let lowered = native(&deck(&timing(&groups)));
    let durations: Vec<u32> = first_slide(&lowered)
        .timeline
        .iter()
        .map(|s| s.duration_ms.0)
        .collect();
    assert_eq!(durations, [700, 0, 500, 3_600_000]);
    let hostile = reported(&lowered, "PX2411");
    assert_eq!(hostile.len(), 2);
    assert_eq!(
        (&hostile[0]["native"], &hostile[0]["read"]),
        (&json!("NaN"), &json!("invalid"))
    );
    assert_eq!(
        (&hostile[1]["native"], &hostile[1]["read"]),
        (&json!("4294967295"), &json!("clamped"))
    );
}

#[test]
fn targets_name_the_slides_own_shapes() {
    let paragraphs = "<p:tgtEl><p:spTgt spid=\"3\"><p:txEl><p:pRg st=\"1\" end=\"2\"/></p:txEl></p:spTgt></p:tgtEl>";
    let background = "<p:tgtEl><p:spTgt spid=\"4\"><p:bg/></p:spTgt></p:tgtEl>";
    let characters = "<p:tgtEl><p:spTgt spid=\"5\"><p:txEl><p:charRg st=\"0\" end=\"2\"/></p:txEl></p:spTgt></p:tgtEl>";
    let slide_target = "<p:tgtEl><p:sldTgt/></p:tgtEl>";
    let groups: String = [
        fade_in(paragraphs, "500"),
        fade_in(background, "500"),
        fade_in(characters, "500"),
        fade_in(&target(99), "500"),
        fade_in(slide_target, "500"),
    ]
    .iter()
    .map(|b| click_group(&slot(0, &effect(ENTR_FADE, "0", b))))
    .collect();
    let lowered = native(&deck(&timing(&groups)));
    let steps = steps(&lowered);
    assert_eq!(steps.len(), 3);
    assert_eq!(
        steps[0]["target"],
        json!({ "element": "pp-256.3", "paragraphs": [1, 2] })
    );
    assert_eq!(steps[0]["delivery"], "all");
    assert_eq!(
        steps[1]["target"],
        json!({ "element": "pp-256.4", "background": true })
    );
    assert_eq!(steps[2]["target"], json!({ "element": "pp-256.5" }));
    let mut reasons: Vec<String> = reported(&lowered, "PX2411")
        .iter()
        .map(|d| d["reason"].as_str().unwrap().to_owned())
        .collect();
    reasons.sort();
    assert_eq!(
        reasons,
        [
            "a character range plays on the whole shape",
            "an effect on a shape that is not on the slide is dropped",
            "an effect on a slide target is dropped",
        ]
    );
}

#[test]
fn delivery_comes_from_iterate_and_the_build_list() {
    let iterate = |ty: &str, interval: &str, back: &str| {
        format!(
            "<p:par><p:cTn id=\"9\" {ENTR_FADE} fill=\"hold\"><p:stCondLst><p:cond delay=\"0\"/></p:stCondLst><p:iterate type=\"{ty}\"{back}>{interval}</p:iterate><p:childTnLst>{}</p:childTnLst></p:cTn></p:par>",
            fade(3)
        )
    };
    let by_paragraph = effect(
        ENTR_FADE,
        "0",
        &fade_in(
            "<p:tgtEl><p:spTgt spid=\"4\"><p:txEl><p:pRg st=\"0\" end=\"0\"/></p:txEl></p:spTgt></p:tgtEl>",
            "500",
        ),
    );
    let groups: String = [
        iterate("wd", "<p:tmAbs val=\"50\"/>", ""),
        iterate("lt", "<p:tmPct val=\"10000\"/>", " backwards=\"1\""),
        iterate("el", "<p:tmPct val=\"10000\"/>", ""),
        by_paragraph,
    ]
    .iter()
    .map(|e| click_group(&slot(0, e)))
    .collect();
    let lowered = native(&deck(&timing_with(
        &groups,
        "",
        "<p:bldP spid=\"4\" grpId=\"0\" build=\"p\"/>",
    )));
    let steps = steps(&lowered);
    let delivery: Vec<&str> = steps
        .iter()
        .map(|s| s["delivery"].as_str().unwrap())
        .collect();
    assert_eq!(delivery, ["byWord", "byChar", "all", "byParagraph"]);
    assert_eq!(
        steps[0]["timing"]["stagger"],
        json!({ "perUnit": { "ms": 50 }, "order": "forward" })
    );
    assert_eq!(
        steps[1]["timing"]["stagger"],
        json!({ "perUnit": { "fraction": 0.1 }, "order": "backward" })
    );
    assert!(steps[2]["timing"].get("stagger").is_none());
    assert_eq!(
        first_slide(&lowered).timeline[3].delivery,
        Delivery::ByParagraph
    );
}

#[test]
fn repeats_auto_reverse_and_easing_read_bounded() {
    let with = |attrs: &str, end: &str| {
        format!(
            "<p:par><p:cTn id=\"9\" presetID=\"8\" presetClass=\"emph\" presetSubtype=\"0\" fill=\"hold\" {attrs}><p:stCondLst><p:cond delay=\"0\"/></p:stCondLst>{end}<p:childTnLst><p:animRot by=\"21600000\">{}</p:animRot></p:childTnLst></p:cTn></p:par>",
            cbhvr("500", &target(3), "r")
        )
    };
    let next = "<p:endCondLst><p:cond evt=\"onNext\" delay=\"0\"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:endCondLst>";
    let groups: String = [
        with("repeatCount=\"2500\"", ""),
        with("repeatCount=\"indefinite\"", next),
        with("repeatCount=\"indefinite\"", ""),
        with("repeatDur=\"1500\"", ""),
        with("repeatCount=\"4294967000\" autoRev=\"1\"", ""),
        with("repeatCount=\"0\"", ""),
        with("accel=\"20000\" decel=\"30000\"", ""),
        with("accel=\"60000\" decel=\"50000\"", ""),
    ]
    .iter()
    .map(|e| click_group(&slot(0, e)))
    .collect();
    let lowered = native(&deck(&timing(&groups)));
    let steps = steps(&lowered);
    let repeats: Vec<Value> = steps
        .iter()
        .map(|s| s["timing"].get("repeat").cloned().unwrap_or(Value::Null))
        .collect();
    assert_eq!(
        repeats,
        [
            json!({ "count": 2.5 }),
            json!("untilNextClick"),
            json!("untilEndOfSlide"),
            json!({ "count": 3.0 }),
            json!({ "count": 1000.0 }),
            Value::Null,
            Value::Null,
            Value::Null
        ]
    );
    assert_eq!(steps[4]["timing"]["autoReverse"], true);
    assert_eq!(
        steps[6]["timing"]["easing"],
        json!({ "accelDecel": { "accel": 0.2, "decel": 0.3 } })
    );
    assert!(steps[7]["timing"].get("easing").is_none());
    let hostile = reported(&lowered, "PX2411");
    assert_eq!(hostile.len(), 2);
    assert_eq!(hostile[0]["native"], "4294967");
    assert_eq!(hostile[1]["read"], "invalid");
    assert_eq!(reported(&lowered, "PX2412")[0]["option"], "accel");
}

#[test]
fn after_effects_hide_or_dim() {
    let after = |children: &str| {
        format!(
            "<p:par><p:cTn id=\"20\" fill=\"hold\" afterEffect=\"1\"><p:stCondLst><p:cond delay=\"500\"/></p:stCondLst><p:childTnLst>{children}</p:childTnLst></p:cTn></p:par>"
        )
    };
    let hide = format!(
        "<p:set>{}<p:to><p:strVal val=\"hidden\"/></p:to></p:set>",
        cbhvr("1", &target(3), "style.visibility")
    );
    let dim = format!(
        "<p:set>{}<p:to><p:clrVal><a:srgbClr val=\"808080\"/></p:clrVal></p:to></p:set>",
        cbhvr("1", &target(4), "style.color")
    );
    let scheme = format!(
        "<p:set>{}<p:to><p:clrVal><a:schemeClr val=\"accent1\"/></p:clrVal></p:to></p:set>",
        cbhvr("1", &target(5), "style.color")
    );
    let groups: String = [(3, hide), (4, dim), (5, scheme)]
        .iter()
        .map(|(spid, after_children)| {
            click_group(&slot(
                0,
                &format!(
                    "{}{}",
                    effect(ENTR_FADE, "0", &fade(*spid)),
                    after(after_children)
                ),
            ))
        })
        .collect();
    let lowered = native(&deck(&timing(&groups)));
    let steps = steps(&lowered);
    assert_eq!(steps.len(), 3);
    assert_eq!(steps[0]["timing"]["after"], "hide");
    assert_eq!(steps[1]["timing"]["after"], json!({ "dim": "#808080" }));
    assert!(steps[2]["timing"].get("after").is_none());
    assert_eq!(reported(&lowered, "PX2412")[0]["option"], "after");
}

#[test]
fn interactive_sequences_and_conditions_on_nodes() {
    let interactive = format!(
        "<p:seq concurrent=\"1\" nextAc=\"seek\"><p:cTn id=\"30\" fill=\"hold\" nodeType=\"interactiveSeq\"><p:stCondLst><p:cond evt=\"onClick\" delay=\"0\"><p:tgtEl><p:spTgt spid=\"4\"/></p:tgtEl></p:cond></p:stCondLst><p:childTnLst>{}</p:childTnLst></p:cTn></p:seq>",
        click_group(&slot(0, &effect(ENTR_FADE, "0", &fade(5))))
    );
    let on = |id: u32, evt: &str, node: u32, spid: u32| {
        format!(
            "<p:par><p:cTn id=\"{id}\" {ENTR_FADE} fill=\"hold\"><p:stCondLst><p:cond evt=\"{evt}\" delay=\"100\"><p:tn val=\"{node}\"/></p:cond></p:stCondLst><p:childTnLst>{}</p:childTnLst></p:cTn></p:par>",
            fade(spid)
        )
    };
    let groups = click_group(&slot(
        0,
        &format!(
            "{}{}{}{}",
            on(40, "onEnd", 40, 3),
            on(41, "onBegin", 42, 4),
            on(42, "onEnd", 41, 5),
            on(43, "onEnd", 99, 6),
        ),
    ));
    let lowered = native(&deck(&timing_with(&groups, &interactive, "")));
    let steps = steps(&lowered);
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0]["target"]["element"], "pp-256.6");
    assert_eq!(steps[0]["trigger"], "click");
    assert!(steps[0]["timing"].get("offsetMs").is_none());
    assert_eq!(steps[0]["delayMs"], 100);
    let whats: Vec<String> = reported(&lowered, "PX2411")
        .iter()
        .map(|d| {
            format!(
                "{} {}",
                d["what"].as_str().unwrap(),
                d["spid"].as_str().unwrap_or("")
            )
        })
        .collect();
    assert_eq!(whats, ["start 3", "start 4", "start 5"]);
    // The interactive sequence's effect is left out with its own code.
    let interactive = reported(&lowered, "PX2403");
    assert_eq!(interactive.len(), 1);
    assert_eq!(interactive[0]["effect"], "pptx:entr.10");
    assert_eq!(interactive[0]["spid"], "5");
}

#[test]
fn hostile_trees_stay_in_bounds() {
    // Nested past the depth limit: the effect inside is dropped and named.
    let mut nested = effect(ENTR_FADE, "0", &fade(3));
    for _ in 0..TIMING_DEPTH + 8 {
        nested = format!(
            "<p:par><p:cTn id=\"5\" fill=\"hold\"><p:childTnLst>{nested}</p:childTnLst></p:cTn></p:par>"
        );
    }
    let lowered = native(&deck(&timing(&click_group(&slot(0, &nested)))));
    assert!(first_slide(&lowered).timeline.is_empty());
    let dropped = reported(&lowered, "PX2411");
    assert_eq!(dropped.len(), 1);
    assert_eq!(
        (
            &dropped[0]["what"],
            &dropped[0]["effect"],
            &dropped[0]["spid"]
        ),
        (&json!("nested"), &json!("pptx:entr.10"), &json!("3"))
    );
    // More effects than a slide's steps: the rest are dropped and reported.
    let many: String = (0..STEPS_PER_SLIDE + 3)
        .map(|_| effect(ENTR_FADE, "0", &fade_in(&target(3), "1")))
        .collect();
    let lowered = native(&deck(&timing(&click_group(&slot(0, &many)))));
    assert_eq!(first_slide(&lowered).timeline.len(), STEPS_PER_SLIDE);
    // The three are alike, so they are reported once.
    assert_eq!(reported(&lowered, "PX2411").len(), 1);
    // More time nodes than the budget: reading stops, once reported.
    let wide: String = (0..TIMING_NODES)
        .map(|_| "<p:par><p:cTn id=\"6\"/></p:par>")
        .collect();
    let lowered = native(&deck(&timing(&format!(
        "{}{}",
        click_group(&wide),
        click_group(&slot(0, &effect(ENTR_FADE, "0", &fade(3))))
    ))));
    assert!(first_slide(&lowered).timeline.is_empty());
    assert_eq!(reported(&lowered, "PX2411").len(), 1);
}

#[test]
fn hostile_trees_never_panic() {
    // A deterministic stream of malformed trees: wrong nesting, junk values, missing pieces.
    let pieces = [
        "<p:par/>",
        "<p:seq><p:cTn/></p:seq>",
        "<p:par><p:cTn presetClass=\"entr\" presetID=\"x\" presetSubtype=\"-1\"/></p:par>",
        "<p:par><p:cTn presetClass=\"bogus\"><p:childTnLst><p:anim><p:cBhvr><p:cTn dur=\"5\"/><p:tgtEl><p:spTgt spid=\"3\"/></p:tgtEl><p:attrNameLst><p:attrName>ppt_x</p:attrName></p:attrNameLst></p:cBhvr><p:tavLst><p:tav tm=\"50000\" fmla=\"((((\"><p:val><p:strVal val=\"#ppt_q\"/></p:val></p:tav></p:tavLst></p:anim></p:childTnLst></p:cTn></p:par>",
        "<p:par><p:cTn presetClass=\"path\" presetID=\"0\"><p:childTnLst><p:animMotion path=\"L 1 1 Q\"><p:cBhvr><p:cTn dur=\"indefinite\"/><p:tgtEl><p:spTgt spid=\"4\"/></p:tgtEl></p:cBhvr></p:animMotion></p:childTnLst></p:cTn></p:par>",
        "<p:excl><p:cTn/></p:excl>",
        "<p:par><p:cTn id=\"1\"><p:stCondLst><p:cond evt=\"onEnd\"><p:tn val=\"1\"/></p:cond></p:stCondLst></p:cTn></p:par>",
        "<p:par><p:cTn presetClass=\"emph\" presetID=\"8\" repeatCount=\"NaN\" accel=\"-1\"><p:childTnLst><p:animRot by=\"1e400\"><p:cBhvr><p:cTn dur=\"1\"/><p:tgtEl><p:spTgt spid=\"5\"><p:txEl><p:pRg st=\"9\" end=\"-1\"/></p:txEl></p:spTgt></p:tgtEl></p:cBhvr></p:animRot></p:childTnLst></p:cTn></p:par>",
    ];
    let mut state: u64 = 0x2545_f491;
    for round in 0..64 {
        let mut groups = String::new();
        for _ in 0..6 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let piece = pieces[(state % pieces.len() as u64) as usize];
            let wrapped = match state % 4 {
                0 => piece.to_owned(),
                1 => click_group(piece),
                2 => click_group(&slot((state % 5000) as u32, piece)),
                _ => auto_group(&slot(0, &format!("{piece}{piece}"))),
            };
            groups.push_str(&wrapped);
        }
        let lowered = native(&deck(&timing_with(
            &groups,
            &groups,
            "<p:bldP spid=\"x\" grpId=\"\"/>",
        )));
        for step in &first_slide(&lowered).timeline {
            assert!(
                slidock_ir::validate::validate(step).is_empty(),
                "round {round}: {step:?}"
            );
        }
    }
}

#[test]
fn presets_refine_their_values_from_the_behaviours() {
    let rot = format!(
        "<p:animRot by=\"-10800000\">{}</p:animRot>",
        cbhvr("500", &target(3), "r")
    );
    let scale = format!(
        "<p:animScale>{}<p:by x=\"200000\" y=\"50000\"/></p:animScale>",
        cbhvr("500", &target(4), "")
    );
    let color = format!(
        "<p:animClr clrSpc=\"rgb\" dir=\"cw\">{}<p:to><a:srgbClr val=\"FF0000\"/></p:to></p:animClr>",
        cbhvr("500", &target(5), "style.color")
    );
    let scheme = format!(
        "<p:animClr clrSpc=\"rgb\" dir=\"cw\">{}<p:to><a:schemeClr val=\"accent2\"/></p:to></p:animClr>",
        cbhvr("500", &target(6), "style.color")
    );
    let path = format!(
        "<p:animMotion origin=\"layout\" path=\"M 0 0 L 0.5 0.25 E\" pathEditMode=\"relative\">{}</p:animMotion>",
        cbhvr("500", &target(3), "ppt_x")
    );
    let groups: String = [
        effect(
            "presetID=\"8\" presetClass=\"emph\" presetSubtype=\"0\"",
            "0",
            &rot,
        ),
        effect(
            "presetID=\"6\" presetClass=\"emph\" presetSubtype=\"0\"",
            "0",
            &scale,
        ),
        effect(
            "presetID=\"3\" presetClass=\"emph\" presetSubtype=\"2\"",
            "0",
            &color,
        ),
        effect(
            "presetID=\"3\" presetClass=\"emph\" presetSubtype=\"2\"",
            "0",
            &scheme,
        ),
        effect(
            "presetID=\"0\" presetClass=\"path\" presetSubtype=\"0\"",
            "0",
            &path,
        ),
    ]
    .iter()
    .map(|e| click_group(&slot(0, e)))
    .collect();
    let lowered = native(&deck(&timing(&groups)));
    let steps = steps(&lowered);
    let native_of = |i: usize| {
        steps[i]["effect"]
            .get("native")
            .cloned()
            .unwrap_or(Value::Null)
    };
    assert_eq!(
        native_of(0)["params"],
        json!({ "deg": 180.0, "cw": false, "hold": true })
    );
    assert_eq!(
        native_of(1)["params"]["scale"],
        json!({ "x": 2.0, "y": 0.5 })
    );
    assert_eq!(
        native_of(2)["params"],
        json!({ "property": "text", "to": "#ff0000" })
    );
    assert_eq!(native_of(3), Value::Null);
    assert_eq!(
        first_slide(&lowered).timeline[3].effect.fallback,
        StepFallback::None
    );
    // The 720 x 405 pt slide is 960 x 540 px.
    assert_eq!(
        native_of(4)["params"]["d"],
        json!([["M", 0.0, 0.0], ["L", 480.0, 135.0]])
    );
    assert_eq!(reported(&lowered, "PX2410").len(), 1);
}

#[test]
fn unknown_presets_play_their_behaviours() {
    let tgt = target(3);
    let behaviours = format!(
        "{}<p:anim calcmode=\"lin\" valueType=\"num\">{}<p:tavLst><p:tav tm=\"0\"><p:val><p:strVal val=\"#ppt_y-0.1\"/></p:val></p:tav><p:tav tm=\"100000\"><p:val><p:strVal val=\"#ppt_y\"/></p:val></p:tav></p:tavLst></p:anim><p:animEffect transition=\"in\" filter=\"wipe(down)\">{}</p:animEffect>",
        show(&tgt),
        cbhvr("500", &tgt, "ppt_y"),
        cbhvr("500", &tgt, "")
    );
    let unknown_only = format!(
        "{}<p:set>{}<p:to><p:strVal val=\"0.3\"/></p:to></p:set>",
        show(&target(4)),
        cbhvr("500", &target(4), "xshear")
    );
    let custom = format!(
        "<p:animScale>{}<p:by x=\"150000\" y=\"150000\"/></p:animScale>",
        cbhvr("500", &target(5), "")
    );
    let groups: String = [
        effect(
            "presetID=\"99\" presetClass=\"entr\" presetSubtype=\"0\"",
            "0",
            &behaviours,
        ),
        effect(
            "presetID=\"98\" presetClass=\"entr\" presetSubtype=\"0\"",
            "0",
            &unknown_only,
        ),
        effect("", "0", &custom),
    ]
    .iter()
    .map(|e| click_group(&slot(0, e)))
    .collect();
    let lowered = native(&deck(&timing(&groups)));
    let steps = steps(&lowered);
    let tracks = &steps[0]["effect"]["native"]["params"];
    assert_eq!(steps[0]["effect"]["native"]["effect"], "behaviors");
    let properties: Vec<&str> = tracks
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["property"].as_str().unwrap())
        .collect();
    assert_eq!(properties, ["clipPath", "translate"]);
    assert_eq!(
        tracks[0]["reveal"],
        json!({ "effect": "wipe", "params": { "dir": "u", "soft": true } })
    );
    // 0.1 of the 405 pt slide's height is 54 px.
    assert_eq!(
        tracks[1]["frames"][0]["value"],
        json!({ "x": 0.0, "y": -54.0 })
    );
    assert_eq!(steps[1]["effect"].get("native"), None);
    assert_eq!(steps[1]["effect"]["fallback"], "dissolve");
    assert_eq!(reported(&lowered, "PX2410")[0]["effect"], "pptx:entr.98");
    assert_eq!(steps[2]["kind"], "emphasis");
    assert_eq!(steps[2]["effect"]["id"], "pptx:custom.0");
    assert_eq!(steps[2]["effect"]["native"]["effect"], "behaviors");
}

/// Colour and font emphases (ECMA-376 §19.5.1 `animClr`, §19.5.66 `set`, §19.5.10 attribute names): colours to a
/// colour or by an HSL amount, sets of colours and font switches, and a discrete bold flash compile to tracks; a font
/// family or size change plays the fallback, reported; a step built by unit keeps only its text's tracks.
#[test]
fn colour_and_font_behaviours_compile_to_tracks() {
    let clr = |spid: u32, attr: &str, inner: &str| {
        format!(
            "<p:animClr clrSpc=\"hsl\" dir=\"cw\">{}{inner}</p:animClr>",
            cbhvr("500", &target(spid), attr)
        )
    };
    let set = |spid: u32, attr: &str, to: &str| {
        format!(
            "<p:set>{}<p:to>{to}</p:to></p:set>",
            cbhvr("500", &target(spid), attr)
        )
    };
    let emph =
        |id: u32| format!("presetID=\"{id}\" presetClass=\"emph\" presetSubtype=\"0\" grpId=\"0\"");
    let blend = format!(
        "{}{}",
        clr(3, "style.color", "<p:to><a:srgbClr val=\"333399\"/></p:to>"),
        clr(3, "fillcolor", "<p:to><a:srgbClr val=\"333399\"/></p:to>")
    );
    let darken = clr(
        4,
        "stroke.color",
        "<p:by><p:hsl h=\"0\" s=\"-12000\" l=\"-25000\"/></p:by>",
    );
    let complementary = clr(
        4,
        "fillcolor",
        "<p:by><p:hsl h=\"7200000\" s=\"0\" l=\"0\"/></p:by>",
    );
    let style = [
        set(
            5,
            "style.color",
            "<p:clrVal><a:srgbClr val=\"333399\"/></p:clrVal>",
        ),
        set(5, "style.fontStyle", "<p:strVal val=\"italic\"/>"),
        set(5, "style.fontWeight", "<p:strVal val=\"bold\"/>"),
        set(
            5,
            "style.textDecorationUnderline",
            "<p:strVal val=\"true\"/>",
        ),
    ]
    .concat();
    let flash = format!(
        "<p:anim calcmode=\"discrete\" valueType=\"str\">{}<p:tavLst><p:tav tm=\"0\"><p:val><p:strVal val=\"normal\"/></p:val></p:tav><p:tav tm=\"50000\"><p:val><p:strVal val=\"bold\"/></p:val></p:tav><p:tav tm=\"60000\"><p:val><p:strVal val=\"normal\"/></p:val></p:tav></p:tavLst></p:anim>",
        cbhvr("2000", &target(6), "style.fontWeight")
    );
    let font = set(6, "style.fontFamily", "<p:strVal val=\"Times New Roman\"/>");
    let grow = format!(
        "{}<p:anim by=\"1.5\" calcmode=\"lin\" valueType=\"num\">{}</p:anim>",
        clr(6, "style.color", "<p:to><a:srgbClr val=\"333399\"/></p:to>"),
        cbhvr("500", &target(6), "style.fontSize")
    );
    let rgb = clr(
        6,
        "fillcolor",
        "<p:by><p:rgb r=\"10000\" g=\"0\" b=\"0\"/></p:by>",
    );
    let by_letter = format!(
        "<p:iterate type=\"lt\"><p:tmPct val=\"10000\"/></p:iterate><p:childTnLst>{}{}</p:childTnLst>",
        set(
            3,
            "style.color",
            "<p:clrVal><a:srgbClr val=\"333399\"/></p:clrVal>"
        ),
        set(
            3,
            "fillcolor",
            "<p:clrVal><a:srgbClr val=\"333399\"/></p:clrVal>"
        )
    );
    let iterated = format!(
        "<p:par><p:cTn id=\"9\" fill=\"hold\" {}><p:stCondLst><p:cond delay=\"0\"/></p:stCondLst>{by_letter}</p:cTn></p:par>",
        emph(20)
    );
    let groups: String = [
        effect(&emph(19), "0", &blend),
        effect(&emph(24), "0", &darken),
        effect(&emph(21), "0", &complementary),
        effect(&emph(31), "0", &style),
        effect(&emph(10), "0", &flash),
        effect(&emph(2), "0", &font),
        effect(&emph(25), "0", &rgb),
        iterated,
        effect(&emph(28), "0", &grow),
    ]
    .iter()
    .map(|e| click_group(&slot(0, e)))
    .collect();
    let lowered = native(&deck(&timing(&groups)));
    let steps = steps(&lowered);
    let params = |i: usize| steps[i]["effect"]["native"]["params"].clone();
    let own = json!({ "h": 0.0, "s": 0.0, "l": 0.0 });
    assert_eq!(params(0)[0]["property"], "text");
    assert_eq!(params(0)[1]["property"], "fill");
    assert_eq!(
        params(0)[1]["frames"],
        json!([{ "offset": 0.0, "value": own }, { "offset": 1.0, "value": "#333399" }])
    );
    assert_eq!(params(1)[0]["property"], "line");
    assert_eq!(
        params(1)[0]["frames"][1]["value"],
        json!({ "h": 0.0, "s": -0.12, "l": -0.25 })
    );
    let turn = params(2)[0]["frames"].as_array().unwrap().clone();
    assert_eq!(turn.len(), 9);
    assert_eq!(turn[8]["value"], json!({ "h": 120.0, "s": 0.0, "l": 0.0 }));
    // A `set` is a switch at its begin, of the property it sets.
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
        params(3)[0],
        json!({ "property": "set", "beginMs": 0, "durationMs": 500, "to": { "property": "text", "value": "#333399" } })
    );
    // Bold Flash is a tier-1 entry; its behaviours, read as a custom effect's, hold each value to the next.
    assert_eq!(steps[4]["effect"]["native"]["effect"], "boldFlash");
    for i in [5, 6] {
        assert_eq!(steps[i]["effect"].get("native"), None, "{}", steps[i]);
    }
    let unmapped: Vec<Value> = reported(&lowered, "PX2410")
        .into_iter()
        .map(|data| data["effect"].clone())
        .collect();
    assert_eq!(
        unmapped,
        [
            json!("pptx:emph.2"),
            json!("pptx:emph.25"),
            json!("pptx:emph.28")
        ]
    );
    assert_eq!(steps[8]["effect"].get("native"), None, "{}", steps[8]);
    let letters = params(7);
    assert_eq!(steps[7]["delivery"], "byChar");
    assert_eq!(letters.as_array().unwrap().len(), 1);
    assert_eq!(letters[0]["to"]["property"], "text");
}

/// A behaviour that reverses on its own (`autoRev` on its `p:cTn`, as LibreOffice exports Color Over by Letter's sets)
/// plays its reversal: a switch keeps its value over twice its span, and the step lasts that long. Nothing is
/// reported. The same set without it lasts its own span.
#[test]
fn a_behaviours_own_reversal_plays() {
    let set = |reverse: &str| {
        format!(
            "<p:set><p:cBhvr><p:cTn id=\"0\" dur=\"500\"{reverse} fill=\"hold\"/>{}<p:attrNameLst><p:attrName>style.color</p:attrName></p:attrNameLst></p:cBhvr><p:to><p:clrVal><a:srgbClr val=\"333399\"/></p:clrVal></p:to></p:set>",
            target(3)
        )
    };
    let custom = "presetID=\"0\" presetClass=\"emph\" presetSubtype=\"0\" grpId=\"0\"";
    let groups: String = [
        effect(custom, "0", &set(" autoRev=\"1\"")),
        effect(custom, "0", &set("")),
    ]
    .iter()
    .map(|e| click_group(&slot(0, e)))
    .collect();
    let lowered = native(&deck(&timing(&groups)));
    let steps = steps(&lowered);
    for (step, span) in steps.iter().zip([1000, 500]) {
        assert_eq!(
            step["effect"]["native"]["params"],
            json!([{ "property": "set", "beginMs": 0, "durationMs": span, "to": { "property": "text", "value": "#333399" } }])
        );
        assert_eq!(step["durationMs"], span);
    }
    assert!(reported(&lowered, "PX2410").is_empty());
}

/// A pulse that reverses on its own and a fade under a time filter (`tmFilter`, the flash bulb's out to half and
/// back) fold into their tracks: the effect ends at rest. A time filter that does not read plays without it,
/// reported as timing that reads as its default.
#[test]
fn a_reversal_and_a_time_filter_end_at_rest() {
    let tgt = target(3);
    let pulse = format!(
        "<p:animScale><p:cBhvr><p:cTn id=\"0\" dur=\"250\" autoRev=\"1\" fill=\"hold\"/>{tgt}</p:cBhvr><p:by x=\"105000\" y=\"105000\"/></p:animScale>"
    );
    let fade = |filter: &str| {
        format!(
            "<p:animEffect transition=\"out\" filter=\"fade\"><p:cBhvr><p:cTn id=\"0\" dur=\"500\" tmFilter=\"{filter}\"/>{tgt}</p:cBhvr></p:animEffect>"
        )
    };
    let bulb = "presetID=\"26\" presetClass=\"emph\" presetSubtype=\"0\" grpId=\"0\"";
    let groups: String = [
        effect(
            bulb,
            "0",
            &format!("{pulse}{}", fade("0,0; .2, .5; .8,.5; 1,0")),
        ),
        effect(bulb, "0", &format!("{pulse}{}", fade("0,0; 2,1"))),
    ]
    .iter()
    .map(|e| click_group(&slot(0, e)))
    .collect();
    let lowered = native(&deck(&timing(&groups)));
    let steps = steps(&lowered);
    let tracks = steps[0]["effect"]["native"]["params"].as_array().unwrap();
    let scale = tracks.iter().find(|t| t["property"] == "scale").unwrap();
    assert_eq!(scale["durationMs"], 500);
    assert_eq!(
        scale["frames"],
        json!([
            { "offset": 0.0, "value": { "x": 1.0, "y": 1.0 } },
            { "offset": 0.5, "value": { "x": 1.05, "y": 1.05 } },
            { "offset": 1.0, "value": { "x": 1.0, "y": 1.0 } },
        ])
    );
    let opacity = tracks.iter().find(|t| t["property"] == "opacity").unwrap();
    assert_eq!(
        opacity["frames"],
        json!([
            { "offset": 0.0, "value": 1.0 }, { "offset": 0.2, "value": 0.5 },
            { "offset": 0.8, "value": 0.5 }, { "offset": 1.0, "value": 1.0 },
        ])
    );
    assert_eq!(steps[0]["durationMs"], 500);
    let unread = steps[1]["effect"]["native"]["params"].as_array().unwrap();
    let plain = unread.iter().find(|t| t["property"] == "opacity").unwrap();
    assert_eq!(plain["frames"][1]["value"], 0.0);
    let hostile: Vec<Value> = reported(&lowered, "PX2411")
        .into_iter()
        .map(|data| data["what"].clone())
        .collect();
    assert_eq!(hostile, [json!("timeFilter")]);
    assert!(reported(&lowered, "PX2410").is_empty());
}

/// A behaviour tier 2 cannot compile is named (PX2410, the behaviours that play), never dropped silently; the
/// switches that show a fill or line a colour paints are the deck builder's, so they are not.
#[test]
fn a_behaviour_tier_2_cannot_compile_is_reported() {
    let tgt = target(3);
    let spin = format!(
        "<p:animRot by=\"21600000\">{}</p:animRot>",
        cbhvr("500", &tgt, "r")
    );
    let shear = format!(
        "<p:set>{}<p:to><p:strVal val=\"0.3\"/></p:to></p:set>",
        cbhvr("500", &tgt, "xshear")
    );
    let paint_on = format!(
        "<p:set>{}<p:to><p:strVal val=\"solid\"/></p:to></p:set>",
        cbhvr("500", &tgt, "fill.type")
    );
    let custom = "presetID=\"0\" presetClass=\"emph\" presetSubtype=\"0\" grpId=\"0\"";
    let groups: String = [
        effect(custom, "0", &format!("{spin}{shear}")),
        effect(custom, "0", &format!("{spin}{paint_on}")),
    ]
    .iter()
    .map(|e| click_group(&slot(0, e)))
    .collect();
    let lowered = native(&deck(&timing(&groups)));
    let steps = steps(&lowered);
    for step in &steps {
        let tracks = step["effect"]["native"]["params"].as_array().unwrap();
        assert_eq!(tracks.len(), 1, "{step}");
        assert_eq!(tracks[0]["property"], "rotate");
    }
    let plays: Vec<Value> = reported(&lowered, "PX2410")
        .into_iter()
        .map(|data| data["play"].clone())
        .collect();
    assert_eq!(
        plays,
        [json!("its behaviours without its xshear animation")]
    );
}

/// A switch that does not hold (`fill` remove, or none, with a duration) gives the element its own value back when it
/// ends: a colour its own (no shift), an opacity full. A font switch has no own value in the IR: it holds, and the end
/// is named. A switch with no end (an indefinite duration) holds.
#[test]
fn a_switch_that_does_not_hold_switches_back() {
    let tgt = target(3);
    let set = |attr: &str, to: &str| {
        format!(
            "<p:set><p:cBhvr><p:cTn id=\"0\" dur=\"400\"><p:stCondLst><p:cond delay=\"100\"/></p:stCondLst></p:cTn>{tgt}<p:attrNameLst><p:attrName>{attr}</p:attrName></p:attrNameLst></p:cBhvr><p:to>{to}</p:to></p:set>"
        )
    };
    let custom = "presetID=\"0\" presetClass=\"emph\" presetSubtype=\"0\" grpId=\"0\"";
    let behaviours = [
        set(
            "style.color",
            "<p:clrVal><a:srgbClr val=\"333399\"/></p:clrVal>",
        ),
        set("style.opacity", "<p:strVal val=\"0.5\"/>"),
        set("style.fontWeight", "<p:strVal val=\"bold\"/>"),
        format!(
            "<p:set><p:cBhvr><p:cTn id=\"0\" dur=\"indefinite\"/>{tgt}<p:attrNameLst><p:attrName>style.fontStyle</p:attrName></p:attrNameLst></p:cBhvr><p:to><p:strVal val=\"italic\"/></p:to></p:set>"
        ),
    ]
    .concat();
    let groups = click_group(&slot(0, &effect(custom, "0", &behaviours)));
    let lowered = native(&deck(&timing(&groups)));
    let steps = steps(&lowered);
    let own = json!({ "h": 0.0, "s": 0.0, "l": 0.0 });
    assert_eq!(
        steps[0]["effect"]["native"]["params"],
        json!([
            { "property": "set", "beginMs": 100, "durationMs": 400, "to": { "property": "text", "value": "#333399" } },
            { "property": "set", "beginMs": 500, "durationMs": 0, "to": { "property": "text", "value": own } },
            { "property": "set", "beginMs": 100, "durationMs": 400, "to": { "property": "opacity", "value": 0.5 } },
            { "property": "set", "beginMs": 500, "durationMs": 0, "to": { "property": "opacity", "value": 1.0 } },
            { "property": "set", "beginMs": 100, "durationMs": 400, "to": { "property": "bold", "value": true } },
            { "property": "set", "beginMs": 0, "durationMs": 500, "to": { "property": "italic", "value": true } },
        ])
    );
    let plays: Vec<Value> = reported(&lowered, "PX2410")
        .into_iter()
        .map(|data| data["play"].clone())
        .collect();
    assert_eq!(
        plays,
        [json!(
            "its behaviours without the end of its style.fontWeight switch"
        )]
    );
}

/// Flash Once (`entr.11`) is an entrance whose visibility switch does not hold: it plays as an appear that lasts the
/// switch and hides its target after. The same switch held is a plain appear.
#[test]
fn flash_once_shows_its_target_while_its_switch_lasts() {
    let tgt = target(3);
    let shown = |fill: &str| {
        format!(
            "<p:set><p:cBhvr><p:cTn id=\"0\" dur=\"500\"{fill}><p:stCondLst><p:cond delay=\"0\"/></p:stCondLst></p:cTn>{tgt}<p:attrNameLst><p:attrName>style.visibility</p:attrName></p:attrNameLst></p:cBhvr><p:to><p:strVal val=\"visible\"/></p:to></p:set>"
        )
    };
    let flash = "presetID=\"11\" presetClass=\"entr\" presetSubtype=\"0\" grpId=\"0\"";
    let groups: String = [
        effect(flash, "0", &shown("")),
        effect(flash, "0", &shown(" fill=\"hold\"")),
    ]
    .iter()
    .map(|e| click_group(&slot(0, e)))
    .collect();
    let lowered = native(&deck(&timing(&groups)));
    let steps = steps(&lowered);
    assert_eq!(steps[0]["effect"]["native"]["effect"], "appear");
    assert_eq!(steps[0]["durationMs"], 500);
    assert_eq!(steps[0]["timing"]["after"], "hide");
    assert_eq!(steps[1]["effect"]["native"]["effect"], "appear");
    assert_eq!(steps[1]["durationMs"], 0);
    assert!(
        steps[1]
            .get("timing")
            .is_none_or(|t| t.get("after").is_none()),
        "{}",
        steps[1]
    );
    assert!(reported(&lowered, "PX2410").is_empty());
}

#[test]
fn clip_presets_follow_their_filter_and_drifts_their_motion() {
    let strips = format!(
        "{}<p:animEffect transition=\"in\" filter=\"strips(downRight)\">{}</p:animEffect>",
        show(&target(3)),
        cbhvr("500", &target(3), "")
    );
    let drift = |spid: u32, attr: &str, from: &str| {
        let tgt = target(spid);
        let to = format!("#{attr}");
        format!(
            "{}<p:anim calcmode=\"lin\" valueType=\"num\">{}<p:tavLst><p:tav tm=\"0\"><p:val><p:strVal val=\"{from}\"/></p:val></p:tav><p:tav tm=\"100000\"><p:val><p:strVal val=\"{to}\"/></p:val></p:tav></p:tavLst></p:anim>{}",
            show(&tgt),
            cbhvr("500", &tgt, attr),
            fade_in(&tgt, "500")
        )
    };
    let groups: String = [
        // LibreOffice names subtype 12 left-to-bottom; the filter travels toward the bottom right.
        effect(
            "presetID=\"18\" presetClass=\"entr\" presetSubtype=\"12\"",
            "0",
            &strips,
        ),
        // Ease In is a row the presets table marks approximate: its own behaviours play.
        effect(
            "presetID=\"29\" presetClass=\"entr\" presetSubtype=\"0\"",
            "0",
            &drift(4, "ppt_x", "#ppt_x-0.1"),
        ),
        // Descend floats the way its behaviours move it.
        effect(
            "presetID=\"47\" presetClass=\"entr\" presetSubtype=\"0\"",
            "0",
            &drift(5, "ppt_y", "#ppt_y+0.1"),
        ),
        effect(
            "presetID=\"47\" presetClass=\"entr\" presetSubtype=\"0\"",
            "0",
            &drift(6, "ppt_y", "#ppt_y-0.1"),
        ),
    ]
    .iter()
    .map(|e| click_group(&slot(0, e)))
    .collect();
    let lowered = native(&deck(&timing(&groups)));
    let steps = steps(&lowered);
    assert_eq!(
        steps[0]["effect"]["native"]["params"],
        json!({ "corner": "rd" })
    );
    assert_eq!(steps[1]["effect"]["native"]["effect"], "behaviors");
    let tracks = steps[1]["effect"]["native"]["params"].as_array().unwrap();
    assert!(tracks.iter().any(|track| track["property"] == "translate"));
    assert_eq!(
        steps[2]["effect"]["native"],
        json!({ "id": "pptx:entr.47", "effect": "floatIn", "params": { "dir": "u" } })
    );
    assert_eq!(
        steps[3]["effect"]["native"]["params"],
        json!({ "dir": "d" })
    );
    assert!(reported(&lowered, "PX2413").is_empty());
}

/// A clip preset plays the filter and transition it states: a filter of the preset's own clip sets its parameters,
/// and one its entry cannot play (another clip, or run the other way than the step) plays as its behaviours, with
/// the reason reported (PX2413).
#[test]
fn a_clip_preset_plays_the_filter_and_transition_it_states() {
    let exit_box = |spid: u32, filter: &str, transition: &str| {
        let tgt = target(spid);
        format!(
            "<p:animEffect transition=\"{transition}\" filter=\"{filter}\">{}</p:animEffect><p:set>{}<p:to><p:strVal val=\"hidden\"/></p:to></p:set>",
            cbhvr("500", &tgt, ""),
            cbhvr("1", &tgt, "style.visibility")
        )
    };
    let groups: String = [
        // Box Out whose filter closes inward: the filter decides.
        (3, 32, "box(in)", "out"),
        // Box In whose filter is a split.
        (4, 16, "barn(inHorizontal)", "out"),
        // Box In whose filter runs in, as an entrance's does.
        (5, 16, "box(in)", "in"),
        (6, 16, "box(in)", "sideways"),
    ]
    .iter()
    .map(|(spid, subtype, filter, transition)| {
        click_group(&slot(
            0,
            &effect(
                &format!("presetID=\"4\" presetClass=\"exit\" presetSubtype=\"{subtype}\""),
                "0",
                &exit_box(*spid, filter, transition),
            ),
        ))
    })
    .collect();
    let lowered = native(&deck(&timing(&groups)));
    let steps = steps(&lowered);
    assert_eq!(
        steps[0]["effect"]["native"],
        json!({ "id": "pptx:exit.4/32", "effect": "iris", "params": { "shape": "box", "inOut": "in" } })
    );
    let clip = |step: &Value| {
        let native = &step["effect"]["native"];
        assert_eq!(native["effect"], "behaviors", "{native}");
        native["params"]
            .as_array()
            .unwrap()
            .iter()
            .find(|track| track["property"] == "clipPath")
            .cloned()
    };
    let split = clip(&steps[1]).unwrap();
    assert_eq!(
        split["reveal"],
        json!({ "effect": "split", "params": { "orient": "horz", "inOut": "in" } })
    );
    assert_eq!(split["transition"], "out");
    let forward = clip(&steps[2]).unwrap();
    assert_eq!(
        forward["reveal"],
        json!({ "effect": "iris", "params": { "shape": "box", "inOut": "in" } })
    );
    assert_eq!(forward["transition"], "in");
    // An unknown transition plays no clip, and the behaviours hold nothing else: the fallback plays.
    assert!(steps[3]["effect"].get("native").is_none());
    assert_eq!(steps[3]["effect"]["fallback"], "dissolve");
    assert_eq!(reported(&lowered, "PX2410").len(), 1);
    let notes = reported(&lowered, "PX2413");
    let effects: Vec<&Value> = notes.iter().map(|note| &note["effect"]).collect();
    assert_eq!(effects, ["pptx:exit.4/16"; 3], "{notes:?}");
    assert!(
        notes[0]["reason"]
            .as_str()
            .unwrap()
            .contains("barn(inHorizontal) is not the iris"),
        "{notes:?}"
    );
    assert!(
        notes[1]["reason"].as_str().unwrap().contains("runs in"),
        "{notes:?}"
    );
    assert!(
        notes[2]["reason"]
            .as_str()
            .unwrap()
            .contains("runs sideways"),
        "{notes:?}"
    );
}

/// A preset the table marks approximate plays its behaviours; when they compile to nothing, the approximation
/// plays and is reported once (PX2413).
#[test]
fn an_approximate_preset_plays_its_behaviours_or_reports_the_approximation() {
    let opaque = {
        let tgt = target(3);
        format!(
            "{}<p:set>{}<p:to><p:strVal val=\"0.3\"/></p:to></p:set>",
            show(&tgt),
            cbhvr("500", &tgt, "xshear")
        )
    };
    let groups: String = [effect(
        "presetID=\"29\" presetClass=\"entr\" presetSubtype=\"0\"",
        "0",
        &opaque,
    )]
    .iter()
    .map(|e| click_group(&slot(0, e)))
    .collect();
    let lowered = native(&deck(&timing(&groups)));
    let steps = steps(&lowered);
    assert_eq!(steps[0]["effect"]["native"]["effect"], "fadeAndMove");
    let notes = reported(&lowered, "PX2413");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0]["effect"], "pptx:entr.29");
    assert!(reported(&lowered, "PX2410").is_empty());
}

#[test]
fn the_deck_path_verifies_and_resolves_the_timeline() {
    let probe = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../probes/out/motion-probe.pptx"
    ))
    .unwrap();
    for native_motion in [false, true] {
        let compiled = compile_motion(&probe, "motion-probe.pptx", native_motion);
        assert!(compiled.errors().is_empty(), "{:?}", compiled.errors());
        let presentation = compiled.presentation.as_ref().unwrap();
        let steps: usize = presentation
            .slides
            .iter()
            .map(|slide| slide.timeline.len())
            .sum();
        if native_motion {
            assert_eq!(steps, 339);
            assert!(
                !compiled
                    .diagnostics
                    .iter()
                    .any(|d| d.code.code() == "PX2401")
            );
        } else {
            assert_eq!(steps, 0);
        }
    }
}

#[test]
fn after_effects_on_a_later_click_keep_their_click() {
    // PowerPoint's "hide on next mouse click": the after effect sits in the next click group, alone or beside the
    // next effect. It plays there as a step of its own, so no click is lost.
    let after = |spid: u32, to: &str, attr: &str| {
        format!(
            "<p:par><p:cTn id=\"40\" presetID=\"10\" presetClass=\"entr\" fill=\"hold\" grpId=\"0\" afterEffect=\"1\" nodeType=\"afterEffect\"><p:stCondLst><p:cond delay=\"0\"/></p:stCondLst><p:childTnLst><p:set>{}<p:to>{to}</p:to></p:set></p:childTnLst></p:cTn></p:par>",
            cbhvr("1", &target(spid), attr)
        )
    };
    let hide = |spid: u32| after(spid, "<p:strVal val=\"hidden\"/>", "style.visibility");
    let dim = after(
        5,
        "<p:clrVal><a:srgbClr val=\"808080\"/></p:clrVal>",
        "style.color",
    );
    let groups = [
        click_group(&slot(0, &effect(ENTR_FADE, "0", &fade(3)))),
        click_group(&slot(0, &hide(3))),
        click_group(&slot(
            0,
            &effect(
                ENTR_FADE,
                "0",
                &format!("{}{}", show(&target(4)), fade_in(&target(4), "700")),
            ),
        )),
        click_group(&slot(0, &effect(ENTR_FADE, "0", &fade(5)))),
        click_group(&slot(
            0,
            &format!("{}{}", dim, effect(ENTR_FADE, "0", &fade(6))),
        )),
    ]
    .concat();
    let lowered = native(&deck(&timing(&groups)));
    let played = steps(&lowered);
    let shape = |s: &Value| {
        format!(
            "{} {} {} {}",
            s["trigger"].as_str().unwrap(),
            s["kind"].as_str().unwrap(),
            s["target"]["element"].as_str().unwrap(),
            s["durationMs"]
        )
    };
    assert_eq!(
        played.iter().map(shape).collect::<Vec<_>>(),
        [
            "click entrance pp-256.3 500",
            "click exit pp-256.3 0",
            "click entrance pp-256.4 700",
            "click entrance pp-256.5 500",
            "click emphasis pp-256.5 0",
            "withPrevious entrance pp-256.6 500",
        ]
    );
    assert!(played[0]["timing"].get("after").is_none());
    // A hide alone compiles to no keyframes: it plays the exit fallback, in no time.
    assert_eq!(played[1]["effect"]["fallback"], "dissolve");
    assert_eq!(reported(&lowered, "PX2410")[0]["effect"], "pptx:custom.0");
    // The dim has nothing to play on its own click: reported, the click kept.
    let after_losses: Vec<Value> = reported(&lowered, "PX2411")
        .into_iter()
        .filter(|d| d["what"] == "after")
        .collect();
    assert_eq!(after_losses.len(), 1);
    assert_eq!(after_losses[0]["spid"], "5");
    // In the effect's own slot, the after effect still belongs to it.
    let same = click_group(&slot(
        0,
        &format!("{}{}", effect(ENTR_FADE, "0", &fade(3)), hide(3)),
    ));
    let steps = steps(&native(&deck(&timing(&same))));
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0]["timing"]["after"], "hide");
}

/// The `what`, `native` and `read` of each PX2411, sorted.
fn timing_losses(lowered: &support::Lowered) -> Vec<String> {
    let mut losses: Vec<String> = reported(lowered, "PX2411")
        .iter()
        .map(|d| {
            format!(
                "{} {} {}",
                d["what"].as_str().unwrap_or(""),
                d["native"].as_str().unwrap_or(""),
                d["read"].as_str().unwrap_or("")
            )
            .trim_end()
            .to_owned()
        })
        .collect();
    losses.sort();
    losses
}

#[test]
fn every_value_read_as_other_than_written_is_reported() {
    // A transition duration past an hour clamps, and says so.
    let lowered = native(&deck(
        "<p:transition spd=\"slow\" p14:dur=\"99999999999\" xmlns:p14=\"http://schemas.microsoft.com/office/powerpoint/2010/main\"><p:fade/></p:transition>",
    ));
    assert_eq!(
        first_slide(&lowered)
            .transition
            .as_ref()
            .unwrap()
            .duration_ms
            .0,
        3_600_000
    );
    assert_eq!(timing_losses(&lowered), ["durationMs 99999999999 clamped"]);
    // Iterate intervals out of range, an unknown iterate type.
    let iterate = |spid: u32, ty: &str, interval: &str| {
        format!(
            "<p:par><p:cTn id=\"9\" {ENTR_FADE} fill=\"hold\"><p:stCondLst><p:cond delay=\"0\"/></p:stCondLst><p:iterate type=\"{ty}\">{interval}</p:iterate><p:childTnLst>{}</p:childTnLst></p:cTn></p:par>",
            fade(spid)
        )
    };
    let groups: String = [
        iterate(3, "wd", "<p:tmPct val=\"-1\"/>"),
        iterate(4, "lt", "<p:tmPct val=\"999999\"/>"),
        iterate(5, "wd", "<p:tmAbs val=\"indefinite\"/>"),
        iterate(6, "bogus", "<p:tmAbs val=\"50\"/>"),
    ]
    .iter()
    .map(|e| click_group(&slot(0, e)))
    .collect();
    let lowered = native(&deck(&timing(&groups)));
    let delivered = steps(&lowered);
    let delivery: Vec<&str> = delivered
        .iter()
        .map(|s| s["delivery"].as_str().unwrap())
        .collect();
    assert_eq!(delivery, ["byWord", "byChar", "byWord", "all"]);
    assert!(
        delivered
            .iter()
            .all(|s| s["timing"].get("stagger").is_none())
    );
    assert_eq!(
        timing_losses(&lowered),
        [
            "stagger -1 invalid",
            "stagger 999999 invalid",
            "stagger indefinite invalid"
        ]
    );
    let unknown = reported(&lowered, "PX2412");
    assert_eq!(unknown.len(), 1);
    assert_eq!(
        (
            &unknown[0]["option"],
            &unknown[0]["value"],
            &unknown[0]["used"]
        ),
        (&json!("iterate"), &json!("bogus"), &json!("el"))
    );
    // Keyframe times out of range, an angle that is no number, an effect duration that is none.
    let tav = |tm: &str, val: &str| {
        format!("<p:tav tm=\"{tm}\"><p:val><p:strVal val=\"{val}\"/></p:val></p:tav>")
    };
    let anim = format!(
        "<p:anim calcmode=\"lin\" valueType=\"num\">{}<p:tavLst>{}{}{}</p:tavLst></p:anim>",
        cbhvr("1000", &target(3), "ppt_x"),
        tav("0", "#ppt_x"),
        tav("250000", "#ppt_x+0.1"),
        tav("-5", "#ppt_x")
    );
    let rot = format!(
        "<p:animRot by=\"1e400\">{}</p:animRot>",
        cbhvr("500", &target(4), "r")
    );
    let own_dur = "<p:par><p:cTn id=\"9\" dur=\"NaN\" fill=\"hold\"><p:stCondLst><p:cond delay=\"0\"/></p:stCondLst><p:childTnLst><p:animEffect transition=\"in\" filter=\"fade\"><p:cBhvr><p:cTn id=\"0\"/><p:tgtEl><p:spTgt spid=\"5\"/></p:tgtEl></p:cBhvr></p:animEffect></p:childTnLst></p:cTn></p:par>";
    let groups: String = [
        effect("presetID=\"0\" presetClass=\"emph\"", "0", &anim),
        effect(
            "presetID=\"8\" presetClass=\"emph\" presetSubtype=\"0\"",
            "0",
            &rot,
        ),
        own_dur.to_owned(),
    ]
    .iter()
    .map(|e| click_group(&slot(0, e)))
    .collect();
    let lowered = native(&deck(&timing(&groups)));
    let played = steps(&lowered);
    assert_eq!(played[1]["effect"]["native"]["params"]["deg"], 360.0);
    assert_eq!(played[2]["durationMs"], 500);
    assert_eq!(
        timing_losses(&lowered),
        [
            "amount 1e400 invalid",
            "durationMs NaN invalid",
            "keyframe -5 invalid",
            "keyframe 250000 invalid"
        ]
    );
    // A repeat duration that asks for more repeats than the limit clamps; one shorter than the effect plays it
    // whole.
    let spin = |spid: u32, attrs: &str| {
        format!(
            "<p:par><p:cTn id=\"9\" presetID=\"8\" presetClass=\"emph\" presetSubtype=\"0\" fill=\"hold\" {attrs}><p:stCondLst><p:cond delay=\"0\"/></p:stCondLst><p:childTnLst><p:animRot by=\"21600000\">{}</p:animRot></p:childTnLst></p:cTn></p:par>",
            cbhvr("2", &target(spid), "r")
        )
    };
    let groups: String = [spin(3, "repeatDur=\"3600000\""), spin(4, "repeatDur=\"1\"")]
        .iter()
        .map(|e| click_group(&slot(0, e)))
        .collect();
    let lowered = native(&deck(&timing(&groups)));
    let spun = steps(&lowered);
    assert_eq!(spun[0]["timing"]["repeat"], json!({ "count": 1000.0 }));
    assert!(spun[1]["timing"].get("repeat").is_none());
    assert_eq!(
        timing_losses(&lowered),
        ["repeat", "repeat 1800000 clamped"]
    );
}

/// A media node's mute (or a volume of 0) and loop are the video element's playback; the rest is reported, as are mute
/// and loop on a target that is no video.
#[test]
fn a_media_nodes_mute_and_loop_reach_its_video() {
    let timing = |node: &str, ctn: &str| {
        format!(
            "<p:timing><p:tnLst><p:par><p:cTn id=\"1\" dur=\"indefinite\" restart=\"never\" nodeType=\"tmRoot\"><p:childTnLst><p:video><p:cMediaNode{node}><p:cTn id=\"50\" fill=\"hold\" display=\"0\"{ctn}><p:stCondLst><p:cond delay=\"indefinite\"/></p:stCondLst></p:cTn><p:tgtEl><p:spTgt spid=\"30\"/></p:tgtEl></p:cMediaNode></p:video></p:childTnLst></p:cTn></p:par></p:tnLst></p:timing>"
        )
    };
    let movie = |lowered: &support::Lowered| -> Value {
        let slide = serde_json::to_value(&lowered.presentation().slides[1]).unwrap();
        slide["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|element| element["id"] == "pp-257.30")
            .cloned()
            .unwrap()
    };
    let deck = |node: &str, ctn: &str| {
        support::fixtures::media_timed("st=\"1500\"", None, Some(&timing(node, ctn)))
    };
    let lowered = native(&deck(
        " vol=\"80000\" mute=\"1\" showWhenStopped=\"0\"",
        " repeatCount=\"indefinite\"",
    ));
    let video = movie(&lowered);
    assert_eq!(
        (&video["muted"], &video["loop"]),
        (&json!(true), &json!(true))
    );
    let losses: Vec<String> = reported(&lowered, "PX2411")
        .iter()
        .map(|d| d["options"].as_str().unwrap_or("").to_owned())
        .collect();
    assert_eq!(losses, ["hideWhenStopped"]);
    // A node that asks for neither leaves the video as it is; so does the parity path.
    let lowered = native(&deck(" vol=\"80000\"", ""));
    let video = movie(&lowered);
    assert_eq!(
        (&video["muted"], &video["loop"]),
        (&json!(false), &json!(false))
    );
    let parity = support::lower(&deck(" mute=\"1\"", " repeatCount=\"indefinite\""));
    let slide = serde_json::to_value(&parity.presentation().slides[1]).unwrap();
    let video = slide["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["id"] == "pp-257.30")
        .unwrap();
    assert_eq!(
        (&video["muted"], &video["loop"]),
        (&json!(false), &json!(false))
    );
    // A volume of 0 is muted too, in thousandths of a percent or as a percentage; one that cannot be read is heard.
    for (vol, muted) in [("0", true), ("0%", true), ("1", false), ("loud", false)] {
        let lowered = native(&deck(&format!(" vol=\"{vol}\""), ""));
        assert_eq!(movie(&lowered)["muted"], json!(muted), "vol {vol}");
    }
}

#[test]
fn media_nodes_report_what_the_deck_does_not_play() {
    let video = |spid: u32, node: &str, ctn: &str| {
        format!(
            "<p:video><p:cMediaNode vol=\"80000\"{node}><p:cTn id=\"50\" fill=\"hold\" display=\"0\"{ctn}><p:stCondLst><p:cond delay=\"indefinite\"/></p:stCondLst></p:cTn><p:tgtEl><p:spTgt spid=\"{spid}\"/></p:tgtEl></p:cMediaNode></p:video>"
        )
    };
    let rest = [
        video(3, "", ""),
        video(4, " mute=\"1\"", " repeatCount=\"indefinite\""),
        video(5, " showWhenStopped=\"0\" numSld=\"3\"", ""),
        "<p:audio><p:cMediaNode mute=\"1\"><p:cTn id=\"51\"/><p:tgtEl><p:spTgt spid=\"6\"/></p:tgtEl></p:cMediaNode></p:audio>".to_owned(),
    ]
    .concat();
    let lowered = native(&deck(&timing_with(
        &click_group(&slot(0, &effect(ENTR_FADE, "0", &fade(3)))),
        &rest,
        "",
    )));
    assert_eq!(first_slide(&lowered).timeline.len(), 1);
    let mut media: Vec<String> = reported(&lowered, "PX2411")
        .iter()
        .map(|d| {
            format!(
                "{} {} {}",
                d["what"].as_str().unwrap(),
                d["spid"].as_str().unwrap(),
                d["options"].as_str().unwrap()
            )
        })
        .collect();
    media.sort();
    assert_eq!(
        media,
        [
            "media 4 mute loop",
            "media 5 slides hideWhenStopped",
            "media 6 mute"
        ]
    );
}

/// A behaviour's own repeat (`repeatCount`, `repeatDur` on its `p:cTn`) plays its span, reversal included, that many
/// times, and the step lasts that long. One that does not fold (indefinite) plays once, named in PX2410.
#[test]
fn a_behaviours_own_repeat_plays() {
    let tgt = target(3);
    let pulse = |repeat: &str| {
        format!(
            "<p:animScale><p:cBhvr><p:cTn id=\"0\" dur=\"250\" autoRev=\"1\"{repeat} fill=\"hold\"/>{tgt}</p:cBhvr><p:by x=\"110000\" y=\"110000\"/></p:animScale>"
        )
    };
    let bulb = "presetID=\"26\" presetClass=\"emph\" presetSubtype=\"0\" grpId=\"0\"";
    let groups: String = [
        pulse(" repeatCount=\"3000\""),
        pulse(" repeatDur=\"1000\""),
        pulse(""),
        pulse(" repeatCount=\"indefinite\""),
    ]
    .iter()
    .map(|b| click_group(&slot(0, &effect(bulb, "0", b))))
    .collect();
    let lowered = native(&deck(&timing(&groups)));
    let steps = steps(&lowered);
    let rest = json!({ "x": 1.0, "y": 1.0 });
    let big = json!({ "x": 1.1, "y": 1.1 });
    let scale = |i: usize| steps[i]["effect"]["native"]["params"][0].clone();
    assert_eq!(scale(0)["durationMs"], 1500);
    assert_eq!(steps[0]["durationMs"], 1500);
    let values: Vec<Value> = scale(0)["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["value"].clone())
        .collect();
    assert_eq!(values.iter().filter(|v| **v == big).count(), 3);
    assert_eq!(values.first(), Some(&rest));
    assert_eq!(values.last(), Some(&rest));
    // A total time counts doubled spans: 1000 ms of a 500 ms pulse is twice.
    assert_eq!(scale(1)["durationMs"], 1000);
    assert_eq!(steps[1]["durationMs"], 1000);
    assert_eq!(scale(2)["durationMs"], 500);
    assert_eq!(scale(3)["durationMs"], 500);
    let plays: Vec<Value> = reported(&lowered, "PX2410")
        .into_iter()
        .map(|data| data["play"].clone())
        .collect();
    assert_eq!(
        plays,
        [json!(
            "its behaviours without the repeat of its animScale animation"
        )]
    );
}

/// A colour from the element's own colour, accelerated and reversing on its own (LibreOffice's vertical highlight as
/// PowerPoint writes it), folds into mixes of the own colour that the deck builder resolves: the colour plays, with
/// nothing reported.
#[test]
fn a_colour_from_its_own_folds_through_an_acceleration() {
    let tgt = target(3);
    let colour = format!(
        "<p:animClr clrSpc=\"rgb\"><p:cBhvr><p:cTn id=\"0\" dur=\"500\" accel=\"50000\" autoRev=\"1\" fill=\"hold\"/>{tgt}<p:attrNameLst><p:attrName>style.color</p:attrName></p:attrNameLst></p:cBhvr><p:to><a:srgbClr val=\"333399\"/></p:to></p:animClr>"
    );
    let custom = "presetID=\"0\" presetClass=\"emph\" presetSubtype=\"0\" grpId=\"0\"";
    let groups = click_group(&slot(0, &effect(custom, "0", &colour)));
    let lowered = native(&deck(&timing(&groups)));
    let steps = steps(&lowered);
    let track = &steps[0]["effect"]["native"]["params"][0];
    assert_eq!(track["property"], "text");
    assert_eq!(track["durationMs"], 1000);
    let frames = track["frames"].as_array().unwrap();
    assert!(
        frames.iter().any(|f| f["value"]["toward"] == "#333399"),
        "{track}"
    );
    assert!(frames.iter().any(|f| f["value"] == "#333399"), "{track}");
    assert!(reported(&lowered, "PX2410").is_empty());
}

/// An entrance whose switch to visible does not hold shows its target only until the switch ends, whatever else its
/// behaviours do after: a fade whose 1 ms switch has no `fill` lasts 1 ms and hides its target, as in ODP.
#[test]
fn an_entrance_shows_only_while_its_switch_lasts() {
    let tgt = target(3);
    let shown = |fill: &str| {
        format!(
            "<p:set><p:cBhvr><p:cTn id=\"0\" dur=\"1\"{fill}><p:stCondLst><p:cond delay=\"0\"/></p:stCondLst></p:cTn>{tgt}<p:attrNameLst><p:attrName>style.visibility</p:attrName></p:attrNameLst></p:cBhvr><p:to><p:strVal val=\"visible\"/></p:to></p:set><p:animEffect transition=\"in\" filter=\"fade\"><p:cBhvr><p:cTn id=\"0\" dur=\"500\"/>{tgt}</p:cBhvr></p:animEffect>"
        )
    };
    let fade = "presetID=\"10\" presetClass=\"entr\" presetSubtype=\"0\" grpId=\"0\"";
    let groups: String = [
        effect(fade, "0", &shown("")),
        effect(fade, "0", &shown(" fill=\"hold\"")),
    ]
    .iter()
    .map(|e| click_group(&slot(0, e)))
    .collect();
    let lowered = native(&deck(&timing(&groups)));
    let steps = steps(&lowered);
    assert_eq!(steps[0]["effect"]["native"]["effect"], "fade");
    assert_eq!(steps[0]["durationMs"], 1);
    assert_eq!(steps[0]["timing"]["after"], "hide");
    assert_eq!(steps[1]["durationMs"], 500);
}
