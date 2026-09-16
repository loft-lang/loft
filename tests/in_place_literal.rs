// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN164 C1 step 1 (`@FR-R-InPlaceLiteral`, its STAGING clause) — a record literal assigned to
//! an existing place is written INTO that place, so every field expression must be evaluated
//! BEFORE the first write.
//!
//! `tests/scripts/164-in-place-literal.loft` carries the seventeen cells and says the VALUES
//! hold on both backends; the corpus runner runs it. What is pinned here is the IR SHAPE, which
//! values cannot see: that an initialiser reading the destination is HOISTED into a temp ahead
//! of the writes, that one reading something else is NOT (so the staging has not become "stage
//! everything", which would cost a temp per field of every literal in the language), and that a
//! field destination still writes in place rather than regaining a temporary store.
use std::path::{Path, PathBuf};
use std::process::Command;

const GUARD: &str = "tests/scripts/164-in-place-literal.loft";

fn guard() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(GUARD)
}

fn loft() -> Command {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env_remove("LOFT_NO_ELEMENT_IN_PLACE");
    cmd
}

/// One function's IR as `loft introspect` prints it, with `env` applied.
fn ir_with(func: &str, env: &[(&str, &str)]) -> String {
    let mut cmd = loft();
    cmd.arg("introspect")
        .arg(guard())
        .env("LOFT_TIMEOUT", "240");
    for (k, v) in env {
        cmd.env(k, v);
    }
    ir_of(func, cmd)
}

/// One function's IR as `loft introspect` prints it.
fn ir(func: &str) -> String {
    let mut cmd = loft();
    cmd.arg("introspect")
        .arg(guard())
        .env("LOFT_TIMEOUT", "240");
    ir_of(func, cmd)
}

