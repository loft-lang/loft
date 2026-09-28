// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN174 F5 — a LIBRARY's bytes as a foreign store: the fixture cdylib
//! (`tests/lib/native_pkg`) answers a `vector<u8>` through
//! `LoftStore::foreign_vector_from_owned`, and the cell file reads it beside the copying
//! answer on both backends, under the hoist verifier, the strict-store, poison and leak
//! switches, and the copy A/B (`LOFT_NO_FOREIGN_VIEW=1`, which must print the same lines).
//! A WRITE into the answer, or into a foreign answer beside a vector argument, halts with the
//! foreign store's own advice; the halt is `report_and_exit`, so it is asserted on the
//! spawned binary.  The fixture cdylib must be built (`cd tests/lib/native_pkg/native &&
//! cargo build --release`); the tests skip when it is absent, as `native_loader.rs` does.
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

const PKG: &str = "tests/lib/native_pkg";
const CELLS: &str = "tests/lib/native_pkg/tests/174-foreign-bridge.loft";

fn fixture_built() -> bool {
    let so = if cfg!(target_os = "macos") {
        "libloft_native_test.dylib"
    } else if cfg!(windows) {
        "loft_native_test.dll"
    } else {
        "libloft_native_test.so"
    };
    let p = std::path::Path::new(PKG)
        .join("native/target/release")
        .join(so);
    if !p.exists() {
        eprintln!(
            "skipping: fixture cdylib not built — run: cd {PKG}/native && cargo build --release"
        );
        return false;
    }
    true
}

const WRITE_REFUSED: &str = "\
use native_pkg;
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
use native_pkg;
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
    std::fs::create_dir_all(&root).expect("scratch dir");
    let cwd = std::env::current_dir().expect("cwd");
    let path = if inline {
        let p = root.join("prog.loft");
        std::fs::write(&p, src).expect("write program");
        p
    } else {
        cwd.join(src)
    };
    let mut cmd = Command::new(loft_bin());
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
    let _ = std::fs::remove_dir_all(&root);
    (out.status.success(), stdout, text)
}

#[test]
fn every_read_of_a_bridge_answer_matches_the_copying_bridge_on_both_backends() {
    if !fixture_built() {
        return;
    }
    for backend in ["--interpret", "--native"] {
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
    for backend in ["--interpret", "--native"] {
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
fn a_write_into_a_bridge_answer_halts_with_the_copy_first_advice_on_both_backends() {
    if !fixture_built() {
        return;
    }
    for (tag, prog, landed) in [
        ("refused", WRITE_REFUSED, "reached: the write landed"),
        ("grow_refused", GROW_REFUSED, "reached: the grow landed"),
    ] {
        for backend in ["--interpret", "--native"] {
            let (ok, _, out) = run(tag, backend, prog, true, &[]);
            assert!(
                !ok,
                "{backend} {tag}: the write must halt the program:\n{out}"
            );
            assert!(
                out.contains("write to bytes the program does not own")
                    && out.contains("copy them first"),
                "{backend} {tag}: the halt must be the foreign store's own advice:\n{out}"
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
