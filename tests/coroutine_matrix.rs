// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN16 coroutine-validation matrix.
//!
//! Each test exercises one cell of the (yielded-value-type × drive-
//! context) matrix described in
//! `doc/claude/plans/16-coroutine-validation/00-matrix.md`.  Every cell
//! runs under both the interpreter and `--native`; the harness in
//! `tests/common/cross_mode.rs` (shared with `tuple_matrix`,
//! `closure_matrix`, `template_matrix`) asserts byte-identical stdout.
//!
//! **These cells RUN by default** (@PLN114).  They were `#[ignore]`d as too heavy;
//! measured, the whole 201-cell matrix takes ~10 seconds.  The cost was never the
//! issue — the silence was: two SIGSEGVs, a silent `+1` corruption of every narrow
//! tuple element, a 3.4x memory overhead and a cross-backend par-worker corruption
//! all sat behind that attribute until the set was run by hand.
//!
//! ```bash
//! cargo test --release --test coroutine_matrix -- --ignored
//! ```
//!
//! or a single cell:
//!
//! ```bash
//! cargo test --release --test coroutine_matrix -- --ignored y1_x1_int_for_loop
//! ```
//!
//! ## The frozen matrix (phase 00, locked 2026-05-23)
//!
//! Cell legend: `PASS:test_name` / `FIX:phase` / `CLOSED:reason`.
//!
//! | | X1 — for-loop | X2 — manual `next()` | X3 — higher-order | X4 — comprehension | X5 — yield from |
//! |---|---|---|---|---|---|
//! | **Y1** scalar    | FIX:01 | FIX:01 | FIX:01 | FIX:01 | PASS:y1_x5 |
//! | **Y2** text      | FIX:02 | FIX:02 | FIX:02 | FIX:02 | PASS:y2_x5 |
//! | **Y3** Reference | FIX:03 | FIX:03 | FIX:03 | FIX:03 | PASS:y3_x5 |
//! | **Y4** tuple     | FIX:04 | FIX:04 (gated on T1.8a) | FIX:04 | FIX:04 | PASS:y4_x5 |
//! | **Y5** closure   | FIX:05 (depends on @PLAN15) | FIX:05 | FIX:05 | FIX:05 | CLOSED:measured-loft#1676 |
//! | **Y6** vector    | CLOSED:non-goal | CLOSED:non-goal | CLOSED:non-goal | CLOSED:non-goal | CLOSED:non-goal |
//! | **Y7** float     | PASS:y7_x1 | PASS:y7_x2 | ok¹ | ok¹ | PASS:y7_x5 |
//! | **Y8** single    | PASS:y8_x1 | PASS:y8_x2 | ok¹ | ok¹ | PASS:y8_x5 |
//! | **Y9** enum      | PASS:y9_x1 | PASS:y9_x2 | ok¹ | ok¹ | PASS:y9_x5 |
//!
//! ⚠ **The whole X5 column read `CLOSED:CO1.4-deferred` until 2026-09-25**, on the strength of
//! a COROUTINE.md status line that said `yield from` was deferred to 1.1+ — long after it
//! shipped and was guarded (loft#1277).  Nine cells closed by a stale sentence, over a
//! construct carrying three live defects: on `--native` the statements before a `yield from`
//! re-ran on every advance the delegation served (silent — the produced sequence is
//! unchanged), and a `yield from` beside a loop in either lowering did not compile at all.
//! A closed cell is a claim about the LANGUAGE, so it has to be re-read whenever the thing it
//! names moves; nothing here pointed back at the sentence it depended on.
//! Y6×X5 stays closed for Y6's own reason (a yielded vector is a non-goal), which is the only
//! one of the nine that was ever about the row.  Y5×X5 is closed on a MEASUREMENT rather than a
//! deferral: a delegated fn-ref raises `BUG (#306)` twice on the interpreter (and still answers
//! 36) and is refused on native with two rustc errors leaking past the refusal — loft#1676, a
//! different site with its own matrix owed.  Opening this column is what found it; the other
//! seven cells pass on both backends.
//!
//! ¹ float/single/enum × X3 (higher-order) and X4 (comprehension) verified on
//!   both backends via the full type×context grid (2026-06-18, #401); no
//!   dedicated `cross_mode` cell yet.
//!
//! Each `body` must declare `fn test() { … }` (the entry point) and
//! any helper fns at file scope; the harness appends
//! `fn main() { test(); }`.  loft does not allow nested fn
//! definitions, so helpers must live alongside `fn test`.

