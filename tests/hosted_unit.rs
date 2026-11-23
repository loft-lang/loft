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
//!   A second SABOTAGE (2026-11-22): with `hosted::bridge` carrying an iterator again, the unit
//!   answers `upto` from its own coroutine table.  WITNESS: "assertion failed: upto 0" and a
//!   refused free of store #65534.
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
        cmd.env("LOFT_NO_RUSTC", "1")
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
///   A SABOTAGE (2026-11-22): with a unit's text answers one byte short (`loft_la_set_text`),
///   the run stops at `label_of`.  WITNESS: "the answer is \"hosted:4\" interpreted,
///   \"hosted:\" compiled".
///   A SABOTAGE (2026-11-22): with the file layer struck from what reaches outside the stores
///   (`hosted_verify::reaches_outside`), `append_word` runs twice.  WITNESS: "file 16 bytes".
///   A SABOTAGE (2026-11-22): with the comparison by value struck (`hosted_verify.rs`), the
///   sound run stops at `rename`.  WITNESS: "`rename` leaves another world".
///   A SABOTAGE (2026-11-23): with a text answer written into whatever destination the shared
///   field holds after the call (`extensions.rs`, `bridge_text_dest.take()`), a nested call
///   inside the interpreted twin takes the outer call's.  WITNESS: "`to_text` leaves another
///   world compiled than interpreted: the answer is \"\" interpreted".
///   A SABOTAGE (2026-11-23): with every text work slot of a bridge call handed one buffer
///   (`hosted::bridge`), `framed`'s local is emptied while its answer is built.  WITNESS: "the
///   answer is \"ab:4\\n#4\" interpreted, \"\\n#0\" compiled".
///   A SABOTAGE (2026-11-23): with a constant's store left holding an earlier store's words
///   past the constant's (`loft_const_store`), the unit's first call walks them as blocks.
///   WITNESS: "SIGBUS caught" (`hostedlib_main.loft`'s `churn`).
///   A SABOTAGE (2026-11-23): with `exp` a power of E again (`default/01_code.loft`), the unit's
///   C `pow` and the interpreter answer `decay` apart.  WITNESS: "the answer's bits are
///   0x3f98a9fb43afe516 interpreted, 0x3f98a9fb43afe518 compiled".
///   A SABOTAGE (2026-11-23): with every leaf of a tuple argument read from its first slot
///   (`hosted::bridge`), `spread` answers wrong.  WITNESS: "assertion failed: spread".
///   A SABOTAGE (2026-11-23): with a `&` argument's slot not written back after the call
///   (`hosted::bridge`).  WITNESS: "the `&` argument `x` holds 2a00000000000000 interpreted,
///   2800000000000000 compiled", and without the verify "assertion failed: bump 40".
///   A SABOTAGE (2026-11-23): with the twin started from the slot the compiled call left
///   (`hosted_verify.rs`, the restore struck), `bump` adds twice.  WITNESS: "holds
///   2c00000000000000 interpreted".
///   A SABOTAGE (2026-11-23): with a function value the program hands in left untagged
///   (`loft_la_fn`), the unit looks the program's number up among its own functions.
///   WITNESS: "`twice_by` … the answer's bits are 0x8 interpreted, 0x8000000000000000 compiled",
///   and without the verify "assertion failed: twice_by null".
///   A SABOTAGE (2026-11-23): with a unit parking stores across calls (`loft_host_enter`
///   leaving `cx->no_park` off), `styled`'s lifted record is never freed.  WITNESS:
///   "[strict-store] NEVER FREED: 1 stores not freed at program exit: kt=100 Bag×1".
///   A SABOTAGE (2026-11-23): with the unit's generator step answering "ended" after every
///   step (`loft_shared_gen_next`), `upto` yields nothing.  WITNESS: "assertion failed: upto 0".
///   And a SABOTAGE (2026-11-22): with the verify going on in the compiled COPY of the world
///   (`hosted_verify.rs` before the twin ran on the copy), the caller's loop text points into
///   a buffer the swap freed.  WITNESS: the "nested views" assertion of `hostedlib_main.loft`.
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
    // `rename` writes a text the interpreter writes in place and the open unit writes anew:
    // the same value placed otherwise, accepted by value (`@FR-H-TextReplace`).
    assert!(
        err.contains("alike in value, placed otherwise"),
        "the verify compares a store by value when its bytes differ: {err}"
    );
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
    assert!(
        ok,
        "the shifted program adopts the unit and verifies: {err}"
    );
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
///
/// @falsified-at: none — a SABOTAGE (2026-11-22): with `loft_rt.h`'s image mint writing
///   `s->known = tp;` again, the scan names that line.  CHANNEL: the assertion.  WITNESS:
///   "src/c/loft_rt.h:2694: s->known = tp;".
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

