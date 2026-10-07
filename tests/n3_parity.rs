// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN11 Arc N / N3 Step 1 — the **parity instrument** (the gate for making the
//! native/interpret choice invisible).
//!
//! *Invariant:* a library run native is **byte-identical** to run interpreted.
//!
//! This harness runs the same program three ways and diffs stdout against the
//! interpreted reference:
//! - **interp** — `LOFT_NO_NATIVE_LIBS=1` forces every `use`d library to interpret;
//! - **mixed** — the default (Step 3, default-native): the script interprets, every
//!   `use`d library auto-compiles + dispatches over the shared store — no opt-in;
//! - **native** — `--native` compiles the whole program (script + library).
//!
//! A divergence is a real shared-store-ABI / codegen bug caught **before** the
//! choice becomes invisible — which is exactly what made the default-native flip
//! (Step 3) a proven move rather than a hopeful one.  Seeded with `datalib`, whose
//! public API carries the store-touching types (vector/struct/text/enum, args +
//! returns) across the boundary.

use loft::file_access as fa;
use std::path::Path;
use std::process::Command;

struct Run {
    success: bool,
    stdout: String,
    stderr: String,
}

/// Run the loft binary on `prog` with extra `args` + `env`, capturing output.
fn run(args: &[&str], env: &[(&str, &str)], prog: &Path) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    cmd.arg("--lib")
        .arg("tests/lib")
        .args(args)
        .arg(prog)
        .env("LOFT_NO_CACHE", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft binary");
    Run {
        success: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// Run the loft binary on `prog` against an explicit (usually tmp, writable) `libdir`
/// — for tests that own their fixtures so they never race on a shared `native-auto/`.
fn run_against(libdir: &Path, prog: &Path, env: &[(&str, &str)]) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    cmd.arg("--lib")
        .arg(libdir)
        .arg(prog)
        .env("LOFT_NO_CACHE", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft binary");
    Run {
        success: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// Write a minimal library package (`<libdir>/<name>/{loft.toml,src/<name>.loft}`),
/// returning its `native-auto` dir.  `dep` optionally adds a `[dependencies]` path edge.
fn write_lib(libdir: &Path, name: &str, dep: Option<&str>, body: &str) -> std::path::PathBuf {
    let pkg = libdir.join(name);
    fa::create_dir_all(pkg.join("src")).unwrap();
    let deps = dep.map_or(String::new(), |d| {
        format!("[dependencies]\n{d} = {{ path = \"../{d}\" }}\n")
    });
    fa::write(
        pkg.join("loft.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nloft = \">=0.8\"\n\
             [library]\nentry = \"src/{name}.loft\"\n{deps}"
        ),
    )
    .unwrap();
    fa::write(pkg.join("src").join(format!("{name}.loft")), body).unwrap();
    pkg.join("native-auto")
}

/// Assert the three modes all succeed and produce byte-identical stdout, against
/// the interpreted reference.  Returns the (shared) stdout for value checks.
///
/// @PLN11 Arc N / N5 — mixed-boundary soundness + parity.  Two soundness legs ride
/// on the same three runs (no extra cdylib builds):
/// - **D (parity)** — `mixed`/`native` stdout must equal the interpreted reference;
///   a shared-store-ABI corruption across the interp↔native boundary surfaces as a
///   divergence (the only *runtime* soundness signal the mixed path has — the
///   in-process sanitizers are blind to the spawned cdylib: ASan sees interpreter
///   targets only, the `stack_align_guard` sweep can't see spawned binaries, and
///   Miri can't `dlopen` a cdylib at all).
/// - **E (predictable memory)** — every run arms `LOFT_STORE_GUARD`, which arms the
///   live **Plan-57 Phase-4 Goal-E guard** (`scopes::check`'s
///   `reclaim_unfreed_eligible == 0` assertion) over BOTH the interpreted script and
///   the library's codegen-time scope analysis.  If a native library's codegen left a
///   reclaim-eligible store live-but-dead past a later alloc, the guard PANICS —
///   caught here by the `r.success` check above.  This guard is **positively
///   controlled** (falsifiable): `watermark.rs::phase4_goal_e_guard_is_falsifiable`
///   proves it fires on an injected reclaim regression (`LOFT_STORE_GUARD_INJECT`) and
///   is silent otherwise.  (The older `[store-guard]` *eprintln* asserted below is
///   superseded by that assertion and silent by construction — a cheap secondary, not
///   the real gate.)
fn assert_three_mode_parity(prog: &Path) -> String {
    let guard = ("LOFT_STORE_GUARD", "1");
    let interp = run(&[], &[("LOFT_NO_NATIVE_LIBS", "1"), guard], prog);
    let mixed = run(&[], &[guard], prog);
    let native = run(&["--native"], &[guard], prog);

    for (name, r) in [("interp", &interp), ("mixed", &mixed), ("native", &native)] {
        assert!(
            r.success,
            "{name} mode failed.\nstdout:\n{}\nstderr:\n{}",
            r.stdout, r.stderr
        );
        assert!(
            !r.stderr.contains("[store-guard]"),
            "STORE-GUARD FIRED on the {name} path (Goal E — predictable memory): a \
             store-backed value frees later than the source confines it.\nstderr:\n{}",
            r.stderr
        );
    }
    assert_eq!(
        mixed.stdout, interp.stdout,
        "PARITY DIVERGENCE: mixed (auto-native lib) != interpreted"
    );
    assert_eq!(
        native.stdout, interp.stdout,
        "PARITY DIVERGENCE: --native != interpreted"
    );
    interp.stdout
}

/// A program exercising every store-touching boundary type in BOTH construction
/// directions: a `pub` library type constructed by NATIVE code and returned (the
/// factories `make_point`/`make_circle`), AND constructed by the INTERPRETER and
/// passed into a native function (`Point {...}` / `Circle {...}`).  Both must
/// round-trip byte-identically; vectors/text exercise the same both ways.
const DATALIB_PROG: &str = "use datalib::*;\n\
     fn main() {\n\
     \x20   a = vec_sum([10, 20, 30]);\n\
     \x20   println(\"{a}\");\n\
     \x20   v = range_vec(4);\n\
     \x20   println(\"{vec_sum(v)}\");\n\
     \x20   p = make_point(3, 4);\n\
     \x20   println(\"{point_sum(p)}\");\n\
     \x20   pd = Point { x: 7, y: 8 };\n\
     \x20   println(\"{point_sum(pd)}\");\n\
     \x20   println(\"{shout(\"hi\")}\");\n\
     \x20   c = make_circle(2);\n\
     \x20   println(\"{area(c)}\");\n\
     \x20   cd = Circle { r: 3 };\n\
     \x20   println(\"{area(cd)}\");\n\
     \x20   r = make_rect(2, 5);\n\
     \x20   println(\"{area(r)}\");\n\
     \x20   rd = Rect { w: 3, h: 4 };\n\
     \x20   println(\"{area(rd)}\");\n\
     }\n";

// @speed 1.3
#[test]
fn datalib_store_touching_types_parity() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable (mixed + native modes need it)");
        return;
    }

    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_n3_parity_{pid}"));
    let _ = fa::remove_dir_all(&tmp);
    fa::create_dir_all(&tmp).unwrap();
    let prog = tmp.join("main.loft");
    fa::write(&prog, DATALIB_PROG).unwrap();

    // The library's auto-built cdylib lands in its package's git-ignored dir.
    let native_auto = std::path::Path::new("tests/lib/datalib/native-auto");
    let _ = fa::remove_dir_all(native_auto);

    let stdout = assert_three_mode_parity(&prog);
    // Sanity-anchor the reference so a "parity holds but all three are wrong"
    // regression is also caught.
    assert_eq!(
        stdout, "60\n6\n7\n15\nhi!\n12\n27\n10\n12\n",
        "reference output"
    );

    let _ = fa::remove_dir_all(&tmp);
    let _ = fa::remove_dir_all(native_auto);
}

/// @PLN11 Arc N / N3 Step 2 — the build-failure fallback.
///
/// *Invariant:* a library that can't compile native **silently interprets** —
/// byte-identical, no `exit`, no `OpStaticCall` to an unbuilt symbol.
///
/// `LOFT_FORCE_NATIVE_BUILD_FAIL=1` simulates a cdylib build failure (e.g. a
/// codegen gap the gate didn't catch).  With a `rustc` toolchain present (the
/// normal dev/CI host) that is a REAL build failure, not a graceful fallback: loft
/// refuses to silently interpret it (which would hand back a partly-interpreted
/// binary) and exits non-zero with an actionable message.  No cdylib is built.  The
/// graceful no-toolchain fallback is covered by the rustc-absent tests.
#[test]
fn build_failure_with_rustc_present_hard_fails() {
    let pid = std::process::id();
    let root = std::env::temp_dir().join(format!("loft_n3_fail_{pid}"));
    let _ = fa::remove_dir_all(&root);
    let libdir = root.join("lib");
    // Own the fixture (tmp) so the `no cdylib built` assertion can never race a
    // concurrent test building the same package's `native-auto/`.
    let native_auto = write_lib(
        &libdir,
        "fblib",
        None,
        "pub fn shout(s: text) -> text { s + \"!\" }\n\
         pub fn doubled(v: vector<integer>) -> vector<integer> {\n\
         \x20   out: vector<integer> = [];\n\
         \x20   for x in v { out += [x * 2]; }\n\
         \x20   out\n}\n",
    );
    let prog = root.join("main.loft");
    fa::write(
        &prog,
        "use fblib::*;\nfn main() {\n\
         \x20   println(shout(\"hi\"));\n\
         \x20   println(\"{doubled([1, 2, 3])}\");\n}\n",
    )
    .unwrap();

    let forced = run_against(&libdir, &prog, &[("LOFT_FORCE_NATIVE_BUILD_FAIL", "1")]);

    assert!(
        !forced.success,
        "a native build failure with rustc present must hard-fail, not silently interpret.\nstderr:\n{}",
        forced.stderr
    );
    assert!(
        forced.stderr.contains("real build failure")
            || forced.stderr.contains("refusing to silently interpret"),
        "the hard-fail must explain itself (real build failure, not a missing-toolchain fallback).\nstderr:\n{}",
        forced.stderr
    );
    assert!(
        !fa::exists(&native_auto),
        "no cdylib should have been built when the build is forced to fail"
    );

    let _ = fa::remove_dir_all(&root);
}

/// A consumer of `plainlib` — a library that does **not** opt into
/// `compile = "native"`.  Its API is store-touching (text + vector) in both
/// directions, so a shared-store-ABI bug on the default-native path shows up.
const PLAINLIB_PROG: &str = "use plainlib::*;\n\
     fn main() {\n\
     \x20   println(banner(\"hi\"));\n\
     \x20   println(\"{doubled([1, 2, 3])}\");\n\
     }\n";

/// @PLN11 Arc N / N3 Step 3 — **default-native**, proven on a library that never
/// opted in.
///
/// *Invariant:* with no annotation and no flag, a `use`d library is a native
/// candidate — it auto-builds a cdylib and dispatches, byte-identical to
/// interpreting it.  `LOFT_NO_NATIVE_LIBS=1` is the interpret escape.
///
/// Asserts both directions of the now-default flip: by DEFAULT `plainlib` (which
/// has no `compile = "native"`) auto-builds + dispatches native; under
/// `LOFT_NO_NATIVE_LIBS` it interprets and builds no cdylib.
// @speed 1.2
#[test]
fn default_native_dispatches_unopted_library() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable (default-native build needs it)");
        return;
    }

    let pid = std::process::id();
    let tmp = std::env::temp_dir().join(format!("loft_n3_plainparity_{pid}"));
    let _ = fa::remove_dir_all(&tmp);
    fa::create_dir_all(&tmp).unwrap();
    let prog = tmp.join("main.loft");
    fa::write(&prog, PLAINLIB_PROG).unwrap();
    let native_auto = std::path::Path::new("tests/lib/plainlib/native-auto");

    let interp = run(&[], &[("LOFT_NO_NATIVE_LIBS", "1")], &prog);

    // ESCAPE: `LOFT_NO_NATIVE_LIBS` interprets and builds no cdylib.
    let _ = fa::remove_dir_all(native_auto);
    let escaped = run(&[], &[("LOFT_NO_NATIVE_LIBS", "1")], &prog);
    assert!(
        escaped.success,
        "interpret-escape run failed:\n{}",
        escaped.stderr
    );
    assert!(
        !fa::exists(native_auto),
        "LOFT_NO_NATIVE_LIBS must build no cdylib"
    );

    // DEFAULT: plainlib never opted in, yet default-native auto-builds + dispatches.
    let _ = fa::remove_dir_all(native_auto);
    let dflt = run(&[], &[], &prog);
    assert!(
        dflt.success,
        "default-native run failed:\nstdout:\n{}\nstderr:\n{}",
        dflt.stdout, dflt.stderr
    );
    assert_eq!(
        dflt.stdout, interp.stdout,
        "PARITY DIVERGENCE: default-native (un-opted lib) != interpreted"
    );
    assert!(
        fa::exists(native_auto),
        "default-native must auto-build the cdylib for an un-opted-in library"
    );

    let _ = fa::remove_dir_all(&tmp);
    let _ = fa::remove_dir_all(native_auto);
}

