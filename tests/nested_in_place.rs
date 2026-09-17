// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-InPlaceLiteral` (@PLN164 C6) — a record literal that initialises an embedded record
//! field of a FRESH record (`sc.ops += [Op { paint: Paint { … } }]`) writes its fields into
//! that field, instead of building in a store of its own and deep-copying it there.
//!
//! The cells (`plans/164-activation-arena/bytecode-comparisons/C6-nested-literal-cells.loft`)
//! carry the shapes with their values.  These tests pin that the values hold on both
//! backends in both switch states under the store falsifiers, and that the IR writes the
//! nested fields through the field's place with the switch on and copies with it off.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELLS: &str =
    "doc/claude/plans/164-activation-arena/bytecode-comparisons/C6-nested-literal-cells.loft";

const EXPECTED: &str = "n1 3 45 n2 2 3 40 2 n3 8 5 -1 9 n4 40 20 1030\n\
                        n5 1 2 3 deep n6 4 n4 40 n7 7 2 old\n";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args).env("LOFT_TIMEOUT", "120");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

fn cells() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(CELLS)
        .to_string_lossy()
        .into_owned()
}

/// The IR of one function, from `loft introspect`.
fn ir_of(text: &str, name: &str) -> String {
    let start = text
        .find(&format!("\nfn {name}("))
        .unwrap_or_else(|| panic!("{name} is not in the introspection"));
    let rest = &text[start + 1..];
    let end = rest.find("\nbyte-code for").unwrap_or(rest.len());
    rest[..end].to_string()
}

#[test]
fn the_values_hold_on_both_backends_in_both_switch_states() {
    let src = cells();
    for switch in [None, Some(("LOFT_NO_NESTED_IN_PLACE", "1"))] {
        for backend in ["--interpret", "--native"] {
            for falsifier in [
                ("LOFT_STRICT_STORES", "1"),
                ("LOFT_POISON", "1"),
                ("LOFT_NATIVE_LEAK_CHECK", "1"),
            ] {
                let mut env = vec![falsifier];
                env.extend(switch);
                let res = loft(&[backend, &src], &env);
                let out = String::from_utf8_lossy(&res.stdout);
                let err = String::from_utf8_lossy(&res.stderr);
                let label = format!("{backend} {switch:?} {falsifier:?}");
                assert!(
                    res.status.success(),
                    "{label}: exit {:?}\n{err}",
                    res.status
                );
                assert_eq!(out, EXPECTED.repeat(2), "{label}\n{err}");
                assert!(
                    !err.contains("not freed") && !err.contains("strict-store"),
                    "{label}: a store leaked or was read after its free:\n{err}"
                );
            }
        }
    }
}

#[test]
fn the_nested_fields_are_written_through_the_place() {
    let src = cells();
    let on = String::from_utf8_lossy(&loft(&["introspect", &src], &[]).stdout).into_owned();
    let off = String::from_utf8_lossy(
        &loft(&["introspect", &src], &[("LOFT_NO_NESTED_IN_PLACE", "1")]).stdout,
    )
    .into_owned();
    // n1's element: `paint`'s fields go straight into `OpGetField(_elm_N, 8, …)`, and no
    // `Pa` record is built beside it.
    let n1_on = ir_of(&on, "n_n1");
    let n1_off = ir_of(&off, "n_n1");
    assert!(
        n1_on.contains("OpSetInt(OpGetField(_elm_"),
        "the nested literal must write through the element's field:\n{n1_on}"
    );
    assert!(
        !n1_on.contains("#Object") || !n1_on.contains("ref(Pa)"),
        "no Pa record is built in a store of its own:\n{n1_on}"
    );
    assert!(
        n1_off.contains("OpCopyRecord(") && !n1_off.contains("OpSetInt(OpGetField(_elm_"),
        "the switch builds the literal apart and copies it again:\n{n1_off}"
    );
    // n7 assigns to a place the program can read (`o.mid = Mid { … }`, whose `inner` reads the
    // old value): that nested literal is staged as a construction of its own and copied, in
    // both states.  Its FIRST statement builds `Mid` in a fresh temporary, and there the
    // nested `Inner` is written in place.
    let n7_on = ir_of(&on, "n_n7");
    assert!(
        n7_on.contains("= {#Object(") && n7_on.contains("ref(Inner)"),
        "the readable destination keeps its staged construction:\n{n7_on}"
    );
    assert!(
        n7_on.contains("OpSetText(OpGetField(__ref_p2_"),
        "the fresh temporary's nested literal is written in place:\n{n7_on}"
    );
}
