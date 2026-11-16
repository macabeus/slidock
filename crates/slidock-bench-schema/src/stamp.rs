//! The stamp: every input that can move a score, recorded in each results file. Two
//! results compare only when their stamps agree on everything but the compiler, which is named by a content id over
//! the trees it is built from.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::de::optional;
use crate::error::Error;
use crate::ids::{Lane, PairKey, Sha256, Tier};
use crate::literal::{True, literal};

/// The compiler under test.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CompilerStamp {
    /// The content id of the compiler (and bench) trees ([`content_id`]).
    pub content: Sha256,
    /// HEAD's commit.
    pub commit: String,
    /// The working copy differs from HEAD in the content id's paths.
    pub dirty: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DepsStamp {
    /// The closure digest of `Cargo.lock`: the packages the compiler crates reach.
    pub cargo_lock: Sha256,
    pub rustc: String,
    pub wasm_pack: String,
    /// The major version as `24.x`.
    pub node: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct HostStamp {
    pub platform: String,
    pub arch: String,
}

literal!(
    /// The renderer's colour profile.
    Srgb = "srgb"
);
literal!(
    /// A GIF renders as its first frame.
    FirstFrame = "first-frame"
);
literal!(
    /// A video renders as its poster, drawn by an `<img>`.
    PosterImg = "poster-img"
);

/// How the bench renders what a static page cannot show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RenderPolicy {
    pub gif: FirstFrame,
    pub video: PosterImg,
}

/// The pinned renderer container.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RendererStamp {
    /// The sha256 of the recipe (its files and the platform digest).
    pub recipe: Sha256,
    /// The base image's digest (`sha256:…`).
    pub platform_digest: String,
    pub image_id: String,
    /// `uname -m` inside the container.
    pub arch: String,
    /// `Browser.getVersion`'s product version.
    pub chromium: String,
    /// The CDP client (`chromiumoxide@<v>`).
    pub cdp_client: String,
    pub poppler: String,
    pub raster_long_side_px: u32,
    pub metric_long_side_px: u32,
    pub color_profile: Srgb,
    pub render_policy: RenderPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FontsStamp {
    /// The sha256 of the canonical sorted list of the pinned files' sha256s.
    pub font_set_id: Sha256,
    pub lock_sha256: Sha256,
    /// The synced folder differed from the lock (`--allow-font-drift`).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<True>", optional)]
    pub drift: Option<True>,
}

/// The metric versions (`ssim@1`, `color@1`, `pixel@1`, `text@2`, `shadow@1`, `gradient@1`, `motion@1`, `fonts@1`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct MetricsStamp {
    pub ssim: String,
    pub color: String,
    pub pixel: String,
    pub text: String,
    /// The static lane's.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<String>", optional)]
    pub shadow: Option<String>,
    /// The static lane's.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<String>", optional)]
    pub gradient: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<String>", optional)]
    pub motion: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<String>", optional)]
    pub fonts: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub tiers: Vec<Tier>,
}

/// The parity report a run carries, and the reference it compared against.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ParityStamp {
    /// The golden set the reference was rendered from.
    pub golden: crate::parity::GoldenIdentity,
    /// The sha256 of the reference file.
    pub reference: Sha256,
    /// The sha256 of the canonical parity report.
    pub report: Sha256,
}

/// The size of the compiler's wasm module: as built, and compressed with brotli at quality 11 as a server sends it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct WasmSize {
    pub raw: u64,
    pub brotli11: u64,
}

/// The size of a player build: as built, and gzipped at level 9 (zlib, as the player's build test measures it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ScriptSize {
    pub raw: u64,
    pub gzip9: u64,
}

/// The player's two builds: the IIFE the zip embeds and the ESM module the app imports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct PlayerSizes {
    pub iife: ScriptSize,
    pub esm: ScriptSize,
}

/// One compile of a deck to its zip: its time and the most memory it held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompileCost {
    /// CPU time (user and system), which the machine's load moves little: the native CLI's, and for wasm the
    /// headless shell's renderer processes' (the page and its worker) from the worker's `ready` to the zip.
    pub ms: u64,
    /// The native CLI's peak resident set; the wasm module's linear memory, which only grows.
    pub peak_bytes: u64,
}

