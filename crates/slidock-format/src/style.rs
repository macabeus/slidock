//! Inline styles as CSS reads them, through cssparser's CSS Syntax Level 3 tokenizer: comments, strings, escapes and
//! blocks as a browser tokenizes them, so `u\72l(…)` is a `url()` and `position:fi\78 ed` is `fixed`. A style attribute
//! parses when every item is a declaration (`name: value`), with no unclosed string, comment or bracket and no stray
//! closing bracket.

use cssparser::{Delimiter, ParseError, Parser, SourcePosition, Token};

/// One declaration of a `style` attribute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    /// The property, escapes decoded and lowercased.
    pub property: String,
    /// The value as written, trimmed, without a trailing `!important`.
    pub value: String,
}

impl Declaration {
    /// The value's keyword when it is a single identifier (`absolute`, `fi\78 ed`), lowercased.
    pub fn keyword(&self) -> Option<String> {
        let mut parser = Parser::new(&self.value);
        let keyword = match parser.next() {
            Ok(Token::Ident(name)) => name.to_lowercase(),
            _ => return None,
        };
        parser.is_exhausted().then_some(keyword)
    }

    /// Whether the value uses a viewport unit (`vh`, `svw`, `dvmin`, …), in any function or block. Strings, comments
    /// and URLs are not units.
    pub fn uses_viewport_units(&self) -> bool {
        let mut parser = Parser::new(&self.value);
        any_token(&mut parser, &|token| match token {
            Token::Dimension { unit, .. } => is_viewport_unit(unit),
            _ => false,
        })
    }
}

/// `[sld]?v(h|w|min|max|i|b)`, case-insensitive.
fn is_viewport_unit(unit: &str) -> bool {
    let unit = unit.to_ascii_lowercase();
    let rest = unit
        .strip_prefix(['s', 'l', 'd'])
        .filter(|rest| rest.starts_with('v'))
        .unwrap_or(&unit);
    matches!(rest, "vh" | "vw" | "vmin" | "vmax" | "vi" | "vb")
}

/// Whether `test` holds for any token, nested blocks included.
fn any_token<'i>(parser: &mut Parser<'i>, test: &dyn Fn(&Token<'i>) -> bool) -> bool {
    loop {
        let token = match parser.next() {
            Ok(token) => token.clone(),
            Err(_) => return false,
        };
        if test(&token) {
            return true;
        }
        if opens_block(&token) {
            let found = parser.parse_nested_block(|nested| {
                let found = any_token(nested, test);
                drain(nested);
                Ok::<_, ParseError<()>>(found)
            });
            if found.unwrap_or(false) {
                return true;
            }
        }
    }
}

/// Consumes the rest of a nested block, which `parse_nested_block` requires.
fn drain(parser: &mut Parser<'_>) {
    while parser.next_including_whitespace_and_comments().is_ok() {}
}

fn opens_block(token: &Token<'_>) -> bool {
    matches!(
        token,
        Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock
    )
}

/// Why a style attribute does not parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StyleError {
    pub reason: String,
}

fn invalid<T>(reason: impl Into<String>) -> Result<T, StyleError> {
    Err(StyleError {
        reason: reason.into(),
    })
}

/// The declarations of a `style` attribute, in order.
pub fn parse_style(style: &str) -> Result<Vec<Declaration>, StyleError> {
    let mut parser = Parser::new(style);
    let mut declarations = Vec::new();
    loop {
        skip_trivia(&mut parser)?;
        if parser.is_exhausted() {
            break;
        }
        let mut outcome: Result<Option<Declaration>, StyleError> = Ok(None);
        let parsed = parser.parse_until_after(Delimiter::Semicolon, |item| {
            outcome = declaration(item);
            Ok::<_, ParseError<()>>(())
        });
        // A declaration that stops early leaves its item unconsumed, so its own reason comes first.
        let declaration = outcome?;
        if parsed.is_err() {
            return invalid("the declaration list does not tokenize");
        }
        if let Some(declaration) = declaration {
            declarations.push(declaration);
        }
    }
    Ok(declarations)
}

