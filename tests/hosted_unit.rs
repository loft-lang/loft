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

const SCRIPT: &str = "tests/lib/hostedlib_main.loft";

fn have_cc() -> bool {
    harness_command("cc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// A private copy of the package, so the artifact this run builds is its own.
fn private_lib(tag: &str) -> std::path::PathBuf {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let lib = std::env::temp_dir().join(format!("loft-hosted-unit-{tag}-{}", std::process::id()));
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
    let lib = private_lib("answers");
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

/// `@PLN206 P4d` — `LOFT_HOSTED_VERIFY` runs each adopted call compiled and interpreted and finds
/// the two worlds alike; with a write planted in the unit (`LOFT_HOSTED_PLANT`) it stops at the
/// call that made it, naming the function and the record.
///
/// @falsified-at: none — the receipt is the plant itself: without the verify the planted byte
///   surfaces three statements later as a wrong total; with it, at `fill`.
///   CHANNEL: the exit status and the message.  WITNESS: "`fill` leaves another world".
#[test]
fn the_hosted_verify_passes_a_sound_unit_and_names_a_planted_write() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let run_verify = |plant: bool| {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let lib = private_lib(if plant { "plant" } else { "verify" });
        let mut cmd = harness_command(env!("CARGO_BIN_EXE_loft"));
        cmd.arg("--interpret")
            .arg("--lib")
            .arg(&lib)
            .arg(root.join(SCRIPT))
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_LIB_BACKEND", "c")
            .env("LOFT_NO_RUSTC", "1")
            .env("LOFT_POISON_CLAIM", "1")
            .env("LOFT_STRICT_STORES", "1")
            .env("LOFT_HOSTED_VERIFY", "1");
        if plant {
            cmd.env("LOFT_HOSTED_PLANT", "1");
        }
        let out = cmd.output().expect("run loft");
        let _ = fa::remove_dir_all(&lib);
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let (ok, out, err) = run_verify(false);
    assert!(ok, "a sound unit verifies: {err}");
    assert_eq!(out, "alpha 27 54 20100 201 2 32\n");
    let (ok, _, err) = run_verify(true);
    assert!(!ok, "a planted write is caught");
    assert!(
        err.contains("`fill` leaves another world compiled than interpreted"),
        "the verify names the call that wrote it: {err}"
    );
}

/// `@PLN206 P5` — a hosted unit runs in any program, not only the one that built it: built by
/// `hostedlib_main.loft`, it is adopted by `hostedlib_shift_main.loft`, which loads
/// `tests/lib/hostedshift` first and so numbers `hostedlib`'s types otherwise.  The unit names
/// a type by the building program's number and translates it by key where it crosses into a
/// store (`loft_tp_out`, `loft_tp_in`); the verify compares every store's root type and the
/// answer's, so a crossing left untranslated stops the run.
///
/// @falsified-at: none — a SABOTAGE (2026-11-22): with `loft_tp_out` answering its argument
///   (`src/c/loft_store.h`), the shifted program stops at the first record answer.
///   CHANNEL: the exit status and the message.  WITNESS: "`bag_new` leaves another world
///   compiled than interpreted: the answer's store holds type 107 interpreted, 100 compiled"
///   (the numbers of that build).  HOLDS: the building program, whose numbers are the unit's.
#[test]
fn a_hosted_unit_built_by_one_program_runs_in_another() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let lib = private_lib("shift");
    let pkg = lib.join("hostedshift");
    fa::create_dir_all(pkg.join("src")).expect("package dir");
    for f in ["loft.toml", "src/hostedshift.loft"] {
        let text = fa::read_to_string(root.join("tests/lib/hostedshift").join(f)).expect("read");
        fa::write(pkg.join(f), text).expect("write");
    }
    let run_one = |script: &str| {
        let out = harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg("--interpret")
            .arg("--lib")
            .arg(&lib)
            .arg(root.join(script))
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_LIB_BACKEND", "c")
            .env("LOFT_NO_RUSTC", "1")
            .env("LOFT_POISON_CLAIM", "1")
            .env("LOFT_STRICT_STORES", "1")
            .env("LOFT_HOSTED_VERIFY", "1")
            .output()
            .expect("run loft");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let (ok, out, err) = run_one(SCRIPT);
    assert!(ok, "the building program: {err}");
    assert_eq!(out, "alpha 27 54 20100 201 2 32\n");
    let units = |lib: &std::path::Path| {
        fa::read_dir(lib.join("hostedlib/native-auto"))
            .map(|d| {
                d.iter()
                    .filter_map(|p| p.file_name())
                    .filter(|n| n.ends_with(".dylib") || n.ends_with(".so") || n.ends_with(".dll"))
                    .count()
            })
            .unwrap_or(0)
    };
    assert_eq!(units(&lib), 1, "the building program built one unit");
    let (ok, out, err) = run_one("tests/lib/hostedlib_shift_main.loft");
    let _ = fa::remove_dir_all(&lib);
    assert!(ok, "the shifted program adopts the unit and verifies: {err}");
    assert_eq!(out, "alpha 27 54 20100 201 2 32 7\n");
    assert!(
        err.contains("call(s) compiled and interpreted alike") && !err.contains(": 0 call(s)"),
        "calls crossed the unit: {err}"
    );
}

/// `@PLN206 P5` — the C runtime reads and writes a store's root type only through `LOFT_KNOWN`
/// and `LOFT_SET_KNOWN`, which a hosted unit translates by key (`src/c/loft_store.h`); a raw
/// `known` is marked `LOFT_RAW_KNOWN` with why it may see the store as it is in memory.  A raw
/// one unmarked is a crossing the translation never sees: in a program that numbers types
/// otherwise it reads or writes another type, and nothing says so.
#[test]
fn the_c_runtime_reaches_a_stores_type_only_through_its_translation() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/c");
    let mut raw = Vec::new();
    for p in fa::read_dir(&root).expect("src/c") {
        let Some(name) = p.file_name().map(str::to_string) else {
            continue;
        };
        if !name.ends_with(".h") {
            continue;
        }
        let text = fa::read_to_string(root.join(&name)).expect("read");
        for (i, line) in text.lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with("/*") || t.starts_with('*') || line.contains("LOFT_RAW_KNOWN") {
                continue;
            }
            if t.starts_with("#define LOFT_KNOWN") || t.starts_with("#define LOFT_SET_KNOWN") {
                continue;
            }
            let hit = line.match_indices("known").any(|(at, _)| {
                let before = &line[..at];
                let after = line[at + 5..].chars().next();
                (before.ends_with('.') || before.ends_with("->"))
                    && !after.is_some_and(|c| c.is_alphanumeric() || c == '_')
            });
            if hit {
                raw.push(format!("src/c/{name}:{}: {}", i + 1, line.trim()));
            }
        }
    }
    assert!(
        raw.is_empty(),
        "a store's type reached without LOFT_KNOWN / LOFT_SET_KNOWN (mark a deliberate one \
         LOFT_RAW_KNOWN):\n{}",
        raw.join("\n")
    );
}
