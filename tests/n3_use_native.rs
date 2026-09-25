// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN11 Arc N / N3 Phase A — `use <lib>` auto-compiles a normal loft library
//! to a native cdylib and dispatches to it, on the **real binary**.
//!
//! The fixture `tests/lib/mathnative/` is a NORMAL loft library (no `#native`, no
//! Rust crate) whose `loft.toml` opts in with `[library] compile = "native"`.  A
//! script that does `use mathnative;` and calls its functions must run them
//! natively (the library compiles) while the script interprets — output identical
//! to the all-interpreted run.  This is the headline of Arc N realised end-to-end.

use std::process::Command;

/// Copy `pkgs` out of `tests/lib` into a `lib/` of this test's own, and answer the
/// path to pass as `--lib`.
///
/// A test that WIPES and COUNTS `native-auto/` must own the directory it counts.
/// Two tests here did neither: `use_compile_native_library_dispatches_on_real_binary`
/// and `a_foreign_context_artifact_is_rejected_not_adopted` both wiped and then
/// counted `tests/lib/mathnative/native-auto`, so whichever ran second saw the
/// other's artifact — or had its own wiped mid-run — and the count assert failed
/// in 5 runs out of 8.
///
/// An in-process `Mutex` would not fix it: the suite runs under **nextest**, which
/// gives every test its own PROCESS. Only isolation works, and it is cheap —
/// `tests/lib` is 18 MB without the build directories and 9.4 GB with them, so
/// copying is fast precisely because `native-auto/` is what gets skipped.
///
/// Skipping `native-auto/` is also what makes the copy CORRECT rather than merely
/// small: an inherited artifact is exactly the thing these tests are counting.
fn private_lib(dir: &std::path::Path, pkgs: &[&str]) -> std::path::PathBuf {
    let lib = dir.join("lib");
    for pkg in pkgs {
        copy_pkg(&std::path::Path::new("tests/lib").join(pkg), &lib.join(pkg));
    }
    lib
}

/// Recursive copy that skips build output (`native-auto/`, `target/`).
fn copy_pkg(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).expect("create the private package dir");
    let Ok(entries) = std::fs::read_dir(from) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        if name == "native-auto" || name == "target" {
            continue;
        }
        let src = e.path();
        let dst = to.join(&name);
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            copy_pkg(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).expect("copy a fixture file");
        }
    }
}

/// Is an auto-built cdylib for `mathnative` present in `dir`?
///
/// loft#715 — the artifact name carries the caller's type-layout fingerprint
/// (`libloft_auto_mathnative_<fp>.so`), so two contexts can never name the same
/// file and a process cannot open a library built for someone else's type
/// indices. The test therefore matches the prefix + extension rather than a
/// fixed name; the fingerprint is not knowable from here.
fn cdylib_present(dir: &std::path::Path) -> bool {
    let (prefix, ext) = if cfg!(target_os = "windows") {
        ("loft_auto_mathnative", "dll")
    } else if cfg!(target_os = "macos") {
        ("libloft_auto_mathnative", "dylib")
    } else {
        ("libloft_auto_mathnative", "so")
    };
    std::fs::read_dir(dir).is_ok_and(|rd| {
        rd.flatten().any(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            n.starts_with(prefix) && n.ends_with(ext)
        })
    })
}

// @speed 1.1
#[test]
fn use_compile_native_library_dispatches_on_real_binary() {
    // The binary auto-builds the cdylib via rustc; skip where it isn't available.
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable");
        return;
    }

    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_n3_use_{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let prog = tmp.join("main.loft");
    // A plain script: `use` the library and call its functions — NO `#native`, no
    // execution-mode declaration.  double/add/factorial are normal loft functions.
    std::fs::write(
        &prog,
        "use mathnative;\n\
         fn main() {\n\
         \x20   println(\"{double(21)}\");\n\
         \x20   println(\"{add(3, 4)}\");\n\
         \x20   println(\"{factorial(5)}\");\n\
         }\n",
    )
    .unwrap();

    // The library's auto-built cdylib lands in its package's `native-auto/` dir;
    // its presence afterwards proves the build ran.  A PRIVATE copy of the package,
    // because that directory is asserted on and a sibling test wipes the shared one
    // (see `private_lib`).
    let lib = private_lib(&tmp, &["mathnative"]);
    let native_auto = lib.join("mathnative/native-auto");
    let native_auto = native_auto.as_path();

    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--lib")
        .arg(&lib)
        .arg(&prog)
        .env("LOFT_NO_CACHE", "1") // auto-native programs bypass the program cache anyway
        .output()
        .expect("run the loft binary");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "loft exited non-zero.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // 21*2=42, 3+4=7, 5!=120 — identical to the all-interpreted result.  Because
    // `def.native` is set, the calls compile to `OpStaticCall`; a correct answer
    // means the bridge was wired (an unwired stub would have panicked instead),
    // i.e. the calls dispatched into the auto-built native cdylib.
    assert_eq!(
        stdout, "42\n7\n120\n",
        "auto-native dispatch produced the wrong output"
    );

    // The cdylib was actually built (the native path was taken, not interpreted).
    assert!(
        cdylib_present(native_auto),
        "expected an auto-built cdylib in {}",
        native_auto.display()
    );

    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(native_auto);
}

