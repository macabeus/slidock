use serde_json::json;

use super::*;
use crate::images::Change;
use crate::motion_lane::deck::Target;
use crate::motion_lane::drive::{PlayerDiagnostic, Sample};

fn step(trigger: &str, kind: &str, element: &str, duration: f64, effect: Option<&str>) -> DeckStep {
    DeckStep {
        target: Target {
            element: element.to_owned(),
            paragraphs: None,
            background: None,
        },
        trigger: trigger.to_owned(),
        kind: kind.to_owned(),
        delivery: Some("all".into()),
        delay_ms: json!(0),
        duration_ms: json!(duration),
        native: format!("test:{element}"),
        fallback: "dissolve".into(),
        effect: effect.map(str::to_owned),
        params: None,
        offset_ms: None,
        repeat: None,
        auto_reverse: None,
        stagger: None,
        after: None,
    }
}

fn animation(node: u64, element: &str, delay: f64, duration: f64) -> Animation {
    Animation {
        node,
        element: Some(element.to_owned()),
        paragraph: None,
        split: false,
        layer: Some("s".into()),
        role: "current".into(),
        whole: false,
        keys: vec!["opacity".into(), "transform".into()],
        delay,
        duration,
        iterations: 1.0,
        direction: None,
        easing: Some("linear".into()),
        fill: Some("both".into()),
        end: Some(delay + duration),
    }
}

thread_local! {
    /// Each test frame's name, by its sha256.
    static NAMES: std::cell::RefCell<BTreeMap<Sha256, String>> =
        const { std::cell::RefCell::new(BTreeMap::new()) };
}

fn sha(name: &str) -> Sha256 {
    let sha = Sha256::of(name);
    NAMES.with(|names| names.borrow_mut().insert(sha.clone(), name.to_owned()));
    sha
}

fn name_of(sha: &Sha256) -> String {
    NAMES.with(|names| names.borrow().get(sha).cloned().unwrap_or_default())
}

/// How far an animation has run at `t`: linear from its delay over its duration.
fn linear(a: &Animation, t: f64) -> f64 {
    if a.duration <= 0.0 {
        return if t >= a.delay { 1.0 } else { 0.0 };
    }
    ((t - a.delay) / a.duration).clamp(0.0, 1.0)
}

/// What an animated node shows at progress `p`: an opacity, a box that stays put.
fn faded(a: &Animation, p: f64) -> NodeProbe {
    NodeProbe {
        node: a.node,
        rect: [100.0, 100.0, 50.0, 50.0],
        pose: BTreeMap::from([("opacity".to_owned(), format!("{p:.3}"))]),
        shown: p > 0.0,
        connected: true,
    }
}

type Pose = dyn Fn(&Animation, f64) -> NodeProbe;

/// The batch of `animations` paused as the walk pauses it (`stage_times`): its frames are named `<tag>/<t>`, but its
/// first is `first` and its last `last`; each node shows `pose` at its progress.
fn sampled(
    steps: &[DeckStep],
    group: u32,
    animations: Vec<Animation>,
    tag: &str,
    (first, last): (&str, &str),
    pose: &Pose,
) -> Batch {
    let duration = animations.iter().filter_map(|a| a.end).fold(0.0, f64::max);
    let times = super::super::drive::stage_times(steps, group, &animations, duration);
    let samples: Vec<Sample> = times
        .iter()
        .map(|t| Sample {
            at_ms: *t,
            frame: if *t < 0.0 {
                sha(first)
            } else if same(*t, duration) {
                sha(last)
            } else {
                sha(&format!("{tag}/{t}"))
            },
            nodes: animations.iter().map(|a| pose(a, linear(a, *t))).collect(),
        })
        .collect();
    let mut batch = Batch {
        animations,
        duration_ms: duration,
        frames: Vec::new(),
        samples,
    };
    batch.frames = [0u16, 500, 1000]
        .iter()
        .filter_map(|pm| Some((*pm, batch.sample_pm(*pm)?.frame.clone())))
        .collect();
    batch
}

/// Sets a batch's frame at per-mille `pm`, in its strip and its pauses.
fn set_frame(batch: &mut Batch, pm: u16, frame: Sha256) {
    let at = super::super::drive::at(0.0, batch.duration_ms, pm);
    for sample in batch
        .samples
        .iter_mut()
        .filter(|s| same(s.at_ms, at) || (at < 0.0 && s.at_ms < 0.0))
    {
        sample.frame = frame.clone();
    }
    for entry in batch.frames.iter_mut().filter(|(p, _)| *p == pm) {
        entry.1 = frame.clone();
    }
}

fn batch(animations: Vec<Animation>, tag: &str) -> Batch {
    sampled(
        &[],
        0,
        animations,
        tag,
        (&format!("{tag}/0"), &format!("{tag}/end")),
        &faded,
    )
}

/// Stage k of `steps` settles on `still/k` and its batch ends there; its batch starts on `still/k-1`.
fn stage_of(
    steps: &[DeckStep],
    k: u32,
    animations: Vec<Animation>,
    visible: Vec<Option<bool>>,
    pose: &Pose,
) -> StageSeen {
    let mut batches = Vec::new();
    let mut settled_nodes = Vec::new();
    if !animations.is_empty() {
        settled_nodes = animations.iter().map(|a| pose(a, 1.0)).collect();
        let first = format!("still/{}", k.saturating_sub(1));
        let last = format!("still/{k}");
        batches.push(sampled(
            steps,
            k,
            animations,
            &format!("mid/{k}"),
            (&first, &last),
            pose,
        ));
    }
    StageSeen {
        batches,
        settled: sha(&format!("still/{k}")),
        settled_nodes,
        still: sha(&format!("still/{k}")),
        visible,
        words: None,
    }
}

fn slide(timeline: Vec<DeckStep>) -> DeckSlide {
    DeckSlide {
        id: "s".into(),
        hidden: false,
        timeline,
        transition: None,
        match_keys: None,
    }
}

fn scores(_: &Sha256, _: &Sha256) -> Option<(f64, f64)> {
    Some((0.9, 1.5))
}

/// Frames whose names start with `aa:` differ from the others only by antialiasing.
fn antialiased(a: &Sha256, b: &Sha256) -> bool {
    a == &sha("aa:still/1") || b == &sha("aa:still/1")
}

/// Two frames differ everywhere in a region unless they are the same frame; a frame named `…@x,y` changed around
/// (x, y).
fn changed(a: &Sha256, b: &Sha256, _: Option<[f64; 4]>, _: u8) -> Option<Change> {
    if a == b {
        return Some(Change {
            pixels: 0,
            center: None,
        });
    }
    let centre = |sha: &Sha256| -> Option<(f64, f64)> {
        let name = name_of(sha);
        let (_, at) = name.rsplit_once('@')?;
        let (x, y) = at.split_once(',')?;
        Some((x.parse().ok()?, y.parse().ok()?))
    };
    Some(Change {
        pixels: 1000,
        center: centre(a).or(centre(b)),
    })
}

fn frames<'a>(
    antialias: &'a dyn Fn(&Sha256, &Sha256) -> bool,
    matches: &'a dyn Fn(&Sha256, &Sha256) -> bool,
) -> Frames<'a> {
    Frames {
        scores: &scores,
        antialiased: antialias,
        matches,
        change: &changed,
        displaced: &undisplaced,
    }
}

fn undisplaced(_: &Sha256, _: &Sha256) -> bool {
    false
}

