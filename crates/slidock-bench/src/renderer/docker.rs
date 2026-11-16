//! The renderer container: built from `bench/docker/` (pinned base digest, poppler, the
//! headless shell's switches), started once and reused while its recipe, font set and mounts match. Chromium and
//! poppler both run inside it, so the oracle's rasters and the renders come from one pinned environment. Docker serves
//! bench renders only; compiling runs without it.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::error::{BenchError, Result};
use crate::exec::{ExecOptions, ExecResult, args, exec, exec_text};
use crate::hash::{sha256, sha256_json};

/// The base image's linux/arm64 digest; every build and run uses this platform.
pub const PLATFORM_DIGEST: &str =
    "sha256:beddddcf96f19abec078cd2f3da676f40abd460e0e734ad5eed73627ce161d33";
pub const PLATFORM: &str = "linux/arm64";
const RECIPE_FILES: [&str; 4] = [
    "Dockerfile",
    "fonts.conf",
    "chrome-args.txt",
    "start-chrome.sh",
];
/// The CDP port inside the container, published on the host's loopback only.
const CDP_PORT: &str = "9222/tcp";
const LABEL: &str = "slidock-bench";

/// The sha256 of everything the image is built from.
pub fn recipe_hash(bench_dir: &Path) -> Result<String> {
    let mut files = BTreeMap::new();
    for name in RECIPE_FILES {
        let path = bench_dir.join("docker").join(name);
        files.insert(
            name,
            sha256(std::fs::read(&path).map_err(BenchError::io(&path))?),
        );
    }
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Recipe<'a> {
        files: BTreeMap<&'a str, String>,
        platform_digest: &'a str,
    }
    sha256_json(&Recipe {
        files,
        platform_digest: PLATFORM_DIGEST,
    })
}

pub fn image_tag(recipe: &str) -> String {
    format!(
        "slidock-bench-renderer:{}",
        recipe.get(..12).unwrap_or(recipe)
    )
}

fn docker_options(seconds: u64) -> ExecOptions {
    ExecOptions::timeout(Duration::from_secs(seconds))
}

async fn image_id(tag: &str) -> Result<Option<String>> {
    let result = exec(
        "docker",
        &args(["image", "inspect", "--format", "{{.Id}}", tag]),
        &docker_options(30),
    )
    .await?;
    Ok(result.ok().then(|| result.stdout_text().trim().to_owned()))
}

/// The image built from the recipe.
#[derive(Clone, Debug)]
pub struct Image {
    pub recipe: String,
    pub tag: String,
    pub id: String,
}

/// Builds the image (or finds it built).
pub async fn ensure_image(bench_dir: &Path, rebuild: bool) -> Result<Image> {
    let recipe = recipe_hash(bench_dir)?;
    let tag = image_tag(&recipe);
    let mut id = if rebuild { None } else { image_id(&tag).await? };
    if id.is_none() {
        let context = bench_dir.join("docker").to_string_lossy().into_owned();
        exec_text(
            "docker",
            &args(["build", "--platform", PLATFORM, "-t", &tag, &context]),
            &docker_options(30 * 60),
        )
        .await?;
        id = image_id(&tag).await?;
    }
    let id =
        id.ok_or_else(|| BenchError::failed(format!("docker build produced no image {tag}")))?;
    Ok(Image { recipe, tag, id })
}

/// A running renderer.
#[derive(Clone, Debug)]
pub struct Renderer {
    pub container: String,
    pub recipe: String,
    pub image_id: String,
    pub font_set_id: String,
    /// The CDP endpoint on a loopback port of the host.
    pub http: String,
    /// The browser's WebSocket debugger URL.
    pub ws_endpoint: String,
    /// The host folder mounted at /work.
    pub work: PathBuf,
    /// The container is this process's own ([`own_renderer`]), not the shared one: [`Renderer::release`] removes it.
    pub owned: bool,
}

#[derive(Deserialize)]
struct Inspect {
    #[serde(rename = "State")]
    state: InspectState,
    #[serde(rename = "Config")]
    config: InspectConfig,
    #[serde(rename = "Image")]
    image: String,
}

#[derive(Deserialize)]
struct InspectState {
    #[serde(rename = "Running")]
    running: bool,
}

#[derive(Deserialize)]
struct InspectConfig {
    #[serde(rename = "Labels")]
    labels: Option<BTreeMap<String, String>>,
}

