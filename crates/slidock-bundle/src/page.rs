//! The deck page, `index.html` (`docs/format.md` §2): a Content-Security-Policy that runs only the page's own
//! scripts, the page's metadata and chrome style, the text view, the font style, the deck JSON, one inert
//! `<template data-sd-slide>` per slide, the player, the presenter view's script when the page ships one, and the boot.
//!
//! Everything an indexer needs comes before anything unbounded: the head and the text view lead, and
//! the fonts, the deck JSON, the templates and the player follow in `<body>`, so the text stays inside the first
//! 2 MiB a crawler reads whatever the fonts and inlined media weigh.
//!
//! Everything the page interpolates is escaped for where it sits: the title as text, slide ids as attributes, the
//! `@font-face` rules' family names as CSS strings (by `slidock-fonts`), and the deck JSON so nothing inside it can
//! end its `<script>` (`<` as `\u003c`) or trip an old JavaScript parser (U+2028 and U+2029 as escapes).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest, Sha256};
use slidock_format::escape::{escape_attribute, escape_text};
use slidock_format::markers::{CHROME_STYLE_ID, DECK_SCRIPT_ID, FONTS_STYLE_ID, SLIDE_TEMPLATE};
use slidock_html::deck::Deck;

use crate::{BundleError, SlideTemplate, player};

/// The page's own rules: a black, full-window page under the player's stage (`.sd-page`, fixed to the window), as
/// every page has. A page without a text view has nothing to scroll. With one ([`TEXT_VIEW_CSS`]), the text view is an
/// opaque reading column one window down, above the stage in z-order, so the page scrolls it up over the stage while
/// the stage stays where it is (and is drawn as it is on a page without a view); without scripts it is the page
/// ([`NOSCRIPT_CSS`]). Only a body that holds no text view hides its overflow (a host that replaced the body's content
/// keeps the full-window page), and a browser without `:has()` drops that rule and scrolls.
pub const CHROME_CSS: &str = "html,body{margin:0;padding:0;width:100%;height:100%;background:#000}\
body{overflow:hidden}\
body>noscript p{margin:0;padding:2em;color:#e5e7eb;font:16px/1.5 system-ui,sans-serif;text-align:center}";

/// [`CHROME_CSS`] for a page with a text view: the reading column (46rem wide at most, on a background as wide as the
/// window), one window down while scripts run.
pub const TEXT_VIEW_CSS: &str = "html,body{margin:0;padding:0;width:100%;height:100%;background:#000}\
body:not(:has(>#sd-text)){overflow:hidden}\
#sd-text{position:relative;z-index:1;box-sizing:border-box;margin:100vh 0 0;margin-top:100dvh;\
padding:2rem max(1rem,calc(50% - 22rem)) 4rem;background:#000;color:#e5e7eb;font:16px/1.6 system-ui,sans-serif;\
overflow-wrap:anywhere;overflow-x:auto}\
#sd-text a{color:#93c5fd}\
#sd-text h1{margin:0 0 1rem;font-size:1.75rem;line-height:1.3}\
#sd-text h2{margin:2rem 0 .5rem;font-size:1.25rem;line-height:1.4}\
#sd-text p,#sd-text ul,#sd-text ol,#sd-text table{margin:0 0 .75rem}\
#sd-text ol{list-style:none}\
#sd-text table{border-collapse:collapse}\
#sd-text td,#sd-text th{border:1px solid #6b7280;padding:.25rem .5rem;text-align:left;vertical-align:top}\
#sd-text details{margin:0 0 .75rem;padding:.5rem .75rem;border-left:3px solid #6b7280}\
#sd-text summary{cursor:pointer;color:#d1d5db}";

/// Without scripts no stage holds the first screen: the text view is the page.
pub const NOSCRIPT_CSS: &str = "#sd-text{margin-top:0}";

/// What a page without a text view shows without JavaScript.
pub const NOSCRIPT: &str = "This presentation needs JavaScript to play.";

/// A CSP source expression for an inline script: `'sha256-<base64 of the SHA-256 of its text>'`.
pub fn script_source(script: &str) -> String {
    format!(
        "'sha256-{}'",
        STANDARD.encode(Sha256::digest(script.as_bytes()))
    )
}

/// The page's policy: scripts only by hash, inline styles, media and images from the deck's own files (or `data:`
/// and `blob:`), fonts only inline, and nothing to connect to, frame, submit or rebase.
pub fn content_security_policy(scripts: &[&str]) -> String {
    let hashes: Vec<String> = scripts.iter().map(|script| script_source(script)).collect();
    format!(
        "default-src 'none'; script-src {}; style-src 'unsafe-inline'; img-src 'self' data: blob: file:; \
         media-src 'self' data: blob: file:; font-src data: blob:; connect-src 'none'; frame-src 'none'; \
         object-src 'none'; base-uri 'none'; form-action 'none'",
        hashes.join(" ")
    )
}