/// The single auto-built cdylib under a package's `native-auto/`, if any.
fn single_cdylib(native_auto: &Path) -> Option<std::path::PathBuf> {
    std::fs::read_dir(native_auto)
        .ok()?
        .flatten()
        .find_map(|e| {
            let p = e.path();
            let is_lib = p
                .extension()
                .is_some_and(|x| x == "so" || x == "dylib" || x == "dll");
            is_lib.then_some(p)
        })
}

/// @PLN11 N3 Step 4 — **dev-interpret-on-edit**.
///
/// *Invariant:* a library's first use builds eagerly (native), but **editing** it
/// makes the next run **interpret** the new code with **no `rustc`** (instant loop);
/// once editing settles, a run rebuilds and it is native again.
///
/// Proven on a writable copy of a library across three runs:
/// 1. first run eager-builds the cdylib and dispatches native (`v1`);
/// 2. after an edit, the next run prints the NEW code (`v2`) — so it interpreted,
///    not dispatched the stale cdylib — and the cdylib file is **untouched** (no
///    rebuild, no `rustc`);
/// 3. with the source now unchanged since run 2, the next run **rebuilds** (the
///    cdylib mtime changes) and is native again.
// @speed 2.8
#[test]
fn editing_a_library_interprets_then_rebuilds_when_stable() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable (Step 4 needs the native build)");
        return;
    }

    let pid = std::process::id();
    let root = std::env::temp_dir().join(format!("loft_n3_edit_{pid}"));
    let _ = fa::remove_dir_all(&root);
    let libdir = root.join("lib");
    let pkg = libdir.join("edlib");
    fa::create_dir_all(pkg.join("src")).unwrap();
    fa::write(
        pkg.join("loft.toml"),
        "[package]\nname = \"edlib\"\nversion = \"0.1.0\"\nloft = \">=0.8\"\n\
         [library]\nentry = \"src/edlib.loft\"\n",
    )
    .unwrap();
    let src = pkg.join("src").join("edlib.loft");
    // The body BUILDS its text rather than returning a literal: a text-returning
    // function that hands back a text it does not own is bufferless, and the shared
    // bridge has nowhere to put those bytes, so the gate keeps it interpreted
    // (loft#773).  This test is about the cdylib lifecycle, so its fixture has to be
    // a function that actually reaches the cdylib.
    fa::write(&src, "pub fn greet() -> text { n = 1; return \"v{n}\"; }\n").unwrap();
    let prog = root.join("main.loft");
    fa::write(&prog, "use edlib::*;\nfn main() { println(greet()); }\n").unwrap();
    let native_auto = pkg.join("native-auto");

    // Run the binary against this editable lib dir (no `tests/lib`).
    let run_edit = || -> Run {
        let out = Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg("--lib")
            .arg(&libdir)
            .arg(&prog)
            .env("LOFT_NO_CACHE", "1")
            .output()
            .expect("spawn loft binary");
        Run {
            success: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    };

    // 1. First use → eager build → native.
    let r1 = run_edit();
    assert!(r1.success, "run 1 failed:\n{}", r1.stderr);
    assert_eq!(r1.stdout.trim(), "v1");
    let so = single_cdylib(&native_auto).expect("run 1 must eager-build the cdylib");
    let mtime1 = fa::metadata(&so).unwrap().modified().unwrap();

    // Edit the library (sleep first so the new source mtime is unambiguously newer
    // than the just-built cdylib, even on a coarse-granularity filesystem).
    std::thread::sleep(std::time::Duration::from_millis(1100));
    fa::write(&src, "pub fn greet() -> text { n = 2; return \"v{n}\"; }\n").unwrap();

    // 2. Edit run → interpret the NEW code, NO rebuild.
    let r2 = run_edit();
    assert!(r2.success, "run 2 failed:\n{}", r2.stderr);
    assert_eq!(
        r2.stdout.trim(),
        "v2",
        "an edit must take effect immediately — interpreted, not the stale cdylib"
    );
    let mtime2 = fa::metadata(&so).unwrap().modified().unwrap();
    assert_eq!(
        mtime1, mtime2,
        "the edit run must NOT rebuild the cdylib (no `rustc` per save)"
    );

    // 3. Source unchanged since run 2 → rebuild → native again.
    let r3 = run_edit();
    assert!(r3.success, "run 3 failed:\n{}", r3.stderr);
    assert_eq!(r3.stdout.trim(), "v2");
    let mtime3 = fa::metadata(&so).unwrap().modified().unwrap();
    assert_ne!(
        mtime2, mtime3,
        "once editing settles, the next run rebuilds the cdylib → native"
    );

    let _ = fa::remove_dir_all(&root);
}

