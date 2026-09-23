// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-PushFill`'s window clause — the EMISSION pins, with the two facts it is built on:
//! a comprehension is the counted range it spells (`hoist::range_counters` reads the
//! iterator alone), and a counted push loop's reservation stands in front of every guarded
//! copy of the loop.  A reserved counted push loop whose body reaches its vector through
//! the pushes alone opens ONE `vector::PushWindow`, pushes through it in every copy of the
//! loop, and closes it once; `LOFT_NO_PUSH_WINDOW=1` restores the header push.  The cell
//! corpus (`tests/scripts/158-push-window.loft`) says the VALUES hold on both backends, in
//! every switch state and under the falsifiers; this pins what is emitted.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELLS: &str = "tests/scripts/158-push-window.loft";

/// Per function: `(windows opened, pushes through a window, pushes through the header)`.
/// A loop whose index chains run behind a guard is emitted twice (`@FR-R-GuardedChain`),
/// which doubles the pushes of w1, w2, w3, w8_mk and w9_run — over ONE window each.
const EXPECTED: &[(&str, usize, usize, usize)] = &[
    ("n_w1", 1, 2, 0),
    ("n_w2", 1, 2, 0),
    ("n_w3", 2, 4, 0),
    ("n_w4", 1, 1, 0),
    ("n_w5", 2, 2, 0),
    ("n_w6", 2, 2, 0),
    // The inner loop of the nest: three pushes a pass through one window.
    ("n_w7", 1, 3, 0),
    ("n_w8_mk", 1, 2, 0),
    ("n_w8_mk2", 1, 1, 0),
    ("n_w9_run", 1, 2, 0),
    ("n_w10", 1, 1, 0),
    ("n_w11_dbl", 1, 1, 0),
    ("n_w11", 1, 1, 0),
    ("n_w12", 1, 1, 0),
    // A result vector beside a parameter read: the pushed root's store witness is the
    // buffer the caller handed in, which the ownership test reads as an owned local.
    ("n_d8_mk", 1, 1, 0),
    ("n_a_mk", 1, 1, 0),
    // The admission is reached and declines: the body names the pushed vector (d1), a
    // sibling field of the pushed record (d7), a view of it (d4), or writes a store (d5).
    ("n_d1", 0, 0, 1),
    ("n_d4", 0, 0, 1),
    ("n_d5", 0, 0, 3),
    ("n_d7", 0, 0, 1),
    // Never a counted push loop: `len(v)` reads as another write to the path (d2), a
    // filter is a `continue` (d6); a callee handed the vector declines the whole hoist (d3).
    ("n_d2", 0, 0, 1),
    ("n_d6", 0, 0, 3),
    ("n_d3", 0, 0, 0),
    // `x` owns its store and takes a window; `y`, `z` and `q` are rebound to a call's
    // buffer later, so their root is not provably exclusive and they keep the header.
    ("n_a1", 1, 1, 3),
    ("n_b1", 0, 0, 2),
    // `(R-Base)`'s growth condition: a mint left on its templates is still a growth.
    ("n_r1", 0, 0, 0),
];

const SWITCHES: [&str; 6] = [
    "LOFT_NO_PUSH_WINDOW",
    "LOFT_NO_PUSH_FILL",
    "LOFT_NO_PUSH_HOIST",
    "LOFT_NO_GUARDED_CHAIN",
    "LOFT_NO_VECTOR_BASE",
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
        std::env::temp_dir().join(format!("loft_push_window_{}_{tag}.rs", std::process::id()));
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

/// The emitted body of one function, up to the next top-level `fn`.
fn body<'a>(rust: &'a str, name: &str) -> &'a str {
    let start = rust
        .find(&format!("\nfn {name}("))
        .unwrap_or_else(|| panic!("{name} was not emitted"));
    let rest = &rust[start + 1..];
    let end = rest[3..].find("\nfn ").map_or(rest.len(), |i| i + 3);
    &rest[..end]
}

/// `(windows, windowed pushes, header pushes, closes)` per function.
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
        row.0 += line.matches("= vector::push_window(&").count();
        row.1 += line.matches("stores.push_windowed::<").count();
        row.2 += line.matches("stores.push_hoisted::<").count();
        row.3 += line.matches("stores.push_window_close::<").count();
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
        out.status.success() && stdout.trim_end().ends_with("push window ok"),
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
        Some(("LOFT_NO_PUSH_WINDOW", "1")),
        Some(("LOFT_NO_PUSH_FILL", "1")),
        Some(("LOFT_NO_PUSH_HOIST", "1")),
        Some(("LOFT_NO_GUARDED_CHAIN", "1")),
        Some(("LOFT_NO_VECTOR_BASE", "1")),
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
    for (name, windows, windowed, header) in EXPECTED {
        let row = got.get(*name).copied().unwrap_or_default();
        assert_eq!(
            (row.0, row.1, row.2),
            (*windows, *windowed, *header),
            "{name}: (windows, pushes through one, pushes through the header)"
        );
    }
}

