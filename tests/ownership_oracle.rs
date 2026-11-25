// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! The store records' standing gates, on the one lifetime report (`LOFT_LIFETIME_CHECK`,
//! `@PLN228`, STORE_RECORDS.md § The checks).  Each is compile-only (`--emit-image`), so it
//! runs no program:
//!
//! - **no crying wolf** — the report is clean on the verified-correct corpus and on every
//!   generated fuzz cell;
//! - **true positives** — each injected defect (a dropped free of an owned store, of an adopted
//!   return buffer, a free of a borrowed view, of a never-free binding) is reported, and the same
//!   program without the injection is clean;
//! - **observer** — the report does not change the shipped IR or bytecode;
//! - **the ratchet** — the problem count over `tests/scripts` and the language suite does not
//!   rise (`records_lifetime_ratchet`, ci.yml's advisory `leak-scan` job).
//!
//! The ownership oracle's dump modes (`LOFT_OWN_ORACLE=own`) keep one gate here: its fact reads
//! the same on both backends (`oracle_fact_is_backend_identical_si2`).

use loft::file_access as fa;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

mod common;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The lifetime report of `file`, compiled only with the `--lib` directories given and any
/// extra env: its problem lines (the function, the rule, the kind and the reason,
/// tab-separated), or `None` when it printed no report (a program that does not compile).
fn lifetime_report_of(
    file: &Path,
    libs: &[String],
    extra_env: &[(&str, &str)],
) -> Option<BTreeSet<String>> {
    static RUN: AtomicUsize = AtomicUsize::new(0);
    let image = std::env::temp_dir().join(format!(
        "loft_lifetime_{}_{}.store",
        std::process::id(),
        RUN.fetch_add(1, Ordering::Relaxed)
    ));
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.current_dir(root());
    for lib in libs {
        cmd.arg("--lib").arg(lib);
    }
    cmd.arg("--emit-image")
        .arg(&image)
        .arg(file)
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_LIFETIME_CHECK", "-");
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("the loft binary must be runnable");
    let _ = fa::remove_file(&image);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !stderr.lines().any(|l| l.starts_with("# ")) {
        return None;
    }
    Some(
        stderr
            .lines()
            .filter(|l| !l.starts_with('#') && l.matches('\t').count() >= 5)
            .map(|l| {
                let f: Vec<&str> = l.split('\t').collect();
                format!("{}\t{}\t{}\t{}", f[1], f[2], f[3], f[5])
            })
            .collect(),
    )
}

/// The lifetime report of a program that must compile.  Panics when it printed none — a gate
/// that reads silence as clean proves nothing.
fn lifetime_report(file: &Path, extra_env: &[(&str, &str)]) -> BTreeSet<String> {
    lifetime_report_of(file, &[], extra_env).unwrap_or_else(|| {
        panic!(
            "no lifetime report for {} (did it compile?)",
            file.display()
        )
    })
}

/// The verified-correct corpus the records must stay silent on (no crying wolf): the @PLN94
/// probes, the collection-capture stress, and the shapes two ownership derivations once
/// disagreed on.
const CLEAN_CORPUS: &[&str] = &[
    "doc/claude/plans/94-cfg-ownership-dataflow/probes/00-a1b-silent-blindspot.loft",
    "doc/claude/plans/94-cfg-ownership-dataflow/probes/01-cfg-corpus.loft",
    "doc/claude/plans/94-cfg-ownership-dataflow/probes/02-loops-rd.loft",
    "doc/claude/plans/94-cfg-ownership-dataflow/probes/03-ownership.loft",
    "doc/claude/plans/94-cfg-ownership-dataflow/probes/04-precision.loft",
    "doc/claude/plans/94-cfg-ownership-dataflow/probes/05-interproc.loft",
    "doc/claude/plans/94-cfg-ownership-dataflow/probes/06-capture.loft",
    "tests/scripts/505-collection-capture.loft",
    "tests/scripts/85-struct-copy-return-owned.loft",
    "tests/scripts/1017b-a-conditional-borrow-into-its-own-binding.loft",
    "tests/scripts/1326-a-captured-keyed-collection-rebind-replaces.loft",
    "tests/scripts/1331-a-repointed-literal-accumulator-is-detached.loft",
    // Every assignment of a witnessed local mixes ownership; the witness alone releases it.
    "tests/scripts/1200-a-nullable-record-local-frees-what-it-displaces.loft",
    A1B_UAF,
];

