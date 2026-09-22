// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-InPlace`'s hidden-buffer allowance over a record that HOLDS HEAP — the
//! EMISSION pins.  A loop whose body discharges `m.chunks[i]?` over a `Chunk` with a
//! vector field holds the vector's header and element base, folds the join reads through
//! them, and reads `len(m.chunks)` off the header; `LOFT_NO_NULL_BUFFER_HOIST=1` keeps
//! the loop on no header (the control).  The guard
//! (`tests/scripts/158-heap-discharge-buffer.loft`) says the VALUES hold on both backends
//! under `LOFT_HOIST_VERIFY=1`; this pins what is emitted.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/158-heap-discharge-buffer.loft";

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let out = std::env::temp_dir().join(format!(
        "loft_heap_discharge_{}_{tag}.rs",
        std::process::id()
    ));
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_NULL_BUFFER_HOIST")
        .env_remove("LOFT_NO_VECTOR_HOIST")
        .env_remove("LOFT_NO_VECTOR_BASE");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.output().expect("spawn loft --native-emit");
    assert!(
        out.exists(),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    let rust = std::fs::read_to_string(&out).expect("read the emitted Rust");
    let _ = std::fs::remove_file(&out);
    rust
}

/// The emitted body of one function, up to the next top-level `fn`.
fn body<'a>(rust: &'a str, name: &str) -> &'a str {
    let start = rust
        .find(&format!("\nfn {name}("))
        .unwrap_or_else(|| panic!("{name} was not emitted"));
    let rest = &rust[start + 1..];
    let end = rest[3..].find("\nfn ").map_or(rest.len(), |i| i + 3);
    &rest[..end]
}

fn count(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

#[test]
fn a_loop_discharging_a_heap_record_holds_its_header_and_base() {
    let rust = emit("hoist", &[]);
    let f = body(&rust, "n_find_set");
    assert!(
        f.contains("let __vh_") && f.contains("let __vb_"),
        "find_set: the loop over `m.chunks` holds a header and an element base"
    );
    // (`len(m.chunks)` in the range's bound is still a CALL per iteration: `(R-Wrapper)`
    // inlines a one-op wrapper only over leaf arguments, and `m.chunks` is a field path —
    // a lever of its own, recorded in the round-3 ledger, not this rule's.)
    // The three `m.chunks[i]?.<scalar>` joins fold through the base: one range test and
    // one load each, the join's mint written once in the fallback arm.
    assert_eq!(
        count(f, "/*@FR-R-Base join read*/"),
        3,
        "find_set: the three join reads go through the held base"
    );
    // The two-site loop and the outer/inner shape hold theirs too.
    for name in ["n_two_sites", "n_sum_hexes", "n_probe"] {
        assert!(
            body(&rust, name).contains("let __vh_"),
            "{name}: a discharge of a heap-holding record no longer declines the headers"
        );
    }
}

#[test]
fn the_switch_keeps_the_loop_on_no_header() {
    let rust = emit("off", &[("LOFT_NO_NULL_BUFFER_HOIST", "1")]);
    let f = body(&rust, "n_find_set");
    assert!(
        !f.contains("let __vh_") && !f.contains("/*@FR-R-Base join read*/"),
        "find_set: with the allowance off the join's mint declines the loop's headers"
    );
}
