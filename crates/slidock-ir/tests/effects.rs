//! Every effect kind of the vocabulary round-trips through serde, matching the committed golden
//! (`tests/data/effects.json`), and through ts-rs: `tests/data/effects.ts` types the same values with the
//! `@slidock/types` declarations, so `pnpm --filter @slidock/types typecheck` checks them (with one value it must
//! reject). `UPDATE_GOLDEN=1` rewrites both files.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code"
)]

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{Value, json};
use slidock_ir::effects::{
    AfterEffect, Axis, AxisParams, BuildEffect, ClipTrack, ClockParams, ClockStart,
    ColorChangeParams, ColorMix, ColorProperty, ColorValue, CutParams, Diagonal, Dir, Dir4,
    Dir4Params, DirParams, Distance, Easing, FadeParams, FloatInParams, FlyInParams,
    FlythroughParams, Frame, GrowShrinkParams, HexColor, HslShift, InOut, IntensityParams,
    IrisParams, IrisShape, Keyframes, MagicMoveParams, MorphBy, MorphParams, NativeId, NativeStep,
    NativeTransition, Offset, Orient, OrientParams, PathParams, PathSeg, PeekParams, PlayParams,
    PulseParams, Repeat, RepeatParams, Reveal, RevealParams, Scale, SetTrack, SetValue,
    SpinInParams, SpinParams, SplitParams, Stagger, StaggerAmount, StaggerOrder, StepTiming,
    StripsParams, Through, Track, TransitionEffect, TransitionZoomParams, TransparencyParams,
    Vertical, WheelParams, WipeParams, ZoomOrigin, ZoomParams,
};
use slidock_ir::formats::FileFormat;
use slidock_ir::timeline::{TimelineStep, Transition};
use slidock_ir::units::{Deg, Ms, Px, Ratio, UnitRatio};
use slidock_ir::validate::validate;
use ts_rs::{Config, TS};

fn ratio(x: f64) -> Ratio {
    Ratio::exact(x).unwrap()
}

fn unit(x: f64) -> UnitRatio {
    UnitRatio::exact(x).unwrap()
}

fn px(x: f64) -> Px {
    Px::exact(x).unwrap()
}

fn deg(x: f64) -> Deg {
    Deg::exact(x).unwrap()
}

fn left() -> Dir4Params {
    Dir4Params { dir: Dir4::Left }
}

fn horz() -> OrientParams {
    OrientParams {
        orient: Orient::Horizontal,
    }
}

fn transitions() -> Vec<TransitionEffect> {
    use TransitionEffect as T;
    vec![
        T::None,
        T::Cut(CutParams {
            through_black: true,
        }),
        T::Fade(FadeParams {
            through: Through::Color,
            color: Some(HexColor::rgb(0x33, 0x66, 0x99)),
        }),
        T::Dissolve,
        T::Push(Dir4Params { dir: Dir4::Down }),
        T::Cover(DirParams { dir: Dir::LeftUp }),
        T::Uncover(DirParams {
            dir: Dir::RightDown,
        }),
        T::Wipe(WipeParams {
            dir: Dir::Up,
            soft: true,
        }),
        T::Split(SplitParams {
            orient: Orient::Vertical,
            in_out: InOut::In,
        }),
        T::Iris(IrisParams {
            shape: IrisShape::Diamond,
            in_out: InOut::Out,
        }),
        T::Wheel(WheelParams {
            spokes: 3,
            cw: false,
        }),
        T::Clock(ClockParams {
            cw: true,
            from: ClockStart::Nine,
        }),
        T::Wedge,
        T::Blinds(horz()),
        T::Checker(Dir4Params { dir: Dir4::Right }),
        T::RandomBars(OrientParams {
            orient: Orient::Vertical,
        }),
        T::Strips(StripsParams {
            corner: Diagonal::LeftDown,
        }),
        T::Comb(horz()),
        T::Zoom(TransitionZoomParams {
            in_out: InOut::In,
            rotate: true,
        }),
        T::Flip(left()),
        T::Cube(Dir4Params { dir: Dir4::Up }),
        T::Doors(OrientParams {
            orient: Orient::Vertical,
        }),
        T::Window(horz()),
        T::Gallery(left()),
        T::Conveyor(Dir4Params { dir: Dir4::Right }),
        T::Ferris(left()),
        T::Pan(Dir4Params { dir: Dir4::Down }),
        T::Switch(left()),
        T::Rotate3d(left()),
        T::Reveal(RevealParams {
            dir: Dir4::Right,
            through_black: true,
        }),
        T::Flythrough(FlythroughParams {
            in_out: InOut::Out,
            bounce: true,
        }),
        T::Swap(left()),
        T::PageFlip(left()),
        T::Twist(left()),
        T::Mosaic(left()),
        T::Morph(MorphParams { by: MorphBy::Word }),
        T::MagicMove(MagicMoveParams {
            fade_unmatched: true,
        }),
        T::Random,
    ]
}

