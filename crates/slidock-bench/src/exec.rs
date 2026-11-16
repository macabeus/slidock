//! Child processes with a wall-clock limit and an output limit. Every external tool the bench runs (the compiler,
//! docker, osascript, git, poppler through docker) goes through here, so every stage ends in bounded time and memory.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use crate::error::{BenchError, Result};

/// Captured bytes per stream by default.
pub const MAX_OUTPUT_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub struct ExecOptions {
    pub cwd: Option<PathBuf>,
    /// Variables set on top of this process's environment.
    pub env: BTreeMap<String, String>,
    /// Killed (SIGKILL) when it runs longer.
    pub timeout: Option<Duration>,
    /// Bytes written to stdin, which is closed after them.
    pub input: Option<Vec<u8>>,
    /// Killed when a captured stream grows past this many bytes; [`MAX_OUTPUT_BYTES`] by default.
    pub max_output_bytes: Option<usize>,
    /// Discards stdout and stderr instead of capturing them.
    pub quiet: bool,
    /// Killed when the child's resident set grows past this many bytes (sampled once a second).
    pub rss_limit_bytes: Option<u64>,
}

impl ExecOptions {
    pub fn timeout(duration: Duration) -> ExecOptions {
        ExecOptions {
            timeout: Some(duration),
            ..ExecOptions::default()
        }
    }
}

#[derive(Clone, Debug)]
pub struct ExecResult {
    /// The exit code; `None` when a signal ended the child, or when it was killed.
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: String,
    pub timed_out: bool,
    /// A captured stream passed the limit: the child was killed and the stream cut there.
    pub overflowed: bool,
    /// The resident set passed `rss_limit_bytes`: the child was killed.
    pub out_of_memory: bool,
}

impl ExecResult {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }

    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

/// Reads a stream up to `limit` bytes; `true` when it had more.
async fn capture<R: AsyncRead + Unpin>(stream: Option<R>, limit: usize) -> (Vec<u8>, bool) {
    let Some(mut stream) = stream else {
        return (Vec::new(), false);
    };
    let mut out = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return (out, false),
            Ok(n) => {
                let room = limit.saturating_sub(out.len());
                let take = n.min(room);
                out.extend_from_slice(chunk.get(..take).unwrap_or_default());
                if n > room {
                    return (out, true);
                }
            }
        }
    }
}

/// The resident set size of a process in bytes, from `ps`.
async fn resident_bytes(pid: u32) -> Option<u64> {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    let kib: u64 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .ok()?;
    Some(kib * 1024)
}

/// Waits until the resident set passes `limit`, sampling once a second; never returns without one.
async fn memory_watch(pid: Option<u32>, limit: Option<u64>) {
    let (Some(pid), Some(limit)) = (pid, limit) else {
        return std::future::pending().await;
    };
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if resident_bytes(pid).await.is_some_and(|bytes| bytes > limit) {
            return;
        }
    }
}

async fn wait_timeout(timeout: Option<Duration>) {
    match timeout {
        Some(duration) => tokio::time::sleep(duration).await,
        None => std::future::pending().await,
    }
}

