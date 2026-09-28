// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Characterisation suite for `par` — pins down the current behaviour
//! before plan-06's typed-par redesign refactors the runtime.
//!
//! Each test exercises a specific (input element type) × (output
//! element type) × (thread count) × (worker form) combination and
//! asserts the exact current output.  Subsequent plan-06 phases must
//! keep these tests passing byte-for-byte; any deliberate behaviour
//! change updates the corresponding fixture in the same commit.
//!
//! The matrix is per `doc/claude/plans/06-typed-par/00-baseline-and-bench.md`.
//!
//! # Pre-existing input-element-stride bug (P189-class — discovered
//! while writing this suite)
//!
//! Today's `par(...)` runtime mishandles **primitive-element input
//! vectors** — `vector<integer>`, `vector<float>`, `vector<i32>`,
//! `vector<u8>`, etc.  Each worker reads its input slice with a
//! 12-byte (DbRef) stride regardless of the vector's actual element
//! width, so workers receive garbage from adjacent memory.
//!
//! Confirmed: plain `for x in items` (no par) over a `vector<integer>`
//! works correctly; only par's worker-dispatch is affected.  The bug
//! has been latent since the C54 / P184 narrow-vector migration
//! because every existing par fixture in `tests/scripts/22-threading.loft`
//! uses `vector<Score>` (struct ref) input.
//!
//! Plan-06 phase 4 (typed input/output) is the natural place to fix
//! this — once the runtime reads element stride from the type system
//! instead of a parser-computed integer, primitive-element inputs
//! work uniformly with struct-element inputs.
//!
//! Tests below that exercise primitive-element inputs are gated
//! `#[ignore = "..."]` with the tracking note, so the regression
//! suite will catch it on the day phase 4 lands and the tests get
//! un-`#[ignore]`d.
//!
//! Working tests in this file all use `vector<Score>`-shaped inputs
//! (single-field struct wrapping the underlying scalar) to match
//! today's only-tested path.

mod testing;

use loft::data::Value;

// Shared test scaffolding: `Score { value }` wraps an integer; every
// per-cell test instantiates a fresh `vector<Score>` and runs a
// worker fn that operates on the score, returning the per-cell
// output type.

// ── primitive scalar return paths (Score input, primitive output) ──

#[test]
fn par_struct_to_int_t1() {
    code!(
        "struct Score { value: integer }
fn dbl(s: const Score) -> integer { s.value * 2 }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 10 }, Score { value: 20 }, Score { value: 30 }];
    sum = 0;
    for s in sl par(r = dbl(s), 1) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(120));
}

#[test]
fn par_struct_to_int_t4() {
    code!(
        "struct Score { value: integer }
fn dbl(s: const Score) -> integer { s.value * 2 }
fn run() -> integer {
    sl: vector<Score> = [
        Score { value: 10 }, Score { value: 20 }, Score { value: 30 }, Score { value: 40 },
        Score { value: 50 }, Score { value: 60 }, Score { value: 70 }, Score { value: 80 }
    ];
    sum = 0;
    for s in sl par(r = dbl(s), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(720));
}

#[test]
fn par_struct_to_float_t4() {
    code!(
        "struct Score { value: integer }
fn half(s: const Score) -> float { s.value as float / 2.0 }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 10 }, Score { value: 20 }, Score { value: 30 }, Score { value: 40 }];
    total = 0.0;
    for s in sl par(r = half(s), 4) { total += r; }
    total as integer
}"
    )
    .expr("run()")
    .result(Value::Int(50));
}

#[test]
fn par_struct_to_single_t4() {
    code!(
        "struct Score { value: integer }
fn quad(s: const Score) -> single { (s.value * 4) as single }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }, Score { value: 3 }, Score { value: 4 }];
    sum = 0;
    for s in sl par(r = quad(s), 4) { sum += r as integer; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(40));
}

#[test]
fn par_struct_to_i32_t4() {
    code!(
        "struct Score { value: integer }
fn neg(s: const Score) -> i32 { (-s.value) as i32 }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }, Score { value: 3 }, Score { value: 4 }];
    sum = 0;
    for s in sl par(r = neg(s), 4) { sum += r as integer; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(-10));
}

