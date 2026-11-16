//! Ids and hashes, checked when they are made or read, and the closed vocabularies they combine: frontends, tiers,
//! lanes.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::Error;

macro_rules! checked_string {
    ($(#[$doc:meta])* $name:ident, $what:literal, $rule:literal, $check:expr) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<$name, Error> {
                let value = value.into();
                let check: fn(&str) -> bool = $check;
                if check(&value) {
                    Ok($name(value))
                } else {
                    Err(Error::Id { what: $what, value, rule: $rule })
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = Error;
            fn try_from(value: String) -> Result<$name, Error> {
                $name::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> String {
                value.0
            }
        }

        impl FromStr for $name {
            type Err = Error;
            fn from_str(value: &str) -> Result<$name, Error> {
                $name::new(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `^[a-z0-9][a-z0-9-]{0,63}$`
fn is_presentation_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && value.len() <= 64
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// `^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$`
fn is_slide_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_alphanumeric())
        && value.len() <= 128
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

checked_string!(
    /// A SHA-256 as 64 lowercase hex digits.
    Sha256,
    "sha256",
    "is not 64 lowercase hex digits",
    is_sha256
);

impl Sha256 {
    /// The SHA-256 of `data`.
    pub fn of(data: impl AsRef<[u8]>) -> Sha256 {
        Sha256(slidock_canon::sha256_hex(data))
    }
}

checked_string!(
    /// A presentation's id in the corpus manifest (`demo-a`).
    PresentationId,
    "presentation id",
    "does not match ^[a-z0-9][a-z0-9-]{0,63}$",
    is_presentation_id
);

checked_string!(
    /// A slide id as the compiler mints it (`pp-9001`, `od-p901`, `kn-1002`).
    SlideId,
    "slide id",
    "does not match ^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$",
    is_slide_id
);

/// The input formats the bench compiles. v1 has no PDF frontend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum Frontend {
    Pptx,
    Odp,
    Keynote,
    /// slidock IR JSON: the probe corpus.
    Ir,
}

impl Frontend {
    pub const ALL: [Frontend; 4] = [
        Frontend::Pptx,
        Frontend::Odp,
        Frontend::Keynote,
        Frontend::Ir,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Frontend::Pptx => "pptx",
            Frontend::Odp => "odp",
            Frontend::Keynote => "keynote",
            Frontend::Ir => "ir",
        }
    }

    pub fn from_wire(value: &str) -> Option<Frontend> {
        Frontend::ALL.into_iter().find(|f| f.as_str() == value)
    }
}

impl fmt::Display for Frontend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A presentation's tier. `run` selects smoke and real by default; holdout is read only by the gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Smoke,
    Real,
    Probe,
    Holdout,
}

impl Tier {
    /// What `run` selects without `--tier`.
    pub const DEFAULT: [Tier; 2] = [Tier::Smoke, Tier::Real];
}

/// What a run measures. `static` alone feeds `regression` against the static baseline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum Lane {
    /// The fully built slide against its oracle.
    Static,
    /// The shadow-root-mounted render against the standalone document's.
    Host,
    /// Injected fonts: source attribution and glyph coverage.
    Fonts,
    /// Build stages, transition frames and timeline fidelity.
    Motion,
    /// The downloaded zip opened from `file://`.
    Shell,
}

/// `<presentation>:<frontend>`: the unit a compile runs on and the corpus stamp is kept for. Ordered as its text
/// (`compareIds`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(try_from = "String", into = "String")]
#[ts(type = "string")]
pub struct PairKey {
    presentation: PresentationId,
    frontend: Frontend,
}

impl PairKey {
    pub fn new(presentation: PresentationId, frontend: Frontend) -> PairKey {
        PairKey {
            presentation,
            frontend,
        }
    }

    pub fn presentation(&self) -> &PresentationId {
        &self.presentation
    }

    pub fn frontend(&self) -> Frontend {
        self.frontend
    }
}

impl FromStr for PairKey {
    type Err = Error;
    fn from_str(value: &str) -> Result<PairKey, Error> {
        let invalid = || Error::Id {
            what: "pair key",
            value: value.to_owned(),
            rule: "is not <presentation>:<frontend>",
        };
        let (presentation, frontend) = value.split_once(':').ok_or_else(invalid)?;
        Ok(PairKey {
            presentation: PresentationId::new(presentation).map_err(|_| invalid())?,
            frontend: Frontend::from_wire(frontend).ok_or_else(invalid)?,
        })
    }
}

impl TryFrom<String> for PairKey {
    type Error = Error;
    fn try_from(value: String) -> Result<PairKey, Error> {
        value.parse()
    }
}

impl From<PairKey> for String {
    fn from(value: PairKey) -> String {
        value.to_string()
    }
}

/// Orders ids by their text in UTF-16 code units, as `compareIds` does.
macro_rules! ordered_as_text {
    ($name:ident) => {
        impl Ord for $name {
            fn cmp(&self, other: &$name) -> std::cmp::Ordering {
                slidock_canon::utf16::cmp(&self.to_string(), &other.to_string())
            }
        }

        impl PartialOrd for $name {
            fn partial_cmp(&self, other: &$name) -> Option<std::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }
    };
}

ordered_as_text!(PairKey);
ordered_as_text!(RowId);

impl fmt::Display for PairKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.presentation, self.frontend)
    }
}