mod common;

// ── Phase 00 — harness smoke ────────────────────────────────────────────────
//
// One smoke cell that exercises the cross-mode harness with the
// simplest yielded-scalar / for-loop combination — the cheapest path
// through the coroutine state machine.  If this cell ever breaks,
// every other coroutine cell below should be treated as
// "infrastructure regression, not coroutine regression" until the
// smoke is green again.

cross_mode!(
    y1_x1_int_for_loop_smoke,
    r#"
    fn count_up() -> iterator<integer> {
        i = 0;
        while i < 3 {
            yield i;
            i = i + 1;
        }
    }
    fn test() {
        sum = 0;
        for v in count_up() {
            sum = sum + v;
        }
        print("{sum}\n");
        assert(sum == 3, "y1_x1 for-loop sum");
    }
    "#
);

// ── Phase 01 — Y1 (basic scalars) × X1–X4 ──────────────────────────────────

// Y1×X1 — for-loop driving an integer generator that uses `for v in [...]`
// internally.  Covers the P219 path (for-yield inside a generator) end-to-end.
cross_mode!(
    y1_x1_int_for_yield_in_generator,
    r#"
    fn nums() -> iterator<integer> {
        for n in [10, 20, 30] {
            yield n;
        }
    }
    fn test() {
        sum = 0;
        for v in nums() {
            sum = sum + v;
        }
        print("{sum}\n");
        assert(sum == 60, "y1_x1 for-yield sum");
    }
    "#
);

// Y1×X1 — a generator that CONSUMES another generator with a `for`, between two yields of
// its own.  Two frames are live at once and the outer one's resume point sits inside the
// inner loop.
//
// ⚠ Until 2026-09-25 this cell was called `..._generator_with_helper_yield` and its comment
// read *"generator whose body calls a HELPER that yields.  Stackful semantics: the suspended
// frame must capture the helper's local state"* — which is not what it does: `inner` is a
// generator consumed by a `for`, no yield happens at any depth, and no helper is called.  A
// `yield` in a helper that is not itself a generator is REFUSED (`@FR-G-Yield`), so the shape
// the name promised cannot be written at all — and a green cell with that name is exactly why
// `formal/coroutines.md`'s conformance line could claim stackful yield verified on both
// backends for months.  The cell is good; only its name claimed more than it measures.
cross_mode!(
    y1_x1_int_consumes_another_generator,
    r#"
    fn inner() -> iterator<integer> {
        yield 10;
        yield 20;
    }
    fn outer() -> iterator<integer> {
        yield 1;
        for n in inner() {
            yield n;
        }
        yield 2;
    }
    fn test() {
        sum = 0;
        for v in outer() {
            sum = sum + v;
        }
        print("{sum}\n");
        assert(sum == 33, "y1_x1 helper-yield sum");
    }
    "#
);

// Y1×X2 — manual `next()` calls.  Each next() returns one value; after
// all yields plus one EXTRA call (to advance past the last yield),
// exhausted() returns true.  Matches the contract used in
// `tests/scripts/51-coroutines.loft::one_val + 2× next`: a 3-yield
// generator needs 4 next() calls before exhausted() flips.
cross_mode!(
    y1_x2_int_manual_next,
    r#"
    fn count() -> iterator<integer> {
        yield 10;
        yield 20;
        yield 30;
    }
    fn test() {
        it = count();
        a = next(it);
        b = next(it);
        c = next(it);
        next(it);
        done = exhausted(it);
        print("{a},{b},{c},{done}\n");
        assert(a == 10 && b == 20 && c == 30, "y1_x2 values");
        assert(done, "y1_x2 exhausted after extra next");
    }
    "#
);