#[test]
fn par_struct_to_byte_t4() {
    code!(
        "struct Score { value: integer }
fn small(s: const Score) -> u8 { (s.value & 255) as u8 }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }, Score { value: 3 }, Score { value: 4 }, Score { value: 5 }];
    sum = 0;
    for s in sl par(r = small(s), 4) { sum += r as integer; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(15));
}

#[test]
fn par_struct_to_bool_t4() {
    code!(
        "struct Score { value: integer }
fn pos(s: const Score) -> boolean { s.value > 0 }
fn run() -> integer {
    sl: vector<Score> = [Score { value: -2 }, Score { value: -1 }, Score { value: 0 }, Score { value: 1 }, Score { value: 2 }];
    count = 0;
    for s in sl par(r = pos(s), 4) { if r { count += 1; } }
    count
}"
    )
    .expr("run()")
    .result(Value::Int(2));
}

#[test]
fn par_struct_to_text_t4() {
    code!(
        "struct Score { value: integer }
fn label(s: const Score) -> text { \"v{s.value}\" }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }, Score { value: 3 }, Score { value: 4 }];
    total_len = 0;
    for s in sl par(r = label(s), 4) { total_len += len(r); }
    total_len
}"
    )
    .expr("run()")
    .result(Value::Int(8));
}

// ── reference / struct return paths ────────────────────────────────────

#[test]
fn par_struct_to_struct_t4() {
    code!(
        "struct Score { value: integer }
struct Point { x: integer, y: integer }
fn make_point(s: const Score) -> Point { Point { x: s.value, y: s.value + 1 } }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }, Score { value: 3 }, Score { value: 4 }];
    sum = 0;
    for s in sl par(p = make_point(s), 4) { sum += p.x + p.y; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(24));
}

#[test]
fn par_struct_to_enum_t4() {
    code!(
        "struct Score { value: integer }
enum Sign { Neg, Zero, Pos }
fn classify(s: const Score) -> Sign {
    if s.value < 0 { Neg } else if s.value == 0 { Zero } else { Pos }
}
fn run() -> integer {
    sl: vector<Score> = [Score { value: -2 }, Score { value: -1 }, Score { value: 0 }, Score { value: 1 }, Score { value: 2 }];
    pos_count = 0;
    for s in sl par(g = classify(s), 4) { if g == Pos { pos_count += 1; } }
    pos_count
}"
    )
    .expr("run()")
    .result(Value::Int(2));
}

// ── degenerate inputs ──────────────────────────────────────────────────

#[test]
fn par_empty_input() {
    code!(
        "struct Score { value: integer }
fn dbl(s: const Score) -> integer { s.value * 2 }
fn run() -> integer {
    sl: vector<Score> = [];
    sum = 0;
    for s in sl par(r = dbl(s), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(0));
}

#[test]
fn par_single_element_t4() {
    // Worker count clamps to 1 internally but the surface accepts 4.
    code!(
        "struct Score { value: integer }
fn dbl(s: const Score) -> integer { s.value * 2 }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 42 }];
    sum = 0;
    for s in sl par(r = dbl(s), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(84));
}

#[test]
fn par_max_threads_overprovisioned() {
    // Threads > input length: extra workers should immediately exit.
    code!(
        "struct Score { value: integer }
fn dbl(s: const Score) -> integer { s.value * 2 }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }, Score { value: 3 }, Score { value: 4 }, Score { value: 5 }];
    sum = 0;
    for s in sl par(r = dbl(s), 16) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(30));
}

// ── Form 2: method-style worker (`.method`) ────────────────────────────

#[test]
fn par_form2_method() {
    // This used to acknowledge a false "Variable s is never read": the lint counted
    // `uses`, which the method-receiver position of `s.dbl()` never incremented, so a
    // variable the program plainly reads was reported unread.  The lint now asks whether
    // the emitted body NAMES the variable, and the receiver is right there in it — so the
    // warning is gone and the acknowledgement with it.  `22-threading.loft` sidestepped
    // the same false positive by always using both `x` and `r` in the body.
    code!(
        "struct Score { value: integer }
fn dbl(self: const Score) -> integer { self.value * 2 }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }, Score { value: 3 }];
    sum = 0;
    for s in sl par(r = s.dbl(), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(12));
}