#[test]
fn every_window_opened_is_closed_exactly_once() {
    // The close is what writes the record's length: a window without one leaves the vector
    // at the length it had when the window opened, and two would be one too many headers.
    for (name, row) in counts(&emit("closes", &[])) {
        assert_eq!(
            row.0, row.3,
            "{name}: {} window(s), {} close(s)",
            row.0, row.3
        );
    }
}

#[test]
fn the_reservation_and_the_window_stand_in_front_of_the_guarded_copies() {
    // A guard emits a whole copy of the loop ahead of its `else`.  A reservation written
    // after it stands in the arm that does not run, and a window declared there would be
    // one local per arm instead of one both arms push through.
    let rust = emit("order", &[]);
    let w1 = body(&rust, "n_w1");
    let at = |needle: &str| {
        w1.find(needle)
            .unwrap_or_else(|| panic!("n_w1 lacks `{needle}`:\n{w1}"))
    };
    let (reserve, window, guard) = (
        at("push reserve"),
        at("= vector::push_window(&"),
        at("@FR-R-GuardedChain guard"),
    );
    assert!(
        reserve < window && window < guard,
        "reserve, then the window, then the guard — got {reserve} / {window} / {guard}"
    );
    assert!(
        at("stores.push_window_close::<") > at("/* checked chains */"),
        "the window closes after BOTH copies of the loop"
    );
}

#[test]
fn a_comprehension_is_the_counted_range_it_spells() {
    // Its counter steps through the non-null add (`@FR-R-Counter`), its index chain runs
    // plain behind the guard (`@FR-R-GuardedChain`), and its pushes are reserved.
    let rust = emit("comprehension", &[]);
    let w2 = body(&rust, "n_w2");
    // Since loft#1619 the end `n` is taken once into a local the range proof ranges, so the
    // counter is ranged too and steps through the plain add (`@FR-R-Range`), which is the
    // stronger form of the same claim.
    assert!(
        (w2.contains("var_i__index = ops::op_add_long_nn((var_i__index), (1_i64))")
            || w2.contains("var_i__index = ((var_i__index).wrapping_add(1_i64))"))
            && !w2.contains("var_i__index = ops::op_add_int("),
        "the comprehension's counter steps through the non-null or the plain add:\n{w2}"
    );
    assert!(
        w2.contains("@FR-R-GuardedChain guard") && w2.contains(".wrapping_mul(var_i)"),
        "the comprehension's `i * i + salt` runs plain behind the guard:\n{w2}"
    );
    assert!(
        w2.contains("push reserve"),
        "the comprehension reserves its trip count:\n{w2}"
    );
}

#[test]
fn the_switch_restores_the_header_push() {
    let rust = emit("off", &[("LOFT_NO_PUSH_WINDOW", "1")]);
    assert!(
        !rust.contains("vector::push_window(") && !rust.contains("push_windowed::<"),
        "LOFT_NO_PUSH_WINDOW=1 opens no window anywhere"
    );
    let w1 = body(&rust, "n_w1");
    assert!(
        w1.contains("stores.push_hoisted::<") && w1.contains("push reserve"),
        "w1 pushes through its header again, still reserved:\n{w1}"
    );
}

#[test]
fn the_verify_form_checks_every_windowed_push_and_the_close() {
    let rust = emit("verify", &[("LOFT_HOIST_VERIFY", "1")]);
    let w1 = body(&rust, "n_w1");
    assert!(
        w1.contains("stores.push_windowed::<i64, true>(")
            && w1.contains("stores.push_window_close::<true>("),
        "LOFT_HOIST_VERIFY=1 emits the checking form of the push and the close:\n{w1}"
    );
}

#[test]
fn a_mint_left_on_its_templates_is_still_a_growth() {
    // `(R-Base)`: r1 appends a VECTOR element — no struct, so the mint earns no push
    // header and keeps the runtime's append — beside a read of a sibling field of the same
    // record.  That append grows the record's store, so the sibling takes no element base.
    let rust = emit("growth", &[]);
    let r1 = body(&rust, "n_r1");
    assert!(
        !r1.contains("__vb_"),
        "r1 grows a store through its mint, so it binds no element base:\n{r1}"
    );
}
