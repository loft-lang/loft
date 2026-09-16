// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN164 C3 (`@FR-B-Disturb`, `@FR-B-View`) — A DISTURBANCE ONE FRAME DOWN IS A DISTURBANCE:
//! a container a CALLEE grows or removes from ends the place a caller's view names, so the view
//! materialises exactly as it does when the same statement is written inline.
//!
//! `tests/scripts/164-callee-disturb.loft` carries the fifteen pairs and says the VALUES agree
//! on both backends; the corpus runner runs it.  What is pinned here is what a value comparison
//! cannot see: that the switch really does restore the defect (so the guard's `@falsified-at:`
//! is a measurement and not a claim), that the advice NAMES the callee — `(B-View)` promises
//! the author is told, and a reader told "`sc` grows" with no append anywhere in the function
//! is not told — and that the summary admits exactly the callees that disturb a parameter and
//! no others, which is the direction that would go wrong quietly: a widened summary
//! materialises views whose writes land today, and every one of those is a program losing its
//! meaning rather than a compiler losing an optimisation.
use std::path::{Path, PathBuf};
use std::process::Command;

const GUARD: &str = "tests/scripts/164-callee-disturb.loft";

fn guard() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(GUARD)
}

fn loft() -> Command {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "240")
        .env_remove("LOFT_NO_CALLEE_DISTURB");
    cmd
}

/// Run the guard in `mode` with `env`; the exit status, stdout and stderr.
fn run(mode: &str, env: &[(&str, &str)]) -> (bool, String, String) {
    let mut cmd = loft();
    cmd.arg(mode).arg(guard());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const STRICT: &[(&str, &str)] = &[
    ("LOFT_POISON", "1"),
    ("LOFT_POISON_CLAIM", "1"),
    ("LOFT_STRICT_STORES", "1"),
];
const STRICT_NATIVE: &[(&str, &str)] = &[
    ("LOFT_POISON", "1"),
    ("LOFT_POISON_CLAIM", "1"),
    ("LOFT_STRICT_STORES", "1"),
    ("LOFT_NATIVE_LEAK_CHECK", "1"),
];
const OFF: &[(&str, &str)] = &[("LOFT_NO_CALLEE_DISTURB", "1")];

#[test]
fn the_pairs_agree_on_both_backends_under_every_falsifier() {
    let (ok, out, err) = run("--interpret", STRICT);
    assert!(ok, "the guard failed on --interpret:\n{err}");
    assert!(
        out.contains("15 pairs, inline and callee agree"),
        "the guard did not reach its last pair:\n{out}"
    );
    let (native_ok, native, native_err) = run("--native", STRICT_NATIVE);
    assert!(native_ok, "the guard failed on --native:\n{native_err}");
    assert_eq!(out, native, "the two backends must print the same");
}

/// The switch restores the per-frame answer, and the guard then FAILS — which is what makes its
/// `@falsified-at:` receipt a measurement.  A guard that passed either way would be pinning
/// nothing.
#[test]
fn the_switch_restores_the_defect_the_guard_catches() {
    for mode in ["--interpret", "--native"] {
        let (ok, _, err) = run(mode, OFF);
        assert!(
            !ok,
            "{mode} with LOFT_NO_CALLEE_DISTURB=1 must fail the guard — without that the guard \
             measures nothing"
        );
        assert!(
            err.contains("c2 grow, realloc: callee 4294967401, want 3"),
            "{mode} must fail on c2 with the stale read the unit removes:\n{err}"
        );
    }
}

/// `(B-View)` promises the author is TOLD.  For a disturbance one frame down that promise needs
/// the callee's NAME: nothing in `c_grow_callee` appends to `sc`, so a reader given only
/// *"`sc` grows"* goes looking for a statement that is not there — the same failure the `Grown`
/// sentence was split off to avoid.
#[test]
fn the_advice_names_the_callee_the_disturbance_came_through() {
    let (_, _, err) = run("--interpret", &[]);
    assert!(
        err.contains(
            "in `c_grow_callee`, `e` was copied out of `sc` because `sc` grows inside the call \
             to `grow_field` while `e` is in use"
        ),
        "a growth one frame down must name its callee:\n{err}"
    );
    assert!(
        err.contains(
            "in `c_rm_moves_callee`, `e` was copied out of `sc` because `sc` is modified inside \
             the call to `remove_first` while `e` is in use"
        ),
        "a removal one frame down must name its callee:\n{err}"
    );
    // The INLINE twin keeps the sentence it always had: no call, no clause.
    assert!(
        err.contains(
            "in `c_grow_inline`, `e` was copied out of `sc` because `sc` grows while `e` is in use"
        ),
        "an inline growth must not gain a callee clause:\n{err}"
    );
    // Every control must be silent: an advice line there would be asserting that writes stop
    // reaching the container, over a binding the cells prove still aliases.
    for quiet in [
        "c_alias_callee",
        "c_other_callee",
        "c_rebuild_callee",
        "c_sibling_callee",
        "c_unrelated_callee",
    ] {
        assert!(
            !err.contains(&format!("in `{quiet}`")),
            "`{quiet}` still aliases, so it must draw no copy notice:\n{err}"
        );
    }
}

/// The summary admits exactly the callees that disturb a caller's container through a parameter.
/// `mk` is the control that matters most: it GROWS `__retbuf.els` building its own result, and
/// counting a hidden return buffer as a caller's container would report a disturbance at every
/// call to every record-returning function in the program.
#[test]
fn the_summary_admits_the_disturbing_callees_and_no_others() {
    let mut cmd = loft();
    cmd.arg("--interpret")
        .arg(guard())
        .env("LOFT_TRACE_DISTURB", "1");
    let out = cmd.output().expect("spawn loft");
    let err = String::from_utf8_lossy(&out.stderr);
    let mut named: Vec<&str> = err
        .lines()
        .filter_map(|l| l.strip_prefix("[disturb] "))
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    named.sort_unstable();
    named.dedup();
    assert_eq!(
        named,
        [
            // Reached through the call graph: each one forwards its own parameter downward.
            "n_c_param_callee",
            // Disturbs its own parameter inline, which is the same fact about its body.
            "n_c_param_inline",
            "n_grow_deep",
            "n_grow_field",
            "n_grow_sibling",
            "n_grow_vec",
            "n_remove_first",
            "n_remove_last",
        ],
        "the summary must name every definition that disturbs a caller's container through a \
         parameter, and nothing else.  The three absences are the load-bearing half: `n_mk` \
         GROWS `__retbuf.els` building its own result, and counting a hidden return buffer \
         would report a disturbance at every call to every record-returning function; \
         `n_rebuild_field` writes `sc.els = [x]`, a clear and then the appends, which \
         `(B-Disturb)` says is an overwrite and not a disturbance; `n_touch_nothing` and \
         `n_write_other_field` do nothing to a container at all:\n{err}"
    );
}
