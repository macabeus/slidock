//! The page's own metadata: its default title and description, the check of its public URL, and the
//! notice for a page past what a crawler reads.

use serde::{Deserialize, Serialize};
use slidock_html::deck::Deck;
use thiserror::Error;
use ts_rs::TS;

use crate::{BundleReport, INDEX_PATH};

/// The longest default description, in characters: about what a search snippet shows.
pub const DESCRIPTION_CHARS: usize = 160;

/// What Googlebot reads of a file: its first 2 MiB, uncompressed.
pub const CRAWL_BYTES: u64 = 2 * 1024 * 1024;

/// The separator of the default description's titles.
const SEPARATOR: &str = " · ";

/// A page's title and description when the user gives none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "bundle/")]
pub struct PageDefaults {
    /// The first shown slide's title, else the input's name.
    pub title: String,
    /// The shown slides' distinct titles after the page title, in order, joined with ` · ` and cut at the last whole
    /// title within 160 characters; none when there is none.
    pub description: Option<String>,
}

/// The defaults of a deck's page, from its deck JSON (the CLI and the web app call the same function, so the dialog
/// prefills what the CLI writes).
pub fn page_defaults(deck: &Deck) -> PageDefaults {
    let mut shown = deck.slides.iter().filter(|slide| !slide.hidden);
    let first = shown.next().map(|slide| collapse(&slide.title));
    let title = first
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| deck.title.clone());
    let mut titles: Vec<String> = Vec::new();
    for slide in deck.slides.iter().filter(|slide| !slide.hidden) {
        let text = collapse(&slide.title);
        if !text.is_empty() && text != title && !titles.contains(&text) {
            titles.push(text);
        }
    }
    let mut description = String::new();
    for text in titles {
        let joined = if description.is_empty() {
            text
        } else {
            format!("{description}{SEPARATOR}{text}")
        };
        if joined.chars().count() > DESCRIPTION_CHARS {
            break;
        }
        description = joined;
    }
    PageDefaults {
        title,
        description: (!description.is_empty()).then_some(description),
    }
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Why a public URL was refused.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum SiteUrlError {
    #[error("the public URL must be an absolute http:// or https:// URL: {0}")]
    NotHttp(String),
    #[error("the public URL must not hold spaces or control characters: {0}")]
    Whitespace(String),
    #[error("the public URL must not hold a user name or password: {0}")]
    UserInfo(String),
    #[error("the public URL must not hold a fragment (#…): {0}")]
    Fragment(String),
    #[error("the public URL has no host: {0}")]
    NoHost(String),
}

/// `url` when a page may name it as its own canonical URL: absolute `http` or `https`, with a host, no user info,
/// no fragment, and no whitespace, control character or backslash (which browsers read as a slash). Anything else
/// is refused, never repaired.
pub fn site_url(url: &str) -> Result<String, SiteUrlError> {
    let refused = || url.to_owned();
    if url
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return Err(SiteUrlError::Whitespace(refused()));
    }
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| SiteUrlError::NotHttp(refused()))?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return Err(SiteUrlError::NotHttp(refused()));
    }
    if url.contains('#') {
        return Err(SiteUrlError::Fragment(refused()));
    }
    let authority = rest.split(['/', '?']).next().unwrap_or_default();
    if authority.contains('@') {
        return Err(SiteUrlError::UserInfo(refused()));
    }
    let host = match authority.strip_prefix('[') {
        Some(ipv6) => ipv6.split(']').next().unwrap_or_default(),
        None => authority.split(':').next().unwrap_or_default(),
    };
    if host.is_empty() {
        return Err(SiteUrlError::NoHost(refused()));
    }
    Ok(url.to_owned())
}

/// The notice for an `index.html` past the 2 MiB a crawler reads (from the report's file sizes); `None` within it.
/// `text_view`: the page carries one, which then sits inside that part.
pub fn size_notice(report: &BundleReport, text_view: bool) -> Option<String> {
    let bytes = report
        .files
        .iter()
        .find(|file| file.path == INDEX_PATH)?
        .bytes;
    if bytes <= CRAWL_BYTES {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let mib = bytes as f64 / 1_048_576.0;
    let held = if text_view {
        "that holds the page's text but not the player, so Google's preview shows no slide"
    } else {
        "the page has no text view (--no-text-view), so Google indexes none of its slides"
    };
    Some(format!(
        "{INDEX_PATH} is {mib:.1} MiB. Google reads the first 2 MiB: {held}. Leave out single-file media or whole \
         fonts to fit."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_every_url_but_an_absolute_http_one() {
        for good in [
            "https://example.com/talk/",
            "http://example.com",
            "HTTPS://example.com:8443/a?b=c",
            "https://[::1]:8080/x",
        ] {
            assert_eq!(site_url(good).as_deref(), Ok(good), "{good}");
        }
        for bad in [
            "ftp://example.com/",
            "/talk/index.html",
            "talk/index.html",
            "example.com",
            "javascript://example.com/%0aalert(1)",
            "https://user:pass@example.com/",
            "https://example.com/#slide-2",
            "https:///path",
            "https://example.com/a b",
            " https://example.com/",
            "https://example.com\\@evil.test/",
            "",
        ] {
            assert!(site_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_notice_names_the_size_past_two_mebibytes() {
        let report = |bytes| BundleReport {
            files: vec![crate::BundledFile {
                path: INDEX_PATH.to_owned(),
                bytes,
            }],
            ..BundleReport::default()
        };
        assert_eq!(size_notice(&report(CRAWL_BYTES), true), None);
        let notice = size_notice(&report(5_353_730), true).unwrap();
        assert!(
            notice.starts_with("index.html is 5.1 MiB. Google reads the first 2 MiB"),
            "{notice}"
        );
    }
}