const A1B_UAF: &str = "tests/scripts/85-temp-subject-borrow-return-uaf.loft";

/// No crying wolf: the records report nothing across the verified-correct corpus.
#[test]
fn records_clean_on_correct_corpus() {
    for &f in CLEAN_CORPUS {
        let problems = lifetime_report(&root().join(f), &[]);
        assert!(
            problems.is_empty(),
            "the store records flagged correct code in {f}:\n{}",
            problems.into_iter().collect::<Vec<_>>().join("\n")
        );
    }
}

/// The known-wrong A1b plan (the promotion collapse: a temp subject built INTO the return
/// buffer, then a borrowed view of it returned) fails its own fixture at run time, and the
/// correct default does not — the run-time channel this class is caught on.
///
/// It takes all three opt-outs to reach that plan: `LOFT_NO_A1B` restores the collapse,
/// loft#872's work-ref/argument step-over keeps the roles apart anyway, and loft#1078's
/// pass-2-only mint site cannot be handed the promoted buffer.  Any one of the three makes the
/// fixture right — independent guards on the same collapse.
#[test]
fn the_a1b_wrong_plan_fails_at_run_time() {
    let wrong_env: &[(&str, &str)] = &[
        ("LOFT_NO_A1B", "1"),
        ("LOFT_NO_WORKREF_STEPOVER", "1"),
        ("LOFT_NO_P2_OBJECT_WORKREF", "1"),
    ];
    let run = |env: &[(&str, &str)]| -> (bool, String) {
        let mut cmd = loft::platform::process::harness_command(loft_bin());
        cmd.arg("--interpret")
            .arg(root().join(A1B_UAF))
            .env("LOFT_NO_CACHE", "1");
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("run loft");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let (wrong_ok, wrong_err) = run(wrong_env);
    assert!(
        !wrong_ok && wrong_err.contains("assertion failed"),
        "the known-wrong A1b plan must fail its fixture at run time; got ok={wrong_ok}:\n{wrong_err}"
    );
    let (right_ok, right_err) = run(&[]);
    assert!(
        right_ok,
        "the correct default plan must pass its fixture:\n{right_err}"
    );
}

/// Observer: the report changes nothing shipped — `introspect` (the IR and bytecode) is
/// byte-identical with the lifetime check on and off.
#[test]
fn the_lifetime_check_is_a_pure_observer() {
    let introspect = |on: bool| {
        let mut cmd = loft::platform::process::harness_command(loft_bin());
        cmd.arg("introspect").arg(root().join(A1B_UAF));
        if on {
            cmd.env("LOFT_LIFETIME_CHECK", "-");
        }
        let out = cmd.output().expect("run loft introspect");
        out.stdout // the report goes to stderr; stdout is the shipped artefact
    };
    assert_eq!(
        introspect(false),
        introspect(true),
        "the lifetime check changed shipped `introspect` output"
    );
}

/// The ownership fact (`LOFT_OWN_ORACLE=own`) is computed at compile time, so it reads
/// identically under `--interpret` and `--native` (`@FR-O-NoDiverge`).  Release-gated.
#[test]
#[ignore = "native backend needs a C compile per run — runs nightly in miri.yml's `release-gate-sweeps` job; by hand: `cargo test --release --test ownership_oracle oracle_fact_is_backend_identical_si2 -- --ignored`"]
fn oracle_fact_is_backend_identical_si2() {
    let own_facts = |native: bool| -> Vec<String> {
        let mut cmd = loft::platform::process::harness_command(loft_bin());
        cmd.arg(if native { "--native" } else { "--interpret" })
            .arg(root().join("doc/claude/plans/94-cfg-ownership-dataflow/probes/03-ownership.loft"))
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_OWN_ORACLE", "own")
            .env("LOFT_TIMEOUT", "120");
        let out = cmd.output().expect("the loft binary must be runnable");
        let mut v: Vec<String> = String::from_utf8_lossy(&out.stderr)
            .lines()
            .filter(|l| l.starts_with("OWN "))
            .map(str::to_string)
            .collect();
        v.sort();
        v
    };
    assert_eq!(
        own_facts(false),
        own_facts(true),
        "the ownership fact differs between --interpret and --native — @FR-O-NoDiverge violated"
    );
}

/// No crying wolf at generative scale: every `program_ownership` case the @PLN85 fuzz grammar
/// generates (9 shapes × 2 values × 3 churn = 54) reads clean.
// @speed 3.6
#[test]
fn records_clean_on_generated_fuzz_corpus() {
    let fuzz_dir = root().join("doc/claude/plans/85-store-lifetime-retirement/fuzz");
    let cells = std::env::temp_dir().join(format!("loft_oracle_fuzz_{}", std::process::id()));
    let _ = fa::remove_dir_all(&cells);
    fa::create_dir_all(&cells).expect("create cell dir");
    let generated = loft::platform::process::harness_command("python3")
        .arg(fuzz_dir.join("grammar_gen.py"))
        .arg("--out")
        .arg(&cells)
        .output()
        .expect("python3 must run the grammar generator");
    assert!(
        generated.status.success(),
        "grammar_gen failed: {}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let mut offenders = Vec::new();
    let mut count = 0usize;
    for entry in fa::read_dir(&cells).expect("read cells") {
        let path = entry.os_spelling();
        if !fa::has_extension(&path, "loft") {
            continue;
        }
        count += 1;
        let problems = lifetime_report(&path, &[]);
        if !problems.is_empty() {
            offenders.push(format!(
                "{}:\n{}",
                fa::file_name(&path).unwrap(),
                problems.into_iter().collect::<Vec<_>>().join("\n")
            ));
        }
    }
    let _ = fa::remove_dir_all(&cells);
    assert!(count >= 54, "expected ≥54 generated cells, got {count}");
    assert!(
        offenders.is_empty(),
        "the store records cried wolf on {} generated fuzz cell(s):\n{}",
        offenders.len(),
        offenders.join("\n---\n")
    );
}

/// The problem count over `tests/scripts` and every language-suite program may only fall.
/// Each line is a records gap or a compiler defect (STORE_RECORDS.md § Every new defect); one
/// resolved lowers the baseline.
const LIFETIME_BASELINE: usize = 38;

/// The programs the ratchet must read a report of, at least: every compiling program of
/// `tests/scripts` and the suite reports, so a sweep that read far fewer read silence.
const LIFETIME_REPORTS_FLOOR: usize = 3000;

#[test]
#[ignore = "sweeps tests/scripts and the suite — runs on every PR in ci.yml's advisory `leak-scan` job; by hand: `cargo test --release --test ownership_oracle records_lifetime_ratchet -- --ignored`"]
fn records_lifetime_ratchet() {
    let dir = root().join("tests/scripts");
    let mut paths: Vec<PathBuf> = fa::read_dir(&dir)
        .expect("read tests/scripts")
        .into_iter()
        .map(|e| e.os_spelling())
        .filter(|p| fa::has_extension(p, "loft"))
        .collect();
    paths.sort();
    paths.extend(common::corpus::suite_programs());
    let tests_lib = root().join("tests/lib").to_string_lossy().to_string();
    let mut found = Vec::new();
    let mut reports = 0usize;
    for path in paths {
        let source = fa::read_to_string(&path).unwrap_or_default();
        let mut libs = common::corpus::args_lib_dirs(&source, &path);
        if path.starts_with(&dir) {
            libs.push(tests_lib.clone());
        }
        let Some(problems) = lifetime_report_of(&path, &libs, &[]) else {
            continue;
        };
        reports += 1;
        for p in problems {
            found.push(format!("{}: {p}", path.display()));
        }
    }
    assert!(
        reports >= LIFETIME_REPORTS_FLOOR,
        "the ratchet read only {reports} lifetime reports (floor {LIFETIME_REPORTS_FLOOR}): \
         most programs did not compile"
    );
    assert!(
        found.len() <= LIFETIME_BASELINE,
        "lifetime problems REGRESSED: {} > baseline {LIFETIME_BASELINE}:\n{}",
        found.len(),
        found.join("\n")
    );
    if found.len() < LIFETIME_BASELINE {
        eprintln!(
            "lifetime ratchet PROGRESS: {} < {LIFETIME_BASELINE} — lower LIFETIME_BASELINE to {}.",
            found.len(),
            found.len()
        );
    }
    eprintln!(
        "lifetime ratchet: {} problem(s) over {reports} report(s)",
        found.len()
    );
}

/// An injected defect is reported, and the program without it is clean: `inject` is the env
/// that makes the compiler emit the defect, `function` the function the report must name, and
/// `name` the variable its reason must name.
fn injected_is_reported(file: &str, inject: (&str, &str), function: &str, name: &str) {
    let mine = |env: &[(&str, &str)]| -> Vec<String> {
        lifetime_report(&root().join(file), env)
            .into_iter()
            .filter(|p| p.starts_with(&format!("{function}\t")))
            .collect()
    };
    let clean = mine(&[]);
    assert!(
        clean.is_empty(),
        "the records cried wolf on the un-injected {file}: {clean:?}"
    );
    let injected = mine(&[inject]);
    assert!(
        injected.iter().any(|p| p.contains(&format!("`{name}`"))),
        "the records FAILED to report the injected defect ({}={}) in {function} (vacuous?): {injected:?}",
        inject.0,
        inject.1
    );
}

/// A dropped free of a vector local's `OpDatabase` store (`__vdb_1`) is a leak.
#[test]
fn records_flag_an_injected_leak() {
    injected_is_reported(
        "doc/claude/plans/94-cfg-ownership-dataflow/probes/07-leak-positive-control.loft",
        ("LOFT_OWN_INJECT_DROP_FREE", "__vdb_1"),
        "n_make_local",
        "__vdb_1",
    );
}

/// A dropped free of the hidden return buffer a caller allocates and adopts (`__ref_1`, not
/// `OpDatabase`-minted in the caller) is a leak.
#[test]
fn records_flag_an_injected_adopted_leak() {
    injected_is_reported(
        "doc/claude/plans/94-cfg-ownership-dataflow/probes/09-adopt-leak-positive-control.loft",
        ("LOFT_OWN_INJECT_DROP_FREE", "__ref_1"),
        "n_use_adopted",
        "__ref_1",
    );
}

/// A free of a dep-carrying view (`bview`) of a store its copy owns frees that store twice.
#[test]
fn records_flag_an_injected_free_of_a_view() {
    injected_is_reported(
        "doc/claude/plans/94-cfg-ownership-dataflow/probes/08-overfree-positive-control.loft",
        ("LOFT_OWN_INJECT_FREE_BORROWED", "bview"),
        "n_peek",
        "__vdb_1",
    );
}

/// @PLN153 phase 4 — the same view declared `vector<integer>?` (`@FR-L-Null`: the same
/// storage) is the same defect.
#[test]
fn records_flag_an_injected_free_of_a_nullable_view() {
    injected_is_reported(
        "doc/claude/plans/94-cfg-ownership-dataflow/probes/08b-overfree-positive-control-nullable.loft",
        ("LOFT_OWN_INJECT_FREE_BORROWED", "bview"),
        "n_peek",
        "__vdb_1",
    );
}

/// `@FR-O-Override`: a free of a never-free binding (`d`, a nullable local that only views a
/// projection) in a spelling neither backend intercepts releases what it views.
#[test]
fn records_flag_an_injected_free_of_a_never_free_binding() {
    injected_is_reported(
        "tests/scripts/a-nullable-view-local-does-not-free-what-it-displaces.loft",
        ("LOFT_OWN_INJECT_FREE_SKIPFREE", "d"),
        "n_view_arg_field_then_mint",
        "d",
    );
}
