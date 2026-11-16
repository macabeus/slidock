//! Schema sweeps shared by the synthetic and golden suites: every member of a presentation set to `null`, and
//! every integer rewritten as an integral float.

use serde_json::{Number, Value};

/// One step into a JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum Seg {
    Key(String),
    Index(usize),
}

pub type Path = Vec<Seg>;

pub fn show(path: &[Seg]) -> String {
    path.iter()
        .map(|seg| match seg {
            Seg::Key(key) => key.clone(),
            Seg::Index(i) => i.to_string(),
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// Every object member and array item of `value` with its path, outside raw JSON values (`ext`, `payload`).
pub fn members(value: &Value) -> Vec<(Path, &Value)> {
    let mut out = Vec::new();
    let mut stack: Vec<(Path, &Value)> = vec![(Vec::new(), value)];
    while let Some((path, value)) = stack.pop() {
        match value {
            Value::Object(map) => {
                for (key, member) in map {
                    let mut at = path.clone();
                    at.push(Seg::Key(key.clone()));
                    out.push((at.clone(), member));
                    if key != "ext" && key != "payload" {
                        stack.push((at, member));
                    }
                }
            }
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    let mut at = path.clone();
                    at.push(Seg::Index(i));
                    out.push((at.clone(), item));
                    stack.push((at, item));
                }
            }
            _ => {}
        }
    }
    out
}

fn get<'a>(root: &'a Value, path: &[Seg]) -> &'a Value {
    path.iter().fold(root, |value, seg| match seg {
        Seg::Key(key) => &value[key.as_str()],
        Seg::Index(i) => &value[*i],
    })
}

/// `root` with the value at `path` replaced.
pub fn with(root: &Value, path: &[Seg], value: Value) -> Value {
    let mut copy = root.clone();
    let mut slot = &mut copy;
    for seg in path {
        slot = match seg {
            Seg::Key(key) => &mut slot[key.as_str()],
            Seg::Index(i) => &mut slot[*i],
        };
    }
    *slot = value;
    copy
}

/// Whether zod's schema takes `null` at `path`: an element's stroke (a path's is a flag), a run's highlight or
/// link (an element's link is not nullable), a table slot, or a raw JSON value.
pub fn takes_null(root: &Value, path: &[Seg]) -> bool {
    let Some((last, parent)) = path.split_last() else {
        return false;
    };
    let in_element = get(root, parent).get("id").is_some();
    match last {
        Seg::Key(key) if key == "highlight" || key == "payload" => true,
        Seg::Key(key) if key == "stroke" => in_element,
        Seg::Key(key) if key == "link" => !in_element,
        Seg::Index(_) => matches!(parent.last(), Some(Seg::Key(key)) if key == "cells"),
        Seg::Key(_) => false,
    }
}

/// Each member of `root` set to `null`, with whether zod's schema takes it.
pub fn nulled(root: &Value) -> Vec<(Path, Value, bool)> {
    members(root)
        .into_iter()
        .filter(|(_, value)| !value.is_null())
        .map(|(path, _)| {
            let accepted = takes_null(root, &path);
            let input = with(root, &path, Value::Null);
            (path, input, accepted)
        })
        .collect()
}

/// Each integer of `root` written as an integral float (`0` as `-0.0`).
pub fn floated(root: &Value) -> Vec<(Path, Value)> {
    members(root)
        .into_iter()
        .filter_map(|(path, value)| {
            let n = value.as_i64()?;
            let x = if n == 0 { -0.0 } else { n as f64 };
            let float = Number::from_f64(x)?;
            assert!(float.is_f64());
            let input = with(root, &path, Value::Number(float));
            Some((path, input))
        })
        .collect()
}
