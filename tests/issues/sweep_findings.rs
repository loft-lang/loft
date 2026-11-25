// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Regression guards for undefined behaviour found by Miri, the stability-sweep findings, and
//! arity growth of a forward caller.

use super::*;

// ── @PLAN53 clusters 3-5 — Miri-found UB regression guards ──────────────────
//
// These three tests guard the soundness fixes landed in commit batch on
// 2026-05-31 (PR #236).  Each is pure interpreter (no FFI, no threads) so
// it can run under `cargo +nightly miri test --test issues -- --exact <name>`.
//
// All three use `p213_struct_field_basic_int`'s idiom (same `code!` macro,
// same `cached_default()` harness) so Miri's per-process stdlib-parse cost is
// shared.
//
// Cluster overview:
//   3 — aliasing reborrow in cross-store copy (Stacked-Borrows UB)
//   4 — reading uninitialised padding bytes in the fn-ref slot (uninit UB)
//   5 — String buffer leak via free_text without clear() (leak UB)

/// @PLAN53 cluster 3 — cross-store copy aliasing.
///
/// Pre-fix code (commit c7755122): `vector_add` formed `&[Store]` (from_ref)
/// AND `&mut [Store]` (from_mut) over the same `self.allocations` slice
/// simultaneously — overlapping whole-slice borrows, UB under Miri's Stacked
/// Borrows even though the two indices are distinct.
///
/// Fix: `copy_block_cross_store` helper uses `get_disjoint_mut([i,j])` to get
/// two non-aliasing `&mut Store` from disjoint sub-ranges.
///
/// Trigger: two instances of the same struct type each live in their own store.
/// Appending one's vector field to the other's (`c1.data += c2.data`) hits
/// `Stores::vector_add` with `v_r.store_nr != v_other.store_nr`.
///
/// Miri gate: `cargo +nightly miri test --test issues -- --exact
/// plan53_cluster3_cross_store_vector_add`
#[test]
fn plan53_cluster3_cross_store_vector_add() {
    code!(
        "struct Bag { data: vector<integer> }
fn run() -> integer {
    c1 = Bag { data: [1, 2, 3] };
    c2 = Bag { data: [40, 50] };
    c1.data += c2.data;
    len(c1.data)
}"
    )
    .expr("run()")
    .result(Value::Int(5));
}

/// @PLAN53 cluster 4 — uninitialised padding in fn-ref slot read.
///
/// Pre-fix code (commit 37109ccf): OpVarFnRef / OpPutFnRef read/wrote the
/// 20-byte fn-ref slot as `[u8; 20]`.  DbRef's 2-byte tail padding (slot
/// bytes 18..20) is never initialised by a typed DbRef store, so reading the
/// slot as an integer array requires bytes 18..20 to be initialised — Miri
/// hard-UB: "encountered uninitialized memory" at byte [18].
///
/// Fix: use `[std::mem::MaybeUninit<u8>; 20]` — copies the bytes without
/// requiring them initialised.  Byte-identical on all concrete hardware.
///
/// Trigger: `f = double` emits OpPutFnRef (write the 20-byte slot); passing
/// `f` to `apply` emits OpVarFnRef (read the slot).  Both ops touch the
/// uninit-padded DbRef tail.
///
/// Miri gate: `cargo +nightly miri test --test issues -- --exact
/// plan53_cluster4_fn_ref_slot_uninit_padding`
#[test]
fn plan53_cluster4_fn_ref_slot_uninit_padding() {
    code!(
        "fn double(x: integer) -> integer { x * 2 }
fn apply(f: fn(integer) -> integer, v: integer) -> integer { f(v) }
fn run() -> integer {
    f = double;
    apply(f, 7)
}"
    )
    .expr("run()")
    .result(Value::Int(14));
}

