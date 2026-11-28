// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! A call to a function declared LATER is never handed its caller's return buffer.
//!
//! The work-ref names (`__ref_N`) are claimed in a different order on the two parser passes: a
//! callee declared later has no known result on pass 1, so its call mints a buffer on pass 2
//! only.  Pass 1 promotes the return site's work-ref to the return-buffer ARGUMENT, and the
//! shifted pass-2 call claimed that name with a request of the same type — the call's answer
//! landed in the caller's record, and the local holding it freed the caller's store.  A work-ref
//! that resolves to an argument is reused only by the site that minted it
//! (`Function::argument_of_another_site`).  The values are asserted on both backends by
//! `tests/scripts/a-later-declared-call-is-never-handed-the-return-buffer.loft`; this pins the IR.
use std::path::{Path, PathBuf};

const GUARD: &str = "tests/scripts/a-later-declared-call-is-never-handed-the-return-buffer.loft";

#[test]
fn a_later_declared_call_gets_a_buffer_of_its_own() {
    let guard = Path::new(env!("CARGO_MANIFEST_DIR")).join(GUARD);
    let out = loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg("introspect")
        .arg(&guard)
        .env("LOFT_TIMEOUT", "240")
        .output()
        .expect("spawn loft");
    assert!(
        out.status.success(),
        "introspect failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    let head = text
        .lines()
        .find(|l| l.starts_with("fn n_f(") && l.contains("{#block"))
        .expect("f's IR");
    // The return buffer is f's last parameter: `…, <buf>:Ex) -> Ex["<buf>"]`.
    let buf = head
        .split_once(" -> Ex[\"")
        .and_then(|(_, r)| r.split_once('"'))
        .map(|(b, _)| b.to_owned())
        .expect("f answers through a return buffer");
    let call = text
        .lines()
        .find(|l| l.contains("= n_condition("))
        .expect("the call to condition");
    assert!(
        !call.contains(&format!("{buf}(0))")),
        "the call to `condition` is handed f's return buffer `{buf}`:\n{call}"
    );
}

/// loft#2693 (`@FR-O-Buffer`, `@FR-H-FreeTwice`) — a loop exit standing lexically ahead of a
/// rebind that adopts a literal construction buffer into an already-live outer binding (a local
/// promoted onto the return buffer because its initial call targets a callee declared below, a
/// choice rebind of a promoted buffer, a nullable record local, a rebound parameter, or an
/// outer-loop view local rebound in an inner loop) sees the pairing on every pass rather than
/// freeing the adopted store twice.
#[test]
fn a_loop_exit_ahead_of_an_adopting_literal_rebind_frees_once() {
    use loft::file_access as fa;

    let dir = std::env::temp_dir().join(format!("loft_2693_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("mkdir");
    let file = dir.join("loop_exit_before_adopting_rebind.loft");
    fa::write(
        &file,
        "\
struct Inner { a: integer }
struct Rich { sub: Inner, xs: vector<integer> }
struct P { v: integer }

fn f_below(p: P, k: integer) -> P {
  x = later_p(p);
  for i in 0..3 {
    if i == k { return p; }
    x = P { v: i + 20 };
  }
  x
}
fn later_p(a: P) -> P { if a.v % 2 == 1 { return P { v: a.v + 100 }; } a }

fn f_rich(p: Rich, k: integer) -> Rich {
  x = later_rich(p);
  for i in 0..3 {
    if i == k { return p; }
    x = Rich { sub: Inner { a: i + 20 }, xs: [i, i + 1] };
  }
  x
}
fn later_rich(a: Rich) -> Rich {
  if a.sub.a % 2 == 1 { return Rich { sub: Inner { a: a.sub.a + 100 }, xs: [99] }; }
  a
}

fn f_choice(p: P, k: integer, c: boolean) -> P {
  x = P { v: 100 };
  for i in 0..3 {
    if i == k { return p; }
    x = if c { P { v: i + 20 } } else { P { v: i + 30 } };
  }
  x
}

fn f_opt(k: integer, c: boolean) -> integer {
  x: P? = P { v: 100 };
  for i in 0..3 {
    if i == k { return 0; }
    x = if c { P { v: i + 20 } } else { null };
  }
  if x { x.v } else { -1 }
}

fn f_param(p: P, k: integer, c: boolean) -> integer {
  for i in 0..3 {
    if i == k { return 0; }
    p = if c { P { v: i + 20 } } else { P { v: i + 30 } };
  }
  p.v
}

fn f_view_outer(q: P, k: integer, c: boolean) -> integer {
  x: P? = q;
  for i in 0..3 {
    if i == k { return 0; }
    x = if c { P { v: i + 20 } } else { null };
  }
  if x { x.v } else { -1 }
}

fn f_view_nested(q: P, k: integer, c: boolean) -> integer {
  s = 0;
  for o in 0..2 {
    x: P? = q;
    for i in 0..3 {
      if i == k { return s; }
      x = if c { P { v: i + 20 + o } } else { null };
    }
    if x { s += x.v; }
  }
  s
}

fn f_sibling(q: P, k: integer, c: boolean) -> integer {
  s = 0;
  if k >= 0 {
    x: P? = P { v: 50 };
    if x { s += x.v; }
  }
  for i in 0..3 {
    if i == k { return s; }
    x: P? = if c { P { v: i + 10 } } else { q };
    if x { s += x.v; }
  }
  s
}

fn main() {
  q = P { v: 4 };
  q_odd = P { v: 5 };
  t1 = 0; t1_odd = 0;
  for k in 0..4 { t1 += f_below(q, k).v; t1_odd += f_below(q_odd, k).v; }
  assert(t1 == 4 + 4 + 4 + 22, \"t1: {t1}\");
  assert(t1_odd == 5 + 5 + 5 + 22, \"t1_odd: {t1_odd}\");

  rq = Rich { sub: Inner { a: 4 }, xs: [1, 2, 3] };
  rq_odd = Rich { sub: Inner { a: 5 }, xs: [7] };
  tr = 0; tr_odd = 0;
  for k in 0..4 {
    a = f_rich(rq, k);
    b = f_rich(rq_odd, k);
    tr += a.sub.a + len(a.xs);
    tr_odd += b.sub.a + len(b.xs);
  }
  assert(tr == 7 * 3 + (22 + 2), \"tr: {tr}\");
  assert(tr_odd == 6 * 3 + (22 + 2), \"tr_odd: {tr_odd}\");

  t3 = 0;
  for k in 0..4 { t3 += f_choice(q, k, true).v + f_choice(q, k, false).v; }
  assert(t3 == (4 + 4 + 4 + 22) + (4 + 4 + 4 + 32), \"t3: {t3}\");

  t4 = 0;
  for k in 0..4 { t4 += f_opt(k, true) + f_opt(k, false); }
  assert(t4 == 22 + (-1), \"t4: {t4}\");

  t5 = 0;
  for k in 0..4 { t5 += f_param(q, k, true); }
  assert(t5 == 22, \"t5: {t5}\");

  t6 = 0;
  for k in 0..4 { t6 += f_view_outer(q, k, true); }
  assert(t6 == 22, \"t6: {t6}\");

  assert(f_view_nested(q, 1, true) == 0 && f_view_nested(q, 9, true) == 22 + 23, \"t7\");
  assert(f_sibling(q, 1, true) == 60 && f_sibling(q, 9, true) == 50 + 10 + 11 + 12, \"t8\");
  assert(f_mat_copy(0) == 1 && f_mat_copy(2) == 1011 && f_mat_copy(4) == 1141, \"t9\");
  println(\"ok\");
}

enum Ty { TVoid, TNever, TInt { min: integer }, TText { dep: vector<integer> } }
struct Ex { code: integer, tp: Ty }
fn ex_of(i: integer) -> Ex {
  Ex { code: i, tp: if i % 3 == 0 { TInt { min: i } } else if i % 3 == 1 { TText { dep: [i] } } else { TNever } }
}
fn ty_kind(t: Ty) -> integer {
  match t { TVoid => 1, TNever => 2, TInt { min } => 10 + min, TText { dep } => 100 + len(dep) }
}
// 9. Materialised view inside loop followed by enum-unit literal (`loft#2334` guard):
// `t = x.tp` strips `t`'s deps into an owner, so the later `t = TVoid` copies rather than adopting.
fn f_mat_copy(k: integer) -> integer {
  t = TVoid;
  acc = 0;
  x = ex_of(0);
  for s in 0..k {
    x = ex_of(s);
    t = x.tp;
    if s % 2 == 0 {
      if !(t is TNever) { t = TVoid; }
      continue;
    }
    x = ex_of(s + 7);
    acc += ty_kind(t);
    t = TVoid;
  }
  acc * 10 + ty_kind(t)
}
",
    )
    .expect("write test");

    for args in [
        &["--interpret"][..],
        &["--native"][..],
        &["--native", "--lean"][..],
    ] {
        let out =
            loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
                .args(args)
                .arg(&file)
                .env("LOFT_TIMEOUT", "120")
                .env("LOFT_NO_CACHE", "1")
                .env("LOFT_LEAK_CHECK", "1")
                .env("LOFT_STRICT_STORES", "1")
                .env("LOFT_POISON", "1")
                .env("LOFT_POISON_CLAIM", "1")
                .env("LOFT_NATIVE_LEAK_CHECK", "1")
                .output()
                .expect("spawn loft");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success() && stdout.trim_end().ends_with("ok"),
            "{args:?} failed (exit {:?}):\nstdout: {stdout}\nstderr: {stderr}",
            out.status
        );
    }

    // Positive control: with literal-buffer pairing disabled, the double free fires on both backends.
    for backend in ["--interpret", "--native"] {
        let out =
            loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
                .arg(backend)
                .arg(&file)
                .env("LOFT_TIMEOUT", "120")
                .env("LOFT_NO_CACHE", "1")
                .env("LOFT_NO_LITERAL_BUFFER_PAIR", "1")
                .env("LOFT_STRICT_STORES", "1")
                .output()
                .expect("spawn loft control");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success() && stderr.contains("freed twice"),
            "{backend} control must report the double free:\nstderr: {stderr}"
        );
    }

    let _ = fa::remove_dir_all(&dir);
}
