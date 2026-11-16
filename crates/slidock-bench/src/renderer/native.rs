//! The renderer's Chromium run on the host instead of in the container: the same headless shell build Playwright 1.60
//! installs for this platform, started with the container's switch list. It sees the host's fonts, so its pixels are
//! not the container's. The fonts lane takes it where Docker is unavailable: glyph attribution only asks which glyphs
//! a face the deck ships drew, and every glyph it did not draw is the renderer's own, whichever fonts that renderer
//! has.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use crate::error::{BenchError, Result};
use crate::hash::{sha256, sha256_json};

use super::docker::wait_for_browser;

/// The headless shell revision Playwright 1.60 installs (the container's is `chromium_headless_shell-1223`).
const PLAYWRIGHT_REVISION: &str = "chromium_headless_shell-1223";

/// The switches of the container's start script that name its own paths and ports; the host's replace them.
const CONTAINER_ONLY: [&str; 2] = ["--user-data-dir=", "--remote-debugging-port="];

/// `SLIDOCK_BENCH_CHROMIUM`, else Playwright's install of the pinned revision in the user's cache.
pub fn default_binary() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SLIDOCK_BENCH_CHROMIUM").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let cache = if cfg!(target_os = "macos") {
        home.join("Library/Caches/ms-playwright")
    } else {
        home.join(".cache/ms-playwright")
    };
    let folder = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "chrome-headless-shell-mac-arm64/chrome-headless-shell",
        ("macos", _) => "chrome-headless-shell-mac-x64/chrome-headless-shell",
        (_, "aarch64") => "chrome-linux-arm64/headless_shell",
        _ => "chrome-linux/headless_shell",
    };
    Some(cache.join(PLAYWRIGHT_REVISION).join(folder))
}

/// The container's switches without the ones that name its paths and ports.
pub fn host_switches(chrome_args: &str) -> Vec<String> {
    chrome_args
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !CONTAINER_ONLY.iter().any(|prefix| line.starts_with(prefix)))
        .map(str::to_owned)
        .collect()
}

/// The switch's name: `--name` of `--name=value`.
fn switch_name(switch: &str) -> &str {
    switch.split_once('=').map_or(switch, |(name, _)| name)
}

/// `switches` with each of `replacing` in place of the ones of its name, or after them.
fn with_switches(mut switches: Vec<String>, replacing: &[String]) -> Vec<String> {
    for switch in replacing {
        switches.retain(|s| switch_name(s) != switch_name(switch));
        switches.push(switch.clone());
    }
    switches
}

/// A CPU time as macOS `ps -o time=` prints it (`[[dd-]hh:]mm:ss.cc`), in milliseconds. A time in whole seconds (Linux
/// procps) is not read: the budgets' time floor assumes steps of 10 ms.
fn parse_cpu_time(text: &str) -> Option<u64> {
    let text = text.trim();
    let (days, rest) = match text.split_once('-') {
        Some((days, rest)) => (days.parse::<u64>().ok()?, rest),
        None => (0, text),
    };
    let (clock, fraction) = rest.split_once('.')?;
    let mut seconds = 0u64;
    for part in clock.split(':') {
        seconds = seconds
            .checked_mul(60)?
            .checked_add(part.parse::<u64>().ok()?)?;
    }
    if fraction.len() != 2 {
        return None;
    }
    let centis = fraction.parse::<u64>().ok()?;
    days.checked_mul(86_400)?
        .checked_add(seconds)?
        .checked_mul(1000)?
        .checked_add(centis * 10)
}

