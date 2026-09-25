// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Compile-time COMPLEXITY guards.
//!
//! Ordinary tests catch a wrong answer. These catch a right answer that arrives
//! too late to be one: a compile that is superlinear in the size of the source.
//! That failure mode has no diagnostic and no wrong output — the process simply
//! sits at 99 % CPU — so it reads as a hang, and the only thing that reports it
//! is a bound like the ones here.
//!
//! The bounds are deliberately loose. A guard that fails when the box is busy
//! teaches people to ignore it, so each one is set roughly two orders of
//! magnitude above the measured healthy time and still an order of magnitude
//! BELOW the defect it guards. What it detects is a change of complexity class,
//! which moves the number by 100×, not a 20 % regression — `make speed` is the
//! instrument for those.

use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// loft#854 — a vector literal's elements must not cost O(n²) to compile.
///
/// `use_analysis::ownership_of` summarises the WHOLE function to answer about one
/// value, and `scopes::scan_set` asks once per assignment. A vector literal is one
/// assignment per element, so an n-element literal walked (and cloned the defining
/// right-hand sides of) the function n times. crawler's generated terrain file — a
/// single 86 400-element `vector<integer>` — took over 13 minutes, and five of its
/// `make` targets inherited that, so none of them were ever run.
///
/// Measured on the fix commit, `--interpret --check`, this box:
///
/// | elements | before  | after |
/// |---------:|--------:|------:|
/// |    2 000 |  0.68 s | 0.07 s |
/// |    4 000 |  2.34 s | 0.08 s |
/// |    8 000 |  9.42 s | 0.14 s |
/// |   16 000 | ~37 s   | 0.23 s |
///
/// A doubling cost 4× before and costs ~1.6× now. Both sides of the 20 000-element
/// bound below are MEASURED, not extrapolated — the guard was run against the
/// reverted fix to prove it can fail: **69.9 s before, 0.33 s after**. The 30 s
/// budget therefore sits ~90× above the healthy time and less than half the
/// defect's, which is the margin a complexity guard wants in both directions.
#[test]
fn issue854_a_vector_literal_compiles_in_linear_time() {
    const N: usize = 20_000;
    const BUDGET_SECS: u64 = 30;

    let mut src = String::with_capacity(N * 4 + 128);
    src.push_str("pub fn big() -> vector<integer> { [");
    for i in 0..N {
        if i > 0 {
            src.push(',');
        }
        src.push_str(&((i * 7 + 3) % 97).to_string());
    }
    src.push_str("] }\nfn main() { v = big(); println(\"len={len(v)}\"); }\n");

    let dir = std::env::temp_dir().join("loft_issue854_scaling");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let file = dir.join("literal.loft");
    std::fs::write(&file, &src).expect("write the fixture");

    let t = Instant::now();
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg("--check")
        .arg(&file)
        // Without this the guard is VACUOUS on every run after the first: loft
        // answers an unchanged file from the startup cache in ~90 ms, so a
        // reintroduced O(n²) compile would sit behind a cache hit and pass.
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_ERRORS", "compact")
        .env("LOFT_TIMEOUT", "600")
        .output()
        .expect("failed to invoke the loft binary");
    let took = t.elapsed();

    assert!(
        out.status.success(),
        "the fixture must COMPILE — a guard that measures a failing compile measures \
         nothing: stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        took.as_secs() < BUDGET_SECS,
        "compiling a {N}-element vector literal took {took:?}, over the {BUDGET_SECS}s \
         budget — that is a complexity regression, not slowness. The healthy time is \
         ~0.3s; O(n²) here is ~58s. Profile it with \
         `make profile ARGS=\"--interpret --check <file>\" PROFILE_FLAGS=\"--no-cache\"` \
         and see PERFORMANCE.md § Profiling a run."
    );
}

/// `LOFT_TIMING` splits the front end by phase, and the phases account for all of it.
///
/// `parse_default`, `parse_user`, `scopes` and `lints` are four timers, and `front_end` is a
/// fifth clock started with the first of them.  A timer wired around the wrong span reports
/// zero or double-counts, and either way the four stop summing to the fifth, which is what
/// this compares.  The only work between them is the `#c` binding gate, so a healthy sum
/// sits within a few percent of the total; the bound is loose because it is a WIRING check,
/// not a measurement.
#[test]
fn loft_timing_phases_sum_to_the_front_end() {
    let dir = std::env::temp_dir().join(format!("loft_timing_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("t.loft");
    std::fs::write(
        &file,
        "fn main() {\n  x = 0;\n  for i in 0..10 { x += i; }\n  println(\"{x}\");\n}\n",
    )
    .expect("write fixture");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg("--check")
        .arg(&file)
        // A warm cache skips the parse, and a parse that did not run times nothing.
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TIMING", "1")
        .env("LOFT_TIMEOUT", "120")
        .output()
        .expect("failed to invoke the loft binary");
    let _ = std::fs::remove_dir_all(&dir);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "fixture must compile: {stderr}");
    let field = |name: &str| -> f64 {
        let key = format!("{name}=");
        let at = stderr
            .find(&key)
            .unwrap_or_else(|| panic!("LOFT_TIMING printed no `{name}=`:\n{stderr}"));
        let rest = &stderr[at + key.len()..];
        rest[..rest.find("ms").expect("a value in ms")]
            .parse()
            .expect("a number")
    };
    let phases = ["parse_default", "parse_user", "scopes", "lints"].map(field);
    let total = field("front_end");
    for (name, v) in ["parse_default", "parse_user", "scopes", "lints"]
        .iter()
        .zip(phases)
    {
        assert!(
            v > 0.0,
            "`{name}` timed nothing ({v} ms) — its timer is not around the phase"
        );
    }
    let sum: f64 = phases.iter().sum();
    assert!(
        (sum - total).abs() <= total * 0.10,
        "the phases sum to {sum:.2} ms against a front end of {total:.2} ms — a timer is \
         missing a phase or counting one twice:\n{stderr}"
    );
}
