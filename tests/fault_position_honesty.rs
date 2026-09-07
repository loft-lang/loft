// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A reported fault position names the statement that faulted, or nothing (loft#1262).
//!
//! `State::source_spans` is sparse — only the fault-prone constructs the parser wraps
//! in a `Span` get an entry.  The lookup was `range(..=pc).next_back()`, which is the
//! right question for a pc INSIDE such a construct and the wrong one for any other:
//! the nearest preceding span then belongs to whatever unrelated statement happened
//! to be wrapped last.  The table now records each span's pc range, so a pc no span
//! covers answers `None` instead of a confident wrong line.
//!
//! Both directions are pinned below.  `a_call_site_still_reports_its_position` is the
//! one that keeps the rest honest: without it, a lookup that resolved NOTHING would
//! satisfy every row about not lying.

use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Run `src` on the interpreter and return everything it printed.
fn run(tag: &str, src: &str) -> String {
    let root = std::env::temp_dir().join(format!("loft_1262_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("mkdir");
    let file = root.join("p.loft");
    std::fs::write(&file, src).expect("write p.loft");
    let out = Command::new(loft_bin())
        .current_dir(&root)
        .args(["--interpret", "p.loft"])
        .env("LOFT_TIMEOUT", "120")
        // The probes read the panic hook's own output.  A Rust backtrace prints frames
        // spelled `at <path>:<line>:<col>` too, so leaving this to the harness makes the
        // rows below answer differently under `RUST_BACKTRACE=1` — which every CI
        // workflow here sets.
        .env("RUST_BACKTRACE", "0")
        .output()
        .expect("run loft");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&root);
    text
}

/// The loft source position the panic hook prints, if any.
///
/// `at ` alone does not say it: a Rust backtrace frame is spelled the same way, so the
/// bare prefix answers "yes" for `at /rustc/…/panicking.rs:679:5` — a line about the
/// panic machinery, not a claim about the user's program.  The position this file is
/// about always names a `.loft` file.
fn at_line(out: &str) -> Option<&str> {
    out.lines()
        .find(|l| l.trim_start().starts_with("at ") && l.contains(".loft:"))
}

/// A write through a locked store faults from inside the store layer.  The write is
/// an assignment, which carries no span of its own, so the hook has nothing that
/// covers it — and the padding statements above give it something plausible to
/// inherit.  The reported line was 7, the arithmetic; the write is on line 8.
///
/// loft#1405 changed what this probe's fault SAYS — the author's own `#lock` now
/// reaches them as a loft fault instead of the store layer's `assert!`, so the text
/// moved from "Write to read-only store" to the rendered `locked store` error.  The
/// property under test is unchanged and still the point: that fault carries
/// `position: None`, so a pc no span covers must still be given no line at all.
#[test]
fn an_uncovered_fault_does_not_borrow_an_earlier_statements_line() {
    let out = run(
        "padded",
        "struct Counter { value: integer }\n\
         fn main() {\n\
         \x20 d = Counter { value: 5 };\n\
         \x20 d#lock = true;\n\
         \x20 x = 1;\n\
         \x20 y = 2;\n\
         \x20 z = x + y;\n\
         \x20 d.value = 77;\n\
         \x20 print(\"{z}\");\n\
         }\n",
    );
    assert!(
        out.contains("locked store"),
        "the probe must still reach the store-lock fault:\n{out}"
    );
    assert!(
        at_line(&out).is_none(),
        "a fault no span covers must not be given a position at all, and it was \
         given line 7 — the arithmetic, not the write on line 8:\n{out}"
    );
}

/// The same fault with no preceding statement in the user's file at all.  The nearest
/// recorded span is then in the stdlib, 227 bytecode positions back, and the reader
/// was sent to `default/05_coroutine.loft` — a file the program never mentions.
#[test]
fn an_uncovered_fault_never_names_a_file_the_program_never_used() {
    let out = run(
        "bare",
        "struct Counter { value: integer }\n\
         fn main() {\n\
         \x20 d = Counter { value: 5 };\n\
         \x20 d#lock = true;\n\
         \x20 d.value = 77;\n\
         }\n",
    );
    assert!(
        out.contains("locked store"),
        "the probe must still reach the store-lock fault:\n{out}"
    );
    assert!(
        at_line(&out).is_none(),
        "a fault no span covers must not be positioned:\n{out}"
    );
    assert!(
        !out.contains("05_coroutine.loft"),
        "no stdlib file may be named for a fault in the user's own program:\n{out}"
    );
}

/// The control, and the reason the lookup keeps a domain rather than being switched
/// off.  A call IS wrapped, so a fault raised at one resolves to the call site
/// through the same covering lookup.  `panic` reports through the loft diagnostic
/// renderer, which reads `State::source_loc_for` — if making that lookup exact had
/// cost it its answers, this row would go silent.
#[test]
fn a_call_site_still_reports_its_position() {
    let out = run(
        "covered",
        "fn main() {\n\
         \x20 x = 1;\n\
         \x20 panic(\"boom\");\n\
         }\n",
    );
    assert!(
        out.contains("p.loft:3:"),
        "the call site on line 3 must still be reported:\n{out}"
    );
}

/// A failing `assert` takes the same route, and is the shape a reader meets most.
#[test]
fn a_failing_assert_still_reports_its_position() {
    let out = run(
        "assert",
        "fn main() {\n\
         \x20 x = 1;\n\
         \x20 assert(x == 2, \"x must be two\");\n\
         }\n",
    );
    assert!(
        out.contains("p.loft:3:"),
        "the assert on line 3 must still be reported:\n{out}"
    );
}
