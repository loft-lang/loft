// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-SliceBorrow` — the EMISSION pins.  A text local bound to a value slice of a text
//! parameter (or of a local that itself borrows one) is bound as the `&str` the slice
//! answers — `let var_t: &str = OpGetTextSub(var_line, …)` — and read bare, with no
//! `String` built; a local that escapes (an append, a rebind to a built text, a link, a
//! `&text` argument, a tuple, a return), a slice of a `&text` or rebound parameter, a slice
//! of an owned local, a record FIELD bound from a slice, and a generator keep the copy;
//! `LOFT_NO_SLICE_BORROW=1` restores the copy for this clause while the sibling clauses of
//! `(R-TextBorrow)` stay on.  The guard (`tests/scripts/192-slice-borrow.loft`) says the
//! VALUES hold on both backends; this pins what is emitted.
use loft::file_access as fa;
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/192-slice-borrow.loft";

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let out =
        std::env::temp_dir().join(format!("loft_slice_borrow_{}_{tag}.rs", std::process::id()));
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-rust")
        .arg("--native-emit")
        .arg(&out)
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_SLICE_BORROW")
        .env_remove("LOFT_NO_TEXT_BORROW");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.output().expect("spawn loft --native-emit");
    assert!(
        fa::exists(&out),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    let rust = fa::read_to_string(&out).expect("read the emitted Rust");
    let _ = fa::remove_file(&out);
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

/// The slices that borrow, with the local each binds as a `&str`.
const BORROWS: [(&str, &str); 13] = [
    ("n_t1", "t"),
    ("n_t2", "head"),
    ("n_t2", "tail"),
    ("n_t2", "mid"),
    ("n_t3", "t"),
    ("n_t4", "a"),
    ("n_t4", "b"),
    ("n_t4", "c"),
    ("n_t4", "d"),
    ("n_t4", "e"),
    ("n_t5", "t"),
    ("n_t7", "t"),
    ("n_t11", "t"),
];

/// The functions whose slice-bound locals keep the copy, and why (the cell file names
/// each): an escape, a source that is not a plain parameter, a promoted return.
const COPIES: [&str; 6] = ["n_t6", "n_t7r", "n_t8", "n_t9", "n_t9b", "n_t10"];

#[test]
fn a_slice_of_a_parameter_binds_a_borrow_and_reads_bare() {
    let rust = emit("borrow", &[]);
    for (name, var) in BORROWS {
        let b = body(&rust, name);
        assert_eq!(
            count(b, &format!("var_{var}: &str = OpGetTextSub(")),
            1,
            "{name}: `{var}` is bound as the `&str` the slice answers"
        );
        assert_eq!(
            count(b, &format!("let mut var_{var}: String")),
            0,
            "{name}: `{var}` is not bound as an owned String"
        );
        assert_eq!(
            count(b, &format!("&var_{var}")),
            0,
            "{name}: `{var}` reads bare"
        );
    }
    // `mid` is a slice of `head`, itself a borrowed slice: the source is the borrowed local.
    assert_eq!(
        count(body(&rust, "n_t2"), "OpGetTextSub(var_head, 1_i64, 2_i64)"),
        1,
        "t2: a slice of a borrowed slice reads the borrow"
    );
}

#[test]
fn a_slice_that_escapes_or_has_no_borrowable_source_keeps_the_copy() {
    let rust = emit("copies", &[]);
    for name in COPIES {
        let b = body(&rust, name);
        assert_eq!(
            count(b, ": &str = OpGetTextSub("),
            0,
            "{name}: no slice-bound local is borrowed"
        );
        assert!(
            count(b, "OpGetTextSub(") >= 1 && count(b, ".to_string()") >= 1,
            "{name}: the slice is copied into an owned String"
        );
    }
    // A record FIELD bound from a slice is a store write, which copies whatever the
    // local beside it does.
    let b = body(&rust, "n_t7");
    assert_eq!(
        count(b, "set_str("),
        1,
        "t7: the field is written into the store"
    );
    // A generator keeps the copy: its locals live in the coroutine's struct.
    assert_eq!(
        count(
            &rust,
            "self.var_t = (OpGetTextSub(var_line, 0_i64, 2_i64)).to_string()"
        ),
        1,
        "gen: the yielded slice is copied into the coroutine state"
    );
}

#[test]
fn the_switch_restores_the_copy_and_leaves_the_sibling_clauses_on() {
    let rust = emit("switch", &[("LOFT_NO_SLICE_BORROW", "1")]);
    for (name, var) in BORROWS {
        let b = body(&rust, name);
        assert_eq!(
            count(b, &format!("var_{var}: &str = OpGetTextSub(")),
            0,
            "{name}: `{var}` is copied under LOFT_NO_SLICE_BORROW=1"
        );
    }
    let b = body(&rust, "n_t3");
    assert_eq!(
        count(b, "let mut var_t: String"),
        1,
        "t3: `t` is an owned String"
    );
    // The walk clause of `(R-TextBorrow)` is untouched by the switch: the stdlib's join
    // still borrows its element.
    let j = body(&rust, "t_6vector_join");
    assert_eq!(
        count(j, "let var_p: &str = "),
        1,
        "join still binds its element as a `&str`"
    );
}
