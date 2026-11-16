//! Paths into a presentation, printed as text: identifier keys joined with `.`, indexes as `[n]`, other keys as
//! `["…"]`, and the empty path as `(presentation)`. Diagnostic `data.path` values are these strings.

use std::borrow::Cow;
use std::fmt;

/// One step of a path: an object key or an array index.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PathSeg {
    Key(Cow<'static, str>),
    Index(usize),
}

impl From<&'static str> for PathSeg {
    fn from(key: &'static str) -> Self {
        PathSeg::Key(Cow::Borrowed(key))
    }
}

impl From<String> for PathSeg {
    fn from(key: String) -> Self {
        PathSeg::Key(Cow::Owned(key))
    }
}

impl From<usize> for PathSeg {
    fn from(index: usize) -> Self {
        PathSeg::Index(index)
    }
}

/// A path from the presentation root.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct IrPath(Vec<PathSeg>);

impl IrPath {
    pub fn root() -> Self {
        IrPath(Vec::new())
    }

    pub fn segments(&self) -> &[PathSeg] {
        &self.0
    }

    pub fn push(&mut self, seg: impl Into<PathSeg>) {
        self.0.push(seg.into());
    }

    pub fn pop(&mut self) {
        self.0.pop();
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn truncate(&mut self, len: usize) {
        self.0.truncate(len);
    }

    /// This path with `seg` appended.
    pub fn with(&self, seg: impl Into<PathSeg>) -> IrPath {
        let mut path = self.clone();
        path.push(seg);
        path
    }

    /// This path with every segment of `segs` appended.
    pub fn join<I, S>(&self, segs: I) -> IrPath
    where
        I: IntoIterator<Item = S>,
        S: Into<PathSeg>,
    {
        let mut path = self.clone();
        for seg in segs {
            path.push(seg);
        }
        path
    }

    pub fn extend(&mut self, segs: impl IntoIterator<Item = PathSeg>) {
        self.0.extend(segs);
    }
}

impl FromIterator<PathSeg> for IrPath {
    fn from_iter<I: IntoIterator<Item = PathSeg>>(iter: I) -> Self {
        IrPath(iter.into_iter().collect())
    }
}

/// `^[A-Za-z_$][A-Za-z0-9_$]*$`
fn is_identifier(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

impl fmt::Display for IrPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return f.write_str("(presentation)");
        }
        for (i, seg) in self.0.iter().enumerate() {
            match seg {
                PathSeg::Index(index) => write!(f, "[{index}]")?,
                PathSeg::Key(key) if is_identifier(key) => {
                    if i > 0 {
                        f.write_str(".")?;
                    }
                    f.write_str(key)?;
                }
                PathSeg::Key(key) => {
                    let mut quoted = String::new();
                    slidock_canon::write_json_string(key, &mut quoted);
                    write!(f, "[{quoted}]")?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prints_like_format_path() {
        assert_eq!(IrPath::root().to_string(), "(presentation)");
        let path = IrPath::root().join([
            "slides".into(),
            PathSeg::Index(0),
            "elements".into(),
            2.into(),
        ]);
        assert_eq!(path.to_string(), "slides[0].elements[2]");
        let path = IrPath::root().join(["layouts", "pp-2147483649"]);
        assert_eq!(path.to_string(), "layouts[\"pp-2147483649\"]");
        let path = IrPath::root().join(["listStyle", "0", "$x", "_y1"]);
        assert_eq!(path.to_string(), "listStyle[\"0\"].$x._y1");
        assert_eq!(IrPath::root().with(3).to_string(), "[3]");
        assert_eq!(IrPath::root().with("a\"b").to_string(), "[\"a\\\"b\"]");
    }
}
