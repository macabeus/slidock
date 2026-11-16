//! ECMA-376's preset shapes (Part 1, §20.1.10.56 and Annex D, `presetShapeDefinitions.xml`) as paths in a
//! `w` × `h` box. `data/presets.json` holds each preset's adjustment defaults, guides and paths in the standard's own
//! terms; `build.rs` compiles it into [`PRESETS`], with every formula parsed and every name resolved to a slot.

use indexmap::IndexMap;
use slidock_ir::geometry::PresetName;

use crate::arc::Point;
use crate::formula::builtin_values;
use crate::op::Op;
use crate::path::{PathBuilder, ShapePath, scale_segment};

/// A value: a slot (a built-in, `av` or `gd` guide) or a literal.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Operand {
    Slot(u16),
    Lit(f64),
}

/// One guide: its name and formula (missing arguments are 0).
#[derive(Debug)]
pub(crate) struct GuideDef {
    pub(crate) name: &'static str,
    pub(crate) op: Op,
    pub(crate) args: [Operand; 3],
}

/// A path command over operands: `A` is `arcTo wR hR stAng swAng`.
#[derive(Debug)]
pub(crate) enum Command {
    M(Operand, Operand),
    L(Operand, Operand),
    A(Operand, Operand, Operand, Operand),
    Q(Operand, Operand, Operand, Operand),
    C(Operand, Operand, Operand, Operand, Operand, Operand),
    Z,
}

/// How a preset path is filled: `None` paints no fill; the shading variants fill, untinted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PathFill {
    Norm,
    None,
    Lighten,
    LightenLess,
    Darken,
    DarkenLess,
}

#[derive(Debug)]
pub(crate) struct PathDef {
    /// The path's own coordinate space, when it differs from the shape's.
    pub(crate) w: Option<f64>,
    pub(crate) h: Option<f64>,
    pub(crate) fill: PathFill,
    pub(crate) stroke: bool,
    pub(crate) commands: &'static [Command],
}

#[derive(Debug)]
pub(crate) struct PresetDef {
    pub(crate) av: &'static [GuideDef],
    pub(crate) gd: &'static [GuideDef],
    /// Built-ins, `av` and `gd` guides together.
    pub(crate) slots: u16,
    pub(crate) paths: &'static [PathDef],
}

include!(concat!(env!("OUT_DIR"), "/presets.rs"));

fn definition(name: PresetName) -> Option<&'static PresetDef> {
    let index = NAMES.binary_search(&name.as_str()).ok()?;
    PRESETS.get(index)
}

/// The adjustment names of a preset (its `av` guides), in order.
pub fn adjust_names(name: PresetName) -> Vec<&'static str> {
    definition(name)
        .map(|preset| preset.av.iter().map(|guide| guide.name).collect())
        .unwrap_or_default()
}

/// The names of the presets this crate defines, sorted by code unit (the IR's `PRESET_NAMES`).
pub fn defined_names() -> &'static [&'static str] {
    &NAMES
}

struct Slots(Vec<f64>);

impl Slots {
    fn value(&self, operand: Operand) -> f64 {
        match operand {
            // Every slot is defined before it is read: build.rs resolves names in order.
            Operand::Slot(slot) => self.0.get(usize::from(slot)).copied().unwrap_or(f64::NAN),
            Operand::Lit(value) => value,
        }
    }

    fn point(&self, x: Operand, y: Operand) -> Point {
        Point::new(self.value(x), self.value(y))
    }

    fn evaluate(&mut self, guide: &GuideDef, overrides: &IndexMap<String, f64>) {
        let value = match overrides.get(guide.name) {
            Some(&value) => value,
            None => {
                let [x, y, z] = guide.args.map(|arg| self.value(arg));
                guide.op.apply(x, y, z)
            }
        };
        self.0.push(value);
    }
}

/// A path traced in its own coordinate space, then mapped onto the shape's box: an arc's angles are visual angles
/// in the path's space, so a non-uniform scale applied first would end each arc elsewhere.
fn trace(path: &PathDef, slots: &Slots, w: f64, h: f64) -> ShapePath {
    let scale = |own: Option<f64>, size: f64| match own {
        Some(own) if own != 0.0 && !own.is_nan() => size / own,
        _ => 1.0,
    };
    let (sx, sy) = (scale(path.w, w), scale(path.h, h));
    let mut builder = PathBuilder::new();
    for command in path.commands {
        match *command {
            Command::M(x, y) => builder.move_to(slots.point(x, y)),
            Command::L(x, y) => builder.line_to(slots.point(x, y)),
            Command::Q(x1, y1, x, y) => builder.quad_to(slots.point(x1, y1), slots.point(x, y)),
            Command::C(x1, y1, x2, y2, x, y) => {
                builder.cubic_to(slots.point(x1, y1), slots.point(x2, y2), slots.point(x, y))
            }
            Command::A(w_r, h_r, st_ang, sw_ang) => {
                builder.arc_to(
                    slots.value(w_r),
                    slots.value(h_r),
                    slots.value(st_ang),
                    slots.value(sw_ang),
                );
            }
            Command::Z => builder.close(),
        }
    }
    ShapePath {
        segments: builder
            .into_segments()
            .into_iter()
            .map(|segment| scale_segment(segment, sx, sy))
            .collect(),
        fill: path.fill != PathFill::None,
        stroke: path.stroke,
    }
}

/// A preset's paths in a `w` × `h` box, with the adjustments the shape sets over the preset's defaults (a name
/// that is not one of the preset's adjustments is ignored).
pub fn preset_paths(
    name: PresetName,
    w: f64,
    h: f64,
    adjust: &IndexMap<String, f64>,
) -> Vec<ShapePath> {
    // Every preset name has a definition: the table is generated from the same list (`defined_names`).
    let Some(preset) = definition(name) else {
        return Vec::new();
    };
    let mut slots = Slots(Vec::with_capacity(usize::from(preset.slots)));
    slots.0.extend(builtin_values(w, h));
    for guide in preset.av {
        slots.evaluate(guide, adjust);
    }
    let none = IndexMap::new();
    for guide in preset.gd {
        slots.evaluate(guide, &none);
    }
    preset
        .paths
        .iter()
        .map(|path| trace(path, &slots, w, h))
        .collect()
}