/// @PLN11 Arc N / N3 (B3) — silent per-function fallback: a library where one
/// public function is shared-store-dispatchable (`triple`) and another is not
/// (`apply_inc` calls through a function reference — a `CallRef` the gate
/// conservatively excludes).  The gate splits the library silently — `triple`
/// compiles into the cdylib + dispatches native; `apply_inc` stays interpreted —
/// with no user-facing error and the script calling both alike.
///
/// Also verifies the synthetic-exclusion invariant: `apply_inc`'s nested lambda
/// (`__lambda_N`, made `pub_visible` by the enclosing `pub fn`) is NOT a dispatch
/// target — it is a fn-ref target, not script-callable public API — so no
/// `loft_shared_n___lambda` symbol appears in the cdylib.
// @speed 1.2
#[test]
fn mixed_library_dispatches_native_and_interprets_rest() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable");
        return;
    }

    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_n3_mixed_{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let prog = tmp.join("main.loft");
    std::fs::write(
        &prog,
        "use mathmixed;\n\
         fn main() {\n\
         \x20   println(\"{triple(7)}\");\n\
         \x20   println(\"{apply_inc(10)}\");\n\
         }\n",
    )
    .unwrap();

    let native_auto = std::path::Path::new("tests/lib/mathmixed/native-auto");
    let _ = std::fs::remove_dir_all(native_auto);

    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--lib")
        .arg("tests/lib")
        .arg(&prog)
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("run the loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "loft exited non-zero.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // triple(7)=21 (native), apply_inc(10)=11 (interpreted) — identical to all-interp.
    assert_eq!(stdout, "21\n11\n", "mixed-library output");

    // The gate split the library: the cdylib exports the dispatchable `triple` but
    // NOT `apply_inc` (CallRef → interpreted), and NOT its synthetic lambda.
    // loft#715 — the generated source is named for the caller's type-layout
    // fingerprint, like the cdylib beside it, so find it by prefix.
    let rs_path = std::fs::read_dir(native_auto)
        .expect("native-auto dir should exist")
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("loft_auto_mathmixed") && n.ends_with(".rs"))
        })
        .expect("generated cdylib source should exist");
    let lib_rs = std::fs::read_to_string(&rs_path).expect("read generated cdylib source");
    assert!(
        lib_rs.contains("loft_shared_n_triple"),
        "triple should have a native bridge"
    );
    assert!(
        !lib_rs.contains("loft_shared_n_apply_inc"),
        "apply_inc (CallRef) must NOT be dispatched native — it stays interpreted"
    );
    assert!(
        !lib_rs.contains("__lambda"),
        "a synthetic lambda must NOT be a native dispatch target"
    );

    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(native_auto);
}

