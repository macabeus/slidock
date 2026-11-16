//! Keynote motion on synthetic presentations: effect ids and attribute fields read onto the effect vocabulary,
//! direction numbers, timing (with and after builds, easing, repeats), motion paths, Magic Move keys, the motion
//! diagnostics (KN1404–KN1409) and hostile timing data. The steps' and transitions' IR fields stay the parity path's.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde_json::{Value, json};
use slidock_ir::effects::{limits, strip_native_motion};
use slidock_ir::ids::SlideId;
use support::harness::{Lowered, lower, lower_native, lower_native_reader};
use support::keynote::{Presentation, geometry, id_of, merge, rgb};

/// A value as the lowered JSON reads it (canonical numbers).
fn canonical<T: serde::Serialize>(value: &T) -> Value {
    let text = slidock_canon::to_canonical_json(value, slidock_canon::Layout::Compact).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn kn(reference: &Value) -> String {
    format!("kn-{}", id_of(reference))
}

fn shape_at(p: &mut Presentation, x: f64, w: f64) -> Value {
    p.add(
        "TSD.ShapeArchive",
        json!({ "super": { "geometry": geometry(x, 0.0, w, w) },
                "pathsource": { "scalar_path_source": { "type": 0, "scalar": 0, "naturalSize": { "width": w, "height": w } } } }),
    )
}

fn text_at(p: &mut Presentation, text: &str, x: f64) -> Value {
    let storage = p.storage(text, None, json!({}));
    p.text_box(storage, geometry(x, 0.0, 300.0, 100.0), None)
}

/// A slide whose only transition is `attributes` (a `TransitionAttributesArchive`).
fn transition_slide(p: &mut Presentation, drawables: Vec<Value>, attributes: Value) -> Value {
    p.slide(
        json!({ "drawables_z_order": drawables, "transition": { "attributes": attributes } }),
        json!({}),
        None,
    )
}

fn animation(effect: &str, extra: Value) -> Value {
    merge(
        json!({ "animation_type": "Transition", "effect": effect, "duration": 1.0 }),
        extra,
    )
}

/// One build of `drawable` with `animation` attributes and `attributes` beside them, in one chunk.
fn build(p: &mut Presentation, drawable: &Value, animation: Value, attributes: Value) -> Value {
    p.add(
        "KN.BuildArchive",
        json!({ "drawable": drawable, "delivery": "All at Once",
                "attributes": merge(json!({ "animationAttributes": animation }), attributes) }),
    )
}

fn chunk(p: &mut Presentation, build: &Value, chunk: Value) -> Value {
    p.add(
        "KN.BuildChunkArchive",
        merge(json!({ "build": build, "duration": 1.0 }), chunk),
    )
}

fn built(effect: &str, kind: &str) -> Value {
    json!({ "animation_type": kind, "effect": effect, "duration": 1.0 })
}

/// A slide with one shape built by `animation`/`attributes`, lowered by the motion frontend: its step.
fn one_build(animation: Value, attributes: Value) -> (Value, Lowered) {
    let mut p = Presentation::new();
    let shape = shape_at(&mut p, 0.0, 10.0);
    let b = build(&mut p, &shape, animation, attributes);
    let c = chunk(&mut p, &b, json!({}));
    p.slide(
        json!({ "drawables_z_order": [shape], "builds": [b], "buildChunks": [c] }),
        json!({}),
        None,
    );
    let lowered = lower_native(&p.bytes());
    (lowered.json["slides"][0]["timeline"][0].clone(), lowered)
}

/// A slide whose transition is `attributes`, then a slide: the transition into the second (the deck path plays
/// Keynote's transition into the slide after the one that stores it).
fn one_transition(attributes: Value) -> (Value, Lowered) {
    let mut p = Presentation::new();
    transition_slide(&mut p, vec![], attributes);
    p.slide(json!({}), json!({}), None);
    let lowered = lower_native(&p.bytes());
    assert!(lowered.json["slides"][0].get("transition").is_none());
    (lowered.json["slides"][1]["transition"].clone(), lowered)
}

/// The messages of `code`, sorted (the report orders a container's diagnostics by its own key).
fn sorted_messages(lowered: &Lowered, code: &str) -> Vec<String> {
    let mut out = messages(lowered, code);
    out.sort();
    out
}

fn messages(lowered: &Lowered, code: &str) -> Vec<String> {
    lowered
        .diagnostics
        .iter()
        .filter(|d| d.code.code() == code)
        .map(|d| d.message.clone())
        .collect()
}

#[test]
fn transitions_read_their_direction_colour_and_options() {
    let (push, lowered) = one_transition(json!({
        "animationAttributes": animation("apple:push", json!({ "direction": 12 })),
    }));
    assert_eq!(
        push,
        json!({ "effect": { "id": "keynote:apple:push", "fallback": "push",
                "native": { "id": "keynote:apple:push", "effect": "push", "params": { "dir": "l" } } },
                "durationMs": 1000, "easing": "in-out" })
    );
    // Keynote's own import of the PPTX motion probe pins 12 (`push dir="l"`): nothing to report.
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.codes());
    let (down, _) = one_transition(json!({
        "animationAttributes": animation("apple:move in", json!({ "direction": 13 })),
    }));
    assert_eq!(
        down["effect"]["native"],
        json!({ "id": "keynote:apple:move in", "effect": "cover", "params": { "dir": "d" } })
    );
    let (legacy, _) = one_transition(json!({
        "animationAttributes": animation("apple:reveal", json!({})), "database_direction": 23,
    }));
    assert_eq!(legacy["effect"]["native"]["params"], json!({ "dir": "ru" }));
    let (orange, _) = one_transition(json!({
        "animationAttributes": animation("apple:fade", json!({ "color": rgb(1.0, 0.5, 0.0) })),
    }));
    assert_eq!(
        orange["effect"]["native"]["params"],
        json!({ "through": "color", "color": "#ff8000" })
    );
    let (white, _) = one_transition(json!({
        "animationAttributes": animation("apple:fade", json!({ "color": rgb(1.0, 1.0, 1.0) })),
    }));
    assert_eq!(
        white["effect"]["native"]["params"],
        json!({ "through": "white" })
    );
    // Only a fade through a colour reads one.
    let (dissolve, _) = one_transition(json!({
        "animationAttributes": animation("apple:dissolve", json!({ "color": rgb(1.0, 0.5, 0.0) })),
    }));
    assert_eq!(
        dissolve["effect"]["native"]["params"],
        json!({ "through": "none" })
    );
    let (magic, lowered) = one_transition(json!({
        "animationAttributes": animation("apple:magic-move-implied-motion-path", json!({})),
        "custom_magic_move_fade_unmatched_objects": false, "custom_timing_curve": 4,
        "custom_text_delivery_type": 3,
    }));
    assert_eq!(
        magic["effect"]["native"],
        json!({ "id": "keynote:apple:magic-move-implied-motion-path", "effect": "magicMove",
                "params": { "fadeUnmatched": false } })
    );
    assert_eq!(magic["easing"], json!("in-out"));
    assert_eq!(
        messages(&lowered, "KN1404"),
        [
            "The apple:magic-move-implied-motion-path effect plays as an approximation: text matched by character morphs as whole objects"
        ]
    );
}

/// Keynote's move in (`apple:slide`) plays the cover its movie shows (the outgoing slide stays still) and its PPTX
/// export writes, steered by its direction like any cover, with no approximation reported; a push stays a push.
#[test]
fn move_in_plays_a_cover() {
    for (number, params) in [
        (0, json!({ "dir": "l" })),
        (11, json!({ "dir": "r" })),
        (23, json!({ "dir": "ru" })),
    ] {
        let (transition, lowered) = one_transition(json!({
            "animationAttributes": animation("apple:slide", json!({ "direction": number })),
        }));
        assert_eq!(
            transition["effect"]["native"],
            json!({ "id": "keynote:apple:slide", "effect": "cover", "params": params }),
            "{number}"
        );
        // Only the IR's fallback reading: the runtime without the vocabulary plays a dissolve.
        assert_eq!(lowered.codes(), ["KN1403"]);
    }
    let (push, _) =
        one_transition(json!({ "animationAttributes": animation("apple:push", json!({})) }));
    assert_eq!(push["effect"]["native"]["effect"], json!("push"));
}

