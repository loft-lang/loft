// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! Plan-57 Phase B (Mechanism B) regression: a captured MUTABLE closure cell
//! (`Reference(__cell_*, _)`) is OWNED by the closure record and freed by the
//! record's cascade (`Stores::free_named`), NOT by the defining frame.
//!
//! Before the fix, `get_free_vars` emitted a defining-frame `OpFreeRef` on the
//! cell and `parser/vectors.rs` bumped its ref-count (`OpIncRc`) so the frame
//! free only decremented.  The ref-count has since been REMOVED entirely
//! (plan-57 Phase C): `free_named` now frees unconditionally, so a captured cell
//! whose frame-free was not suppressed would be freed for real at a factory's
//! return, the next factory call would reuse the freed slot, and two coexisting
//! closures would alias one store → use-after-free / wrong output (probes
//! 02 / 09 / 12).
//!
//! The fix suppresses the captured-cell frame free, so the closure record's
//! cascade is the sole owner.  With the ref-count gone, unconditional free is the
//! DEFAULT — so these shapes run in default mode on BOTH backends and assert
//! correct output: the deterministic guard for a Mechanism-B regression.

use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// `(label, program)` — each prints `PASS` only if its asserts hold.
const CASES: &[(&str, &str)] = &[
    // Deterministic alias detector: f2's cell must NOT alias f1's after f1's
    // factory frame exits.  Under the bug, `a2` reads f2's value (1) not 2.
    (
        "alias_detector",
        r#"
fn make() -> fn() -> integer { n = 0; fn() -> integer { n = n + 1; n } }
fn main() {
  f1 = make();
  a1 = f1();
  f2 = make();
  z = f2();
  a2 = f1();
  assert(a1 == 1, "a1");
  assert(a2 == 2, "a2 ALIAS BUG (f1/f2 share a cell)");
  println("PASS");
}
"#,
    ),
    // Probe 02: two factories, calls interleaved (crashed native pre-fix).
    (
        "two_factory_interleaved",
        r#"
fn make() -> fn() -> integer { n = 0; fn() -> integer { n = n + 1; n } }
fn main() {
  f1 = make();
  f2 = make();
  assert(f1() == 1 && f2() == 1 && f1() == 2 && f2() == 2, "interleave");
  println("PASS");
}
"#,
    ),
    // Probe 12: two closures coexist, calls NOT interleaved.
    (
        "two_coexist_no_interleave",
        r#"
fn make() -> fn() -> integer { n = 0; fn() -> integer { n = n + 1; n } }
fn main() {
  f1 = make();
  f2 = make();
  a = f1() + f1();
  b = f2() + f2();
  assert(a == 3 && b == 3, "coexist");
  println("PASS");
}
"#,
    ),
    // Three coexisting factories — pushes slot reuse further.
    (
        "three_coexist",
        r#"
fn make() -> fn() -> integer { n = 0; fn() -> integer { n = n + 1; n } }
fn main() {
  f1 = make();
  f2 = make();
  f3 = make();
  assert(f1() + f2() + f3() == 3, "three");
  assert(f1() + f2() + f3() == 6, "three again");
  println("PASS");
}
"#,
    ),
];

fn run(mode: &str, src: &std::path::Path) -> (bool, String) {
    let out = Command::new(loft_bin())
        .arg(mode)
        .arg(src)
        .output()
        .unwrap_or_else(|e| panic!("spawn loft {mode}: {e}"));
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success() && combined.contains("PASS"), combined)
}

#[test]
fn mechanism_b_coexisting_closures() {
    let dir = std::env::temp_dir();
    for (label, prog) in CASES {
        let tmp = dir.join(format!("loft_closure_cell_{label}.loft"));
        fa::write(&tmp, prog).unwrap();
        for mode in ["--interpret", "--native"] {
            let (ok, output) = run(mode, &tmp);
            assert!(
                ok,
                "Mechanism-B regression: case `{label}` failed on {mode}\n{output}"
            );
        }
    }
}
