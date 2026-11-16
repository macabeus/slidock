//! Compiler parity with the goldens, levels B (the composed documents and their renders) and C (the IR): each row
//! of a run against the golden reference, by the composed document's bytes, then the render's sha256, with
//! render-vs-render scores where they drift; and the run's IR against the golden IR, container by container, with
//! the first differing JSON path.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;
use slidock_bench_schema::parity::{
    Allowlist, CompilerParity, CompilerRow, IrEntry, IrParity, IrStatus, ParityReference,
    ParityStatus, RenderVsRender, first_difference,
};
use slidock_bench_schema::results::Results;
use slidock_bench_schema::{PairKey, RowId};

use crate::error::{BenchError, Result};
use crate::golden::GoldenSet;
use crate::images::render_vs_render;
use crate::results::row_paths;

/// The top-level fields of an IR file that hold containers, compared one container at a time.
const CONTAINERS: [&str; 4] = ["slides", "layouts", "masters", "themes"];

/// The `presentation` container: every top-level field that holds no containers (`$schema`, `defaults`, `fonts`,
/// `media`, `size`, a frontend's `ext`, …), and the slides' order as their ids.
fn presentation_fields(ir: &Value) -> Value {
    let mut fields: serde_json::Map<String, Value> = ir
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| !CONTAINERS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let order: Vec<Value> = ir
        .get("slides")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|slide| slide.get("id").cloned().unwrap_or(Value::Null))
        .collect();
    fields.insert("slides".to_owned(), Value::Array(order));
    Value::Object(fields)
}

/// One pair's IR files, parsed.
pub struct PairIr {
    pub ours: Value,
    pub reference: Value,
}

impl PairIr {
    pub fn read(ours: &Path, reference: &Path) -> Result<Option<PairIr>> {
        let read = |path: &Path| -> Result<Option<Value>> {
            match std::fs::read(path) {
                Ok(bytes) => serde_json::from_slice(&bytes)
                    .map(Some)
                    .map_err(|e| BenchError::failed(format!("{}: {e}", path.display()))),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(BenchError::Io {
                    path: path.to_path_buf(),
                    source: error,
                }),
            }
        };
        Ok(match (read(ours)?, read(reference)?) {
            (Some(ours), Some(reference)) => Some(PairIr { ours, reference }),
            _ => None,
        })
    }