fn track<V>(frames: Vec<(f64, V)>) -> Track<V> {
    Track {
        begin_ms: Ms(0),
        duration_ms: Ms(500),
        easing: None,
        frames: frames
            .into_iter()
            .map(|(offset, value)| Frame {
                offset: unit(offset),
                value,
            })
            .collect(),
    }
}

fn builds() -> Vec<BuildEffect> {
    use BuildEffect as B;
    let axis = |axis| AxisParams { axis };
    vec![
        B::Appear,
        B::Fade,
        B::Dissolve,
        B::FlyIn(FlyInParams {
            dir: Dir::Right,
            distance: Distance::OffSlide,
        }),
        B::FloatIn(FloatInParams { dir: Vertical::Up }),
        B::Peek(PeekParams {
            dir: Dir4::Up,
            slow: true,
        }),
        B::Wipe(WipeParams {
            dir: Dir::Down,
            soft: true,
        }),
        B::Split(SplitParams {
            orient: Orient::Horizontal,
            in_out: InOut::Out,
        }),
        B::Iris(IrisParams {
            shape: IrisShape::Plus,
            in_out: InOut::In,
        }),
        B::Wheel(WheelParams {
            spokes: 8,
            cw: true,
        }),
        B::Wedge,
        B::Clock(ClockParams {
            cw: false,
            from: ClockStart::Twelve,
        }),
        B::Blinds(horz()),
        B::Checker(Dir4Params { dir: Dir4::Down }),
        B::RandomBars(horz()),
        B::Strips(StripsParams {
            corner: Diagonal::RightUp,
        }),
        B::Zoom(ZoomParams {
            in_out: InOut::In,
            origin: ZoomOrigin::SlideCenter,
            slight: false,
            fade: true,
            overshoot: false,
        }),
        B::Grow(axis(Axis::Both)),
        B::Expand(axis(Axis::Horizontal)),
        B::Compress(axis(Axis::Vertical)),
        B::Stretch(axis(Axis::Horizontal)),
        B::GrowTurn,
        B::Swivel(axis(Axis::Horizontal)),
        B::Flip(axis(Axis::Vertical)),
        B::SpinIn(SpinInParams {
            deg: deg(360.0),
            cw: true,
        }),
        B::Bounce,
        B::Drop,
        B::Pop,
        B::Boomerang,
        B::Sling,
        B::Whip,
        B::Swish,
        B::Typewriter,
        B::Credits,
        B::DriftAndScale(DirParams { dir: Dir::Left }),
        B::FadeAndMove(DirParams { dir: Dir::Up }),
        B::Random,
        B::Pulse(PulseParams { scale: ratio(1.05) }),
        B::GrowShrink(GrowShrinkParams {
            scale: Scale {
                x: ratio(1.5),
                y: ratio(0.75),
            },
            hold: true,
        }),
        B::Spin(SpinParams {
            deg: deg(90.0),
            cw: false,
            hold: true,
        }),
        B::Teeter(IntensityParams {
            intensity: unit(0.5),
        }),
        B::Jiggle(IntensityParams {
            intensity: unit(0.25),
        }),
        B::Wave(IntensityParams {
            intensity: unit(1.0),
        }),
        B::Blink(RepeatParams { repeat: 3 }),
        B::Flash(RepeatParams { repeat: 1 }),
        B::BoldFlash(RepeatParams { repeat: 2 }),
        B::Transparency(TransparencyParams {
            alpha: unit(0.5),
            hold: false,
        }),
        B::ColorChange(ColorChangeParams {
            property: ColorProperty::Text,
            to: HexColor::rgb(0xff, 0x00, 0x00),
        }),
        B::Path(PathParams {
            d: vec![
                PathSeg::MoveTo(Px::ZERO, Px::ZERO),
                PathSeg::LineTo(px(120.5), px(-40.0)),
                PathSeg::CurveTo(
                    px(130.0),
                    px(-40.0),
                    px(140.0),
                    px(-30.0),
                    px(140.0),
                    px(-20.0),
                ),
                PathSeg::Close,
            ],
            auto_rotate: true,
            hold: true,
        }),
        B::Play(PlayParams { from: Ms(1500) }),
        B::Pause,
        B::Stop,
        B::Toggle,
        B::Behaviors(vec![
            Keyframes::Opacity(track(vec![(0.0, unit(0.0)), (1.0, unit(1.0))])),
            Keyframes::Translate(Track {
                easing: Some(Easing::AccelDecel {
                    accel: unit(0.25),
                    decel: unit(0.25),
                }),
                ..track(vec![
                    (
                        0.0,
                        Offset {
                            x: px(-200.0),
                            y: Px::ZERO,
                        },
                    ),
                    (
                        1.0,
                        Offset {
                            x: Px::ZERO,
                            y: Px::ZERO,
                        },
                    ),
                ])
            }),
            Keyframes::Rotate(track(vec![
                (0.0, deg(-90.0)),
                (0.5, deg(10.0)),
                (1.0, Deg::ZERO),
            ])),
            Keyframes::Scale(track(vec![
                (
                    0.0,
                    Scale {
                        x: Ratio::ZERO,
                        y: ratio(1.0),
                    },
                ),
                (
                    1.0,
                    Scale {
                        x: ratio(1.0),
                        y: ratio(1.0),
                    },
                ),
            ])),
            Keyframes::Blur(track(vec![(0.0, px(8.0)), (1.0, Px::ZERO)])),
            Keyframes::ClipPath(ClipTrack {
                begin_ms: Ms(250),
                duration_ms: Ms(250),
                easing: Some(Easing::Out),
                reveal: Reveal::Wipe(WipeParams {
                    dir: Dir::Right,
                    soft: false,
                }),
                transition: InOut::In,
            }),
            Keyframes::Text(track(vec![
                (0.0, ColorValue::Shift(HslShift::NONE)),
                (
                    0.5,
                    ColorValue::Shift(HslShift {
                        h: deg(60.0),
                        s: Ratio::ZERO,
                        l: ratio(-0.125),
                    }),
                ),
                (1.0, ColorValue::Hex(HexColor::rgb(0x33, 0x33, 0x99))),
            ])),
            Keyframes::Fill(track(vec![
                (0.0, ColorValue::Hex(HexColor::WHITE)),
                (1.0, ColorValue::Hex(HexColor::BLACK)),
            ])),
            Keyframes::Line(track(vec![
                (0.0, ColorValue::Shift(HslShift::NONE)),
                (
                    0.5,
                    ColorValue::Mix(ColorMix {
                        shift: HslShift::NONE,
                        toward: HexColor::BLACK,
                        by: unit(0.5),
                    }),
                ),
                (1.0, ColorValue::Hex(HexColor::BLACK)),
            ])),
            Keyframes::Bold(track(vec![
                (0.0, false),
                (0.5, true),
                (0.6, false),
                (1.0, false),
            ])),
            Keyframes::Italic(track(vec![(0.0, true), (1.0, true)])),
            Keyframes::Underline(track(vec![(0.0, true), (1.0, true)])),
            Keyframes::Set(SetTrack {
                begin_ms: Ms(0),
                duration_ms: Ms(500),
                to: SetValue::Bold(true),
            }),
            Keyframes::Set(SetTrack {
                begin_ms: Ms(100),
                duration_ms: Ms(1),
                to: SetValue::Text(ColorValue::Hex(HexColor::rgb(0x33, 0x33, 0x99))),
            }),
            Keyframes::Set(SetTrack {
                begin_ms: Ms(0),
                duration_ms: Ms(500),
                to: SetValue::Opacity(unit(0.5)),
            }),
        ]),
    ]
}

