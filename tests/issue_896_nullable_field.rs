// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#896 — a nullable struct FIELD (`maybe: Inner?`) must answer as absent.
//!
//! The field used to be stored as a dense `Inner`, byte-identical to a non-nullable one, so
//! absence had nowhere to live: `OpGetField` handed back a `DbRef` into the PARENT's record
//! (whose `rec` is never 0), every reader saw a present-but-zeroed value, and a store of `null`
//! found nothing to clear. All three readers of the declaration disagreed with it — `??` never
//! took its default, `== null` was always false, and `f = null` kept the previous value.
//!
//! The representation that fixes it already shipped for vector ELEMENTS: the synthetic
//! `__nullable<S>` enum, where absent is discriminant `0`. What was missing is that the
//! field-side rewrite matched a bare `Reference` while `S?` reaches the type table as
//! `Optional(Reference(S))` — so it fired on exactly the complement of its intended set.
//!
//! The cells below are the boundary matrix that drove the fix, and `dense_control` is what
//! keeps it a boundary: a NON-nullable `Inner` field must stay dense and pay for no
//! discriminant. `w*` cells cover what the first matrix could not see — a heap payload (the
//! `OpClearKeyed` that keeps a replaced `Some` from leaking), a struct copy carrying the
//! discriminant, and the value crossing a call in both directions.

use loft::file_access as fa;
use std::path::Path;
use std::process::Command;