/// @PLN11 N3 F2 — interdependent libraries are **fully native**.
///
/// *Invariant:* when a consumer uses both a library (`top`) and the library it
/// depends on (`base`) directly, BOTH get their own cdylib — so `base`'s functions
/// dispatch native even when called directly, not just when reached through `top`.
///
/// Regression for the diverged-resolution bug: a library pulled in transitively (or
/// used directly after being loaded transitively, so the direct `use` dedups) was
/// resolved via the direct/sibling path, which — unlike `apply_manifest_side_effects`
/// — never recorded it as a native candidate, so it had no cdylib and its direct
/// calls interpreted.  Now both build, and the result matches interpreting.
// @speed 1.7
#[test]
fn interdependent_libraries_are_fully_native() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable (F2 needs the native build)");
        return;
    }

    let pid = std::process::id();
    let root = std::env::temp_dir().join(format!("loft_n3_diamond_{pid}"));
    let _ = fa::remove_dir_all(&root);
    let libdir = root.join("lib");
    let base_auto = write_lib(
        &libdir,
        "dbase",
        None,
        "pub fn base_double(v: vector<integer>) -> vector<integer> {\n\
         \x20   out: vector<integer> = [];\n\
         \x20   for x in v { out += [x * 2]; }\n\
         \x20   out\n}\n",
    );
    let top_auto = write_lib(
        &libdir,
        "dtop",
        Some("dbase"),
        "pub use dbase::*;\n\
         pub fn top_sum(v: vector<integer>) -> integer {\n\
         \x20   d = base_double(v);\n\
         \x20   t = 0;\n\
         \x20   for x in d { t += x; }\n\
         \x20   t\n}\n",
    );
    let prog = root.join("main.loft");
    // Diamond: consumer uses BOTH `dtop` and its dependency `dbase` directly.
    fa::write(
        &prog,
        "use dtop::*;\nuse dbase::*;\nfn main() {\n\
         \x20   println(\"{top_sum([1, 2, 3])}\");\n\
         \x20   println(\"{base_double([5, 6])}\");\n}\n",
    )
    .unwrap();

    let interp = run_against(&libdir, &prog, &[("LOFT_NO_NATIVE_LIBS", "1")]);
    let native = run_against(&libdir, &prog, &[]);

    assert!(
        native.success,
        "default-native run failed:\n{}",
        native.stderr
    );
    assert_eq!(
        native.stdout, interp.stdout,
        "PARITY DIVERGENCE: interdependent libs (default-native) != interpreted"
    );
    assert_eq!(interp.stdout, "12\n[10,12]\n", "reference output");
    assert!(
        single_cdylib(&top_auto).is_some(),
        "the dependent library `dtop` must build its cdylib"
    );
    assert!(
        single_cdylib(&base_auto).is_some(),
        "the dependency `dbase`, used directly, must ALSO build its OWN cdylib (F2)"
    );

    let _ = fa::remove_dir_all(&root);
}