/// Two click steps on `a` and `b` (a fly-in and a fade), the clean walk of them, and its check.
fn two_clicks() -> (DeckSlide, SlideSeen) {
    let deck = slide(vec![
        step("click", "entrance", "a", 500.0, Some("flyIn")),
        step("withPrevious", "entrance", "b", 700.0, Some("fade")),
    ]);
    let seen = SlideSeen {
        index: 0,
        before: None,
        transition: None,
        stages: vec![
            stage_of(
                &deck.timeline,
                0,
                Vec::new(),
                vec![Some(false), Some(false)],
                &faded,
            ),
            stage_of(
                &deck.timeline,
                1,
                vec![animation(1, "a", 0.0, 500.0), animation(2, "b", 0.0, 700.0)],
                vec![Some(true), Some(true)],
                &faded,
            ),
        ],
        diagnostics: Vec::new(),
        failed: None,
    };
    (deck, seen)
}

fn run(deck: &DeckSlide, seen: &SlideSeen) -> SlideCheck {
    run_with(deck, seen, &frames(&antialiased, &|_, _| false))
}

fn run_with(deck: &DeckSlide, seen: &SlideSeen, frames: &Frames<'_>) -> SlideCheck {
    check_slide(&SlideInput {
        slide: deck,
        seen,
        first: false,
        previous: None,
        next: None,
        frames,
    })
}

#[test]
fn a_walk_that_plays_the_deck_checks_out() {
    let (deck, seen) = two_clicks();
    let checked = run(&deck, &seen);
    assert!(
        checked.fidelity.mismatches.is_empty(),
        "{:?}",
        checked.fidelity.mismatches
    );
    assert!(
        checked.fidelity.order && checked.fidelity.durations_exact && checked.fidelity.delays_exact
    );
    assert_eq!(
        checked.fidelity.steps,
        StepCounts {
            expected: 2,
            played: 2
        }
    );
    assert!(checked.items.iter().all(|i| i.problems.is_empty()));
    assert!(
        checked
            .rows
            .iter()
            .all(|(_, outcome, ..)| *outcome == MotionOutcome::MotionMatch)
    );
    assert_eq!(checked.positions.len(), 2);
    let plays = &checked.fidelity.plays;
    assert_eq!(plays[1].actual.unwrap().duration_ms, 700.0);
    assert_eq!(plays[0].played_in, Some(1));
}

#[test]
fn steps_played_out_of_order_fail() {
    let (mut deck, mut seen) = two_clicks();
    deck.timeline[1].trigger = "afterPrevious".into();
    // The deck has b after a; the player starts b first and a after it.
    seen.stages[1].batches[0].animations = vec![
        animation(1, "a", 700.0, 500.0),
        animation(2, "b", 0.0, 700.0),
    ];
    let checked = run(&deck, &seen);
    assert!(!checked.fidelity.order);
    assert!(
        checked
            .fidelity
            .mismatches
            .iter()
            .any(|m| m.field == TimelineField::Order)
    );
    assert!(checked.items.iter().all(|i| !i.problems.is_empty()));
    assert!(
        checked
            .rows
            .iter()
            .any(|(step, outcome, ..)| *step == RowStep::Stage(1)
                && *outcome == MotionOutcome::MotionMismatch)
    );
}

#[test]
fn an_effect_that_does_not_end_on_the_still_render_fails() {
    let (deck, mut seen) = two_clicks();
    set_frame(&mut seen.stages[1].batches[0], 1000, sha("broken end"));
    let checked = run(&deck, &seen);
    let (_, outcome, metrics, ..) = checked
        .rows
        .iter()
        .find(|(step, ..)| *step == RowStep::Stage(1))
        .unwrap();
    assert_eq!(*outcome, MotionOutcome::MotionMismatch);
    assert_eq!(
        metrics.end_state.as_ref().unwrap().status,
        FrameMatch::Differs
    );
    assert_eq!(metrics.ssim, Some(0.9));
    assert!(
        checked
            .items
            .iter()
            .all(|i| i.problems.iter().any(|p| p.check == "end-state"))
    );
}

#[test]
fn a_step_shown_before_its_click_fails_the_reveal() {
    let (deck, mut seen) = two_clicks();
    seen.stages[0].visible = vec![Some(true), Some(false)];
    let checked = run(&deck, &seen);
    let (_, outcome, metrics, ..) = &checked.rows[0];
    assert_eq!(*outcome, MotionOutcome::MotionMismatch);
    assert_eq!(metrics.reveal.as_ref().unwrap().wrong.len(), 1);
}

#[test]
fn timing_and_group_differences_are_named() {
    let (mut deck, mut seen) = two_clicks();
    deck.timeline[1].delay_ms = json!(100);
    seen.stages[1].batches[0].animations[1].duration = 650.0;
    let checked = run(&deck, &seen);
    let fields: Vec<TimelineField> = checked
        .fidelity
        .mismatches
        .iter()
        .map(|m| m.field)
        .collect();
    assert_eq!(fields, [TimelineField::Start, TimelineField::Duration]);
    // A step the player plays a click early.
    let (deck, mut seen) = two_clicks();
    let b = seen.stages[1].batches[0].animations.pop().unwrap();
    seen.stages[0].batches.push(batch(vec![b], "early"));
    let checked = run(&deck, &seen);
    assert!(
        checked
            .fidelity
            .mismatches
            .iter()
            .any(|m| m.field == TimelineField::Group && m.step == Some(1))
    );
}

#[test]
fn a_reported_fallback_plays_the_fallback() {
    let (deck, mut seen) = two_clicks();
    seen.diagnostics.push(PlayerDiagnostic {
        code: "RT0006".into(),
        message: "slide s step 0: unknown effect flyIn, played dissolve".into(),
    });
    let checked = run(&deck, &seen);
    assert_eq!(checked.items[0].played, Played::Fallback("dissolve".into()));
    assert_eq!(checked.items[1].played, Played::Native("fade".into()));
    assert_eq!(checked.fidelity.effects["test:a"].fallback, 1);
    assert_eq!(checked.rows[1].1, MotionOutcome::MotionFallback);
}

#[test]
fn a_property_outside_the_allow_list_fails() {
    assert!(allowed_key("clipPath") && allowed_key("maskImage") && allowed_key("--sd-color"));
    assert!(!allowed_key("width") && !allowed_key("left") && !allowed_key("--other"));
    let (deck, mut seen) = two_clicks();
    seen.stages[1].batches[0].animations[0]
        .keys
        .push("width".into());
    let checked = run(&deck, &seen);
    assert!(
        checked
            .fidelity
            .mismatches
            .iter()
            .any(|m| m.field == TimelineField::Property && m.actual == "width")
    );
}

/// A transition batch paused at 0, 250, 500, 750 and 1000 ‰, its frames named, its incoming layer (node 9) shown by
/// `pose`.
fn transition_batch(frames: [&str; 5], pose: &Pose) -> Batch {
    let layer = Animation {
        role: "incoming".into(),
        whole: true,
        element: None,
        ..animation(9, "x", 0.0, 750.0)
    };
    let samples = [0u16, 250, 500, 750, 1000]
        .iter()
        .zip(frames)
        .map(|(pm, name)| {
            let t = super::super::drive::at(0.0, 750.0, *pm);
            Sample {
                at_ms: t,
                frame: sha(name),
                nodes: vec![pose(&layer, linear(&layer, t))],
            }
        })
        .collect();
    let mut batch = Batch {
        animations: vec![layer],
        duration_ms: 750.0,
        frames: Vec::new(),
        samples,
    };
    batch.frames = [0u16, 250, 500, 750, 1000]
        .iter()
        .filter_map(|pm| Some((*pm, batch.sample_pm(*pm)?.frame.clone())))
        .collect();
    batch
}

