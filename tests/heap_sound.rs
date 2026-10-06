// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-H-Sound` — a program the checker accepts never reads a store after its free, on either
//! backend.
//!
//! An ordinary run cannot see such a read: a freed arena slot keeps its bytes, so a stale read
//! answers the right value until the slot is handed to someone else.  Each cell here therefore
//! runs with both instruments armed.  `LOFT_POISON` overwrites a freed store, and
//! `LOFT_STRICT_STORES` never recycles a slot, so every read of a freed store is reported by name.
//! A cell passes when it prints its hand-computed value, exits cleanly and reports no
//! use-after-free.  These are subprocess cells because both switches are read once per process.
//!
//! The nightly `LOFT_POISON` sweep runs only the interpreter, and every defect below was
//! `--native`-only:
//! - a call handed a POOLED buffer (`@FR-O-Buffer`) adopts it on one pass and mints on the next,
//!   and the rebind freed the displaced store, which was the buffer itself.  The interpreter
//!   already asked whether the displaced store was the buffer; both backends now ask
//!   `Function::displaced_buffer_witness`.
//! - a loop buffer (`@FR-R-LoopBuffer`) was kept across iterations while a local bound from its
//!   field owned the store and freed it at the iteration's end.  `hoist::loop_buffers` now
//!   declines such a buffer.
//! - a local promoted onto the return buffer, rebound from a call through a function
//!   reference, freed the store it displaced, which could be the buffer the caller handed.
//!   The three adopt-path displaced frees now ask the entry witness, as the rebind does.
//!
//! The controls are the shapes one step away, which were right.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

const RB: &str = "struct Rb { name: text, n: integer }\n\
                  fn rb_mk(k: integer) -> Rb { Rb { name: \"r{k}\", n: k } }\n\
                  fn rb_cond(c: integer) -> Rb { \
                  if c > 0 { r = rb_mk(1); if r.n > 99 { return r; } } rb_mk(3) }\n";

/// Run `body` in `mode` with `LOFT_POISON` and either `LOFT_STRICT_STORES` (no slot is ever
/// reused, so a stale read is named) or the plain exit leak check (slots ARE reused, which is
/// the only way to see a free decided by a slot NUMBER that a newer store now carries); answer
/// the `R` line, or what went wrong.
fn run_armed(tag: &str, body: &str, mode: &str, strict: bool) -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "loft_heap_sound_{tag}_{}_{}.loft",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, format!("{body}\n")).expect("write cell");
    let out = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg(mode)
        .arg(&path)
        .env("LOFT_TIMEOUT", "240")
        .env("LOFT_POISON", "1")
        .env(
            if strict {
                "LOFT_STRICT_STORES"
            } else {
                "LOFT_NATIVE_LEAK_CHECK"
            },
            "1",
        )
        .output()
        .expect("run loft");
    let _ = std::fs::remove_file(&path);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let uaf = text.matches("USE AFTER FREE").count()
        + text.matches("out of bounds").count()
        + text.matches("not freed").count();
    let value = text.lines().find(|l| l.starts_with('R')).unwrap_or("");
    format!("{value} exit={} uaf={uaf}", out.status.code().unwrap_or(-1))
}

fn check(cells: &[(&str, String, &str)]) {
    check_armed(cells, true);
}

/// [`check`] with slots reused, for a defect only a recycled slot number can show.
fn check_reusing(cells: &[(&str, String, &str)]) {
    check_armed(cells, false);
}

fn check_armed(cells: &[(&str, String, &str)], strict: bool) {
    let mut wrong = Vec::new();
    for (tag, body, want) in cells {
        let want = format!("{want} exit=0 uaf=0");
        for mode in ["--interpret", "--native"] {
            let got = run_armed(tag, body, mode, strict);
            if got != want {
                wrong.push(format!("{tag} {mode}: got `{got}`, want `{want}`"));
            }
        }
    }
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}

/// A call handed a pooled buffer, in a loop whose passes alternate between adopting the buffer
/// and minting a store of the callee's own.  The rebind must not free the buffer it displaces.
#[test]
fn a_rebind_never_frees_the_pooled_buffer_its_call_was_handed() {
    check(&[
        (
            "alternating",
            format!(
                "{RB}fn main() {{ t = 0; for k in 0..6 {{ t += rb_cond(k % 2).n; }} println(\"R{{t}}\"); }}"
            ),
            "R18",
        ),
        // The same rebind in a generator, whose buffer is a coroutine field.
        (
            "generator",
            format!(
                "{RB}fn gen() -> iterator<integer> {{ for k in 0..6 {{ x = rb_cond(k % 2); yield x.n; }} }}\n\
                 fn main() {{ t = 0; for v in gen() {{ t += v; }} println(\"R{{t}}\"); }}"
            ),
            "R18",
        ),
        // CONTROL: a local declared before the loop and rebound from the same call.
        (
            "declared_outside",
            format!(
                "{RB}fn main() {{ t = 0; x = rb_cond(0); \
                 for k in 0..6 {{ x = rb_cond(k % 2); t += x.n; }} println(\"R{{t}} {{x.name}}\"); }}"
            ),
            "R18 r3",
        ),
        // CONTROL: a vector result, whose buffer the callee does not adopt this way.
        (
            "vector_result",
            "fn vc(c: integer) -> vector<integer> { if c > 0 { r = [1, 2]; \
             if len(r) > 9 { return r; } } [3, 4] }\n\
             fn main() { t = 0; for k in 0..6 { t += vc(k % 2)[1]; } println(\"R{t}\"); }"
                .to_string(),
            "R24",
        ),
    ]);
}

