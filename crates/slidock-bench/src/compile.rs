//! The compiler under test always runs as a child process: the native `slidock` CLI, one process per (presentation,
//! frontend), so a crash, a hang or a runaway allocation costs only that pair's rows. Wall-clock limits are the
//! bench's, and memory is watched by sampling the child's resident size.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use slidock_format::manifest::ManifestSlide;

use crate::cache::{read_json, remove, write_json};
use crate::error::{BenchError, Result};
use crate::exec::{ExecOptions, args, exec, exec_text};
use crate::renderer::route::manifest_of;

pub const COMPILE_TIMEOUT: Duration = Duration::from_secs(600);
pub const COMPILE_ONLY_TIMEOUT: Duration = Duration::from_secs(120);
/// The child is killed above this resident size.
pub const RSS_LIMIT_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// How a compile ended. Exit codes of `slidock compile`: 0 ok, 1 errors or failed containers, 2 refused input;
/// anything else crashed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompileStatus {
    Ok,
    Errors,
    Refused,
    Crash,
    Timeout,
    Memory,
}

/// The working copy's compiler, built once per process with cargo, or the binary `SLIDOCK_COMPILER` names.
pub async fn compiler_binary(
    repo: &Path,
    target_dir: &Path,
    explicit: Option<&Path>,
) -> Result<PathBuf> {
    if let Some(binary) = explicit {
        return Ok(binary.to_path_buf());
    }
    let options = ExecOptions {
        cwd: Some(repo.to_path_buf()),
        env: BTreeMap::from([(
            "CARGO_TARGET_DIR".to_owned(),
            target_dir.to_string_lossy().into_owned(),
        )]),
        ..ExecOptions::timeout(Duration::from_secs(60 * 60))
    };
    exec_text(
        "cargo",
        &args([
            "build",
            "--release",
            "--locked",
            "-p",
            "slidock-cli",
            "--bin",
            "slidock",
        ]),
        &options,
    )
    .await?;
    Ok(target_dir.join("release/slidock"))
}

#[derive(Clone, Debug)]
pub struct CompileOutcome {
    pub status: CompileStatus,
    pub stderr: String,
}

/// Runs `slidock <args>` with the bench's limits.
pub async fn run_compiler(
    binary: &Path,
    arguments: &[String],
    timeout: Duration,
    quiet: bool,
) -> Result<CompileOutcome> {
    run_compiler_with(binary, arguments, timeout, quiet, BTreeMap::new()).await
}

/// [`run_compiler`] with variables set on top of this process's environment.
pub async fn run_compiler_with(
    binary: &Path,
    arguments: &[String],
    timeout: Duration,
    quiet: bool,
    env: BTreeMap<String, String>,
) -> Result<CompileOutcome> {
    let options = ExecOptions {
        timeout: Some(timeout),
        quiet,
        rss_limit_bytes: Some(RSS_LIMIT_BYTES),
        env,
        ..ExecOptions::default()
    };
    let result = exec(binary.as_os_str(), &args(arguments), &options).await?;
    let status = if result.out_of_memory {
        CompileStatus::Memory
    } else if result.timed_out {
        CompileStatus::Timeout
    } else {
        match result.code {
            Some(0) => CompileStatus::Ok,
            Some(1) => CompileStatus::Errors,
            Some(2) => CompileStatus::Refused,
            _ => CompileStatus::Crash,
        }
    };
    Ok(CompileOutcome {
        status,
        stderr: result.stderr,
    })
}

/// A diagnostic of the compile report, the fields the bench reads.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ReportDiagnostic {
    pub code: String,
    pub severity: String,
    #[serde(default)]
    pub container: Option<String>,
    #[serde(default)]
    pub feature: Option<String>,
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub message: Option<String>,
    /// The IR element the diagnostic names.
    #[serde(default)]
    pub node: Option<String>,
}

