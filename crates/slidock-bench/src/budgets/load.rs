//! The machine's 1-minute load average, which decides whether `budgets` may judge CPU times. Other processes that
//! hold cores stretch a compile's CPU time (lower clocks, shared caches): with audiomxd and configd holding a core,
//! every deck measured 7 to 17 % more CPU time than on the same code at a quieter moment. A load above the core count
//! makes that noise larger than any headroom, so times measured then are not judged, and not recorded.

use std::fmt;

use crate::error::{BenchError, Result};
use crate::exec::{ExecOptions, args, exec_text};

/// A 1-minute load average, in hundredths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Load(u64);

impl Load {
    pub fn from_hundredths(hundredths: u64) -> Load {
        Load(hundredths)
    }

    /// Whether a machine of `cores` cores at this load is quiet enough to judge times: the load at most the cores.
    pub fn quiet_on(self, cores: u64) -> bool {
        self.0 <= cores.saturating_mul(100)
    }
}

impl fmt::Display for Load {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{:02}", self.0 / 100, self.0 % 100)
    }
}

/// The logical cores the scheduler may run on.
pub fn cores() -> u64 {
    std::thread::available_parallelism().map_or(1, |n| u64::try_from(n.get()).unwrap_or(u64::MAX))
}

/// A load figure: digits, optionally a point and one or two more. Anything else is refused.
fn parse_figure(text: &str) -> Option<Load> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole)
        || whole.len() > 9
        || (text.contains('.') && !digits(fraction))
        || fraction.len() > 2
    {
        return None;
    }
    let whole: u64 = whole.parse().ok()?;
    let fraction: u64 = match fraction.len() {
        0 => 0,
        1 => fraction.parse::<u64>().ok()? * 10,
        _ => fraction.parse().ok()?,
    };
    Some(Load(whole * 100 + fraction))
}

/// The 1-minute figure of macOS's `sysctl -n vm.loadavg` (`{ 1.23 4.56 7.89 }`).
pub fn parse_sysctl(text: &str) -> Option<Load> {
    let inner = text.trim().strip_prefix('{')?.strip_suffix('}')?;
    let figures: Vec<&str> = inner.split_whitespace().collect();
    match figures.as_slice() {
        [one, five, fifteen] => {
            parse_figure(five)?;
            parse_figure(fifteen)?;
            parse_figure(one)
        }
        _ => None,
    }
}

/// The 1-minute figure of Linux's `/proc/loadavg` (`1.23 4.56 7.89 2/345 6789`).
pub fn parse_proc(text: &str) -> Option<Load> {
    let fields: Vec<&str> = text.split_whitespace().collect();
    match fields.as_slice() {
        [one, five, fifteen, _tasks, _last] => {
            parse_figure(five)?;
            parse_figure(fifteen)?;
            parse_figure(one)
        }
        _ => None,
    }
}

/// The machine's 1-minute load now. A reading that does not parse is refused, never taken as quiet.
pub async fn read() -> Result<Load> {
    if cfg!(target_os = "linux") {
        let path = std::path::Path::new("/proc/loadavg");
        let text = std::fs::read_to_string(path).map_err(BenchError::io(path))?;
        return parse_proc(&text).ok_or_else(|| {
            BenchError::refused(format!(
                "/proc/loadavg: {:?} is not a load average",
                text.trim()
            ))
        });
    }
    let text = exec_text(
        "sysctl",
        &args(["-n", "vm.loadavg"]),
        &ExecOptions::timeout(std::time::Duration::from_secs(10)),
    )
    .await?;
    parse_sysctl(&text).ok_or_else(|| {
        BenchError::refused(format!(
            "sysctl vm.loadavg: {:?} is not a load average",
            text.trim()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_one_minute_figure_is_read_from_sysctl_and_procfs() {
        assert_eq!(parse_sysctl("{ 18.40 19.82 14.13 }\n"), Some(Load(1840)));
        assert_eq!(parse_sysctl("{ 0.05 1 2.5 }"), Some(Load(5)));
        assert_eq!(parse_proc("2.31 1.07 0.99 3/412 81234\n"), Some(Load(231)));
        assert_eq!(parse_proc("10 9.5 9.25 1/1 1"), Some(Load(1000)));
    }

    #[test]
    fn a_reading_that_is_not_a_load_average_is_refused() {
        for text in [
            "",
            "{ }",
            "{ 1.23 4.56 }",
            "{ 1.23 4.56 7.89 1.00 }",
            "1.23 4.56 7.89",
            "{ -1.23 4.56 7.89 }",
            "{ 1.234 4.56 7.89 }",
            "{ 1. 4.56 7.89 }",
            "{ .5 4.56 7.89 }",
            "{ 1,23 4.56 7.89 }",
            "{ 1.23 x 7.89 }",
            "{ 9999999999.00 4.56 7.89 }",
            "{ nan 4.56 7.89 }",
        ] {
            assert_eq!(parse_sysctl(text), None, "{text:?}");
        }
        for text in [
            "",
            "1.23 4.56 7.89",
            "1.23 4.56 7.89 2/345",
            "a 4.56 7.89 2/345 6789",
        ] {
            assert_eq!(parse_proc(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_machine_is_quiet_up_to_a_load_of_its_core_count() {
        assert!(Load(0).quiet_on(10));
        assert!(Load(1000).quiet_on(10));
        assert!(!Load(1001).quiet_on(10));
        assert!(!Load(1840).quiet_on(10));
        assert!(!Load(1).quiet_on(0));
        assert_eq!(Load(1840).to_string(), "18.40");
        assert_eq!(Load(5).to_string(), "0.05");
        assert!(cores() >= 1);
    }

    #[tokio::test]
    async fn this_machine_has_a_load_average() {
        let load = read().await.unwrap();
        assert!(load < Load(100_000), "{load}");
    }
}
