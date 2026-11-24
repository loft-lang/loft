// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@PLN225 R1` — the standard library's compiled functions run as a hosted C unit, built once
//! per key in the user cache and adopted by every interpreted program, as a `use`d library's
//! unit is (`src/compiled_stdlib.rs`).  No rustc takes part.
//!
//! @falsified-at: none — the route is new, so the receipt is a SABOTAGE (2026-11-23): with
//!   `compiled_stdlib::mark` answering `None` (the unit built, its path never handed to the
//!   loader), every compiled function interprets.  CHANNEL: stderr under `LOFT_TIMING`.
//!   WITNESS: "compiled stdlib: 0 function(s) dispatched".  HOLDS: the answers, which agree either way.
use loft::file_access as fa;
use loft::platform::process::harness_command;

const PROGRAM: &str = r#"
fn main() {
    line = "alpha beta,gamma delta,epsilon zeta";
    parts = line.split(',');
    words = "one two  three".split(' ');
    println("{len(parts)} {parts[2]} {len(words)} {parts.join("|")}");
    println("{"/usr/local/lib/file.txt".basename()} {"/usr/local/lib/file.txt".dir()}");
    println("{"abc123".is_alphanumeric()} {"abc".is_lowercase()} {"ABC".is_uppercase()}");
    println("{"12345".is_numeric()} {"  \t".is_whitespace()} {"héllo".char_slice(1, 3)}");
    n = 0;
    for i in 0..2000 { if "w{i}".is_alphanumeric() { n += 1; } }
    println("{n}");
}
"#;

const EXPECTED: &str = "3 epsilon zeta 4 alpha beta|gamma delta|epsilon zeta\n\
file.txt /usr/local/lib\n\
true true true\n\
true true él\n\
2000\n";

fn have_cc() -> bool {
    harness_command("cc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn run(dir: &std::path::Path, cache: &std::path::Path, envs: &[(&str, &str)]) -> (String, String) {
    let file = dir.join("stdlib_unit.loft");
    fa::write(&file, PROGRAM).expect("write the program");
    let mut cmd = harness_command(env!("CARGO_BIN_EXE_loft"));
    cmd.arg("--interpret")
        .arg(&file)
        .env("XDG_CACHE_HOME", cache)
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_NO_RUSTC", "1")
        .env("LOFT_TIMING", "1");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("loft runs");
    assert!(
        out.status.success(),
        "the program fails: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("loft-stdlib-unit-{tag}-{}", std::process::id()));
    let _ = fa::remove_dir_all(&d);
    fa::create_dir_all(&d).expect("scratch dir");
    d
}

/// The first interpreted run builds the unit, a later one adopts it unchanged, and both answer
/// as the interpreted standard library does — compiled and interpreted calls compared one by
/// one under `LOFT_HOSTED_VERIFY`.
#[test]
fn the_standard_library_runs_as_its_c_unit_and_answers_as_interpreted() {
    if !have_cc() {
        eprintln!("skipped: no C compiler");
        return;
    }
    let dir = scratch("adopt");
    let cache = dir.join("cache");
    let (out, err) = run(&dir, &cache, &[("LOFT_HOSTED_VERIFY", "1")]);
    assert_eq!(out, EXPECTED, "{err}");
    assert!(
        err.contains("compiled stdlib: 16 function(s) dispatched"),
        "{err}"
    );
    assert!(err.contains("compiled and interpreted alike"), "{err}");
    assert!(!err.contains("differ"), "{err}");
    let units = cache.join("loft/stdlib-units");
    let names: Vec<std::path::PathBuf> = fa::read_dir(&units)
        .expect("the unit's directory")
        .into_iter()
        .map(|e| e.os_spelling())
        .filter(|p| fa::has_extension(p, "names"))
        .collect();
    assert_eq!(names.len(), 1, "one unit for one key: {names:?}");
    let carried = fa::read_to_string(&names[0]).expect("the names file");
    for f in [
        "t_4text_split",
        "t_4text_basename",
        "t_4text_is_alphanumeric",
        "t_6vector_join",
    ] {
        assert!(carried.lines().any(|l| l == f), "{f} is carried: {carried}");
    }
    let built = fa::read_dir(&units)
        .expect("the unit's directory")
        .into_iter()
        .map(|e| e.os_spelling())
        .find(|p| {
            ["so", "dylib", "dll"]
                .iter()
                .any(|x| fa::has_extension(p, x))
        })
        .expect("the shared object");
    let before = fa::metadata(&built)
        .and_then(|m| m.modified())
        .expect("its time");
    let (again, err) = run(&dir, &cache, &[]);
    assert_eq!(again, EXPECTED, "{err}");
    assert!(
        err.contains("compiled stdlib: 16 function(s) dispatched"),
        "{err}"
    );
    let after = fa::metadata(&built)
        .and_then(|m| m.modified())
        .expect("its time");
    assert_eq!(
        before, after,
        "the second run adopts the unit, it does not build it again"
    );
    let (interpreted, err) = run(&dir, &cache, &[("LOFT_NO_COMPILED_STDLIB", "1")]);
    assert_eq!(interpreted, EXPECTED, "{err}");
    assert!(
        err.contains("compiled stdlib: 0 function(s) dispatched"),
        "{err}"
    );
    let _ = fa::remove_dir_all(&dir);
}
