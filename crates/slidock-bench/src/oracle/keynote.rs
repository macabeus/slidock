//! The Keynote oracle (`scripts/export-keynote.applescript`): Keynote itself exports each deck to PDF, once per (source
//! sha256, exporter), in the attended `oracle` command only. `run` reads only the export already on disk: in an
//! unattended loop the first Apple Event would block on the macOS Automation prompt, and Keynote can block on font,
//! version or iCloud sheets.
//!
//! The export opens a copy of the deck (never the corpus file), with `all stages:false` (every slide at its final
//! build state), and closes it unsaved. `oracle --stages` exports the same way with `all stages:true` (one page per
//! build stage, the motion lane's stage oracle) into the stages folder.

use std::path::{Path, PathBuf};
use std::time::Duration;

use slidock_bench_schema::corpus::KeynoteExporter;

use crate::cache::temp_sibling;
use crate::error::{BenchError, Result};
use crate::exec::{ExecOptions, args, exec, exec_text};

/// Keynote 14.5 by default; Keynote Creator Studio only for a file 14.5 refuses.
pub const KEYNOTE_BUNDLE_IDS: [&str; 2] = ["com.apple.iWork.Keynote", "com.apple.Keynote"];
pub const EXPORT_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Set once this process asked Keynote for an export: only then may it quit a Keynote it found not running.
pub static EXPORTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

pub fn exporter_id(exporter: &KeynoteExporter) -> String {
    format!(
        "{}-{}-{}",
        exporter.bundle_id, exporter.version, exporter.build
    )
}

/// The export of a source by an exporter.
pub fn export_path(keynote_dir: &Path, source_sha: &str, exporter: &KeynoteExporter) -> PathBuf {
    keynote_dir
        .join(source_sha)
        .join(format!("{}.pdf", exporter_id(exporter)))
}

/// Which PDF Keynote writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportKind {
    /// Every slide at its final build state (`all stages:false`): the static oracle.
    Final,
    /// One page per build stage (`all stages:true`): the motion lane's stage oracle.
    AllStages,
}

/// The all-stages export of a source by an exporter, in the stages folder (`SLIDOCK_STAGES_ORACLES`).
pub fn stages_export_path(
    stages_dir: &Path,
    source_sha: &str,
    exporter: &KeynoteExporter,
) -> PathBuf {
    stages_dir
        .join(source_sha)
        .join(format!("{}-all-stages.pdf", exporter_id(exporter)))
}

/// The script's arguments: the app, the copy it opens, where it writes, and `all-stages` for a stages export.
pub fn export_arguments(
    script: &Path,
    bundle_id: &str,
    copy: &Path,
    out: &Path,
    kind: ExportKind,
) -> Vec<std::ffi::OsString> {
    let mut arguments: Vec<std::ffi::OsString> =
        vec![script.into(), bundle_id.into(), copy.into(), out.into()];
    if kind == ExportKind::AllStages {
        arguments.push("all-stages".into());
    }
    arguments
}

/// Asks the app for its version over Apple Events (this launches it). A failure here is almost always the
/// Automation permission, which one interactive run grants.
pub async fn probe_keynote(bundle_id: &str) -> Result<()> {
    let script = format!("tell application id \"{bundle_id}\" to get version");
    let result = exec(
        "osascript",
        &args(["-e", &script]),
        &ExecOptions::timeout(PROBE_TIMEOUT),
    )
    .await?;
    if !result.ok() {
        return Err(BenchError::refused(format!(
            "Keynote ({bundle_id}) did not answer an Apple Event. Run `slidock-bench oracle` once in an \
             interactive Terminal and allow it to control Keynote (System Settings → Privacy & Security → \
             Automation). {}",
            result.stderr.trim()
        )));
    }
    Ok(())
}

/// Where an app is installed, from LaunchServices. AppleScript's `path to application id` would launch it.
pub async fn app_path(bundle_id: &str) -> Result<PathBuf> {
    let script = format!(
        "ObjC.import('AppKit'); \
         const url = $.NSWorkspace.sharedWorkspace.URLForApplicationWithBundleIdentifier('{bundle_id}'); \
         url.isNil() ? '' : url.path.js"
    );
    let path = exec_text(
        "osascript",
        &args(["-l", "JavaScript", "-e", &script]),
        &ExecOptions::timeout(PROBE_TIMEOUT),
    )
    .await?;
    let path = path.trim();
    if path.is_empty() {
        return Err(BenchError::refused(format!("{bundle_id} is not installed")));
    }
    Ok(PathBuf::from(path))
}