/// A layer pushed in from the right (content moving left): at progress `p` it sits `(1 - p)` of the slide right.
fn from_right(a: &Animation, p: f64) -> NodeProbe {
    let x = (1.0 - p) * 1280.0;
    NodeProbe {
        node: a.node,
        rect: [x, 0.0, 1280.0, 720.0],
        pose: BTreeMap::from([(
            "transform".to_owned(),
            format!("matrix(1, 0, 0, 1, {x}, 0)"),
        )]),
        shown: true,
        connected: true,
    }
}

fn pushed(seen: &mut SlideSeen, deck: &mut DeckSlide, pose: &Pose) {
    deck.transition = Some(DeckTransition {
        native: "pptx:push".into(),
        fallback: "push".into(),
        duration_ms: json!(750),
        effect: Some("push".into()),
        params: Some(json!({"dir": "l"})),
        advance: None,
    });
    seen.before = Some(sha("outgoing"));
    seen.stages[0].settled_nodes.push(pose(
        &Animation {
            role: "incoming".into(),
            whole: true,
            ..animation(9, "x", 0.0, 750.0)
        },
        1.0,
    ));
    seen.transition = Some(transition_batch(
        ["outgoing", "t/250", "t/500", "still/0", "still/0"],
        pose,
    ));
}

