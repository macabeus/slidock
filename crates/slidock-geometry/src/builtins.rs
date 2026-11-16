//! The built-in guide names, in the order [`crate::formula::builtin_values`] computes them. `build.rs` includes
//! this file to resolve the preset table's names.

pub const BUILTIN_NAMES: [&str; 41] = [
    "w", "h", "l", "t", "r", "b", "hc", "vc", "ss", "ls", "cd2", "cd4", "cd8", "3cd4", "3cd8",
    "5cd8", "7cd8", "wd2", "wd3", "wd4", "wd5", "wd6", "wd8", "wd10", "wd12", "wd32", "hd2", "hd3",
    "hd4", "hd5", "hd6", "hd8", "hd10", "hd12", "hd32", "ssd2", "ssd4", "ssd6", "ssd8", "ssd16",
    "ssd32",
];

/// The divisors of `wdN` and `hdN`, then of `ssdN`.
pub const WH_DIVISORS: [u8; 9] = [2, 3, 4, 5, 6, 8, 10, 12, 32];
pub const SS_DIVISORS: [u8; 6] = [2, 4, 6, 8, 16, 32];
