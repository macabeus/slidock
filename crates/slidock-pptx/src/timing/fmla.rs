//! Animation formulas (ECMA-376 Part 1 §19.5.79 `tav@fmla`, and the `p:strVal` values of `p:anim`): a small infix
//! language over the shape's box and the animation's progress, parsed to a tree and evaluated in Rust. Nothing but
//! this grammar is read, and both the text and the tree are bounded.
//!
//! ```text
//! formula := sum
//! sum     := product (("+" | "-") product)*
//! product := unary (("*" | "/") unary)*
//! unary   := ("-" | "+") unary | power
//! power   := primary ("^" unary)?                      right-associative: 2^3^2 = 2^9
//! primary := number | variable | constant | function "(" sum ("," sum)* ")" | "(" sum ")"
//! number  := digits ["." digits] | "." digits, then an optional exponent "e" ["+" | "-"] digits
//! variable:= "$" | ["#"] ("ppt_x" | "ppt_y" | "ppt_w" | "ppt_h")
//! constant:= "pi" | "e"
//! function:= abs acos asin atan ceil cos cosh deg exp floor ln max min rad rand sin sinh sqrt tan tanh
//! ```
//!
//! `$` is the value being shaped (the interpolated `tav` value, which is the progress 0 to 1 when the values run
//! from 0 to 1); `ppt_x`/`ppt_y` are the shape's centre and `ppt_w`/`ppt_h` its size, as fractions of the slide.
//! `-2^2` is `-(2^2)`. Parentheses, function arguments, signs and exponents nest at most
//! [`FORMULA_DEPTH`] levels. `max` and `min` take two arguments, every other function one. `rand(x)` draws from `[0, x)`
//! with a generator seeded by the caller, so a replay draws the same numbers.

use slidock_ir::effects::limits::{FORMULA_BYTES, FORMULA_DEPTH};

/// Why a formula is not read or cannot be evaluated.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum FormulaError {
    #[error("the formula is longer than {FORMULA_BYTES} bytes")]
    TooLong,
    #[error("the formula nests deeper than {FORMULA_DEPTH} levels")]
    TooDeep,
    #[error("unexpected {0:?} at byte {1}")]
    Unexpected(String, usize),
    #[error("the formula ends early")]
    Truncated,
    #[error("unknown name {0:?}")]
    UnknownName(String),
    #[error("{0} takes {1} argument(s)")]
    Arity(&'static str, usize),
    #[error("the value is not a finite number")]
    NotFinite,
}

/// A variable a formula reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Var {
    /// `$`.
    Value,
    X,
    Y,
    W,
    H,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Func {
    Abs,
    Acos,
    Asin,
    Atan,
    Ceil,
    Cos,
    Cosh,
    Deg,
    Exp,
    Floor,
    Ln,
    Max,
    Min,
    Rad,
    Rand,
    Sin,
    Sinh,
    Sqrt,
    Tan,
    Tanh,
}

impl Func {
    fn parse(name: &str) -> Option<Func> {
        Some(match name {
            "abs" => Func::Abs,
            "acos" => Func::Acos,
            "asin" => Func::Asin,
            "atan" => Func::Atan,
            "ceil" => Func::Ceil,
            "cos" => Func::Cos,
            "cosh" => Func::Cosh,
            "deg" => Func::Deg,
            "exp" => Func::Exp,
            "floor" => Func::Floor,
            "ln" => Func::Ln,
            "max" => Func::Max,
            "min" => Func::Min,
            "rad" => Func::Rad,
            "rand" => Func::Rand,
            "sin" => Func::Sin,
            "sinh" => Func::Sinh,
            "sqrt" => Func::Sqrt,
            "tan" => Func::Tan,
            "tanh" => Func::Tanh,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Func::Abs => "abs",
            Func::Acos => "acos",
            Func::Asin => "asin",
            Func::Atan => "atan",
            Func::Ceil => "ceil",
            Func::Cos => "cos",
            Func::Cosh => "cosh",
            Func::Deg => "deg",
            Func::Exp => "exp",
            Func::Floor => "floor",
            Func::Ln => "ln",
            Func::Max => "max",
            Func::Min => "min",
            Func::Rad => "rad",
            Func::Rand => "rand",
            Func::Sin => "sin",
            Func::Sinh => "sinh",
            Func::Sqrt => "sqrt",
            Func::Tan => "tan",
            Func::Tanh => "tanh",
        }
    }

    fn arity(self) -> usize {
        match self {
            Func::Max | Func::Min => 2,
            _ => 1,
        }
    }
}

/// A parsed formula.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Expr {
    Num(f64),
    Var(Var),
    Neg(Box<Expr>),
    Bin(Op, Box<Expr>, Box<Expr>),
    Call(Func, Vec<Expr>),
}

