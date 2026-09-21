// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-GroupPush`, `@FR-R-PushRec`'s heap clause and `@FR-R-Mint`'s rebound clause — the
//! EMISSION pins.  A mint group that holds no push header binds one of its own after its
//! reservation (`//@FR-R-GroupPush group push header`), a heap-owning element's mint zeroes
//! its slot (`push_record_hoisted_zero`), and a loop whose body rebinds a mover to a fresh
//! buffer or element slot hoists instead of declining.  `LOFT_NO_GROUP_PUSH=1`,
//! `LOFT_NO_HEAP_RECORD_PUSH=1` and `LOFT_NO_REBOUND_MOVER=1` each restore one clause's
//! pre-form.  The rebound clause also reaches `(R-Scalar)`'s write-set walk: an emitter-owned
//! mint (a loop buffer's, a loop record's) and the element-first copy into a fresh element are
//! typed instead of declining every scalar of the loop.  The cell corpus
//! (`tests/scripts/157-group-push.loft`) says the VALUES hold on both backends, in every switch
//! state and under the falsifiers; this pins what is emitted.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELLS: &str = "tests/scripts/157-group-push.loft";

/// Per function: `(loop record-push headers, GROUP headers, zeroing mints, fused uses — a
/// slot or a finish, template mints, hoist blocks opened)`.  Written beside the cells before
/// the emission was read, then checked against it.
const EXPECTED: &[(&str, usize, usize, usize, usize, usize, usize)] = &[
    // g1: two literal groups of value-record calls, no loop — one header each, three
    // mints and three finishes through them.
    ("n_g1", 0, 2, 0, 6, 0, 0),
    // g2: the side loop hoists although `pts` is rebound per pass (to the element slot);
    // the Frond mint goes through the loop's header with its slot zeroed, the point group
    // through its own header — six fused uses, no template.
    ("n_g2_build", 1, 1, 1, 6, 0, 1),
    // g3: a heap-owning element appended outside any loop — the GROUP header, zeroed.
    ("n_g3_mk", 0, 1, 1, 2, 0, 0),
    // g4: a builder that grows another vector is a store write — the group declines and
    // all three mints keep their templates.
    ("n_g4", 0, 0, 0, 0, 3, 0),
    // g5: the self-reading group — its two headers serve the three mints; the length read
    // is a store read.
    ("n_g5", 0, 2, 0, 6, 0, 0),
    // g6: thirteen mints across a growth step, two groups.
    ("n_g6", 0, 2, 0, 26, 0, 0),
    // g7: the mover rebound to a VIEW declines the LOOP (no hoist block, no loop header);
    // the literal that seeds `big` and the push through the view each take a group header
    // of their own — a group is self-contained, so the view's header is derived after the
    // reservation and grows the viewed record through itself.
    ("n_g7", 0, 2, 0, 4, 0, 0),
    // g8: the outer loop hoists past a loop buffer (`sides`) declared per pass; the `base`
    // literal and the point group are groups; the Frond mint is the loop header's, zeroed.
    ("n_g8_build", 1, 2, 1, 12, 0, 1),
    // g9: the fractal move — the callee's fronds move into `out` through the loop header
    // (the heap element zeroed), no group.
    ("n_g9", 1, 0, 1, 2, 0, 2),
    // g10: one group per `if` arm on the same per-pass vector, the Frond through the loop's.
    ("n_g10_build", 1, 2, 1, 12, 0, 1),
];

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_GROUP_PUSH")
        .env_remove("LOFT_NO_HEAP_RECORD_PUSH")
        .env_remove("LOFT_NO_REBOUND_MOVER")
        .env_remove("LOFT_NO_RECORD_PUSH")
        .env_remove("LOFT_HOIST_VERIFY");
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
    let out = std::env::temp_dir().join(format!("loft_group_push_{}_{tag}.rs", std::process::id()));
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

type Row = (usize, usize, usize, usize, usize, usize);

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
        row.1 += usize::from(line.contains("@FR-R-GroupPush group push header"));
        row.2 += line.matches("push_record_hoisted_zero").count();
        row.3 += line.matches("push_record_hoisted").count()
            + line.matches("push_record_finish").count();
        row.4 +=
            line.matches("OpNewRecord(cell,").count() + line.matches("OpNewRecordNP(cell,").count();
        row.5 += usize::from(line.contains("loft#885 loop-invariant vector headers"));
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
        out.status.success() && stdout.trim_end().ends_with("group push ok"),
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
        Some(("LOFT_NO_GROUP_PUSH", "1")),
        Some(("LOFT_NO_HEAP_RECORD_PUSH", "1")),
        Some(("LOFT_NO_REBOUND_MOVER", "1")),
        Some(("LOFT_NO_RECORD_PUSH", "1")),
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
    let rust = emit("on", &[]);
    let got = counts(&rust);
    for (name, loop_hdr, groups, zeros, uses, templates, blocks) in EXPECTED {
        let row = got
            .get(*name)
            .copied()
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(
            row,
            (*loop_hdr, *groups, *zeros, *uses, *templates, *blocks),
            "{name}: (loop headers, group headers, zeroing mints, fused uses, templates, hoist blocks)"
        );
    }
    // Every group header is closed where its group ends, so the audit knows the holder died.
    assert_eq!(
        rust.matches("@FR-R-GroupPush group push header").count(),
        rust.matches("// @FR-R-GroupPush group __ph_").count(),
        "every group header must carry its close marker"
    );
}