/// `@PLN206 P5` — a library's `[c] shim` is one image in the process: the hosted unit links the
/// shim's shared library, the one the interpreter opened for the parse, so a shim's state (a
/// counter, a generator, a device table) is one whichever side calls it.  A copy linked in from
/// the shim's archive is a second state; which copy an interpreted `#c` call then reaches is the
/// platform's lookup order, so the writer's choice is what this holds: the unit names the
/// shared library and carries no archive.  `bump` runs in the unit and `label` interprets.
///
/// @falsified-at: none — a SABOTAGE (2026-11-22): with the hosted branch of `c_libraries`
///   switched off (`src/c/mod.rs`), the unit links `libhostedshim_shim_….a` whole.  CHANNEL:
///   the unit's link lines.  WITNESS: no link line names the shim's shared library.  HOLDS:
///   `9 count 9` (this platform's lookup reached the unit's copy).
#[test]
fn a_hosted_unit_shares_its_shim_with_the_interpreter() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let lib = std::env::temp_dir().join(format!("loft-hosted-unit-shim-{}", std::process::id()));
    let _ = fa::remove_dir_all(&lib);
    let pkg = lib.join("hostedshim");
    for f in ["loft.toml", "src/hostedshim.loft", "c/hostedshim.c"] {
        let to = pkg.join(f);
        fa::create_dir_all(to.parent().expect("parent")).expect("package dir");
        let text = fa::read_to_string(root.join("tests/lib/hostedshim").join(f)).expect("read");
        fa::write(to, text).expect("write");
    }
    let out = harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg("--interpret")
        .arg("--lib")
        .arg(&lib)
        .arg(root.join("tests/lib/hostedshim_main.loft"))
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_NO_RUSTC", "1")
        .output()
        .expect("run loft");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let unit: String = fa::read_dir(pkg.join("native-auto"))
        .unwrap_or_default()
        .iter()
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.starts_with("loft_auto") && n.ends_with(".c"))
        })
        .map(|p| fa::read_to_string(p.native()).unwrap_or_default())
        .collect();
    let _ = fa::remove_dir_all(&lib);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout, "9 count 9\n");
    let links: Vec<&str> = unit
        .lines()
        .filter_map(|l| l.strip_prefix("/* link: ")?.strip_suffix(" */"))
        .collect();
    assert!(
        links
            .iter()
            .any(|l| l.contains("hostedshim_shim") && !l.ends_with(".a")),
        "the unit links the shim's shared library: {links:?}"
    );
    assert!(
        !unit.contains("/* shim-archive:") && !links.iter().any(|l| l.ends_with(".a")),
        "the unit carries no copy of the shim: {links:?}"
    );
}

