// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-SplitTable` — the EMISSION pins.  A `parts = text.split(c)` bound to a local
//! and then only `len`'d, indexed or walked collects its pieces as `Vec<&str>` at the bind
//! and never calls `t_4text_split`; a parameter nothing writes is borrowed and every other
//! source is copied at the bind; the walk's loop variable is a borrowed `&str`; the raising
//! and the nullable element read take their own getters; the buffer the call would have
//! filled is never minted; a returned, appended, written, handed-away, copied or rebound
//! vector, a variable separator, a bind read outside its block and a generator keep the
//! vector; `LOFT_NO_SPLIT_TABLE=1` restores the vector everywhere.  The guard
//! (`tests/scripts/158-split-table.loft`) says the VALUES hold on both backends; this pins
//! what is emitted.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/158-split-table.loft";

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let out =
        std::env::temp_dir().join(format!("loft_split_table_{}_{tag}.rs", std::process::id()));
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_SPLIT_TABLE")
        .env_remove("LOFT_NO_WRAPPER_INLINE");
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

const TABLE: &str = "loft::codegen_runtime::lazy_split(";

#[test]
fn a_split_bound_to_a_name_is_a_table_and_builds_no_vector() {
    let rust = emit("table", &[]);
    // (function, tables in it): one per `x = ….split(<constant>)` whose reads the table
    // answers.
    for (name, tables) in [
        ("n_indexed", 1),
        ("n_count_first", 1),
        ("n_shape", 1),
        ("n_at", 1),
        ("n_second", 1),
        ("n_third", 1),
        ("n_s6", 1),
        ("n_s7", 1),
        ("n_grow", 1),
        ("n_s9", 3),
        ("n_s10", 1),
        ("n_head", 1),
        ("n_s12", 2),
        ("n_s13", 1),
        ("n_pick", 1),
        ("n_s16", 1),
        ("n_s18", 2),
        ("n_s21", 1),
    ] {
        let b = body(&rust, name);
        assert_eq!(
            count(b, ").collect() /* @FR-R-SplitTable */"),
            tables,
            "{name}: every admitted bind collects its pieces at the bind"
        );
        assert_eq!(
            count(b, TABLE),
            tables,
            "{name}: the pieces are the lazy split's"
        );
    }
    // Where every split of the function is a table, the call is gone.
    for name in [
        "n_indexed",
        "n_count_first",
        "n_shape",
        "n_at",
        "n_second",
        "n_third",
        "n_s6",
        "n_s7",
        "n_grow",
        "n_s10",
        "n_head",
        "n_s12",
        "n_s13",
        "n_pick",
        "n_s16",
        "n_s21",
    ] {
        assert!(
            !body(&rust, name).contains("t_4text_split("),
            "{name}: no vector is built for a table"
        );
    }
}

#[test]
fn every_reader_answers_from_the_table() {
    let rust = emit("readers", &[]);
    let indexed = body(&rust, "n_indexed");
    assert_eq!(
        count(indexed, ".len() as i64)"),
        2,
        "indexed: `len(parts)` twice, both the table's length"
    );
    assert_eq!(
        count(indexed, "split_table_get(&__st_"),
        1,
        "indexed: `parts[i]?` is the nullable slice at i"
    );
    assert!(
        !indexed.contains("vec_header(") && !indexed.contains("length_vector("),
        "indexed: no header is derived and no vector length read for a table"
    );
    // The walk through the name: its alias binds nothing, its element read is the slice,
    // its loop variable a borrowed `&str`, and its length test the table's length.
    let shape = body(&rust, "n_shape");
    assert!(
        shape.contains("() /* @FR-R-SplitTable walk of the table */")
            && shape.contains("let var_p: &str = ")
            && count(shape, "split_table_get(&__st_") == 1
            && count(shape, ".len() as i64)") == 2,
        "shape: `for p in parts` walks the table, `p` borrowed"
    );
    // The raising read of a bare `v[i]` binds its index first and raises what the op does.
    let third = body(&rust, "n_third");
    assert!(
        third.contains("split_table_get_or_raise(stores, &__st_") && third.contains(", __vi) }"),
        "third: `q: text? = parts[2]` untested is the raising read"
    );
    assert!(
        body(&rust, "n_second").contains("split_table_get(&__st_"),
        "second: the null-tested bind is the nullable read"
    );
    // Two tables in one function keep their own names (the loft variable's number), and
    // the walk's `#index` reads the other table.
    let s12 = body(&rust, "n_s12");
    assert!(
        count(s12, ": Vec<&str> = ") == 2
            && s12.contains("split_table_get(&__st_0, (var_k__index) as i64)")
            && s12.contains("split_table_get(&__st_2, (var_k__index) as i64)"),
        "s12: two tables, the walk of one indexing the other"
    );
}

