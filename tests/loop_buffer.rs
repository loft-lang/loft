// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-al (`@FR-R-LoopBuffer`) — the EMISSION pins.  A vector local declared `[]`
//! inside a loop is backed by a per-site buffer whose every mint after the first is a
//! length reset (`vector::vector_buffer_reset`) and whose literal field zero is not
//! emitted; a `text` vector keeps the re-mint; `LOFT_NO_LOOP_BUFFER_REUSE=1` restores the
//! re-mint everywhere.  The cell corpus (`bytecode-comparisons/V-al-loop-buffer-cells.loft`)
//! says the VALUES hold on both backends; this pins what is emitted.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-al-loop-buffer-cells.loft";

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    // One file per TEST: the tests run on threads of one process.
    let out =
        std::env::temp_dir().join(format!("loft_loop_buffer_{}_{tag}.rs", std::process::id()));
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_LOOP_BUFFER_REUSE");
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

#[test]
fn a_loop_local_vector_keeps_its_store_and_resets_its_length() {
    let rust = emit("keep", &[]);
    for name in ["n_l1", "n_l4", "n_l5", "n_l6", "n_l7"] {
        let b = body(&rust, name);
        assert!(
            b.contains("vector::vector_buffer_reset(&var___vdb_"),
            "{name}: the in-loop mint is a length reset after the first pass"
        );
        assert!(
            b.contains("if var___vdb_1.store_nr == u16::MAX || var___vdb_1.rec == 0 { var___vdb_1 = OpDatabase")
                || b.contains("if var___vdb_2.store_nr == u16::MAX || var___vdb_2.rec == 0 { var___vdb_2 = OpDatabase"),
            "{name}: the first pass still mints"
        );
    }
    // The nested cell keeps BOTH buffers: the outer loop's and the inner loop's.
    let l3 = body(&rust, "n_l3");
    assert!(
        l3.contains("vector::vector_buffer_reset(&var___vdb_1")
            && l3.contains("vector::vector_buffer_reset(&var___vdb_2"),
        "l3: the outer and the inner loop's vectors both keep their stores"
    );
}

#[test]
fn the_literal_field_zero_is_not_emitted_for_a_loop_buffer() {
    let rust = emit("zero", &[]);
    let l1 = body(&rust, "n_l1");
    // The kept buffer's vector field is never written back to null: the zero the `[]`
    // literal lowers to would drop the vector the reuse keeps.
    assert!(
        !l1.contains("let db = (var___vdb_1); let v = if _v_val == i64::MIN"),
        "l1: the `[]` literal's field zero is dropped for a loop buffer"
    );
}

#[test]
fn a_text_vector_keeps_the_re_mint() {
    let rust = emit("text", &[]);
    let l2 = body(&rust, "n_l2");
    assert!(
        !l2.contains("vector_buffer_reset"),
        "l2: elements that own heap are cleared by the mint, never length-reset"
    );
    assert!(
        l2.contains("var___vdb_1 = OpDatabase"),
        "l2: the re-mint stands"
    );
}

#[test]
fn the_switch_restores_the_re_mint_everywhere() {
    let rust = emit("switch", &[("LOFT_NO_LOOP_BUFFER_REUSE", "1")]);
    assert!(
        !rust.contains("vector_buffer_reset"),
        "LOFT_NO_LOOP_BUFFER_REUSE=1 must emit no length reset"
    );
    let l1 = body(&rust, "n_l1");
    assert!(
        l1.contains("var___vdb_1 = OpDatabase"),
        "l1 under the switch re-mints per iteration"
    );
}