/// A native build failure with a `rustc` toolchain present is a HARD ERROR — loft
/// refuses to silently degrade to the interpreter — and `LOFT_REQUIRE_NATIVE` names
/// itself as the reason.  This guards the **library** chokepoint:
/// `LOFT_FORCE_NATIVE_BUILD_FAIL` deterministically drives the auto-native build to
/// `Err` (a `rustc` toolchain IS present on the host), so both arms hard-fail from
/// the SAME forced failure, with DIFFERENT reasons on stderr:
///  * default (no env)  → exit ≠ 0, no output, "a real build failure" (rustc present);
///  * `LOFT_REQUIRE_NATIVE=1` → exit ≠ 0, no output, the env var named as the reason.
///
/// The graceful no-toolchain fallback (the only remaining silent-interpret path) is
/// covered by `require_native_errors_when_rustc_is_absent` below.
#[test]
fn native_build_failure_hard_fails_default_and_under_require() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_n3_require_{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let prog = tmp.join("main.loft");
    std::fs::write(
        &prog,
        "use mathnative;\n\
         fn main() {\n\
         \x20   println(\"{double(21)}\");\n\
         }\n",
    )
    .unwrap();

    let run = |require: bool| {
        let mut c = Command::new(env!("CARGO_BIN_EXE_loft"));
        c.arg("--lib")
            .arg("tests/lib")
            .arg(&prog)
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_FORCE_NATIVE_BUILD_FAIL", "1");
        if require {
            c.env("LOFT_REQUIRE_NATIVE", "1");
        }
        c.output().expect("run the loft binary")
    };

    // Default (no env): with rustc present, the forced build failure is a REAL
    // failure — loft refuses to silently interpret it, exits non-zero, and runs no
    // program output.  The reason names the present toolchain, NOT LOFT_REQUIRE_NATIVE.
    let def = run(false);
    let def_stdout = String::from_utf8_lossy(&def.stdout);
    let def_stderr = String::from_utf8_lossy(&def.stderr);
    assert!(
        !def.status.success(),
        "a native build failure with rustc present must hard-fail by default.\nstdout:\n{def_stdout}\nstderr:\n{def_stderr}"
    );
    assert!(
        !def_stdout.contains("42"),
        "a hard-failed build must not run the interpreted fallback (saw program output)"
    );
    assert!(
        def_stderr.contains("real build failure") && !def_stderr.contains("LOFT_REQUIRE_NATIVE"),
        "the default hard-fail names the present toolchain, not LOFT_REQUIRE_NATIVE.\nstderr:\n{def_stderr}"
    );

    // Strict: the same forced failure is now a hard error — no program output, a
    // non-zero exit, and the reason (the env var + the failing library) on stderr.
    let strict = run(true);
    let strict_stdout = String::from_utf8_lossy(&strict.stdout);
    let strict_stderr = String::from_utf8_lossy(&strict.stderr);
    assert!(
        !strict.status.success(),
        "LOFT_REQUIRE_NATIVE must turn the fallback into a non-zero exit.\nstdout:\n{strict_stdout}\nstderr:\n{strict_stderr}"
    );
    assert!(
        !strict_stdout.contains("42"),
        "strict mode must refuse to run the interpreted fallback (saw program output)"
    );
    assert!(
        strict_stderr.contains("LOFT_REQUIRE_NATIVE")
            && strict_stderr.contains("failed to build native"),
        "strict error must name the env var and the reason.\nstderr:\n{strict_stderr}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Guards the **main-program** chokepoint of `LOFT_REQUIRE_NATIVE`.  Forces a native
/// fallback by hiding `rustc` (empty `PATH`) on a cache-bypassed `--native` run; under
/// the env var that must be a hard error naming the missing toolchain, not a silent
/// degrade to the interpreter.  Skipped on Windows where the `PATH` model + binary
/// resolution differ enough that an empty `PATH` is not a clean way to hide `rustc`.
#[test]
#[cfg(not(target_os = "windows"))]
fn require_native_errors_when_rustc_is_absent() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_n3_norustc_{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let prog = tmp.join("main.loft");
    std::fs::write(&prog, "fn main() {\n    print(\"ran\")\n}\n").unwrap();

    // Empty PATH ⇒ `rustc` (invoked by bare name) is NotFound; loft itself runs
    // because it is launched by absolute path.  Cache bypass forces a compile attempt.
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--native")
        .arg(&prog)
        .env("PATH", "")
        .env("LOFT_NATIVE_NO_CACHE", "1")
        .env("LOFT_REQUIRE_NATIVE", "1")
        .output()
        .expect("run the loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "LOFT_REQUIRE_NATIVE must error when rustc is absent.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        !stdout.contains("ran"),
        "strict mode must not fall through to the interpreter (saw program output)"
    );
    assert!(
        stderr.contains("LOFT_REQUIRE_NATIVE"),
        "strict error must name the env var.\nstderr:\n{stderr}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// #460 — the package that OWNS the entry file is the *script*, not a `use`d
/// library: it must never be auto-native-compiled, even though it carries a
/// `loft.toml`.  Its export set is entry-point dependent — running `entry_a.loft`
/// parses only `mod_a` (`val_a`), running `entry_b.loft` parses only `mod_b`
/// (`val_b`) — so a cdylib built for one entry exports the wrong symbol set for
/// the other.  Before the fix, the second run found the first run's cdylib
/// "fresh", skipped the rebuild, then marked its own export set against it →
/// `OpStaticCall` to a bridge symbol the `.so` never built → the `compile.rs`
/// panic stub (crawler's `make test` gate, exit 101).
///
/// The invariant: the entry package produces NO `native-auto/` cdylib at all
/// (the "libraries compile, scripts interpret" model), so running two different
/// entry points from the same package in sequence both succeed cleanly.
#[test]
fn entry_package_is_never_auto_native_compiled() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable");
        return;
    }

    // The fixture `tests/lib/selfpkg/` is a normal package (loft.toml, no
    // [native]) run DIRECTLY via two entries that `use` disjoint local modules.
    let pkg = std::path::Path::new("tests/lib/selfpkg");
    let native_auto = pkg.join("native-auto");
    let _ = std::fs::remove_dir_all(&native_auto);

    let run = |entry: &str| {
        Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg("--lib")
            .arg("tests/lib")
            .arg(pkg.join(entry))
            .env("LOFT_NO_CACHE", "1")
            .output()
            .expect("run the loft binary")
    };

    // entry_a built the (wrong) cdylib before the fix; entry_b is where the
    // stale-export-set adoption used to panic.
    let a = run("entry_a.loft");
    let b = run("entry_b.loft");

    for (name, out) in [("entry_a", &a), ("entry_b", &b)] {
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success(),
            "{name} must run clean (no cdylib-dispatch panic).\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(
            !stderr.contains("could not be wired"),
            "{name}: an entry-package function was marked for cdylib dispatch (the #460 \
             marking bug).\nstderr:\n{stderr}"
        );
    }
    assert_eq!(
        String::from_utf8_lossy(&a.stdout),
        "111\n",
        "entry_a output"
    );
    assert_eq!(
        String::from_utf8_lossy(&b.stdout),
        "222\n",
        "entry_b output"
    );

    // The decisive invariant: the entry package is the script, so it never
    // builds a cdylib — the structural cause of the stale-export-set mismatch.
    assert!(
        !native_auto.exists(),
        "the entry package must NOT be auto-native-compiled, but {} exists",
        native_auto.display()
    );
}

/// #461 — an auto-native cdylib hardcodes type-table INDICES (e.g. `OpWriteFile`'s
/// `db_tp`), but those indices shift with which libraries are loaded, and the
/// cdylib resolves them against the caller's SHARED `Stores` at runtime.  A cdylib
/// is cached per-library, so one consumer's build can be reused by another whose
/// type table differs — making the baked indices resolve to the WRONG type and
/// silently corrupt (the moros GLB header wrote 8-byte fields for `as i32`).
///
/// The fixture `binwriter` writes a 2-field i32 header via `f += X as i32`; the
/// fixture `typeshift` adds struct types that shift `binwriter`'s `i32` index
/// (verified: `db_tp` 64 → 67).  Build `binwriter`'s cdylib in the bare context,
/// then call it from a context where `typeshift` is also loaded: the freshness key
/// must notice the layout changed and rebuild, so the write stays 4 bytes wide.
#[test]
fn cdylib_type_indices_stay_valid_across_consumer_contexts() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable");
        return;
    }

    let native_auto = std::path::Path::new("tests/lib/binwriter/native-auto");
    let _ = std::fs::remove_dir_all(native_auto);

    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_n3_461_{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    // Bare context: only binwriter loaded — its cdylib bakes binwriter's `i32` index.
    std::fs::write(
        tmp.join("bare.loft"),
        "use binwriter;\nfn main() { write_magic(arguments()[0], 2); }\n",
    )
    .unwrap();
    // Shifted context: typeshift's struct types move `i32` to a different index.
    std::fs::write(
        tmp.join("shifted.loft"),
        "use typeshift;\nuse binwriter;\n\
         fn main() { _ = ts_touch(); write_magic(arguments()[0], 2); }\n",
    )
    .unwrap();

    let run = |entry: &str, out: &std::path::Path| {
        Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg("--interpret")
            .arg("--lib")
            .arg("tests/lib")
            .arg(tmp.join(entry))
            .arg(out)
            .env("LOFT_NO_CACHE", "1")
            .output()
            .expect("run the loft binary")
    };

    // Build + cache binwriter's cdylib in the bare context first…
    let bare_out = tmp.join("bare.bin");
    let a = run("bare.loft", &bare_out);
    assert!(
        a.status.success(),
        "bare run failed.\nstderr:\n{}",
        String::from_utf8_lossy(&a.stderr)
    );
    // …then reuse it from the shifted context, where the baked index is wrong
    // unless the cdylib is rebuilt for this layout.
    let shifted_out = tmp.join("shifted.bin");
    let b = run("shifted.loft", &shifted_out);
    assert!(
        b.status.success(),
        "shifted run failed.\nstderr:\n{}",
        String::from_utf8_lossy(&b.stderr)
    );

    // Each header field is a 4-byte i32: magic 'glTF' (LE) + version 2 → 8 bytes.
    // A stale-index cdylib would write 8-byte i64 fields (16 bytes, version split).
    let want: &[u8] = &[0x67, 0x6c, 0x54, 0x46, 0x02, 0x00, 0x00, 0x00];
    for (name, path) in [("bare", &bare_out), ("shifted", &shifted_out)] {
        let bytes = std::fs::read(path).expect("read output");
        assert_eq!(
            bytes, want,
            "{name} context wrote the wrong header — `as i32` did not narrow to 4 bytes \
             (stale cdylib type index resolved against the host table)"
        );
    }

    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(native_auto);
}

