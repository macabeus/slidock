//! Bounds the types do not hold by construction: numeric ranges, string and array lengths, origin parts. zod
//! checks them while parsing; here [`Validate`] walks a parsed value and collects every issue with its path, so a
//! value out of range is a diagnostic on its node (IR0008, IR0011, or IR0001 for a too-short string or array),
//! never a crash.

use crate::diagnostics::{DiagnosticCode, codes};
use crate::ids::NodeId;
use crate::path::{IrPath, PathSeg};
use crate::units::Check;

/// What an issue says beyond its code and path.
#[derive(Clone, Debug, PartialEq)]
pub enum IssueDetail {
    /// A bound: `what` (`number`, `string`, `array`) at most or at least `limit`.
    Bound { what: &'static str, limit: String },
    /// Any other issue, with zod's kind of message.
    Reason(String),
}

/// One bound or validity issue.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub code: DiagnosticCode,
    pub path: IrPath,
    /// The innermost element on the path.
    pub node: Option<NodeId>,
    pub detail: IssueDetail,
}

/// Walks a value, tracking the path and the innermost element, and collects issues.
#[derive(Debug, Default)]
pub struct Validator {
    path: IrPath,
    nodes: Vec<NodeId>,
    issues: Vec<Issue>,
}

impl Validator {
    pub fn new() -> Self {
        Self::default()
    }

    /// A validator whose paths start at `path`.
    pub fn at_path(path: IrPath) -> Self {
        Validator {
            path,
            ..Self::default()
        }
    }

    pub fn into_issues(self) -> Vec<Issue> {
        self.issues
    }

    pub fn issues(&self) -> &[Issue] {
        &self.issues
    }

    pub fn path(&self) -> &IrPath {
        &self.path
    }

    /// Runs `f` one segment deeper.
    pub fn at(&mut self, seg: impl Into<PathSeg>, f: impl FnOnce(&mut Validator)) {
        self.path.push(seg);
        f(self);
        self.path.pop();
    }

    /// Validates `value` under `key`.
    pub fn field<T: Validate + ?Sized>(&mut self, key: &'static str, value: &T) {
        self.at(key, |v| value.validate(v));
    }

    /// Runs `f` inside the element `id`, which becomes the node of its issues.
    pub fn element(&mut self, id: &NodeId, f: impl FnOnce(&mut Validator)) {
        self.nodes.push(id.clone());
        f(self);
        self.nodes.pop();
    }

    pub fn report(&mut self, code: DiagnosticCode, detail: IssueDetail) {
        self.issues.push(Issue {
            code,
            path: self.path.clone(),
            node: self.nodes.last().cloned(),
            detail,
        });
    }

    fn too_big(&mut self, what: &'static str, limit: f64) {
        self.report(
            codes::IR0008,
            IssueDetail::Bound {
                what,
                limit: slidock_canon::to_js_string(limit),
            },
        );
    }

    fn too_small_number(&mut self, limit: f64) {
        self.report(
            codes::IR0008,
            IssueDetail::Bound {
                what: "number",
                limit: slidock_canon::to_js_string(limit),
            },
        );
    }

    /// A number through zod's `checks`, each failed bound reported in order; the grid holds for a parsed value.
    pub fn checks(&mut self, x: f64, checks: &[Check]) {
        for check in checks {
            if check.holds(x, true) {
                continue;
            }
            match *check {
                Check::Min { limit, .. } => self.too_small_number(limit),
                Check::Max { limit, .. } => self.too_big("number", limit),
                Check::Grid => {}
            }
        }
    }

    /// A number at most `max`.
    pub fn max(&mut self, x: f64, max: f64) {
        if x > max {
            self.too_big("number", max);
        }
    }

    /// A number at least `min` (`exclusive` for zod's `gt`).
    pub fn min(&mut self, x: f64, min: f64, exclusive: bool) {
        if x < min || (exclusive && x <= min) {
            self.too_small_number(min);
        }
    }

    /// An integer within `min..=max`.
    pub fn int(&mut self, x: i64, min: i64, max: i64) {
        if x < min {
            self.too_small_number(min as f64);
        }
        if x > max {
            self.too_big("number", max as f64);
        }
    }

    /// A string of `min..=max` UTF-16 code units: too long is IR0008, too short IR0001, as zod reports them.
    pub fn string(&mut self, s: &str, min: usize, max: usize) {
        let len = slidock_canon::utf16::len(s);
        if len < min {
            self.report(
                codes::IR0001,
                IssueDetail::Reason(format!(
                    "Too small: expected string to have >={min} characters"
                )),
            );
        }
        if len > max {
            self.too_big("string", max as f64);
        }
    }