#[test]
fn a_discharged_element_read_borrows_its_slice() {
    let rust = emit("discharge", &[]);
    // `parts[i]?` in a value position: the temp is the slice, the null test reads it bare,
    // and the block yields the slice — no `String` is built on either side.
    let indexed = body(&rust, "n_indexed");
    assert!(
        indexed.contains("let var___ncc_1: &str = loft::codegen_runtime::split_table_get(&__st_")
            && indexed.contains("_ret = if (((var___ncc_1) != loft::state::STRING_NULL) as u8) == 1 {&*(var___ncc_1)} else {&*(\"\")};")
            && !indexed.contains("_ret.to_string()")
            && !indexed.contains("var___ncc_1: String"),
        "indexed: the `?` temp borrows the slice and the block yields it"
    );
    // `parts[i] ?? d` into a local: the temp is the slice; the arm copies it into the
    // local, which owns its text.
    let at = body(&rust, "n_at");
    assert!(
        at.contains("let var___ncc_1: &str = ")
            && at.contains("(var___ncc_1).to_string()")
            && !at.contains("var___ncc_1.clone()"),
        "at: the `??` temp borrows the slice, the local takes a copy"
    );
    // A discharge of a VECTOR's element (no table) keeps the owned temp.
    let s17 = body(&rust, "n_s17");
    assert!(
        s17.contains("var___ncc_1: String = ") && !s17.contains("var___ncc_1: &str"),
        "s17: a discharge of a real vector's element still copies"
    );
    let off = emit("discharge_off", &[("LOFT_NO_TEXT_BORROW", "1")]);
    assert!(
        !off.contains(": &str = loft::codegen_runtime::split_table_get(")
            && body(&off, "n_indexed").contains("_ret.to_string()"),
        "LOFT_NO_TEXT_BORROW=1 keeps every discharge temp an owned String"
    );
}

#[test]
fn an_unwritten_parameter_is_borrowed_and_any_other_source_is_a_copy() {
    let rust = emit("source", &[]);
    for name in [
        "n_indexed",
        "n_count_first",
        "n_shape",
        "n_at",
        "n_second",
        "n_head",
    ] {
        let b = body(&rust, name);
        assert!(
            b.contains("lazy_split(&*(var_src), ") && !b.contains("__st_src_"),
            "{name}: a text parameter nothing writes is borrowed for the block"
        );
    }
    // A local written after the bind, a by-reference text the function writes, and the
    // expression sources of s9 (a field, a call, a format string) are each copied once.
    for (name, copies) in [("n_s7", 1), ("n_grow", 1), ("n_s9", 3), ("n_s10", 1)] {
        let b = body(&rust, name);
        assert_eq!(
            count(b, "let __st_src_"),
            copies,
            "{name}: a source the table cannot borrow is copied where the call stood"
        );
        assert!(
            !b.contains("lazy_split(&*("),
            "{name}: nothing here is a parameter the table may borrow"
        );
    }
}

#[test]
fn the_buffer_of_a_table_is_never_minted() {
    let rust = emit("buffer", &[]);
    let table = body(&rust, "n_indexed");
    assert!(
        table.contains("let mut var___ref_1: DbRef = DbRef::NULL;")
            && !table.contains("OpDatabase("),
        "indexed: the split's buffer stays the null sentinel"
    );
    let kept = emit("buffer_off", &[("LOFT_NO_SPLIT_TABLE", "1")]);
    assert!(
        body(&kept, "n_indexed").contains("var___ref_1 = OpDatabase("),
        "indexed: with the switch set the buffer is minted for the call"
    );
}

#[test]
fn every_other_use_keeps_the_vector() {
    let rust = emit("decline", &[]);
    for (name, kept) in [
        // returned
        ("n_returned", 1),
        // appended to, an element written, handed to `join`, copied, rebound (twice)
        ("n_s17", 6),
        // bound inside an `if`, read after it
        ("n_s19", 1),
    ] {
        let b = body(&rust, name);
        assert_eq!(
            (count(b, "t_4text_split("), count(b, TABLE)),
            (kept, 0),
            "{name}: every declined bind keeps its vector"
        );
    }
    // s18: the variable separator keeps its vector; the `rev` and the comprehension are
    // walks of a table.
    let s18 = body(&rust, "n_s18");
    assert_eq!(
        (count(s18, "t_4text_split("), count(s18, TABLE)),
        (1, 2),
        "s18: a separator that is not a constant keeps the vector; `rev` and a filter walk the table"
    );
    // s9's slice of a split reads the vector through another op.
    assert_eq!(
        count(body(&rust, "n_s9"), "t_4text_split("),
        1,
        "s9: the sliced split keeps its vector"
    );
    // A generator's table would live on the state machine; it keeps the vector.  Its
    // functions are emitted under the generator's own names, so the check is over the
    // whole file: exactly the binds counted above are tables, and `pieces` is not one.
    let admitted = 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 3 + 1 + 1 + 2 + 1 + 1 + 1 + 2 + 1;
    assert_eq!(
        count(&rust, ").collect() /* @FR-R-SplitTable */"),
        admitted,
        "pieces: a generator keeps the vector — no table beyond the admitted binds"
    );
}

#[test]
fn the_switch_restores_the_vector_everywhere() {
    // The embedded source text names the getter in its own comments, so the markers
    // searched for are the emitted forms.
    let rust = emit("switch", &[("LOFT_NO_SPLIT_TABLE", "1")]);
    assert!(
        !rust.contains("let __st_") && !rust.contains("split_table_get(&"),
        "LOFT_NO_SPLIT_TABLE=1 must emit no table"
    );
    let rust = emit("wrapper", &[("LOFT_NO_WRAPPER_INLINE", "1")]);
    assert!(
        !rust.contains("let __st_"),
        "with `len` kept a call no table is admitted: the table rides on (R-Wrapper)"
    );
}
