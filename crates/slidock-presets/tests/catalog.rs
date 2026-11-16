//! Each format's effect catalog (PowerPoint's, LibreOffice's and Keynote's transitions and builds) maps row by row:
//! every native effect to a vocabulary entry or a declared fallback, with no option read as a default, and every
//! entry valid and round-tripping through serde.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code"
)]

use slidock_ir::effects::{BuildEffect, BuildFamily, StepKind, TransitionEffect};
use slidock_ir::validate::validate;
use slidock_presets::{BuildPlay, BuildRow, Certainty, TransitionRow, keynote, odp, pptx};

fn check_transition(what: &str, row: &TransitionRow) {
    if let Some(effect) = &row.effect {
        assert_eq!(row.canonical_fallback, effect.fallback(), "{what}");
        assert!(
            validate(effect).is_empty(),
            "{what}: {:?}",
            validate(effect)
        );
        let json = serde_json::to_value(effect).unwrap();
        assert_eq!(
            serde_json::from_value::<TransitionEffect>(json).unwrap(),
            *effect,
            "{what}"
        );
    }
}

fn check_build(what: &str, row: &BuildRow, kind: StepKind) {
    if let BuildPlay::Effect(effect) = &row.play {
        assert!(
            validate(effect).is_empty(),
            "{what}: {:?}",
            validate(effect)
        );
        assert!(effect.plays_in(kind), "{what} in {kind:?}");
        let json = serde_json::to_value(effect).unwrap();
        assert_eq!(
            serde_json::from_value::<BuildEffect>(json).unwrap(),
            *effect,
            "{what}"
        );
    }
}

/// A `p:transition` child and the values of each of its options (ECMA-376 §19.3.1, [MS-PPTX] §2.3).
type TransitionCase = (
    &'static str,
    &'static [(&'static str, &'static [&'static str])],
);

const SIDE: &[&str] = &["l", "u", "r", "d"];
const EIGHT: &[&str] = &["l", "u", "r", "d", "lu", "ru", "ld", "rd"];
const ORIENT: &[&str] = &["horz", "vert"];
const IN_OUT: &[&str] = &["in", "out"];
const LEFT_RIGHT: &[&str] = &["l", "r"];
const FLAG: &[&str] = &["0", "1"];
const SPOKES: &[&str] = &["1", "2", "3", "4", "8"];

/// Every transition PowerPoint writes.
const PPTX_TRANSITIONS: &[TransitionCase] = &[
    ("blinds", &[("dir", ORIENT)]),
    ("checker", &[("dir", ORIENT)]),
    ("comb", &[("dir", ORIENT)]),
    ("randomBar", &[("dir", ORIENT)]),
    ("cover", &[("dir", EIGHT)]),
    ("pull", &[("dir", EIGHT)]),
    ("push", &[("dir", SIDE)]),
    ("wipe", &[("dir", SIDE)]),
    ("strips", &[("dir", &["lu", "ru", "ld", "rd"])]),
    ("split", &[("orient", ORIENT), ("dir", IN_OUT)]),
    ("zoom", &[("dir", IN_OUT)]),
    ("wheel", &[("spokes", SPOKES)]),
    ("fade", &[("thruBlk", FLAG)]),
    ("cut", &[("thruBlk", FLAG)]),
    ("circle", &[]),
    ("diamond", &[]),
    ("plus", &[]),
    ("wedge", &[]),
    ("newsflash", &[]),
    ("random", &[]),
    ("dissolve", &[]),
    // p14
    ("vortex", &[("dir", SIDE)]),
    ("switch", &[("dir", LEFT_RIGHT)]),
    ("flip", &[("dir", LEFT_RIGHT)]),
    ("ripple", &[]),
    ("honeycomb", &[]),
    (
        "prism",
        &[("dir", SIDE), ("isContent", FLAG), ("isInverted", FLAG)],
    ),
    ("doors", &[("dir", ORIENT)]),
    ("window", &[("dir", ORIENT)]),
    ("ferris", &[("dir", LEFT_RIGHT)]),
    ("gallery", &[("dir", LEFT_RIGHT)]),
    ("conveyor", &[("dir", LEFT_RIGHT)]),
    ("pan", &[("dir", SIDE)]),
    ("glitter", &[("dir", SIDE)]),
    ("warp", &[("dir", IN_OUT)]),
    ("flythrough", &[("dir", IN_OUT), ("hasBounce", FLAG)]),
    ("flash", &[]),
    ("shred", &[]),
    ("reveal", &[("thruBlk", FLAG), ("dir", LEFT_RIGHT)]),
    ("wheelReverse", &[("spokes", SPOKES)]),
    // p15
    ("prstTrans", &[("prst", &pptx::PRST_TRANS)]),
    // p159
    ("morph", &[("option", &["byObject", "byWord", "byChar"])]),
];