/// loft#717 — an auto-built cdylib must be VERIFIED against the type layout it
/// was generated for, not merely trusted because its filename matches.
///
/// #715 content-addressed the artifact so two contexts can never name the same
/// file, and closed the class "by construction". That is an argument, and it
/// holds exactly as long as two things stay true: the fingerprint keeps covering
/// every layout difference, and nothing else can put a file at that name. Neither
/// is checkable at runtime, and when either fails the artifact is not slightly
/// wrong — the generated cdylib hardcodes type-table INDICES, so it resolves them
/// against a foreign table and reads at the wrong offsets. That is silent memory
/// corruption, whose crash lands arbitrarily far from its cause.
///
/// So the artifact now names its own layout (`loft_type_layout_fp_v1`) and the
/// adopter asks. This plants one context's artifact at the other's exact filename
/// — the aliasing #715 argues is unreachable — and requires that it be rejected.
///
/// The test carries its own control, because "it rebuilt" has a boring competing
/// explanation: copying a file changes its mtime, and a rebuild triggered by mtime
/// would pass this test while verifying nothing. So the SAME copy is done with a
/// MATCHING artifact first, and that one must be adopted. Both arms churn the
/// mtime identically; only the declared layout differs.
// @speed 3.3
#[test]
fn a_foreign_context_artifact_is_rejected_not_adopted() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable");
        return;
    }
    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_717_layout_{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    // A PRIVATE copy: this test COUNTS the artifacts in `native-auto/`, and a
    // sibling test builds into the shared one (see `private_lib`).
    let lib = private_lib(&tmp, &["mathnative", "typeshift"]);
    let native_auto = lib.join("mathnative/native-auto");
    let native_auto = native_auto.as_path();

    // Two programs over the SAME library whose type tables differ: the second
    // loads another library first, which shifts every later type index.
    let bare = tmp.join("bare.loft");
    std::fs::write(
        &bare,
        "use mathnative;\nfn main() { println(\"{double(21)}\"); }\n",
    )
    .unwrap();
    let shifted = tmp.join("shifted.loft");
    std::fs::write(
        &shifted,
        "use typeshift;\nuse mathnative;\n\
         fn main() { _ = ts_touch(); println(\"{double(21)}\"); }\n",
    )
    .unwrap();

    let run = |prog: &std::path::Path| {
        Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg("--lib")
            .arg(&lib)
            .arg(prog)
            .env("LOFT_NO_CACHE", "1")
            .output()
            .expect("run the loft binary")
    };
    let sos = || -> Vec<std::path::PathBuf> {
        let mut v: Vec<_> = std::fs::read_dir(native_auto)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    // `.dll` too — Windows names an auto-built cdylib `<stem>.dll`
                    // (`native_lib.rs::cdylib_file_name`), so a filter of just
                    // `so`/`dylib` counts ZERO there and the artifact assertions below
                    // read as "nothing was built".  `cdylib_present` above already
                    // spells all three out, and `n3_parity.rs` filters on all three;
                    // only this closure was short.  The import-library sidecar
                    // (`<stem>.dll.lib`) has extension `lib`, so it is not counted twice.
                    .filter(|p| {
                        p.extension()
                            .is_some_and(|e| e == "so" || e == "dylib" || e == "dll")
                    })
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    };

    let _ = std::fs::remove_dir_all(native_auto);
    assert!(run(&bare).status.success(), "bare context runs");
    let after_bare = sos();
    assert_eq!(after_bare.len(), 1, "the bare context built one artifact");
    let bare_so = after_bare[0].clone();

    assert!(run(&shifted).status.success(), "shifted context runs");
    let two = sos();
    if two.len() < 2 {
        // Both contexts fingerprinted the same, so there is no foreign artifact to
        // plant and nothing to assert. Say so rather than passing quietly.
        eprintln!("skip: the two contexts share a type-layout fingerprint");
        let _ = std::fs::remove_dir_all(&tmp);
        return;
    }
    let other_so = two.iter().find(|p| **p != bare_so).unwrap().clone();

    let mtime = |p: &std::path::Path| std::fs::metadata(p).unwrap().modified().unwrap();

    // CONTROL: the bare context's OWN artifact, re-copied over itself. Same mtime
    // churn, matching layout — it must be adopted, or the test below proves nothing.
    let own = std::fs::read(&bare_so).unwrap();
    std::fs::write(&bare_so, &own).unwrap();
    let before = mtime(&bare_so);
    assert!(run(&bare).status.success(), "control run succeeds");
    assert_eq!(
        mtime(&bare_so),
        before,
        "CONTROL FAILED: a matching artifact was rebuilt anyway, so this test cannot \
         tell verification from mtime churn"
    );

    // TEST: the other context's artifact at this context's exact filename.
    let foreign = std::fs::read(&other_so).unwrap();
    std::fs::write(&bare_so, &foreign).unwrap();
    let before = mtime(&bare_so);
    let out = run(&bare);
    assert!(
        out.status.success(),
        "the run must recover, not fail.\nstderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_ne!(
        mtime(&bare_so),
        before,
        "a cdylib built for a DIFFERENT type layout was adopted instead of rebuilt — \
         its hardcoded type indices would resolve against this context's table"
    );
    // Compared by digest: a rebuild is not byte-identical to the original (rustc
    // embeds paths and is not reproducible here), so the claim that holds is that
    // what sits there is no longer the FOREIGN artifact.
    let digest = |b: &[u8]| -> (usize, u64) {
        let mut h: u64 = 1469598103934665603;
        for &x in b {
            h = (h ^ u64::from(x)).wrapping_mul(1099511628211);
        }
        (b.len(), h)
    };
    assert_ne!(
        digest(&std::fs::read(&bare_so).unwrap()),
        digest(&foreign),
        "the foreign artifact is still in place after the rebuild"
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("42"),
        "and it still computes the right answer"
    );

    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(native_auto);
}

