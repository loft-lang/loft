// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN174 F4a + `(Slice-Value)` — the EMISSION pins: a scalar-element slice bound to a local
//! copies its span in one block (`OpSliceVector`) while a heap, struct or type-variable
//! element keeps the per-element loop; `LOFT_NO_SLICE_COPY=1` keeps the loop everywhere;
//! and the two cell files answer the same lines on both backends with the copy on and off
//! (the A/B that falsifies the rewrite).  The VALUES are the cells' own.
use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

const SLICE_CELLS: &str = "tests/scripts/174-slice-copy.loft";
const VALUE_CELLS: &str =
    "tests/scripts/174-a-slice-is-a-vector-wherever-a-vector-is-expected.loft";

/// The emitted-IR probe: one function per element kind.
const PROBE: &str = "\
struct P { x: integer, y: integer }
fn ints(v: vector<integer>) -> vector<integer> { s = v[1..3]; s }
fn bytes(v: vector<u8>) -> vector<u8> { s = v[1..3]; s }
fn floats(v: vector<float>) -> vector<float> { s = v[1..3]; s }
fn texts(v: vector<text>) -> vector<text> { s = v[1..3]; s }
fn vecs(v: vector<vector<integer>>) -> vector<vector<integer>> { s = v[1..3]; s }
fn recs(v: vector<P>) -> vector<P> { s = v[1..3]; s }
fn gen<T>(v: vector<T>) -> vector<T> { s = v[1..3]; s }
fn main() {
  println(\"{len(ints([1, 2, 3, 4]))} {len(bytes([1, 2, 3, 4]))} {len(floats([1.0, 2.0, 3.0]))} \
{len(texts([\\\"a\\\", \\\"b\\\", \\\"c\\\"]))} {len(vecs([[1], [2], [3]]))} \
{len(recs([P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }]))} {len(gen([7, 8, 9]))}\");
}
";

fn introspect(src: &str, envs: &[(&str, &str)]) -> String {
    let root = std::env::temp_dir().join(format!("loft_slice_copy_{}", std::process::id()));
    fa::create_dir_all(&root).expect("scratch dir");
    let path = root.join("probe.loft");
    fa::write(&path, src).expect("write probe");
    let mut cmd = Command::new(loft_bin());
    cmd.arg("introspect").arg(&path).env("RUST_BACKTRACE", "0");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run loft introspect");
    let _ = fa::remove_dir_all(&root);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
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
fn a_scalar_element_slice_copies_in_one_block_and_every_other_kind_keeps_the_loop() {
    let dump = introspect(PROBE, &[]);
    for f in ["n_ints", "n_bytes", "n_floats"] {
        let ir = ir_of(&dump, f);
        assert!(
            ir.contains("OpSliceVector(") && !ir.contains("Slice materialise"),
            "{f}: a scalar element must copy in one block:\n{ir}"
        );
    }
    for f in ["n_texts", "n_vecs", "n_recs"] {
        let ir = ir_of(&dump, f);
        assert!(
            !ir.contains("OpSliceVector(") && ir.contains("Slice materialise"),
            "{f}: a heap or struct element must keep the loop:\n{ir}"
        );
    }
    // The generic's INSTANCE at `integer` still keeps the loop: a monomorph is re-lowered
    // from the template's element-copy triplets (`rewrite_vector_write_triplets`), which
    // the block copy is not part of — a known gap, pinned so its closing is a visible
    // change (@PLN174 F4a).  Three block copies, one per scalar function above.
    assert_eq!(
        dump.matches("OpSliceVector(").count(),
        3,
        "exactly the three scalar functions copy in one block:\n{dump}"
    );
}

#[test]
fn the_switch_keeps_the_loop_everywhere() {
    let dump = introspect(PROBE, &[("LOFT_NO_SLICE_COPY", "1")]);
    assert!(
        !dump.contains("OpSliceVector("),
        "LOFT_NO_SLICE_COPY=1 must emit no block copy:\n{dump}"
    );
    assert!(
        dump.matches("Slice materialise").count() >= 7,
        "every slice must be the loop under the switch:\n{dump}"
    );
}

fn run_cells(file: &str, backend: &str, envs: &[(&str, &str)]) -> String {
    let root = std::env::temp_dir().join(format!(
        "loft_slice_cells_{}_{}",
        std::process::id(),
        backend.trim_start_matches('-')
    ));
    fa::create_dir_all(&root).expect("scratch dir");
    let path = std::env::current_dir().expect("cwd").join(file);
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
    let _ = fa::remove_dir_all(&root);
    assert!(
        out.status.success(),
        "{file} on {backend} with {envs:?} failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn the_copy_and_the_loop_answer_the_same_lines_on_both_backends() {
    for file in [SLICE_CELLS, VALUE_CELLS] {
        for backend in ["--interpret", "--native"] {
            let with = run_cells(file, backend, &[]);
            let without = run_cells(file, backend, &[("LOFT_NO_SLICE_COPY", "1")]);
            assert_eq!(
                with, without,
                "{file} on {backend}: the copy and the loop must agree"
            );
            assert!(
                with.contains(" ok"),
                "{file} on {backend} must reach its last line:\n{with}"
            );
        }
    }
}