/// @PLN118 arc F — the shared-store bridge must not ORPHAN the fallback destination
/// it allocates when the interpreted caller forwards a null retbuf.
///
/// A struct-returning library fn whose body is a NESTED call (`wrap_v3` returns
/// `make_v3(...)`) makes the caller re-sentinel its retbuf variable each iteration,
/// so every call after the first forwards a null hidden-dest ref.  The bridge then
/// allocates a fallback record (`null_named` + `OpDatabase`), but the inner
/// struct-literal `make_v3` ignores its retbuf and returns a fresh store — orphaning
/// the fallback, one leaked store per call, ONLY across the interp↔cdylib boundary
/// (whole-native has no bridge and never leaked).  The fix frees the orphaned
/// fallback after the call.
///
/// loft#688 added a SECOND, upstream guard against the same orphan: the NRVO return
/// buffer is a promoted local that lives in the never-swept argument scope, and it is
/// now freed at each return that delivers a different store.  The bridge's fallback
/// dest is exactly that shape, so `LOFT_NO_BRIDGE_ORPHAN_FREE=1` no longer reproduces
/// the leak — it was the differential positive control here, and it is now defence in
/// depth instead.  Both cells therefore assert leak-free: if EITHER mechanism regresses
/// into leaking, this fails.
///
/// Non-vacuity of the leak instrument itself now rides on
/// `tests/scripts/688-abandoned-return-candidate-leak.loft`, which was verified to
/// report leaks against a pre-fix binary and is swept by `run_test`'s leak gate.
/// The bridge-level free may now be redundant for this shape; proving it dead needs
/// its own sentinel sweep, so it stays.
// @speed 1.2
#[test]
fn shared_bridge_nested_return_no_orphan_leak() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable (mixed mode needs it)");
        return;
    }
    let pid = std::process::id();
    let root = std::env::temp_dir().join(format!("loft_arcf_orphan_{pid}"));
    let _ = fa::remove_dir_all(&root);
    let libdir = root.join("libs");
    fa::create_dir_all(&libdir).unwrap();

    let native_auto = write_lib(
        &libdir,
        "arcf",
        None,
        "pub struct V3 { x: float, y: float, z: float }\n\
         pub fn make_v3(a: float, b: float, c: float) -> V3 { V3 { x: a, y: b, z: c } }\n\
         pub fn wrap_v3(a: integer) -> V3 { make_v3(a as float, 0.0, 0.0) }\n",
    );
    let prog = root.join("main.loft");
    fa::write(
        &prog,
        "use arcf::*;\nfn main() {\n\
         \x20   total = 0.0;\n\
         \x20   n = 0;\n\
         \x20   while n < 50 { c = wrap_v3(n); total = total + c.x; n = n + 1; }\n\
         \x20   println(\"{total}\");\n}\n",
    )
    .unwrap();

    const LEAK_MARK: &str = "not freed at program exit";

    // The leak is interp↔cdylib-boundary-specific: the SCRIPT must interpret while
    // the library auto-compiles to a cdylib (so `wrap_v3` dispatches through the
    // shared bridge).  Force `--interpret` explicitly — whole-`--native` (this box's
    // default backend) has no bridge and could never surface the leak, making the
    // test vacuous.  Each run rebuilds the cdylib fresh so neither races a concurrent
    // suite's shared target artifacts.
    let run_interp = |env: &[(&str, &str)]| -> Run {
        let _ = fa::remove_dir_all(&native_auto);
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
        cmd.arg("--interpret")
            .arg("--lib")
            .arg(&libdir)
            .arg(&prog)
            .env("LOFT_NO_CACHE", "1");
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("spawn loft binary");
        Run {
            success: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    };

    // Fix ON (default): the nested-return call must be leak-free AND correct
    // (sum of 0..49 = 1225).
    let fixed = run_interp(&[]);
    assert!(fixed.success, "fixed run failed:\n{}", fixed.stderr);
    assert_eq!(fixed.stdout.trim(), "1225", "reference output (sum 0..49)");
    assert!(
        !fixed.stderr.contains(LEAK_MARK),
        "@PLN118 arc F REGRESSION: the shared bridge orphaned its fallback dest — \
         leak with the fix active.\nstderr:\n{}",
        fixed.stderr
    );

    // Defence in depth: with the bridge's own orphan-free disabled, loft#688's
    // ownership sweep must still reclaim the fallback dest — it is an NRVO buffer
    // that this call does not deliver, which is precisely what that sweep frees.
    let control = run_interp(&[("LOFT_NO_BRIDGE_ORPHAN_FREE", "1")]);
    assert!(control.success, "control run failed:\n{}", control.stderr);
    assert_eq!(
        control.stdout.trim(),
        "1225",
        "the second guard must not change the value"
    );
    assert!(
        !control.stderr.contains(LEAK_MARK),
        "loft#688 REGRESSION: with the bridge orphan-free disabled the ownership sweep \
         no longer reclaims the fallback dest, so the orphan is back.\nstderr:\n{}",
        control.stderr
    );

    let _ = fa::remove_dir_all(&root);
}