/// A loop buffer whose field is bound to a local that owns the store: a value branch whose other
/// arm mints, in one loop, in nested loops and at two assignment sites.  The buffer must not be
/// kept across iterations, since the local frees its store at the iteration's end.
#[test]
fn a_loop_buffer_is_not_kept_when_a_local_frees_its_store() {
    const CP: &str = "cp = [41, 42, 43]; m = fn(k: integer) -> vector<integer> { [7, 8] }; s = 0;";
    check(&[
        (
            "closure_arm",
            format!(
                "fn main() {{ {CP} for i in 0..6 {{ r = if i % 2 == 0 {{ m(0) }} else {{ cp }}; \
                 s += r[1]; }} println(\"R{{s}} {{cp[1]}}\"); }}"
            ),
            "R150 42",
        ),
        (
            "nested_loops",
            format!(
                "fn main() {{ {CP} for j in 0..2 {{ for i in 0..3 {{ \
                 r = if i % 2 == 0 {{ m(0) }} else {{ cp }}; s += r[1]; }} }} \
                 println(\"R{{s}} {{cp[1]}}\"); }}"
            ),
            "R116 42",
        ),
        (
            "two_sites",
            format!(
                "fn main() {{ {CP} for i in 0..6 {{ r: vector<integer> = []; \
                 if i % 2 == 0 {{ r = m(0); }} else {{ r = cp; }} s += r[1]; }} \
                 println(\"R{{s}} {{cp[1]}}\"); }}"
            ),
            "R150 42",
        ),
        // CONTROLS: a named function's mint and a literal arm, beside the same variable arm.
        (
            "named_fn_arm",
            "fn mk(k: integer) -> vector<integer> { [k, k + 1] }\n\
             fn main() { cp = [41, 42, 43]; s = 0; \
             for i in 0..6 { r = if i % 2 == 0 { mk(7) } else { cp }; s += r[1]; } \
             println(\"R{s} {cp[1]}\"); }"
                .to_string(),
            "R150 42",
        ),
        (
            "literal_arm",
            "fn main() { cp = [41, 42, 43]; s = 0; \
             for i in 0..6 { r = if i % 2 == 0 { [7, 8] } else { cp }; s += r[1]; } \
             println(\"R{s} {cp[1]}\"); }"
                .to_string(),
            "R150 42",
        ),
    ]);
}

/// A local promoted onto the return buffer, rebound from a call through a function reference.
/// The adopt path frees the store the local displaces, and that store may be the buffer the
/// CALLER handed.  It must be guarded by the entry witness (`_rb_w_<name>`), as the rebind's own
/// free is.
#[test]
fn a_displaced_free_never_releases_the_callers_buffer() {
    const CANVAS: &str = "struct Canvas { data: vector<integer>, w: integer }\n\
                          fn lit_canvas(t: integer) -> Canvas { Canvas { data: [t, t + 1], w: t } }\n";
    const MAIN: &str =
        "{ s = 0; for t in 0..3 { m = CALL; s += m.w + m.data[1]; } println(\"R{s}\"); }";
    let main = |call: &str| format!("fn main() {}", MAIN.replace("CALL", call));
    check(&[
        (
            "fn_ref_rebind",
            format!(
                "{CANVAS}fn rf(f: fn(integer) -> Canvas, t: integer) -> Canvas {{ \
                 cv = Canvas {{ data: [], w: 0 }}; cv = f(t); cv }}\n{}",
                main("rf(lit_canvas, t + 4)")
            ),
            "R33",
        ),
        (
            "fn_ref_first_bind",
            format!(
                "{CANVAS}fn rb(f: fn(integer) -> Canvas, t: integer) -> Canvas {{ cv = f(t); cv }}\n{}",
                main("rb(lit_canvas, t + 4)")
            ),
            "R33",
        ),
        // CONTROLS: a closure built in the function, and a named call.
        (
            "closure",
            format!(
                "{CANVAS}fn rc(t: integer) -> Canvas {{ \
                 f = fn(k: integer) -> Canvas {{ Canvas {{ data: [k, k + 1], w: k }} }}; \
                 cv = Canvas {{ data: [], w: 0 }}; cv = f(t); cv }}\n{}",
                main("rc(t + 4)")
            ),
            "R33",
        ),
        (
            "named_call",
            format!(
                "{CANVAS}fn rn(t: integer) -> Canvas {{ \
                 cv = Canvas {{ data: [], w: 0 }}; cv = lit_canvas(t); cv }}\n{}",
                main("rn(t + 4)")
            ),
            "R33",
        ),
    ]);
}