fn timings() -> Vec<StepTiming> {
    vec![
        StepTiming::default(),
        StepTiming {
            offset_ms: Some(Ms(1200)),
            easing: Some(Easing::Bezier(
                ratio(0.25),
                ratio(0.1),
                ratio(0.25),
                ratio(1.0),
            )),
            stagger: Some(Stagger {
                per_unit: StaggerAmount::Fraction(unit(0.1)),
                order: StaggerOrder::Random(42),
            }),
            repeat: Some(Repeat::Count(ratio(2.5))),
            auto_reverse: true,
            after: Some(AfterEffect::Dim(HexColor::rgb(0x80, 0x80, 0x80))),
        },
        StepTiming {
            stagger: Some(Stagger {
                per_unit: StaggerAmount::Ms(Ms(50)),
                order: StaggerOrder::FromCenter,
            }),
            repeat: Some(Repeat::UntilNextClick),
            after: Some(AfterEffect::Hide),
            easing: Some(Easing::InOut),
            ..StepTiming::default()
        },
        StepTiming {
            repeat: Some(Repeat::UntilEndOfSlide),
            after: Some(AfterEffect::Keep),
            ..StepTiming::default()
        },
    ]
}

fn golden() -> Value {
    let step = json!({
        "target": { "element": "pp-256.title", "paragraphs": [0, 2], "background": true },
        "trigger": "click", "delayMs": 0, "durationMs": 500, "kind": "entrance",
        "effect": {
            "id": "pptx:entr.2", "fallback": "dissolve",
            "native": serde_json::to_value(NativeStep {
                id: NativeId::new(FileFormat::Pptx.into(), "entr.2/4"),
                effect: BuildEffect::FlyIn(FlyInParams { dir: Dir::Up, distance: Distance::OffSlide }),
            }).unwrap()
        },
        "delivery": "byParagraph",
        "timing": { "offsetMs": 500, "easing": "out" }
    });
    let transition = json!({
        "effect": {
            "id": "keynote:apple:move_in", "fallback": "push",
            "native": serde_json::to_value(NativeTransition {
                id: NativeId::new(FileFormat::Keynote.into(), "apple:move in"),
                effect: TransitionEffect::Cover(DirParams { dir: Dir::Left }),
            }).unwrap()
        },
        "durationMs": 1000,
        "advance": { "onClick": true, "afterMs": 2000 },
        "easing": "in-out"
    });
    json!({
        "transitions": transitions(),
        "builds": builds(),
        "timings": timings(),
        "step": step,
        "transition": transition,
    })
}