/// Which frame of a slide a row scores. Static rows score the fully built slide.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RowStep {
    /// The fully built slide: `<p>:<slide>:<fe>`.
    Static,
    /// The settled state before click `k + 1` (motion lane): `<p>:<slide>:<fe>#s<k>`.
    Stage(u32),
    /// The transition into the slide at this per-mille of its duration (motion lane): `<p>:<slide>:<fe>#t@<pm>`.
    Transition(u16),
}

/// A row: `<presentation>:<slide>:<frontend>`, with a `#s<k>` or `#t@<pm>` suffix for motion rows. Ordered as its
/// text, so maps of rows iterate in id order.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(try_from = "String", into = "String")]
#[ts(type = "string")]
pub struct RowId {
    presentation: PresentationId,
    slide: SlideId,
    frontend: Frontend,
    step: RowStep,
}

impl RowId {
    pub fn new(presentation: PresentationId, slide: SlideId, frontend: Frontend) -> RowId {
        RowId {
            presentation,
            slide,
            frontend,
            step: RowStep::Static,
        }
    }

    /// The same slide's row at another step.
    pub fn with_step(&self, step: RowStep) -> RowId {
        RowId {
            step,
            ..self.clone()
        }
    }

    pub fn presentation(&self) -> &PresentationId {
        &self.presentation
    }

    pub fn slide(&self) -> &SlideId {
        &self.slide
    }

    pub fn frontend(&self) -> Frontend {
        self.frontend
    }

    pub fn step(&self) -> RowStep {
        self.step
    }

    pub fn pair(&self) -> PairKey {
        PairKey::new(self.presentation.clone(), self.frontend)
    }
}

impl FromStr for RowId {
    type Err = Error;
    fn from_str(value: &str) -> Result<RowId, Error> {
        let invalid = || Error::Id {
            what: "row id",
            value: value.to_owned(),
            rule: "is not <presentation>:<slide>:<frontend>[#s<k>|#t@<pm>]",
        };
        let (base, step) = match value.split_once('#') {
            None => (value, RowStep::Static),
            Some((base, suffix)) => (base, parse_step(suffix).ok_or_else(invalid)?),
        };
        let mut parts = base.split(':');
        let (Some(presentation), Some(slide), Some(frontend), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(invalid());
        };
        Ok(RowId {
            presentation: PresentationId::new(presentation).map_err(|_| invalid())?,
            slide: SlideId::new(slide).map_err(|_| invalid())?,
            frontend: Frontend::from_wire(frontend).ok_or_else(invalid)?,
            step,
        })
    }
}

/// A canonical decimal: no sign, no leading zero.
fn decimal<T: FromStr>(text: &str) -> Option<T> {
    let canonical = text == "0"
        || (!text.is_empty() && !text.starts_with('0') && text.bytes().all(|b| b.is_ascii_digit()));
    canonical.then(|| text.parse().ok()).flatten()
}

fn parse_step(suffix: &str) -> Option<RowStep> {
    if let Some(k) = suffix.strip_prefix("s") {
        return decimal(k).map(RowStep::Stage);
    }
    let pm: u16 = decimal(suffix.strip_prefix("t@")?)?;
    (1..1000).contains(&pm).then_some(RowStep::Transition(pm))
}

impl TryFrom<String> for RowId {
    type Error = Error;
    fn try_from(value: String) -> Result<RowId, Error> {
        value.parse()
    }
}

impl From<RowId> for String {
    fn from(value: RowId) -> String {
        value.to_string()
    }
}

