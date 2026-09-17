// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! loft#1549, `@FR-H-ClearRelease`'s record clause — A REUSED RECORD BUFFER RELEASES WHAT IT
//! HELD.  `(R-Reuse)` allocates a call site's return buffer once and hands it to every call;
//! the callee's literal overwrites every handle it writes, so before each call after the
//! first the buffer's previous occupant is released — the `else` arm of the pool's lazy
//! guard, `if OpRefIsNull(b) { OpDatabase(b, T) } else OpClear(b, T)`.  The guard
//! `tests/scripts/1549-a-pooled-buffer-releases-its-previous-occupant.loft` measures the
//! memory and the values; this pins the DECISION the scope pass makes, which both backends
//! read: a record with a heap field is released, a record of scalars is not, and a
//! struct-enum is released through its PARENT type, the one its walk dispatches from.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "doc/claude/plans/164-activation-arena/bytecode-comparisons/1549-pooled-buffer-release-cells.loft";

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

fn loft() -> Command {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "200")
        .env_remove("LOFT_NO_LAZY_BUFFER")
        .env_remove("LOFT_NO_RETBUF_REUSE");
    cmd
}

fn introspect(env: &[(&str, &str)]) -> String {
    let mut cmd = loft();
    cmd.arg("introspect").arg(cells());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn loft");
    assert!(
        out.status.success(),
        "introspect failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The IR of `name` as `loft introspect` prints it: from its header to its closing block.
fn ir_of<'a>(text: &'a str, name: &str) -> &'a str {
    let start = text
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("{name}'s IR"));
    let rest = &text[start..];
    let end = rest
        .find("}#block(1)")
        .expect("the function's closing block");
    &rest[..end]
}

/// The type operand of every `<op>(__ref_1(1), <tp>i32)` in `ir`.
fn operands(ir: &str, op: &str) -> Vec<String> {
    let head = format!("{op}(__ref_1(1), ");
    ir.match_indices(&head)
        .map(|(at, _)| {
            let rest = &ir[at + head.len()..];
            rest[..rest.find("i32)").expect("a typed operand")].to_owned()
        })
        .collect()
}

#[test]
fn a_reused_buffer_of_a_heap_owning_record_releases_before_each_refill() {
    let text = introspect(&[]);
    for name in [
        "n_c1", "n_c2", "n_c3", "n_c4", "n_c5", "n_c6", "n_c9", "n_c10", "n_c11",
    ] {
        let ir = ir_of(&text, name);
        let released = operands(ir, "OpClear");
        assert!(
            released.len() == 1 && released == operands(ir, "OpDatabase"),
            "{name}'s pooled buffer must be released by the type it is minted as:\n{ir}"
        );
    }
}

#[test]
fn a_buffer_of_scalars_takes_no_release() {
    let text = introspect(&[]);
    let ir = ir_of(&text, "n_c8");
    assert!(
        operands(ir, "OpClear").is_empty() && operands(ir, "OpDatabase").len() == 1,
        "a record of scalars owns nothing, so its reuse releases nothing:\n{ir}"
    );
}

#[test]
fn a_struct_enum_buffer_is_released_through_its_parent_type() {
    let text = introspect(&[]);
    let caller = ir_of(&text, "n_c12");
    let released = operands(caller, "OpClear");
    assert!(
        released.len() == 1 && released == operands(caller, "OpDatabase"),
        "c12's buffer is released by the type it is minted as:\n{caller}"
    );
    // The callee's payload literals mint their VARIANT types, which differ from the parent
    // the release walks — the buffer holds the previous call's variant, not the next one's.
    let callee = ir_of(&text, "n_mk_shape");
    assert!(
        callee.contains("OpDatabase(__retbuf(0), ")
            && !callee.contains(&format!("OpDatabase(__retbuf(0), {}i32)", released[0])),
        "mk_shape's variant literals must mint a type other than the parent {}:\n{callee}",
        released[0]
    );
}

#[test]
fn a_buffer_minted_at_entry_releases_before_each_use() {
    let text = introspect(&[("LOFT_NO_LAZY_BUFFER", "1")]);
    let ir = ir_of(&text, "n_c1");
    let mint = ir.find("OpDatabase(__ref_1(1), ").expect("the entry mint");
    let release = ir.find("OpClear(__ref_1(1), ").expect("the release");
    let call = ir.find("n_mk_v(").expect("the call");
    assert!(
        mint < release && release < call,
        "the eager form mints first and releases in front of the call:\n{ir}"
    );
}

#[test]
fn native_lowers_the_release_to_remove_claims() {
    let out_path = std::env::temp_dir().join(format!(
        "loft_pooled_buffer_release_{}.rs",
        std::process::id()
    ));
    let out = loft()
        .arg("--native-emit")
        .arg(&out_path)
        .arg(cells())
        .output()
        .expect("spawn loft");
    assert!(
        out.status.success(),
        "--native-emit failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let rust = std::fs::read_to_string(&out_path).expect("the emitted Rust");
    let _ = std::fs::remove_file(&out_path);
    let body = |name: &str| {
        let start = rust
            .find(&format!("fn {name}("))
            .unwrap_or_else(|| panic!("{name} in the emission"));
        let rest = &rust[start..];
        let end = rest[1..].find("\nfn ").map_or(rest.len(), |e| e + 1);
        rest[..end].to_owned()
    };
    assert!(
        body("n_c1").contains("stores.remove_claims(&(var___ref_1)"),
        "c1's pooled buffer must be released on native too:\n{}",
        body("n_c1")
    );
    assert!(
        !body("n_c8").contains("remove_claims"),
        "c8's buffer holds a record of scalars and releases nothing:\n{}",
        body("n_c8")
    );
}
