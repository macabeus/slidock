//! Diagnostics: what frontends, passes, backends and the verifier report. A diagnostic is a registered code and a site;
//! its severity and feature come from the registry.

mod registry;

use std::borrow::Cow;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::features::FeatureId;
use crate::ids::NodeId;
use crate::origin::Origin;

pub use registry::{DiagnosticCode, UnregisteredCode, codes};

/// Most severe first. `error`: a presentation or container failed to compile; `decline`: a feature is present but
/// unrepresented, visibly (the slide goes unscored); `lossy`: approximated, or lost invisibly in the static
/// render; `warning`: unusual input handled; `info`: facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ir/")]
pub enum Severity {
    Error,
    Decline,
    Lossy,
    Warning,
    Info,
}

impl Severity {
    pub const ALL: [Severity; 5] = [
        Severity::Error,
        Severity::Decline,
        Severity::Lossy,
        Severity::Warning,
        Severity::Info,
    ];

    /// Whether `self` is at least as severe as `floor`.
    pub fn at_least(self, floor: Severity) -> bool {
        self <= floor
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Decline => "decline",
            Severity::Lossy => "lossy",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }
}

/// The part of the compiler that reported a diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    Frontend,
    Pass,
    Backend,
    Driver,
    Verify,
}

/// Code prefixes, one per emitter family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CodePrefix {
    Ir,
    Kn,
    Px,
    Gs,
    Pd,
    Mid,
    Html,
    Rx,
    Exp,
    Od,
}

impl CodePrefix {
    pub const ALL: [CodePrefix; 10] = [
        CodePrefix::Ir,
        CodePrefix::Kn,
        CodePrefix::Px,
        CodePrefix::Gs,
        CodePrefix::Pd,
        CodePrefix::Mid,
        CodePrefix::Html,
        CodePrefix::Rx,
        CodePrefix::Exp,
        CodePrefix::Od,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            CodePrefix::Ir => "IR",
            CodePrefix::Kn => "KN",
            CodePrefix::Px => "PX",
            CodePrefix::Gs => "GS",
            CodePrefix::Pd => "PD",
            CodePrefix::Mid => "MID",
            CodePrefix::Html => "HTML",
            CodePrefix::Rx => "RX",
            CodePrefix::Exp => "EXP",
            CodePrefix::Od => "OD",
        }
    }

    /// The numbers the prefix owns, inclusive.
    pub const fn range(self) -> (u16, u16) {
        match self {
            CodePrefix::Ir => (0, 999),
            CodePrefix::Kn => (1000, 1999),
            CodePrefix::Px => (2000, 2999),
            CodePrefix::Gs => (3000, 3999),
            CodePrefix::Pd => (4000, 4999),
            CodePrefix::Mid => (5000, 5999),
            CodePrefix::Html => (6000, 6999),
            CodePrefix::Rx => (7000, 7999),
            CodePrefix::Exp => (8000, 8999),
            CodePrefix::Od => (9000, 9999),
        }
    }

    /// Prefixes held for emitters slidock does not have; none of their codes is registered.
    pub const fn reserved(self) -> bool {
        matches!(
            self,
            CodePrefix::Gs | CodePrefix::Pd | CodePrefix::Rx | CodePrefix::Exp
        )
    }
}

/// Whether `code` has the shape of a diagnostic code (`^(IR|KN|PX|GS|PD|MID|HTML|RX|EXP|OD)\d{4}$`), registered or
/// not.
pub fn is_code_shaped(code: &str) -> bool {
    let digits = code.trim_start_matches(|c: char| c.is_ascii_uppercase());
    let letters = &code[..code.len() - digits.len()];
    digits.len() == 4
        && digits.bytes().all(|b| b.is_ascii_digit())
        && CodePrefix::ALL
            .iter()
            .any(|prefix| prefix.as_str() == letters)
}

/// At most this many data keys; the first ones are kept.
pub const MAX_DATA_KEYS: usize = 16;
/// Data strings are clipped to this many UTF-16 code units.
pub const MAX_DATA_STRING: usize = 256;