/// The effect names a ts-rs declaration lists (`{ "effect": "push", … }`).
fn declared_names(declaration: &str) -> BTreeSet<String> {
    declaration
        .split("\"effect\": \"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .map(str::to_owned)
        .collect()
}

fn names(values: &Value) -> BTreeSet<String> {
    values
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value["effect"].as_str().unwrap().to_owned())
        .collect()
}

fn typescript(golden: &Value) -> String {
    let pretty = |value: &Value| serde_json::to_string_pretty(value).unwrap();
    format!(
        "// Generated by slidock-ir's `effects` test from effects.json: the golden effects, typed with the\n\
         // @slidock/types declarations. Do not edit.\n\
         import type {{ BuildEffect, DeckStep, TransitionEffect }} from '../../../../packages/types/src';\n\n\
         export const transitions: TransitionEffect[] = {};\n\n\
         export const builds: BuildEffect[] = {};\n\n\
         export const steps: (BuildEffect & Partial<Pick<DeckStep, 'offsetMs' | 'easing' | 'stagger' | 'repeat' | 'autoReverse' | 'after'>>)[] = {};\n\n\
         // A push moves straight: a diagonal is not a push direction.\n\
         // @ts-expect-error a push takes l, r, u or d\n\
         export const rejected: TransitionEffect = {{ effect: 'push', params: {{ dir: 'lu' }} }};\n",
        pretty(&golden["transitions"]),
        pretty(&golden["builds"]),
        pretty(&Value::Array(
            golden["timings"]
                .as_array()
                .unwrap()
                .iter()
                .zip(golden["builds"].as_array().unwrap())
                .map(|(timing, effect)| {
                    let mut step = effect.as_object().unwrap().clone();
                    step.extend(timing.as_object().unwrap().clone());
                    Value::Object(step)
                })
                .collect()
        )),
    )
}