fn run(args: &[&str], prog: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args(args)
        .arg(prog)
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("spawn loft binary");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

fn rustc_available() -> bool {
    Command::new("rustc").arg("--version").output().is_ok()
}

fn probe(name: &str, body: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("loft_issue896_probe");
    fa::create_dir_all(&dir).expect("probe dir");
    let path = dir.join(format!("{name}.loft"));
    fa::write(&path, body).expect("write probe");
    path
}

/// Run one cell on BOTH backends and require the exact expected first line. Both halves
/// matter: the interpreter tolerated `OpCopyRecord` of a null source as a silent no-op, and
/// native refused to compile it at all, so a cell proven on one backend says nothing about
/// the other.
fn assert_both(name: &str, body: &str, want: &str) {
    let prog = probe(name, body);
    let (ok, out) = run(&["--interpret"], &prog);
    assert!(ok, "{name}: interpret run failed; output={out:?}");
    let got = out.lines().next().unwrap_or_default();
    assert_eq!(got, want, "{name}: interpret");
    if rustc_available() {
        let (ok, out) = run(&["--native"], &prog);
        assert!(ok, "{name}: native run failed; output={out:?}");
        let got = out.lines().next().unwrap_or_default();
        assert_eq!(got, want, "{name}: native");
    }
}

const HDR: &str = "struct Inner { z: integer }\nstruct H { maybe: Inner?, tag: integer }\n";

#[test]
fn construct_with_null_is_accepted() {
    assert_both(
        "c1",
        &format!(
            "{HDR}fn main() {{ h = H {{ maybe: null, tag: 1 }}; println(\"tag {{h.tag}}\"); }}"
        ),
        "tag 1",
    );
}

#[test]
fn construct_with_value_is_accepted() {
    assert_both(
        "c2",
        &format!(
            "{HDR}fn main() {{ h = H {{ maybe: Inner{{z:9}}, tag: 1 }}; println(\"tag {{h.tag}}\"); }}"
        ),
        "tag 1",
    );
}

/// The omitted field is the shape that did not even COMPILE on native: `object_init` left the
/// dense field to a `to_default` of `null`, which emitted `OpCopyRecord(null, …)` and reached
/// rustc as `OpCopyRecord(cell, (), …)` — `()` where a `DbRef` is expected.
#[test]
fn construct_with_field_omitted_compiles_on_both_backends() {
    assert_both(
        "c3",
        &format!("{HDR}fn main() {{ h = H {{ tag: 1 }}; println(\"tag {{h.tag}}\"); }}"),
        "tag 1",
    );
}

#[test]
fn coalesce_takes_the_default_when_absent() {
    assert_both(
        "c4",
        &format!(
            "{HDR}fn main() {{ h = H {{ maybe: null, tag: 1 }}; println(\"z {{(h.maybe ?? Inner{{z:-1}}).z}}\"); }}"
        ),
        "z -1",
    );
}

#[test]
fn coalesce_keeps_the_value_when_present() {
    assert_both(
        "c5",
        &format!(
            "{HDR}fn main() {{ h = H {{ maybe: Inner{{z:9}}, tag: 1 }}; println(\"z {{(h.maybe ?? Inner{{z:-1}}).z}}\"); }}"
        ),
        "z 9",
    );
}

#[test]
fn equality_against_null_answers_true_when_absent() {
    assert_both(
        "c6",
        &format!(
            "{HDR}fn main() {{ h = H {{ maybe: null, tag: 1 }}; println(\"isnull {{h.maybe == null}}\"); }}"
        ),
        "isnull true",
    );
}

#[test]
fn equality_against_null_answers_false_when_present() {
    assert_both(
        "c7",
        &format!(
            "{HDR}fn main() {{ h = H {{ maybe: Inner{{z:9}}, tag: 1 }}; println(\"isnull {{h.maybe == null}}\"); }}"
        ),
        "isnull false",
    );
}

#[test]
fn assigning_a_value_makes_the_field_present() {
    assert_both(
        "c8",
        &format!(
            "{HDR}fn main() {{ h = H {{ maybe: null, tag: 1 }}; h.maybe = Inner{{z:9}}; \
             println(\"z {{(h.maybe ?? Inner{{z:-1}}).z}} isnull {{h.maybe == null}}\"); }}"
        ),
        "z 9 isnull false",
    );
}

#[test]
fn assigning_null_clears_the_field() {
    assert_both(
        "c9",
        &format!(
            "{HDR}fn main() {{ h = H {{ maybe: Inner{{z:9}}, tag: 1 }}; h.maybe = null; \
             println(\"z {{(h.maybe ?? Inner{{z:-1}}).z}} isnull {{h.maybe == null}}\"); }}"
        ),
        "z -1 isnull true",
    );
}

/// A bare read through a present nullable field still reaches the payload — the enum does not
/// force a `??` on code that already knows the value is there.
#[test]
fn a_bare_read_reaches_the_payload() {
    assert_both(
        "c10",
        &format!(
            "{HDR}fn main() {{ h = H {{ maybe: Inner{{z:7}}, tag: 1 }}; println(\"bare {{h.maybe.z}}\"); }}"
        ),
        "bare 7",
    );
}

/// The boundary: a field typed `Inner` (no `?`) cannot be absent, so it must stay DENSE. This
/// is the cell that fails if the rewrite ever again selects on something other than the `?`.
#[test]
fn a_non_nullable_struct_field_stays_dense() {
    assert_both(
        "c11",
        "struct Inner { z: integer }\nstruct D { inner: Inner, tag: integer }\n\
         fn main() { d = D { inner: Inner{z:5}, tag: 1 }; println(\"dense {d.inner.z}\"); }",
        "dense 5",
    );
}

#[test]
fn a_host_inside_a_vector_still_answers_absent() {
    assert_both(
        "c12",
        &format!(
            "{HDR}fn main() {{ v: vector<H> = [H{{maybe:null,tag:1}}]; \
             println(\"v {{v[0].tag}} isnull {{v[0].maybe == null}}\"); }}"
        ),
        "v 1 isnull true",
    );
}

/// A `Some` holding a heap payload is released before the slot is reused. Three states in one
/// run — present, cleared, present again — because a clear that frees nothing and a clear that
/// frees twice both read as "works" from a single transition.
#[test]
fn a_text_payload_survives_clear_and_reuse() {
    let body = "struct Inner { s: text }\nstruct H { maybe: Inner?, tag: integer }\n\
        fn main() {\n\
        h = H { maybe: Inner{s:\"alpha\"}, tag: 1 };\n\
        println(\"1 {(h.maybe ?? Inner{s:\\\"none\\\"}).s}\");\n\
        h.maybe = null;\n\
        println(\"2 {(h.maybe ?? Inner{s:\\\"none\\\"}).s}\");\n\
        h.maybe = Inner{s:\"beta\"};\n\
        println(\"3 {(h.maybe ?? Inner{s:\\\"none\\\"}).s}\");\n\
        }";
    let prog = probe("w1", body);
    for args in [vec!["--interpret"], vec!["--native"]] {
        if args[0] == "--native" && !rustc_available() {
            continue;
        }
        let (ok, out) = run(&args, &prog);
        assert!(ok, "w1 {args:?} failed; output={out:?}");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.first().copied(), Some("1 alpha"), "w1 {args:?}");
        assert_eq!(lines.get(1).copied(), Some("2 none"), "w1 {args:?}");
        assert_eq!(lines.get(2).copied(), Some("3 beta"), "w1 {args:?}");
    }
}

