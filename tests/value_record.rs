// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-aa (`@FR-R-ValueRecord`) — a NO-HEAP RECORD returned BY VALUE: an admitted
//! function returns its record's fields as a Rust tuple (in registers, no return buffer,
//! no store round-trip) exactly as a TUPLE return already did, and its call sites read
//! tuple elements instead of a store.  The cell corpus
//! (`bytecode-comparisons/V-aa-value-record-cells.loft`) says the VALUES hold; this pins
//! the EMISSION — which functions are admitted, and that every declining shape (a stored
//! record, a heap field, too many fields, a passed-on or returned-onward result, a
//! forwarding body) keeps its buffer — and the switch: `LOFT_NO_VALUE_RECORD=1` restores the
//! return buffer for every admitted function (default-on since @PLN157 § V-ah stage 1 gave the
//! call-site gate the emitter's own fn-ref arm scan).
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-aa-value-record-cells.loft";

/// `(function, returns a tuple?)` — the predictions written beside the cells.
const EXPECTED: &[(&str, bool)] = &[
    ("n_mk_smp", true),         // c1: every site reads fields
    ("n_mk_mixed", true),       // c2: mixed scalar field types
    ("n_mk_smp_kept", true), // c3: a site appends it — a copy FROM the tuple, materialised (§ V-ah)
    ("n_mk_owner", false),   // c4: a `text` field owns heap
    ("n_mk_big", false),     // c5: past the field-count bound
    ("n_mk_smp_passed", false), // c6: the result is passed on
    ("n_mk_cond", false),    // c7: the arms build INTO one shared buffer local and the tail
    // RETURNS that local — an owned tail, which the value form would have to mint (declines)
    ("n_mk_inner", true), // c8: read field-wise inside an admitted caller's own build
    ("n_mk_ret", true),   // c9: its caller forwards it — a forwarding tail (§ V-ah)
    ("n_pass_through", true), // c9: the forwarding body is admitted with its callee
];

/// The § V-ah tails (`bytecode-comparisons/V-ah-value-tail-cells.loft`): a forwarding
/// tail, a selecting tail, the branch-bound local and the builder delivered into a
/// destination, each beside the shape that must still decline.
const TAIL_CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-ah-value-tail-cells.loft";
const TAIL_EXPECTED: &[(&str, bool)] = &[
    ("n_pt", true),         // t1: delivered into a push slot — the tuple is materialised
    ("n_half", true),       // t2: a forwarding tail
    ("n_mk_kept", true),    // t3: its record is appended — a copy FROM a value local materialises
    ("n_fwd_kept", true),   // t3: forwards an admitted callee
    ("n_pt5", false),       // t5: forwarded by a body a mixed-arm branch declines
    ("n_half5", false),     // t5: a mixed-arm branch needs the record
    ("n_sel", true),        // t6: a selecting tail (a view of a `const` parameter)
    ("n_ctrl", true),       // t7: a selecting tail with an early return
    ("n_half_chord", true), // t8: selecting calls without their brackets, then a forward
    ("n_own", false),       // t9: an OWNED tail — the value form would mint per call
    ("n_pt10", false),      // t10: a discharge arm joined with a variable
    ("n_maybe", false),     // t10: a nullable result has no tuple
    ("n_ping", true),       // t11: a branch of two admitted calls, mutually recursive
    ("n_pong", true),       // t11: the forwarding half of the pair
    ("n_pt12", false),      // t12: delivered into a field / element through a `__lift_` temp
    ("n_pt13", false),      // t13: its result is passed as an argument (`disc(given)`)
    ("n_disc", false),      // t13: a JOIN tail — a view on one arm, a mint on the other
    ("t_2Pt_gpick", true),  // t14: a generic instance's statement-join selecting tail
    ("t_4Coin_gmax", true), // t14: the same over a ONE-field record — the `(i64,)` tuple
    ("n_sel15", true),      // t15: a lifted selecting tail over by-value parameters
    ("n_sel15c", true),     // t15: the same over `const` parameters
];

