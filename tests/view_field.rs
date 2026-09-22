// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN164 C5 (`@FR-O-ViewField`, `@FR-R-ValueRecord`) — a returned record's heap field as a
//! VIEW LEAF: the vector is delivered as a REFERENCE to the place it already lives in, so the
//! record's store and the deep copy into it are gone.
//!
//! `tests/scripts/164-view-field.loft` carries the cells and says the VALUES do not move; the
//! corpus runner runs it with the unit off, which is the record form the values are hand-
//! computed from.  This file carries the structural half, because the values cannot falsify an
//! admission: the copy the record form makes is correct, so every cell passes either way.  What
//! changes is the emitted signature and what the site reads, and that is what is pinned here —
//! together with every DECLINE, so one that quietly stops declining shows up as a failure here
//! rather than as a wrong answer in a consumer.
use std::path::{Path, PathBuf};
use std::process::Command;

const GUARD: &str = "tests/scripts/164-view-field.loft";
/// @PLN164 E-1 — the forward: a function that keeps its record returning an admitted
/// callee's answer writes the tuple into its own buffer.
const FORWARD: &str = "tests/scripts/164-forward-tuple.loft";

fn loft() -> Command {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_VIEW_FIELD")
        .env_remove("LOFT_NO_FORWARD_TUPLE");
    cmd
}

/// The generated Rust for the guard, with `env` applied.  `--native-emit` writes a file and
/// exits, which is the whole compilation this unit changes without running it.
fn emitted(env: &[(&str, &str)]) -> String {
    emitted_from(GUARD, env)
}

