// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN164 C5 (`@FR-O-ViewField`, `@FR-R-ValueRecord`) — a returned record's heap field as a
//! VIEW LEAF: the vector is delivered as a REFERENCE to the place it already lives in, so the
//! record's store and the deep copy into it are gone.
//!
//! `tests/scripts/164-view-field.loft` carries the cells and says the VALUES do not move; the
//! corpus runner runs it with the unit off, which is the record form the values are hand-
//! computed from.  This file carries the structural half, because the values cannot falsify an
//! admission: the copy the record form makes is correct, so every cell passes either way.  What
//! changes is the emitted signature and what the site reads, and that is what is pinned here —
//! together with every DECLINE, so one that quietly stops declining shows up as a failure here
//! rather than as a wrong answer in a consumer.
use std::path::{Path, PathBuf};
use std::process::Command;

const GUARD: &str = "tests/scripts/164-view-field.loft";
/// @PLN164 E-1 — the forward: a function that keeps its record returning an admitted
/// callee's answer writes the tuple into its own buffer.
const FORWARD: &str = "tests/scripts/164-forward-tuple.loft";

fn loft() -> Command {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_VIEW_FIELD")
        .env_remove("LOFT_NO_FORWARD_TUPLE");
    cmd
}

/// The cells' values, on both backends and under the store falsifiers: a view leaf is never
/// freed and never read after its place moved.
#[test]
fn the_cells_hold_on_both_backends_under_every_falsifier() {
    let strict: &[(&str, &str)] = &[
        ("LOFT_POISON", "1"),
        ("LOFT_POISON_CLAIM", "1"),
        ("LOFT_STRICT_STORES", "1"),
        ("LOFT_NATIVE_LEAK_CHECK", "1"),
    ];
    for (file, done) in [
        (GUARD, "164-view-field: every cell holds"),
        (FORWARD, "164-forward-tuple: every cell holds"),
    ] {
        // The interpreter and the two `--lean` runs are CLOSED programs, where the IR's tuple
        // carries the view; a plain `--native` is open and runs the writers' own form.
        for backend in [
            &["--interpret"][..],
            &["--native"],
            &["--native", "--lean"],
            &["--native-c", "--lean"],
        ] {
            let mut cmd = loft();
            cmd.args(backend)
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(file));
            for (k, v) in strict {
                cmd.env(k, v);
            }
            let out = cmd.output().expect("spawn loft");
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(
                out.status.success() && text.contains(done),
                "{file} on {backend:?} did not hold under the falsifiers:\n{text}"
            );
            assert!(
                !text.contains("stores not freed"),
                "{file} on {backend:?} leaked:\n{text}"
            );
        }
    }
}
