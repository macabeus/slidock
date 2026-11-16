//! Hyperlinks (`a:hlinkClick`): an external URL, a jump to another slide of the presentation, or a show action. URLs
//! are kept as written and never fetched; links to anything else (files, macros, programs) are dropped.

use indexmap::IndexMap;
use slidock_canon::utf16;
use slidock_ir::ids::NodeId;
use slidock_ir::text::{Link, LinkAction};

use crate::package::Rels;
use crate::xml::Element;

const URL_MAX: usize = 2048;

pub(crate) struct LinkTargets<'a> {
    /// The container's relationships.
    pub rels: &'a Rels,
    /// Slide ids by part.
    pub slides: &'a IndexMap<String, NodeId>,
}

fn action(name: &str) -> Option<LinkAction> {
    match name.to_lowercase().as_str() {
        "nextslide" => Some(LinkAction::Next),
        "previousslide" => Some(LinkAction::Prev),
        "firstslide" => Some(LinkAction::First),
        "lastslide" => Some(LinkAction::Last),
        _ => None,
    }
}

/// The jump of `ppaction://hlinkshowjump?jump=<word>`.
fn show_jump(action: &str) -> Option<&str> {
    let rest = action.strip_prefix("ppaction://hlinkshowjump?jump=")?;
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    rest.get(..end).filter(|word| !word.is_empty())
}

pub(crate) fn read_link(hlink: &Element, targets: &LinkTargets<'_>) -> Option<Link> {
    let written = hlink.attr("action").unwrap_or("");
    if let Some(jump) = show_jump(written) {
        return action(jump).map(|action| Link::Action { action });
    }
    let rel = targets.rels.get(hlink.rel_attr("id").unwrap_or(""))?;
    if written.starts_with("ppaction://hlinksldjump") || rel.kind == "slide" {
        if rel.external {
            return None;
        }
        return targets.slides.get(&rel.target).map(|slide| Link::Slide {
            slide: slide.clone(),
        });
    }
    let length = utf16::len(&rel.target);
    (rel.external && rel.kind == "hyperlink" && length > 0 && length <= URL_MAX).then(|| {
        Link::Url {
            url: rel.target.clone(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_word_of_a_show_jump() {
        assert_eq!(
            show_jump("ppaction://hlinkshowjump?jump=nextslide&x"),
            Some("nextslide")
        );
        assert_eq!(show_jump("ppaction://hlinkshowjump?jump="), None);
        assert_eq!(show_jump("ppaction://hlinksldjump"), None);
        assert_eq!(action("LastSlide"), Some(LinkAction::Last));
        assert_eq!(action("endshow"), None);
    }
}