/// [`emitted`] for another guard.
fn emitted_from(file: &str, env: &[(&str, &str)]) -> String {
    // One path per CALL, not per env: the tests run in parallel in one process, so two of
    // them sharing a name means one reads a file the other has already removed.
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let out = std::env::temp_dir().join(format!(
        "loft_view_field_{}_{}.rs",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let mut cmd = loft();
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(file));
    for (k, v) in env {
        cmd.env(k, v);
    }
    let run = cmd.output().expect("spawn loft");
    assert!(
        run.status.success(),
        "--native-emit failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let text = std::fs::read_to_string(&out).expect("the emitted source");
    let _ = std::fs::remove_file(&out);
    text
}

/// One function's emitted SIGNATURE line.
fn signature(src: &str, func: &str) -> String {
    let head = format!("fn n_{func}(");
    src.lines()
        .find(|l| l.starts_with(&head))
        .unwrap_or_else(|| panic!("{func} is not in the emission"))
        .to_owned()
}

/// Both units are on by default; the name says what the pins below assume.
const ON: &[(&str, &str)] = &[];

/// The admitted shape: two scalars and a REFERENCE, with the caller's buffer argument gone.
#[test]
fn an_admitted_function_returns_its_heap_field_as_a_reference() {
    let src = emitted(ON);
    for func in [
        "a1", "a2", "a3", "a4", "a5", "c1f", "c2f", "p1", "p2", "p8", "p11", "p12", "p13", "p14",
        "p16",
    ] {
        let sig = signature(&src, func);
        assert!(
            sig.contains("-> (bool, bool, DbRef)"),
            "{func} must answer the value tuple with a view leaf:\n{sig}"
        );
        let params = &sig[..sig.find(") ->").unwrap_or(sig.len())];
        assert!(
            !params.contains("__retbuf"),
            "{func} must drop the caller's return buffer:\n{sig}"
        );
    }
}

/// And the SITE reads the tuple's own reference where it read a field of a record.
#[test]
fn a_site_reads_the_leaf_off_the_tuple() {
    let src = emitted(ON);
    assert!(
        src.contains("let mut var_m1: (bool, bool, DbRef) = n_a1(cell, var_s1, 3_i64);"),
        "the site binds the tuple and passes no buffer"
    );
    // `len(m1.pts)` is its op over the path (`(R-Wrapper)` over a pure vector path, since
    // @PLN158 round 3), and the path over a value-record local reads the tuple's leaf.
    assert!(
        src.contains("vector::length_vector(&(var_m1.2), &stores.allocations)"),
        "a length reads the leaf's reference straight off the tuple"
    );
}

/// The LEAF each exit writes is the one the per-path proof chose: the appended element's own
/// field where every path appended through the same temp, and the container's LAST element
/// where the arms appended through different temps.
#[test]
fn a_joined_append_views_the_last_element() {
    let src = emitted(ON);
    let body = |func: &str| {
        let start = src
            .find(&format!("fn n_{func}("))
            .unwrap_or_else(|| panic!("{func} is not in the emission"));
        let rest = &src[start..];
        rest[..rest[1..].find("\nfn ").map_or(rest.len(), |e| e + 1)].to_owned()
    };
    for func in ["p1", "p11", "p12", "p14"] {
        let b = body(func);
        assert!(
            b.contains("let _ve = vector::get_vector(&_vc, 12u32, -1, &stores.allocations)"),
            "{func} appends in more than one arm, so it views the last element:\n{b}"
        );
    }
    for func in ["a1", "p2", "p8", "p13", "p16"] {
        let b = body(func);
        assert!(
            b.contains("let _vl = var__elm_") && !b.contains("get_vector(&_vc"),
            "{func} appends through one temp on every path, so it views that element:\n{b}"
        );
    }
}

/// A FORWARD keeps its callee admitted: the function that keeps its record writes the
/// tuple into the buffer it was handed, and without the forward switch the same callees
/// fall back to the record form, because the forward is a site that consumes their record.
#[test]
fn a_forward_writes_the_tuple_into_its_own_buffer() {
    let both = emitted_from(FORWARD, ON);
    for func in ["nm", "hit", "hit2", "sz"] {
        let sig = signature(&both, func);
        let params = &sig[..sig.find(") ->").unwrap_or(sig.len())];
        assert!(
            sig.contains("-> (") && !params.contains("__re"),
            "{func} is forwarded, and stays admitted:\n{sig}"
        );
    }
    for func in ["fr", "fz", "fp", "fr_outer"] {
        let sig = signature(&both, func);
        assert!(
            sig.contains(") -> DbRef {"),
            "{func} keeps its record:\n{sig}"
        );
    }
    for call in [
        "{ let __vt = n_nm(cell); let mut __vd = var___ref_1; \
         if !(__vd.store_nr != u16::MAX && __vd.rec != 0) { __vd = OpDatabase(cell, __vd, ",
        "{ let __vt = n_hit(cell, var_sc, 2_i64); let mut __vd = var___ref_1; ",
        "{ let __vt = n_sz(cell, ",
    ] {
        assert!(
            both.contains(call),
            "the forward writes the tuple into the handed buffer: `{call}` is missing"
        );
    }
    let views_only = emitted_from(FORWARD, &[("LOFT_NO_FORWARD_TUPLE", "1")]);
    for func in ["nm", "hit", "hit2"] {
        let sig = signature(&views_only, func);
        assert!(
            sig.contains(") -> DbRef {"),
            "without the forward switch {func} is consumed by a forward and keeps its record:\n{sig}"
        );
    }
}

/// The switch restores the record form whole — the unit's own control.
#[test]
fn a_pooled_caller_mints_nothing_for_a_tuple() {
    // e1: `a1` called in a loop, so its hidden buffer is pooled — minted once and released
    // (`OpClear`) before every reuse.  With the callee answering a tuple the buffer is DEAD,
    // and both its mint and its release are emitted as nothing (@PLN164 E-1); the release
    // alone used to keep a heap-owning buffer alive.
    let src = emitted(ON);
    let start = src.find("fn n_main(").expect("main is emitted");
    let main = &src[start..];
    let main = &main[..main.find("  } /*block_1").unwrap_or(main.len())];
    assert!(
        main.contains("let mut var_me1: (bool, bool, DbRef) = n_a1(cell, var_se1,"),
        "e1 binds the tuple:\n{main}"
    );
    assert!(
        !main.contains("remove_claims"),
        "no release of a buffer nobody minted:\n{main}"
    );
    assert!(
        !main.contains("OpDatabase(cell,var___ref"),
        "no hidden buffer is minted in main:\n{main}"
    );
}

#[test]
fn the_switch_restores_the_record() {
    let src = emitted(&[("LOFT_NO_VIEW_FIELD", "1")]);
    let sig = signature(&src, "a1");
    assert!(
        sig.contains("var___retbuf: DbRef) -> DbRef"),
        "with the unit off a1 keeps its return buffer:\n{sig}"
    );
    let on = emitted(ON);
    assert!(
        signature(&on, "a1").contains("-> (bool, bool, DbRef)"),
        "by default a1 answers the tuple"
    );
}

/// Every decline, by the condition it stands for.  A decline that quietly stops declining is
/// this unit's whole risk: three of these would be SILENT in a consumer.
#[test]
fn every_decline_keeps_the_record_form() {
    let src = emitted(ON);
    for (func, why) in [
        ("d1", "the source is owned by the frame"),
        (
            "d2",
            "the source is a `?`-discharge, whose ownership is a join",
        ),
        ("d3", "the source has two owning destinations"),
        ("d4", "the callee grows the viewed container again"),
        ("d5", "a text field is not a leaf type"),
        (
            "d6",
            "the field is filled by a literal, which pushes element by element",
        ),
        ("b1f", "a site appends to the field"),
        (
            "b2f",
            "a site grows the container between the bind and the read",
        ),
        ("b3f", "a site grows it through a call"),
        ("b4f", "a site keeps the record"),
        ("b5f", "a site reads before the bind in a loop"),
        ("b6f", "a site iterates the field"),
        (
            "b7f",
            "a site reads an element of the field and writes through it",
        ),
        (
            "b8f",
            "a site removes from the container between the bind and the read",
        ),
        (
            "p3",
            "the append stands under a loop the exit is outside of",
        ),
        ("p4", "only one arm appends"),
        ("p5", "an unviewed append follows the viewed ones"),
        ("p6", "a removal follows the viewed append"),
        ("p7", "the local grows after its copy"),
        ("p7b", "the local grows after a joined copy"),
        ("p7c", "the local is rebound after its copy"),
        ("p7d", "an element of the local is written after its copy"),
        ("p9", "the arms copy different locals"),
        ("p10", "the arms append into different containers"),
        ("p15", "one arm appends a second, unviewed element"),
        (
            "p17",
            "the local grows inside the literal, between the copy and the finish",
        ),
    ] {
        let sig = signature(&src, func);
        assert!(
            sig.contains("var___retbuf: DbRef) -> DbRef"),
            "{func} must keep the record form — {why}:\n{sig}"
        );
    }
}

/// The cells' values, on both backends and under the store falsifiers: a view leaf is never
/// freed and never read after its place moved.
#[test]
fn the_cells_hold_on_both_backends_under_every_falsifier() {
    let strict: &[(&str, &str)] = &[
        ("LOFT_POISON", "1"),
        ("LOFT_POISON_CLAIM", "1"),
        ("LOFT_STRICT_STORES", "1"),
        ("LOFT_NATIVE_LEAK_CHECK", "1"),
    ];
    for (file, done) in [
        (GUARD, "164-view-field: every cell holds"),
        (FORWARD, "164-forward-tuple: every cell holds"),
    ] {
        for backend in ["--interpret", "--native"] {
            let mut cmd = loft();
            cmd.arg(backend)
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(file));
            for (k, v) in strict {
                cmd.env(k, v);
            }
            let out = cmd.output().expect("spawn loft");
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(
                out.status.success() && text.contains(done),
                "{file} on {backend} did not hold under the falsifiers:\n{text}"
            );
            assert!(
                !text.contains("stores not freed"),
                "{file} on {backend} leaked:\n{text}"
            );
        }
    }
}