/// `(H-FreeAll)` — nothing is left unfreed at exit.  A generic instance called inline, whose
/// function-reference call hands up a store it minted: the frame above must carry the guard
/// that releases it, which a call-graph walk skipping instances (`i_…`) judged unnecessary.
#[test]
fn a_store_handed_up_through_a_generic_instance_is_released() {
    const P: &str = "struct P { n: integer }\n\
                     fn once<T>(x: T, f: fn(T) -> T) -> T { f(x) }\n\
                     fn once_p(x: P, f: fn(P) -> P) -> P { f(x) }\n";
    let main = |setup: &str, step: &str| {
        format!("{P}fn main() {{ {setup} s = 0; for i in 0..4 {{ {step} }} println(\"R{{s}}\"); }}")
    };
    check(&[
        (
            "capture",
            main(
                "cap = P { n: 7 }; f = fn(v: P) -> P { cap };",
                "s += once(P { n: 41 }, f).n;",
            ),
            "R28",
        ),
        (
            "mint",
            main(
                "f = fn(v: P) -> P { P { n: v.n + 1 } }; \
                 if len(\"ab\") == 2 { f = fn(v: P) -> P { P { n: v.n + 2 } }; }",
                "s += once(P { n: 41 }, f).n;",
            ),
            "R172",
        ),
        // CONTROLS: the concrete twin, and the result bound before it is read.
        (
            "concrete",
            main(
                "cap = P { n: 7 }; f = fn(v: P) -> P { cap };",
                "s += once_p(P { n: 41 }, f).n;",
            ),
            "R28",
        ),
        (
            "bound",
            main(
                "cap = P { n: 7 }; f = fn(v: P) -> P { cap };",
                "r = once(P { n: 41 }, f); s += r.n;",
            ),
            "R28",
        ),
    ]);
}

/// `(H-FreeAll)` with `@FR-O-Buffer` — a vector local promoted onto the return buffer and
/// bound to a call that delivers its own store (`o = text as vector<R>; o`) fills the buffer.
/// Rebound, it handed back the call's store while the caller released only its buffer.
#[test]
fn a_vector_buffer_bound_to_a_forwarding_call_is_filled() {
    const R: &str = "struct R { n: integer }\n\
                     const J: text = \"[{{\\\"n\\\":1}},{{\\\"n\\\":2}}]\";\n";
    let main = |f: &str| {
        format!(
            "{R}{f}\nfn main() {{ t = 0; for i in 0..3 {{ v = f(); t += len(v) + v[1].n; }} \
             println(\"R{{t}}\"); }}"
        )
    };
    check(&[
        (
            "bound_local",
            main("fn f() -> vector<R> { o = J as vector<R>; o }"),
            "R12",
        ),
        (
            "bound_then_appended",
            main("fn f() -> vector<R> { o = J as vector<R>; o += [R { n: 9 }]; o }"),
            "R15",
        ),
        // CONTROLS: the tail spelling, which was already copied in, and a projection out of a
        // call result (chained, as `zt12` c7), which is a view the clear would empty first.
        (
            "tail",
            main("fn f() -> vector<R> { J as vector<R> }"),
            "R12",
        ),
        (
            "projection",
            main(
                "struct I { rs: vector<R> }\nstruct W { inner: I }\n\
                 fn mkw() -> W { W { inner: I { rs: J as vector<R> } } }\n\
                 fn f() -> vector<R> { return mkw().inner.rs; }",
            ),
            "R12",
        ),
    ]);
}