/// loft#672 — a `boolean` library function whose body compares a field of a
/// RECORD-RETURNING CALL must compile in the auto-cdylib.
///
/// `return node_at().kind == 0` lifts the call into a `__lift_N` temp, which makes the
/// assignment's RHS an `Insert` (preamble + value).  That arm emitted the value raw and
/// returned early, skipping the storage-form coercion every other RHS gets — so a
/// `boolean` (stored as `u8`) was assigned a Rust `bool`: `error[E0308]: expected u8,
/// found bool`, and the library became unconsumable from every `--native` binary.
///
/// Only the cdylib path shows it: a library's own `loft test --native` compiles the
/// function INLINE and passes, so a green library suite never exercised this.  The test
/// therefore runs all three modes — and asserts VALUES, since a cast bug that compiles
/// could still invert a comparison.
///
/// `narrow_widen` covers the other half of the same coercion (a narrow int widening to
/// the `integer` slot), which the early return dropped identically.
// @speed 1.3
#[test]
fn boolean_compare_of_lifted_ref_field_builds_in_cdylib_672() {
    let pid = std::process::id();
    let root = std::env::temp_dir().join(format!("loft_672_{pid}"));
    let _ = fa::remove_dir_all(&root);
    let libdir = root.join("lib");
    write_lib(
        &libdir,
        "liftlib",
        None,
        "pub struct Node { kind: integer, flag: boolean, small: u8 }\n\
         fn node_at() -> Node { return Node { kind: 7, flag: true, small: 200 }; }\n\
         pub fn cmp_false() -> boolean { return node_at().kind == 0; }\n\
         pub fn cmp_true() -> boolean { return node_at().kind == 7; }\n\
         pub fn cmp_ne() -> boolean { return node_at().kind != 0; }\n\
         pub fn cmp_lt() -> boolean { return node_at().kind < 10; }\n\
         pub fn bool_field() -> boolean { return node_at().flag; }\n\
         pub fn negated() -> boolean { return !(node_at().kind == 0); }\n\
         pub fn narrow_widen() -> integer { return node_at().small; }\n\
         pub fn via_local() -> boolean { n = node_at(); return n.kind == 0; }\n",
    );
    let prog = root.join("main.loft");
    fa::write(
        &prog,
        "use liftlib::*;\n\
         fn main() {\n\
         \x20   println(\"{cmp_false()} {cmp_true()} {cmp_ne()} {cmp_lt()}\");\n\
         \x20   println(\"{bool_field()} {negated()} {narrow_widen()} {via_local()}\");\n\
         }\n",
    )
    .unwrap();

    // Hand-computed: kind=7, flag=true, small=200.
    let want = "false true true true\ntrue true 200 false\n";
    for (mode, args) in [
        ("interp", vec!["--interpret"]),
        ("mixed", vec![]),
        ("native", vec!["--native"]),
    ] {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
        cmd.arg("--lib").arg(&libdir).arg(&prog);
        for a in &args {
            cmd.arg(a);
        }
        let out = cmd
            .env("LOFT_NO_CACHE", "1")
            .output()
            .expect("spawn loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            out.status.success(),
            "[{mode}] the library must build — E0308 here is the #672 dropped cast.\
             \nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(
            stdout.contains(want),
            "[{mode}] wrong values\n  want: {want:?}\n  got:  {stdout:?}\n{stderr}"
        );
    }

    let _ = fa::remove_dir_all(&root);
}