/// What a formula's variables hold.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Env {
    pub value: f64,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// The generator `rand` draws from: xorshift64*, from a fixed seed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        // xorshift never leaves 0, so the seed is mixed with a non-zero constant.
        Rng(seed ^ 0x9e37_79b9_7f4a_7c15)
    }

    /// A number in `[0, 1)`.
    fn next(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        let bits = x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11;
        bits as f64 / (1u64 << 53) as f64
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Num(f64),
    Name(String),
    Hash(String),
    Dollar,
    Op(char),
    Open,
    Close,
    Comma,
}

fn tokens(text: &str) -> Result<Vec<(Token, usize)>, FormulaError> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let start = i;
        match b {
            b' ' | b'\t' | b'\r' | b'\n' => {
                i += 1;
                continue;
            }
            b'0'..=b'9' | b'.' => {
                let mut end = i;
                let digits = |from: usize| {
                    let mut at = from;
                    while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                        at += 1;
                    }
                    at
                };
                end = digits(end);
                if bytes.get(end) == Some(&b'.') {
                    end = digits(end + 1);
                }
                if matches!(bytes.get(end), Some(b'e' | b'E')) {
                    let sign = usize::from(matches!(bytes.get(end + 1), Some(b'+' | b'-')));
                    if bytes.get(end + 1 + sign).is_some_and(u8::is_ascii_digit) {
                        end = digits(end + 1 + sign);
                    }
                }
                let literal = text.get(start..end).unwrap_or("");
                let value: f64 = literal
                    .parse()
                    .map_err(|_| FormulaError::Unexpected(literal.to_owned(), start))?;
                out.push((Token::Num(value), start));
                i = end;
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'#' => {
                let hash = b == b'#';
                let mut end = i + usize::from(hash);
                while bytes
                    .get(end)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
                {
                    end += 1;
                }
                let name = text.get(start + usize::from(hash)..end).unwrap_or("");
                if name.is_empty() {
                    return Err(FormulaError::Unexpected("#".to_owned(), start));
                }
                out.push((
                    if hash {
                        Token::Hash(name.to_owned())
                    } else {
                        Token::Name(name.to_owned())
                    },
                    start,
                ));
                i = end;
            }
            b'$' => {
                out.push((Token::Dollar, start));
                i += 1;
            }
            b'+' | b'-' | b'*' | b'/' | b'^' => {
                out.push((Token::Op(char::from(b)), start));
                i += 1;
            }
            b'(' => {
                out.push((Token::Open, start));
                i += 1;
            }
            b')' => {
                out.push((Token::Close, start));
                i += 1;
            }
            b',' => {
                out.push((Token::Comma, start));
                i += 1;
            }
            _ => {
                let c = text.get(start..).and_then(|rest| rest.chars().next());
                return Err(FormulaError::Unexpected(
                    c.map(String::from).unwrap_or_default(),
                    start,
                ));
            }
        }
    }
    Ok(out)
}

fn variable(name: &str) -> Option<Var> {
    Some(match name {
        "ppt_x" => Var::X,
        "ppt_y" => Var::Y,
        "ppt_w" => Var::W,
        "ppt_h" => Var::H,
        _ => return None,
    })
}