/// The installed app's version and build, from its Info.plist (without launching it).
pub async fn installed_exporter(bundle_id: &str) -> Result<KeynoteExporter> {
    let plist = app_path(bundle_id).await?.join("Contents/Info.plist");
    let plist = plist.to_string_lossy().into_owned();
    let key = |name: &'static str| {
        let plist = plist.clone();
        async move {
            exec_text(
                "plutil",
                &args(["-extract", name, "raw", &plist]),
                &ExecOptions::timeout(PROBE_TIMEOUT),
            )
            .await
            .map(|text| text.trim().to_owned())
        }
    };
    Ok(KeynoteExporter {
        bundle_id: bundle_id.to_owned(),
        version: key("CFBundleShortVersionString").await?,
        build: key("CFBundleVersion").await?,
    })
}

/// The processes of a process list (`ps -axo pid=,comm=`: a pid and an executable path per line) that run the app
/// at `app`.
fn app_pids(processes: &str, app: &Path) -> Vec<u32> {
    let executables = app.join("Contents/MacOS");
    processes
        .lines()
        .filter_map(|line| {
            let (pid, command) = line.trim().split_once(' ')?;
            (Path::new(command.trim()).parent() == Some(executables.as_path()))
                .then(|| pid.parse().ok())
                .flatten()
        })
        .collect()
}

/// The processes running an app. It reads the process list: LaunchServices' answer (`application id … is running`)
/// is `false` for every app inside a sandboxed session.
async fn running_pids(bundle_id: &str) -> Result<Vec<u32>> {
    let app = app_path(bundle_id).await?;
    let processes = exec_text(
        "ps",
        &args(["-axo", "pid=,comm="]),
        &ExecOptions::timeout(PROBE_TIMEOUT),
    )
    .await?;
    Ok(app_pids(&processes, &app))
}

/// Whether an app is running, so the bench quits only what it launched.
pub async fn is_running(bundle_id: &str) -> Result<bool> {
    Ok(!running_pids(bundle_id).await?.is_empty())
}

/// How long an app has to quit before the bench terminates it.
const QUIT_GRACE: Duration = Duration::from_secs(30);

/// Quits an app the bench launched, unsaved. An app launched in the background can ignore the Apple Event, so one
/// still running after [`QUIT_GRACE`] is sent SIGTERM.
pub async fn quit_keynote(bundle_id: &str) -> Result<()> {
    let script = format!("tell application id \"{bundle_id}\" to quit saving no");
    exec(
        "osascript",
        &args(["-e", &script]),
        &ExecOptions::timeout(Duration::from_secs(60)),
    )
    .await?;
    let mut waited = Duration::ZERO;
    while waited < QUIT_GRACE {
        if running_pids(bundle_id).await?.is_empty() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        waited += Duration::from_secs(1);
    }
    for pid in running_pids(bundle_id).await? {
        exec(
            "kill",
            &args(["-TERM", &pid.to_string()]),
            &ExecOptions::timeout(PROBE_TIMEOUT),
        )
        .await?;
    }
    Ok(())
}

/// Exports `source_file` (a corpus file, copied first so Keynote opens only the copy) to `out` (its folder holds
/// the copy while Keynote works). The copy is removed afterwards; on timeout the process is killed and Keynote quit
/// unsaved. An export already at `out` is kept: Keynote is not opened.
pub async fn export_keynote(
    script: &Path,
    out: &Path,
    source_file: &Path,
    exporter: &KeynoteExporter,
    kind: ExportKind,
) -> Result<std::result::Result<PathBuf, String>> {
    let out = out.to_path_buf();
    if out.exists() {
        return Ok(Ok(out));
    }
    let dir = out
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| BenchError::failed(format!("{} has no folder", out.display())))?;
    std::fs::create_dir_all(&dir).map_err(BenchError::io(&dir))?;
    let copy = dir.join(format!("source.{}.key", std::process::id()));
    let temp = PathBuf::from(format!("{}.pdf", temp_sibling(&out).display()));
    copy_deck(source_file, &copy)?;
    EXPORTED.store(true, std::sync::atomic::Ordering::SeqCst);
    let outcome = async {
        let result = exec(
            "osascript",
            &export_arguments(script, &exporter.bundle_id, &copy, &temp, kind),
            &ExecOptions::timeout(EXPORT_TIMEOUT),
        )
        .await?;
        if result.timed_out {
            quit_keynote(&exporter.bundle_id).await?;
            return Ok(Err(format!(
                "the export took longer than {} min",
                EXPORT_TIMEOUT.as_secs() / 60
            )));
        }
        let written = std::fs::metadata(&temp).is_ok_and(|m| m.len() > 0);
        if !result.ok() || !written {
            let reason = result.stderr.trim();
            return Ok(Err(if reason.is_empty() {
                "no PDF was written".to_owned()
            } else {
                reason.to_owned()
            }));
        }
        std::fs::rename(&temp, &out).map_err(BenchError::io(&out))?;
        Ok(Ok(out.clone()))
    }
    .await;
    let _ = crate::cache::remove(&copy);
    let _ = crate::cache::remove(&temp);
    outcome
}