#[test]
fn every_effect_kind_round_trips() {
    let golden = golden();
    let config = Config::new();
    // The golden covers every kind the declarations name.
    assert_eq!(
        names(&golden["transitions"]),
        declared_names(&TransitionEffect::decl(&config))
    );
    assert_eq!(
        names(&golden["builds"]),
        declared_names(&BuildEffect::decl(&config))
    );
    assert_eq!(golden["transitions"].as_array().unwrap().len(), 38);
    assert_eq!(golden["builds"].as_array().unwrap().len(), 54);

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let json_path = dir.join("effects.json");
    let ts_path = dir.join("effects.ts");
    let text = format!("{}\n", serde_json::to_string_pretty(&golden).unwrap());
    let ts = typescript(&golden);
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&json_path, &text).unwrap();
        std::fs::write(&ts_path, &ts).unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(&json_path).unwrap(),
        text,
        "rerun with UPDATE_GOLDEN=1"
    );
    assert_eq!(
        std::fs::read_to_string(&ts_path).unwrap(),
        ts,
        "rerun with UPDATE_GOLDEN=1"
    );

    // Reading the committed golden back gives the same values and the same bytes, all valid.
    let committed: Value = serde_json::from_str(&text).unwrap();
    let transitions: Vec<TransitionEffect> =
        serde_json::from_value(committed["transitions"].clone()).unwrap();
    let builds: Vec<BuildEffect> = serde_json::from_value(committed["builds"].clone()).unwrap();
    let timings: Vec<StepTiming> = serde_json::from_value(committed["timings"].clone()).unwrap();
    assert_eq!(transitions, self::transitions());
    assert_eq!(builds, self::builds());
    assert_eq!(timings, self::timings());
    for effect in &transitions {
        assert!(
            validate(effect).is_empty(),
            "{effect:?}: {:?}",
            validate(effect)
        );
    }
    for effect in &builds {
        assert!(
            validate(effect).is_empty(),
            "{effect:?}: {:?}",
            validate(effect)
        );
    }
    for timing in &timings {
        assert!(validate(timing).is_empty());
    }
    let step: TimelineStep = serde_json::from_value(committed["step"].clone()).unwrap();
    assert_eq!(serde_json::to_value(&step).unwrap(), committed["step"]);
    assert!(validate(&step).is_empty(), "{:?}", validate(&step));
    let transition: Transition = serde_json::from_value(committed["transition"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(&transition).unwrap(),
        committed["transition"]
    );
    assert!(validate(&transition).is_empty());
}

