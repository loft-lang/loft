// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#969 / loft#950 — a `for` binding over a collection FIELD is a VIEW, and freeing
//! it at scope exit destroys the struct that owns it.
//!
//! `loop_variable` adopted pass 2's type only when pass 1 had left the slot `Unknown`, and
//! a known-but-DEPLESS type is not unknown.  The collection may acquire its dep only on
//! pass 2, so the right answer was computed and thrown away; the binding was then marked
//! OWNS and the loop's exit freed the store it was only looking into.  The next allocation
//! reused the slot, and every later read through the host struct saw whatever landed there
//! — in moros' client a scalar field came back as the f64 bits of −31.4965, and in
//! dryopea an 8-element vector read as 1.
//!
//! ⚠ **Why a value-level guard exists at all, beside the predicate one.**
//! `src/variables/mod.rs` asserts the adoption rule directly, deliberately: whether pass 1
//! knows the dep is a property of the WHOLE PROGRAM, so a script-level guard risks pinning
//! luck rather than the rule.  That reasoning is right, and it is also why the defect was
//! reported twice from two consumers before anything caught it — neither reporter could
//! cut it down, and seven ingredient-by-ingredient rebuilds of dryopea's case all came out
//! green.  This fixture is the reduction done in the other direction: DOWN from a program
//! that was red, until only the trigger was left.
//!
//! ⚠⚠ **The trigger is a FORWARD REFERENCE, and it is the one line that matters.**
//! `Holder` names `hd_items: vector<Item>` before `Item` is declared.  Move the `Item`
//! declaration above `Holder` and the same program is green on the buggy binary — that is
//! exactly "what pass 1 knows", made into two lines a reader can see.  Anyone editing
//! `tests/fixtures/loop_view_dep/holderlib/src/holder.loft` must keep that order, or this
//! test passes while testing nothing.
//!
//! Calibrated: red on `origin/main` (`3 emit=1`) and green here, on BOTH backends.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Run the consumer fixture on one backend and return its stdout+stderr.
fn run(backend: &str) -> String {
    let root = repo_root();
    let dir = root.join("tests/fixtures/loop_view_dep/consumer");
    // The program cache is keyed on the source, not on the binary's ownership analysis, so
    // a tree that has run this fixture before would answer from the cache and the assertion
    // below would be reading a previous binary's result.
    // The test-profile `loft` (what every other spawning test runs): this fixture needs
    // the binary built from THIS tree, not the release one `make ci` last built — and
    // under the iteration loop the release binary is rebuilt only for the suites that
    // spawn it (@PLN159 I), so spawning it here cost `--subject scopes` a 95 s
    // non-incremental build for one test.
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    if !backend.is_empty() {
        cmd.arg(backend);
    }
    let out = cmd
        .arg("run.loft")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TIMEOUT", "300")
        .current_dir(&dir)
        .output()
        .expect("run the loop_view_dep consumer fixture");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The reported shape: emit, measure, emit — and the third call must still see every
/// element.  Asserted on BOTH backends because the fault is a store lifetime decided at
/// parse time, so it reaches the interpreter and the generated Rust alike.
#[test]
fn a_loop_over_a_collection_field_does_not_free_its_owner() {
    for backend in ["--interpret", ""] {
        let all = run(backend);
        let which = if backend.is_empty() {
            "native"
        } else {
            "interpret"
        };
        // The first two calls pin the fixture as LIVE: a run that never emitted or never
        // measured would satisfy the third assertion vacuously.
        assert!(
            all.contains("1 emit=3"),
            "[{which}] the first emit must see all three items — the fixture is not \
             running what it claims\n{all}"
        );
        assert!(
            all.contains("2 span=2"),
            "[{which}] the measuring walk must answer 2.0 before anything is corrupted\n{all}"
        );
        // The defect: the measuring walk freed the store its binding was only viewing, so
        // the emitter after it saw a vector of one.
        assert!(
            all.contains("3 emit=3"),
            "[{which}] loft#969 — the second emit lost elements, so the loop binding over \
             `hd_items` freed the Holder it only borrows\n{all}"
        );
    }
}

/// The fixture's own guard: the forward reference is the trigger, so a well-meaning tidy-up
/// that declares `Item` before `Holder` would leave this suite green against the bug.
///
/// Reads the order out of the file rather than trusting a comment in it.
#[test]
fn the_fixture_still_declares_item_after_holder() {
    let src = std::fs::read_to_string(
        repo_root().join("tests/fixtures/loop_view_dep/holderlib/src/holder.loft"),
    )
    .expect("read the holder fixture");
    let holder = src
        .find("pub struct Holder")
        .expect("the fixture must declare Holder");
    let item = src
        .find("pub struct Item")
        .expect("the fixture must declare Item");
    assert!(
        holder < item,
        "`Holder` must name `vector<Item>` BEFORE `Item` is declared — that forward \
         reference is what leaves pass 1 without the field's dep, and it is the whole \
         trigger for loft#969.  With the order swapped this fixture passes on a binary \
         that still has the bug."
    );
}
