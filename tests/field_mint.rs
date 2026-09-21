// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-Mint`'s field clause — the EMISSION pins.  A record append to a vector FIELD of a
//! plain record (`m.verts += [Vertex { … }]`, lowered as `OpNewRecord(m, parent_tp, fld)`)
//! holds a record-push header keyed by the path a read of the field already has, and emits
//! its mint and its finish through it.  `LOFT_NO_FIELD_MINT=1` restores the templates.  The
//! cell corpus (`tests/scripts/158-field-mint.loft`) says the VALUES hold on both backends,
//! in every switch state and under the falsifiers; this pins what is emitted.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELLS: &str = "tests/scripts/158-field-mint.loft";

/// Per function: `(loop record-push headers, fused slots, fused finishes, template mints)`.
/// A loop whose index chains run behind a guard is emitted twice (`@FR-R-GuardedChain`),
/// which is why `f2_build` shows five fused slots for its three appends and `f8` two
/// templates for its one.
const EXPECTED: &[(&str, usize, usize, usize, usize)] = &[
    // f1: one field, one header, no template.
    ("n_f1", 1, 1, 1, 0),
    // f2: two fields of ONE root — two headers, and the nested loop re-uses them.
    ("n_f2_build", 2, 5, 5, 0),
    // f4: a parameter root holds the header alone…
    ("n_f4_fill", 1, 1, 1, 0),
    // …and declines the loop beside another vector's header (`@FR-R-Alias`).
    ("n_f4_copy", 0, 0, 0, 1),
    // f5: a text-field element — the text set claims a record, so the loop declines.
    ("n_f5_build", 0, 0, 0, 1),
    // f6: nested literals written into the fresh slot.
    ("n_f6", 1, 1, 1, 0),
    // f7: the path runs through an inline sub-record.
    ("n_f7", 1, 1, 1, 0),
    // f8: a linked group's member and f9: a keyed field keep the general append.
    ("n_f8", 0, 0, 0, 2),
    ("n_f9", 0, 0, 0, 1),
    // f10: the root is rebound in the body.
    ("n_f10", 0, 0, 0, 2),
    // f11/f12: a view of the field beside the header (f12's seed literal is a template).
    ("n_f11", 1, 1, 1, 0),
    ("n_f12", 1, 1, 1, 1),
    // f13: a callee reads the length the finish writes back.
    ("n_f13", 1, 1, 1, 0),
    // f14: the chunk list's header, and one rooted on the loop variable.
    ("n_f14", 2, 3, 3, 0),
    ("n_f17", 1, 1, 1, 0),
    ("n_f18", 1, 1, 1, 1),
    ("n_f20", 1, 1, 1, 0),
    ("n_f21", 1, 2, 2, 0),
    ("n_f22", 1, 1, 1, 0),
    // f24: a struct-enum variant's field.
    ("n_f24", 0, 0, 0, 1),
    // f26: the header is held whatever the root turns out to be at run time.
    ("n_f26_fill", 1, 1, 1, 0),
    // f27: the element-first early mint AND its finish through the loop's header, the
    // point group through its own.
    ("n_f27_build", 1, 3, 3, 0),
    // f28/f29: a call-filled element declines whole — both halves keep their templates.
    ("n_f28_build", 0, 0, 0, 1),
    ("n_f29_build", 0, 0, 0, 1),
];

const SWITCHES: [&str; 5] = [
    "LOFT_NO_FIELD_MINT",
    "LOFT_NO_RECORD_PUSH",
    "LOFT_NO_HEAP_RECORD_PUSH",
    "LOFT_NO_ELEMENT_FIRST",
    "LOFT_HOIST_VERIFY",
];

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args).env("LOFT_TIMEOUT", "300");
    for s in SWITCHES {
        cmd.env_remove(s);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    // One file per TEST: the tests run on threads of one process.
    let out = std::env::temp_dir().join(format!("loft_field_mint_{}_{tag}.rs", std::process::id()));
    let status = loft(
        &[
            "--native-emit",
            out.to_str().unwrap(),
            cells().to_str().unwrap(),
        ],
        env,
    );
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

type Row = (usize, usize, usize, usize);

fn counts(rust: &str) -> HashMap<String, Row> {
    let mut map: HashMap<String, Row> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
            map.entry(current.clone()).or_default();
        }
        let row = map.entry(current.clone()).or_default();
        row.0 += usize::from(line.contains("V-t record push header"));
        row.1 += line.matches("push_record_hoisted").count();
        row.2 += line.matches("push_record_finish").count();
        row.3 +=
            line.matches("OpNewRecord(cell,").count() + line.matches("OpNewRecordNP(cell,").count();
    }
    map
}

fn run(args: &[&str], env: &[(&str, &str)]) -> String {
    let mut a: Vec<&str> = args.to_vec();
    let s = cells().to_str().unwrap().to_string();
    a.push(&s);
    let out = loft(&a, env);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success() && stdout.trim_end().ends_with("field mint ok"),
        "{args:?} {env:?}: exit {:?}\nstdout: {stdout}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

#[test]
fn the_cells_answer_the_interpreter_in_every_switch_state_under_the_falsifiers() {
    let oracle = run(&["--interpret"], &[("LOFT_STRICT_STORES", "1")]);
    for switch in [
        None,
        Some(("LOFT_NO_FIELD_MINT", "1")),
        Some(("LOFT_NO_RECORD_PUSH", "1")),
        Some(("LOFT_NO_HEAP_RECORD_PUSH", "1")),
        Some(("LOFT_NO_ELEMENT_FIRST", "1")),
    ] {
        for falsifier in [
            ("LOFT_STRICT_STORES", "1"),
            ("LOFT_POISON", "1"),
            ("LOFT_POISON_CLAIM", "1"),
            ("LOFT_NATIVE_LEAK_CHECK", "1"),
            ("LOFT_HOIST_VERIFY", "1"),
        ] {
            let mut env = vec![falsifier];
            env.extend(switch);
            let got = run(&["--native"], &env);
            assert_eq!(
                got, oracle,
                "native under {env:?} must print what the interpreter printed"
            );
        }
    }
}

#[test]
fn each_cell_emits_exactly_the_forms_predicted() {
    let got = counts(&emit("on", &[]));
    for (name, headers, slots, finishes, templates) in EXPECTED {
        let row = got
            .get(*name)
            .copied()
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(
            row,
            (*headers, *slots, *finishes, *templates),
            "{name}: (loop headers, fused slots, fused finishes, template mints)"
        );
    }
}

/// The switch restores the template for every FIELD append and leaves the bare-variable
/// appends of the corpus (f27's point group) on the headers they had.
#[test]
fn the_switch_restores_the_templates() {
    let got = counts(&emit("off", &[("LOFT_NO_FIELD_MINT", "1")]));
    for (name, _, _, _, _) in EXPECTED {
        let row = got.get(*name).copied().unwrap_or_default();
        assert_eq!(
            row.0, 0,
            "{name}: no loop header is rooted on a field with the switch on"
        );
    }
    let f27 = got.get("n_f27_build").copied().unwrap_or_default();
    assert_eq!(
        (f27.1, f27.2, f27.3),
        (2, 2, 1),
        "f27: the point group keeps its group header, the Fr element its template"
    );
}
