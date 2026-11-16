//! Cooperative time limits. The compiler reads no clock of its own (`std::time::Instant` panics on wasm): the host
//! injects a [`Clock`], and code checks a [`Deadline`] between stages, parts and slides. Without a clock nothing
//! times out; the browser host stops a run by terminating its worker.

use thiserror::Error;

/// The host's clock, in milliseconds from any fixed point.
pub trait Clock {
    fn now_ms(&self) -> f64;
}

impl<F: Fn() -> f64> Clock for F {
    fn now_ms(&self) -> f64 {
        self()
    }
}

/// A stage ran past its limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[error("stopped: over its time limit")]
pub struct Stopped;

/// A limit on a stage's time, from when it started on the host's clock.
#[derive(Clone, Copy)]
pub struct Deadline<'a> {
    clock: Option<&'a dyn Clock>,
    started: f64,
    limit_ms: u64,
}

impl std::fmt::Debug for Deadline<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Deadline")
            .field("clocked", &self.clock.is_some())
            .field("started", &self.started)
            .field("limit_ms", &self.limit_ms)
            .finish()
    }
}

impl<'a> Deadline<'a> {
    /// A deadline `limit_ms` from now on `clock`; one that never passes without a clock.
    pub fn start(clock: Option<&'a dyn Clock>, limit_ms: u64) -> Self {
        let started = clock.map_or(0.0, Clock::now_ms);
        Deadline {
            clock,
            started,
            limit_ms,
        }
    }

    /// A deadline that never passes.
    pub fn never() -> Self {
        Deadline::start(None, u64::MAX)
    }

    pub fn limit_ms(&self) -> u64 {
        self.limit_ms
    }

    /// Whether the limit has passed.
    pub fn expired(&self) -> bool {
        self.clock
            .is_some_and(|clock| clock.now_ms() - self.started >= self.limit_ms as f64)
    }

    /// Whether this passes before `other`; one with a clock passes before one without.
    pub fn sooner_than(&self, other: &Deadline<'_>) -> bool {
        let end = |deadline: &Deadline<'_>| {
            deadline
                .clock
                .map(|_| deadline.started + deadline.limit_ms as f64)
        };
        match (end(self), end(other)) {
            (Some(mine), Some(theirs)) => mine < theirs,
            (Some(_), None) => true,
            (None, _) => false,
        }
    }

    /// Whichever of the two passes first (this one when they pass together).
    pub fn earlier(self, other: Deadline<'a>) -> Deadline<'a> {
        if other.sooner_than(&self) {
            other
        } else {
            self
        }
    }

    /// `Err(Stopped)` once the limit has passed.
    pub fn check(&self) -> Result<(), Stopped> {
        if self.expired() { Err(Stopped) } else { Ok(()) }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn passes_at_its_limit_on_the_injected_clock() {
        let now = Cell::new(100.0);
        let clock = || now.get();
        let deadline = Deadline::start(Some(&clock), 5);
        assert_eq!(deadline.check(), Ok(()));
        now.set(104.9);
        assert!(!deadline.expired());
        now.set(105.0);
        assert_eq!(deadline.check(), Err(Stopped));
    }

    #[test]
    fn the_earlier_of_two_is_the_one_that_passes_first() {
        let now = Cell::new(100.0);
        let clock = || now.get();
        let long = Deadline::start(Some(&clock), 50);
        now.set(120.0);
        let short = Deadline::start(Some(&clock), 10);
        assert_eq!(long.earlier(short).limit_ms(), 10);
        assert_eq!(short.earlier(long).limit_ms(), 10);
        let late = Deadline::start(Some(&clock), 40);
        assert_eq!(long.earlier(late).limit_ms(), 50);
        assert_eq!(Deadline::never().earlier(late).limit_ms(), 40);
        assert_eq!(late.earlier(Deadline::never()).limit_ms(), 40);
        now.set(130.0);
        assert!(long.earlier(short).expired());
        assert!(!late.expired());
    }

    #[test]
    fn never_passes_without_a_clock() {
        assert!(!Deadline::start(None, 0).expired());
        assert_eq!(Deadline::never().check(), Ok(()));
    }
}
