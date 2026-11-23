// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-Mint`'s text clause — PINNED on `tests/scripts/157-a-fresh-element-text-keeps-the-loop-hold.loft`.
//! The cells' values pass whether or not a loop holds its header, and a hold that should have
//! been refused answers right until the store happens to move, so this runs them where a stale
//! hold cannot hide — under `LOFT_HOIST_VERIFY=1` (every held read checked) and `LOFT_POISON=1`
//! (every growth moves and scribbles) — in both switch states, and pins which loops hold,
//! read off `LOFT_TRACE_HOIST_DECLINE`.
use std::path::{Path, PathBuf};
use std::process::Output;

const CELLS: &str = "tests/scripts/157-a-fresh-element-text-keeps-the-loop-hold.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS))
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_FRESH_TEXT_SET")
        .env_remove("LOFT_NO_TEXT_SET_HEADER")
        .env_remove("LOFT_HOIST_VERIFY")
        .env_remove("LOFT_TRACE_HOIST_DECLINE");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

#[test]
fn the_cells_hold_under_the_hoist_falsifiers_in_both_switch_states() {
    for mode in ["--native", "--ship"] {
        for switch in [None, Some(("LOFT_NO_FRESH_TEXT_SET", "1"))] {
            {
                // Every detector at once: one build per switch state, each detector still on
                // (falsifying a guard is a one-time receipt, not a cost of every run).
                let mut env = vec![("LOFT_HOIST_VERIFY", "1"), ("LOFT_POISON", "1")];
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
