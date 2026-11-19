// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! The slot matrix as a gate (@PLN221): every operand position of the syntax, with every
//! form of expression in it, on the interpreter and the C backend.
//!
//! `scripts/slot_matrix.py gate` generates the cells from `scripts/syntax_slots.tsv` and
//! `scripts/syntax_fillers.tsv`, runs them, and checks four things of each: how often the
//! operand ran, the order its operands ran in, what the statement left behind against the
//! same statement with the operand bound to a local first, and that no store is left behind
//! or freed twice.  The cells that do not pass today are listed in
//! `scripts/slot_matrix.baseline`.  This test fails on a cell that is not ok and not listed —
//! a change that made an operand run twice, reordered two operands, lost a write or leaked —
//! and on a listed cell that passes again, so that a fix removes its lines.
//!
//! Unix only: the cells run through `loft --tests` with the C backend, and the runner reads
//! that output's form.  Skipped (not failed) where `python3` is unavailable.
//!
//! @falsified-at: hand-measured — with `LOFT` naming the release build of 2026-11-03
//! (`v2026.11.0`) the gate exits 1 with 120 `NEW` cells (the `for` source run once per step,
//! loft#2573, among them) and 41 `PASSES` lines (the regressions since that release); on
//! the build this baseline was written for it exits 0.
//!   CHANNEL: the gate's exit code and its `NEW` / `PASSES` lines.
//!   ARMED: `LOFT_LEAK_CHECK=1`, set by the script itself.
//!   WITNESS: `NEW it-for.coll-call.sorted [interp] (count): the operand ran 5 time(s)`.
//!   HOLDS: `slot_matrix.py check` and `selftest` pass on both builds.

#[cfg(unix)]
#[test]
fn no_cell_of_the_slot_matrix_fails_outside_its_baseline() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = match loft::platform::process::harness_command("python3")
        .arg(root.join("scripts/slot_matrix.py"))
        .arg("gate")
        .env("LOFT", env!("CARGO_BIN_EXE_loft"))
        .current_dir(root)
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            eprintln!(
                "SKIP no_cell_of_the_slot_matrix_fails_outside_its_baseline: python3 unavailable ({e})"
            );
            return;
        }
    };
    assert!(
        out.status.success(),
        "the slot matrix is off its baseline:\n{}{}\n\
         A `NEW` cell is a statement that stopped evaluating an operand once, in order, to the \
         right result, without a leak: fix the lowering.  A `PASSES` line is a fixed cell: \
         remove it from scripts/slot_matrix.baseline (`slot_matrix.py gate --update`, then \
         read the diff).  `python3 scripts/slot_matrix.py run --filler all --slot <id> -v` \
         shows one slot.",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}