/// Every combination of a case's option values.
fn combinations(
    options: &[(&'static str, &'static [&'static str])],
) -> Vec<Vec<(&'static str, &'static str)>> {
    let mut all = vec![Vec::new()];
    for (name, values) in options {
        all = all
            .into_iter()
            .flat_map(|chosen: Vec<(&str, &str)>| {
                values.iter().map(move |value| {
                    let mut next = chosen.clone();
                    next.push((*name, *value));
                    next
                })
            })
            .collect();
    }
    all
}

#[test]
fn every_pptx_transition_maps() {
    let mut rows = 0;
    for (element, options) in PPTX_TRANSITIONS {
        // The element with its schema defaults, then with every option value.
        let bare = pptx::transition(element, |_| None).unwrap_or_else(|| panic!("{element}"));
        assert!(bare.unknown.is_empty(), "{element}");
        for chosen in combinations(options) {
            let lookup = pptx::transition(element, |name| {
                chosen
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| *value)
            })
            .unwrap_or_else(|| panic!("{element} {chosen:?}"));
            assert!(
                lookup.unknown.is_empty(),
                "{element} {chosen:?}: {:?}",
                lookup.unknown
            );
            check_transition(element, &lookup.row);
            rows += 1;
        }
    }
    assert_eq!(PPTX_TRANSITIONS.len(), 42);
    assert_eq!(rows, 134);
}

fn pptx_effect(element: &str, attrs: &[(&str, &'static str)]) -> Option<TransitionEffect> {
    pptx::transition(element, |name| {
        attrs
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| *value)
    })
    .unwrap()
    .row
    .effect
}

#[test]
fn pptx_transitions_keep_the_content_moves_convention() {
    let json = |effect: Option<TransitionEffect>| serde_json::to_value(effect.unwrap()).unwrap();
    assert_eq!(
        json(pptx_effect("push", &[("dir", "d")])),
        serde_json::json!({ "effect": "push", "params": { "dir": "d" } })
    );
    assert_eq!(
        json(pptx_effect("push", &[])),
        serde_json::json!({ "effect": "push", "params": { "dir": "l" } })
    );
    assert_eq!(
        json(pptx_effect("pull", &[("dir", "ru")])),
        serde_json::json!({ "effect": "uncover", "params": { "dir": "ru" } })
    );
    assert_eq!(
        json(pptx_effect("checker", &[("dir", "vert")])),
        serde_json::json!({ "effect": "checker", "params": { "dir": "d" } })
    );
    assert_eq!(
        json(pptx_effect("strips", &[])),
        serde_json::json!({ "effect": "strips", "params": { "corner": "lu" } })
    );
    assert_eq!(
        json(pptx_effect("wheelReverse", &[("spokes", "3")])),
        serde_json::json!({ "effect": "wheel", "params": { "spokes": 3, "cw": false } })
    );
    assert_eq!(
        json(pptx_effect("flash", &[])),
        serde_json::json!({ "effect": "fade", "params": { "through": "white" } })
    );
    assert_eq!(
        json(pptx_effect("morph", &[("option", "byChar")])),
        serde_json::json!({ "effect": "morph", "params": { "by": "char" } })
    );
    for element in [
        "vortex",
        "ripple",
        "glitter",
        "honeycomb",
        "shred",
        "prism",
        "prstTrans",
    ] {
        let row = pptx::transition(element, |_| None).unwrap().row;
        assert_eq!(row.effect, None, "{element}");
        assert_eq!(
            row.canonical_fallback,
            slidock_ir::effects::TransitionFallback::Dissolve
        );
    }
}

#[test]
fn unknown_pptx_values_fail_closed() {
    assert!(pptx::transition("sparkle", |_| None).is_none());
    let lookup = pptx::transition("push", |name| (name == "dir").then_some("lu")).unwrap();
    assert_eq!(
        serde_json::to_value(lookup.row.effect.unwrap()).unwrap(),
        serde_json::json!({ "effect": "push", "params": { "dir": "l" } })
    );
    assert_eq!(lookup.unknown.len(), 1);
    assert_eq!(lookup.unknown[0].option, "dir");
    assert_eq!(lookup.unknown[0].used, "l");
    let wheel = pptx::transition("wheel", |name| (name == "spokes").then_some("9")).unwrap();
    assert_eq!(wheel.unknown[0].value, "9");
    let prst = pptx::transition("prstTrans", |name| (name == "prst").then_some("vortex2")).unwrap();
    assert_eq!(prst.unknown.len(), 1);
}

/// PowerPoint's preset ids per class.
const PPTX_ENTRANCES: [u32; 52] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
    27, 28, 29, 30, 31, 34, 35, 37, 38, 39, 40, 41, 42, 43, 45, 47, 48, 49, 50, 51, 52, 53, 54, 55,
    56, 58,
];
const PPTX_EMPHASES: [u32; 31] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 14, 15, 16, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 30, 31,
    32, 33, 34, 35, 36,
];

#[test]
fn every_pptx_preset_maps() {
    let mut rows = 0;
    for (class, ids) in [
        ("entr", PPTX_ENTRANCES.to_vec()),
        ("exit", PPTX_ENTRANCES.to_vec()),
        ("emph", PPTX_EMPHASES.to_vec()),
        ("path", (0..=pptx::PATHS).collect()),
    ] {
        let kind = pptx::PptxClass::parse(class).unwrap().kind();
        for id in ids {
            let lookup = pptx::build(class, id, 0).unwrap_or_else(|| panic!("{class}.{id}"));
            assert!(lookup.unknown.is_empty(), "{class}.{id}");
            check_build(&format!("{class}.{id}"), &lookup.row, kind);
            rows += 1;
        }
    }
    assert_eq!(rows, 52 + 52 + 31 + 65);
    for (class, ids) in [
        (pptx::PptxClass::Entrance, PPTX_ENTRANCES.to_vec()),
        (pptx::PptxClass::Exit, PPTX_ENTRANCES.to_vec()),
        (pptx::PptxClass::Emphasis, PPTX_EMPHASES.to_vec()),
    ] {
        let mut listed: Vec<u32> = pptx::presets(class).map(|(id, _)| id).collect();
        listed.sort_unstable();
        assert_eq!(listed, ids, "{class:?}");
    }
    assert_eq!(
        pptx::preset_name(pptx::PptxClass::Exit, 2),
        Some("ooo-exit-fly-out")
    );
    assert!(pptx::build("entr", 57, 0).is_none());
    assert!(pptx::build("emph", 11, 0).is_none());
    assert!(pptx::build("path", 65, 0).is_none());
    assert!(pptx::build("entrance", 1, 0).is_none());
}

fn preset_json(class: &str, id: u32, subtype: u32) -> serde_json::Value {
    let lookup = pptx::build(class, id, subtype).unwrap();
    assert!(
        lookup.unknown.is_empty(),
        "{class}.{id}/{subtype}: {:?}",
        lookup.unknown
    );
    serde_json::to_value(lookup.row.build_effect().unwrap()).unwrap()
}

#[test]
fn pptx_subtypes_decode_to_directions() {
    use serde_json::json;
    // Fly In from the left moves right; Fly Out to the left moves left.
    assert_eq!(preset_json("entr", 2, 8)["params"]["dir"], json!("r"));
    assert_eq!(preset_json("exit", 2, 8)["params"]["dir"], json!("l"));
    assert_eq!(preset_json("entr", 2, 3)["params"]["dir"], json!("ld"));
    assert_eq!(preset_json("entr", 2, 0)["params"]["dir"], json!("u"));
    // A wipe from the top moves down, entering or leaving.
    assert_eq!(preset_json("entr", 22, 1)["params"]["dir"], json!("d"));
    assert_eq!(preset_json("exit", 22, 1)["params"]["dir"], json!("d"));
    assert_eq!(
        preset_json("entr", 16, 21),
        json!({ "effect": "split", "params": { "orient": "vert", "inOut": "in" } })
    );
    assert_eq!(preset_json("entr", 5, 5)["params"]["dir"], json!("d"));
    assert_eq!(preset_json("entr", 5, 10)["params"]["dir"], json!("r"));
    // Strips 3 is LibreOffice's right-to-top, which plays `waterfallWipe horizontalLeft` reversed: toward the top
    // left. Blinds 10 (LibreOffice's `vertical`) plays `blindsWipe horizontal`.
    assert_eq!(preset_json("entr", 18, 3)["params"]["corner"], json!("lu"));
    assert_eq!(preset_json("exit", 18, 3)["params"]["corner"], json!("rd"));
    assert_eq!(
        preset_json("entr", 3, 10)["params"]["orient"],
        json!("horz")
    );
    assert_eq!(
        preset_json("exit", 14, 5)["params"]["orient"],
        json!("vert")
    );
    assert_eq!(preset_json("entr", 21, 8)["params"]["spokes"], json!(8));
    assert_eq!(
        preset_json("entr", 23, 528)["params"]["origin"],
        json!("slideCenter")
    );
    assert_eq!(
        preset_json("entr", 23, 272)["params"]["slight"],
        json!(true)
    );
    assert_eq!(
        preset_json("entr", 4, 32)["params"],
        json!({ "shape": "box", "inOut": "out" })
    );
    assert_eq!(
        preset_json("entr", 42, 0),
        json!({ "effect": "floatIn", "params": { "dir": "u" } })
    );
    assert_eq!(preset_json("entr", 53, 0)["params"]["fade"], json!(true));
    assert_eq!(pptx::build_id("entr", 2, 4).to_string(), "pptx:entr.2/4");
    assert_eq!(pptx::build_id("entr", 10, 0).to_string(), "pptx:entr.10");
    // An unknown subtype reads as the preset's default, and is listed.
    let odd = pptx::build("entr", 2, 7).unwrap();
    assert_eq!(odd.unknown.len(), 1);
    assert_eq!(odd.unknown[0].value, "7");
    assert_eq!(
        serde_json::to_value(odd.row.build_effect().unwrap()).unwrap()["params"]["dir"],
        json!("u")
    );
}

#[test]
fn every_libreoffice_preset_maps() {
    let effects = slidock_presets::libreoffice_effects();
    assert_eq!(effects.len(), 305);
    let mut per_class = std::collections::BTreeMap::new();
    for preset in effects {
        let lookup = odp::build(preset.class, preset.id, preset.subtype)
            .unwrap_or_else(|| panic!("{preset:?}"));
        assert!(
            lookup.unknown.is_empty(),
            "{preset:?}: {:?}",
            lookup.unknown
        );
        let kind = slidock_presets::ooo::PresetClass::parse(preset.class)
            .unwrap()
            .kind()
            .unwrap();
        check_build(preset.id, &lookup.row, kind);
        *per_class.entry(preset.class).or_insert(0) += 1;
    }
    assert_eq!(
        per_class.into_iter().collect::<Vec<_>>(),
        [
            ("emphasis", 34),
            ("entrance", 112),
            ("exit", 92),
            ("media-call", 3),
            ("motion-path", 64)
        ]
    );
    assert_eq!(
        effects
            .iter()
            .filter(|preset| preset.iterate.is_some())
            .count(),
        21
    );
}

#[test]
fn every_libreoffice_transition_maps() {
    let transitions = slidock_presets::libreoffice_transitions();
    assert_eq!(transitions.len(), 69);
    let mut mapped = 0;
    for preset in transitions {
        let lookup = odp::transition(
            preset.smil_type,
            preset.subtype,
            preset.reverse,
            preset.fade_color,
        )
        .unwrap_or_else(|| panic!("{preset:?}"));
        assert!(
            lookup.unknown.is_empty(),
            "{preset:?}: {:?}",
            lookup.unknown
        );
        check_transition(preset.id, &lookup.row);
        mapped += usize::from(lookup.row.effect.is_some());
    }
    // Snake and spiral wipes (6) keep their fallback.
    assert_eq!(mapped, 69 - 8);
}

fn odp_transition(id: &str) -> serde_json::Value {
    let preset = slidock_presets::libreoffice_transitions()
        .iter()
        .find(|preset| preset.id == id)
        .unwrap();
    let lookup = odp::transition(
        preset.smil_type,
        preset.subtype,
        preset.reverse,
        preset.fade_color,
    )
    .unwrap();
    serde_json::to_value(lookup.row.effect.unwrap()).unwrap()
}

#[test]
fn odp_transitions_name_the_motion_as_libreoffice_does() {
    use serde_json::json;
    // LibreOffice's preset names say where the content goes; the vocabulary's `dir` agrees.
    for (id, effect) in [
        (
            "push-down",
            json!({ "effect": "push", "params": { "dir": "d" } }),
        ),
        (
            "push-left",
            json!({ "effect": "push", "params": { "dir": "l" } }),
        ),
        (
            "cover-right-up",
            json!({ "effect": "cover", "params": { "dir": "ru" } }),
        ),
        (
            "uncover-down",
            json!({ "effect": "uncover", "params": { "dir": "d" } }),
        ),
        (
            "uncover-left-up",
            json!({ "effect": "uncover", "params": { "dir": "lu" } }),
        ),
        (
            "wipe-up",
            json!({ "effect": "wipe", "params": { "dir": "u", "soft": false } }),
        ),
        (
            "wipe-left",
            json!({ "effect": "wipe", "params": { "dir": "l", "soft": false } }),
        ),
        (
            "split-vertical-in",
            json!({ "effect": "split", "params": { "orient": "vert", "inOut": "in" } }),
        ),
        (
            "box-in",
            json!({ "effect": "iris", "params": { "shape": "box", "inOut": "in" } }),
        ),
        (
            "diagonal-squares-right-up",
            json!({ "effect": "strips", "params": { "corner": "ru" } }),
        ),
        (
            "fade-through-white",
            json!({ "effect": "fade", "params": { "through": "white" } }),
        ),
        (
            "cut-through-black",
            json!({ "effect": "cut", "params": { "throughBlack": true } }),
        ),
        (
            "reverse-clock-wipe-twelve",
            json!({ "effect": "clock", "params": { "cw": false, "from": "12" } }),
        ),
        (
            "wheel-clockwise-3-spokes",
            json!({ "effect": "wheel", "params": { "spokes": 3, "cw": true } }),
        ),
    ] {
        assert_eq!(odp_transition(id), effect, "{id}");
    }
    let colored = odp::transition("fade", Some("fadeOverColor"), false, Some("#336699")).unwrap();
    assert_eq!(
        serde_json::to_value(colored.row.effect.unwrap()).unwrap(),
        json!({ "effect": "fade", "params": { "through": "color", "color": "#336699" } })
    );
    let bad = odp::transition("fade", Some("fadeOverColor"), false, Some("blue")).unwrap();
    assert_eq!(bad.unknown[0].option, "smil:fadeColor");
    assert!(odp::transition("teleport", None, false, None).is_none());
    let odd = odp::transition("pushWipe", Some("fromNowhere"), false, None).unwrap();
    assert_eq!(odd.unknown[0].used, "fromLeft");
    assert_eq!(
        odp::transition_id("pushWipe", Some("fromLeft"), false).to_string(),
        "odp:pushWipe/fromLeft"
    );
    assert_eq!(
        odp::build_id("ooo-entrance-fly-in").to_string(),
        "odp:ooo-entrance-fly-in"
    );
}

#[test]
fn odp_presets_decode_their_sub_types() {
    use serde_json::json;
    let effect = |class: &str, id: &str, subtype: Option<&str>| {
        let lookup = odp::build(class, id, subtype).unwrap();
        assert!(lookup.unknown.is_empty());
        serde_json::to_value(lookup.row.build_effect().unwrap()).unwrap()
    };
    assert_eq!(
        effect("entrance", "ooo-entrance-fly-in", Some("from-bottom"))["params"]["dir"],
        json!("u")
    );
    assert_eq!(
        effect("exit", "ooo-exit-fly-out", Some("to-bottom"))["params"]["dir"],
        json!("d")
    );
    assert_eq!(
        effect("exit", "ooo-exit-crawl-out", Some("to-left"))["params"],
        json!({ "dir": "l", "slow": true })
    );
    assert_eq!(
        effect("entrance", "ooo-entrance-swivel", Some("vertical"))["params"]["axis"],
        json!("horz")
    );
    assert_eq!(
        effect("exit", "ooo-exit-zoom", Some("544"))["params"]["origin"],
        json!("slideCenter")
    );
    assert_eq!(
        effect("media-call", "ooo-media-start", None),
        json!({ "effect": "play", "params": { "from": 0 } })
    );
    let lettering = odp::build("entrance", "ooo-entrance-colored-lettering", None)
        .unwrap()
        .row;
    assert_eq!(
        lettering.delivery,
        Some(slidock_ir::effects::Delivery::ByChar)
    );
    assert!(lettering.approximate);
    let custom = odp::build("emphasis", "ooo-emphasis-darken", None)
        .unwrap()
        .row;
    assert_eq!(custom.play, BuildPlay::Behaviors);
    assert_eq!(
        custom.canonical_fallback,
        slidock_ir::effects::StepFallback::None
    );
    assert!(odp::build("entrance", "ooo-entrance-teleport", None).is_none());
    assert!(odp::build("custom", "ooo-entrance-appear", None).is_none());
    assert!(odp::build("entrance", "ooo-exit-fade-out", None).is_none());
    let odd = odp::build("entrance", "ooo-entrance-wheel", Some("9")).unwrap();
    assert_eq!(odd.unknown[0].used, "1");
}

/// Keynote 14.5's built-in effect ids.
const KEYNOTE_BUILT_IN: &[&str] = &[
    "apple:dissolve",
    "apple:c-dissolve",
    "apple:bc-dissolve",
    "apple:gallery-dissolve",
    "apple:motion dissolve",
    "apple:dissolve character",
    "apple:appear",
    "apple:bc-appear",
    "apple:gallery-appear",
    "apple:fade",
    "apple:move in",
    "apple:move in character",
    "apple:bc-move",
    "apple:gallery-move",
    "apple:push",
    "apple:ca-push",
    "apple:reveal",
    "apple:wipe",
    "apple:wipe-iris",
    "apple:wipe-radial",
    "apple:scale",
    "apple:bc-scale-big",
    "apple:zoom",
    "apple:zoom character",
    "apple:bc-zoom",
    "apple:bc-zoom-big",
    "apple:bw-zoom-big",
    "apple:sidezoom",
    "apple:ca-zoom",
    "apple:pop",
    "apple:bc-pop",
    "apple:bw-pop",
    "apple:ca-pop",
    "apple:drop",
    "apple:dropbuild",
    "apple:dropbuild2",
    "apple:bc-drop",
    "apple:fall",
    "apple:spin",
    "apple:twirl",
    "apple:twist-and-scale",
    "apple:revolve",
    "apple:ca-revolve",
    "apple:fade and move",
    "apple:fade-and-move",
    "apple:fade and move character",
    "apple:gallery-fade-and-move",
    "apple:drift",
    "apple:drift and scale",
    "apple:drift and scale character",
    "apple:drift object",
    "apple:bc-typewriter",
    "apple:keyboard",
    "apple:compress",
    "apple:bc-expand",
    "apple:pivot",
    "apple:pivot-build",
    "apple:superflip",
    "apple:bc-superflip",
    "apple:bc-flip",
    "apple:pageflip",
    "apple:ca-cube",
    "apple:3d-rotate",
    "apple:3d-rotatinggrow",
    "apple:3d-crane",
    "apple:ca-swing",
    "apple:ca-isometric",
    "apple:doorway",
    "apple:apple-grid",
    "apple:slide",
    "apple:drive-by",
    "apple:drive-by object",
    "apple:magic-move-implied-motion-path",
    "apple:ca-implied-motion-path",
    "apple:ca-anagram",
    "apple:ca-text-shimmer",
    "apple:ca-text-sparkle",
    "apple:ca-dissolve-and-flip",
    "apple:action-motion-path",
    "apple:emphasis-motion-path",
    "apple:action-rotation",
    "apple:emphasis-rotation",
    "apple:action-scale",
    "apple:emphasis-scale",
    "apple:action-opacity",
    "apple:emphasis-opacity",
    "apple:action-blink",
    "apple:action-pulse",
    "apple:action-pop",
    "apple:action-bounce",
    "apple:action-jiggle",
    "apple:action-flip",
    "apple:movie-start",
    "apple:movie-stop",
    "apple:audio-start",
    "apple:audio-stop",
    "apple:web-video-start",
    "apple:magic chart",
    "apple:bounce",
    "apple:burn",
    "apple:fall-apart",
    "apple:fallingtiles",
    "apple:droplet",
    "apple:flash",
    "apple:bc-orbital",
    "apple:bw-dissolve",
    "apple:bc-zoom-big character",
    "apple:bc-3",
    "com.apple.iWork.Keynote.BUKAnvil",
    "com.apple.iWork.Keynote.BUKFlashBulbs",
    "com.apple.iWork.Keynote.BUKFlop",
    "com.apple.iWork.Keynote.BUKLensFlare",
    "com.apple.iWork.Keynote.BUKTwist",
    "com.apple.iWork.Keynote.KLNBCBlast",
    "com.apple.iWork.Keynote.KLNBCConvergence",
    "com.apple.iWork.Keynote.KLNBCImplodeExplode",
    "com.apple.iWork.Keynote.KLNBlur",
    "com.apple.iWork.Keynote.KLNBouncy",
    "com.apple.iWork.Keynote.KLNBulletFlare",
    "com.apple.iWork.Keynote.KLNColorPlanes",
    "com.apple.iWork.Keynote.KLNComet",
    "com.apple.iWork.Keynote.KLNConfetti",
    "com.apple.iWork.Keynote.KLNConfettiByWord",
    "com.apple.iWork.Keynote.KLNConfettiByCharacter",
    "com.apple.iWork.Keynote.KLNDiffuse",
    "com.apple.iWork.Keynote.KLNExplosion",
    "com.apple.iWork.Keynote.KLNFlame",
    "com.apple.iWork.Keynote.KLNShimmer",
    "com.apple.iWork.Keynote.KLNShockwave",
    "com.apple.iWork.Keynote.KLNSparkle",
    "com.apple.iWork.Keynote.KLNSquish",
    "com.apple.iWork.Keynote.KLNSwap",
    "com.apple.iWork.Keynote.KNBuildSkid",
    "com.apple.iWork.Keynote.KNBuildSkidByCharacter",
    "com.apple.iWork.Keynote.KNFireworks",
];

#[test]
fn every_keynote_id_maps() {
    assert_eq!(KEYNOTE_BUILT_IN.len(), 135);
    for id in KEYNOTE_BUILT_IN {
        assert!(keynote::IDS.contains(id), "{id} is missing from the table");
    }
    assert_eq!(keynote::IDS.len(), 150);
    let mut sorted = keynote::IDS.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), keynote::IDS.len(), "duplicate ids");
    let (mut builds, mut transitions) = (0, 0);
    for id in keynote::IDS {
        let build = keynote::build(id, StepKind::Entrance);
        let transition = keynote::transition(id);
        assert!(
            build.is_some() || transition.is_some(),
            "{id} maps to nothing"
        );
        if let Some(row) = &build {
            // Media and motion ids lower to media and motion steps.
            let kind = match row.build_effect().map(BuildEffect::family) {
                Some(BuildFamily::Media) => StepKind::Media,
                Some(BuildFamily::Motion) => StepKind::Motion,
                _ => StepKind::Entrance,
            };
            check_build(id, row, kind);
            builds += 1;
        }
        if let Some(row) = &transition {
            check_transition(id, row);
            transitions += 1;
        }
    }
    assert_eq!((builds, transitions), (134, 89));
    assert!(keynote::build("apple:teleport", StepKind::Entrance).is_none());
    assert!(keynote::transition("com.apple.iWork.Keynote.exportPDF").is_none());
}