/// A deck's compile, natively (the CLI) and as wasm (headless Chromium, through the app's compile worker).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DeckCost {
    pub native: CompileCost,
    pub wasm: CompileCost,
}

/// The sizes and compile costs `slidock-bench budgets` measured for the run's compiler.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PerfStamp {
    pub wasm: WasmSize,
    /// The sha256 of the measured module.
    pub wasm_sha256: Sha256,
    pub player: PlayerSizes,
    /// Per pair; each time is the fastest of `runs` compiles, each peak the largest.
    pub decks: BTreeMap<PairKey, DeckCost>,
    pub runs: u32,
    /// The headless Chromium the wasm compiles ran in.
    pub chromium: String,
}

/// A run's stamp.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Stamp {
    pub compiler: CompilerStamp,
    pub deps: DepsStamp,
    pub host: HostStamp,
    pub renderer: RendererStamp,
    pub fonts: FontsStamp,
    pub metrics: MetricsStamp,
    pub classifier: String,
    /// The harness hash ([`HarnessInputs::hash`]).
    pub harness: Sha256,
    /// Per pair, the hash of the manifest entries that define its rows (`Presentation::corpus_stamp`).
    pub corpus: BTreeMap<PairKey, Sha256>,
    pub selection: Selection,
    pub lane: Lane,
    /// An overlay run: only some rows were scored again.
    pub partial: bool,
    /// The determinism report's hash, written by `baseline`.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Sha256>", optional)]
    pub determinism: Option<Sha256>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<ParityStamp>", optional)]
    pub parity: Option<ParityStamp>,
    /// The sizes and compile costs, written by `budgets`.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<PerfStamp>", optional)]
    pub perf: Option<PerfStamp>,
}

impl Stamp {
    /// A run's key, which names its folder under `runs/`: the stamp without `partial`, so an identical run reuses
    /// its folder, and without what is stamped after the run (`determinism` by `baseline`, `parity` by
    /// `parity report`, `perf` by `budgets`), so the key stays the folder's.
    pub fn run_key(&self) -> Result<Sha256, Error> {
        crate::sha256_json(&RunKeyInput(self))
    }

    /// The fields that differ beyond the compiler, for the pairs compared: non-empty means the two runs are
    /// incomparable (`regression` and `diff` exit 2).
    pub fn incomparable(&self, other: &Stamp, pairs: &[PairKey]) -> Vec<String> {
        let mut fields = Vec::new();
        let mut check = |name: &str, equal: bool| {
            if !equal {
                fields.push(name.to_owned());
            }
        };
        check("harness", self.harness == other.harness);
        check(
            "renderer.recipe",
            self.renderer.recipe == other.renderer.recipe,
        );
        check(
            "renderer.platformDigest",
            self.renderer.platform_digest == other.renderer.platform_digest,
        );
        check("renderer.arch", self.renderer.arch == other.renderer.arch);
        check(
            "fonts.fontSetId",
            self.fonts.font_set_id == other.fonts.font_set_id,
        );
        check("metrics", self.metrics == other.metrics);
        check("classifier", self.classifier == other.classifier);
        check("deps", self.deps == other.deps);
        check("lane", self.lane == other.lane);
        for pair in pairs {
            check(
                &format!("corpus[{pair}]"),
                self.corpus.get(pair) == other.corpus.get(pair),
            );
        }
        fields
    }
}

/// The stamp as a run key hashes it.
struct RunKeyInput<'a>(&'a Stamp);

impl Serialize for RunKeyInput<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let s = self.0;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("compiler", &s.compiler)?;
        map.serialize_entry("deps", &s.deps)?;
        map.serialize_entry("host", &s.host)?;
        map.serialize_entry("renderer", &s.renderer)?;
        map.serialize_entry("fonts", &s.fonts)?;
        map.serialize_entry("metrics", &s.metrics)?;
        map.serialize_entry("classifier", &s.classifier)?;
        map.serialize_entry("harness", &s.harness)?;
        map.serialize_entry("corpus", &s.corpus)?;
        map.serialize_entry("selection", &s.selection)?;
        map.serialize_entry("lane", &s.lane)?;
        map.end()
    }
}

