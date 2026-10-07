// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @FR-F-Spec — a format width is a MINIMUM field size, so a width at or below zero pads
//! with nothing.
//!
//! The renderers reach a negative width by arithmetic, never by spelling: a sign or a
//! `0x` marker is emitted first and then subtracted from the field, and a spec that gives
//! no width at all starts that subtraction from zero.  `ops::format_text` counted its pad
//! characters into a `usize`, so `-1` became 18_446_744_073_709_551_615 of them —
//! `println("{-1:0}")` asked the allocator for the whole field in one call and the process
//! was OOM-killed.
//!
//! **This is the one property a value assertion cannot carry.** A cell that checks the
//! rendered string is the right guard for what the padding IS, and `tests/scripts/`
//! carries those; but if the clamp regresses, such a cell does not fail — it exhausts the
//! machine's memory and takes every other process on the box with it, which is how the
//! defect was found.  So the guard here is a BOUND: the same programs, run in a child
//! process under a hard address-space cap, must finish.  A regression fails this test in
//! milliseconds instead of triggering the kernel OOM killer.

use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Command;

/// A cap far above what these one-line programs need and far below the runaway.
const ADDRESS_SPACE_KIB: &str = "2000000"; // 2 GiB

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Run a one-line loft program under `ulimit -v`, returning its stdout.
///
/// The cap is applied by the shell that execs the compiler, so it covers the whole run
/// rather than a single allocation site.
fn run_capped(source: &str) -> String {
    let dir = std::env::temp_dir().join(format!("loft-fmt-bound-{}", std::process::id()));
    fa::create_dir_all(&dir).expect("scratch dir");
    // One file per source.  The tests here run as threads of a single process, so a
    // shared probe path lets one case overwrite the file another has not yet read.
    let mut key = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(source, &mut key);
    let file = dir.join(format!(
        "probe-{:016x}.loft",
        std::hash::Hasher::finish(&key)
    ));
    fa::write(&file, source).expect("write probe");
    let script = format!(
        "ulimit -v {ADDRESS_SPACE_KIB}; exec {} --interpret {}",
        loft_bin().display(),
        file.display()
    );
    let out = Command::new("bash")
        .arg("-c")
        .arg(&script)
        .env("LOFT_TIMEOUT", "60")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("failed to invoke loft under a memory cap");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "a bounded width must render, not exhaust memory.\n\
         source: {source}\nstatus: {:?}\nstdout: {stdout}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

/// Every spelling that makes a renderer subtract more than the width it was given.
///
/// Each pairs the program with the string it must produce, so the test states the padding
/// as well as bounding it — a cell that merely completed would pass on `-1` rendered as an
/// empty string.
/// ⚠ Windows has no `ulimit -v`, so this guard cannot be run there.
///
/// The bound IS the guard — the header above says so: a cell that checks the rendered string
/// is a different test, and this one exists to fail in milliseconds instead of OOM-killing the
/// box. Git Bash accepts `ulimit -v` and does not enforce it, so pointing this at a working
/// shell on Windows would make it pass while measuring nothing, which is worse than not running
/// it. `ignore` rather than `#[cfg]` so the Windows report SAYS it was skipped instead of
/// silently listing one test fewer.
///
/// (Before this, the Windows leg failed for an unrelated reason: a bare `bash` resolves to
/// `C:\Windows\System32\bash.exe`, the WSL launcher, which has no distribution installed —
/// so the test reported a memory-cap failure that was really a missing shell.)
// @PLN184 C2 approved exemption (owner, 2026-10-07): `ulimit -v` (an address-space cap on the child) has no Windows equivalent; Windows substitute: none
#[cfg_attr(windows, ignore = "no `ulimit -v` on Windows; the cap is the guard")]
#[test]
fn a_width_below_zero_pads_nothing() {
    for (source, want) in [
        // A sign subtracted from a zero width: the decimal arm of `format_long`.
        (r#"fn main() { println("{-1:0}"); }"#, "-1"),
        (
            r#"fn main() { println("{-9223372036854775807:0}"); }"#,
            "-9223372036854775807",
        ),
        // The `+` a spec asked for is a sign the same arithmetic subtracts.
        (r#"fn main() { println("{1:+0}"); }"#, "+1"),
        // The float twin, reached through `format_signed`.
        (r#"fn main() { println("{-2.7:.0}"); }"#, "-3"),
        (r#"fn main() { println("{-2.7f:.0}"); }"#, "-3"),
        // A radix marker is wider than the sign, so it goes further below zero.
        (r#"fn main() { println("{255:#06x}"); }"#, "0x00ff"),
    ] {
        let got = run_capped(source);
        assert_eq!(got.trim_end(), want, "for {source}");
    }
}

/// The control: the same clamp must not swallow a width that IS positive.
///
/// Without this row the test above passes on a renderer that ignores every width, which is
/// the cheapest wrong way to stop an over-long pad.
/// ⚠ Windows has no `ulimit -v`, so this guard cannot be run there.
///
/// The bound IS the guard — the header above says so: a cell that checks the rendered string
/// is a different test, and this one exists to fail in milliseconds instead of OOM-killing the
/// box. Git Bash accepts `ulimit -v` and does not enforce it, so pointing this at a working
/// shell on Windows would make it pass while measuring nothing, which is worse than not running
/// it. `ignore` rather than `#[cfg]` so the Windows report SAYS it was skipped instead of
/// silently listing one test fewer.
///
/// (Before this, the Windows leg failed for an unrelated reason: a bare `bash` resolves to
/// `C:\Windows\System32\bash.exe`, the WSL launcher, which has no distribution installed —
/// so the test reported a memory-cap failure that was really a missing shell.)
// @PLN184 C2 approved exemption (owner, 2026-10-07): `ulimit -v` (an address-space cap on the child) has no Windows equivalent; Windows substitute: none
#[cfg_attr(windows, ignore = "no `ulimit -v` on Windows; the cap is the guard")]
#[test]
fn a_positive_width_still_pads() {
    for (source, want) in [
        (r#"fn main() { println("{-1:04}"); }"#, "-001"),
        (r#"fn main() { println("{1:+04}"); }"#, "+001"),
        (r#"fn main() { println("{-3.5:08.2}"); }"#, "-0003.50"),
        (r#"fn main() { println("{42:>6}"); }"#, "    42"),
    ] {
        let got = run_capped(source);
        assert_eq!(got.trim_end(), want, "for {source}");
    }
}
