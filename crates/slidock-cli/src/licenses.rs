//! `slidock licenses`: prints slidock's licence and notices and those of everything the binary is built from
//! (`THIRD-PARTY-NOTICES.txt`, written by `cargo xtask notices`).

use std::io::Write as _;

use crate::Exit;

/// The notices, a copy of the repository's `THIRD-PARTY-NOTICES.txt` (`cargo xtask notices --check` keeps the two
/// equal).
pub const NOTICES: &str = include_str!("../THIRD-PARTY-NOTICES.txt");

pub fn run() -> Exit {
    match std::io::stdout().lock().write_all(NOTICES.as_bytes()) {
        Ok(()) => Exit::Ok,
        // A closed pipe (`slidock licenses | head`) is not a failure.
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Exit::Ok,
        Err(error) => {
            eprintln!("error: {error}");
            Exit::Failed
        }
    }
}
