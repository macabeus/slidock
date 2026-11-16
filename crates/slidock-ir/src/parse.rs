//! Reading IR JSON with the path of the first structural error, which the verifier reports as IR0001 (or IR0007
//! and IR0008 for numbers off their grid or beyond what it can hold).
//!
//! Numbers are read as JavaScript reads them: an integral number is an integer whatever its spelling (`500.0`,
//! `5e2`, `-0.0`), so integer fields take it, as zod's `z.int()` does. An integer beyond its field's Rust type
//! (a negative count, a weight above `u32::MAX`, font units beyond `i32`) is IR0001 where zod reports the field's
//! range as IR0008.
//!
//! Nesting is bounded at [`MAX_DEPTH`] arrays and objects, serde_json's own bound, which keeps the recursive reader
//! within the stack on every target. Valid IR nests far less (16 groups deep is about 50 levels), so deeper input is
//! refused as IR0001 at the root (a group beyond 16 levels that fits the bound is IR0005); only an `ext` or `payload`
//! value nested that deep is valid input refused.

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::de::decode;
use crate::diagnostics::{DiagnosticCode, codes};
use crate::path::IrPath;
use crate::validate::IssueDetail;

/// The deepest nesting of arrays and objects read.
pub const MAX_DEPTH: usize = 127;

/// The largest integer a JS number holds exactly (`Number.MAX_SAFE_INTEGER`).
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// One issue of a structural error.
#[derive(Clone, Debug, PartialEq)]
pub struct StructureIssue {
    pub code: DiagnosticCode,
    pub detail: IssueDetail,
}

/// The first structural error of a value: where, and what is wrong there (one issue, or each check a number
/// fails, in zod's order).
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("{} at {path}: {}", self.code(), self.reason())]
pub struct StructureError {
    pub path: IrPath,
    pub issues: Vec<StructureIssue>,
}

impl StructureError {
    fn root(reason: impl Into<String>) -> Self {
        StructureError {
            path: IrPath::root(),
            issues: vec![StructureIssue {
                code: codes::IR0001,
                detail: IssueDetail::Reason(reason.into()),
            }],
        }
    }

    /// The code of the first issue.
    pub fn code(&self) -> DiagnosticCode {
        self.issues
            .first()
            .map_or(codes::IR0001, |issue| issue.code)
    }

    /// The first issue's reason, or the limit it exceeds.
    pub fn reason(&self) -> &str {
        match self.issues.first().map(|issue| &issue.detail) {
            Some(IssueDetail::Reason(reason)) => reason,
            Some(IssueDetail::Bound { limit, .. }) => limit,
            None => "",
        }
    }
}

/// Rewrites integral numbers as integers and checks the nesting depth (from `depth`, where `value` sits in the
/// document), without recursion.
fn read_as_js(value: &mut Value, depth: usize) -> Result<(), StructureError> {
    let mut stack = vec![(value, depth)];
    while let Some((value, depth)) = stack.pop() {
        match value {
            Value::Number(n) => {
                if let Some(x) = n.as_f64().filter(|_| n.is_f64())
                    && x.fract() == 0.0
                    && x.abs() <= MAX_SAFE_INTEGER
                {
                    *n = serde_json::Number::from(x as i64);
                }
            }
            Value::Array(_) | Value::Object(_) if depth >= MAX_DEPTH => {
                return Err(StructureError::root(format!(
                    "Too deep: nested beyond {MAX_DEPTH} arrays and objects"
                )));
            }
            Value::Array(items) => stack.extend(items.iter_mut().map(|item| (item, depth + 1))),
            Value::Object(members) => {
                stack.extend(members.values_mut().map(|member| (member, depth + 1)));
            }
            Value::Null | Value::Bool(_) | Value::String(_) => {}
        }
    }
    Ok(())
}

/// Reads `value` as `T`.
pub fn from_value<T: DeserializeOwned>(value: Value) -> Result<T, StructureError> {
    from_value_at(value, 0)
}

/// Reads `value`, which sits `depth` arrays and objects deep in its document, as `T`. An error's path is from
/// `value`, not from the document's root.
pub(crate) fn from_value_at<T: DeserializeOwned>(
    mut value: Value,
    depth: usize,
) -> Result<T, StructureError> {
    read_as_js(&mut value, depth)?;
    serde_path_to_error::deserialize(value).map_err(|error| {
        let decoded = decode(&error);
        StructureError {
            path: decoded.path.into_iter().collect(),
            issues: decoded.issues,
        }
    })
}