/// `@PLN206 P5` — the verify's interpreted twin answers where an interpreted caller reads: a
/// library of two modules (`tests/lib/hostedmods`, as `mesh3d` is) whose `quad` builds each
/// corner from `geo::v3` answers handed to `vtx`.  The twin interprets those calls too and hands
/// each the caller's hidden buffer (`State::call_back_into`); answered into a store of its own,
/// the interpreted caller read an unwritten buffer.
///
/// @falsified-at: none — a SABOTAGE (2026-11-22): with the twin handed no buffers
///   (`twin_call` passing `&dests[..0]`), the sound run stops.  CHANNEL: the exit status and
///   the message.  WITNESS: "`quad` leaves another world compiled than interpreted: the answer
///   is {name:\"quad\",vs:[{pos:{x:0,y:0,z:0}".  HOLDS: the compiled answer.
#[test]
fn the_verify_twin_answers_where_an_interpreted_caller_reads() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let lib = std::env::temp_dir().join(format!("loft-hosted-unit-mods-{}", std::process::id()));
    let _ = fa::remove_dir_all(&lib);
    let pkg = lib.join("hostedmods");
    for f in [
        "loft.toml",
        "src/hostedmods.loft",
        "src/geo.loft",
        "src/shape.loft",
    ] {
        let to = pkg.join(f);
        fa::create_dir_all(to.parent().expect("parent")).expect("package dir");
        let text = fa::read_to_string(root.join("tests/lib/hostedmods").join(f)).expect("read");
        fa::write(to, text).expect("write");
    }
    let out = harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg("--interpret")
        .arg("--lib")
        .arg(&lib)
        .arg(root.join("tests/lib/hostedmods_main.loft"))
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_NO_RUSTC", "1")
        .env("LOFT_POISON_CLAIM", "1")
        .env("LOFT_HOSTED_VERIFY", "1")
        .output()
        .expect("run loft");
    let _ = fa::remove_dir_all(&lib);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "quad -0.5 0.5\n");
    assert!(
        err.contains("call(s) compiled and interpreted alike"),
        "{err}"
    );
}

/// `@PLN206 P5` — a library adopted with no C compiler: what one run built (the hosted unit, the
/// `[c] shim`) serves a run that cannot run `cc`, as a prebuilt library does.  A shim's name
/// carries the compiler's identity, which a run without one cannot compute: the shim is found by
/// its sources key (`c_shim::source_key`, the `.src` beside it).
///
/// @falsified-at: none — a SABOTAGE (2026-11-22): with the `.src` beside a built shim not written
///   (`c_shim::build`), the run without a compiler stops at the parse.  CHANNEL: the exit status.
///   WITNESS: "`[c] shim` needs a C compiler and `cc` could not be run".
#[test]
fn a_library_built_once_is_adopted_with_no_c_compiler() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let lib = std::env::temp_dir().join(format!("loft-hosted-unit-nocc-{}", std::process::id()));
    let _ = fa::remove_dir_all(&lib);
    let pkg = lib.join("hostedshim");
    for f in ["loft.toml", "src/hostedshim.loft", "c/hostedshim.c"] {
        let to = pkg.join(f);
        fa::create_dir_all(to.parent().expect("parent")).expect("package dir");
        let text = fa::read_to_string(root.join("tests/lib/hostedshim").join(f)).expect("read");
        fa::write(to, text).expect("write");
    }
    let empty = lib.join("no-cc");
    fa::create_dir_all(&empty).expect("empty dir");
    let run = |path: Option<&std::path::Path>| {
        let mut cmd = harness_command(env!("CARGO_BIN_EXE_loft"));
        cmd.arg("--interpret")
            .arg("--lib")
            .arg(&lib)
            .arg(root.join("tests/lib/hostedshim_main.loft"))
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_NO_RUSTC", "1")
            .env("LOFT_REQUIRE_NATIVE", "1");
        if let Some(p) = path {
            cmd.env("PATH", p);
        }
        let out = cmd.output().expect("run loft");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let (ok, out, err) = run(None);
    assert!(ok, "the run that builds: {err}");
    assert_eq!(out, "9 count 9\n");
    let (ok, out, err) = run(Some(&empty));
    let _ = fa::remove_dir_all(&lib);
    assert!(
        ok,
        "the run without a compiler adopts what was built: {err}"
    );
    assert_eq!(out, "9 count 9\n");
}