/// The compile report: diagnostics, features per container, and which containers each slide shows.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct CompileReport {
    pub diagnostics: Vec<ReportDiagnostic>,
    #[serde(default)]
    pub features: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub shows: Option<BTreeMap<String, Vec<String>>>,
}

/// Where a run keeps one pair's compiled output and records.
pub struct PairPaths {
    pub compiled: PathBuf,
    pub report: PathBuf,
    /// How the compile ended, which re-scoring the saved output reads back.
    pub status: PathBuf,
    pub ir: PathBuf,
}

pub fn pair_paths(run_dir: &Path, presentation: &str, frontend: &str) -> PairPaths {
    let name = format!("{presentation}.{frontend}");
    PairPaths {
        compiled: run_dir.join("compiled").join(&name),
        report: run_dir.join("reports").join(format!("{name}.json")),
        status: run_dir.join("reports").join(format!("{name}.status.json")),
        ir: run_dir.join("ir").join(format!("{name}.ir.json")),
    }
}

/// Where an overlay (`run --only`) keeps its partial compiles, apart from the whole pairs.
pub fn overlay_dir(run_dir: &Path) -> PathBuf {
    run_dir.join("only")
}

/// A pair's compile as the rows read it.
#[derive(Clone, Debug)]
pub struct PairCompile {
    pub status: CompileStatus,
    pub dir: PathBuf,
    pub report: Option<CompileReport>,
    /// Why the compile counts as crashed although the child exited: an unreadable report or status.
    pub problem: Option<String>,
    pub slides: BTreeMap<String, ManifestSlide>,
    /// The slide size in points.
    pub size: Option<(f64, f64)>,
}

#[derive(Serialize, Deserialize)]
struct StatusFile {
    status: CompileStatus,
}

/// A compile's report and manifest read into `result`. A report that fails to parse makes the compile a crash.
pub fn read_compile_output(mut result: PairCompile, report_path: &Path) -> PairCompile {
    if report_path.exists() {
        let parsed = std::fs::read_to_string(report_path)
            .map_err(|error| error.to_string())
            .and_then(|text| {
                serde_json::from_str::<CompileReport>(&text).map_err(|error| error.to_string())
            });
        match parsed {
            Ok(report) => result.report = Some(report),
            Err(error) => {
                result.status = CompileStatus::Crash;
                result.problem = Some(format!("report-invalid: {error}"));
                return result;
            }
        }
    }
    if result.dir.join("presentation.json").exists()
        && let Ok(files) = crate::renderer::route::read_compiled(&result.dir)
        && let Ok(manifest) = manifest_of(&files)
    {
        result.size = Some((manifest.size.w, manifest.size.h));
        result.slides = manifest
            .slides
            .into_iter()
            .map(|slide| (slide.id.clone(), slide))
            .collect();
    }
    result
}

/// What a pair's compile needs.
pub struct CompileSettings {
    pub binary: PathBuf,
    pub fonts_dir: PathBuf,
    /// Discards the child's output.
    pub quiet: bool,
}