// Y1×X3 — higher-order: pump generator output through a closure.  The
// idiomatic equivalent of `map(gen, fn)` is a for-loop that calls the
// closure on each yielded value and accumulates the result.
//
// **@P324 FIXED 2026-05-23** — the CO1.9/S28 over-broad guard was
// demoted to a debug-only warning so the for-loop+accumulate idiom no
// longer panics on interp.  Native was always fine.
cross_mode!(
    y1_x3_int_closure_map,
    r#"
    fn count() -> iterator<integer> {
        yield 1;
        yield 2;
        yield 3;
    }
    fn test() {
        f = fn(x: integer) -> integer { x * 10 };
        out: vector<integer> = [];
        for v in count() {
            out += [f(v)];
        }
        print("{out[0]},{out[1]},{out[2]}\n");
        assert(out[0] == 10 && out[1] == 20 && out[2] == 30, "y1_x3 closure mapped");
    }
    "#
);

// Y1×X3 — reduce shape: closure accumulates iterator output.
cross_mode!(
    y1_x3_int_closure_reduce,
    r#"
    fn count() -> iterator<integer> {
        yield 1;
        yield 2;
        yield 3;
        yield 4;
    }
    fn test() {
        add = fn(a: integer, b: integer) -> integer { a + b };
        acc = 0;
        for v in count() {
            acc = add(acc, v);
        }
        print("{acc}\n");
        assert(acc == 10, "y1_x3 reduce sum");
    }
    "#
);

// ── Phase 03 — Y3 (References) × X1–X4 ─────────────────────────────────────
//
// Y3 yields DbRef-bearing struct records.  The driving lifetime risk:
// each yielded struct lives in a STORE that the generator allocated; the
// consumer reads its fields between yields.  S28 guard interplay
// (@P324) is the active hazard.

// Y3×X1 — for-loop over `iterator<CY3Pt>`.  Each yield emits a fresh
// struct record; the consumer reads `pt.x + pt.y` between yields.
//
// **@P326 FIXED 2026-05-23** — native state machine now has a DbRef
// channel: `LoftCoroutine::next_dbref` + `coroutine_next_dbref` runtime
// helper + size=12 dispatch arm + struct-construction work-var
// pre-declaration (`__ref_*` joins `__vdb_*` / `__work_*`).
cross_mode!(
    y3_x1_ref_for_loop,
    r#"
    struct CY3Pt { x: integer not null, y: integer not null }
    fn points() -> iterator<CY3Pt> {
        yield CY3Pt { x: 1, y: 2 };
        yield CY3Pt { x: 3, y: 4 };
        yield CY3Pt { x: 5, y: 6 };
    }
    fn test() {
        sum = 0;
        for pt in points() {
            sum = sum + pt.x + pt.y;
        }
        print("{sum}\n");
        assert(sum == 21, "y3_x1 ref-yield field-sum");
    }
    "#
);

// ── Phase 04 — Y4 (tuples) × X1–X4 ─────────────────────────────────────────
//
// Tuple yields stress the state machine's value-marshalling: each yielded
// tuple is multi-component and must be reconstructed on the consumer side.

// Y4×X1 — for-loop over `iterator<(integer, integer)>`.
//
// **@P327 FIXED 2026-05-23 (both backends).**  Interp side: for-loop
// uses `OpCoroutineExhausted` (via `iter_for` tuple-coroutine branch).
// Native side: unified `next_into(stores, &mut [i64])` channel from
// plan-16 phase 01 — `value_size`'s high byte distinguishes tuple
// yields from text (both 16 bytes); the consumer allocates a stack
// `[i64; N]` buffer and rebuilds the tuple from buffer slots.
cross_mode!(
    y4_x1_tuple_for_loop,
    r#"
    fn pairs() -> iterator<(integer, integer)> {
        yield (1, 10);
        yield (2, 20);
        yield (3, 30);
    }
    fn test() {
        sum = 0;
        for p in pairs() {
            sum = sum + p.0 + p.1;
        }
        print("{sum}\n");
        assert(sum == 66, "y4_x1 tuple for-loop sum");
    }
    "#
);