#[test]
fn keynote_rows_are_inferred_unless_seen() {
    let appear = keynote::build("apple:appear", StepKind::Entrance).unwrap();
    assert_eq!(appear.certainty, Certainty::Confirmed);
    assert_eq!(
        appear.canonical_fallback,
        slidock_ir::effects::StepFallback::Appear
    );
    let move_in = keynote::build("apple:move in character", StepKind::Exit).unwrap();
    assert_eq!(move_in.certainty, Certainty::Inferred);
    assert!(move_in.refine);
    assert_eq!(
        move_in.delivery,
        Some(slidock_ir::effects::Delivery::ByChar)
    );
    let magic = keynote::transition("apple:magic-move-implied-motion-path").unwrap();
    assert_eq!(magic.certainty, Certainty::Confirmed);
    assert_eq!(
        serde_json::to_value(magic.effect.unwrap()).unwrap(),
        serde_json::json!({ "effect": "magicMove", "params": { "fadeUnmatched": true } })
    );
    let sparkle = keynote::transition("com.apple.iWork.Keynote.KLNSparkle").unwrap();
    assert_eq!(sparkle.effect, None);
    assert_eq!(
        keynote::effect_id("apple:move in").to_string(),
        "keynote:apple:move in"
    );
}

#[test]
fn keynote_directions_decode_as_keynotes_files_pin_them() {
    use keynote::KeynoteDirection::{Motion, Rotation, Zoom};
    use keynote::{Pinned, Steered, direction_certainty};
    use slidock_ir::effects::{Dir, InOut};
    assert_eq!(
        keynote::direction(31),
        Some((Rotation { cw: true }, Pinned::Everywhere))
    );
    assert_eq!(
        keynote::direction(32),
        Some((Rotation { cw: false }, Pinned::Everywhere))
    );
    assert_eq!(
        keynote::direction(11),
        Some((Motion(Dir::Right), Pinned::Everywhere))
    );
    assert_eq!(
        keynote::direction(14),
        Some((Motion(Dir::Up), Pinned::Everywhere))
    );
    assert_eq!(
        keynote::direction(24),
        Some((Motion(Dir::LeftUp), Pinned::Everywhere))
    );
    assert_eq!(
        keynote::direction(42),
        Some((Zoom(InOut::Out), Pinned::OnIris))
    );
    assert_eq!(
        keynote::direction(43),
        Some((Zoom(InOut::In), Pinned::OnZoom))
    );
    // 41/42 on a zoom and 43/44 on an iris stay inferred.
    assert_eq!(
        direction_certainty(21, Steered::Motion),
        Certainty::Confirmed
    );
    assert_eq!(direction_certainty(42, Steered::Iris), Certainty::Confirmed);
    assert_eq!(direction_certainty(42, Steered::Zoom), Certainty::Inferred);
    assert_eq!(direction_certainty(44, Steered::Zoom), Certainty::Confirmed);
    assert_eq!(direction_certainty(43, Steered::Iris), Certainty::Inferred);
    assert_eq!(direction_certainty(7, Steered::Motion), Certainty::Inferred);
    assert_eq!(keynote::direction(0), None);
    assert_eq!(keynote::direction(15), None);
    assert_eq!(keynote::direction(45), None);
    for number in keynote::DIRECTIONS {
        assert!(keynote::direction(number).is_some(), "{number}");
    }
    assert_eq!(
        (0..100)
            .filter(|n| keynote::direction(*n).is_some())
            .count(),
        keynote::DIRECTIONS.len()
    );
}