/// A data value: a string, a number or a boolean.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DataValue {
    Bool(bool),
    Num(f64),
    Str(String),
}

impl DataValue {
    /// `String(value)`, as a message template prints it.
    pub fn to_js_string(&self) -> String {
        match self {
            DataValue::Str(s) => s.clone(),
            DataValue::Num(n) => slidock_canon::to_js_string(*n),
            DataValue::Bool(b) => b.to_string(),
        }
    }
}

impl From<&str> for DataValue {
    fn from(value: &str) -> Self {
        DataValue::Str(value.to_owned())
    }
}

impl From<String> for DataValue {
    fn from(value: String) -> Self {
        DataValue::Str(value)
    }
}

impl From<&String> for DataValue {
    fn from(value: &String) -> Self {
        DataValue::Str(value.clone())
    }
}

impl From<bool> for DataValue {
    fn from(value: bool) -> Self {
        DataValue::Bool(value)
    }
}

impl From<f64> for DataValue {
    fn from(value: f64) -> Self {
        DataValue::Num(value)
    }
}

macro_rules! data_from_int {
    ($($ty:ty),*) => {
        $(impl From<$ty> for DataValue {
            fn from(value: $ty) -> Self {
                DataValue::Num(value as f64)
            }
        })*
    };
}
data_from_int!(u8, u16, u32, u64, usize, i32, i64);

/// Bounded facts about a diagnostic, in insertion order.
pub type DiagnosticData = IndexMap<String, DataValue>;

/// One diagnostic. `severity` is always the registry's for `code`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub severity: Severity,
    pub message: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub feature: Option<FeatureId>,
    /// The slide, layout or master it belongs to; absent for presentation-level diagnostics.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub container: Option<NodeId>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub node: Option<NodeId>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub origin: Option<Origin>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub data: Option<DiagnosticData>,
    pub stage: Stage,
    /// Who emitted it: `pptx`, `cascade-text`, `html`, `verify-source`.
    pub emitter: String,
    /// How many identical diagnostics this one stands for, once merged.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub count: Option<u32>,
}

/// Where a diagnostic points: everything but its stage and emitter, which the sink stamps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiagnosticSite {
    pub container: Option<NodeId>,
    pub node: Option<NodeId>,
    pub origin: Option<Origin>,
    /// Defaults to the code's registered feature.
    pub feature: Option<FeatureId>,
    /// Data entries in order; a `None` value is left out.
    pub data: Vec<(Cow<'static, str>, Option<DataValue>)>,
}

impl DiagnosticSite {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn container(mut self, container: impl Into<Option<NodeId>>) -> Self {
        self.container = container.into();
        self
    }

    pub fn node(mut self, node: impl Into<Option<NodeId>>) -> Self {
        self.node = node.into();
        self
    }

    pub fn origin(mut self, origin: impl Into<Option<Origin>>) -> Self {
        self.origin = origin.into();
        self
    }

    pub fn feature(mut self, feature: FeatureId) -> Self {
        self.feature = Some(feature);
        self
    }

    /// Appends a data entry.
    pub fn data(mut self, key: impl Into<Cow<'static, str>>, value: impl Into<DataValue>) -> Self {
        self.data.push((key.into(), Some(value.into())));
        self
    }

    /// Appends a data entry that may be absent.
    pub fn data_opt<V: Into<DataValue>>(
        mut self,
        key: impl Into<Cow<'static, str>>,
        value: Option<V>,
    ) -> Self {
        self.data.push((key.into(), value.map(Into::into)));
        self
    }
}

/// How frontends, passes and backends report: a code and a site. The severity is the registry's.
pub trait DiagnosticSink {
    fn report(&mut self, code: DiagnosticCode, site: DiagnosticSite);
}