#[test]
fn a_transition_runs_from_the_outgoing_slide_to_the_incoming_one() {
    let (mut deck, mut seen) = two_clicks();
    pushed(&mut seen, &mut deck, &from_right);
    let checked = run(&deck, &seen);
    let transitions: Vec<(RowStep, MotionOutcome)> = checked
        .rows
        .iter()
        .filter(|(step, ..)| matches!(step, RowStep::Transition(_)))
        .map(|(step, outcome, ..)| (*step, *outcome))
        .collect();
    assert_eq!(
        transitions,
        [
            (RowStep::Transition(250), MotionOutcome::MotionMatch),
            (RowStep::Transition(500), MotionOutcome::MotionMatch),
            (RowStep::Transition(750), MotionOutcome::MotionClose),
        ]
    );
    let play = checked.fidelity.transition.as_ref().unwrap();
    assert_eq!(play.from.as_ref().unwrap().status, FrameMatch::Identical);
    assert_eq!(play.to.as_ref().unwrap().status, FrameMatch::Identical);
    assert_eq!(play.params.as_deref(), Some(r#"{"dir":"l"}"#));
    assert!(
        checked.items[0].problems.is_empty(),
        "{:?}",
        checked.items[0].problems
    );
    // A transition that ends elsewhere than the incoming slide fails.
    set_frame(seen.transition.as_mut().unwrap(), 1000, sha("elsewhere"));
    let checked = run(&deck, &seen);
    assert!(
        checked.items[0]
            .problems
            .iter()
            .any(|p| p.text.contains("last frame"))
    );
    // A cut plays nothing, and a played one where the deck has a cut fails.
    deck.transition.as_mut().unwrap().effect = Some("cut".into());
    deck.transition.as_mut().unwrap().params = Some(json!({"throughBlack": false}));
    assert!(!transition_animates(deck.transition.as_ref().unwrap()));
    let checked = run(&deck, &seen);
    assert!(!checked.items[0].problems.is_empty());
}

#[test]
fn a_transition_that_jumps_or_moves_the_wrong_way_fails() {
    // Pushed in from the left where the deck says the content moves left.
    let from_left = |a: &Animation, p: f64| {
        let mut probe = from_right(a, p);
        probe.rect[0] = -probe.rect[0];
        probe
    };
    let (mut deck, mut seen) = two_clicks();
    pushed(&mut seen, &mut deck, &from_left);
    let checked = run(&deck, &seen);
    let problems: Vec<(&str, &str)> = checked.items[0]
        .problems
        .iter()
        .map(|p| (p.check.as_str(), p.detail.as_str()))
        .collect();
    assert_eq!(problems, [("direction", "dir l: its content moves r")]);
    assert!(
        checked
            .rows
            .iter()
            .filter(|(step, ..)| matches!(step, RowStep::Transition(_)))
            .all(|(_, outcome, ..)| *outcome == MotionOutcome::MotionMismatch)
    );
    // A jump cut: everything holds its start until the end.
    let jump = |a: &Animation, p: f64| from_right(a, if p < 1.0 { 0.0 } else { 1.0 });
    let (mut deck, mut seen) = two_clicks();
    pushed(&mut seen, &mut deck, &jump);
    for pm in [250, 500, 750] {
        set_frame(seen.transition.as_mut().unwrap(), pm, sha("outgoing"));
    }
    let checked = run(&deck, &seen);
    assert!(checked.items[0].problems.iter().any(|p| p.check == "moves"));
}

#[test]
fn a_magic_move_keyed_with_the_next_slide_plays_on_the_wrong_pair() {
    let keyed = |id: &str, keys: serde_json::Value| DeckSlide {
        id: id.into(),
        match_keys: Some(keys),
        ..slide(Vec::new())
    };
    let (mut deck, mut seen) = two_clicks();
    pushed(&mut seen, &mut deck, &from_right);
    let transition = deck.transition.as_mut().unwrap();
    transition.effect = Some("magicMove".into());
    transition.params = Some(json!({"fadeUnmatched": true}));
    deck.match_keys = Some(json!({"a": "k1", "b": "k2"}));
    let previous = keyed("p", json!({"p1": "k0"}));
    let next = keyed("n", json!({"n1": "k1"}));
    let never = |_: &Sha256, _: &Sha256| false;
    let frames = frames(&antialiased, &never);
    let input = |previous| SlideInput {
        slide: &deck,
        seen: &seen,
        first: false,
        previous,
        next: Some(&next),
        frames: &frames,
    };
    let checked = check_slide(&input(Some(&previous)));
    assert!(
        checked.items[0]
            .problems
            .iter()
            .any(|p| p.check == "placement")
    );
    // Keyed with the slide it leaves, it plays where it belongs.
    let keyed_previous = keyed("p", json!({"p1": "k2"}));
    let checked = check_slide(&input(Some(&keyed_previous)));
    assert!(
        !checked.items[0]
            .problems
            .iter()
            .any(|p| p.check == "placement")
    );
    // The next slide moves in by its own Magic Move: the keys are that move's, and this one matches nothing.
    let mut moving = deck.clone();
    moving.id = "n".into();
    moving.match_keys = Some(json!({"n1": "k1"}));
    let next = moving;
    let input = SlideInput {
        next: Some(&next),
        ..input(Some(&previous))
    };
    let checked = check_slide(&input);
    assert!(
        !checked.items[0]
            .problems
            .iter()
            .any(|p| p.check == "placement")
    );
}

/// A move fades an unmatched incoming box in late, and the box's entrance starts by itself once the move ends: the
/// move ends on the slide as it opens, without the box (Keynote's lead page), not on the slide once the build settled.
#[test]
fn a_transition_ends_on_the_slide_before_the_builds_that_start_by_themselves() {
    let opening = |shown_at_end: bool| {
        let mut deck = slide(vec![step(
            "afterPrevious",
            "entrance",
            "b",
            500.0,
            Some("fade"),
        )]);
        let mut seen = SlideSeen {
            index: 0,
            before: None,
            transition: None,
            stages: vec![stage_of(
                &deck.timeline,
                0,
                vec![animation(2, "b", 0.0, 500.0)],
                vec![Some(true)],
                &faded,
            )],
            diagnostics: Vec::new(),
            failed: None,
        };
        pushed(&mut seen, &mut deck, &from_right);
        let transition = deck.transition.as_mut().unwrap();
        transition.effect = Some("magicMove".into());
        transition.params = Some(json!({"fadeUnmatched": true}));
        let batch = seen.transition.as_mut().unwrap();
        let late = Animation {
            role: "incoming".into(),
            keys: vec!["opacity".into()],
            ..animation(2, "b", 500.0, 250.0)
        };
        for sample in &mut batch.samples {
            let p = if shown_at_end && sample.at_ms >= 750.0 {
                1.0
            } else {
                0.0
            };
            sample.nodes.push(faded(&late, p));
        }
        batch.animations.push(late);
        set_frame(batch, 1000, sha("aa:still/1"));
        let checked = run(&deck, &seen);
        checked.items[0]
            .problems
            .iter()
            .filter(|p| p.detail.contains("last frame"))
            .map(|p| p.detail.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(opening(false), Vec::<String>::new());
    // Shown at the move's end, before its build plays, it fails.
    assert_eq!(
        opening(true),
        ["its last frame is not the incoming slide: b shows at its end, hidden once settled"]
    );
}

/// A move's first frame may draw the matched elements' edges a device pixel from the outgoing slide's (transforms at
/// fractional offsets); any other transition's first frame is the outgoing slide within the antialias bound.
#[test]
fn a_moves_first_frame_may_place_edges_a_pixel_away() {
    let start_failed = |deck: &DeckSlide, seen: &SlideSeen, displaced: bool| {
        let yes = |_: &Sha256, _: &Sha256| displaced;
        let never = |_: &Sha256, _: &Sha256| false;
        let with = Frames {
            displaced: &yes,
            ..frames(&never, &never)
        };
        run_with(deck, seen, &with).items[0]
            .problems
            .iter()
            .any(|p| p.detail == "its first frame is not the outgoing slide")
    };
    let (mut deck, mut seen) = two_clicks();
    pushed(&mut seen, &mut deck, &from_right);
    set_frame(
        seen.transition.as_mut().unwrap(),
        0,
        sha("outgoing, an edge lower"),
    );
    assert!(start_failed(&deck, &seen, true));
    let transition = deck.transition.as_mut().unwrap();
    transition.effect = Some("magicMove".into());
    transition.params = Some(json!({"fadeUnmatched": true}));
    assert!(!start_failed(&deck, &seen, true));
    assert!(start_failed(&deck, &seen, false));
}

#[test]
fn a_paused_end_frame_may_differ_only_by_antialiasing() {
    let (deck, mut seen) = two_clicks();
    set_frame(&mut seen.stages[1].batches[0], 1000, sha("aa:still/1"));
    let checked = run(&deck, &seen);
    let (_, outcome, metrics, ..) = &checked.rows[1];
    assert_eq!(*outcome, MotionOutcome::MotionMatch);
    let end = metrics.end_state.as_ref().unwrap();
    assert_eq!(end.status, FrameMatch::Antialiased);
    assert!(end.status.passes());
    // The settled frame may differ within the antialias bound, never by the static lane's looser match.
    seen.stages[1].settled = sha("aa:still/1");
    let checked = run(&deck, &seen);
    assert_eq!(checked.rows[1].1, MotionOutcome::MotionMatch);
    let only_metrics = |_: &Sha256, _: &Sha256| false;
    let checked = run_with(&deck, &seen, &frames(&only_metrics, &|_, _| true));
    assert_eq!(checked.rows[1].1, MotionOutcome::MotionMismatch);
}

#[test]
fn an_end_frame_whose_nodes_end_away_from_where_they_settle_fails() {
    // A fade that ends at 60 %: its frame differs from the still render by little, as a composited frame may.
    let short = |a: &Animation, p: f64| faded(a, if a.node == 2 { p * 0.6 } else { p });
    let (deck, mut seen) = two_clicks();
    seen.stages[1] = stage_of(
        &deck.timeline,
        1,
        vec![animation(1, "a", 0.0, 500.0), animation(2, "b", 0.0, 700.0)],
        vec![Some(true), Some(true)],
        &short,
    );
    seen.stages[1].settled_nodes = seen.stages[1]
        .settled_nodes
        .iter()
        .map(|p| NodeProbe {
            pose: BTreeMap::from([("opacity".to_owned(), "1.000".to_owned())]),
            ..p.clone()
        })
        .collect();
    set_frame(&mut seen.stages[1].batches[0], 1000, sha("faded end"));
    let all = |_: &Sha256, _: &Sha256| true;
    let checked = run_with(&deck, &seen, &frames(&all, &all));
    let (_, outcome, metrics, ..) = &checked.rows[1];
    assert_eq!(*outcome, MotionOutcome::MotionMismatch);
    assert_eq!(
        metrics.end_state.as_ref().unwrap().status,
        FrameMatch::Differs
    );
    assert_eq!(
        metrics.failures,
        ["end-state: b opacity ends at 0.600, settles at 1.000"]
    );
    // Where every node ends as it settles, the same frames pass as a composited raster.
    let (deck, mut seen) = two_clicks();
    set_frame(&mut seen.stages[1].batches[0], 1000, sha("faded end"));
    let checked = run_with(&deck, &seen, &frames(&|_, _| false, &all));
    assert_eq!(checked.rows[1].1, MotionOutcome::MotionMatch);
}

#[test]
fn an_effect_that_starts_away_from_the_state_before_it_fails() {
    let (deck, mut seen) = two_clicks();
    // Its first frame shows the entrances the stage before it hid, built: as they end.
    set_frame(&mut seen.stages[1].batches[0], 0, sha("still/1"));
    let checked = run(&deck, &seen);
    let (_, outcome, metrics, ..) = &checked.rows[1];
    assert_eq!(metrics.start_state, Some(false));
    assert_eq!(*outcome, MotionOutcome::MotionMismatch);
    assert_eq!(
        metrics.failures,
        ["start-state: a shows built, b shows built"]
    );
    assert!(
        checked
            .items
            .iter()
            .all(|i| i.problems.iter().any(|p| p.check == "start-state"))
    );
    // An entrance may start in view on its own first keyframe (a motion that starts on the slide), not built.
    set_frame(&mut seen.stages[1].batches[0], 0, sha("blink on"));
    let all = |_: &Sha256, _: &Sha256| true;
    let checked = run_with(&deck, &seen, &frames(&all, &all));
    assert_eq!(checked.rows[1].1, MotionOutcome::MotionMatch);
    // Where the entrances end it is within the antialias bound, but the frame as a whole is no composited raster of
    // the state before it.
    // 12 pixels stray in each entrance's box, 5000 in the whole frame.
    let few = |a: &Sha256, b: &Sha256, region: Option<[f64; 4]>, _: u8| {
        Some(Change {
            pixels: match (a == b, region) {
                (true, _) => 0,
                (false, Some(_)) => 12,
                (false, None) => 5000,
            },
            center: None,
        })
    };
    let never = |_: &Sha256, _: &Sha256| false;
    let strict = Frames {
        change: &few,
        ..frames(&never, &never)
    };
    let checked = run_with(&deck, &seen, &strict);
    assert_eq!(checked.rows[1].2.failures, ["start-state: pixels"]);
    // An entrance that starts in view on its first keyframe: what differs is where the entrances are.
    let in_view = |a: &Sha256, b: &Sha256, region: Option<[f64; 4]>, _: u8| {
        Some(Change {
            pixels: if a == b {
                0
            } else if region.is_some() {
                2400
            } else {
                4900
            },
            center: None,
        })
    };
    let starting = Frames {
        change: &in_view,
        ..frames(&never, &never)
    };
    let checked = run_with(&deck, &seen, &starting);
    assert_eq!(
        checked.rows[1].1,
        MotionOutcome::MotionMatch,
        "{:?}",
        checked.rows[1].2.failures
    );
    // Both within bounds: it starts as the stage before left it.
    let lenient = Frames {
        change: &few,
        ..frames(&all, &all)
    };
    let checked = run_with(&deck, &seen, &lenient);
    assert_eq!(checked.rows[1].1, MotionOutcome::MotionMatch);
}

#[test]
fn a_step_that_jumps_to_its_end_does_not_move() {
    // Every node holds its start until its end (an easing of steps(1, end)).
    let jump = |a: &Animation, p: f64| faded(a, if p < 1.0 { 0.0 } else { 1.0 });
    let (deck, mut seen) = two_clicks();
    seen.stages[1] = stage_of(
        &deck.timeline,
        1,
        vec![animation(1, "a", 0.0, 500.0), animation(2, "b", 0.0, 700.0)],
        vec![Some(true), Some(true)],
        &jump,
    );
    let checked = run(&deck, &seen);
    let moves: Vec<Option<u32>> = checked
        .fidelity
        .mismatches
        .iter()
        .filter(|m| m.field == TimelineField::Moves)
        .map(|m| m.step)
        .collect();
    assert_eq!(moves, [Some(0), Some(1)]);
    assert_eq!(checked.rows[1].1, MotionOutcome::MotionMismatch);
    // Keyframes that change nothing.
    let still = |a: &Animation, _: f64| faded(a, 1.0);
    seen.stages[1] = stage_of(
        &deck.timeline,
        1,
        vec![animation(1, "a", 0.0, 500.0), animation(2, "b", 0.0, 700.0)],
        vec![Some(true), Some(true)],
        &still,
    );
    let checked = run(&deck, &seen);
    assert!(
        checked
            .items
            .iter()
            .all(|i| i.problems.iter().any(|p| p.check == "moves"))
    );
    // Poses that move while no frame between the ends shows it: every middle frame is the first.
    let (deck, mut seen) = two_clicks();
    let batch = &mut seen.stages[1].batches[0];
    let first = batch.samples[0].frame.clone();
    for sample in batch.samples.iter_mut().skip(1) {
        if sample.frame != sha("still/1") {
            sample.frame = first.clone();
        }
    }
    let checked = run(&deck, &seen);
    assert!(
        checked
            .fidelity
            .mismatches
            .iter()
            .any(|m| m.field == TimelineField::Moves
                && m.actual == "no frame between its ends differs from both")
    );
    // A step whose frames never change (a transparent shape) moves by its poses alone.
    let (deck, mut seen) = two_clicks();
    let batch = &mut seen.stages[1].batches[0];
    for sample in batch.samples.iter_mut() {
        sample.frame = sha("still/0");
    }
    seen.stages[1].still = sha("still/0");
    seen.stages[1].settled = sha("still/0");
    let checked = run(&deck, &seen);
    assert!(
        !checked
            .fidelity
            .mismatches
            .iter()
            .any(|m| m.field == TimelineField::Moves)
    );
}

#[test]
fn a_build_moves_the_way_its_dir_says() {
    // `a` flies in moving right: from the left of where it ends.
    let flying = |dx: f64| {
        move |a: &Animation, p: f64| {
            let mut probe = faded(a, p);
            if a.node == 1 {
                probe.rect[0] = 100.0 + (1.0 - p) * dx;
            }
            probe
        }
    };
    let mut deck = two_clicks().0;
    deck.timeline[0].params = Some(json!({"dir": "r", "distance": "offSlide"}));
    let walk = |timeline: &[DeckStep], pose: &Pose| {
        let (_, mut seen) = two_clicks();
        seen.stages[1] = stage_of(
            timeline,
            1,
            vec![animation(1, "a", 0.0, 500.0), animation(2, "b", 0.0, 700.0)],
            vec![Some(true), Some(true)],
            pose,
        );
        seen
    };
    let checked = run(&deck, &walk(&deck.timeline, &flying(-200.0)));
    assert!(
        checked.items.iter().all(|i| i.problems.is_empty()),
        "{:?}",
        checked.items
    );
    // Mirrored: it comes from the right.
    let checked = run(&deck, &walk(&deck.timeline, &flying(200.0)));
    let direction: Vec<&Problem> = checked.items[0]
        .problems
        .iter()
        .filter(|p| p.check == "direction")
        .collect();
    assert_eq!(direction.len(), 1);
    assert_eq!(
        direction[0].detail,
        "direction: expected its content moves r, played its content moves l"
    );
    // An exit moving right leaves toward the right of where it starts.
    deck.timeline[0].kind = "exit".into();
    let leaving = |a: &Animation, p: f64| {
        let mut probe = faded(a, p);
        if a.node == 1 {
            probe.rect[0] = 100.0 + p * 200.0;
        }
        probe
    };
    let checked = run(&deck, &walk(&deck.timeline, &leaving));
    assert!(
        !checked.items[0]
            .problems
            .iter()
            .any(|p| p.check == "direction")
    );
    // A wipe keeps its box: where its middle frame changed (around x 110) against the whole (x 125).
    let mut deck = two_clicks().0;
    deck.timeline[0].effect = Some("wipe".into());
    deck.timeline[0].params = Some(json!({"dir": "r", "soft": false}));
    let (_, mut seen) = two_clicks();
    for sample in seen.stages[1].batches[0].samples.iter_mut() {
        if sample.at_ms > 0.0 && sample.at_ms < 500.0 && sample.at_ms != 250.0 {
            continue;
        }
        if sample.at_ms == 250.0 {
            sample.frame = sha("half@110,125");
        }
    }
    seen.stages[1].batches[0].samples.iter_mut().for_each(|s| {
        if s.at_ms >= 500.0 && s.frame != sha("still/1") {
            s.frame = sha(&format!("late/{}@125,125", s.at_ms));
        }
    });
    let ends = seen.stages[1].batches[0]
        .samples
        .iter()
        .find(|s| s.at_ms == 500.0)
        .map(|s| s.frame.clone())
        .unwrap();
    assert_eq!(name_of(&ends), "late/500@125,125");
    let checked = run(&deck, &seen);
    assert!(
        !checked.items[0]
            .problems
            .iter()
            .any(|p| p.check == "direction"),
        "{:?}",
        checked.items[0]
    );
    // Mirrored: its middle frame changed on the right.
    for sample in seen.stages[1].batches[0].samples.iter_mut() {
        if sample.at_ms == 250.0 {
            sample.frame = sha("half@140,125");
        }
    }
    let checked = run(&deck, &seen);
    assert!(
        checked.items[0]
            .problems
            .iter()
            .any(|p| p.check == "direction")
    );
}

/// A tier-2 step on `element` that turns bold on: a set (a switch at its begin), or a track from off to on.
fn bold_step(trigger: &str, element: &str, delay: f64, set: bool) -> DeckStep {
    let mut step = step(trigger, "emphasis", element, 500.0, Some("behaviors"));
    step.delay_ms = json!(delay);
    step.fallback = "none".into();
    let track = if set {
        json!({"property": "set", "beginMs": 0, "durationMs": 500, "to": {"property": "bold", "value": true}})
    } else {
        json!({"property": "bold", "beginMs": 0, "durationMs": 500,
            "frames": [{"offset": 0, "value": false}, {"offset": 1, "value": true}]})
    };
    step.params = Some(json!([track]));
    step
}

fn bold(a: &Animation, on: bool) -> NodeProbe {
    NodeProbe {
        node: a.node,
        rect: [100.0, 100.0 * a.node as f64, 50.0, 50.0],
        pose: BTreeMap::from([(
            "--sd-anim-bold".to_owned(),
            if on { "bold" } else { "normal" }.to_owned(),
        )]),
        shown: true,
        connected: true,
    }
}

/// A walk of `deck`'s first click group playing `animations` over `duration` ms, paused where the lane pauses it;
/// each node shows `pose` (of its first animation) at the batch time, and settles as it shows at 10 s.
fn walk(
    deck: &DeckSlide,
    animations: Vec<Animation>,
    duration: f64,
    pose: &dyn Fn(&Animation, f64) -> NodeProbe,
) -> SlideSeen {
    let mut firsts: Vec<&Animation> = Vec::new();
    for a in &animations {
        if !firsts.iter().any(|f| f.node == a.node) {
            firsts.push(a);
        }
    }
    let probes = |t: f64| -> Vec<NodeProbe> { firsts.iter().map(|a| pose(a, t)).collect() };
    let times = super::super::drive::stage_times(&deck.timeline, 1, &animations, duration);
    let samples: Vec<Sample> = times
        .iter()
        .map(|t| Sample {
            at_ms: *t,
            frame: if *t < 0.0 {
                sha("still/0")
            } else if same(*t, duration) {
                sha("still/1")
            } else {
                sha(&format!("set/{t}"))
            },
            nodes: probes(*t),
        })
        .collect();
    let settled_nodes = probes(10_000.0);
    let mut batch = Batch {
        animations,
        duration_ms: duration,
        frames: Vec::new(),
        samples,
    };
    batch.frames = [0u16, 500, 1000]
        .iter()
        .filter_map(|pm| Some((*pm, batch.sample_pm(*pm)?.frame.clone())))
        .collect();
    let visible = vec![Some(true); deck.timeline.len()];
    let stages = vec![
        stage_of(&deck.timeline, 0, Vec::new(), visible.clone(), &faded),
        StageSeen {
            batches: vec![batch],
            settled: sha("still/1"),
            settled_nodes,
            still: sha("still/1"),
            visible,
            words: None,
        },
    ];
    SlideSeen {
        index: 0,
        before: None,
        transition: None,
        stages,
        diagnostics: Vec::new(),
        failed: None,
    }
}

/// An animation of `keys` on `node`, held after it ends, as the player plays a tier-2 track.
fn held(node: u64, element: &str, delay: f64, duration: f64, keys: &[&str]) -> Animation {
    let mut a = animation(node, element, delay, duration);
    a.keys = keys.iter().map(|k| (*k).to_owned()).collect();
    a.fill = Some("forwards".into());
    a
}

/// A click on `a` that sets it bold and, 200 ms later, `b` with it; each node shows `pose` at the batch time.
fn setting(set: bool, pose: &dyn Fn(&Animation, f64) -> NodeProbe) -> (DeckSlide, SlideSeen) {
    let deck = slide(vec![
        bold_step("click", "a", 0.0, set),
        bold_step("withPrevious", "b", 200.0, set),
    ]);
    let animations = vec![
        held(1, "a", 0.0, 500.0, &["--sd-anim-bold"]),
        held(2, "b", 200.0, 500.0, &["--sd-anim-bold"]),
    ];
    let seen = walk(&deck, animations, 700.0, pose);
    (deck, seen)
}

/// A node's pose of `keys`.
fn posed(a: &Animation, keys: &[(&str, &str)]) -> NodeProbe {
    NodeProbe {
        node: a.node,
        rect: [100.0, 100.0 * a.node as f64, 50.0, 50.0],
        pose: keys
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        shown: true,
        connected: true,
    }
}

/// The mismatches of step `step` but the step-level timing ones of a run.
fn step_mismatches(checked: &SlideCheck, step: u32) -> Vec<(TimelineField, String, String)> {
    checked
        .fidelity
        .mismatches
        .iter()
        .filter(|m| m.step == Some(step))
        .map(|m| (m.field, m.expected.clone(), m.actual.clone()))
        .collect()
}

fn moves_of(checked: &SlideCheck) -> Vec<(Option<u32>, String, String)> {
    checked
        .fidelity
        .mismatches
        .iter()
        .filter(|m| m.field == TimelineField::Moves)
        .map(|m| (m.step, m.expected.clone(), m.actual.clone()))
        .collect()
}

#[test]
fn a_set_is_a_switch_at_its_begin_not_a_move() {
    // Each set switches at its begin and holds: before it, the state before the stage; from it, the set value.
    let switch = |a: &Animation, t: f64| bold(a, t >= a.delay);
    let (deck, seen) = setting(true, &switch);
    // The walk pauses just after each begin.
    let times: Vec<f64> = seen.stages[1].batches[0]
        .samples
        .iter()
        .map(|s| s.at_ms)
        .collect();
    assert!(times.contains(&1.0) && times.contains(&201.0), "{times:?}");
    let checked = run(&deck, &seen);
    assert_eq!(moves_of(&checked), []);
    assert!(
        checked.items.iter().all(|i| i.problems.is_empty()),
        "{:?}",
        checked.items
    );
    assert_eq!(checked.rows[1].1, MotionOutcome::MotionMatch);
    // Negative control: the same walk of tracks that are not sets fails `moves` (no value between their ends).
    let (deck, seen) = setting(false, &switch);
    let checked = run(&deck, &seen);
    let moves = moves_of(&checked);
    assert_eq!(moves.len(), 2, "{moves:?}");
    assert!(
        moves
            .iter()
            .all(|(_, expected, _)| expected == "motion between its ends")
    );
    assert_eq!(checked.rows[1].1, MotionOutcome::MotionMismatch);
}

#[test]
fn a_set_that_switches_late_early_never_or_only_at_its_end_fails() {
    // At its quarter instead of its begin.
    let late = |a: &Animation, t: f64| bold(a, t >= a.delay + 125.0);
    let (deck, seen) = setting(true, &late);
    let moves = moves_of(&run(&deck, &seen));
    assert_eq!(moves.len(), 2, "{moves:?}");
    assert_eq!(moves[0].1, "a switch at its begin");
    assert!(
        moves[0]
            .2
            .starts_with("a --sd-anim-bold is normal at 1 ms, not the bold it switched to at 0 ms"),
        "{}",
        moves[0].2
    );
    // Before its begin: `b` shows bold from the stage's start, 200 ms early.
    let early = |a: &Animation, t: f64| bold(a, t >= 0.0);
    let (deck, seen) = setting(true, &early);
    let moves = moves_of(&run(&deck, &seen));
    assert_eq!(moves.len(), 1, "{moves:?}");
    assert_eq!(moves[0].0, Some(1));
    assert!(
        moves[0].2.starts_with(
            "b --sd-anim-bold is bold at 1 ms, before its switch at 200 ms (it was normal)"
        ),
        "{}",
        moves[0].2
    );
    // Never: the player made the animation and nothing switched.
    let never = |a: &Animation, _: f64| bold(a, false);
    let (deck, seen) = setting(true, &never);
    let checked = run(&deck, &seen);
    let moves = moves_of(&checked);
    assert_eq!(moves.len(), 2, "{moves:?}");
    assert!(
        moves[1]
            .2
            .starts_with("b --sd-anim-bold is normal at 201 ms, not the bold"),
        "{}",
        moves[1].2
    );
    assert_eq!(checked.rows[1].1, MotionOutcome::MotionMismatch);
    // Over its span, switching only at its end.
    let at_end = |a: &Animation, t: f64| bold(a, t >= a.delay + 500.0);
    let (deck, seen) = setting(true, &at_end);
    let moves = moves_of(&run(&deck, &seen));
    assert_eq!(moves.len(), 2, "{moves:?}");
    // A unit that plays no animation of the key its set switches.
    let (deck, mut seen) = setting(true, &|a: &Animation, t: f64| bold(a, t >= a.delay));
    seen.stages[1].batches[0].animations[1].keys = vec!["--sd-anim-italic".into()];
    let moves = moves_of(&run(&deck, &seen));
    assert_eq!(
        moves,
        [(
            Some(1),
            "a switch at its begin".to_owned(),
            "b plays no --sd-anim-bold switch".to_owned()
        )]
    );
}

/// A tier-2 step of `tracks` on `a`, played natively.
fn behaviors(trigger: &str, delay: f64, duration: f64, tracks: Value) -> DeckStep {
    let mut step = step(trigger, "emphasis", "a", duration, Some("behaviors"));
    step.delay_ms = json!(delay);
    step.fallback = "none".into();
    step.params = Some(tracks);
    step
}

#[test]
fn a_switch_and_its_switch_back_each_hold_until_the_next() {
    // A text colour set that does not hold: red for its 500 ms, then back to the element's own black.
    let deck = slide(vec![behaviors(
        "click",
        0.0,
        500.0,
        json!([
            {"property": "set", "beginMs": 0, "durationMs": 500, "to": {"property": "text", "value": "#e0442e"}},
            {"property": "set", "beginMs": 500, "durationMs": 0, "to": {"property": "text", "value": "#000000"}},
        ]),
    )]);
    let key = "--sd-anim-color";
    let animations = || {
        vec![
            held(1, "a", 0.0, 500.0, &[key]),
            held(1, "a", 500.0, 0.0, &[key]),
        ]
    };
    let red = |a: &Animation, t: f64| {
        let red = (0.0..500.0).contains(&t);
        posed(
            a,
            &[(
                key,
                if red {
                    "rgb(224, 68, 46)"
                } else {
                    "rgb(0, 0, 0)"
                },
            )],
        )
    };
    let seen = walk(&deck, animations(), 500.0, &red);
    let checked = run(&deck, &seen);
    assert_eq!(step_mismatches(&checked, 0), []);
    // Negative control: the switch back never comes.
    let stays = |a: &Animation, t: f64| {
        posed(
            a,
            &[(
                key,
                if t >= 0.0 {
                    "rgb(224, 68, 46)"
                } else {
                    "rgb(0, 0, 0)"
                },
            )],
        )
    };
    let seen = walk(&deck, animations(), 500.0, &stays);
    let moves = moves_of(&run(&deck, &seen));
    assert_eq!(moves.len(), 1, "{moves:?}");
    assert!(
        moves[0]
            .2
            .starts_with("a --sd-anim-color is rgb(224, 68, 46) at 501 ms, not the #000000 it switched to at 500 ms"),
        "{}",
        moves[0].2
    );
}

#[test]
fn a_set_after_another_step_on_its_key_starts_from_what_that_step_left() {
    // Step 0 turns `a` bold over its 500 ms; step 1, after it, switches bold off.
    let deck = slide(vec![
        behaviors(
            "click",
            0.0,
            500.0,
            json!([{"property": "bold", "beginMs": 0, "durationMs": 500,
                "frames": [{"offset": 0, "value": false}, {"offset": 0.5, "value": true}, {"offset": 1, "value": true}]}]),
        ),
        behaviors(
            "afterPrevious",
            100.0,
            500.0,
            json!([{"property": "set", "beginMs": 0, "durationMs": 500, "to": {"property": "bold", "value": false}}]),
        ),
    ]);
    let key = "--sd-anim-bold";
    let animations = || {
        vec![
            held(1, "a", 0.0, 500.0, &[key]),
            held(1, "a", 600.0, 500.0, &[key]),
        ]
    };
    let played = |a: &Animation, t: f64| bold(a, (250.0..600.0).contains(&t));
    let seen = walk(&deck, animations(), 1100.0, &played);
    let checked = run(&deck, &seen);
    assert_eq!(step_mismatches(&checked, 1), []);
    // Negative control: it never switches off.
    let never = |a: &Animation, t: f64| bold(a, t >= 250.0);
    let seen = walk(&deck, animations(), 1100.0, &never);
    let moves = moves_of(&run(&deck, &seen));
    assert_eq!(moves.len(), 1, "{moves:?}");
    assert!(
        moves[0].2.starts_with(
            "a --sd-anim-bold is bold at 601 ms, not the normal it switched to at 600 ms"
        ),
        "{}",
        moves[0].2
    );
}

#[test]
fn a_set_may_last_its_track_or_no_time() {
    let deck = slide(vec![bold_step("click", "a", 0.0, true)]);
    let switch = |a: &Animation, t: f64| bold(a, t >= 0.0);
    for length in [500.0, 0.0] {
        let seen = walk(
            &deck,
            vec![held(1, "a", 0.0, length, &["--sd-anim-bold"])],
            500.0,
            &switch,
        );
        let checked = run(&deck, &seen);
        assert_eq!(step_mismatches(&checked, 0), [], "{length}");
    }
    // Negative control: any other length is not the track's.
    let seen = walk(
        &deck,
        vec![held(1, "a", 0.0, 250.0, &["--sd-anim-bold"])],
        500.0,
        &switch,
    );
    assert_eq!(
        step_mismatches(&run(&deck, &seen), 0),
        [(
            TimelineField::Duration,
            "500 or 0".to_owned(),
            "250".to_owned()
        )]
    );
}

#[test]
fn a_set_beside_a_moving_track_is_checked_as_a_switch_and_does_not_count_as_moving() {
    let deck = slide(vec![behaviors(
        "click",
        0.0,
        500.0,
        json!([
            {"property": "set", "beginMs": 0, "durationMs": 500, "to": {"property": "bold", "value": true}},
            {"property": "opacity", "beginMs": 0, "durationMs": 500,
                "frames": [{"offset": 0, "value": 0}, {"offset": 1, "value": 1}]},
        ]),
    )]);
    let animations = || {
        vec![
            held(1, "a", 0.0, 500.0, &["--sd-anim-bold"]),
            held(1, "a", 0.0, 500.0, &["opacity"]),
        ]
    };
    let pose = |bolds: bool, fades: bool| {
        move |a: &Animation, t: f64| {
            let opacity = if fades {
                (t / 500.0).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let opacity = opacity.to_string();
            let weight = if bolds && t >= 0.0 { "bold" } else { "normal" };
            posed(a, &[("--sd-anim-bold", weight), ("opacity", &opacity)])
        }
    };
    let seen = walk(&deck, animations(), 500.0, &pose(true, true));
    assert_eq!(step_mismatches(&run(&deck, &seen), 0), []);
    // Negative control: it fades, and never turns bold.
    let seen = walk(&deck, animations(), 500.0, &pose(false, true));
    let moves = moves_of(&run(&deck, &seen));
    assert_eq!(moves.len(), 1, "{moves:?}");
    assert_eq!(moves[0].1, "a switch at its begin");
}

#[test]
fn a_switch_and_back_does_not_make_a_still_track_move() {
    // Bold on, then off at 375 ms, beside an opacity track that never changes.
    let deck = slide(vec![behaviors(
        "click",
        0.0,
        500.0,
        json!([
            {"property": "set", "beginMs": 0, "durationMs": 375, "to": {"property": "bold", "value": true}},
            {"property": "set", "beginMs": 375, "durationMs": 0, "to": {"property": "bold", "value": false}},
            {"property": "opacity", "beginMs": 0, "durationMs": 500,
                "frames": [{"offset": 0, "value": 0}, {"offset": 1, "value": 1}]},
        ]),
    )]);
    let animations = vec![
        held(1, "a", 0.0, 375.0, &["--sd-anim-bold"]),
        held(1, "a", 375.0, 0.0, &["--sd-anim-bold"]),
        held(1, "a", 0.0, 500.0, &["opacity"]),
    ];
    let still = |a: &Animation, t: f64| {
        let weight = if (0.0..375.0).contains(&t) {
            "bold"
        } else {
            "normal"
        };
        posed(a, &[("--sd-anim-bold", weight), ("opacity", "1")])
    };
    let seen = walk(&deck, animations, 500.0, &still);
    let moves = moves_of(&run(&deck, &seen));
    assert_eq!(
        moves,
        [(
            Some(0),
            "motion between its ends".to_owned(),
            "no property it animates is between its start and end values at a quarter, the middle or three quarters"
                .to_owned()
        )]
    );
}

#[test]
fn a_set_the_lane_cannot_read_fails_closed() {
    let deck = slide(vec![behaviors(
        "click",
        0.0,
        500.0,
        json!([{"property": "set", "beginMs": 0, "durationMs": 500, "to": {"property": "font", "value": "Arial"}}]),
    )]);
    let seen = walk(
        &deck,
        vec![held(1, "a", 0.0, 500.0, &["--sd-anim-bold"])],
        500.0,
        &|a, t| bold(a, t >= 0.0),
    );
    let moves = moves_of(&run(&deck, &seen));
    assert_eq!(moves.len(), 1, "{moves:?}");
    assert!(
        moves[0].2.starts_with("an unknown switch"),
        "{}",
        moves[0].2
    );
}

#[test]
fn a_tier_2_step_plays_each_track_at_its_own_begin_and_length() {
    let (mut deck, _) = two_clicks();
    // `a` plays one track 100 ms into the step, for twice the step's length (a reversal folded into it).
    deck.timeline[0].effect = Some("behaviors".into());
    deck.timeline[0].params = Some(
        json!([{"property": "opacity", "beginMs": 100, "durationMs": 1000,
        "frames": [{"offset": 0, "value": 0}, {"offset": 0.5, "value": 1}, {"offset": 1, "value": 0.5}]}]),
    );
    let walk = |a: Vec<Animation>| SlideSeen {
        index: 0,
        before: None,
        transition: None,
        stages: vec![
            stage_of(
                &deck.timeline,
                0,
                Vec::new(),
                vec![Some(false), Some(false)],
                &faded,
            ),
            stage_of(&deck.timeline, 1, a, vec![Some(true), Some(true)], &faded),
        ],
        diagnostics: Vec::new(),
        failed: None,
    };
    let timing = |seen: &SlideSeen| -> Vec<(TimelineField, String, String)> {
        run(&deck, seen)
            .fidelity
            .mismatches
            .into_iter()
            .filter(|m| m.step == Some(0))
            .map(|m| (m.field, m.expected, m.actual))
            .collect()
    };
    let played = walk(vec![
        animation(1, "a", 100.0, 1000.0),
        animation(2, "b", 0.0, 700.0),
    ]);
    assert_eq!(timing(&played), []);
    // Played over the step's own span instead: neither its begin nor its length is the track's.
    let stretched = walk(vec![
        animation(1, "a", 0.0, 500.0),
        animation(2, "b", 0.0, 700.0),
    ]);
    assert_eq!(
        timing(&stretched),
        [
            (TimelineField::Start, "100".to_owned(), "0".to_owned()),
            (TimelineField::Duration, "1000".to_owned(), "500".to_owned())
        ]
    );
}

#[test]
fn a_small_change_beyond_the_antialias_bound_moves_next_to_a_jump_at_an_end() {
    // A by-character exit: its text changes a little at each pause, and its whole shape hides at its end.
    let walk = |strong: u32| {
        let (deck, seen) = two_clicks();
        let jump = move |a: &Sha256, b: &Sha256, _: Option<[f64; 4]>, tolerance: u8| {
            let end = |s: &Sha256| {
                let name = name_of(s);
                name == "still/1" || name.ends_with("/500")
            };
            let pixels = if a == b {
                0
            } else if end(a) || end(b) {
                100_000
            } else if tolerance == ANTIALIAS_CHANNEL {
                strong
            } else {
                1200
            };
            Some(Change {
                pixels,
                center: None,
            })
        };
        let never = |_: &Sha256, _: &Sha256| false;
        let with = Frames {
            change: &jump,
            ..frames(&never, &never)
        };
        run_with(&deck, &seen, &with)
            .fidelity
            .mismatches
            .into_iter()
            .filter(|m| m.field == TimelineField::Moves)
            .count()
    };
    // 1,200 pixels changed is under a twentieth of the jump, but 400 of them change beyond the antialias channel.
    assert_eq!(walk(400), 0);
    // Within the antialias bound it is a composited layer's noise: no frame between the ends differs from both.
    assert_eq!(walk(100), 2);
}

#[test]
fn a_step_of_sets_that_lasts_no_time_is_checked_as_a_switch() {
    let mut deck = slide(vec![bold_step("click", "a", 0.0, true)]);
    deck.timeline[0].duration_ms = json!(0);
    deck.timeline[0].params = Some(
        json!([{"property": "set", "beginMs": 0, "durationMs": 0, "to": {"property": "bold", "value": true}}]),
    );
    let animations = || vec![held(1, "a", 0.0, 0.0, &["--sd-anim-bold"])];
    let seen = walk(&deck, animations(), 0.0, &|a, t| bold(a, t >= 0.0));
    assert_eq!(step_mismatches(&run(&deck, &seen), 0), []);
    // Negative control: it never switches.
    let seen = walk(&deck, animations(), 0.0, &|a, _| bold(a, false));
    let moves = moves_of(&run(&deck, &seen));
    assert_eq!(moves.len(), 1, "{moves:?}");
}
