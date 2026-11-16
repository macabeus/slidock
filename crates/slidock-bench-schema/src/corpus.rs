//! The corpus manifest, `slidock-corpus@1`: which
//! presentations are scored, from which source files (found by sha256, the name is a hint), against which oracle,
//! with which frozen rows.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use slidock_ir::Nullable;
use slidock_ir::features::FeatureId;
use ts_rs::{Config, TS};

use crate::de::{double_option, nullable, optional};
use crate::error::{Error, Issues};
use crate::ids::{Frontend, PairKey, PresentationId, RowId, Sha256, SlideId, Tier};
use crate::literal::{One, True, literal};

/// A feature of the closed vocabulary (`slidock_ir::features::FeatureId`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Feature(pub FeatureId);

impl Feature {
    pub fn as_str(self) -> &'static str {
        self.0.as_str()
    }
}

impl PartialOrd for Feature {
    fn partial_cmp(&self, other: &Feature) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Feature {
    fn cmp(&self, other: &Feature) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl TS for Feature {
    type WithoutGenerics = Self;
    type OptionInnerType = Self;
    fn docs() -> Option<String> {
        Some(
            "/**\n * A source feature of the closed vocabulary (slidock-ir's `FeatureId`).\n */\n"
                .to_owned(),
        )
    }
    fn name(_: &Config) -> String {
        "Feature".to_owned()
    }
    fn decl(cfg: &Config) -> String {
        format!("type Feature = {};", Self::inline(cfg))
    }
    fn decl_concrete(cfg: &Config) -> String {
        Self::decl(cfg)
    }
    fn inline(_: &Config) -> String {
        FeatureId::ALL
            .iter()
            .map(|feature| format!("\"{}\"", feature.as_str()))
            .collect::<Vec<_>>()
            .join(" | ")
    }
    fn output_path() -> Option<std::path::PathBuf> {
        Some("Feature.ts".into())
    }
}

/// The corpus manifest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CorpusManifest {
    pub version: One,
    pub presentations: Vec<Presentation>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Presentation {
    pub id: PresentationId,
    pub tier: Tier,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<String>", optional)]
    pub title: Option<String>,
    /// Features the presentation is known to use: each frontend must detect at least these.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Vec<Feature>>", optional)]
    pub features: Option<Vec<Feature>>,
    /// At least one, at most one per frontend.
    pub sources: Vec<Source>,
    pub oracle: Oracle,
    /// The motion lane's oracle: one page per build stage.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<StagesOracle>", optional)]
    pub stages: Option<StagesOracle>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<BTreeMap<Frontend, Vec<KnownLoss>>>", optional)]
    pub known_loss: Option<BTreeMap<Frontend, Vec<KnownLoss>>>,
    /// What the fonts lane expects to find.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<FontsExpect>", optional)]
    pub fonts_expect: Option<FontsExpect>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Source {
    pub frontend: Frontend,
    /// The file's name when the manifest was written: a hint only, sources resolve by sha256.
    pub file: String,
    /// Null only for a holdout file the gate is yet to read.
    #[serde(with = "nullable")]
    #[ts(as = "Option<Sha256>")]
    pub sha256: Option<Sha256>,
    #[serde(with = "nullable")]
    #[ts(as = "Option<u64>")]
    pub bytes: Option<u64>,
    /// The shown slides, in show order: oracle page k is row k. Absent until `corpus record-rows` records them;
    /// never stored for a holdout.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Vec<SlideId>>", optional)]
    pub rows: Option<Vec<SlideId>>,
    /// Holdout only: the row count, recorded by the gate.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    #[ts(as = "Nullable<u64>", optional)]
    pub row_count: Nullable<u64>,
    /// Holdout only: the sha256 of the canonical row list, recorded by the gate.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    #[ts(as = "Nullable<Sha256>", optional)]
    pub rows_sha256: Nullable<Sha256>,
    /// Rows classed and shown, summarised apart from the others. No v1 frontend sets it.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<True>", optional)]
    pub reference: Option<True>,
}

/// What a presentation's rows are scored against.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum Oracle {
    /// A PDF export (Google Slides), read from the corpus.
    #[serde(rename = "pdf-file")]
    PdfFile(PdfOracle),
    /// Keynote's own PDF export, made by the attended `oracle` command.
    #[serde(rename = "keynote-export")]
    KeynoteExport(KeynoteOracle),
}

/// Per raster key (extracting code, poppler, renderer platform), the sha256 of every page raster.
pub type PageHashes = BTreeMap<String, Vec<Sha256>>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PdfOracle {
    pub file: String,
    #[serde(with = "nullable")]
    #[ts(as = "Option<Sha256>")]
    pub sha256: Option<Sha256>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<PdfExporter>", optional)]
    pub exporter: Option<PdfExporter>,
    /// The page rasters, by raster key: the oracle's identity.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<PageHashes>", optional)]
    pub page_hashes: Option<PageHashes>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct PdfExporter {
    pub app: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<String>", optional)]
    pub via: Option<String>,
    #[serde(with = "nullable")]
    #[ts(as = "Option<String>")]
    pub version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeynoteExporter {
    pub bundle_id: String,
    pub version: String,
    pub build: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeynoteOracle {
    /// The `.key` source the export is made from.
    pub source: String,
    pub exporter: KeynoteExporter,
    /// The accepted export's sha256, written by `oracle --record`.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Sha256>", optional)]
    pub sha256: Option<Sha256>,
    /// The PostScript names the export must embed (the `pdffonts` check).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Vec<String>>", optional)]
    pub fonts: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<PageHashes>", optional)]
    pub page_hashes: Option<PageHashes>,
}

literal!(
    /// The stages oracle's kind.
    KeynoteStages = "keynote-stages"
);

/// Keynote's export with `all stages:true`: one page per build stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StagesOracle {
    pub kind: KeynoteStages,
    pub exporter: KeynoteExporter,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Sha256>", optional)]
    pub sha256: Option<Sha256>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<PageHashes>", optional)]
    pub page_hashes: Option<PageHashes>,
    /// Pages per slide, recorded by `oracle --record --stages`: their sum is the page count.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<BTreeMap<SlideId, u32>>", optional)]
    pub pages_per_row: Option<BTreeMap<SlideId, u32>>,
}

/// Which side lost a feature.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum LossKind {
    /// The source file lost it.
    Source,
    /// The oracle lacks it.
    Oracle,
}

/// A loss the manifest records: rows that show the feature are flagged and kept out of acceptance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct KnownLoss {
    pub kind: LossKind,
    pub feature: Feature,
    pub note: String,
}

/// The fonts lane's expectations of a presentation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FontsExpect {
    /// Families the source file embeds (`.fntdata`, ODP `Fonts/`).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Vec<String>>", optional)]
    pub embedded: Option<Vec<String>>,
    /// Families the Google provider must resolve.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Vec<String>>", optional)]
    pub open_families: Option<Vec<String>>,
}

/// One expected row: a shown slide of a (presentation, frontend) pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowRef {
    pub id: RowId,
    /// The oracle page that shows it, 1-based.
    pub page: u32,
    pub reference: bool,
}

