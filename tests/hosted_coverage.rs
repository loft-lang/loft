// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@PLN206` coverage — `scripts/hosted_coverage.py`, the check that every library runs compiled:
//! the public functions a library's hosted C unit leaves interpreted, against a baseline that only
//! shrinks.  `tests/lib/hostedlib` leaves one (`upto`, an iterator): a baseline naming it passes,
//! one missing it goes red (coverage lost), one naming a function the unit carries goes red
//! (coverage won and not written down), and `--strict` goes red while anything stays
//! interpreted.
//!
//! @falsified-at: none — a SABOTAGE (2026-11-22): with `build-native` printing no `interpreted:`
//!   lines (`src/main.rs`), every library reads as compiled whole and the measured baseline no
//!   longer holds.  CHANNEL: the script's exit status.  WITNESS: "now compiled, still in the
//!   baseline (write it down: --update): upto".
use loft::file_access as fa;
use loft::platform::process::harness_command;

fn have_tools() -> bool {
    let ok = |p: &str| {
        harness_command(p)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    };
    ok("cc") && ok("python3")
}

#[test]
fn the_coverage_check_holds_its_baseline_both_ways() {
    if !have_tools() {
        eprintln!("skipped: no cc or python3 on PATH");
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let work = std::env::temp_dir().join(format!("loft-hosted-coverage-{}", std::process::id()));
    let _ = fa::remove_dir_all(&work);
    let pkg = work.join("hostedlib");
    fa::create_dir_all(pkg.join("src")).expect("package dir");
    for f in ["loft.toml", "src/hostedlib.loft"] {
        let text = fa::read_to_string(root.join("tests/lib/hostedlib").join(f)).expect("read");
        fa::write(pkg.join(f), text).expect("write");
    }
    let baseline = work.join("coverage.baseline");
    let check = |lines: &str, strict: bool| {
        fa::write(&baseline, lines).expect("write the baseline");
        let mut cmd = harness_command("python3");
        cmd.arg(root.join("scripts/hosted_coverage.py"))
            .arg("--loft")
            .arg(env!("CARGO_BIN_EXE_loft"))
            .arg("--baseline")
            .arg(&baseline);
        if strict {
            cmd.arg("--strict");
        }
        let out = cmd
            .arg(&pkg)
            .env("LOFT_NO_CACHE", "1")
            .output()
            .expect("run the check");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    };
    let both = "hostedlib\tupto\tan iterator\n";
    let (ok, out) = check(both, false);
    assert!(ok, "the measured baseline holds: {out}");
    assert!(out.contains("hostedlib: 1 interpreted"), "{out}");
    let (ok, out) = check("", false);
    assert!(!ok, "a function interpreted and not listed is red: {out}");
    assert!(
        out.contains("now interpreted, not in the baseline: upto"),
        "{out}"
    );
    let (ok, out) = check(&format!("{both}hostedlib\tbag_new\ta record\n"), false);
    assert!(!ok, "a listed function the unit carries is red: {out}");
    assert!(
        out.contains("now compiled, still in the baseline (write it down: --update): bag_new"),
        "{out}"
    );
    let (ok, out) = check(both, true);
    let _ = fa::remove_dir_all(&work);
    assert!(
        !ok,
        "--strict is red while anything stays interpreted: {out}"
    );
}
