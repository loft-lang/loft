// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN174 F2 + F4b — a FOREIGN store at the program level: `file_map(path)` serves a file's
//! bytes through the store contract with no copy, a slice of it bound to a local is a VIEW
//! of the mapping (no copy either), every read op answers what the copying `read_bytes`
//! answers (the two cell files, on both backends and under the hoist verifier, the
//! strict-store and the poison switches), the view and its copy print the same lines
//! (`LOFT_NO_FOREIGN_VIEW=1`, the A/B), and a WRITE into the mapped bytes or into a view of
//! them is refused when the program is COMPILED, with the advice to copy first: the mapping
//! is foreign data, value-const (`file_map` answers `const vector<u8>`, `(Const-Foreign)`,
//! @C139), and so is a view of it.  Asserted on the spawned binary, so the refusal is what an
//! author sees.  The EMISSION pin at the end says which binds take the view op.
use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

const READS: &str = "tests/scripts/174-foreign-file.loft";
const VIEWS: &str = "tests/scripts/174-foreign-view.loft";

const WRITE_REFUSED: &str = "\
fn wr_path() -> text { return \"loft_174_refused.tmp\"; }
fn main() {
  assert(write_bytes(wr_path(), [1, 2, 3, 4]), \"write\");
  m = file_map(wr_path()) ?? [];
  assert(len(m) == 4 && m[1] == 2, \"mapped {len(m)}\");
  m[1] = 9;
  println(\"reached: the write landed ({m[1]})\");
}
";

/// A write INTO a view: an element store, and a grow.  With the view off (the copy) both
/// land, and the program prints the copy's values.
const VIEW_WRITE_REFUSED: &str = "\
fn vw_path() -> text { return \"loft_174_view_refused.tmp\"; }
fn main() {
  assert(write_bytes(vw_path(), [1, 2, 3, 4, 5, 6]), \"write\");
  m = file_map(vw_path()) ?? [];
  s = m[1..4];
  assert(len(s) == 3 && s[0] == 2, \"the slice {len(s)}\");
  s[0] = 9;
  s += [7];
  println(\"reached: the writes landed ({s[0]} {len(s)})\");
}
";

/// The emitted-IR probe: which binds may take a view.
const PROBE: &str = "\
fn fresh(v: vector<u8>) -> integer { s = v[1..3]; len(s) }
fn rebound(v: vector<u8>) -> integer { s = v[1..3]; s = v[2..4]; len(s) }
fn appended(v: vector<u8>) -> integer { s = v[1..3]; s += v[2..4]; len(s) }
fn returned(v: vector<u8>) -> vector<u8> { s = v[1..3]; s }
fn argument(v: vector<u8>) -> integer { v = v[1..3]; len(v) }
fn main() {
  v: vector<u8> = [1, 2, 3, 4, 5];
  println(\"{fresh(v)} {rebound(v)} {appended(v)} {len(returned(v))} {argument(v)}\");
}
";

/// Run `src` (a file path, or a program written to a scratch dir) on `backend` with
/// `envs`, from a scratch directory of its own so the files the program writes land
/// nowhere shared.  Answers `(exit ok, stdout, stdout + stderr)`.
fn run(
    tag: &str,
    backend: &str,
    src: &str,
    inline: bool,
    envs: &[(&str, &str)],
) -> (bool, String, String) {
    let root = std::env::temp_dir().join(format!(
        "loft_174_{tag}_{}_{}",
        backend.trim_start_matches('-'),
        std::process::id()
    ));
    fa::create_dir_all(&root).expect("scratch dir");
    let path = if inline {
        let p = root.join("prog.loft");
        fa::write(&p, src).expect("write program");
        p
    } else {
        std::env::current_dir().expect("cwd").join(src)
    };
    let mut cmd = Command::new(loft_bin());
    cmd.arg(backend)
        .arg(&path)
        .current_dir(&root)
        .env("LOFT_TIMEOUT", "180")
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
fn every_read_of_a_mapped_file_and_of_a_view_answers_the_copying_path_on_both_backends() {
    for (file, last) in [(READS, "foreign file ok"), (VIEWS, "foreign view ok")] {
        for backend in ["--interpret", "--native"] {
            for (tag, envs) in [
                ("plain", vec![]),
                ("verify", vec![("LOFT_HOIST_VERIFY", "1")]),
                ("strict", vec![("LOFT_STRICT_STORES", "1")]),
                ("poison", vec![("LOFT_POISON", "1")]),
                ("leak", vec![("LOFT_STORES", "warn")]),
            ] {
                let (ok, _, out) = run(tag, backend, file, false, &envs);
                assert!(
                    ok && out.contains(last),
                    "{file} {backend} {tag}: the cells must pass:\n{out}"
                );
                assert!(
                    !out.to_lowercase().contains("leak"),
                    "{file} {backend} {tag}: every mapping and view must be released:\n{out}"
                );
            }
        }
    }
}

#[test]
fn the_view_and_the_copy_print_the_same_lines_on_both_backends() {
    for file in [READS, VIEWS] {
        for backend in ["--interpret", "--native"] {
            let (ok, with, _) = run("ab_view", backend, file, false, &[]);
            let (ok2, without, _) = run(
                "ab_copy",
                backend,
                file,
                false,
                &[("LOFT_NO_FOREIGN_VIEW", "1")],
            );
            assert!(ok && ok2, "{file} on {backend}: both forms must pass");
            assert_eq!(
                with, without,
                "{file} on {backend}: the view and the copy must agree"
            );
        }
    }
}

#[test]
fn a_write_into_a_mapped_file_is_refused_at_compile_time_with_the_copy_advice() {
    for backend in ["--interpret", "--native"] {
        let (ok, _, out) = run("refused", backend, WRITE_REFUSED, true, &[]);
        assert!(!ok, "{backend}: the write must be refused:\n{out}");
        assert!(
            out.contains("Cannot modify 'm': it holds read-only data the program does not own")
                && out.contains("copy it first"),
            "{backend}: the refusal must be the compile-time one with the copy cure:\n{out}"
        );
        assert!(
            !out.contains("write to bytes the program does not own"),
            "{backend}: refused before the program runs, not at run time:\n{out}"
        );
        assert!(
            !out.contains("reached: the write landed"),
            "{backend}: the write must not land:\n{out}"
        );
        assert!(
            !out.contains("out of bounds"),
            "{backend}: a refused write is not a corrupt reference:\n{out}"
        );
    }
}

#[test]
fn a_write_into_a_view_is_refused_at_compile_time_and_lands_on_the_copy() {
    for backend in ["--interpret", "--native"] {
        let (ok, _, out) = run("view_refused", backend, VIEW_WRITE_REFUSED, true, &[]);
        assert!(
            !ok,
            "{backend}: a write into a view must be refused:\n{out}"
        );
        assert!(
            out.contains("Cannot modify 's': it holds read-only data the program does not own")
                && out.contains("copy it first"),
            "{backend}: the refusal must name the view and the copy cure:\n{out}"
        );
        assert!(
            !out.contains("reached: the writes landed") && !out.contains("out of bounds"),
            "{backend}: the write must neither land nor read as corrupt:\n{out}"
        );
        // The refusal is the language's, not the view's: with the view off (the slice is a
        // copy underneath) the same program is refused the same way.
        let (ok, _, out) = run(
            "view_copy",
            backend,
            VIEW_WRITE_REFUSED,
            true,
            &[("LOFT_NO_FOREIGN_VIEW", "1")],
        );
        assert!(
            !ok && out.contains("Cannot modify 's': it holds read-only data"),
            "{backend}: under LOFT_NO_FOREIGN_VIEW=1 the write is refused the same way:\n{out}"
        );
    }
}

/// The IR of function `name` in the dump.
fn ir_of(dump: &str, name: &str) -> String {
    let start = dump
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("no IR for {name}:\n{dump}"));
    let rest = &dump[start..];
    let end = rest[1..].find("\nfn ").map_or(rest.len(), |i| i + 1);
    rest[..end].to_string()
}

#[test]
fn a_bind_into_the_locals_own_store_takes_the_view_op_and_every_other_shape_the_copy() {
    let root = std::env::temp_dir().join(format!("loft_174_probe_{}", std::process::id()));
    fa::create_dir_all(&root).expect("scratch dir");
    let path = root.join("probe.loft");
    fa::write(&path, PROBE).expect("write probe");
    let out = Command::new(loft_bin())
        .arg("introspect")
        .arg(&path)
        .env("RUST_BACKTRACE", "0")
        .output()
        .expect("run loft introspect");
    let _ = fa::remove_dir_all(&root);
    let dump = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // A fresh local and a rebound one own their `__vdb_N` backing: the view op.
    for f in ["n_fresh", "n_rebound"] {
        let ir = ir_of(&dump, f);
        assert!(
            ir.contains("OpSliceView(") && !ir.contains("OpSliceVector("),
            "{f}: a bind into the local's own store takes the view op:\n{ir}"
        );
    }
    assert_eq!(
        ir_of(&dump, "n_rebound").matches("OpSliceView(").count(),
        2,
        "the rebind takes the view op too"
    );
    // An append keeps the copy beside the first bind's view.
    let ir = ir_of(&dump, "n_appended");
    assert!(
        ir.contains("OpSliceView(") && ir.contains("OpSliceVector("),
        "n_appended: `+=` copies:\n{ir}"
    );
    // A returned local is the caller's buffer, and an argument the caller's vector: neither
    // store is the local's own, so the returned local copies.  The argument's `v = v[1..3]`
    // is a slice of the vector itself and keeps its range in place (`OpKeepRange`); a
    // foreign or locked store takes the copy form at run time (`vector_keep_range`).
    let ir = ir_of(&dump, "n_returned");
    assert!(
        ir.contains("OpSliceVector(") && !ir.contains("OpSliceView("),
        "n_returned: the return buffer is not the local's own store:\n{ir}"
    );
    let ir = ir_of(&dump, "n_argument");
    assert!(
        ir.contains("OpKeepRange(v") && !ir.contains("___p390_tmp"),
        "n_argument: the parameter keeps its own slice in place:\n{ir}"
    );
}