    fn slide<'a>(ir: &'a Value, id: &str) -> Option<(usize, &'a Value)> {
        ir.get("slides")?
            .as_array()?
            .iter()
            .enumerate()
            .find(|(_, slide)| slide.get("id").and_then(Value::as_str) == Some(id))
    }

    /// Every container of either side: `slide:<id>`, `layout:<id>`, `master:<id>`, `theme:<id>`, each with the JSON
    /// path it sits at in the reference.
    fn containers(&self) -> BTreeMap<String, (String, Option<&Value>, Option<&Value>)> {
        let mut out = BTreeMap::new();
        for ir in [&self.reference, &self.ours] {
            for slide in ir
                .get("slides")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(id) = slide.get("id").and_then(Value::as_str) {
                    let at = Self::slide(&self.reference, id).map_or_else(
                        || format!("$.slides[id={id}]"),
                        |(i, _)| format!("$.slides[{i}]"),
                    );
                    out.entry(format!("slide:{id}")).or_insert((
                        at,
                        Self::slide(&self.ours, id).map(|(_, s)| s),
                        Self::slide(&self.reference, id).map(|(_, s)| s),
                    ));
                }
            }
            for (kind, key) in [
                ("layout", "layouts"),
                ("master", "masters"),
                ("theme", "themes"),
            ] {
                for id in ir
                    .get(key)
                    .and_then(Value::as_object)
                    .into_iter()
                    .flatten()
                    .map(|(k, _)| k)
                {
                    out.entry(format!("{kind}:{id}")).or_insert((
                        format!("$.{key}.{id}"),
                        self.ours.get(key).and_then(|m| m.get(id)),
                        self.reference.get(key).and_then(|m| m.get(id)),
                    ));
                }
            }
        }
        out
    }

    /// Level C: every container equal, differing at a path, or on one side only.
    pub fn container_parity(&self, pair: &PairKey, into: &mut IrParity) {
        for (name, (at, ours, reference)) in self.containers() {
            let entry = match (ours, reference) {
                (Some(a), Some(b)) => match first_difference(a, b) {
                    None => IrEntry {
                        status: IrStatus::Equal,
                        path: None,
                    },
                    Some(path) => IrEntry {
                        status: IrStatus::Differs,
                        path: Some(rebase(&path, &at)),
                    },
                },
                _ => IrEntry {
                    status: IrStatus::Missing,
                    path: Some(at),
                },
            };
            into.containers.insert(format!("{pair}:{name}"), entry);
        }
        let entry = match first_difference(
            &presentation_fields(&self.ours),
            &presentation_fields(&self.reference),
        ) {
            None => IrEntry {
                status: IrStatus::Equal,
                path: None,
            },
            Some(path) => IrEntry {
                status: IrStatus::Differs,
                path: Some(path),
            },
        };
        into.containers
            .insert(format!("{pair}:presentation"), entry);
    }

    /// The first differing path of a row's IR slice: its slide, then its layout, master and theme, then the
    /// `presentation` container.
    pub fn row_difference(&self, slide: &str) -> Option<String> {
        let Some((index, reference)) = Self::slide(&self.reference, slide) else {
            return Some(format!("$.slides: no slide {slide} in the golden IR"));
        };
        let Some((_, ours)) = Self::slide(&self.ours, slide) else {
            return Some(format!("$.slides: no slide {slide} in the run's IR"));
        };
        let at = format!("$.slides[{index}]");
        if let Some(path) = first_difference(ours, reference) {
            return Some(rebase(&path, &at));
        }
        let layout = reference.get("layout").and_then(Value::as_str);
        let master = layout.and_then(|id| {
            self.reference
                .get("layouts")?
                .get(id)?
                .get("master")?
                .as_str()
        });
        let theme = master.and_then(|id| {
            self.reference
                .get("masters")?
                .get(id)?
                .get("theme")?
                .as_str()
        });
        for (key, id) in [("layouts", layout), ("masters", master), ("themes", theme)] {
            let Some(id) = id else { continue };
            let a = self.ours.get(key).and_then(|m| m.get(id));
            let b = self.reference.get(key).and_then(|m| m.get(id));
            let at = format!("$.{key}.{id}");
            match (a, b) {
                (Some(a), Some(b)) => {
                    if let Some(path) = first_difference(a, b) {
                        return Some(rebase(&path, &at));
                    }
                }
                (None, None) => {}
                _ => return Some(at),
            }
        }
        first_difference(
            &presentation_fields(&self.ours),
            &presentation_fields(&self.reference),
        )
    }
}

/// `$…` (a path from a container's root) under the container's own path.
fn rebase(path: &str, at: &str) -> String {
    format!("{at}{}", path.strip_prefix('$').unwrap_or(path))
}

/// Where B finds the renders and the IR.
pub struct ParityInputs<'a> {
    /// The run folder.
    pub run_dir: &'a Path,
    /// The run folder of `parity reference` (the golden output rendered by this bench), for the reference's renders.
    pub reference_renders: Option<&'a Path>,
    /// The golden set, for the golden IR of each pair.
    pub golden: &'a GoldenSet,
}