// Y4×X2 — manual next() on a tuple-yielding generator.
cross_mode!(
    y4_x2_tuple_manual_next,
    r#"
    fn pairs() -> iterator<(integer, integer)> {
        yield (10, 20);
        yield (30, 40);
    }
    fn test() {
        it = pairs();
        a = next(it);
        b = next(it);
        print("{a.0},{a.1},{b.0},{b.1}\n");
        assert(a.0 == 10 && a.1 == 20, "y4_x2 first");
        assert(b.0 == 30 && b.1 == 40, "y4_x2 second");
    }
    "#
);

// Y3×X2 — manual next() on a struct-yielding generator.  @P326 FIXED.
cross_mode!(
    y3_x2_ref_manual_next,
    r#"
    struct CY3Pt { x: integer not null, y: integer not null }
    fn points() -> iterator<CY3Pt> {
        yield CY3Pt { x: 10, y: 20 };
        yield CY3Pt { x: 30, y: 40 };
    }
    fn test() {
        it = points();
        a = next(it);
        b = next(it);
        print("{a.x},{a.y},{b.x},{b.y}\n");
        assert(a.x == 10 && a.y == 20, "y3_x2 first");
        assert(b.x == 30 && b.y == 40, "y3_x2 second");
    }
    "#
);

// ── Phase 02 — Y2 (text) × X1–X4 ───────────────────────────────────────────
//
// Active risk per @PLAN16 README: yielded text's backing String lives on
// the suspended frame, not on the consumer's stack.  Each cell here pins
// the lifetime contract on one drive context.

// Y2×X1 — for-loop over a text generator.  The simplest text-yield shape:
// each `yield "literal"` produces a static-text element; the for-loop
// consumes them in order.  Closed @P211 pinned this in `tests/issues.rs`;
// this cell is the matrix-shaped regression carrier.
cross_mode!(
    y2_x1_text_for_loop,
    r#"
    fn names() -> iterator<text> {
        yield "alice";
        yield "bob";
        yield "carol";
    }
    fn test() {
        joined = "";
        for s in names() {
            joined = joined + s + ",";
        }
        print("{joined}\n");
        assert(joined == "alice,bob,carol,", "y2_x1 text concat");
    }
    "#
);

// Y2×X1 — text yielded by a `while` loop with a captured parameter.
// Format-string interpolation reads the parameter on each yield.  Closed
// @P218 pinned the work-buffer scoping; this cell is the matrix carrier.
cross_mode!(
    y2_x1_text_format_with_param,
    r#"
    fn make_labels(label: text, n: integer) -> iterator<text> {
        i = 0;
        while i < n {
            yield "{label}={i}";
            i = i + 1;
        }
    }
    fn test() {
        total = 0;
        for s in make_labels("x", 3) {
            total = total + s.len();
        }
        print("{total}\n");
        assert(total == "x=0".len() + "x=1".len() + "x=2".len(), "y2_x1 format-with-param len");
    }
    "#
);

// Y2×X2 — manual `next()` on a text generator.
cross_mode!(
    y2_x2_text_manual_next,
    r#"
    fn names() -> iterator<text> {
        yield "alice";
        yield "bob";
    }
    fn test() {
        it = names();
        a = next(it);
        b = next(it);
        next(it);
        done = exhausted(it);
        print("{a},{b},{done}\n");
        assert(a == "alice", "y2_x2 first");
        assert(b == "bob", "y2_x2 second");
        assert(done, "y2_x2 exhausted");
    }
    "#
);

