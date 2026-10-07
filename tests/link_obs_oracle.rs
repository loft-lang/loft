// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN102 transparent-link widening — the OBSERVABILITY oracle guard (build step 3).
//!
//! Pins `use_analysis::link_observability_of`'s verdicts (via `LOFT_DUMP_LINK_OBS`) against Matrix O,
//! both backends. Per copy-fill bind `a = s.v`, the oracle answers "would a shared-store LINK be
//! UNOBSERVABLE (copy ≡ link)?" — neither side's store is mutated after the bind, ALIAS-AWARE. Step 4
//! links only when this AND `link_is_safe` (step 2) hold, so a false `unobservable` would let a write
//! silently cross the copy boundary. The load-bearing cell is O5 (a `&`-alias written after the bind,
//! created BEFORE it) — only the alias-aware clause can flag it. Report-only; no codegen yet.

use std::collections::HashMap;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn probe() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/scripts/link-obs-oracle.loft")
}

/// `fn-name -> unobservable` for every `var=a` verdict (each cell uses a distinct `a`).
fn verdicts(backend: &str) -> HashMap<String, bool> {
    let out = loft::platform::process::harness_command(loft_bin())
        .arg(backend)
        .arg(probe())
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_DUMP_LINK_OBS", "1")
        .env("LOFT_TIMEOUT", "120")
        .output()
        .expect("run loft");
    assert!(out.status.success(), "[{backend}] probe must run clean");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut m = HashMap::new();
    for line in stderr.lines() {
        let Some(rest) = line.strip_prefix("link-obs-dbg: ") else {
            continue;
        };
        let (mut f, mut is_a, mut unobs) = (None, false, None);
        for tok in rest.split_whitespace() {
            if let Some(x) = tok.strip_prefix("fn=") {
                f = Some(x.to_string());
            } else if tok == "var=a" {
                is_a = true;
            } else if let Some(x) = tok.strip_prefix("unobs=") {
                unobs = x.parse::<u8>().ok().map(|n| n == 1);
            }
        }
        if is_a && let (Some(f), Some(u)) = (f, unobs) {
            m.insert(f, u);
        }
    }
    m
}

fn assert_matrix_o(backend: &str) {
    let v = verdicts(backend);
    let get = |name: &str| -> bool {
        *v.get(name)
            .unwrap_or_else(|| panic!("[{backend}] missing verdict for {name}\n{v:?}"))
    };

    // UNOBSERVABLE (positive control) — only the genuinely read-only-both shape.
    assert!(
        get("n_o1_readonly_both"),
        "[{backend}] O1 (read-only both) must be UNOBSERVABLE\n{v:?}"
    );
    // OBSERVABLE — a wrong `unobservable` here would let a write cross the copy boundary under a link.
    assert!(
        !get("n_o2_write_local"),
        "[{backend}] O2 (local mutated) must be OBSERVABLE\n{v:?}"
    );
    assert!(
        !get("n_o3_write_source"),
        "[{backend}] O3 (source mutated) must be OBSERVABLE\n{v:?}"
    );
    assert!(
        !get("n_o4_sibling_alias"),
        "[{backend}] O4 (sibling &-alias write-through) must be OBSERVABLE\n{v:?}"
    );
    // O5 — the load-bearing alias-aware proof: the `&` precedes the bind, so ONLY the alias-aware
    // clause can flag it. If this reads unobservable, the alias clause is dead and O4 passed for the
    // wrong reason.
    assert!(
        !get("n_o5_alias_before_bind"),
        "[{backend}] O5 (alias created before the bind, written after) must be OBSERVABLE — the \
         alias-aware clause is load-bearing here\n{v:?}"
    );

    // Non-vacuous: both verdicts appear.
    assert!(
        v.values().any(|&u| u) && v.values().any(|&u| !u),
        "[{backend}] the oracle must emit both unobservable and observable (not vacuous)\n{v:?}"
    );
}

#[test]
fn matrix_o_interpret() {
    assert_matrix_o("--interpret");
}

#[test]
fn matrix_o_native() {
    assert_matrix_o("--native");
}