#[test]
fn effects_print_vocabulary_names_and_params() {
    // The manifest's transition shape.
    let push = TransitionEffect::Push(Dir4Params { dir: Dir4::Left });
    assert_eq!(
        serde_json::to_value(push).unwrap(),
        json!({ "effect": "push", "params": { "dir": "l" } })
    );
    assert_eq!(
        serde_json::to_value(TransitionEffect::Wedge).unwrap(),
        json!({ "effect": "wedge" })
    );
    let fly = NativeStep {
        id: NativeId::new(FileFormat::Pptx.into(), "entr.2/4"),
        effect: BuildEffect::FlyIn(FlyInParams {
            dir: Dir::Up,
            distance: Distance::OffSlide,
        }),
    };
    assert_eq!(
        serde_json::to_value(&fly).unwrap(),
        json!({ "id": "pptx:entr.2/4", "effect": "flyIn", "params": { "dir": "u", "distance": "offSlide" } })
    );
    for (bad, why) in [
        (json!({ "effect": "fly-in" }), "an unknown name"),
        (json!({ "effect": "flyIn" }), "missing params"),
        (
            json!({ "effect": "flyIn", "params": { "dir": "u" } }),
            "a missing param",
        ),
        (
            json!({ "effect": "flyIn", "params": { "dir": "up", "distance": "offSlide" } }),
            "an unknown direction",
        ),
        (
            json!({ "effect": "flyIn", "params": { "dir": "u", "distance": "offSlide", "speed": 2 } }),
            "an unknown param",
        ),
        (
            json!({ "effect": "fade", "params": {}, "x": 1 }),
            "an unknown key",
        ),
        (
            json!({ "effect": "path", "params": { "d": [["Q", 1, 2]], "autoRotate": false, "hold": true } }),
            "an unknown segment",
        ),
        (
            json!({ "effect": "path", "params": { "d": [["M", 1]], "autoRotate": false, "hold": true } }),
            "a short segment",
        ),
        (
            json!({ "effect": "path", "params": { "d": [["M", 1, 2, 3]], "autoRotate": false, "hold": true } }),
            "a long segment",
        ),
    ] {
        assert!(serde_json::from_value::<BuildEffect>(bad).is_err(), "{why}");
    }
    // A switch prints as its own track kind, with the property it switches and the value.
    let set = Keyframes::Set(SetTrack {
        begin_ms: Ms(0),
        duration_ms: Ms(500),
        to: SetValue::Bold(true),
    });
    assert_eq!(
        serde_json::to_value(&set).unwrap(),
        json!({ "property": "set", "beginMs": 0, "durationMs": 500, "to": { "property": "bold", "value": true } })
    );
    for bad in [
        json!({ "property": "set", "beginMs": 0, "durationMs": 500, "to": { "property": "bold", "value": "bold" } }),
        json!({ "property": "set", "beginMs": 0, "durationMs": 500, "to": { "property": "fontSize", "value": 2 } }),
        json!({ "property": "set", "beginMs": 0, "durationMs": 500, "to": { "property": "bold", "value": true }, "frames": [] }),
        json!({ "property": "set", "beginMs": 0, "durationMs": 500 }),
    ] {
        assert!(
            serde_json::from_value::<Keyframes>(bad.clone()).is_err(),
            "{bad}"
        );
    }
    // A key beside `effect` and `params` is the container's to refuse: the IR's native effect does, a deck reader
    // ignores it.
    let unknown_key = json!({ "id": "pptx:entr.2", "effect": "appear", "extra": true });
    assert!(serde_json::from_value::<NativeStep>(unknown_key).is_err());
    let unknown_key = json!({ "id": "pptx:entr.2", "effect": "flyIn", "params": { "dir": "u", "distance": "slight" }, "x": 1 });
    assert!(serde_json::from_value::<NativeStep>(unknown_key).is_err());
    assert!(
        serde_json::from_value::<NativeStep>(json!({ "id": "pptx:entr.2", "effect": "appear" }))
            .is_ok()
    );
}

fn issues(effect: &BuildEffect) -> Vec<String> {
    validate(effect)
        .iter()
        .map(|issue| issue.path.to_string())
        .collect()
}

