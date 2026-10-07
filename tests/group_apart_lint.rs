// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `advice[linked-group-apart]` — the QUIET half.
//!
//! Two collections over one element type in one struct are one record set (@FR-Col-Group),
//! and the declaration is the only place that is decidable: by the time a `len` reads 0 the
//! question looks like an empty collection instead. The advice names it there.
//!
//! Every group is a legitimate declaration, so the whole design rests on when this stays
//! SILENT — an advice that fires on the idiom is one every reader learns to ignore, which
//! costs more than the silence it replaced. Adjacency is the signal: the idiom is written
//! together, a group nobody intended is two fields added at different times with unrelated
//! ones between them.
//!
//! `tests/e1_code_set.rs` owns the code as a frozen handle and pins that it RENDERS. This
//! file pins the four cases it must not fire on, which is the half a trigger program cannot
//! show.
//!
//! Binary-invoked like `tests/dead_code_lint.rs`: these are end-to-end compile diagnostics on
//! stderr. `LOFT_NO_CACHE` because the warm program cache skips the re-parse that produces
//! them.

use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Command;

const CODE: &str = "advice[linked-group-apart]";

/// Compile-and-run `src` on the interpreter, returning stderr.
fn diagnostics_of(name: &str, src: &str) -> String {
    let dir = std::env::temp_dir().join("loft_group_apart_lint");
    fa::create_dir_all(&dir).expect("probe dir");
    let path = dir.join(format!("{name}.loft"));
    fa::write(&path, src).expect("write probe");
    let out = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg("--interpret")
        .arg(&path)
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("spawn loft");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

const ELEM: &str = "struct Ga { k: integer, n: text }\n";

fn body(fields: &str) -> String {
    format!(
        "{ELEM}struct GaS {{ {fields} }}\n\
         fn main() {{ s = GaS {{ }}; print(\"{{s.tick}}\"); }}\n"
    )
}

#[test]
fn it_fires_when_an_unrelated_field_sits_between_the_members() {
    // The control for every silent case below: without one that fires, they are all vacuous.
    let err = diagnostics_of(
        "apart",
        &body("a: vector<Ga>, tick: integer, b: hash<Ga[k]>"),
    );
    assert!(
        err.contains(CODE),
        "expected the advice on a spread-out group; stderr={err}"
    );
    // It names the member that JOINED — the later one — not the one declared first.
    assert!(
        err.contains("`b` shares one record set with `a`"),
        "expected the later member to be named; stderr={err}"
    );
}

#[test]
fn it_reaches_a_struct_enum_variant() {
    // A variant holds fields like a struct and forms a group on the same terms, so the advice
    // has to run there too — the parser reaches variant fields by a separate loop, which is
    // where the same rewrite had been missing entirely.
    let err = diagnostics_of(
        "variant",
        &format!(
            "{ELEM}enum GaH {{ GaOnly {{ a: vector<Ga>, tick: integer, b: hash<Ga[k]> }} }}\n\
             fn main() {{ v: GaH = GaOnly {{ a: [], tick: 1, b: [] }}; print(\"{{v.tick}}\"); }}\n"
        ),
    );
    assert!(
        err.contains(CODE),
        "expected the advice inside an enum variant; stderr={err}"
    );
    // A variant carries an implicit `enum` discriminator field the source never wrote, so a
    // position resolved by attribute INDEX points one field too far. This pins the caret on
    // the member it names.
    assert!(
        err.contains("`b` shares one record set with `a`"),
        "expected the later member named in a variant; stderr={err}"
    );
}

#[test]
fn a_member_carrying_its_own_question_mark_is_still_a_member() {
    // `Optional(τ)` is τ's slot plus a compile-time bit (@FR-L-Null), and the group forms at
    // runtime for a nullable member exactly as for a dense one — so the advice owes the same
    // answer for both spellings.  `collection_element` matched the unpeeled type while its
    // sibling `is_keyed_collection` peeled, so a `?` member was dropped from the group before
    // `keyed` was consulted and BOTH advices went silent on a group that really exists.
    // Quiet on a real group is the one failure this lint cannot afford: the declaration is
    // the only place the question is decidable.
    //
    // The cure is at the shared home, so it moves BOTH advices over that question — the
    // sibling `advice[linked-group-double-fill]` was measured silent on the same two
    // spellings and speaks now. This file pins the half it owns.
    for (name, fields) in [
        ("q_keyed", "a: vector<Ga>, tick: integer, b: hash<Ga[k]>?"),
        ("q_vector", "a: vector<Ga>?, tick: integer, b: hash<Ga[k]>"),
        ("q_both", "a: vector<Ga>?, tick: integer, b: hash<Ga[k]>?"),
    ] {
        let err = diagnostics_of(name, &body(fields));
        assert!(
            err.contains(CODE),
            "silent on a group whose member carries a `?` ({fields}); stderr={err}"
        );
    }
}

#[test]
fn it_is_quiet_when_the_members_are_adjacent() {
    // The idiom. Firing here is what would make the advice noise on correct code.
    let err = diagnostics_of(
        "together",
        &body("a: vector<Ga>, b: hash<Ga[k]>, tick: integer"),
    );
    assert!(!err.contains(CODE), "fired on the idiom; stderr={err}");
}

#[test]
fn it_is_quiet_without_a_keyed_member() {
    // Two plain vectors over one element type are two collections and always were, so
    // there is no group to be apart.
    let err = diagnostics_of(
        "two_vectors",
        &body("a: vector<Ga>, tick: integer, b: vector<Ga>"),
    );
    assert!(
        !err.contains(CODE),
        "fired on two plain vectors, which are not a group; stderr={err}"
    );
}

#[test]
fn it_is_quiet_for_different_element_types() {
    let err = diagnostics_of(
        "distinct",
        &format!(
            "struct GaOther {{ k: integer }}\n\
             {ELEM}struct GaS {{ a: vector<Ga>, tick: integer, b: hash<GaOther[k]> }}\n\
             fn main() {{ s = GaS {{ }}; print(\"{{s.tick}}\"); }}\n"
        ),
    );
    assert!(
        !err.contains(CODE),
        "fired on collections over different element types; stderr={err}"
    );
}

#[test]
fn the_opt_out_silences_it() {
    let dir = std::env::temp_dir().join("loft_group_apart_lint");
    fa::create_dir_all(&dir).expect("probe dir");
    let path = dir.join("optout.loft");
    fa::write(&path, body("a: vector<Ga>, tick: integer, b: hash<Ga[k]>")).expect("write probe");
    let out = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg("--interpret")
        .arg(&path)
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_NO_GROUP_APART", "1")
        .output()
        .expect("spawn loft");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !err.contains(CODE),
        "LOFT_NO_GROUP_APART did not silence it; stderr={err}"
    );
}
