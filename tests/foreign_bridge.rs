// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @PLN203 C-gap: the bridge-answer reads differ on the C — its `--native` runs as `--native-rust` until then.
//! @PLN174 F5 — a LIBRARY's bytes as a foreign store: the fixture cdylib
//! (`tests/lib/native_pkg`) answers a `vector<u8>` through
//! `LoftStore::foreign_vector_from_owned`, and the cell file reads it beside the copying
//! answer on both backends, under the hoist verifier, the strict-store, poison and leak
//! switches, and the copy A/B (`LOFT_NO_FOREIGN_VIEW=1`, which must print the same lines).
//! The bridge declares its foreign answers `-> const vector<u8>` (`(Const-Foreign)`, @C139),
//! so a WRITE into the answer, or into a foreign answer beside a vector argument, is refused
//! when the program is COMPILED, with the copy advice; asserted on the spawned binary.  The fixture cdylib must be built (`cd tests/lib/native_pkg/native &&
//! cargo build --release`); the tests skip when it is absent, as `native_loader.rs` does.
use loft::file_access as fa;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

const PKG: &str = "tests/lib/native_pkg";
const CELLS: &str = "tests/lib/native_pkg/tests/174-foreign-bridge.loft";

fn fixture_built() -> bool {
    let so = loft::native_lib::platform_cdylib_name("loft_native_test");
    let p = std::path::Path::new(PKG)
        .join("native/target/release")
        .join(so);
    if !fa::exists(&p) {
        eprintln!(
            "skipping: fixture cdylib not built — run: cd {PKG}/native && cargo build --release"
        );
        return false;
    }
    true
}