struct Parser {
    tokens: Vec<(Token, usize)>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at).map(|(token, _)| token)
    }

    fn bump(&mut self) -> Option<(Token, usize)> {
        let token = self.tokens.get(self.at).cloned();
        self.at += 1;
        token
    }

    fn unexpected(&self) -> FormulaError {
        match self.tokens.get(self.at) {
            Some((token, at)) => FormulaError::Unexpected(format!("{token:?}"), *at),
            None => FormulaError::Truncated,
        }
    }

    fn enter(depth: usize) -> Result<usize, FormulaError> {
        if depth >= FORMULA_DEPTH {
            Err(FormulaError::TooDeep)
        } else {
            Ok(depth + 1)
        }
    }

    fn sum(&mut self, depth: usize) -> Result<Expr, FormulaError> {
        let mut left = self.product(depth)?;
        while let Some(Token::Op(op @ ('+' | '-'))) = self.peek() {
            let op = if *op == '+' { Op::Add } else { Op::Sub };
            self.at += 1;
            let right = self.product(depth)?;
            left = Expr::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn product(&mut self, depth: usize) -> Result<Expr, FormulaError> {
        let mut left = self.unary(depth)?;
        while let Some(Token::Op(op @ ('*' | '/'))) = self.peek() {
            let op = if *op == '*' { Op::Mul } else { Op::Div };
            self.at += 1;
            let right = self.unary(depth)?;
            left = Expr::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn unary(&mut self, depth: usize) -> Result<Expr, FormulaError> {
        match self.peek() {
            Some(Token::Op('-')) => {
                self.at += 1;
                Ok(Expr::Neg(Box::new(self.unary(Parser::enter(depth)?)?)))
            }
            Some(Token::Op('+')) => {
                self.at += 1;
                self.unary(Parser::enter(depth)?)
            }
            _ => self.power(depth),
        }
    }

    fn power(&mut self, depth: usize) -> Result<Expr, FormulaError> {
        let base = self.primary(depth)?;
        if let Some(Token::Op('^')) = self.peek() {
            self.at += 1;
            let exponent = self.unary(Parser::enter(depth)?)?;
            return Ok(Expr::Bin(Op::Pow, Box::new(base), Box::new(exponent)));
        }
        Ok(base)
    }

    fn primary(&mut self, depth: usize) -> Result<Expr, FormulaError> {
        let Some((token, at)) = self.bump() else {
            return Err(FormulaError::Truncated);
        };
        match token {
            Token::Num(value) => Ok(Expr::Num(value)),
            Token::Dollar => Ok(Expr::Var(Var::Value)),
            Token::Hash(name) => variable(&name)
                .map(Expr::Var)
                .ok_or(FormulaError::UnknownName(format!("#{name}"))),
            Token::Name(name) => {
                if let Some(var) = variable(&name) {
                    return Ok(Expr::Var(var));
                }
                match name.as_str() {
                    "pi" => return Ok(Expr::Num(std::f64::consts::PI)),
                    "e" => return Ok(Expr::Num(std::f64::consts::E)),
                    _ => {}
                }
                let func = Func::parse(&name).ok_or(FormulaError::UnknownName(name))?;
                if self.bump().map(|(token, _)| token) != Some(Token::Open) {
                    self.at -= 1;
                    return Err(self.unexpected());
                }
                let inner = Parser::enter(depth)?;
                let mut args = vec![self.sum(inner)?];
                while self.peek() == Some(&Token::Comma) {
                    self.at += 1;
                    if args.len() >= func.arity() {
                        return Err(FormulaError::Arity(func.name(), func.arity()));
                    }
                    args.push(self.sum(inner)?);
                }
                if self.bump().map(|(token, _)| token) != Some(Token::Close) {
                    self.at -= 1;
                    return Err(self.unexpected());
                }
                if args.len() != func.arity() {
                    return Err(FormulaError::Arity(func.name(), func.arity()));
                }
                Ok(Expr::Call(func, args))
            }
            Token::Open => {
                let inner = self.sum(Parser::enter(depth)?)?;
                if self.bump().map(|(token, _)| token) != Some(Token::Close) {
                    self.at -= 1;
                    return Err(self.unexpected());
                }
                Ok(inner)
            }
            other => Err(FormulaError::Unexpected(format!("{other:?}"), at)),
        }
    }
}

/// Parses a formula.
pub(crate) fn parse(text: &str) -> Result<Expr, FormulaError> {
    if text.len() > FORMULA_BYTES {
        return Err(FormulaError::TooLong);
    }
    let mut parser = Parser {
        tokens: tokens(text)?,
        at: 0,
    };
    let expr = parser.sum(0)?;
    if parser.at < parser.tokens.len() {
        return Err(parser.unexpected());
    }
    Ok(expr)
}

fn finite(x: f64) -> Result<f64, FormulaError> {
    if x.is_finite() {
        Ok(x)
    } else {
        Err(FormulaError::NotFinite)
    }
}

/// `x^y`: by multiplication for small integer exponents, through `exp(y ln x)` for a positive base.
fn power(x: f64, y: f64) -> Result<f64, FormulaError> {
    if y.fract() == 0.0 && y.abs() <= 64.0 {
        let mut out = 1.0;
        for _ in 0..(y.abs() as u32) {
            out *= x;
        }
        return finite(if y < 0.0 { 1.0 / out } else { out });
    }
    if x > 0.0 {
        return finite(slidock_math::exp(y * slidock_math::log(x)));
    }
    if x == 0.0 && y > 0.0 {
        return Ok(0.0);
    }
    Err(FormulaError::NotFinite)
}

impl Expr {
    /// The formula's value; `rng` serves `rand`.
    pub fn eval(&self, env: &Env, rng: &mut Rng) -> Result<f64, FormulaError> {
        let value = match self {
            Expr::Num(value) => *value,
            Expr::Var(var) => match var {
                Var::Value => env.value,
                Var::X => env.x,
                Var::Y => env.y,
                Var::W => env.w,
                Var::H => env.h,
            },
            Expr::Neg(inner) => -inner.eval(env, rng)?,
            Expr::Bin(op, left, right) => {
                let (a, b) = (left.eval(env, rng)?, right.eval(env, rng)?);
                match op {
                    Op::Add => a + b,
                    Op::Sub => a - b,
                    Op::Mul => a * b,
                    Op::Div => a / b,
                    Op::Pow => power(a, b)?,
                }
            }
            Expr::Call(func, args) => {
                let mut values = Vec::with_capacity(args.len());
                for arg in args {
                    values.push(arg.eval(env, rng)?);
                }
                let x = values.first().copied().unwrap_or(f64::NAN);
                let y = values.get(1).copied().unwrap_or(f64::NAN);
                let exp = slidock_math::exp;
                match func {
                    Func::Abs => x.abs(),
                    Func::Acos => slidock_math::acos(x),
                    Func::Asin => slidock_math::asin(x),
                    Func::Atan => slidock_math::atan(x),
                    Func::Ceil => x.ceil(),
                    Func::Cos => slidock_math::cos(x),
                    Func::Cosh => (exp(x) + exp(-x)) / 2.0,
                    Func::Deg => x * 180.0 / std::f64::consts::PI,
                    Func::Exp => exp(x),
                    Func::Floor => x.floor(),
                    Func::Ln if x > 0.0 => slidock_math::log(x),
                    Func::Ln => f64::NAN,
                    Func::Max => x.max(y),
                    Func::Min => x.min(y),
                    Func::Rad => x * std::f64::consts::PI / 180.0,
                    Func::Rand => x * rng.next(),
                    Func::Sin => slidock_math::sin(x),
                    Func::Sinh => (exp(x) - exp(-x)) / 2.0,
                    Func::Sqrt => x.sqrt(),
                    Func::Tan => slidock_math::tan(x),
                    Func::Tanh => {
                        let (a, b) = (exp(x), exp(-x));
                        (a - b) / (a + b)
                    }
                }
            }
        };
        finite(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval_with(text: &str, env: Env) -> f64 {
        parse(text).unwrap().eval(&env, &mut Rng::new(1)).unwrap()
    }

    fn eval(text: &str) -> f64 {
        eval_with(text, Env::default())
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn reads_numbers_variables_and_constants() {
        assert_eq!(eval("12"), 12.0);
        assert_eq!(eval("1.5"), 1.5);
        assert_eq!(eval(".25"), 0.25);
        assert_eq!(eval("2e3"), 2000.0);
        assert_eq!(eval("2.5E-1"), 0.25);
        assert!(close(eval("pi"), std::f64::consts::PI));
        assert!(close(eval("e"), std::f64::consts::E));
        // `2e` is 2 times nothing: an exponent needs digits, so `e` here is a stray name.
        assert!(parse("2e").is_err());
        let env = Env {
            value: 0.25,
            x: 0.5,
            y: 0.4,
            w: 0.2,
            h: 0.1,
        };
        assert_eq!(eval_with("$", env), 0.25);
        assert_eq!(eval_with("#ppt_x", env), 0.5);
        assert_eq!(eval_with("ppt_y", env), 0.4);
        assert_eq!(eval_with("#ppt_w", env), 0.2);
        assert_eq!(eval_with("#ppt_h", env), 0.1);
        assert!(close(eval_with("0-#ppt_w/2", env), -0.1));
        assert!(close(eval_with("1+#ppt_h/2", env), 1.05));
    }

    #[test]
    fn follows_precedence_and_associativity() {
        assert_eq!(eval("1+2*3"), 7.0);
        assert_eq!(eval("(1+2)*3"), 9.0);
        assert_eq!(eval("8/4/2"), 1.0);
        assert_eq!(eval("10-4-3"), 3.0);
        assert_eq!(eval("2^3^2"), 512.0);
        assert_eq!(eval("-2^2"), -4.0);
        assert_eq!(eval("2^-1"), 0.5);
        assert_eq!(eval("--3"), 3.0);
        assert_eq!(eval("+3"), 3.0);
        assert_eq!(eval("2*-3"), -6.0);
        assert_eq!(eval(" 1 +\t2 "), 3.0);
    }

    #[test]
    fn evaluates_every_function() {
        let pi = std::f64::consts::PI;
        for (text, expected) in [
            ("abs(-2)", 2.0),
            ("acos(1)", 0.0),
            ("asin(0)", 0.0),
            ("atan(0)", 0.0),
            ("ceil(1.2)", 2.0),
            ("cos(0)", 1.0),
            ("cosh(0)", 1.0),
            ("deg(pi)", 180.0),
            ("exp(0)", 1.0),
            ("floor(1.8)", 1.0),
            ("ln(e)", 1.0),
            ("max(2,3)", 3.0),
            ("min(2,3)", 2.0),
            ("rad(180)", pi),
            ("sin(0)", 0.0),
            ("sinh(0)", 0.0),
            ("sqrt(9)", 3.0),
            ("tan(0)", 0.0),
            ("tanh(0)", 0.0),
            ("2^0.5", std::f64::consts::SQRT_2),
        ] {
            assert!(close(eval(text), expected), "{text}");
        }
        assert!(close(eval("sin(pi/2)"), 1.0));
        assert!(close(
            eval("cosh(1)"),
            (slidock_math::exp(1.0) + slidock_math::exp(-1.0)) / 2.0
        ));
    }

    #[test]
    fn rand_is_seeded_and_bounded() {
        let formula = parse("rand(10)").unwrap();
        let draw = |seed| {
            let mut rng = Rng::new(seed);
            (0..5)
                .map(|_| formula.eval(&Env::default(), &mut rng).unwrap())
                .collect::<Vec<_>>()
        };
        let first = draw(7);
        assert_eq!(first, draw(7));
        assert_ne!(first, draw(8));
        assert!(first.iter().all(|x| (0.0..10.0).contains(x)));
        assert_ne!(first[0], first[1]);
    }

    #[test]
    fn evaluates_powerpoint_preset_formulas() {
        // PowerPoint's Spiral In and Bounce write formulas like these.
        let env = Env {
            value: 0.5,
            x: 0.5,
            y: 0.5,
            w: 0.2,
            h: 0.2,
        };
        let spiral = "#ppt_x+(cos(-2*pi*(1-$))*-#ppt_x-sin(-2*pi*(1-$))*(1-#ppt_y))*(1-$)";
        assert!(eval_with(spiral, env).is_finite());
        assert!(close(eval_with("#ppt_y-sin(pi*$)/3", env), 0.5 - 1.0 / 3.0));
    }

    #[test]
    fn refuses_anything_else() {
        for (text, error) in [
            ("", FormulaError::Truncated),
            ("1+", FormulaError::Truncated),
            ("(1", FormulaError::Truncated),
            ("foo", FormulaError::UnknownName("foo".into())),
            ("#foo", FormulaError::UnknownName("#foo".into())),
            ("PPT_X", FormulaError::UnknownName("PPT_X".into())),
            ("max(1)", FormulaError::Arity("max", 2)),
            ("sin(1,2)", FormulaError::Arity("sin", 1)),
            ("1;2", FormulaError::Unexpected(";".into(), 1)),
            ("1 2", FormulaError::Unexpected("Num(2.0)".into(), 2)),
            ("eval(1)", FormulaError::UnknownName("eval".into())),
            ("1%2", FormulaError::Unexpected("%".into(), 1)),
            ("sin 1", FormulaError::Unexpected("Num(1.0)".into(), 4)),
            ("#", FormulaError::Unexpected("#".into(), 0)),
            ("\u{e9}", FormulaError::Unexpected("\u{e9}".into(), 0)),
        ] {
            assert_eq!(parse(text), Err(error), "{text:?}");
        }
        for text in [
            "1/0",
            "ln(0)",
            "sqrt(-1)",
            "acos(2)",
            "(-8)^0.5",
            "exp(1000)",
        ] {
            assert_eq!(
                parse(text).unwrap().eval(&Env::default(), &mut Rng::new(1)),
                Err(FormulaError::NotFinite),
                "{text}"
            );
        }
    }

    #[test]
    fn bounds_length_and_depth() {
        let long = format!("1{}", "+1".repeat(FORMULA_BYTES / 2));
        assert_eq!(parse(&long), Err(FormulaError::TooLong));
        let fits = format!("1{}", "+1".repeat(FORMULA_BYTES / 2 - 1));
        assert_eq!(fits.len(), FORMULA_BYTES - 1);
        assert_eq!(eval(&fits), (FORMULA_BYTES / 2) as f64);
        // Parentheses, function arguments, unary signs and exponents nest; a chain of binary operators does not.
        let nested = |n: usize| format!("{}$/2{}", "(".repeat(n), ")".repeat(n));
        assert!(parse(&nested(FORMULA_DEPTH)).is_ok());
        assert_eq!(
            parse(&nested(FORMULA_DEPTH + 1)),
            Err(FormulaError::TooDeep)
        );
        assert_eq!(parse(&nested(400)), Err(FormulaError::TooDeep));
        let unary = format!("{}1", "-".repeat(64));
        assert_eq!(parse(&unary), Err(FormulaError::TooDeep));
        let calls = format!("{}1{}", "abs(".repeat(40), ")".repeat(40));
        assert_eq!(parse(&calls), Err(FormulaError::TooDeep));
        // A long flat chain is not deep.
        assert!(parse(&format!("1{}", "*1".repeat(300))).is_ok());
    }
}
