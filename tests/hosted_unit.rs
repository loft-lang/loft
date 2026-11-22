// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@PLN206 P4` — a library adopted as a hosted C unit works on the interpreter's own stores
//! (`doc/claude/plans/206-one-store-model/p3-p4-design.md`).  `tests/lib/hostedlib` answers a
//! record through the caller's buffer, grows a vector field in the caller's store two hundred
//! times, walks it, reads a text: the program that uses it must answer as it does interpreted,
//! under `LOFT_POISON`, with every call dispatched into the C (`LOFT_REQUIRE_NATIVE`), and the
//! profiler counting those calls, which proves they ran there.
//!
//! @falsified-at: none — the route is new, so the receipt is a SABOTAGE (2026-11-22): with
//!   `loft_store_make`'s `loft_host_sync()` struck (`src/c/loft_rt.h`), the unit keeps the table it
//!   read before the interpreter added the store; `deep(400)` makes it move.
//!   CHANNEL: the exit status.  WITNESS: "hosted: error: write to a locked store".
//!   HOLDS: the interpreted run.  Without `deep` the table never moved and the sabotage passed:
//!   a stale length only skipped frees, which `LOFT_STRICT_STORES` now fails too.
use loft::file_access as fa;
use loft::platform::process::harness_command;

const SCRIPT: &str = "tests/scripts/a-hosted-c-library-works-on-the-interpreters-stores.loft";

fn have_cc() -> bool {
    harness_command("cc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// A private copy of the package, so the artifact this run builds is its own.
fn private_lib() -> std::path::PathBuf {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let lib = std::env::temp_dir().join(format!("loft-hosted-unit-{}", std::process::id()));
    let _ = fa::remove_dir_all(&lib);
    let pkg = lib.join("hostedlib");
    fa::create_dir_all(pkg.join("src")).expect("package dir");
    for f in ["loft.toml", "src/hostedlib.loft"] {
        let text = fa::read_to_string(root.join("tests/lib/hostedlib").join(f)).expect("read");
        fa::write(pkg.join(f), text).expect("write");
    }
    lib
}

fn run(lib: &std::path::Path, hosted: bool) -> (bool, String, String) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut cmd = harness_command(env!("CARGO_BIN_EXE_loft"));
    cmd.arg("--interpret")
        .arg("--lib")
        .arg(lib)
        .arg(root.join(SCRIPT))
        .env("LOFT_NO_CACHE", "1");
    if hosted {
        cmd.env("LOFT_LIB_BACKEND", "c")
            .env("LOFT_NO_RUSTC", "1")
            .env("LOFT_REQUIRE_NATIVE", "1")
            .env("LOFT_POISON", "1")
            .env("LOFT_STRICT_STORES", "1")
            .env("LOFT_PROFILE", "1");
    } else {
        cmd.env("LOFT_NO_NATIVE_LIBS", "1");
    }
    let out = cmd.output().expect("run loft");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn a_hosted_c_library_works_on_the_interpreters_stores() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let lib = private_lib();
    let (ok, want, err) = run(&lib, false);
    assert!(ok, "interpreted: {err}");
    assert_eq!(want, "alpha 27 54 20100 201 2 32\n");
    let (ok, got, err) = run(&lib, true);
    assert!(ok, "hosted: {err}");
    assert_eq!(got, want, "the hosted unit answers as the interpreter does");
    // The positive control: the calls ran in the C (the profiler counts each one that left the
    // interpreter), and what was built is a C unit.
    let calls: u32 = err
        .split("LIBRARIES (")
        .nth(1)
        .and_then(|r| r.split(' ').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    assert!(calls >= 8, "the library's calls ran in the unit: {err}");
    let built = fa::read_dir(lib.join("hostedlib/native-auto"))
        .expect("native-auto")
        .iter()
        .any(|e| fa::has_extension(e.os_spelling(), "c"));
    assert!(built, "a hosted C unit was built");
    let _ = fa::remove_dir_all(&lib);
}
