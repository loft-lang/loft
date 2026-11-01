// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#1190 — `advice[avoidable-copy]` is the BORROW worklist, so it is silent where no
//! borrow exists.
//!
//! A `value struct` (@PLN101) is stored INLINE wherever it lives, so writing one into a
//! container writes its bytes into storage the container already has: nothing is allocated,
//! and the rewrite the advice names buys exactly zero.  A plain record that owns no heap is
//! the same once it lands IN a structure — a vector's element and a record's field are slots
//! of the container — so its copy is silent there too (COPY_DIAGNOSTICS § What remains open).
//!
//! And a copy is advised only with a rewrite usable at its site (DIAGNOSTICS.md § Fix lines):
//! each control below reads its source again in a statement of its own that can move above
//! the copy — a read in the statement that also reads the copy's destination is a NECESSARY
//! copy, and says nothing.
//!
//! **This is the gate; the `.loft` guard beside it is not.** The defect is a diagnostic that
//! must NOT fire, and neither channel the corpus scores can see that — `check_diagnostics`
//! logs an unexpected warning without failing ("Other warnings are not fatal"), and
//! `make falsify` compares exit / assertions / leak / panic / refusals, none of which move
//! when a notice appears. So the absence is asserted HERE, by counting the notices on stderr,
//! on both backends. `tests/scripts/1190-an-inline-value-copy-has-no-advice.loft` carries the
//! VALUE half — that the suppression did not turn the copy into an alias — which the corpus
//! does score.
//!
//! Why the binary and not the in-process harness: these are end-to-end compile diagnostics on
//! stderr, the same approach `tests/dead_code_lint.rs` takes, and `LOFT_NO_CACHE` is needed
//! because a warm run skips the re-parse that emits them.

use loft::file_access as fa;
use std::io::Write;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// The program under test: four inline-value copies that must be silent, and one copy that
/// must still be advised.
const PROGRAM: &str = r#"
value struct Stamp { ms: integer, tick: integer }
value struct Wrapped { at: Stamp, id: integer }
value struct Heapy { xs: vector<integer>, n: integer }
struct Recorded { ms: integer }

fn main() {
  a = Stamp { ms: 1, tick: 10 };
  v = [a];
  print("{v[0].ms}{a.tick}");

  s = Stamp { ms: 7, tick: 70 };
  w = Wrapped { at: s, id: 3 };
  print("{w.at.ms}{s.tick}");

  n = Wrapped { at: s, id: 4 };
  ns = [n];
  print("{ns[0].at.tick}{n.id}");

  h = Heapy { xs: [1, 2], n: 1 };
  hv = [h];
  hn = h.n;
  print("{len(hv[0].xs)}{hn}");

  r = Recorded { ms: 1 };
  rv = [r];
  print("{rv[0].ms}{r.ms}\n");
}
"#;

