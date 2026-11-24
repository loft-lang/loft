// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Keyed collections in local variables, bounded generics, coroutines and `yield`, text
//! accumulators, and closures over locals.

use super::*;

// ── P188: local-var keyed collections ────────────────────────────────────────
// `out: sorted<T[key]> = []; out += T {...}; out` used to panic at
// `keys::mut_store` because the local's slot was never allocated a backing
// store: the slot allocator gave it a position but neither the bytecode
// codegen nor the native generator emitted an OpDatabase init for keyed
// collection locals.  After P188, `gen_set_first_keyed_null` (bytecode) and
// `emit_null_dbref`'s sorted/hash/index/spatial arm (native) allocate the
// store and zero the root pointer; subsequent `+= T {...}` operations grow
// the collection in place via record_new's Parts::Sorted/Hash/Index/Radix
// dispatch.
#[test]
fn p188_sorted_local_via_plus_equals() {
    code!(
        "struct P188Tag { id: integer, label: text }
fn build() -> sorted<P188Tag[id]> {
    out: sorted<P188Tag[id]> = [];
    out += P188Tag { id: 2, label: \"v2\" };
    out += P188Tag { id: 1, label: \"v1\" };
    out
}"
    )
    .expr("build().len()")
    .result(Value::Int(2));
}

/// P189 — `vector<(T1, T2, …)>` literal construction used to panic
/// at `src/parser/vectors.rs:1398` because `Type::Tuple` had no
/// `def_nr` (no `tuple_def` analogue of `vector_def`).  Fix: register
/// a synthetic struct (`__tuple<T1,T2,…>`) at parse time when
/// `sub_type` sees `vector<(...)>`, expose it via `type_def_nr` /
/// `type_elm`'s new Tuple arm.  This test pins the construction +
/// `len()` path; element ACCESS via `pairs[0].0` is still broken
/// (TupleGet reads the DbRef's bytes as inline tuple) and stays
/// out-of-scope here.
#[test]
fn p189_vector_tuple_literal_constructs() {
    code!(
        "fn build() -> integer {
    pairs: vector<(integer, integer)> = [(1, 10), (2, 20), (3, 30)];
    pairs.len()
}"
    )
    .expr("build()")
    .result(Value::Int(3));
}

/// P189c — `vector<(integer, integer)>` element bytes are now
/// written via per-attribute `set_field` calls in `new_record`'s
/// `Value::Tuple` arm (mirrors the struct-literal `Value::Insert`
/// path).  Verifies via the par worker (which reads the tuple via
/// the wide-input dispatch landed in 4d.A) that the bytes round-trip
/// correctly: each pair (i, i*10) has i+i*10 = 11*i, summed across
/// rows = 11*(1+2+3+4) = 110.  This avoids P189b's broken sequential
/// `pairs[0].0` access path by reading via the worker's slot 0
/// (which gets the raw 16 bytes pushed by execute_at_raw_primitive_input_wide).
#[test]
fn p189c_vector_tuple_element_bytes_written() {
    code!(
        "fn pair_sum(p: const (integer, integer)) -> integer { p.0 + p.1 }
fn run() -> integer {
    pairs: vector<(integer, integer)> = [(1, 10), (2, 20), (3, 30), (4, 40)];
    sum = 0;
    for p in pairs par(r = pair_sum(p), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(110));
}

/// Plan-06 phase 4d.B (sorted) — `for s in sorted_items par(...)`
/// no longer hangs the worker.  The desugar in
/// `parse_for` (collections.rs) detects keyed-collection input,
/// allocates a temp `vector<reference<T>>` via `materialise_keyed_for_par`,
/// walks the source via `OpIterate`/`OpStep` appending each element,
/// and re-routes par() to the materialised vector.  Closes the
/// `par_sorted_input_t4` canary; `par_hash_input_t4` and
/// `par_index_input_t4` still ignored (different interaction with
/// pre-existing iterator special-cases).
#[test]
fn p4d_b_par_over_sorted_via_materialise() {
    code!(
        "struct P4dScore { value: integer }
fn p4d_dbl(s: const P4dScore) -> integer { s.value * 2 }
fn run() -> integer {
    items: sorted<P4dScore[value]> = [];
    items += P4dScore { value: 30 };
    items += P4dScore { value: 10 };
    items += P4dScore { value: 20 };
    sum = 0;
    for s in items par(r = p4d_dbl(s), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(120));
}

/// P190 — `for x in <local sorted/hash/index>` used to panic at
/// `src/state/codegen.rs:1689` with "Too few parameters on
/// OpIterate (got 2, need 6)".  Root cause: P188 enabled local-var
/// keyed collections but `src/parser/vectors.rs::get_type` looked
/// up the database type-name (e.g. `sorted<Score[value]>`) which
/// is only registered via `fill_database` for struct fields.  Fix:
/// register the type on demand in `get_type` when the name lookup
/// misses — mirrors the struct-field path's `database.sorted` /
/// `database.hash` / `database.index` calls, idempotent so no
/// double-registration risk.
#[test]
fn p190_local_var_sorted_iteration() {
    code!(
        "struct P190Score { value: integer }
fn test() {
    items: sorted<P190Score[value]> = [];
    items += P190Score { value: 30 };
    items += P190Score { value: 10 };
    items += P190Score { value: 20 };
    sum = 0;
    for s in items { sum += s.value; }
    assert(sum == 60, \"sum={sum}, expected 60\");
}"
    )
    .result(Value::Null);
}

/// P188 follow-up — `field += elem` for keyed-collection fields
/// (hash/sorted/index/spatial<T[key]>) and for vector fields with
/// struct-literal RHS were broken.  Two bugs:
///
/// 1. The struct-literal RHS (`Score{name:"a", value:10}`) parses
///    with the LHS field as its target, so the field-init steps
///    wrote into the field's storage (overwriting the hash/index
///    root pointer) instead of into a fresh element record.  Fix:
///    after allocating a new element via `new_record_field_op`,
///    walk the steps and substitute the LHS field expression with
///    `Var(elm)` (`substitute_value` helper).
///
/// 2. The local-var `+=` codepath in `new_record` looked up the
///    keyed-collection's known_type via
///    `data.def(type_def_nr(lhs_tp)).known_type` — but
///    `type_def_nr` returns the GENERIC alias (`hash` / `index`),
///    not the specific `hash<Score[name]>` instantiation.  The
///    alias's known_type pointed at a Vector type, so
///    `record_finish` dispatched through `Parts::Vector` and
///    appended raw bytes instead of calling `hash::add` /
///    `tree::add`.  Fix: register the specific keyed-collection db
///    type directly (`database.hash(c, key)` / `index(c, key)` /
///    etc.) — idempotent with the gen_set_first_keyed_null and
///    typedef-walker registrations.
#[test]
fn p188_struct_field_hash_pluseq_struct_literal() {
    code!(
        "struct P188aScore { name: text, value: integer }
struct P188aDb { items: hash<P188aScore[name]> }
fn test() {
    db = P188aDb { items: [] };
    db.items += P188aScore { name: \"a\", value: 10 };
    db.items += P188aScore { name: \"b\", value: 20 };
    db.items += P188aScore { name: \"c\", value: 30 };
    assert(len(db.items) == 3, \"len={len(db.items)}, expected 3\");
    sum = 0;
    for s in db.items { sum += s.value; }
    assert(sum == 60, \"sum={sum}, expected 60\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p188_struct_field_index_pluseq_struct_literal() {
    code!(
        "struct P188bScore { name: text, value: integer }
struct P188bDb { items: index<P188bScore[name]> }
fn test() {
    db = P188bDb { items: [] };
    db.items += P188bScore { name: \"a\", value: 10 };
    db.items += P188bScore { name: \"b\", value: 20 };
    db.items += P188bScore { name: \"c\", value: 30 };
    sum = 0;
    for s in db.items { sum += s.value; }
    assert(sum == 60, \"sum={sum}, expected 60\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p188_local_var_hash_pluseq_struct_literal() {
    code!(
        "struct P188cScore { name: text, value: integer }
fn test() {
    h: hash<P188cScore[name]> = [];
    h += P188cScore { name: \"a\", value: 10 };
    h += P188cScore { name: \"b\", value: 20 };
    h += P188cScore { name: \"c\", value: 30 };
    assert(len(h) == 3, \"len={len(h)}, expected 3\");
    sum = 0;
    for s in h { sum += s.value; }
    assert(sum == 60, \"sum={sum}, expected 60\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p188_local_var_index_pluseq_struct_literal() {
    code!(
        "struct P188dScore { name: text, value: integer }
fn test() {
    ix: index<P188dScore[name]> = [];
    ix += P188dScore { name: \"a\", value: 10 };
    ix += P188dScore { name: \"b\", value: 20 };
    ix += P188dScore { name: \"c\", value: 30 };
    sum = 0;
    for s in ix { sum += s.value; }
    assert(sum == 60, \"sum={sum}, expected 60\");
}"
    )
    .result(Value::Null);
}

/// P189b — `vector<(T1, T2, …)>` index access used to return
/// garbage because `OpGetVector` returns a 12-byte `DbRef` to the
/// inline tuple bytes but `OpTupleGet` reads from the local slot
/// directly (assuming inline-on-stack representation).  Result:
/// `pairs[0].0` decoded the DbRef bytes (`store_nr | (rec << 32)`)
/// as `i64`, producing `21474836482` instead of `1` for `(1, 10)`.
///
/// Fix: when index/iter access on `vector<(T1, T2, …)>` produces a
/// DbRef, `unbox_tuple_from_dbref` (in `parser/fields.rs`) wraps
/// the DbRef in a fresh work-ref and emits per-element loads via
/// `get_val` (`OpGetInt` / `OpGetText` / etc.) into a
/// `Value::Tuple` so the assignment target receives the proper
/// stack-tuple representation.  Same helper handles text elements
/// correctly because `OpGetText` inflates the 4-byte heap pointer
/// to the 16-byte stack `Str`.
///
/// **Out of scope for this fix:** for-loop iteration `for p in pairs`
/// — the iteration's break-check (`if OpNot(loop_var) { break }`)
/// requires the loop var to be a DbRef so the null-sentinel works.
/// Wrapping the loop var as `RefVar(Tuple)` would propagate the
/// DbRef cleanly but `gen_set_first_at_tos` doesn't yet know how
/// to allocate a RefVar(Tuple) slot.  Use index access (`pairs[i].0`)
/// as a workaround until that codegen lands.
#[test]
fn p189b_vector_tuple_index_access() {
    code!(
        "fn test() {
    pairs: vector<(integer, integer)> = [(1, 10), (2, 20), (3, 30)];
    p = pairs[1];
    assert(p.0 == 2, \"p.0={p.0}, expected 2\");
    assert(p.1 == 20, \"p.1={p.1}, expected 20\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p189b_vector_tuple_int_text_index_access() {
    code!(
        "fn test() {
    pairs: vector<(integer, text)> = [(1, \"one\"), (2, \"two\"), (3, \"three\")];
    p = pairs[2];
    assert(p.0 == 3, \"p.0={p.0}, expected 3\");
    assert(len(p.1) == 5, \"len(p.1)={len(p.1)}, expected 5\");
}"
    )
    .result(Value::Null);
}

/// P189b — for-loop iteration over `vector<(T1, T2, …)>`.
///
/// Element binding (`for p in pairs`) used to fail with
/// `Field access not supported on type tuple([…])` because the
/// loop var was typed as the bare tuple.  The parser now retypes
/// it as `Reference(__tuple<…>)` so per-element loads (`p.0`,
/// `p.1`) route through the struct-style field-access path,
/// matching the index-access fix in `p189b_vector_tuple_index_access`.
#[test]
fn p189b_vector_tuple_for_loop_int_int() {
    code!(
        "fn test() {
    pairs: vector<(integer, integer)> = [(1, 10), (2, 20), (3, 30)];
    sum_a = 0;
    sum_b = 0;
    for p in pairs {
        sum_a += p.0;
        sum_b += p.1;
    }
    assert(sum_a == 6, \"sum_a={sum_a}, expected 6\");
    assert(sum_b == 60, \"sum_b={sum_b}, expected 60\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p189b_vector_tuple_for_loop_int_text() {
    code!(
        "fn test() {
    labels: vector<(integer, text)> = [(1, \"one\"), (2, \"two\"), (3, \"three\")];
    sum_id = 0;
    sum_len = 0;
    for q in labels {
        sum_id += q.0;
        sum_len += len(q.1);
    }
    assert(sum_id == 6, \"sum_id={sum_id}, expected 6\");
    assert(sum_len == 11, \"sum_len={sum_len}, expected 11\");
}"
    )
    .result(Value::Null);
}

/// P193 — eager init for `local: keyed_collection<T> = []`.
///
/// Without the fix, the empty `[]` literal parses to
/// `Value::Insert(empty)` which doesn't match codegen's
/// `Set(v, Null) → gen_set_first_keyed_null` arm — so the var
/// gets no init bytecode.  Lazy init then fires on first WRITE,
/// inside any enclosing loop body, re-allocating the data store
/// per iteration and overwriting the root pointer.  Symptom:
/// `for i in 0..N { ix += Score{id:i, value:i}; }` left
/// `len(ix) == 1` (only the last add) and leaked N stores.
///
/// Fix path:
/// - `parser/operators.rs::create_keyed` rewrites `Set(v, Insert([]))`
///   to `Set(v, Null)` for keyed-collection types so codegen's
///   gen_set_first_keyed_null fires at the declaration site.
/// - `data.rs::heap_dep` and `scopes.rs::get_free_vars` now
///   recognise Sorted/Hash/Index/Radix as heap-owned, so
///   scope-exit `OpFreeRef` is emitted (no more "stores not
///   freed" warnings on program exit).
#[test]
fn p193_local_var_index_init_then_loop_add() {
    code!(
        "struct P193aScore { id: integer, value: integer }
fn test() {
    ix: index<P193aScore[id]> = [];
    for i in 0..10 { ix += P193aScore { id: i, value: i }; }
    assert(len(ix) == 10, \"len={len(ix)}, expected 10\");
    sum = 0;
    for s in ix { sum += s.value; }
    assert(sum == 45, \"sum={sum}, expected 45\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p193_local_var_hash_init_then_loop_add() {
    code!(
        "struct P193bScore { id: integer, value: integer }
fn test() {
    h: hash<P193bScore[id]> = [];
    for i in 0..10 { h += P193bScore { id: i, value: i }; }
    assert(len(h) == 10, \"len={len(h)}, expected 10\");
    sum = 0;
    for s in h { sum += s.value; }
    assert(sum == 45, \"sum={sum}, expected 45\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p193_local_var_index_read_before_write() {
    // Reading the collection BEFORE any write used to panic with
    // "Incorrect var ix[65535] versus N" because the init never
    // emitted.  With eager init, len() returns 0 immediately.
    code!(
        "struct P193cScore { id: integer, value: integer }
fn test() {
    ix: index<P193cScore[id]> = [];
    assert(len(ix) == 0, \"empty len={len(ix)}, expected 0\");
}"
    )
    .result(Value::Null);
}

/// Scale test for the P188 += keyed-collection fix when the adds
/// happen OUTSIDE a loop and iteration runs over the result.  The
/// 3-element test above proves dispatch correctness; this test
/// proves the RB tree rebalance + iteration sequence hold for many
/// inserts (sum 0..49 = 1225).  After P193 closed eager init, the
/// loop-form scale test is also covered (`p193_local_var_index_init_then_loop_add`).
#[test]
fn p188_local_var_index_scale_50_elements_unrolled() {
    let mut body = String::from(
        "struct P188eScore { id: integer, value: integer }
fn test() {
    ix: index<P188eScore[id]> = [];\n",
    );
    for i in 0..50 {
        body.push_str(&format!(
            "    ix += P188eScore {{ id: {i}, value: {i} }};\n"
        ));
    }
    body.push_str(
        "    assert(len(ix) == 50, \"len={len(ix)}, expected 50\");
    sum = 0;
    n = 0;
    for s in ix { sum += s.value; n += 1; }
    assert(n == 50, \"iter count={n}, expected 50\");
    assert(sum == 1225, \"sum={sum}, expected 1225\");
}",
    );
    code!(&body).result(Value::Null);
}

/// P192 — `len()` was missing for `hash<T[key]>` and
/// `index<T[key]>` collections.  Only `vector` and `sorted` had
/// overloads.  Fix: added `OpLengthHash` (walks the bucket array
/// via `hash::count`) and `OpLengthIndex` (walks the red-black
/// tree via `tree::count`).  Hash gets a normal stdlib overload
/// (`pub fn len(self: hash)`); index uses a parser hook in
/// `src/parser/mod.rs::call()` because `OpLengthIndex` needs a
/// `const u16` bookkeeping-offset arg that's only computable at
/// parse time via `database.fields(tp)`.
#[test]
fn p192_len_hash_struct_field() {
    code!(
        "struct P192aScore { name: text, value: integer }
struct P192aDb { items: hash<P192aScore[name]> }
fn test() {
    db = P192aDb { items: [
        P192aScore { name: \"a\", value: 10 },
        P192aScore { name: \"b\", value: 20 },
        P192aScore { name: \"c\", value: 30 }
    ] };
    n = len(db.items);
    assert(n == 3, \"hash len={n}, expected 3\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p192_len_index_struct_field() {
    code!(
        "struct P192bScore { name: text, value: integer }
struct P192bDb { items: index<P192bScore[name]> }
fn test() {
    db = P192bDb { items: [
        P192bScore { name: \"a\", value: 10 },
        P192bScore { name: \"b\", value: 20 },
        P192bScore { name: \"c\", value: 30 }
    ] };
    n = len(db.items);
    assert(n == 3, \"index len={n}, expected 3\");
}"
    )
    .result(Value::Null);
}

/// P191 — `index<T[key]>` iteration produced wrong sums (e.g.
/// `sum=10` instead of `sum=60` for three `Score` records).  Root
/// cause: `database.index` (src/database/types.rs:957) appended
/// `#left_N` / `#right_N` bookkeeping fields with `content =
/// self.name("integer")` (8 bytes), but `tree::add` writes those
/// pointers via `set_i32_raw` at hardcoded offsets `[pos, pos+4,
/// pos+8]` — an alignment-aware layout placed the 8-byte fields 8
/// bytes apart, so tree pointers landed in the wrong record bytes
/// and the right-child link was never followed during iteration.
/// Fix: switch bookkeeping to 4-byte `int<0,false>` so the layout
/// matches `tree::add`'s offsets.
///
/// Verified by `validate_all_layouts_index_bookkeeping_after_p191_fix_no_issues`
/// in `src/database/types.rs::layout_tests`.
#[test]
fn p191_struct_field_index_iteration_after_layout_fix() {
    code!(
        "struct P191Score { name: text, value: integer }
struct P191Db { items: index<P191Score[name]> }
fn test() {
    db = P191Db { items: [
        P191Score { name: \"a\", value: 10 },
        P191Score { name: \"b\", value: 20 },
        P191Score { name: \"c\", value: 30 }
    ] };
    sum = 0;
    for s in db.items { sum += s.value; }
    assert(sum == 60, \"sum={sum}, expected 60\");
}"
    )
    .result(Value::Null);
}

/// Plan-06 phase 4d.A.2 diagnostic V1 — vector<fn-ref> index access.
///
/// Localises the bug behind `par_vec_of_fns_input_t4`'s infinite loop:
/// is vector<fn-ref> storage broken (root cause A/B), or is the par
/// dispatcher broken (root cause C)?
///
/// If this test fails: vector storage is broken — par is downstream.
/// If this test passes: storage works; check V2 (for-loop iteration)
/// and V3 (par with single element) next.
#[test]
fn p4d_a2_vector_fn_ref_index_access() {
    code!(
        "fn dbl(x: integer) -> integer { x * 2 }
fn apply(f: fn(integer) -> integer) -> integer { f(10) }
fn run() -> integer {
    fs: vector<fn(integer) -> integer> = [dbl];
    f = fs[0];
    apply(f)
}"
    )
    .expr("run()")
    .result(Value::Int(20));
}

/// Plan-06 phase 4d.A.2 diagnostic V2 — non-par for-loop iteration.
///
/// Keeps the canary's `for f in fs` shape but drops `par(...)` so we
/// can see if iteration alone hangs or if par-specific dispatch is the
/// culprit.  If this hangs too: the bug is in for-loop iteration codegen
/// for vector<fn-ref>.  If this returns 20: the bug is par-specific.
#[test]
// @P343 FIXED 2026-05-26: `for f in fs` over a `vector<fn-ref>` now
// dispatches each element (was: returned 0 because the loop broke on
// iteration 0).  The parse-failure ignore reason was stale.  Native is
// covered by `tests/scripts/repro_p343.loft` (runs under both backends).
fn p4d_a2_vector_fn_ref_for_loop() {
    code!(
        "fn dbl(x: integer) -> integer { x * 2 }
fn apply(f: fn(integer) -> integer) -> integer { f(10) }
fn run() -> integer {
    fs: vector<fn(integer) -> integer> = [dbl];
    total = 0;
    for f in fs { total += apply(f); }
    total
}"
    )
    .expr("run()")
    .result(Value::Int(20));
}

/// Plan-06 phase 4d.A.2 diagnostic V3 — par with single element.
///
/// If V1/V2 fail but V3 works (or hangs): the parse path is gated on
/// `par(...)` syntax.  V3 hanging at runtime would cleanly localise
/// the bug to the par dispatcher (matching the canary).
// p4dA2 — the par-dispatch HANG over `vector<fn-ref>` input is FIXED on the interpreter
// (this test, formerly timing out at 15s, now completes). The `--native` E0308 residual
// (par-fnref delivery emitting a bare `DbRef` where `(u32, DbRef)` is expected) is also
// FIXED (@PLN90 W6: `tuple_arg_prep` gained a `Type::Function` arm). Native coverage lives
// in `tests/scripts/507-par-vector-fnref.loft` (runs under both backends); this `code!`
// test keeps the interpreter-side hang lock-in.
#[test]
fn p4d_a2_par_vector_fn_ref_single() {
    code!(
        "fn dbl(x: integer) -> integer { x * 2 }
fn apply(f: fn(integer) -> integer) -> integer { f(10) }
fn run() -> integer {
    fs: vector<fn(integer) -> integer> = [dbl];
    total = 0;
    for f in fs par(r = apply(f), 1) { total += r; }
    total
}"
    )
    .expr("run()")
    .result(Value::Int(20));
}

/// P194 — tuple-typed struct field reassignment.
///
/// Before the fix: `p.v = (100, 200)` triggered the "Tuple
/// destructuring requires plain variable names" diagnostic because
/// `get_val::Type::Tuple` returns `Value::Tuple([reads])` for the
/// tuple field read, which then matched the destructuring branch
/// in `parse_assign`.  The fix routes a tuple-of-reads LHS through
/// `emit_tuple_set_ops` instead.
#[test]
fn p194_tuple_field_reassign() {
    code!(
        "struct Pair { v: (integer, integer) }
fn run() -> integer {
    p = Pair { v: (3, 4) };
    p.v = (100, 200);
    p.v.0 + p.v.1
}"
    )
    .expr("run()")
    .result(Value::Int(300));
}

/// P194 — tuple-typed reassignment with multiple writes.
#[test]
fn p194_tuple_field_reassign_twice() {
    code!(
        "struct Pair { v: (integer, integer) }
fn run() -> integer {
    p = Pair { v: (3, 4) };
    p.v = (100, 200);
    p.v = (500, 600);
    p.v.0 + p.v.1
}"
    )
    .expr("run()")
    .result(Value::Int(1100));
}

/// P197 — returning a `text` element from a tuple struct field.
///
/// Before the fix:
/// - Index `.0` returned garbage characters (`Str` with a dangling
///   ptr into a freed host record).
/// - Index `.1` and indices in larger tuples hard-crashed with
///   `ptr::copy_nonoverlapping requires that both pointer arguments
///   are aligned and non-null`.
/// - Native codegen produced a `(String, String)` work-var temp,
///   then borrowed `&temp.0` past its drop — `rustc` rejected with
///   `borrowed value does not live long enough`.
///
/// Two-part fix:
/// 1. `Type::depending`/`Type::depend` now recurse into
///    `Type::Tuple` elements so a tuple struct field read carries
///    the host as a dep on each text/reference element.
/// 2. `parse_part`'s tuple-index branch short-circuits when `code`
///    is already a literal `Value::Tuple([reads])` — return the
///    indexed read directly instead of materialising a
///    `(String, String)` work-var temp.
#[test]
fn p197_text_returned_from_tuple_field() {
    code!(
        "struct A { v: (text, text) }
fn first() -> text {
    a = A { v: (\"hello\", \"world\") };
    a.v.0
}"
    )
    .expr("first()")
    .result(Value::Text("hello".to_string()));
}

#[test]
fn p197_text_returned_from_tuple_field_index_one() {
    code!(
        "struct A { v: (text, text) }
fn second() -> text {
    a = A { v: (\"hello\", \"world\") };
    a.v.1
}"
    )
    .expr("second()")
    .result(Value::Text("world".to_string()));
}

#[test]
fn p197_text_returned_from_mixed_tuple_field() {
    code!(
        "struct P { v: (integer, integer, text) }
fn third() -> text {
    p = P { v: (1, 2, \"hello\") };
    p.v.2
}"
    )
    .expr("third()")
    .result(Value::Text("hello".to_string()));
}

// plan-17 phase 01 regressions — bounded-generic / interface validation.

/// plan-17/01 (C): built-in `integer` satisfies `Printable` automatically
/// per the documented contract.  Before the stdlib `to_text` impls landed,
/// `<T: Printable>(v: vector<T>)` rejected `vector<integer>` with
/// "'integer' does not satisfy interface 'Printable': missing to_text".
#[test]
fn plan17_printable_integer_satisfies() {
    code!(
        "fn first<T: Printable>(v: vector<T>) -> text { v[0].to_text() }
fn run() -> text {
    nums: vector<integer> = [10, 20, 30];
    first(nums)
}"
    )
    .expr("run()")
    .result(Value::Text("10".to_string()));
}

/// plan-17/01 (A): `<T: Bound>(...) -> (T, T)` must monomorphise the
/// return type's tuple element types via `substitute_type`.  Before the
/// fix the function signature stayed `(DbRef, DbRef)` (parametric T form)
/// while parameters substituted to `i64`, causing native E0308.  Explicit
/// element-type annotation is needed today because implicit type-inference
/// from generic-call results doesn't yet propagate the substituted return
/// type — see plan-17 phase 01 follow-up.
#[test]
fn plan17_generic_tuple_return_with_annotation() {
    code!(
        "fn min_max<T: Ordered>(a: T, b: T) -> (T, T) {
    if a < b { (a, b) } else { (b, a) }
}
fn run() -> integer {
    t: (integer, integer) = min_max(7, 3);
    t.0 * 10 + t.1
}"
    )
    .expr("run()")
    .result(Value::Int(37));
}

/// plan-17/01 (A) caveat — closed 2026-05-04.  Two coordinated changes:
/// new `predict_generic_return_type` helper (pure read, no def
/// mutation), and first-pass dispatch in `parser/mod.rs::call`.
/// Was: `t = min_max(7, 3)` without explicit type annotation typed
/// `t` as `Type::Unknown` because `try_generic_instantiation` was
/// second-pass-only; downstream `t.0` rejected with "Expect token ;"
/// (parser doesn't see Tuple on Unknown receiver), and that error
/// aborted second pass entirely.  Now: the prediction helper computes
/// the substituted return type on first pass without creating the
/// monomorphised def (which would otherwise capture stale first-pass
/// body IR).  The receiving variable gets the right Tuple type from
/// first pass; `t.0` parses correctly; second pass runs full
/// instantiation as before.
#[test]
fn plan17_a_implicit_generic_tuple_type_inference() {
    code!(
        "fn min_max<T: Ordered>(a: T, b: T) -> (T, T) {
    if a < b { (a, b) } else { (b, a) }
}
fn run() -> integer {
    t = min_max(7, 3);
    t.0 * 10 + t.1
}"
    )
    .expr("run()")
    .result(Value::Int(37));
}

/// P212 — closed 2026-05-04.  Nested tuple literals
/// (`((1,2),(3,4))`, triply nested, etc.) panicked at
/// `src/state/codegen.rs:1527` because the inline match in
/// `gen_set_first_at_tos`'s `Type::Tuple` arm had no case for an
/// inner element of `Type::Tuple(_)` — it fell through to the
/// "unsupported elem" panic.  Fix extracts the per-leaf
/// `OpPut*` emission into a recursive helper
/// `emit_tuple_put_ops` that descends through nested tuples,
/// computing each leaf's absolute slot offset.  Iteration is
/// reverse-order to match the depth-first push order used by
/// tuple-literal evaluation.
#[test]
fn p212_nested_tuple_literal() {
    code!(
        "fn run() -> integer {
    t = ((1, 2), (3, 4));
    t.0.0 * 1000 + t.0.1 * 100 + t.1.0 * 10 + t.1.1
}"
    )
    .expr("run()")
    .result(Value::Int(1234));
}

/// P212 follow-up — triply nested tuple literal `(1, (2, (3, 4)))`.
#[test]
fn p212_triply_nested_tuple_literal() {
    code!(
        "fn run() -> integer {
    t = (1, (2, (3, 4)));
    t.0 * 1000 + t.1.0 * 100 + t.1.1.0 * 10 + t.1.1.1
}"
    )
    .expr("run()")
    .result(Value::Int(1234));
}

/// P210 — closed 2026-05-04.  Native coroutine `while … { yield … }`
/// silently returned 0 because `collect_segments` in
/// `src/generation/coroutine.rs` only recognised `Value::Block`
/// containing yields (the for-loop shape) and missed `Value::Loop`
/// (the while-loop shape).  The state machine ended up with no arms,
/// so every `next_i64` call returned `COROUTINE_EXHAUSTED` and the
/// driving for-loop broke immediately.  Interp drives generators via
/// the bytecode VM, not the state-machine lowering, so it was
/// unaffected.  Fix extends the matcher to `Value::Block(_) |
/// Value::Loop(_)`.
#[test]
fn p210_native_coroutine_while_yield() {
    code!(
        "fn count_to(n: integer) -> iterator<integer> {
    i = 0;
    while i < n {
        yield i;
        i = i + 1;
    }
}
fn run() -> integer {
    sum = 0;
    for v in count_to(5) {
        sum = sum + v;
    }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(10));
}

/// P211 — closed 2026-05-05.  Coroutine `yield text` now round-trips
/// through the native state-machine.  The trait `LoftCoroutine` only
/// had `next_i64`, so the lowering for a `iterator<text>` generator
/// emitted `return ("alice") as i64;` (rejected by rustc with E0606
/// "casting `&'static str` as `i64` is invalid"), and the consumer's
/// `OpCoroutineNext` size-16 fallback emitted `... as i32` followed
/// by `.to_string()` (E0425 "cast cannot be followed by a method
/// call").  Fix adds `next_text` next to `next_i64` on the trait
/// (each defaulted to its type's exhaustion sentinel), routes
/// text-yielding generators to override `next_text`, and dispatches
/// `OpCoroutineNext` size 16 (= `size_of::<&str>()`) to a new
/// `coroutine_next_text` runtime helper.  Interp drove the test
/// through the bytecode VM (which already supported size-16 yields)
/// so it always worked; this test pins the native fix via the
/// `tests/scripts/51-coroutines.loft` integration test that runs
/// under `--native`, plus an interp-side smoke check here.
#[test]
fn p211_coroutine_yield_text() {
    // Sum the character lengths of three yielded texts.  Length-sum
    // guards against both the original interp symptom (empty stdout)
    // and the native codegen errors (E0606 / E0425) that motivated
    // the fix, while sidestepping a separate text-concat issue
    // (`out + s + ","` codegen) that's unrelated to P211.
    code!(
        "fn names() -> iterator<text> {
    yield \"alice\";
    yield \"bob\";
    yield \"carol\";
}
fn run() -> integer {
    total = 0;
    for s in names() { total = total + s.len(); }
    total
}"
    )
    .expr("run()")
    .result(Value::Int(13));
}

/// P211 follow-up — text yield driven by a `while` loop.  Combines
/// P210's `Value::Loop` arm in `collect_segments` with the new
/// `next_text` channel — the state machine must collect a `Loop`
/// segment AND emit `next_text` (not `next_i64`) when the yield
/// type is `text`.
#[test]
fn p211_coroutine_yield_text_while() {
    code!(
        "fn ticks_for_p211(n: integer) -> iterator<text> {
    i = 0;
    while i < n {
        yield \"tick\";
        i = i + 1;
    }
}
fn run() -> integer {
    seen = 0;
    for t in ticks_for_p211(3) { if t == \"tick\" { seen = seen + 1; } }
    seen
}"
    )
    .expr("run()")
    .result(Value::Int(3));
}

/// P219 — closed 2026-05-05.  Vector-element ForLoopBody in a
/// generator emitted invalid Rust for the eager-collect factory.
/// Root cause: scopes' `insert_free` adds `Return(Null)` at the
/// end of the function body block; `output_block`'s
/// `patch_hoisted_returns` (Pass 2) coalesces `[…, Loop(…),
/// Return(Null)]` into `[…, Return(Loop(…))]`; `Value::Return(Loop)`
/// then emits as `return 'l4: loop {…}`, which is invalid because
/// the loop is unit-typed and the factory expects
/// `Box<dyn LoftCoroutine>`.  Range-for didn't trip this because
/// its IR shape doesn't include a top-level `Loop` operator that
/// the patch can pair with the trailing `Return(Null)`.  Fix in
/// `src/generation/coroutine.rs::emit_for_body_factory`: strip
/// trailing `Return` ops from the body's operator list before
/// `generate_expr_buf` runs.  The factory drives the body purely
/// for its yield side effects (populates `__values`); the actual
/// factory return is `Box::new(struct_name { … })` emitted after.
#[test]
fn p219_vector_for_yield_in_generator() {
    code!(
        "fn nums() -> iterator<integer> {
    for n in [10, 20, 30] {
        yield n;
    }
}
fn run() -> integer {
    sum = 0;
    for x in nums() { sum = sum + x; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(60));
}

/// P219 follow-up — text yield from a vector-for body.  Same fix
/// site, different yield type.
#[test]
fn p219_vector_for_yield_text() {
    code!(
        "fn names() -> iterator<text> {
    for n in [\"a\", \"bb\", \"ccc\"] {
        yield n;
    }
}
fn run() -> integer {
    total = 0;
    for s in names() { total = total + s.len(); }
    total
}"
    )
    .expr("run()")
    .result(Value::Int(6)); // 1 + 2 + 3
}

/// P223 — closed 2026-05-05.  Self-prepend `s = "literal" + s`
/// produced wrong output on both backends.  Two compounding bugs:
/// (a) `parse_operators` captured `orig_var = s` BEFORE the
///     recursive parse filled `code` — for a literal-first concat,
///     `code` ends up as `Text("literal")`, but `orig_var` still
///     pointed at `s`, so `parse_append_text` used `s` as the
///     accumulator and emitted `OpAppendText(s, "literal")` as the
///     first op.  Combined with `assign_text`'s pre-clear when
///     no self-append is detected, this destroyed `s`'s original
///     content.
/// (b) `code_references_var` (used by `assign_text` to decide when
///     to wrap the RHS in a protective work-text) didn't walk
///     `Value::Block` — the work-text Block produced by
///     `parse_append_text` for `"lit" + var` carries `Var(var)`
///     deep inside, so the wrap was skipped and the interpreter's
///     clear-before-evaluate text-Set semantics destroyed the
///     content before reading.
/// Fix: (1) `parse_operators` only passes `orig_var` to
/// `parse_append_text` when `code` (unspanned) still equals
/// `Var(orig_var)` after recursion; falls back to `u16::MAX`
/// otherwise.  (2) `code_references_var` walks `Value::Block`.
/// (3) `Parser::append_to_text` (RefVar(Text) parameter path)
/// gained the same self-reference wrap as `assign_text`, so the
/// text-return-buffer case gets the same protection.  (4) Native
/// codegen's `Set(RefVar(Text), …)` emission wraps the RHS in
/// parens before appending `.to_string()` to fix Rust method-call
/// precedence (`&var.to_string()` parses as `&(var.to_string())`,
/// E0308; `(&var).to_string()` is correct).
#[test]
fn p223_self_prepend_local_text() {
    code!(
        "fn run() -> text {
    s = \"world\";
    s = \"hello \" + s;
    s
}"
    )
    .expr("run()")
    .result(Value::Text("hello world".to_string()));
}

/// P223 follow-up — RefVar(Text) parameter path (text-returning
/// function).  Same shape but exercises `append_to_text`'s wrap
/// rather than `assign_text`'s.
#[test]
fn p223_self_prepend_in_text_returning_fn() {
    code!(
        "fn run() -> text {
    out = \"end\";
    out = \"start: \" + out;
    out
}"
    )
    .expr("run()")
    .result(Value::Text("start: end".to_string()));
}

/// P218 — closed 2026-05-05.  Coroutine yielding a format string
/// that interpolates a captured parameter rejected under native with
/// E0425 ("cannot find value `var___work_2` in this scope").  The
/// IR's function-entry `Set(__work_N, "")` ops were emitted inside
/// state 0's match arm via `let mut var___work_N: String = …`,
/// scoping the binding to arm 0 only — every later arm referencing
/// the same buffer (e.g. a sibling yield with another format string)
/// failed to compile.  Two emit sites needed the fix: `emit_next_i64`
/// (the state-machine method body) and `emit_for_body_factory` (the
/// eager-collect factory used when the body contains a for-loop or
/// while-loop with yields).  Both now pre-declare `__work_*` text
/// locals at function scope before the per-state code, then mark
/// them in `self.declared` so the per-state Set ops emit as plain
/// assignments.  Pinned by `tests/issues.rs::p218_*` and the
/// extended `tests/scripts/51-coroutines.loft` (covers both backends
/// via `tests/native.rs`).
#[test]
fn p218_coroutine_yield_format_with_param() {
    code!(
        "fn greet(who: text) -> iterator<text> {
    yield \"hello, {who}\";
    yield \"bye, {who}\";
}
fn run() -> integer {
    total = 0;
    for s in greet(\"world\") { total = total + s.len(); }
    total
}"
    )
    .expr("run()")
    .result(Value::Int(22)); // "hello, world" (12) + "bye, world" (10)
}

/// P218 follow-up — same fix-shape but on the eager-collect factory
/// path (`emit_for_body_factory`).  A `while` body yielding a format
/// string that interpolates both a captured parameter AND a
/// state-machine local (the loop counter) needs the work-buffer
/// pre-declaration on the factory side.
#[test]
fn p218_coroutine_while_yield_format() {
    code!(
        "fn enumerate_for_p218(label: text, n: integer) -> iterator<text> {
    i = 0;
    while i < n {
        yield \"{label}={i}\";
        i = i + 1;
    }
}
fn run() -> integer {
    total = 0;
    for s in enumerate_for_p218(\"x\", 3) { total = total + s.len(); }
    total
}"
    )
    .expr("run()")
    .result(Value::Int(9)); // "x=0" (3) + "x=1" (3) + "x=2" (3)
}

/// P217 — closed 2026-05-05.  Text-accumulator pattern `out = out + …`
/// emitted `OpAppendText(out, Var(out))` as the FIRST op (self-append),
/// followed by the per-piece appends.  Interp doubled the existing
/// content (`"x"; out = out + "y"` → `"xxy"`); native rejected with
/// E0502 ("cannot borrow `*var_out` as mutable because it is also
/// borrowed as immutable") because the lowering produced
/// `*var_out += &*(&*var_out);`.  Fix in
/// `src/parser/vectors.rs::parse_append_text` detects when the first
/// piece is `Value::Var(orig_var)` (the destination itself, possibly
/// Span-wrapped) and skips the redundant initial append.  The
/// accumulator already holds the correct starting value; emitting
/// the self-append corrupted it.
#[test]
fn p217_text_self_accumulator() {
    code!(
        "fn run() -> text {
    out = \"x\";
    out = out + \"y\";
    out
}"
    )
    .expr("run()")
    .result(Value::Text("xy".to_string()));
}

/// P220 — closed 2026-05-05.  Empty `""` literals stored in a
/// `vector<text>` and then deep-copied (via `vector_add`, struct
/// field assignment, `OpCopyRecord`, parallel-worker boundary etc.)
/// were silently re-classified as `null` on the destination.  Root
/// cause was `Stores::copy_claims` in `src/database/allocation.rs`:
/// the text arm checked the *string content* via `s.is_empty()` to
/// decide whether to allocate or write the null sentinel.  But
/// `get_str(0)` returns `STRING_NULL` (`"\0"`, len 1) for a null
/// source, so the empty-content check never fired for genuine
/// nulls — it ONLY fired when the source had a real allocated
/// empty string.  Result: copying a `""` element produced `null`
/// on the destination.  Surfaced during TIC_TAC_TOE v2 development
/// (the v2 server's comment at
/// `lib/game_protocol/examples/tictactoe_server_v2.loft:53-56`
/// recorded the workaround discovery — "use null consistently").
/// Fix discriminates on the source `cur` field (non-zero =
/// allocated, regardless of length), preserving null on
/// null-source and round-tripping `""` correctly.
#[test]
fn p220_empty_string_in_vector_text_round_trips_through_struct_field() {
    code!(
        "struct G { cells: vector<text> }
fn run() -> integer {
    cs: vector<text> = [];
    for _ in 0..3 { cs += [\"\"]; }
    g = G { cells: cs };
    if g.cells[0] == \"\" { 1 } else { 0 }
}"
    )
    .expr("run()")
    .result(Value::Int(1));
}

/// P220 follow-up — null sentinel must still be preserved across
/// the same deep-copy path.  A `null` source must NOT become an
/// allocated empty-string after a round-trip; it must stay null.
/// (The original buggy code conflated empty content with null —
/// the fix's discriminator is the source `cur` field, so this
/// test pins the null-preserving behaviour.)
#[test]
fn p220_null_text_preserved_through_struct_field() {
    code!(
        "struct G { cells: vector<text> }
fn run() -> integer {
    cs: vector<text> = [];
    cs += [null];
    g = G { cells: cs };
    if g.cells[0] == null { 1 } else { 0 }
}"
    )
    // loft#1232 — the store into a dense `vector<text>` element now says so.  This test is
    // about the null SURVIVING the field round-trip, and `(N-Store)` warns without changing
    // that: the slot reserves its null distinctly, so the store proceeds and reads back null,
    // which is exactly what the result below asserts.
    .warning(concat!(
        "`null` is stored into element 0 of this vector literal of the non-null scalar type ",
        "`text` — the slot holds null; declare it `text?` to make that explicit at ",
        "p220_null_text_preserved_through_struct_field:4:12"
    ))
    .expr("run()")
    .result(Value::Int(1));
}

/// P217 follow-up — three-piece accumulator `out = out + a + b` (the
/// idiom that surfaced via P211's text-yield concat).  Without the
/// fix, the lowering emitted `OpAppendText(out, Var(out));
/// OpAppendText(out, "a"); OpAppendText(out, "b")` so the
/// destination's existing value was duplicated before each new piece
/// was appended.
#[test]
fn p217_text_accumulator_chain() {
    code!(
        "fn run() -> text {
    out = \"\";
    for s in [\"alice\", \"bob\", \"carol\"] {
        out = out + s + \",\";
    }
    out
}"
    )
    .expr("run()")
    .result(Value::Text("alice,bob,carol,".to_string()));
}

/// P209 — closed 2026-05-04.  Match guard arms with pattern bindings
/// (`x if x < 0 => …`) saw the binding variable as uninitialised
/// because the binding `v_set(x, subject)` was prepended only to the
/// arm body, not to the guard expression.  Result: `x` read as 0
/// inside the guard, so `x if x < 0` failed for every input on both
/// backends, and `x if x == 0` matched everything (interp shifted
/// arms by one; native always returned arm 2).  Fix in
/// `src/parser/control.rs::parse_scalar_match` wraps the guard in a
/// `binding_guard` block whose statements run the bindings first
/// then evaluate the guard.  The enum-variant struct-field path at
/// `build_scalar_chain`'s call site already did this correctly; the
/// scalar-match path was the missing case.
#[test]
fn p209_scalar_match_guard_sees_pattern_binding() {
    // Three-arm classify: input -3 must reach arm 1 (`x < 0`).
    code!(
        "fn classify(n: integer) -> text {
    match n {
        x if x < 0 => \"neg\",
        x if x == 0 => \"zero\",
        _ => \"pos\",
    }
}
fn run() -> text {
    \"{classify(-3)}|{classify(0)}|{classify(7)}\"
}"
    )
    .expr("run()")
    .result(Value::Text("neg|zero|pos".to_string()));
}

/// plan-19 phase 03 — closed 2026-05-04.  Method-on-parent-enum
/// dispatch.  When a method is declared on an enum
/// (`fn classify(self: Shape)`) and called via `.method()` syntax on
/// a variant value (`s = Circle { … }; s.classify()`), the parser
/// previously rejected with "Unknown field Circle.classify".  The
/// fix in `parser/fields.rs` looks up the method on the parent
/// enum's namespace (`t_<n>Shape_<method>`) before emitting the
/// unknown-field error, runs on both passes so the call's return
/// type propagates into first-pass inference of the enclosing
/// variable, and dispatches via `parse_method`.
#[test]
fn plan19_method_on_enum_variant_via_dot() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect { w: float, h: float },
}
fn classify(self: Shape) -> float {
    match self {
        Circle { radius } => 3.14 * radius * radius,
        Rect { w, h } => w * h,
    }
}
fn run() -> float {
    s = Circle { radius: 2.0 };
    s.classify()
}"
    )
    .expr("run()")
    .result(Value::Float(12.56));
}

/// plan-17/01 (B) — closed 2026-05-04.  Two coordinated fixes: the
/// I7 bounded-method dispatch in fields.rs now runs on both passes,
/// and definitions.rs installs bounds plus t-stubs on the first pass
/// too (was second-pass-only).  Was: `<T: Printable>(x: T) -> text`
/// with body `x.to_text() + "!"` rejected with "No matching operator
/// '+' on 'unknown(0)' and 'text'" because `x.to_text()` returned
/// `Type::Unknown(0)` on first pass.  Now: bounds and t-stubs install
/// on both passes (forward-decl tolerated via silent skip when the
/// interface isn't yet known); the I7 dispatch runs on both passes
/// and returns the bound's declared method return type from first
/// pass onward.  The receiving variable (`s` in `s = x.to_text()`)
/// is correctly typed `text`, and downstream operators like `s
/// concat-op "!"` resolve cleanly.
#[test]
fn plan17_b_bounded_method_return_type_propagates() {
    code!(
        "fn label<T: Printable>(x: T) -> text {
    x.to_text() + \"!\"
}
fn run() -> text { label(42) }"
    )
    .expr("run()")
    .result(Value::Text("42!".to_string()));
}

/// P224 — closed 2026-05-05.  A coroutine yielded a value derived from
/// a function-local variable (declared inside the generator body, not a
/// parameter); native rejected with E0425 because the local was
/// declared as a `let mut` inside state-arm 0's match arm and out of
/// scope from arm 1+.  Even with arm-scope fixed, the value would not
/// have persisted across `next_*` calls (each call's stack is fresh).
/// Fix promotes non-argument coroutine-body locals (primitive + text
/// types) to fields on the generator struct so writes from one state
/// arm survive into the next.  See `coroutine_persistent_locals` in
/// `src/generation/coroutine.rs`.
#[test]
fn p224_coroutine_local_int_capture() {
    code!(
        "fn gen() -> iterator<integer> {
    n = 10;
    yield n + 1;
    yield n + 2;
}
fn run() -> integer {
    total = 0;
    for x in gen() { total = total + x; }
    total
}"
    )
    .expr("run()")
    .result(Value::Int(23)); // 11 + 12
}

/// P224 — text counterpart.  A text-typed function-local interpolated
/// into a yielded format string.  Same root cause + fix as the
/// integer case; verifies the persistent-locals mechanism handles
/// `Type::Text` (factory-init `String::new()`, `&self.var_X` reads,
/// `self.var_X = (…).to_string()` writes).
#[test]
fn p224_coroutine_local_text_capture() {
    code!(
        "fn gen() -> iterator<text> {
    name = \"alice\";
    yield \"hi, {name}\";
    yield \"bye, {name}\";
}
fn run() -> integer {
    total = 0;
    for s in gen() { total = total + s.len(); }
    total
}"
    )
    .expr("run()")
    .result(Value::Int(19)); // "hi, alice" (9) + "bye, alice" (10)
}

/// P225 — closed 2026-05-05.  `yield from` mixed with `Simple` yields
/// in the same generator produced duplicate output of the first yield
/// on native.  Root cause: the eager-collect factory pushes ALL
/// segments (Simple, YieldFrom, ForLoopBody) into `__values`, but
/// `emit_next_i64` ALSO emitted per-segment match arms that
/// re-executed the Simple yield via `return val`.  State 0 returned
/// `"start: hi"` directly, then state 1 popped `__values[0]` which
/// was also `"start: hi"`.  Fix: when any segment is `ForLoopBody`,
/// the impl collapses to a single pop-from-buffer arm — the factory
/// owns all the work.
#[test]
fn p225_yield_from_mixed_with_simple_yields() {
    code!(
        "fn inner(s: text) -> iterator<text> {
    yield \"mid: {s}\";
}
fn outer(s: text) -> iterator<text> {
    yield \"start: {s}\";
    yield from inner(s);
    yield \"end: {s}\";
}
fn run() -> integer {
    total = 0;
    for x in outer(\"hi\") { total = total + x.len(); }
    total
}"
    )
    .expr("run()")
    // "start: hi" (9) + "mid: hi" (7) + "end: hi" (7) = 23
    .result(Value::Int(23));
}

/// P227 — closed 2026-05-05.  Calling a `fn(...) -> text` value via
/// fn-ref dispatch crashed both backends, regardless of where the
/// fn-ref lived (local variable, struct field, parameter) or whether
/// the lambda captured.
///
/// Two independent root causes:
///
/// 1. **Native** dispatch wrapper allocated `_fnref_work` block-scoped,
///    so the lambda's returned `Str` borrowed a buffer that dropped
///    before the outer `.to_string()` read its bytes ⇒
///    `ptr::copy_nonoverlapping` UB.
/// 2. **Interp** parser sites in `parser/control.rs` and
///    `parser/operators.rs` allocated work-buffers as
///    `(0..deps.len())` where `deps = []` for fn-ref types ⇒ zero
///    buffers pushed ⇒ lambda's stack slot for `__work_1` read
///    garbage ⇒ SIGSEGV.
///
/// Fix:
/// - Parser allocates exactly ONE `work_text` var when the fn-ref
///   return type is `Type::Text`, regardless of `deps.len()`.  The
///   buffer lives at caller-function scope.
/// - Native `output_call_ref` (a) detects hidden attrs by TYPE
///   (`Type::RefVar(Type::Text)`) instead of name (work-buffer attrs
///   are named after user-shadowed text vars, not `__work_*`); (b)
///   strips the trailing work-buffer arg from candidate matching;
///   (c) threads the parser-injected `&mut String` into the dispatch
///   arm via `_farg_<n>` instead of allocating its own block-scope
///   buffer.
///
/// Pinned by p227_text_fn_ref_local_call,
/// p227_text_fn_ref_struct_field, p227_text_fn_ref_capturing_closure,
/// p227_text_fn_ref_via_parameter.
#[test]
fn p227_text_fn_ref_local_call() {
    code!(
        "fn run() -> text {
    f: fn(integer) -> text = fn(n: integer) -> text { \"v: {n}\" };
    f(42)
}"
    )
    .expr("run()")
    .result(Value::Text("v: 42".to_string()));
}

/// P227 — capturing closure assigned to a local fn-ref variable.
/// Verifies the closure record is correctly threaded alongside the
/// work-buffer arg (closure DbRef + `&mut String` are distinct
/// hidden parameters; the dispatch synth-args walk must place each
/// at the right attribute slot).
#[test]
fn p227_text_fn_ref_local_with_capture() {
    code!(
        "fn run() -> text {
    label = \"hi\";
    f: fn(integer) -> text = fn(n: integer) -> text { \"{label}: {n}\" };
    f(42)
}"
    )
    .expr("run()")
    .result(Value::Text("hi: 42".to_string()));
}

/// P227 — text-returning fn-ref stored in a struct field, no capture.
/// Exercises the same dispatch path as P213's int variant but with
/// the work-buffer wired through.
#[test]
fn p227_text_fn_ref_struct_field() {
    code!(
        "struct G { fmt: fn(integer) -> text }
fn run() -> text {
    g = G { fmt: fn(n: integer) -> text { \"v: {n}\" } };
    g.fmt(42)
}"
    )
    .expr("run()")
    .result(Value::Text("v: 42".to_string()));
}

/// P227 — text-returning fn-ref stored in a struct field WITH
/// capturing closure (the original P227 reproducer).  The lambda
/// captures `label` and reads it from the closure record while
/// formatting into the caller's work-buffer.
#[test]
fn p227_text_fn_ref_struct_field_capture() {
    code!(
        "struct G { fmt: fn(integer) -> text }
fn run() -> text {
    label = \"z\";
    g = G { fmt: fn(n: integer) -> text { \"{label}: {n}\" } };
    g.fmt(42)
}"
    )
    .expr("run()")
    .result(Value::Text("z: 42".to_string()));
}

/// P214 — closed 2026-05-05.  `vector<fn(integer) -> integer>` of
/// non-capturing closures panicked under interp (`fn_call_ref:
/// d_nr=12884901896 out of range`) and rejected on native with E0605
/// `DbRef as (u32, DbRef)`.  Two coordinated changes:
///
/// 1. **Parser** (`src/parser/fields.rs`): the vector-element-size
///    computation in `parse_index_apply` falls back to
///    `narrow_vector_content` for fn-ref types so `elm_size` is 4
///    (the d_nr stride) instead of 0 (which made every index hit
///    slot 0).  Adds a `Type::Function` branch that reads the d_nr
///    via `OpGetInt4` and pairs it with `OpNullRefSentinel` for the
///    closure DbRef half — assembling the (u32, DbRef) tuple shape
///    via the existing `fn_ref_field_read` block-name shortcut in
///    native codegen.
/// 2. **Native init** (`src/generation/mod.rs::emit_field`): when
///    the field's vector content is `Type::Function`, emit
///    `db.vector(narrow_int)` instead of `db.vector(u16::MAX)` so
///    the runtime parent-tracking pass in `Stores::field` finds
///    the proper int content type.
#[test]
fn p214_vector_of_noncapturing_closures() {
    code!(
        "fn run() -> integer {
    v: vector<fn(integer) -> integer> = [
        fn(x: integer) -> integer { x + 1 },
        fn(x: integer) -> integer { x * 2 },
    ];
    v[0](10) + v[1](5)
}"
    )
    .expr("run()")
    .result(Value::Int(21)); // 11 + 10
}

/// P215 — closed 2026-05-05.  A closure-typed local variable defined
/// in an outer scope was unreachable from inside an inner closure
/// body that called it (`Unknown function 'inner'`).  Two coordinated
/// changes:
///
/// 1. **Parser** (`src/parser/control.rs::try_fn_ref_call`): when
///    `name` is in `capture_context` with a `Type::Function` type and
///    not in the current function's vars, mirror the standard
///    capture mechanism — push to `captured_names`, create a
///    placeholder local var, and detect the capture at emit time via
///    `capture_context` (stable across both passes).
///
/// 2. **Closure-record write** (`src/parser/mod.rs::emit_fn_ref_field_write`):
///    lift the P213-deferred "only inline lambda literals" diagnostic
///    when both target and source are non-capturing — target field
///    has 4B int layout (`assigned_lambda_d_nr == u32::MAX`) and
///    source var is not in `closure_vars` (the existing
///    capturing-fn-ref tracker).  Emit `OpSetInt4(target, pos,
///    Value::FnRefDnr(src))` to project the d_nr.
///
/// 3. **Closure-record read symmetry** (`src/parser/mod.rs::get_field`):
///    for `Type::Function` fields with 4B int layout, synthesise a
///    null DbRef for the closure half via `OpNullRefSentinel`
///    instead of reading at `pos+4` (which would corrupt the next
///    attribute's bytes — the legacy 4B layout has no
///    `__closure_rec` half).
///
/// New IR variant `Value::FnRefDnr(u16)` projects the d_nr from a
/// fn-ref Var on both backends — interp via `OpVarInt(slot_pos)`
/// (the dispatcher reads 8 bytes regardless of declared type),
/// native via `(var_<name>.0 as i64)` tuple projection.
///
/// Capturing source lambdas (where `inner` itself captures from
/// further out) remain deferred — the closure-record's 4B layout
/// can't hold the source's closure DbRef, and lifting that requires
/// extending `synthesize_closure_record` to register the 8B split
/// layout for fn-ref captures.
#[test]
fn p215_nested_closure_call() {
    code!(
        "fn run() -> integer {
    inner = fn(x: integer) -> integer { x + 5 };
    outer = fn(y: integer) -> integer { inner(y) + 1 };
    outer(10)
}"
    )
    .expr("run()")
    // inner(10) = 15; outer(10) = 16
    .result(Value::Int(16));
}

/// P215 — multiple non-capturing fn-refs captured into a single
/// closure body.  Validates that several captures coexist in the
/// closure record, each correctly populated and dispatched.  Each
/// captured lambda is non-capturing — the case the P215 fix
/// supports.  Capturing-source-into-closure remains deferred
/// (requires `synthesize_closure_record` to register the 8B split
/// layout when the source itself captures).
#[test]
fn p215_multiple_captures_in_one_closure() {
    code!(
        "fn run() -> integer {
    add_one = fn(x: integer) -> integer { x + 1 };
    times_two = fn(x: integer) -> integer { x * 2 };
    minus_three = fn(x: integer) -> integer { x - 3 };
    pipeline = fn(n: integer) -> integer {
        minus_three(times_two(add_one(n)))
    };
    pipeline(5)
}"
    )
    .expr("run()")
    // 5+1=6 → 6*2=12 → 12-3=9
    .result(Value::Int(9));
}

/// P222 — closed 2026-05-06.  `s = s + s` rejected on native with
/// E0502 ("cannot borrow `*var_s` as mutable because it is also
/// borrowed as immutable").  After the P217 self-append strip the
/// IR became `OpAppendText(s, Var(s))`, which the native emitter
/// lowered to `var_s += &*(&var_s);` — `&mut` and `&` on the same
/// place.  Fix in `src/generation/text.rs::append_text` detects
/// when the RHS expression references the destination variable and
/// hoists the value through a fresh `String` so the self-borrow
/// never overlaps the `+=` target.  Interp already produced the
/// correct `"abab"` after the P217 fix; this test pins both
/// backends so a future codegen refactor cannot reintroduce the
/// self-borrow.
#[test]
fn p222_text_self_double() {
    code!(
        "fn run() -> text {
    s = \"ab\";
    s = s + s;
    s
}"
    )
    .expr("run()")
    .result(Value::Text("abab".to_string()));
}

/// P222 follow-up — triple self-reference `v = v + v + v` exercises
/// the codegen path twice (two `OpAppendText(v, Var(v))` ops after
/// the P217 strip).  Each must hoist independently; the second
/// append reads `v`'s already-doubled value, producing 8 chars
/// (`"ab"` → `"abab"` → `"abababab"`).  Both backends must agree.
#[test]
fn p222_text_triple_self_reference() {
    code!(
        "fn run() -> text {
    v = \"ab\";
    v = v + v + v;
    v
}"
    )
    .expr("run()")
    .result(Value::Text("abababab".to_string()));
}

/// P228 — closed 2026-05-06.  `label = t.0;` (where `t` is a tuple
/// with a text-typed first element, e.g. `t = ("hello", 42)`)
/// rejected on native with E0308 because the emitted Rust was
/// `let mut var_label: String = &var_t.0.to_string();` —
/// `&var_t.0.to_string()` parses as `&(var_t.0.to_string())` per
/// Rust method-call precedence, producing `&String` against a
/// declared `String`.  The `tuple_text_elem_clone` detection in
/// `src/generation/dispatch.rs::output_set` (added by T1.8a)
/// handled the same shape but pattern-matched `Value::TupleGet`
/// directly, missing the `Value::Span(TupleGet)` wrapper the
/// parser puts around every assignment RHS — so the `.clone()`
/// fast-path never fired and codegen fell through to the buggy
/// `&...to_string()` form.  Fix unspans `to` before pattern-matching;
/// also extends the symmetric `text_local_clone` Var detection to
/// unspan for the same reason.  Interp was unaffected (no `&` /
/// `.to_string()` precedence concern).
#[test]
fn p228_text_tuple_element_assignment() {
    code!(
        "fn run() -> text {
    t = (\"hello\", 42);
    label = t.0;
    label
}"
    )
    .expr("run()")
    .result(Value::Text("hello".to_string()));
}

/// P228 follow-up — text element at index > 0 (mixed-type tuple
/// with a text element in the middle).  Same Span-wrapped TupleGet
/// shape; pins that the fix is index-agnostic, not specific to `.0`.
#[test]
fn p228_text_tuple_element_at_higher_index() {
    code!(
        "fn run() -> text {
    t = (42, \"world\", 99);
    s = t.1;
    s
}"
    )
    .expr("run()")
    .result(Value::Text("world".to_string()));
}

/// P226 — closed 2026-05-06.  Vector literals (`[1,2,3]`) inside a
/// state-machine generator (Simple-yield-only — no for-loop body so
/// the eager-collect path doesn't engage) allocated a `__vdb_*`
/// `DbRef` slot that the per-state codegen declared inside one
/// match arm via `let mut var___vdb_N: DbRef = stores.null_named(...)`,
/// scoping the binding to that arm only.  A subsequent state arm
/// referencing the same `__vdb_N` (e.g. two `Simple` yields each
/// containing a vector literal) failed to compile with E0425
/// ("cannot find value `var___vdb_N` in this scope").  Same scoping
/// family as P218 (`__work_*` text format buffers) and P224 (general
/// user locals).  Fix in `src/generation/coroutine.rs::emit_next_i64`
/// pre-declares any non-argument `__vdb_*` local at function scope
/// (mirroring P218's text pre-declaration), then adds the var to
/// `self.declared` so the IR's per-state Set ops emit as plain
/// assignments rather than re-declarations.  Interp was unaffected —
/// the bytecode VM's variable scope is per-function, not per-arm.
#[test]
fn p226_vector_literal_in_yield_across_simple_arms() {
    code!(
        "fn nums() -> iterator<integer> {
    yield [1, 2, 3].len();
    yield [10, 20, 30, 40].len();
}
fn run() -> integer {
    total = 0;
    for v in nums() { total = total + v; }
    total
}"
    )
    .expr("run()")
    .result(Value::Int(7)); // 3 + 4
}

/// P226 follow-up — vector literal bound to a local first, then
/// referenced.  Triggers the same `__vdb_*` slot allocation across
/// state arms; pins that the fix covers the indirect shape too.
#[test]
fn p226_vector_literal_via_local_across_simple_arms() {
    code!(
        "fn nums() -> iterator<integer> {
    a = [1, 2, 3];
    yield a.len();
    b = [10, 20, 30, 40];
    yield b.len();
}
fn run() -> integer {
    total = 0;
    for v in nums() { total = total + v; }
    total
}"
    )
    .expr("run()")
    .result(Value::Int(7));
}

/// P232 — closed 2026-05-06.  `env_variable(name)` returned a `Str`
/// whose pointer dangled into a dropped local `OsString` (or, on the
/// WASM build, a dropped `String`).  Calling code observed garbage
/// bytes — `env_variable("MYVAR")` reading `"hello"` came back as
/// random non-UTF8 sequences, `as integer` then produced `null`.
/// The bug had been latent because the in-tree test
/// (`tests/scripts/19-files.loft:99`) only checked the unset case
/// (where the empty-string return path bypassed the dangling
/// branch).  Surfaced 2026-05-06 while wiring `LOFT_TICTACTOE_PORT`
/// for P231.  Fix in `src/database/format.rs::Stores::os_variable`
/// changes the signature from a static `fn(name: &str) -> Str` to
/// `&mut self`, pushes the resolved value into `self.scratch`, and
/// returns a `Str` borrowing from the persistent buffer (mirrors the
/// P205 pattern for text-returning natives).  `n_env_variable` and
/// the `#rust"stores.os_variable(@name)"` template both follow.
/// Validates by setting the env var in-process and round-tripping
/// through the loft runtime.
#[test]
fn p232_env_variable_round_trips_set_value() {
    // Use a process-unique var name to avoid clashes with concurrent
    // test threads.  SAFETY: `set_var` is unsafe in Rust 2024 because
    // racing readers in other threads may observe a torn value; this
    // test sets, reads, and removes synchronously and never relies on
    // a parallel reader, so the race window is empty within this test.
    let var = format!("LOFT_P232_PROBE_{}", std::process::id());
    // SAFETY: see comment above.
    unsafe {
        std::env::set_var(&var, "round-trip");
    }
    let src = format!(
        "fn run() -> text {{
    env_variable(\"{var}\")
}}"
    );
    code!(&src)
        .expr("run()")
        .result(Value::Text("round-trip".to_string()));
    // SAFETY: see comment above.
    unsafe {
        std::env::remove_var(&var);
    }
}

/// P230 — closed 2026-05-06.  `yield` inside an `if` block within a
/// generator emitted a raw Rust `yield` keyword on native (E0627
/// "yield expression outside of coroutine literal"), instead of
/// translating into a state-machine return.  Interp worked because
/// the bytecode VM handles every `OpCoroutineYield` generically.
/// Root cause: `src/generation/coroutine.rs::collect_segments` only
/// matched yields at the TOP LEVEL of a generator body's operator
/// list (Simple / YieldFrom / ForLoopBody) plus `Block` and `Loop`
/// containing yields — `Value::If` was missed.  An `if`-with-yield
/// fell through to the `pre` accumulator, then `output_code_inner`
/// hit `Value::Yield` and emitted literal `yield ...` Rust syntax.
/// Fix extends the ForLoopBody matcher to `Value::Block(_) |
/// Value::Loop(_) | Value::If(_, _, _)` so the eager-collect
/// factory's `yield_collect = true` mode emits `__values.push(...)`
/// instead of `yield ...` (mirroring how Block-with-yield was
/// already handled).  `contains_yield` already walked through
/// `Value::If` so detection works without further changes.
#[test]
fn p230_yield_in_if_block() {
    code!(
        "fn cond() -> iterator<integer> {
    n = 5;
    if n > 0 { yield n; }
    yield 99;
}
fn run() -> integer {
    total = 0;
    for v in cond() { total = total + v; }
    total
}"
    )
    .expr("run()")
    .result(Value::Int(104)); // 5 + 99
}

/// P230 follow-up — yield in else branch.  Same scoping family;
/// pins that the fix covers both arms of an if/else, not just the
/// then-arm.
#[test]
fn p230_yield_in_else_branch() {
    code!(
        "fn cond() -> iterator<integer> {
    n = -5;
    if n > 0 { yield 1; } else { yield n; }
    yield 99;
}
fn run() -> integer {
    total = 0;
    for v in cond() { total = total + v; }
    total
}"
    )
    .expr("run()")
    .result(Value::Int(94)); // -5 + 99
}

/// P230 follow-up — yield in both arms of if/else, multiple yields
/// per arm, mixed with a top-level yield after.  Stresses the
/// eager-collect path's ability to interleave conditional and
/// unconditional yields in the same generator.
#[test]
fn p230_yield_in_both_branches_with_trailing_simple() {
    code!(
        "fn cond() -> iterator<integer> {
    n = -5;
    if n > 0 { yield 1; yield 2; }
    else { yield n; yield n - 1; }
    yield 99;
}
fn run() -> integer {
    sum = 0;
    for v in cond() { sum = sum + v; }
    sum
}"
    )
    .expr("run()")
    // -5 + -6 + 99 = 88
    .result(Value::Int(88));
}
