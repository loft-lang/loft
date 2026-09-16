// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-ao (`@FR-R-Invariant`) — the EMISSION pins.  An integer chain a loop cannot
//! change (`+ - * neg & | ^` over literals and variables the loop neither rebinds nor lets
//! escape) is evaluated at its first use and answered from a memo after; the memo is
//! declared at the innermost loop that spells the chain; a rebound leaf, an escaped leaf
//! (a by-reference or fn-ref argument), a field read and a chain of literals alone decline;
//! one chain spelled twice is one memo; `LOFT_NO_INVARIANT_HOIST=1` restores the per-use
//! evaluation everywhere and `LOFT_HOIST_VERIFY=1` asserts the memo at every use.  The
//! overflow note fires where the FIRST evaluation stands — `LOFT_DEV_SOFT_HALT=1` counts
//! it.  The cell corpus (`bytecode-comparisons/V-ao-invariant-arith-cells.loft`) says the
//! VALUES hold on both backends; this pins what is emitted.
//!
//! Beside it, loft#1534: the non-sentinel proof's escape collector reads a by-reference
//! argument through its `OpCreateStack` spelling, so a callee's overflow into a local the
//! proof trusted answers `null` on native as it does on the interpreter.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-ao-invariant-arith-cells.loft";

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    // One file per TEST: the tests run on threads of one process, so a name keyed on the
    // pid alone would make them delete each other's emission.
    let out = std::env::temp_dir().join(format!(
        "loft_invariant_arith_{}_{tag}.rs",
        std::process::id()
    ));
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_INVARIANT_HOIST")
        .env_remove("LOFT_HOIST_VERIFY");
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

/// How many memos `b` declares (each declaration line carries the § V-ao marker), and how
/// many uses read one.
fn memos(b: &str) -> (usize, usize) {
    (
        b.matches("//@PLN157 § V-ao invariant chain").count(),
        b.matches("{ if !__ia_").count(),
    )
}

