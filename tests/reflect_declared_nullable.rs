// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#995 — `FieldInfo.nullable` follows the DECLARATION for every field kind.
//!
//! It is documented as *"was the field DECLARED nullable (`text?` rather than `text`)?"*
//! and named as the fact a generated `CREATE TABLE` needs for `NOT NULL` (@F107). It
//! answered that correctly for the seven scalar kinds and a constant `true` for the four
//! non-scalar ones — enum, record, vector and keyed collection — so a generic serialiser
//! or ORM emitted every one of those columns as nullable, dropping a `NOT NULL` the
//! declaration had asked for.
//!
//! The two spellings genuinely differ, which is what makes it a lost fact rather than two
//! things that are really one: constructing all eight fields with `null` and asking
//! `x.f == null` answers `false` for the non-null spelling and `true` for the `?` one, on
//! both backends. That comparison is the ground truth this file checks reflection against,
//! rather than a hard-coded table — a cell that agreed with a table but not with the
//! language would be measuring the wrong thing.
//!
//! The cause was scope, not logic: @PLN25 DN1 derives the flag from the `Optional`
//! wrapper, and the rollout gated that on `is_non_null_scalar`. The synthetic tuple
//! attributes have derived it from every element type since @PLN114; a declared field now
//! agrees with them.

use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Both spellings of every non-scalar kind, plus two scalars as the control that was
/// always right. Each field is constructed with `null` so the run also reports what the
/// declaration decides at runtime — EXCEPT the two kinds whose not-null spelling really
/// does hold the null it is handed, `n: integer` and `e: Col995`, which are constructed
/// with a value instead.
///
/// The enum joined that exception once a value enum's two absent bytes were read the
/// same way: `C995 { e: null }` writes the enum's null sentinel, and the field then
/// RENDERS `null` and discharges through `??`, so `x.e == null` answering `true` is the
/// language agreeing with itself rather than a defect. It was `false` only while the
/// comparison contradicted both the renderer and the coalesce. That makes a null-valued
/// not-null enum field useless as the ground truth for a DECLARED-nullability flag —
/// the two facts genuinely differ there — so the cell asks the same question of a field
/// holding a real variant, exactly as the `integer` control already does.
/// `p`/`pq` are the POINTER spelling, and they are here because they are the one kind
/// where the `?` is not the question: #328 made `reference<T>` in field position a
/// pointer, and a pointer holds null however it is written. Reading the truth out of the
/// run rather than a table is what makes that fall out instead of having to be known.
const DECLARED_ABOVE: &str = "struct At995 { p: integer }\n\
enum Col995 { Red, Green }\n\
struct C995 {\n\
\x20 e: Col995,          eq: Col995?,\n\
\x20 v: vector<integer>, vq: vector<integer>?,\n\
\x20 h: hash<At995[p]>,  hq: hash<At995[p]>?,\n\
\x20 r: At995,           rq: At995?,\n\
\x20 p: reference<At995>, pq: reference<At995>?,\n\
\x20 n: integer,         nq: integer?,\n\
}\n\
fn main() {\n\
\x20 x = C995 { e: Col995::Red, eq: null, v: null, vq: null, h: null, hq: null,\n\
\x20            r: null, rq: null, p: null, pq: null, n: 0, nq: null };\n\
\x20 println(\"truth e={x.e == null} eq={x.eq == null} v={x.v == null} vq={x.vq == null}\");\n\
\x20 println(\"truth h={x.h == null} hq={x.hq == null} r={x.r == null} rq={x.rq == null}\");\n\
\x20 println(\"truth p={x.p == null} pq={x.pq == null} n={x.n == null} nq={x.nq == null}\");\n\
\x20 for f in type_of(x).fields { println(\"field {f.name}={f.nullable}\") }\n\
}\n";

