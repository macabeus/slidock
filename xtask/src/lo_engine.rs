//! `cargo xtask lo-engine`: the motion probes' iris and split clips as LibreOffice's own presentation engine plays
//! them, against slidock's reading. Each probe deck (`probes/out/motion-probe.odp`, `.pptx`) is exported to SVG by
//! the pinned LibreOffice (the bench's `slidock-probes-lo` image, built from `probes/fonts/Dockerfile`, no network):
//! the export embeds LibreOffice's engine (`presentation_engine.js`, its `ClippingFunctor` and `transitionfactorytab`
//! port) and the deck's animations as LibreOffice read them. The deck is compiled with slidock
//! (`--emit ir --native-motion`), the player's clip generators are bundled with the repository's esbuild, and
//! `xtask/lo-engine/check.mjs` samples both sides' frames in Chromium (requests other than `file:` aborted) and
//! compares what each plays: the motion (whether the shape's centre shows, whether the shown part grows or shrinks)
//! and the shape (the iris outline, the split's orientation). Every row must agree. A step LibreOffice reads as its
//! random choice (`ooo-entrance-random`, `ooo-exit-random`) is skipped and reported: its engine draws another preset
//! for it at each run (`xtask/lo-engine/rows.mjs`). CI does not run it: it needs Docker and Chromium, so the gate
//! runs it.

use std::path::{Path, PathBuf};
use std::process::Command;

const USAGE: &str = "usage: cargo xtask lo-engine [--slidock <binary>] [--out <dir>]

  Exports probes/out/motion-probe.{odp,pptx} to SVG with the pinned LibreOffice (Docker, no network) and checks
  every iris and split clip slidock plays against LibreOffice's engine: its motion, iris shape and split orientation.
  Random presets are skipped and reported: LibreOffice draws another effect for them at each run.
  --slidock   the slidock CLI to compile the probes with (default: built with cargo, debug profile)
  --out       the work directory (default: <target>/lo-engine); its report.json holds every row's frames";

/// The LibreOffice image, the bench's (`crates/slidock-bench/src/lo_roundtrip.rs` `LO_IMAGE`): LibreOffice 25.2.3.2.
pub const LO_IMAGE: &str = "slidock-probes-lo:trixie-20261030";
/// Its recipe, relative to the repository.
const LO_RECIPE: &str = "probes/fonts";
/// The probe decks, by name (their extension).
const DECKS: [&str; 2] = ["odp", "pptx"];

struct Args {
    slidock: Option<PathBuf>,
    out: Option<PathBuf>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut parsed = Args {
        slidock: None,
        out: None,
    };
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let value = |rest: &mut std::slice::Iter<'_, String>| {
            rest.next()
                .map(PathBuf::from)
                .ok_or_else(|| USAGE.to_owned())
        };
        match arg.as_str() {
            "--slidock" => parsed.slidock = Some(value(&mut rest)?),
            "--out" => parsed.out = Some(value(&mut rest)?),
            _ => return Err(USAGE.to_owned()),
        }
    }
    Ok(parsed)
}