/// Compiles one source into the run folder; `only` compiles those slides into the overlay folder, so the pair's
/// whole compile stays as it was. A whole compile drops the pair's overlay, which it supersedes.
pub async fn compile_pair(
    settings: &CompileSettings,
    run_dir: &Path,
    presentation: &str,
    frontend: &str,
    file: &Path,
    only: Option<&[String]>,
) -> Result<PairCompile> {
    let base = if only.is_some() {
        overlay_dir(run_dir)
    } else {
        run_dir.to_path_buf()
    };
    let paths = pair_paths(&base, presentation, frontend);
    let mut stale = vec![&paths.compiled, &paths.report, &paths.status, &paths.ir];
    let overlay = pair_paths(&overlay_dir(run_dir), presentation, frontend);
    if only.is_none() {
        stale.extend([
            &overlay.compiled,
            &overlay.report,
            &overlay.status,
            &overlay.ir,
        ]);
    }
    for path in stale {
        remove(path)?;
    }
    for path in [&paths.report, &paths.ir, &paths.compiled] {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(BenchError::io(parent))?;
        }
    }
    let mut arguments: Vec<String> = vec![
        "compile".into(),
        file.to_string_lossy().into_owned(),
        "--frontend".into(),
        frontend.into(),
        "--out".into(),
        paths.compiled.to_string_lossy().into_owned(),
        "--emit".into(),
        "documents,report,ir".into(),
        "--fonts".into(),
        settings.fonts_dir.to_string_lossy().into_owned(),
    ];
    if let Some(only) = only {
        arguments.push("--only".into());
        arguments.push(only.join(","));
    }
    let timeout = if only.is_some() {
        COMPILE_ONLY_TIMEOUT
    } else {
        COMPILE_TIMEOUT
    };
    let outcome = run_compiler(&settings.binary, &arguments, timeout, settings.quiet).await?;
    // The CLI writes the report and the IR beside the documents; the run keeps them apart.
    for (from, to) in [
        ("report.json", &paths.report),
        ("presentation-ir.json", &paths.ir),
    ] {
        let written = paths.compiled.join(from);
        if written.exists() {
            std::fs::rename(&written, to).map_err(BenchError::io(&written))?;
        }
    }
    write_json(
        &paths.status,
        &StatusFile {
            status: outcome.status,
        },
    )?;
    Ok(read_compile_output(
        PairCompile {
            status: outcome.status,
            dir: paths.compiled.clone(),
            report: None,
            problem: None,
            slides: BTreeMap::new(),
            size: None,
        },
        &paths.report,
    ))
}

/// A compile already in a run folder, read back with the status it ended with.
pub fn read_pair_compile(run_dir: &Path, presentation: &str, frontend: &str) -> PairCompile {
    let paths = pair_paths(run_dir, presentation, frontend);
    let crashed = PairCompile {
        status: CompileStatus::Crash,
        dir: paths.compiled.clone(),
        report: None,
        problem: None,
        slides: BTreeMap::new(),
        size: None,
    };
    match read_json::<StatusFile>(&paths.status) {
        Ok(Some(file)) => read_compile_output(
            PairCompile {
                status: file.status,
                ..crashed
            },
            &paths.report,
        ),
        _ => PairCompile {
            problem: Some("the run folder records no compile status".to_owned()),
            ..crashed
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn maps_exit_codes_to_statuses() {
        let script = |code: &str| vec!["-c".to_owned(), format!("exit {code}")];
        let sh = Path::new("/bin/sh");
        for (code, status) in [
            ("0", CompileStatus::Ok),
            ("1", CompileStatus::Errors),
            ("2", CompileStatus::Refused),
            ("3", CompileStatus::Crash),
        ] {
            let outcome = run_compiler(sh, &script(code), Duration::from_secs(10), true)
                .await
                .unwrap();
            assert_eq!(outcome.status, status, "{code}");
        }
        let slow = vec!["-c".to_owned(), "sleep 5".to_owned()];
        let outcome = run_compiler(sh, &slow, Duration::from_millis(200), true)
            .await
            .unwrap();
        assert_eq!(outcome.status, CompileStatus::Timeout);
    }

    #[test]
    fn an_unreadable_report_is_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        let report = dir.path().join("r.json");
        std::fs::write(&report, "{\"diagnostics\": 3}").unwrap();
        let read = read_compile_output(
            PairCompile {
                status: CompileStatus::Ok,
                dir: dir.path().join("none"),
                report: None,
                problem: None,
                slides: BTreeMap::new(),
                size: None,
            },
            &report,
        );
        assert_eq!(read.status, CompileStatus::Crash);
        assert!(read.problem.unwrap().starts_with("report-invalid"));
    }

    #[test]
    fn a_folder_without_a_status_reads_as_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        let read = read_pair_compile(dir.path(), "p", "pptx");
        assert_eq!(read.status, CompileStatus::Crash);
        assert!(read.problem.is_some());
    }
}