/// loft#777 — a **dependency** edit must invalidate every dependent's cdylib.
///
/// *Invariant:* an artifact is stale when any source that CONTRIBUTES to it is
/// newer — not just the source of the package that owns it.
///
/// A cdylib carries its dependencies inlined (`emit_program` emits the export set
/// *and its transitive deps*) and EXPORTS those copies under the same
/// `loft_shared_<name>` symbol the dependency's own cdylib exports.  Whichever
/// library loads first wins the lookup.  So when the freshness question was asked
/// only about the owning package, editing `base` rebuilt `base` correctly while
/// `dep` — whose own sources never change again — kept serving its stale inlined
/// copy of `base`'s function, and won.  Permanently: no later run could clear it,
/// only deleting `native-auto/` by hand.
///
/// It was reported as a consumer-SIZE effect (a 5,900-line program stale where an
/// 8-line one tracked the edit) because you need a second library in the graph,
/// loaded first, before anything can shadow the fresh one — a small consumer that
/// loads the edited library directly was always right.  So this test's shape is
/// the real axis: `dep` is `use`d, `base` is reached only THROUGH it.
///
/// Non-vacuous by construction: the cold run pins the pre-edit answer, so a fix
/// that simply never dispatched native would fail the first assert.
// @speed 3.9
#[test]
fn a_dependency_edit_invalidates_its_dependents_cdylib() {
    dependency_edit_reaches_a_dependent("777", &[]);
}

