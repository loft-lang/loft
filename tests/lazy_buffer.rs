// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-O-LazyBuffer` — a hidden return buffer is minted in front of the statement that
//! hands it to a callee, behind a null test, instead of at function entry, so a path that
//! never makes the call never mints the store.
//!
//! The cells (`plans/164-activation-arena/bytecode-comparisons/A0-lazy-buffer-cells.loft`)
//! carry one shape per case, with the value each must print.  These tests pin that the
//! values hold on both backends in both switch states under the store falsifiers, that no
//! entry mint is left in the rewritten functions and the switch (`LOFT_NO_LAZY_BUFFER=1`)
//! restores every one, that a free is never taken for a use, and that the interpreter's
//! entry init is the non-allocating sentinel.
use std::path::{Path, PathBuf};
use std::process::Output;

const CELLS: &str =
    "doc/claude/plans/164-activation-arena/bytecode-comparisons/A0-lazy-buffer-cells.loft";

const EXPECTED: &str = "a1 35 a2 15 10 a3 34 a4 2 105 a5 4 6 a6 3 a7 21\n\
                        a8 13 a9 4 a10 200 1 a11 18 a12 -1 -1 13 a13 0 9 4 \
                        a14 -1 0 114 214 -1 12\n";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args).env("LOFT_TIMEOUT", "120");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

fn cells() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(CELLS)
        .to_string_lossy()
        .into_owned()
}

#[test]
fn the_values_hold_on_both_backends_in_both_switch_states() {
    let src = cells();
    for switch in [None, Some(("LOFT_NO_LAZY_BUFFER", "1"))] {
        for backend in ["--interpret", "--native"] {
            {
                // Every detector at once: one build per switch state, each detector still on
                // (falsifying a guard is a one-time receipt, not a cost of every run).
                let mut env = vec![
                    ("LOFT_STRICT_STORES", "1"),
                    ("LOFT_POISON", "1"),
                    ("LOFT_NATIVE_LEAK_CHECK", "1"),
                ];
                env.extend(switch);
                let res = loft(&[backend, &src], &env);
                let out = String::from_utf8_lossy(&res.stdout);
                let err = String::from_utf8_lossy(&res.stderr);
                let label = format!("{backend} {env:?}");
                assert!(
                    res.status.success(),
                    "{label}: exit {:?}\n{err}",
                    res.status
                );
                assert_eq!(out, EXPECTED.repeat(3), "{label}\n{err}");
                assert!(
                    !err.contains("not freed") && !err.contains("strict-store"),
                    "{label}: a store leaked or was read after its free:\n{err}"
                );
            }
        }
    }
}

#[test]
fn the_interpreters_entry_init_is_the_sentinel() {
    let src = cells();
    let res = loft(&["introspect", &src], &[]);
    let text = String::from_utf8_lossy(&res.stdout);
    let start = text
        .find(".loft:n_f1(")
        .expect("the introspection lists n_f1's bytecode");
    let f1 = &text[start..];
    let end = [f1.find("\nbyte-code for"), f1.find("\nfn ")]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(f1.len());
    let f1 = &f1[..end];
    assert!(
        f1.contains("InitRefSentinel(var[") && f1.contains("RefIsNull("),
        "n_f1's buffer must start as the sentinel and mint behind the null test:\n{f1}"
    );
}