impl fmt::Display for RowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.presentation, self.slide, self.frontend)?;
        match self.step {
            RowStep::Static => Ok(()),
            RowStep::Stage(k) => write!(f, "#s{k}"),
            RowStep::Transition(pm) => write!(f, "#t@{pm}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_is_64_lowercase_hex_digits() {
        let ok = "0".repeat(63) + "a";
        assert!(Sha256::new(ok.clone()).is_ok());
        assert!(Sha256::new(ok.to_uppercase()).is_err());
        assert!(Sha256::new("0".repeat(63)).is_err());
        assert!(Sha256::new("0".repeat(65)).is_err());
        assert!(Sha256::new("g".repeat(64)).is_err());
        assert!(serde_json::from_str::<Sha256>("\"abc\"").is_err());
        assert_eq!(
            serde_json::to_string(&Sha256::new(ok.clone()).unwrap()).unwrap(),
            format!("\"{ok}\"")
        );
    }

    #[test]
    fn presentation_ids_follow_the_manifest_pattern() {
        for ok in ["demo-a", "demo-c", "0", &"a".repeat(64)] {
            assert!(PresentationId::new(ok).is_ok(), "{ok}");
        }
        for bad in ["", "-a", "Demo-a", "a_b", "a:b", &"a".repeat(65)] {
            assert!(PresentationId::new(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn slide_ids_follow_the_compiler_pattern() {
        for ok in ["pp-9001", "od-p901", "kn-1002", "A.b_c-d", &"a".repeat(128)] {
            assert!(SlideId::new(ok).is_ok(), "{ok}");
        }
        for bad in ["", ".a", "a:b", "a#b", "a b", &"a".repeat(129)] {
            assert!(SlideId::new(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn frontends_exclude_pdf() {
        assert_eq!(
            serde_json::from_str::<Frontend>("\"keynote\"").unwrap(),
            Frontend::Keynote
        );
        assert!(serde_json::from_str::<Frontend>("\"pdf\"").is_err());
        for frontend in Frontend::ALL {
            assert_eq!(
                serde_json::to_string(&frontend).unwrap(),
                format!("\"{}\"", frontend.as_str())
            );
        }
    }

    #[test]
    fn row_ids_round_trip_with_their_step() {
        for (text, step) in [
            ("demo-a:pp-9001:pptx", RowStep::Static),
            ("demo-b:kn-1001:keynote#s0", RowStep::Stage(0)),
            ("demo-b:kn-1001:keynote#s12", RowStep::Stage(12)),
            ("demo-c:od-p901:odp#t@250", RowStep::Transition(250)),
            ("probe:s1:ir#t@750", RowStep::Transition(750)),
        ] {
            let id: RowId = text.parse().unwrap();
            assert_eq!(id.step(), step, "{text}");
            assert_eq!(id.to_string(), text);
            let json = serde_json::to_string(&id).unwrap();
            assert_eq!(serde_json::from_str::<RowId>(&json).unwrap(), id);
        }
        let id: RowId = "demo-a:pp-9001:pptx".parse().unwrap();
        assert_eq!(id.presentation().as_str(), "demo-a");
        assert_eq!(id.slide().as_str(), "pp-9001");
        assert_eq!(id.frontend(), Frontend::Pptx);
        assert_eq!(id.pair().to_string(), "demo-a:pptx");
        assert_eq!(
            id.with_step(RowStep::Stage(1)).to_string(),
            "demo-a:pp-9001:pptx#s1"
        );
    }

    #[test]
    fn row_ids_refuse_other_shapes() {
        for bad in [
            "demo-a:pp-9001",
            "demo-a:pp-9001:pdf",
            "demo-a:pp-9001:pptx:x",
            "Demo-a:pp-9001:pptx",
            "demo-a::pptx",
            "demo-a:pp-9001:pptx#",
            "demo-a:pp-9001:pptx#s",
            "demo-a:pp-9001:pptx#s01",
            "demo-a:pp-9001:pptx#s-1",
            "demo-a:pp-9001:pptx#t@0",
            "demo-a:pp-9001:pptx#t@1000",
            "demo-a:pp-9001:pptx#t250",
            "demo-a:pp-9001:pptx#x1",
        ] {
            assert!(bad.parse::<RowId>().is_err(), "{bad}");
        }
    }

    #[test]
    fn ids_order_as_their_text() {
        let texts = [
            "a-b:s1:odp",
            "a:s10:pptx",
            "a:s1:keynote",
            "a:s1:keynote#s0",
            "a:s1:keynote#t@250",
            "a:s1:odp",
            "a:s1:pptx",
            "a:s2:pptx",
        ];
        let ids: std::collections::BTreeSet<RowId> =
            texts.iter().rev().map(|t| t.parse().unwrap()).collect();
        let order: Vec<String> = ids.iter().map(RowId::to_string).collect();
        assert_eq!(order, texts);

        let pairs: std::collections::BTreeSet<PairKey> =
            ["a:pptx", "a:odp", "a-b:odp", "a:keynote"]
                .iter()
                .map(|t| t.parse().unwrap())
                .collect();
        let order: Vec<String> = pairs.iter().map(PairKey::to_string).collect();
        assert_eq!(order, ["a-b:odp", "a:keynote", "a:odp", "a:pptx"]);
    }

    #[test]
    fn pair_keys_round_trip() {
        let key: PairKey = "demo-a:odp".parse().unwrap();
        assert_eq!(key.presentation().as_str(), "demo-a");
        assert_eq!(key.frontend(), Frontend::Odp);
        assert_eq!(key.to_string(), "demo-a:odp");
        for bad in ["demo-a", "demo-a:pdf", "demo-a:odp:x", ":odp"] {
            assert!(bad.parse::<PairKey>().is_err(), "{bad}");
        }
    }
}