/// loft#999 — the same guard with the layout probe's RELOCATION forced to fail.
///
/// Before a rebuild, `cached_or_build_shared_cdylib` asks the artifact which type
/// layout it was built for, and it asks a copy set aside from the real path: macOS
/// dyld caches a loaded image by PATH and its `dlclose` is a no-op, so probing the
/// real path and then rebuilding there serves the pre-edit image to the very run
/// that wrote the correct file.
/// The probe used to fall back to asking IN PLACE when that copy could not be made —
/// which restored that behaviour on one unlucky `copy`, and no run afterwards could
/// clear it.  That is the whole of loft#999: this assertion failed on `macos-latest`
/// twice in a fortnight and passed the other eleven times, because how a probe
/// answers must not depend on whether a temp file could be written.
///
/// `LOFT_FORCE_PROBE_RELOCATE_FAIL` makes the relocation fail on every platform, so
/// the luck is gone from the test: the probe must now answer "cannot adopt" and the
/// caller must rebuild.  Verified non-vacuous — with the in-place fallback restored,
/// this fails at round 2 with `left: "2"`, the reported failure verbatim.
// @speed 5.3
#[test]
fn a_dependency_edit_reaches_a_dependent_when_the_layout_probe_cannot_relocate() {
    dependency_edit_reaches_a_dependent("999", &[("LOFT_FORCE_PROBE_RELOCATE_FAIL", "1")]);
}

