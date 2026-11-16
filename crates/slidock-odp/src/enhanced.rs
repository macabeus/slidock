//! A custom shape drawn from its own enhanced geometry (ODF 1.3 Part 3: §19.145 `draw:enhanced-path`, §19.171
//! `draw:formula`, §19.170 `draw:modifiers`, §19.141 `draw:equation`), for the shapes whose `draw:type` maps to no
//! preset: LibreOffice's own shapes (its callouts among them) and drawn ones, which LibreOffice writes with their
//! path and equations. The path is in `svg:viewBox` units, stretched to the frame.
//!
//! Each `N` ends a path; `F` and `S` drop its fill or line. Arcs follow LibreOffice 25.2's drawing of them
//! (`EnhancedCustomShape2d`): `A`/`B` run counter-clockwise on the page and `W`/`V` clockwise, from the ellipse point
//! on the ray to the first radial point to the one on the ray to the second (the same points make a whole turn); `T`
//! and `U` take a centre, two radii and two angles in degrees clockwise from 3 o'clock, and run clockwise between the
//! ellipse's points on those rays, a whole ellipse when the angles agree in whole degrees modulo 360; `X` and `Y` draw
//! a quarter ellipse whose first tangent is horizontal or vertical, alternating with each point. An `if(a, b, c)` is
//! `b` when `a > 0`.
//!
//! Anything the evaluator cannot read (an unknown command or function, a missing equation, a loop between equations,
//! a value that is not a number) leaves the shape unread, with the reason.

use std::f64::consts::PI;

use indexmap::IndexMap;
use slidock_core_api::lowering::LowerError;
use slidock_ir::geometry::{CustomGeometry, CustomPath, CustomTag, FillRule, Geometry, Seg};
use slidock_ir::units::PathUnit;
use slidock_math::{atan, atan2, cos, sin, tan};

use crate::graphic::split_list;
use crate::names::qn;
use crate::odf::{Element, Ns};
use crate::svg_path::Budget;
use crate::units::plain_number;

/// The most equations a shape may hold.
const MAX_EQUATIONS: usize = 1024;
/// How deep a formula may nest, and how long a chain of equations may be.
const MAX_DEPTH: usize = 128;
/// The longest formula read.
const MAX_FORMULA: usize = 4096;
/// The view box LibreOffice gives a shape without one.
const DEFAULT_VIEW_BOX: [f64; 4] = [0.0, 0.0, 21600.0, 21600.0];
/// Coordinates beyond this are not drawn.
const UNIT_LIMIT: f64 = 1e9;

/// A named value a formula or a path can read.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Constant {
    Pi,
    Left,
    Top,
    Right,
    Bottom,
    Width,
    Height,
    XStretch,
    YStretch,
    HasStroke,
    HasFill,
    LogWidth,
    LogHeight,
}