/// The CPU time of the descendants of `root` whose command line holds `marker`, in milliseconds, from
/// `ps -A -o pid=,ppid=,time=,command=`. A line it cannot read fails, as does a time in whole seconds: a tree it cannot
/// see whole would undercount, and a step of a second would swamp a small deck's compile.
fn tree_cpu_ms(listing: &str, root: u32, marker: &str) -> Option<u64> {
    let mut rows: Vec<(u32, u32, u64, bool)> = Vec::new();
    for line in listing.lines().filter(|l| !l.trim().is_empty()) {
        let mut fields = line.split_whitespace();
        let pid = fields.next()?.parse().ok()?;
        let ppid = fields.next()?.parse().ok()?;
        let time = parse_cpu_time(fields.next()?)?;
        let marked = fields.any(|word| word == marker);
        rows.push((pid, ppid, time, marked));
    }
    let mut tree = vec![root];
    let mut grown = true;
    while grown {
        grown = false;
        for (pid, ppid, _, _) in &rows {
            if tree.contains(ppid) && !tree.contains(pid) {
                tree.push(*pid);
                grown = true;
            }
        }
    }
    rows.iter()
        .filter(|(pid, _, _, marked)| *marked && tree.contains(pid))
        .try_fold(0u64, |sum, (_, _, time, _)| sum.checked_add(*time))
}

impl NativeBrowser {
    /// The CPU time the browser's renderer processes have used, in milliseconds: the pages and their workers.
    pub async fn renderer_cpu_ms(&self) -> Result<u64> {
        let root = self
            .pid()
            .ok_or_else(|| BenchError::failed("the browser has exited"))?;
        let listing = crate::exec::exec_text(
            "ps",
            &crate::exec::args(["-A", "-o", "pid=,ppid=,time=,command="]),
            &crate::exec::ExecOptions::timeout(Duration::from_secs(30)),
        )
        .await?;
        tree_cpu_ms(&listing, root, "--type=renderer").ok_or_else(|| {
            BenchError::failed(
                "ps: a process line could not be read, or its CPU time has no centiseconds",
            )
        })
    }
}

/// A headless shell running on the host. Dropping it kills the browser.
pub struct NativeBrowser {
    child: tokio::process::Child,
    /// The browser's WebSocket debugger URL.
    pub ws_endpoint: String,
    /// What the run is stamped with: the binary's and the switches' sha256.
    pub recipe: String,
    profile: PathBuf,
}

impl NativeBrowser {
    /// Starts the binary with the container's switches, a fresh profile under `profile` and a free loopback port.
    pub async fn start(bench_dir: &Path, binary: &Path, profile: &Path) -> Result<NativeBrowser> {
        NativeBrowser::start_with(bench_dir, binary, profile, &[]).await
    }

