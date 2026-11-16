//! How a value is written into a document by writers that build markup as text. A printer that builds a tree
//! escapes text and attributes through its serialiser and needs only [`css_string`].

/// A quoted CSS string (a font family, a `url()` argument) that cannot end the string, the declaration, the rule or
/// a `<style>` element: quotes, backslashes, angle brackets and line breaks become CSS escapes.
pub fn css_string(value: &str) -> String {
    quoted_css(value, |c| {
        matches!(c, '\\' | '"' | '<' | '>' | '\n' | '\r' | '\u{c}')
    })
}

/// `value` in double quotes, each character `escape` selects written as `\<hex> `.
pub(crate) fn quoted_css(value: &str, escape: impl Fn(char) -> bool) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        if escape(c) {
            out.push_str(&format!("\\{:x} ", u32::from(c)));
        } else {
            out.push(c);
        }
    }
    out.push('"');
    out
}

/// Text content: `&`, `<` and `>` as entities.
pub fn escape_text(value: &str) -> String {
    escape_with(value, |c| match c {
        '&' => Some("&amp;"),
        '<' => Some("&lt;"),
        '>' => Some("&gt;"),
        _ => None,
    })
}

/// A double-quoted attribute value: `&`, `<` and `"` as entities.
pub fn escape_attribute(value: &str) -> String {
    escape_with(value, |c| match c {
        '&' => Some("&amp;"),
        '<' => Some("&lt;"),
        '"' => Some("&quot;"),
        _ => None,
    })
}

pub(crate) fn escape_with(value: &str, entity: impl Fn(char) -> Option<&'static str>) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match entity(c) {
            Some(text) => out.push_str(text),
            None => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_css_strings_that_cannot_end_the_string_the_declaration_or_the_element() {
        assert_eq!(css_string("Helvetica Neue"), "\"Helvetica Neue\"");
        let hostile = css_string("x\"; background:url(evil)</style><script>\\");
        assert_eq!(
            hostile,
            "\"x\\22 ; background:url(evil)\\3c /style\\3e \\3c script\\3e \\5c \""
        );
        let inner = &hostile[1..hostile.len() - 1];
        assert!(!inner.contains(['"', '<', '>']));
        assert_eq!(css_string("a\nb"), "\"a\\a b\"");
        assert_eq!(css_string("a\r\u{c}b"), "\"a\\d \\c b\"");
    }

    #[test]
    fn escapes_text_and_double_quoted_attributes() {
        assert_eq!(
            escape_text("a < b & c > d \"q\""),
            "a &lt; b &amp; c &gt; d \"q\""
        );
        assert_eq!(
            escape_attribute("a \"b\" <c> & d"),
            "a &quot;b&quot; &lt;c> &amp; d"
        );
    }
}