/// The hashed part of a pair's corpus stamp.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CorpusStampInput<'a> {
    source: &'a Source,
    oracle: &'a Oracle,
    known_loss: &'a [KnownLoss],
}

impl CorpusManifest {
    /// Parses and validates a manifest.
    pub fn parse(text: &str) -> Result<CorpusManifest, Error> {
        let manifest: CorpusManifest = crate::parse(text)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// The manifest as committed: validated, canonical, two-space indent.
    pub fn to_canonical(&self) -> Result<String, Error> {
        self.validate()?;
        crate::to_canonical(self)
    }

    /// The rules no single field states (`manifestSchema.superRefine`).
    pub fn validate(&self) -> Result<(), Error> {
        let mut issues = Issues::default();
        let mut ids = BTreeSet::new();
        for presentation in &self.presentations {
            let id = &presentation.id;
            if !ids.insert(id) {
                issues.push(format!("Presentation {id} is listed twice"));
            }
            if presentation.sources.is_empty() {
                issues.push(format!("Presentation {id} lists no source"));
            }
            let mut frontends = BTreeSet::new();
            for source in &presentation.sources {
                if !frontends.insert(source.frontend) {
                    issues.push(format!("Presentation {id} lists a frontend twice"));
                }
                if source.file.is_empty() {
                    issues.push(format!("{id}: a {} source has no file", source.frontend));
                }
                let holdout = presentation.tier == Tier::Holdout;
                if holdout && source.rows.is_some() {
                    issues.push(format!(
                        "Holdout presentation {id} stores a row count, never rows"
                    ));
                }
                if !holdout && (source.sha256.is_none() || source.bytes.is_none()) {
                    issues.push(format!("{id}: {} needs its sha256 and size", source.file));
                }
            }
            match &presentation.oracle {
                Oracle::PdfFile(oracle) => {
                    if oracle.file.is_empty() {
                        issues.push(format!("{id}: the oracle has no file"));
                    }
                    if presentation.tier != Tier::Holdout && oracle.sha256.is_none() {
                        issues.push(format!("{id}: the oracle needs its sha256"));
                    }
                }
                Oracle::KeynoteExport(oracle) => {
                    if oracle.source.is_empty() {
                        issues.push(format!("{id}: the oracle names no source"));
                    }
                    if oracle.fonts.iter().flatten().any(String::is_empty) {
                        issues.push(format!("{id}: the oracle lists an empty font name"));
                    }
                }
            }
            if presentation.tier == Tier::Holdout
                && presentation
                    .stages
                    .as_ref()
                    .is_some_and(|stages| stages.pages_per_row.is_some())
            {
                issues.push(format!(
                    "Holdout presentation {id} stores stage pages per row, which name its rows"
                ));
            }
        }
        issues.finish()
    }

    pub fn presentation(&self, id: &PresentationId) -> Option<&Presentation> {
        self.presentations.iter().find(|p| &p.id == id)
    }

    /// Every expected row of the presentations in `tiers`, in manifest order.
    pub fn expected_rows(&self, tiers: &[Tier]) -> Vec<RowRef> {
        self.presentations
            .iter()
            .filter(|presentation| tiers.contains(&presentation.tier))
            .flat_map(|presentation| {
                presentation
                    .sources
                    .iter()
                    .flat_map(move |source| presentation.rows(source))
            })
            .collect()
    }
}

impl Presentation {
    pub fn source(&self, frontend: Frontend) -> Option<&Source> {
        self.sources.iter().find(|s| s.frontend == frontend)
    }

