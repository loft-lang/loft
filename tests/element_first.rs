// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-z (`@FR-R-ElemFirst`) — the element-first build: a local vector consumed
//! exactly once as a record-literal field of an append is built INSIDE the appended
//! element (minted at the first temp's declaration; the length bump stays at the
//! append's finish), and the reservation, the mint, the paired handle-zeros and the
//! paired `OpAppendVector` copies vanish from the append site.  The cell corpus
//! (`bytecode-comparisons/V-z-element-first-cells.loft`) says the VALUES hold; this
//! pins the EMISSION — which appends earn the early mint, and that every declining
//! shape (a read after the append, an `out` read in between, an append under an `if`
//! arm) keeps the temp-store build — and the switch (`LOFT_NO_ELEMENT_FIRST=1`).  A pair
//! the IR applies (`opt::elem_first`, a literal-built temp) is counted from its trace
//! (`LOFT_TRACE_IR_ELEM_FIRST`), one the writer applies from its mint comment.
use loft::file_access as fa;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-z-element-first-cells.loft";

const MARK: &str = "V-z element minted";

/// The emitted Rust, and per function the pairs the IR applied — `(local, field)` by
/// destination, read from the pass's trace.
fn emit(src: &Path, out: &Path, env: &[(&str, &str)]) -> (String, HashMap<String, (usize, usize)>) {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    // `--lean`: the CLOSED program, where the value records the view-exit pairs ride on run
    // (NATIVE.md § Optimisation tiers).
    cmd.arg("--lean")
        .arg("--native")
        .arg("--native-emit")
        .arg(out)
        .arg(src)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_TRACE_IR_ELEM_FIRST", "1")
        .env("LOFT_NO_CACHE", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.output().expect("spawn loft --native-emit");
    assert!(
        fa::exists(out),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    let mut ir: HashMap<String, (usize, usize)> = HashMap::new();
    for line in String::from_utf8_lossy(&status.stderr).lines() {
        if let Some(rest) = line.strip_prefix("[ir-elem-first] fn=")
            && let Some((name, dest)) = rest.split_once(" pair ")
        {
            let e = ir.entry(name.to_string()).or_default();
            if dest == "field" {
                e.1 += 1;
            } else {
                e.0 += 1;
            }
        }
    }
    (fa::read_to_string(out).expect("read the emitted Rust"), ir)
}

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

#[test]
fn the_switch_restores_the_temp_store_build() {
    let out = std::env::temp_dir().join("loft_element_first_off.rs");
    let rust = emit(&cells(), &out, &[("LOFT_NO_ELEMENT_FIRST", "1")]);
    assert_eq!(
        rust.0.matches(MARK).count() + rust.1.len(),
        0,
        "under LOFT_NO_ELEMENT_FIRST=1 every literal field keeps its temp-store build"
    );
    let _ = fa::remove_file(&out);
}