/// `@PLN206 P5` — a hosted unit is the library's, the same text wherever it is built: generated
/// from the library alone (`native_artifact::parse_library_alone`) and naming its sources from
/// the package's parent directory (`Cx::source_name`), so a unit built by a library's CI is the
/// unit a user's machine would build.  The same package built by `loft build-native` in two
/// directories writes byte-identical C.
///
/// @falsified-at: none — a SABOTAGE (2026-11-22): with `Cx::source_name` answering the file as
///   it was found, the two units differ in every fault site and frame.  CHANNEL: the comparison
///   of the two `.c` files.  WITNESS: "the units differ".
#[test]
fn a_hosted_unit_is_the_same_text_wherever_it_is_built() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let build = |tag: &str| {
        let lib = std::env::temp_dir().join(format!(
            "loft-hosted-unit-text-{tag}-{}",
            std::process::id()
        ));
        let _ = fa::remove_dir_all(&lib);
        let pkg = lib.join("hostedlib");
        fa::create_dir_all(pkg.join("src")).expect("package dir");
        for f in ["loft.toml", "src/hostedlib.loft"] {
            let text = fa::read_to_string(root.join("tests/lib/hostedlib").join(f)).expect("read");
            fa::write(pkg.join(f), text).expect("write");
        }
        let out = harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg("build-native")
            .arg(&pkg)
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_NO_RUSTC", "1")
            .output()
            .expect("run loft build-native");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let report = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(report.contains("backend: c"), "a C unit says so: {report}");
        let unit: String = fa::read_dir(pkg.join("native-auto"))
            .unwrap_or_default()
            .iter()
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.starts_with("loft_auto") && n.ends_with(".c"))
            })
            .map(|p| fa::read_to_string(p.native()).unwrap_or_default())
            .collect();
        let _ = fa::remove_dir_all(&lib);
        unit
    };
    let (a, b) = (build("a"), build("b"));
    assert!(!a.is_empty(), "the unit was written");
    assert!(a == b, "the units differ");
}

/// A library's own function named like a stdlib native is the library's: a unit that reaches
/// `crt::file_map` (the self-hosted front end's library) reaches no mapped file, so it is built
/// without the foreign-store layer a hosted unit cannot carry.  The define is the stdlib's
/// `file_map` only (`c::unit_defines`, `Definition::is_stdlib`).
///
/// @falsified-at: none — a SABOTAGE (2026-11-23): with the test by name alone again
///   (`unit_defines`, `def.is_stdlib()` struck), the unit fails `cc`.  CHANNEL: the exit status.
///   WITNESS: "a hosted unit keeps no store field of its own".
#[test]
fn a_library_function_named_like_a_stdlib_native_builds_as_a_hosted_unit() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let lib = std::env::temp_dir().join(format!("loft-hosted-unit-name-{}", std::process::id()));
    let _ = fa::remove_dir_all(&lib);
    let pkg = lib.join("maplib");
    fa::create_dir_all(pkg.join("src")).expect("package dir");
    fa::write(
        pkg.join("loft.toml"),
        "[package]\nname    = \"maplib\"\nversion = \"0.1.0\"\nloft    = \">=0.8\"\n\n\
         [library]\nentry   = \"src/maplib.loft\"\n",
    )
    .expect("write manifest");
    fa::write(
        pkg.join("src/maplib.loft"),
        "pub fn file_map(out: vector<integer>, n: integer) -> integer {\n  \
         for i in 0..n { out += [i * 3]; }\n  out.len()\n}\n",
    )
    .expect("write library");
    let prog = lib.join("main.loft");
    fa::write(
        &prog,
        "use maplib;\n\nfn main() {\n  v: vector<integer> = [];\n  \
         n = maplib::file_map(v, 5);\n  println(\"{n} {v}\");\n}\n",
    )
    .expect("write program");
    let run = |hosted: bool| {
        let mut cmd = harness_command(env!("CARGO_BIN_EXE_loft"));
        cmd.arg("--interpret")
            .arg("--lib")
            .arg(&lib)
            .arg(&prog)
            .env("LOFT_NO_CACHE", "1");
        if hosted {
            cmd.env("LOFT_LIB_BACKEND", "c")
                .env("LOFT_NO_RUSTC", "1")
                .env("LOFT_REQUIRE_NATIVE", "1");
        } else {
            cmd.env("LOFT_NO_NATIVE_LIBS", "1");
        }
        let out = cmd.output().expect("run loft");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let (ok, want, err) = run(false);
    assert!(ok, "interpreted: {err}");
    assert_eq!(want, "5 [0,3,6,9,12]\n");
    let (ok, got, err) = run(true);
    assert!(ok, "hosted: {err}");
    assert_eq!(got, want, "the hosted unit answers as the interpreter does");
    let _ = fa::remove_dir_all(&lib);
}