/// g11/g12 — the write-set half of the rebound clause: the loop past a per-pass loop buffer
/// hoists the parameter's scalars (`sp.a`, `sp.b`, and `sp.n` for the range end), and the
/// loop beside a loop record keeps the vector's header while the record's mint — a write of
/// its whole type — evicts the scalars of that type (`sp` is a `Spec` too: none hoist there,
/// the type-keyed conservatism `(R-Scalar)` already has).
#[test]
fn an_owned_mint_leaves_the_scalar_write_set_typed() {
    let rust = emit("scalars", &[]);
    let g11 = body(&rust, "n_g11_run");
    assert_eq!(
        g11.matches("let __vs_").count(),
        3,
        "g11: sp's three fields hoist past the loop buffer's mint:\n{g11}"
    );
    assert_eq!(
        g11.matches("loft#885 loop-invariant vector headers")
            .count(),
        2,
        "g11: both loops open a hoist block:\n{g11}"
    );
    let g12 = body(&rust, "n_g12_run");
    assert!(
        g12.contains("vector::vec_header(&(var_v)") && g12.contains("@FR-R-LoopRecord kept record"),
        "g12: the vector's header holds beside the kept loop record:\n{g12}"
    );
    assert_eq!(
        g12.matches("let __vs_").count(),
        0,
        "g12: the loop record's mint writes its whole type, which evicts sp's fields:\n{g12}"
    );
    // The switch takes the write-set clause with it: the OUTER loop (the one with the buffer's
    // mint) declines whole; the inner `for s in sides` has no owned mint and still hoists
    // `sp.a` and `sp.b` — one hoist block, two scalars.
    let rust = emit("scalars_off", &[("LOFT_NO_REBOUND_MOVER", "1")]);
    let g11 = body(&rust, "n_g11_run");
    assert_eq!(
        (
            g11.matches("let __vs_").count(),
            g11.matches("loft#885 loop-invariant vector headers")
                .count()
        ),
        (2, 1),
        "g11 under LOFT_NO_REBOUND_MOVER=1 keeps only the inner loop's hoists:\n{g11}"
    );
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
fn each_switch_restores_its_clause_alone() {
    // No group header anywhere; the templates return for every group-only cell.
    let rust = emit("no_group", &[("LOFT_NO_GROUP_PUSH", "1")]);
    let got = counts(&rust);
    for (name, _, groups, ..) in EXPECTED {
        let row = got[*name];
        assert_eq!(
            row.1, 0,
            "{name}: LOFT_NO_GROUP_PUSH=1 binds no group header"
        );
        if *groups > 0 {
            assert!(
                row.4 > 0,
                "{name}: LOFT_NO_GROUP_PUSH=1 brings the template mint back"
            );
        }
    }
    // The loop headers and their zeroing mints stay: the clause is the group's alone.
    assert_eq!(
        got["n_g2_build"].0, 1,
        "g2 keeps its loop header under LOFT_NO_GROUP_PUSH=1"
    );
    assert_eq!(
        got["n_g9"].2, 1,
        "g9 keeps its zeroing mint under LOFT_NO_GROUP_PUSH=1"
    );

    // No zeroing mint anywhere: a heap-owning element keeps its template, a no-heap one its
    // header — the Pt groups still fuse.
    let rust = emit("no_heap", &[("LOFT_NO_HEAP_RECORD_PUSH", "1")]);
    let got = counts(&rust);
    for (name, ..) in EXPECTED {
        assert_eq!(
            got[*name].2, 0,
            "{name}: LOFT_NO_HEAP_RECORD_PUSH=1 zeroes no slot"
        );
    }
    for name in ["n_g2_build", "n_g3_mk", "n_g8_build", "n_g9", "n_g10_build"] {
        assert!(
            got[name].4 > 0,
            "{name}: the heap element's template mint is back"
        );
    }
    assert_eq!(
        got["n_g2_build"].1, 1,
        "g2's point group still takes its header"
    );
    assert_eq!(
        got["n_g1"],
        (0, 2, 0, 6, 0, 0),
        "g1 is untouched by the heap switch"
    );

    // The rebound-mover loops decline whole again; the groups inside them stay.
    let rust = emit("no_rebound", &[("LOFT_NO_REBOUND_MOVER", "1")]);
    let got = counts(&rust);
    for name in ["n_g2_build", "n_g8_build", "n_g10_build"] {
        let row = got[name];
        assert_eq!(
            (row.0, row.5),
            (0, 0),
            "{name}: LOFT_NO_REBOUND_MOVER=1 declines the loop"
        );
        assert!(row.1 > 0, "{name}: its groups still bind their own headers");
        assert!(row.4 > 0, "{name}: the Frond mint is a template again");
    }
    assert_eq!(
        got["n_g9"],
        (1, 0, 1, 2, 0, 2),
        "g9 rebinds no mover and is untouched"
    );
}
