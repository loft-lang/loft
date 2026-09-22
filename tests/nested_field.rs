// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-RecPtr`'s path clause — the EMISSION pins.  A scalar field reached through INLINE
//! sub-records (`v.pos.x`) is read and written through the record view's address at the
//! summed offset.  `LOFT_NO_NESTED_FIELD=1` restores the store read.  The cell corpus
//! (`tests/scripts/158-nested-field.loft`) says the VALUES hold on both backends, in every
//! switch state and under the falsifiers; this pins what is emitted.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELLS: &str = "tests/scripts/158-nested-field.loft";

/// Per function, with the clause ON and OFF: `(record addresses bound, reads through one,
/// writes through one)`.  A view whose only accesses are nested binds no address without
/// the clause — nothing it could serve — which is why n2, n6, n11 and n12 go from none.
const EXPECTED: &[(&str, Row, Row)] = &[
    ("n_n1", (1, 7, 0), (1, 1, 0)),
    ("n_n2", (2, 6, 3), (0, 0, 0)),
    // Its own append is a mint window: five more writes through an address, per arm.
    ("n_n3", (4, 6, 12), (3, 1, 4)),
    // The sub-record view `p = v.pos` had its own address before; `v` gains one.
    ("n_n4", (2, 2, 1), (1, 0, 1)),
    ("n_n5", (3, 4, 4), (2, 1, 1)),
    // A nullable view: the null address answers the sentinel and drops the write.
    ("n_n6", (1, 1, 1), (0, 0, 0)),
    // A parameter is bound by no statement: no address either way.
    ("n_n8_len2", (0, 0, 0), (0, 0, 0)),
    // `h.bag.items[1]` forms no path; `h.bag.n` and `h.id` beside it do.
    ("n_n9", (4, 4, 5), (3, 1, 4)),
    // The literal assignment is three in-place sets; the whole-record copy of a no-heap
    // `V3` is an in-place write too (`@FR-R-InPlace`'s copy clause), so both walks hold one.
    ("n_n11", (2, 6, 3), (0, 0, 0)),
    ("n_n12", (1, 2, 1), (0, 0, 0)),
];

const SWITCHES: [&str; 4] = [
    "LOFT_NO_NESTED_FIELD",
    "LOFT_NO_RECORD_PTR",
    "LOFT_NO_BASE_RECPTR",
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
    let out =
        std::env::temp_dir().join(format!("loft_nested_field_{}_{tag}.rs", std::process::id()));
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

type Row = (usize, usize, usize);

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
        row.0 += line.matches("let __pa_").count();
        row.1 += line.matches("rec_get::<").count();
        row.2 += line.matches("rec_set::<").count();
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
        out.status.success() && stdout.trim_end().ends_with("nested field ok"),
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
        Some(("LOFT_NO_NESTED_FIELD", "1")),
        Some(("LOFT_NO_RECORD_PTR", "1")),
        Some(("LOFT_NO_BASE_RECPTR", "1")),
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
fn each_cell_emits_exactly_the_forms_predicted_with_the_clause_on_and_off() {
    let on = counts(&emit("on", &[]));
    let off = counts(&emit("off", &[("LOFT_NO_NESTED_FIELD", "1")]));
    for (name, want_on, want_off) in EXPECTED {
        assert_eq!(
            on.get(*name).copied().unwrap_or_default(),
            *want_on,
            "{name} ON: (addresses, reads through one, writes through one)"
        );
        assert_eq!(
            off.get(*name).copied().unwrap_or_default(),
            *want_off,
            "{name} OFF: (addresses, reads through one, writes through one)"
        );
    }
}

/// Under `LOFT_HOIST_VERIFY=1` a nested read is compared with the path walked the
/// unrewritten way — `rec_get`'s own check re-reads at the offset it is given and so
/// cannot see one summed wrongly.
#[test]
fn the_checking_form_walks_the_path() {
    let rust = emit("verify", &[("LOFT_HOIST_VERIFY", "1")]);
    let n1 = counts(&rust).get("n_n1").copied().unwrap_or_default();
    assert_eq!(n1.1, 7, "n1 still reads through the address under verify");
    let body = rust
        .split("\nfn n_n1(")
        .nth(1)
        .and_then(|b| b.split("\nfn ").next())
        .expect("n_n1 emitted");
    assert_eq!(
        body.matches("vector::path_read_verify(").count(),
        6,
        "each of n1's six nested reads is compared with the path walked:\n{body}"
    );
}
