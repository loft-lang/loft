// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-ak (`@FR-R-Base`) and § V-aj (`@FR-R-Counter`, `@FR-R-LitDiv`) — the EMISSION
//! pins.  A growth-free loop binds an element base beside each hoisted header and reads and
//! writes through it; a loop that pushes binds none; a growth-free inner loop under a pushing
//! outer one binds a base of the HELD header for its own extent; a `?`-discharged record
//! element (a null-discharge buffer, § V-ad) declines the base; `LOFT_NO_VECTOR_BASE=1`
//! restores the header-only form everywhere.  A counted range's counter steps through the
//! non-null add, and a division by a literal is one sentinel test and a divide.  The cell
//! corpus (`bytecode-comparisons/V-ak-vector-base-cells.loft`) says the VALUES hold on both
//! backends; this pins what is emitted.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-ak-vector-base-cells.loft";

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    // One file per TEST: the tests run on threads of one process, so a name keyed on the
    // pid alone made them delete each other's emission.
    let out =
        std::env::temp_dir().join(format!("loft_vector_base_{}_{tag}.rs", std::process::id()));
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_VECTOR_BASE")
        .env_remove("LOFT_NO_NN_FAST");
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

#[test]
fn a_growth_free_loop_reads_and_writes_through_the_base() {
    let rust = emit("growth_free", &[]);
    let b1 = body(&rust, "n_b1");
    assert!(
        b1.contains("let __vb_") && b1.contains("vector::vec_base(&__vh_"),
        "b1 binds an element base beside its headers"
    );
    assert!(
        b1.contains("vector::get_elem_at::<i64, false>(&__vh_")
            && !b1.contains("get_elem_hoisted::<"),
        "b1's reads go through the base"
    );
    assert!(
        b1.contains("stores.vec_set_at::<i64, false>(&__vh_"),
        "b1's in-place write goes through the base"
    );
    let b5 = body(&rust, "n_b5");
    assert!(
        b5.contains("get_elem_at::<"),
        "b5's store-free callee does not cost the base"
    );
    let b6 = body(&rust, "n_b6");
    assert!(b6.contains("get_elem_at::<"), "b6 reads through the base");
}

#[test]
fn a_growing_loop_binds_no_base_and_an_inner_growth_free_loop_binds_one_of_the_held_header() {
    let rust = emit("growing", &[]);
    let b2 = body(&rust, "n_b2");
    assert!(
        !b2.contains("__vb_"),
        "b2 pushes, so it binds no base: {b2}"
    );
    let b3 = body(&rust, "n_b3");
    assert!(
        b3.contains("element base of the held header"),
        "b3's inner loop binds a base of the header its pushing outer loop holds"
    );
    assert!(
        b3.contains("get_elem_at::<"),
        "b3's inner reads go through that base"
    );
    // b4 mints a null-discharge buffer (§ V-ad): a FRESH store, or a clear of the buffer's
    // own — neither moves an element a base addresses, so since 2026-09-18 the loop binds
    // its base (before, the mint counted as a growth and the loop held headers alone).
    // The element itself is a RECORD, so its reads are not fused element loads: the base is
    // bound, and the discharged view `p` reads its fields through its own address
    // (`@FR-R-RecPtr`, `tests/record_ptr.rs`).
    let b4 = body(&rust, "n_b4");
    assert!(
        b4.contains("§ V-ak element base") && b4.contains("vector::rec_get::<i64>(__pa_"),
        "b4's discharge-buffer mint must not keep the loop off its base:\n{b4}"
    );
}

#[test]
fn the_switch_restores_the_header_only_form() {
    let rust = emit("switch", &[("LOFT_NO_VECTOR_BASE", "1")]);
    assert!(
        !rust.contains("__vb_"),
        "LOFT_NO_VECTOR_BASE=1 binds no base anywhere"
    );
    assert!(
        body(&rust, "n_b1").contains("get_elem_hoisted::<"),
        "the header-only read is back"
    );
}

#[test]
fn a_counted_range_steps_non_null_and_a_literal_division_is_one_test() {
    let rust = emit("range", &[]);
    let b7 = body(&rust, "n_b7");
    // The step was `ops::op_add_long_nn` until loft#1558 made counted-range counters
    // ranged: `b7`'s index runs -1..=4, so `+ 1` cannot fault and the step is the
    // processor's operator with no sentinel test at all.  Strictly better than the
    // non-null helper this asserted, and the helper would now be a REGRESSION here.
    assert!(
        b7.contains("var_i__index = ((var_i__index).wrapping_add(1_i64));"),
        "the counter steps through the plain add: {b7}"
    );
    assert!(
        b7.contains("if _d == i64::MIN { i64::MIN } else { _d / (255_i64) }"),
        "the division by a literal is one sentinel test and a divide: {b7}"
    );
    assert!(
        !b7.contains("op_div_int_nullable"),
        "the guarded template is gone"
    );
    let b8 = body(&rust, "n_b8");
    // As with b7, the step is plain since loft#1558 ranged counted-range counters; the
    // non-null helper here would now be a regression.
    assert!(
        b8.contains("var_i__index = ((var_i__index).wrapping_add(1_i64));"),
        "an inclusive range to a literal end steps through the plain add: {b8}"
    );
    let off = emit("range_off", &[("LOFT_NO_NN_FAST", "1")]);
    assert!(
        body(&off, "n_b7").contains("op_div_int_nullable")
            || body(&off, "n_b7").contains("op_div_long_nullable"),
        "LOFT_NO_NN_FAST=1 restores the guarded division"
    );
}
