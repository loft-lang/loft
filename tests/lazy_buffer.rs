// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-O-LazyBuffer` — a hidden return buffer is minted in front of the statement that
//! hands it to a callee, behind a null test, instead of at function entry, so a path that
//! never makes the call never mints the store.
//!
//! The cells (`plans/164-activation-arena/bytecode-comparisons/A0-lazy-buffer-cells.loft`)
//! carry one shape per case, with the value each must print.  These tests pin that the
//! values hold on both backends in both switch states under the store falsifiers, that no
//! entry mint is left in the rewritten functions and the switch (`LOFT_NO_LAZY_BUFFER=1`)
//! restores every one, that a free is never taken for a use, and that the interpreter's
//! entry init is the non-allocating sentinel.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELLS: &str =
    "doc/claude/plans/164-activation-arena/bytecode-comparisons/A0-lazy-buffer-cells.loft";

const EXPECTED: &str = "a1 35 a2 15 10 a3 34 a4 2 105 a5 4 6 a6 3 a7 21\n\
                        a8 13 a9 4 a10 200 1 a11 18 a12 -1 -1 13 a13 0 9 4 \
                        a14 -1 0 114 214 -1 12\n";

/// The functions that minted a buffer at entry before the rewrite, with the number of
/// guarded mints each carries after it (`a5`'s loop-carried pair is guarded at the call
/// and at the rotation, `f15` is the record-buffer pool).
const REWRITTEN: [(&str, usize); 13] = [
    ("n_f1", 1),
    // ×2 since `@FR-R-GuardedChain` (2026-09-20): a2's loop is an INNERMOST counted loop with
    // a chain (`i - 1`), so its body is emitted twice — the guarded plain copy and the checked
    // `else` arm — and the one buffer's guarded mint is counted in each.
    ("n_a2", 2),
    ("n_a3", 2),
    ("n_f4", 2),
    ("n_a5", 4),
    ("n_a6", 1),
    ("n_a8", 2),
    ("n_a9", 1),
    ("n_f10", 1),
    ("n_a11", 1),
    ("n_f12", 1),
    ("n_f13", 1),
    ("n_f15", 1),
];

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

fn emit(out: &Path, env: &[(&str, &str)]) -> String {
    let src = cells();
    let res = loft(&["--native-emit", &out.to_string_lossy(), &src], env);
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

/// `(entry mints, guarded mints)` of a buffer: an entry mint is a bare assignment line
/// before the body's first `// loft:` marker, a guarded one sits behind the null test.
fn mints(b: &str) -> (usize, usize) {
    let prelude = b.find("// loft:").map_or(b, |i| &b[..i]);
    let entry = prelude
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with("var___ref_") && t.contains(" = OpDatabase(cell,var___ref_")
        })
        .count();
    let guard = "store_nr == u16::MAX) as u8) == 1 {";
    let guarded = b
        .match_indices(guard)
        .filter(|(i, _)| {
            let after = b[i + guard.len()..].trim_start();
            let line = after.split('\n').next().unwrap_or("");
            after.starts_with("var___ref_") && line.contains(" = OpDatabase(cell,var___ref_")
        })
        .count();
    (entry, guarded)
}

#[test]
fn the_values_hold_on_both_backends_in_both_switch_states() {
    let src = cells();
    for switch in [None, Some(("LOFT_NO_LAZY_BUFFER", "1"))] {
        for backend in ["--interpret", "--native"] {
            for falsifier in [
                ("LOFT_STRICT_STORES", "1"),
                ("LOFT_POISON", "1"),
                ("LOFT_NATIVE_LEAK_CHECK", "1"),
            ] {
                let mut env = vec![falsifier];
                env.extend(switch);
                let res = loft(&[backend, &src], &env);
                let out = String::from_utf8_lossy(&res.stdout);
                let err = String::from_utf8_lossy(&res.stderr);
                let label = format!("{backend} {switch:?} {falsifier:?}");
                assert!(
                    res.status.success(),
                    "{label}: exit {:?}\n{err}",
                    res.status
                );
                assert_eq!(out, EXPECTED.repeat(3), "{label}\n{err}");
                assert!(
                    !err.contains("not freed") && !err.contains("strict-store"),
                    "{label}: a store leaked or was read after its free:\n{err}"
                );
            }
        }
    }
}

#[test]
fn no_entry_mint_is_left_and_the_switch_restores_them() {
    let on_path = std::env::temp_dir().join("loft_lazy_buffer_on.rs");
    let off_path = std::env::temp_dir().join("loft_lazy_buffer_off.rs");
    let on = emit(&on_path, &[]);
    let off = emit(&off_path, &[("LOFT_NO_LAZY_BUFFER", "1")]);
    for (name, guards) in REWRITTEN {
        let (entry, guarded) = mints(body(&on, name));
        assert_eq!(entry, 0, "{name}: a buffer is still minted at entry");
        assert_eq!(
            guarded,
            guards,
            "{name}: guarded mints\n{}",
            body(&on, name)
        );
        let (entry_off, guarded_off) = mints(body(&off, name));
        assert!(
            entry_off >= 1,
            "{name}: the switch must mint at entry again"
        );
        assert_eq!(guarded_off, 0, "{name}: the switch leaves no guard");
    }
    // The early-return path of `f1` releases a buffer it never minted: the free stays, and
    // nothing mints in front of it.
    let f1 = body(&on, "n_f1");
    let ret = f1.find("return var___ret").expect("f1 returns early");
    assert!(
        !f1[..ret].contains("= OpDatabase("),
        "f1 mints before its early return:\n{f1}"
    );
    assert!(f1[..ret].contains("OpFreeRef(cell,var___ref_1"), "{f1}");
    let _ = std::fs::remove_file(&on_path);
    let _ = std::fs::remove_file(&off_path);
}

#[test]
fn the_interpreters_entry_init_is_the_sentinel() {
    let src = cells();
    let res = loft(&["introspect", &src], &[]);
    let text = String::from_utf8_lossy(&res.stdout);
    let start = text
        .find(".loft:n_f1(")
        .expect("the introspection lists n_f1's bytecode");
    let f1 = &text[start..];
    let end = [f1.find("\nbyte-code for"), f1.find("\nfn ")]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(f1.len());
    let f1 = &f1[..end];
    assert!(
        f1.contains("InitRefSentinel(var[") && f1.contains("RefIsNull("),
        "n_f1's buffer must start as the sentinel and mint behind the null test:\n{f1}"
    );
}
