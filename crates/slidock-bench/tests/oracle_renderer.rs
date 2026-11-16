//! `slidock-bench oracle` releases the renderer container it starts, run as a child process against a fake `docker`
//! (a shell script that logs every call and keeps each container's state in a file named after it) and a fake
//! browser endpoint: a container the command started is its own (not the cache's shared one) and is removed when the
//! command passes, when it fails after the start, and when the container never answers; the shared renderer, found
//! running or started by another command while the oracle runs, is never removed. `renderer stop` removes the shared
//! one and a container whose command ended without removing it. A fake `osascript` answers that no Keynote is
//! installed, so nothing is launched or asked.
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// `$state/c/<name>.json` is a running container's `docker container inspect`, `$state/c/<name>.owner` the pid of
/// the process that owns it. With `$state/concurrent` set, the oracle's first exec in its own container also starts
/// the shared renderer (its name less the own suffix), as a `renderer start` or a static `run` would meanwhile.
const DOCKER: &str = r#"#!/bin/sh
state="$FAKE_DOCKER_STATE"
mkdir -p "$state/c"
echo "$*" >> "$state/calls"
case "$1" in
image) echo "sha256:fake-image" ;;
container)
  if [ -f "$state/c/$3.json" ]; then cat "$state/c/$3.json"; else echo "No such container" >&2; exit 1; fi ;;
run)
  shift
  labels=""; name=""; owner=""
  while [ $# -gt 0 ]; do
    case "$1" in
    --name) name="$2"; shift ;;
    --label)
      key="${2%%=*}"; value="${2#*=}"
      if [ "$key" = "slidock-bench.owner" ]; then owner="$value"; fi
      labels="$labels${labels:+,}\"$key\":\"$value\""
      shift ;;
    esac
    shift
  done
  printf '[{"State":{"Running":true},"Image":"sha256:fake-image","Config":{"Labels":{%s}}}]' "$labels" > "$state/c/$name.json"
  if [ -n "$owner" ]; then echo "$owner" > "$state/c/$name.owner"; fi
  echo "$name" > "$state/last-run"
  echo fake-container-id ;;
port)
  if [ -f "$state/no-port" ]; then echo "no public port" >&2; exit 1; fi
  echo "127.0.0.1:$FAKE_BROWSER_PORT" ;;
exec)
  if [ -f "$state/concurrent" ]; then
    rm -f "$state/concurrent"
    shared=$(echo "$2" | sed 's/-p[0-9]*-[0-9a-f]*$//')
    sed 's/,"slidock-bench.owner":"[0-9]*"//' "$state/c/$2.json" > "$state/c/$shared.json"
  fi
  case "$3" in
  uname) echo aarch64 ;;
  pdftoppm) if [ -f "$state/bad-poppler" ]; then echo "garbled" >&2; else echo "pdftoppm version 24.02.0" >&2; fi ;;
  esac ;;
