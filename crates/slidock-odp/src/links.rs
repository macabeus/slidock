//! Hyperlinks: `xlink:href` on `draw:a` and `text:a` (a URL, or `#<page name>` for another slide of the presentation),
//! and the click actions of `presentation:event-listener` (next, previous, first or last slide, or a named slide). Web,
//! mail and FTP URLs keep their written form; other targets (files, programs, macros) are dropped.

use std::collections::BTreeMap;

use slidock_canon::utf16;
use slidock_ir::ids::NodeId;
use slidock_ir::text::{Link, LinkAction};

use crate::names::qn;
use crate::odf::{Element, Ns};
use crate::package::decode_uri_component;

const URL_MAX: usize = 2048;

/// `^(?:https?|mailto|ftp):`, any case.
fn has_url_scheme(href: &str) -> bool {
    let lower: String = href
        .chars()
        .take(7)
        .collect::<String>()
        .to_ascii_lowercase();
    ["http:", "https:", "mailto:", "ftp:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
}

/// An `xlink:href` as a link: `#<page name>` jumps to that slide, a web, mail or FTP URL opens.
pub(crate) fn read_href(href: Option<&str>, pages: &BTreeMap<&str, NodeId>) -> Option<Link> {
    let href = href.filter(|s| !s.is_empty())?;
    if let Some(name) = href.strip_prefix('#') {
        // A malformed escape names the page as written.
        let name = decode_uri_component(name).unwrap_or_else(|| name.to_owned());
        return pages.get(name.as_str()).map(|slide| Link::Slide {
            slide: slide.clone(),
        });
    }
    (has_url_scheme(href) && utf16::len(href) <= URL_MAX).then(|| Link::Url {
        url: href.to_owned(),
    })
}

/// What clicking a drawable does (`office:event-listeners`), when it moves through the show.
pub(crate) fn click_action(element: &Element, pages: &BTreeMap<&str, NodeId>) -> Option<Link> {
    let listeners = element.child(Ns::Office, "event-listeners")?;
    for listener in listeners.children_of(Ns::Presentation, "event-listener") {
        if listener.attr(qn!("script:event-name")) != Some("dom:click") {
            continue;
        }
        let action = listener.attr(qn!("presentation:action")).unwrap_or("");
        let named = match action {
            "next-page" => Some(LinkAction::Next),
            "previous-page" => Some(LinkAction::Prev),
            "first-page" => Some(LinkAction::First),
            "last-page" => Some(LinkAction::Last),
            _ => None,
        };
        if let Some(action) = named {
            return Some(Link::Action { action });
        }
        if action == "show" {
            return read_href(listener.attr(qn!("xlink:href")), pages);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::odf::parse;

    fn pages() -> BTreeMap<&'static str, NodeId> {
        BTreeMap::from([("Intro", NodeId::new("od-p1").unwrap())])
    }

    fn url(href: &str) -> Option<Link> {
        read_href(Some(href), &pages())
    }

    fn opens(href: &str) -> Option<Link> {
        Some(Link::Url {
            url: href.to_owned(),
        })
    }

    #[test]
    fn opens_web_mail_and_ftp_urls_and_jumps_to_named_pages() {
        for href in [
            "http://a.example",
            "https://a.example/x",
            "mailto:a@example.com",
            "ftp://a.example/f",
            "HTTPS://A.EXAMPLE",
            "Ftp://a.example",
        ] {
            assert_eq!(url(href), opens(href), "{href}");
        }
        for href in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "macro:Standard.Module1",
            "ftps://a.example",
            "http//a.example",
            "",
        ] {
            assert_eq!(url(href), None, "{href}");
        }
        let longest = format!("https://{}", "a".repeat(2040));
        assert_eq!(url(&longest), opens(&longest));
        assert_eq!(url(&format!("{longest}a")), None);
        let slide = Some(Link::Slide {
            slide: NodeId::new("od-p1").unwrap(),
        });
        assert_eq!(url("#Intro"), slide);
        assert_eq!(url("#Intr%6F"), slide);
        assert_eq!(url("#Outro"), None);
        assert_eq!(read_href(None, &pages()), None);
    }

    fn clicked(action: &str, href: &str) -> Option<Link> {
        let element = parse(&format!(
            concat!(
                r#"<draw:frame xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" "#,
                r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" "#,
                r#"xmlns:presentation="urn:oasis:names:tc:opendocument:xmlns:presentation:1.0" "#,
                r#"xmlns:script="urn:oasis:names:tc:opendocument:xmlns:script:1.0" "#,
                r#"xmlns:xlink="http://www.w3.org/1999/xlink"><office:event-listeners>"#,
                r#"<presentation:event-listener script:event-name="dom:mouseover" presentation:action="last-page"/>"#,
                r#"<presentation:event-listener script:event-name="dom:click" presentation:action="{}" "#,
                r#"xlink:href="{}"/></office:event-listeners></draw:frame>"#
            ),
            action, href
        ));
        click_action(&element, &pages())
    }

    #[test]
    fn moves_through_the_show_on_a_click() {
        let action = |action| Some(Link::Action { action });
        assert_eq!(clicked("next-page", ""), action(LinkAction::Next));
        assert_eq!(clicked("previous-page", ""), action(LinkAction::Prev));
        assert_eq!(clicked("first-page", ""), action(LinkAction::First));
        assert_eq!(clicked("last-page", ""), action(LinkAction::Last));
        assert_eq!(
            clicked("show", "#Intro"),
            Some(Link::Slide {
                slide: NodeId::new("od-p1").unwrap()
            })
        );
        assert_eq!(
            clicked("show", "https://a.example"),
            opens("https://a.example")
        );
        for other in ["shown", "execute", "sound", "hide", "stop", ""] {
            assert_eq!(clicked(other, "#Intro"), None, "{other}");
        }
    }
}