/// @PLAN53 cluster 5 — `free_text` String buffer leak.
///
/// Pre-fix code (commit e0e094f7): `free_text` called `shrink_to(0)` WITHOUT
/// a preceding `clear()`.  `shrink_to(0)` only shrinks capacity down to
/// `max(capacity, len)` — on a non-empty String, `len > 0` so the buffer
/// survives and leaks (the store holds String as raw bytes and never runs
/// Drop; `free_text` is the sole deallocation point).  Miri's leak checker
/// caught it during p213 teardown.
///
/// Fix: call `clear()` unconditionally before `shrink_to(0)` — `len=0` lets
/// `shrink_to(0)` drop capacity to 0 and free the buffer.
///
/// Trigger: assign a non-empty text variable, then reassign it.  Each
/// reassignment calls `free_text` on the old String.  On the pre-fix code
/// Miri's `-Zmiri-leak-check` flags the retained allocation; on the fixed
/// code the buffer is freed.
///
/// Miri gate: `cargo +nightly miri test --test issues -- --exact
/// plan53_cluster5_free_text_buffer_dealloc`
#[test]
fn plan53_cluster5_free_text_buffer_dealloc() {
    code!(
        "fn run() -> text {
    msg = \"hello\";
    msg = \"world\";
    msg
}"
    )
    .expr("run()")
    .result(Value::Text("world".to_string()));
}

// ── Issue 313 ────────────────────────────────────────────────────────────────
// Capturing closure in a struct field crashed when invoked from a function
// DEFINED BEFORE the constructing function (interp SIGSEGV in opcode
// dispatch; native OOB at u16::MAX).
//
// Root cause: pass 1 never recorded the capturing assignment on
// `Attribute::assigned_lambda_d_nr` (lambdas emit as a bare `Int(d_nr)` in
// pass 1), so `fill_database` always laid out the legacy 4B field — the
// "working" same-fn shape wrote the closure record past the declared layout
// into allocation slack.  In pass 2 the flag re-derived in body-parse order,
// so a reader parsed before the assigning body emitted the legacy
// null-sentinel closure read against the split write.
//
// Fix: pass 1 detects capture via the lambda def's `closure_record` (set by
// `synthesize_closure_record` in both passes) so the layout truly splits;
// pass-2 read/write shapes consult the database layout (the one order-stable
// home of the fact) via `Parser::fn_ref_field_is_split`; native codegen
// mirrors the registered layout (`<attr>__closure_rec` ChildRec field).
// Cross-backend cells: tests/closure_matrix.rs
// `c1_d3_int_capture_field_cross_fn_invoker_first` and
// `c2_d3_text_capture_field_cross_fn_invoker_first`.