/// Keynote's own output settles the transition rows it shows: the wipe with no direction moves right with a
/// feathered edge, and its move in (`apple:slide`) is a cover.
#[test]
fn keynote_rows_its_own_output_settles_are_confirmed() {
    use serde_json::json;
    let wipe = keynote::transition("apple:wipe").unwrap();
    assert_eq!(wipe.certainty, Certainty::Confirmed);
    assert!(!wipe.approximate);
    assert_eq!(
        serde_json::to_value(wipe.effect.unwrap()).unwrap(),
        json!({ "effect": "wipe", "params": { "dir": "r", "soft": true } })
    );
    let slide = keynote::transition("apple:slide").unwrap();
    assert_eq!(slide.certainty, Certainty::Confirmed);
    assert!(!slide.approximate);
    assert_eq!(
        serde_json::to_value(slide.effect.unwrap()).unwrap(),
        json!({ "effect": "cover", "params": { "dir": "l" } })
    );
    // The drive-by stays an inferred approximation.
    let drive = keynote::transition("apple:drive-by").unwrap();
    assert_eq!(drive.certainty, Certainty::Inferred);
    assert!(drive.approximate);
}

/// A Keynote row carries its own curve (what plays when the transition stores none) only where the movie of the
/// transition catalog measured it, and the ids sharing that row: ease-in-out for the dissolves, the fades through a
/// colour and the pushes; a fitted curve for the move in and the reveal. Every other row leaves it to the player.
#[test]
fn keynote_rows_carry_only_the_curves_its_movie_measured() {
    use serde_json::json;
    let own: Vec<(&str, serde_json::Value)> = keynote::IDS
        .iter()
        .filter_map(|id| Some((*id, keynote::transition(id)?.own_easing?)))
        .map(|(id, easing)| (id, serde_json::to_value(easing).unwrap()))
        .collect();
    let in_out = json!("in-out");
    assert_eq!(
        own,
        [
            ("apple:bc-dissolve", in_out.clone()),
            ("apple:c-dissolve", in_out.clone()),
            ("apple:ca-push", in_out.clone()),
            ("apple:dissolve", in_out.clone()),
            ("apple:fade", in_out.clone()),
            ("apple:gallery-dissolve", in_out.clone()),
            ("apple:motion dissolve", in_out.clone()),
            ("apple:push", in_out.clone()),
            ("apple:reveal", json!({ "bezier": [0.48, 0.0, 0.56, 0.32] })),
            ("apple:slide", json!({ "bezier": [0.62, 0.92, 0.62, 0.98] })),
            ("com.apple.iWork.Keynote.BLTFadeThruColor", in_out),
        ]
    );
}