/// Runs a command to its end, or kills it at the first limit it passes.
pub async fn exec(
    program: impl Into<OsString>,
    args: &[OsString],
    options: &ExecOptions,
) -> Result<ExecResult> {
    let program: OsString = program.into();
    let mut command = Command::new(&program);
    command
        .args(args)
        .envs(&options.env)
        .stdin(if options.input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .kill_on_drop(true);
    if options.quiet {
        command.stdout(Stdio::null()).stderr(Stdio::null());
    } else {
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
    }
    if let Some(cwd) = &options.cwd {
        command.current_dir(cwd);
    }
    let mut child = command
        .spawn()
        .map_err(|error| BenchError::failed(format!("{}: {error}", program.to_string_lossy())))?;
    if let (Some(input), Some(mut stdin)) = (&options.input, child.stdin.take()) {
        let input = input.clone();
        tokio::spawn(async move {
            let _ = stdin.write_all(&input).await;
        });
    }
    let limit = options.max_output_bytes.unwrap_or(MAX_OUTPUT_BYTES);
    let stdout = tokio::spawn(capture(child.stdout.take(), limit));
    let stderr = tokio::spawn(capture(child.stderr.take(), limit));
    let pid = child.id();

    let mut timed_out = false;
    let mut out_of_memory = false;
    let status = tokio::select! {
        status = child.wait() => status.ok(),
        () = wait_timeout(options.timeout) => {
            timed_out = true;
            None
        }
        () = memory_watch(pid, options.rss_limit_bytes) => {
            out_of_memory = true;
            None
        }
    };
    if status.is_none() {
        let _ = child.kill().await;
    }
    let (stdout, out_overflow) = stdout.await.unwrap_or_default();
    let (stderr, err_overflow) = stderr.await.unwrap_or_default();
    let overflowed = out_overflow || err_overflow;
    if overflowed && status.is_some() {
        let _ = child.kill().await;
    }
    Ok(ExecResult {
        code: if overflowed {
            None
        } else {
            status.and_then(|s| s.code())
        },
        stdout,
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        timed_out,
        overflowed,
        out_of_memory,
    })
}

/// `args` as the `OsString`s [`exec`] takes.
pub fn args<I, S>(items: I) -> Vec<OsString>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    items.into_iter().map(Into::into).collect()
}

/// Runs a command that must succeed and returns its stdout as text.
pub async fn exec_text(
    program: &str,
    arguments: &[OsString],
    options: &ExecOptions,
) -> Result<String> {
    let result = exec(program, arguments, options).await?;
    if !result.ok() {
        let why = if result.timed_out {
            "timed out".to_owned()
        } else if result.overflowed {
            "wrote too much output".to_owned()
        } else {
            format!("exited {:?}", result.code)
        };
        let line: Vec<String> = arguments
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let stderr: String = result.stderr.trim().chars().take(2000).collect();
        return Err(BenchError::failed(format!(
            "{program} {} {why}: {stderr}",
            line.join(" ")
        )));
    }
    Ok(result.stdout_text())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn captures_and_reports_exit_codes() {
        let result = exec(
            "sh",
            &args(["-c", "printf hi; printf err >&2; exit 3"]),
            &ExecOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(result.code, Some(3));
        assert_eq!(result.stdout, b"hi");
        assert_eq!(result.stderr, "err");
        assert!(!result.timed_out && !result.overflowed);
    }

    #[tokio::test]
    async fn kills_at_the_timeout() {
        let options = ExecOptions::timeout(Duration::from_millis(200));
        let result = exec("sleep", &args(["5"]), &options).await.unwrap();
        assert!(result.timed_out);
        assert_eq!(result.code, None);
    }

    #[tokio::test]
    async fn cuts_and_kills_at_the_output_limit() {
        let options = ExecOptions {
            max_output_bytes: Some(10),
            ..ExecOptions::default()
        };
        let result = exec("sh", &args(["-c", "yes | head -c 100000"]), &options)
            .await
            .unwrap();
        assert!(result.overflowed);
        assert_eq!(result.code, None);
        assert_eq!(result.stdout.len(), 10);
    }

    #[tokio::test]
    async fn writes_stdin() {
        let options = ExecOptions {
            input: Some(b"abc".to_vec()),
            ..ExecOptions::default()
        };
        let result = exec("cat", &[], &options).await.unwrap();
        assert_eq!(result.stdout, b"abc");
    }

    #[tokio::test]
    async fn exec_text_fails_on_a_nonzero_exit() {
        let error = exec_text(
            "sh",
            &args(["-c", "echo no >&2; exit 1"]),
            &ExecOptions::default(),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("no"), "{error}");
    }
}