async fn inspect_container(name: &str) -> Result<Option<Inspect>> {
    let result = exec(
        "docker",
        &args(["container", "inspect", name]),
        &docker_options(30),
    )
    .await?;
    if !result.ok() {
        return Ok(None);
    }
    let mut parsed: Vec<Inspect> = serde_json::from_slice(&result.stdout)
        .map_err(|error| BenchError::failed(format!("docker container inspect {name}: {error}")))?;
    Ok(parsed.pop())
}

async fn host_port(name: &str) -> Result<u16> {
    let text = exec_text(
        "docker",
        &args(["port", name, CDP_PORT]),
        &docker_options(30),
    )
    .await?;
    text.lines()
        .filter_map(|line| line.trim().rsplit(':').next())
        .find_map(|port| port.parse().ok())
        .ok_or_else(|| BenchError::failed(format!("no host port for {name}: {text}")))
}

#[derive(Deserialize)]
struct Version {
    #[serde(rename = "webSocketDebuggerUrl")]
    web_socket_debugger_url: String,
}

/// Polls `/json/version` until the browser answers, and returns its WebSocket URL.
pub(crate) async fn wait_for_browser(http: &str, timeout: Duration) -> Result<String> {
    let url = format!("{http}/json/version");
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let attempt = url.clone();
        let answer = tokio::task::spawn_blocking(move || -> Option<String> {
            let mut response = ureq::get(&attempt).call().ok()?;
            let mut text = String::new();
            response
                .body_mut()
                .as_reader()
                .read_to_string(&mut text)
                .ok()?;
            let version: Version = serde_json::from_str(&text).ok()?;
            Some(version.web_socket_debugger_url)
        })
        .await
        .ok()
        .flatten();
        if let Some(ws) = answer {
            return Ok(ws);
        }
        if tokio::time::Instant::now() > deadline {
            return Err(BenchError::failed(format!(
                "the renderer did not answer on {url} within {} s",
                timeout.as_secs()
            )));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// The shared container of a cache: one per recipe and work folder, so each cache keeps its own.
pub fn shared_name(recipe: &str, work: &Path) -> String {
    format!(
        "slidock-bench-{}-{}",
        recipe.get(..12).unwrap_or(recipe),
        sha256(work.to_string_lossy().as_bytes())
            .get(..8)
            .unwrap_or_default()
    )
}

/// A renderer started from the same image with another fontconfig: the fonts lane's, which sees only the folder
/// mounted at /fonts with no family aliased to another (`bench/docker/<key>/fonts.conf`).
#[derive(Clone, Copy, Debug)]
pub struct Variant<'a> {
    /// The folder under `bench/docker/` that holds its `fonts.conf`; also part of its container name.
    pub key: &'a str,
}

impl Variant<'_> {
    fn fonts_conf(&self, bench_dir: &Path) -> PathBuf {
        bench_dir.join("docker").join(self.key).join("fonts.conf")
    }

    /// The variant's recipe: the image's, and its `fonts.conf`.
    pub fn recipe(&self, bench_dir: &Path) -> Result<String> {
        let conf = self.fonts_conf(bench_dir);
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct VariantRecipe<'a> {
            image: String,
            variant: &'a str,
            fonts_conf: String,
        }
        sha256_json(&VariantRecipe {
            image: recipe_hash(bench_dir)?,
            variant: self.key,
            fonts_conf: sha256(std::fs::read(&conf).map_err(BenchError::io(&conf))?),
        })
    }

    /// Its container of a cache: one per work folder, beside the shared renderer.
    pub fn name(&self, recipe: &str, work: &Path) -> String {
        let shared = shared_name(recipe, work);
        match shared.strip_prefix("slidock-bench-") {
            Some(rest) => format!("slidock-bench-{}-{rest}", self.key),
            None => format!("{shared}-{}", self.key),
        }
    }
}

/// Starts the cache's shared renderer, or reuses it when its image, recipe, font set and work folder match; a
/// mismatching one is replaced. `fonts` is mounted read-only at /fonts, `work` at /work. Every command may reuse it,
/// so none removes it but `renderer stop`.
pub async fn start_renderer(
    bench_dir: &Path,
    fonts: &Path,
    work: &Path,
    font_set_id: &str,
) -> Result<Renderer> {
    start_container(bench_dir, fonts, work, font_set_id, None, Owner::Shared).await
}