#[test]
fn validation_bounds_params_per_kind() {
    let wheel = |spokes| BuildEffect::Wheel(WheelParams { spokes, cw: true });
    assert!(issues(&wheel(1)).is_empty());
    assert!(issues(&wheel(8)).is_empty());
    assert_eq!(issues(&wheel(0)), ["spokes"]);
    assert_eq!(issues(&wheel(9)), ["spokes"]);
    let spin = |x| {
        BuildEffect::Spin(SpinParams {
            deg: deg(x),
            cw: true,
            hold: false,
        })
    };
    assert!(issues(&spin(3600.0)).is_empty());
    assert_eq!(issues(&spin(3600.001)), ["deg"]);
    assert_eq!(issues(&spin(0.0)), ["deg"]);
    let blink = |repeat| BuildEffect::Blink(RepeatParams { repeat });
    assert_eq!(issues(&blink(0)), ["repeat"]);
    assert_eq!(issues(&blink(1001)), ["repeat"]);
    let grow = BuildEffect::GrowShrink(GrowShrinkParams {
        scale: Scale {
            x: ratio(101.0),
            y: ratio(-1.0),
        },
        hold: true,
    });
    assert_eq!(issues(&grow), ["scale.x", "scale.y"]);
    let path = |d| {
        BuildEffect::Path(PathParams {
            d,
            auto_rotate: false,
            hold: true,
        })
    };
    assert_eq!(issues(&path(Vec::new())), ["d"]);
    assert_eq!(
        issues(&path(vec![PathSeg::LineTo(Px::ZERO, Px::ZERO)])),
        ["d[0]"]
    );
    assert_eq!(
        issues(&path(vec![PathSeg::MoveTo(Px::ZERO, px(200_001.0))])),
        ["d[0][2]"]
    );
    let long = vec![PathSeg::MoveTo(Px::ZERO, Px::ZERO); 4097];
    assert_eq!(issues(&path(long)), ["d"]);
    assert_eq!(
        issues(&BuildEffect::Behaviors(Vec::new())),
        ["(presentation)"]
    );
    let backwards = BuildEffect::Behaviors(vec![Keyframes::Opacity(track(vec![
        (0.0, unit(0.0)),
        (0.75, unit(0.5)),
        (0.5, unit(0.7)),
        (1.0, unit(1.0)),
    ]))]);
    assert_eq!(issues(&backwards), ["[0].frames[2].offset"]);
    let open_ended = BuildEffect::Behaviors(vec![Keyframes::Opacity(track(vec![
        (0.0, unit(0.0)),
        (0.5, unit(1.0)),
    ]))]);
    assert_eq!(issues(&open_ended), ["[0].frames"]);
    let one = BuildEffect::Behaviors(vec![Keyframes::Opacity(track(vec![(0.0, unit(0.0))]))]);
    assert_eq!(issues(&one), ["[0].frames", "[0].frames"]);
    let late = BuildEffect::Behaviors(vec![Keyframes::Opacity(Track {
        begin_ms: Ms(Ms::MAX),
        ..track(vec![(0.0, unit(0.0)), (1.0, unit(1.0))])
    })]);
    assert_eq!(issues(&late), ["[0].durationMs"]);
    let blurred = BuildEffect::Behaviors(vec![Keyframes::Blur(track(vec![
        (0.0, px(1001.0)),
        (1.0, Px::ZERO),
    ]))]);
    assert_eq!(issues(&blurred), ["[0].frames[0].value"]);
    let shift = |h: f64, s: f64, l: f64| {
        BuildEffect::Behaviors(vec![Keyframes::Fill(track(vec![
            (0.0, ColorValue::Shift(HslShift::NONE)),
            (
                1.0,
                ColorValue::Shift(HslShift {
                    h: deg(h),
                    s: ratio(s),
                    l: ratio(l),
                }),
            ),
        ]))])
    };
    assert!(issues(&shift(-360.0, -1.0, 1.0)).is_empty());
    assert_eq!(
        issues(&shift(360.001, 1.001, -1.001)),
        [
            "[0].frames[1].value.h",
            "[0].frames[1].value.s",
            "[0].frames[1].value.l"
        ]
    );
    let mix = |h: f64| {
        BuildEffect::Behaviors(vec![Keyframes::Text(track(vec![
            (0.0, ColorValue::Shift(HslShift::NONE)),
            (
                0.5,
                ColorValue::Mix(ColorMix {
                    shift: HslShift {
                        h: deg(h),
                        s: Ratio::ZERO,
                        l: Ratio::ZERO,
                    },
                    toward: HexColor::BLACK,
                    by: unit(0.5),
                }),
            ),
            (1.0, ColorValue::Hex(HexColor::BLACK)),
        ]))])
    };
    assert!(issues(&mix(360.0)).is_empty());
    assert_eq!(issues(&mix(360.001)), ["[0].frames[1].value.shift.h"]);
    let switch = BuildEffect::Behaviors(vec![Keyframes::Bold(track(vec![(0.0, true)]))]);
    assert_eq!(issues(&switch), ["[0].frames", "[0].frames"]);
    let set = |begin: u32, to| {
        BuildEffect::Behaviors(vec![Keyframes::Set(SetTrack {
            begin_ms: Ms(begin),
            duration_ms: Ms(500),
            to,
        })])
    };
    assert!(issues(&set(0, SetValue::Underline(false))).is_empty());
    assert_eq!(
        issues(&set(Ms::MAX, SetValue::Bold(true))),
        ["[0].durationMs"]
    );
    let off_hue = SetValue::Line(ColorValue::Shift(HslShift {
        h: deg(400.0),
        s: Ratio::ZERO,
        l: Ratio::ZERO,
    }));
    assert_eq!(issues(&set(0, off_hue)), ["[0].to.value.h"]);
    let fade = |through, color| TransitionEffect::Fade(FadeParams { through, color });
    assert!(validate(&fade(Through::Black, None)).is_empty());
    assert_eq!(
        validate(&fade(Through::Color, None))[0].path.to_string(),
        "color"
    );
    assert_eq!(
        validate(&fade(Through::White, Some(HexColor::BLACK)))[0]
            .path
            .to_string(),
        "color"
    );
    assert!(
        validate(&TransitionEffect::Wheel(WheelParams {
            spokes: 0,
            cw: true
        }))
        .len()
            == 1
    );
}

