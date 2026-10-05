// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `(R-ReturnField)` (`@FR-R-ReturnField`) — the returned field of an owned local is the
//! local's store handed over at the field's position.  The guard
//! `tests/scripts/a-returned-field-of-an-owned-local-hands-its-store-over.loft` says the
//! values hold on both backends; this pins the SHAPE and what it buys: the exit block is a
//! hand-over with no mint and no copy, the buffer frees it keeps are witnessed against the
//! root, the store census of the struct-enum cell halves on both backends, the copy census of
//! the plain cell reads zero, and `LOFT_NO_RETURN_FIELD=1` restores the parser's copy.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-returned-field-of-an-owned-local-hands-its-store-over.loft";

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

fn loft() -> Command {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_RETURN_FIELD");
    cmd
}

fn with_env(cmd: &mut Command, env: &[(&str, &str)]) {
    for (k, v) in env {
        cmd.env(k, v);
    }
}

/// The store mints (`[db] OpDatabase … #65535` lines of `LOFT_TRACE_DB=1`) one cell makes
/// over the guard's 2000 rounds.
fn store_mints(mode: &str, cell: &str, env: &[(&str, &str)]) -> usize {
    let mut cmd = loft();
    cmd.arg(mode)
        .arg(cells())
        .arg(cell)
        .env("LOFT_TRACE_DB", "1");
    with_env(&mut cmd, env);
    let out = cmd.output().expect("spawn loft");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|l| l.contains("OpDatabase") && l.contains("#65535"))
        .count()
}

/// The `[copy]` lines of `LOFT_COPY_DUMP=1` one cell makes on the interpreter.
fn copies(cell: &str, env: &[(&str, &str)]) -> usize {
    let mut cmd = loft();
    cmd.arg("--interpret")
        .arg(cells())
        .arg(cell)
        .env("LOFT_COPY_DUMP", "1");
    with_env(&mut cmd, env);
    let out = cmd.output().expect("spawn loft");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|l| l.starts_with("[copy]"))
        .count()
}

fn introspect(env: &[(&str, &str)]) -> String {
    let mut cmd = loft();
    cmd.arg("introspect").arg(cells());
    with_env(&mut cmd, env);
    let out = cmd.output().expect("spawn loft introspect");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The first listing of `fn <name>(` up to the next top-level `fn`.
fn section<'a>(text: &'a str, name: &str) -> &'a str {
    let start = text
        .find(&format!("\nfn {name}("))
        .unwrap_or_else(|| panic!("{name} is not in the listing"));
    let rest = &text[start + 1..];
    let end = rest[3..].find("\nfn ").map_or(rest.len(), |i| i + 3);
    &rest[..end]
}

const OFF: &[(&str, &str)] = &[("LOFT_NO_RETURN_FIELD", "1")];

/// The exits are hand-overs: no mint of the buffer, no copy, the field's address returned,
/// and every buffer free the exit keeps witnessed against the root — the plain root (r1),
/// the lifted root (r10), the pooled loop root (r12), and the alias (r5, whose owner's free
/// is witnessed against the alias), and the natural spelling over a forwarding decoder (r14,
/// whose call buffer's guard stands inside the exit).  The parameter cell (r4) keeps the copy.
#[test]
fn the_exits_hand_the_root_store_over() {
    let on = introspect(&[]);
    for (name, ret, witnessed) in [
        (
            "n_r1",
            "return OpGetField(p(1), 0i32,",
            "OpFreeRefIfDistinct(__ref_1(1), p(1))",
        ),
        (
            "n_r10",
            "return OpGetField(__lift_1(2), 12i32,",
            "OpFreeRefIfDistinct(__ref_1(1), __lift_1(2))",
        ),
        (
            "n_r12",
            "return OpGetField(p(5), 12i32,",
            "OpFreeRefIfDistinct(__ref_1(1), p(5))",
        ),
        (
            "n_r5",
            "return OpGetField(q(1), 0i32,",
            "OpFreeRefIfDistinct(p(1), q(1))",
        ),
        (
            "n_r8",
            "return v(1);",
            "OpFreeRefIfDistinct(__ref_1(1), d(1))",
        ),
        (
            "n_r14",
            "return OpGetField(__lift_1(2), 0i32,",
            "OpFreeRefIfDistinct(__ref_1(1), __lift_1(2))",
        ),
    ] {
        let ir = section(&on, name);
        assert!(
            ir.contains("return_field_handover"),
            "{name}: the exit is a hand-over\n{ir}"
        );
        assert!(
            ir.contains(ret),
            "{name}: the field's address is returned\n{ir}"
        );
        assert!(
            ir.contains(witnessed),
            "{name}: the buffer free is witnessed against the root\n{ir}"
        );
        assert!(!ir.contains("OpCopyRecord("), "{name}: no copy left\n{ir}");
        assert!(
            !ir.contains("materialized_view_return"),
            "{name}: the copy block is gone\n{ir}"
        );
    }
    // r14: the pooled buffer's mint-or-release guard stays where the scope pass put it, in
    // front of the lifted call, inside the hand-over.
    let r14 = section(&on, "n_r14");
    assert!(
        r14.contains("if OpRefIsNull(__ref_1(1))"),
        "r14: the call's buffer guard is kept\n{r14}"
    );
    let r4 = section(&on, "n_r4");
    assert!(
        !r4.contains("return_field_handover"),
        "r4: a parameter's field keeps the copy\n{r4}"
    );
    let off = introspect(OFF);
    let r1_off = section(&off, "n_r1");
    assert!(
        r1_off.contains("materialized_view_return") && r1_off.contains("OpCopyRecord("),
        "r1 under the switch: the copy is back\n{r1_off}"
    );
}

/// 2000 rounds, one `r9` call each (`@FR-M-Match`: the match subject is evaluated once).  r9 (a
/// struct-enum wrapper whose field is the answer) mints the wrapper alone per call — the
/// switch adds the field's copy — on both backends; r1's copy census reads zero and the
/// switch puts one copy per call back.
#[test]
fn the_censuses_drop() {
    for mode in ["--interpret", "--native"] {
        let on = store_mints(mode, "r9", &[]);
        let off = store_mints(mode, "r9", OFF);
        assert_eq!((on, off), (2000, 4000), "{mode}: r9 mints (on, off)");
    }
    assert_eq!(copies("r1", &[]), 0, "r1: no copy with the hand-over");
    assert_eq!(
        copies("r1", OFF),
        2000,
        "r1: one copy per call under the switch"
    );
}