/// A build wipe plays the feathered edge Keynote's build movies show, as its transition wipe does, unreported. With
/// no direction number it plays the row's default, moving right, unreported as every directional build's default is.
#[test]
fn a_build_wipe_plays_soft_as_keynotes_movies_show() {
    let (step, lowered) = one_build(built("apple:wipe", "In"), json!({}));
    assert_eq!(step["effect"]["native"]["params"]["soft"], json!(true));
    assert_eq!(step["effect"]["native"]["params"]["dir"], json!("r"));
    // Only the IR's fallback reading: no approximation.
    assert_eq!(lowered.codes(), ["KN1402"]);
    let (_, lowered) =
        one_transition(json!({ "animationAttributes": animation("apple:wipe", json!({})) }));
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.codes());
}

#[test]
fn unknown_and_unfitting_directions_keep_the_default() {
    for (effect, number, default) in [
        ("apple:push", 99, json!({ "dir": "l" })),
        ("apple:push", 21, json!({ "dir": "l" })),
        ("apple:wipe", 31, json!({ "dir": "r", "soft": true })),
    ] {
        let (transition, lowered) = one_transition(json!({
            "animationAttributes": animation(effect, json!({ "direction": number })),
        }));
        assert_eq!(
            transition["effect"]["native"]["params"], default,
            "{effect} {number}"
        );
        assert_eq!(
            messages(&lowered, "KN1405"),
            [format!(
                "The direction {number} of the {effect} effect is unknown; it plays its default direction"
            )]
        );
    }
    let (rtl, lowered) = one_transition(json!({
        "animationAttributes": animation("apple:wipe", json!({ "direction": 11, "writing_direction_is_rtl": true })),
    }));
    assert_eq!(rtl["effect"]["native"]["params"]["dir"], json!("l"));
    assert_eq!(messages(&lowered, "KN1404").len(), 1);
    let (none, lowered) = one_transition(json!({
        "animationAttributes": animation("apple:push", json!({ "direction": 0 })),
    }));
    assert_eq!(none["effect"]["native"]["params"], json!({ "dir": "l" }));
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.codes());
}

#[test]
fn timing_curves_map_to_easings_and_custom_ones_to_beziers() {
    let curve = |value: Value, extra: Value| {
        let attributes = merge(
            json!({ "animationAttributes": animation("apple:dissolve", extra), "custom_timing_curve": value }),
            json!({}),
        );
        one_transition(attributes)
    };
    for (value, easing) in [(1, "linear"), (2, "in"), (3, "out"), (4, "in-out")] {
        assert_eq!(curve(json!(value), json!({})).0["easing"], json!(easing));
    }
    let bezier = json!({ "custom_effect_timing_curve_1": { "bezier_path_source": { "path": { "elements": [
        { "type": 1, "points": [{ "x": 0, "y": 0 }] },
        { "type": 4, "points": [{ "x": 25, "y": 10 }, { "x": 25, "y": 100 }, { "x": 100, "y": 100 }] },
    ] } } } });
    let (custom, lowered) = curve(json!(5), bezier);
    assert_eq!(custom["easing"], json!({ "bezier": [0.25, 0.1, 0.25, 1] }));
    assert!(lowered.diagnostics.is_empty());
    let (unreadable, lowered) = curve(json!(5), json!({}));
    assert_eq!(unreadable["easing"], json!("in-out"));
    assert_eq!(
        messages(&lowered, "KN1408"),
        ["The timing curve of the apple:dissolve effect plays as in-out"]
    );
    let (unknown, lowered) = curve(json!(9), json!({}));
    assert_eq!(unknown["easing"], json!("linear"));
    assert_eq!(messages(&lowered, "KN1408").len(), 1);
    // A dissolve that stores no curve plays its own, ease-in-out (an_unset_timing_curve_is_the_effects_own).
    let (unset, lowered) = curve(Value::Null, json!({}));
    assert_eq!(unset["easing"], json!("in-out"));
    assert!(messages(&lowered, "KN1408").is_empty());
}

/// A transition that stores no `custom_timing_curve` plays its effect's own curve, as Keynote's movie of the
/// transition catalog plays it: ease-in-out for a dissolve, a fade through a colour and a push, the move in's and the
/// reveal's fitted curves, and none (the player's default) where the movie measured none. A stored curve still wins.
#[test]
fn an_unset_timing_curve_is_the_effects_own() {
    for (effect, easing) in [
        ("apple:dissolve", json!("in-out")),
        ("com.apple.iWork.Keynote.BLTFadeThruColor", json!("in-out")),
        ("apple:push", json!("in-out")),
        ("apple:slide", json!({ "bezier": [0.62, 0.92, 0.62, 0.98] })),
        ("apple:reveal", json!({ "bezier": [0.48, 0, 0.56, 0.32] })),
    ] {
        let (transition, lowered) =
            one_transition(json!({ "animationAttributes": animation(effect, json!({})) }));
        assert_eq!(transition["easing"], easing, "{effect}");
        assert!(messages(&lowered, "KN1408").is_empty(), "{effect}");
    }
    for effect in ["apple:wipe-iris", "apple:move in", "apple:flash"] {
        let (transition, lowered) =
            one_transition(json!({ "animationAttributes": animation(effect, json!({})) }));
        assert!(!transition["effect"]["native"].is_null(), "{effect}");
        assert!(transition.get("easing").is_none(), "{effect}: {transition}");
        assert!(messages(&lowered, "KN1408").is_empty(), "{effect}");
    }
    let (stored, _) = one_transition(json!({
        "animationAttributes": animation("apple:reveal", json!({})), "custom_timing_curve": 1,
    }));
    assert_eq!(stored["easing"], json!("linear"));
}

#[test]
fn builds_read_direction_actions_and_options() {
    let (fly, lowered) = one_build(
        json!({ "animation_type": "In", "effect": "apple:move in", "duration": 1.0, "direction": 11 }),
        json!({ "action_acceleration": 2 }),
    );
    assert_eq!(
        fly["effect"],
        json!({ "id": "keynote:apple:move_in", "fallback": "dissolve",
                "native": { "id": "keynote:apple:move in", "effect": "flyIn", "params": { "dir": "r", "distance": "offSlide" } } })
    );
    assert_eq!(fly["timing"], json!({ "offsetMs": 0, "easing": "out" }));
    assert!(messages(&lowered, "KN1404").is_empty());
    let (rotation, lowered) = one_build(
        built("apple:action-rotation", "Action"),
        json!({ "action_rotationAngle": 90, "action_rotationDirection": 32 }),
    );
    assert_eq!(rotation["kind"], json!("emphasis"));
    assert_eq!(
        rotation["effect"]["native"]["params"],
        json!({ "deg": 90, "cw": false, "hold": true })
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .all(|d| d.code.code() == "KN1402")
    );
    let (scale, _) = one_build(
        built("apple:action-scale", "Action"),
        json!({ "action_scaleSize": 2.5 }),
    );
    assert_eq!(
        scale["effect"]["native"]["params"],
        json!({ "scale": { "x": 2.5, "y": 2.5 }, "hold": true })
    );
    let (opacity, _) = one_build(
        built("apple:action-opacity", "Action"),
        json!({ "action_colorAlpha": 0.3 }),
    );
    assert_eq!(
        opacity["effect"]["native"]["params"],
        json!({ "alpha": 0.3, "hold": true })
    );
    let (jiggle, _) = one_build(
        built("apple:action-jiggle", "Action"),
        json!({ "custom_action_jiggleIntensity": 2, "custom_action_repeatCount": 3 }),
    );
    assert_eq!(
        jiggle["effect"]["native"]["params"],
        json!({ "intensity": 1 })
    );
    assert_eq!(jiggle["timing"]["repeat"], json!({ "count": 3 }));
    let (zoom, _) = one_build(built("apple:zoom", "In"), json!({ "custom_bounce": true }));
    assert_eq!(zoom["effect"]["native"]["params"]["overshoot"], json!(true));
    let (bounce, lowered) = one_build(
        built("apple:move in", "In"),
        json!({ "custom_bounce": true }),
    );
    assert_eq!(bounce["effect"]["native"]["effect"], json!("flyIn"));
    assert_eq!(
        messages(&lowered, "KN1404"),
        ["The apple:move in effect plays as an approximation: the bounce option is not played"]
    );
}

