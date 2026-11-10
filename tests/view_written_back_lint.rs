// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `warning[view-written-back]` — both halves.
//!
//! A struct place read into a local is a VIEW of that place (`@FR-B-View`), and a write into
//! the place does not end it (`@FR-B-Disturb`: an overwritten place stays), so saving a field
//! that way and writing it back later restores nothing: the write-back copies the place onto
//! itself.  The warning stands on that write-back when something wrote the place in between.
//!
//! A warning gates a library's CI, so where it stays SILENT is pinned as tightly as where it
//! fires: a write-back with nothing written between is a harmless no-op, a write on the other
//! arm of an `if` never reaches it, a copy (`T { ..place }`) or a view the walk materialised is
//! a real backup, and another element or another field is another place.
//!
//! `tests/e1_code_set.rs` owns the code as a frozen handle and pins that it RENDERS.
//! Binary-invoked like `tests/group_apart_lint.rs`; `LOFT_NO_CACHE` because the warm program
//! cache skips the re-parse that produces the diagnostics.

use loft::file_access as fa;
use std::path::PathBuf;

const CODE: &str = "warning[view-written-back]";

const TYPES: &str = "struct Lib { name: text }\n\
     struct A { lib: Lib, k: integer }\n\
     struct State { lib: Lib, other: Lib, a: A, depth: integer }\n\
     struct N { lib: Lib? }\n\
     fn mk() -> State { State { lib: Lib { name: \"core\" }, other: Lib { name: \"o\" }, \
     a: A { lib: Lib { name: \"ai\" }, k: 0 }, depth: 0 } }\n";

/// How many times `src` (after [`TYPES`]) draws the warning, compiled on the interpreter.
fn reports(name: &str, src: &str) -> usize {
    let dir = std::env::temp_dir().join("loft_view_written_back_lint");
    fa::create_dir_all(&dir).expect("probe dir");
    let path = dir.join(format!("{name}.loft"));
    fa::write(&path, format!("{TYPES}{src}\n")).expect("write probe");
    let out = loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg("--interpret")
        .arg(&path)
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("spawn loft");
    let err = String::from_utf8_lossy(&out.stderr);
    // A program that does not compile draws no report either, which would make every quiet
    // cell pass for the wrong reason.
    assert!(out.status.success(), "{name} did not run:\n{err}");
    err.matches(CODE).count()
}

/// Every cell with the count it must draw; the failures are listed together.
fn check(cells: &[(&str, usize, &str)]) {
    let wrong: Vec<String> = cells
        .iter()
        .filter_map(|(name, want, src)| {
            let got = reports(name, src);
            (got != *want).then(|| format!("{name}: {got} report(s), want {want}"))
        })
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn a_write_back_after_the_place_was_written_is_reported_on_every_spelling() {
    check(&[
        (
            "field",
            1,
            "fn main() { st = mk(); keep = st.lib; st.lib = Lib { name: \"g\" }; st.lib = keep; \
             println(\"{st.lib.name}\"); }",
        ),
        (
            "field_of_the_place",
            1,
            "fn main() { st = mk(); keep = st.lib; st.lib.name = \"x\"; st.lib = keep; \
             println(\"{st.lib.name}\"); }",
        ),
        (
            "then_arm",
            1,
            "fn main() { st = mk(); keep = st.lib; if st.depth == 0 { st.lib = Lib { name: \"g\" }; } \
             st.lib = keep; println(\"{st.lib.name}\"); }",
        ),
        (
            "element",
            1,
            "fn main() { v = [Lib { name: \"a\" }, Lib { name: \"b\" }]; keep = v[0]; \
             v[0] = Lib { name: \"g\" }; v[0] = keep; println(\"{v[0].name}\"); }",
        ),
        (
            "loop_before",
            1,
            "fn main() { st = mk(); keep = st.lib; for i in 0..3 { st.lib = Lib { name: \"g{i}\" }; } \
             st.lib = keep; println(\"{st.lib.name}\"); }",
        ),
        (
            "loop_next_turn",
            1,
            "fn main() { st = mk(); keep = st.lib; for i in 0..2 { st.lib = keep; \
             st.lib = Lib { name: \"g{i}\" }; } println(\"{st.lib.name}\"); }",
        ),
        (
            "loop_inside",
            1,
            "fn main() { st = mk(); for i in 0..2 { keep = st.lib; st.lib = Lib { name: \"g{i}\" }; \
             st.lib = keep; } println(\"{st.lib.name}\"); }",
        ),
        (
            "enclosing_record",
            1,
            "fn main() { st = mk(); keep = st.a.lib; st.a = A { lib: Lib { name: \"g\" }, k: 1 }; \
             st.a.lib = keep; println(\"{st.a.lib.name}\"); }",
        ),
        (
            "parameter",
            1,
            "fn f(st: State) { keep = st.lib; st.lib = Lib { name: \"g\" }; st.lib = keep; }\n\
             fn main() { s = mk(); f(s); println(\"{s.lib.name}\"); }",
        ),
        (
            "nullable_field",
            1,
            "fn main() { n = N { lib: Lib { name: \"core\" } }; keep = n.lib; \
             n.lib = Lib { name: \"g\" }; n.lib = keep; println(\"{n.lib!.name}\"); }",
        ),
    ]);
}

#[test]
fn a_write_back_that_restores_what_the_view_was_bound_to_is_quiet() {
    check(&[
        (
            "another_field_written",
            0,
            "fn main() { st = mk(); keep = st.lib; st.depth = 1; st.lib = keep; \
             println(\"{st.lib.name} {st.depth}\"); }",
        ),
        (
            "else_arm",
            0,
            "fn main() { st = mk(); keep = st.lib; if st.depth == 0 { st.lib = Lib { name: \"g\" }; } \
             else { st.lib = keep; } println(\"{st.lib.name}\"); }",
        ),
        (
            "a_copy",
            0,
            "fn main() { st = mk(); keep = Lib { ..st.lib }; st.lib = Lib { name: \"g\" }; \
             st.lib = keep; println(\"{st.lib.name}\"); }",
        ),
        (
            "another_element",
            0,
            "fn main() { v = [Lib { name: \"a\" }, Lib { name: \"b\" }]; keep = v[0]; \
             v[1] = Lib { name: \"g\" }; v[0] = keep; println(\"{v[0].name} {v[1].name}\"); }",
        ),
        (
            "another_place",
            0,
            "fn main() { st = mk(); keep = st.lib; st.lib = Lib { name: \"g\" }; st.other = keep; \
             println(\"{st.other.name}\"); }",
        ),
        (
            "view_rebound",
            0,
            "fn main() { st = mk(); keep = st.lib; st.lib = Lib { name: \"g\" }; keep = st.other; \
             st.lib = keep; println(\"{st.lib.name}\"); }",
        ),
        (
            "written_back_twice",
            0,
            "fn main() { st = mk(); keep = st.lib; st.lib = keep; st.lib = keep; \
             println(\"{st.lib.name}\"); }",
        ),
        (
            "written_through_the_view",
            0,
            "fn main() { v = [Lib { name: \"a\" }, Lib { name: \"b\" }]; e = v[0]; \
             e.name = \"x\"; v[0] = e; println(\"{v[0].name}\"); }",
        ),
        (
            "materialised",
            0,
            "fn main() { st = mk(); keep = st.lib; st = mk(); st.lib.name = \"z\"; st.lib = keep; \
             println(\"{st.lib.name}\"); }",
        ),
    ]);
}
