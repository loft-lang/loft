// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-GuardFree` — the EMISSION pins.  A function from which nothing that registers a
//! fn-ref return buffer is reachable, cycles included, constructs no `FnRefBufGuard` and
//! keeps its depth push; a function that dispatches a fn-ref, calls one that does (at any
//! depth), hands a capture back through a join tail, or is a generator keeps its guard;
//! `LOFT_NO_GUARD_FREE=1` restores the guard on every non-leaf frame.  The guard
//! (`tests/scripts/158-guard-free.loft`) scores the rule on the leak channel; this pins
//! what is emitted, in both tiers.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/158-guard-free.loft";

fn emit(tag: &str, lean: bool, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let out = std::env::temp_dir().join(format!("loft_guard_free_{}_{tag}.rs", std::process::id()));
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    if lean {
        cmd.arg("--native-release");
    }
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_GUARD_FREE")
        .env_remove("LOFT_NO_LEAF_CHAIN")
        .env_remove("LOFT_NO_LEAF_PRELUDE");
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

const GUARD: &str = "let _fnref_guard = codegen_runtime::FnRefBufGuard::new(cell, ";

/// The recursive functions no registrant reaches: no guard, the depth push kept.
const FREE: [&str; 5] = [
    "n_fib",
    "n_ladder",
    "n_even_big",
    "n_odd_big",
    "n_count_down",
];

/// The functions a registrant reaches: the guard stays.  `through` dispatches a fn-ref
/// itself; `top` and `mid` reach `leaf_dispatch`'s; `g6` builds and calls lambdas whose
/// tails hand a capture back; `total` runs a generator, which is opaque.
const KEPT: [&str; 6] = [
    "n_through",
    "n_leaf_dispatch",
    "n_mid",
    "n_top",
    "n_g6",
    "n_total",
];

#[test]
fn a_frame_no_registrant_reaches_carries_no_guard_in_the_named_tier() {
    let rust = emit("named", false, &[]);
    for name in FREE {
        let b = body(&rust, name);
        assert!(
            !b.contains(GUARD),
            "{name}: no registrant is reachable, so no guard"
        );
        assert!(
            b.contains("cr_call_push(")
                && b.contains("let _call_guard = codegen_runtime::CallGuard;"),
            "{name}: the depth push and its pop stay — the recursion cap is not this rule's"
        );
    }
    for name in KEPT {
        assert!(
            body(&rust, name).contains(GUARD),
            "{name}: a registrant is reachable, so the guard stays"
        );
    }
}

#[test]
fn the_lean_tier_agrees_and_keeps_its_own_elisions() {
    let rust = emit("lean", true, &[]);
    for name in FREE {
        assert!(
            !body(&rust, name).contains(GUARD),
            "{name}: no guard in the lean tier either"
        );
    }
    // `fib` is on a cycle, so `(R-LeafChain)` cannot drop its frame: the lean push stays.
    assert!(
        body(&rust, "n_fib").contains("cr_call_push_lean("),
        "fib: a recursive frame keeps its lean depth push"
    );
    for name in KEPT {
        assert!(
            body(&rust, name).contains(GUARD),
            "{name}: the guard stays in the lean tier"
        );
    }
}

#[test]
fn the_switch_restores_every_guard() {
    let rust = emit("switch", false, &[("LOFT_NO_GUARD_FREE", "1")]);
    for name in FREE.iter().chain(KEPT.iter()) {
        assert!(
            body(&rust, name).contains(GUARD),
            "{name}: LOFT_NO_GUARD_FREE=1 constructs the guard on every non-leaf frame"
        );
    }
}