/// Keynote's build movies (of its import of the PPTX probe) settle the build rows they show: the wipe's feathered
/// edge, the iris as a circle whatever the PPTX asked for (its feathered edge has no slot in the vocabulary, so it
/// approximates), the zoom fading as it scales, and an exit's scale direction naming what the element does.
#[test]
fn keynote_build_rows_its_movies_settle_are_confirmed() {
    use keynote::KeynoteDirection::{Motion, Rotation, Zoom};
    use serde_json::json;
    use slidock_ir::effects::{Dir, InOut};
    for kind in [StepKind::Entrance, StepKind::Exit] {
        let wipe = keynote::build("apple:wipe", kind).unwrap();
        assert_eq!(wipe.certainty, Certainty::Confirmed);
        assert!(!wipe.approximate);
        assert_eq!(
            build_json(&wipe),
            json!({ "effect": "wipe", "params": { "dir": "r", "soft": true } })
        );
        let iris = keynote::build("apple:wipe-iris", kind).unwrap();
        assert_eq!(iris.certainty, Certainty::Confirmed);
        assert!(iris.approximate);
        assert_eq!(build_json(&iris)["params"]["shape"], json!("circle"));
        let zoom = keynote::build("apple:zoom", kind).unwrap();
        assert_eq!(zoom.certainty, Certainty::Confirmed);
        assert!(!zoom.approximate);
        assert_eq!(build_json(&zoom)["params"]["fade"], json!(true));
    }
    // The scale builds no movie shows stay as inferred.
    let scale = keynote::build("apple:scale", StepKind::Entrance).unwrap();
    assert_eq!(scale.certainty, Certainty::Inferred);
    assert_eq!(build_json(&scale)["params"]["fade"], json!(false));
    for (number, exit) in [
        (41, Zoom(InOut::Out)),
        (42, Zoom(InOut::In)),
        (43, Zoom(InOut::Out)),
        (44, Zoom(InOut::In)),
        (12, Motion(Dir::Left)),
        (32, Rotation { cw: false }),
    ] {
        let (entrance, _) = keynote::direction(number).unwrap();
        assert_eq!(entrance.on_exit(), exit, "{number}");
        assert_eq!(exit.on_exit(), entrance, "{number}");
    }
}

