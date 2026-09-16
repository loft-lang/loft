// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-LeafChain` — in the lean tier (`--native-release`) a function whose whole call
//! tree is frameless carries no prelude: no depth entry and no fn-ref buffer guard.  Every
//! user function it reaches has a loft body, none is on a cycle, and none calls a fn-ref.
//!
//! The cells (`plans/157-native-4x-drawing/bytecode-comparisons/N4b-leaf-chain-cells.loft`)
//! carry one shape per case.  These tests pin which functions keep a frame, that the switch
//! (`LOFT_NO_LEAF_CHAIN=1`) and the named tier keep every non-leaf frame, that the lean
//! build answers what the interpreter answers, and that runaway recursion through a
//! frameless helper still ends in the depth cap's report rather than a native stack crash.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/N4b-leaf-chain-cells.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
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

fn emit(out: &Path, tier: &[&str], env: &[(&str, &str)]) -> String {
    let src = cells();
    let out_s = out.to_string_lossy();
    let mut args: Vec<&str> = tier.to_vec();
    args.extend(["--native-emit", &out_s, &src]);
    let res = loft(&args, env);
    assert!(
        out.exists(),
        "no Rust emitted (exit {:?}): {}",
        res.status,
        String::from_utf8_lossy(&res.stderr)
    );
    std::fs::read_to_string(out).expect("read the emitted Rust")
}

/// The body of one emitted function.
fn body<'a>(rust: &'a str, name: &str) -> &'a str {
    let start = rust
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("{name} was not emitted"));
    let rest = &rust[start..];
    let end = rest[1..].find("\nfn ").map_or(rest.len(), |i| i + 1);
    &rest[..end]
}

fn has_frame(rust: &str, name: &str) -> bool {
    let b = body(rust, name);
    let push = b.contains("cr_call_push");
    let guard = b.contains("FnRefBufGuard");
    assert_eq!(
        push, guard,
        "{name}: the depth entry and the buffer guard go together:\n{b}"
    );
    push
}

/// Frameless in the lean tier by the chain rule; framed without it.
const CHAIN: [&str; 4] = ["n_at", "n_matches_at", "n_count_word", "n_bytes_of"];
/// Leaves: frameless in every tier (N4).
const LEAVES: [&str; 2] = ["n_lower_byte", "n_dbl"];
/// On a cycle, calling one, calling a fn-ref, or calling a native: framed in every tier.
const FRAMED: [&str; 9] = [
    "n_fact",
    "n_is_odd",
    "n_is_even",
    "n_fact_plus",
    "n_apply",
    "n_twice",
    "n_checked",
    "n_depth_sum",
    "n_use_down",
];

#[test]
fn the_lean_tier_elides_a_frameless_tree() {
    let out = std::env::temp_dir().join("loft_leaf_chain_on.rs");
    let rust = emit(&out, &["--native-release"], &[]);
    for name in CHAIN.iter().chain(LEAVES.iter()) {
        assert!(!has_frame(&rust, name), "{name} must carry no frame in the lean tier");
    }
    for name in FRAMED {
        assert!(has_frame(&rust, name), "{name} must keep its frame");
    }
    let _ = std::fs::remove_file(&out);
}

#[test]
fn the_switch_and_the_named_tier_keep_every_non_leaf_frame() {
    for (label, tier, env) in [
        ("LOFT_NO_LEAF_CHAIN=1", &["--native-release"][..], &[("LOFT_NO_LEAF_CHAIN", "1")][..]),
        ("the named tier", &[][..], &[][..]),
    ] {
        let out = std::env::temp_dir().join(format!(
            "loft_leaf_chain_{}.rs",
            if tier.is_empty() { "named" } else { "off" }
        ));
        let rust = emit(&out, tier, env);
        for name in CHAIN.iter().chain(FRAMED.iter()) {
            assert!(has_frame(&rust, name), "{label}: {name} must keep its frame");
        }
        for name in LEAVES {
            assert!(!has_frame(&rust, name), "{label}: the leaf {name} stays frameless");
        }
        let _ = std::fs::remove_file(&out);
    }
}

#[test]
fn the_lean_build_answers_what_the_interpreter_answers() {
    let src = cells();
    let interp = loft(&["--interpret", &src], &[]);
    let lean = loft(
        &["--native-release", &src],
        &[("LOFT_NATIVE_LEAK_CHECK", "1"), ("LOFT_STRICT_STORES", "1")],
    );
    let expected = "c1 97 99 -1 -1\nc2 3\nc3 3628800\nc4 true true false\nc5 240\nc6 41\n\
                    c7 5\nc8 1194\nc9 3 104 121\nc9 3 104 121\nc9 3 104 121\nc10 8\n";
    assert_eq!(String::from_utf8_lossy(&interp.stdout), expected, "the oracle moved");
    assert_eq!(
        String::from_utf8_lossy(&lean.stdout),
        expected,
        "lean stderr: {}",
        String::from_utf8_lossy(&lean.stderr)
    );
    assert!(lean.status.success(), "lean exit {:?}", lean.status);
    let err = String::from_utf8_lossy(&lean.stderr);
    assert!(
        !err.contains("not freed") && !err.contains("strict-store"),
        "the lean build leaked or read a freed store:\n{err}"
    );
}

#[test]
fn runaway_recursion_through_a_frameless_helper_hits_the_depth_cap() {
    let dir = std::env::temp_dir().join(format!("loft_leaf_chain_runaway_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let prog = dir.join("runaway.loft");
    std::fs::write(
        &prog,
        "fn at(s: text, i: integer) -> integer { if i < 0 || i >= size(s) { -1 } else { s.byte_at(i) } }\n\
         fn pick(n: integer) -> integer { at(\"ab\", n % 2) }\n\
         fn runaway(n: integer) -> integer { pick(n) + runaway(n + 1) }\n\
         fn main() { println(\"{runaway(0)}\"); }\n",
    )
    .expect("write the probe");
    let res = loft(&["--native-release", &prog.to_string_lossy()], &[]);
    let err = String::from_utf8_lossy(&res.stderr);
    assert_eq!(res.status.code(), Some(1), "a clean fault exit, not a signal:\n{err}");
    assert!(
        err.contains("call stack overflow"),
        "the depth cap must report the runaway recursion:\n{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
