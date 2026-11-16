//! The seventeen guide operators (ECMA-376 Part 1, §20.1.9.11). `build.rs` includes this file to parse the
//! preset table, so it depends on nothing.

/// A guide formula's operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    /// `*/ x y z`: x·y / z, 0 when z is 0.
    MulDiv,
    /// `+- x y z`: x + y − z.
    AddSub,
    /// `+/ x y z`: (x + y) / z, 0 when z is 0.
    AddDiv,
    /// `?: x y z`: y when x > 0, else z.
    IfElse,
    Abs,
    /// `at2 x y`: the angle of (x, y).
    At2,
    /// `cat2 x y z`: x·cos(atan2(z, y)).
    Cat2,
    /// `cos x y`: x·cos(y).
    Cos,
    Max,
    Min,
    /// `mod x y z`: √(x² + y² + z²).
    Mod,
    /// `pin x y z`: y within [x, z].
    Pin,
    /// `sat2 x y z`: x·sin(atan2(z, y)).
    Sat2,
    /// `sin x y`: x·sin(y).
    Sin,
    Sqrt,
    /// `tan x y`: x·tan(y).
    Tan,
    /// `val x`: x.
    Val,
}

impl Op {
    pub const ALL: [Op; 17] = [
        Op::MulDiv,
        Op::AddSub,
        Op::AddDiv,
        Op::IfElse,
        Op::Abs,
        Op::At2,
        Op::Cat2,
        Op::Cos,
        Op::Max,
        Op::Min,
        Op::Mod,
        Op::Pin,
        Op::Sat2,
        Op::Sin,
        Op::Sqrt,
        Op::Tan,
        Op::Val,
    ];

    /// The operator as a formula writes it.
    pub fn name(self) -> &'static str {
        match self {
            Op::MulDiv => "*/",
            Op::AddSub => "+-",
            Op::AddDiv => "+/",
            Op::IfElse => "?:",
            Op::Abs => "abs",
            Op::At2 => "at2",
            Op::Cat2 => "cat2",
            Op::Cos => "cos",
            Op::Max => "max",
            Op::Min => "min",
            Op::Mod => "mod",
            Op::Pin => "pin",
            Op::Sat2 => "sat2",
            Op::Sin => "sin",
            Op::Sqrt => "sqrt",
            Op::Tan => "tan",
            Op::Val => "val",
        }
    }

    pub fn parse(name: &str) -> Option<Op> {
        Op::ALL.into_iter().find(|op| op.name() == name)
    }

    /// How many arguments the operator reads; a formula may write more, which are ignored.
    pub fn arity(self) -> usize {
        match self {
            Op::Abs | Op::Sqrt | Op::Val => 1,
            Op::At2 | Op::Cos | Op::Max | Op::Min | Op::Sin | Op::Tan => 2,
            Op::MulDiv
            | Op::AddSub
            | Op::AddDiv
            | Op::IfElse
            | Op::Cat2
            | Op::Mod
            | Op::Pin
            | Op::Sat2 => 3,
        }
    }
}