const WRITE_REFUSED: &str = "\
use native_pkg::*;
fn main() {
  f = ext_make_bytes_foreign(4);
  assert(f[1] == 1, \"read {f[1]}\");
  f[1] = 9;
  println(\"reached: the write landed ({f[1]})\");
}
";

/// A grow of a foreign answer given beside a vector ARGUMENT (the answer took a store of
/// its own, since the argument pinned the bridge's store).
const GROW_REFUSED: &str = "\
use native_pkg::*;
fn main() {
  c = ext_make_bytes(4);
  r = ext_reverse_foreign(c);
  assert(r[0] == 3, \"read {r[0]}\");
  r += [7];
  println(\"reached: the grow landed ({len(r)})\");
}
";

/// Run `src` (the cell file, or a program written to a scratch dir) on `backend` with
/// `envs`, from a scratch directory of its own.  Answers `(exit ok, stdout, stdout + stderr)`.
fn run(
    tag: &str,
    backend: &str,
    src: &str,
    inline: bool,
    envs: &[(&str, &str)],
) -> (bool, String, String) {
    let root = std::env::temp_dir().join(format!(
        "loft_174_bridge_{tag}_{}_{}",
        backend.trim_start_matches('-'),
        std::process::id()
    ));
    fa::create_dir_all(&root).expect("scratch dir");
    let cwd = std::env::current_dir().expect("cwd");
    let path = if inline {
        let p = root.join("prog.loft");
        fa::write(&p, src).expect("write program");
        p
    } else {
        cwd.join(src)
    };
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.arg(backend)
        .arg("--lib")
        .arg(cwd.join(PKG))
        .arg(&path)
        .current_dir(&root)
        .env("LOFT_TIMEOUT", "300")
        .env("RUST_BACKTRACE", "0");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run loft");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let text = format!("{stdout}{}", String::from_utf8_lossy(&out.stderr));
    let _ = fa::remove_dir_all(&root);
    (out.status.success(), stdout, text)
}

#[test]
fn every_read_of_a_bridge_answer_matches_the_copying_bridge_on_both_backends() {
    if !fixture_built() {
        return;
    }
    for backend in ["--interpret", "--native-rust"] {
        for (tag, envs) in [
            ("plain", vec![]),
            ("verify", vec![("LOFT_HOIST_VERIFY", "1")]),
            ("strict", vec![("LOFT_STRICT_STORES", "1")]),
            ("poison", vec![("LOFT_POISON", "1")]),
            ("leak", vec![("LOFT_STORES", "warn")]),
        ] {
            let (ok, _, out) = run(tag, backend, CELLS, false, &envs);
            assert!(
                ok && out.contains("foreign bridge ok"),
                "{backend} {tag}: the cells must pass:\n{out}"
            );
            assert!(
                !out.to_lowercase().contains("leak"),
                "{backend} {tag}: every block and view must be released:\n{out}"
            );
        }
    }
}

#[test]
fn the_view_and_the_copy_print_the_same_lines_on_both_backends() {
    if !fixture_built() {
        return;
    }
    for backend in ["--interpret", "--native-rust"] {
        let (ok, with, _) = run("ab_view", backend, CELLS, false, &[]);
        let (ok2, without, _) = run(
            "ab_copy",
            backend,
            CELLS,
            false,
            &[("LOFT_NO_FOREIGN_VIEW", "1")],
        );
        assert!(ok && ok2, "{backend}: both forms must pass");
        assert_eq!(with, without, "{backend}: the view and the copy must agree");
    }
}

#[test]
fn a_write_into_a_bridge_answer_is_refused_at_compile_time_with_the_copy_advice() {
    if !fixture_built() {
        return;
    }
    for (tag, prog, landed) in [
        ("refused", WRITE_REFUSED, "reached: the write landed"),
        ("grow_refused", GROW_REFUSED, "reached: the grow landed"),
    ] {
        for backend in ["--interpret", "--native-rust"] {
            let (ok, _, out) = run(tag, backend, prog, true, &[]);
            assert!(!ok, "{backend} {tag}: the write must be refused:\n{out}");
            assert!(
                out.contains("it holds read-only data the program does not own")
                    && out.contains("copy it first"),
                "{backend} {tag}: the compile-time refusal with the copy cure:\n{out}"
            );
            assert!(
                !out.contains("write to bytes the program does not own"),
                "{backend} {tag}: refused before the program runs, not at run time:\n{out}"
            );
            assert!(
                !out.contains(landed),
                "{backend} {tag}: the write must not land:\n{out}"
            );
            assert!(
                !out.contains("out of bounds"),
                "{backend} {tag}: a refused write is not a corrupt reference:\n{out}"
            );
        }
    }
}

/// A library answering a NARROW integer (`i32` at the C-ABI) answers its null as `i32::MIN`;
/// every backend must read it as null, as the interpreter's bridge (`loft_ffi::widen_i32`)
/// does.  The rustc backend used to widen it plainly: `b` read -2147483648, not null.
const NARROW_NULL: &str = "\
use native_pkg::*;
fn main() {
  a = ext_narrow(21);
  b = ext_narrow(-1);
  println(\"a={a} b={b} b-null={!b} sum={a + (b ?? 1000)}\");
}
";

/// A MAPPED file handed to a library (`Stores::bridge_args`): the library reads a vector
/// through the one store it is handed, so the foreign bytes are copied into a record of that
/// store for the call.  The C backend handed the foreign record id itself, which the library
/// read out of bounds (the program died with no output).  c1: the mapped vector alone (a store
/// is minted); c2: a foreign answer from it; c3: an owned vector beside it; c4: many calls.
const MAPPED_ARGUMENT: &str = "\
use native_pkg::*;
fn show(v: const vector<u8>) -> text { s = \"\"; for b in v { s += \"{b},\"; } s }
fn main() {
  f = \"mapped_arg_{ticks()}.tmp\";
  src: vector<u8> = [10, 20, 30, 40, 250];
  assert(write_bytes(f, src), \"write\");
  m = file_map(f) ?? [];
  r1 = ext_reverse_owned(m);
  r2 = ext_reverse_foreign(m);
  r3 = ext_reverse_owned(src);
  println(\"c1 {show(r1)} {len(r1)} c2 {show(r2)} {len(r2)} c3 {show(r3)} m {show(m)} {len(m)}\");
  for i in 0..300 { r = ext_reverse_owned(m); assert(len(r) == 5 && r[0] == 250, \"c4 {i}\"); }
  println(\"c4 ok\");
  delete(f);
}
";

/// The fixture is a Rust crate, which a C build never loads (`native_build::c_target`); the C
/// half of both cells runs against a C library in `tests/ffi_c_library.rs`.
#[test]
fn a_narrow_null_answer_and_a_mapped_argument_cross_a_library_call_on_every_backend() {
    if !fixture_built() {
        return;
    }
    for backend in ["--interpret", "--native-rust"] {
        let (ok, out, all) = run("narrow", backend, NARROW_NULL, true, &[]);
        assert!(
            ok && out.contains("a=42 b=null b-null=true sum=1042"),
            "{backend}: a narrow null must read as null:\n{all}"
        );
        for (tag, envs) in [
            ("mapped", vec![]),
            ("mapped_poison", vec![("LOFT_POISON", "1")]),
            ("mapped_leak", vec![("LOFT_STORES", "warn")]),
        ] {
            let (ok, out, all) = run(tag, backend, MAPPED_ARGUMENT, true, &envs);
            assert!(
                ok && out.contains(
                    "c1 250,40,30,20,10, 5 c2 250,40,30,20,10, 5 c3 250,40,30,20,10, m 10,20,30,40,250, 5"
                ) && out.contains("c4 ok"),
                "{backend} {tag}: a mapped argument must reach the library whole:\n{all}"
            );
            assert!(
                !all.to_lowercase().contains("leak"),
                "{backend} {tag}: the copies must be released:\n{all}"
            );
        }
    }
}