/// Reads JSON text as `T`; text that is not JSON, or nests beyond [`MAX_DEPTH`], is an IR0001 at the root.
pub fn from_str<T: DeserializeOwned>(text: &str) -> Result<T, StructureError> {
    let value: Value = serde_json::from_str(text).map_err(|error| {
        if error.to_string().starts_with("recursion limit exceeded") {
            StructureError::root(format!(
                "Too deep: nested beyond {MAX_DEPTH} arrays and objects"
            ))
        } else {
            StructureError::root(error.to_string())
        }
    })?;
    from_value(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Element;

    #[test]
    fn names_the_first_structural_error_by_its_full_path() {
        let element = serde_json::json!({
            "id": "ir-g", "origin": { "kind": "agent" }, "kind": "group",
            "children": [{ "id": "ir-g.a", "origin": { "kind": "agent" }, "kind": "shape",
                "frame": { "x": 0, "y": 0.0001, "w": 1, "h": 1, "rotation": 0, "flipH": false, "flipV": false } }]
        });
        let error = from_value::<Element>(element).unwrap_err();
        assert_eq!(error.code(), codes::IR0007);
        assert_eq!(error.path.to_string(), "children[0].frame.y");
        assert_eq!(error.reason(), "Not a multiple of 0.001");
        let error = from_str::<Element>(r#"{"id":"ir-a","origin":{"kind":"agent"},"kind":"blob"}"#)
            .unwrap_err();
        assert_eq!(error.code(), codes::IR0001);
        assert!(from_str::<Element>("not json").is_err());
    }

    fn shape(frame: &str) -> String {
        format!(
            r#"{{"id":"ir-a","origin":{{"kind":"agent"}},"kind":"shape","frame":{frame},"placeholder":{{"role":"body","index":5e0}}}}"#
        )
    }

    #[test]
    fn reads_integral_numbers_as_integers() {
        let text =
            shape(r#"{"x":-0.0,"y":0,"w":1e2,"h":1.5,"rotation":0,"flipH":false,"flipV":false}"#);
        let element = from_str::<Element>(&text).unwrap();
        let printed = crate::canonical::to_json(&element).unwrap();
        assert!(printed.contains(r#""index":5"#), "{printed}");
        assert!(printed.contains(r#""x":0,"#), "{printed}");
        assert!(printed.contains(r#""w":100,"#), "{printed}");
        let text = shape(r#"{"x":0,"y":0,"w":1,"h":1,"rotation":0,"flipH":false,"flipV":false}"#)
            .replace("5e0", "5.5");
        assert_eq!(
            from_str::<Element>(&text).unwrap_err().code(),
            codes::IR0001
        );
    }

    #[test]
    fn a_number_reports_each_check_it_fails() {
        let text = shape(
            r#"{"x":100000.0005,"y":0,"w":-200000,"h":1,"rotation":0,"flipH":false,"flipV":false}"#,
        );
        let error = from_str::<Element>(&text).unwrap_err();
        assert_eq!(error.path.to_string(), "frame.x");
        let codes: Vec<_> = error.issues.iter().map(|issue| issue.code).collect();
        assert_eq!(codes, [codes::IR0008, codes::IR0007]);
    }

    fn nested(depth: usize) -> String {
        format!("{}{}", "[".repeat(depth), "]".repeat(depth))
    }

    #[test]
    fn bounds_the_nesting_depth_on_both_entries() {
        for depth in [MAX_DEPTH, MAX_DEPTH + 1, MAX_DEPTH + 50] {
            let text = nested(depth);
            let from_text = from_str::<Value>(&text);
            let mut value = Value::Null;
            for _ in 0..depth {
                value = Value::Array(vec![value]);
            }
            let from_tree = from_value::<Value>(value);
            assert_eq!(from_text.is_ok(), depth <= MAX_DEPTH, "{depth}");
            assert_eq!(from_tree.is_ok(), depth <= MAX_DEPTH, "{depth}");
            if let (Err(a), Err(b)) = (from_text, from_tree) {
                assert_eq!(a, b);
                assert!(a.reason().starts_with("Too deep"));
            }
        }
    }
}