/// `(H-FreeAll)` — a call result bound to a lifted local is released even when nothing reads it.
/// With a value-record function anywhere in the program (`mks`, a flat record returned as a
/// tuple), the dead-buffer judgement counted only a local's MENTIONS and its frees; a `Set`
/// target is no mention, so `__lift_1 = mk()` read as a buffer never minted and its free was
/// emitted as nothing.
#[test]
fn a_discarded_record_result_is_released_beside_a_value_record() {
    const DOC: &str = "struct Doc { buf: vector<integer>, n: integer }\n\
                       struct S { a: integer, b: integer }\n\
                       fn mk() -> Doc { return Doc { buf: [1, 2], n: 1 }; }\n\
                       fn mks() -> S { return S { a: 1, b: 2 }; }\n\
                       fn nd(d: Doc) -> Doc { return Doc { buf: d.buf, n: d.n + 1 }; }\n";
    let main = |body: &str| format!("{DOC}fn main() {{ {body} }}");
    check(&[
        ("fresh", main("mk(); println(\"R{mks().a}\");"), "R1"),
        (
            "derived",
            main("d = mk(); nd(d); println(\"R{d.n}\");"),
            "R1",
        ),
        (
            "loop",
            main("d = mk(); for i in 0..3 { nd(d); } println(\"R{len(d.buf)}\");"),
            "R2",
        ),
        // CONTROLS: the result bound and read, and a flat record discarded.
        (
            "bound",
            main("d = mk(); e = nd(d); println(\"R{e.n}\");"),
            "R2",
        ),
        ("flat", main("mks(); println(\"R{mks().b}\");"), "R2"),
    ]);
}

/// `(H-FreeAll)` with `@FR-O-Witness` — a call that returns its argument is COPIED into the
/// local it binds (`c = id(b)`), and a later rebind of `c` releases that copy.  The oracle read
/// the callee's return as a borrow, so the owner tracker was never pointed at the copy, and the
/// rebind dropped it.
#[test]
fn a_copied_call_result_is_released_when_rebound() {
    const BOX: &str = "struct It { name: text, n: integer }\n\
                       struct HE { k: text, v: integer }\n\
                       struct VB { items: vector<It> }\n\
                       struct HB { by: hash<HE[k]> }\n\
                       fn id_v(b: VB) -> VB { return b; }\n\
                       fn id_h(b: HB) -> HB { return b; }\n";
    let main = |body: &str| format!("{BOX}fn main() {{ {body} }}");
    check(&[
        (
            "vector_field",
            main(
                "b = VB { items: [It { name: \"a\", n: 1 }] }; c = id_v(b); \
                 c = VB { items: [It { name: \"d\", n: 9 }] }; \
                 println(\"R{c.items[0].n} {b.items[0].n}\");",
            ),
            "R9 1",
        ),
        (
            "hash_field",
            main(
                "b = HB { by: [HE { k: \"a\", v: 1 }] }; c = id_h(b); \
                 c = HB { by: [HE { k: \"d\", v: 9 }] }; \
                 println(\"R{c.by[\"d\"].v} {b.by[\"a\"].v}\");",
            ),
            "R9 1",
        ),
        // CONTROLS: the copy never rebound, and a plain whole-value copy rebound.
        (
            "not_rebound",
            main(
                "b = VB { items: [It { name: \"a\", n: 1 }] }; c = id_v(b); \
                 println(\"R{c.items[0].n}\");",
            ),
            "R1",
        ),
        (
            "plain_copy",
            main(
                "b = VB { items: [It { name: \"a\", n: 1 }] }; c = b; \
                 c = VB { items: [It { name: \"d\", n: 9 }] }; println(\"R{c.items[0].n}\");",
            ),
            "R9",
        ),
    ]);
}

/// `(H-FreeAll)` — a store a closure call mints is handed up for its frame to release, and one
/// ALLOCATION is one entry on that list.  The entry a freed store left behind (`k2 = hc(0)`
/// rebound: its owner released the first result) names the same slot as the next result minted
/// into it, and refusing that one on the slot number left it to nobody.  Only a reused slot
/// shows it, so this runs with slots reused.
#[test]
fn a_minted_store_in_a_reused_slot_is_still_released() {
    const P: &str = "struct P { x: integer, y: integer }\n";
    check_reusing(&[
        (
            "rebound_then_called",
            format!(
                "{P}fn main() {{ cap: P? = P {{ x: 11, y: 1 }}; hc = fn(n: integer) -> P? {{ cap }}; \
                 k2: P? = null; k2 = hc(0); k2 = hc(0); \
                 println(\"R{{(k2 ?? P {{ x: 0, y: 0 }}).x}} {{(hc(0) ?? P {{ x: 0, y: 0 }}).x}}\"); }}"
            ),
            "R11 11",
        ),
        // CONTROL: no rebind, so no stale entry shares the slot.
        (
            "called_twice",
            format!(
                "{P}fn main() {{ cap: P? = P {{ x: 11, y: 1 }}; hc = fn(n: integer) -> P? {{ cap }}; \
                 k: P? = null; k = hc(0); \
                 println(\"R{{(k ?? P {{ x: 0, y: 0 }}).x}} {{(hc(0) ?? P {{ x: 0, y: 0 }}).x}}\"); }}"
            ),
            "R11 11",
        ),
    ]);
}
