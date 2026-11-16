//! The host's clock, which the CLI injects for `--timings`: the compiler itself reads none.
#![allow(
    clippy::disallowed_types,
    reason = "the CLI is the host that owns the clock; it never runs in wasm"
)]

use std::time::Instant;

/// Milliseconds since the command started.
pub struct Wall(Instant);

impl Wall {
    pub fn start() -> Self {
        Wall(Instant::now())
    }
}

impl slidock::Clock for Wall {
    fn now_ms(&self) -> f64 {
        self.0.elapsed().as_secs_f64() * 1000.0
    }
}