#[test]
fn out_of_range_action_values_keep_the_default_and_counts_clamp() {
    let (rotation, lowered) = one_build(
        built("apple:action-rotation", "Action"),
        json!({ "action_rotationAngle": "NaN", "action_rotationDirection": 7 }),
    );
    assert_eq!(
        rotation["effect"]["native"]["params"],
        json!({ "deg": 360, "cw": true, "hold": true })
    );
    assert_eq!(messages(&lowered, "KN1404").len(), 1);
    assert_eq!(
        messages(&lowered, "KN1405"),
        [
            "The direction 7 of the apple:action-rotation effect is unknown; it plays its default direction"
        ]
    );
    for (field, value) in [
        ("action_rotationAngle", json!(1e9)),
        ("action_rotationAngle", json!(-10)),
        ("action_scaleSize", json!(0)),
        ("action_scaleSize", json!("Infinity")),
        ("action_colorAlpha", json!(1.5)),
        ("custom_action_jiggleIntensity", json!(9)),
    ] {
        let effect = match field {
            "action_rotationAngle" => "apple:action-rotation",
            "action_scaleSize" => "apple:action-scale",
            "action_colorAlpha" => "apple:action-opacity",
            _ => "apple:action-jiggle",
        };
        let (step, lowered) = one_build(built(effect, "Action"), json!({ field: value }));
        let default =
            slidock_presets::keynote::build(effect, slidock_ir::effects::StepKind::Emphasis)
                .unwrap()
                .build_effect()
                .cloned()
                .unwrap();
        assert_eq!(
            step["effect"]["native"]["params"],
            canonical(&default)["params"],
            "{field}"
        );
        assert_eq!(messages(&lowered, "KN1404").len(), 1, "{field}");
    }
    let (blink, lowered) = one_build(
        built("apple:action-blink", "Action"),
        json!({ "custom_action_repeatCount": 4_000_000_000_u32 }),
    );
    assert_eq!(
        blink["effect"]["native"]["params"],
        json!({ "repeat": limits::REPEAT_COUNT })
    );
    assert_eq!(
        messages(&lowered, "KN1404"),
        [format!(
            "The apple:action-blink effect plays as an approximation: the repeat count 4000000000 is clamped to {}",
            limits::REPEAT_COUNT
        )]
    );
}

#[test]
fn a_motion_path_plays_as_keyframes_and_an_unreadable_one_as_the_fallback() {
    let path = json!({ "action_motionPathSource": { "bezier_path_source": { "path": { "elements": [
        { "type": 1, "points": [{ "x": 100, "y": 100 }] },
        { "type": 2, "points": [{ "x": 400, "y": 100 }] },
    ] } } } });
    let (step, lowered) = one_build(built("apple:action-motion-path", "Action"), path);
    assert_eq!(step["kind"], json!("emphasis"));
    assert_eq!(
        step["effect"]["native"],
        json!({ "id": "keynote:apple:action-motion-path", "effect": "behaviors", "params": [
            { "property": "translate", "beginMs": 0, "durationMs": 1000, "frames": [
                { "offset": 0, "value": { "x": 0, "y": 0 } },
                { "offset": 1, "value": { "x": 400, "y": 0 } },
            ] },
        ] })
    );
    assert!(messages(&lowered, "KN1404").is_empty());
    // Keynote's own form: editable nodes, the way back from a closed path's last node a straight line.
    let node = |x: f64, y: f64| json!({ "inControlPoint": { "x": x, "y": y }, "nodePoint": { "x": x, "y": y }, "outControlPoint": { "x": x, "y": y }, "type": 1 });
    let editable = json!({ "action_motionPathSource": { "editable_bezier_path_source": { "subpaths": [
        { "nodes": [node(0.0, 0.0), node(96.0, 0.0), node(96.0, 54.0)], "closed": true },
    ], "naturalSize": { "width": 96, "height": 54 } } } });
    let (step, lowered) = one_build(built("apple:action-motion-path", "Action"), editable);
    assert_eq!(
        step["effect"]["native"]["params"][0]["frames"],
        json!([
            { "offset": 0, "value": { "x": 0, "y": 0 } },
            { "offset": 0.36902, "value": { "x": 128, "y": 0 } },
            { "offset": 0.5766, "value": { "x": 128, "y": 72 } },
            { "offset": 1, "value": { "x": 0, "y": 0 } },
        ])
    );
    assert!(messages(&lowered, "KN1404").is_empty());
    let (unstored, lowered) = one_build(built("apple:action-motion-path", "Action"), json!({}));
    assert!(unstored["effect"].get("native").is_none());
    assert_eq!(
        messages(&lowered, "KN1404"),
        [
            "The apple:action-motion-path effect plays as an approximation: the motion path is not stored as a path; the fallback plays"
        ]
    );
    // A node without its handles, and a second subpath, are not guessed at.
    let bare = json!({ "nodePoint": { "x": 96.0, "y": 0.0 }, "type": 1 });
    for (subpaths, reason) in [
        (
            json!([{ "nodes": [node(0.0, 0.0), bare], "closed": false }]),
            "the motion path lacks a point or a coordinate it plays; the fallback plays",
        ),
        (
            json!([
                { "nodes": [node(0.0, 0.0), node(96.0, 0.0)], "closed": false },
                { "nodes": [node(0.0, 54.0), node(96.0, 54.0)], "closed": false },
            ]),
            "the motion path has several subpaths, and how Keynote moves between them is not known; the fallback plays",
        ),
    ] {
        let attributes = json!({ "action_motionPathSource": { "editable_bezier_path_source": {
            "subpaths": subpaths, "naturalSize": { "width": 96, "height": 54 },
        } } });
        let (step, lowered) = one_build(built("apple:action-motion-path", "Action"), attributes);
        assert!(step["effect"].get("native").is_none(), "{reason}");
        assert_eq!(
            messages(&lowered, "KN1404"),
            [format!(
                "The apple:action-motion-path effect plays as an approximation: {reason}"
            )]
        );
    }
}

#[test]
fn effects_without_an_entry_or_unplayable_in_their_kind_play_the_fallback() {
    for (effect, kind) in [
        ("com.apple.iWork.Keynote.KLNSparkle", "In"),
        ("apple:not-an-effect", "In"),
        ("apple:movie-stop", "Out"),
        ("", "In"),
    ] {
        let (step, lowered) = one_build(built(effect, kind), json!({}));
        assert!(step["effect"].get("native").is_none(), "{effect}");
        assert_eq!(step["timing"], json!({ "offsetMs": 0 }), "{effect}");
        assert_eq!(lowered.codes(), ["KN1402"], "{effect}");
    }
    let (movie, lowered) = one_build(built("apple:movie-start", "In"), json!({}));
    assert_eq!(movie["kind"], json!("media"));
    assert_eq!(
        movie["effect"]["native"],
        json!({ "id": "keynote:apple:movie-start", "effect": "play", "params": { "from": 0 } })
    );
    assert!(lowered.diagnostics.is_empty());
}

#[test]
fn delivery_orders_other_than_forward_are_reported() {
    let mut p = Presentation::new();
    let text = text_at(&mut p, "one two three", 0.0);
    let order = |p: &mut Presentation, option: u32, delivery: u32| {
        let b = build(
            p,
            &text,
            built("apple:dissolve", "In"),
            json!({ "custom_deliveryOption": option, "custom_textDelivery": delivery }),
        );
        let c = chunk(p, &b, json!({}));
        (b, c)
    };
    let (b1, c1) = order(&mut p, 2, 2);
    let (b2, c2) = order(&mut p, 1, 3);
    let (b3, c3) = order(&mut p, 5, 1);
    p.slide(
        json!({ "drawables_z_order": [text], "builds": [b1.clone(), b2, b3], "buildChunks": [c1, c2, c3] }),
        json!({}),
        None,
    );
    let lowered = lower_native(&p.bytes());
    assert_eq!(
        messages(&lowered, "KN1406"),
        [format!(
            "The delivery order backward of build {} plays forward",
            id_of(&b1)
        )]
    );
}

