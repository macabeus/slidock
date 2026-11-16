//! What the command prints, and how much: `-q` prints errors only, the default adds warnings, losses, declines and a
//! summary of the rest, `-v` prints everything. A closed stdout or stderr (`slidock … | head`) ends that stream's
//! output quietly and the command carries on: a broken pipe is not an error.

use std::fmt;
use std::io::{self, Write};
use std::sync::atomic::{AtomicU8, Ordering};

/// How much the command prints on stderr.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verbosity {
    /// Errors only.
    Quiet,
    /// Errors, warnings, losses and declines, and one summary line.
    Normal,
    /// Everything, info diagnostics and the font sources included.
    Verbose,
}

static VERBOSITY: AtomicU8 = AtomicU8::new(1);

pub fn set_verbosity(verbosity: Verbosity) {
    VERBOSITY.store(verbosity as u8, Ordering::Relaxed);
}

pub fn verbosity() -> Verbosity {
    match VERBOSITY.load(Ordering::Relaxed) {
        0 => Verbosity::Quiet,
        1 => Verbosity::Normal,
        _ => Verbosity::Verbose,
    }
}

/// Prints one line on stderr; a write that fails (a closed stderr) is dropped.
pub fn line(text: impl fmt::Display) {
    let _ = writeln!(io::stderr().lock(), "{text}");
}

/// Prints one line on stderr at `-v` only.
pub fn verbose(text: impl fmt::Display) {
    if verbosity() == Verbosity::Verbose {
        line(text);
    }
}

/// Prints one line on stderr unless `-q`.
pub fn normal(text: impl fmt::Display) {
    if verbosity() >= Verbosity::Normal {
        line(text);
    }
}

/// Writes `bytes` to stdout; a reader that closed the pipe ends the output, which is not an error.
pub fn stdout(bytes: &[u8]) -> io::Result<()> {
    let mut out = io::stdout().lock();
    match out.write_all(bytes).and_then(|()| out.flush()) {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}