/// Consumes whitespace and comments, refusing an unclosed comment.
fn skip_trivia(parser: &mut Parser<'_>) -> Result<(), StyleError> {
    loop {
        let state = parser.state();
        let before = parser.position();
        match parser.next_including_whitespace_and_comments() {
            Ok(Token::WhiteSpace(_)) => {}
            Ok(Token::Comment(text)) => {
                let token = Token::Comment(text);
                check_closed(parser, before, &token)?;
            }
            _ => {
                parser.reset(&state);
                return Ok(());
            }
        }
    }
}

/// One `;`-separated item: `None` for an empty one.
fn declaration<'i>(parser: &mut Parser<'i>) -> Result<Option<Declaration>, StyleError> {
    skip_trivia(parser)?;
    let first = parser.position();
    let property = match parser.next_including_whitespace_and_comments() {
        Err(_) => return Ok(None),
        Ok(Token::Ident(name)) => name.to_lowercase(),
        Ok(Token::AtKeyword(_)) => return invalid("a atrule is not a declaration"),
        Ok(token) => {
            let token = token.clone();
            let raw = parser.slice_from(first);
            let word = raw.strip_suffix('(').unwrap_or(raw);
            check_token(parser, first, &token)?;
            return invalid(format!("Unknown word {word}"));
        }
    };
    let word = parser.slice_from(first);
    skip_trivia(parser)?;
    match parser.next_including_whitespace_and_comments() {
        Ok(Token::Colon) => {}
        Ok(Token::CurlyBracketBlock) => return invalid("a rule is not a declaration"),
        Ok(_) | Err(_) => return invalid(format!("Unknown word {word}")),
    }
    let custom = property.starts_with("--");
    let start = parser.position();
    let mut significant: Vec<(SourcePosition, Token<'i>)> = Vec::new();
    loop {
        let before = parser.position();
        let token = match parser.next_including_whitespace_and_comments() {
            Ok(token) => token.clone(),
            Err(_) => break,
        };
        check_closed(parser, before, &token)?;
        match &token {
            Token::WhiteSpace(_) | Token::Comment(_) => continue,
            Token::Colon if !custom => {
                // A second top-level colon is a missing semicolon (`color:red background:blue`); `progid:` is the
                // one legacy value that holds one.
                let after_progid = matches!(significant.last(), Some((_, Token::Ident(name))) if name.eq_ignore_ascii_case("progid"));
                if !after_progid {
                    return invalid("Missed semicolon");
                }
            }
            Token::CurlyBracketBlock if !custom => return invalid("a rule is not a declaration"),
            _ => {}
        }
        if opens_block(&token) {
            check_nested_block(parser)?;
        }
        significant.push((before, token));
    }
    let mut end_of_value = None;
    if let [.., (bang, Token::Delim('!')), (_, Token::Ident(word))] = significant.as_slice()
        && word.eq_ignore_ascii_case("important")
    {
        end_of_value = Some(*bang);
    }
    let value = match end_of_value {
        Some(bang) => slice_between(parser, start, bang),
        None => parser.slice_from(start),
    };
    Ok(Some(Declaration {
        property,
        value: slidock_canon::js_trim(value).to_owned(),
    }))
}

fn slice_between<'i>(parser: &Parser<'i>, start: SourcePosition, end: SourcePosition) -> &'i str {
    let whole = parser.slice_from(start);
    whole
        .get(..end.byte_index().saturating_sub(start.byte_index()))
        .unwrap_or(whole)
}

/// The block the token just read opened, checked as a value is. A block is closed when its closing bracket follows
/// its content: one that reaches the end of input instead is refused, whatever its last character is.
fn check_nested_block(parser: &mut Parser<'_>) -> Result<(), StyleError> {
    let content_end = parser
        .parse_nested_block(|nested| {
            let checked = check_block(nested);
            drain(nested);
            Ok::<_, ParseError<()>>(checked.map(|()| nested.position()))
        })
        .map_err(|_| StyleError {
            reason: "brackets nest too deeply".into(),
        })??;
    if parser.position() == content_end {
        return invalid("Unclosed bracket");
    }
    Ok(())
}