#[test]
fn the_libreoffice_data_keeps_its_licence() {
    for file in [
        include_str!("../src/libreoffice/effects.rs"),
        include_str!("../src/libreoffice/transitions.rs"),
        include_str!("../src/libreoffice/pptx.rs"),
        include_str!("../src/libreoffice/migration.rs"),
    ] {
        assert!(file.starts_with("// * This file is part of the LibreOffice project."));
        assert!(file.contains("Mozilla Public\n// * License, v. 2.0"));
        assert!(file.contains(slidock_presets::LIBREOFFICE_COMMIT));
    }
    let notice = include_str!("../../../NOTICE");
    assert!(notice.contains("Mozilla Public License 2.0"));
    assert!(notice.contains("crates/slidock-presets/src/libreoffice/"));
    assert!(notice.contains("`pptx.rs`") && notice.contains("commontimenodecontext.cxx"));
    assert!(notice.contains("`migration.rs`") && notice.contains("EffectMigration.cxx"));
    assert!(notice.contains(slidock_presets::LIBREOFFICE_COMMIT));
    // The MIT mapping reads LibreOffice's PowerPoint tables; it holds no copy of them.
    let mapping = include_str!("../src/pptx.rs");
    assert!(!mapping.contains("\"from-") && !mapping.contains("\"ooo-"));
}

/// LibreOffice's sub-type conversion, with its per-preset exceptions: Stretch's 10 is `across`, the checkerboard's
/// 5 and 10 are `downward` and `across`, the wheel keeps its number of spokes, and an emphasis keeps its number.
#[test]
fn pptx_subtypes_convert_as_libreoffice_does() {
    use pptx::PptxClass::{Emphasis, Entrance, Exit};
    let token = |class, id, subtype| pptx::subtype_token(class, id, subtype);
    assert_eq!(token(Entrance, 2, 8).as_deref(), Some("from-left"));
    assert_eq!(token(Exit, 2, 9).as_deref(), Some("from-top-left"));
    assert_eq!(token(Entrance, 17, 10).as_deref(), Some("across"));
    assert_eq!(token(Exit, 17, 10).as_deref(), Some("across"));
    assert_eq!(token(Entrance, 16, 10).as_deref(), Some("vertical"));
    assert_eq!(token(Entrance, 5, 5).as_deref(), Some("downward"));
    assert_eq!(token(Entrance, 18, 6).as_deref(), Some("right-to-bottom"));
    assert_eq!(token(Entrance, 21, 8).as_deref(), Some("8"));
    assert_eq!(token(Emphasis, 6, 8).as_deref(), Some("8"));
    assert_eq!(token(Entrance, 2, 7).as_deref(), Some("7"));
    assert_eq!(token(Entrance, 2, 0), None);
    // Stretch across stretches horizontally.
    assert_eq!(preset_json("entr", 17, 10)["params"]["axis"], "horz");
}

fn build_json(row: &BuildRow) -> serde_json::Value {
    serde_json::to_value(row.build_effect().unwrap()).unwrap()
}

