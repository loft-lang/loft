// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-O-Move` at a LIFT — a `__lift_N` temp bound from a callee that returns its promoted
//! local adopts the minted store as a named bind does (`Scopes::lift_set`).  The guard
//! `tests/scripts/a-lifted-call-result-adopts-like-a-named-bind.loft` says the values hold on
//! both backends; this pins what the change BUYS and how it is spelled: the lifted cell mints
//! exactly as many stores as its named control on either backend, the lifted bind is a plain
//! adopt with the buffer's free guarded by identity, and `LOFT_NO_ADOPT_FIRST_BIND=1` restores
//! the copy — one store more per call.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/a-lifted-call-result-adopts-like-a-named-bind.loft";

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

fn loft() -> Command {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_ADOPT_FIRST_BIND");
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

const OFF: &[(&str, &str)] = &[("LOFT_NO_ADOPT_FIRST_BIND", "1")];

/// 2000 rounds; the named control and the lifted cell mint one store a call (the callee's
/// wrapper, whose store `(R-ReturnField)` hands through at the field's position since
/// 2026-09-30 — it was two, the wrapper and a copy of the field, before that rule), and the
/// switch puts the copy's store at the lift back.
#[test]
fn the_lifted_cell_mints_as_the_named_control_does() {
    for mode in ["--interpret", "--native"] {
        let named = store_mints(mode, "c1", &[]);
        let lifted = store_mints(mode, "c2", &[]);
        assert_eq!(named, 2000, "{mode}: the named control's mints");
        assert_eq!(lifted, named, "{mode}: the lifted cell's mints");
        let lifted_off = store_mints(mode, "c2", OFF);
        assert_eq!(
            lifted_off,
            lifted + 2000,
            "{mode}: the switch restores the copy's store per call"
        );
    }
}

/// The lifted bind is the named bind's: a plain adopt, the buffer's free guarded by
/// identity against the temp, and no copy op in the function; under the switch the copy
/// (`CopyRefOrNull` into a store minted for it) is back.
#[test]
fn the_lifted_bind_is_spelled_as_the_named_one() {
    let on = introspect(&[]);
    let c2 = section(&on, "n_c2");
    assert!(
        c2.contains("OpFreeRefIfDistinct(__ref_1(1), __lift_1(1))"),
        "c2: the buffer's free is guarded by the temp\n{c2}"
    );
    assert!(
        !c2.contains("CopyRefOrNull") && !c2.contains("OpCopyRecord"),
        "c2: no copy of the callee's result\n{c2}"
    );
    let c1 = section(&on, "n_c1");
    assert!(
        c1.contains("OpFreeRefIfDistinct(__ref_1(1), z(1))"),
        "c1: the named control's guarded free\n{c1}"
    );
    let off = introspect(OFF);
    let c2_off = section(&off, "n_c2");
    assert!(
        c2_off.contains("CopyRefOrNull"),
        "c2 under the switch: the copy is back\n{c2_off}"
    );
}