/// loft#739 — a `hash<T[key]>` over a LIBRARY-IMPORTED struct shifted the
/// native program's type-id table, so every id baked into the emitted ops from
/// that point on named a different type than the compiler meant.
///
/// The generated `init()` REPLAYS the parse-time registration order; the type
/// ids it operates on are plain integers baked in at compile time. A keyed
/// collection that a struct field references is normally created inline right
/// after its container, so the emitter deliberately keeps it out of the
/// standalone stream. That assumption breaks when the library's own API takes
/// the keyed collection as a parameter: `fill_all` then pre-registers
/// `hash<KTile[tkey]>` while the LIBRARY is being filled — before the importing
/// program's struct exists — so its id PRECEDES its container's. Emitting it
/// inline dropped one position from the sequence and every later id came out
/// one low.
///
/// The visible damage was silent: `f#read as u16` returned null because its
/// `db_tp` const now resolved to a struct, while `as u8`, `as i16` and `as i32`
/// out of the same handle stayed correct. Which width breaks depends on where
/// the shift lands, so the test asserts all four widths and pins each value —
/// a fix that merely stops the null while reading the wrong width still fails.
///
/// `tile_count`'s parameter in `tests/lib/keyedlib.loft` is the trigger and must
/// stay; a library that only exports `KTile` does not reproduce this.
#[test]
fn keyed_collection_over_imported_struct_keeps_type_ids_aligned() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable (--native needs it)");
        return;
    }

    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_i739_{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let prog = tmp.join("main.loft");
    let bin = tmp.join("probe.bin");
    let bin_path = bin.to_string_lossy().replace('\\', "/");

    // `Blk` is never constructed and no store is ever bound — declaring the
    // field is the whole trigger. The four reads then prove the baked `db_tp`
    // consts still name the types the compiler chose.
    std::fs::write(
        &prog,
        format!(
            "use keyedlib::(KTile);\n\
             \n\
             struct Blk {{ tiles: hash<KTile[tkey]> }}\n\
             \n\
             fn main() {{\n\
             \x20   p = \"{bin_path}\";\n\
             \x20   _ = delete(p);\n\
             \x20   {{ w = file(p); w#format = LittleEndian;\n\
             \x20     w += (65 as u8);\n\
             \x20     w += (258 as i16? ?? (0 as i16));\n\
             \x20     w += (66051 as i32? ?? (0 as i32));\n\
             \x20     w += (515 as u16? ?? (0 as u16)); }}\n\
             \x20   f = file(p); f#format = LittleEndian;\n\
             \x20   a = f#read as u8;\n\
             \x20   b = f#read as i16;\n\
             \x20   c = f#read as i32;\n\
             \x20   d = f#read as u16;\n\
             \x20   println(\"{{a}} {{b}} {{c}} {{d}}\");\n\
             }}\n"
        ),
    )
    .unwrap();

    let mut outputs = Vec::new();
    for mode in ["--interpret", "--native"] {
        let out = Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(mode)
            .arg("--lib")
            .arg("tests/lib")
            .arg(&prog)
            .env("LOFT_NO_CACHE", "1")
            .output()
            .expect("run the loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success(),
            "{mode} exited non-zero.\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        // A drift is reported by `Stores::verify_schema_ids`, which names the
        // first id where the generated schema and the compiler's disagree.
        assert!(
            !stderr.contains("diverges from the compiler"),
            "{mode}: the generated schema drifted from the compiler's.\n{stderr}"
        );
        assert!(
            stdout.trim() == "65 258 66051 515",
            "{mode}: a sized read resolved its `db_tp` to the wrong type — \
             expected `65 258 66051 515`, got `{}`",
            stdout.trim()
        );
        outputs.push(stdout);
    }
    // The interpreter was correct throughout, so equality is the standing
    // guarantee this regression broke — assert it rather than only the values.
    assert_eq!(
        outputs[0], outputs[1],
        "the two backends disagree on the sized reads"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// loft#746 — with the same second declaration in play, INSERTING into a
/// separate keyed collection over that library struct aborted.
///
/// Same fixture and same trigger as the test above: `use keyedlib::(KTile)`
/// pre-registers `hash<KTile[tkey]>` while the LIBRARY is filled, then `Blk`'s
/// field registers it again from the importing program.  There the damage was a
/// sized read answering null; here it is `record_new` resolving the insert's
/// element type to `Blk` — a struct, not a collection — and raising "Cannot add
/// to none-structure 'Blk'" from `src/database/structures.rs`.  Deleting the
/// `Blk` line makes the identical program run, which is what made the report
/// read as "an unused declaration breaks an unrelated insert".
///
/// This needs its own guard: `LOFT_STRICT_SCHEMA_IDS` stays SILENT on this
/// shape, so the schema-drift assertion in the test above does not cover it.
/// Both the loop count and `tile_count` are asserted — the count alone passes on
/// a build that inserts nothing, and passing the collection back through the
/// library's own API is what proves the element type survived the round trip.
#[test]
fn inserting_into_a_keyed_collection_over_an_imported_struct_works() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable (--native needs it)");
        return;
    }

    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_i746_{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let prog = tmp.join("main.loft");

    // `Blk` is never constructed — declaring the field is the whole trigger, so
    // it must stay. The keys are large and scattered because the report's were;
    // they are not load-bearing (0.. behaves the same), but keeping them costs
    // nothing and matches the shape that was filed.
    std::fs::write(
        &prog,
        "use keyedlib::(KTile, tile_count);\n\
         \n\
         struct Blk { tiles: hash<KTile[tkey]> }\n\
         \n\
         fn main() {\n\
         \x20   idx: hash<KTile[tkey]> = [];\n\
         \x20   for i in 0..100 { idx += KTile { tkey: 128000000 + i, name: \"t{i}\" }; }\n\
         \x20   n = 0;\n\
         \x20   for t in idx { n += 1; }\n\
         \x20   println(\"{n} {tile_count(idx)}\");\n\
         }\n",
    )
    .unwrap();

    let mut outputs = Vec::new();
    for mode in ["--interpret", "--native"] {
        let out = Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(mode)
            .arg("--lib")
            .arg("tests/lib")
            .arg(&prog)
            .env("LOFT_NO_CACHE", "1")
            .output()
            .expect("run the loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success(),
            "{mode} exited non-zero — the insert resolved its element type to a \
             non-collection.\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(
            stdout.trim() == "100 100",
            "{mode}: expected `100 100`, got `{}`",
            stdout.trim()
        );
        outputs.push(stdout);
    }
    assert_eq!(
        outputs[0], outputs[1],
        "the two backends disagree on the keyed inserts"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// loft#715's tail — a package's `native-auto/` stays BOUNDED.
///
/// The artifact name carries the consumer's type-layout fingerprint, so a new file
/// appears per distinct consumer context, and nothing collected the old ones.
/// Measured before this guard existed: `tests/lib/typeshift/native-auto` held 532
/// artifacts and 9.1 GB, growing ~28 MB per suite run, and the tree carried 25 GB
/// of it.  Disk, not correctness — which is exactly why nobody looked.
///
/// Builds more distinct contexts than the keep window and requires the directory to
/// stop growing.  The count is what carries this: a fix that pruned nothing would
/// still leave every program RUNNING, so no behavioural assertion can see it.
// @speed 12.8
#[test]
fn a_packages_artifact_directory_stays_bounded() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable");
        return;
    }
    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_prune_{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    // A private copy, for the same reason the two tests above take one: this
    // COUNTS `native-auto/`, so it must own it.
    let lib = private_lib(&tmp, &["mathnative", "typeshift"]);
    let native_auto = lib.join("mathnative/native-auto");
    std::fs::create_dir_all(&native_auto).unwrap();

    // loft#831, residual half — a stand-in for the `[c] shim` cdylib a package builds
    // into this same directory. It is seeded HERE, before the loop, because the sweep
    // orders by mtime and a shim is content-keyed and built exactly ONCE: it is
    // permanently the oldest file, and therefore the first thing an age-ordered sweep
    // takes. Deleting an auto-native artifact costs a rebuild; deleting the shim
    // deletes the only definition of the package's `#c` symbols, and the next run dies
    // with "symbol not found — or check the spelling", naming neither the library nor
    // the sweep. Reproduced against `tests/fixtures/sqldb/sqlite` before the fix:
    // saturate, run once, shim gone, exit 101.
    //
    // It rides along with the bound assertion rather than in a test of its own because
    // the twelve rustc builds below are the whole cost, and running them twice put ~12s
    // on the PR's critical path — `n3_use_native` sits in nextest's single-slot
    // `heavy-serial` group, so that is 12s nothing else can overlap with.
    let ext = if cfg!(target_os = "windows") {
        "dll"
    } else if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    };
    let prefix = if cfg!(target_os = "windows") {
        ""
    } else {
        "lib"
    };
    let shim = native_auto.join(format!("{prefix}mathnative_shim_00000000deadbeef.{ext}"));
    assert!(
        write_decoy_cdylib(&shim, &[], &tmp),
        "rustc could not build the stand-in shim"
    );

    // Each program pads its own type table with a different number of structs, so
    // every one is a distinct layout fingerprint and mints its own artifact.
    let mut built = 0;
    for n in 0..12 {
        let pad: String = (0..n)
            .map(|i| format!("struct Pad{i} {{ p_a: integer, p_b: text }}\n"))
            .collect();
        let prog = tmp.join(format!("ctx{n}.loft"));
        std::fs::write(
            &prog,
            format!("use mathnative;\n{pad}fn main() {{ println(\"{{double(21)}}\"); }}\n"),
        )
        .unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg("--lib")
            .arg(&lib)
            .arg(&prog)
            .env("LOFT_NO_CACHE", "1")
            .output()
            .expect("run the loft binary");
        assert!(
            out.status.success(),
            "context {n} must still RUN — pruning is a disk policy, never a failure.\
             \nstderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "42\n",
            "context {n} produced the wrong answer"
        );
        built += 1;
    }

    let count = std::fs::read_dir(&native_auto)
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|x| x == "so" || x == "dylib" || x == "dll")
                })
                .count()
        })
        .unwrap_or(0);

    // The CONTROL: distinct contexts really did mint distinct artifacts, so the
    // bound below is a bound and not an artifact of everything sharing one name.
    assert!(
        count > 1,
        "the {built} contexts shared one artifact, so this test proves nothing about \
         pruning — the padding no longer shifts the layout fingerprint"
    );
    // The sweep bounds only the family it BUILT, so the decoy is not counted here.
    let family = std::fs::read_dir(&native_auto)
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    // The LIBRARIES only: each artifact also leaves the generated `.rs`
                    // and `.args` it was built from, and counting those reads as 3x.
                    e.path().extension().is_some_and(|x| x == ext)
                        && e.path()
                            .file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.contains("loft_auto_"))
                })
                .count()
        })
        .unwrap_or(0);
    assert!(
        family <= 8,
        "`native-auto/` grew to {family} auto-native artifacts from {built} contexts; \
         it must keep at most the 8 most recent (`KEEP_ARTIFACTS`) — total files {count}"
    );
    // And the sweep took nothing that was not its own: a `[c]` shim living in the same
    // directory would take the package's whole `#c` surface with it (loft#831).
    assert!(
        shim.exists(),
        "the sweep deleted a foreign library from native-auto/"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

// ── loft#831 — a cdylib that cannot be dispatched through must INTERPRET ─────
//
// The auto-native model has always promised a fallback: "a library that can't
// compile native silently interprets, no `exit`, no `OpStaticCall` to an unbuilt
// symbol."  It was enforced against the wrong fact.  Marking happened because the
// BUILD succeeded, and a build succeeding does not mean this process can dispatch
// through the result — the artifact can be linked against a different
// `libloft.rlib`, be missing a system library, or be replaced by a concurrent
// build between the freshness check and the load.  `byte_code` had by then emitted
// `OpStaticCall` to a symbol nothing would wire, and the first call hit the
// `compile.rs` panic stub (exit 101) with the loft body it was compiled from
// sitting in the same process, ready to run.
//
// crawler measured the consequence: their 88-test gate is green serially and loses
// a DIFFERENT test on each parallel run, which cost them a 15x speedup because a
// suite that fails somewhere new each time is worse than a slow one.
//
// The fix asks the question whose answer matters — load the artifact and `dlsym`
// each bridge — before deciding to dispatch, so an unresolvable symbol simply
// leaves its function interpreting.  Loading also PINS the image for the process,
// so a concurrent prune or rebuild cannot invalidate the decision afterwards.
//
// The fixture is a real artifact replaced by a valid cdylib that exports no
// `loft_shared_*` symbol.  That shape is adopted by every freshness check there
// is: the file exists, it is newer than the sources, it opens, and it declares no
// type layout — which `artifact_matches_layout` reads as "a hand-written cdylib,
// fine to adopt".  Verified against this exact fixture: with the probe removed,
// `--interpret` dies at `compile.rs:365` with exit 101.

/// Compile a stand-in cdylib at `at`, exporting exactly `exports` (bare
/// `extern "C" fn() -> u64` stubs).  With none, it is the artifact that loads and
/// resolves nothing; with one, it forces the PARTIAL case — some functions
/// dispatch native and the rest interpret, in the same run.
fn write_decoy_cdylib(at: &std::path::Path, exports: &[&str], scratch: &std::path::Path) -> bool {
    let src = scratch.join("decoy.rs");
    let mut body = String::new();
    for sym in exports {
        body.push_str(&format!(
            "#[unsafe(no_mangle)] pub extern \"C\" fn {sym}() -> u64 {{ 0 }}\n"
        ));
    }
    // A cdylib with no exports at all still links; the marker keeps it non-empty.
    body.push_str("#[unsafe(no_mangle)] pub extern \"C\" fn loft_decoy_marker() -> u64 { 0 }\n");
    std::fs::write(&src, body).expect("write the decoy source");
    Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition")
        .arg("2021")
        .arg(&src)
        .arg("-o")
        .arg(at)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// The one auto-built artifact under `dir`.
fn sole_artifact(dir: &std::path::Path) -> std::path::PathBuf {
    let ext = if cfg!(target_os = "windows") {
        "dll"
    } else if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    };
    let mut found: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .expect("read native-auto")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == ext))
        .collect();
    assert_eq!(found.len(), 1, "expected exactly one artifact in {dir:?}");
    found.pop().expect("the artifact")
}