pub fn run(args: &[String]) -> Result<(), String> {
    let args = parse(args)?;
    let root = repo_root();
    let target = match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => root.join("target"),
    };
    let out = args.out.unwrap_or_else(|| target.join("lo-engine"));
    // Docker mounts only an absolute path.
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let out = out
        .canonicalize()
        .map_err(|e| format!("{}: {e}", out.display()))?;
    let slidock = match args.slidock {
        Some(binary) => binary,
        None => build_slidock(&target)?,
    };
    ensure_image(&root)?;
    let mut decks = Vec::new();
    for deck in DECKS {
        let source = root.join(format!("probes/out/motion-probe.{deck}"));
        let work = out.join(deck);
        let input = work.join("in");
        std::fs::create_dir_all(&input).map_err(|e| format!("{}: {e}", input.display()))?;
        std::fs::copy(&source, input.join(format!("motion-probe.{deck}")))
            .map_err(|e| format!("{}: {e}", source.display()))?;
        export_svg(&work, deck)?;
        let ir = work.join("ir");
        run_tool(
            Command::new(&slidock)
                .arg("compile")
                .arg(&source)
                .args(["--emit", "ir", "--native-motion", "-o"])
                .arg(&ir),
            "slidock compile",
        )?;
        decks.push((
            deck,
            work.join("svg/motion-probe.svg"),
            ir.join("presentation-ir.json"),
        ));
    }
    let clip = out.join("clip.mjs");
    run_tool(
        Command::new(root.join("packages/player/node_modules/.bin/esbuild"))
            .arg(root.join("packages/player/src/effects/clip.ts"))
            .args(["--bundle", "--format=esm", "--log-level=warning"])
            .arg(format!("--outfile={}", clip.display())),
        "esbuild (pnpm install first)",
    )?;
    let mut check = Command::new("node");
    check
        .arg(root.join("xtask/lo-engine/check.mjs"))
        .arg(out.join("report.json"))
        .arg(&clip);
    for (name, svg, ir) in &decks {
        check.arg(name).arg(svg).arg(ir);
    }
    let status = check
        .status()
        .map_err(|e| format!("node xtask/lo-engine/check.mjs: {e}"))?;
    println!("report: {}", out.join("report.json").display());
    if status.success() {
        Ok(())
    } else {
        Err("slidock plays an iris or split clip otherwise than LibreOffice's engine".to_owned())
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn run_tool(command: &mut Command, what: &str) -> Result<(), String> {
    let output = command.output().map_err(|e| format!("{what}: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{what}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

fn build_slidock(target: &Path) -> Result<PathBuf, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    run_tool(
        Command::new(cargo).args(["build", "-q", "-p", "slidock-cli"]),
        "cargo build -p slidock-cli",
    )?;
    Ok(target.join("debug/slidock"))
}

/// The LibreOffice image, built from its recipe when it is missing.
fn ensure_image(root: &Path) -> Result<(), String> {
    let found = Command::new("docker")
        .args(["image", "inspect", "--format", "{{.Id}}", LO_IMAGE])
        .output()
        .map_err(|e| format!("docker: {e}"))?;
    if found.status.success() {
        return Ok(());
    }
    run_tool(
        Command::new("docker")
            .args(["build", "-t", LO_IMAGE])
            .arg(root.join(LO_RECIPE)),
        &format!("docker build {LO_IMAGE}"),
    )
}

/// `<work>/in/motion-probe.<deck>` exported to `<work>/svg/motion-probe.svg`, with no network.
fn export_svg(work: &Path, deck: &str) -> Result<(), String> {
    let svg = work.join("svg/motion-probe.svg");
    if svg.exists() {
        std::fs::remove_file(&svg).map_err(|e| format!("{}: {e}", svg.display()))?;
    }
    let mount = format!("{}:/work", work.display());
    let script = format!(
        "soffice --version > /work/version.txt 2>/dev/null; mkdir -p /work/svg && soffice --headless --norestore \
         --convert-to svg --outdir /work/svg /work/in/motion-probe.{deck}"
    );
    run_tool(
        Command::new("docker").args([
            "run",
            "--rm",
            "--network",
            "none",
            "-v",
            &mount,
            "-e",
            "HOME=/tmp/home",
            LO_IMAGE,
            "sh",
            "-c",
            &script,
        ]),
        "soffice --convert-to svg",
    )?;
    if !svg.is_file() {
        return Err(format!("soffice wrote no {}", svg.display()));
    }
    let version = std::fs::read_to_string(work.join("version.txt")).unwrap_or_default();
    println!("{deck}: {}", version.trim());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_image_is_the_benchs() {
        let bench =
            std::fs::read_to_string(repo_root().join("crates/slidock-bench/src/lo_roundtrip.rs"))
                .unwrap();
        assert!(
            bench.contains(&format!("pub const LO_IMAGE: &str = \"{LO_IMAGE}\";")),
            "the bench's LO_IMAGE moved"
        );
    }

    /// The step rules of `check.mjs` (`rows.mjs`): random presets are skipped, a random step on one side only is a
    /// problem.
    #[test]
    fn the_step_rules_pass_their_node_tests() {
        let output = match Command::new("node")
            .args(["--test", "--test-reporter=tap"])
            .arg(repo_root().join("xtask/lo-engine/rows.test.mjs"))
            .output()
        {
            Ok(output) => output,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("skipped: `node` is not installed, so rows.test.mjs does not run");
                return;
            }
            Err(error) => panic!("node: {error}"),
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{stdout}");
        assert!(stdout.contains("# pass 4"), "{stdout}");
        assert!(stdout.contains("# fail 0"), "{stdout}");
    }

    #[test]
    fn arguments() {
        let args = |list: &[&str]| parse(&list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
        let parsed = args(&["--slidock", "/bin/slidock", "--out", "o"]).unwrap();
        assert_eq!(parsed.slidock, Some(PathBuf::from("/bin/slidock")));
        assert_eq!(parsed.out, Some(PathBuf::from("o")));
        assert!(args(&["--slidock"]).is_err());
        assert!(args(&["--check"]).is_err());
    }
}