#[test]
fn with_and_after_builds_carry_their_start_inside_the_click_group() {
    let mut p = Presentation::new();
    let shapes: Vec<Value> = (0..5)
        .map(|i| shape_at(&mut p, f64::from(i) * 20.0, 10.0))
        .collect();
    let builds: Vec<Value> = shapes
        .iter()
        .map(|shape| build(&mut p, shape, built("apple:dissolve", "In"), json!({})))
        .collect();
    let chunks = vec![
        chunk(
            &mut p,
            &builds[0],
            json!({ "automatic": true, "referent": true, "delay": 0.5 }),
        ),
        chunk(
            &mut p,
            &builds[1],
            json!({ "automatic": true, "referent": false, "delay": 0.25 }),
        ),
        chunk(
            &mut p,
            &builds[2],
            json!({ "automatic": true, "referent": true, "delay": 0.1, "duration": 2.0 }),
        ),
        chunk(
            &mut p,
            &builds[3],
            json!({ "automatic": false, "delay": 0.2 }),
        ),
        chunk(
            &mut p,
            &builds[4],
            json!({ "automatic": true, "referent": false }),
        ),
    ];
    p.slide(
        json!({ "drawables_z_order": shapes, "builds": builds, "buildChunks": chunks }),
        json!({}),
        None,
    );
    let lowered = lower_native(&p.bytes());
    let steps: Vec<Value> = lowered.json["slides"][0]["timeline"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| json!([s["trigger"], s["delayMs"], s["timing"]["offsetMs"]]))
        .collect();
    assert_eq!(
        steps,
        [
            json!(["afterPrevious", 500, 500]),
            json!(["afterPrevious", 250, 750]),
            json!(["afterPrevious", 100, 1850]),
            json!(["click", 200, 200]),
            json!(["afterPrevious", 0, 200]),
        ]
    );
}

#[test]
fn the_parity_path_keeps_keynotes_placement_and_reports_no_native_motion() {
    let mut p = Presentation::new();
    let before = text_at(&mut p, "before", 0.0);
    let shape = shape_at(&mut p, 0.0, 10.0);
    let b = build(
        &mut p,
        &shape,
        json!({ "animation_type": "In", "effect": "apple:move in", "duration": 1.0, "direction": 77 }),
        json!({}),
    );
    let c = chunk(&mut p, &b, json!({}));
    transition_slide(
        &mut p,
        vec![before],
        json!({ "animationAttributes": animation("apple:magic-move-implied-motion-path", json!({})) }),
    );
    p.slide(
        json!({ "drawables_z_order": [shape], "builds": [b], "buildChunks": [c] }),
        json!({}),
        None,
    );
    let bytes = p.bytes();
    let (parity, native) = (lower(&bytes), lower_native(&bytes));
    // The parity path keeps the transition on the slide that stores it, the deck path on the slide it plays into;
    // nothing else differs.
    assert!(parity.json["slides"][0]["transition"].is_object());
    let mut moved = native.json.clone();
    let transition = moved["slides"][1]
        .as_object_mut()
        .unwrap()
        .remove("transition")
        .unwrap();
    assert!(moved["slides"][0].get("transition").is_none());
    moved["slides"][0]["transition"] = transition;
    assert_eq!(parity.json, moved);
    assert!(parity.json["slides"][1]["timeline"][0]["effect"]["native"].is_object());
    assert_eq!(parity.codes(), ["KN1402"]);
    assert_eq!(native.codes(), ["KN1402", "KN1405", "KN1407"]);
}

#[test]
fn stripped_native_data_leaves_the_parity_paths_fields() {
    let (step, lowered) = one_build(
        json!({ "animation_type": "In", "effect": "apple:move in character", "duration": 0.5, "direction": 14 }),
        json!({ "action_acceleration": 3 }),
    );
    assert!(step["effect"].get("native").is_some());
    let mut stripped = lowered.presentation.clone();
    strip_native_motion(&mut stripped);
    let stripped = serde_json::to_value(&stripped).unwrap();
    assert_eq!(
        stripped["slides"][0]["timeline"][0],
        json!({ "target": { "element": step["target"]["element"] }, "trigger": "click", "delayMs": 0,
                "durationMs": 1000, "kind": "entrance",
                "effect": { "id": "keynote:apple:move_in_character", "fallback": "dissolve" }, "delivery": "byChar" })
    );
}

/// Two slides joined by Magic Move: a shared picture and text box that move, a shape that grows, a duplicated
/// text box (ambiguous), and an object on one side only.
fn magic_move_pair(p: &mut Presentation, transition: &str) -> (Vec<Value>, Vec<Value>) {
    let photo = p.data("photo.png", support::fixtures::grey_png(200));
    let image = |p: &mut Presentation, x: f64| {
        p.add(
            "TSD.ImageArchive",
            json!({ "super": { "geometry": geometry(x, 50.0, 80.0, 40.0) }, "data": photo }),
        )
    };
    let first = vec![
        image(p, 0.0),
        text_at(p, "Title", 0.0),
        shape_at(p, 0.0, 10.0),
        text_at(p, "twice", 0.0),
        text_at(p, "twice", 300.0),
        text_at(p, "gone", 600.0),
    ];
    let second = vec![
        image(p, 500.0),
        text_at(p, "Title", 200.0),
        shape_at(p, 100.0, 40.0),
        text_at(p, "twice", 10.0),
        text_at(p, "new", 600.0),
    ];
    transition_slide(
        p,
        first.clone(),
        json!({ "animationAttributes": animation(transition, json!({})) }),
    );
    p.slide(
        json!({ "drawables_z_order": second.clone() }),
        json!({}),
        None,
    );
    (first, second)
}

fn keys(lowered: &Lowered, slide: usize) -> Vec<Option<String>> {
    lowered
        .elements(slide)
        .iter()
        .map(|e| e.get("matchKey").map(|k| k.as_str().unwrap().to_owned()))
        .collect()
}

#[test]
fn magic_move_keys_the_unique_contents_of_both_slides() {
    let mut p = Presentation::new();
    magic_move_pair(&mut p, "apple:magic-move-implied-motion-path");
    let lowered = lower_native(&p.bytes());
    let (first, second) = (keys(&lowered, 0), keys(&lowered, 1));
    // The picture, the title and the shape (at another size) match; "twice" takes two places on the first slide and
    // one on the second, so none of its copies match.
    for i in 0..3 {
        assert!(
            first[i].is_some() && first[i] == second[i],
            "{i}: {first:?} {second:?}"
        );
        assert!(first[i].as_ref().unwrap().starts_with("kn:"));
    }
    assert!(first[3].is_some() && first[4].is_some() && first[3] != first[4]);
    assert!(second[3].is_some());
    assert!(!second.contains(&first[3]) && !second.contains(&first[4]));
    assert!(first[5].is_some() && !second.contains(&first[5]));
    assert!(second[4].is_some() && !first.contains(&second[4]));
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.codes());

    // Another transition keys nothing.
    let mut p = Presentation::new();
    magic_move_pair(&mut p, "apple:dissolve");
    let lowered = lower_native(&p.bytes());
    assert!(
        keys(&lowered, 0)
            .iter()
            .chain(&keys(&lowered, 1))
            .all(Option::is_none)
    );
}