/// A git tree entry the content id covers. `blob` is the git blob id (`sha1("blob <len>\0" + bytes)`), or for
/// `Cargo.lock` the closure digest of the packages the compiler crates reach.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentEntry {
    pub mode: String,
    pub blob: String,
    pub path: String,
}

/// The content id: the sha256 of the sorted lines `<mode> <blob> <path>`, joined by newlines.
pub fn content_id(entries: &[ContentEntry]) -> Sha256 {
    let mut lines: Vec<String> = entries
        .iter()
        .map(|e| format!("{} {} {}", e.mode, e.blob, e.path))
        .collect();
    lines.sort_by(|a, b| slidock_canon::utf16::cmp(a, b));
    Sha256::of(lines.join("\n"))
}

/// What the harness hash covers besides the compiler: the bench's files and every diagnostic's severity.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct HarnessInputs {
    /// Path → sha256 of the file's bytes.
    pub files: BTreeMap<String, Sha256>,
    /// Diagnostic code → severity, from slidock-ir's registry.
    pub severities: BTreeMap<String, String>,
}

impl HarnessInputs {
    pub fn hash(&self) -> Result<Sha256, Error> {
        crate::sha256_json(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(c: char) -> Sha256 {
        Sha256::new(c.to_string().repeat(64)).unwrap()
    }

    pub(crate) fn stamp() -> Stamp {
        Stamp {
            compiler: CompilerStamp {
                content: sha('a'),
                commit: "0123abc".into(),
                dirty: false,
            },
            deps: DepsStamp {
                cargo_lock: sha('b'),
                rustc: "1.99.1".into(),
                wasm_pack: "0.13.1".into(),
                node: "24.x".into(),
            },
            host: HostStamp {
                platform: "darwin".into(),
                arch: "arm64".into(),
            },
            renderer: RendererStamp {
                recipe: sha('c'),
                platform_digest: "sha256:beef".into(),
                image_id: "sha256:cafe".into(),
                arch: "aarch64".into(),
                chromium: "148.0.7778.0".into(),
                cdp_client: "chromiumoxide@0.9.1".into(),
                poppler: "24.02.0".into(),
                raster_long_side_px: 1920,
                metric_long_side_px: 960,
                color_profile: Srgb::Value,
                render_policy: RenderPolicy::default(),
            },
            fonts: FontsStamp {
                font_set_id: sha('d'),
                lock_sha256: sha('e'),
                drift: None,
            },
            metrics: MetricsStamp {
                ssim: "ssim@1".into(),
                color: "color@1".into(),
                pixel: "pixel@1".into(),
                text: "text@2".into(),
                shadow: None,
                gradient: None,
                motion: None,
                fonts: None,
            },
            classifier: "classifier@2".into(),
            harness: sha('f'),
            corpus: BTreeMap::from([("demo-a:pptx".parse().unwrap(), sha('1'))]),
            selection: Selection {
                tiers: vec![Tier::Real, Tier::Smoke],
            },
            lane: Lane::Static,
            partial: false,
            determinism: None,
            parity: None,
            perf: None,
        }
    }

    #[test]
    fn run_key_is_the_stamp_the_run_started_with() {
        let base = stamp();
        let key = base.run_key().unwrap();
        let mut value = serde_json::to_value(&base).unwrap();
        value.as_object_mut().unwrap().remove("partial");
        assert_eq!(key, crate::sha256_json(&value).unwrap());

        let mut stamped = base.clone();
        stamped.partial = true;
        stamped.determinism = Some(sha('9'));
        stamped.parity = Some(ParityStamp {
            golden: crate::parity::GoldenIdentity {
                commit: "c".into(),
                manifest: sha('5'),
            },
            reference: sha('8'),
            report: sha('7'),
        });
        let cost = CompileCost {
            ms: 1,
            peak_bytes: 2,
        };
        stamped.perf = Some(PerfStamp {
            wasm: WasmSize {
                raw: 3,
                brotli11: 1,
            },
            wasm_sha256: sha('6'),
            player: PlayerSizes {
                iife: ScriptSize { raw: 2, gzip9: 1 },
                esm: ScriptSize { raw: 2, gzip9: 1 },
            },
            decks: BTreeMap::from([(
                "demo-a:pptx".parse().unwrap(),
                DeckCost {
                    native: cost,
                    wasm: cost,
                },
            )]),
            runs: 3,
            chromium: "148.0.7778.0".into(),
        });
        assert_eq!(stamped.run_key().unwrap(), key);
        let text = serde_json::to_string(&stamped).unwrap();
        assert!(text.contains(r#""decks":{"demo-a:pptx":{"native":{"ms":1,"peakBytes":2}"#));
        assert_eq!(serde_json::from_str::<Stamp>(&text).unwrap(), stamped);

        let mut other = base.clone();
        other.lane = Lane::Motion;
        assert_ne!(other.run_key().unwrap(), key);
        let mut other = base.clone();
        other.selection.tiers = vec![Tier::Probe];
        assert_ne!(other.run_key().unwrap(), key);
    }

    #[test]
    fn incomparable_names_every_field_beyond_the_compiler() {
        let base = stamp();
        let mut other = base.clone();
        other.compiler.content = sha('0');
        other.compiler.dirty = true;
        other.host.platform = "linux".into();
        let pairs = ["demo-a:pptx".parse().unwrap()];
        assert!(base.incomparable(&other, &pairs).is_empty());
        other.renderer.arch = "x86_64".into();
        other.metrics.motion = Some("motion@1".into());
        other.corpus.clear();
        assert_eq!(
            base.incomparable(&other, &pairs),
            ["renderer.arch", "metrics", "corpus[demo-a:pptx]"]
        );
    }

    #[test]
    fn content_id_sorts_the_entry_lines() {
        let entry = |path: &str| ContentEntry {
            mode: "100644".into(),
            blob: "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391".into(),
            path: path.into(),
        };
        let id = content_id(&[entry("b"), entry("a")]);
        assert_eq!(id, content_id(&[entry("a"), entry("b")]));
        let text = "100644 e69de29bb2d1d6434b8b29ae775ad8c2e48c5391 a\n100644 \
                    e69de29bb2d1d6434b8b29ae775ad8c2e48c5391 b";
        assert_eq!(id.as_str(), slidock_canon::sha256_hex(text));
    }

    #[test]
    fn harness_hash_is_the_canonical_json_of_files_and_severities() {
        let inputs = HarnessInputs {
            files: BTreeMap::from([("crates/slidock-bench/src/main.rs".into(), sha('a'))]),
            severities: BTreeMap::from([("IR0001".into(), "error".into())]),
        };
        let text = format!(
            r#"{{"files":{{"crates/slidock-bench/src/main.rs":"{}"}},"severities":{{"IR0001":"error"}}}}"#,
            "a".repeat(64)
        );
        assert_eq!(
            inputs.hash().unwrap().as_str(),
            slidock_canon::sha256_hex(text)
        );
    }

    #[test]
    fn refuses_unknown_and_wrong_literal_fields() {
        let text = serde_json::to_string(&stamp()).unwrap();
        assert_eq!(crate::parse::<Stamp>(&text).unwrap(), stamp());
        for (from, to) in [
            (r#""colorProfile":"srgb""#, r#""colorProfile":"p3""#),
            (r#""gif":"first-frame""#, r#""gif":"animated""#),
            (r#""lane":"static""#, r#""lane":"all""#),
            (r#""partial":false"#, r#""partial":false,"x":1"#),
            (r#""tiers":["real","smoke"]"#, r#""tiers":["gold"]"#),
        ] {
            assert!(text.contains(from), "{from}");
            assert!(
                crate::parse::<Stamp>(&text.replace(from, to)).is_err(),
                "{to}"
            );
        }
    }
}