ps)
  for owner in "$state"/c/*.owner; do
    [ -f "$owner" ] || continue
    echo "$(basename "$owner" .owner) $(cat "$owner")"
  done ;;
rm)
  if [ -f "$state/c/$3.json" ]; then rm -f "$state/c/$3.json" "$state/c/$3.owner"; else exit 1; fi ;;
esac
"#;

/// Answers every request with a browser's `/json/version`.
fn fake_browser() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            let body = r#"{"webSocketDebuggerUrl":"ws://127.0.0.1:1/devtools/browser/fake"}"#;
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    port
}

struct Bench {
    dir: tempfile::TempDir,
    port: u16,
}

impl Bench {
    /// An empty manifest, an empty corpus folder, and a cache whose font set holds every file the lock pins (named by
    /// hash, which is all the set check reads).
    fn new() -> Bench {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for sub in ["bin", "state", "corpus", "cache/fonts"] {
            std::fs::create_dir_all(root.join(sub)).unwrap();
        }
        let script = |name: &str, text: &str| {
            let path = root.join("bin").join(name);
            std::fs::write(&path, text).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        script("docker", DOCKER);
        script("osascript", "#!/bin/sh\nexit 0\n");
        std::fs::write(
            root.join("manifest.json"),
            r#"{"version":1,"presentations":[]}"#,
        )
        .unwrap();
        let lock: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/fonts/fonts.lock.json"),
            )
            .unwrap(),
        )
        .unwrap();
        for entry in lock["fonts"].as_array().unwrap() {
            let file = entry["file"].as_str().unwrap();
            let extension = file
                .rfind('.')
                .map(|at| file[at..].to_ascii_lowercase())
                .unwrap();
            let name = format!("{}{extension}", entry["sha256"].as_str().unwrap());
            std::fs::write(root.join("cache/fonts").join(name), "").unwrap();
        }
        Bench {
            dir,
            port: fake_browser(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn oracle(&self) -> Output {
        self.bench(&["oracle"])
    }

    fn bench(&self, arguments: &[&str]) -> Output {
        let path = format!(
            "{}:{}",
            self.path("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        Command::new(env!("CARGO_BIN_EXE_slidock-bench"))
            .args(arguments)
            .env("PATH", path)
            .env("FAKE_DOCKER_STATE", self.path("state"))
            .env("FAKE_BROWSER_PORT", self.port.to_string())
            .env("SLIDOCK_PRIVATE", self.path("private"))
            .env("SLIDOCK_CORPUS_DIR", self.path("corpus"))
            .env("SLIDOCK_CORPUS_MANIFEST", self.path("manifest.json"))
            .env("SLIDOCK_BENCH_CACHE", self.path("cache"))
            .output()
            .unwrap()
    }

    /// The docker calls since the last one, by verb (`container inspect`, `run`, `rm`, …), and forgets them.
    fn calls(&self) -> Vec<String> {
        let path = self.path("state/calls");
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        std::fs::remove_file(&path).unwrap_or_default();
        text.lines()
            .map(|line| line.split(' ').next().unwrap_or_default().to_owned())
            .collect()
    }

    /// The running containers' names, sorted.
    fn running(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.path("state/c"))
            .map(|dir| {
                dir.filter_map(|entry| {
                    let name = entry.ok()?.file_name().into_string().ok()?;
                    name.strip_suffix(".json").map(str::to_owned)
                })
                .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// The name the last `docker run` started.
    fn last_run(&self) -> String {
        std::fs::read_to_string(self.path("state/last-run"))
            .unwrap()
            .trim()
            .to_owned()
    }

    fn flag(&self, name: &str, on: bool) {
        let path = self.path("state").join(name);
        if on {
            std::fs::write(path, "").unwrap();
        } else {
            std::fs::remove_file(path).unwrap_or_default();
        }
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The shared renderer's name: an own container's name less its `-p<pid>-<hex>` suffix.
fn shared_of(own: &str) -> String {
    let (base, _) = own.rsplit_once("-p").unwrap();
    base.to_owned()
}

#[test]
fn a_container_the_command_started_is_its_own_and_removed_whether_it_passes_or_fails() {
    let bench = Bench::new();
    let passed = bench.oracle();
    assert_eq!(passed.status.code(), Some(0), "{}", stderr(&passed));
    let calls = bench.calls();
    let run = calls.iter().position(|c| c == "run").unwrap();
    assert_eq!(calls.last().map(String::as_str), Some("rm"), "{calls:?}");
    assert!(run < calls.len() - 1);
    assert!(bench.running().is_empty());
    // Its name is its own: the shared one with this process's suffix.
    let own = bench.last_run();
    assert!(own.starts_with("slidock-bench-"), "{own}");
    assert_ne!(shared_of(&own), own);
    // Another oracle's container has another name.
    assert_eq!(bench.oracle().status.code(), Some(0));
    assert_ne!(bench.last_run(), own);
    bench.calls();

    // A failure after the start (poppler's version does not read).
    bench.flag("bad-poppler", true);
    let failed = bench.oracle();
    assert_eq!(failed.status.code(), Some(1), "{}", stderr(&failed));
    assert!(
        stderr(&failed).contains("pdftoppm -v"),
        "{}",
        stderr(&failed)
    );
    let calls = bench.calls();
    assert!(calls.contains(&"run".to_owned()), "{calls:?}");
    assert_eq!(calls.last().map(String::as_str), Some("rm"), "{calls:?}");
    assert!(bench.running().is_empty());
    bench.flag("bad-poppler", false);

    // A container that never answers: the start fails and removes it.
    bench.flag("no-port", true);
    let unready = bench.oracle();
    assert_eq!(unready.status.code(), Some(1), "{}", stderr(&unready));
    let calls = bench.calls();
    assert!(calls.contains(&"run".to_owned()), "{calls:?}");
    assert_eq!(calls.last().map(String::as_str), Some("rm"), "{calls:?}");
    assert!(bench.running().is_empty());
}

#[test]
fn the_shared_renderer_another_command_starts_meanwhile_keeps_running() {
    let bench = Bench::new();
    bench.flag("concurrent", true);
    let passed = bench.oracle();
    assert_eq!(passed.status.code(), Some(0), "{}", stderr(&passed));
    // The oracle removed its own container and left the shared one, which another command uses.
    let shared = shared_of(&bench.last_run());
    assert_eq!(bench.running(), std::slice::from_ref(&shared));
    let calls = std::fs::read_to_string(bench.path("state/calls")).unwrap();
    assert!(!calls.contains(&format!("rm -f {shared}\n")), "{calls}");
}

#[test]
fn a_running_renderer_the_command_reused_keeps_running() {
    let bench = Bench::new();
    bench.flag("concurrent", true);
    assert_eq!(bench.oracle().status.code(), Some(0));
    let shared = bench.running();
    assert_eq!(shared.len(), 1);
    bench.calls();
    // The shared renderer is running with the same labels: the next run reuses it.
    let reused = bench.oracle();
    assert_eq!(reused.status.code(), Some(0), "{}", stderr(&reused));
    let calls = bench.calls();
    assert!(!calls.contains(&"run".to_owned()), "{calls:?}");
    assert!(!calls.contains(&"rm".to_owned()), "{calls:?}");
    assert_eq!(bench.running(), shared);

    // It fails with it: still not removed.
    bench.flag("bad-poppler", true);
    assert_eq!(bench.oracle().status.code(), Some(1));
    assert!(!bench.calls().contains(&"rm".to_owned()));
    assert_eq!(bench.running(), shared);
}

#[test]
fn renderer_stop_removes_a_container_whose_command_ended_without_removing_it() {
    let bench = Bench::new();
    let container = |name: &str, owner: u32| {
        std::fs::create_dir_all(bench.path("state/c")).unwrap();
        std::fs::write(bench.path(&format!("state/c/{name}.json")), "[]").unwrap();
        std::fs::write(
            bench.path(&format!("state/c/{name}.owner")),
            owner.to_string(),
        )
        .unwrap();
    };
    let mut ended = Command::new("true").spawn().unwrap();
    let gone = ended.id();
    ended.wait().unwrap();
    container("slidock-bench-orphan-p1-a", gone);
    container("slidock-bench-live-p2-b", std::process::id());
    let stopped = bench.bench(&["renderer", "stop"]);
    assert_eq!(stopped.status.code(), Some(0), "{}", stderr(&stopped));
    let out = String::from_utf8_lossy(&stopped.stdout).into_owned();
    assert!(out.contains("removed slidock-bench-orphan-p1-a"), "{out}");
    // The one whose process still runs is that command's to remove.
    assert_eq!(bench.running(), ["slidock-bench-live-p2-b"]);
}
