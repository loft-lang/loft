// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `--dev-soft-halt` surfaces integer overflow, on both backends (loft#1265).
//!
//! `formal/operational.md` (E-Report) states the flag's contract in as many words — it
//! "still surfaces these recoverable faults (uniformly: div0, overflow, OOB)" — and
//! overflow was the one it missed.  It is also the one with nothing else to fall back on:
//! div0 writes a Warn log at an undefended site and an overrun has its own, while overflow
//! is silent everywhere by design (the null IS the signal), so this flag was the whole of
//! its observability.
//!
//! The fix reports from `checked_long!`'s `None` arm, which already existed to build the
//! sentinel, so no non-overflowing operation gained a test.  Both backends call the same
//! `ops::` functions (`#rust"ops::op_add_int(@v1, @v2)"`), which is why one site serves
//! both — and why both legs are checked here rather than assumed from one.
//!
//! Four properties, because each regresses on its own:
//!   1. an undefended overflow is SURFACED, and names its operands;
//!   2. a DEFENDED one (`?? d`) stays silent — (E-Report) gives the guard the null, and
//!      the div0 half of the same rule is already silent;
//!   3. the run exits NON-ZERO, the way its div0 and OOB peers do — a triage flag whose
//!      only fault exits 0 reports the run as clean;
//!   4. with the flag OFF nothing changes at all — (E-Uncomp) is mode-independent, so the
//!      value is still null, silently, and the run still exits 0.
//!
//! Property 4 and the `2 + 3` row are the controls.  Without them "every row reports" is
//! equally satisfied by a reporter that fires on all arithmetic, and "the flag surfaces
//! it" by one that surfaces it always.
//!
//! Falsified at 2c641133 (a detached worktree): `an_undefended_overflow_...` fails there
//! with 0 surfaced overflows against 3, while the div0 peer still reports — so the row is
//! measuring the overflow channel and not the flag as a whole.  The flag-OFF test passes
//! on BOTH trees, which is what makes it a control rather than a second copy of the claim.
//! The control tree has no `target/release`, so `have_rustc()` skips its `--native` leg
//! there; the fixed tree runs both, and the fix is one shared `ops::` function either way.

use loft::file_access as fa;
use std::path::PathBuf;

/// Run `src` on `backend`, with the soft-halt flag on or off.
fn run(backend: &str, src: &str, tag: &str, soft_halt: bool) -> (i32, String, String) {
    let dir = std::env::temp_dir().join(format!("loft_1265_{}_{tag}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("p.loft");
    fa::write(&path, src).expect("write");
    let mut cmd = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"));
    cmd.args([backend, path.to_str().unwrap()])
        .env("LOFT_TIMEOUT", "120")
        .current_dir(env!("CARGO_MANIFEST_DIR"));
    if soft_halt {
        cmd.env("LOFT_DEV_SOFT_HALT", "1");
    } else {
        cmd.env_remove("LOFT_DEV_SOFT_HALT");
    }
    let out = cmd.output().expect("spawn loft");
    let _ = fa::remove_dir_all(&dir);
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The faults are written as STATEMENTS, not inside the interpolation.  A format hole is a
/// guarded context — it emits the `*Nullable` ops so it can render `null(/0)` — so every
/// fault written inside one is silent by (E-Report), div0 included.  A probe that spelled
/// them `{hi + 1}` would report nothing and look like the pre-fix build.
const SRC: &str = "\
fn main() {
  hi = 9223372036854775807;
  lo = -9223372036854775807;
  z = 0;
  a = hi + 1;         println(\"A={a}\");
  b = lo - 2;         println(\"B={b}\");
  c = hi * 2;         println(\"C={c}\");
  d = (hi + 1) ?? 42; println(\"D={d}\");
  e = 7 / z;          println(\"E={e}\");
  g = 2 + 3;          println(\"G={g}\");
}
";

/// `rustc` is needed for the `--native` leg; skip cleanly where it is absent, like the
/// other native suites.
fn have_rustc() -> bool {
    loft::platform::process::harness_command("rustc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
        && fa::exists(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/release"))
}

fn backends() -> Vec<&'static str> {
    let mut v = vec!["--interpret"];
    if have_rustc() {
        v.push("--native");
    } else {
        println!("dev_soft_halt_surfaces_overflow: --native leg skipped (no rustc)");
    }
    v
}

#[test]
fn an_undefended_overflow_is_surfaced_and_ends_the_run_nonzero() {
    for backend in backends() {
        let (code, stdout, stderr) = run(backend, SRC, &format!("on{}", &backend[2..]), true);
        let overflows: Vec<&str> = stderr
            .lines()
            .filter(|l| l.contains("soft-halt: integer overflow"))
            .collect();
        assert_eq!(
            overflows.len(),
            3,
            "[{backend}] the three undefended overflows must each be surfaced, and the \
             defended one must not; got {overflows:?}\nstderr: {stderr}"
        );
        assert!(
            stderr.contains("soft-halt: integer overflow: 9223372036854775807 + 1"),
            "[{backend}] the report names the operation and its operands:\n{stderr}"
        );
        // The guard owns the null: `(hi + 1) ?? 42` is silent and answers its default.
        assert!(
            stdout.contains("D=42"),
            "[{backend}] the defended overflow still discharges to its default:\n{stdout}"
        );
        // Its peer is still reported, and under its own name.
        assert!(
            stderr.contains("soft-halt: divide by zero"),
            "[{backend}] the div0 peer must still report — an overflow report that had \
             replaced it would satisfy the rows above:\n{stderr}"
        );
        // The control: ordinary arithmetic is untouched and reports nothing.
        assert!(
            stdout.contains("G=5"),
            "[{backend}] arithmetic that does not overflow is unchanged:\n{stdout}"
        );
        assert_eq!(
            code, 1,
            "[{backend}] a soft-halt run that surfaced a fault exits non-zero:\n{stderr}"
        );
    }
}

/// With the flag off the program is byte-for-byte what it always was: overflow is null,
/// silently, and the run succeeds.  (E-Uncomp) is mode-independent — the report is a
/// debugging tool layered on top, never a change to what the program computes.
#[test]
fn with_the_flag_off_nothing_is_reported_and_the_run_succeeds() {
    for backend in backends() {
        let (code, stdout, stderr) = run(backend, SRC, &format!("off{}", &backend[2..]), false);
        assert!(
            !stderr.contains("soft-halt"),
            "[{backend}] nothing may be surfaced without the flag:\n{stderr}"
        );
        assert!(
            stdout.contains("A=null") && stdout.contains("B=null") && stdout.contains("C=null"),
            "[{backend}] overflow still yields the null sentinel:\n{stdout}"
        );
        assert!(
            stdout.contains("D=42") && stdout.contains("G=5"),
            "[{backend}] the defended and the ordinary rows are unchanged:\n{stdout}"
        );
        assert_eq!(
            code, 0,
            "[{backend}] a recoverable fault does not fail a run on its own:\n{stderr}"
        );
    }
}
