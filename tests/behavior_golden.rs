// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN102 arc-E flip-gate — Gate 2: the behavioural-OUTPUT drift gate (the instrument).
//
// A curated corpus (tests/golden/behavior/corpus.loft) prints one labelled VALUE per
// frozen semantic surface; its stdout is pinned to tests/golden/behavior/corpus.out.
// A change to that output is a behavioural drift — and POST-FREEZE, a change to a VALID
// program's VALID output is a silent break (the general silent-semantics detector that
// the layout gate cannot see). The corpus prints VALUES only, never error PROSE, so an
// (improvable, E1) wording change never trips it — only frozen behaviour does.
//
// Two assertions:
//   1. drift  — the interpreter's output equals the committed golden.
//   2. parity — --native produces the SAME output (the master invariant; a divergence
//               IS the bug). Bounded by LOFT_TIMEOUT (rustc can hang).
//
// A behavioural change is not forbidden — it must be DELIBERATE: re-bless the golden
//   LOFT_BLESS_BEHAVIOR=1 cargo test --release --test behavior_golden behavior_golden_interpret
// and, post-freeze, classify it (additive / bugfix / silent-break) — a silent-break
// requires a CONTRACT_VERSION bump, enforced git-side by scripts/check_contract_goldens.sh.

use loft::file_access as fa;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/behavior/corpus.loft")
}
fn golden() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/behavior/corpus.out")
}

/// Run the corpus on `backend`, returning stdout (errors rendered compact so a typed
/// error would surface as its stable CODE, not prose).
fn run(backend: &str) -> String {
    let out = loft::platform::process::harness_command(loft_bin())
        .arg(backend)
        .arg(corpus())
        .env("LOFT_ERRORS", "compact")
        .env("LOFT_TIMEOUT", "120")
        .output()
        .expect("failed to invoke loft binary");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn behavior_golden_interpret() {
    let got = run("--interpret");

    if std::env::var("LOFT_BLESS_BEHAVIOR").is_ok() {
        fa::create_dir_all(golden().parent().unwrap()).unwrap();
        fa::write(golden(), &got).unwrap();
        eprintln!("blessed {}", golden().display());
        eprintln!(
            "@PLN102 flip-gate: a behavioural re-bless is a semantic change — post-freeze \
             classify it (additive/bugfix/silent-break); a silent-break needs a \
             CONTRACT_VERSION bump."
        );
        return;
    }

    let expected = fa::read_to_string(golden()).unwrap_or_else(|_| {
        panic!(
            "missing behaviour golden {} — regenerate with LOFT_BLESS_BEHAVIOR=1",
            golden().display()
        )
    });
    assert_eq!(
        got, expected,
        "\nBEHAVIOURAL OUTPUT CHANGED. If intentional: re-bless (LOFT_BLESS_BEHAVIOR=1), \
         hand-verify the diff, and (post-freeze) classify it — a changed VALUE on a VALID \
         program is a silent semantics break. If not: a regression just got caught.\n"
    );
}

#[test]
fn behavior_parity_native_equals_interpret() {
    // The master invariant: interpret == native. A divergence here IS the bug.
    let interp = run("--interpret");
    let native = run("--native");
    assert_eq!(
        interp, native,
        "\nBACKEND DIVERGENCE on the behaviour corpus: --native output differs from \
         --interpret. This is the #1 stability red-flag (two generators reading one IR, \
         drifting).\n"
    );
}