#[test]
fn issue_313_closure_field_invoked_cross_fn_invoker_first() {
    code!(
        "struct Counter { n: integer }
struct K { cb: fn(text) }
fn fire(k: K, p: text) { c = k.cb; c(p); }
fn fire_direct(k: K, p: text) { k.cb(p); }
fn run() -> integer {
    w = Counter { n: 0 };
    k = K { cb: fn(p: text) { w.n = w.n + 1; } };
    fire(k, \"a\");
    fire_direct(k, \"b\");
    w.n
}"
    )
    .expr("run()")
    .result(Value::Int(2));
}

// ── Issue 314 ────────────────────────────────────────────────────────────────
// A bare scalar captured by two closures — one reading, one writing — crashed
// at runtime ("Write to read-only store … CONST_STORE") when the reader lambda
// was parsed before the writer.  The shape only worked through shared heap
// cells with no defined owner ("first death wins"), so it is REJECTED at
// compile time instead (GOALS.md § "Stability trumps features";
// DESIGN_DECISIONS.md).  Shared mutable state belongs in a struct, which both
// closures may capture.

/// @C74 — a mutated scalar may be captured by only one closure.
#[test]
fn issue_314_scalar_shared_by_two_closures_rejected() {
    code!(
        "fn run2(a: fn(), b: fn()) { b(); a(); }
fn test_it() {
    t = 0;
    run2(fn() { u = t + 1; print(\"u={u}\"); }, fn() { t = t + 1; });
}"
    )
    .error(
        "variable `t` is mutated through a closure and captured by 2 closures; \
         sharing a mutable variable between closures is not supported — hold the \
         shared state in a struct field instead (e.g. `state = State { t: ... }` \
         captured by all closures) \
         at issue_314_scalar_shared_by_two_closures_rejected:4:53",
    );
}

// The sound single-closure accumulator (one record, one owner) stays supported.
#[test]
fn issue_314_single_closure_accumulator_still_works() {
    code!(
        "fn run(a: fn()) { a(); a(); }
fn run_it() -> integer {
    t = 0;
    run(fn() { t = t + 1; });
    t
}"
    )
    .expr("run_it()")
    .result(Value::Int(2));
}

// ── Issue 318 ────────────────────────────────────────────────────────────────
// A capturing closure escaping the function that owns its captures kept raw
// DbRefs into that frame's stores — freed at return, reused by later
// allocations, silently corrupting unrelated objects (no crash, UAF detector
// blind).  Three escape sinks are now rejected at compile time
// (GOALS.md § "Stability trumps features"): returning a closure-carrying
// struct (R1), writing a capturing closure into a struct received as an
// argument (R2), and collections of closure-carrying structs (R3).  Bare
// closure returns (the case-C factory) stay supported.  Probes:
// /tmp/p_followups/e*.loft; predicate: `Parser::type_carries_closure`.

/// @C75 — a closure-carrying struct keeps its closure record in a hidden field; a write to
/// another field by name still finds that field's attribute.
#[test]
fn a_field_after_a_capturing_fn_field_is_written_by_name() {
    // A struct with a capturing fn field lays out a hidden `<f>__closure_rec` field with no
    // attribute, so a layout field's index is not its attribute index: the write's const check
    // resolves the attribute by name.  By index it read past the attribute list (an ICE).
    code!(
        "struct V { a: integer }
struct H1 { f: fn(integer) -> integer, tag: integer }
fn run() -> integer {
    v = V { a: 3 };
    a = H1 { f: fn(x: integer) -> integer { v.a * 10 + x }, tag: 1 };
    a.tag = 9;
    a.tag * 100 + a.f(1)
}"
    )
    .expr("run()")
    .result(Value::Int(931));
}

#[test]
fn issue_1867_a_returned_struct_keeps_its_closure_capture() {
    // `@FR-L-Escape`, `@FR-L-CapOwn` — the struct's closure record is built in its own store
    // and adopts `w`, so the capture outlives `make`'s frame: 8 and 9 through a fn-ref read
    // out of the field, then 10 through the field itself.
    code!(
        "struct Counter { n: integer }
struct K { cb: fn() -> integer }
fn make() -> K {
    w = Counter { n: 7 };
    K { cb: fn() -> integer { w.n = w.n + 1; w.n } }
}
fn run() -> integer { k = make(); c = k.cb; c(); c() * 100 + k.cb() }"
    )
    .expr("run()")
    .result(Value::Int(910));
}

#[test]
fn issue_1867_a_closure_written_into_an_argument_struct_keeps_its_capture() {
    // `@FR-L-Escape` — the closure record is built in the caller's struct, adopts `w`, and the
    // second `attach` releases the record it displaces: 8, then 21 and 22 from the new one.
    code!(
        "struct Counter { n: integer }
struct K { cb: fn() -> integer }
struct H { k: K }
fn orig() -> integer { 0 }
fn attach(h: H, n: integer) {
    w = Counter { n: n };
    h.k = K { cb: fn() -> integer { w.n = w.n + 1; w.n } };
}
fn run() -> integer {
    h = H { k: K { cb: orig } };
    attach(h, 7);
    a = (h.k.cb)();
    attach(h, 20);
    b = (h.k.cb)();
    a * 10000 + b * 100 + (h.k.cb)()
}"
    )
    .expr("run()")
    .result(Value::Int(82122));
}

#[test]
fn issue_318_vector_of_closure_carrying_struct_rejected() {
    code!(
        "struct Counter { n: integer }
struct K { cb: fn() }
fn test_it() {
    w = Counter { n: 7 };
    v: vector<K> = [K { cb: fn() { w.n = w.n + 1; } }];
}"
    )
    .warning("Variable v is never read at issue_318_vector_of_closure_carrying_struct_rejected:5:7")
    .error(
        "collection of a struct type that holds a capturing closure is not \
         supported — element copies would dangle into the constructing \
         function's frame; keep closure holders in local variables and pass \
         them down as arguments \
         at issue_318_vector_of_closure_carrying_struct_rejected:5:19",
    );
}

// The supported shapes stay supported: closure-carrying struct in a local,
// passed DOWN as an argument (#313's matrix), and the bare factory return.
#[test]
fn issue_318_local_closure_struct_passed_down_still_works() {
    code!(
        "struct Counter { n: integer }
struct K { cb: fn(text) }
fn fire(k: K, p: text) { c = k.cb; c(p); }
fn run() -> integer {
    w = Counter { n: 0 };
    k = K { cb: fn(p: text) { w.n = w.n + 1; } };
    fire(k, \"a\");
    w.n
}"
    )
    .expr("run()")
    .result(Value::Int(1));
}

// ── Issue 323 ────────────────────────────────────────────────────────────────
// A factory-returned bare closure capturing a local struct freed the capture
// at the factory's return (the return-position OpFreeRef): the escaped
// closure's record kept a DbRef into the dead frame and corrupted whatever
// reused the slot.  Both backends were affected — interp only LOOKED sound
// because its allocation order happened not to reuse the slot in small
// programs.  Fix: `get_free_vars` (scopes.rs) widens the Plan-57 captured-cell
// suppression to every Reference-typed capture; the closure record's cascade
// is the single owner of the captured store.

#[test]
fn issue_323_factory_closure_reference_capture_survives_reuse() {
    code!(
        "struct Counter { n: integer }
fn make() -> fn() -> integer {
    w = Counter { n: 7 };
    fn() -> integer { w.n = w.n + 1; w.n }
}
fn run() -> integer {
    f = make();
    spam1 = Counter { n: 31337 };
    spam2 = Counter { n: 41414 };
    a = f();
    b = f();
    a * 1000000 + b * 100 + (spam1.n - 31337) + (spam2.n - 41414)
}"
    )
    .expr("run()")
    .result(Value::Int(8_000_900));
}

// Within-frame closures keep working: the record's frame-exit free cascades
// the captured store, so nothing leaks and the capture stays live for the
// whole frame.
#[test]
fn issue_323_in_frame_reference_capture_still_works() {
    code!(
        "struct Counter { n: integer }
fn run() -> integer {
    w = Counter { n: 7 };
    f = fn() { w.n = w.n + 1; };
    f();
    f();
    w.n
}"
    )
    .expr("run()")
    .result(Value::Int(9));
}

// ── Issue 328 ────────────────────────────────────────────────────────────────
// `reference<T>` struct fields were never laid out: the parse erased the
// pointer-ness to the same Type as inline nesting, so `fill_database` either
// embedded T's bytes inline (writes silently deep-copied — violating the
// documented pointer semantics) or, for `reference<Self>`, rejected the
// struct with the self-contradictory "use reference<Node>" cycle error;
// `next: null` construction panicked on the unpositioned-field marker.
//
// Fix: in struct-field position `reference<T>` parses to the auto-Reference
// share marker (`Type::Reference(d, [u16::MAX])`), riding the proven
// 12-byte Parts::DbRef layout + OpGetDbRef/OpSetDbRef paths; the value-cycle
// checker skips marker fields (making `reference<Self>` legal); pointer
// field assignment repoints via OpSetDbRef (`= null` writes the sentinel);
// self-deps from `x = x.next` are stripped at the type merge (a var cannot
// borrow from itself — the dep flipped codegen into the dependent-view
// class and corrupted the frame).

#[test]
fn issue_328_reference_field_pointer_semantics() {
    code!(
        "struct Leaf { value: integer }
struct Node { value: integer, next: reference<Leaf> }
fn run() -> integer {
    a = Leaf { value: 1 };
    b = Leaf { value: 2 };
    n = Node { value: 0, next: a };
    a.value = 41;
    aliased = n.next.value;
    n.next = b;
    repointed = n.next.value;
    untouched = a.value;
    n.next = null;
    cleared = if n.next == null { 1 } else { 0 };
    d = Node { value: 9, next: null };
    default_null = if d.next == null { 1 } else { 0 };
    aliased * 100000 + repointed * 1000 + untouched * 10 + cleared * 2 + default_null
}"
    )
    .expr("run()")
    // `@FR-D-NoRef` — a bare pointer has no default, so the null is written out (loft#1931);
    // writing it is `(N-Store)`'s warn tier, the cure being `reference<Leaf>?`.
    .warning(
        "`null` is stored into the field of the non-null type `reference<Leaf>` — the slot \
         holds null; declare it `reference<Leaf>?` to make that explicit at \
         issue_328_reference_field_pointer_semantics:14:32",
    )
    .result(Value::Int(4_102_413));
}

#[test]
fn issue_328_reference_self_recursive_walk() {
    // The terminator is a `null`, so the field is declared `reference<Node>?` and the walker
    // `Node?` — the shapes the language gained in loft#1316.  Both were unwritable before
    // (`reference<Node>?` failed layout), which is why this walk used to carry an undeclared
    // null in a non-null slot.  The subject is unchanged: the recursive walk over a
    // self-referencing pointer field, which is #328's codegen path.
    code!(
        "struct Node { value: integer, next: reference<Node>? }
fn run() -> integer {
    c = Node { value: 4, next: null };
    b = Node { value: 2, next: c };
    a = Node { value: 1, next: b };
    m = a.next;
    m.value = 20;
    cur: Node? = a;
    total = 0;
    while cur != null {
        total = total + cur.value;
        cur = cur.next;
    }
    total
}"
    )
    .expr("run()")
    .result(Value::Int(25));
}

// `x = x.next` (same-var self-read reassign): the stripped self-dep keeps
// the var on the plain ref-slot codegen path (pre-fix: InitCreateStack frame
// corruption → SIGSEGV reading the result).
#[test]
fn issue_328_self_reassign_through_reference_field() {
    code!(
        "struct Node { value: integer, next: reference<Node>? }
fn run() -> integer {
    b = Node { value: 2, next: null };
    x: Node? = Node { value: 1, next: b };
    x = x.next;
    x.value ?? 0
}"
    )
    .expr("run()")
    .result(Value::Int(2));
}

// ── Issue 330 (FIXED 2026-06-11) ─────────────────────────────────────────────
// Self-reading reassignment, three repairs sharing one invariant ("the old
// store outlives every RHS read"):
// 1. parser: a reassignment-construction whose fields READ the target routes
//    to the fresh-work-ref path instead of the in-place OpDatabase re-init
//    (objects.rs construction_mentions lookahead);
// 2. parser: `x = x` is the identity — emitted as nothing (both backends);
// 3. codegen: the pre-Set free now uses the recursive scopes::value_reads_var
//    predicate (not the top-level-arg S1 scan); a self-reading RHS stashes
//    the old DbRef and frees it post-assignment via OpFreeRefIfDistinct.

#[test]
fn issue_330_degenerate_self_assignment() {
    code!(
        "struct S { v: integer }
fn run() -> integer {
    x = S { v: 7 };
    x = x;
    x.v
}"
    )
    .expr("run()")
    .result(Value::Int(7));
}

#[test]
fn issue_330_self_reading_literal_reassignment() {
    code!(
        "struct S { v: integer }
fn run() -> integer {
    x = S { v: 7 };
    x = S { v: x.v + 1 };
    x.v
}"
    )
    .expr("run()")
    .result(Value::Int(8));
}

// ── Issue 332 (stability-sweep F6; documented, NOT fixed) ────────────────────
// A nullable narrow-integer field across its whole life: OMITTED, given a value,
// and written back to null.  Every width answers `null` where it is absent and the
// value where it is present, and the narrow encodings (i16's raw-0 null, i32's
// i32::MIN) are invisible from loft.
//
// The omitted half is `formal/types.md` (D-Opt): a nullable's default IS null.  This
// test previously asserted the opposite — that an omitted `i16?` reads `0` — on the
// strength of two citations that do not hold up: "LOFT.md § constructors", a section
// that does not exist quoting a sentence that is not in the file, and
// `06-structs.loft`, which declares no nullable field at all.  Nothing else locked it.
#[test]
fn issue_332_nullable_narrow_field_null_roundtrip() {
    code!(
        "struct N { a: i16?, c: i32?, d: integer?, tail: integer }
fn run() -> integer {
    n = N { tail: 1 };
    om = 0;
    if n.a == null { om += 100; }
    if n.c == null { om += 10; }
    if n.d == null { om += 1; }
    n.a = 5; n.c = 5; n.d = 5;
    st = 0;
    if n.a == 5 { st += 100; }
    if n.c == 5 { st += 10; }
    if n.d == 5 { st += 1; }
    n.a = null; n.c = null; n.d = null;
    re = 0;
    if n.a == null { re += 100; }
    if n.c == null { re += 10; }
    if n.d == null { re += 1; }
    om * 1000000 + st * 1000 + re
}"
    )
    .expr("run()")
    .result(Value::Int(111_111_111));
}

// FIXED 2026-06-11 per the documented design (LOFT.md § integer widths):
// a nullable byte field reserves the 256th code as the null sentinel (255
// distinct values, 0..=254) via the OpGetByteNullable/OpSetByteNullable op
// pair the parser picks on attribute nullability; `not null` byte fields
// keep the full 256-value range through the raw pair.  byte_width now
// counts the sentinel code (limit-derived nullable ranges widen when they
// have no spare code), and the dead `||` range checks in Store::set_byte /
// set_short are `&&`.
#[test]
fn issue_334_nullable_byte_field_null_roundtrip() {
    code!(
        "struct N { b: u8?, tail: integer }
fn run() -> integer {
    n = N { tail: 1 };
    n.b = null;
    if n.b == null { 1 } else { 0 }
}"
    )
    .expr("run()")
    .result(Value::Int(1));
}

// ── Issue 333 (stability-sweep F4; documented, NOT fixed) ────────────────────
// Float division by zero must follow the documented null semantics (the
// divide-by-zero lint promises null; native yields null) — the interpreter
// currently aborts with a hard error instead.

// RESOLVED 2026-06-11: the issue's premise was inverted — per plan-07 4f.5
// the RAISE is the semantics (interp was right); native missed it because
// `raise_runtime` only recorded and nothing checked.  Fixed by
// `NATIVE_FAIL_FAST` (database/mod.rs) armed in the generated binary's main
// + the had_fatal exit backstop.  Cross-backend guard:
// tests/scripts/178-i333-div-zero-raises.loft (@EXPECT_FAIL on both suites).
#[test]
fn issue_333_undefended_div_zero_raises() {
    code!(
        "fn run() -> integer {
    z = 0;
    a = 5 % z ?? -1;
    a
}"
    )
    .expr("run()")
    .result(Value::Int(-1));
}

// #334 companion: `not null` byte fields keep the full 0..=255 range.
#[test]
fn issue_334_not_null_byte_keeps_full_range() {
    code!(
        "struct M { full: u8, tail: integer }
fn run() -> integer {
    m = M { full: 255, tail: 1 };
    m.full
}"
    )
    .expr("run()")
    .result(Value::Int(255));
}

/// @PLAN59 — par dispatch classified hidden attrs by NAME PREFIX
/// ('__'-named ⇒ text buffer): a worker whose tail calls a wider
/// heap-returning fn gets a wrapper-promoted hidden dest named
/// `__ref_1`, which landed in the text bucket — frame underflow
/// "No elements left on the stack 8 < 12" at runtime.  Classification
/// is now by TYPE.  (The native arm of this shape is a separate
/// pre-existing par gap — plain vector-returning workers don't compile
/// natively either; tracked in plans/59-return-abi.)
#[test]
fn plan59_par_worker_over_wrapper_promoted_callee() {
    let dir = std::env::temp_dir();
    let path = dir.join("plan59_landmine.loft");
    fa::write(
        &path,
        r#"
fn use_first() -> integer {
  v = wrapped(2);
  len(v)
}

pub fn wrapped(n: integer) -> vector<integer> {
  widened(n, 1)
}

pub fn widened(n: integer, extra: integer) -> vector<integer> {
  acc: vector<integer> = [];
  for i in 0..n + extra { acc += [i * 10]; }
  acc
}

fn worker(x: integer) -> vector<integer> {
  wrapped(x)
}

fn main() {
  assert(use_first() == 3, "plain: {use_first()}");
  inputs = [1, 2, 3];
  outs: vector<integer> = [];
  for x in inputs par(r = worker(x), 2) {
    outs += [len(r)];
  }
  assert(len(outs) == 3, "par count: {len(outs)}");
  print("ok");
}
"#,
    )
    .unwrap();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bin = std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"));
    if !fa::exists(&bin) {
        // Sanitizer CI jobs (ASan gate, stack_align sweep) run the test
        // binaries without building the release CLI — skip like the
        // engine_host_kernel spawning tests do instead of dying NotFound.
        eprintln!("skipping: release loft not built");
        return;
    }
    let out = loft::platform::process::harness_command(bin)
        .args(["--interpret", "--no-warnings"])
        .arg(&path)
        .current_dir(&root)
        .output()
        .expect("run loft");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success() && stdout.contains("ok"),
        "par over wrapper-promoted callee regressed: stdout={stdout:?} stderr={stderr:?}"
    );
}

// ── @PLAN59 pass-2 arity growth — forward CALLER of a multi-return-site fn ──
// Found 2026-06-12 by the restored armed-assert channel: `ref_return`'s
// pass-1 growth re-fires in PASS 2 under different work-ref names (pass 2
// sees the forward callee already parsed, so the per-site work refs differ
// from pass 1's), and a caller parsed earlier in pass 2 holds a short arg
// list — codegen halts with "Too few parameters on n_pick (got 2, need 4)"
// on BOTH backends.  The armed assert "@PLAN59: arity grew in PASS 2 on
// plain fn" (parser/control.rs ref_return) flags exactly this.  Repro:
// /tmp/p_followups/p14_forward_caller.loft.  The fix is pass-stable
// ref_return promotion for the forward-callee shape (F1 family).
#[test]
fn pass2_arity_growth_forward_caller() {
    code!(
        "struct S { v: integer }
fn use_pick(json: text) -> S { pick(json) }
fn pick(json: text) -> S {
  if json == \"\" { return mk(71006); }
  result = mk(71007);
  if json == \"bad\" { return mk(71008); }
  result
}
fn mk(n: integer) -> S { s = S { v: n }; s }
fn run() -> integer {
    use_pick(\"\").v + use_pick(\"x\").v + use_pick(\"bad\").v
}"
    )
    .expr("run()")
    .result(Value::Int(71006 + 71007 + 71008));
}

// The 3-LEVEL forward chain — same class, one level deeper: `outer`'s
// single tail call to `use_pick` materializes one work ref PER hidden
// buffer of the callee, so buffer need CASCADES through forward-call
// chains and no pass-1 syntactic site count can bound it (a per-site
// pre-provision fix was tried 2026-06-12 and reverted: it fixed the
// 2-level cell and moved the crash here).  The class fix is a
// between-pass buffer-count fixpoint (the typedef-resolution slot runs
// after all pass-1 signatures + bodies exist) — or caller-local backing
// for the non-chained work refs.  Repro:
// /tmp/p_followups/p15_three_level.loft.
#[test]
fn pass2_arity_growth_forward_chain() {
    code!(
        "struct S { v: integer }
fn outer(json: text) -> S { use_pick(json) }
fn use_pick(json: text) -> S { pick(json) }
fn pick(json: text) -> S {
  if json == \"\" { return mk(71006); }
  result = mk(71007);
  if json == \"bad\" { return mk(71008); }
  result
}
fn mk(n: integer) -> S { s = S { v: n }; s }
fn run() -> integer {
    outer(\"\").v + outer(\"x\").v
}"
    )
    .expr("run()")
    .result(Value::Int(71006 + 71007));
}

// SELF-RECURSION cell (design-round probe, 2026-06-12): a multi-site fn
// whose tail return calls ITSELF makes the per-site buffer recurrence
// self-referential — buffers(f) = 2 + buffers(f) has NO finite fixpoint,
// so each parse pass adds more hidden attrs and codegen halts ("Too few
// parameters on n_count_down (got 5, need 7)").  No forward reference
// involved: this cell FALSIFIES the between-pass fixpoint design (it
// cannot terminate here); only the one-buffer-per-fn family covers it.
// Repro: /tmp/p_followups/casc_recursive.loft.
#[test]
fn pass2_arity_growth_self_recursive() {
    code!(
        "struct S { v: integer }
fn mk(n: integer) -> S { s = S { v: n }; s }
fn count_down(n: integer) -> S {
  if n <= 0 { return mk(91000); }
  if n == 99 { return mk(91099); }
  count_down(n - 1)
}
fn run() -> integer { count_down(3).v }"
    )
    .expr("run()")
    .result(Value::Int(91000));
}

// MUTUAL-RECURSION cell (design-round probe, 2026-06-12): two multi-site
// fns tail-calling each other — a cycle always contains a forward edge,
// so this crashes like the forward chain ("Too few parameters on
// n_odd_pick (got 3, need 5)") and no parse ORDER can fix it.  Repro:
// /tmp/p_followups/casc_mutual.loft.
#[test]
fn pass2_arity_growth_mutual_recursion() {
    code!(
        "struct S { v: integer }
fn mk(n: integer) -> S { s = S { v: n }; s }
fn even_pick(n: integer) -> S {
  if n <= 0 { return mk(92000); }
  odd_pick(n - 1)
}
fn odd_pick(n: integer) -> S {
  if n <= 0 { return mk(92001); }
  even_pick(n - 1)
}
fn run() -> integer { even_pick(4).v }"
    )
    .expr("run()")
    .result(Value::Int(92000));
}

// #355 — the VECTOR arm of the one-buffer return binding: a forward
// caller of a multi-return-site vector fn silently returned the WRONG
// element (per-site buffer growth mis-wired the forward call).  Mid-body
// vector returns now chain/copy into the one buffer like Reference
// returns (RetSite::MidReturn never renames a site local — the 01b
// hazard); the wrapper leg guards the #120 dep-recovery mirror.
#[test]
fn one_buffer_vector_forward_caller() {
    code!(
        "fn use_pickv(json: text) -> vector<integer> { pickv(json) }
fn pickv(json: text) -> vector<integer> {
  if json == \"\" { return mkv(1); }
  result = mkv(2);
  if json == \"bad\" { return mkv(3); }
  result
}
fn mkv(n: integer) -> vector<integer> { v: vector<integer> = []; v += [n]; v }
fn run() -> integer {
    use_pickv(\"\")[0] + use_pickv(\"x\")[0] + use_pickv(\"bad\")[0]
}"
    )
    .expr("run()")
    .result(Value::Int(6));
}

// #356 — a mid-body `return f(g(x))` (argument lifting decomposes the
// bare call) returned the null sentinel on native: scopes' `returned_var`
// saw no Var in the lifted tail, so the epilogue emitted `Return(Null)`.
// The site's value now gets the canonical `{ buf = call(...); buf }`
// shape on every pass (including the pass-2 re-find of a pass-1-promoted
// work ref by name).
#[test]
fn mid_body_nested_call_return_value() {
    code!(
        "struct S { v: integer }
fn mk(n: integer) -> S { s = S { v: n }; s }
fn wrap(x: S) -> S { s = S { v: x.v + 7 }; s }
fn nested(json: text) -> S {
  if json == \"n\" { return wrap(mk(94000)); }
  mk(94100)
}
fn run() -> integer { nested(\"n\").v + nested(\"x\").v }"
    )
    .expr("run()")
    .result(Value::Int(94007 + 94100));
}