    pub fn pair(&self, frontend: Frontend) -> PairKey {
        PairKey::new(self.id.clone(), frontend)
    }

    /// A source's frozen rows, or none until they are recorded.
    pub fn rows(&self, source: &Source) -> Vec<RowRef> {
        source
            .rows
            .iter()
            .flatten()
            .zip(1u32..)
            .map(|(slide, page)| RowRef {
                id: RowId::new(self.id.clone(), slide.clone(), source.frontend),
                page,
                reference: source.reference.is_some(),
            })
            .collect()
    }

    /// The pair's corpus stamp: the hash of the manifest entries that define its rows (its source, the oracle and
    /// its known losses). `None` when the presentation has no source for `frontend`.
    pub fn corpus_stamp(&self, frontend: Frontend) -> Result<Option<(PairKey, Sha256)>, Error> {
        let Some(source) = self.source(frontend) else {
            return Ok(None);
        };
        let known_loss = self
            .known_loss
            .as_ref()
            .and_then(|losses| losses.get(&frontend))
            .map_or(&[][..], Vec::as_slice);
        let digest = crate::sha256_json(&CorpusStampInput {
            source,
            oracle: &self.oracle,
            known_loss,
        })?;
        Ok(Some((self.pair(frontend), digest)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4";

    fn manifest(presentations: &str) -> String {
        format!(r#"{{"version": 1, "presentations": [{presentations}]}}"#)
    }

    fn real(id: &str) -> String {
        format!(
            r#"{{"id": "{id}", "tier": "real", "features": ["text.run"],
                "sources": [{{"frontend": "pptx", "file": "a.pptx", "sha256": "{SHA}", "bytes": 3,
                              "rows": ["pp-1", "pp-2"]}}],
                "oracle": {{"kind": "pdf-file", "file": "a.pdf", "sha256": "{SHA}",
                            "exporter": {{"app": "Google Slides", "version": null}}}},
                "knownLoss": {{"pptx": [{{"kind": "source", "feature": "stroke", "note": "n"}}]}}}}"#
        )
    }

    #[test]
    fn parses_and_round_trips_canonically() {
        let parsed = CorpusManifest::parse(&manifest(&real("demo-a"))).unwrap();
        let text = parsed.to_canonical().unwrap();
        assert!(text.ends_with("}\n"));
        assert!(text.contains("\n  \"presentations\": ["));
        assert_eq!(CorpusManifest::parse(&text).unwrap(), parsed);
        assert_eq!(
            CorpusManifest::parse(&text)
                .unwrap()
                .to_canonical()
                .unwrap(),
            text
        );
        let rows = parsed.expected_rows(&Tier::DEFAULT);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].id.to_string(), "demo-a:pp-2:pptx");
        assert_eq!(rows[1].page, 2);
    }

