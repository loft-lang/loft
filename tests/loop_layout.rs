// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! The interpreter's two loop layouts, PINNED on their cells: a counted loop's variable shares
//! its range index's slot (`slot_alias`, `tests/scripts/a-counted-loop-variable-shares-its-index-slot.loft`)
//! and a loop whose first statement carries its exit test runs that test at its bottom
//! (`gen_rotated_loop`, `tests/scripts/a-loop-tests-at-its-bottom.loft`).  The cells' values
//! pass with either layout, so only this pin sees one drift: which loops share a slot, read off
//! `LOFT_TRACE_LOOP_VAR_ALIAS`, and how many jumps each function's bytecode carries with the
//! rotation on and off.  And because the default run never reaches the switched-off paths
//! (every counted loop rotates, so the top-tested alias path runs only under
//! `LOFT_NO_LOOP_ROTATE=1`), both cell files run under all four switch states here.
use std::path::{Path, PathBuf};

const ALIAS_CELLS: &str = "tests/scripts/a-counted-loop-variable-shares-its-index-slot.loft";
const ROTATE_CELLS: &str = "tests/scripts/a-loop-tests-at-its-bottom.loft";

/// `(function, loop variable, shares its index's slot)` for every counted loop of the cells.
const ALIASES: &[(&str, &str, bool)] = &[
    ("n_inner", "k", true),
    ("n_squares", "i", true),
    ("n_test_plain", "i", true),
    ("n_test_inclusive", "i", true),
    ("n_test_variable_start", "i", true),
    ("n_test_written_in_body", "j", false),     // `j = j * 2`
    ("n_test_reference_parameter", "i", false), // `bump(i)` takes `&integer`
    ("n_test_link", "k", false),                // `l = &k`
    ("n_test_nested", "i", true),
    ("n_test_nested", "j", true),
    ("n_test_break_continue", "i", true),
    ("n_test_closure", "i", true),
    ("n_test_parameter_start", "i", true), // the two-counter iterator
    ("n_test_same_name_twice", "i", true),
    ("n_test_same_name_twice", "i#1", true),
    ("n_test_callee_between_reads", "i", true),
];

/// `(function, jumps with the rotation on, off)`: each rotated loop trades its jump back for
/// the one entering at its test, and drops the jump past its exit.
const JUMPS: &[(&str, usize, usize)] = &[
    ("n_first_over", 1, 2),
    ("n_from", 1, 2),
    ("n_test_while", 2, 3),
    ("n_test_zero_rounds", 6, 11),
    ("n_test_continue_while", 3, 4),
    ("n_test_inner_continue_and_break", 7, 11),
    ("n_test_inclusive_variable_end", 3, 4),
    ("n_test_comprehension", 3, 4),
    ("n_test_filtered", 4, 5),
    ("n_test_condition_count", 2, 3),
];

fn loft(args: &[&str], cells: &str, env: &[(&str, &str)]) -> (String, String, bool) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(cells);
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_LOOP_ROTATE")
        .env_remove("LOFT_NO_LOOP_VAR_ALIAS")
        .env_remove("LOFT_TRACE_LOOP_VAR_ALIAS");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

/// The bytecode listing of `name`, to the next listing or printed function.
fn bytecode_of<'a>(dump: &'a str, name: &str) -> &'a str {
    let head = format!(":{name}(");
    let start = dump
        .match_indices("byte-code for ")
        .map(|(i, _)| i)
        .find(|&i| dump[i..].lines().next().is_some_and(|l| l.contains(&head)))
        .unwrap_or_else(|| panic!("no bytecode printed for {name}"));
    let rest = &dump[start + 1..];
    let end = [rest.find("byte-code for "), rest.find("\nfn ")]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(rest.len());
    &rest[..end]
}

#[test]
fn which_loop_variables_share_their_index_slot() {
    let (_, err, ok) = loft(
        &["--interpret"],
        ALIAS_CELLS,
        &[("LOFT_TRACE_LOOP_VAR_ALIAS", "1")],
    );
    assert!(ok, "the cells failed:\n{err}");
    for (f, v, shares) in ALIASES {
        let line = if *shares {
            format!("loop-var-alias: {f} {v} shares ")
        } else {
            format!("loop-var-alias: {f} {v} declined")
        };
        assert!(
            err.lines().any(|l| l.starts_with(&line)),
            "{f} {v}: expected `{line}…` in the trace:\n{err}"
        );
    }
    assert!(
        !err.contains("loop-var-alias: n_test_reversed"),
        "a reversed range is no counted range and is never offered"
    );
}

#[test]
fn a_rotated_loop_carries_one_jump_fewer() {
    let (on, _, _) = loft(&["introspect"], ROTATE_CELLS, &[]);
    let (off, _, _) = loft(
        &["introspect"],
        ROTATE_CELLS,
        &[("LOFT_NO_LOOP_ROTATE", "1")],
    );
    for (f, with, without) in JUMPS {
        assert_eq!(
            bytecode_of(&on, f).matches("GotoWord(").count(),
            *with,
            "{f}: jumps with the rotation on"
        );
        assert_eq!(
            bytecode_of(&off, f).matches("GotoWord(").count(),
            *without,
            "{f}: jumps with the rotation off"
        );
    }
}

#[test]
fn both_cell_files_hold_under_every_switch_state() {
    let states: [&[(&str, &str)]; 4] = [
        &[],
        &[("LOFT_NO_LOOP_ROTATE", "1")],
        &[("LOFT_NO_LOOP_VAR_ALIAS", "1")],
        &[
            ("LOFT_NO_LOOP_ROTATE", "1"),
            ("LOFT_NO_LOOP_VAR_ALIAS", "1"),
        ],
    ];
    for cells in [ALIAS_CELLS, ROTATE_CELLS] {
        for env in states {
            let (out, err, ok) = loft(&["--interpret"], cells, env);
            assert!(
                ok && out.trim_end().ends_with("ok"),
                "{cells} under {env:?}:\n{out}\n{err}"
            );
        }
    }
}