/// A renderer for one command, which [`Renderer::release`] removes once the command is done: the cache's shared
/// renderer when it is running and matches (reused, left running), else a container of this process's own under a
/// name no other command looks up, so removing it never takes a renderer from under another command.
pub async fn own_renderer(
    bench_dir: &Path,
    fonts: &Path,
    work: &Path,
    font_set_id: &str,
) -> Result<Renderer> {
    start_container(bench_dir, fonts, work, font_set_id, None, Owner::Process).await
}

/// A variant of the renderer for one command: the same image, `fonts` at /fonts, and the variant's fontconfig, in a
/// container of this process's own ([`own_renderer`]).
pub async fn start_variant(
    bench_dir: &Path,
    fonts: &Path,
    work: &Path,
    font_set_id: &str,
    variant: Variant<'_>,
) -> Result<Renderer> {
    start_container(
        bench_dir,
        fonts,
        work,
        font_set_id,
        Some(variant),
        Owner::Process,
    )
    .await
}

/// Who a container a call starts belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Owner {
    /// The cache's shared name: started or reused by any command, removed by `renderer stop` only.
    Shared,
    /// The process: a name of its own ([`own_name`]), removed by the process.
    Process,
}

/// A container name of this process's own beside `base`: the process id and a count of this process's starts, so
/// neither another running process nor a later call of this one starts or reuses a container under it.
fn own_name(base: &str) -> String {
    static STARTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let start = STARTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{base}-p{}-{start:x}", std::process::id())
}

async fn start_container(
    bench_dir: &Path,
    fonts: &Path,
    work: &Path,
    font_set_id: &str,
    variant: Option<Variant<'_>>,
    owner: Owner,
) -> Result<Renderer> {
    std::fs::create_dir_all(work).map_err(BenchError::io(work))?;
    let image = ensure_image(bench_dir, false).await?;
    let recipe = match variant {
        Some(variant) => variant.recipe(bench_dir)?,
        None => image.recipe.clone(),
    };
    let shared = shared_name(&image.recipe, work);
    let work_text = work.to_string_lossy().into_owned();
    let labels = [
        (format!("{LABEL}.recipe"), recipe.clone()),
        (format!("{LABEL}.fontset"), font_set_id.to_owned()),
        (format!("{LABEL}.work"), work_text.clone()),
    ];
    // Only the shared renderer is reused; a variant always runs in a container of its own.
    let existing = match variant {
        None => inspect_container(&shared).await?,
        Some(_) => None,
    };
    let matches = existing.as_ref().is_some_and(|c| {
        c.state.running
            && c.image == image.id
            && labels.iter().all(|(key, value)| {
                c.config.labels.as_ref().and_then(|l| l.get(key)) == Some(value)
            })
    });
    let name = match (owner, variant) {
        _ if matches => shared,
        (Owner::Shared, _) => {
            if existing.is_some() {
                exec("docker", &args(["rm", "-f", &shared]), &docker_options(60)).await?;
            }
            shared
        }
        (Owner::Process, variant) => {
            let name = own_name(&match variant {
                Some(variant) => variant.name(&image.recipe, work),
                None => shared,
            });
            // A container under this name was left by an earlier process with this pid, which has ended.
            if inspect_container(&name).await?.is_some() {
                exec("docker", &args(["rm", "-f", &name]), &docker_options(60)).await?;
            }
            name
        }
    };
    let owned = !matches && owner == Owner::Process;
    if !matches {
        let mut arguments: Vec<String> = [
            "run",
            "-d",
            "--rm",
            "--init",
            "--platform",
            PLATFORM,
            "--name",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        arguments.push(name.clone());
        for (key, value) in &labels {
            arguments.push("--label".to_owned());
            arguments.push(format!("{key}={value}"));
        }
        if owned {
            arguments.push("--label".to_owned());
            arguments.push(format!("{LABEL}.owner={}", std::process::id()));
        }
        arguments.extend([
            "-p".to_owned(),
            format!("127.0.0.1::{}", CDP_PORT.trim_end_matches("/tcp")),
            "--shm-size=1g".to_owned(),
            "-v".to_owned(),
            format!("{}:/fonts:ro", fonts.display()),
            "-v".to_owned(),
            format!("{work_text}:/work"),
        ]);
        if let Some(variant) = variant {
            arguments.extend([
                "-v".to_owned(),
                format!(
                    "{}:/etc/slidock-bench/fonts.conf:ro",
                    variant.fonts_conf(bench_dir).display()
                ),
            ]);
        }
        arguments.extend([
            "--memory".to_owned(),
            "4g".to_owned(),
            "--cpus".to_owned(),
            "4".to_owned(),
            "--security-opt".to_owned(),
            "no-new-privileges".to_owned(),
            image.tag.clone(),
        ]);
        match exec_text("docker", &args(arguments), &docker_options(120)).await {
            Ok(_) => {}
            Err(error) if owned => return Err(discard(&name, error).await),
            Err(error) => return Err(error),
        }
    }
    let ready = async {
        let port = host_port(&name).await?;
        let http = format!("http://127.0.0.1:{port}");
        let ws_endpoint = wait_for_browser(&http, Duration::from_secs(60)).await?;
        Ok((http, ws_endpoint))
    };
    // A container of this process's own that never answered is removed with the error.
    let (http, ws_endpoint) = match ready.await {
        Ok(ready) => ready,
        Err(error) if owned => return Err(discard(&name, error).await),
        Err(error) => return Err(error),
    };
    Ok(Renderer {
        container: name,
        recipe,
        image_id: image.id,
        font_set_id: font_set_id.to_owned(),
        http,
        ws_endpoint,
        work: work.to_path_buf(),
        owned,
    })
}

/// Removes a container of this process's own, and returns the error that stopped it (a failed removal is printed).
async fn discard(name: &str, error: BenchError) -> BenchError {
    if let Err(removal) = remove_container(name).await {
        eprintln!("{removal}");
    }
    error
}

async fn remove_container(name: &str) -> Result<bool> {
    Ok(
        exec("docker", &args(["rm", "-f", name]), &docker_options(60))
            .await?
            .ok(),
    )
}

/// What a command returns once it released its renderer ([`Renderer::release`]): the command's own error first,
/// else the release's; a release error behind a command error is printed.
pub fn released<T>(outcome: Result<T>, release: Result<bool>) -> Result<T> {
    match (outcome, release) {
        (Ok(value), Ok(_)) => Ok(value),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), released) => {
            if let Err(release) = released {
                eprintln!("{release}");
            }
            Err(error)
        }
    }
}

