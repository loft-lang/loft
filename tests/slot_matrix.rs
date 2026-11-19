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
//! `scripts/slot_matrix.baseline`.
//!
//! One test per slot FAMILY, so that a red name says where to look and the families run
//! beside one another.  A test fails for one of two reasons, which its output names first:
//! `BROKEN` — a cell that is not ok and not in the baseline, which is a change that made an
//! operand run twice, reordered two operands, lost a write or leaked; and `FIXED` — a
//! baseline line whose cell passes again, which wants only `gate --update` beside the fix.
//!
//! No platform gate: the script runs wherever `python3` and a C compiler do, and reads only
//! what `loft --tests` prints.  Skipped (not failed) where `python3` is unavailable.
//!
//! Seven tests and not one for a second reason: on a CI runner the whole matrix as ONE test
//! took 421 s, past half the per-test limit (`scripts/test_duration_gate.py`).
//!
//! @falsified-at: hand-measured — with `LOFT` naming the release build of 2026-11-03
//! (`v2026.11.0`) `gate --family iteration` exits 1 with a `BROKEN` block (the `for` source
//! run once per step, loft#2573) and a `FIXED` block; on the build this baseline was written
//! for every family exits 0.
//!   CHANNEL: the gate's exit code and its `BROKEN` / `FIXED` blocks.
//!   ARMED: `LOFT_LEAK_CHECK=1`, set by the script itself.
//!   WITNESS: `NEW it-for.coll-call.sorted [interp] (count): the operand ran 5 time(s)`.
//!   HOLDS: `slot_matrix.py check` and `selftest` pass on both builds.

fn gate(family: &str) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = match loft::platform::process::harness_command("python3")
        .arg(root.join("scripts/slot_matrix.py"))
        .args(["gate", "--family", family])
        .env("LOFT", env!("CARGO_BIN_EXE_loft"))
        .current_dir(root)
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            eprintln!("SKIP the slot matrix's `{family}` gate: python3 unavailable ({e})");
            return;
        }
    };
    assert!(
        out.status.success(),
        "the `{family}` slots of the slot matrix are off their baseline:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// One test per family of `scripts/syntax_slots.tsv`.  A family added there and not here
/// would go unchecked, which `every_family_of_the_register_has_its_gate` refuses.
macro_rules! family_gates {
    ($($name:ident => $family:literal),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                gate($family);
            }
        )*
        const GATED: &[&str] = &[$($family),*];
    };
}

family_gates! {
    iteration_slots_hold_their_baseline => "iteration",
    write_slots_hold_their_baseline => "write",
    read_slots_hold_their_baseline => "read",
    call_slots_hold_their_baseline => "call",
    operator_slots_hold_their_baseline => "operator",
    control_slots_hold_their_baseline => "control",
    construct_slots_hold_their_baseline => "construct",
}

#[test]
fn every_family_of_the_register_has_its_gate() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let register = std::fs::read_to_string(root.join("scripts/syntax_slots.tsv"))
        .expect("scripts/syntax_slots.tsv");
    for line in register
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let family = line.split('\t').nth(1).unwrap_or("");
        assert!(
            GATED.contains(&family),
            "slot family `{family}` has no gate in tests/slot_matrix.rs: add it to `family_gates!`"
        );
    }
}
