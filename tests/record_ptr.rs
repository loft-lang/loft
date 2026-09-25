// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-RecPtr` — the EMISSION pins.  A plain-record VIEW (`e = tbl[i]?`, `s = o.inner`)
//! carries its address (`__pa_N`) for the rest of its block: every scalar field read is
//! `vector::rec_get` through it, every in-place field write `vector::rec_set`, and a callee
//! twin's scalar inputs are read through it at the call.  A view the remainder rebinds, a
//! remainder that grows a store or frees a record, and a nullable view all decline.
//! `LOFT_NO_RECORD_PTR=1` (and `LOFT_NO_VECTOR_BASE=1`, one rule) restores the store reads.
//! The cell corpus (`tests/scripts/157-record-ptr.loft`) says the VALUES hold on both
//! backends, in every switch state and under the falsifiers; this pins what is emitted.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELLS: &str = "tests/scripts/157-record-ptr.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_RECORD_PTR")
        .env_remove("LOFT_NO_VECTOR_BASE")
        .env_remove("LOFT_HOIST_VERIFY");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    // One file per TEST: the tests run on threads of one process.
    let out = std::env::temp_dir().join(format!("loft_record_ptr_{}_{tag}.rs", std::process::id()));
    let status = loft(
        &[
            "--native-emit",
            out.to_str().unwrap(),
            src.to_str().unwrap(),
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

fn run_ok(args: &[&str], env: &[(&str, &str)]) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut a: Vec<&str> = args.to_vec();
    let s = src.to_str().unwrap().to_string();
    a.push(&s);
    let out = loft(&a, env);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.trim_end().ends_with("ok"),
        "{args:?} {env:?}: exit {:?}\nstdout: {stdout}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn the_cells_hold_on_both_backends_in_every_switch_state_under_the_falsifiers() {
    run_ok(&["--interpret"], &[("LOFT_STRICT_STORES", "1")]);
    for switch in [
        None,
        Some(("LOFT_NO_RECORD_PTR", "1")),
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
            run_ok(&["--native"], &env);
        }
    }
}

#[test]
fn a_view_binds_its_address_and_reads_and_writes_go_through_it() {
    let rust = emit("on", &[]);
    let main = body(&rust, "n_main");
    // r1, r2, r3 (two views), r5, r6's SECOND binding, r8 (nullable), r9, r10, r12's `e` (its
    // copy `e2 = e` reads the fields), r13 and r16's three loop variables — fourteen addresses.
    assert_eq!(
        main.matches("//@FR-R-RecPtr record view address for")
            .count(),
        14,
        "main should bind exactly its fourteen admitted views:\n{main}"
    );
    assert!(
        main.contains("vector::rec_get::<i64>(__pa_"),
        "an integer field read goes through the address:\n{main}"
    );
    assert!(
        main.contains("vector::rec_get::<f32>(__pa_")
            && main.contains("vector::rec_get::<f64>(__pa_"),
        "r10's single and float reads go through the address:\n{main}"
    );
    assert!(
        main.contains("vector::rec_set::<i64>(__pa_"),
        "r2's write through the view is one store through the address:\n{main}"
    );
    // r5 — the callee's parameter is a TUPLE PARAMETER (`(R-ValueLocal)`, 2026-09-25), so
    // the site reads the view's two fields through the address into the tuple it hands
    // over — where the `__inv` twin took them as inputs read the same way before.
    assert!(
        main.contains("n_rp_ex(cell, (unsafe { vector::rec_get::<i64>(__pa_"),
        "r5's call must read its tuple argument through the address:\n{main}"
    );
    // r4 — a field projection view, read and written; r11 — a view written beside an
    // element write through the vector.
    let r4 = body(&rust, "n_rp_r4");
    assert!(
        r4.contains("record view address for var_s4") && r4.contains("rec_set::<i64>("),
        "the projection view binds its address and writes through it:\n{r4}"
    );
    let r11 = body(&rust, "n_rp_r11");
    assert!(
        r11.contains("record view address for var_e11") && r11.contains("rec_get::<i64>("),
        "the element view beside another route's write binds its address:\n{r11}"
    );
}

#[test]
fn the_switches_restore_the_store_reads() {
    for switch in ["LOFT_NO_RECORD_PTR", "LOFT_NO_VECTOR_BASE"] {
        let rust = emit(switch, &[(switch, "1")]);
        assert!(
            !rust.contains("//@FR-R-RecPtr record view address for"),
            "{switch}=1 must bind no record view address"
        );
        assert!(
            !rust.contains("vector::rec_get::<") && !rust.contains("vector::rec_set::<"),
            "{switch}=1 must read and write through the store again"
        );
        let main = body(&rust, "n_main");
        // r5's tuple argument is read through the store again, field by field.
        assert!(
            main.contains("n_rp_ex(cell, ({{let db = (var_e); if db.rec == 0"),
            "{switch}=1: r5 reads its tuple argument through the store:\n{main}"
        );
    }
}

#[test]
fn the_declines_are_the_designed_ones() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let out = loft(
        &["--native-emit", "/dev/null", src.to_str().unwrap()],
        &[("LOFT_TRACE_RECPTR", "1")],
    );
    let trace = String::from_utf8_lossy(&out.stderr);
    let lines: Vec<&str> = trace
        .lines()
        .filter(|l| l.starts_with("recptr: n_main declines `e"))
        .collect();
    // r6's first binding (rebound), r7 (a push), r9's `e9` and r12's copy pair.
    assert!(
        lines
            .iter()
            .any(|l| l.ends_with("the remainder rebinds the view")),
        "r6's first binding must decline as a rebind:\n{trace}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.ends_with("the remainder may grow a store")),
        "r7's push must decline the view:\n{trace}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("`e2`") && l.contains("frees a record before a use")),
        "r12's copy is a fresh record released before a use of it:\n{trace}"
    );
}