/// Removes the cache's shared renderer (or the named one); `true` when one was running.
pub async fn stop_renderer(bench_dir: &Path, work: &Path, name: Option<&str>) -> Result<bool> {
    let target = match name {
        Some(name) => name.to_owned(),
        None => shared_name(&recipe_hash(bench_dir)?, work),
    };
    remove_container(&target).await
}

/// Removes every renderer container a process started for itself ([`own_renderer`]) and left behind when it ended
/// (interrupted before its release): those whose owner process is gone. Returns their names.
pub async fn remove_orphans() -> Result<Vec<String>> {
    let listed = exec_text(
        "docker",
        &args([
            "ps",
            "-a",
            "--filter",
            &format!("label={LABEL}.owner"),
            "--format",
            &format!("{{{{.Names}}}} {{{{.Label \"{LABEL}.owner\"}}}}"),
        ]),
        &docker_options(30),
    )
    .await?;
    let mut removed = Vec::new();
    for (name, owner) in owned_containers(&listed) {
        if owner == std::process::id() || process_alive(owner) {
            continue;
        }
        if remove_container(name).await? {
            removed.push(name.to_owned());
        }
    }
    Ok(removed)
}

/// The `<name> <owner pid>` lines of `docker ps`; a line without a readable pid is not one of ours.
fn owned_containers(listed: &str) -> Vec<(&str, u32)> {
    listed
        .lines()
        .filter_map(|line| {
            let (name, owner) = line.trim().split_once(' ')?;
            Some((name, owner.trim().parse().ok()?))
        })
        .filter(|(name, _)| name.starts_with("slidock-bench-"))
        .collect()
}

/// Whether `pid` names a running process, as `kill(pid, 0)` reads it: only `ESRCH` says it is gone. `EPERM` is a
/// process of another user, alive; any other answer, or a pid the call cannot name, keeps its container.
fn process_alive(pid: u32) -> bool {
    let Ok(raw) = i32::try_from(pid) else {
        return true;
    };
    !matches!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(raw), None),
        Err(nix::errno::Errno::ESRCH)
    )
}

/// Whether the cache's shared renderer is running.
pub async fn renderer_running(bench_dir: &Path, work: &Path) -> Result<Option<String>> {
    let name = shared_name(&recipe_hash(bench_dir)?, work);
    Ok(inspect_container(&name)
        .await?
        .filter(|c| c.state.running)
        .map(|_| name))
}

impl Renderer {
    /// Removes the container when it is this process's own ([`Renderer::owned`]), and leaves the shared one running;
    /// `true` when one was removed.
    pub async fn release(&self) -> Result<bool> {
        if !self.owned {
            return Ok(false);
        }
        remove_container(&self.container).await
    }

