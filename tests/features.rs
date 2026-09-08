// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @I81 / @PLN92 strand 3 — the interpret half of the "example-must-run" guard.
//!
//! Every `tests/docs/features/*.loft` is a `## Example` extracted from a
//! `loft-lang/features` issue by `tools/features/gen.loft` (only complete-program
//! examples land here — library / syntax fragments are mirrored but not tested).
//! This runs each on the interpreter and asserts a clean exit; if an authored
//! example stops running, CI goes red.  The native half is
//! `tests/native.rs::native_features`; the no-drift half is `make features-check`.

use std::path::PathBuf;
use std::process::Command;

/// Collect the generated feature examples (sorted for a stable failure order).
fn feature_examples() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = match std::fs::read_dir("tests/docs/features") {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    files.sort();
    files
}

// @speed 3.5
#[test]
fn features_examples_interpret() {
    let files = feature_examples();
    assert!(
        !files.is_empty(),
        "no tests/docs/features/*.loft found — run `make features-gen`"
    );
    // loft#1238 — establish the precondition instead of assuming it.
    //
    // Each example runs under a 60s budget. An example that `use`s a library needs that
    // library's cdylib BUILT, and the first run on a checkout, or after a loft-ffi or flag
    // change, finds it missing or stale (@PLN159 phase C keyed the cdylib on those alone,
    // so a plain loft commit no longer moves it). Under a parallel runner every
    // process that wants it arrives at once, they queue on the one global native-build lock,
    // and whoever is at the back is killed by its own budget having built nothing — so the next
    // attempt starts from the same stale state and repeats it. That is this test's flake, and
    // it is not the example being slow: the one that tripped it takes 0.1s warm, and twelve
    // concurrent copies finish in 0.16s.
    //
    // `make ci` warms before the suite, and while that holds this is a no-op costing one
    // subprocess. But a test that depends on the Makefile having warmed for it is a test whose
    // precondition lives somewhere else — it passes or fails on invocation order, which is
    // exactly how this reached the flake list. Warming here makes the test self-sufficient
    // under `cargo test`, `cargo nextest`, and a bare `--test features` alike.
    //
    // Deliberately NOT under a timeout: a cold build legitimately takes minutes, and it happens
    // ONCE here rather than inside some example's budget. A failure to warm is not fatal either
    // — the examples then behave as they did before, and the timing report below says why.
    let warm = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args(["cache", "warm", "--from", "tests/docs/features"])
        .output();
    if let Ok(w) = warm
        && !w.status.success()
    {
        eprintln!(
            "note: `loft cache warm` returned {} before the feature examples — a library-using \
             example may now pay a cold build inside its 60s budget (loft#1238)",
            w.status
        );
    }
    let mut failures = Vec::new();
    // loft#1238 — time every example, and report the slowest few WITH the failure.
    //
    // This test hard-kills an example at `LOFT_TIMEOUT`, and when it fired the report named
    // the example and the phase and nothing else — so a run that took 71s could not be told
    // apart from one where a single example stalled while the rest were instant. Both readings
    // were live, and choosing between them needed a reproduction nobody had: the example that
    // tripped it takes 0.1s on its own, and twelve concurrent copies finish in 0.16s.
    //
    // The timing is collected unconditionally and printed only ON FAILURE, so a green run stays
    // silent. It is not a threshold and it does not gate: it turns the next occurrence into
    // evidence about WHICH of the two shapes this is, which is what the issue is missing.
    let mut timings: Vec<(std::time::Duration, PathBuf)> = Vec::new();
    for f in &files {
        let started = std::time::Instant::now();
        let out = Command::new(env!("CARGO_BIN_EXE_loft"))
            .args(["--interpret", &f.to_string_lossy()])
            .env("LOFT_TIMEOUT", "60")
            // loft#1238 — arm the build timing in the CHILD, so a failure carries WHY it was
            // slow and not merely that it was.  The example that trips this does `use random`,
            // and the 60s is a cdylib rebuild: `make ci` rebuilds loft, which moves
            // `native_artifact_cache_key` (a content hash of the loft build), which makes every
            // cached native artifact stale, and the first user of each pays the rebuild. That
            // showed up here as `cdylibstale loft_random|stamped=…|cur=…`, which is the line
            // that ends the investigation.
            //
            // The child's stderr is captured and reproduced only on failure, so a green run
            // prints nothing extra.
            .env("LOFT_TIMING", "1")
            .output()
            .expect("spawn loft");
        timings.push((started.elapsed(), f.clone()));
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if !out.status.success() || combined.contains("panicked") {
            // Wide enough to keep the `[loft-timing]` lines above the failure itself: the
            // build events are emitted BEFORE the program runs, so a 6-line tail cut off
            // exactly the evidence (loft#1238).
            let tail: Vec<&str> = combined.lines().rev().take(20).collect();
            let tail: Vec<&str> = tail.into_iter().rev().collect();
            failures.push(format!("{}:\n  {}", f.display(), tail.join("\n  ")));
        }
    }
    let slowest = if failures.is_empty() {
        String::new()
    } else {
        timings.sort_by_key(|(d, _)| std::cmp::Reverse(*d));
        let total: f64 = timings.iter().map(|(d, _)| d.as_secs_f64()).sum();
        let rows: Vec<String> = timings
            .iter()
            .take(5)
            .map(|(d, p)| {
                format!(
                    "  {:>7.2}s  {}",
                    d.as_secs_f64(),
                    p.file_name().unwrap_or(p.as_os_str()).to_string_lossy()
                )
            })
            .collect();
        format!(
            "\n\n{} examples took {total:.1}s in total; the slowest were:\n{}\n\
             (one example far above the rest is a stall in THAT example; every example slow \
             is the box being saturated — loft#1238)",
            timings.len(),
            rows.join("\n")
        )
    };
    assert!(
        failures.is_empty(),
        "{} feature example(s) failed on --interpret:\n{}{slowest}",
        failures.len(),
        failures.join("\n---\n")
    );
}