#[test]
fn magic_move_pairs_shown_slides_and_keys_a_slide_alone_as_in_the_whole() {
    let mut p = Presentation::new();
    let a = text_at(&mut p, "same", 0.0);
    let skipped = text_at(&mut p, "same", 0.0);
    let b = text_at(&mut p, "same", 50.0);
    let first = transition_slide(
        &mut p,
        vec![a],
        json!({ "animationAttributes": animation("apple:magic-move-implied-motion-path", json!({})) }),
    );
    let hidden = p.slide(
        json!({ "drawables_z_order": [skipped] }),
        json!({ "isSkipped": true }),
        None,
    );
    let last = p.slide(json!({ "drawables_z_order": [b] }), json!({}), None);
    let bytes = p.bytes();
    let whole = lower_native(&bytes);
    assert!(keys(&whole, 0)[0].is_some());
    assert_eq!(keys(&whole, 0), keys(&whole, 2));
    assert_eq!(keys(&whole, 1), [None]);
    let only: Vec<SlideId> = [&last, &first]
        .iter()
        .map(|node| SlideId::new(kn(node)).unwrap())
        .collect();
    let alone = lower_native_reader(&slidock_io::BytesReader::new(&bytes), Some(&only[..1]));
    assert_eq!(alone.json["slides"][0], whole.json["slides"][2]);
    // The Magic Move plays into the next shown slide; the skipped one carries none.
    assert_eq!(
        whole.json["slides"][2]["transition"]["effect"]["native"]["effect"],
        json!("magicMove")
    );
    assert!(whole.json["slides"][0].get("transition").is_none());
    assert!(whole.json["slides"][1].get("transition").is_none());
    let _ = hidden;
}

#[test]
fn a_magic_move_with_nothing_in_common_is_reported() {
    let mut p = Presentation::new();
    let a = text_at(&mut p, "before", 0.0);
    let b = text_at(&mut p, "after", 0.0);
    transition_slide(
        &mut p,
        vec![a],
        json!({ "animationAttributes": animation("apple:magic-move-implied-motion-path", json!({})) }),
    );
    let to = p.slide(json!({ "drawables_z_order": [b] }), json!({}), None);
    let lowered = lower_native(&p.bytes());
    let reported: Vec<(String, String)> = lowered
        .diagnostics
        .iter()
        .map(|d| (d.code.code().to_owned(), d.message.clone()))
        .collect();
    assert_eq!(
        reported,
        [(
            "KN1407".to_owned(),
            format!(
                "The Magic Move into slide {} matches no object; every object fades",
                kn(&to)
            )
        )]
    );
    assert_eq!(
        lowered.diagnostics[0].container.as_ref().unwrap().as_str(),
        kn(&to)
    );
    assert!(lower(&p.bytes()).diagnostics.is_empty());
}

#[test]
fn hostile_timing_is_bounded() {
    // More chunks than a slide's steps carry native data for, with NaN and huge timings.
    let mut p = Presentation::new();
    let shape = shape_at(&mut p, 0.0, 10.0);
    let b = build(
        &mut p,
        &shape,
        json!({ "animation_type": "In", "effect": "apple:dissolve", "duration": "NaN" }),
        json!({ "custom_action_repeatCount": 4_000_000_000_u32 }),
    );
    let count = limits::STEPS_PER_SLIDE + 3;
    let c = chunk(
        &mut p,
        &b,
        json!({ "automatic": true, "referent": true, "delay": 1e12, "duration": "Infinity" }),
    );
    p.slide(
        json!({ "drawables_z_order": [shape], "builds": [b], "buildChunks": vec![c; count] }),
        json!({}),
        None,
    );
    let lowered = lower_native(&p.bytes());
    let steps = lowered.json["slides"][0]["timeline"].as_array().unwrap();
    assert_eq!(steps.len(), count);
    assert!(
        steps[..limits::STEPS_PER_SLIDE]
            .iter()
            .all(|s| s["timing"]["offsetMs"] == json!(3_600_000))
    );
    assert!(
        steps[limits::STEPS_PER_SLIDE..]
            .iter()
            .all(|s| s.get("timing").is_none())
    );
    assert_eq!(messages(&lowered, "KN1404").len(), 1);
    assert!(steps.iter().all(|s| s["delayMs"] == json!(3_600_000)));

    // A chunk that names itself as its build, and a build that names a chunk: no step, no hang.
    let mut p = Presentation::new();
    let shape = shape_at(&mut p, 0.0, 10.0);
    let looped = p.add(
        "KN.BuildChunkArchive",
        json!({ "build": { "identifier": "1001" } }),
    );
    p.slide(
        json!({ "drawables_z_order": [shape], "buildChunks": [looped] }),
        json!({}),
        None,
    );
    let lowered = lower_native(&p.bytes());
    assert_eq!(lowered.codes(), ["KN1401"]);
}

#[test]
fn magic_move_matches_groups_by_their_children() {
    let mut p = Presentation::new();
    let nested = |p: &mut Presentation, x: f64| {
        let mut inner = shape_at(p, x, 10.0);
        for _ in 0..5 {
            inner = p.add(
                "TSD.GroupArchive",
                json!({ "super": { "geometry": geometry(x, 0.0, 10.0, 10.0) }, "children": [inner] }),
            );
        }
        inner
    };
    let first = nested(&mut p, 0.0);
    let second = nested(&mut p, 300.0);
    transition_slide(
        &mut p,
        vec![first],
        json!({ "animationAttributes": animation("apple:magic-move-implied-motion-path", json!({})) }),
    );
    p.slide(json!({ "drawables_z_order": [second] }), json!({}), None);
    let lowered = lower_native(&p.bytes());
    assert_eq!(lowered.elements(0)[0]["kind"], json!("group"));
    assert!(keys(&lowered, 0)[0].is_some());
    assert_eq!(keys(&lowered, 0), keys(&lowered, 1));
}

#[test]
fn magic_move_matches_a_drawn_shape_at_another_size() {
    let triangle = |p: &mut Presentation, x: f64, w: f64| {
        p.add(
            "TSD.ShapeArchive",
            json!({ "super": { "geometry": geometry(x, 0.0, w, w) },
                    "pathsource": { "bezier_path_source": { "naturalSize": { "width": w, "height": w }, "path": { "elements": [
                        { "type": 1, "points": [{ "x": 0, "y": w }] },
                        { "type": 2, "points": [{ "x": w / 2.0, "y": 0 }] },
                        { "type": 2, "points": [{ "x": w, "y": w }] },
                        { "type": 5, "points": [] },
                    ] } } } }),
        )
    };
    let mut p = Presentation::new();
    let small = triangle(&mut p, 0.0, 10.0);
    let large = triangle(&mut p, 200.0, 40.0);
    transition_slide(
        &mut p,
        vec![small],
        json!({ "animationAttributes": animation("apple:magic-move-implied-motion-path", json!({})) }),
    );
    p.slide(json!({ "drawables_z_order": [large] }), json!({}), None);
    let lowered = lower_native(&p.bytes());
    assert_ne!(
        lowered.elements(0)[0]["geometry"],
        lowered.elements(1)[0]["geometry"]
    );
    assert!(keys(&lowered, 0)[0].is_some());
    assert_eq!(keys(&lowered, 0), keys(&lowered, 1));
}

#[test]
fn unknown_or_mismatched_animation_types_play_the_fallback() {
    for (animation, effect, codes, reason) in [
        (
            json!("Weird"),
            "apple:dissolve",
            vec!["KN1404"],
            "the animation type Weird is unknown; the fallback plays",
        ),
        (
            Value::Null,
            "apple:appear",
            vec!["KN1404"],
            "the build states no animation type; the fallback plays",
        ),
        (
            json!("Action"),
            "apple:dissolve",
            vec!["KN1404"],
            "an Action build does not play it; the fallback plays",
        ),
        (
            json!("In"),
            "apple:action-rotation",
            vec!["KN1402", "KN1404"],
            "an In build does not play it; the fallback plays",
        ),
    ] {
        let mut attributes = json!({ "effect": effect, "duration": 1.0 });
        if !animation.is_null() {
            attributes["animation_type"] = animation.clone();
        }
        let (step, lowered) = one_build(attributes.clone(), json!({}));
        assert!(
            step["effect"].get("native").is_none(),
            "{animation} {effect}"
        );
        assert_eq!(lowered.codes(), codes, "{animation} {effect}");
        assert_eq!(
            messages(&lowered, "KN1404"),
            [format!(
                "The {effect} effect plays as an approximation: {reason}"
            )]
        );
    }
    // A media command plays by its id, whatever its type, as the IR's kind says.
    let (movie, lowered) = one_build(built("apple:movie-start", "Weird"), json!({}));
    assert_eq!(movie["effect"]["native"]["effect"], json!("play"));
    assert!(lowered.diagnostics.is_empty());
}

