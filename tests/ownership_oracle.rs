// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! @PLN94 Phase 5 — the CFG+dataflow ownership oracle as a STANDING check, run BESIDE the shipped
//! analysis (coexistence: it never drives codegen; SI-1 holds). This binary makes the H-tier gates
//! permanent so every CI run re-proves them:
//!
//! - **4.2 no crying wolf** — `LOFT_OWN_ORACLE=check` is 0 RED on the verified-correct corpus
//!   (`oracle_clean_on_correct_corpus`).
//! - **4.3 true positive** — the known-wrong `LOFT_NO_A1B` plan is flagged RED, and the correct
//!   default is not (`oracle_flags_the_a1b_wrong_plan`).
//! - **SI-1 observer** — the oracle does not perturb shipped output: `introspect` is byte-identical
//!   with the oracle on vs off (`oracle_is_a_pure_observer_si1`).
//! - **SI-2 backend fact-identity** — the fact reads identically under `--interpret` and `--native`
//!   (`oracle_fact_is_backend_identical_si2`, release-gated: rustc per run).
//!
//! Design + the strictness verification (why it catches the A1b class the runtime gates miss):
//! `doc/claude/plans/94-cfg-ownership-dataflow/PHASE4_DESIGN.md`.

use loft::file_access as fa;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Run the oracle in `mode` over `file` (with any extra env) and return its stderr lines.
fn run_oracle(file: &str, mode: &str, extra_env: &[(&str, &str)], native: bool) -> String {
    let mut cmd = Command::new(loft_bin());
    cmd.arg(if native { "--native" } else { "--interpret" })
        .arg(root().join(file))
        .env("LOFT_NO_CACHE", "1") // force scopes::check to re-run on the user file
        .env("LOFT_OWN_ORACLE", mode);
    if native {
        cmd.env("LOFT_TIMEOUT", "120"); // rustc can hang — name the phase if it does
    }
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("the loft binary must be runnable");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The deduplicated `RED …` findings the `check` mode reports (it re-runs per compilation unit, so
/// the same finding repeats; the SET is what matters).
fn check_reds(file: &str, extra_env: &[(&str, &str)]) -> BTreeSet<String> {
    run_oracle(file, "check", extra_env, false)
        .lines()
        .filter(|l| l.starts_with("RED "))
        .map(str::to_string)
        .collect()
}

/// The `OWN <fn> …` fact lines (mode `own`), sorted — the per-function shadow-diff summary. Used for
/// the SI-2 backend-identity comparison.
fn own_facts(file: &str, native: bool) -> Vec<String> {
    let mut v: Vec<String> = run_oracle(file, "own", &[], native)
        .lines()
        .filter(|l| l.starts_with("OWN "))
        .map(str::to_string)
        .collect();
    v.sort();
    v
}

/// The verified-correct corpus the check must stay silent on (no crying wolf). Kept to files proven
/// 0-RED (the @PLN94 probes + the collection-capture stress). `85-struct-copy-return-owned`
/// (`n_choose`, the retbuf-materialisation residual) is now INCLUDED — the `reminted` rule resolved it
/// (a `var = src` copy into a var re-minted via `OpDatabase` owns; @PLN94 n_choose fix).
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
    // The three shapes the two derivations disagreed on over the 1247-file corpus until the
    // `@FR-O-Oracle` walk (QUALITY-history.md B7r): a local minted once and rebound by a call that may
    // hand back its argument, a keyed literal repointed at a captured collection inside a
    // closure, and a call delivering through a hidden buffer.  Zero disagreements is the claim.
    "tests/scripts/1017b-a-conditional-borrow-into-its-own-binding.loft",
    "tests/scripts/1326-a-captured-keyed-collection-rebind-replaces.loft",
    "tests/scripts/1331-a-repointed-literal-accumulator-is-detached.loft",
];

const A1B_UAF: &str = "tests/scripts/85-temp-subject-borrow-return-uaf.loft";

/// 4.2 — the check cries no wolf: 0 RED across the verified-correct corpus.
#[test]
fn oracle_clean_on_correct_corpus() {
    for &f in CLEAN_CORPUS {
        let reds = check_reds(f, &[]);
        assert!(
            reds.is_empty(),
            "the ownership oracle flagged correct code (false positive) in {f}:\n{}",
            reds.iter().cloned().collect::<Vec<_>>().join("\n")
        );
    }
}

/// 4.3 — the known-wrong A1b plan (the promotion collapse: a temp subject built INTO the return
/// buffer, then a borrowed view of it returned) fails its own fixture at run time, and the correct
/// default does not — the runtime channel this class is caught on.
///
/// It takes ALL THREE opt-outs to reach that plan now. `LOFT_NO_A1B` restores the promotion
/// collapse, but loft#872's work-ref/argument step-over then keeps the roles apart anyway — pass 2's
/// mint for the inner call asks for a `vector` and the promoted buffer is a record, so it steps to a
/// fresh local and `n_h` comes out correct. loft#1078 added the third: the value-position object
/// literal is a pass-2-ONLY mint site, so it now draws from the `__ref_p2_N` sequence and cannot be
/// handed the promoted buffer whatever the other two do. Any ONE of the three makes this fixture
/// right, which is worth knowing: they are independent guards on the same collapse, and the gate has
/// to disable all of them to have a defect to catch.
///
/// This gate used to ask Check A to FLAG the wrong plan, and it did — but the disagreement it
/// reported was the shadow's own weaker callee-to-caller base translation (`Borrowed(MAX)` for an
/// argument built in place) against the oracle's mint shortcut (`Owned` for any `OpDatabase` var),
/// two derivation defects that happened to differ on this shape.  With the mint shortcut refined and
/// one translation shared by both (`use_analysis::structural_arg_base`), the two derivations agree on
/// the wrong plan as on the right one, and Check A stays CLEAN on both — that is asserted here too,
/// so a future divergence on this fixture is a finding.  Check A's own true positive is the injected
/// one below.
#[test]
fn the_a1b_wrong_plan_fails_at_run_time_and_the_oracles_agree_on_it() {
    let wrong_env: &[(&str, &str)] = &[
        ("LOFT_NO_A1B", "1"),
        ("LOFT_NO_WORKREF_STEPOVER", "1"),
        ("LOFT_NO_P2_OBJECT_WORKREF", "1"),
    ];
    let run = |env: &[(&str, &str)]| -> (bool, String) {
        let mut cmd = Command::new(loft_bin());
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
    for env in [wrong_env, &[][..]] {
        let reds = check_reds(A1B_UAF, env);
        let disagree: Vec<&String> = reds
            .iter()
            .filter(|r| r.contains("fact-disagree"))
            .collect();
        assert!(
            disagree.is_empty(),
            "the two ownership derivations disagreed on the A1b fixture under {env:?}: {disagree:?}"
        );
    }
}

/// Check A's TRUE-POSITIVE gate, injected: `LOFT_OWN_INJECT_FACT_OWNED=inner` forces the shadow's
/// fact for `inner` — a vector header viewing its own `__vdb_` backing, `Borrowed` to the oracle and
/// never filled, so no `Join` softens the comparison — to `Owned`, and Check A must report the
/// `fact-disagree`; the un-injected run must be clean.  Symmetric to the leak-scan and over-free
/// injections — a report whose zero cannot be made non-zero proves nothing.
#[test]
fn oracle_fact_check_flags_an_injected_owned_fact() {
    let disagree = |env: &[(&str, &str)]| -> Vec<String> {
        check_reds(A1B_UAF, env)
            .into_iter()
            .filter(|r| r.contains("fact-disagree") && r.contains("inner"))
            .collect()
    };
    assert!(
        disagree(&[]).is_empty(),
        "Check A cried wolf on the un-injected fixture: {:?}",
        disagree(&[])
    );
    let injected = disagree(&[("LOFT_OWN_INJECT_FACT_OWNED", "inner")]);
    assert!(
        injected
            .iter()
            .any(|r| r.contains("mine=Owned") && r.contains("B=Borrowed")),
        "Check A FAILED to flag the injected Owned fact for inner (vacuous?): {injected:?}"
    );
}

/// SI-1 — the oracle is a pure observer: `introspect` stdout (the shipped IR + bytecode) is
/// byte-identical with the oracle ON (`check`) vs OFF. Proof it only observes.
#[test]
fn oracle_is_a_pure_observer_si1() {
    let introspect = |mode: Option<&str>| {
        let mut cmd = Command::new(loft_bin());
        cmd.arg("introspect").arg(root().join(A1B_UAF));
        if let Some(m) = mode {
            cmd.env("LOFT_OWN_ORACLE", m);
        }
        let out = cmd.output().expect("run loft introspect");
        out.stdout // the oracle writes to stderr only; stdout is the shipped artefact
    };
    assert_eq!(
        introspect(None),
        introspect(Some("check")),
        "the oracle changed shipped `introspect` output — SI-1 (observer) violated"
    );
}

/// SI-2 — backend fact-identity: the ownership fact is computed at compile time (`scopes::check`),
/// so it must read identically under `--interpret` and `--native`. Release-gated (rustc per run).
#[test]
#[ignore = "native backend needs rustc per run — runs nightly in miri.yml's `release-gate-sweeps` job; by hand: `cargo test --release --test ownership_oracle oracle_fact_is_backend_identical_si2 -- --ignored`"]
fn oracle_fact_is_backend_identical_si2() {
    let f = "doc/claude/plans/94-cfg-ownership-dataflow/probes/03-ownership.loft";
    assert_eq!(
        own_facts(f, false),
        own_facts(f, true),
        "the ownership fact differs between --interpret and --native — SI-2 (O-NoDiverge) violated"
    );
}

/// 5.2 — the fuzzer hook: every generated `program_ownership` case (9 shapes × 2 values × 3 churn =
/// 54) runs the oracle check; the whole grammar must be 0 RED (no crying wolf at generative scale,
/// not just the fixed corpus). The generator is the @PLN85 fuzz harness's `grammar_gen.py`.
// @speed 3.6
#[test]
fn oracle_clean_on_generated_fuzz_corpus() {
    let fuzz_dir = root().join("doc/claude/plans/85-store-lifetime-retirement/fuzz");
    let cells = std::env::temp_dir().join(format!("loft_oracle_fuzz_{}", std::process::id()));
    let _ = fa::remove_dir_all(&cells);
    fa::create_dir_all(&cells).expect("create cell dir");
    let generated = Command::new("python3")
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
        // `check_reds` takes a repo-relative path; this cell is absolute, so run the binary directly.
        let out = Command::new(loft_bin())
            .arg("--interpret")
            .arg(&path)
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_OWN_ORACLE", "check")
            .output()
            .expect("run loft on a fuzz cell");
        let reds: BTreeSet<String> = String::from_utf8_lossy(&out.stderr)
            .lines()
            .filter(|l| l.starts_with("RED "))
            .map(str::to_string)
            .collect();
        if !reds.is_empty() {
            offenders.push(format!(
                "{}:\n{}",
                fa::file_name(&path).unwrap(),
                reds.iter().cloned().collect::<Vec<_>>().join("\n")
            ));
        }
    }
    let _ = fa::remove_dir_all(&cells);
    assert!(count >= 54, "expected ≥54 generated cells, got {count}");
    assert!(
        offenders.is_empty(),
        "the ownership oracle cried wolf on {} generated fuzz cell(s):\n{}",
        offenders.len(),
        offenders.join("\n---\n")
    );
}

/// The all-vars leak scan (`LOFT_OWN_ORACLE=check-leak`) ratchet baseline — the "raise it → flag it,
/// don't revert" workflow that drove it 927 → 0 (recognising each codegen-transfer artifact:
/// retbuf/param aliasing, the phantom `__retbuf`, `par` queue frees, work-refs). The scan is now
/// PROMOTED onto `check` (both the `OpDatabase` and the adopted-owned classes) with two firing
/// true-positives; this sweep re-arms the ratchet as a regression guard. It asserts the count does not
/// REGRESS above the baseline; a future gap recognised lowers it further.
const LEAK_SCAN_BASELINE: usize = 0;

#[test]
#[ignore = "sweeps tests/scripts under check-leak — runs on every PR in ci.yml's advisory `leak-scan` job; by hand: `cargo test --release --test ownership_oracle oracle_leak_scan_ratchet -- --ignored`"]
#[allow(clippy::absurd_extreme_comparisons)] // baseline reached 0 — the ratchet endpoint
fn oracle_leak_scan_ratchet() {
    let dir = root().join("tests/scripts");
    let mut total = 0usize;
    for entry in fa::read_dir(&dir).expect("read tests/scripts") {
        let path = entry.os_spelling();
        if !fa::has_extension(&path, "loft") {
            continue;
        }
        let out = Command::new(loft_bin())
            .arg("--interpret")
            .arg(&path)
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_OWN_ORACLE", "check-leak")
            .output()
            .expect("run loft check-leak");
        let reds: BTreeSet<String> = String::from_utf8_lossy(&out.stderr)
            .lines()
            .filter(|l| l.starts_with("RED ") && l.contains(": leak "))
            .map(str::to_string)
            .collect();
        total += reds.len();
    }
    assert!(
        total <= LEAK_SCAN_BASELINE,
        "leak-scan findings REGRESSED: {total} > baseline {LEAK_SCAN_BASELINE}. \
         Update LEAK_SCAN_BASELINE only DOWNWARD (each transfer artifact recognised lowers it)."
    );
    if total < LEAK_SCAN_BASELINE {
        eprintln!(
            "leak-scan ratchet PROGRESS: {total} < {LEAK_SCAN_BASELINE} — lower LEAK_SCAN_BASELINE to {total}."
        );
    }
}

/// The leak-scan TRUE-POSITIVE gate (on the PROMOTED default `check` path): proving the definite-leak
/// scan is not vacuous. A positive-control fixture owns a vector local (`buf`, backed by the
/// OpDatabase store `__vdb_1`), freed at scope exit. `LOFT_OWN_INJECT_DROP_FREE=__vdb_1` drops that
/// free (a genuine leak — the runtime leak-check agrees); the scan MUST flag `__vdb_1` under `check`,
/// and the un-injected run MUST be clean.
#[test]
fn oracle_leak_scan_flags_an_injected_leak() {
    let ctrl = "doc/claude/plans/94-cfg-ownership-dataflow/probes/07-leak-positive-control.loft";
    let reds = |env: &[(&str, &str)]| -> Vec<String> {
        let mut cmd = Command::new(loft_bin());
        cmd.arg("--interpret")
            .arg(root().join(ctrl))
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_OWN_ORACLE", "check"); // promoted: the leak scan runs on the default path
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("run loft check-leak");
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .filter(|l| {
                l.starts_with("RED ") && l.contains(": leak ") && l.contains("n_make_local")
            })
            .map(str::to_string)
            .collect()
    };
    assert!(
        reds(&[]).is_empty(),
        "check-leak cried wolf on the un-injected positive control: {:?}",
        reds(&[])
    );
    let injected = reds(&[("LOFT_OWN_INJECT_DROP_FREE", "__vdb_1")]);
    assert!(
        injected.iter().any(|r| r.contains("__vdb_1")),
        "check-leak FAILED to flag the injected __vdb_1 leak (vacuous?): {injected:?}"
    );
}

/// The ADOPTED-leak TRUE-POSITIVE gate (on the PROMOTED default `check` path): proving the leak scan's
/// adopted-owned class is not vacuous. The positive control binds `adopted = make()`; the caller
/// allocates a hidden NRVO return buffer `__ref_1` (a `caller_hidden_buf` work-ref) it OWNS + frees.
/// `__ref_1` is NOT `OpDatabase`-minted in the caller's body, so the OpDatabase-only recognizer missed
/// it — the gap this class closes. `LOFT_OWN_INJECT_DROP_FREE=__ref_1` drops that free (a genuine leak
/// the runtime leak-check also flags); the scan MUST go RED on `n_use_adopted`'s `__ref_1`, and the
/// un-injected run MUST be clean. (The injection is name-global, so other functions' `__ref_1` may also
/// flag — the assertion pins the fixture's own function.)
#[test]
fn oracle_adopt_leak_flags_an_injected_leak() {
    let ctrl =
        "doc/claude/plans/94-cfg-ownership-dataflow/probes/09-adopt-leak-positive-control.loft";
    let reds = |env: &[(&str, &str)]| -> Vec<String> {
        let mut cmd = Command::new(loft_bin());
        cmd.arg("--interpret")
            .arg(root().join(ctrl))
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_OWN_ORACLE", "check"); // promoted: the adopted class rides on the default path
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("run loft check");
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .filter(|l| {
                l.starts_with("RED ") && l.contains(": leak ") && l.contains("n_use_adopted")
            })
            .map(str::to_string)
            .collect()
    };
    assert!(
        reds(&[]).is_empty(),
        "the leak scan cried wolf on the un-injected adopted positive control: {:?}",
        reds(&[])
    );
    let injected = reds(&[("LOFT_OWN_INJECT_DROP_FREE", "__ref_1")]);
    assert!(
        injected.iter().any(|r| r.contains("__ref_1")),
        "the leak scan FAILED to flag the injected adopted __ref_1 leak (vacuous?): {injected:?}"
    );
}

/// The over-free TRUE-POSITIVE gate (on the PROMOTED default `check` path): proving the over-free
/// check (`run_over_free_check`, Check B) is not vacuous. A positive-control fixture binds a
/// dep-carrying view `bview` of a copied store; the copy owns + frees the store, so `get_free_vars`
/// correctly does NOT free `bview`. `LOFT_OWN_INJECT_FREE_BORROWED=bview` forces an unconditional
/// `OpFreeRef(bview)` (a genuine over-free / double-free); the check MUST flag `free-of-borrowed bview`
/// under `check`, and the un-injected run MUST be clean. Symmetric to the leak-scan injection.
#[test]
fn oracle_over_free_check_flags_an_injected_free() {
    let ctrl =
        "doc/claude/plans/94-cfg-ownership-dataflow/probes/08-overfree-positive-control.loft";
    let reds = |env: &[(&str, &str)]| -> Vec<String> {
        let mut cmd = Command::new(loft_bin());
        cmd.arg("--interpret")
            .arg(root().join(ctrl))
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_OWN_ORACLE", "check"); // promoted: the over-free check runs on the default path
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("run loft check");
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .filter(|l| {
                l.starts_with("RED ") && l.contains("free-of-borrowed") && l.contains("bview")
            })
            .map(str::to_string)
            .collect()
    };
    assert!(
        reds(&[]).is_empty(),
        "the over-free check cried wolf on the un-injected positive control: {:?}",
        reds(&[])
    );
    let injected = reds(&[("LOFT_OWN_INJECT_FREE_BORROWED", "bview")]);
    assert!(
        injected.iter().any(|r| r.contains("bview")),
        "the over-free check FAILED to flag the injected bview over-free (vacuous?): {injected:?}"
    );
}

/// The never-free TRUE-POSITIVE gate (Check D, on the promoted `check` path): proving the
/// @FR-O-Override check is not vacuous.  The B7p guard marks `d` never-free (a nullable local that
/// only views a projection); `LOFT_OWN_INJECT_FREE_SKIPFREE=d` makes `get_free_vars` name a
/// witness-guarded free of `d` against itself — a run-time no-op, but a free of a never-free
/// binding in a spelling neither backend intercepts, which the check MUST report; the un-injected
/// run MUST be clean for `d`.
/// @PLN153 phase 4 — the over-free check sees a NULLABLE view local as the heap local it is.
///
/// `08b` is the positive control (08) with `bview` declared `vector<integer>?`: the same
/// dep-carrying view of the holder's store behind a nullability marker (`@FR-L-Null`, the
/// same storage).  Check B's heap filter asked `heap_dep()` of the local's type BARE, so the
/// nullable spelling was never a candidate and the injected over-free went unflagged while the
/// dense control's was flagged — the oracle green over exactly the twin @PLN153 is about.
/// Both halves: the un-injected run stays clean, the injected one goes RED on `bview`.
#[test]
fn oracle_over_free_check_sees_a_nullable_view_local() {
    let ctrl = "doc/claude/plans/94-cfg-ownership-dataflow/probes/08b-overfree-positive-control-nullable.loft";
    let reds = |env: &[(&str, &str)]| -> Vec<String> {
        let mut cmd = Command::new(loft_bin());
        cmd.arg("--interpret")
            .arg(root().join(ctrl))
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_OWN_ORACLE", "check");
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("run loft check");
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .filter(|l| {
                l.starts_with("RED ") && l.contains("free-of-borrowed") && l.contains("bview")
            })
            .map(str::to_string)
            .collect()
    };
    assert!(
        reds(&[]).is_empty(),
        "the over-free check cried wolf on the un-injected nullable control: {:?}",
        reds(&[])
    );
    let injected = reds(&[("LOFT_OWN_INJECT_FREE_BORROWED", "bview")]);
    assert!(
        injected.iter().any(|r| r.contains("bview")),
        "the over-free check FAILED to flag the injected over-free of a NULLABLE view (the \
         heap filter read the wrapper as not-a-heap-local): {injected:?}"
    );
}

#[test]
fn oracle_override_check_flags_an_injected_free_of_a_never_free_binding() {
    let guard = "tests/scripts/a-nullable-view-local-does-not-free-what-it-displaces.loft";
    let reds = |env: &[(&str, &str)]| -> Vec<String> {
        let mut cmd = Command::new(loft_bin());
        cmd.arg("--check")
            .arg(root().join(guard))
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_OWN_ORACLE", "check");
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("run loft check");
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .filter(|l| {
                l.starts_with("RED ") && l.contains("never-free-freed") && l.contains(" d (v")
            })
            .map(str::to_string)
            .collect()
    };
    assert!(
        reds(&[]).is_empty(),
        "Check D cried wolf on the un-injected guard: {:?}",
        reds(&[])
    );
    let injected = reds(&[("LOFT_OWN_INJECT_FREE_SKIPFREE", "d")]);
    assert!(
        injected.iter().any(|r| r.contains("OpFreeRefIfDistinct")),
        "Check D FAILED to flag the injected free of the never-free `d` (vacuous?): {injected:?}"
    );
}

/// One release mechanism per local: a local whose assignments MIX ownership gets an owner
/// witness (loft#1336) and is never-free, so it must not ALSO be given the loft#1200
/// displacement flag — that flag's guarded `OpFreeRef` was dropped at codegen by the never-free
/// veto on both backends, a dead free the IR carried for every witnessed local.  Check D reports
/// such a dropped free as a `NOTE`; the 1200 guard, whose locals all take the witness, must
/// produce none.  (Observer-only when the witness is switched off, hence the env pin.)
#[test]
fn a_witnessed_local_carries_no_dead_displacement_free() {
    let guard = "tests/scripts/1200-a-nullable-record-local-frees-what-it-displaces.loft";
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--check")
        .arg(root().join(guard))
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_OWN_ORACLE", "check")
        .env_remove("LOFT_NO_OWNER_WITNESS");
    let out = cmd.output().expect("run loft check");
    let notes: Vec<String> = String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|l| l.starts_with("NOTE ") && l.contains("never-free"))
        .map(str::to_string)
        .collect();
    assert!(
        notes.is_empty(),
        "a witnessed local still carries a second, dead release in the IR:\n{}",
        notes.join("\n")
    );
}