// Y2×X3 — text yielded into a closure-driven map.  Builds a single text
// accumulator (no store mutation between yields once the closure is
// applied — the `+ s` concat returns a new String each iteration).
// NOTE: this cell may trip @P324 too if the text concat backing store
// is mutated between yields — file the divergence if it surfaces.
cross_mode!(
    y2_x3_text_closure_chain,
    r#"
    fn names() -> iterator<text> {
        yield "a";
        yield "b";
        yield "c";
    }
    fn test() {
        wrap = fn(s: text) -> text { "[" + s + "]" };
        acc = "";
        for s in names() {
            acc = acc + wrap(s);
        }
        print("{acc}\n");
        assert(acc == "[a][b][c]", "y2_x3 closure-chain");
    }
    "#
);

// Y2×X4 — text comprehension `[for s in gen() { transform(s) }]`.
// **@P325 FIXED 2026-05-23** (same shape as Y1×X4 above).
cross_mode!(
    y2_x4_text_comprehension,
    r#"
    fn names() -> iterator<text> {
        yield "alice";
        yield "bob";
    }
    fn test() {
        out = [for s in names() { "<" + s + ">" }];
        print("{out[0]},{out[1]}\n");
        assert(out[0] == "<alice>" && out[1] == "<bob>", "y2_x4 text comprehension");
    }
    "#
);

// Y1×X4 — comprehension `[for v in gen() { expr }]` over a generator.
// Builds a vector from the iterator's elements via loft's comprehension
// syntax (LOFT.md § Comprehensions).
//
// **@P325 FIXED 2026-05-23** — comprehensions over coroutines now emit
// an `OpCoroutineExhausted(__gen_N)` break check (mirror of the @P327
// fix for the for-loop driver).  Without this, the loop ran forever
// appending until the store overflowed its 2 GiB word limit
// (interp panic via `store.rs:643`; native = same path).
cross_mode!(
    y1_x4_int_comprehension,
    r#"
    fn count() -> iterator<integer> {
        yield 1;
        yield 2;
        yield 3;
    }
    fn test() {
        out = [for v in count() { v * 100 }];
        print("{out[0]},{out[1]},{out[2]}\n");
        assert(out[0] == 100 && out[1] == 200 && out[2] == 300, "y1_x4 comprehension");
    }
    "#
);

// ── Phase 05 — Y5 (closures) × X1–X2 ───────────────────────────────────────
//
// @P328 — closure-yielding generators: native via the unified `next_into`
// channel (tag 2 = `(u32, DbRef)` rebuild); interp via the fn-ref-yield
// rewrite at parse time (bare `Value::Int(d_nr)` → `Value::FnRef(d, MAX, _)`
// + null-DbRef sentinel emit when `clos_var == u16::MAX`).

cross_mode!(
    y5_x1_closure_for_loop_noncapturing,
    r#"
    fn fns() -> iterator<fn(integer) -> integer> {
        yield fn(x: integer) -> integer { x * 10 };
        yield fn(x: integer) -> integer { x + 100 };
    }
    fn test() {
        total = 0;
        for f in fns() {
            total = total + f(7);
        }
        print("{total}\n");
        assert(total == 177, "y5_x1 closure for-loop (non-cap)");
    }
    "#
);

cross_mode!(
    y5_x2_closure_manual_next_capturing,
    r#"
    fn fns(base: integer) -> iterator<fn(integer) -> integer> {
        yield fn(x: integer) -> integer { x + base };
    }
    fn test() {
        it = fns(100);
        f = next(it);
        result = f(5);
        print("{result}\n");
        assert(result == 105, "y5_x2 closure manual next (capturing)");
    }
    "#
);