/// Keynote's own files pin the numbers (`slidock_presets::keynote::direction`): a pinned reading plays silently, an exit's
/// in/out flip included (Keynote's build movies show it), and an inferred one (41/42 on a zoom, 43/44 on an iris) is
/// reported. An iris also reports the feathered edge it plays hard.
#[test]
fn direction_numbers_read_as_keynote_writes_them_and_exits_reverse_a_zoom() {
    let (spin, lowered) = one_build(
        json!({ "animation_type": "In", "effect": "apple:spin", "duration": 1.0, "direction": 32 }),
        json!({}),
    );
    assert_eq!(spin["effect"]["native"]["params"]["cw"], json!(false));
    assert!(messages(&lowered, "KN1404").is_empty());
    for (effect, kind, number, expected, reported) in [
        ("apple:zoom", "In", 43, "in", 0),
        ("apple:zoom", "In", 44, "out", 0),
        ("apple:zoom", "Out", 43, "out", 0),
        ("apple:zoom", "Out", 44, "in", 0),
        ("apple:zoom", "In", 41, "in", 1),
        ("apple:zoom", "In", 42, "out", 1),
        ("apple:zoom", "Out", 41, "out", 1),
        ("apple:wipe-iris", "In", 41, "in", 1),
        ("apple:wipe-iris", "In", 42, "out", 1),
        ("apple:wipe-iris", "Out", 41, "out", 1),
        ("apple:wipe-iris", "Out", 42, "in", 1),
        ("apple:wipe-iris", "In", 43, "in", 2),
    ] {
        let (step, lowered) = one_build(
            json!({ "animation_type": kind, "effect": effect, "duration": 1.0, "direction": number }),
            json!({}),
        );
        assert_eq!(
            step["effect"]["native"]["params"]["inOut"],
            json!(expected),
            "{effect} {kind} {number}"
        );
        let reasons = messages(&lowered, "KN1404");
        let count = reasons
            .first()
            .map_or(0, |reason| reason.matches("; ").count() + 1);
        assert_eq!(count, reported, "{effect} {kind} {number}: {reasons:?}");
    }
}

#[test]
fn rejected_values_print_as_javascript_numbers() {
    let (_, lowered) = one_build(
        built("apple:action-rotation", "Action"),
        json!({ "action_rotationAngle": 1e308 }),
    );
    assert_eq!(
        messages(&lowered, "KN1404"),
        [
            "The apple:action-rotation effect plays as an approximation: the rotation angle 1e+308 is out of range; the default plays"
        ]
    );
}

/// The steps of a slide whose builds of one shape each are `chunks` (with `attributes` on the builds), by
/// (trigger, delay, offset), and the slide's lowering.
fn chunked(builds: &[(Value, Value)]) -> (Vec<Value>, Lowered) {
    let mut p = Presentation::new();
    let shapes: Vec<Value> = (0..builds.len())
        .map(|i| shape_at(&mut p, i as f64 * 20.0, 10.0))
        .collect();
    let (mut bs, mut cs) = (Vec::new(), Vec::new());
    for (shape, (attributes, c)) in shapes.iter().zip(builds) {
        let b = build(
            &mut p,
            shape,
            built("apple:dissolve", "In"),
            attributes.clone(),
        );
        cs.push(chunk(&mut p, &b, c.clone()));
        bs.push(b);
    }
    p.slide(
        json!({ "drawables_z_order": shapes, "builds": bs, "buildChunks": cs }),
        json!({}),
        None,
    );
    let lowered = lower_native(&p.bytes());
    let steps = lowered.json["slides"][0]["timeline"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| json!([s["trigger"], s["delayMs"], s["timing"]["offsetMs"]]))
        .collect();
    (steps, lowered)
}

#[test]
fn automatic_delays_tell_with_from_after_and_event_triggers_are_checked() {
    let automatic = |referent: bool| json!({ "automatic": true, "referent": referent });
    // The build's automatic delay decides, and gives the delay a chunk does not state.
    let (steps, lowered) = chunked(&[
        (
            json!({}),
            json!({ "automatic": true, "referent": true, "delay": 0.5 }),
        ),
        (json!({ "delayAutomaticWith": 0.25 }), automatic(false)),
        (json!({ "delayAutomaticAfter": 0.1 }), automatic(true)),
    ]);
    assert_eq!(
        steps,
        [
            json!(["afterPrevious", 500, 500]),
            json!(["afterPrevious", 0, 750]),
            json!(["afterPrevious", 0, 1850]),
        ]
    );
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.codes());
    // A referent that disagrees, both delays, a delay out of range and an unknown event trigger are reported.
    let (steps, lowered) = chunked(&[
        (json!({}), automatic(true)),
        (json!({ "delayAutomaticWith": 0.0 }), automatic(true)),
        (
            json!({ "delayAutomaticWith": 0.0, "delayAutomaticAfter": 0.0 }),
            automatic(false),
        ),
        (json!({ "delayAutomaticAfter": -1.0 }), automatic(true)),
        (json!({ "eventTrigger": 7 }), json!({})),
    ]);
    assert_eq!(
        steps,
        [
            json!(["afterPrevious", 0, 0]),
            json!(["afterPrevious", 0, 0]),
            json!(["afterPrevious", 0, 0]),
            json!(["afterPrevious", 0, 1000]),
            json!(["click", 0, 0]),
        ]
    );
    assert_eq!(
        sorted_messages(&lowered, "KN1404"),
        [
            "The apple:dissolve effect plays as an approximation: the automatic delay -1 is out of range; the chunk's referent plays",
            "The apple:dissolve effect plays as an approximation: the build states both a with and an after delay; the chunk's referent plays",
            "The apple:dissolve effect plays as an approximation: the chunk's referent says after and its build's automatic delay says with; with plays",
            "The apple:dissolve effect plays as an approximation: the event trigger 7 is unknown; the chunk's own trigger plays",
        ]
    );
    // Keynote 14.5 writes PowerPoint's click, with-previous (delay 0.25 s) and after-previous (delay 0.5 s) as
    // event triggers 1, 0 and 2 beside the chunks' referents (its import of the PPTX motion probe, slide 189).
    let (steps, lowered) = chunked(&[
        (json!({ "eventTrigger": 1 }), json!({ "referent": true })),
        (
            json!({ "eventTrigger": 0 }),
            json!({ "automatic": true, "referent": false, "delay": 0.25 }),
        ),
        (
            json!({ "eventTrigger": 2 }),
            json!({ "automatic": true, "referent": true, "delay": 0.5 }),
        ),
    ]);
    assert_eq!(
        steps,
        [
            json!(["click", 0, 0]),
            json!(["afterPrevious", 250, 250]),
            json!(["afterPrevious", 500, 1750]),
        ]
    );
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.codes());
    // A trigger that disagrees with its chunk is reported; the chunk plays.
    let (steps, lowered) = chunked(&[
        (json!({}), json!({})),
        (json!({ "eventTrigger": 2 }), automatic(false)),
        (json!({ "eventTrigger": 0 }), json!({})),
    ]);
    assert_eq!(
        steps,
        [
            json!(["click", 0, 0]),
            json!(["afterPrevious", 0, 0]),
            json!(["click", 0, 0]),
        ]
    );
    assert_eq!(
        sorted_messages(&lowered, "KN1404"),
        [
            "The apple:dissolve effect plays as an approximation: the event trigger says after; the chunk's own trigger, with, plays",
            "The apple:dissolve effect plays as an approximation: the event trigger says with; the chunk's own trigger, click, plays",
        ]
    );
}

