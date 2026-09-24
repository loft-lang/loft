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
    // Every example runs with its `use`d libraries INTERPRETED (`LOFT_NO_NATIVE_LIBS=1`).
    //
    // By default `--interpret` builds a used library's native cdylib first and falls back to
    // interpreting it, in silence, when that build fails — so the build never decided this
    // test's verdict, and it was all of its cost: two examples (`use lexer`, `use parser`) run
    // in 0.2 s, while building their cdylibs cold took 3 min here and 451 s on the Windows
    // runner, where a test may take half its 600 s limit (`scripts/test_duration_gate.py`).
    // It was also this test's flake (loft#1238): parallel examples queued on the one global
    // native-build lock and the one at the back ran out of its budget having built nothing.
    // The examples run natively in `tests/native.rs::native_features`; this is the
    // interpreter half, and now it measures only the interpreter.
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
    // The examples are independent processes, so they run several at a time (they total ~7 s
    // one after another here); with no library to build there is no shared lock for them to
    // queue on.  Results and timings are collected per example and reported in corpus order.
    let workers = std::thread::available_parallelism()
        .map_or(2, std::num::NonZero::get)
        .min(8);
    let next = std::sync::atomic::AtomicUsize::new(0);
    type Outcome = (std::time::Duration, Option<String>);
    let results: std::sync::Mutex<Vec<Option<Outcome>>> =
        std::sync::Mutex::new(files.iter().map(|_| None).collect());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(f) = files.get(k) else {
                        break;
                    };
                    let started = std::time::Instant::now();
                    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
                        .args(["--interpret", &f.to_string_lossy()])
                        .env("LOFT_TIMEOUT", "60")
                        .env("LOFT_NO_NATIVE_LIBS", "1")
                        // loft#1238 — arm the build timing in the CHILD, so a failure carries
                        // WHY it was slow (a `cdylibstale` line is what ends that investigation).
                        // The child's stderr is reproduced only on failure.
                        .env("LOFT_TIMING", "1")
                        .output()
                        .expect("spawn loft");
                    let elapsed = started.elapsed();
                    let combined = format!(
                        "{}{}",
                        String::from_utf8_lossy(&out.stdout),
                        String::from_utf8_lossy(&out.stderr)
                    );
                    let failure =
                        (!out.status.success() || combined.contains("panicked")).then(|| {
                            // Wide enough to keep the `[loft-timing]` lines above the failure itself:
                            // the build events are emitted BEFORE the program runs (loft#1238).
                            let tail: Vec<&str> = combined.lines().rev().take(20).collect();
                            let tail: Vec<&str> = tail.into_iter().rev().collect();
                            format!("{}:\n  {}", f.display(), tail.join("\n  "))
                        });
                    results.lock().unwrap()[k] = Some((elapsed, failure));
                }
            });
        }
    });
    let mut timings: Vec<(std::time::Duration, PathBuf)> = Vec::new();
    for (f, r) in files.iter().zip(results.into_inner().unwrap()) {
        let (elapsed, failure) = r.expect("every example ran");
        timings.push((elapsed, f.clone()));
        failures.extend(failure);
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
