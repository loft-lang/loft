// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-CharWalk` — the EMISSION pins.  `for c in T` over a text VARIABLE takes an ASCII
//! byte in one move and keeps the written step as its `else` arm; where nothing in the
//! loop writes T the null test is asked once before the loop and not inside it; a call or
//! literal source is bound once and walked like a variable; `LOFT_NO_CHAR_WALK=1` restores the written
//! step everywhere; `LOFT_HOIST_VERIFY=1` runs the written step beside the fast arm.  The
//! guard (`tests/scripts/158-char-walk.loft`) says the VALUES hold on both backends; this
//! pins what is emitted.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str = "tests/scripts/158-char-walk.loft";

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let out = std::env::temp_dir().join(format!("loft_char_walk_{}_{tag}.rs", std::process::id()));
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_CHAR_WALK")
        .env_remove("LOFT_HOIST_VERIFY");
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

const FAST: &str = ".wrapping_sub(1) < 0x7F {";
const NULL_ONCE: &str = "//@FR-R-CharWalk the null test, asked once";
const NULL_EACH: &str = "!= loft::state::STRING_NULL) as u8) != 1) as u8) == 1 { //break";

/// (function, character walks in it, of which take the fast step, of which ask the null
/// test once).  A walk that does not ask it once asks it per iteration.
const EXPECTED: [(&str, usize, usize, usize); 9] = [
    ("n_trace", 1, 1, 1),
    ("n_w5", 1, 1, 1),
    // The body grows / rebinds the walked local.  The walk takes its source once, into a
    // hidden local nothing writes (`@FR-I-For`, loft#1619), so its null test is hoisted too.
    ("n_w6", 1, 1, 1),
    ("n_w7", 1, 1, 1),
    ("n_w8", 1, 1, 1),
    ("n_w9", 2, 2, 2),
    // A call source and a literal source are bound ONCE to a hidden local (`@FR-I-Text`,
    // 2026-09-23) and walked as the variable that local is: fast step, null test hoisted.
    ("n_w10", 2, 2, 2),
    ("n_w11", 1, 1, 1),
    // The standard library's own walk, which `split` is built on.
    ("t_4text_split", 1, 1, 1),
];

#[test]
fn each_walk_emits_exactly_the_forms_predicted() {
    let rust = emit("on", &[]);
    for (name, walks, fast, once) in EXPECTED {
        let b = body(&rust, name);
        assert_eq!(
            (count(b, FAST), count(b, NULL_ONCE), count(b, NULL_EACH)),
            (fast, once, walks - once),
            "{name}: (fast steps, null tests asked once, null tests per iteration)"
        );
    }
    assert_eq!(
        count(&rust, "char_walk_verify("),
        0,
        "the default emits no check"
    );
}

#[test]
fn the_switch_restores_the_written_step_everywhere() {
    let rust = emit("off", &[("LOFT_NO_CHAR_WALK", "1")]);
    assert_eq!(count(&rust, FAST), 0, "LOFT_NO_CHAR_WALK=1: no fast step");
    assert_eq!(
        count(&rust, NULL_ONCE),
        0,
        "LOFT_NO_CHAR_WALK=1: no null test hoisted"
    );
    for (name, walks, _, _) in EXPECTED {
        let b = body(&rust, name);
        assert_eq!(
            count(b, NULL_EACH),
            walks,
            "{name}: every walk asks its null test per iteration with the switch on"
        );
    }
}

#[test]
fn the_checking_form_runs_the_written_step_beside_the_fast_arm() {
    let rust = emit("verify", &[("LOFT_HOIST_VERIFY", "1")]);
    for (name, _, fast, once) in EXPECTED {
        let b = body(&rust, name);
        assert_eq!(
            count(b, "vector::char_walk_verify(__cw_fast, "),
            fast,
            "{name}: every fast step is compared with the written one"
        );
        assert_eq!(
            count(b, "@FR-R-CharWalk: the text became null inside the walk"),
            once,
            "{name}: every hoisted null test is asserted inside the loop"
        );
    }
}