#[test]
fn chunk_indices_build_options_and_legacy_fields_are_read() {
    // A chunk index that disagrees with the chunk's place in its build, options with no slot, a chart's rotation.
    let (_, lowered) = chunked(&[
        (json!({}), json!({ "index": 3 })),
        (
            json!({ "custom_action_scale": 1.5, "startOffset": 2 }),
            json!({}),
        ),
        (
            json!({ "ChartRotation3D": 60.0, "eventTrigger": 1 }),
            json!({}),
        ),
    ]);
    assert_eq!(
        sorted_messages(&lowered, "KN1404"),
        [
            "The apple:dissolve effect plays as an approximation: the action scale option is not played; the start and end offsets option is not played",
            "The apple:dissolve effect plays as an approximation: the chunk's index 3 is not its place 0 in the build; the build order plays",
        ]
    );
    let (_, lowered) = one_build(
        json!({ "animation_type": "In", "effect": "apple:dissolve", "duration": 1.0, "custom_detail": 0.5 }),
        json!({}),
    );
    assert_eq!(
        messages(&lowered, "KN1404"),
        ["The apple:dissolve effect plays as an approximation: the detail option is not played"]
    );
    // A build stored in the legacy fields only plays the fallback.
    let (step, lowered) = one_build(
        Value::Null,
        json!({ "animationAttributes": null, "database_effect": "apple:dissolve", "database_duration": 2.0 }),
    );
    assert!(step["effect"].get("native").is_none());
    assert_eq!(lowered.codes(), ["KN1402", "KN1404"]);
    assert_eq!(
        messages(&lowered, "KN1404"),
        [
            "The none effect plays as an approximation: the build is stored in the legacy fields (apple:dissolve), which are not read; the fallback plays"
        ]
    );
    // A transition stored in the legacy fields only is left out, as on the parity path.
    let (transition, lowered) = one_transition(
        json!({ "database_effect": "apple:push", "database_duration": 1.0, "database_is_automatic": true }),
    );
    assert!(transition.is_null());
    assert_eq!(
        messages(&lowered, "KN1409"),
        [
            "The apple:push transition is left out: it is stored in the legacy fields, which are not read"
        ]
    );
}

#[test]
fn custom_curves_path_styles_and_random_orders_are_read() {
    let cubic = |y: f64| {
        json!({ "bezier_path_source": { "path": { "elements": [
            { "type": 1, "points": [{ "x": 0, "y": 0 }] },
            { "type": 4, "points": [{ "x": 25, "y": y }, { "x": 25, "y": 100 }, { "x": 100, "y": 100 }] },
        ] } } })
    };
    let transition = |extra: Value| {
        one_transition(json!({
            "animationAttributes": animation("apple:dissolve", merge(json!({ "custom_effect_timing_curve_1": cubic(10.0) }), extra)),
            "custom_timing_curve": 5,
        }))
    };
    let (same, lowered) = transition(json!({ "custom_effect_timing_curve_2": cubic(10.0) }));
    assert_eq!(same["easing"], json!({ "bezier": [0.25, 0.1, 0.25, 1] }));
    assert!(lowered.diagnostics.is_empty());
    let (other, lowered) = transition(json!({ "custom_effect_timing_curve_3": cubic(50.0) }));
    assert_eq!(other["easing"], json!({ "bezier": [0.25, 0.1, 0.25, 1] }));
    assert_eq!(
        messages(&lowered, "KN1408"),
        ["The timing curve of the apple:dissolve effect plays as its first curve"]
    );

    let line = json!({ "bezier_path_source": { "path": { "elements": [
        { "type": 1, "points": [{ "x": 100, "y": 100 }] },
        { "type": 2, "points": [{ "x": 400, "y": 100 }] },
    ] } } });
    for (style, reason) in [
        (json!(0), None),
        (
            json!(1),
            Some("the curved path's stored segments are straight and play straight"),
        ),
        (
            json!(9),
            Some("the path style 9 is unknown; the stored path plays"),
        ),
    ] {
        let (step, lowered) = one_build(
            built("apple:action-motion-path", "Action"),
            json!({ "action_motionPathSource": line, "curveStyle": style }),
        );
        assert_eq!(step["effect"]["native"]["effect"], json!("behaviors"));
        let expected: Vec<String> = reason
            .map(|r| format!("The apple:action-motion-path effect plays as an approximation: {r}"))
            .into_iter()
            .collect();
        assert_eq!(messages(&lowered, "KN1404"), expected, "{style}");
    }
    let (string_only, lowered) = one_build(
        built("apple:action-motion-path", "Action"),
        json!({ "motionPathString": "M 0 0 L 10 10" }),
    );
    assert!(string_only["effect"].get("native").is_none());
    assert_eq!(
        messages(&lowered, "KN1404"),
        [
            "The apple:action-motion-path effect plays as an approximation: the motion path is stored only as a string, which is not read; the fallback plays"
        ]
    );

    let mut p = Presentation::new();
    let text = text_at(&mut p, "one two three", 0.0);
    let b = build(
        &mut p,
        &text,
        json!({ "animation_type": "In", "effect": "apple:dissolve", "duration": 1.0, "random_number_seed": 42 }),
        json!({ "custom_deliveryOption": 5, "custom_textDelivery": 2 }),
    );
    let c = chunk(&mut p, &b, json!({}));
    p.slide(
        json!({ "drawables_z_order": [text], "builds": [b.clone()], "buildChunks": [c] }),
        json!({}),
        None,
    );
    let lowered = lower_native(&p.bytes());
    assert_eq!(
        messages(&lowered, "KN1406"),
        [format!(
            "The delivery order random (seed 42) of build {} plays forward",
            id_of(&b)
        )]
    );
}

#[test]
fn magic_move_matches_stacked_copies_as_one_candidate() {
    let mut p = Presentation::new();
    let photo = p.data("photo.png", support::fixtures::grey_png(200));
    let image = |p: &mut Presentation, x: f64| {
        p.add(
            "TSD.ImageArchive",
            json!({ "super": { "geometry": geometry(x, 50.0, 80.0, 40.0) }, "data": photo }),
        )
    };
    // Two copies of one picture in the same place, then one copy elsewhere; two copies apart match no single copy.
    let first = vec![
        image(&mut p, 0.0),
        image(&mut p, 0.0),
        text_at(&mut p, "apart", 0.0),
        text_at(&mut p, "apart", 300.0),
    ];
    let second = vec![image(&mut p, 500.0), text_at(&mut p, "apart", 10.0)];
    transition_slide(
        &mut p,
        first,
        json!({ "animationAttributes": animation("apple:magic-move-implied-motion-path", json!({})) }),
    );
    p.slide(json!({ "drawables_z_order": second }), json!({}), None);
    let lowered = lower_native(&p.bytes());
    let (first, second) = (keys(&lowered, 0), keys(&lowered, 1));
    // The top copy (painted last) morphs; the copy under it fades.
    assert_eq!(first[0], None);
    assert!(first[1].is_some() && first[1] == second[0]);
    assert!(second[1].is_some());
    assert!(!first.contains(&second[1]));
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.codes());
}

/// A diagram's boxes and arrows are copies of one content in several places: Keynote moves each with the diagram, so
/// two slides holding as many copies pair them in painting order (a talk's panel sliding in by Magic Move).
#[test]
fn magic_move_pairs_copies_in_as_many_places_in_painting_order() {
    let mut p = Presentation::new();
    let first = vec![
        shape_at(&mut p, 1100.0, 10.0),
        text_at(&mut p, "Parser", 1100.0),
        shape_at(&mut p, 1300.0, 10.0),
    ];
    // The copies moved, the second copy's place stacked twice, and one more copy on the third slide.
    let second = vec![
        shape_at(&mut p, 500.0, 10.0),
        shape_at(&mut p, 500.0, 10.0),
        text_at(&mut p, "Parser", 500.0),
        shape_at(&mut p, 700.0, 10.0),
    ];
    let third = vec![
        shape_at(&mut p, 0.0, 10.0),
        shape_at(&mut p, 200.0, 10.0),
        shape_at(&mut p, 400.0, 10.0),
    ];
    let magic = || json!({ "animationAttributes": animation("apple:magic-move-implied-motion-path", json!({})) });
    transition_slide(&mut p, first, magic());
    transition_slide(&mut p, second, magic());
    p.slide(json!({ "drawables_z_order": third }), json!({}), None);
    let lowered = lower_native(&p.bytes());
    let (first, second, third) = (keys(&lowered, 0), keys(&lowered, 1), keys(&lowered, 2));
    // Two places on each of the first two slides: the first place pairs with the first, the second with the second.
    assert!(first.iter().all(Option::is_some), "{first:?}");
    assert_ne!(first[0], first[2]);
    assert_eq!(second[0], None, "a stacked copy under the top one fades");
    assert_eq!((&second[1], &second[3]), (&first[0], &first[2]));
    assert_eq!(second[2], first[1]);
    // Three places on the third slide: none pairs with the second slide's two, so that Magic Move has nothing in
    // common (KN1407).
    assert!(
        third
            .iter()
            .all(|key| key.is_some() && !second.contains(key))
    );
    assert_eq!(lowered.codes(), ["KN1407"]);
}