/// An exit `E` plays as `reverse(entrance E(opposite(dir)))`, its other parameters unchanged. So
/// a zoom exit names the entrance it reverses: LibreOffice's exit `out` shrinks the element to nothing
/// (`width;0`), the zoom-in entrance (`0;width`) played backwards. Iris and split exits are the entrance filters
/// with `smil:mode="out"`, which LibreOffice's engine plays as the entrance of the same name backwards: exit box `in`
/// opens a hole from the centre, the `in` entrance reversed.
#[test]
fn exits_reverse_the_entrance_they_name() {
    use serde_json::json;
    let odp_exit = |id: &str, subtype: Option<&str>| {
        build_json(&odp::build("exit", id, subtype).unwrap().row)["params"].clone()
    };
    for (subtype, in_out, origin, slight) in [
        ("out", "in", "object", false),
        ("out-slightly", "in", "object", true),
        ("544", "in", "slideCenter", false),
        ("in", "out", "object", false),
        ("in-slightly", "out", "object", true),
        ("20", "out", "slideCenter", false),
    ] {
        let params = odp_exit("ooo-exit-zoom", Some(subtype));
        assert_eq!(params["inOut"], json!(in_out), "{subtype}");
        assert_eq!(params["origin"], json!(origin), "{subtype}");
        assert_eq!(params["slight"], json!(slight), "{subtype}");
    }
    // The entrances read as named.
    let entrance = build_json(
        &odp::build("entrance", "ooo-entrance-zoom", Some("out"))
            .unwrap()
            .row,
    );
    assert_eq!(entrance["params"]["inOut"], json!("out"));
    // Fade out and zoom shrinks away: the fade-in-and-zoom entrance (from small) reversed.
    assert_eq!(
        odp_exit("ooo-exit-fade-out-and-zoom", None),
        build_json(
            &odp::build("entrance", "ooo-entrance-fade-in-and-zoom", None)
                .unwrap()
                .row
        )["params"]
    );
    assert_eq!(
        odp_exit("ooo-exit-fade-out-and-zoom", None)["inOut"],
        json!("in")
    );
    // PowerPoint's exit Zoom subtype 32 is LibreOffice's exit `out`.
    assert_eq!(preset_json("exit", 23, 32)["params"]["inOut"], json!("in"));
    assert_eq!(preset_json("entr", 23, 32)["params"]["inOut"], json!("out"));
    // Iris and split exits, and their entrances, name the entrance their filter plays.
    for (id, subtype, in_out) in [
        ("ooo-exit-box", Some("in"), "in"),
        ("ooo-exit-box", Some("out"), "out"),
        ("ooo-exit-box", None, "in"),
        ("ooo-exit-circle", Some("in"), "in"),
        ("ooo-exit-diamond", Some("out"), "out"),
        ("ooo-exit-plus", Some("in"), "in"),
        ("ooo-exit-split", Some("horizontal-in"), "in"),
        ("ooo-exit-split", Some("horizontal-out"), "out"),
        ("ooo-exit-split", Some("vertical-in"), "in"),
        ("ooo-exit-split", Some("vertical-out"), "out"),
        ("ooo-exit-split", None, "out"),
    ] {
        assert_eq!(
            odp_exit(id, subtype)["inOut"],
            json!(in_out),
            "{id} {subtype:?}"
        );
    }
    let odp_entrance = |id: &str, subtype: Option<&str>| {
        build_json(&odp::build("entrance", id, subtype).unwrap().row)["params"]["inOut"].clone()
    };
    assert_eq!(odp_entrance("ooo-entrance-box", Some("in")), json!("in"));
    assert_eq!(
        odp_entrance("ooo-entrance-split", Some("vertical-out")),
        json!("out")
    );
    // PowerPoint's exit Box In (subtype 16) and Split Horizontal In (26) are LibreOffice's exit `in`, the filter
    // `box(in)` and `barn(inHorizontal)` played out.
    assert_eq!(preset_json("exit", 4, 16)["params"]["inOut"], json!("in"));
    assert_eq!(preset_json("entr", 4, 16)["params"]["inOut"], json!("in"));
    assert_eq!(preset_json("exit", 16, 26)["params"]["inOut"], json!("in"));
    assert_eq!(preset_json("exit", 16, 42)["params"]["inOut"], json!("out"));
}

/// An axis is the dimension the source app animates (`effects.xml`'s animated attribute), never a sub-type's word:
/// swivel `vertical` animates the width (`width*sin(…)`), so it is horizontal; compress animates the width
/// (`width+.3` to `width`); stretchy `downward` the height.
#[test]
fn axes_are_the_dimension_libreoffice_animates() {
    use serde_json::json;
    let axis = |class: &str, id: &str, subtype: Option<&str>| {
        build_json(&odp::build(class, id, subtype).unwrap().row)["params"]["axis"].clone()
    };
    for (class, id, subtype, want) in [
        ("entrance", "ooo-entrance-swivel", Some("vertical"), "horz"),
        (
            "entrance",
            "ooo-entrance-swivel",
            Some("horizontal"),
            "vert",
        ),
        ("exit", "ooo-exit-swivel", Some("vertical"), "horz"),
        ("entrance", "ooo-entrance-compress", None, "horz"),
        ("entrance", "ooo-entrance-expand", None, "horz"),
        ("entrance", "ooo-entrance-stretchy", Some("across"), "horz"),
        (
            "entrance",
            "ooo-entrance-stretchy",
            Some("downward"),
            "vert",
        ),
        ("exit", "ooo-exit-collapse", Some("across"), "horz"),
        ("entrance", "ooo-entrance-fade-in-and-swivel", None, "horz"),
    ] {
        assert_eq!(axis(class, id, subtype), json!(want), "{id} {subtype:?}");
    }
    // PowerPoint's swivel 10 (LibreOffice `vertical`) and Compress read the same.
    assert_eq!(preset_json("entr", 19, 10)["params"]["axis"], json!("horz"));
    assert_eq!(preset_json("entr", 50, 0)["params"]["axis"], json!("horz"));
}

