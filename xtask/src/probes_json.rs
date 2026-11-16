//! A small ordered JSON value and its pretty printer (2-space indent, trailing newline), for the probe manifests and
//! expectations. Objects keep insertion order, so the output depends only on the generator.

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    /// A number printed as written (decimal literals such as `0.5`).
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn obj() -> Json {
        Json::Obj(Vec::new())
    }

    /// Appends a field (builder style).
    pub fn with(mut self, key: &str, value: impl Into<Json>) -> Json {
        if let Json::Obj(fields) = &mut self {
            fields.push((key.to_owned(), value.into()));
        }
        self
    }

    /// Appends a field unless the value is `None`.
    pub fn with_opt(self, key: &str, value: Option<impl Into<Json>>) -> Json {
        match value {
            Some(value) => self.with(key, value),
            None => self,
        }
    }

    pub fn push(&mut self, key: &str, value: impl Into<Json>) {
        if let Json::Obj(fields) = self {
            fields.push((key.to_owned(), value.into()));
        }
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(fields) => fields
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn pretty(&self) -> String {
        self.pretty_until(usize::MAX)
    }

    /// Pretty-printed down to `depth`; deeper arrays and objects print on one line.
    pub fn pretty_until(&self, depth: usize) -> String {
        let mut out = String::new();
        self.write(&mut out, 0, depth);
        out.push('\n');
        out
    }

    fn write(&self, out: &mut String, depth: usize, flat_from: usize) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            Json::Int(value) => out.push_str(&value.to_string()),
            Json::Num(value) => out.push_str(value),
            Json::Str(value) => escape(out, value),
            Json::Arr(items) if items.is_empty() => out.push_str("[]"),
            Json::Obj(fields) if fields.is_empty() => out.push_str("{}"),
            Json::Arr(items) if depth >= flat_from || items.iter().all(Json::is_scalar) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    item.write(out, depth + 1, flat_from);
                }
                out.push(']');
            }
            Json::Obj(fields) if depth >= flat_from => {
                out.push('{');
                for (i, (key, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    escape(out, key);
                    out.push_str(": ");
                    value.write(out, depth + 1, flat_from);
                }
                out.push('}');
            }
            Json::Arr(items) => {
                out.push_str("[\n");
                for (i, item) in items.iter().enumerate() {
                    indent(out, depth + 1);
                    item.write(out, depth + 1, flat_from);
                    if i + 1 < items.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                indent(out, depth);
                out.push(']');
            }
            Json::Obj(fields) => {
                out.push_str("{\n");
                for (i, (key, value)) in fields.iter().enumerate() {
                    indent(out, depth + 1);
                    escape(out, key);
                    out.push_str(": ");
                    value.write(out, depth + 1, flat_from);
                    if i + 1 < fields.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                indent(out, depth);
                out.push('}');
            }
        }
    }

    fn is_scalar(&self) -> bool {
        !matches!(self, Json::Arr(_) | Json::Obj(_))
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn escape(out: &mut String, value: &str) {
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

impl From<bool> for Json {
    fn from(value: bool) -> Json {
        Json::Bool(value)
    }
}

impl From<u32> for Json {
    fn from(value: u32) -> Json {
        Json::Int(i64::from(value))
    }
}

impl From<i64> for Json {
    fn from(value: i64) -> Json {
        Json::Int(value)
    }
}

impl From<usize> for Json {
    fn from(value: usize) -> Json {
        Json::Int(value as i64)
    }
}

impl From<&str> for Json {
    fn from(value: &str) -> Json {
        Json::Str(value.to_owned())
    }
}

impl From<String> for Json {
    fn from(value: String) -> Json {
        Json::Str(value)
    }
}

impl From<&String> for Json {
    fn from(value: &String) -> Json {
        Json::Str(value.clone())
    }
}

impl<T: Into<Json>> From<Vec<T>> for Json {
    fn from(value: Vec<T>) -> Json {
        Json::Arr(value.into_iter().map(Into::into).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prints_ordered_and_escaped() {
        let value = Json::obj()
            .with("b", 1u32)
            .with("a", "x\"y")
            .with("list", vec![1u32, 2])
            .with("nested", Json::obj().with("n", Json::Num("0.5".into())))
            .with("empty", Json::obj());
        assert_eq!(
            Json::obj()
                .with(
                    "a",
                    Json::obj().with("b", vec![Json::obj().with("c", 1u32)])
                )
                .pretty_until(1),
            "{\n  \"a\": {\"b\": [{\"c\": 1}]}\n}\n"
        );
        assert_eq!(
            value.pretty(),
            "{\n  \"b\": 1,\n  \"a\": \"x\\\"y\",\n  \"list\": [1, 2],\n  \"nested\": {\n    \"n\": 0.5\n  },\n  \"empty\": {}\n}\n"
        );
    }
}