// ── correctness — order independence within the result vector ─────────

#[test]
fn par_results_pair_with_inputs_in_order() {
    // Body sees x and r paired in input order — fused-form invariant
    // that plan-06 must preserve.
    code!(
        "struct Score { value: integer }
fn dbl(s: const Score) -> integer { s.value * 2 }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 10 }, Score { value: 20 }, Score { value: 30 }, Score { value: 40 }];
    weighted = 0;
    pos = 1;
    for s in sl par(r = dbl(s), 4) {
        weighted += pos * r;
        pos += 1;
    }
    weighted
}"
    )
    .expr("run()")
    .result(Value::Int(600));
}

// ── auto-light heuristic — observable via execution-correctness ────────

#[test]
fn par_pure_arithmetic_worker_runs_clean() {
    // Pure-arithmetic worker: no allocation, no shared writes.
    // Today's parser-side check_light_eligible classifies this as light;
    // the auto-light analyser in plan-06 phase 5 must continue to.
    code!(
        "struct Score { value: integer }
fn cube(s: const Score) -> integer { s.value * s.value * s.value }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }, Score { value: 3 }, Score { value: 4 }];
    sum = 0;
    for s in sl par(r = cube(s), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(100));
}

// ─────────────────────────────────────────────────────────────────────
// Shapes that don't work today.  Each `#[ignore]` entry is a canary
// — un-`#[ignore]` it when the relevant plan-06 phase lands and the
// shape starts working.  The plan file is the authoritative inventory
// of these gaps with their fix targets; see
// `doc/claude/plans/06-typed-par/01-output-store.md § Surface gaps
// closed by phase 1`.  We do NOT file one PROBLEMS.md entry per gap —
// the plan is the single source of truth.
// ─────────────────────────────────────────────────────────────────────