/// Copies a deck (a package folder or a single file) without following it anywhere else.
fn copy_deck(from: &Path, to: &Path) -> Result<()> {
    let meta = std::fs::metadata(from).map_err(BenchError::io(from))?;
    if meta.is_dir() {
        std::fs::create_dir_all(to).map_err(BenchError::io(to))?;
        for entry in std::fs::read_dir(from).map_err(BenchError::io(from))? {
            let entry = entry.map_err(BenchError::io(from))?;
            copy_deck(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to)
            .map(|_| ())
            .map_err(BenchError::io(from))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_are_named_by_source_and_exporter() {
        let exporter = KeynoteExporter {
            bundle_id: "com.apple.iWork.Keynote".into(),
            version: "14.5".into(),
            build: "7045.0.17".into(),
        };
        assert_eq!(
            exporter_id(&exporter),
            "com.apple.iWork.Keynote-14.5-7045.0.17"
        );
        assert_eq!(
            export_path(Path::new("/k"), "abc", &exporter),
            Path::new("/k/abc/com.apple.iWork.Keynote-14.5-7045.0.17.pdf")
        );
    }

    #[test]
    fn a_stages_export_asks_for_every_stage_and_lands_beside_the_others() {
        let exporter = KeynoteExporter {
            bundle_id: "com.apple.iWork.Keynote".into(),
            version: "14.5".into(),
            build: "7045.0.17".into(),
        };
        assert_eq!(
            stages_export_path(Path::new("/s"), "abc", &exporter),
            Path::new("/s/abc/com.apple.iWork.Keynote-14.5-7045.0.17-all-stages.pdf")
        );
        let script = Path::new("export-keynote.applescript");
        let (copy, out) = (
            Path::new("/s/abc/source.1.key"),
            Path::new("/s/abc/out.pdf"),
        );
        let final_args =
            export_arguments(script, &exporter.bundle_id, copy, out, ExportKind::Final);
        assert_eq!(final_args.len(), 4);
        let stages = export_arguments(
            script,
            &exporter.bundle_id,
            copy,
            out,
            ExportKind::AllStages,
        );
        assert_eq!(
            stages
                .last()
                .map(|a| a.to_string_lossy().into_owned())
                .as_deref(),
            Some("all-stages")
        );
        assert_eq!(stages[..4], final_args[..]);
    }

    #[test]
    fn the_script_turns_all_stages_on_only_when_asked() {
        let script = include_str!("../../../../bench/scripts/export-keynote.applescript");
        assert!(script.contains("all stages:allStages"));
        assert!(script.contains("item 4 of argv is \"all-stages\""));
    }

    #[test]
    fn an_app_runs_when_its_own_executable_does() {
        let processes = "    1 /sbin/launchd\n27253 /Applications/Keynote Creator Studio.app/Contents/MacOS/Keynote\n";
        let keynote = Path::new("/Applications/Keynote.app");
        assert!(app_pids(processes, keynote).is_empty());
        let studio = Path::new("/Applications/Keynote Creator Studio.app");
        assert_eq!(app_pids(processes, studio), [27253]);
        let both = format!("{processes}87733 /Applications/Keynote.app/Contents/MacOS/Keynote\n");
        assert_eq!(app_pids(&both, keynote), [87733]);
        let helper = "9 /Applications/Keynote.app/Contents/MacOS/Helpers/Keynote Helper\n";
        assert!(app_pids(helper, keynote).is_empty());
    }

    #[test]
    fn copies_package_decks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("deck.key/Data")).unwrap();
        std::fs::write(dir.path().join("deck.key/Data/a"), "x").unwrap();
        copy_deck(&dir.path().join("deck.key"), &dir.path().join("copy.key")).unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("copy.key/Data/a")).unwrap(),
            b"x"
        );
    }
}
