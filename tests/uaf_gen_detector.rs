// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `LOFT_UAF_GEN` (detector c) — calibration guard.
//!
//! The detector stamps each DbRef pushed onto the operand stack with its store's
//! generation and, at the matching pop, reports a stamp older than the store's current
//! gen: the slot was freed while that ref was live.
//!
//! Its stamps used to go stale. `put_stack` is the only writer that keeps the shadow in
//! step with the eval stack, and it is not the only writer OF the eval stack — the
//! `copy_result` return slide moves bytes with a raw `copy_block`. The destination offset
//! therefore kept whatever stamp its previous occupant had left, and the next pop compared
//! a returned DbRef against a generation belonging to some earlier, unrelated value. Any
//! loop calling a struct-returning function reported a use-after-free that was not there:
//! 25 of the corpus scripts, all of them clean under `LOFT_NO_SLOT_REUSE=1` +
//! `LOFT_POISON=1` (no slot reuse means a genuine stale read must hit poisoned bytes, and
//! none did).
//!
//! Both halves are pinned here, because either alone is worthless:
//!
//!   - **Silent on clean code.** The two scripts below are the shapes that used to report
//!     — a `??` element bind in a loop, and a nested iterate whose stamp was compared
//!     against a DIFFERENT store entirely.
//!   - **Not silent because it is dead.** `LOFT_UAF_GEN_INJECT=1` ages every ref just after
//!     it is stamped, so each one is stale while live exactly as a premature free would
//!     leave it. The detector must then report. Without this direction a passing
//!     no-reports test would be satisfied by a detector that can no longer fire at all —
//!     which is the failure mode a false-positive fix invites.

use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn script(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts")
        .join(name)
}

/// Shapes that reported before the fix. Both are ordinary passing programs.
const PROBES: [&str; 2] = [
    "723-ncc-loop-element-bind.loft",
    "pln105-expose-iterate.loft",
];

/// Count `[uaf-gen]` reports from an `--interpret` run of `name`.
fn reports(name: &str, inject: bool) -> (usize, String) {
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.arg("--interpret")
        .arg(script(name))
        .env("LOFT_TIMEOUT", "180")
        .env("LOFT_UAF_GEN", "1");
    if inject {
        cmd.env("LOFT_UAF_GEN_INJECT", "1");
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    (stderr.matches("[uaf-gen]").count(), stderr)
}

#[test]
fn uaf_gen_is_silent_on_clean_programs() {
    for name in PROBES {
        let (n, stderr) = reports(name, false);
        assert_eq!(
            n, 0,
            "{name}: LOFT_UAF_GEN must not report on a clean program — a stamp left by an \
             earlier occupant of an eval-stack offset is not evidence about the ref popped \
             there now\nstderr:\n{stderr}"
        );
    }
}

#[test]
fn uaf_gen_still_fires_when_a_ref_goes_stale() {
    for name in PROBES {
        let (n, _) = reports(name, true);
        assert!(
            n > 0,
            "{name}: LOFT_UAF_GEN reported nothing under LOFT_UAF_GEN_INJECT=1, so its \
             silence on clean programs proves nothing — the stamp/compare path is dead"
        );
    }
}