/// Fills `code`'s template from `data`; a placeholder without a value stays as written.
pub fn render_message(code: DiagnosticCode, data: &DiagnosticData) -> String {
    let template = code.template();
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let (before, from_open) = rest.split_at(open);
        out.push_str(before);
        let after_open = &from_open[1..];
        let key_len = after_open
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(after_open.len());
        let key = &after_open[..key_len];
        if key_len > 0 && after_open[key_len..].starts_with('}') {
            match data.get(key) {
                Some(value) => out.push_str(&value.to_js_string()),
                None => {
                    out.push('{');
                    out.push_str(key);
                    out.push('}');
                }
            }
            rest = &after_open[key_len + 1..];
        } else {
            out.push('{');
            rest = after_open;
        }
    }
    out.push_str(rest);
    out
}

/// A diagnostic for `code`, with the registry's severity. Data keeps the first 16 entries, absent ones counted, then
/// leaves the absent ones out, and clips strings to 256 UTF-16 code units.
pub fn make_diagnostic(
    code: DiagnosticCode,
    stage: Stage,
    emitter: &str,
    site: DiagnosticSite,
) -> Diagnostic {
    let mut data = DiagnosticData::new();
    for (key, value) in site.data.into_iter().take(MAX_DATA_KEYS) {
        let Some(value) = value else { continue };
        let value = match value {
            DataValue::Str(s) => {
                DataValue::Str(slidock_canon::utf16::clip(&s, MAX_DATA_STRING).to_owned())
            }
            other => other,
        };
        data.insert(key.into_owned(), value);
    }
    Diagnostic {
        code,
        severity: code.severity(),
        message: render_message(code, &data),
        feature: site.feature.or(code.feature()),
        container: site.container,
        node: site.node,
        origin: site.origin,
        data: (!data.is_empty()).then_some(data),
        stage,
        emitter: emitter.to_owned(),
        count: None,
    }
}

/// A sink that collects diagnostics, stamping them with a stage and an emitter.
#[derive(Clone, Debug)]
pub struct Collect {
    pub stage: Stage,
    pub emitter: String,
    pub diagnostics: Vec<Diagnostic>,
}

impl Collect {
    pub fn new(stage: Stage, emitter: impl Into<String>) -> Self {
        Collect {
            stage,
            emitter: emitter.into(),
            diagnostics: Vec::new(),
        }
    }
}