fn run(backend: &str, path: &PathBuf) -> (String, String, Option<i32>) {
    let out = loft::platform::process::harness_command(loft_bin())
        .arg(backend)
        .arg(path)
        .env("LOFT_TIMEOUT", "180")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("failed to invoke loft binary");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn write_program(dir: &PathBuf) -> PathBuf {
    fa::create_dir_all(dir).expect("scratch dir");
    let path = dir.join("value_struct_copy.loft");
    let mut f = fa::create(&path).expect("write program");
    f.write_all(PROGRAM.as_bytes()).expect("write program");
    path
}

fn assert_backend(backend: &str) {
    let dir = std::env::temp_dir().join(format!(
        "loft_1190_copy_advice_{}",
        backend.trim_matches('-')
    ));
    let path = write_program(&dir);
    let (stdout, diag, code) = run(backend, &path);

    assert_eq!(
        code,
        Some(0),
        "[{backend}] the program must run\n{stdout}\n---\n{diag}"
    );
    assert!(
        stdout.contains("11"),
        "[{backend}] the program must reach its output\n{stdout}"
    );

    // The one that MUST still be advised. `Heapy` is the control that says the marker is not
    // the whole answer: `value struct` says how the value is STORED, not what it OWNS, and a
    // `vector` field beneath one is a real duplicated store.
    assert!(
        diag.contains("copy of Heapy"),
        "[{backend}] `copy of Heapy` must still be advised\n{diag}"
    );

    // The ones that must be SILENT — an inline value with no heap part anywhere beneath it,
    // as an element, as a field, and nested two deep; and a plain record with none, placed
    // as an element.
    for unwanted in ["copy of Stamp", "copy of Wrapped", "copy of Recorded"] {
        assert!(
            !diag.contains(unwanted),
            "[{backend}] `{unwanted}` allocates nothing — there is no borrow to advise\n{diag}"
        );
    }

    // Counted as well as named: a notice that moved to another type, or a second one on the
    // same type, is a regression this file exists to catch.
    let n = diag.matches("advice[avoidable-copy]").count();
    assert_eq!(
        n, 1,
        "[{backend}] want exactly 1 avoidable-copy notice (Heapy), got {n}\n{diag}"
    );
}

#[test]
fn inline_value_copies_are_not_advised_interpret() {
    assert_backend("--interpret");
}

#[test]
fn inline_value_copies_are_not_advised_native() {
    assert_backend("--native");
}

/// A second loop over one name binds `f#1` and a third `f#2` (`Variables::loop_binding`) — the
/// native backend declares a local per name — and the advice printed that stored name: "copy of
/// R — `f#1` is still used after this point".  The author wrote `f`.  The name comes from
/// `Variables::written_name`; `tests/scripts/a-diagnostic-names-a-second-loops-variable-as-it-
/// was-written.loft` carries the values, and this carries the diagnostic, which the suite's
/// in-process harness never produces (it runs the advice's pass only in the CLI).
const SHADOWED_LOOPS: &str = r#"
struct R { id: integer, ws: vector<integer> }
struct Q { id: integer, ks: vector<integer> }
fn mk_r(n: integer) -> vector<R> { out: vector<R> = []; for i in 0..n { out += [R { id: i + 1, ws: [i] }]; } out }
fn mk_q(n: integer) -> vector<Q> { out: vector<Q> = []; for i in 0..n { out += [Q { id: (i + 1) * 10, ks: [i, i] }]; } out }
fn main() {
  a: vector<R> = [];
  for f in mk_r(2) { a += [f]; }
  b: vector<R> = [];
  s = 0;
  for f in mk_r(3) { b += [f]; s += f.id; }
  c: vector<Q> = [];
  t = 0;
  for f in mk_q(2) { c += [f]; t += f.id; }
  print("{len(a)} {len(b)} {s} {len(c)} {t}\n");
}
"#;

fn assert_written_loop_names(backend: &str) {
    let dir = std::env::temp_dir().join(format!(
        "loft_written_loop_names_{}",
        backend.trim_matches('-')
    ));
    fa::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join("shadowed_loops.loft");
    fa::write(&path, SHADOWED_LOOPS).expect("write program");
    let (stdout, diag, code) = run(backend, &path);
    assert_eq!(
        code,
        Some(0),
        "[{backend}] the program must run\n{stdout}\n---\n{diag}"
    );
    assert!(
        stdout.contains("2 3 6 2 30"),
        "[{backend}] the program must answer its values\n{stdout}"
    );
    for want in [
        "copy of R — `f` is used again at line",
        "copy of Q — `f` is used again at line",
    ] {
        assert!(diag.contains(want), "[{backend}] want `{want}`\n{diag}");
    }
    for leaked in ["`f#1`", "`f#2`"] {
        assert!(
            !diag.contains(leaked),
            "[{backend}] the advice names the compiler's {leaked}, which the author never wrote\n{diag}"
        );
    }
}

#[test]
fn a_second_loops_variable_is_advised_by_its_written_name_interpret() {
    assert_written_loop_names("--interpret");
}

#[test]
fn a_second_loops_variable_is_advised_by_its_written_name_native() {
    assert_written_loop_names("--native");
}

/// A copy of a FIELD (`dst.ks += src.ks`, `H { v: o.ks }`, `xs[0] = o.inner`) is decided by
/// nothing about the record it was read from: ending `o`'s life moves nothing out of it, and the
/// emitted copy is the same op whether `o` is used again or not.  The advice read the copy as one
/// of the whole record — "copy of M — `src` is still used after this point, so it could not be
/// moved" — which names a type nobody copied and a cure that changes nothing.  A PARAMETER copied
/// into a field (`H { v: xs }`, `xs` read after) is the same case one step over: its store is the
/// caller's, and the copy is the same op whether `xs` is read again or not.  A copy of a whole
/// LOCAL that survives (`H { v: ks }` with `ks` read after) is the control: it still advises.
const PROJECTIONS: &str = r#"
struct M { ks: vector<integer>, vs: vector<integer> }
struct H { v: vector<integer> }
struct In { a: vector<integer> }
struct O { inner: In, n: integer }
fn app(dst: M, src: const M) {
  dst.ks += src.ks;
  dst.vs += src.vs;
}
fn hold(xs: vector<integer>) -> integer {
  h = H { v: xs };
  len(h.v) + len(xs)
}
fn main() {
  a = M { ks: [1], vs: [2] };
  b = M { ks: [3, 4], vs: [5] };
  app(a, b);
  o = M { ks: [6, 7, 8], vs: [] };
  h = H { v: o.ks };
  r = O { inner: In { a: [9] }, n: 2 };
  xs: vector<In> = [In { a: [] }];
  xs[0] = r.inner;
  ks = [10, 11];
  k = H { v: ks };
  nk = len(ks);
  print("{len(a.ks)} {len(a.vs)} {len(h.v)} {len(o.ks)} {len(xs[0].a)} {r.n} {len(k.v)} {nk} {hold([12, 13, 14])}\n");
}
"#;

fn assert_projection_copies(backend: &str) {
    let dir = std::env::temp_dir().join(format!(
        "loft_projection_copy_advice_{}",
        backend.trim_matches('-')
    ));
    fa::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join("projection_copies.loft");
    fa::write(&path, PROJECTIONS).expect("write program");
    let (stdout, diag, code) = run(backend, &path);
    assert_eq!(
        code,
        Some(0),
        "[{backend}] the program must run\n{stdout}\n---\n{diag}"
    );
    // a.ks = [1, 3, 4], a.vs = [2, 5], h.v = o.ks = 3 long, xs[0].a = [9], r.n = 2, k.v = ks = 2,
    // and `hold` answers its field and its parameter, 3 + 3.
    assert!(
        stdout.contains("3 2 3 3 1 2 2 2 6"),
        "[{backend}] the program must answer its values\n{stdout}"
    );
    for unwanted in ["copy of M", "copy of O", "`xs` is still used"] {
        assert!(
            !diag.contains(unwanted),
            "[{backend}] `{unwanted}`: a field was copied, and no move of its record avoids that\n{diag}"
        );
    }
    let n = diag.matches("advice[avoidable-copy]").count();
    assert!(
        n == 1 && diag.contains("copy of vector<integer> — `ks` is used again at line"),
        "[{backend}] want exactly the whole-variable control advised, got {n}\n{diag}"
    );
}

#[test]
fn a_copied_field_is_not_advised_as_its_record_interpret() {
    assert_projection_copies("--interpret");
}

#[test]
fn a_copied_field_is_not_advised_as_its_record_native() {
    assert_projection_copies("--native");
}

/// loft#2039 — `(H-Move)`: an owned droppable placed into a structure MOVES, read off the line
/// whatever follows, and the byte copy that carries it is that move.  The advice said "`a` is
/// still used after this point, so it could not be moved" at three such sites — a read in the
/// same literal, a move under one branch with a read on the other, a move in both arms — while
/// the drop trace printed one release each.  A non-droppable copy beside them is the control.
const MOVED_DROPPABLES: &str = r#"
struct H { id: integer }
fn OpDrop(self: H) { print("D{self.id} "); }
fn mk(i: integer) -> H { H { id: i } }
struct S { h: H, tag: integer }
struct Holder { v: vector<integer> }
fn a1() { a = mk(1); s = S { h: a, tag: a.id }; print("R{s.h.id} T{s.tag} "); }
fn a9(c: boolean) { a = mk(9); if c { s = S { h: a, tag: 0 }; print("R{s.h.id} "); } else { print("A{a.id} "); } }
fn a10(c: boolean) { a = mk(10); v: vector<H> = []; if c { s = S { h: a, tag: 0 }; print("S{s.h.id} "); } else { v += [a]; print("V{len(v)} "); } }
fn use_it(s: vector<integer>) -> integer { len(s) }
fn main() {
  a1(); a9(true); a9(false); a10(true); a10(false);
  src = [1, 2, 3]; h = Holder { v: src };
  n = use_it(src);
  println("| {n} {len(h.v)}");
}
"#;

fn assert_moved_droppables(backend: &str) {
    let dir = std::env::temp_dir().join(format!(
        "loft_2039_moved_droppables_{}",
        backend.trim_matches('-')
    ));
    fa::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join("moved_droppables.loft");
    fa::create(&path)
        .and_then(|mut f| f.write_all(MOVED_DROPPABLES.as_bytes()))
        .expect("write program");
    let (stdout, diag, code) = run(backend, &path);
    assert_eq!(
        code,
        Some(0),
        "[{backend}] the program must run\n{stdout}\n---\n{diag}"
    );
    // One release per H, each where its holder or its owner dies — the move the rule names.
    assert!(
        stdout.contains("R1 T1 D1 R9 D9 A9 D9 S10 D10 V1 D10 | 3 3"),
        "[{backend}] the program must answer its values and release each H once\n{stdout}"
    );
    assert!(
        !diag.contains("copy of H"),
        "[{backend}] a moved droppable is not a copy to advise\n{diag}"
    );
    let n = diag.matches("advice[avoidable-copy]").count();
    assert!(
        n == 1 && diag.contains("copy of vector<integer> — `src` is used again at line"),
        "[{backend}] want exactly the non-droppable control advised, got {n}\n{diag}"
    );
}

#[test]
fn a_moved_droppable_is_not_advised_as_a_copy_interpret() {
    assert_moved_droppables("--interpret");
}

#[test]
fn a_moved_droppable_is_not_advised_as_a_copy_native() {
    assert_moved_droppables("--native");
}

/// loft#2342 — "still used after this point" is a PATH question.  A read in a `match` arm
/// exclusive with the copy's, and a read of a loop variable on the loop's next turn after it is
/// assigned again, are no later use: each copy is the source's last use on every path, so it is
/// a move and says nothing.  The control reads the source again in its OWN arm, by a statement
/// that can run before the copy, and is advised with that move.
const PATHS: &str = r#"
enum Node { NdNull, NdText { s: text }, NdCall { d: integer, args: vector<Node> } }
struct Acc { src: vector<Node> }
fn cl(d: integer, args: vector<Node>) -> Node { NdCall { d: d, args: args } }
fn marm(k: integer, x: Node) -> Node {
  pos = NdText { s: "p" };
  code = NdNull;
  match k {
    1 => { code = cl(1, [x, pos]); },
    2 => { code = cl(2, [x, pos]); },
    _ => { code = cl(3, [x, pos]); }
  }
  code
}
fn lex(n: integer) -> Acc {
  r = Acc { src: [] };
  at = 0;
  while at < n {
    t = if at == 0 { NdNull } else { NdText { s: "{at}" } };
    r.src += [t];
    at += 1;
  }
  r
}
fn control(k: integer) -> integer {
  w = NdText { s: "w" };
  h = Acc { src: [] };
  if k > 0 {
    h.src += [w];
    seen = w is NdText;
    return if seen { len(h.src) } else { 0 };
  }
  0
}
fn main() {
  c = marm(2, NdNull);
  n = match c { NdCall { d, args } => d * 10 + len(args), _ => -1 };
  print("{n} {len(lex(3).src)} {control(1)}\n");
}
"#;

fn assert_paths(backend: &str) {
    let dir = std::env::temp_dir().join(format!("loft_2342_paths_{}", backend.trim_matches('-')));
    fa::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join("paths.loft");
    fa::write(&path, PATHS).expect("write program");
    let (stdout, diag, code) = run(backend, &path);
    assert_eq!(
        code,
        Some(0),
        "[{backend}] the program must run\n{stdout}\n---\n{diag}"
    );
    // marm(2): NdCall { d: 2, args: [x, pos] } → 22; lex(3) appends three; control(1) → 1.
    assert!(
        stdout.contains("22 3 1"),
        "[{backend}] the program must answer its values\n{stdout}"
    );
    for unwanted in ["`pos`", "`t` is used again"] {
        assert!(
            !diag.contains(unwanted),
            "[{backend}] {unwanted}: no path reads it after the copy, so the copy is a move\n{diag}"
        );
    }
    let n = diag.matches("advice[avoidable-copy]").count();
    assert!(
        n == 1 && diag.contains("`w` is used again at line"),
        "[{backend}] want exactly the same-arm control advised, got {n}\n{diag}"
    );
}

#[test]
fn a_read_no_path_reaches_is_no_later_use_interpret() {
    assert_paths("--interpret");
}

#[test]
fn a_read_no_path_reaches_is_no_later_use_native() {
    assert_paths("--native");
}
