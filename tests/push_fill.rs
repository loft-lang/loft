// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-am (`@FR-R-PushFill`) — the EMISSION pins.  A counted loop that is one push
//! of an invariant is one `stores.push_fill` with the per-element loop under `if !__pf_N`;
//! a counted loop pushing computed values reserves its pushes times its trip count before
//! it runs (`vector::reserve_more`); a loop with a `break` or a call for its end takes
//! neither; `LOFT_NO_PUSH_FILL=1` restores the per-push form everywhere.  The cell corpus
//! (`bytecode-comparisons/V-am-push-fill-cells.loft`) says the VALUES hold on both
//! backends; this pins what is emitted.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-am-push-fill-cells.loft";

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let out = std::env::temp_dir().join(format!("loft_push_fill_{}_{tag}.rs", std::process::id()));
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_PUSH_FILL")
        .env_remove("LOFT_NO_PUSH_HOIST");
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
fn one_push_of_an_invariant_is_one_fill_with_the_loop_as_its_fallback() {
    let rust = emit("fill", &[]);
    for (name, ty) in [
        ("n_p1", "i64"),
        ("n_p2", "i64"),
        ("n_p6", "f64"),
        ("n_p11", "i64"),
    ] {
        let b = body(&rust, name);
        assert!(
            b.contains(&format!("stores.push_fill::<{ty}, false>(&mut __ph_")),
            "{name}: the constant push loop is one fill"
        );
        assert!(
            b.contains("if !__pf_"),
            "{name}: the per-element loop is the fallback"
        );
    }
}

#[test]
fn computed_pushes_reserve_their_trip_count_before_the_loop() {
    let rust = emit("reserve", &[]);
    for name in ["n_p4", "n_p7", "n_p10"] {
        let b = body(&rust, name);
        assert!(
            b.contains("vector::reserve_more(&(var_v), _pn.saturating_mul("),
            "{name}: the loop reserves its pushes times its trip count"
        );
        assert!(
            !b.contains("push_fill::<"),
            "{name}: computed values are no fill"
        );
    }
    let p4 = body(&rust, "n_p4");
    assert!(
        p4.contains("_pn.saturating_mul(3_i64)"),
        "p4 pushes three per iteration and reserves three per iteration"
    );
}

#[test]
fn an_early_exit_declines_and_a_call_for_the_end_does_not() {
    let rust = emit("decline", &[]);
    let p5 = body(&rust, "n_p5");
    assert!(
        !p5.contains("push_fill::<") && !p5.contains("reserve_more"),
        "p5: a `break` in the body takes no fast path"
    );
    // A call for the range's end is taken ONCE, before the first round (`@FR-I-Range`,
    // loft#1619), so the end is invariant and the loop is the fill it looks like.
    let p9 = body(&rust, "n_p9");
    assert!(
        p9.contains("push_fill::<") || p9.contains("reserve_more"),
        "p9: a call for the end is taken once, so the loop takes the fast path:\n{p9}"
    );
}

#[test]
fn the_switch_restores_the_per_push_form_everywhere() {
    let rust = emit("switch", &[("LOFT_NO_PUSH_FILL", "1")]);
    assert!(
        !rust.contains("push_fill::<") && !rust.contains("reserve_more"),
        "LOFT_NO_PUSH_FILL=1 must emit no fill and no reserve"
    );
}