#[test]
fn par_int_to_int_t4_primitive_input() {
    code!(
        "fn dbl(x: integer) -> integer { x * 2 }
fn run() -> integer {
    items: vector<integer> = [10, 20, 30, 40, 50, 60, 70, 80];
    sum = 0;
    for x in items par(r = dbl(x), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(720));
}

#[test]
fn par_float_input_t4() {
    code!(
        "fn dbl(x: float) -> float { x * 2.0 }
fn run() -> integer {
    items: vector<float> = [1.0, 2.0, 3.0, 4.0];
    total = 0.0;
    for x in items par(r = dbl(x), 4) { total += r; }
    total as integer
}"
    )
    .expr("run()")
    .result(Value::Int(20));
}

#[test]
fn par_i32_input_t4() {
    code!(
        "fn dbl(x: i32) -> integer { (x as integer) * 2 }
fn run() -> integer {
    items: vector<i32> = [10 as i32, 20 as i32, 30 as i32, 40 as i32];
    sum = 0;
    for x in items par(r = dbl(x), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(200));
}

#[test]
fn par_u8_input_t4() {
    code!(
        "fn dbl(x: u8) -> integer { (x as integer) * 2 }
fn run() -> integer {
    items: vector<u8> = [1 as u8, 2 as u8, 3 as u8, 4 as u8];
    sum = 0;
    for x in items par(r = dbl(x), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(20));
}

#[test]
fn par_text_input_t4() {
    code!(
        "fn count_chars(s: text) -> integer { len(s) }
fn run() -> integer {
    items: vector<text> = [\"hi\", \"hello\", \"x\", \"abc\"];
    sum = 0;
    for x in items par(r = count_chars(x), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(11));
}

#[test]
fn par_struct_to_struct_enum_t4() {
    // Struct-enum (variant with fields) returned from a par worker.
    // Plan-06 phase 1 G1: the parser routes any heap-typed return
    // (`heap_def_nr().is_some()`) through the ref path, so worker
    // returns of `Enum(_, true, _)` deep-copy via copy_from_worker
    // exactly like `Reference<T>` returns.
    code!(
        "struct Score { value: integer }
enum Verdict {
    Pass { score: integer },
    Fail { reason: text }
}
fn classify(s: const Score) -> Verdict {
    v: Verdict = Pass { score: s.value };
    if s.value < 0 { v = Fail { reason: \"negative\" }; }
    v
}
fn run() -> integer {
    sl: vector<Score> = [Score { value: 10 }, Score { value: -5 }, Score { value: 20 }];
    pass_sum = 0;
    for s in sl par(v = classify(s), 4) {
        if v is Pass { score } { pass_sum += score; }
    }
    pass_sum
}"
    )
    .expr("run()")
    .result(Value::Int(30));
}

// ── phase 0d canaries — additional D11 spectrum entries ─────────────
// Each test asserts the eventual correct behaviour.  Today the
// shape either fails to compile or returns garbage; the relevant
// plan-06 phase un-`#[ignore]`s the test as the gap closes.

#[test]
fn par_enum_input_t4() {
    code!(
        "enum Color { Red, Green, Blue }
fn idx(c: Color) -> integer { if c == Red { 1 } else if c == Green { 2 } else { 3 } }
fn run() -> integer {
    items: vector<Color> = [Red, Green, Blue, Red];
    sum = 0;
    for c in items par(r = idx(c), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(7));
}

#[test]
fn par_vec_of_refs_input_t4() {
    // Workers receive a Reference (DbRef into the parent's store)
    // and return a derived value.  Distinct from vector<Struct>
    // because the elements are ref-typed, not inline struct.
    code!(
        "struct Item { value: integer }
struct Holder { i: reference Item }
fn from_holder(h: const Holder) -> integer { h.i.value * 2 }
fn run() -> integer {
    a: Item = Item { value: 10 };
    b: Item = Item { value: 20 };
    items: vector<Holder> = [Holder { i: a }, Holder { i: b }];
    sum = 0;
    for h in items par(r = from_holder(h), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(60));
}

#[test]
fn par_nested_vector_input_t4() {
    // Worker iterates the inner vector.  Outer is the par input.
    code!(
        "struct Bag { items: vector<integer> }
fn sum_bag(b: const Bag) -> integer {
    s = 0;
    for x in b.items { s += x; }
    s
}
fn run() -> integer {
    bags: vector<Bag> = [
        Bag { items: [1, 2, 3] },
        Bag { items: [10, 20] },
        Bag { items: [100] }
    ];
    total = 0;
    for b in bags par(r = sum_bag(b), 4) { total += r; }
    total
}"
    )
    .expr("run()")
    .result(Value::Int(136));
}

// par-vec-of-fns-input: closed by ARC.md A6.c via two surgical fixes:
// (1) `Data::narrow_vector_content` (src/data.rs) extended to route
//     `Type::Function` to `database.int(0, false)` (Parts::Int with
//     size=4) instead of falling through to `def_nr("i32").known_type`,
//     a placeholder type with size=0 that caused `vector_append` to
//     use stride=0 — every literal element overwrote offset 8.
// (2) `read_tuple_at_wide` (src/parallel.rs) special-cased for
//     Type::Function elements: zero-extend the 4-byte d_nr from
//     storage to 8 bytes for the worker's i64 d_nr slot, and write
//     a `(u16::MAX, 0, 0)` sentinel for the closure DbRef portion
//     (vector<fn> can only store non-capturing functions).
#[test]
fn par_vec_of_fns_input_t4() {
    // Workers receive a fn-ref and call it on a fixed input.
    // Whether vector<fn> is even constructable today is part of
    // what the canary documents.
    code!(
        "fn dbl(x: integer) -> integer { x * 2 }
fn triple(x: integer) -> integer { x * 3 }
fn quad(x: integer) -> integer { x * 4 }
fn apply(f: fn(integer) -> integer) -> integer { f(10) }
fn run() -> integer {
    fs: vector<fn(integer) -> integer> = [dbl, triple, quad];
    total = 0;
    for f in fs par(r = apply(f), 4) { total += r; }
    total
}"
    )
    .expr("run()")
    .result(Value::Int(90));
}

#[test]
fn par_sorted_input_t4() {
    code!(
        "struct Score { value: integer }
fn dbl(s: const Score) -> integer { s.value * 2 }
fn run() -> integer {
    sorted_items: sorted<Score[value]> = [];
    sorted_items += Score { value: 30 };
    sorted_items += Score { value: 10 };
    sorted_items += Score { value: 20 };
    sum = 0;
    for s in sorted_items par(r = dbl(s), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(120));
}

/// Closed by commits `ab08ad0` (P191 — index bookkeeping layout) +
/// `592cde8` (P188 follow-up — local-var keyed-collection += dispatch
/// fix and struct-literal RHS retarget).  The 4d.B materialise-then-
/// route desugar (sorted path) automatically extends to hash once
/// the local-var hash + struct-literal `+=` and iteration paths are
/// correct.
#[test]
fn par_hash_input_t4() {
    code!(
        "struct Score { name: text, value: integer }
fn dbl(s: const Score) -> integer { s.value * 2 }
fn run() -> integer {
    h: hash<Score[name]> = [];
    h += Score { name: \"a\", value: 10 };
    h += Score { name: \"b\", value: 20 };
    h += Score { name: \"c\", value: 30 };
    sum = 0;
    for s in h par(r = dbl(s), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(120));
}

/// Closed by commits `ab08ad0` (P191 — `database.index` bookkeeping
/// fields are now 4-byte `int<0,false>` instead of 8-byte `integer`,
/// matching `tree::add`'s hardcoded RB_LEFT=0 / RB_RIGHT=4 offsets)
/// and `592cde8` (P188 — local-var `+=` dispatches to the specific
/// `database.index(c, key)` instead of the generic `index` alias).
#[test]
fn par_index_input_t4() {
    code!(
        "struct Score { name: text, value: integer }
fn dbl(s: const Score) -> integer { s.value * 2 }
fn run() -> integer {
    ix: index<Score[name]> = [];
    ix += Score { name: \"a\", value: 10 };
    ix += Score { name: \"b\", value: 20 };
    sum = 0;
    for s in ix par(r = dbl(s), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(60));
}

#[test]
fn par_struct_to_large_struct_t4() {
    // Today's parser-side check_light_eligible / parse_parallel_for
    // rejects any return type with var_size > 8 unless it's a
    // Reference.  A large by-value struct hits this.
    code!(
        "struct Score { value: integer }
struct Wide { a: integer, b: integer, c: integer }
fn make_wide(s: const Score) -> Wide { Wide { a: s.value, b: s.value * 2, c: s.value * 3 } }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }, Score { value: 3 }];
    sum = 0;
    for s in sl par(w = make_wide(s), 4) { sum += w.a + w.b + w.c; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(36));
}

// par-vector-return: closed by ARC.md A6.a — `run_parallel_queue_ref`
// now detects `hidden = true` attributes (added by `ref_return` for
// Vector / Reference / Enum-payload returns) and pre-allocates a
// 100-word backing store per hidden arg per row in the worker's
// output store namespace.  `execute_at_ref` gained a
// `hidden_dests: &[DbRef]` slice that gets pushed as 12 bytes per
// destination after the input arg and before regular extras —
// matching the parameter layout the codegen assumes.  Each
// destination's store is dispenser-allocated, so adoption +
// rebase + revive_record_chain pull it back into parent
// alongside the result DbRef.
#[test]
fn par_struct_to_vector_t4() {
    // Worker constructs and returns a vector per element.  Today
    // the runtime can't represent this.  After phase 1, the per-
    // worker output store holds vector records like any other.
    code!(
        "struct Score { value: integer }
fn replicate(s: const Score) -> vector<integer> {
    out: vector<integer> = [];
    for i in 0..s.value { out += [i]; }
    out
}
fn run() -> integer {
    sl: vector<Score> = [Score { value: 2 }, Score { value: 3 }];
    total_len = 0;
    for s in sl par(v = replicate(s), 4) { total_len += len(v); }
    total_len
}"
    )
    .expr("run()")
    .result(Value::Int(5));
}

// par-keyed-collection-return: closed by ARC.md A6.d — extended the
// parser's ref-mode matcher (return_size = -1 sentinel) plus
// early/late `route_ref_queue` gates to accept Type::Sorted / Hash /
// Index / Radix, sharing the existing DbRef-return Queue path.
// `data::owned_elements` already enumerates each keyed type's
// internal owned-DbRef fields so the rebase walk is correct without
// further machinery.
#[test]
fn par_struct_to_keyed_collection_t4() {
    code!(
        "struct Score { value: integer }
struct Tag { id: integer, label: text }
fn build_tags(s: const Score) -> sorted<Tag[id]> {
    out: sorted<Tag[id]> = [];
    out += Tag { id: s.value, label: \"v{s.value}\" };
    out += Tag { id: s.value + 100, label: \"hi\" };
    out
}
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }];
    total_len = 0;
    for s in sl par(t = build_tags(s), 4) { total_len += len(t); }
    total_len
}"
    )
    .expr("run()")
    .result(Value::Int(4));
}

// par-fn-return: closed by ARC.md A6.b — packed-buffer Queue route
// for fn-ref returns.  `Stores::par_fn_buffer_stack` (Vec<Vec<u8>>,
// 20B per row) holds each worker's 8B d_nr + 12B closure DbRef
// blob.  `run_parallel_queue_fn` writes rows directly via
// `execute_at_raw_to`; `n_parallel_buf_get_fn` reads 20 bytes and
// pushes them onto the operand stack.  Body substitution diverges
// from `replace_var_in_ir` for fn-ref returns: b_var stays a real
// variable with a 20-byte slot, and `Set(b_var, Call(buf_get_fn,
// [idx]))` is prepended to each iteration body so `f(10)` (parsed
// as `CallRef(b_var, [Int(10)])`) reads b_var's slot correctly.
//
// `parallel_buf_get_fn`'s declared return type is `fn(integer) ->
// integer` — chosen so `variables::size(Type::Function, Argument)`
// returns 20, matching the runtime push via `put_stack::<[u8; 20]>`.
#[test]
fn par_struct_to_fn_t4() {
    // Worker selects and returns a fn-ref based on its input.
    code!(
        "struct Score { value: integer }
fn dbl(x: integer) -> integer { x * 2 }
fn triple(x: integer) -> integer { x * 3 }
fn pick(s: const Score) -> fn(integer) -> integer {
    if s.value > 0 { dbl } else { triple }
}
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: -1 }, Score { value: 2 }];
    sum = 0;
    for s in sl par(f = pick(s), 4) { sum += f(10); }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(70));
}

// ─────────────────────────────────────────────────────────────────────
// Phase 9 canaries — tuple input/output for par.
//
// Plan-06 promises full type coverage; tuples are first-class.
// These canaries un-ignore in plan-06 phase 9 (after T1.8a's function
// tuple-return convention lands as 9a).
// See doc/claude/plans/06-typed-par/09-tuple-support.md.
// ─────────────────────────────────────────────────────────────────────

#[test]
fn par_tuple_input_int_int() {
    // Worker reads a tuple element and returns a primitive.
    code!(
        "fn first(p: const (integer, integer)) -> integer { p.0 }
fn run() -> integer {
    pairs: vector<(integer, integer)> = [(1, 10), (2, 20), (3, 30), (4, 40)];
    sum = 0;
    for p in pairs par(r = first(p), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(10));
}

#[test]
fn par_tuple_input_int_text() {
    // Worker reads a text element from a tuple input.
    code!(
        "fn label_len(p: const (integer, text)) -> integer { len(p.1) }
fn run() -> integer {
    pairs: vector<(integer, text)> = [(1, \"one\"), (2, \"two\"), (3, \"three\")];
    sum = 0;
    for p in pairs par(r = label_len(p), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(11));
}

#[test]
fn par_tuple_return_int_int() {
    // Worker returns a tuple of two integers; main collects sum of both elements.
    code!(
        "struct Score { value: integer }
fn split(s: const Score) -> (integer, integer) { (s.value, s.value * 2) }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }, Score { value: 3 }];
    sum = 0;
    for s in sl par(r = split(s), 4) { sum += r.0 + r.1; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(18));
}

#[test]
fn par_tuple_return_int_text() {
    // Worker returns a tuple containing text; rebase must translate the DbRef.
    code!(
        "struct Score { value: integer }
fn tag(s: const Score) -> (integer, text) { (s.value, \"v{s.value}\") }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }];
    total_len = 0;
    for s in sl par(r = tag(s), 4) { total_len += len(r.1); }
    total_len
}"
    )
    .expr("run()")
    .result(Value::Int(4));
}

#[test]
fn par_tuple_return_struct_text() {
    // Worker returns a tuple combining a struct ref and text;
    // rebase must walk both DbRef element offsets.
    //
    // Un-ignored 2026-05-08 once the P234 runtime fix routed
    // tuple-with-Reference returns through the synthetic struct.
    // s=1: Point{x=1,y=2} + "p1" → 1+2+2 = 5
    // s=2: Point{x=2,y=3} + "p2" → 2+3+2 = 7
    // sum = 12.  (Original ignore note had `Value::Int(11)`; that
    // was an author's miscount the canary couldn't expose because
    // the function-tuple-return ABI was broken before P234.)
    code!(
        "struct Point { x: integer, y: integer }
struct Score { value: integer }
fn make(s: const Score) -> (Point, text) {
    (Point { x: s.value, y: s.value + 1 }, \"p{s.value}\")
}
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }];
    sum = 0;
    for s in sl par(r = make(s), 4) { sum += r.0.x + r.0.y + len(r.1); }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(12));
}

#[test]
fn par_tuple_destructure_in_for() {
    // Tuple destructuring directly in the fused-for-par binding.
    code!(
        "fn add(a: integer, b: integer) -> integer { a + b }
fn run() -> integer {
    pairs: vector<(integer, integer)> = [(1, 2), (3, 4), (5, 6), (7, 8)];
    sum = 0;
    for (a, b) in pairs par(r = add(a, b), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(36));
}

// A collection element is a plain fn-ref (DESIGN_DECISIONS.md @C116): a capturing
// lambda is refused at the literal, and the message names the struct-field route.
// The non-capturing element beside it is the shape that passes, so the refusal is
// about the CAPTURE, not about a lambda in a vector.  (loft#1358)

#[test]
fn par_vec_of_capturing_fns_t4() {
    code!(
        "fn apply(f: fn(integer) -> integer) -> integer { f(10) }
fn run() -> integer {
    a = 2;
    fs: vector<fn(integer) -> integer> = [
        |x| { x * 2 },
        |x| { x * a }
    ];
    total = 0;
    for f in fs par(r = apply(f), 4) { total += r; }
    total
}"
    )
    .error(
        "a capturing closure cannot be stored in a collection: a collection has one element \
         layout and each capture set is its own record shape — hold it in a struct field, or \
         store a non-capturing fn that reads the state from a struct field at \
         par_vec_of_capturing_fns_t4:6:22",
    );
}

// ── Tuple-arity coverage canaries (phase 9 inventory follow-up) ──
//
// Phase 9's narrative says "any arity, any element type" but the existing 4
// canaries are all 2-arity scalar/text/Reference combinations.  Two
// canaries cover the "any-arity" claim concretely:
//   - 3-arity scalar tuple I/O
//   - Nested-tuple I/O (`((A, B), C)`)
// Both reuse the existing par_tuple_return ignore reason since they live in
// the same fix surface.

#[test]
fn par_tuple_return_three_arity() {
    code!(
        "struct Score { value: integer }
fn triple(s: const Score) -> (integer, integer, integer) {
    (s.value, s.value * 2, s.value * 3)
}
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }];
    sum = 0;
    for s in sl par(r = triple(s), 4) { sum += r.0 + r.1 + r.2; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(18)); // (1+2+3) + (2+4+6) = 18
}

#[test]
fn par_tuple_return_nested() {
    code!(
        "struct Score { value: integer }
fn pair_and_double(s: const Score) -> ((integer, integer), integer) {
    ((s.value, s.value + 1), s.value * 2)
}
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }];
    sum = 0;
    for s in sl par(r = pair_and_double(s), 4) { sum += r.0.0 + r.0.1 + r.1; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(14)); // ((1,2),2)+((2,3),4) → (1+2+2)+(2+3+4) = 5+9 = 14
}