impl Constant {
    fn parse(name: &str) -> Option<Constant> {
        Some(match name {
            "pi" => Constant::Pi,
            "left" => Constant::Left,
            "top" => Constant::Top,
            "right" => Constant::Right,
            "bottom" => Constant::Bottom,
            "width" => Constant::Width,
            "height" => Constant::Height,
            "xstretch" => Constant::XStretch,
            "ystretch" => Constant::YStretch,
            "hasstroke" => Constant::HasStroke,
            "hasfill" => Constant::HasFill,
            "logwidth" => Constant::LogWidth,
            "logheight" => Constant::LogHeight,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Function {
    Abs,
    Sqrt,
    Sin,
    Cos,
    Tan,
    Atan,
    Atan2,
    Min,
    Max,
    If,
}

impl Function {
    fn parse(name: &str) -> Option<(Function, usize)> {
        Some(match name {
            "abs" => (Function::Abs, 1),
            "sqrt" => (Function::Sqrt, 1),
            "sin" => (Function::Sin, 1),
            "cos" => (Function::Cos, 1),
            "tan" => (Function::Tan, 1),
            "atan" => (Function::Atan, 1),
            "atan2" => (Function::Atan2, 2),
            "min" => (Function::Min, 2),
            "max" => (Function::Max, 2),
            "if" => (Function::If, 3),
            _ => return None,
        })
    }
}

/// A parsed formula.
#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Number(f64),
    Modifier(usize),
    Equation(String),
    Constant(Constant),
    Negate(Box<Expr>),
    Binary(u8, Box<Expr>, Box<Expr>),
    Call(Function, Vec<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f64),
    Modifier(usize),
    Equation(String),
    Name(String),
    Symbol(u8),
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The tokens of a formula or a path: numbers, `$n`, `?name`, names and symbols, white space and commas between. In a
/// path every capital letter is a command of its own, and a sign belongs to the number after it.
fn tokens(text: &str, path: bool) -> Result<Vec<Token>, String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    let run = |from: usize, keep: fn(u8) -> bool| {
        bytes
            .get(from..)
            .map_or(0, |rest| rest.iter().take_while(|b| keep(**b)).count())
    };
    while let Some(&b) = bytes.get(at) {
        let signed = path
            && matches!(b, b'-' | b'+')
            && bytes
                .get(at + 1)
                .is_some_and(|b| b.is_ascii_digit() || *b == b'.');
        if b.is_ascii_whitespace() || (path && b == b',') {
            at += 1;
        } else if path && b.is_ascii_uppercase() {
            out.push(Token::Name(char::from(b).to_string()));
            at += 1;
        } else if b.is_ascii_digit() || b == b'.' || signed {
            let digits_from = at + usize::from(signed);
            let mut end = digits_from + run(digits_from, |b| b.is_ascii_digit() || b == b'.');
            if matches!(bytes.get(end), Some(b'e' | b'E')) {
                let sign = usize::from(matches!(bytes.get(end + 1), Some(b'-' | b'+')));
                let digits = run(end + 1 + sign, |b| b.is_ascii_digit());
                if digits > 0 {
                    end += 1 + sign + digits;
                }
            }
            let number = text
                .get(at..end)
                .and_then(|s| s.parse::<f64>().ok())
                .ok_or_else(|| format!("the number {:?} is malformed", text.get(at..end)))?;
            out.push(Token::Number(number));
            at = end;
        } else if b == b'$' {
            let digits = run(at + 1, |b| b.is_ascii_digit());
            let index = text
                .get(at + 1..at + 1 + digits)
                .and_then(|s| s.parse::<usize>().ok())
                .ok_or("a modifier reference has no number")?;
            out.push(Token::Modifier(index));
            at += 1 + digits;
        } else if b == b'?' {
            let length = run(at + 1, is_name_byte);
            let name = text.get(at + 1..at + 1 + length).unwrap_or_default();
            if name.is_empty() {
                return Err("an equation reference has no name".to_owned());
            }
            out.push(Token::Equation(name.to_owned()));
            at += 1 + length;
        } else if b.is_ascii_alphabetic() {
            let length = run(at, is_name_byte);
            out.push(Token::Name(
                text.get(at..at + length).unwrap_or_default().to_owned(),
            ));
            at += length;
        } else if b"+-*/(),".contains(&b) {
            out.push(Token::Symbol(b));
            at += 1;
        } else {
            return Err(format!("the character {:?} is not read", char::from(b)));
        }
    }
    Ok(out)
}

/// A formula's parser: sums of products of signed factors.
struct Parser<'t> {
    tokens: &'t [Token],
    at: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn symbol(&mut self, symbol: u8) -> bool {
        if self.peek() == Some(&Token::Symbol(symbol)) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, symbol: u8) -> Result<(), String> {
        if self.symbol(symbol) {
            Ok(())
        } else {
            Err(format!("{:?} is missing", char::from(symbol)))
        }
    }

    fn sum(&mut self, depth: usize) -> Result<Expr, String> {
        let mut left = self.product(depth)?;
        loop {
            let op = match self.peek() {
                Some(Token::Symbol(op @ (b'+' | b'-'))) => *op,
                _ => return Ok(left),
            };
            self.at += 1;
            let right = self.product(depth)?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn product(&mut self, depth: usize) -> Result<Expr, String> {
        let mut left = self.factor(depth)?;
        loop {
            let op = match self.peek() {
                Some(Token::Symbol(op @ (b'*' | b'/'))) => *op,
                _ => return Ok(left),
            };
            self.at += 1;
            let right = self.factor(depth)?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn factor(&mut self, depth: usize) -> Result<Expr, String> {
        if depth > MAX_DEPTH {
            return Err("a formula nests too deeply".to_owned());
        }
        let Some(token) = self.peek().cloned() else {
            return Err("a formula ends early".to_owned());
        };
        self.at += 1;
        Ok(match token {
            Token::Number(n) => Expr::Number(n),
            Token::Modifier(i) => Expr::Modifier(i),
            Token::Equation(name) => Expr::Equation(name),
            Token::Symbol(b'-') => Expr::Negate(Box::new(self.factor(depth + 1)?)),
            Token::Symbol(b'+') => self.factor(depth + 1)?,
            Token::Symbol(b'(') => {
                let inner = self.sum(depth + 1)?;
                self.expect(b')')?;
                inner
            }
            Token::Symbol(other) => {
                return Err(format!("{:?} is out of place", char::from(other)));
            }
            Token::Name(name) => {
                if let Some(constant) = Constant::parse(&name) {
                    Expr::Constant(constant)
                } else if let Some((function, arity)) = Function::parse(&name) {
                    self.expect(b'(')?;
                    let mut args = Vec::with_capacity(arity);
                    for i in 0..arity {
                        if i > 0 {
                            self.expect(b',')?;
                        }
                        args.push(self.sum(depth + 1)?);
                    }
                    self.expect(b')')?;
                    Expr::Call(function, args)
                } else {
                    return Err(format!(
                        "the name {:?} is not read",
                        slidock_canon::utf16::clip(&name, 32)
                    ));
                }
            }
        })
    }
}

fn parse_formula(text: &str) -> Result<Expr, String> {
    if text.len() > MAX_FORMULA {
        return Err("a formula is too long".to_owned());
    }
    let tokens = tokens(text, false)?;
    let mut parser = Parser {
        tokens: &tokens,
        at: 0,
    };
    let expr = parser.sum(0)?;
    if parser.at != tokens.len() {
        return Err("a formula has more after its end".to_owned());
    }
    Ok(expr)
}

/// What a shape's formulas read: its modifiers, its equations and the constants of its box.
struct Scope {
    modifiers: Vec<f64>,
    names: IndexMap<String, usize>,
    formulas: Vec<Expr>,
    values: Vec<Option<f64>>,
    pending: Vec<bool>,
    view_box: [f64; 4],
    stretch: (f64, f64),
    /// The frame in 1/100 mm.
    logic: (f64, f64),
}

impl Scope {
    fn constant(&self, constant: Constant) -> f64 {
        let [x, y, w, h] = self.view_box;
        match constant {
            Constant::Pi => PI,
            Constant::Left => x,
            Constant::Top => y,
            Constant::Right => x + w,
            Constant::Bottom => y + h,
            Constant::Width => w,
            Constant::Height => h,
            Constant::XStretch => self.stretch.0,
            Constant::YStretch => self.stretch.1,
            Constant::HasStroke | Constant::HasFill => 1.0,
            Constant::LogWidth => self.logic.0,
            Constant::LogHeight => self.logic.1,
        }
    }

    fn equation(&mut self, name: &str, depth: usize) -> Result<f64, String> {
        let Some(&index) = self.names.get(name) else {
            return Err(format!(
                "the equation {:?} is not defined",
                slidock_canon::utf16::clip(name, 32)
            ));
        };
        if let Some(Some(value)) = self.values.get(index) {
            return Ok(*value);
        }
        if depth > MAX_DEPTH || self.pending.get(index).copied().unwrap_or(true) {
            return Err("the equations refer to each other in a loop".to_owned());
        }
        if let Some(flag) = self.pending.get_mut(index) {
            *flag = true;
        }
        let formula = self
            .formulas
            .get(index)
            .cloned()
            .unwrap_or(Expr::Number(0.0));
        let value = self.eval(&formula, depth + 1)?;
        if let Some(slot) = self.values.get_mut(index) {
            *slot = Some(value);
        }
        if let Some(flag) = self.pending.get_mut(index) {
            *flag = false;
        }
        Ok(value)
    }

    fn eval(&mut self, expr: &Expr, depth: usize) -> Result<f64, String> {
        if depth > MAX_DEPTH {
            return Err("a formula nests too deeply".to_owned());
        }
        let value = match expr {
            Expr::Number(n) => *n,
            Expr::Modifier(i) => self.modifiers.get(*i).copied().unwrap_or(0.0),
            Expr::Equation(name) => self.equation(name, depth + 1)?,
            Expr::Constant(constant) => self.constant(*constant),
            Expr::Negate(inner) => -self.eval(inner, depth + 1)?,
            Expr::Binary(op, a, b) => {
                let (a, b) = (self.eval(a, depth + 1)?, self.eval(b, depth + 1)?);
                match op {
                    b'+' => a + b,
                    b'-' => a - b,
                    b'*' => a * b,
                    _ => a / b,
                }
            }
            Expr::Call(function, args) => {
                let mut values = Vec::with_capacity(args.len());
                for arg in args {
                    values.push(self.eval(arg, depth + 1)?);
                }
                let arg = |i: usize| values.get(i).copied().unwrap_or(0.0);
                match function {
                    Function::Abs => arg(0).abs(),
                    Function::Sqrt => arg(0).sqrt(),
                    Function::Sin => sin(arg(0)),
                    Function::Cos => cos(arg(0)),
                    Function::Tan => tan(arg(0)),
                    Function::Atan => atan(arg(0)),
                    Function::Atan2 => atan2(arg(0), arg(1)),
                    Function::Min => slidock_geometry::js_min(arg(0), arg(1)),
                    Function::Max => slidock_geometry::js_max(arg(0), arg(1)),
                    Function::If => {
                        if arg(0) > 0.0 {
                            arg(1)
                        } else {
                            arg(2)
                        }
                    }
                }
            }
        };
        if value.is_finite() {
            Ok(value)
        } else {
            Err("a formula's value is not a number".to_owned())
        }
    }

    /// A path parameter: a number, a modifier, an equation or a constant.
    fn parameter(&mut self, token: &Token) -> Result<f64, String> {
        match token {
            Token::Number(n) => Ok(*n),
            Token::Modifier(i) => Ok(self.modifiers.get(*i).copied().unwrap_or(0.0)),
            Token::Equation(name) => self.equation(name, 0),
            Token::Name(name) => Constant::parse(name)
                .map(|constant| self.constant(constant))
                .ok_or_else(|| {
                    format!(
                        "the parameter {:?} is not read",
                        slidock_canon::utf16::clip(name, 32)
                    )
                }),
            Token::Symbol(b'-') => Err("a path parameter is signed apart".to_owned()),
            Token::Symbol(_) => Err("a path parameter is a symbol".to_owned()),
        }
    }
}

type Point = (f64, f64);

/// The paths of a shape as they are drawn.
struct Builder<'b> {
    paths: Vec<CustomPath>,
    segments: Vec<Seg<f64>>,
    fill: bool,
    stroke: bool,
    current: Option<Point>,
    start: Option<Point>,
    budget: &'b mut Budget,
    view_box: [f64; 4],
}

fn unit(x: f64) -> PathUnit {
    let clamped = slidock_geometry::js_max(-UNIT_LIMIT, slidock_geometry::js_min(UNIT_LIMIT, x));
    PathUnit::quantise(clamped).unwrap_or(PathUnit::ZERO)
}

impl Builder<'_> {
    fn push(&mut self, segment: Seg<f64>) -> Result<(), LowerError> {
        self.budget.spend()?;
        self.segments.push(segment);
        Ok(())
    }

    fn move_to(&mut self, p: Point) -> Result<(), LowerError> {
        self.push(Seg::M(p.0, p.1))?;
        self.current = Some(p);
        self.start = Some(p);
        Ok(())
    }

    /// A line to `p`, or a move when nothing is drawn yet.
    fn line_to(&mut self, p: Point) -> Result<(), LowerError> {
        if self.current.is_none() {
            return self.move_to(p);
        }
        self.push(Seg::L(p.0, p.1))?;
        self.current = Some(p);
        Ok(())
    }

    fn cubic(&mut self, a: Point, b: Point, p: Point) -> Result<(), LowerError> {
        if self.current.is_none() {
            self.move_to(a)?;
        }
        self.push(Seg::C(a.0, a.1, b.0, b.1, p.0, p.1))?;
        self.current = Some(p);
        Ok(())
    }

    fn quadratic(&mut self, a: Point, p: Point) -> Result<(), LowerError> {
        if self.current.is_none() {
            self.move_to(a)?;
        }
        self.push(Seg::Q(a.0, a.1, p.0, p.1))?;
        self.current = Some(p);
        Ok(())
    }

    fn close(&mut self) -> Result<(), LowerError> {
        if self.current.is_some() {
            self.push(Seg::Z)?;
            self.current = self.start;
        }
        Ok(())
    }

    /// Ends a path (`N`): what it drew becomes one of the shape's paths.
    fn end(&mut self) {
        let segments = std::mem::take(&mut self.segments);
        let drawn = segments.iter().any(|s| !matches!(s, Seg::M(..) | Seg::Z));
        if drawn && (self.fill || self.stroke) {
            let [x, y, w, h] = self.view_box;
            let shift = |v: f64, by: f64| unit(v - by);
            self.paths.push(CustomPath {
                w: unit(w),
                h: unit(h),
                fill: self.fill,
                stroke: self.stroke,
                fill_rule: FillRule::Evenodd,
                segments: segments
                    .into_iter()
                    .map(|s| match s {
                        Seg::M(a, b) => Seg::M(shift(a, x), shift(b, y)),
                        Seg::L(a, b) => Seg::L(shift(a, x), shift(b, y)),
                        Seg::Q(a, b, c, d) => {
                            Seg::Q(shift(a, x), shift(b, y), shift(c, x), shift(d, y))
                        }
                        Seg::C(a, b, c, d, e, f) => Seg::C(
                            shift(a, x),
                            shift(b, y),
                            shift(c, x),
                            shift(d, y),
                            shift(e, x),
                            shift(f, y),
                        ),
                        Seg::Z => Seg::Z,
                    })
                    .collect(),
            });
        }
        self.fill = true;
        self.stroke = true;
        self.current = None;
        self.start = None;
    }

    /// The ellipse arc about `centre` with radii `r` from parameter angle `from`, turning by `sweep` (radians; the
    /// angles count counter-clockwise on the page, y down), as cubics of at most a quarter turn each. `join` draws a
    /// line from the current point to the arc's start; otherwise the arc starts a new subpath.
    fn arc(
        &mut self,
        centre: Point,
        r: Point,
        from: f64,
        sweep: f64,
        join: bool,
    ) -> Result<(), LowerError> {
        let at = |t: f64| (centre.0 + r.0 * cos(t), centre.1 - r.1 * sin(t));
        let tangent = |t: f64| (-r.0 * sin(t), -r.1 * cos(t));
        let start = at(from);
        if join {
            self.line_to(start)?;
        } else {
            self.move_to(start)?;
        }
        let pieces = slidock_geometry::js_max(1.0, (sweep.abs() / (PI / 2.0) - 1e-9).ceil());
        let step = sweep / pieces;
        let k = (4.0 / 3.0) * tan(step / 4.0);
        let mut i = 0.0;
        while i < pieces {
            let (a0, a1) = (from + i * step, from + (i + 1.0) * step);
            let (p0, p1) = (at(a0), at(a1));
            let (t0, t1) = (tangent(a0), tangent(a1));
            self.cubic(
                (p0.0 + k * t0.0, p0.1 + k * t0.1),
                (p1.0 - k * t1.0, p1.1 - k * t1.1),
                p1,
            )?;
            i += 1.0;
        }
        Ok(())
    }
}

/// The parameter angle of the ellipse point on the ray from its centre through `p` (counter-clockwise on the page).
fn ray_angle(centre: Point, r: Point, p: Point) -> f64 {
    let direction = atan2(centre.1 - p.1, p.0 - centre.0);
    atan2(r.0 * sin(direction), r.1 * cos(direction))
}

/// The counter-clockwise turn from `from` to `to`, in (0, 2π]: the same angles make a whole turn.
fn ccw_turn(from: f64, to: f64) -> f64 {
    let mut turn = (to - from) % (2.0 * PI);
    if turn <= 0.0 {
        turn += 2.0 * PI;
    }
    turn
}

/// How many parameters a command takes per repetition.
fn arity(command: u8) -> Option<usize> {
    Some(match command {
        b'Z' | b'N' | b'F' | b'S' => 0,
        b'M' | b'L' | b'X' | b'Y' => 2,
        b'Q' => 4,
        b'C' | b'T' | b'U' => 6,
        b'A' | b'B' | b'W' | b'V' => 8,
        _ => return None,
    })
}

/// A shape's geometry from its `draw:enhanced-geometry`, in a frame of `frame` points; `Ok(Err(reason))` when the
/// geometry has no path or one this evaluator cannot read.
pub(crate) fn enhanced_geometry(
    enhanced: &Element,
    frame: (f64, f64),
    budget: &mut Budget,
) -> Result<Result<Geometry, String>, LowerError> {
    let Some(path) = enhanced.attr(qn!("draw:enhanced-path")) else {
        return Ok(Err("it has no enhanced path".to_owned()));
    };
    let mut scope = match scope_of(enhanced, frame) {
        Ok(scope) => scope,
        Err(reason) => return Ok(Err(reason)),
    };
    let tokens = match tokens(path, true) {
        Ok(tokens) => tokens,
        Err(reason) => return Ok(Err(reason)),
    };
    let view_box = scope.view_box;
    let mut builder = Builder {
        paths: Vec::new(),
        segments: Vec::new(),
        fill: true,
        stroke: true,
        current: None,
        start: None,
        budget,
        view_box,
    };
    match draw(&tokens, &mut scope, &mut builder)? {
        Ok(()) => {}
        Err(reason) => return Ok(Err(reason)),
    }
    builder.end();
    if builder.paths.is_empty() {
        return Ok(Err("its path draws nothing".to_owned()));
    }
    Ok(Ok(Geometry::Custom(CustomGeometry {
        kind: CustomTag,
        paths: builder.paths,
    })))
}

fn scope_of(enhanced: &Element, frame: (f64, f64)) -> Result<Scope, String> {
    let view_box = match enhanced.attr(qn!("svg:viewBox")) {
        None => DEFAULT_VIEW_BOX,
        Some(text) => {
            let parts: Option<Vec<f64>> = split_list(slidock_canon::js_trim(text))
                .into_iter()
                .map(|part| plain_number(Some(part)))
                .collect();
            match parts.as_deref() {
                Some(&[x, y, w, h]) if w > 0.0 && h > 0.0 => [x, y, w, h],
                _ => return Err("its view box is malformed".to_owned()),
            }
        }
    };
    let modifiers = split_list(slidock_canon::js_trim(
        enhanced.attr(qn!("draw:modifiers")).unwrap_or(""),
    ))
    .into_iter()
    .map(|part| plain_number(Some(part)).unwrap_or(0.0))
    .collect();
    let mut names = IndexMap::new();
    let mut formulas = Vec::new();
    for equation in enhanced.children_of(Ns::Draw, "equation") {
        if formulas.len() >= MAX_EQUATIONS {
            return Err("it has too many equations".to_owned());
        }
        let name = equation.attr(qn!("draw:name")).unwrap_or("").to_owned();
        let formula = parse_formula(equation.attr(qn!("draw:formula")).unwrap_or(""))?;
        names.insert(name, formulas.len());
        formulas.push(formula);
    }
    let stretch = |name| {
        enhanced
            .attr(name)
            .and_then(|value| plain_number(Some(value)))
            .unwrap_or(0.0)
    };
    // 1/100 mm per point.
    let logic = 2540.0 / 72.0;
    Ok(Scope {
        modifiers,
        values: vec![None; formulas.len()],
        pending: vec![false; formulas.len()],
        names,
        formulas,
        view_box,
        stretch: (
            stretch(qn!("draw:path-stretchpoint-x")),
            stretch(qn!("draw:path-stretchpoint-y")),
        ),
        logic: (frame.0 * logic, frame.1 * logic),
    })
}

/// Draws the path's commands into `builder`.
fn draw(
    tokens: &[Token],
    scope: &mut Scope,
    builder: &mut Builder<'_>,
) -> Result<Result<(), String>, LowerError> {
    let mut at = 0;
    while let Some(token) = tokens.get(at) {
        let command = match token {
            Token::Name(name) if name.len() == 1 => name.as_bytes().first().copied().unwrap_or(0),
            _ => {
                return Ok(Err("a path parameter comes before any command".to_owned()));
            }
        };
        at += 1;
        let Some(count) = arity(command) else {
            return Ok(Err(format!(
                "the path command {:?} is not read",
                char::from(command)
            )));
        };
        let mut values = Vec::new();
        while let Some(token) = tokens.get(at) {
            if matches!(token, Token::Name(name) if name.len() == 1) {
                break;
            }
            match scope.parameter(token) {
                Ok(value) => values.push(value),
                Err(reason) => return Ok(Err(reason)),
            }
            at += 1;
        }
        if count == 0 {
            if let Err(reason) = run(command, builder)? {
                return Ok(Err(reason));
            }
            continue;
        }
        // A command may repeat its parameters; an incomplete repetition (a trailing `M`) draws nothing.
        for (index, group) in values.chunks_exact(count).enumerate() {
            if let Err(reason) = run_group(command, group, index, builder)? {
                return Ok(Err(reason));
            }
        }
    }
    Ok(Ok(()))
}

/// A command without parameters.
fn run(command: u8, builder: &mut Builder<'_>) -> Result<Result<(), String>, LowerError> {
    match command {
        b'Z' => builder.close()?,
        b'N' => builder.end(),
        b'F' => builder.fill = false,
        b'S' => builder.stroke = false,
        other => {
            return Ok(Err(format!(
                "the path command {:?} is not read",
                char::from(other)
            )));
        }
    }
    Ok(Ok(()))
}

/// The `index`th repetition of a command's parameters.
fn run_group(
    command: u8,
    v: &[f64],
    index: usize,
    builder: &mut Builder<'_>,
) -> Result<Result<(), String>, LowerError> {
    let at = |i: usize| v.get(i).copied().unwrap_or(0.0);
    let p = |i: usize| (at(i), at(i + 1));
    match command {
        // Pairs after a move's first are lines.
        b'M' if index == 0 => builder.move_to(p(0))?,
        b'M' | b'L' => builder.line_to(p(0))?,
        b'C' => builder.cubic(p(0), p(2), p(4))?,
        b'Q' => builder.quadratic(p(0), p(2))?,
        b'X' | b'Y' => quadrant((command == b'X') == index.is_multiple_of(2), p(0), builder)?,
        b'T' | b'U' => {
            let (centre, r) = (p(0), (at(2).abs(), at(3).abs()));
            let (from, to) = (at(4), at(5));
            // Whole degrees decide a whole turn, as LibreOffice compares them.
            let whole = (from.trunc() % 360.0) == (to.trunc() % 360.0);
            let (from, sweep) = if whole {
                (PI / 2.0, 2.0 * PI)
            } else {
                // The ends are the ellipse's points on the rays at those angles, clockwise from 3 o'clock on the
                // page; the arc runs clockwise from one to the other.
                let ray = |degrees: f64| {
                    let a = degrees * PI / 180.0;
                    ray_angle(centre, r, (centre.0 + cos(a), centre.1 + sin(a)))
                };
                let (a, b) = (ray(from), ray(to));
                (a, -ccw_turn(b, a))
            };
            builder.arc(centre, r, from, sweep, command == b'T')?;
        }
        b'A' | b'B' | b'W' | b'V' => {
            let (a, b) = (p(0), p(2));
            let centre = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
            let r = ((b.0 - a.0).abs() / 2.0, (b.1 - a.1).abs() / 2.0);
            let (from, to) = (ray_angle(centre, r, p(4)), ray_angle(centre, r, p(6)));
            let join = matches!(command, b'A' | b'W');
            if matches!(command, b'A' | b'B') {
                builder.arc(centre, r, from, ccw_turn(from, to), join)?;
            } else {
                builder.arc(centre, r, from, -ccw_turn(to, from), join)?;
            }
        }
        other => {
            return Ok(Err(format!(
                "the path command {:?} is not read",
                char::from(other)
            )));
        }
    }
    Ok(Ok(()))
}

/// `X`/`Y`: a quarter ellipse from the current point to `p`, leaving horizontally (`horizontal`) or vertically.
fn quadrant(horizontal: bool, p: Point, builder: &mut Builder<'_>) -> Result<(), LowerError> {
    let Some(from) = builder.current else {
        return builder.move_to(p);
    };
    // The corner of the quarter's box: level with the start when it leaves horizontally.
    let corner = if horizontal {
        (p.0, from.1)
    } else {
        (from.0, p.1)
    };
    let k = 4.0 / 3.0 * (2.0_f64.sqrt() - 1.0);
    builder.cubic(
        (
            from.0 + k * (corner.0 - from.0),
            from.1 + k * (corner.1 - from.1),
        ),
        (p.0 + k * (corner.0 - p.0), p.1 + k * (corner.1 - p.1)),
        p,
    )
}