/// A vector payload behaves the same as a text one — the clear releases the nested collection
/// rather than leaving it reachable only through a slot that now reads absent.
#[test]
fn a_vector_payload_is_released_on_clear() {
    assert_both(
        "w2",
        "struct Inner { v: vector<integer> }\nstruct H { maybe: Inner?, tag: integer }\n\
         fn main() {\n\
         h = H { maybe: Inner{v:[1,2,3]}, tag: 1 };\n\
         h.maybe = null;\n\
         println(\"len {len((h.maybe ?? Inner{v:[]}).v)}\");\n\
         }",
        "len 0",
    );
}

/// A struct copy carries the discriminant in BOTH states. Copying only ever moved the payload
/// bytes before, so an absent field could not survive a copy that had nothing to say it.
#[test]
fn a_copy_carries_presence_in_both_states() {
    let body = format!(
        "{HDR}fn main() {{\n\
         a = H {{ maybe: null, tag: 1 }};\n\
         b = a;\n\
         println(\"absent {{b.maybe == null}}\");\n\
         c = H {{ maybe: Inner{{z:5}}, tag: 2 }};\n\
         d = c;\n\
         println(\"present {{d.maybe == null}} z {{(d.maybe ?? Inner{{z:-1}}).z}}\");\n\
         }}"
    );
    let prog = probe("w3", &body);
    for args in [vec!["--interpret"], vec!["--native"]] {
        if args[0] == "--native" && !rustc_available() {
            continue;
        }
        let (ok, out) = run(&args, &prog);
        assert!(ok, "w3 {args:?} failed; output={out:?}");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.first().copied(), Some("absent true"), "w3 {args:?}");
        assert_eq!(
            lines.get(1).copied(),
            Some("present false z 5"),
            "w3 {args:?}"
        );
    }
}

/// The field crosses a call in both directions — as a by-value parameter and as a return.
#[test]
fn presence_survives_a_call_in_both_directions() {
    let body = format!(
        "{HDR}fn take(h: H) -> boolean {{ h.maybe == null }}\n\
         fn make(present: boolean) -> H {{ \
         if present {{ H{{maybe:Inner{{z:4}},tag:1}} }} else {{ H{{maybe:null,tag:1}} }} }}\n\
         fn main() {{\n\
         println(\"param {{take(H{{maybe:null,tag:1}})}} {{take(H{{maybe:Inner{{z:1}},tag:1}})}}\");\n\
         println(\"ret {{make(false).maybe == null}} {{make(true).maybe == null}}\");\n\
         }}"
    );
    let prog = probe("w4", &body);
    for args in [vec!["--interpret"], vec!["--native"]] {
        if args[0] == "--native" && !rustc_available() {
            continue;
        }
        let (ok, out) = run(&args, &prog);
        assert!(ok, "w4 {args:?} failed; output={out:?}");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines.first().copied(),
            Some("param true false"),
            "w4 {args:?}"
        );
        assert_eq!(lines.get(1).copied(), Some("ret true false"), "w4 {args:?}");
    }
}

/// Nesting: a nullable field inside the payload of another nullable field. The inner
/// discriminant lives inside the outer `Some`, so a layout that placed either at a fixed
/// offset would read the wrong byte here.
#[test]
fn a_nullable_field_nests_inside_a_nullable_payload() {
    let body = "struct Leaf { z: integer }\nstruct Mid { leaf: Leaf?, m: integer }\n\
        struct Top { mid: Mid?, t: integer }\n\
        fn main() {\n\
        x = Top { mid: null, t: 1 };\n\
        println(\"outer {x.mid == null}\");\n\
        y = Top { mid: Mid{leaf:null, m:2}, t: 1 };\n\
        println(\"inner {(y.mid ?? Mid{leaf:null,m:0}).leaf == null}\");\n\
        }";
    let prog = probe("w7", body);
    for args in [vec!["--interpret"], vec!["--native"]] {
        if args[0] == "--native" && !rustc_available() {
            continue;
        }
        let (ok, out) = run(&args, &prog);
        assert!(ok, "w7 {args:?} failed; output={out:?}");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.first().copied(), Some("outer true"), "w7 {args:?}");
        assert_eq!(lines.get(1).copied(), Some("inner true"), "w7 {args:?}");
    }
}