fn ir_of(func: &str, mut cmd: Command) -> String {
    let out = cmd.output().expect("spawn loft");
    assert!(
        out.status.success(),
        "introspect failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    let head = format!("fn n_{func}(");
    let start = text
        .match_indices(&head)
        .map(|(i, _)| i)
        .find(|i| {
            text[*i..]
                .lines()
                .next()
                .is_some_and(|l| l.contains("{#block"))
        })
        .unwrap_or_else(|| panic!("{func}'s IR"));
    let rest = &text[start..];
    let end = rest.find("}#block(1)").expect("the closing block") + "}#block(1)".len();
    rest[..end].to_owned()
}

/// A field destination writes straight into the place: the receiver of every write is the
/// projection, and no store is minted for the literal.
#[test]
fn a_field_destination_writes_into_the_place() {
    let body = ir("f_partial");
    assert!(
        !body.contains("OpDatabase(") && !body.contains("OpCopyRecord"),
        "a field destination must not mint a temporary store or copy:\n{body}"
    );
    assert!(
        body.contains("OpGetField(bx(0)"),
        "the writes must take the place as their receiver:\n{body}"
    );
}

/// The staging clause: an initialiser that reads the destination is evaluated into a temp, and
/// every such temp is bound BEFORE the first write to the place.
#[test]
fn an_initialiser_that_reads_the_place_is_staged_before_the_first_write() {
    let body = ir("f_swap");
    let first_write = body
        .find("OpSet")
        .unwrap_or_else(|| panic!("a write to the place:\n{body}"));
    let hoists = body.match_indices("__work_").count() + body.match_indices("__ref_").count();
    assert!(
        hoists >= 2,
        "both reads of the destination must be staged into temps:\n{body}"
    );
    // Every staged read has to be bound ahead of the first write; a single one landing after it
    // is the defect this unit closes.
    for (at, _) in body.match_indices("OpGetInt(OpGetField(bx(0)") {
        assert!(
            at < first_write,
            "a read of the destination is evaluated AFTER the first write to it:\n{body}"
        );
    }
}

/// The other half: an initialiser provably reading ELSEWHERE is not staged. Without this the
/// rule degenerates into "stage every field of every literal", which no measurement asked for.
///
/// `tw.a = El { a: tw.b.a, … }` reads a SIBLING record field, which arrives as
/// `OpGetField(tw, 28)` against a destination at offset 0 — disjoint, and provably so.
#[test]
fn an_initialiser_that_provably_reads_elsewhere_is_not_staged() {
    let body = ir("f_sibling");
    assert!(
        !body.contains("__work_") && !body.contains("__ref_"),
        "reading the sibling `tw.b` does not read the place `tw.a`, so nothing is staged:\n{body}"
    );
}

/// The conservatism, pinned so a later narrowing is a deliberate change rather than a surprise:
/// a sibling read spelled with a TYPED accessor (`len(bx.tag)` is `OpGetText(bx, 28)`) stages a
/// temp it does not need, because only `OpGetField` carries an offset this can trust — see
/// `Parser::reads_place` for why that is not closed with a list of ops.
#[test]
fn a_sibling_read_through_a_typed_accessor_stages_conservatively() {
    let body = ir("f_container");
    assert!(
        body.contains("__ref_1(1):integer"),
        "the conservative stage is expected here; if this now spares, the span-based test \
         landed and this pin should move to asserting that:\n{body}"
    );
}

/// The cells hold on both backends and under every store falsifier — the hoisted temps must
/// neither leak the old value nor release it twice.
#[test]
fn the_cells_hold_on_both_backends_under_every_falsifier() {
    let strict: &[(&str, &str)] = &[
        ("LOFT_POISON", "1"),
        ("LOFT_POISON_CLAIM", "1"),
        ("LOFT_STRICT_STORES", "1"),
    ];
    let mut outs = Vec::new();
    for (mode, extra) in [
        ("--interpret", None),
        ("--native", Some(("LOFT_NATIVE_LEAK_CHECK", "1"))),
    ] {
        let mut cmd = loft();
        cmd.arg(mode).arg(guard()).env("LOFT_TIMEOUT", "240");
        for (k, v) in strict {
            cmd.env(k, v);
        }
        if let Some((k, v)) = extra {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("spawn loft");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{mode} failed:\n{err}");
        assert!(
            !err.contains("[strict-store]") && !err.contains("not freed at program exit"),
            "{mode} reported a store-lifetime fault:\n{err}"
        );
        outs.push(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    assert!(
        outs[0].contains("25 cells"),
        "the guard did not reach its last cell:\n{}",
        outs[0]
    );
    assert_eq!(outs[0], outs[1], "the two backends must print the same");
}

/// C1 step 2 — an ELEMENT destination writes into the slot: no store minted for the literal, no
/// deep copy into the slot, and the writes take the element place as their receiver.
///
/// This is step 2's whole falsifier, and it has to be structural.  `LOFT_NO_ELEMENT_IN_PLACE=1`
/// restores the COPY, and every cell still passes under it — because the copy is correct.  What
/// the switch costs is a store and a deep copy per assignment, which no value can see.
#[test]
fn an_element_destination_writes_into_the_slot() {
    let body = ir("e_partial");
    assert!(
        !body.contains("OpDatabase(") && !body.contains("OpCopyRecord"),
        "an element destination must mint no store and copy nothing:\n{body}"
    );
    // Both spellings, because an element read has two (`OpGetVector(base, size, index)` and
    // `OpVectorRef(base, index)`) and asserting one of them is how this pin first passed
    // vacuously — the same half-answer QUALITY.md's `spellings` screen exists to find.
    assert!(
        body.contains("OpGetVector(") || body.contains("OpVectorRef("),
        "the writes must take the element place as their receiver:\n{body}"
    );
    let off = ir_with("e_partial", &[("LOFT_NO_ELEMENT_IN_PLACE", "1")]);
    assert!(
        off.contains("OpDatabase(") && off.contains("OpCopyRecord"),
        "the switch must restore the temporary store and the copy:\n{off}"
    );
}

/// The two DECLINES keep the copy, with the switch off.  A decline that silently stopped
/// declining is the expensive direction: `v[bump()]` would call `bump` once per field write and
/// `v[len(v) - 2]` would re-read a length the writes may have moved.
#[test]
fn a_place_that_cannot_be_re_derived_keeps_the_copy() {
    for (func, why) in [
        ("e_effect_index", "an index with a side effect"),
        ("e_computed_index", "a computed index"),
        // Measured, not reasoned: in place this released the slot's own vector and then stored
        // the handle back to it, and the field read 0 on both backends.
        ("e_self_heap", "a type with a collection field"),
        // `@FR-Col-Group`: an element write owes the group its unlink-and-relink, which only
        // the copy road carries.  In place, `by_k` kept the record under its OLD key.
        ("e_grouped", "a member of a linked collection group"),
    ] {
        let body = ir(func);
        assert!(
            body.contains("OpCopyRecord"),
            "{why} must keep the copy:\n{body}"
        );
    }
}
