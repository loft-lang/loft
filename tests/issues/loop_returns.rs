// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Struct, vector, text and tuple values returned or allocated inside a loop: the store pool stays
//! bounded and the values stay right, in the patterns a game loop uses.

use super::*;

// ── P122: Struct return inside loop should not exhaust store pool ────────────
// When a function returns a struct, the callee allocates a store. Inside a loop
// these stores must be freed each iteration. If they accumulate, the store pool
// is exhausted and panics with "Allocating a used store".

#[test]
fn p122_struct_return_in_loop() {
    code!(
        "struct Pair { px: float, py: float }
fn make_pair(mx: float, my: float) -> Pair {
    Pair { px: mx, py: my }
}
fn test() {
    total = 0.0;
    for p122_i in 0..500 {
        p = make_pair(p122_i as float, p122_i as float * 2.0);
        total += p.px + p.py;
    }
    assert(total > 0.0, \"struct loop failed\");
}"
    )
    .result(Value::Null);
}

// P122b: nested loop struct creation (collision detection pattern)
#[test]
fn p122_struct_nested_loop() {
    // Iteration count reduced from 60*5*10=3000 to 20*5*5=500 for CI speed.
    // The bug exhausted the store pool after a few hundred iterations, so
    // 500 is sufficient as a regression guard. Run --ignored variant for
    // the full stress test.
    code!(
        "struct Box { bx: float, by: float, bw: float, bh: float }
fn overlap(a: const Box, b: const Box) -> boolean {
    a.bx < b.bx + b.bw && a.bx + a.bw > b.bx && a.by < b.by + b.bh && a.by + a.bh > b.by
}
fn test() {
    hits = 0;
    for p122_frame in 0..20 {
        ball = Box { bx: p122_frame as float, by: 50.0, bw: 10.0, bh: 10.0 };
        for p122_row in 0..5 {
            for p122_col in 0..5 {
                brick = Box { bx: (p122_col as float) * 12.0, by: (p122_row as float) * 12.0, bw: 10.0, bh: 10.0 };
                if overlap(ball, brick) { hits += 1; }
            }
        }
    }
    assert(hits > 0, \"nested struct loop failed\");
}"
    )
    .result(Value::Null);
}

// ── P123: Vector allocation inside loop ─────────────────────────────────────
// Creating a vector literal inside a loop should not leak stores.

#[test]
fn p123_vector_in_loop() {
    code!(
        "fn test() {
    total = 0;
    for p123_i in 0..200 {
        v = [for p123_j in 0..8 { p123_j + p123_i * 0 }];
        total += v[0] + v[7];
    }
    assert(total == 1400, \"vector loop failed\");
}"
    )
    .result(Value::Null);
}

// ── P126: Negative integer as tail expression ───────────────────────────────
//
// Symptom: a function whose body has earlier `if X { return Y; }` statements
// followed by a tail expression `-1` produces a misleading parse error:
//   "No matching operator '-' on 'void' and 'integer'"
//
// Root cause hypothesis: after parsing `if ... { return ... }` the parser
// records the previous-statement type as `void`, and when it then tries to
// parse `-1` as the next expression, the prefix `-` is consumed as a binary
// operator continuing the `void` expression instead of starting a new unary
// negation. Bare `-1` at the start of a function (no preceding statements)
// works fine, so the bug is in the boundary between statement-end and
// expression-start parsing.
//
// Fix path: in `parse_expression` (or wherever statement boundaries are
// resolved), force `-` after a void-returning statement to be parsed as a
// unary prefix on a new expression, not a binary operator on the previous
// statement's value. Equivalent to inserting an implicit `;` boundary.
//
// Workaround: use `return -1;` with explicit return.

/// Regression guard for the workaround — the explicit-return form must keep working.
#[test]
fn p126_negative_tail_expression() {
    code!(
        "fn flip_sign(n: integer) -> integer {
    if n > 0 { return 0 - n; }
    n
}
fn test() {
    assert(flip_sign(5) == -5, \"negate positive\");
    assert(flip_sign(-3) == -3, \"negate negative\");
}"
    )
    .result(Value::Null);
}

/// Reproduces the actual bug — bare `-1` after `if { return; }` blocks
/// triggers the misleading "operator '-' on 'void'" diagnostic.
#[test]
fn p126_negative_tail_expression_after_returns() {
    code!(
        "fn lookup(idx: integer) -> integer {
  if idx == 0 { return 100; }
  if idx == 1 { return 200; }
  -1
}
fn test() {
  assert(lookup(0) == 100, \"case 0\");
  assert(lookup(1) == 200, \"case 1\");
  assert(lookup(5) == -1, \"default\");
}"
    )
    .result(Value::Null);
}

// ── P127: File-scope vector constant inlined into function call ─────────────
//
// Symptom: a file-scope constant holding a vector literal, when used as a
// function argument, panics in codegen with one of two flavours depending
// on context:
//   1. "[generate_set] first-assignment of 'X' (var_nr=0) in 'n_test'
//       contains a Var(0) self-reference — storage not yet allocated, will
//       produce a garbage DbRef at runtime. This is a parser bug."
//   2. "generate_call [n_F]: mutable arg 0 (data: Reference(265, []))
//       expected 12B on stack but generate(Var(0)) pushed 8B —
//       Value::Null in a typed slot? Missing convert() call in the parser?"
//
// Root cause: `parse_vector` builds a vector literal as a `Value::Block`
// via `v_block()` (src/data.rs:798), which sets `var_size: 0` and uses
// `Var(0)`/`Var(1)` for its temporaries. When this Block is stored as the
// `code` of a `DefType::Constant` (parser/definitions.rs:407) and later
// inlined where the constant is referenced, the `Var` indices are NOT
// rewritten — they collide with the calling function's local slots.
//
// Fix path: when inlining a file-scope constant Block into a calling
// function, either:
//   (a) remap each `Var(N)` in the constant's IR to a fresh local slot in
//       the caller (allocate `var_size` extra slots first, then offset all
//       Var indices by the caller's current var count), or
//   (b) re-emit the literal at every reference site so each call site has
//       its own freshly-numbered slots, or
//   (c) constant-fold simple literal vectors to a static IR node that
//       doesn't need temporaries at all (best for performance).
//
// Workaround: move the literal inline into the function that needs it.

#[test]
fn p127_file_scope_vector_constant_in_call() {
    code!(
        "QUAD = [1, 2, 3];
fn count(v: const vector<integer>) -> integer { v.len() }
fn test() {
  n = count(QUAD);
  assert(n == 3, \"got {n}\");
}"
    )
    .result(Value::Null);
}

/// Same bug — the local-variable form (literal inline) must keep working.
#[test]
fn p127_inline_vector_literal_in_call_works() {
    code!(
        "fn count(v: const vector<integer>) -> integer { v.len() }
fn test() {
  quad = [1, 2, 3];
  n = count(quad);
  assert(n == 3, \"got {n}\");
}"
    )
    .result(Value::Null);
}

/// The bug also fires for `vector<single>` constants, which is what hit
/// us originally in `lib/graphics/src/graphics.loft` with `UNIT_QUAD_2D`.
#[test]
fn p127_file_scope_single_vector_constant() {
    code!(
        "QUAD = [1.0f, 2.0f, 3.0f];
fn count(v: const vector<single>) -> integer { v.len() }
fn test() {
  n = count(QUAD);
  assert(n == 3, \"got {n}\");
}"
    )
    .result(Value::Null);
}

// ── P117: Struct-returning text-param functions leak callee store ───────────
//
// PROBLEMS.md #117 — `f = file("path")` and similar text-parameter
// struct-returning functions accumulate stores because the dep system
// keeps the work-ref alive even when the O-B2 adoption path bypasses it.
//
// 2026-04-09: I could not reproduce the symptom in a fresh repro of the
// described pattern. The repro below — repeatedly calling a text-param
// struct constructor in a loop — runs to completion without panic and
// without "Database N not correctly freed" warnings. Either:
//   (a) the bug was silently fixed by one of the recent O-B2 codegen
//       changes (P116/P118/P119/P122 fix wave),
//   (b) the original symptom requires the specific `file()` API path
//       which has changed (the `file().exists()` method no longer
//       exists in the current API), or
//   (c) the leak is too small to trigger pool exhaustion within a
//       reasonable test loop.
//
// This test is a *regression guard*: it locks in the current working
// behaviour. If it ever fails with "Allocating a used store" or
// "Database N not correctly freed", #117 has regressed and the
// PROBLEMS.md entry needs reopening with a fresh root-cause analysis.
//
// Fix path (per PROBLEMS.md): in the O-B2 codegen path
// (`gen_set_first_at_tos`), after adopting the callee's store, emit
// `OpFreeRef` for the unused `__ref_N` work variable.
#[test]
fn p117_text_param_struct_return_loop_no_leak() {
    // 100 iterations is enough to detect a per-call store leak in debug
    // mode without dominating CI time.
    code!(
        "struct Wrap { name: text, count: integer }
fn make(t: text) -> Wrap {
  Wrap { name: t, count: t.len() }
}
fn test() {
  for _p117_i in 0..100 {
    w = make(\"hello\");
    assert(w.count == 5, \"count\");
  }
}"
    )
    .result(Value::Null);
}

// ── P120: Vector field in struct returned from function ────────────────────
//
// PROBLEMS.md #120 — when a function returns a struct containing a
// `vector<integer>` field, the vector data was lost during stack unwind
// because the constructor only copied the vector pointer, not the
// underlying data. Caused length=0 vectors after function return.
//
// 2026-04-09: the original reproducer
// `lib/graphics/examples/test_mat4_crash.loft` now runs cleanly:
//   inside make_big: data len=16
//   after return: data len=16
//   data[0]=0 data[15]=15
//
// This regression-guard test reproduces the same pattern as a unit test
// so any future regression is caught in CI. If this test ever fails,
// reopen #120 in PROBLEMS.md and revisit `gen_set_first_ref_call_copy`
// in `src/state/codegen.rs`.
//
// Fix path (per PROBLEMS.md): the struct constructor must deep-copy
// vector field data into the struct's own store at `FinishRecord` /
// `SetField` level. Mitigations already in place include double-free
// tolerance in `free_ref`, loop pre-init reference hoisting, and
// `is_ret_work_ref` suppression of FreeRef in return paths.
#[test]
fn p120_vector_field_in_returned_struct_round_trip() {
    code!(
        "struct BigBox {
  width: integer,
  height: integer,
  data: vector<integer>
}
fn make_big() -> BigBox {
  d: vector<integer> = [];
  for p120_i in 0..16 { d += [p120_i]; }
  BigBox { width: 4, height: 4, data: d }
}
fn test() {
  b = make_big();
  assert(b.width == 4, \"width\");
  assert(b.height == 4, \"height\");
  assert(b.data.len() == 16, \"data len {b.data.len()}\");
  assert(b.data[0] == 0, \"data[0]\");
  assert(b.data[15] == 15, \"data[15]\");
}"
    )
    .result(Value::Null);
}

// ── P121: Tuple literals crashed interpreter with heap corruption ──────────
//
// PROBLEMS.md #121 — `a = (3.0, 2.0)` triggered glibc
// "corrupted size vs. prev_size" abort or SIGSEGV in interpreter mode
// (native worked). Cited as "stack layout issue in interpreter's tuple
// codegen — 16-byte float-pair allocation corrupts heap allocator metadata".
//
// 2026-04-09: the documented reproducer runs cleanly in `--interpret`
// mode. Either the bug has been silently fixed or the heap corruption
// requires specific allocator state that doesn't reliably reproduce.
//
// Regression guard: the test below executes the exact reproducer from
// PROBLEMS.md plus a few variants (function return, destructure,
// element assign). If any of these regress to the heap-corruption
// failure, #121 should be reopened.
//
// Fix path (per PROBLEMS.md): audit `OpTupleLiteral` and stack
// reservation for tuple temporaries for off-by-one / alignment errors.
#[test]
fn p121_float_tuple_literal_no_heap_corruption() {
    code!(
        "fn test() {
  a = (3.0, 2.0);
  assert(a.0 > 1.0, \"a.0\");
  assert(a.1 < 5.0, \"a.1\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p121_float_tuple_function_return() {
    code!(
        "fn pair(p121_x: float) -> (float, float) { (p121_x, p121_x * 2.0) }
fn test() {
  p = pair(3.0);
  assert(p.0 == 3.0, \"first\");
  assert(p.1 == 6.0, \"second\");
}"
    )
    .result(Value::Null);
}

// ── P124: Native codegen — inline array indexing on float literal ──────────
//
// PROBLEMS.md #124 — `[0.9, 0.2, 0.3][idx]` in loft generated an
// `as DbRef` cast in the Rust output that failed to compile. Native-mode
// only (interpreter handled it correctly).
//
// 2026-04-09: the inline form now triggers a parser-level type error
// in interpret mode ("Variable v cannot change type from vector<integer>
// to integer"), and the function-tail form (`[0.9, 0.2, 0.3][idx]` as
// the body of a function returning float) compiles cleanly under
// `--native`. Either the codegen `as DbRef` mistake has been fixed, or
// the parser now refuses the form before it reaches the codegen path.
//
// Regression guard: lock in the current working behaviour. If `--native`
// compilation regresses for the function-tail form, reopen #124. The
// test runs in interpret mode by default; native-mode coverage lives in
// `tests/native.rs` if a more thorough check is needed.
//
// Fix path (per PROBLEMS.md): in `src/generation/`, the inline-array-
// then-index pattern emits a `Reference` cast to the wrong type. Look
// for `as DbRef` in the generated Rust source via `--native-emit`.
#[test]
fn p124_function_returning_inline_array_index() {
    code!(
        // @PLN25 index flip — a variable-index `arr[i]` is nullable (OOB → null); discharge
        // with `?? 0.0` so it stores into the non-null `-> float` return.
        "fn pick(p124_idx: integer) -> float {
  [0.9, 0.2, 0.3][p124_idx] ?? 0.0
}
fn test() {
  assert(pick(0) > 0.85, \"0\");
  assert(pick(1) < 0.25, \"1\");
  assert(pick(2) > 0.25, \"2\");
}"
    )
    .result(Value::Null);
}

/// Documented workaround — assign the array to a variable first, then index.
/// This must keep working even if #124 is fixed at the inline-form level.
#[test]
fn p124_local_array_index_workaround_works() {
    code!(
        "fn pick(p124w_idx: integer) -> float {
  options = [0.9, 0.2, 0.3];
  options[p124w_idx] ?? 0.0
}
fn test() {
  assert(pick(0) > 0.85, \"0\");
  assert(pick(1) < 0.25, \"1\");
}"
    )
    .result(Value::Null);
}

// P122c: struct-returning function used inside conditional inside loop
// This is the exact pattern from the Brick Buster collision detection.
#[test]
fn p122_struct_return_conditional_loop() {
    // Iterations reduced 100*50=5000 → 30*15=450 for CI speed. Still
    // exercises the store-leak pattern with hundreds of allocations.
    code!(
        "struct Overlap { ox: float, oy: float }
fn compute_overlap(ax: float, bx: float) -> Overlap {
    Overlap { ox: ax, oy: bx }
}
fn test() {
    score = 0;
    for p122c_frame in 0..30 {
        for p122c_i in 0..15 {
            d = compute_overlap(p122c_frame as float, p122c_i as float);
            if d.ox > 10.0 {
                score += 1;
            }
        }
    }
    assert(score > 0, \"conditional struct failed\");
}"
    )
    .result(Value::Null);
}

// P122d: struct created inside loop body (not from function return)
#[test]
fn p122_struct_literal_in_loop() {
    code!(
        "struct Rect { rx: float, ry: float, rw: float, rh: float }
fn test() {
    count = 0;
    for p122d_i in 0..500 {
        r = Rect { rx: p122d_i as float, ry: 0.0, rw: 10.0, rh: 10.0 };
        if r.rx > 100.0 { count += 1; }
    }
    assert(count == 399, \"struct literal loop failed\");
}"
    )
    .result(Value::Null);
}

// P122e: very long loop (simulating game frames) — exhaustion stress test.
//
// In release mode the 100 000 struct allocations complete in ~0.05s and
// the test is a real store-exhaustion regression guard that rides along
// with `cargo test --release` (CI's default).  In debug mode the same
// body takes ~10 minutes because the loft bytecode interpreter is
// dominated by debug Rust overhead — so we cfg-gate the `#[ignore]`
// attribute to debug_assertions only, not the whole test.  That keeps
// `cargo test` (debug) fast for day-to-day iteration while CI continues
// to exercise the real stress path.  Run manually in debug with
// `cargo test --ignored p122_long_running_struct_loop` when needed.
#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "P122 stress, 10 min in debug: --release, or --ignored"
)]
fn p122_long_running_struct_loop() {
    code!(
        "struct Overlap { ox: float, oy: float }
fn depth(ax: float, ay: float, bx: float, by: float) -> Overlap {
    Overlap { ox: ax - bx, oy: ay - by }
}
fn test() {
    score = 0;
    // 10000 frames * 10 bricks = 100,000 struct allocations
    for p122e_f in 0..10000 {
        for p122e_b in 0..10 {
            d = depth(p122e_f as float, 50.0, p122e_b as float * 8.0, 20.0);
            if d.ox > 0.0 && d.oy > 0.0 { score += 1; }
        }
    }
    assert(score > 0, \"long struct loop failed\");
}"
    )
    .result(Value::Null);
}

// ── GL-pattern verification tests ─────────────────────────────────────────
//
// These tests replicate the actual patterns from the GL renderer and game
// loop that historically triggered store leaks (P122), vector leaks (P123),
// struct-return leaks (P117), and heap corruption (P121).
//
// Each test is designed to run in both debug mode (with assertions) and
// release mode. They use sustained iteration counts that would exhaust the
// store pool if leaks were present.

// ── P122 GL pattern: mat4-style struct with vector field, returned per frame ──
//
// This replicates the `math::mat4_mul` / `mat4_trs` pattern from the renderer:
// a struct containing a vector<float> is constructed and returned from a function
// called once per frame. In the real renderer this happens via mat4_look_at,
// mat4_perspective, mat4_mul — each allocating a store for the Mat4 + its
// vector field.
#[test]
fn p122_gl_mat4_vector_field_per_frame() {
    code!(
        "struct M4 { m: vector<float> }

fn make_m4(gm4_s: float) -> M4 {
    M4 { m: [gm4_s, 0.0, 0.0, 0.0,
             0.0, gm4_s, 0.0, 0.0,
             0.0, 0.0, gm4_s, 0.0,
             0.0, 0.0, 0.0, 1.0] }
}

fn mul_m4(gm4_a: const M4, gm4_b: const M4) -> M4 {
    gm4_r = M4 { m: [0.0, 0.0, 0.0, 0.0,
                      0.0, 0.0, 0.0, 0.0,
                      0.0, 0.0, 0.0, 0.0,
                      0.0, 0.0, 0.0, 0.0] };
    for gm4_i in 0..4 {
        for gm4_j in 0..4 {
            gm4_sum = 0.0;
            for gm4_k in 0..4 {
                gm4_sum += gm4_a.m[gm4_i * 4 + gm4_k] * gm4_b.m[gm4_k * 4 + gm4_j];
            }
            gm4_r.m[gm4_i * 4 + gm4_j] = gm4_sum;
        }
    }
    gm4_r
}

fn test() {
    // Simulate 500 render frames, each creating 3 matrices and multiplying
    // Total: 500 * 5 = 2500 struct-with-vector allocations
    gm4_check = 0.0;
    for gm4_frame in 0..500 {
        gm4_view = make_m4(1.0);
        gm4_proj = make_m4(2.0);
        gm4_model = make_m4((gm4_frame as float) * 0.001 + 1.0);
        gm4_vp = mul_m4(gm4_view, gm4_proj);
        gm4_mvp = mul_m4(gm4_vp, gm4_model);
        gm4_check += gm4_mvp.m[0];
    }
    assert(gm4_check > 0.0, \"mat4 GL loop failed: {gm4_check}\");
}"
    )
    .result(Value::Null);
}

// ── P122 GL pattern: collision detection with struct Rect + Overlap ────────
//
// This replicates the Brick Buster collision loop using the *struct-based* API
// (not the raw-float workaround). Each frame checks N bricks for collision
// with M balls, creating Rect and Overlap structs per check.
#[test]
fn p122_gl_collision_struct_api() {
    code!(
        "struct Rect { rx: float, ry: float, rw: float, rh: float }
struct Overlap { ox: float, oy: float }

fn rects_overlap(gc_a: const Rect, gc_b: const Rect) -> boolean {
    gc_a.rx < gc_b.rx + gc_b.rw && gc_a.rx + gc_a.rw > gc_b.rx &&
    gc_a.ry < gc_b.ry + gc_b.rh && gc_a.ry + gc_a.rh > gc_b.ry
}

fn overlap_depth(gc_a: const Rect, gc_b: const Rect) -> Overlap {
    if !rects_overlap(gc_a, gc_b) { return Overlap { ox: 0.0, oy: 0.0 }; }
    gc_dx = min(gc_a.rx + gc_a.rw - gc_b.rx, gc_b.rx + gc_b.rw - gc_a.rx);
    gc_dy = min(gc_a.ry + gc_a.rh - gc_b.ry, gc_b.ry + gc_b.rh - gc_a.ry);
    Overlap { ox: gc_dx, oy: gc_dy }
}

fn test() {
    gc_hits = 0;
    // 200 frames, 8 bricks per frame = 1600 collision checks
    // Each check creates 2 Rect + 1 Overlap (conditionally)
    for gc_frame in 0..200 {
        gc_ball = Rect { rx: (gc_frame as float) * 0.5, ry: 50.0, rw: 8.0, rh: 8.0 };
        for gc_brick in 0..8 {
            gc_br = Rect {
                rx: (gc_brick as float) * 40.0, ry: 45.0,
                rw: 35.0, rh: 12.0
            };
            gc_d = overlap_depth(gc_ball, gc_br);
            if gc_d.ox > 0.0 && gc_d.oy > 0.0 {
                gc_hits += 1;
            }
        }
    }
    assert(gc_hits > 0, \"collision struct loop failed: {gc_hits}\");
}"
    )
    .result(Value::Null);
}

// ── P120 minimal isolation tests ───────────────────────────────────────────
//
// These tests isolate the exact pattern that leaks stores. Each adds one
// element of complexity to find the boundary between "works" and "leaks".

/// P120 atom A: struct field overwrite once, no loop.
/// Does a single overwrite of a struct field with a function return leak?
///
/// Root cause (from execution trace): `make_inner` allocates store 3 for the
/// returned Inner struct. `CopyRecord` copies the data from store 3 into
/// store 2 (the Outer's field). But no `FreeRef` is emitted for store 3
/// after the copy — it becomes orphaned. Debug mode catches this at exit
/// with "Database 3 not correctly freed".
#[test]
fn p120_field_overwrite_once() {
    code!(
        "struct Inner { ix: float, iy: float }
struct Outer { pos: Inner }

fn make_inner(p120a_v: float) -> Inner {
    Inner { ix: p120a_v, iy: p120a_v * 2.0 }
}

fn test() {
    p120a_o = Outer { pos: Inner { ix: 0.0, iy: 0.0 } };
    p120a_o.pos = make_inner(5.0);
    assert(p120a_o.pos.ix == 5.0, \"overwrite once: {p120a_o.pos.ix}\");
}"
    )
    .result(Value::Null);
}

/// P120 atom B: struct field overwrite twice, no loop.
/// The second overwrite must free the store from the first.
#[test]
fn p120_field_overwrite_twice() {
    code!(
        "struct Inner { ix: float, iy: float }
struct Outer { pos: Inner }

fn make_inner(p120b_v: float) -> Inner {
    Inner { ix: p120b_v, iy: p120b_v * 2.0 }
}

fn test() {
    p120b_o = Outer { pos: Inner { ix: 0.0, iy: 0.0 } };
    p120b_o.pos = make_inner(1.0);
    p120b_o.pos = make_inner(2.0);
    assert(p120b_o.pos.ix == 2.0, \"overwrite twice: {p120b_o.pos.ix}\");
}"
    )
    .result(Value::Null);
}

/// P120 atom C: struct field overwrite in a short loop (3 iterations).
#[test]
fn p120_field_overwrite_short_loop() {
    code!(
        "struct Inner { ix: float, iy: float }
struct Outer { pos: Inner }

fn make_inner(p120c_v: float) -> Inner {
    Inner { ix: p120c_v, iy: p120c_v * 2.0 }
}

fn test() {
    p120c_o = Outer { pos: Inner { ix: 0.0, iy: 0.0 } };
    for p120c_i in 0..3 {
        p120c_o.pos = make_inner(p120c_i as float);
    }
    assert(p120c_o.pos.ix == 2.0, \"short loop: {p120c_o.pos.ix}\");
}"
    )
    .result(Value::Null);
}

/// P120 atom D: local variable overwrite in a loop (NOT a field).
/// This should NOT leak — the store is on the stack, not in a struct.
#[test]
fn p120_local_overwrite_in_loop() {
    code!(
        "struct Inner { ix: float, iy: float }

fn make_inner(p120d_v: float) -> Inner {
    Inner { ix: p120d_v, iy: p120d_v * 2.0 }
}

fn test() {
    p120d_sum = 0.0;
    for p120d_i in 0..100 {
        p120d_val = make_inner(p120d_i as float);
        p120d_sum += p120d_val.ix;
    }
    assert(p120d_sum > 0.0, \"local overwrite: {p120d_sum}\");
}"
    )
    .result(Value::Null);
}

/// P120 atom E: struct field overwrite with text field (triggers P117 area).
#[test]
fn p120_field_overwrite_with_text() {
    code!(
        "struct Named { label: text, val: integer }
struct Container { item: Named }

fn make_named(p120e_s: text, p120e_n: integer) -> Named {
    Named { label: p120e_s, val: p120e_n }
}

fn test() {
    p120e_c = Container { item: Named { label: \"init\", val: 0 } };
    for p120e_i in 0..10 {
        p120e_c.item = make_named(\"iter_{p120e_i}\", p120e_i);
    }
    assert(p120e_c.item.val == 9, \"text field: {p120e_c.item.val}\");
}"
    )
    .result(Value::Null);
}

// ── P120 GL-pattern: struct return inside conditional inside loop ──────────
//
// Replicates the renderer transform update: a struct-returning function is
// called inside an `if` branch inside the render loop. P120 triggers when
// copy_record tries to delete a record in a locked store.
//
// Passes in release mode but fails in debug mode with "Database N not
// correctly freed" — confirming the store leak is real. The leaked store
// is from overwriting a struct field with a new struct-returning function
// call: the old store is not freed before the new one is assigned.
#[test]
fn p120_struct_return_in_conditional_in_loop() {
    code!(
        "struct Transform { tx: float, ty: float, tz: float }
struct Node { name: text, xform: Transform }

fn make_transform(gl_t: float) -> Transform {
    Transform { tx: sin(gl_t) * 2.0, ty: cos(gl_t), tz: 0.0 }
}

fn test() {
    gl_nd = Node { name: \"cube\", xform: Transform { tx: 0.0, ty: 0.0, tz: 0.0 } };
    gl_sum = 0.0;
    for gl_frame in 0..1000 {
        gl_time = (gl_frame as float) * 0.01;
        // Conditional struct return — the P120 pattern
        if gl_frame % 2 == 0 {
            gl_nd.xform = make_transform(gl_time);
        }
        gl_sum += gl_nd.xform.tx;
    }
    assert(gl_sum != 0.0, \"conditional struct return sum should be nonzero\");
}"
    )
    .result(Value::Null);
}

// ── P120 pattern: multiple struct field updates per frame ──────────────────
//
// Replicates the renderer's per-frame update of multiple node transforms
// in a scene graph. Each node's transform is overwritten with a new struct.
//
// Passes in release mode but fails in debug mode with "Database 9 not
// correctly freed" — same root cause as the conditional test above.
// The store allocated for the old Vec3 value is not freed when the field
// is overwritten with a new struct from make_pos().
#[test]
fn p120_multi_node_transform_update() {
    code!(
        "struct Vec3 { vx: float, vy: float, vz: float }
struct SceneNode { pos: Vec3, scale: float }

fn make_pos(mn_t: float, mn_i: integer) -> Vec3 {
    Vec3 { vx: sin(mn_t + mn_i as float), vy: cos(mn_t), vz: 0.0 }
}

fn test() {
    // 4 nodes, each updated per frame — like a small scene graph
    mn_n0 = SceneNode { pos: Vec3 { vx: 0.0, vy: 0.0, vz: 0.0 }, scale: 1.0 };
    mn_n1 = SceneNode { pos: Vec3 { vx: 0.0, vy: 0.0, vz: 0.0 }, scale: 1.0 };
    mn_n2 = SceneNode { pos: Vec3 { vx: 0.0, vy: 0.0, vz: 0.0 }, scale: 1.0 };
    mn_n3 = SceneNode { pos: Vec3 { vx: 0.0, vy: 0.0, vz: 0.0 }, scale: 1.0 };
    mn_sum = 0.0;
    for mn_frame in 0..500 {
        mn_t = (mn_frame as float) * 0.02;
        mn_n0.pos = make_pos(mn_t, 0);
        mn_n1.pos = make_pos(mn_t, 1);
        mn_n2.pos = make_pos(mn_t, 2);
        mn_n3.pos = make_pos(mn_t, 3);
        mn_sum += mn_n0.pos.vx + mn_n1.pos.vy + mn_n2.pos.vx + mn_n3.pos.vy;
    }
    assert(mn_sum != 0.0, \"multi-node update sum should be nonzero\");
}"
    )
    .result(Value::Null);
}

// ── P117 GL pattern: text-param struct return in a tight loop ──────────────
//
// The original P117 bug: a function that takes a text parameter and returns
// a struct leaks the callee's store. This test calls such a function in
// a sustained loop to verify the leak is actually gone.
#[test]
fn p117_gl_text_param_struct_return_sustained() {
    code!(
        "struct Asset { path: text, size: integer }

fn load_asset(tp_name: text) -> Asset {
    Asset { path: tp_name, size: tp_name.len() }
}

fn test() {
    tp_total = 0;
    for tp_i in 0..2000 {
        tp_a = load_asset(\"textures/brick_{tp_i}.png\");
        tp_total += tp_a.size;
    }
    assert(tp_total > 0, \"text-param struct loop failed: {tp_total}\");
}"
    )
    .result(Value::Null);
}

// ── P117 pattern: multiple text-param struct returns per iteration ─────────
//
// Stresses the text-param return path with multiple calls per loop iteration,
// mimicking loading different asset types each frame.
#[test]
fn p117_gl_multi_text_struct_per_frame() {
    code!(
        "struct FileRef { name: text, found: boolean }

fn lookup(mt_path: text) -> FileRef {
    FileRef { name: mt_path, found: mt_path.len() > 5 }
}

fn test() {
    mt_found = 0;
    for mt_i in 0..1000 {
        mt_a = lookup(\"shader/vert_{mt_i}.glsl\");
        mt_b = lookup(\"shader/frag_{mt_i}.glsl\");
        mt_c = lookup(\"tex/d.png\");
        if mt_a.found { mt_found += 1; }
        if mt_b.found { mt_found += 1; }
        if mt_c.found { mt_found += 1; }
    }
    assert(mt_found > 0, \"multi-text struct failed: {mt_found}\");
}"
    )
    .result(Value::Null);
}

// ── P121 pattern: tuple usage in a sustained loop ─────────────────────────
//
// The original P121 bug was heap corruption from tuple literals. This test
// creates tuples in a loop to verify the fix holds under sustained use.
#[test]
fn p121_tuple_sustained_loop() {
    code!(
        "fn make_pair(tp_x: float, tp_y: float) -> (float, float) {
    (tp_x, tp_y)
}

fn test() {
    tp_sum = 0.0;
    for tp_i in 0..1000 {
        tp_p = make_pair(tp_i as float, (tp_i as float) * 0.5);
        tp_sum += tp_p.0 + tp_p.1;
    }
    assert(tp_sum > 0.0, \"tuple loop failed: {tp_sum}\");
}"
    )
    .result(Value::Null);
}

// ── P121 pattern: nested tuple operations ─────────────────────────────────
//
// Tests tuple element access, arithmetic on tuple fields, and tuple
// construction from other tuple elements — more complex than a simple literal.
#[test]
fn p121_tuple_nested_operations() {
    code!(
        "fn swap_pair(tn_a: float, tn_b: float) -> (float, float) {
    (tn_b, tn_a)
}

fn test() {
    tn_sum = 0.0;
    for tn_i in 0..500 {
        tn_p1 = (tn_i as float, (tn_i as float) * 2.0);
        tn_p2 = swap_pair(tn_p1.0, tn_p1.1);
        tn_sum += tn_p2.0 - tn_p2.1;
    }
    // Each iteration: p2 = (i*2, i), so p2.0 - p2.1 = i*2 - i = i
    // sum = 0 + 1 + 2 + ... + 499 = 124750
    assert(tn_sum > 124000.0, \"nested tuple failed: {tn_sum}\");
}"
    )
    .result(Value::Null);
}

// ── P123 GL pattern: vector allocation per frame ──────────────────────────
//
// Replicates the renderer's per-frame vertex data construction: a vector of
// floats is built each frame (like uploading new vertex positions). This is
// the pattern that exhausted the store pool before P123 was fixed.
#[test]
fn p123_gl_vector_per_frame_sustained() {
    code!(
        "fn test() {
    vf_total = 0;
    for vf_frame in 0..1000 {
        // Build vertex data each frame (like gl_upload_vertices)
        vf_verts = [for vf_v in 0..12 { (vf_v + vf_frame) as float * 0.01 }];
        vf_total += vf_verts.len();
    }
    assert(vf_total == 12000, \"per-frame vector failed: {vf_total}\");
}"
    )
    .result(Value::Null);
}

// ── P123 pattern: multiple vector allocations per frame ───────────────────
//
// The renderer builds multiple vectors per frame: positions, normals, colors,
// indices. Each is a fresh allocation. This test creates 4 vectors per
// iteration for 500 iterations = 2000 vector allocations.
#[test]
fn p123_gl_multi_vector_per_frame() {
    code!(
        "fn test() {
    mv_check = 0;
    for mv_frame in 0..500 {
        mv_pos = [for mv_i in 0..6 { mv_i as float + mv_frame as float * 0.001 }];
        mv_norm = [for mv_i in 0..6 { 0.0 }];
        mv_col = [for mv_i in 0..6 { 1.0 }];
        mv_idx = [for mv_i in 0..3 { mv_i }];
        mv_check += mv_pos.len() + mv_norm.len() + mv_col.len() + mv_idx.len();
    }
    assert(mv_check == 10500, \"multi-vector per frame failed: {mv_check}\");
}"
    )
    .result(Value::Null);
}

// ── Combined GL pattern: struct + vector + text in game loop ──────────────
//
// This is the "full Brick Buster frame" pattern combining all the bug areas:
// struct collision detection, vector per-frame data, text for debug output,
// all inside a sustained game loop.
#[test]
fn gl_combined_game_loop_stress() {
    code!(
        "struct Ball { bx: float, by: float }
struct Brick { brx: float, bry: float, hp: integer }

fn make_ball(cb_frame: integer) -> Ball {
    Ball { bx: (cb_frame as float) * 0.3, by: 50.0 }
}

fn check_hit(cb_ball: const Ball, cb_brick: const Brick) -> boolean {
    abs(cb_ball.bx - cb_brick.brx) < 20.0 && abs(cb_ball.by - cb_brick.bry) < 10.0
}

fn format_score(cb_score: integer) -> text {
    \"Score: {cb_score}\"
}

fn test() {
    cb_score = 0;
    cb_last_text = \"\";
    for cb_frame in 0..300 {
        cb_b = make_ball(cb_frame);
        // Check 10 bricks per frame
        cb_active = [for cb_i in 0..10 { 1 }];
        for cb_i in 0..10 {
            if cb_active[cb_i] == 0 { continue; }
            cb_brick = Brick { brx: (cb_i as float) * 30.0, bry: 45.0, hp: 1 };
            if check_hit(cb_b, cb_brick) {
                cb_score += 1;
                cb_active[cb_i] = 0;
            }
        }
        // Text allocation per frame (like HUD update)
        if cb_frame % 60 == 0 {
            cb_last_text = format_score(cb_score);
        }
    }
    assert(cb_score > 0, \"combined game loop failed: {cb_score}\");
    assert(cb_last_text.len() > 0, \"text output empty\");
}"
    )
    .result(Value::Null);
}
