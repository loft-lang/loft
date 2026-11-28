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
//! The rules it guards beyond the ones each slot row cites: `@FR-E-Once` (an operand of any
//! form is reduced once), with `@FR-E-Field`, `@FR-E-Index`, `@FR-E-Is`, `@FR-E-Record`,
//! `@FR-I-While` and `@FR-E-Assert` for the forms no other rule covered.
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
//! What these tests run is the whole matrix on the interpreter and a SAMPLE of it on the C
//! backend: every slot with its plain variable and with the first filler of its type.
//! Compiling every cell costs about a tenth of a second each, and on a CI runner the whole
//! matrix on both backends ran past the per-test limit whether it was one test (421 s) or
//! seven beside one another (four of them at 600 s).  The whole matrix on the C backend is
//! `slot_matrix.py gate --full`, which is also what rewrites the baseline.
//!
//! @falsified-at: hand-measured — with `LOFT` naming the release build of 2026-11-03
//! (`v2026.11.0`) `gate --family iteration` exits 1 with a `BROKEN` block (the `for` source
//! run once per step, loft#2573) and a `FIXED` block; on the build this baseline was written
//! for every family exits 0.
//!   CHANNEL: the gate's exit code and its `BROKEN` / `FIXED` blocks.
//!   ARMED: `LOFT_LEAK_CHECK=1`, set by the script itself.
//!   WITNESS: `NEW it-for.coll-call.sorted [interp] (count): the operand ran 5 time(s)`.
//!   HOLDS: `slot_matrix.py check` and `selftest` pass on both builds.

/// One family's gate at a time.  Each gate runs its cells on a pool of half the cores, and
/// every cell compiles a C program; the test harness runs the seven family tests at once, so
/// without this the binary ran seven pools together — 70 compiles on a 20-core laptop,
/// 11 GB of memory and 19 GB of swap until the OOM killer stopped it.  Serialised, the
/// binary uses what one `slot_matrix.py gate` run uses.
static ONE_GATE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gate(family: &str) {
    run_gate(&["gate", "--family", family], &format!("the `{family}` slots"));
}

fn run_gate(args: &[&str], what: &str) {
    let _one = ONE_GATE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = match loft::platform::process::harness_command("python3")
        .arg(root.join("scripts/slot_matrix.py"))
        .args(args)
        .env("LOFT", env!("CARGO_BIN_EXE_loft"))
        .current_dir(root)
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            eprintln!("SKIP the slot matrix's gate of {what}: python3 unavailable ({e})");
            return;
        }
    };
    assert!(
        out.status.success(),
        "{what} of the slot matrix are off their baseline:\n{}{}",
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

/// The sequence axis (`scripts/slot_sequence.py`): a fixed seeded sample of sequences of
/// statements over one local, each answer computed by the generator's own model, run under
/// the leak checks, `LOFT_STRICT_STORES` and `LOFT_POISON`; every cell interpreted, every
/// sixth compiled.  Its baseline is `scripts/slot_sequence.baseline`.
///
/// @falsified-at: hand-measured — with `LOFT` naming origin/main @ ebff34f71 the sequence
/// cells of loft#2671, #2673, #2674, #2677, #2681 and #2684 fail
/// (`slot_matrix.py seq run --cells <their names>`); on the build with their fixes they pass.
#[test]
fn sequences_hold_their_baseline() {
    run_gate(&["seq", "gate"], "the sequences");
}

#[test]
fn every_family_of_the_register_has_its_gate() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let register = loft::file_access::read_to_string(root.join("scripts/syntax_slots.tsv"))
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