// ── #401 — Y7/Y8/Y9 (float / single / enum) × X1 ──────────────────────────
//
// These element types were absent from the frozen matrix above.  Their null
// sentinel (NaN for float/single, the enum sentinel) differs from the i64
// transport's exhaustion sentinel, so the value-sentinel termination path
// (`convert(value, Boolean)` → `OpNot`) never fired: the interp codegen of the
// doomed check spun forever (12+ min hang), and native mis-typed the i64
// channel as f64 (E0308).  Fixed by routing EVERY coroutine loop through the
// state-based `OpCoroutineExhausted`, plus tagged native value channels
// (3 = f64::from_bits, 4 = f32::from_bits, 5 = enum-as-uN) with matching
// producer `to_bits` writes.  See gh #401.

cross_mode!(
    y7_x1_float_for_loop,
    r#"
    fn vals() -> iterator<float> { yield 1.5; yield 2.5; yield 3.0; }
    fn test() {
        sum = 0.0;
        for v in vals() { sum = sum + v; }
        print("{sum}\n");
        assert(sum == 7.0, "y7_x1 float for-loop sum");
    }
    "#
);

cross_mode!(
    y8_x1_single_for_loop,
    r#"
    fn vals() -> iterator<single> { yield 1.5 as single; yield 2.5 as single; }
    fn test() {
        sum = 0.0 as single;
        for v in vals() { sum = sum + v; }
        print("{sum}\n");
        assert(sum == (4.0 as single), "y8_x1 single for-loop sum");
    }
    "#
);

cross_mode!(
    y9_x1_enum_for_loop,
    r#"
    enum Color { Red, Green, Blue }
    fn vals() -> iterator<Color> { yield Color.Red; yield Color.Blue; }
    fn test() {
        count = 0;
        last = Color.Red;
        for v in vals() { count = count + 1; last = v; }
        print("{count}\n");
        assert(count == 2, "y9_x1 enum for-loop count");
        assert(last == Color.Blue, "y9_x1 enum for-loop last value");
    }
    "#
);

// ── X5 — `yield from` (delegation) ──────────────────────────────────────────
//
// `@FR-G-Delegate`.  One column per yielded type, all of them consuming a delegating
// generator to exhaustion, because the channel a delegated value rides is the SAME one a
// direct yield rides (`coroutine_layout::channel_tag`) and a column that only ever passed
// integers would say nothing about the tagged float / single / enum channels #401 added.
//
// Each cell puts a STATEMENT before the `yield from` and checks its effect is counted once.
// That is the part `--native` got wrong: the delegation took a single state and had to stay
// in it to keep pulling, so re-entering it re-ran everything above it in the segment.  The
// value cells and the compile cells live in `tests/scripts/a-delegation*.loft`; these are the
// per-TYPE crossing those two files deliberately hold fixed.

cross_mode!(
    y1_x5_int_yield_from,
    r#"
    fn inner() -> iterator<integer> { yield 10; yield 20; }
    fn outer(lg: reference<Lg>) -> iterator<integer> {
        lg.n = lg.n + 1;
        yield from inner();
        yield 30;
    }
    struct Lg { n: integer }
    fn test() {
        lg = Lg { n: 0 };
        sum = 0;
        for v in outer(lg) { sum = sum + v; }
        print("{sum}\n");
        assert(sum == 60, "y1_x5 delegated integer sum: {sum}");
        assert(lg.n == 1, "y1_x5 the prefix runs once per activation: {lg.n}");
    }
    "#
);

cross_mode!(
    y2_x5_text_yield_from,
    r#"
    fn inner() -> iterator<text> { yield "a"; yield "b"; }
    fn outer() -> iterator<text> {
        s = "c";
        yield from inner();
        yield s;
    }
    fn test() {
        out = "";
        for v in outer() { out += v; }
        print("{out}\n");
        assert(out == "abc", "y2_x5 delegated text: {out}");
    }
    "#
);

cross_mode!(
    y3_x5_ref_yield_from,
    r#"
    struct Pt { x: integer }
    fn inner() -> iterator<Pt> { yield Pt { x: 1 }; yield Pt { x: 2 }; }
    fn outer() -> iterator<Pt> {
        yield from inner();
        yield Pt { x: 3 };
    }
    fn test() {
        sum = 0;
        for p in outer() { sum = sum + p.x; }
        print("{sum}\n");
        assert(sum == 6, "y3_x5 delegated record sum: {sum}");
    }
    "#
);