/// The deck path plays each Keynote transition into the shown slide after the one storing it, with that slide's own
/// advance; the last shown slide's own transition leads nowhere and is reported left out (KN1409); a slide that advances by itself with
/// nothing played into it gets a cut that carries its advance.
#[test]
fn transitions_play_into_the_next_shown_slide_with_each_slides_own_advance() {
    let mut p = Presentation::new();
    let automatic = |effect: &str| json!({ "animationAttributes": animation(effect, json!({ "is_automatic": true, "delay": 2.0 })) });
    transition_slide(&mut p, vec![], automatic("none"));
    transition_slide(&mut p, vec![], automatic("apple:push"));
    p.slide(
        json!({ "transition": { "attributes": { "animationAttributes": animation("apple:wipe", json!({})) } } }),
        json!({ "isSkipped": true }),
        None,
    );
    transition_slide(
        &mut p,
        vec![],
        json!({ "animationAttributes": animation("apple:reveal", json!({})) }),
    );
    let bytes = p.bytes();
    let native = lower_native(&bytes);
    let transitions: Vec<Value> = native.json["slides"]
        .as_array()
        .unwrap()
        .iter()
        .map(|slide| slide["transition"].clone())
        .collect();
    assert_eq!(
        transitions[0],
        json!({ "effect": { "id": "keynote:none", "fallback": "none",
                            "native": { "id": "keynote:none", "effect": "none" } },
                "durationMs": 0, "advance": { "onClick": false, "afterMs": 2000 } })
    );
    // The push stored on the second slide plays into the fourth (the third is skipped), whose own advance is none.
    assert_eq!(transitions[1]["effect"]["id"], json!("keynote:none"));
    assert_eq!(transitions[1]["advance"]["afterMs"], json!(2000));
    assert_eq!(transitions[2], Value::Null);
    assert_eq!(transitions[3]["effect"]["id"], json!("keynote:apple:push"));
    assert!(transitions[3].get("advance").is_none());
    assert_eq!(
        messages(&native, "KN1409"),
        [
            "The apple:reveal transition is left out: it leaves the last shown slide, and no slide follows it"
        ]
    );
    assert!(messages(&native, "KN1404").is_empty());
    // The parity path keeps each where Keynote stores it.
    let parity = lower(&bytes);
    assert_eq!(
        parity.json["slides"][1]["transition"]["effect"]["id"],
        json!("keynote:apple:push")
    );
    assert_eq!(
        parity.json["slides"][3]["transition"]["effect"]["id"],
        json!("keynote:apple:reveal")
    );
    assert!(parity.json["slides"][0].get("transition").is_none());
}

/// A moved transition's reports go with the slide it plays into: its unmapped effect (KN1403) and its feature. A
/// skipped slide's own transition never plays and reports nothing, as Keynote never leaves that slide; the last shown
/// slide's is left out (KN1409) and reports nothing else.
#[test]
fn a_moved_transition_reports_on_the_slide_it_plays_into() {
    let mut p = Presentation::new();
    let plain = |effect: &str| json!({ "animationAttributes": animation(effect, json!({})) });
    transition_slide(&mut p, vec![], plain("apple:unknown-one"));
    transition_slide(&mut p, vec![], plain("none"));
    p.slide(
        json!({ "transition": { "attributes": plain("apple:unknown-two") } }),
        json!({ "isSkipped": true }),
        None,
    );
    transition_slide(&mut p, vec![], plain("apple:unknown-three"));
    let bytes = p.bytes();
    let ids: Vec<String> = lower(&bytes).json["slides"]
        .as_array()
        .unwrap()
        .iter()
        .map(|slide| slide["id"].as_str().unwrap().to_owned())
        .collect();
    let unmapped = |lowered: &Lowered| -> Vec<(String, String)> {
        lowered
            .diagnostics
            .iter()
            .filter(|d| d.code.code() == "KN1403")
            .map(|d| {
                let container = d.container.as_ref().map(|c| c.as_str().to_owned());
                (container.unwrap_or_default(), d.message.clone())
            })
            .collect()
    };
    let transitions = |lowered: &Lowered| -> Vec<bool> {
        ids.iter()
            .map(|id| lowered.features(id).contains(&"slide.transition"))
            .collect()
    };
    let native = lower_native(&bytes);
    assert_eq!(
        unmapped(&native),
        [(
            ids[1].clone(),
            "The transition apple:unknown-one is not supported and plays as a dissolve".to_owned()
        )]
    );
    assert_eq!(transitions(&native), [false, true, false, false]);
    assert_eq!(
        messages(&native, "KN1409"),
        [
            "The apple:unknown-three transition is left out: it leaves the last shown slide, and no slide follows it"
        ]
    );
    // The parity path reports each where Keynote stores it.
    let parity = lower(&bytes);
    let stored: Vec<String> = unmapped(&parity).into_iter().map(|(c, _)| c).collect();
    assert_eq!(stored, [ids[0].clone(), ids[2].clone(), ids[3].clone()]);
    assert_eq!(transitions(&parity), [true, false, true, true]);
    assert!(parity.codes().iter().all(|code| *code != "KN1409"));
}

/// Keynote's text delivery decides on the deck path: by object plays the text whole whatever the effect's name
/// says (the corpus's `apple:dissolve character` builds); the parity path keeps its by-character reading.
#[test]
fn text_delivery_by_object_plays_whole_on_the_deck_path() {
    for (text_delivery, delivery, chunking, parity, reported) in [
        (json!(1), "all", "All at Once", "byChar", 0),
        (json!(1), "byParagraph", "By Paragraph", "byChar", 0),
        (json!(2), "byWord", "All at Once", "byChar", 0),
        (json!(0), "byChar", "All at Once", "byChar", 0),
        (Value::Null, "byChar", "All at Once", "byChar", 0),
        (json!(9), "byChar", "All at Once", "byChar", 1),
    ] {
        let mut p = Presentation::new();
        let text = text_at(&mut p, "one two", 0.0);
        let mut attributes =
            json!({ "animationAttributes": built("apple:dissolve character", "In") });
        if !text_delivery.is_null() {
            attributes["custom_textDelivery"] = text_delivery.clone();
        }
        let b = p.add(
            "KN.BuildArchive",
            json!({ "drawable": text, "delivery": chunking, "attributes": attributes }),
        );
        let c = chunk(&mut p, &b, json!({}));
        p.slide(
            json!({ "drawables_z_order": [text], "builds": [b], "buildChunks": [c] }),
            json!({}),
            None,
        );
        let bytes = p.bytes();
        let native = lower_native(&bytes);
        let step = &native.json["slides"][0]["timeline"][0];
        assert_eq!(
            step["delivery"],
            json!(delivery),
            "{text_delivery} {chunking}"
        );
        assert_eq!(step["effect"]["native"]["effect"], json!("fade"));
        assert_eq!(
            messages(&native, "KN1404").len(),
            reported,
            "{text_delivery}"
        );
        assert_eq!(
            lower(&bytes).json["slides"][0]["timeline"][0]["delivery"],
            json!(parity)
        );
    }
}