/// The § V-an chains (`bytecode-comparisons/V-an-chain-cells.loft`): the parser's
/// `return f(…)` lowering hands the callee the function's OWN return buffer and returns
/// it, and a local the parser promotes INTO that buffer is the same variable under the
/// local's name — both are the phantom parameter bound from a value shape, which the value
/// form takes as a value local (`hoist::own_retbuf`).
const CHAIN_CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-an-chain-cells.loft";
const CHAIN_EXPECTED: &[(&str, bool)] = &[
    ("n_scan_fail", true),   // every site is a chain or a tail of an admitted body
    ("n_read_uint", true),   // n1: two early `return scan_fail()` chains before an Object tail
    ("n_hit", true),         // n2: chained from inside a `while` body
    ("n_find_eq", true),     // n2: the chain in the loop, the tail forwards `scan_fail`
    ("n_lo", true),          // n3: the first of two chains
    ("n_hi", true),          // n3: the second
    ("n_pick", true),        // n3: two chains to different callees, then an Object tail
    ("n_count_down", true),  // n4: a recursive chain
    ("n_bump", true),        // n5: chained with a record LITERAL argument
    ("n_via_obj", true),     // n5: the chain whose sibling buffer holds that literal
    ("n_mk6", false),        // n6: chained from a body that keeps its buffer
    ("n_keep6", false),      // n6: an owned tail after a field write — not a value shape
    ("n_mk7", false),        // n7: passed on as a record by `sum7`
    ("n_fwd7", false),       // n7: its chain forwards a declined callee
    ("n_read_num8", true),   // n8: bound into the caller's PROMOTED local
    ("n_at_width", true),    // n8: the promoted local is the phantom, read as the tuple
    ("n_scan3_fail", true),  // n9: forwarded by `read_rgb_at`
    ("n_read_rgb_at", true), // n9: bound to value locals of ANOTHER type in `read_colour_pair`
    ("n_read_colour_pair", true), // n9: its early returns are Objects carrying their own frees + return
];

fn emit(src: &Path, out: &Path, env: &[(&str, &str)]) -> String {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(out)
        .arg(src)
        .env("LOFT_TIMEOUT", "120")
        // The default build: § V-aa is default-on since its call-site gate reads the fn-ref
        // arm scan from the emitter's own home (@PLN157 § V-ah stage 1).  A test that wants
        // the return buffer back passes the switch through `env`.
        .env_remove("LOFT_NO_VALUE_RECORD");
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
    std::fs::read_to_string(out).expect("read the emitted Rust")
}

