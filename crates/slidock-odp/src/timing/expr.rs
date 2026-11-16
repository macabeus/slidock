//! LibreOffice's value expressions (`smil:values="1+height/2;y"`, `anim:formula="x+sin(2*pi*$)*0.05"`): numbers,
//! the shape's `x`, `y`, `width` and `height` (fractions of the slide), `$` (the animated value or the progress),
//! `pi` and `e`, the four operators, unary signs, and the functions `abs sqrt sin cos tan asin acos atan exp log`
//! (one argument) and `min max` (two). An expression is bounded in bytes ([`FORMULA_BYTES`]) and in nesting
//! ([`FORMULA_DEPTH`]); past either it does not parse. Transcendental functions go through fdlibm.

use slidock_ir::effects::limits::{FORMULA_BYTES, FORMULA_DEPTH};

/// Why an expression does not parse or evaluate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExprError {
    TooLong,
    TooDeep,
    Syntax,
    /// It evaluates to NaN or an infinity.
    NotFinite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Var {
    X,
    Y,
    Width,
    Height,
    Dollar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Func {
    Abs,
    Sqrt,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Exp,
    Log,
    Min,
    Max,
}

impl Func {
    fn named(name: &str) -> Option<Func> {
        Some(match name {
            "abs" => Func::Abs,
            "sqrt" => Func::Sqrt,
            "sin" => Func::Sin,
            "cos" => Func::Cos,
            "tan" => Func::Tan,
            "asin" => Func::Asin,
            "acos" => Func::Acos,
            "atan" => Func::Atan,
            "exp" => Func::Exp,
            "log" => Func::Log,
            "min" => Func::Min,
            "max" => Func::Max,
            _ => return None,
        })
    }

    fn binary(self) -> bool {
        matches!(self, Func::Min | Func::Max)
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Num(f64),
    Var(Var),
    Neg(Box<Node>),
    Add(Box<Node>, Box<Node>),
    Sub(Box<Node>, Box<Node>),
    Mul(Box<Node>, Box<Node>),
    Div(Box<Node>, Box<Node>),
    Call(Func, Box<Node>, Option<Box<Node>>),
}

/// A parsed expression.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Expr(Node);

/// The values an expression reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Vars {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub dollar: f64,
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
    depth: usize,
}

impl Parser<'_> {
    fn skip_space(&mut self) {
        while self.bytes.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_space();
        self.bytes.get(self.at).copied()
    }

    fn eat(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn nested<T>(
        &mut self,
        parse: impl FnOnce(&mut Self) -> Result<T, ExprError>,
    ) -> Result<T, ExprError> {
        self.depth += 1;
        if self.depth > FORMULA_DEPTH {
            return Err(ExprError::TooDeep);
        }
        let out = parse(self);
        self.depth -= 1;
        out
    }

    fn sum(&mut self) -> Result<Node, ExprError> {
        let mut left = self.product()?;
        loop {
            if self.eat(b'+') {
                left = Node::Add(Box::new(left), Box::new(self.product()?));
            } else if self.eat(b'-') {
                left = Node::Sub(Box::new(left), Box::new(self.product()?));
            } else {
                return Ok(left);
            }
        }
    }

    fn product(&mut self) -> Result<Node, ExprError> {
        let mut left = self.unary()?;
        loop {
            if self.eat(b'*') {
                left = Node::Mul(Box::new(left), Box::new(self.unary()?));
            } else if self.eat(b'/') {
                left = Node::Div(Box::new(left), Box::new(self.unary()?));
            } else {
                return Ok(left);
            }
        }
    }

    fn unary(&mut self) -> Result<Node, ExprError> {
        if self.eat(b'-') {
            return self.nested(|p| Ok(Node::Neg(Box::new(p.unary()?))));
        }
        if self.eat(b'+') {
            return self.nested(Self::unary);
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Node, ExprError> {
        match self.peek() {
            Some(b'(') => {
                self.at += 1;
                let inner = self.nested(Self::sum)?;
                if self.eat(b')') {
                    Ok(inner)
                } else {
                    Err(ExprError::Syntax)
                }
            }
            Some(b'$') => {
                self.at += 1;
                Ok(Node::Var(Var::Dollar))
            }
            Some(b) if b.is_ascii_digit() || b == b'.' => self.number(),
            Some(b) if b.is_ascii_alphabetic() => self.word(),
            _ => Err(ExprError::Syntax),
        }
    }

    fn number(&mut self) -> Result<Node, ExprError> {
        let start = self.at;
        while self
            .bytes
            .get(self.at)
            .is_some_and(|b| b.is_ascii_digit() || *b == b'.')
        {
            self.at += 1;
        }
        let text = std::str::from_utf8(self.bytes.get(start..self.at).unwrap_or_default())
            .map_err(|_| ExprError::Syntax)?;
        if text.matches('.').count() > 1 || text == "." {
            return Err(ExprError::Syntax);
        }
        text.parse::<f64>()
            .ok()
            .filter(|x| x.is_finite())
            .map(Node::Num)
            .ok_or(ExprError::Syntax)
    }

    fn word(&mut self) -> Result<Node, ExprError> {
        let start = self.at;
        while self
            .bytes
            .get(self.at)
            .is_some_and(u8::is_ascii_alphanumeric)
        {
            self.at += 1;
        }
        let name = std::str::from_utf8(self.bytes.get(start..self.at).unwrap_or_default())
            .map_err(|_| ExprError::Syntax)?;
        match name {
            "x" => return Ok(Node::Var(Var::X)),
            "y" => return Ok(Node::Var(Var::Y)),
            "width" => return Ok(Node::Var(Var::Width)),
            "height" => return Ok(Node::Var(Var::Height)),
            "pi" => return Ok(Node::Num(std::f64::consts::PI)),
            "e" => return Ok(Node::Num(std::f64::consts::E)),
            _ => {}
        }
        let func = Func::named(name).ok_or(ExprError::Syntax)?;
        if !self.eat(b'(') {
            return Err(ExprError::Syntax);
        }
        self.nested(|p| {
            let first = p.sum()?;
            let second = if func.binary() {
                if !p.eat(b',') {
                    return Err(ExprError::Syntax);
                }
                Some(Box::new(p.sum()?))
            } else {
                None
            };
            if p.eat(b')') {
                Ok(Node::Call(func, Box::new(first), second))
            } else {
                Err(ExprError::Syntax)
            }
        })
    }
}

impl Expr {
    pub(crate) fn parse(text: &str) -> Result<Expr, ExprError> {
        if text.len() > FORMULA_BYTES {
            return Err(ExprError::TooLong);
        }
        let mut parser = Parser {
            bytes: text.as_bytes(),
            at: 0,
            depth: 0,
        };
        let node = parser.sum()?;
        if parser.peek().is_some() {
            return Err(ExprError::Syntax);
        }
        Ok(Expr(node))
    }

    pub(crate) fn eval(&self, vars: &Vars) -> Result<f64, ExprError> {
        let value = eval(&self.0, vars);
        if value.is_finite() {
            Ok(value)
        } else {
            Err(ExprError::NotFinite)
        }
    }
}

/// Evaluates a node. Its depth is bounded by the source's length ([`FORMULA_BYTES`]): parentheses and calls nest at
/// most [`FORMULA_DEPTH`] levels, and an operator chain (`1+1+…`) adds one level per operator.
fn eval(node: &Node, vars: &Vars) -> f64 {
    match node {
        Node::Num(x) => *x,
        Node::Var(Var::X) => vars.x,
        Node::Var(Var::Y) => vars.y,
        Node::Var(Var::Width) => vars.width,
        Node::Var(Var::Height) => vars.height,
        Node::Var(Var::Dollar) => vars.dollar,
        Node::Neg(a) => -eval(a, vars),
        Node::Add(a, b) => eval(a, vars) + eval(b, vars),
        Node::Sub(a, b) => eval(a, vars) - eval(b, vars),
        Node::Mul(a, b) => eval(a, vars) * eval(b, vars),
        Node::Div(a, b) => eval(a, vars) / eval(b, vars),
        Node::Call(func, a, b) => {
            let x = eval(a, vars);
            let y = b.as_deref().map_or(0.0, |b| eval(b, vars));
            match func {
                Func::Abs => x.abs(),
                Func::Sqrt => x.sqrt(),
                Func::Sin => slidock_math::sin(x),
                Func::Cos => slidock_math::cos(x),
                Func::Tan => slidock_math::tan(x),
                Func::Asin => slidock_math::asin(x),
                Func::Acos => slidock_math::acos(x),
                Func::Atan => slidock_math::atan(x),
                Func::Exp => slidock_math::exp(x),
                Func::Log => slidock_math::log(x),
                Func::Min => x.min(y),
                Func::Max => x.max(y),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str, vars: Vars) -> f64 {
        Expr::parse(text).unwrap().eval(&vars).unwrap()
    }

    #[test]
    fn evaluates_libreoffice_expressions() {
        let vars = Vars {
            x: 0.2,
            y: 0.3,
            width: 0.4,
            height: 0.5,
            dollar: 0.25,
        };
        assert_eq!(at("1+height/2", vars), 1.25);
        assert_eq!(at("0-width/2", vars), -0.2);
        assert_eq!(at("x-0.1", vars), 0.2 - 0.1);
        assert_eq!(at("-x", vars), -0.2);
        assert_eq!(at("2*-x", vars), -0.4);
        assert_eq!(at("min(max(width*height,.3),1)", vars), 0.3);
        assert_eq!(
            at("x+sin(2*pi*$)*0.05", vars),
            0.2 + slidock_math::sin(0.5 * std::f64::consts::PI) * 0.05
        );
        assert_eq!(
            at("(6*min(max(width*height,.3),1)-7.4)/-.7*width", vars),
            (6.0 * 0.3 - 7.4) / -0.7 * 0.4
        );
    }

    #[test]
    fn refuses_bad_and_hostile_expressions() {
        for bad in [
            "", "x+", "foo(1)", "sin 1", "min(1)", "1..2", "(x", "x)", "2 3", "1e5", "NaN",
        ] {
            assert_eq!(Expr::parse(bad), Err(ExprError::Syntax), "{bad}");
        }
        let deep = format!("{}x{}", "(".repeat(400), ")".repeat(400));
        assert_eq!(Expr::parse(&deep), Err(ExprError::TooDeep));
        let ok = format!(
            "{}x{}",
            "(".repeat(FORMULA_DEPTH),
            ")".repeat(FORMULA_DEPTH)
        );
        assert!(Expr::parse(&ok).is_ok());
        let over = format!(
            "{}x{}",
            "(".repeat(FORMULA_DEPTH + 1),
            ")".repeat(FORMULA_DEPTH + 1)
        );
        assert_eq!(Expr::parse(&over), Err(ExprError::TooDeep));
        assert_eq!(
            Expr::parse(&"-".repeat(40).to_string()),
            Err(ExprError::TooDeep)
        );
        assert_eq!(Expr::parse(&"1+".repeat(600)), Err(ExprError::TooLong));
        assert_eq!(
            Expr::parse("1/0").unwrap().eval(&Vars::default()),
            Err(ExprError::NotFinite)
        );
    }
}