/// The tokens inside a block, checked as a value's are.
fn check_block(parser: &mut Parser<'_>) -> Result<(), StyleError> {
    loop {
        let before = parser.position();
        let token = match parser.next_including_whitespace_and_comments() {
            Ok(token) => token.clone(),
            Err(_) => return Ok(()),
        };
        check_token(parser, before, &token)?;
    }
}

/// Checks one token, and the block it opens.
fn check_token<'i>(
    parser: &mut Parser<'i>,
    before: SourcePosition,
    token: &Token<'i>,
) -> Result<(), StyleError> {
    check_closed(parser, before, token)?;
    if opens_block(token) {
        check_nested_block(parser)?;
    }
    Ok(())
}

/// Refuses a token that a browser reads only by recovering from an error: an unclosed string, comment or URL, a bad
/// string, or a stray closing bracket.
fn check_closed(
    parser: &Parser<'_>,
    before: SourcePosition,
    token: &Token<'_>,
) -> Result<(), StyleError> {
    let raw = parser.slice_from(before);
    match token {
        Token::QuotedString(_) => {
            let quote = raw.chars().next().unwrap_or('"');
            if raw.len() < 2 || !closes(raw, quote) {
                return invalid("Unclosed string");
            }
        }
        Token::BadString(_) => return invalid("Unclosed string"),
        Token::Comment(_) => {
            if raw.len() < 4 || !raw.ends_with("*/") {
                return invalid("Unclosed comment");
            }
        }
        Token::UnquotedUrl(_) | Token::BadUrl(_) => {
            if !closes(raw, ')') {
                return invalid("Unclosed bracket");
            }
        }
        Token::CloseParenthesis | Token::CloseSquareBracket => {
            return invalid("Unexpected closing bracket");
        }
        Token::CloseCurlyBracket => return invalid("Unexpected }"),
        _ => {}
    }
    Ok(())
}

/// Whether `raw` ends with `close` that no backslash escapes.
fn closes(raw: &str, close: char) -> bool {
    let Some(body) = raw.strip_suffix(close) else {
        return false;
    };
    let backslashes = body.chars().rev().take_while(|c| *c == '\\').count();
    backslashes % 2 == 0
}

/// Functions that load an image from a string argument without `url()`: the format never writes them.
const URL_STRING_FUNCTIONS: &[&str] = &[
    "image-set",
    "-webkit-image-set",
    "image",
    "src",
    "cross-fade",
];

/// A URL a CSS value loads, and the byte span of its whole function (`url(…)`, escapes and quotes included).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CssUrl {
    pub url: String,
    pub start: usize,
    pub end: usize,
}

/// Every URL a CSS value loads, in order, nested functions included. A function that loads from a plain string
/// (`image-set("x.png" 1x)`) is reported as `<name>()` over its name and parenthesis, which no reference rule
/// accepts.
pub fn css_urls(value: &str) -> Vec<CssUrl> {
    let mut parser = Parser::new(value);
    let mut urls = Vec::new();
    collect_urls(&mut parser, &mut urls);
    urls
}