    /// Runs a tool inside the renderer (poppler); paths under /work are the host's `work` folder.
    pub async fn exec(&self, arguments: &[&str], timeout: Duration) -> Result<ExecResult> {
        let mut all = vec!["exec", self.container.as_str()];
        all.extend_from_slice(arguments);
        exec("docker", &args(all), &ExecOptions::timeout(timeout)).await
    }

    /// The container's own facts: CPU architecture and poppler version.
    pub async fn facts(&self) -> Result<(String, String)> {
        let arch = self.exec(&["uname", "-m"], Duration::from_secs(30)).await?;
        if !arch.ok() {
            return Err(BenchError::failed(format!(
                "uname in the renderer: {}",
                arch.stderr
            )));
        }
        let version = self
            .exec(&["pdftoppm", "-v"], Duration::from_secs(30))
            .await?;
        let text = format!("{}{}", version.stderr, version.stdout_text());
        let poppler = text
            .split("version ")
            .nth(1)
            .and_then(|rest| {
                rest.split(|c: char| !(c.is_ascii_digit() || c == '.'))
                    .next()
            })
            .filter(|v| !v.is_empty())
            .ok_or_else(|| BenchError::failed(format!("unexpected pdftoppm -v output: {text}")))?;
        Ok((arch.stdout_text().trim().to_owned(), poppler.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_recipe_covers_every_file_and_the_platform() {
        let bench = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench");
        let recipe = recipe_hash(&bench).unwrap();
        assert_eq!(recipe.len(), 64);
        assert!(image_tag(&recipe).starts_with("slidock-bench-renderer:"));
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("docker")).unwrap();
        for name in RECIPE_FILES {
            std::fs::copy(
                bench.join("docker").join(name),
                dir.path().join("docker").join(name),
            )
            .unwrap();
        }
        assert_eq!(recipe_hash(dir.path()).unwrap(), recipe);
        std::fs::write(dir.path().join("docker/chrome-args.txt"), "--headless\n").unwrap();
        assert_ne!(recipe_hash(dir.path()).unwrap(), recipe);
    }

    #[test]
    fn own_names_are_the_shared_name_and_this_process() {
        let shared = shared_name(&"a".repeat(64), Path::new("/w1"));
        let one = own_name(&shared);
        let two = own_name(&shared);
        let prefix = format!("{shared}-p{}-", std::process::id());
        assert!(one.len() > prefix.len());
        assert!(one.starts_with(&prefix), "{one}");
        assert_ne!(one, two);
        assert_ne!(one, shared);
    }

    #[test]
    fn a_process_is_gone_only_when_no_process_has_its_pid() {
        assert!(process_alive(std::process::id()));
        // pid 1 belongs to root: signalling it fails with EPERM, and it is alive.
        assert!(process_alive(1));
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(!process_alive(pid));
        assert!(process_alive(u32::MAX));
    }

    #[test]
    fn owned_containers_are_ours_with_a_pid() {
        let listed =
            "slidock-bench-a-p1-f 41\nslidock-bench-b \nother-p2-f 42\nslidock-bench-c-p3-f x\n";
        assert_eq!(owned_containers(listed), [("slidock-bench-a-p1-f", 41)]);
    }

    #[test]
    fn a_command_error_comes_before_its_release_error() {
        let failed = || Err::<bool, _>(BenchError::failed("rm failed"));
        assert_eq!(released(Ok(1), Ok(true)).unwrap(), 1);
        assert_eq!(released(Ok(1), Ok(false)).unwrap(), 1);
        // A passing command whose container could not be removed fails.
        assert_eq!(
            released(Ok(1), failed()).unwrap_err().to_string(),
            BenchError::failed("rm failed").to_string()
        );
        // A failing command keeps its own error.
        let command = || Err::<u8, _>(BenchError::refused("no oracle"));
        assert_eq!(
            released(command(), Ok(true)).unwrap_err().to_string(),
            "refused: no oracle"
        );
        assert_eq!(
            released(command(), failed()).unwrap_err().to_string(),
            "refused: no oracle"
        );
    }

    #[test]
    fn shared_names_are_per_recipe_and_work_folder() {
        let a = shared_name(&"a".repeat(64), Path::new("/w1"));
        assert!(a.starts_with("slidock-bench-aaaaaaaaaaaa-"));
        assert_ne!(a, shared_name(&"a".repeat(64), Path::new("/w2")));
    }
}