/// Levels B and C over a run.
pub fn compiler_parity(
    reference: &ParityReference,
    run: &Results,
    allowlist: &Allowlist,
    inputs: &ParityInputs<'_>,
    strict: bool,
) -> Result<(CompilerParity, IrParity)> {
    let mut irs: BTreeMap<PairKey, Option<PairIr>> = BTreeMap::new();
    let mut ir_parity = IrParity::default();
    let pairs: std::collections::BTreeSet<PairKey> =
        reference.rows.keys().map(RowId::pair).collect();
    for pair in pairs {
        let name = format!("{}.{}", pair.presentation(), pair.frontend());
        let ours = inputs.run_dir.join("ir").join(format!("{name}.ir.json"));
        let ir = match inputs
            .golden
            .static_pair(pair.presentation().as_str(), pair.frontend().as_str())
        {
            Some(dir) => PairIr::read(&ours, &dir.join("presentation-ir.json"))?,
            None => None,
        };
        if let Some(ir) = &ir {
            ir.container_parity(&pair, &mut ir_parity);
        }
        irs.insert(pair, ir);
    }
    let mut rows = BTreeMap::new();
    for (id, row) in &reference.rows {
        let ours = run.rows.get(id);
        let mut compared = CompilerRow::compare(id, row, ours, allowlist);
        if compared.status != ParityStatus::Identical && compared.status != ParityStatus::Missing {
            compared.first_ir_diff = irs
                .get(&id.pair())
                .and_then(Option::as_ref)
                .and_then(|ir| ir.row_difference(id.slide().as_str()));
        }
        let drifted = matches!(
            compared.status,
            ParityStatus::DriftBetter | ParityStatus::DriftSameClass | ParityStatus::Regressed
        );
        if drifted
            && let Some(reference_run) = inputs.reference_renders
            && ours.is_some_and(|r| r.render.is_some())
            && row.render.is_some()
        {
            let a = std::fs::read(row_paths(inputs.run_dir, id).render);
            let b = std::fs::read(row_paths(reference_run, id).render);
            if let (Ok(a), Ok(b)) = (a, b) {
                let (ssim, delta_e) = render_vs_render(&b, &a)?;
                compared.render_vs_render = Some(RenderVsRender { ssim, delta_e });
            }
        }
        rows.insert(id.clone(), compared);
    }
    Ok((CompilerParity::build(rows, &run.rows, strict), ir_parity))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ir(fill: &str) -> Value {
        json!({
            "$schema": "s", "defaults": {}, "fonts": {}, "media": {}, "phase": "resolved", "size": {"w": 1, "h": 1},
            "slides": [{"id": "s1", "layout": "l1", "elements": [{"fill": fill}]}, {"id": "s2", "layout": "l1", "elements": []}],
            "layouts": {"l1": {"master": "m1"}},
            "masters": {"m1": {"theme": "t1"}},
            "themes": {"t1": {"color": "red"}}
        })
    }

    #[test]
    fn the_row_slice_names_its_first_differing_path() {
        let pair = PairIr {
            ours: ir("blue"),
            reference: ir("red"),
        };
        assert_eq!(
            pair.row_difference("s1").as_deref(),
            Some("$.slides[0].elements[0].fill")
        );
        assert_eq!(pair.row_difference("s2"), None);
        let mut theme = ir("red");
        theme["themes"]["t1"]["color"] = json!("green");
        let pair = PairIr {
            ours: theme,
            reference: ir("red"),
        };
        assert_eq!(
            pair.row_difference("s2").as_deref(),
            Some("$.themes.t1.color")
        );
    }

    #[test]
    fn containers_report_equal_differing_and_missing() {
        let mut ours = ir("blue");
        ours["layouts"]["l2"] = json!({"master": "m1"});
        let pair = PairIr {
            ours,
            reference: ir("red"),
        };
        let mut parity = IrParity::default();
        let key: PairKey = "demo:pptx".parse().unwrap();
        pair.container_parity(&key, &mut parity);
        let status = |name: &str| {
            parity
                .containers
                .get(&format!("demo:pptx:{name}"))
                .map(|e| e.status)
        };
        assert_eq!(status("slide:s1"), Some(IrStatus::Differs));
        assert_eq!(status("slide:s2"), Some(IrStatus::Equal));
        assert_eq!(status("layout:l2"), Some(IrStatus::Missing));
        assert_eq!(status("theme:t1"), Some(IrStatus::Equal));
        assert_eq!(status("presentation"), Some(IrStatus::Equal));
        assert_eq!(
            parity
                .containers
                .get("demo:pptx:slide:s1")
                .and_then(|e| e.path.as_deref()),
            Some("$.slides[0].elements[0].fill")
        );
    }

    #[test]
    fn the_presentation_container_holds_every_other_top_level_field() {
        let key: PairKey = "demo:keynote".parse().unwrap();
        let path = |ours: Value| {
            let pair = PairIr {
                ours,
                reference: ir("red"),
            };
            let mut parity = IrParity::default();
            pair.container_parity(&key, &mut parity);
            let entry = parity.containers.get("demo:keynote:presentation").unwrap();
            (entry.status, entry.path.clone(), pair.row_difference("s2"))
        };
        let mut ext = ir("red");
        ext["ext"] = json!({"keynote": {"probe": "changed"}});
        let expected = Some("$.ext".to_owned());
        assert_eq!(path(ext), (IrStatus::Differs, expected.clone(), expected));
        let mut nested = ir("red");
        nested["size"]["w"] = json!(2);
        assert_eq!(path(nested).1.as_deref(), Some("$.size.w"));
        let mut reordered = ir("red");
        reordered["slides"].as_array_mut().unwrap().reverse();
        assert_eq!(path(reordered).1.as_deref(), Some("$.slides[0]"));
        assert_eq!(path(ir("red")).0, IrStatus::Equal);
    }
}

