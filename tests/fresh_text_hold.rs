// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-Mint`'s text clause — PINNED on `tests/scripts/157-a-fresh-element-text-keeps-the-loop-hold.loft`.
//! The cells' values pass whether or not a loop holds its header, and a hold that should have
//! been refused answers right until the store happens to move, so this runs them where a stale
//! hold cannot hide — under `LOFT_HOIST_VERIFY=1` (every held read checked) and `LOFT_POISON=1`
//! (every growth moves and scribbles) — in both switch states, and pins which loops hold,
//! read off `LOFT_TRACE_HOIST_DECLINE`.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELLS: &str = "tests/scripts/157-a-fresh-element-text-keeps-the-loop-hold.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS))
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_FRESH_TEXT_SET")
        .env_remove("LOFT_HOIST_VERIFY")
        .env_remove("LOFT_TRACE_HOIST_DECLINE");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

#[test]
fn the_cells_hold_under_the_hoist_falsifiers_in_both_switch_states() {
    for mode in ["--native", "--native-release"] {
        for switch in [None, Some(("LOFT_NO_FRESH_TEXT_SET", "1"))] {
            for falsifier in [("LOFT_HOIST_VERIFY", "1"), ("LOFT_POISON", "1")] {
                let mut env = vec![falsifier];
                env.extend(switch);
                let out = loft(&[mode], &env);
                let stdout = String::from_utf8_lossy(&out.stdout);
                assert!(
                    out.status.success() && stdout.trim_end().ends_with("hold ok"),
                    "{mode} {env:?}: exit {:?}\nstdout: {stdout}\nstderr: {}",
                    out.status,
                    String::from_utf8_lossy(&out.stderr)
                );
            }
        }
    }
}

#[test]
fn a_fresh_elements_text_keeps_the_hold_and_a_shared_store_refuses_it() {
    // Each decline trace line for a loop of `build` or `main`.
    let declines = |env: &[(&str, &str)]| -> Vec<String> {
        let mut e = vec![("LOFT_TRACE_HOIST_DECLINE", "1")];
        e.extend_from_slice(env);
        let out = loft(&["--native-release", "--native-emit", "/dev/null"], &e);
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .filter(|l| l.starts_with("hoist: n_build loop") || l.starts_with("hoist: n_main loop"))
            .map(str::to_string)
            .collect()
    };
    // On: only h4 declines — a text written into an EXISTING element of the
    // read vector's store is no fresh mint.  h1's `build` and h3 hold; h2, minting into `d.out`
    // while reading `d.src`, is refused by the mint's own aliasing decision and declines nowhere
    // here.
    let src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS))
        .expect("read the cells");
    let h4 = src
        .lines()
        .position(|l| l.contains("d.out[0] = T {"))
        .expect("h4's loop")
        + 1;
    let at = format!(".loft:{h4}:");
    let on = declines(&[]);
    assert!(
        !on.is_empty() && on.iter().all(|l| l.contains(&at)),
        "only h4's write (line {h4}) declines: {on:#?}"
    );
    // Off: the fresh-element text writes block their loops again, `build`'s among them.
    let off = declines(&[("LOFT_NO_FRESH_TEXT_SET", "1")]);
    assert!(
        off.iter().any(|l| l.starts_with("hoist: n_build loop")),
        "the switch restores the declines: {off:#?}"
    );
}