impl DiagnosticSink for Collect {
    fn report(&mut self, code: DiagnosticCode, site: DiagnosticSite) {
        let diagnostic = make_diagnostic(code, self.stage, &self.emitter, site);
        self.diagnostics.push(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use super::codes::*;
    use super::*;

    #[test]
    fn registry_codes_and_names_are_unique_and_kebab_case() {
        let mut codes: Vec<&str> = DiagnosticCode::ALL.iter().map(|c| c.code()).collect();
        let mut names: Vec<&str> = DiagnosticCode::ALL.iter().map(|c| c.name()).collect();
        codes.sort_unstable();
        codes.dedup();
        names.sort_unstable();
        names.dedup();
        assert_eq!(codes.len(), DiagnosticCode::ALL.len());
        assert_eq!(names.len(), DiagnosticCode::ALL.len());
        for code in DiagnosticCode::ALL {
            let name = code.name();
            assert!(
                !name.is_empty()
                    && name.split('-').all(|part| !part.is_empty()
                        && part
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())),
                "{name}"
            );
        }
    }

    #[test]
    fn codes_sit_in_their_prefix_range_outside_reserved_ones() {
        for code in DiagnosticCode::ALL {
            assert!(is_code_shaped(code.code()), "{code}");
            let prefix = code.prefix();
            assert_eq!(
                prefix.as_str(),
                code.code().trim_end_matches(|c: char| c.is_ascii_digit())
            );
            let (from, to) = prefix.range();
            assert!((from..=to).contains(&code.number()), "{code}");
            assert!(!prefix.reserved(), "{code}");
        }
    }

    #[test]
    fn registers_every_ported_emitter_and_no_retired_number() {
        // The 179 static codes, the motion codes (KN1404–KN1409, PX2403, PX2410–PX2413, OD9404, OD9410–OD9413,
        // HTML6010, HTML6011), the refused vector originals (KN1113, PX2310, OD9306), an unread Instant Alpha
        // outline (KN1114), a medium the host's browser decoded (MID5608) and the preview drawn in place of an
        // original no host decoded (MID5609), a PPTX shadow not drawn (PX2111), a font found but refused (MID5404), an
        // emoji family drawn by the viewer's emoji font (MID5406), a part or slide listed again (PX2013, KN1012) and a
        // container left out of the output budget (IR0914) and a raster left out of the transcode budget (MID5610).
        assert_eq!(DiagnosticCode::ALL.len(), 179 + 18 + 6 + 1 + 2 + 4);
        assert!(DiagnosticCode::lookup("IR0112").is_none());
        assert!(DiagnosticCode::lookup("PX2999").is_none());
        assert!(DiagnosticCode::lookup("PD4001").is_none());
        assert_eq!(
            DiagnosticCode::lookup("PX2001").map(|c| c.name()),
            Some("not-pptx")
        );
        assert_eq!(IR0909.name(), "invariant-violated");
        assert_eq!(IR0910.severity(), Severity::Decline);
        assert_eq!(PX2401.feature(), Some(FeatureId::Animation));
    }

    #[test]
    fn templates_name_alphanumeric_placeholders() {
        for code in DiagnosticCode::ALL {
            let template = code.template();
            let mut rest = template;
            while let Some(open) = rest.find('{') {
                let after = &rest[open + 1..];
                let close = after.find('}').expect("a closed placeholder");
                let key = &after[..close];
                assert!(
                    !key.is_empty() && key.bytes().all(|b| b.is_ascii_alphanumeric()),
                    "{code}: {key}"
                );
                rest = &after[close + 1..];
            }
        }
    }

    #[test]
    fn renders_templates_and_keeps_unknown_placeholders() {
        let mut data = DiagnosticData::new();
        data.insert("id".into(), "pp-256".into());
        assert_eq!(
            render_message(IR0002, &data),
            "Id pp-256 is used more than once"
        );
        assert_eq!(
            render_message(IR0002, &DiagnosticData::new()),
            "Id {id} is used more than once"
        );
        let mut data = DiagnosticData::new();
        data.insert("kind".into(), "group".into());
        data.insert("limit".into(), 16u32.into());
        assert_eq!(render_message(IR0005, &data), "group depth exceeds 16");
    }

    #[test]
    fn make_diagnostic_takes_the_registry_severity() {
        let site = DiagnosticSite::new()
            .container(NodeId::new("pp-256").unwrap())
            .node(NodeId::new("pp-256.4").unwrap())
            .data("kind", "media")
            .data("target", "abc")
            .data_opt::<&str>("skipped", None);
        let diagnostic = make_diagnostic(IR0003, Stage::Verify, "verify-source", site);
        let json =
            slidock_canon::to_canonical_json(&diagnostic, slidock_canon::Layout::Compact).unwrap();
        assert_eq!(
            json,
            r#"{"code":"IR0003","container":"pp-256","data":{"kind":"media","target":"abc"},"emitter":"verify-source","message":"The media reference abc does not resolve","node":"pp-256.4","severity":"error","stage":"verify"}"#
        );
    }

    #[test]
    fn bounds_data() {
        let mut site = DiagnosticSite::new();
        for i in 0..20 {
            site = site.data(format!("k{i}"), "x".repeat(300));
        }
        let diagnostic = make_diagnostic(IR0001, Stage::Verify, "test", site);
        let data = diagnostic.data.unwrap();
        assert_eq!(data.len(), 16);
        assert_eq!(data.get("k0"), Some(&DataValue::Str("x".repeat(256))));
    }

    #[test]
    fn severities_order_most_severe_first() {
        assert!(Severity::Error.at_least(Severity::Lossy));
        assert!(Severity::Lossy.at_least(Severity::Lossy));
        assert!(!Severity::Info.at_least(Severity::Warning));
    }
}