/// The same struct with its member types declared BELOW it.
///
/// The flag is deposited on pass 1 and never revisited, so a forward reference is the
/// shape that could read the wrong declaration — the pass-1 type of a field whose type is
/// not yet resolved is not the type the author wrote.
///
/// ⚠ No ground-truth line here, and the reason is a PRE-EXISTING and separate defect: a
/// forward-declared NULLABLE record or enum field types as `unknown?` at a comparison
/// site, so `x.rq == null` is *"No matching operator '==' on 'unknown?' and 'null'"*.
/// Measured identical on a pre-fix binary, so it is not fallout from this change — but it
/// does mean this cell has to compare reflection against a written-down expectation
/// rather than against the language. That is the weaker oracle, which is why the
/// declared-ABOVE cells carry the real one.
const DECLARED_BELOW: &str = "struct C995 {\n\
\x20 e: Col995,          eq: Col995?,\n\
\x20 v: vector<integer>, vq: vector<integer>?,\n\
\x20 h: hash<At995[p]>,  hq: hash<At995[p]>?,\n\
\x20 r: At995,           rq: At995?,\n\
\x20 n: integer,         nq: integer?,\n\
}\n\
struct At995 { p: integer }\n\
enum Col995 { Red, Green }\n\
fn main() {\n\
\x20 x = C995 { e: null, eq: null, v: null, vq: null, h: null, hq: null,\n\
\x20            r: null, rq: null, n: 0, nq: null };\n\
\x20 for f in type_of(x).fields { println(\"field {f.name}={f.nullable}\") }\n\
}\n";

/// What the declaration says, field by field — the `?` four are nullable and the rest are
/// not. Written out only for the forward-declared cell; see its note.
const DECLARED: &[(&str, bool)] = &[
    ("e", false),
    ("eq", true),
    ("v", false),
    ("vq", true),
    ("h", false),
    ("hq", true),
    ("r", false),
    ("rq", true),
    ("n", false),
    ("nq", true),
];

fn run(tag: &str, src: &str, backend: &str) -> String {
    // The BACKEND is part of the name: the two cells for one source run in parallel and
    // each removes its probe, so a shared path is one cell deleting the other's file.
    let backend_tag = backend.trim_start_matches('-');
    let path = std::env::temp_dir().join(format!(
        "loft_995_{tag}_{backend_tag}_{}.loft",
        std::process::id()
    ));
    fa::write(&path, src).expect("write probe");
    let out = Command::new(loft_bin())
        .arg(backend)
        .arg(&path)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("spawn loft");
    let _ = fa::remove_file(&path);
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success(),
        "[{tag}/{backend}] the probe must run:\n{all}"
    );
    all
}

/// Reflection must answer what the language answers — for every kind, both spellings.
fn assert_reflection_matches_the_declaration(tag: &str, src: &str, backend: &str) {
    let all = run(tag, src, backend);
    // `x.f == null` is false exactly where the field cannot hold null, which is where
    // `nullable` must be false. Reading the truth out of the run rather than hard-coding
    // it keeps the two halves from being one assertion written twice.
    let truth: Vec<(String, bool)> = all
        .lines()
        .filter(|l| l.starts_with("truth "))
        .flat_map(|l| {
            l.trim_start_matches("truth ")
                .split(' ')
                .collect::<Vec<_>>()
        })
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            Some((k.to_string(), v == "true"))
        })
        .collect();
    assert_eq!(
        truth.len(),
        12,
        "[{tag}/{backend}] every field must report its runtime nullability:\n{all}"
    );
    for (name, holds_null) in truth {
        let want = format!("field {name}={holds_null}");
        assert!(
            all.contains(&want),
            "[{tag}/{backend}] `{name}` holds null = {holds_null} at runtime, so \
             FieldInfo.nullable must say {holds_null} — a serialiser reads this for \
             `NOT NULL` (loft#995)\n{all}"
        );
    }
}

#[test]
fn declared_above_interpret() {
    assert_reflection_matches_the_declaration("above", DECLARED_ABOVE, "--interpret");
}

#[test]
fn declared_above_native() {
    assert_reflection_matches_the_declaration("above", DECLARED_ABOVE, "--native");
}

/// A member type declared below its user must not change what the field reports.
fn assert_reflection_matches_the_written_declaration(backend: &str) {
    let all = run("below", DECLARED_BELOW, backend);
    for (name, nullable) in DECLARED {
        let want = format!("field {name}={nullable}");
        assert!(
            all.contains(&want),
            "[below/{backend}] `{name}` is declared {}, and declaration ORDER is not part \
             of the question — the flag is deposited on pass 1 and never revisited, which \
             is what makes a forward reference the shape to check (loft#995)\n{all}",
            if *nullable { "nullable" } else { "non-null" }
        );
    }
}

#[test]
fn declared_below_interpret() {
    assert_reflection_matches_the_written_declaration("--interpret");
}

#[test]
fn declared_below_native() {
    assert_reflection_matches_the_written_declaration("--native");
}
