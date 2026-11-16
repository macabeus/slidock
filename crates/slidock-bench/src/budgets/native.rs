//! The native leg: the CLI compiles each pair to a zip in a helper process of its own (`slidock-bench
//! measure-child`), whose children are that compile and the samplers that watch its memory, so the helper's
//! `getrusage(RUSAGE_CHILDREN)` is the compile's: its peak resident set, and its CPU time (user and system), which
//! the machine's load moves far less than the wall clock.

use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use slidock_bench_schema::stamp::CompileCost;

use crate::compile::RSS_LIMIT_BYTES;
use crate::error::{BenchError, Result};
use crate::exec::{ExecOptions, exec};

/// What `measure-child` prints: how the child ended, its wall-clock and CPU time, and the largest resident set of
/// the helper's children.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChildCost {
    pub code: Option<i32>,
    pub timed_out: bool,
    pub out_of_memory: bool,
    pub ms: u64,
    /// User and system time.
    pub cpu_ms: u64,
    pub peak_bytes: u64,
}

/// The children's usage this process has waited for: the largest resident set in bytes (`ru_maxrss` is in bytes on
/// macOS and in KiB elsewhere) and the CPU time in milliseconds.
fn children_usage() -> Result<(u64, u64)> {
    let usage = nix::sys::resource::getrusage(nix::sys::resource::UsageWho::RUSAGE_CHILDREN)
        .map_err(|error| BenchError::failed(format!("getrusage: {error}")))?;
    let max_rss = u64::try_from(usage.max_rss()).unwrap_or(0);
    let peak = if cfg!(target_os = "macos") {
        max_rss
    } else {
        max_rss.saturating_mul(1024)
    };
    let millis = |t: nix::sys::time::TimeVal| {
        u64::try_from(t.tv_sec())
            .unwrap_or(0)
            .saturating_mul(1000)
            .saturating_add(u64::try_from(t.tv_usec()).unwrap_or(0) / 1000)
    };
    Ok((
        peak,
        millis(usage.user_time()).saturating_add(millis(usage.system_time())),
    ))
}

/// Runs `program` to its end within `timeout` and the compiler's memory limit, and measures it. Meant to run in a
/// process of its own (`measure-child`): an earlier child of the same process would count in the peak.
pub async fn measure_child(
    program: &OsString,
    arguments: &[OsString],
    timeout: Duration,
) -> Result<ChildCost> {
    let options = ExecOptions {
        timeout: Some(timeout),
        quiet: true,
        rss_limit_bytes: Some(RSS_LIMIT_BYTES),
        ..ExecOptions::default()
    };
    let started = tokio::time::Instant::now();
    let result = exec(program.clone(), arguments, &options).await?;
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let (peak_bytes, cpu_ms) = children_usage()?;
    Ok(ChildCost {
        code: result.code,
        timed_out: result.timed_out,
        out_of_memory: result.out_of_memory,
        ms,
        cpu_ms,
        peak_bytes,
    })
}

/// The CLI's arguments for a pair: the deck as a user compiles it (embedded fonts and the bundled substitutes), with
/// no network request.
pub fn compile_arguments(input: &Path, zip: &Path) -> Vec<OsString> {
    vec![
        "compile".into(),
        input.as_os_str().to_owned(),
        "-o".into(),
        zip.as_os_str().to_owned(),
        "--offline".into(),
    ]
}

/// Compiles one pair through `measure-child`. An exit other than 0 (clean) or 1 (errors or failed containers in a
/// deck that still printed), a timeout or the memory limit fails: its cost is not the deck's.
pub async fn measure_compile(
    bench: &Path,
    binary: &Path,
    input: &Path,
    zip: &Path,
    timeout: Duration,
) -> Result<CompileCost> {
    let mut arguments: Vec<OsString> = vec![
        "measure-child".into(),
        "--timeout-secs".into(),
        timeout.as_secs().to_string().into(),
        "--".into(),
        binary.as_os_str().to_owned(),
    ];
    arguments.extend(compile_arguments(input, zip));
    let options = ExecOptions::timeout(timeout + Duration::from_secs(60));
    let result = exec(bench.as_os_str().to_owned(), &arguments, &options).await?;
    if !result.ok() {
        return Err(BenchError::failed(format!(
            "measure-child {} exited {:?}: {}",
            input.display(),
            result.code,
            result.stderr.trim()
        )));
    }
    let cost: ChildCost = serde_json::from_slice(&result.stdout).map_err(|error| {
        BenchError::failed(format!("measure-child {}: {error}", input.display()))
    })?;
    accepted(&cost).map_err(|reason| BenchError::failed(format!("{}: {reason}", input.display())))
}

