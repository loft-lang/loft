// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-PushFill`'s record clause — the EMISSION pins.  A counted loop appending records
//! through mint groups (under `if` arms or not) reserves its trips, opens ONE push window,
//! mints every group through it (`push_record_windowed`), finishes with the window's
//! length bump, and closes the window once after the loop; a loop that reads the vector,
//! can `break`, grows a fresh element's own field, mints to two vectors or names a view of
//! the vector keeps its header mints; `LOFT_NO_PUSH_WINDOW=1` keeps every mint on the
//! header and `LOFT_HOIST_VERIFY=1` picks the checking monomorphisation.  The guard
//! (`tests/scripts/158-record-window.loft`) says the VALUES hold on both backends; this pins
//! what is emitted.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/158-record-window.loft";

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let out = std::env::temp_dir().join(format!(
        "loft_record_window_{}_{tag}.rs",
        std::process::id()
    ));
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_PUSH_WINDOW")
        .env_remove("LOFT_NO_PUSH_FILL")
        .env_remove("LOFT_NO_RECORD_PUSH")
        .env_remove("LOFT_HOIST_VERIFY");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.output().expect("spawn loft --native-emit");
    assert!(
        out.exists(),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    let rust = std::fs::read_to_string(&out).expect("read the emitted Rust");
    let _ = std::fs::remove_file(&out);
    rust
}

/// The emitted body of one function, up to the next top-level `fn`.
fn body<'a>(rust: &'a str, name: &str) -> &'a str {
    let start = rust
        .find(&format!("\nfn {name}("))
        .unwrap_or_else(|| panic!("{name} was not emitted"));
    let rest = &rust[start + 1..];
    let end = rest[3..].find("\nfn ").map_or(rest.len(), |i| i + 3);
    &rest[..end]
}

/// `(windows opened, windowed mints, closes, header mints)` in one function.
fn forms(b: &str) -> (usize, usize, usize, usize) {
    (
        b.matches("record push window").count(),
        b.matches("push_record_windowed").count(),
        b.matches("window closed").count(),
        b.matches("push_record_hoisted").count(),
    )
}

/// (function, windows, windowed mints, closes, header mints).
const FORMS: [(&str, usize, usize, usize, usize); 11] = [
    ("n_r1", 1, 6, 1, 0), // three arms (the guarded chain copies the loop: 6 mints)
    ("n_r2", 1, 1, 1, 0), // one mint per pass
    ("n_r3", 1, 1, 1, 0), // an `if` without `else`
    ("n_r4", 0, 0, 0, 4), // DECLINES: the literal grows the fresh element's own field
    ("n_r5", 1, 1, 1, 0), // a nested-record element
    ("n_build", 2, 2, 2, 0), // two loops, a computed end and an inclusive range
    ("n_r7", 0, 0, 0, 1), // DECLINES: the body reads the vector
    ("n_r8", 0, 0, 0, 1), // DECLINES: the body can break
    ("n_r9", 1, 1, 1, 0), // a view read only after the close
    ("n_r10", 0, 0, 0, 2), // DECLINES: two vectors
    ("n_r11", 0, 0, 0, 1), // DECLINES: the view is read inside the body
];

#[test]
fn a_record_append_loop_mints_through_one_window_and_closes_it_once() {
    let rust = emit("on", &[]);
    for (name, w, m, c, h) in FORMS {
        assert_eq!(
            forms(body(&rust, name)),
            (w, m, c, h),
            "{name}: (windows, windowed mints, closes, header mints)"
        );
    }
    // The windowed mint's address is the window's next slot, never a store resolution.
    let r1 = body(&rust, "n_r1");
    assert!(
        r1.matches("windowed mint address").count() == 6 && !r1.contains("vector::rec_ptr("),
        "r1: every minted element's address comes off the window"
    );
}

#[test]
fn the_switch_keeps_every_mint_on_the_header_and_the_verify_form_checks_the_window() {
    let off = emit("off", &[("LOFT_NO_PUSH_WINDOW", "1")]);
    for (name, _, m, _, h) in FORMS {
        let got = forms(body(&off, name));
        assert_eq!(
            got.0 + got.1 + got.2,
            0,
            "LOFT_NO_PUSH_WINDOW=1: {name} opens no window"
        );
        assert_eq!(
            got.3,
            m + h,
            "LOFT_NO_PUSH_WINDOW=1: {name} mints every group through the header"
        );
    }
    let verify = emit("verify", &[("LOFT_HOIST_VERIFY", "1")]);
    assert!(
        body(&verify, "n_r2").contains("push_record_windowed::<false, true>"),
        "LOFT_HOIST_VERIFY=1 picks the checking monomorphisation"
    );
}