/// Does `rust` declare `name` with a TUPLE return (the value form)?
fn returns_tuple(rust: &str, name: &str) -> bool {
    rust.lines()
        .find(|l| l.starts_with(&format!("fn {name}(")))
        .map(|l| {
            let ret = l.split("->").nth(1).unwrap_or("");
            ret.trim_start().starts_with('(')
        })
        .unwrap_or_else(|| panic!("{name} was not emitted"))
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

#[test]
fn each_cell_returns_by_value_exactly_where_predicted() {
    let out = std::env::temp_dir().join("loft_value_record_on.rs");
    let rust = emit(&cells(), &out, &[]);
    for (name, want) in EXPECTED {
        assert_eq!(
            returns_tuple(&rust, name),
            *want,
            "{name}: returns its record by value"
        );
    }
    // An admitted callee takes no return buffer, and its call site passes none.
    assert!(
        !rust.contains(
            "fn n_mk_smp(cell: &std::cell::UnsafeCell<Stores>, mut var_v: f64, mut var___retbuf"
        ),
        "an admitted fn must not keep its __retbuf parameter"
    );
    let _ = std::fs::remove_file(&out);
}

#[test]
fn each_tail_cell_returns_by_value_exactly_where_predicted() {
    let out = std::env::temp_dir().join("loft_value_tail_on.rs");
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(TAIL_CELLS);
    let rust = emit(&src, &out, &[]);
    for (name, want) in TAIL_EXPECTED {
        assert_eq!(
            returns_tuple(&rust, name),
            *want,
            "{name}: returns its record by value"
        );
    }
    // The forwarding tail's own buffer parameter is gone from its signature, whatever the
    // parser named it (`__ref_1` here, not `__retbuf`), and no site passes one.
    assert!(
        rust.contains("fn n_half(cell: &std::cell::UnsafeCell<Stores>, mut var_a: f64, mut var_b: f64) -> (f64, f64)"),
        "the forwarding tail keeps only its declared parameters"
    );
    assert!(
        !rust.contains("n_half(cell, 7_f64, 9_f64, var_"),
        "a call to the forwarding tail passes no buffer"
    );
    // The selecting call site carries no protect bracket: the borrow it guarded is gone.
    let hc = rust
        .split("fn n_half_chord(")
        .nth(1)
        .and_then(|s| s.split("\nfn ").next())
        .expect("n_half_chord emitted");
    assert!(
        !hc.contains("n_protect_store_frees"),
        "a selecting callee's tuple needs no borrow bracket"
    );
    // The join buffers of a branch that binds a TUPLE are dead: minted by nothing, freed by
    // nothing (t4); a branch that keeps a record arm still mints its buffer (t5).
    let body_of = |name: &str| {
        rust.split(&format!("\nfn {name}("))
            .nth(1)
            .and_then(|s| s.split("\nfn ").next())
            .unwrap_or_else(|| panic!("{name} emitted"))
            .to_string()
    };
    let t4 = body_of("n_t4");
    assert!(
        !t4.contains("OpDatabase(cell,var___ref"),
        "t4's join buffers are dead and must not be minted"
    );
    assert!(
        !t4.contains("OpFreeRef(cell,var___ref"),
        "t4's join buffers are dead and must not be freed"
    );
    // (A mixed-arm branch delivers its buffer lazily at the call, so its receipt is the
    // buffer ARGUMENT the declined callee still takes and the free at scope exit.)
    let t5 = body_of("n_t5");
    assert!(
        t5.contains("n_half5(cell, 2_f64, 6_f64, var___ref_1)"),
        "t5's declined callee still takes its buffer"
    );
    assert!(
        t5.contains("OpFreeRef(cell,var___ref_1"),
        "t5's live buffer is still freed"
    );
    // A generic instance's selecting tail binds its join local from a parameter's VIEW in
    // each statement arm (t14): the value form reads the view's fields into the tuple and
    // neither mints a store for the join nor deep-copies into it.
    for name in ["t_2Pt_gpick", "t_4Coin_gmax"] {
        let body = body_of(name);
        assert!(
            !body.contains("OpDatabase(cell,") && !body.contains("OpCopyRecord(cell,"),
            "{name}'s join local is a tuple: no mint, no copy"
        );
        assert!(
            body.contains("var____ret_join_1 = ("),
            "{name}'s join local is bound to the view's field tuple"
        );
    }
    // The source-level selecting tail lifts each arm's parameter read into a `__lift_N`
    // copy (t15): the lift is a value local bound to the parameter's field tuple, so the
    // arm mints nothing and the store the record form hands up is never made.
    for name in ["n_sel15", "n_sel15c"] {
        let body = body_of(name);
        assert!(
            !body.contains("OpDatabase(cell,") && !body.contains("OpCopyRecord(cell,"),
            "{name}'s lifts are tuples: no mint, no copy"
        );
        assert!(
            body.contains("var___lift_1 = (") && body.contains("let mut var___lift_1: (f64, f64)"),
            "{name}'s lift is bound to the view's field tuple"
        );
    }
    let _ = std::fs::remove_file(&out);
}

#[test]
fn each_chain_cell_returns_by_value_exactly_where_predicted() {
    let out = std::env::temp_dir().join("loft_value_chain_on.rs");
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CHAIN_CELLS);
    let rust = emit(&src, &out, &[]);
    for (name, want) in CHAIN_EXPECTED {
        assert_eq!(
            returns_tuple(&rust, name),
            *want,
            "{name}: returns its record by value"
        );
    }
    let body_of = |name: &str| {
        rust.split(&format!("\nfn {name}("))
            .nth(1)
            .and_then(|s| s.split("\nfn ").next())
            .unwrap_or_else(|| panic!("{name} emitted"))
            .to_string()
    };
    // The chain's assignment lands in the phantom, declared at its tuple's zero and bound
    // from the callee's tuple call — no buffer argument, no mint, no copy.
    for name in ["n_read_uint", "n_pick", "n_count_down"] {
        let body = body_of(name);
        assert!(
            body.contains("let mut var___ref_1: (bool, i64, f64) = Default::default();"),
            "{name} declares its phantom as the tuple"
        );
        assert!(
            !body.contains("OpDatabase(cell,") && !body.contains("OpCopyRecord(cell,"),
            "{name} mints and copies nothing"
        );
    }
    // The promoted local IS the phantom: read field-wise off the tuple, returned whole.
    let body = body_of("n_at_width");
    assert!(
        body.contains("let mut var_n: (bool, i64, f64) = Default::default();")
            && body.contains("var_n = n_read_num8(cell, ")
            && body.contains("var_n.0"),
        "n_at_width's promoted local is its tuple"
    );
    // An `Object` that returns what it builds: the tuple first, then the return.
    let body = body_of("n_read_colour_pair");
    assert!(
        body.contains("let __obj = (") && body.contains("return __obj"),
        "n_read_colour_pair's early returns are tuples returned from their own block"
    );
    assert!(
        !body.contains("OpDatabase(cell,") && !body.contains("OpCopyRecord(cell,"),
        "n_read_colour_pair mints and copies nothing"
    );
    let _ = std::fs::remove_file(&out);
}

#[test]
fn the_switch_restores_every_return_buffer() {
    let out = std::env::temp_dir().join("loft_value_record_off.rs");
    // `LOFT_NO_VALUE_RECORD=1` is the bisect step for a wrong field out of a
    // record-returning call on native: every admitted function keeps its buffer again.
    let rust = emit(&cells(), &out, &[("LOFT_NO_VALUE_RECORD", "1")]);
    for (name, _) in EXPECTED {
        assert!(
            !returns_tuple(&rust, name),
            "{name}: LOFT_NO_VALUE_RECORD=1 must restore its return buffer"
        );
    }
    let _ = std::fs::remove_file(&out);
}
