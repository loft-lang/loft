// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-TextBorrow` — the EMISSION pins.  The loop variable of `for p in vector<text>`
//! (and of a lazily split text) is bound as a `&str` and read bare, with no `String` built
//! per element; the walk holds the vector's header and length; a body that writes a store,
//! rebinds `p`, links to it, hands it to a `&text` parameter, tuples it or returns it keeps
//! the copy; `LOFT_NO_TEXT_BORROW=1` restores the copy everywhere; `LOFT_HOIST_VERIFY=1`
//! re-reads the element at the walk's release.  The guard
//! (`tests/scripts/158-text-borrow.loft`) says the VALUES hold on both backends; this pins
//! what is emitted.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/158-text-borrow.loft";

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let out =
        std::env::temp_dir().join(format!("loft_text_borrow_{}_{tag}.rs", std::process::id()));
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_TEXT_BORROW")
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

fn count(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

/// The walks that borrow, with the loop variable each binds as a `&str`.
const BORROWS: [(&str, &str); 7] = [
    ("n_t1", "p"),
    ("n_t2", "p"),
    ("n_t3", "p"),
    ("n_t4", "line"),
    ("n_t4", "w"),
    ("n_t15", "p"),
    ("n_t16", "p"),
];

/// The walks that keep the copy, and why (the cell file names each).
const COPIES: [&str; 10] = [
    "n_t6", "n_t7", "n_t8", "n_t9", "n_t10", "n_t11", "n_t12", "n_t13", "n_t17", "n_t18",
];

#[test]
fn a_walk_of_texts_binds_its_element_as_a_borrow_and_reads_it_bare() {
    let rust = emit("borrow", &[]);
    for (name, var) in BORROWS {
        let b = body(&rust, name);
        assert_eq!(
            count(b, &format!("let var_{var}: &str = ")),
            1,
            "{name}: `{var}` is bound as a `&str`"
        );
        assert_eq!(
            count(b, &format!("let mut var_{var} = ")),
            0,
            "{name}: `{var}` is not bound as an owned String"
        );
        assert_eq!(
            count(b, &format!("&var_{var}")),
            0,
            "{name}: `{var}` reads bare"
        );
    }
    // The standard library's join, the row the rule was priced on.
    let j = body(&rust, "t_6vector_join");
    assert_eq!(
        count(j, "let var_p: &str = "),
        1,
        "join binds its element as a `&str`"
    );
    assert_eq!(
        count(j, ".to_string()"),
        1,
        "join converts only its empty-text seed"
    );
}

#[test]
fn a_vector_walk_holds_its_header_and_length() {
    let rust = emit("header", &[]);
    for name in ["n_t1", "n_t2", "n_t3", "n_t16"] {
        let b = body(&rust, name);
        assert!(
            count(b, "vector::vec_header(") >= 1,
            "{name}: the walked vector's header is derived once"
        );
        assert_eq!(
            count(b, "vector::length_vector("),
            0,
            "{name}: the length test reads the held header"
        );
    }
}

#[test]
fn a_walk_that_writes_a_store_or_lets_its_variable_escape_keeps_the_copy() {
    let rust = emit("copies", &[]);
    for name in COPIES {
        let b = body(&rust, name);
        assert_eq!(
            count(b, ": &str = {"),
            0,
            "{name}: the loop variable is not borrowed"
        );
    }
}

/// `@FR-R-Base`'s text clause — the walk's element read goes through the held base and the
/// store's span derived once beside it; `LOFT_NO_TEXT_BASE=1` resolves the store per element.
#[test]
fn a_text_element_is_read_through_the_held_base_and_span() {
    let rust = emit("text_base", &[]);
    // t1's walk (`for w in words`) holds a header and a base: the element read is one
    // `text_elem_at` over them, with the span bound beside the base.
    let t1 = body(&rust, "n_t1");
    assert!(
        t1.contains("vector::text_span_of(&__vh_")
            && t1.contains("vector::text_elem_at::<false>(&__vh_")
            && !t1.contains("get_str("),
        "t1: the walk's element read slices off the held span:\n{t1}"
    );
    let off = emit("text_base_off", &[("LOFT_NO_TEXT_BASE", "1")]);
    let t1 = body(&off, "n_t1");
    assert!(
        !t1.contains("text_elem_at") && t1.contains("get_str("),
        "LOFT_NO_TEXT_BASE=1: the element read resolves the store per element again"
    );
    let verify = emit("text_base_verify", &[("LOFT_HOIST_VERIFY", "1")]);
    assert!(
        body(&verify, "n_t1").contains("vector::text_elem_at::<true>(&__vh_"),
        "LOFT_HOIST_VERIFY=1 picks the checking monomorphisation"
    );
}

#[test]
fn the_switch_restores_the_copy_everywhere() {
    let rust = emit("off", &[("LOFT_NO_TEXT_BORROW", "1")]);
    assert_eq!(
        count(&rust, ": &str = { //iter next"),
        0,
        "LOFT_NO_TEXT_BORROW=1: no walk borrows its element"
    );
    let j = body(&rust, "t_6vector_join");
    assert_eq!(
        count(j, "let mut var_p = "),
        1,
        "join copies its element again"
    );
}

#[test]
fn the_checking_form_rereads_the_element_at_the_walks_release() {
    let rust = emit("verify", &[("LOFT_HOIST_VERIFY", "1")]);
    for (name, var) in BORROWS {
        let b = body(&rust, name);
        // A lazy split's piece has no store to re-read: nothing is verified there.
        let expect = if name == "n_t4" { 0 } else { 3 };
        assert_eq!(
            count(b, &format!("vector::text_borrow_verify(var_{var}, ")),
            expect,
            "{name}: `{var}` is re-read at each of the walk's three releases"
        );
    }
    let off = emit("plain", &[]);
    assert_eq!(
        count(&off, "text_borrow_verify("),
        0,
        "the default emits no check"
    );
}
