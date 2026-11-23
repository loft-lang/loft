// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-Base`'s twin clause — the EMISSION pins.  A callee twin (`__inv`, `@FR-R-Inputs`)
//! takes the element BASE of each header it is handed (`__ib_k: *const u8`), a view of that
//! path inside the twin shares it (`__vb_N`), and the twin's fused reads and writes take the
//! one-load form (`get_elem_at` / `vec_set_at`) instead of resolving the store per element.
//! A caller whose loop holds a base passes it; one whose loop grows a store derives one from
//! the held header AT the call.  `LOFT_NO_TWIN_BASE=1` restores the header-only twin.  The
//! cell corpus (`tests/scripts/157-twin-base.loft`) says the VALUES hold on both backends,
//! in both switch states and under the falsifiers; this pins what is emitted and runs it.
use std::path::{Path, PathBuf};
use std::process::Output;

const CELLS: &str = "tests/scripts/157-twin-base.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_TWIN_BASE")
        .env_remove("LOFT_HOIST_VERIFY");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

fn run_ok(args: &[&str], env: &[(&str, &str)]) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut a: Vec<&str> = args.to_vec();
    let s = src.to_str().unwrap().to_string();
    a.push(&s);
    let out = loft(&a, env);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.trim_end().ends_with("ok"),
        "{args:?} {env:?}: exit {:?}\nstdout: {stdout}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn the_cells_hold_on_both_backends_in_both_switch_states_under_the_falsifiers() {
    run_ok(&["--interpret"], &[("LOFT_STRICT_STORES", "1")]);
    for switch in [None, Some(("LOFT_NO_TWIN_BASE", "1"))] {
        {
            // Every detector at once: one build per switch state, each detector still on
            // (falsifying a guard is a one-time receipt, not a cost of every run).
            let mut env = vec![
                ("LOFT_STRICT_STORES", "1"),
                ("LOFT_POISON", "1"),
                ("LOFT_POISON_CLAIM", "1"),
                ("LOFT_NATIVE_LEAK_CHECK", "1"),
                ("LOFT_HOIST_VERIFY", "1"),
            ];
            env.extend(switch);
            run_ok(&["--native"], &env);
        }
    }
}
