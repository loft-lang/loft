// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-N-Store` — the nullable-store warning puts its caret on the VALUE, not on the slot.
//!
//! A value with no position of its own — a local read, an `if` the parser built — used to fall
//! back to wherever the slot was: the `{` after a return type, the `)` of a call, the `;` after
//! a binding, the `]` of an index.  The cure the warning offers rewrites the value, so the caret
//! must stand on it.  Each store site now hands over the value's first token (or the arm the cure
//! describes), and this test pins every site: for each case line, every warning reported there
//! lands inside the value's text, and the line draws at least one warning.

use loft::file_access as fa;

const PROGRAM: &str = r#"struct R { n: integer, s: text }
struct H { k: integer }
fn take(a: integer) -> integer { a + 1 }
fn if_tail(v: vector<R>, i: integer, b: boolean) -> integer {
  if b { 4 } else { v[i].n }
}
fn local_tail(v: vector<R>, i: integer) -> integer {
  x = v[i].n;
  x
}
fn argument(v: vector<R>, i: integer) -> integer {
  x = v[i].n;
  take(x)
}
fn field(v: vector<R>, i: integer) -> H {
  x = v[i].n;
  H { k: x }
}
fn binding(v: vector<R>, i: integer) -> integer {
  x = v[i].n;
  y: integer = x;
  y
}
fn early(v: vector<R>, i: integer, b: boolean) -> integer {
  x = v[i].n;
  if b {
    return x;
  }
  0
}
fn field_write(v: vector<R>, i: integer, h: H) {
  x = v[i].n;
  h.k = x;
}
fn element(v: vector<R>, i: integer, w: vector<integer>) {
  x = v[i].n;
  w += [x];
}
fn yielded(v: vector<R>, i: integer) -> iterator<integer> {
  x = v[i].n;
  yield x;
}
fn indexed(v: vector<R>, i: integer, w: vector<integer>) -> integer {
  x = v[i].n;
  w[x]
}
fn text_tail(v: vector<R>, i: integer, b: boolean) -> text {
  x = v[i].s;
  if b { "a" } else { x }
}
fn tuple_member(v: vector<R>, i: integer) -> (integer, integer) {
  x = v[i].n;
  t = (1, 2);
  t.1 = x;
  t
}
fn inserted(v: vector<R>, i: integer, w: vector<integer>) {
  x = v[i].n;
  insert(w, 0, x);
}
fn main() {
  v = [R { n: 0, s: "q" }];
  h = H { k: 0 };
  w: vector<integer> = [5];
  field_write(v, 0, h);
  element(v, 0, w);
  inserted(v, 0, w);
  for y in yielded(v, 0) { println("{y}"); }
  println("{if_tail(v, 0, false)} {local_tail(v, 0)} {argument(v, 0)} {field(v, 0).k}");
  println("{binding(v, 0)} {early(v, 0, true)} {indexed(v, 0, w)} {text_tail(v, 0, false)}");
  println("{tuple_member(v, 0).1}");
}
"#;

/// Each store site: the line's text (unique in the program) and the value the caret belongs on.
const CASES: &[(&str, &str, &str)] = &[
    (
        "an `if` arm as a tail",
        "  if b { 4 } else { v[i].n }",
        "v[i].n",
    ),
    ("a local read as a tail", "  x\n", "x"),
    ("a call argument", "  take(x)", "x"),
    ("a struct field value", "  H { k: x }", "x"),
    ("a typed binding", "  y: integer = x;", "x"),
    ("an explicit return", "    return x;", "x"),
    ("a field write", "  h.k = x;", "x"),
    ("a vector literal element", "  w += [x];", "x"),
    ("a yielded value", "  yield x;", "x"),
    ("an index", "  w[x]", "x"),
    (
        "a text `if` as a tail",
        "  if b { \"a\" } else { x }",
        "if b { \"a\" } else { x }",
    ),
    ("a tuple member write", "  t.1 = x;", "x"),
    ("an `insert` element", "  insert(w, 0, x);", "x"),
];

#[test]
fn every_nullable_store_points_at_its_value() {
    let dir = std::env::temp_dir().join(format!("loft-nstore-caret-{}", std::process::id()));
    fa::create_dir_all(&dir).expect("scratch dir");
    let file = dir.join("caret.loft");
    fa::write(&file, PROGRAM).expect("write probe");
    let out = loft::platform::process::harness_command(std::path::PathBuf::from(env!(
        "CARGO_BIN_EXE_loft"
    )))
    .arg("--interpret")
    .arg(&file)
    .env("LOFT_TIMEOUT", "60")
    .env("LOFT_NO_CACHE", "1")
    .output()
    .expect("failed to invoke loft");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "the probe must run:\n{err}");
    // Every `--> file:line:col` the run printed, found by the file's NAME: on Windows the
    // temp directory may print in its long form while `temp_dir()` answered the 8.3 one.
    let name = "caret.loft:";
    let carets: Vec<(usize, usize)> = err
        .lines()
        .filter_map(|l| l.trim_start().strip_prefix("--> "))
        .filter_map(|at| at.rfind(name).map(|i| &at[i + name.len()..]))
        .filter_map(|lc| {
            let (l, c) = lc.split_once(':')?;
            Some((l.parse().ok()?, c.parse().ok()?))
        })
        .collect();
    let lines: Vec<&str> = PROGRAM.lines().collect();
    for (what, line_text, value) in CASES {
        let wanted = line_text.trim_end_matches('\n');
        let idx = lines
            .iter()
            .position(|l| *l == wanted)
            .unwrap_or_else(|| panic!("{what}: line `{wanted}` not in the program"));
        let line = idx + 1;
        let start = lines[idx].find(value).expect("value on its line") + 1;
        let end = start + value.len() - 1;
        let here: Vec<usize> = carets
            .iter()
            .filter(|(l, _)| *l == line)
            .map(|(_, c)| *c)
            .collect();
        assert!(
            !here.is_empty(),
            "{what}: no warning on line {line}:\n{err}"
        );
        for col in here {
            assert!(
                (start..=end).contains(&col),
                "{what}: the caret is at {line}:{col}, outside the value `{value}` \
                 (columns {start}..={end}):\n{err}"
            );
        }
    }
}