/// Run `src` as a native program with `env`, answering (stdout, stderr).
fn run_native(tag: &str, src: &str, env: &[(&str, &str)]) -> (String, String) {
    let file = std::env::temp_dir().join(format!(
        "loft_invariant_arith_{}_{tag}.loft",
        std::process::id()
    ));
    std::fs::write(&file, src).expect("write the probe");
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native")
        .arg(&file)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_INVARIANT_HOIST")
        .env_remove("LOFT_NO_NN_FAST");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft --native");
    let _ = std::fs::remove_file(&file);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn the_tap_shape_memoises_its_invariant_part_at_the_innermost_loop() {
    let rust = emit("tap", &[]);
    let b = body(&rust, "n_m1");
    // The x loop's `yy * iw + xmin` and the ch loop's `yy * 2` — one memo each, declared
    // at the loop that spells it and read once there.
    assert_eq!(memos(b), (2, 2), "m1 memos:\n{b}");
    assert!(
        b.contains("((var_yy), (var_iw))), (var_xmin))); __ia_"),
        "the tap's memo evaluates `yy * iw + xmin` at its first use:\n{b}"
    );
    // m7: `a * b + i * 10` is invariant in the j loop (i is the outer counter) — one memo,
    // three ops, and the i loop itself declares none.
    let b7 = body(&rust, "n_m7");
    assert_eq!(memos(b7), (1, 1), "m7 memos:\n{b7}");
    assert!(
        b7.contains("//@PLN157 § V-ao invariant chain, 3 ops"),
        "m7's memo carries the whole invariant chain:\n{b7}"
    );
}

#[test]
fn a_rebound_or_escaped_leaf_a_field_read_and_a_literal_chain_decline() {
    let rust = emit("decline", &[]);
    for (name, why) in [
        ("n_m2", "a leaf the body rebinds"),
        ("n_m3", "a leaf handed to a by-reference parameter"),
        ("n_m13", "a field read is not a leaf"),
        ("n_m15", "a leaf handed to a fn-ref call"),
        ("n_m16", "a leaf rebound inside a nested loop"),
    ] {
        let b = body(&rust, name);
        assert_eq!(
            memos(b),
            (0, 0),
            "{name} ({why}) must memoise nothing:\n{b}"
        );
    }
}

#[test]
fn one_chain_spelled_twice_is_one_memo_and_every_loop_shape_takes_one() {
    let rust = emit("shapes", &[]);
    let b8 = body(&rust, "n_m8");
    assert_eq!(memos(b8), (1, 2), "m8: one memo, two uses:\n{b8}");
    for (name, what) in [
        ("n_m9", "a while loop"),
        ("n_m10", "the range's end in the loop's own test"),
        ("n_m11", "a vector iteration"),
        ("n_m12", "negation and the bitwise ops"),
        ("n_m14", "parameters as leaves"),
    ] {
        let b = body(&rust, name);
        assert_eq!(memos(b).0, 1, "{name} ({what}) declares one memo:\n{b}");
    }
    assert!(
        body(&rust, "n_m12").contains("//@PLN157 § V-ao invariant chain, 4 ops"),
        "m12's memo is the whole `((-a) & 255) | (b ^ 7)` chain"
    );
}

#[test]
fn the_switch_restores_the_per_use_form_and_the_verify_form_asserts_every_use() {
    let off = emit("off", &[("LOFT_NO_INVARIANT_HOIST", "1")]);
    assert!(!off.contains("__ia_"), "the switch leaves no memo anywhere");
    let verify = emit("verify", &[("LOFT_HOIST_VERIFY", "1")]);
    let b = body(&verify, "n_m1");
    assert!(
        b.contains("assert!(__c == __ia_"),
        "the verify form compares every use with a fresh evaluation:\n{b}"
    );
}

const BIG: &str = "big = 4611686018427387904;";

/// The overflow note (`LOFT_DEV_SOFT_HALT=1`) fires where the first evaluation stands: once
/// for a loop that evaluates the chain three times (the per-use form notes three), never for
/// a zero-trip loop, and once for a chain that stands after a `continue` the first iteration
/// takes.
#[test]
fn the_overflow_note_fires_where_the_first_evaluation_stands() {
    let m4 = format!(
        "fn main() {{ {BIG} s = 0; for i in 0..3 {{ s += (big * 4 + i) ?? 7; }} println(\"s {{s}}\"); }}"
    );
    let m5 = format!(
        "fn main() {{ {BIG} s = 0; n = 0; for i in 0..n {{ s += (big * 4 + i) ?? 7; }} println(\"s {{s}}\"); }}"
    );
    let m6 = format!(
        "fn main() {{ {BIG} s = 0; for i in 0..2 {{ if i == 0 {{ continue; }} s += (big * 4 + i) ?? 7; }} println(\"s {{s}}\"); }}"
    );
    let notes = |out: &(String, String)| out.1.matches("soft-halt: integer overflow").count();
    let halt = [("LOFT_DEV_SOFT_HALT", "1")];
    let both = [
        ("LOFT_DEV_SOFT_HALT", "1"),
        ("LOFT_NO_INVARIANT_HOIST", "1"),
    ];
    let r = run_native("m4", &m4, &halt);
    assert!(r.0.contains("s 21"), "m4 value:\n{}\n{}", r.0, r.1);
    assert_eq!(
        notes(&r),
        1,
        "m4: the memo notes the overflow once:\n{}",
        r.1
    );
    let r = run_native("m4off", &m4, &both);
    assert_eq!(
        notes(&r),
        3,
        "m4 per-use: three evaluations, three notes:\n{}",
        r.1
    );
    let r = run_native("m5", &m5, &halt);
    assert!(r.0.contains("s 0"), "m5 value:\n{}", r.0);
    assert_eq!(
        notes(&r),
        0,
        "m5: a zero-trip loop evaluates nothing:\n{}",
        r.1
    );
    let r = run_native("m6", &m6, &halt);
    assert!(r.0.contains("s 7"), "m6 value:\n{}", r.0);
    assert_eq!(
        notes(&r),
        1,
        "m6: the first evaluation is the second iteration's:\n{}",
        r.1
    );
}

/// loft#1534 — a local handed to a `&integer` parameter escapes the non-sentinel proof
/// through its `OpCreateStack` spelling: the callee's overflow leaves it null, and `k + 1`
/// answers null on native as the interpreter does (the proven form answered a number).
#[test]
fn a_by_reference_argument_escapes_the_non_sentinel_proof() {
    let src = "fn poison(n: &integer) { n = n * 4; }\n\
               fn main() { k = 1; k = 4611686018427387904; poison(k); r = k + 1; println(\"r {r}\"); }\n";
    let (out, err) = run_native("escape", src, &[]);
    assert!(
        out.contains("r null"),
        "loft#1534: native must answer null:\n{out}\n{err}"
    );
}