/// A child's cost, when it compiled: its CPU time and its peak.
fn accepted(cost: &ChildCost) -> std::result::Result<CompileCost, String> {
    if cost.timed_out {
        return Err("the compile timed out".into());
    }
    if cost.out_of_memory {
        return Err("the compile passed the memory limit".into());
    }
    match cost.code {
        Some(0 | 1) => Ok(CompileCost {
            ms: cost.cpu_ms,
            peak_bytes: cost.peak_bytes,
        }),
        code => Err(format!("the compile exited {code:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_peak_is_the_childs_own() {
        // A child that holds a 64 MiB string, then exits. This test process has other children, so only the lower
        // bound is the child's own; `measure-child` runs in a process with no other.
        let script = "my $x = 'a' x (64 * 1024 * 1024); exit(length($x) > 0 ? 0 : 1);";
        let cost = measure_child(
            &"perl".into(),
            &["-e".into(), script.into()],
            Duration::from_secs(60),
        )
        .await
        .unwrap();
        assert_eq!(cost.code, Some(0));
        assert!(!cost.timed_out && !cost.out_of_memory);
        assert!(cost.peak_bytes > 60 * 1024 * 1024, "{}", cost.peak_bytes);
        assert!(cost.peak_bytes < 1024 * 1024 * 1024, "{}", cost.peak_bytes);
    }

    #[tokio::test]
    async fn the_cpu_time_leaves_out_the_time_a_child_waits() {
        // A child that sleeps half a second: wall clock, but no CPU time.
        let cost = measure_child(&"sleep".into(), &["0.5".into()], Duration::from_secs(30))
            .await
            .unwrap();
        assert_eq!(cost.code, Some(0));
        assert!(cost.ms >= 500, "{}", cost.ms);
        // This test process's other children count too, but none of them spins for 400 ms.
        assert!(cost.cpu_ms < cost.ms, "{} < {}", cost.cpu_ms, cost.ms);
    }

    #[tokio::test]
    async fn a_child_past_its_timeout_is_killed_and_reported() {
        let cost = measure_child(&"sleep".into(), &["30".into()], Duration::from_millis(300))
            .await
            .unwrap();
        assert!(cost.timed_out);
        assert!(cost.ms < 10_000, "{}", cost.ms);
        assert!(accepted(&cost).is_err());
    }

    #[test]
    fn only_a_compile_that_printed_counts() {
        let cost = |code, timed_out, out_of_memory| ChildCost {
            code,
            timed_out,
            out_of_memory,
            ms: 70,
            cpu_ms: 7,
            peak_bytes: 9,
        };
        // The CPU time, not the wall clock.
        assert_eq!(
            accepted(&cost(Some(0), false, false)),
            Ok(CompileCost {
                ms: 7,
                peak_bytes: 9
            })
        );
        assert!(accepted(&cost(Some(1), false, false)).is_ok());
        assert!(accepted(&cost(Some(2), false, false)).is_err());
        assert!(accepted(&cost(None, false, false)).is_err());
        assert!(accepted(&cost(Some(0), true, false)).is_err());
        assert!(accepted(&cost(Some(0), false, true)).is_err());
    }

    #[test]
    fn the_cli_compiles_offline_to_a_zip() {
        let arguments = compile_arguments(Path::new("/c/deck.key"), Path::new("/w/deck.zip"));
        assert_eq!(
            arguments,
            ["compile", "/c/deck.key", "-o", "/w/deck.zip", "--offline"].map(OsString::from)
        );
    }
}
