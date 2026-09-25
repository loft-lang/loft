// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-Escape` — the two API-design advices: which `pub` functions they name and, as
//! importantly, which they leave alone.  A shape a rewrite still reaches, a no-heap
//! intermediate, a stdlib producer, an answered intermediate and a private function are the
//! negatives; the switch silences both.

use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

const PROGRAM: &str = r#"
struct Piece { start: integer, count: integer }
struct Doc { buf: vector<integer>, pieces: vector<Piece>, title: text }
struct Decoded { op: text, fields: vector<integer> }
struct Bounds { w: float, h: float }
pub fn pieces(d: Doc) -> vector<Piece> { d.pieces }
pub fn buffer(self: Doc) -> vector<integer> { self.buf }
pub fn kept(d: const Doc) -> vector<Piece> { d.pieces }
fn pieces_private(d: Doc) -> vector<Piece> { d.pieces }
pub fn starts(d: Doc) -> vector<integer> { r: vector<integer> = []; for p in d.pieces { r += [p.start]; } r }
fn decode(frame: vector<integer>) -> Decoded { Decoded { op: "op{frame[0] ?? 0}", fields: [1, 2, 3] } }
fn field_text(x: Decoded, which: text) -> text { "{x.op}:{which}" }
pub fn req_op(frame: vector<integer>) -> text { return field_text(decode(frame), "op"); }
pub fn req_n(frame: vector<integer>) -> integer { len(decode(frame).fields) }
fn bounds(d: Doc) -> Bounds { Bounds { w: len(d.buf) as float, h: 1.0 } }
pub fn area(d: Doc) -> float { bounds(d).w * bounds(d).h }
pub fn first_word(s: text) -> text { s.split(' ')[0] ?? "" }
pub fn drop_first(d: Doc) -> Doc { np: vector<Piece> = []; for i in 1..len(d.pieces) { np += [d.pieces[i]?]; } return Doc { buf: d.buf, pieces: np, title: d.title }; }
pub fn decoded(frame: vector<integer>) -> Decoded { decode(frame) }
fn main() {
  d = Doc { buf: [1, 2, 3], pieces: [Piece { start: 0, count: 3 }], title: "t" };
  println("{len(pieces(d))} {len(d.buffer())} {len(kept(d))} {len(pieces_private(d))} {len(starts(d))} {req_op([7])} {req_n([7])} {area(d)} {first_word("a b")} {len(drop_first(d).pieces)} {decoded([1]).op}");
}
"#;

/// The advice lines of a run: `(code, named function)`.
fn advices(env: &[(&str, &str)]) -> Vec<(String, String)> {
    let dir = std::env::temp_dir().join(format!("loft-api-advice-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let file = dir.join("api.loft");
    std::fs::write(&file, PROGRAM).expect("write probe");
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--interpret")
        .arg(&file)
        .env("LOFT_TIMEOUT", "60")
        .env("LOFT_NO_CACHE", "1")
        .env_remove("LOFT_NO_API_ADVICE");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("failed to invoke loft");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "the probe must compile and run: {stderr}"
    );
    stderr
        .lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("advice[api-")?;
            let (code, tail) = rest.split_once("]: `")?;
            let (name, _) = tail.split_once('`')?;
            Some((format!("api-{code}"), name.to_string()))
        })
        .collect()
}

#[test]
fn the_two_shapes_are_named_and_the_negatives_stay_silent() {
    let got = advices(&[]);
    let expected = [
        ("api-copies-collection", "pieces"),
        ("api-copies-collection", "Doc.buffer"),
        ("api-copies-collection", "kept"),
        ("api-redoes-per-field", "req_op"),
        ("api-redoes-per-field", "req_n"),
    ];
    for (code, name) in expected {
        assert!(
            got.iter().any(|(c, n)| c == code && n == name),
            "{code} must name `{name}`; got {got:?}"
        );
    }
    for silent in [
        "pieces_private",
        "starts",
        "area",
        "first_word",
        "drop_first",
        "decoded",
    ] {
        assert!(
            !got.iter().any(|(_, n)| n == silent),
            "`{silent}` is not a design fault and must stay silent; got {got:?}"
        );
    }
    assert_eq!(
        got.len(),
        expected.len(),
        "exactly the five, once each: {got:?}"
    );
}

#[test]
fn the_switch_silences_both() {
    assert!(advices(&[("LOFT_NO_API_ADVICE", "1")]).is_empty());
}
