// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-RecPtr`'s base clause and `@FR-R-Counter`'s iteration clause — the EMISSION pins.
//! The loop variable of `for e in v` takes its address from the element base the loop
//! holds (`base + index * size` under the in-range test), and the iteration's `#index`
//! steps through the non-null add.  `LOFT_NO_BASE_RECPTR=1` restores `vector::rec_ptr`.
//! The cell corpus (`tests/scripts/158-iteration-base.loft`) says the VALUES hold on both
//! backends, in every switch state and under the falsifiers; this pins what is emitted.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELLS: &str = "tests/scripts/158-iteration-base.loft";

/// Per function: `(addresses from a held base, addresses from rec_ptr, unchecked index
/// steps, checked index steps)`.  A `rec_ptr` beside a base address is the mint window of
/// an append the cell makes itself (`@FR-R-RecPtr`'s mint clause), not an iteration.
const EXPECTED: &[(&str, usize, usize, usize, usize)] = &[
    ("n_w1_sum", 1, 0, 1, 0),
    // The twin shares the caller's header and base, and takes the address from them.
    ("n_w1_sum__inv", 1, 0, 1, 0),
    ("n_w1", 2, 0, 2, 0),
    // …and the cell's own append mints through a push window (`@FR-R-PushFill`'s record
    // clause), whose address is the window's slot — no `rec_ptr`.
    ("n_w2", 1, 0, 1, 0),
    // Two indexes over ONE base.
    ("n_w3", 2, 0, 2, 0),
    // The write-through walk and the read walk.
    ("n_w4", 2, 0, 2, 0),
    // The chunk walk that appends holds no address; the two read walks do.
    ("n_w5", 2, 2, 3, 0),
    // A callee grows the walked vector: no address, the index still bounded.
    ("n_w6", 0, 0, 1, 0),
    // The walk that appends elsewhere holds none; the read walk after it does.
    ("n_w7", 1, 1, 2, 0),
    ("n_w8", 1, 0, 1, 0),
    ("n_w9", 1, 0, 1, 0),
    // Heap-owning elements: the text and vector field reads are not fused, but the walk
    // holds its base (`len(it.nm)` is a text-value op, no writer since `@FR-R-TextBorrow`)
    // and `it.k` is read through the iteration's address.
    ("n_w11", 1, 0, 1, 0),
    ("n_w12", 1, 0, 1, 0),
    // A vector of scalars is another iterator: its step stays checked.
    ("n_w13", 0, 0, 0, 1),
    ("n_w14", 1, 0, 1, 0),
    // `e = v[i]?` binds by explicit index: `rec_ptr`, as before.
    ("n_w15", 0, 1, 0, 0),
];

const SWITCHES: [&str; 5] = [
    "LOFT_NO_BASE_RECPTR",
    "LOFT_NO_RECORD_PTR",
    "LOFT_NO_VECTOR_BASE",
    "LOFT_NO_NN_FAST",
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
    let out = std::env::temp_dir().join(format!(
        "loft_iteration_base_{}_{tag}.rs",
        std::process::id()
    ));
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
        row.0 += usize::from(line.contains("from the held base"));
        row.1 += line.matches("vector::rec_ptr(").count();
        row.2 += line.matches("__index = ops::op_add_long_nn(").count();
        row.3 += line.matches("__index = ops::op_add_int(").count();
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
        out.status.success() && stdout.trim_end().ends_with("iteration base ok"),
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
        Some(("LOFT_NO_BASE_RECPTR", "1")),
        Some(("LOFT_NO_RECORD_PTR", "1")),
        Some(("LOFT_NO_VECTOR_BASE", "1")),
        Some(("LOFT_NO_NN_FAST", "1")),
    ] {
        for falsifier in [
            ("LOFT_STRICT_STORES", "1"),
            ("LOFT_POISON", "1"),
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
    for (name, held, resolved, plain, checked) in EXPECTED {
        let row = got
            .get(*name)
            .copied()
            .unwrap_or_else(|| panic!("{name} was not emitted"));
        assert_eq!(
            row,
            (*held, *resolved, *plain, *checked),
            "{name}: (addresses from a base, addresses from rec_ptr, unchecked steps, checked steps)"
        );
    }
}

/// The switch puts every iteration's address back on `vector::rec_ptr`, one for one.
#[test]
fn the_switch_restores_rec_ptr() {
    let on = counts(&emit("on2", &[]));
    let off = counts(&emit("off", &[("LOFT_NO_BASE_RECPTR", "1")]));
    for (name, held, resolved, _, _) in EXPECTED {
        let row = off.get(*name).copied().unwrap_or_default();
        assert_eq!(
            (row.0, row.1),
            (0, held + resolved),
            "{name}: every address resolves the store with the switch on"
        );
        assert_eq!(
            on.get(*name).map(|r| r.2),
            off.get(*name).map(|r| r.2),
            "{name}: the step"
        );
    }
}