    #[test]
    fn refuses_unknown_keys_features_and_frontends() {
        let base = real("demo-a");
        for (from, to) in [
            (r#""tier": "real""#, r#""tier": "real", "extra": 1"#),
            (r#""text.run""#, r#""text.nope""#),
            (r#""frontend": "pptx""#, r#""frontend": "pdf""#),
            (r#""kind": "pdf-file""#, r#""kind": "pdf""#),
            (r#""version": null"#, r#""version": null, "x": 1"#),
            (r#""version": null"#, r#""via": null, "version": null"#),
            (r#", "version": null"#, ""),
        ] {
            let text = manifest(&base.replace(from, to));
            assert!(CorpusManifest::parse(&text).is_err(), "{to}");
        }
        assert!(CorpusManifest::parse(r#"{"version": 2, "presentations": []}"#).is_err());
    }

    #[test]
    fn validates_the_cross_field_rules() {
        let twice = manifest(&format!("{},{}", real("demo-a"), real("demo-a")));
        let error = CorpusManifest::parse(&twice).unwrap_err().to_string();
        assert!(error.contains("listed twice"), "{error}");

        let two_pptx = manifest(&real("demo-a").replace(
            r#""sources": ["#,
            &format!(
                r#""sources": [{{"frontend": "pptx", "file": "b", "sha256": "{SHA}", "bytes": 1}},"#
            ),
        ));
        let error = CorpusManifest::parse(&two_pptx).unwrap_err().to_string();
        assert!(error.contains("frontend twice"), "{error}");

        let no_hash = manifest(&real("demo-a").replacen(
            &format!(r#""sha256": "{SHA}""#),
            r#""sha256": null"#,
            1,
        ));
        let error = CorpusManifest::parse(&no_hash).unwrap_err().to_string();
        assert!(error.contains("needs its sha256 and size"), "{error}");

        let no_oracle_hash = manifest(&real("demo-a").replace(
            &format!(r#""file": "a.pdf", "sha256": "{SHA}""#),
            r#""file": "a.pdf", "sha256": null"#,
        ));
        let error = CorpusManifest::parse(&no_oracle_hash)
            .unwrap_err()
            .to_string();
        assert!(error.contains("oracle needs its sha256"), "{error}");

        let holdout_rows =
            manifest(&real("demo-a").replace(r#""tier": "real""#, r#""tier": "holdout""#));
        let error = CorpusManifest::parse(&holdout_rows)
            .unwrap_err()
            .to_string();
        assert!(error.contains("never rows"), "{error}");

        let no_sources = manifest(
            r#"{"id": "x", "tier": "real", "sources": [],
                "oracle": {"kind": "keynote-export", "source": "x.key",
                           "exporter": {"bundleId": "b", "version": "14.5", "build": "1"}}}"#,
        );
        let error = CorpusManifest::parse(&no_sources).unwrap_err().to_string();
        assert!(error.contains("no source"), "{error}");
    }

    #[test]
    fn a_holdout_keeps_null_and_absent_apart() {
        let holdout = manifest(
            r#"{"id": "h", "tier": "holdout",
                "sources": [{"frontend": "odp", "file": "h.odp", "sha256": null, "bytes": null,
                             "rowCount": null, "rowsSha256": null}],
                "oracle": {"kind": "pdf-file", "file": "h.pdf", "sha256": null, "pageHashes": {}}}"#,
        );
        let parsed = CorpusManifest::parse(&holdout).unwrap();
        let source = &parsed.presentations[0].sources[0];
        assert_eq!(source.row_count, Some(None));
        assert_eq!(source.rows, None);
        let text = parsed.to_canonical().unwrap();
        assert!(text.contains("\"rowCount\": null"), "{text}");
        assert!(!text.contains("\"rows\""), "{text}");
    }

    #[test]
    fn corpus_stamp_hashes_source_oracle_and_losses() {
        let parsed = CorpusManifest::parse(&manifest(&real("demo-a"))).unwrap();
        let presentation = &parsed.presentations[0];
        let (pair, digest) = presentation.corpus_stamp(Frontend::Pptx).unwrap().unwrap();
        assert_eq!(pair.to_string(), "demo-a:pptx");
        let expected = crate::sha256_json(&serde_json::json!({
            "source": presentation.sources[0],
            "oracle": presentation.oracle,
            "knownLoss": presentation.known_loss.as_ref().unwrap()[&Frontend::Pptx],
        }))
        .unwrap();
        assert_eq!(digest, expected);
        assert!(presentation.corpus_stamp(Frontend::Odp).unwrap().is_none());
    }
}