#[test]
fn a_step_plays_only_effects_of_its_kind() {
    let mut step = golden()["step"].clone();
    let parsed: TimelineStep = serde_json::from_value(step.clone()).unwrap();
    assert!(validate(&parsed).is_empty());
    step["kind"] = json!("media");
    let media: TimelineStep = serde_json::from_value(step.clone()).unwrap();
    let issues = validate(&media);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].path.to_string(), "effect.native");
    step["kind"] = json!("emphasis");
    assert!(validate(&serde_json::from_value::<TimelineStep>(step.clone()).unwrap()).is_empty());
    step["kind"] = json!("motion");
    assert_eq!(
        validate(&serde_json::from_value::<TimelineStep>(step.clone()).unwrap()).len(),
        1
    );
    step["kind"] = json!("entrance");
    step["effect"]["native"]["params"]["dir"] = json!("x");
    assert!(serde_json::from_value::<TimelineStep>(step.clone()).is_err());
    step["effect"]["native"] =
        json!({ "id": "pptx:entr.22", "effect": "wheel", "params": { "spokes": 0, "cw": true } });
    let wheel: TimelineStep = serde_json::from_value(step.clone()).unwrap();
    assert_eq!(
        validate(&wheel)[0].path.to_string(),
        "effect.native.params.spokes"
    );
    step["timing"] = json!({ "repeat": { "count": -1 } });
    step["effect"]["native"] = Value::Null;
    assert!(serde_json::from_value::<TimelineStep>(step.clone()).is_err());
    step["effect"].as_object_mut().unwrap().remove("native");
    let repeat: TimelineStep = serde_json::from_value(step).unwrap();
    assert_eq!(validate(&repeat)[0].path.to_string(), "timing.repeat.count");
}

#[test]
fn steps_and_transitions_without_native_fields_print_as_before() {
    // The native fields are left out when unset, so an IR without them keeps its bytes.
    let step = json!({
        "target": { "element": "ir-s1.title" }, "trigger": "click", "delayMs": 0, "durationMs": 500,
        "kind": "entrance", "effect": { "id": "pptx:fade", "fallback": "dissolve" }, "delivery": "all"
    });
    let parsed: TimelineStep = serde_json::from_value(step.clone()).unwrap();
    assert!(parsed.effect.native.is_none() && parsed.timing.is_none() && !parsed.target.background);
    assert_eq!(serde_json::to_value(&parsed).unwrap(), step);
    let transition =
        json!({ "effect": { "id": "pptx:push", "fallback": "push" }, "durationMs": 700 });
    let parsed: Transition = serde_json::from_value(transition.clone()).unwrap();
    assert_eq!(serde_json::to_value(&parsed).unwrap(), transition);
}
