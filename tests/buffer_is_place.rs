// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN164 C2 (`@FR-R-Place`, `@FR-O-Buffer`) — a call whose vector result goes into a
//! destination that already EXISTS, and which no argument reaches, is handed that place as its
//! return buffer.
//!
//! `tests/scripts/164-buffer-is-the-place.loft` carries the eleven cells and says the VALUES do
//! not move; the corpus runner runs it. This file carries the whole of the falsifier, because
//! the switch cannot: `LOFT_NO_BUFFER_IS_PLACE=1` restores the COPY, and the copy is correct,
//! so every cell passes either way. What the switch costs — a buffer store, an
//! element-by-element `OpAppendVector` and a free, per assignment — is visible only in the IR.
use std::path::{Path, PathBuf};
use std::process::Command;

const GUARD: &str = "tests/scripts/164-buffer-is-the-place.loft";

fn guard() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(GUARD)
}

fn loft() -> Command {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "240")
        .env_remove("LOFT_NO_BUFFER_IS_PLACE");
    cmd
}

/// One function's IR as `loft introspect` prints it, with `env` applied.
fn ir(func: &str, env: &[(&str, &str)]) -> String {
    let mut cmd = loft();
    cmd.arg("introspect").arg(guard());
    for (k, v) in env {
        cmd.env(k, v);
    }
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

/// The admitted shape: the buffer variable is bound to the destination and the call fills it.
/// Nothing is copied, nothing is freed, and no store is minted for the result.
#[test]
fn an_admitted_call_is_handed_the_destination_as_its_buffer() {
    let body = ir("a1", &[]);
    assert!(
        !body.contains("OpAppendVector"),
        "an admitted call copies nothing into the destination:\n{body}"
    );
    assert!(
        !body.contains("OpFreeRef"),
        "the buffer is a PLACE, so there is nothing to free (@FR-O-Buffer):\n{body}"
    );
    assert!(
        !body.contains("_p154_rhs"),
        "the result temp is gone with the copy:\n{body}"
    );
    assert!(
        body.contains("__ref_1(1):vector<integer> = OpGetField(h(0)"),
        "the buffer variable must be bound to the destination place:\n{body}"
    );
}

/// The switch restores all three. Without this the unit would have no falsifier at all, since
/// the values are identical either way.
#[test]
fn the_switch_restores_the_buffer_store_the_copy_and_the_free() {
    let body = ir("a1", &[("LOFT_NO_BUFFER_IS_PLACE", "1")]);
    assert!(
        body.contains("OpAppendVector") && body.contains("OpFreeRef") && body.contains("_p154_rhs"),
        "with the switch the copy shape must come back whole:\n{body}"
    );
}

/// The declines that keep the copy. `d1` is true BY CONSTRUCTION — the callee clears its buffer
/// before reading its arguments, so an argument reaching the destination would read an emptied
/// vector. The other two the CORPUS taught, after this unit shipped a first cut without them:
///
/// * `d7` — a callee that MINTS into its buffer rather than filling it. A vector LITERAL return
///   lowers to `OpDatabase` on the buffer PARAMETER, so it would mint over the destination.
///   Three sampled callees suggested `(R-Place)`'s decline for this was vacuous. It is not.
/// * `d8` — a field of a struct-ENUM VARIANT, whose place exists only if the enum holds that
///   variant, so it does not unconditionally EXIST as the rule requires.
#[test]
fn a_place_or_callee_the_rule_declines_keeps_the_copy() {
    for (func, why) in [
        ("d1", "an argument that reaches the destination"),
        ("d7", "a callee that mints into its buffer"),
        ("d8", "a field of an enum variant"),
    ] {
        let body = ir(func, &[]);
        assert!(
            body.contains("OpAppendVector"),
            "{why} must keep the copy:\n{body}"
        );
    }
}

/// A grouped destination is ADMITTED here — unlike C1's in-place literal, the group maintenance
/// brackets the fill and a call sits inside it — but only because
/// `group_reindex_after_vector_write` learned this shape. Before it did, the vector filled and
/// the keyed view stayed empty, which is `@FR-Col-Group`'s exact silence.
#[test]
fn a_grouped_destination_is_admitted_and_still_re_indexed() {
    let body = ir("d6", &[]);
    assert!(
        !body.contains("OpAppendVector"),
        "the grouped destination takes the rewrite too:\n{body}"
    );
    assert!(
        body.contains("OpClearKeyed") && body.contains("OpIndexGroup"),
        "the group maintenance must still bracket the fill (@FR-Col-Group):\n{body}"
    );
}

/// The cells hold on both backends and under every store falsifier — a buffer that is a place
/// must never be freed.
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
        cmd.arg(mode).arg(guard());
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
        outs[0].contains("13 cells"),
        "the guard did not reach its last cell:\n{}",
        outs[0]
    );
    assert_eq!(outs[0], outs[1], "the two backends must print the same");
}