    /// [`NativeBrowser::start`] with `replacing` in place of the container's switches of the same names
    /// (`--name=value`), or after them.
    pub async fn start_with(
        bench_dir: &Path,
        binary: &Path,
        profile: &Path,
        replacing: &[String],
    ) -> Result<NativeBrowser> {
        if !binary.is_file() {
            return Err(BenchError::refused(format!(
                "no headless shell at {}: install Playwright 1.60's Chromium or set SLIDOCK_BENCH_CHROMIUM",
                binary.display()
            )));
        }
        let args_path = bench_dir.join("docker/chrome-args.txt");
        let chrome_args =
            std::fs::read_to_string(&args_path).map_err(BenchError::io(&args_path))?;
        let switches = with_switches(host_switches(&chrome_args), replacing);
        let recipe = {
            #[derive(serde::Serialize)]
            struct NativeRecipe<'a> {
                binary: String,
                switches: &'a [String],
            }
            sha256_json(&NativeRecipe {
                binary: sha256(std::fs::read(binary).map_err(BenchError::io(binary))?),
                switches: &switches,
            })?
        };
        crate::cache::remove(profile)?;
        std::fs::create_dir_all(profile).map_err(BenchError::io(profile))?;
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .map_err(|error| BenchError::failed(format!("no free loopback port: {error}")))?
            .port();
        let child = tokio::process::Command::new(binary)
            .args(&switches)
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("--remote-debugging-address=127.0.0.1")
            .arg(format!("--remote-debugging-port={port}"))
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| BenchError::failed(format!("{}: {error}", binary.display())))?;
        let mut browser = NativeBrowser {
            child,
            ws_endpoint: String::new(),
            recipe,
            profile: profile.to_path_buf(),
        };
        browser.ws_endpoint =
            wait_for_browser(&format!("http://127.0.0.1:{port}"), Duration::from_secs(60)).await?;
        Ok(browser)
    }

    /// The browser process's id.
    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    /// Kills the browser and removes its profile.
    pub async fn stop(mut self) -> Result<()> {
        let _ = self.child.kill().await;
        crate::cache::remove(&self.profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_times_read_as_ps_prints_them() {
        assert_eq!(parse_cpu_time("0:00.53"), Some(530));
        assert_eq!(
            parse_cpu_time("316:50.19"),
            Some((316 * 60 + 50) * 1000 + 190)
        );
        assert_eq!(parse_cpu_time("01:02:03.04"), Some(3_723_040));
        assert_eq!(
            parse_cpu_time("2-00:00:01.00"),
            Some((2 * 86_400 + 1) * 1000)
        );
        // Whole seconds, as Linux procps prints them, are too coarse for a small deck's compile.
        for bad in [
            "",
            "x",
            "1:2.3",
            "1:-2",
            "1:02.345",
            "01:02:03",
            "2-00:00:01",
            "0:05",
        ] {
            assert_eq!(parse_cpu_time(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_trees_cpu_time_is_its_marked_descendants() {
        let listing = "  1 0 9:00.00 /sbin/launchd\n 10 1 0:01.00 shell --headless\n \
                       11 10 0:02.50 shell --type=renderer --lang=en\n 12 11 0:00.25 shell --type=renderer\n \
                       14 10 0:04.00 shell --type=gpu-process\n 13 1 0:07.00 other --type=renderer\n";
        assert_eq!(tree_cpu_ms(listing, 10, "--type=renderer"), Some(2_750));
        assert_eq!(tree_cpu_ms(listing, 12, "--type=renderer"), Some(250));
        assert_eq!(tree_cpu_ms(listing, 99, "--type=renderer"), Some(0));
        // A process that only mentions the marker inside a longer word is not one.
        assert_eq!(
            tree_cpu_ms(
                " 10 1 0:01.00 x --type=renderer-ish\n",
                10,
                "--type=renderer"
            ),
            Some(0)
        );
        assert_eq!(
            tree_cpu_ms("  10 1 garbage x\n", 10, "--type=renderer"),
            None
        );
        assert_eq!(
            tree_cpu_ms(
                " 10 1 00:00:01 shell\n 11 10 00:00:02 shell --type=renderer\n",
                10,
                "--type=renderer"
            ),
            None
        );
    }

    #[test]
    fn a_replacing_switch_takes_the_place_of_its_namesakes() {
        let switches = vec![
            "--headless".to_owned(),
            "--host-resolver-rules=MAP * ~NOTFOUND".to_owned(),
        ];
        let rules = "--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1".to_owned();
        assert_eq!(
            with_switches(switches.clone(), std::slice::from_ref(&rules)),
            ["--headless".to_owned(), rules]
        );
        assert_eq!(
            with_switches(switches.clone(), &["--mute-audio".to_owned()]).len(),
            3
        );
        assert_eq!(with_switches(switches.clone(), &[]), switches);
    }

    #[test]
    fn the_host_keeps_every_switch_but_the_containers_paths_and_port() {
        let args = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/docker/chrome-args.txt"),
        )
        .unwrap();
        let switches = host_switches(&args);
        assert!(switches.contains(&"--headless".to_owned()));
        assert!(switches.contains(&"--font-render-hinting=none".to_owned()));
        assert!(switches.contains(&"--host-resolver-rules=MAP * ~NOTFOUND".to_owned()));
        assert!(!switches.iter().any(|s| s.starts_with("--user-data-dir")));
        assert!(
            !switches
                .iter()
                .any(|s| s.starts_with("--remote-debugging-port"))
        );
        assert_eq!(
            switches.len(),
            args.lines().filter(|l| !l.is_empty()).count() - 2
        );
    }
}