/// The shared body: `dep` inlines `base`, the consumer names only `dep`, `base` is
/// edited, and every run afterwards must report the edited rule.  `tag` keeps the
/// two callers' scratch trees apart (same pid, one test binary); `extra_env` is
/// applied to every `loft` invocation.
fn dependency_edit_reaches_a_dependent(tag: &str, extra_env: &[(&str, &str)]) {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable (needs the native build)");
        return;
    }
    let pid = std::process::id();
    let root = std::env::temp_dir().join(format!("loft_{tag}_dep_{pid}"));
    let _ = fa::remove_dir_all(&root);
    let libdir = root.join("lib");

    let write_pkg = |name: &str, body: &str| {
        let pkg = libdir.join(name);
        fa::create_dir_all(pkg.join("src")).unwrap();
        fa::write(
            pkg.join("loft.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nloft = \">=0.8\"\n\
                 [library]\nentry = \"src/{name}.loft\"\n"
            ),
        )
        .unwrap();
        fa::write(pkg.join("src").join(format!("{name}.loft")), body).unwrap();
    };

    // `base` holds the rule under edit; `dep` calls it, so `dep`'s cdylib inlines it.
    let base_src = |limit: i32| {
        format!(
            "pub fn deep(n: integer) -> integer {{ if n > {limit} {{ return 2 }} return 0; }}\n"
        )
    };
    write_pkg("base", &base_src(2));
    write_pkg(
        "dep",
        "pub use base::*;\npub fn check(n: integer) -> integer { return deep(n); }\n",
    );

    // The consumer names ONLY `dep`, so `base` is reached transitively — and `dep`
    // is registered (and dlopened) first, which is what lets its copy shadow.
    let prog = root.join("main.loft");
    fa::write(
        &prog,
        "use dep::*;\nfn main() { println(\"{check(3)}\"); }\n",
    )
    .unwrap();

    let run = || -> String {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
        cmd.arg("--interpret")
            .arg("--lib")
            .arg(&libdir)
            .arg(&prog)
            .env("LOFT_NO_CACHE", "1");
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("spawn loft binary");
        assert!(
            out.status.success(),
            "run failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };

    // Cold: 3 > 2, so the rule fires.
    assert_eq!(run(), "2", "cold run must report the pre-edit rule");

    // Edit ONLY `base`. `dep`'s own sources are untouched from here on, which is
    // exactly why its artifact used to look fresh forever.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    write_pkg("base", &base_src(9));

    // Every run from here must report the edited rule: the first interprets
    // (dev-interpret-on-edit), a later one rebuilds and dispatches native again.
    // Run it three times so a fix that only worked while the artifact was missing,
    // or only for one run, still fails.
    for round in 1..=3 {
        assert_eq!(
            run(),
            "0",
            "round {round}: a `base` edit must reach a consumer that only names `dep`"
        );
    }

    let _ = fa::remove_dir_all(&root);
}

/// loft#999 — an artifact that cannot be moved off its path is REBUILT, not adopted.
///
/// This is the policy behind the round-2 failure, asserted directly and on every
/// platform: the layout probe must never `dlopen` a path its caller may replace, so
/// when it cannot link or copy the artifact aside it has to answer "cannot adopt".
/// The alternative the fallback used to take — ask in place — is correct on Linux
/// and silently serves a stale image on macOS, which is exactly the property a
/// cross-platform test cannot see and a nightly notices twice a fortnight.
///
/// Non-vacuous by construction: the third run, with the fault injection removed,
/// must ADOPT the same artifact.  Without that cell, "the artifact was rebuilt"
/// would also pass on a loft that rebuilds unconditionally.
// @speed 1.6
#[test]
fn an_unrelocatable_layout_probe_rebuilds_instead_of_adopting() {
    if Command::new("rustc").arg("--version").output().is_err() {
        eprintln!("skip: rustc unavailable (needs the native build)");
        return;
    }
    let pid = std::process::id();
    let root = std::env::temp_dir().join(format!("loft_999_probe_{pid}"));
    let _ = fa::remove_dir_all(&root);
    let pkg = root.join("lib").join("solo");
    fa::create_dir_all(pkg.join("src")).unwrap();
    fa::write(
        pkg.join("loft.toml"),
        "[package]\nname = \"solo\"\nversion = \"0.1.0\"\nloft = \">=0.8\"\n\
         [library]\nentry = \"src/solo.loft\"\n",
    )
    .unwrap();
    fa::write(
        pkg.join("src").join("solo.loft"),
        "pub fn seven() -> integer { return 7; }\n",
    )
    .unwrap();
    let prog = root.join("main.loft");
    fa::write(
        &prog,
        "use solo::*;\nfn main() { println(\"{seven()}\"); }\n",
    )
    .unwrap();

    let run = |env: &[(&str, &str)]| -> (String, String) {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
        cmd.arg("--interpret")
            .arg("--lib")
            .arg(root.join("lib"))
            .arg(&prog)
            .env("LOFT_NO_CACHE", "1");
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("spawn loft binary");
        assert!(
            out.status.success(),
            "run failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        (
            String::from_utf8_lossy(&out.stdout).trim().to_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };

    // The artifact `solo`'s cdylib was published as, and when.
    let auto = pkg.join("native-auto");
    let artifact_mtime = || -> std::time::SystemTime {
        let ext = if cfg!(target_os = "windows") {
            "dll"
        } else if cfg!(target_os = "macos") {
            "dylib"
        } else {
            "so"
        };
        let mut newest: Option<std::time::SystemTime> = None;
        for e in std::fs::read_dir(&auto)
            .expect("native-auto must exist")
            .flatten()
        {
            if e.path().extension().is_some_and(|x| x == ext)
                && let Ok(m) = e.metadata().and_then(|m| m.modified())
                && newest.is_none_or(|n| m > n)
            {
                newest = Some(m);
            }
        }
        newest.expect("the cold run must have built an artifact")
    };

    // Cold: builds the artifact and answers from it.
    assert_eq!(run(&[]).0, "7", "cold run");
    let cold = artifact_mtime();

    // Unchanged sources, but the probe cannot be relocated: the artifact must NOT
    // be adopted — and the reason must be said, because an unexplained rebuild is
    // recoverable while an unexplained stale answer is not.
    let (out, err) = run(&[("LOFT_FORCE_PROBE_RELOCATE_FAIL", "1")]);
    assert_eq!(
        out, "7",
        "an unprobeable artifact must still give the right answer"
    );
    assert!(
        err.contains("to read its type layout"),
        "the refusal to adopt must name itself; stderr was:\n{err}"
    );
    let forced = artifact_mtime();
    assert!(
        forced > cold,
        "an artifact the probe cannot move off its path must be REBUILT, not adopted          (mtime unchanged: {cold:?})"
    );

    // Control: without the injection the same artifact IS adopted, so the assertion
    // above is about the probe and not about loft rebuilding on every run.
    assert_eq!(run(&[]).0, "7", "control run");
    assert_eq!(
        artifact_mtime(),
        forced,
        "a fresh artifact must be adopted, not rebuilt"
    );

    let _ = fa::remove_dir_all(&root);
}