cross_mode!(
    y4_x5_tuple_yield_from,
    r#"
    fn inner() -> iterator<(integer, integer)> { yield (1, 2); yield (3, 4); }
    fn outer() -> iterator<(integer, integer)> {
        yield from inner();
        yield (5, 6);
    }
    fn test() {
        sum = 0;
        for t in outer() { sum = sum + t.0 * 10 + t.1; }
        print("{sum}\n");
        assert(sum == 102, "y4_x5 delegated tuple sum: {sum}");
    }
    "#
);

cross_mode!(
    y7_x5_float_yield_from,
    r#"
    fn inner() -> iterator<float> { yield 1.5; yield 2.5; }
    fn outer() -> iterator<float> {
        yield from inner();
        yield 3.0;
    }
    fn test() {
        sum = 0.0;
        for v in outer() { sum = sum + v; }
        print("{sum}\n");
        assert(sum == 7.0, "y7_x5 delegated float sum: {sum}");
    }
    "#
);

cross_mode!(
    y8_x5_single_yield_from,
    r#"
    fn inner() -> iterator<single> { yield 1.5 as single; yield 2.5 as single; }
    fn outer() -> iterator<single> {
        yield from inner();
        yield 4.0 as single;
    }
    fn test() {
        sum = 0.0 as single;
        for v in outer() { sum = sum + v; }
        print("{sum}\n");
        assert(sum == (8.0 as single), "y8_x5 delegated single sum");
    }
    "#
);

cross_mode!(
    y9_x5_enum_yield_from,
    r#"
    enum Shade { Dark, Mid, Light }
    fn inner() -> iterator<Shade> { yield Shade.Dark; yield Shade.Mid; }
    fn outer() -> iterator<Shade> {
        yield from inner();
        yield Shade.Light;
    }
    fn test() {
        count = 0;
        last = Shade.Dark;
        for v in outer() { count = count + 1; last = v; }
        print("{count}\n");
        assert(count == 3, "y9_x5 delegated enum count: {count}");
        assert(last == Shade.Light, "y9_x5 delegated enum last value");
    }
    "#
);

// X2 — manual `next()`.  #401 follow-up: the manual-next codegen path
// (control.rs) duplicated the channel decision and dropped float/single/enum,
// so `let a: f64 = next(g)` was native E0308.  Now both paths share
// `coroutine_layout::channel_tag`.

cross_mode!(
    y7_x2_float_manual_next,
    r#"
    fn vals() -> iterator<float> { yield 1.5; yield 2.5; }
    fn test() {
        g = vals();
        a = next(g);
        b = next(g);
        print("{a} {b}\n");
        assert(a == 1.5 && b == 2.5, "y7_x2 float manual next");
    }
    "#
);

cross_mode!(
    y8_x2_single_manual_next,
    r#"
    fn vals() -> iterator<single> { yield 1.5 as single; yield 2.5 as single; }
    fn test() {
        g = vals();
        a = next(g);
        b = next(g);
        print("{a} {b}\n");
        assert(a == (1.5 as single) && b == (2.5 as single), "y8_x2 single manual next");
    }
    "#
);

cross_mode!(
    y9_x2_enum_manual_next,
    r#"
    enum Color { Red, Green, Blue }
    fn vals() -> iterator<Color> { yield Color.Red; yield Color.Blue; }
    fn test() {
        g = vals();
        a = next(g);
        b = next(g);
        print("{a == Color.Red} {b == Color.Blue}\n");
        assert(a == Color.Red && b == Color.Blue, "y9_x2 enum manual next");
    }
    "#
);