    /// An array of `min..=max` items: too long is IR0008, too short IR0001.
    pub fn array(&mut self, len: usize, min: usize, max: usize) {
        if len < min {
            self.report(
                codes::IR0001,
                IssueDetail::Reason(format!("Too small: expected array to have >={min} items")),
            );
        }
        if len > max {
            self.too_big("array", max as f64);
        }
    }

    /// An issue tagged with its own code and message.
    pub fn invalid(&mut self, code: DiagnosticCode, reason: impl Into<String>) {
        self.report(code, IssueDetail::Reason(reason.into()));
    }
}

/// A value whose bounds the verifier checks.
pub trait Validate {
    fn validate(&self, v: &mut Validator);
}

impl<T: Validate + ?Sized> Validate for &T {
    fn validate(&self, v: &mut Validator) {
        (**self).validate(v);
    }
}

impl<T: Validate + ?Sized> Validate for Box<T> {
    fn validate(&self, v: &mut Validator) {
        (**self).validate(v);
    }
}

impl<T: Validate> Validate for Option<T> {
    fn validate(&self, v: &mut Validator) {
        if let Some(value) = self {
            value.validate(v);
        }
    }
}

impl<T: Validate> Validate for Vec<T> {
    fn validate(&self, v: &mut Validator) {
        for (i, item) in self.iter().enumerate() {
            v.at(i, |v| item.validate(v));
        }
    }
}

impl<K: AsRef<str>, V: Validate> Validate for indexmap::IndexMap<K, V> {
    fn validate(&self, v: &mut Validator) {
        for (key, value) in self {
            v.at(key.as_ref().to_owned(), |v| value.validate(v));
        }
    }
}

impl<V: Validate> Validate for std::collections::BTreeMap<String, V> {
    fn validate(&self, v: &mut Validator) {
        for (key, value) in self {
            v.at(key.clone(), |v| value.validate(v));
        }
    }
}

macro_rules! unbounded {
    ($($ty:ty),*) => {
        $(impl Validate for $ty {
            fn validate(&self, _: &mut Validator) {}
        })*
    };
}
unbounded!(
    bool,
    u8,
    u16,
    u32,
    u64,
    i64,
    f64,
    NodeId,
    crate::ids::Sha256,
    crate::formats::EffectId
);

macro_rules! ranged_unit {
    ($($ty:ident),*) => {
        $(impl Validate for crate::units::$ty {
            fn validate(&self, v: &mut Validator) {
                v.checks(self.to_f64(), crate::units::$ty::CHECKS);
            }
        })*
    };
}
ranged_unit!(Pt, Size, Deg, Angle, Ratio, UnitRatio, Px, PathUnit);

impl Validate for crate::units::Ms {
    fn validate(&self, v: &mut Validator) {
        v.int(i64::from(self.0), 0, i64::from(crate::units::Ms::MAX));
    }
}

/// Validates `value` from the root.
pub fn validate<T: Validate + ?Sized>(value: &T) -> Vec<Issue> {
    let mut v = Validator::new();
    value.validate(&mut v);
    v.into_issues()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::{Angle, Pt};

    #[test]
    fn reports_bounds_with_paths() {
        let mut v = Validator::new();
        v.field("x", &Pt::exact(100_001.0).unwrap());
        v.at("list", |v| v.at(2, |v| v.string("", 1, 4)));
        v.field("a", &Angle::exact(360.0).unwrap());
        let issues = v.into_issues();
        assert_eq!(issues.len(), 3);
        assert_eq!(issues[0].path.to_string(), "x");
        assert_eq!(
            issues[0].detail,
            IssueDetail::Bound {
                what: "number",
                limit: "100000".into()
            }
        );
        assert_eq!(issues[1].code, codes::IR0001);
        assert_eq!(issues[1].path.to_string(), "list[2]");
        assert_eq!(
            issues[2].detail,
            IssueDetail::Bound {
                what: "number",
                limit: "360".into()
            }
        );
    }

    #[test]
    fn names_the_innermost_element() {
        let mut v = Validator::new();
        let outer = NodeId::new("ir-a").unwrap();
        let inner = NodeId::new("ir-b").unwrap();
        v.element(&outer, |v| {
            v.element(&inner, |v| v.max(2.0, 1.0));
            v.max(2.0, 1.0);
        });
        let issues = v.into_issues();
        assert_eq!(issues[0].node.as_ref(), Some(&inner));
        assert_eq!(issues[1].node.as_ref(), Some(&outer));
    }
}
