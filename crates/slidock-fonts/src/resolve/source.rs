//! Font sources and the order they fill face slots in.

use serde::Serialize;

/// Where a face comes from. The bundled substitutes are not a source: they are the floor under every source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceKind {
    /// Embedded in the presentation (PPTX `.fntdata`, ODP `Fonts/`).
    Embedded,
    /// Installed on the viewer's computer: Local Font Access in the web app, the system font folders in the CLI.
    ViewerInstalled,
    Google,
    /// Files the user dropped (the CLI's `--fonts` folders).
    UserDropped,
}

impl SourceKind {
    pub const ALL: [SourceKind; 4] = [
        SourceKind::Embedded,
        SourceKind::ViewerInstalled,
        SourceKind::Google,
        SourceKind::UserDropped,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            SourceKind::Embedded => "embedded",
            SourceKind::ViewerInstalled => "viewerInstalled",
            SourceKind::Google => "google",
            SourceKind::UserDropped => "userDropped",
        }
    }
}

/// The sources that may fill slots, first to last: a slot takes the face of the first source that has it exactly.
/// A source left out is off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceOrder {
    sources: [SourceKind; 4],
    len: usize,
}

impl Default for SourceOrder {
    /// Embedded, then installed, then Google Fonts, then dropped files.
    fn default() -> Self {
        SourceOrder {
            sources: SourceKind::ALL,
            len: SourceKind::ALL.len(),
        }
    }
}

impl SourceOrder {
    /// The sources in this order; a repeated source keeps its first place.
    pub fn new(sources: impl IntoIterator<Item = SourceKind>) -> SourceOrder {
        let mut order = SourceOrder::off();
        for source in sources {
            if !order.contains(source)
                && let Some(slot) = order.sources.get_mut(order.len)
            {
                *slot = source;
                order.len += 1;
            }
        }
        order
    }

    /// No source: only the bundled floor binds.
    pub const fn off() -> SourceOrder {
        SourceOrder {
            sources: SourceKind::ALL,
            len: 0,
        }
    }

    /// This order without one source.
    pub fn without(&self, source: SourceKind) -> SourceOrder {
        SourceOrder::new(self.sources().iter().copied().filter(|s| *s != source))
    }

    pub fn sources(&self) -> &[SourceKind] {
        self.sources.get(..self.len).unwrap_or_default()
    }

    pub fn contains(&self, source: SourceKind) -> bool {
        self.sources().contains(&source)
    }

    pub fn is_off(&self) -> bool {
        self.len == 0
    }

    /// The source's place, first is 0; `None` when it is off.
    pub fn rank(&self, source: SourceKind) -> Option<usize> {
        self.sources().iter().position(|s| *s == source)
    }

    /// Whether `a` comes before `b` (an off source comes after every source that is on).
    pub fn before(&self, a: SourceKind, b: SourceKind) -> bool {
        match (self.rank(a), self.rank(b)) {
            (Some(a), Some(b)) => a < b,
            (Some(_), None) => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_order_is_embedded_installed_google_dropped() {
        let order = SourceOrder::default();
        assert_eq!(order.sources(), &SourceKind::ALL);
        assert!(order.before(SourceKind::Embedded, SourceKind::Google));
        assert!(!order.before(SourceKind::UserDropped, SourceKind::Google));
    }

    #[test]
    fn a_repeated_source_keeps_its_first_place_and_a_missing_one_is_off() {
        let order = SourceOrder::new([
            SourceKind::UserDropped,
            SourceKind::Embedded,
            SourceKind::UserDropped,
        ]);
        assert_eq!(
            order.sources(),
            &[SourceKind::UserDropped, SourceKind::Embedded]
        );
        assert_eq!(order.rank(SourceKind::Google), None);
        assert!(order.before(SourceKind::Embedded, SourceKind::Google));
        assert!(
            !order
                .without(SourceKind::Embedded)
                .contains(SourceKind::Embedded)
        );
        assert!(SourceOrder::off().is_off());
        assert_eq!(SourceOrder::off().sources(), &[]);
    }
}