/// The bench over slidock's goldens (feature `golden`; skips, saying so, when the set the committed manifest pins is
/// absent): every pinned file checks out, and every pair's golden static-lane output reads as a compile, composes
/// every slide, and its IR is level-C equal to itself, container by container and row by row.
#[cfg(all(test, feature = "golden"))]
mod golden {
    use super::*;
    use crate::compile::read_pair_compile;
    use crate::env::BenchEnv;
    use crate::renderer::route::{composed_documents, read_compiled};

    #[test]
    fn every_golden_pair_reads_composes_and_equals_its_own_ir() {
        let env = BenchEnv::from_process().unwrap();
        let set = match GoldenSet::open(&env.golden_manifest, &env.golden_dir).unwrap() {
            Ok(set) => set,
            Err(reason) => {
                eprintln!("skipped: {reason}");
                return;
            }
        };
        let pairs = set.static_pairs();
        assert!(!pairs.is_empty(), "the goldens hold no static-lane output");
        let run = tempfile::tempdir().unwrap();
        let mut slides = 0;
        for pair in &pairs {
            let key: PairKey = pair.replacen('.', ":", 1).parse().unwrap();
            let (presentation, frontend) = (key.presentation().as_str(), key.frontend().as_str());
            let dir = set.static_pair(presentation, frontend).unwrap();
            crate::commands::parity::link_golden_pair(&dir, run.path(), presentation, frontend)
                .unwrap();
            let compile = read_pair_compile(run.path(), presentation, frontend);
            assert!(compile.problem.is_none(), "{pair}: {:?}", compile.problem);
            assert!(compile.report.is_some(), "{pair}: no report");
            let documents = composed_documents(&read_compiled(&compile.dir).unwrap()).unwrap();
            assert_eq!(documents.len(), compile.slides.len(), "{pair}");
            slides += documents.len();
            let ir_path = dir.join("presentation-ir.json");
            let ir = PairIr::read(&ir_path, &ir_path).unwrap().unwrap();
            let mut parity = IrParity::default();
            ir.container_parity(&key, &mut parity);
            assert!(
                parity
                    .containers
                    .values()
                    .all(|entry| entry.status == IrStatus::Equal),
                "{pair}"
            );
            for slide in compile.slides.keys() {
                assert_eq!(ir.row_difference(slide), None, "{pair} {slide}");
            }
        }
        eprintln!("{} golden pairs, {slides} slides composed", pairs.len());
    }
}
