//! JavaScript's whitespace set: the ECMAScript WhiteSpace and LineTerminator productions, which `\s`,
//! `String.prototype.trim` and its kin use. It differs from `char::is_whitespace`: JavaScript includes U+FEFF and
//! excludes U+0085 and U+180E.

/// Whether `c` matches JavaScript's `\s`.
pub const fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'..='\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
    )
}

/// `s.trim()`.
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_whitespace)
}

/// `s.trimStart()`.
pub fn js_trim_start(s: &str) -> &str {
    s.trim_start_matches(is_js_whitespace)
}

/// `s.trimEnd()`.
pub fn js_trim_end(s: &str) -> &str {
    s.trim_end_matches(is_js_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn differs_from_rust_whitespace() {
        assert!(is_js_whitespace('\u{FEFF}') && !'\u{FEFF}'.is_whitespace());
        assert!(!is_js_whitespace('\u{0085}') && '\u{0085}'.is_whitespace());
        assert!(!is_js_whitespace('\u{180E}'));
        assert_eq!(js_trim("\u{FEFF} a \u{0085}"), "a \u{0085}");
        assert_eq!(js_trim_start("\u{3000}a "), "a ");
        assert_eq!(js_trim_end(" a\u{2029}"), " a");
    }
}