/// A preset maps to a vocabulary entry exactly only where the entry plays its motion; otherwise the row is
/// approximate (and a source that carries the preset's behaviours plays those).
#[test]
fn rows_whose_entry_plays_another_motion_are_approximate() {
    let approximate = |class: &str, id: &str, subtype: Option<&str>| {
        odp::build(class, id, subtype).unwrap().row.approximate
    };
    for (class, id, subtype) in [
        ("entrance", "ooo-entrance-stretchy", Some("from-left")),
        ("entrance", "ooo-entrance-stretchy", Some("from-top-right")),
        ("exit", "ooo-exit-collapse", Some("from-bottom")),
        ("entrance", "ooo-entrance-magnify", None),
        (
            "entrance",
            "ooo-entrance-zoom",
            Some("out-from-screen-center"),
        ),
        ("exit", "ooo-exit-zoom", Some("20")),
        ("entrance", "ooo-entrance-curve-up", None),
        ("exit", "ooo-exit-curve-down", None),
        ("entrance", "ooo-entrance-ease-in", None),
        ("emphasis", "ooo-emphasis-blast", None),
    ] {
        assert!(approximate(class, id, subtype), "{id} {subtype:?}");
    }
    for (class, id, subtype) in [
        ("entrance", "ooo-entrance-stretchy", Some("across")),
        (
            "entrance",
            "ooo-entrance-zoom",
            Some("in-from-screen-center"),
        ),
        ("entrance", "ooo-entrance-zoom", Some("in-slightly")),
        ("exit", "ooo-exit-zoom", Some("544")),
        ("exit", "ooo-exit-zoom", Some("out")),
        ("entrance", "ooo-entrance-compress", None),
    ] {
        assert!(!approximate(class, id, subtype), "{id} {subtype:?}");
    }
    assert!(pptx::build("entr", 23, 36).unwrap().row.approximate);
    assert!(pptx::build("exit", 23, 20).unwrap().row.approximate);
    assert!(!pptx::build("exit", 23, 544).unwrap().row.approximate);
}

/// `soft` follows the renderer of the source: PowerPoint feathers its wipes, slide transition and build alike;
/// LibreOffice's `barWipe` (ODP builds and transitions share it) is hard, and Keynote's wipe is
/// feathered, transition and build alike, as its movies play it.
#[test]
fn a_wipe_is_soft_where_its_renderer_feathers_it() {
    use serde_json::json;
    let soft = |effect: serde_json::Value| effect["params"]["soft"].clone();
    assert_eq!(
        soft(serde_json::to_value(pptx_effect("wipe", &[("dir", "r")]).unwrap()).unwrap()),
        json!(true)
    );
    for class in ["entr", "exit"] {
        for subtype in [1, 2, 4, 8] {
            assert_eq!(
                soft(preset_json(class, 22, subtype)),
                json!(true),
                "{class} {subtype}"
            );
        }
    }
    let odp_wipe = odp::transition("barWipe", Some("leftToRight"), false, None).unwrap();
    assert_eq!(
        soft(serde_json::to_value(odp_wipe.row.effect.unwrap()).unwrap()),
        json!(false)
    );
    for class in ["entrance", "exit"] {
        let id = format!("ooo-{class}-wipe");
        let row = odp::build(class, &id, Some("from-left")).unwrap().row;
        assert_eq!(soft(build_json(&row)), json!(false), "{class}");
    }
    let keynote = keynote::transition("apple:wipe").unwrap();
    assert_eq!(
        soft(serde_json::to_value(keynote.effect.unwrap()).unwrap()),
        json!(true)
    );
    for kind in [StepKind::Entrance, StepKind::Exit] {
        let row = keynote::build("apple:wipe", kind).unwrap();
        assert_eq!(soft(build_json(&row)), json!(true), "{kind:?}");
    }
}

/// The same LibreOffice-converted effect reads the same from PPTX and ODP where LibreOffice's import pins what
/// OOXML's token means (the vertical axis, the corners, the wipe); `l` and `r` on push, cover and pull, which its
/// import reads the other way (`push dir="l"` becomes `pushWipe fromLeft`, a push to the right), are confirmed by
/// Keynote's own export and movie instead.
#[test]
fn pptx_directions_are_confirmed_where_libreoffice_pins_them() {
    let pptx_row = |element: &str, dir: &'static str| {
        pptx::transition(element, |name| (name == "dir").then_some(dir))
            .unwrap()
            .row
    };
    let odp_effect = |smil: &str, subtype: &str, reverse: bool| {
        odp::transition(smil, Some(subtype), reverse, None)
            .unwrap()
            .row
            .effect
            .unwrap()
    };
    // LibreOffice's import: `push d` → pushWipe fromTop, `push u` → fromBottom, `cover lu` → slideWipe
    // fromBottomRight, `pull rd` → slideWipe fromTopLeft reversed, `wipe l` → barWipe leftToRight reversed.
    for (element, dir, smil, subtype, reverse) in [
        ("push", "d", "pushWipe", "fromTop", false),
        ("push", "u", "pushWipe", "fromBottom", false),
        ("cover", "lu", "slideWipe", "fromBottomRight", false),
        ("pull", "rd", "slideWipe", "fromTopLeft", true),
        ("wipe", "l", "barWipe", "leftToRight", true),
        ("wipe", "d", "barWipe", "topToBottom", false),
    ] {
        let row = pptx_row(element, dir);
        let pptx = serde_json::to_value(row.effect.unwrap()).unwrap();
        let odp = serde_json::to_value(odp_effect(smil, subtype, reverse)).unwrap();
        assert_eq!(pptx["effect"], odp["effect"], "{element} {dir}");
        assert_eq!(
            pptx["params"]["dir"], odp["params"]["dir"],
            "{element} {dir}"
        );
        assert_eq!(row.certainty, Certainty::Confirmed, "{element} {dir}");
    }
    // `push l` → pushWipe fromLeft in LibreOffice: a push to the right there, to the left here.
    let push = pptx_row("push", "l");
    assert_eq!(
        serde_json::to_value(push.effect.unwrap()).unwrap()["params"]["dir"],
        "l"
    );
    assert_eq!(
        serde_json::to_value(odp_effect("pushWipe", "fromLeft", false)).unwrap()["params"]["dir"],
        "r"
    );
    // Keynote's PPTX export writes its default Push, Move In and Reveal (content moving left in its movie export) as
    // `dir="l"`: the horizontal axis is the content's motion too.
    for (element, dir) in [("push", "l"), ("push", "r"), ("cover", "r"), ("pull", "l")] {
        assert_eq!(
            pptx_row(element, dir).certainty,
            Certainty::Confirmed,
            "{element} {dir}"
        );
    }
    // The Office 2010 effects have no LibreOffice import.
    for element in [
        "reveal", "flip", "switch", "gallery", "conveyor", "ferris", "pan",
    ] {
        assert_eq!(
            pptx_row(element, "r").certainty,
            Certainty::Inferred,
            "{element}"
        );
    }
}

/// A row's canonical fallback is the vocabulary's, not the IR's: where PPTX and Keynote print their own fallback
/// (dissolve for anything but fade, dissolve, push and wipe), the IR keeps it.
#[test]
fn canonical_fallbacks_are_the_vocabularys() {
    use slidock_ir::effects::TransitionFallback;
    for element in ["cover", "pull", "split", "circle", "diamond"] {
        let row = pptx::transition(element, |_| None).unwrap().row;
        assert_ne!(
            row.canonical_fallback,
            TransitionFallback::Dissolve,
            "{element}"
        );
    }
    let iris = keynote::transition("apple:wipe-iris").unwrap();
    assert_eq!(iris.canonical_fallback, TransitionFallback::Wipe);
}