fn collect_urls(parser: &mut Parser<'_>, urls: &mut Vec<CssUrl>) {
    loop {
        let start = parser.position().byte_index();
        let token = match parser.next_including_whitespace_and_comments() {
            Ok(token) => token.clone(),
            Err(_) => return,
        };
        match &token {
            Token::UnquotedUrl(url) => urls.push(CssUrl {
                url: url.to_string(),
                start,
                end: parser.position().byte_index(),
            }),
            // cssparser keeps a bad URL's text as written, whitespace before `)` included.
            Token::BadUrl(url) => urls.push(CssUrl {
                url: slidock_canon::js_trim(url).to_owned(),
                start,
                end: parser.position().byte_index(),
            }),
            Token::Function(name) if name.eq_ignore_ascii_case("url") => {
                let argument = parser.parse_nested_block(|nested| {
                    let argument = match nested.next() {
                        Ok(Token::QuotedString(text)) => Some(text.to_string()),
                        _ => None,
                    };
                    drain(nested);
                    Ok::<_, ParseError<()>>(argument)
                });
                urls.push(CssUrl {
                    url: argument.ok().flatten().unwrap_or_default(),
                    start,
                    end: parser.position().byte_index(),
                });
            }
            Token::Function(name) => {
                let lower = name.to_ascii_lowercase();
                if URL_STRING_FUNCTIONS.contains(&lower.as_str()) {
                    urls.push(CssUrl {
                        url: format!("{lower}()"),
                        start,
                        end: parser.position().byte_index(),
                    });
                }
                let _ = parser.parse_nested_block(|nested| {
                    collect_urls(nested, urls);
                    Ok::<_, ParseError<()>>(())
                });
            }
            token if opens_block(token) => {
                let _ = parser.parse_nested_block(|nested| {
                    collect_urls(nested, urls);
                    Ok::<_, ParseError<()>>(())
                });
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declarations(style: &str) -> Vec<(String, String)> {
        parse_style(style)
            .unwrap()
            .into_iter()
            .map(|d| (d.property, d.value))
            .collect()
    }

    fn pair(property: &str, value: &str) -> (String, String) {
        (property.to_owned(), value.to_owned())
    }

    #[test]
    fn splits_declarations_skipping_comments() {
        assert_eq!(
            declarations("left:0pt; /* note */ color:RED"),
            [pair("left", "0pt"), pair("color", "RED")]
        );
        assert_eq!(
            declarations("POSITION : absolute !important;;top:1pt;font-family:\"a;b\",x"),
            [
                pair("position", "absolute"),
                pair("top", "1pt"),
                pair("font-family", "\"a;b\",x")
            ]
        );
        assert_eq!(
            declarations("--x: {a:b}; width:calc(1px + 2px)"),
            [pair("--x", "{a:b}"), pair("width", "calc(1px + 2px)")]
        );
        assert_eq!(declarations(""), []);
        assert_eq!(
            declarations("filter:progid:DXImageTransform.Microsoft.gradient()"),
            [pair(
                "filter",
                "progid:DXImageTransform.Microsoft.gradient()"
            )]
        );
    }

    fn reason(style: &str) -> String {
        parse_style(style).unwrap_err().reason
    }

    #[test]
    fn refuses_what_is_not_a_declaration_list_with_its_reason() {
        for (style, expected) in [
            ("color:red;}", "Unexpected }"),
            ("color:red}", "Unexpected }"),
            ("color red", "Unknown word color"),
            ("COLOR", "Unknown word COLOR"),
            ("*zoom:1", "Unknown word *"),
            ("fn(a) b", "Unknown word fn"),
            ("a{b:c}", "a rule is not a declaration"),
            ("color:red{x}", "a rule is not a declaration"),
            ("@media x{}", "a atrule is not a declaration"),
            ("color:red background:blue", "Missed semicolon"),
            ("x:y:z", "Missed semicolon"),
            ("content:\"abc", "Unclosed string"),
            ("content:'a\\'", "Unclosed string"),
            ("x:fn(\"a)", "Unclosed string"),
            ("left:0pt /* open", "Unclosed comment"),
            ("left:0pt; /* open", "Unclosed comment"),
            ("/* open", "Unclosed comment"),
            ("x:fn(/* a)", "Unclosed comment"),
            ("width:1px)", "Unexpected closing bracket"),
        ] {
            assert_eq!(reason(style), expected, "{style}");
        }
    }

    #[test]
    fn refuses_a_bracket_the_input_ends_inside_whatever_its_last_character() {
        for style in [
            "width:calc(1px",
            "width:calc(1px + (2px)",
            "x:fn(()",
            "x:calc((1px)",
            "x:fn(a[b]",
            "x:[a(b)",
            "--x:(a",
            "--x:{a",
            "x:fn(\\)",
            "background:url(a.png",
            "background:url(a\\)",
            "background:url(a b",
        ] {
            assert_eq!(reason(style), "Unclosed bracket", "{style}");
        }
        for (style, value) in [
            ("x:a((b)c)", "a((b)c)"),
            ("x:fn(a\\\\)", "fn(a\\\\)"),
            ("x:url(a b)", "url(a b)"),
            ("x:[a(b)]", "[a(b)]"),
        ] {
            assert_eq!(declarations(style), [pair("x", value)], "{style}");
        }
    }

    #[test]
    fn reads_keywords_and_viewport_units_as_tokens() {
        let decl = |value: &str| Declaration {
            property: "x".into(),
            value: value.into(),
        };
        assert_eq!(decl("Fixed").keyword().as_deref(), Some("fixed"));
        assert_eq!(decl("fi\\78 ed").keyword().as_deref(), Some("fixed"));
        assert_eq!(decl("\"fixed\"").keyword(), None);
        assert_eq!(decl("fixed fixed").keyword(), None);
        for value in [
            "50vw",
            "calc(100vh - 1pt)",
            "1SVH",
            "-2.5dvmin",
            "+1lvb",
            "1e2vi",
            ".5vmax",
        ] {
            assert!(decl(value).uses_viewport_units(), "{value}");
        }
        for value in [
            "10vhx",
            "#10vh",
            "a10vh",
            "\"10vh\"",
            "url(10vh)",
            "/* 10vh */ 1pt",
            "10pt",
            "1xvh",
        ] {
            assert!(!decl(value).uses_viewport_units(), "{value}");
        }
    }

    #[test]
    fn finds_urls_the_way_browsers_do() {
        let urls: Vec<String> = css_urls("url( \"a b.png\" ) url(c\\29.png) URL(d)")
            .into_iter()
            .map(|found| found.url)
            .collect();
        assert_eq!(urls, ["a b.png", "c).png", "d"]);
        let value = "x u\\72l(e.png) image-set(\"f.png\" 1x, url(g.png) 2x) \"url(h)\" /* url(i) */ var(--a, url('j'))";
        let found = css_urls(value);
        let urls: Vec<&str> = found.iter().map(|found| found.url.as_str()).collect();
        assert_eq!(urls, ["e.png", "image-set()", "g.png", "j"]);
        let urls: Vec<String> = css_urls("url(\"k\" x) url(\"l\")")
            .into_iter()
            .map(|found| found.url)
            .collect();
        assert_eq!(urls, ["k", "l"]);
        assert_eq!(&value[found[0].start..found[0].end], "u\\72l(e.png)");
        assert_eq!(&value[found[1].start..found[1].end], "image-set(");
        assert_eq!(&value[found[3].start..found[3].end], "url('j')");
        // A bad URL (whitespace inside an unquoted one) is reported too, so no rule depends on how a browser
        // recovers from it.
        let urls: Vec<String> = css_urls("url(https://x.y/a b.png) url( c d ) url(e f")
            .into_iter()
            .map(|found| found.url)
            .collect();
        assert_eq!(urls, ["https://x.y/a b.png", "c d", "e f"]);
    }

    #[test]
    fn reports_every_function_that_loads_from_a_string() {
        for name in [
            "image-set",
            "-webkit-image-set",
            "image",
            "src",
            "cross-fade",
            "Cross-Fade",
        ] {
            let urls: Vec<String> = css_urls(&format!("{name}(\"a.png\" 1x)"))
                .into_iter()
                .map(|found| found.url)
                .collect();
            assert_eq!(urls, [format!("{}()", name.to_ascii_lowercase())], "{name}");
        }
        assert_eq!(css_urls("linear-gradient(\"a.png\")"), []);
    }
}
