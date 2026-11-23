// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-LiftReuse` — the EMISSION pins.  Lifted call results that are tuples of scalars
//! share one variable where their live ranges over the body's top-level operators do not
//! overlap (`opt::lift_reuse`, the last rewrite of `close_program`).  The pins count the IR
//! pass's trace: how many lifts each function holds and how many variables they share.
//! `LOFT_NO_LIFT_REUSE=1` keeps a variable per lift.  The cells
//! (`tests/scripts/lifted-values-whose-lives-do-not-overlap-share-one-variable.loft`) say the
//! VALUES hold on both backends; this pins what is shared.
use std::path::{Path, PathBuf};

const CELLS: &str =
    "tests/scripts/lifted-values-whose-lives-do-not-overlap-share-one-variable.loft";
const BUILDER: &str =
    "tests/scripts/a-function-of-many-record-temporaries-compiles-in-seconds.loft";

/// The pass's trace lines for `file`, `fn=<name> <lifts> lift(s) share <vars> variable(s)`
/// reduced to `(name, lifts, vars)`.
fn shared(file: &str, env: &[(&str, &str)]) -> Vec<(String, usize, usize)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(["--interpret", src.to_str().unwrap()])
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TRACE_LIFT_REUSE", "1")
        .env_remove("LOFT_NO_LIFT_REUSE");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    assert!(
        out.status.success(),
        "{file} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter_map(|l| l.strip_prefix("[lift-reuse] fn="))
        .map(|l| {
            let w: Vec<&str> = l.split(' ').collect();
            (
                w[0].to_string(),
                w[1].parse().unwrap(),
                w[4].parse().unwrap(),
            )
        })
        .collect()
}

// Each cell function's lifts in the fewest variables their live ranges allow: c1 and c3 hold
// two `V3`s at once; c4 one at a time; c5 one `V3` and one `V2` (its middle line reads the
// first `V2` into a float before the second is written).  c2's lifts sit under an `if` and
// decline.
#[test]
fn the_cells_share_by_live_range() {
    let got = shared(CELLS, &[]);
    let want = [
        ("n_c1", 6, 2),
        ("n_c3", 4, 2),
        ("n_c4", 3, 1),
        ("n_c5", 6, 2),
    ];
    let want: Vec<(String, usize, usize)> = want
        .iter()
        .map(|(n, l, v)| ((*n).to_string(), *l, *v))
        .collect();
    assert_eq!(got, want);
}

// loft#2613: 128 builder lines of four lifts each (two `Vec3`, a `Vec2`, a `Vertex`) are four
// variables, one per type and role, where each lift had one.
#[test]
fn a_straight_line_builder_holds_one_variable_per_role() {
    assert_eq!(shared(BUILDER, &[]), vec![("n_build".to_string(), 512, 4)]);
}

#[test]
fn the_switch_keeps_a_variable_per_lift() {
    assert!(shared(CELLS, &[("LOFT_NO_LIFT_REUSE", "1")]).is_empty());
}
