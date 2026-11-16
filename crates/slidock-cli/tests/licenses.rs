//! `slidock licenses` prints the notices: slidock's licence and NOTICE, and every crate, package and font licence
//! of what the binary is built from, as the repository's `THIRD-PARTY-NOTICES.txt` holds them.
#![allow(clippy::unwrap_used, clippy::panic, reason = "test code")]

use std::process::Command;

#[test]
fn prints_the_repositorys_notices() {
    let output = Command::new(env!("CARGO_BIN_EXE_slidock"))
        .arg("licenses")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let printed = String::from_utf8(output.stdout).unwrap();
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../THIRD-PARTY-NOTICES.txt");
    assert!(
        printed == std::fs::read_to_string(&root).unwrap(),
        "stale: run cargo xtask notices"
    );
    for needle in [
        "Copyright (c) 2026 Bruno Macabeus",
        "Copyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.",
        "© Ecma International",
        "Copyright (c) Tailwind Labs, Inc.",
        "Mozilla Public License Version 2.0",
        "cssparser 0.38.0",
        "webpki-roots",
        "Bitstream Vera Fonts Copyright",
    ] {
        assert!(printed.contains(needle), "{needle}");
    }
    assert!(
        Command::new(env!("CARGO_BIN_EXE_slidock"))
            .args(["licenses", "--extra"])
            .output()
            .unwrap()
            .status
            .code()
            == Some(2)
    );
}