#[test]
fn an_unwirable_cdylib_interprets_instead_of_panicking() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable");
        return;
    }
    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_831_unwirable_{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let prog = tmp.join("main.loft");
    std::fs::write(
        &prog,
        "use mathnative;\n\
         fn main() {\n\
         \x20   println(\"{double(21)}\");\n\
         \x20   println(\"{add(3, 4)}\");\n\
         \x20   println(\"{factorial(5)}\");\n\
         }\n",
    )
    .unwrap();
    let lib = private_lib(&tmp, &["mathnative"]);
    let native_auto = lib.join("mathnative/native-auto");

    let run = |extra: &[(&str, &str)]| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
        // `--interpret`: under `--native` the library's functions compile into the
        // whole-program binary and the cdylib is never dispatched to, so the
        // interpreter is the backend that can reach the stub at all.
        cmd.arg("--interpret")
            .arg("--lib")
            .arg(&lib)
            .arg(&prog)
            .env("LOFT_NO_CACHE", "1");
        for (k, v) in extra {
            cmd.env(k, v);
        }
        cmd.output().expect("run the loft binary")
    };

    // 1. A real run first — this builds the artifact whose place the decoy takes.
    let first = run(&[]);
    assert!(
        first.status.success(),
        "the baseline native-dispatch run must pass.\nstderr:\n{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&first.stdout), "42\n7\n120\n");

    // 2. Replace it with a cdylib that loads and exports no bridge at all.
    let artifact = sole_artifact(&native_auto);
    assert!(
        write_decoy_cdylib(&artifact, &[], &tmp),
        "rustc could not build the decoy cdylib"
    );

    let out = run(&[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The whole point: same answers, from the interpreted bodies.
    assert!(
        out.status.success(),
        "an unwirable cdylib must not take the program down (loft#831).\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert_eq!(
        stdout, "42\n7\n120\n",
        "the interpreted fallback must produce identical results"
    );
    assert!(
        !stderr.contains("could not be wired"),
        "nothing may be marked for cdylib dispatch once the probe fails.\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("native function not loaded"),
        "the compile.rs panic stub must be unreachable.\nstderr:\n{stderr}"
    );
    // One line for the library, naming what fell back — not one per function.
    assert!(
        stderr.contains("runs 3 function(s) interpreted this run"),
        "the fallback must be reported once, with a count.\nstderr:\n{stderr}"
    );

    // 3. `LOFT_REQUIRE_NATIVE` exists so a performance run can never silently
    //    interpret.  This is a native→interpreter degrade, so it must hard-fail
    //    and name itself — the same contract the build-failure arm already has.
    let strict = run(&[("LOFT_REQUIRE_NATIVE", "1")]);
    let strict_err = String::from_utf8_lossy(&strict.stderr);
    assert!(
        !strict.status.success(),
        "LOFT_REQUIRE_NATIVE must refuse a run that would interpret.\nstderr:\n{strict_err}"
    );
    assert!(
        strict_err.contains("LOFT_REQUIRE_NATIVE is set"),
        "the refusal must name the env var.\nstderr:\n{strict_err}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// The PARTIAL cell: an artifact exporting SOME of the export set's bridges.
/// Marking has to split — the resolvable function dispatches native, the rest
/// interpret — rather than being all-or-nothing in either direction.  The
/// exported bridge is deliberately one the script never calls, so nothing
/// dispatches into the stand-in's (deliberately wrong-ABI) body.
#[test]
fn a_partially_exporting_cdylib_marks_only_what_resolves() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable");
        return;
    }
    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_831_partial_{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let prog = tmp.join("main.loft");
    std::fs::write(
        &prog,
        "use mathnative;\n\
         fn main() {\n\
         \x20   println(\"{double(21)}\");\n\
         \x20   println(\"{add(3, 4)}\");\n\
         }\n",
    )
    .unwrap();
    let lib = private_lib(&tmp, &["mathnative"]);
    let native_auto = lib.join("mathnative/native-auto");

    let run = || {
        Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg("--interpret")
            .arg("--lib")
            .arg(&lib)
            .arg(&prog)
            .env("LOFT_NO_CACHE", "1")
            .output()
            .expect("run the loft binary")
    };

    assert!(run().status.success(), "baseline run");
    let artifact = sole_artifact(&native_auto);
    assert!(
        write_decoy_cdylib(&artifact, &["loft_shared_n_factorial"], &tmp),
        "rustc could not build the decoy cdylib"
    );

    let out = run();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "partial resolution must still run.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert_eq!(stdout, "42\n7\n");
    assert!(
        stderr.contains("runs 2 function(s) interpreted this run"),
        "exactly the two unresolvable functions fall back — factorial's bridge \
         resolved, so it stays marked.\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("could not be wired"),
        "a marked function must be one that wires.\nstderr:\n{stderr}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// loft#1663 — a library function served by its auto-built cdylib and called THROUGH A
/// FN-REF answers the record it built, whatever its body's shape.
///
/// A fn-ref call hands the callee a return buffer that is a real store with no record in
/// it (`stores.null()`, `pos` 8).  The shared bridge treated only `rec == 0 && pos == 0`
/// as "no destination", so that buffer passed as a destination: the bridge wrote the
/// result into record 0 and the caller read `null`.  It showed only where the callee
/// WROTE into the handed buffer — `return <call>;` forwarding a scalar record (the
/// value-record tuple is written into the buffer) — and not where the callee minted its
/// own store, which is why the expression body and a record holding text were right.
///
/// The cells cross the destination kinds the bridge tests (record, struct-enum, vector
/// — the last keeps the pair, since a store with `rec == 0` there is a valid empty
/// vector) with the two ways a fn-ref reaches the call (a local, a parameter), and a
/// loop that forwards per iteration.  Every expected value is written out by hand.
// @speed 1.2
#[test]
fn a_fn_ref_call_into_a_native_library_answers_the_record_it_built() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable");
        return;
    }
    let tmp = std::env::temp_dir().join(format!("loft_n3_1663_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let pkg = tmp.join("lib/fwd1663");
    std::fs::create_dir_all(pkg.join("src")).unwrap();
    std::fs::write(
        pkg.join("loft.toml"),
        "[package]\nname = \"fwd1663\"\nversion = \"0.1.0\"\nloft = \">=0.8\"\n\n\
         [library]\nentry = \"src/fwd1663.loft\"\ncompile = \"native\"\n",
    )
    .unwrap();
    std::fs::write(
        pkg.join("src/fwd1663.loft"),
        "pub struct St { size: float }\n\
         pub fn default_st() -> St { return St { size: 1.0 }; }\n\
         pub fn unit(_id: integer) -> St { return default_st(); }\n\
         pub fn mk(k: integer) -> St { return St { size: k as float }; }\n\
         pub fn fwd(k: integer) -> St { return mk(k); }\n\
         pub enum Sh { Circle { r: float }, Box { w: float } }\n\
         pub fn mk_sh() -> Sh { return Box { w: 2.5 }; }\n\
         pub fn sh(_id: integer) -> Sh { return mk_sh(); }\n\
         pub fn mkv() -> vector<integer> { return [7, 8, 9]; }\n\
         pub fn vs(_id: integer) -> vector<integer> { return mkv(); }\n",
    )
    .unwrap();
    let prog = tmp.join("main.loft");
    std::fs::write(
        &prog,
        "use fwd1663;\n\
         fn call_it(f: fn(integer) -> fwd1663::St) -> float { b = f(0); b.size }\n\
         fn main() {\n\
         \x20 a = unit(0);\n\
         \x20 r = unit;\n\
         \x20 b = r(0);\n\
         \x20 println(\"direct={a.size} via_ref={b.size} via_param={call_it(unit)}\");\n\
         \x20 f = fwd;\n\
         \x20 s = 0.0;\n\
         \x20 for i in 0..6 { c = f(i); s += c.size; }\n\
         \x20 println(\"sum={s}\");\n\
         \x20 g = sh;\n\
         \x20 match g(0) { fwd1663::Box { w } => println(\"box={w}\"), fwd1663::Circle { r } => println(\"circle={r}\") }\n\
         \x20 h = vs;\n\
         \x20 v = h(0);\n\
         \x20 println(\"len={len(v)} last={v[2]}\");\n\
         }\n",
    )
    .unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_loft"))
            .args(["--interpret", "--lib"])
            .arg(tmp.join("lib"))
            .arg(&prog)
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_STORES", "warn")
            .output()
            .expect("run the loft binary")
    };
    // The first run interprets the library while it builds the cdylib; the SECOND is the
    // one that dispatches into it, and the one the defect lived in.
    let _ = run();
    let out = run();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "loft exited non-zero.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    let built = std::fs::read_dir(pkg.join("native-auto")).is_ok_and(|rd| {
        rd.flatten().any(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            n.contains("loft_auto_fwd1663")
                && (n.ends_with(".so") || n.ends_with(".dll") || n.ends_with(".dylib"))
        })
    });
    assert!(
        built,
        "the library's cdylib was not built — the native path was not taken"
    );
    // 0+1+2+3+4+5 = 15; `Box { w: 2.5 }`; `[7, 8, 9]`.
    assert_eq!(
        stdout, "direct=1 via_ref=1 via_param=1\nsum=15\nbox=2.5\nlen=3 last=9\n",
        "a fn-ref call into the native library answered the wrong value\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("leak"),
        "the fn-ref calls leaked a store:\n{stderr}"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}