/// The deck JSON as the text of `<script type="application/json">`.
pub fn deck_script(deck: &Deck) -> Result<String, BundleError> {
    let json = deck
        .to_script_json()
        .map_err(|error| BundleError::Deck(error.to_string()))?;
    Ok(json
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029"))
}

/// What the page is made of.
pub struct Page<'a> {
    /// Its `title` is the page's title.
    pub deck: &'a Deck,
    /// The `@font-face` rules, in order.
    pub font_rules: &'a [String],
    pub templates: &'a [SlideTemplate],
    /// The page's description, for its snippet and link previews.
    pub description: Option<&'a str>,
    /// The deck's public URL (checked by [`crate::site_url`]): its canonical URL.
    pub site_url: Option<&'a str>,
    /// `<article id="sd-text">`, or `None` for a page without one.
    pub text_view: Option<&'a str>,
    /// Ship the presenter view: its script, and the boot that hands it to the player.
    pub presenter: bool,
}

/// The head's metadata: the description, Open Graph and Twitter card tags, and the canonical URL when the deck has a
/// public one. No image, no robots directive and no clock value.
fn metadata(title: &str, description: Option<&str>, site_url: Option<&str>) -> Vec<String> {
    let mut tags = Vec::new();
    let meta = |key: &str, name: &str, content: &str| {
        format!(
            "<meta {key}=\"{name}\" content=\"{}\">",
            escape_attribute(content)
        )
    };
    if let Some(description) = description {
        tags.push(meta("name", "description", description));
    }
    tags.push(meta("property", "og:type", "website"));
    tags.push(meta("property", "og:title", title));
    if let Some(description) = description {
        tags.push(meta("property", "og:description", description));
    }
    tags.push(meta("name", "twitter:card", "summary"));
    if let Some(url) = site_url {
        tags.push(format!(
            "<link rel=\"canonical\" href=\"{}\">",
            escape_attribute(url)
        ));
        tags.push(meta("property", "og:url", url));
    }
    tags
}

/// Writes `index.html`.
pub fn index_html(page: &Page<'_>) -> Result<String, BundleError> {
    let deck = page.deck;
    let json = deck_script(deck)?;
    let lang = deck
        .lang
        .as_deref()
        .map(|lang| format!(" lang=\"{}\"", escape_attribute(lang)))
        .unwrap_or_default();
    let scripts = player::scripts(page.presenter);
    let csp = content_security_policy(&scripts);
    let size = json.len()
        + page.font_rules.iter().map(String::len).sum::<usize>()
        + page
            .templates
            .iter()
            .map(|t| t.html.len() + 64)
            .sum::<usize>()
        + scripts
            .iter()
            .map(|script| script.len() + 20)
            .sum::<usize>()
        + page.text_view.map_or(0, str::len)
        + 4096;
    let mut html = String::with_capacity(size);
    let mut line = |text: &str| {
        html.push_str(text);
        html.push('\n');
    };
    line("<!doctype html>");
    line(&format!("<html{lang}>"));
    line("<head>");
    line("<meta charset=\"utf-8\">");
    line("<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">");
    line(&format!(
        "<meta http-equiv=\"Content-Security-Policy\" content=\"{}\">",
        escape_attribute(&csp)
    ));
    line(&format!("<title>{}</title>", escape_text(&deck.title)));
    for tag in metadata(&deck.title, page.description, page.site_url) {
        line(&tag);
    }
    let chrome = if page.text_view.is_some() {
        TEXT_VIEW_CSS
    } else {
        CHROME_CSS
    };
    line(&format!("<style id=\"{CHROME_STYLE_ID}\">{chrome}</style>"));
    if page.text_view.is_some() {
        line(&format!(
            "<noscript><style>{NOSCRIPT_CSS}</style></noscript>"
        ));
    }
    line("</head>");
    line("<body>");
    match page.text_view {
        Some(view) => line(view),
        None => line(&format!("<noscript><p>{NOSCRIPT}</p></noscript>")),
    }
    line(&format!(
        "<style id=\"{FONTS_STYLE_ID}\">{}</style>",
        page.font_rules.join("\n")
    ));
    line(&format!(
        "<script type=\"application/json\" id=\"{DECK_SCRIPT_ID}\">{json}</script>"
    ));
    for template in page.templates {
        line(&format!(
            "<template {SLIDE_TEMPLATE}=\"{}\">{}</template>",
            escape_attribute(&template.id),
            template.html
        ));
    }
    for script in &scripts {
        line(&format!("<script>{script}</script>"));
    }
    line("</body>");
    line("</html>");
    Ok(html)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_a_script_as_csp_source_expressions_do() {
        // `echo -n 'alert(1)' | openssl dgst -sha256 -binary | base64`
        assert_eq!(
            script_source("alert(1)"),
            "'sha256-bhHHL3z2vDgxUt0W3dWQOrprscmda2Y5pLsLg4GF+pI='"
        );
    }
}