/// The Coroutines chapter promises that "the abandoned generator frame is freed
/// automatically", and until this test nothing could witness it. Coroutine frames live in a
/// side-table on `State` (`coroutines: Vec<Option<Box<CoroutineFrame>>>`), deliberately
/// outside the store system — so `check_store_leaks` is structurally blind to a leaked frame
/// and its silence proves nothing about this claim.
///
/// The witness is indirect and exact: give the generator a store-backed local, then abandon
/// the generator N times and read `LOFT_ALLOC_REPORT`. A frame that is not reclaimed keeps
/// that local's store alive, so the property is not a threshold but an INVARIANCE — `peak`
/// (maximum concurrently-live stores) must not move with N, while `allocs` (alloc/free cycles)
/// must scale with it. Those two together are what separate "every frame gave its store back"
/// from "the program happened to allocate a lot": a threshold on `peak` alone would pass on a
/// leak whose stores were pooled, and an exit-time leak count cannot see growth at all.
///
/// Measured: `peak=3` at 50, 200 and 800 rounds, with `allocs` at 52, 202 and 802.
///
/// And the channel is known to move, which is what keeps the invariance from being vacuous:
/// the same generator nested three deep, so three frames are live at once, reads `peak=5`
/// against the `peak=3` of the create-and-abandon loop. Each concurrently-live frame is worth
/// one live store here, so a frame retained per round would track the round count exactly.
#[test]
fn an_abandoned_generator_frame_does_not_accumulate() {
    // One store-backed local (`buf`) inside the generator, abandoned after three values.
    let source = |rounds: usize| {
        format!(
            "\
fn gen_with_vec(limit: integer) -> iterator<integer> {{
  buf = [1, 2, 3];
  for i in 1..=limit {{ yield i + len(buf); }}
}}
fn main() {{
  total = 0;
  for round in 0..{rounds} {{
    seen = 0;
    for n in gen_with_vec(1000) {{ seen += 1; total += n; if seen >= 3 {{ break; }} }}
  }}
  print(\"{{total}}\\n\");
}}
"
        )
    };

    // (peak, allocs) for one round count.
    let measure = |rounds: usize| -> (usize, usize) {
        let script = std::env::temp_dir().join(format!("loft_coro_frame_reclaim_{rounds}.loft"));
        std::fs::write(&script, source(rounds)).expect("write temp script");
        let out = std::process::Command::new(std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft")))
            .arg("--interpret")
            .arg(&script)
            .env("LOFT_ALLOC_REPORT", "1")
            .output()
            .expect("failed to invoke loft binary");
        let _ = std::fs::remove_file(&script);
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();

        // Each round takes the same three values (4 + 5 + 6 = 15). Checking the total keeps a
        // run that produced nothing from making every assertion below vacuous.
        assert_eq!(
            stdout.trim(),
            (rounds * 15).to_string(),
            "each of the {rounds} rounds must take three values; stderr={stderr:?}"
        );

        let line = stderr
            .lines()
            .find(|l| l.starts_with("loft-alloc:"))
            .unwrap_or_else(|| panic!("LOFT_ALLOC_REPORT produced no report; stderr={stderr:?}"));
        let field = |name: &str| -> usize {
            line.split_whitespace()
                .find_map(|t| t.strip_prefix(name)?.parse().ok())
                .unwrap_or_else(|| panic!("no `{name}` in {line:?}"))
        };
        (field("peak="), field("allocs="))
    };

    let (peak_small, allocs_small) = measure(50);
    let (peak_large, allocs_large) = measure(800);

    // The generator really is built and torn down once per round — otherwise the invariance
    // below would be about a loop that never allocated.
    assert!(
        allocs_large >= allocs_small + 700,
        "each round must allocate: allocs went {allocs_small} -> {allocs_large} for 50 -> 800          rounds"
    );
    // The claim itself. Sixteen times the abandoned generators must not raise the live-store
    // watermark by even one: a frame retained per round would put `peak_large` in the hundreds.
    assert_eq!(
        peak_small, peak_large,
        "an abandoned frame must give its store back, so the live-store peak cannot move with          the number of abandoned generators: peak {peak_small} at 50 rounds, {peak_large} at 800"
    );
}
