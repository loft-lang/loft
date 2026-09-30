// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-LazySplit` — the EMISSION pins.  A `for piece in text.split(c)` loop iterates
//! `codegen_runtime::lazy_split` and never calls `t_4text_split`; a parameter nothing
//! writes is borrowed and every other source is iterated as a copy; the buffer the call
//! would have filled is never minted; a variable separator, a `rev` and a generator keep
//! the vector; `LOFT_NO_LAZY_SPLIT=1` restores the vector everywhere.  The guard
//! (`tests/scripts/157-lazy-split.loft`) says the VALUES hold on both backends; this pins
//! what is emitted.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/157-lazy-split.loft";

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let out = std::env::temp_dir().join(format!("loft_lazy_split_{}_{tag}.rs", std::process::id()));
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_LAZY_SPLIT");
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

/// The emitted body of one function, up to the next top-level item.  Not only the next `fn`:
/// a lazy generator is emitted as a `struct` and its `impl` (loft#1586), and a body sliced to
/// the next `fn` took in the generator declared after it.
fn body<'a>(rust: &'a str, name: &str) -> &'a str {
    let start = rust
        .find(&format!("\nfn {name}("))
        .unwrap_or_else(|| panic!("{name} was not emitted"));
    let rest = &rust[start + 1..];
    let end = ["\nfn ", "\nstruct ", "\nimpl ", "\nenum "]
        .iter()
        .filter_map(|item| rest[3..].find(item))
        .min()
        .map_or(rest.len(), |i| i + 3);
    &rest[..end]
}

fn count(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

#[test]
fn a_loop_over_a_split_iterates_the_text_and_builds_no_vector() {
    let rust = emit("lazy", &[]);
    // (function, lazy loops in it): one per `for … in ….split(<constant>)`.
    for (name, loops) in [
        ("n_join_of", 1),
        ("n_count_of", 1),
        ("n_null_piece", 1),
        ("n_shape_of", 1),
        ("n_s4", 2),
        ("n_s5", 1),
        ("n_s6", 1),
        ("n_first_long", 1),
        ("n_s8", 2),
        ("n_s9", 1),
        ("n_grow", 1),
        ("n_s11", 5),
        ("n_s12", 1),
        ("n_s16", 3),
        ("n_s19", 1),
        ("n_s20", 2),
    ] {
        let b = body(&rust, name);
        assert_eq!(
            count(b, "loft::codegen_runtime::lazy_split("),
            loops,
            "{name}: every loop over a constant split is lazy"
        );
        assert_eq!(
            count(b, ".next() { Some(__piece) => __piece"),
            loops,
            "{name}: each lazy loop reads its piece from the iterator"
        );
        assert!(
            !b.contains("t_4text_split("),
            "{name}: no vector is built for a lazy loop"
        );
    }
}

#[test]
fn an_unwritten_parameter_is_borrowed_and_any_other_source_is_a_copy() {
    let rust = emit("source", &[]);
    for name in ["n_join_of", "n_count_of", "n_shape_of", "n_first_long"] {
        let b = body(&rust, name);
        assert!(
            b.contains("lazy_split(&*(var_src), ") && !b.contains("__ls_src_"),
            "{name}: a text parameter nothing writes is borrowed for the loop"
        );
    }
    // A local the body writes, a by-reference text, and the expression sources of s11
    // (a call, a field, a slice, a format string, a `trim`) are each iterated as a copy.
    for (name, copies) in [("n_s9", 1), ("n_grow", 1), ("n_s11", 5), ("n_s8", 2)] {
        let b = body(&rust, name);
        assert_eq!(
            count(b, ": String = ("),
            copies,
            "{name}: a source the loop cannot borrow is copied where the call stood"
        );
        assert!(
            !b.contains("lazy_split(&*("),
            "{name}: nothing here is a parameter the loop may borrow"
        );
    }
}

#[test]
fn the_buffer_of_a_lazy_split_is_never_minted() {
    let rust = emit("buffer", &[]);
    let lazy = body(&rust, "n_count_of");
    assert!(
        lazy.contains("let mut var___ref_1: DbRef = DbRef::NULL;") && !lazy.contains("OpDatabase("),
        "count_of: the split's buffer stays the null sentinel"
    );
    let kept = emit("buffer_off", &[("LOFT_NO_LAZY_SPLIT", "1")]);
    assert!(
        body(&kept, "n_count_of").contains("var___ref_1 = OpDatabase("),
        "count_of: with the switch set the buffer is minted for the call"
    );
}

#[test]
fn a_variable_separator_a_rev_and_a_generator_keep_the_vector() {
    let rust = emit("decline", &[]);
    let by = body(&rust, "n_by");
    assert!(
        by.contains("t_4text_split(") && !by.contains("lazy_split("),
        "by: a separator that is not a constant keeps the vector"
    );
    // s17 is a filtered loop (lazy) and a `rev` over a split (the vector).
    let s17 = body(&rust, "n_s17");
    assert_eq!(
        (count(s17, "lazy_split("), count(s17, "t_4text_split(")),
        (1, 1),
        "s17: the filter is lazy, the `rev` keeps its vector"
    );
    assert!(
        !rust.contains("fn n_pieces") || !body(&rust, "n_pieces").contains("lazy_split("),
        "pieces: a generator's loop is re-entered across `next` and keeps the vector"
    );
    // s21 discharges a nullable text twice: `(src ?? "").split(c)` is a loop over a call
    // (lazy, a copy of the expression); `src.split(c)?` binds the vector through the
    // discharge, which is not the plain bind, and keeps it.
    let s21 = body(&rust, "n_opt_shape");
    assert_eq!(
        (count(s21, "lazy_split("), count(s21, "t_4text_split(")),
        (1, 1),
        "opt_shape: the discharged VECTOR keeps its vector"
    );
    // s14 binds the vector to a name first: not a loop over a call, so not this rule's —
    // it is `(R-SplitTable)`'s (`tests/split_table.rs`), whose table collects the same
    // pieces at the bind and answers its length and its index.
    let s14 = body(&rust, "n_s14");
    assert!(
        !s14.contains("__ls_done_") && s14.contains(").collect() /* @FR-R-SplitTable */"),
        "s14: a split bound to a local is a table, not a lazy loop"
    );
}

#[test]
fn the_switch_restores_the_vector_everywhere() {
    let rust = emit("switch", &[("LOFT_NO_LAZY_SPLIT", "1")]);
    // The iterator itself stays in use: `(R-SplitTable)` collects it for s14's table, under
    // its own switch.  What this switch removes is the lazy LOOP form.
    assert!(
        !rust.contains("let mut __ls_") && !rust.contains("__ls_done_"),
        "LOFT_NO_LAZY_SPLIT=1 must emit no lazy split loop"
    );
}
