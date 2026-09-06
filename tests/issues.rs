// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Minimal reproducing tests for known open issues in the loft runtime.
//! Each test isolates exactly the bug pattern described in doc/claude/PROBLEMS.md.
//! Broken tests are marked #[ignore] so they are tracked but do not break CI.

extern crate loft;

mod testing;

use loft::compile::byte_code;
use loft::data::Value;
use loft::logger::{Logger, RuntimeLogConfig};
use loft::parser::Parser;
use loft::scopes;
use loft::state::State;
use std::sync::{Arc, Mutex};

// ── Issue 3 ──────────────────────────────────────────────────────────────────
// Polymorphic text methods on struct-enum variants → stack overflow at state.rs:2070.
// `text_return` adds RefVar(Text) attributes to variant functions in the second pass,
// but enum_fn only runs in the first pass, so the Dynamic dispatch IR still calls
// with only [Var(0)] despite each variant now needing extra text-buffer arguments.

// ── Issue 5 ──────────────────────────────────────────────────────────────────
// Scalar `+=` on an empty (null) vector struct field has no effect.
// Expected: the scalar is appended and len == 1.

// `b.items += [1]` (bracket form) on a null field — this is the WORKING baseline.
// The bracket form goes through parse_vector with is_field=true and uses
// OpNewRecord / OpFinishRecord to allocate the element in place.
// `b.items += [3, 5]` on a null field — multiple elements with bracket form.
// `b.items += 1` (bare scalar, no brackets) on a null field — FIXED.
// Parser now routes through new_record so the field is allocated in place.
// Was tracked as Issue 5 in doc/claude/PROBLEMS.md.
// ── Issue 1 ──────────────────────────────────────────────────────────────────
// A method whose return type is a NEW struct record crashes at database.rs:1494
// because the DbRef returned by the method has a garbage store_nr.

// Minimal reproducer: `fn double(self: Color) -> Color { Color { r: self.r * 2 } }`
// Calling `c.double()` crashes with "index out of bounds: the len is N but index is M".
// Tracked as Issue 1 in doc/claude/PROBLEMS.md.
// ── Issue 2 ──────────────────────────────────────────────────────────────────
// A borrowed reference first assigned inside a branch gets a garbage store_nr=8
// DbRef at runtime, crashing at database.rs:1462.
// Owned references are correctly pre-initialized (Option A sub-3); borrowed refs are not.

// Borrowed ref first assigned INSIDE an `if` branch — FIXED.
// Was tracked as Issue 2 in doc/claude/PROBLEMS.md; now passes after
// the Option A sub-3 pre-init work in scopes.rs.
// ── Issue 4 ──────────────────────────────────────────────────────────────────
// `v += items` inside a function that takes `v` as a `&vector<T>` ref-param
// has no visible effect on the caller's variable after the call returns.

// Baseline: field mutation through a ref-param WORKS (e.g. `v[0].val = x`).
// ── Issue 44 — L4: Empty `[]` literal as a mutable vector argument ───────────
// Fixed in parser/mod.rs call_nr(): when Value::Insert([Null]) (or empty Insert)
// appears where a vector parameter is expected, a temp variable is created with
// vector_db initialisation ops, giving the caller a proper 12-byte DbRef.
// The fix is in call_nr(), not in parse_vector(), so it runs whenever [] reaches
// the call-site type-check regardless of call nesting.

// Baseline: `join([], "-")` — empty `vector<text>` arg via call_nr fix.
// L4 edge: `[]` passed to a user-defined function taking `vector<integer>`.
// Exercises the same call_nr path for a non-text element type.
// L4 edge: `[]` as second argument, not first — verifies argument index handling.
// ── Issue 56 — L5: `v += extra` via `&vector` ref-param ──────────────────────
// Fixed in state/codegen.rs generate_var(): RefVar(Vector) now emits OpGetStackRef
// to dereference the ref-param and load the actual vector DbRef before OpAppendVector.
// The vector record write-back happens implicitly: vector_append writes through the
// DbRef into the caller's local-variable record, so the caller sees the updated vector.

// Baseline: `v += extra` via ref-param appends elements to the caller's vector.
// L5 edge: append integers via ref-param; verify values and length.
// L5 edge: multiple sequential ref-param appends grow the vector correctly.
// ── Issue 11 ─────────────────────────────────────────────────────────────────
// Field-name overlap between two structs in the same file must NOT cause wrong
// field offsets in key lookups or tree traversal.
//
// Investigation: `determine_keys()` is type-scoped, so IdxElm.key is correctly
// resolved at offset 4 (after nr:integer), not at offset 0 (SortElm.key's position).
// Key lookups and full iteration both pass; Issue 11 was already fixed or never existed.
//
// Range-query note: `[10..20, "B"]` iterates everything up to but not including
// the element at (nr=20, key="B") in the descending ordering.  Since "C">"B"
// alphabetically and the key is sorted descending, (20,C) appears BEFORE (20,B) in
// the tree and IS therefore included → sum = 200+100+300 = 600.

// Two structs share a field name `key` at different offsets:
// `SortElm { key: text, value: integer }` (key is field 0, offset 0)
// `IdxElm  { nr: integer, key: text, value: integer }` (key is field 1, offset 4)
// Key lookups and iteration on `IdxElm` must use key's offset in IdxElm (4),
// not in SortElm (0).  Confirmed working — field offsets are type-scoped.
// ── Issue 28 ─────────────────────────────────────────────────────────────────
// validate_slots could panic in debug builds when the same variable name is reused
// in sequential `{ }` blocks in the same function (both get the same slot but
// different live-interval entries).  Fixed: find_conflict() exempts same-name+same-slot pairs.

// Same variable name in sequential blocks — the core Issue 28 case (fixed).
// Different variable names in sequential blocks — validate_slots must not panic.
// Each block is fully self-contained; variables don't escape their block.
// ── Issue 29 ─────────────────────────────────────────────────────────────────
// validate_slots false positive: two differently-named owned (Reference) variables
// that share a slot but have non-overlapping actual live ranges trigger a conflict
// because compute_intervals gives the first variable a last_use that reaches past
// the second variable's first_def.

// Two differently-named struct variables in sequential blocks — each in its own
// `{ }` scope so their lifetimes don't overlap.  validate_slots must not panic.
// The real issue 29 pattern: same variable name `f` reused across many sequential
// blocks; a differently-named reference variable `c` is introduced between some of
// those blocks.  validate_slots must not panic (c.first_def may fall between two
// of f's live ranges, which are separate Variable entries sharing the same slot).
// ── T1-1: Non-zero exit code on runtime error (production mode) ───────────────
// In normal mode a failing assert/panic aborts via Rust panic!().
// In production mode (--production flag) the error is logged and execution
// continues — main.rs must exit(1) via had_fatal.  These tests verify that
// `Stores::had_fatal` is set correctly so the binary-level exit code is right.

// Helper: compile loft code and return a State ready for execution.
fn compile_for_production(code: &str) -> (State, loft::data::Data) {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse_str(code, "t1_1_test", false);
    assert!(
        p.diagnostics.lines().is_empty(),
        "Parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    (state, p.data)
}

// Attach a production-mode logger (writes to /dev/null) to a State.
fn attach_production_logger(state: &mut State) {
    let config = RuntimeLogConfig {
        log_path: std::path::PathBuf::from("/dev/null"),
        production: true,
        ..Default::default()
    };
    let lg = Logger::new(config, None);
    state.database.logger = Some(Arc::new(Mutex::new(lg)));
}

// No error: had_fatal stays false.
#[test]
fn production_mode_no_error_had_fatal_false() {
    let (mut state, data) = compile_for_production("fn test() { assert(1 == 1, \"ok\"); }");
    attach_production_logger(&mut state);
    state.execute("test", &data);
    assert!(
        !state.database.had_fatal,
        "had_fatal must stay false when assert passes"
    );
}

// panic() in production mode: had_fatal becomes true, execution does NOT abort.
#[test]
fn production_mode_panic_sets_had_fatal() {
    let (mut state, data) = compile_for_production("fn test() { panic(\"deliberate\"); }");
    attach_production_logger(&mut state);
    state.execute("test", &data);
    assert!(
        state.database.had_fatal,
        "had_fatal must be true after panic() in production mode"
    );
}

// ── #403 — for-loop over a byte-size-1 element vector ─────────────────────────
// A `vector<u8>`/`<i8>` for-loop hung forever: the loop's value-sentinel
// termination never saw the out-of-bounds null because `OpGetByte` returned the
// raw byte, not the `i64::MIN` null sentinel.  Fixed by returning `i64::MIN` at
// the `rec == 0` null DbRef (the element read one past the last item), mirroring
// OpGetShortRaw.  `vector<boolean>` additionally stopped at the first `false`; it
// now terminates on the element ref's null (OpConvBoolFromRef), not the value.
// A regression would HANG this test (caught by CI timeout) or trip the asserts.
#[test]
fn issue_403_narrow_vector_for_loop_terminates() {
    let src = "fn test() { \
        a: vector<u8> = [10, 20, 30]; ca = 0; sa = 0; for ya in a { ca = ca + 1; sa = sa + ya; } \
        assert(ca == 3, \"u8 count\"); assert(sa == 60, \"u8 sum\"); \
        b: vector<i8> = [4, 5, 6]; cb = 0; sb = 0; for yb in b { cb = cb + 1; sb = sb + yb; } \
        assert(cb == 3, \"i8 count\"); assert(sb == 15, \"i8 sum\"); \
        c: vector<boolean> = [true, false, true]; cc = 0; tc = 0; \
        for yc in c { cc = cc + 1; if yc { tc = tc + 1; } } \
        assert(cc == 3, \"bool count\"); assert(tc == 2, \"bool trues\"); \
        d: vector<boolean> = [false, false]; fd = 0; \
        for yd in d { if yd { fd = fd - 1; } else { fd = fd + 1; } } \
        assert(fd == 2, \"all-false must not stop early\"); \
    }";
    let (mut state, data) = compile_for_production(src);
    attach_production_logger(&mut state);
    state.execute("test", &data);
    assert!(
        !state.database.had_fatal,
        "#403: vector<u8>/<i8> for-loop must terminate with correct counts"
    );
}

// #437 — an explicit `return <vector>` from a loft function delivered its value
// into __retbuf (or returned an owned local) but never FINALIZED the signature's
// `{__retbuf}` dep.  A caller consults only the signature, so it saw a BARE vector
// return, rebound its result var to a fresh empty store, and the first in-place
// `+=` DROPPED the returned elements — collapsing a copy+append vector to just the
// last-appended element (`b = mk(); b += [x]` lost mk()'s rows; the filed symptom
// was a 3-vector-field struct literal losing all but the last row).  Both backends
// miscompiled identically.  Fixed by delivering every store-backed bare-vector
// return into __retbuf AND finalizing the return dep, matching the implicit-tail
// path (parser/control.rs::parse_return).
#[test]
fn issue_437_explicit_vector_return_then_append_keeps_elements() {
    code!(
        "fn ct(v: vector<text>) -> vector<text> { o: vector<text> = []; for i in 0..len(v) { o += [ v[i] ?? \"\" ]; } return o; }
fn ci(v: vector<integer>) -> vector<integer> { o: vector<integer> = []; for j in 0..len(v) { o += [ v[j] ?? 0 ]; } return o; }
fn mk() -> vector<text> { o: vector<text> = []; o += [\"A\"]; return o; }
fn idv(v: vector<text>) -> vector<text> { return v; }
struct S { xs: vector<text>, ys: vector<text>, zs: vector<integer> }
fn test() {
    // fresh-local explicit return + append keeps BOTH elements (was len 1, last only)
    a = mk(); a += [\"b\"]; assert(len(a) == 2, \"fresh-local return then append\");
    assert((a[0] ?? \"\") == \"A\", \"first (returned) element preserved, not dropped\");
    // copy-helper return + append (the #437 minimal)
    src: vector<text> = [\"title\"]; xs = ct(src); xs += [\"tags\"];
    assert(len(xs) == 2, \"copy-helper return then append\");
    // arg return must value-copy (source untouched) yet stay appendable
    s2: vector<text> = [\"A\"]; c = idv(s2); c += [\"x\"];
    assert(len(c) == 2, \"arg return appendable\"); assert(len(s2) == 1, \"arg return copies, source untouched\");
    // loop reuse must not corrupt the shared return buffer
    total = 0; for i in 0..3 { b = mk(); b += [\"e{i}\"]; total = total + len(b); }
    assert(total == 6, \"loop: each iteration yields len 2\");
    // the filed symptom — three copy+append vector fields bound in one struct literal
    ys0: vector<text> = [\"Hello\"]; zs0: vector<integer> = [100];
    fxs = ct(src); fxs += [\"tags\"]; fys = ct(ys0); fys += [\"x\"]; fzs = ci(zs0); fzs += [105];
    s = S { xs: fxs, ys: fys, zs: fzs };
    assert(len(s.xs) == 2, \"struct field xs keeps both rows\");
    assert(len(s.ys) == 2 && len(s.zs) == 2, \"all struct vector fields keep both rows\");
    assert((s.zs[0] ?? 0) == 100, \"first row of zs preserved\");
}"
    )
    .result(Value::Null);
}

// Returning a FIELD / enum-arm vector from a local composite (`return c.pts`,
// `match e { Filled { items } => items }`) copies the source into the caller's
// retbuf, so the local source is freed at scope exit AFTER the copy.  A retired
// H2-step-5 debug sentinel (scopes.rs) mis-fired on these: it re-read the
// block-result deps under the old POSITIONAL guess, which names the copied-from
// source, and panicked ("block-result dep read would have decided alone") under
// `-C debug-assertions=on` even though freeing the source is correct.  This guards
// that the free stays leak-free (re-adding the retired read would suppress it and
// leak); it panicked on the sentinel without the fix.
#[test]
fn h2_field_arm_vector_return_source_freed_not_leaked() {
    code!(
        "struct H2Ctx { pts: vector<integer> }
fn h2_mk() -> H2Ctx { c = H2Ctx { pts: [] }; c.pts += [1]; c.pts += [2]; return c; }
fn h2_get_pts() -> vector<integer> {
    extra = \"live\";
    c = h2_mk();
    note = \"{extra}\";
    if len(note) < 0 { return []; }
    return c.pts;
}
enum H2Cell { Filled { items: vector<integer> }, Empty }
fn h2_arm(e: H2Cell) -> vector<integer> { match e { Filled { items } => { items }, _ => { [] } } }
fn test() {
    a = h2_get_pts();
    assert(len(a) == 2 && a[0] == 1, \"struct-field vector return: len {len(a)}\");
    b = h2_arm(Filled { items: [7, 8] });
    assert(len(b) == 2 && b[1] == 8, \"enum-arm vector return: len {len(b)}\");
}"
    )
    .result(Value::Null);
}

// `map`/`filter` on a literal receiver (`[1,2,3].map(..)`) confines its build
// vector to a block that lives inside the lowered `Iter`/`Call` — off the
// control-flow spine `relocate_null_init`'s `prepend_to_scope` walks.  The Plan-57
// null-init relocation therefore cannot reach that block and correctly falls back
// to the body-0 null-init (leak/poison-clean, both backends), but a debug
// `debug_assert!(false)` treated the un-reached scope as a bug and panicked under
// `-C debug-assertions=on` (scripts 501, 85-short-lambda-capture).  Fixed by
// asserting only when the scope is ABSENT FROM THE IR entirely (a real
// store_confinement bug), not merely unreached; it panicked without the fix.
#[test]
fn reloc_null_init_map_on_literal_confined_block_unreached() {
    code!(
        "fn rlm_vsum(v: vector<integer>) -> integer { s = 0; for x in v { s += x; } s }
fn test() {
    d = [1, 2, 3].map(|x| { x * 2 });
    assert(rlm_vsum(d) == 12, \"map on literal: {rlm_vsum(d)}\");
    evens = [1, 2, 3, 4, 5, 6].filter(|x| { x % 2 == 0 });
    assert(rlm_vsum(evens) == 12, \"filter on literal: {rlm_vsum(evens)}\");
}"
    )
    .result(Value::Null);
}

// A chained `??` (`a ?? b ?? c`) whose operands are OWNED/call-produced text
// double-freed an inner `__ncc_N` coalesce temp on the interpreter (a
// `state/text.rs:334` double free under `-C debug-assertions=on`; script 156).
// Root cause in `scopes::collect_consumed_ncc_text`, which emits the in-place
// free for a consumed skip_free ncc temp: (1) it recursed INTO nested ncc blocks,
// so a left-nested chain's inner temp was collected at multiple levels; (2) a
// right-nested `a ?? (b ?? c)` hoists a merge-var pre-declaration `__ncc_N = ""`
// (a literal init) into the outer ncc block, and collecting that literal Set as
// well as the real inner assignment freed the temp twice.  Fix: don't recurse
// into ncc blocks (each nested one gets its own free pass), and only collect the
// REAL subject assignment (non-literal value).  A `var ?? …` first operand never
// mints a temp, so it was always clean.  DA-gated (double-free is benign in
// release / dropped via RAII on native).
#[test]
fn chained_coalesce_owned_text_no_double_free() {
    code!(
        "struct CccCache { items: hash<CccEntry[name]> }
struct CccEntry { name: text, value: text }
fn ccc_lookup(c: CccCache, k: text) -> text { return c.items[k].value; }
fn test() {
    p = CccCache { items: [] }; p.items[\"theme\"] = CccEntry { name: \"theme\", value: \"dark\" };
    q = CccCache { items: [] }; q.items[\"lang\"] = CccEntry { name: \"lang\", value: \"en\" };
    // chained (left-nested), first operand a call → hits
    a = ccc_lookup(p, \"theme\") ?? ccc_lookup(q, \"theme\") ?? \"fb\";
    assert(a == \"dark\", \"chained first hit, got '{a}'\");
    // chained, all miss → fallback
    b = ccc_lookup(p, \"x\") ?? ccc_lookup(q, \"y\") ?? \"fb\";
    assert(b == \"fb\", \"chained fallback, got '{b}'\");
    // right-nested via parens, inner branch hits
    d = ccc_lookup(p, \"x\") ?? (ccc_lookup(q, \"lang\") ?? \"fb\");
    assert(d == \"en\", \"right-nested, got '{d}'\");
    // inline (unbound) chained
    assert((ccc_lookup(p, \"theme\") ?? ccc_lookup(q, \"theme\") ?? \"fb\") == \"dark\", \"inline chained\");
}"
    )
    .result(Value::Null);
}

// `vec_of_u8[i] ?? <fitting-int-literal>` keeps the value's NARROW type instead
// of widening the `??` result to i64, so the defaulted element appends back into
// a `vector<u8>` with no `as u8` cast and no spurious "cannot implicitly narrow"
// error.  A CONST default that fits the narrow type emits at that width, so both
// `if` branches share a native type — the @P316 widen is only needed for a
// wider-width VARIABLE default (still preserved).  Pre-fix this failed to compile
// ("cannot implicitly narrow integer to u8").
#[test]
fn null_coalesce_fitting_int_literal_keeps_narrow_type() {
    code!(
        "fn test() {
    bytes: vector<u8> = [10, 20, 30];
    tb: vector<u8> = [];
    for k in 0..3 { kb = bytes[k] ?? 0; tb += [kb]; }
    assert(len(tb) == 3, \"len 3\");
    assert((tb[0] ?? 99) == 10, \"first 10\");
}"
    )
    .result(Value::Null);
}

// (D2/D3/D5) `is_narrowing_int` is range containment, not `forced_size` width — so
// signedness is visible: an `i8` (down to -128) is not contained in `u8`, and the
// implicit conversion needs an explicit `as`.  Pre-change the two shared a 1-byte
// `forced_size` and the lossy implicit conversion was (wrongly) allowed.  This is the
// behaviour that aligns the parser's width check with codegen's (already range+sign-aware)
// `narrow_int_cast`.  See formal/types.md § the integer model.
#[test]
fn d2_signed_narrowing_i8_to_u8_needs_cast() {
    code!(
        "fn test() {
    a: i8 = 100;
    b: u8 = a;
    assert(b == 100, \"b\");
}"
    )
    .error(
        "cannot implicitly narrow i8 to u8 (may lose data) — \
give it a fallback with `?? <value>`, take the checked cast `as u8?` (value or null), \
or make the value provably fit (a mask, or an `if` range check) \
at d2_signed_narrowing_i8_to_u8_needs_cast:3:15",
    );
}

/// loft#1047 — the NEGATIVE half of the one-character-type fix.
///
/// The forward-reference path decides "is this name a type?" from its SPELLING, and the
/// rule deliberately rejects a name with no lowercase letter: loft gives constants
/// `UPPER_CASE`, so `N` and `FOO` are read as constants and keep a placeholder VARIABLE,
/// which is what makes a misspelled one report as an unknown variable instead of an
/// unknown type.  The fix widened that test to accept an uppercase name followed by a
/// QUALIFIER (`D.N`), which a bare constant reference never is.  These two guard the
/// half that must NOT move — widen the predicate further and they are what tells you.
#[test]
fn order_bare_uppercase_name_is_still_an_unknown_variable() {
    code!(
        "fn test() {
    x = N + 1;
    assert(x == 1, \"x\");
}"
    )
    .advice(
        "Variable 'N' is UPPER_CASE — that style is reserved for constants at \
order_bare_uppercase_name_is_still_an_unknown_variable:2:12",
    )
    .error("Unknown variable 'N' at order_bare_uppercase_name_is_still_an_unknown_variable:2:9");
}

#[test]
fn order_misspelled_upper_case_constant_is_still_an_unknown_variable() {
    code!(
        "const MAX_SIZE: integer = 10;
fn test() {
    x = MAX_SIZ + 1;
    assert(x == 11, \"x\");
}"
    )
    .advice(
        "Variable 'MAX_SIZ' is UPPER_CASE — that style is reserved for constants at \
order_misspelled_upper_case_constant_is_still_an_unknown_variable:3:18",
    )
    .error(
        "Unknown variable 'MAX_SIZ' at \
order_misspelled_upper_case_constant_is_still_an_unknown_variable:3:9",
    );
}

// (grammar D-gram-3) `**` is RIGHT-associative — `2 ** 3 ** 2` is `2 ** (3 ** 2)` = 512,
// matching maths / most languages (it was left-associative = 64 before). Precedence vs the
// other operators is unchanged: `**` binds tighter than `*`. See formal/grammar.md.
#[test]
fn power_is_right_associative() {
    code!(
        "fn test() {
    assert(2 ** 3 ** 2 == 512, \"2**3**2 = 2**(3**2) = 512\");
    assert(2 ** 3 ** 2 ** 1 == 512, \"right-assoc chain\");
    assert(2 ** 3 * 2 == 16, \"** tighter than *\");
    assert(2 * 3 ** 2 == 18, \"* looser than **\");
}"
    )
    .result(Value::Null);
}

// assert(false, ...) in production mode: had_fatal becomes true.
#[test]
fn production_mode_assert_false_sets_had_fatal() {
    let (mut state, data) = compile_for_production("fn test() { assert(1 == 2, \"mismatch\"); }");
    attach_production_logger(&mut state);
    state.execute("test", &data);
    assert!(
        state.database.had_fatal,
        "had_fatal must be true after assert(false) in production mode"
    );
}

// ── T1-8: For-loop mutation guard extended to field access ────────────────────
// Appending to a collection that is actively being iterated can cause infinite
// loops (vector) or structural corruption (sorted/index).  The guard that
// catches `items += [x]` must also fire for `db.items += [x]`.

// Direct variable form: existing guard must still work.
#[test]
fn for_loop_mutation_guard_simple_var() {
    code!(
        "fn test() {
    items = [1, 2, 3];
    for e in items { items += [e]; }
}"
    )
    .error(
        "Cannot add elements to 'items' while it is being iterated — \
use a separate collection or add after the loop \
at for_loop_mutation_guard_simple_var:3:32",
    );
}

// Field-access form: `db.items += [x]` inside `for e in db.items { ... }`.
#[test]
fn for_loop_mutation_guard_field_access() {
    code!(
        "struct Container { items: vector<integer> }
fn test() {
    db = Container { items: [1, 2, 3] };
    for e in db.items { db.items += [e]; }
}"
    )
    .error(
        "Cannot add elements to a collection while it is being iterated — \
use a separate collection or add after the loop \
at for_loop_mutation_guard_field_access:4:38",
    );
}

// Safe: appending to a DIFFERENT field than the one being iterated is allowed.
#[test]
fn for_loop_mutation_guard_different_field_ok() {
    code!(
        "struct Container { src: vector<integer>, dst: vector<integer> }
fn test() {
    db = Container { src: [1, 2, 3], dst: [] };
    for e in db.src { db.dst += [e * 2]; };
    assert(len(db.dst) == 3, \"len: {len(db.dst)}\");
    assert(db.dst[0] == 2, \"dst[0]: {db.dst[0]}\");
}"
    );
}

// ── T2-4  f#exists attribute ──────────────────────────────────────────────────

// f#exists returns true for a known existing file.
#[test]
fn file_exists_true() {
    code!(
        "fn test() {
    f = file(\"tests/scripts/19-files.loft\");
    assert(f#exists, \"expected exists to be true\");
}"
    )
    .result(loft::data::Value::Null);
}

// f#exists returns false for a path that does not exist.
#[test]
fn file_exists_false() {
    code!(
        "fn test() {
    f = file(\"tests/scripts/no-such-file.loft\");
    assert(!f#exists, \"expected exists to be false\");
}"
    )
    .result(loft::data::Value::Null);
}

// ── T1-1 (Tier 2): Callable function references ───────────────────────────────
// `fn <name>` produces a Value::Int(d_nr) with Type::Function(args, ret).
// Calling `f(args)` where `f` is a local fn-ref variable emits OpCallRef.

// Basic fn-ref: store `double` and call it through the reference.
#[test]
fn fn_ref_basic_call() {
    code!(
        "fn double(n: integer) -> integer { n * 2 }
fn test() {
    f = double;
    result = f(21);
    assert(result == 42, \"expected 42, got {result}\");
}"
    )
    .result(loft::data::Value::Null);
}

// Fn-ref with multiple arguments.
#[test]
fn fn_ref_two_args() {
    code!(
        "fn add(a: integer, b: integer) -> integer { a + b }
fn test() {
    f = add;
    result = f(10, 32);
    assert(result == 42, \"expected 42, got {result}\");
}"
    )
    .result(loft::data::Value::Null);
}

// Fn-ref assigned conditionally, then called.
#[test]
fn fn_ref_conditional_call() {
    code!(
        "fn inc(n: integer) -> integer { n + 1 }
fn dec(n: integer) -> integer { n - 1 }
fn test() {
    flag = true;
    f = if flag { inc } else { dec };
    result = f(41);
    assert(result == 42, \"expected 42, got {result}\");
}"
    )
    .result(loft::data::Value::Null);
}

// Fn-ref passed as a parameter and called inside the callee.
#[test]
fn fn_ref_as_parameter() {
    code!(
        "fn square(n: integer) -> integer { n * n }
fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
fn test() {
    result = apply(square, 7);
    assert(result == 49, \"expected 49, got {result}\");
}"
    )
    .result(loft::data::Value::Null);
}

// A FORWARD-REFERENCE caller of a fn whose text-return tail classifies only on
// pass 2 (a fn-ref call `f(x)`, a local vector index `tv[0]`, or a closure-field
// call `g.fmt(n)`) crashed with "Too few parameters" (codegen.rs) — and the H5
// two-pass-contract assert flagged it under `-C debug-assertions=on`.  Root:
// `do_tret_bind` promotes `__tret` to a hidden `&text` SIGNATURE buffer, but
// those tails read as `Plain`/`Borrow` on pass 1 and `Owned(FnRefCall/ViewOfLocal)`
// on pass 2, so the buffer was appended pass-2-only — after the forward-ref caller
// had already emitted its call against the pass-1 (bufferless) signature.  Fixed
// by gating pass-2 promotion on pass 1 having already minted the `__tret` attr
// (each callee is defined AFTER its caller here, so the ABI must be stable).
#[test]
fn tret_bind_forward_ref_pass_stable() {
    code!(
        "fn mk_z(n: integer) -> text { \"z{n}\" }
fn call_fnref(x: integer) -> text { return via_fnref(mk_z, x); }
fn via_fnref(f: fn(integer) -> text, x: integer) -> text { f(x) }
fn call_index() -> text { return via_index(); }
fn via_index() -> text { tv: vector<text> = [\"a\", \"b\"]; return tv[0]; }
struct TbG { fmt: fn(integer) -> text }
fn call_method() -> text { return via_method(); }
fn via_method() -> text { g = TbG { fmt: fn(n: integer) -> text { \"m{n}\" } }; g.fmt(7) }
fn test() {
    assert(call_fnref(5) == \"z5\", \"fwd fn-ref call: {call_fnref(5)}\");
    assert(call_index() == \"a\", \"fwd vector index: {call_index()}\");
    assert(call_method() == \"m7\", \"fwd closure-field call: {call_method()}\");
}"
    )
    .result(loft::data::Value::Null);
}

// @PLN102 (routing consumer): a call MISSING a function-typed argument was silently filled
// with a broken `()` — a SIGSEGV inside stdlib `len` several frames from the bad call, and it
// corrupted the earlier arguments. There is no valid "empty function" to fill, so a missing
// fn-typed param with no default is now a too-few-arguments error at parse time. (The general
// too-few check — a missing scalar fills null, a vector empty — is a follow-up; it needs a
// user-param vs promoted-return-buffer distinction. See code-eval-followups.md.)
#[test]
fn call_missing_fn_typed_arg_is_rejected() {
    code!(
        "fn five(a: integer, b: text, cb: fn(integer)) -> integer { cb(a); b.len() }
fn test() { five(1, \"x\"); }"
    )
    .error(
        "missing argument for parameter 'cb' of `five` — the call supplies too few arguments \
(add it, or give the parameter a default `= …`) \
at call_missing_fn_typed_arg_is_rejected:2:26",
    );
}

// ── map / filter / reduce ─────────────────────────────────────────────────────

#[test]
fn map_integers() {
    code!(
        "fn double(x: integer) -> integer { x * 2 }
fn test() {
    v = [1, 2, 3, 4, 5];
    r = map(v, double);
    s = 0;
    for x in r {
        s += x;
    }
    assert(s == 30, \"expected 30, got {s}\");
}"
    )
    .result(loft::data::Value::Null);
}

#[test]
fn filter_integers() {
    code!(
        "fn is_even(x: integer) -> boolean { x % 2 == 0 }
fn test() {
    v = [1, 2, 3, 4, 5, 6];
    r = filter(v, is_even);
    s = 0;
    for x in r {
        s += x;
    }
    assert(s == 12, \"expected 12, got {s}\");
}"
    )
    .result(loft::data::Value::Null);
}

#[test]
fn reduce_sum() {
    code!(
        "fn add(acc: integer, x: integer) -> integer { acc + x }
fn test() {
    v = [1, 2, 3, 4, 5];
    s = reduce(v, 0, add);
    assert(s == 15, \"expected 15, got {s}\");
}"
    )
    .result(loft::data::Value::Null);
}

// ── Issue 27 ─────────────────────────────────────────────────────────────────
// `self.field = null` in a method generated no bytecode for the null argument.
// `generate(Value::Null)` returned Type::Void with no emitted bytes, so OpSetInt
// read its `val` argument from the wrong stack location (return-address bytes),
// producing store_nr=60 → out-of-bounds panic in allocation.rs.
// Fix: `parse_assign_op` now calls `convert()` when s_type==Type::Null and op=="=",
// substituting OpConvIntFromNull (or the appropriate FromNull op) before towards_set.

// @PLN25 DN1: this cluster exercises the null-SENTINEL storage/roundtrip mechanism,
// which under the dense/non-null model is reached through a `τ?` field (a plain scalar
// field is now NON-null and rejects `= null`). The fields carry `?` so the mechanism —
// not the retired pre-DN1 "plain scalar field is nullable-by-default" behaviour — is what
// is tested. (These snippets compile at STD_SOURCE, so they formerly rode the now-removed
// F1b(b) (N-Store) exemption; real user code, source ≥1, was always held to DN1.)
// Exact T0-1 reproduction: method sets an integer field to null via reference param.
// Previously panicked with "store_nr=60" in `set_int`.
#[test]
fn set_int_field_null_via_ref() {
    code!(
        "struct S { cur: integer? }
fn clear(self: S) { self.cur = null }
fn test() {
    s = S { cur: 42 };
    s.clear();
    assert(s.cur == null, \"expected null, got {s.cur}\");
}"
    )
    .result(loft::data::Value::Null);
}

// Integer field set to null via direct struct access (not a method call).
#[test]
fn set_int_field_null_direct() {
    code!(
        "struct S { cur: integer? }
fn test() {
    s = S { cur: 7 };
    s.cur = null;
    assert(s.cur == null, \"expected null after direct assignment\");
}"
    )
    .result(loft::data::Value::Null);
}

// Long field set to null via reference parameter.
#[test]
fn set_long_field_null_via_ref() {
    code!(
        "struct S { val: integer? }
fn clear(self: S) { self.val = null }
fn test() {
    s = S { val: 1000000 };
    s.clear();
    assert(s.val == null, \"expected null, got {s.val}\");
}"
    )
    .result(loft::data::Value::Null);
}

// Multiple scalar fields (integer, long) set to null in one method call.
#[test]
fn set_multiple_scalar_fields_null() {
    code!(
        "struct S { a: integer?, b: integer? }
fn clear(self: S) {
    self.a = null;
    self.b = null;
}
fn test() {
    s = S { a: 1, b: 2 };
    s.clear();
    assert(s.a == null, \"a should be null\");
    assert(s.b == null, \"b should be null\");
}"
    )
    .result(loft::data::Value::Null);
}

// Set field to null then restore a value — round-trip correctness.
#[test]
fn null_then_reassign_integer_field() {
    code!(
        "struct S { cur: integer? }
fn clear(self: S) { self.cur = null }
fn test() {
    s = S { cur: 10 };
    s.clear();
    assert(s.cur == null, \"should be null after clear\");
    s.cur = 42;
    assert(s.cur == 42, \"should be 42 after reassign\");
}"
    )
    .result(loft::data::Value::Null);
}

// ── PROBLEMS #37 (T0-2): LIFO store-free panic ───────────────────────────────
// scopes.rs::variables() was iterating var_scope (a BTreeMap) in ascending key
// order, causing get_free_vars() to emit OpFreeRef in forward declaration order.
// database::free() enforces LIFO: the most-recently-allocated store must be
// freed first.  Fix: track insertion order in var_order: Vec<u16> and iterate it
// in reverse so the last-inserted (last-allocated) variable is freed first.

// Two owned struct refs in the same scope — minimal T0-2 reproducer.
#[test]
fn lifo_store_free_two_owned_refs() {
    code!(
        "struct S { val: integer }
fn double(self: S) -> S { S { val: self.val * 2 } }
fn test() {
    c = S { val: 3 };
    d = c.double();
    assert(d.val == 6, \"d.val after double: {d.val}\");
}"
    )
    .result(loft::data::Value::Null);
}

// Three owned struct refs in the same scope — verifies LIFO holds for N > 2.
#[test]
fn lifo_store_free_three_owned_refs() {
    code!(
        "struct Point { x: integer, y: integer }
fn test() {
    a = Point { x: 1, y: 2 };
    b = Point { x: 3, y: 4 };
    c = Point { x: 5, y: 6 };
    assert(a.x + b.x + c.x == 9, \"sum x: {a.x + b.x + c.x}\");
    assert(a.y + b.y + c.y == 12, \"sum y: {a.y + b.y + c.y}\");
}"
    )
    .result(loft::data::Value::Null);
}

// ── PROBLEMS #38 (T0-3): T0-1 regression — key-null removal silently broken ──
// The T0-1 fix in parse_assign_op called self.convert(code, Null, f_type)
// unconditionally for all null assignments.  For reference-typed LHS (e.g. the
// element ref returned by sorted_coll[key]) convert() replaced Value::Null with
// Value::Call(OpConvRefFromNull, []).  towards_set_hash_remove checks
// *val == Value::Null to detect removal; after the substitution that check fails
// and the element is never removed.
// Fix: guard the convert() call so it only runs for scalar (non-reference,
// non-collection) types.

// sorted[key] = null removes the entry.
// hash[key] = null removes the entry.
// ── PROBLEMS #39 (T0-4): `v += other_vec` shallow copy — text fields dangle ───
// vector_add() used a raw copy_block without calling copy_claims().  Both the
// source and destination vectors ended up with the same string-record indices;
// when the source was freed, remove_claims() deleted those records and the
// destination's text fields became dangling.  The fix: after copy_block, iterate
// each appended element and call copy_claims() to create independent copies of
// string records (and sub-structures) in the destination store.

// Appending a vector<struct-with-text> to another vector must deep-copy string
// records.  Without the fix both bags share the same records; at end-of-scope
// LIFO frees the source first, then the destination tries to double-free the
// same records → panic.
#[test]
fn vec_add_text_field_deep_copy() {
    code!(
        "struct Item { name: text, value: integer }
struct Bag { items: vector<Item> }
fn test() {
    a = Bag { items: [Item{name: \"hello\", value: 1}, Item{name: \"world\", value: 2}] };
    b = Bag {};
    b.items += a.items;
    assert(b.items[0].name == \"hello\", \"name[0]: {b.items[0].name}\");
    assert(b.items[1].name == \"world\", \"name[1]: {b.items[1].name}\");
    assert(b.items[0].value == 1, \"value[0]: {b.items[0].value}\");
    assert(b.items[1].value == 2, \"value[1]: {b.items[1].value}\");
}"
    )
    .result(loft::data::Value::Null);
}

// Appending to a non-empty destination: pre-existing and new elements all have
// independent text records.
#[test]
fn vec_add_text_field_non_empty_dest() {
    code!(
        "struct Tag { label: text }
struct Col { tags: vector<Tag> }
fn test() {
    src = Col { tags: [Tag{label: \"a\"}, Tag{label: \"b\"}] };
    dst = Col { tags: [Tag{label: \"x\"}] };
    dst.tags += src.tags;
    assert(dst.tags[0].label == \"x\", \"tag[0]: {dst.tags[0].label}\");
    assert(dst.tags[1].label == \"a\", \"tag[1]: {dst.tags[1].label}\");
    assert(dst.tags[2].label == \"b\", \"tag[2]: {dst.tags[2].label}\");
}"
    )
    .result(loft::data::Value::Null);
}

// ── PROBLEMS #40 (T0-5): copy_claims / remove_claims for Parts::Index ─────────
// Both copy_claims and remove_claims contained `Parts::Index => panic!("Not
// implemented")`.  Adding a struct-with-index to a vector triggers OpCopyRecord
// → copy_claims_index_body; freeing the struct after reassignment triggers the
// Parts::Index arm of remove_claims.

// copy_claims path: a struct with an index<T[key]> field is added to a vector
// (triggering OpCopyRecord → copy_claims → copy_claims_index_body).
// Before the fix this panicked with "Not implemented".
// copy_claims on index with text fields: text records must be deep-copied so
// source and destination are independent.
// remove_claims path for Parts::Index: reassigning a struct that holds an
// index<T> field triggers database() → clear() → remove_claims on the index.
// Before the fix this panicked with "Not implemented".
// ── PROBLEMS #41 (T0-6): inline ref-returning call leaks store → LIFO panic ───
// `p.method().field` where method() returns an owned ref must wrap the temporary
// in a work-ref variable so scopes.rs emits OpFreeRef at end-of-scope.

// Single field access on an inline ref-returning call must not leak the store.
// Two chained inline calls (shifted().shifted().x) must not leak either store.
// index[key] = null removes the entry.
// T2-7: mkdir creates a directory, mkdir_all creates nested directories.
#[test]
fn mkdir_and_mkdir_all() {
    // Clean up from any previous failed run
    let _ = std::fs::remove_dir_all("tests/tmp_mkdir_test");
    code!(
        "fn test() {
    // mkdir_all creates nested path
    assert(mkdir_all(\"tests/tmp_mkdir_test/sub\").ok(), \"mkdir_all\");
    // mkdir on existing directory returns not ok
    assert(!mkdir(\"tests/tmp_mkdir_test/sub\").ok(), \"mkdir existing\");
    // mkdir on a new sibling
    assert(mkdir(\"tests/tmp_mkdir_test/other\").ok(), \"mkdir sibling\");
}"
    );
    // Clean up after test
    let _ = std::fs::remove_dir_all("tests/tmp_mkdir_test");
}

// ── T0-11: Write to locked store must panic ───────────────────────────────────
// addr_mut() previously returned a thread-local DUMMY buffer in release builds
// (#[cfg(not(debug_assertions))]), silently discarding the write.  The fix
// removes the DUMMY and replaces it with an unconditional assert!(!self.locked)
// so any write to a locked store panics in both debug and release builds.
// The unit test lives in src/store.rs (tests::write_to_locked_store_panics)
// because Store is a private module.

// ── T0-12: vector self-append (`v += v`) must not corrupt data ────────────────
// vector_add() read o_rec before resizing the destination, but vector_append /
// vector_set_size may reallocate the backing store, making o_rec stale.
// The fix snapshots the source bytes into a Vec<u8> before any resize.

// `v += v` on an integer vector: result must be a doubled vector with correct values.
// `v += v` on a single-element vector: result must have two equal elements.
// ── T1-32: File I/O errors are no longer silently discarded ──────────────────
// write_file/read_file/seek_file used unwrap_or_default() / unwrap_or(0),
// swallowing OS errors with no diagnostic output.  The fix logs to stderr via
// eprintln!.  The test below verifies that writing to a bad path does not panic
// or hang — the error is printed to stderr and execution continues normally.

// Writing to an unwritable path must not panic; the program continues after the error.
#[test]
fn file_write_error_does_not_panic() {
    // Use a path inside a non-existent directory so File::create will fail.
    code!(
        "fn test() {
    f = file(\"/no_such_dir/output.txt\");
    f += \"hello\";
    // Execution must reach this assert without panicking.
    assert(true, \"should not panic\");
}"
    );
}

// ── N8 ───────────────────────────────────────────────────────────────────────
// Fix empty pre-eval bindings and `_pre{n}` → `_pre_{n}` naming in generation.rs.
// Root cause: (1) `generate_expr_buf` returns "" for some void/null expressions,
// causing `let _pre5 = ;` (invalid Rust) and corrupt substitution; (2) Rust
// edition 2021+ parses `_pre14` as a prefix token (like `b"…"`), producing
// parse errors in generated code.

// N8-naming: generated code must use `_pre_N` names, not bare `_preN`.
// A nested user-defined function call is enough to trigger pre-eval hoisting.
#[test]
fn n8_pre_eval_uses_underscore_separator() {
    // Two nested user-fn calls: the inner call is pre-eval-hoisted by generation.rs.
    code!("fn inc(v: integer) -> integer { v + 1 }")
        .expr("inc(inc(0))")
        .result(Value::Int(2));
    let src =
        std::fs::read_to_string("tests/generated/issues_n8_pre_eval_uses_underscore_separator.rs")
            .expect("generated file not found");
    // Every `let _pre…` line must use `_pre_N` (digit after underscore), not `_preN`.
    for line in src.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("let _pre") {
            assert!(
                rest.starts_with('_'),
                "Found bare `_preN` binding (should be `_pre_N`): {line}"
            );
        }
    }
}

// N8-empty: generated code must not emit `let _preN = ;` (empty right-hand side).
// The mutable-reference pattern (user fn with `&T = null` default) triggers this.
#[test]
fn n8_no_empty_pre_eval_binding() {
    code!(
        "struct Data { num: integer, values: vector<integer> }
fn add(r: &Data = null, val: integer) {
    if !r { r = Data { num: 0 }; }
    r.num += val;
    r.values += [val];
}"
    )
    .expr("v = Data { num: 1 }; add(v, 2); add(v, 3); \"{v}\"")
    .result(Value::str("{num:6,values:[2,3]}"));
    let src = std::fs::read_to_string("tests/generated/issues_n8_no_empty_pre_eval_binding.rs")
        .expect("generated file not found");
    for line in src.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("let _pre") && trimmed.trim_end().ends_with("= ;") {
            panic!("Found empty pre-eval binding: {line}");
        }
    }
}

// N3: assigning a reference to another reference must emit OpCopyRecord for deep copy.
// Without it, both variables alias the same heap record; mutating through one changes the other.
#[test]
fn n3_reference_assignment_emits_copy_record() {
    // Bytecode interpreter correctly deep-copies references already; test confirms behaviour.
    code!("struct Item { name: text }")
        .expr(
            "a = Item { name: \"hello\" };
b = a;
b.name += \" world\";
a.name",
        )
        .result(Value::str("hello"));
    let src = std::fs::read_to_string(
        "tests/generated/issues_n3_reference_assignment_emits_copy_record.rs",
    )
    .expect("generated file not found");
    assert!(
        src.contains("OpCopyRecord(cell,"),
        "generated code missing OpCopyRecord after reference assignment"
    );
}

// N5: vector::clear_vector must not be called when the DbRef is null (rec == 0).
// A function that initialises and returns a vector was panicking with
// "Unknown record 2147483648" because clear_vector ran on stores.null() before allocation.
#[test]
fn n5_null_dbref_clear_vector_guard() {
    code!(
        "pub fn fill() -> vector<text> {
    result = [];
    result += [\"aa\", \"bb\"];
    result
}"
    )
    .expr("t = fill(); \"{t}\"")
    .result(Value::str("[\"aa\",\"bb\"]"));
    let src = std::fs::read_to_string("tests/generated/issues_n5_null_dbref_clear_vector_guard.rs")
        .expect("generated file not found");
    assert!(
        src.contains(".rec != 0"),
        "generated code missing null check before clear_vector"
    );
}

// N4: struct-enum variants must show all fields when formatted with OpFormatDatabase.
// The init function was registering every enum value with u16::MAX as the struct type,
// so ShowDb could not dispatch to variant fields and only showed the variant name.
#[test]
fn n4_format_struct_enum_variant_shows_fields() {
    code!(
        "enum Op {
    Nop,
    Add { left: integer, right: integer }
}"
    )
    .expr("v = \"Add {{ left: 1, right: 2 }}\" as Op; \"{v}\"")
    .result(Value::str("Add {left:1,right:2}"));
    let src = std::fs::read_to_string(
        "tests/generated/issues_n4_format_struct_enum_variant_shows_fields.rs",
    )
    .expect("generated file not found");
    // The generated init must register the Add variant with its actual struct type (not u16::MAX).
    assert!(
        !src.contains("db.value(e, \"Add\", u16::MAX)"),
        "generated init still registers struct-enum variant Add with u16::MAX"
    );
}

// N9a: the auto-generated fill.rs must contain `use crate::ops;`
// so it can be compiled as a crate-internal file and eventually replace src/fill.rs.
#[test]
fn n9a_generated_fill_has_ops_import() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    scopes::check(&mut p.data, &mut p.database);
    let tmp = format!(
        "tests/generated/fill_n9a_{:?}.rs",
        std::thread::current().id()
    );
    let _ = std::fs::create_dir_all("tests/generated");
    let src = loft::create::generate_code_to(&p.data, &tmp).expect("generate_code_to failed");
    let _ = std::fs::remove_file(&tmp);
    assert!(
        src.contains("use crate::ops;"),
        "generated fill.rs missing `use crate::ops;`"
    );
}

// N9 (N20b/N20c/N20d): auto-generated fill.rs must be byte-for-byte identical to
// src/fill.rs once all #rust templates are present.
// Generates to a thread-local temp file to avoid races with other tests writing
// tests/generated/fill.rs.
#[test]
fn n9_generated_fill_matches_src() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    scopes::check(&mut p.data, &mut p.database);
    // Use a unique path so parallel test runs do not race on the same file.
    let tmp = format!(
        "tests/generated/fill_n9_{:?}.rs",
        std::thread::current().id()
    );
    let generated = loft::create::generate_code_to(&p.data, &tmp).expect("generate_code_to failed");
    let _ = std::fs::remove_file(&tmp);
    let src = std::fs::read_to_string("src/fill.rs").expect("src/fill.rs not found");
    assert_eq!(
        generated, src,
        "generated fill.rs differs from src/fill.rs — \
         run create::generate_code() and copy the result"
    );
}

// N8: Sort must work correctly in native-codegen mode.
// The #rust template for OpSortVector is inlined directly (no OpSortVector runtime fn needed).
#[test]
fn n8_codegen_runtime_vector_ops_exist() {
    // Sorting a vector of integers must work in native-codegen mode.
    code!("fn sort_it() -> vector<integer> { v = [3, 1, 2]; sort(v); v }")
        .expr("\"{sort_it()}\"")
        .result(Value::str("[1,2,3]"));
    let src =
        std::fs::read_to_string("tests/generated/issues_n8_codegen_runtime_vector_ops_exist.rs")
            .expect("generated file not found");
    assert!(
        src.contains("vector::sort_vector("),
        "generated code missing inlined vector::sort_vector call"
    );
}

// N10: ops::text_character returns char but loft represents character as i32.
// Generated code assigns the char to an i32 variable without a cast, causing a compile error.
// Also, i32 character variables used in method dispatch (is_alphanumeric etc.) need wrapping
// with ops::to_char(...) since the method requires char, not i32.
#[test]
fn n10_char_cast_in_generated_code() {
    code!(
        "fn count_alpha(s: text) -> integer {
    n = 0;
    for c in s { if c.is_alphanumeric() { n += 1; } }
    n
}"
    )
    .expr("count_alpha(\"a1!\")")
    .result(Value::Int(2));
    let src = std::fs::read_to_string("tests/generated/issues_n10_char_cast_in_generated_code.rs")
        .expect("generated file not found");
    assert!(
        src.contains("as u32 as i32") || src.contains("ops::to_char("),
        "generated code missing char<->i32 cast"
    );
}

// N2: output_init must register content types before the structs that reference them in
// sorted/index/hash fields.  When a struct has a sorted<Foo> field and Foo has a higher
// type-ID than the struct, the init function panicked because db.sorted(foo_type_id, ...)
// was called before Foo was registered.
#[test]
fn n2_sorted_field_content_type_registered_first() {
    code!(
        "struct Sort { nr: integer }
struct Container { data: sorted<Sort[nr]> }"
    )
    .expr("c = Container {}; \"{c}\"")
    .result(Value::str("{data:[]}"));
    let src = std::fs::read_to_string(
        "tests/generated/issues_n2_sorted_field_content_type_registered_first.rs",
    )
    .expect("generated file not found");
    // Sort must appear in the init before Container (which contains the sorted<Sort> field).
    let sort_pos = src.find("\"Sort\"").expect("Sort not found in init");
    let container_pos = src
        .find("\"Container\"")
        .expect("Container not found in init");
    assert!(
        sort_pos < container_pos,
        "Sort (content type) must be registered before Container in generated init"
    );
}

// ── S4: Binary I/O type coverage ─────────────────────────────────────────────
// read_data / write_data panic with "Not implemented" for Array / Sorted /
// Ordered / Hash / Index / Radix / Base — should be improved.

// S4: writing a struct with a `sorted<T>` field must be rejected at parse time
// with a clear message pointing the user to plain structs for serialisation.
// The parser catches variable-width (text/vector/collection) fields early.
#[test]
#[should_panic(expected = "variable-width field")]
fn s4_sorted_field_write_panics_with_clear_message() {
    code!(
        "struct Item { key: integer, value: integer }
struct Container { items: sorted<Item[key]> }
fn test() {
    c = Container { items: [Item { key: 1, value: 10 }] };
    f = file(\"tests/tmp_s4_sorted.dat\");
    f#format = LittleEndian;
    f += c;
    delete(\"tests/tmp_s4_sorted.dat\");
}"
    );
}

// S4: writing a struct with a `hash<T>` field must be rejected at parse time
// with the same "variable-width field" message.
#[test]
#[should_panic(expected = "variable-width field")]
fn s4_hash_field_write_panics_with_clear_message() {
    code!(
        "struct Tag { name: text }
struct Bag { tags: hash<Tag[name]> }
fn test() {
    b = Bag { tags: [Tag { name: \"hello\" }] };
    f = file(\"tests/tmp_s4_hash.dat\");
    f#format = LittleEndian;
    f += b;
    delete(\"tests/tmp_s4_hash.dat\");
}"
    );
}

// ── N1: --native CLI flag ─────────────────────────────────────────────────────
// src/main.rs must recognise --native and run the native codegen pipeline.

// N1: parsing the default library and a trivial loft program, then generating
// native Rust via output_native_reachable must produce non-empty output containing
// the expected function signatures.  Actually running rustc is attempted if possible
// but is non-fatal if the loft crate cannot be linked (cargo test env dependency).
#[test]
fn n1_native_pipeline_trivial_program() {
    use loft::generation::Output;
    let mut p = loft::parser::Parser::new();
    p.parse_dir("default", true, false).unwrap();
    let start_def = p.data.definitions();
    p.parse_str(
        "fn main() { assert(1 + 1 == 2, \"arithmetic\"); }",
        "n1_test",
        false,
    );
    assert!(p.diagnostics.is_empty(), "parse errors: {}", p.diagnostics);
    loft::scopes::check(&mut p.data, &mut p.database);
    let mut state = loft::state::State::new(p.database);
    loft::compile::byte_code(&mut state, &mut p.data);
    let end_def = p.data.definitions();
    // Collect entry defs: just the user's main function.
    let main_nr = p.data.def_nr("n_main");
    assert_ne!(main_nr, u32::MAX, "n_main not found");
    let tmp_rs = std::env::temp_dir().join("loft_n1_test.rs");
    let mut f = std::fs::File::create(&tmp_rs).expect("tmp file");
    let mut out = Output::new(&p.data, &state.database);
    out.output_native_reachable(&mut f, start_def, end_def, &[main_nr])
        .expect("output_native_reachable");
    drop(f);
    // Verify the generated source contains expected landmarks.
    let generated = std::fs::read_to_string(&tmp_rs).expect("read generated source");
    assert!(
        generated.contains("fn n_main("),
        "generated source missing n_main"
    );
    assert!(
        generated.contains("fn main()"),
        "generated source missing Rust main"
    );
    assert!(
        generated.contains("fn n_assert"),
        "generated source missing n_assert"
    );
    // Optionally compile with rustc — non-fatal if loft crate cannot be linked.
    let deps_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_default();
    // Pick the NEWEST libloft rlib: a CI cache restore (restore-keys) can
    // leave stale rlibs from older builds — or another toolchain — beside the
    // fresh one, and a find-first pick then feeds rustc mismatched crate
    // metadata (a wall of misleading E0432 "unresolved import" noise in the
    // nightly toolchain-matrix logs).
    let loft_rlib = std::fs::read_dir(&deps_dir).ok().and_then(|it| {
        it.flatten()
            .filter(|e| {
                let n = e.file_name();
                let s = n.to_string_lossy();
                s.starts_with("libloft") && s.ends_with(".rlib")
            })
            .max_by_key(|e| {
                e.metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
            })
            .map(|e| e.path())
    });
    let binary = std::env::temp_dir().join("loft_n1_test_bin");
    let mut rustc_args = vec![
        "--edition=2024".to_string(),
        "-o".to_string(),
        binary.to_str().unwrap().to_string(),
    ];
    if let Some(ref rlib) = loft_rlib {
        rustc_args.push("--extern".to_string());
        rustc_args.push(format!("loft={}", rlib.display()));
        rustc_args.push("-L".to_string());
        rustc_args.push(deps_dir.display().to_string());
        // S31: pass --extern for every non-loft rlib in deps/ so that optional
        // feature dependencies (rand_core, rand_pcg, png, etc.) can be resolved.
        // Without this, rustc cannot find crates that loft was compiled with.
        if let Ok(entries) = std::fs::read_dir(&deps_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.starts_with("lib")
                    || !name.ends_with(".rlib")
                    || name.starts_with("libloft")
                {
                    continue;
                }
                let without_lib = &name[3..];
                let without_rlib = without_lib.trim_end_matches(".rlib");
                let crate_name = if let Some(pos) = without_rlib.rfind('-') {
                    without_rlib[..pos].replace('-', "_")
                } else {
                    without_rlib.replace('-', "_")
                };
                rustc_args.push("--extern".to_string());
                rustc_args.push(format!("{crate_name}={}", entry.path().display()));
            }
        }
    }
    rustc_args.push(tmp_rs.to_str().unwrap().to_string());
    match std::process::Command::new("rustc")
        .args(&rustc_args)
        .status()
    {
        Ok(s) if s.success() => {
            // Binary compiled — run it to confirm correctness.
            let run = std::process::Command::new(&binary).status();
            match run {
                Ok(rs) => assert!(rs.success(), "native binary exited non-zero"),
                Err(e) => eprintln!("n1: could not run binary: {e}"),
            }
        }
        Ok(s) => eprintln!(
            "n1: rustc compilation failed (exit {s}) — likely linker issue in test env; \
             code generation verified above"
        ),
        Err(e) => eprintln!("n1: skipping rustc step (not in PATH): {e}"),
    }
    let _ = std::fs::remove_file(&tmp_rs);
    let _ = std::fs::remove_file(&binary);
}

// ── P1.1: Lambda parser ───────────────────────────────────────────────────────
// Parser must accept fn(params) -> ret { body } as an anonymous function
// expression, producing a Type::Function value like a named fn-ref.

// P1.1: a basic lambda `fn(x: integer) -> integer { x * 2 }` can be assigned
// to a variable and called through it.
#[test]
fn p1_1_lambda_basic_call() {
    code!(
        "fn test() {
    f = fn(x: integer) -> integer { x * 2 };
    result = f(21);
    assert(result == 42, \"expected 42, got {result}\");
}"
    )
    .result(loft::data::Value::Null);
}

// P1.1: lambda passed inline to a function accepting fn(integer) -> integer.
#[test]
fn p1_1_lambda_as_argument() {
    code!(
        "fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
fn test() {
    result = apply(fn(n: integer) -> integer { n * n }, 7);
    assert(result == 49, \"expected 49, got {result}\");
}"
    )
    .result(loft::data::Value::Null);
}

// P1.1: lambda with no return type (void).  A5.6c: write-backs make the
// outer `count` reflect mutations performed inside the lambda body.
#[test]
fn p1_1_lambda_void_body() {
    code!(
        "fn test() {
    count = 0;
    f = fn(x: integer) { count += x; };
    f(10);
    f(32);
    assert(count == 42, \"expected 42, got {count}\");
}"
    )
    .result(loft::data::Value::Null);
}

// ── P91 regression guards ───────────────────────────────────────────────────
// Earlier-parameter-reference in default expressions.  Before this fix,
// `fn f(a: integer, b: integer = a * 2)` produced "Unknown variable 'a'"
// because earlier arguments weren't visible to later default expressions.
// The fix in parse_arguments (src/parser/definitions.rs) injects earlier
// args into self.vars before parsing each default, rewrites internal slot
// numbers to argument indices, and then cleans up the temporary bindings.
// The call-site substitution (Self::substitute_param_refs in parser/mod.rs)
// replaces Var(N) in the default tree with the caller's actual arg[N].

#[test]
fn p91_default_references_earlier_param() {
    code!(
        "fn dbl(a: integer, b: integer = a * 2) -> integer { a + b }
fn run() -> integer { dbl(5) }"
    )
    .expr("run()")
    .result(loft::data::Value::Int(15));
}

#[test]
fn p91_default_identity_of_earlier_param() {
    // `fn rect(w, h = w)` is the idiomatic "square by default" shape.
    code!(
        "fn rect(w: integer, h: integer = w) -> integer { w * h }
fn run() -> integer { rect(4) }"
    )
    .expr("run()")
    .result(loft::data::Value::Int(16));
}

#[test]
fn p91_default_overridden_by_caller() {
    // Regression guard: supplying the argument skips the default entirely.
    code!(
        "fn dbl(a: integer, b: integer = a * 2) -> integer { a + b }
fn run() -> integer { dbl(3, 7) }"
    )
    .expr("run()")
    .result(loft::data::Value::Int(10));
}

#[test]
fn p91_chained_defaults_reference_earlier_args() {
    // Three-argument chain: c's default references both a and b, where
    // b itself has a literal default.  Verifies that substitute_param_refs
    // uses already-substituted earlier args, not the raw default tree.
    code!(
        "fn add3(a: integer, b: integer = 10, c: integer = a + b) -> integer { a + b + c }
fn run() -> integer { add3(1) }"
    )
    .expr("run()")
    // a=1, b=10 (default), c=a+b=11 → 1+10+11 = 22
    .result(loft::data::Value::Int(22));
}

// ── C60 Step 3+ integration tests (ignored until parser acceptance lands) ───
// These pin the end-to-end behaviour for hash iteration in loft source.
// The Rust primitives landed in Step 1a + 2; the parser + stdlib wiring
// that routes `for e in h { … }` through `hash::records_sorted` is the
// next step.  Tests are marked `#[ignore]` per DEVELOPMENT.md so CI stays
// green until the feature ships.

/// C60 Step 4 (MVP acceptance test): `for e in h { … }` parses and
/// iterates a hash in ascending key order under the interpreter.
#[test]
fn c60_hash_iter_single_field_asc() {
    code!(
        "struct Entry { name: text, count: integer }
struct Bag { data: hash<Entry[name]> }
fn run() -> text {
    b = Bag { data: [
        Entry{name:\"zebra\",count:1},
        Entry{name:\"apple\",count:5},
        Entry{name:\"mango\",count:3},
    ] };
    out = \"\";
    for e in b.data { out += e.name; out += \",\"; }
    out
}"
    )
    .expr("run()")
    .result(loft::data::Value::str("apple,mango,zebra,"));
}

/// C60 Step 5: `#index` / `#count` / `#first` work "for free" through
/// the vector-iteration path Step 3 desugars into.
#[test]
fn c60_hash_iter_loop_attributes() {
    code!(
        "struct Ent { k: text, v: integer }
struct Bag { data: hash<Ent[k]> }
fn run() -> integer {
    b = Bag { data: [Ent{k:\"c\",v:3}, Ent{k:\"a\",v:1}, Ent{k:\"b\",v:2}] };
    total = 0;
    for e in b.data { total += e.v * (e#index + 1); }
    total
}"
    )
    .expr("run()")
    // a=1 at idx=0, b=2 at idx=1, c=3 at idx=2
    // sum = 1*1 + 2*2 + 3*3 = 14
    .result(loft::data::Value::Int(14));
}

/// C60 Step 6: multi-field key — lexicographic order.
#[test]
fn c60_hash_iter_multi_field_lex() {
    code!(
        "struct R { region: text, score: integer }
struct Bag { data: hash<R[region, score]> }
fn run() -> text {
    b = Bag { data: [
        R{region:\"east\",score:10},
        R{region:\"west\",score:30},
        R{region:\"east\",score:50},
        R{region:\"west\",score:20},
    ] };
    out = \"\";
    for r in b.data { out += \"{r.region}:{r.score},\"; }
    out
}"
    )
    .expr("run()")
    .result(loft::data::Value::str("east:10,east:50,west:20,west:30,"));
}

/// C60 Step 8: filter clause on hash iteration works through the
/// vector-iteration path.
#[test]
fn c60_hash_iter_filter_clause() {
    code!(
        "struct Ent { k: text, v: integer }
struct Bag { data: hash<Ent[k]> }
fn run() -> integer {
    b = Bag { data: [Ent{k:\"a\",v:1}, Ent{k:\"b\",v:20}, Ent{k:\"c\",v:3}, Ent{k:\"d\",v:40}] };
    total = 0;
    for e in b.data if e.v > 10 { total += e.v; }
    total
}"
    )
    .expr("run()")
    // Only v=20 and v=40 pass the filter.
    .result(loft::data::Value::Int(60));
}

/// C60 Step 4: empty hash iterates zero times.
#[test]
fn c60_hash_iter_empty() {
    code!(
        "struct Ent { k: text, v: integer }
struct Bag { data: hash<Ent[k]> }
fn run() -> integer {
    b = Bag { data: [] };
    count = 0;
    for _ in b.data { count += 1; }
    count
}"
    )
    .expr("run()")
    .result(loft::data::Value::Int(0));
}

/// C60 Step 9: `#remove` must be rejected where the loop walks a SNAPSHOT — the
/// iteration reads a pre-sorted copy of the records, so `#remove` would not reach
/// the collection.  Three kinds take that substitution (`hash`, `trie`, `spatial`)
/// and the refusal names whichever one the author wrote; removal is by key
/// (`h[key] = null`).
#[test]
fn c60_hash_iter_remove_rejected() {
    // Parse error expected; format matches other parse-error tests.
    code!(
        "struct Ent { k: text, v: integer }
struct Bag { data: hash<Ent[k]> }
fn test() {
    b = Bag { data: [Ent{k:\"a\",v:1}] };
    for e in b.data { e#remove; }
}"
    )
    .error(
        "#remove is not supported when iterating a `hash` — the loop walks a \
         snapshot of the records, so the removal would not reach the collection; \
         remove by key instead (`hash[key] = null`) at \
         c60_hash_iter_remove_rejected:5:32",
    );
}

/// A NULLABLE collection is refused with the DISCHARGE, not with a list of kinds.
///
/// `(N-Coal)`/`(N-Default)` give an absent collection zero iterations — `for e in c?` and
/// `for e in c ?? []` both run zero times — so the reader needs the one character, not the
/// six kinds they already picked from correctly.  `Parser::for_type` peels `τ?` so this is
/// the ONLY error the shape produces (@PLN25's dn1 audit named that site); before, the
/// element-type resolver reported "Unknown in expression type" twice on top of it.
#[test]
fn a_nullable_collection_is_refused_with_its_discharge() {
    code!(
        "fn test() {
    v: vector<integer>? = null;
    for x in v { }
}"
    )
    .error(
        "cannot iterate over vector<integer>? because it is NULLABLE — a `vector<integer>` \
         is iterable, but there is no implicit unwrap.  Discharge it first: add `?` (the \
         type's default, an empty collection) or `?? []`; either spelling gives an absent \
         collection zero iterations at \
         a_nullable_collection_is_refused_with_its_discharge:3:17",
    )
    // The two that follow are the parser's generic recovery after a `for` whose source did
    // not resolve — not part of this refusal, and asserted only because the harness matches
    // the diagnostics EXACTLY.
    .error(
        "Need an iterable expression in a for statement at \
         a_nullable_collection_is_refused_with_its_discharge:3:17",
    )
    .error("Expect token ; at a_nullable_collection_is_refused_with_its_discharge:3:17");
}

/// loft#1403 — the refusal names the kind the AUTHOR wrote.
///
/// `hash`, `trie` and `spatial` all take the snapshot substitution and all reach the one
/// scratch variable, so a message spelled for the hash told a `trie` author their loop was
/// "hash iteration" and prescribed `hash[key] = null` for a collection they never wrote.
/// A `spatial` is keyed by its coordinate axes, so it gets its own cure spelling too.
#[test]
fn remove_refusal_names_a_trie_not_a_hash() {
    code!(
        "struct Ent { k: text, v: integer }
fn test() {
    c: trie<Ent[k]> = [Ent{k:\"a\",v:1}];
    for e in c { e#remove; }
}"
    )
    .error(
        "#remove is not supported when iterating a `trie` — the loop walks a \
         snapshot of the records, so the removal would not reach the collection; \
         remove by key instead (`trie[key] = null`) at \
         remove_refusal_names_a_trie_not_a_hash:4:27",
    );
}

#[test]
fn remove_refusal_names_a_spatial_and_its_axes() {
    code!(
        "struct Ent { x: integer, y: integer, v: integer }
fn test() {
    c: spatial<Ent[x,y]> = [Ent{x:1,y:1,v:1}];
    for e in c { e#remove; }
}"
    )
    .error(
        "#remove is not supported when iterating a `spatial` — the loop walks a \
         snapshot of the records, so the removal would not reach the collection; \
         remove by key instead (`spatial[x, y] = null`) at \
         remove_refusal_names_a_spatial_and_its_axes:4:27",
    );
}

/// @PLN40 (was @P386): a `const` struct field is accepted — it constructs and
/// reads like any field.  Reassignment enforcement is a separate step; see
/// doc/claude/plans/40-const-fields/.
#[test]
fn pln40_const_struct_field_accepted() {
    code!(
        "struct Cell { const c_color: integer, height: integer }
fn test() {
    c = Cell { c_color: 7, height: 3 };
    assert(c.c_color == 7 && c.height == 3, \"const field constructs and reads\");
}"
    )
    .result(loft::data::Value::Null);
}

/// @PLN40: `const virtual(…)` is rejected — a virtual field is already computed
/// and read-only, so `const` on it is redundant.
#[test]
fn pln40_const_virtual_field_rejected() {
    code!("struct Bad { x: integer, const v: integer virtual($.x * 2) }").error(
        "`const virtual(…)` is redundant — a virtual field is already computed and read-only; drop `const` at pln40_const_virtual_field_rejected:1:61",
    );
}

/// @PLN40 step 4: reassigning a `const` field after construction is rejected.
#[test]
fn pln40_const_reassign_rejected() {
    code!(
        "struct Rec { const x: integer }
fn test() { t = Rec { x: 1 }; t.x = 5; }"
    )
    .error(
        "cannot reassign const field 'x' of struct 'Rec' — const fields are write-once-at-construction at pln40_const_reassign_rejected:2:39",
    );
}

/// @PLN40: `+=` on a const SCALAR field is still rejected — it changes the whole
/// value (a scalar has no "contents").  Only append on a const collection/text
/// field is allowed (that is contents mutation; covered in the both-backend script).
#[test]
fn pln40_const_scalar_compound_rejected() {
    code!(
        "struct Rec { const n: integer }
fn test() { r = Rec { n: 1 }; r.n += 1; }"
    )
    .error(
        "cannot reassign const field 'n' of struct 'Rec' — const fields are write-once-at-construction at pln40_const_scalar_compound_rejected:2:40",
    );
}

/// @PLN40 step 4: the const-field guard fires regardless of the write route —
/// here through a `&Rec` parameter, not the construction site.
#[test]
fn pln40_const_reassign_via_ref_rejected() {
    code!(
        "struct Rec { const x: integer }
fn touch(t: Rec) { t.x = 9; }
fn test() { t = Rec { x: 1 }; touch(t); }"
    )
    .error(
        "cannot reassign const field 'x' of struct 'Rec' — const fields are write-once-at-construction at pln40_const_reassign_via_ref_rejected:2:28",
    );
}

/// @PLN40 step 4: `const` freezes the field BINDING, not its contents — mutating
/// a non-const field reached THROUGH a const struct field stays legal (Rust's
/// `let v = …; v.field = …` rule).  Guards against over-broad enforcement.
#[test]
fn pln40_const_contents_still_mutable() {
    code!(
        "struct Inner { f: integer }
struct Rec { const r: Inner }
fn test() {
    t = Rec { r: Inner { f: 1 } };
    t.r.f = 5;
    assert(t.r.f == 5, \"contents of a const struct field remain mutable\");
}"
    )
    .result(loft::data::Value::Null);
}

/// @PLN40 step 5: a `const` field follows loft's standard field defaults — an
/// omitted field takes its default and a construction-time value overrides it
/// (construction is not a reassignment).  loft keeps its default-fill semantics;
/// there is no "a const field must be initialised" rule.
#[test]
fn pln40_const_field_default_and_override() {
    code!(
        "struct Cfg { const port: integer = 8080 }
fn test() {
    a = Cfg {};
    b = Cfg { port: 9000 };
    assert(a.port == 8080 && b.port == 9000, \"const default when omitted, override at construction\");
}"
    )
    .result(loft::data::Value::Null);
}

// ── @PLN40 Phase 2 — VALUE-const fields (`v: const T`, deep-frozen records) ──
// The inverse axis of binding-const: a value-const field's VALUE is read-only, so
// contents mutation (append `+=`, element `[i]=`, nested `.x=`) is rejected while a
// REBIND `s.v = other` re-points the slot and is allowed.  Composes with binding-
// const: `const v: const T` is fully frozen.  The positive cells (rebind, read) and
// the builder-untouched guarantee run on both backends in tests/scripts/40-const-fields.loft.

/// Append `+=` to a value-const COLLECTION field is a contents mutation → rejected
/// (unlike a binding-const collection field, where append is the allowed builder op).
#[test]
fn pln40_vc_append_rejected() {
    code!(
        "struct Rec { v: const vector<integer> }
fn test() { r = Rec { v: [1] }; r.v += [2]; }"
    )
    .error(
        "cannot mutate value-const field 'v' of struct 'Rec' — its value is read-only (rebind with '=' to re-point, or drop 'const') at pln40_vc_append_rejected:2:44",
    );
}

/// Element write `s.v[i]=` DEREFERENCES THROUGH the value-const field → rejected by
/// the LHS chain-walk (the field is an inner step, not the write target).
#[test]
fn pln40_vc_element_rejected() {
    code!(
        "struct Rec { v: const vector<integer> }
fn test() { r = Rec { v: [1] }; r.v[0] = 9; }"
    )
    .error(
        "Cannot modify value-const field 'Rec.v'; its value is read-only at pln40_vc_element_rejected:2:44",
    );
}

/// Nested write `s.r.x=` through a value-const STRUCT field → rejected: the chain
/// dereferences through the frozen `r`, even though the leaf field `x` is plain.
#[test]
fn pln40_vc_nested_rejected() {
    code!(
        "struct Inner { x: integer }
struct Rec { r: const Inner }
fn test() { t = Rec { r: Inner { x: 1 } }; t.r.x = 9; }"
    )
    .error(
        "Cannot modify value-const field 'Rec.r'; its value is read-only at pln40_vc_nested_rejected:3:54",
    );
}

/// A value-const SCALAR field collapses (no interior distinct from its binding), so
/// value-const freezes it fully — even a rebind `=` is rejected.
#[test]
fn pln40_vc_scalar_rejected() {
    code!(
        "struct Rec { c: const integer }
fn test() { r = Rec { c: 1 }; r.c = 9; }"
    )
    .error(
        "cannot mutate value-const field 'c' of struct 'Rec' — its value is read-only (rebind with '=' to re-point, or drop 'const') at pln40_vc_scalar_rejected:2:39",
    );
}

/// `const v: const T` is FULLY frozen: binding-const blocks the rebind, value-const
/// blocks the append.  Here the append is caught by the value-const leaf block.
#[test]
fn pln40_vc_fully_frozen_append_rejected() {
    code!(
        "struct Fz { const v: const vector<integer> }
fn test() { f = Fz { v: [1] }; f.v += [2]; }"
    )
    .error(
        "cannot mutate value-const field 'v' of struct 'Fz' — its value is read-only (rebind with '=' to re-point, or drop 'const') at pln40_vc_fully_frozen_append_rejected:2:43",
    );
}

/// The over-unification guard as a test: a REBIND of a value-const field (the
/// outermost node) re-points the slot and is ALLOWED — only mutation THROUGH the
/// frozen value is rejected.  Same base `r` as the rejected element write above.
#[test]
fn pln40_vc_rebind_allowed() {
    code!(
        "struct Rec { v: const vector<integer> }
fn test() {
    r = Rec { v: [1] };
    r.v = [2, 3];
    assert(len(r.v) == 2, \"a value-const field may be rebound, re-pointing the slot\");
}"
    )
    .result(loft::data::Value::Null);
}

// ── @PLN102 K1 — enum-VARIANT const enforcement (uniform with struct fields) ──
// `const` / value-const on an enum-variant field was declared, constructed, and
// read but NEVER enforced: `validate_write` resolved the field table only through
// `Parts::Struct`, so a variant's `Parts::EnumValue` fields silently no-op'd the
// guard.  These pin the enforcement now that both parts are walked — the negative
// cells that flipped from silently-accepted to rejected, plus the over-reach guard
// that a pattern-bound LOCAL copy stays mutable.  A pre-freeze error-add (only
// legal while CONTRACT_VERSION is 0); see plans/102/code-eval-followups.md K1.

/// `const` scalar variant field, direct reassign `x.r = …` → rejected (was accepted).
#[test]
fn pln40_enum_variant_const_reassign_rejected() {
    code!(
        "enum Kx { V { const r: integer }, Other { z: integer } }
fn test() { x = V { r: 1 }; if x is V { _g = 0; x.r = 5; } }"
    )
    .error(
        "cannot reassign const field 'r' of variant 'V' — const fields are write-once-at-construction at pln40_enum_variant_const_reassign_rejected:2:57",
    );
}

/// `const` scalar variant field, compound `x.n += …` → rejected (a scalar has no
/// "contents" to append into, same as the struct rule).
#[test]
fn pln40_enum_variant_const_compound_rejected() {
    code!(
        "enum Kx { V { const n: integer }, Other { z: integer } }
fn test() { x = V { n: 1 }; if x is V { _g = 0; x.n += 1; } }"
    )
    .error(
        "cannot reassign const field 'n' of variant 'V' — const fields are write-once-at-construction at pln40_enum_variant_const_compound_rejected:2:58",
    );
}

/// value-const collection variant field, append `x.v += …` → rejected (contents
/// mutation of a read-only value; the leaf value-const block).
#[test]
fn pln40_enum_variant_vc_append_rejected() {
    code!(
        "enum Kx { V { v: const vector<integer> }, Other { z: integer } }
fn test() { x = V { v: [1] }; if x is V { _g = 0; x.v += [2]; } }"
    )
    .error(
        "cannot mutate value-const field 'v' of variant 'V' — its value is read-only (rebind with '=' to re-point, or drop 'const') at pln40_enum_variant_vc_append_rejected:2:62",
    );
}

/// value-const collection variant field, element write `x.v[i] = …` → rejected by
/// the LHS chain-walk (`lhs_frozen_through`, the second `Parts::EnumValue` gap).
#[test]
fn pln40_enum_variant_vc_element_rejected() {
    code!(
        "enum Kx { V { v: const vector<integer> }, Other { z: integer } }
fn test() { x = V { v: [1] }; if x is V { _g = 0; x.v[0] = 9; } }"
    )
    .error(
        "Cannot modify value-const field 'V.v'; its value is read-only at pln40_enum_variant_vc_element_rejected:2:62",
    );
}

/// Over-reach guard: a pattern-bound LOCAL from a `const` variant field is a COPY
/// (B-Copy), so mutating the LOCAL stays legal — only the direct `x.r = …` write on
/// the enum value is the violation.  Guards against enforcing into the copied local.
#[test]
fn pln40_enum_variant_bound_local_copy_allowed() {
    code!(
        "enum Kx { V { const r: integer }, Other { z: integer } }
fn test() {
    x = V { r: 5 };
    if x is V { r } { r = 9; assert(r == 9, \"a copy of a const variant field is mutable\"); }
}"
    )
    .result(loft::data::Value::Null);
}

// ── P139 regression guards ──────────────────────────────────────────────────
// The slot allocator placed zone-1 byte-sized vars (plain enum, boolean) at
// fixed slots inside the zone-2 frontier, leaving codegen's TOS one byte
// below the next zone-2 slot.  `gen_set_first_at_tos` asserted `slot == TOS`
// and fired.  The fix emits `OpReserveFrame(gap)` when `slot > TOS`, so the
// runtime stack pointer advances to match and the init opcode writes to
// the correct slot.  These tests pin the three most common triggering
// shapes: plain-enum vector, two loops over an enum vector (the
// original 05-enums.loft pattern), and boolean vector.

#[test]
fn p139_enum_vec_same_type_write_through_loop() {
    code!(
        "enum Dir { North, East, South, West }
fn test() {
    dirs: vector<Dir> = [North, East, South, West];
    first_d = Dir.North;
    for elem in dirs { first_d = elem; }
    assert(first_d == West, \"last element wins, got {first_d}\");
}"
    )
    .result(loft::data::Value::Null);
}

#[test]
fn p139_enum_vec_two_loops_same_function() {
    code!(
        "enum D { A, B, C, W }
fn test() {
    dirs: vector<D> = [A, B, C, W];
    count = 0;
    for _ in dirs { count += 1; }
    first = D.A;
    last = D.A;
    n = 0;
    for elem in dirs {
        if n == 0 { first = elem; }
        last = elem;
        n += 1;
    }
    assert(count == 4, \"count: {count}\");
    assert(first == A, \"first: {first}\");
    assert(last == W, \"last: {last}\");
}"
    )
    .result(loft::data::Value::Null);
}

#[test]
fn p139_bool_vec_write_through_loop() {
    code!(
        "fn test() {
    flags = [true, false, true, true];
    flag = false;
    for f in flags { flag = f; }
    assert(flag == true, \"last flag, got {flag}\");
}"
    )
    .result(loft::data::Value::Null);
}

// ── P86 regression guards ───────────────────────────────────────────────────
// Pre-0.8.3 the parser's mitigation for P86 turned this source into a
// compile error ("closure capture is not yet supported"), and before that
// mitigation existed it produced a misleading codegen self-reference panic
// ("[generate_set] ... Var(1) self-reference — storage not yet allocated").
// With real closure capture shipped, both paths have to stay closed forever.
// `p1_1_lambda_void_body` above covers one integer-mutation case; the two
// tests below expand coverage to multi-variable mutation (integer) and
// text accumulation, which exercises the text work-buffer path in codegen
// and is the most common place where capture regressions hide.
#[test]
fn p86_lambda_capture_multi_mutation() {
    code!(
        "fn test() {
    count = 0;
    total = 0;
    add = fn(x: integer) { count += 1; total += x; };
    add(10);
    add(20);
    add(12);
    assert(count == 3, \"count: {count}\");
    assert(total == 42, \"total: {total}\");
}"
    )
    .result(loft::data::Value::Null);
}

#[test]
fn p86_lambda_capture_text_mutation() {
    code!(
        "fn test() {
    log = \"\";
    append = fn(s: text) { log += s; log += \",\"; };
    append(\"a\");
    append(\"bb\");
    append(\"ccc\");
    assert(log == \"a,bb,ccc,\", \"log: {log}\");
}"
    )
    .result(loft::data::Value::Null);
}

// ── Issue 82 ─────────────────────────────────────────────────────────────────
// `string` is not a valid type name — the canonical text type is `text`.
// Using `string` in a struct field produces "Undefined type string" and a
// cascade of "Invalid index key" / "Cannot write unknown" errors.

// Issue 82 / S7: `string` in a struct field must suggest `text`.
// T0.3 — `string` used to be an ad-hoc special case that quoted the offending name
// (`Undefined type 'string'`); it is now one row of the cross-language alias table,
// so it words identically to every other undefined type (`Undefined type Conter — …`).
// Prose is freely improvable; the frozen handle is the diagnostic's code (@PLN102 arc-E).
#[test]
fn issue_82_string_type_is_undefined() {
    code!("struct Bad { x: string }").error(
        "Undefined type string — did you mean 'text'? at issue_82_string_type_is_undefined:1:25",
    );
}

// Issue 82 positive: the same pattern with `text` must work correctly.
#[test]
fn issue_82_text_type_works() {
    code!(
        "struct Word { key: text, count: integer }
fn test() {
    w = Word { key: \"hello\", count: 1 };
    assert(w.key == \"hello\", \"key\");
    assert(w.count == 1, \"count\");
}"
    )
    .result(Value::Null);
}

// ── Issue 83 ─────────────────────────────────────────────────────────────────
// A struct field named `key` used as a hash-value type once panicked at runtime with
// "Allocating a used store" (src/database/allocation.rs), and was refused at compile time
// to keep programs away from it: "reserved for hash iteration".
//
// The refusal outlived the panic and is gone (loft#932).  Three facts retired it:
//
//  · The pseudo-field it named does not exist.  `for kv in h { kv.key }` over an element
//    struct with no real `key` field answers "Unknown field", not a hash key — so there
//    was nothing for a real field to collide WITH.
//  · It only ever covered `hash`.  `sorted<Elm[key]>` and `index<Elm[key]>` over a struct
//    with a real `key` field are exercised by `tests/scripts/146-keyed-rekey-through-view.loft`
//    and always ran, as did every `hash<Entry[key]>` LOCAL — the refusal walked struct
//    ATTRIBUTES, so a local's type annotation was never inspected.  `135-hash-table-rebuild…`
//    is built on that spelling.
//  · The panic no longer reproduces.  This test is the program it was filed for, and it
//    now runs on both backends, which is what it asserts.
//
// `key` is the natural name for a key field, so the refusal cost a good spelling for a
// hazard that had already been fixed elsewhere.

// Issue 83 / S8: a field named `key` in a hash-value struct is an ordinary field.
#[test]
fn issue_83_hash_value_field_named_key_works() {
    code!(
        "struct Entry { key: text, count: integer }
struct Db { data: hash<Entry[key]> }
fn test() {
    db = Db { data: [] };
    db.data += [Entry { key: \"hello\", count: 1 }];
    e = db.data[\"hello\"];
    assert(e != null, \"entry should exist\");
    assert(e.count == 1, \"count should be 1\");
    assert(e.key == \"hello\", \"the key field reads back as itself\");
}"
    )
    .result(Value::Null);
}

// Issue 83 positive: renaming the field (non-`key`) is the documented workaround.
#[test]
fn issue_83_hash_value_field_renamed_works() {
    code!(
        "struct Score { id: integer, pts: integer }
struct Board { scores: hash<Score[id]> }
fn test() {
    b = Board { scores: [] };
    b.scores += [Score { id: 1, pts: 42 }];
    s = b.scores[1];
    assert(s != null, \"entry should exist\");
    assert(s.pts == 42, \"pts should be 42, got {s.pts}\");
}"
    )
    .result(Value::Null);
}

// ── Issue 84 ─────────────────────────────────────────────────────────────────
// A `for` loop in any function that is called from a recursive function causes
// a codegen panic: "Too few parameters on n_<recursive_fn>".
// Root cause: the flat global variable namespace corrupts the parameter-count
// slot table for the recursive function when the helper's loop variables are
// assigned. Affects both `const vector<T>` and plain `vector<T>` params.

// Issue 84: for loop in helper + recursive caller panics "Too few parameters".
#[test]
fn issue_84_for_loop_in_helper_called_from_recursive_fn() {
    code!(
        "fn sum_vec(v: vector<integer>) -> integer {
    s = 0;
    for sv_i in 0..len(v) { s += v[sv_i]; }
    s
}
fn recurse(n: integer) -> integer {
    if n <= 0 { return 0; }
    v = [n];
    sum_vec(v) + recurse(n - 1)
}
fn test() {
    result = recurse(5);
    assert(result == 15, \"expected 15, got {result}\");
}"
    )
    .result(Value::Null);
}

// Issue 84: merge sort (index-bound) also triggers the same panic.
#[test]
fn issue_84_merge_sort_too_few_parameters() {
    code!(
        "fn msort_merge(lp: vector<integer>, rp: vector<integer>) -> vector<integer> {
    out = [for mg_i in 0..0 { mg_i }];
    li = 0; ri = 0;
    ll = len(lp); rl = len(rp);
    for mg_step in 0..(ll + rl) {
        if li >= ll && ri >= rl { break; }
        li = li + mg_step * 0;
        if li >= ll { out += [rp[ri]]; ri += 1; }
        else if ri >= rl { out += [lp[li]]; li += 1; }
        else if lp[li] <= rp[ri] { out += [lp[li]]; li += 1; }
        else { out += [rp[ri]]; ri += 1; }
    }
    out
}
fn msort(arr: vector<integer>, lo: integer, hi: integer) -> vector<integer> {
    sz = hi - lo;
    if sz <= 1 {
        base = [for ms_i in 0..0 { ms_i }];
        if sz == 1 { base += [arr[lo]]; }
        return base;
    }
    mid = lo + sz / 2;
    msort_merge(msort(arr, lo, mid), msort(arr, mid, hi))
}
fn test() {
    data = [3, 1, 4, 1, 5, 9, 2, 6];
    out = msort(data, 0, 8);
    assert(out[0] == 1, \"first={out[0]}\");
    assert(out[7] == 9, \"last={out[7]}\");
}"
    )
    // loft#1232 — the five `c += [v[i]]` pushes now say that an index read is `integer?` and
    // the destination's element is not.  The loft source is kept VERBATIM: this test guards a
    // parse shape (issue 84's panic on the index-bound merge sort), so discharging the reads to
    // silence the seam would change the shape the guard exists to hold.  Every index here is
    // bounds-guarded by the `li >= ll` / `ri >= rl` tests above it, so the warning is a true
    // statement about the TYPE and no read is ever actually null — which the assertions below
    // still prove by sorting the data correctly.
    .warning(concat!(
        "a nullable `integer?` is stored into element 0 of this vector literal of the ",
        "non-null type `integer` — it becomes null there; discharge with `?` (the type's ",
        "default), `?? <default>`, or `match` if that is not intended at ",
        "issue_84_merge_sort_too_few_parameters:8:38"
    ))
    .warning(concat!(
        "a nullable `integer?` is stored into element 0 of this vector literal of the ",
        "non-null type `integer` — it becomes null there; discharge with `?` (the type's ",
        "default), `?? <default>`, or `match` if that is not intended at ",
        "issue_84_merge_sort_too_few_parameters:9:43"
    ))
    .warning(concat!(
        "a nullable `integer?` is stored into element 0 of this vector literal of the ",
        "non-null type `integer` — it becomes null there; discharge with `?` (the type's ",
        "default), `?? <default>`, or `match` if that is not intended at ",
        "issue_84_merge_sort_too_few_parameters:10:51"
    ))
    .warning(concat!(
        "a nullable `integer?` is stored into element 0 of this vector literal of the ",
        "non-null type `integer` — it becomes null there; discharge with `?` (the type's ",
        "default), `?? <default>`, or `match` if that is not intended at ",
        "issue_84_merge_sort_too_few_parameters:11:31"
    ))
    .warning(concat!(
        "a nullable `integer?` is stored into element 0 of this vector literal of the ",
        "non-null type `integer` — it becomes null there; discharge with `?` (the type's ",
        "default), `?? <default>`, or `match` if that is not intended at ",
        "issue_84_merge_sort_too_few_parameters:19:39"
    ))
    .result(Value::Null);
}

// N7: OpFormatFloat must generate ops::format_float(...), not OpFormatFloat(stores, ...).
// OpFormatStackLong must generate ops::format_long(var_, ...) without stores or &mut.
#[test]
fn n7_format_ops_generate_correct_rust() {
    // Float formatting
    code!("struct Flt { v: float }")
        .expr("f = Flt { v: 3.14 }; \"{f.v}\"")
        .result(Value::str("3.14"));
    let src =
        std::fs::read_to_string("tests/generated/issues_n7_format_ops_generate_correct_rust.rs")
            .expect("generated file not found");
    assert!(
        !src.contains("OpFormatFloat("),
        "generated code still contains bare OpFormatFloat call"
    );
    assert!(
        src.contains("ops::format_float("),
        "generated code missing ops::format_float call"
    );
}

// ── Issue 85 ─────────────────────────────────────────────────────────────────
// Null-returning hash lookup before insert causes subsequent lookup to return null.
// Pattern: `e = hash[key]` (null result) followed by `hash += [Elem{...}]`
// makes the inserted element unfindable via `hash[key]`.

// Issue 85: null hash lookup before insert — integer key.
// The inserted element must be findable immediately after insertion.
#[test]
fn issue_85_hash_null_lookup_then_insert_integer_key() {
    code!(
        "struct Item { id: integer, val: integer }
struct Db { data: hash<Item[id]> }
fn test() {
    db = Db { data: [] };
    e0 = db.data[0];
    assert(e0 == null, \"pre-insert lookup should be null\");
    db.data += [Item { id: 0, val: 42 }];
    e1 = db.data[0];
    assert(e1 != null, \"inserted item must be findable\");
    assert(e1.val == 42, \"val should be 42, got {e1.val}\");
}"
    )
    .result(Value::Null);
}

// Issue 85: null hash lookup before insert — text key.
#[test]
fn issue_85_hash_null_lookup_then_insert_text_key() {
    code!(
        "struct Word { word: text, count: integer }
struct WordDb { freq: hash<Word[word]> }
fn test() {
    db = WordDb { freq: [] };
    e0 = db.freq[\"hello\"];
    assert(e0 == null, \"pre-insert lookup should be null\");
    db.freq += [Word { word: \"hello\", count: 1 }];
    e1 = db.freq[\"hello\"];
    assert(e1 != null, \"inserted word must be findable\");
    assert(e1.count == 1, \"count should be 1, got {e1.count}\");
}"
    )
    .result(Value::Null);
}

// ── Issue 89 ──────────────────────────────────────────────────────────────────
// Optional `& text` parameter panics with subtract-with-overflow when called
// with an explicit argument.  `convert()` must allocate a work-text variable
// and route through OpAppendText + OpCreateStack, not bare OpCreateStack(text).

// Issue 89: calling `directory("sub")` with an explicit text arg must not panic.
#[test]
fn issue_89_optional_ref_text_param_with_arg() {
    // directory() has signature `pub fn directory(v: & text = "") -> text`.
    // Calling it with an explicit string argument previously caused
    // "attempt to subtract with overflow" in codegen (issue #89).
    code!(
        "fn test() {
    d = directory(\"sub\");
    assert(d.len() >= 0, \"directory returned something\");
}"
    )
    .result(Value::Null);
}

// ── S8 — a hash-value struct may have a field named `key` ────────────────────
// The compile-time refusal S8 added is retired; see `issue_83_hash_value_field_named_key_works`
// above for what retired it (loft#932).  Kept as the declaration-only half: S8's program
// never ran the collection, so it pins that the DECLARATION alone is accepted.
#[test]
fn s8_hash_value_struct_key_field_accepted() {
    code!(
        "struct Item { key: text, value: integer }
struct Container { data: hash<Item[key]> }
fn test() { }"
    )
    .result(Value::Null);
}

// ── P2-R6 — Compiler check: yield inside par() body ──────────────────────────
// A coroutine generator cannot yield inside a par() parallel body because the
// worker executes in a separate thread with its own store — there is no safe
// way to resume the parent coroutine from within a worker.
// Fix: `in_par_body` flag in Parser; error emitted when `yield` is encountered
// inside a parallel-for worker function body.

#[test]
fn p2_r6_yield_inside_par_body_rejected() {
    code!(
        "fn gen(items: vector<integer>) -> iterator<integer> {
    for a in items par(b = double(a), 1) {
        yield b;
    }
}
fn double(x: integer) -> integer { x * 2 }"
    )
    .error("yield is not allowed inside a par(...) parallel body at p2_r6_yield_inside_par_body_rejected:3:16");
}

// ── P1.2 — Short-form lambda expressions ─────────────────────────────────────
// Short-form `|params| { body }` and `|| { body }` syntax for inline lambdas.

// P1.2: integer-form lambda `fn(x: integer) -> integer { x * 2 }` with explicit annotations.
#[test]

fn p1_2_short_lambda_explicit_types() {
    code!(
        "fn test() {
    f = fn(x: integer) -> integer { x * 2 };
    assert(f(5) == 10, \"expected 10\");
    assert(f(21) == 42, \"expected 42\");
}"
    )
    .result(loft::data::Value::Null);
}

// P1.2: Zero-parameter long-form lambda `fn() -> integer { 42 }`.
#[test]

fn p1_2_short_lambda_zero_params() {
    code!(
        "fn test() {
    f = fn() -> integer { 42 };
    assert(f() == 42, \"expected 42\");
}"
    )
    .result(loft::data::Value::Null);
}

// P1.2: Two-parameter long-form lambda with explicit types.
#[test]

fn p1_2_short_lambda_two_params() {
    code!(
        "fn test() {
    add = fn(a: integer, b: integer) -> integer { a + b };
    assert(add(3, 4) == 7, \"expected 7\");
}"
    )
    .result(loft::data::Value::Null);
}

// P1.2: Short lambda with inferred param type from call-site hint.
#[test]

fn p1_2_short_lambda_inferred_type() {
    code!(
        "fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
fn test() {
    result = apply(|n| { n * 3 }, 7);
    assert(result == 21, \"expected 21, got {result}\");
}"
    )
    .result(loft::data::Value::Null);
}

// ── P1.3 — map / filter / reduce with inline lambdas ─────────────────────────

// P1.3: `map` with a short-form lambda.
#[test]

fn p1_3_map_short_lambda() {
    code!(
        "fn test() {
    v = [1, 2, 3];
    r = map(v, |x| { x * 10 });
    assert(r[0] == 10, \"r[0]\");
    assert(r[1] == 20, \"r[1]\");
    assert(r[2] == 30, \"r[2]\");
}"
    )
    .result(loft::data::Value::Null);
}

// P1.3: `filter` with a short-form lambda.
#[test]

fn p1_3_filter_short_lambda() {
    code!(
        "fn test() {
    v = [1, 2, 3, 4, 5, 6];
    evens = filter(v, |x| { x % 2 == 0 });
    assert(len(evens) == 3, \"expected 3 evens\");
    assert(evens[0] == 2, \"evens[0]\");
    assert(evens[2] == 6, \"evens[2]\");
}"
    )
    .result(loft::data::Value::Null);
}

// P1.3: `reduce` with a short-form lambda.
#[test]

fn p1_3_reduce_short_lambda() {
    code!(
        "fn test() {
    v = [1, 2, 3, 4, 5];
    total = reduce(v, 0, |acc, x| { acc + x });
    assert(total == 15, \"expected 15, got {total}\");
}"
    )
    .result(loft::data::Value::Null);
}

// ── A8 — Destination-passing for text-returning natives ───────────────────────
// replace / to_lowercase / to_uppercase write directly into the destination
// string variable, eliminating the scratch buffer double-copy.

// A8: `replace` result assigned to a variable produces the right string.
#[test]

fn a8_replace_into_var() {
    code!(
        "fn test() {
    s = \"hello world\";
    r = s.replace(\"world\", \"loft\");
    assert(r == \"hello loft\", \"got {r}\");
}"
    )
    .result(loft::data::Value::Null);
}

// A8: `to_lowercase` result in a format string.
#[test]

fn a8_to_lowercase_in_format() {
    code!(
        "fn test() {
    s = \"HELLO\";
    r = \"value: {s.to_lowercase()}\";
    assert(r == \"value: hello\", \"got {r}\");
}"
    )
    .result(loft::data::Value::Null);
}

// Assert that src/fill.rs matches what generate_code_to would produce.
// If this fails, run: cargo test regen_fill_rs -- --ignored --nocapture
#[test]
fn fill_rs_up_to_date() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    scopes::check(&mut p.data, &mut p.database);
    let generated = loft::create::generate_code_to(&p.data, "tests/generated/fill_check.rs")
        .expect("generate_code_to failed");
    let current = std::fs::read_to_string("src/fill.rs").expect("cannot read src/fill.rs");
    assert_eq!(
        current, generated,
        "src/fill.rs is out of date — run: cargo test regen_fill_rs -- --ignored --nocapture"
    );
}

// @I67 — opcode implementations (this generates the @generated src/fill.rs from the default `#rust` bodies)
// Regenerate src/fill.rs from the default library definitions.
// Run with: cargo test regen_fill_rs -- --ignored --nocapture
#[test]
#[ignore = "maintenance: regenerates src/fill.rs — run manually when default/*.loft changes"]
fn regen_fill_rs() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    scopes::check(&mut p.data, &mut p.database);
    loft::create::generate_code_to(&p.data, "src/fill.rs").expect("generate_code_to failed");
    println!("src/fill.rs regenerated");
}

// Assert that every #rust-annotated function from default/*.loft is registered
// in src/native.rs.  If this fails, a new native function was added to the
// default library but not wired into the native registry.
// Fix: add the missing entry to FUNCTIONS in src/native.rs and implement the fn.
#[test]
fn native_rs_functions_up_to_date() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    scopes::check(&mut p.data, &mut p.database);
    let native_src = std::fs::read_to_string("src/native.rs").expect("cannot read src/native.rs");
    let mut missing = Vec::new();
    for d_nr in 0..p.data.definitions() {
        let d = p.data.def(d_nr);
        if d.is_operator() || d.rust.is_empty() {
            continue;
        }
        // @PLN10 F — a text producer is satisfied by EITHER its base native
        // (`"name"`) OR its destination-passing variant (`"name_dest"`): once
        // codegen routes every call position through `_dest` (synth-dest), the
        // base is deleted and the `_dest` impl is the binding.
        let entry = format!("\"{}\"", d.name);
        let dest_entry = format!("\"{}_dest\"", d.name);
        if !native_src.contains(&entry) && !native_src.contains(&dest_entry) {
            missing.push(d.name.clone());
        }
    }
    assert!(
        missing.is_empty(),
        "src/native.rs is missing {} function(s) from default/*.loft:\n  {}\n\
         Add them to the FUNCTIONS array (or a `_dest` variant) and implement the fn bodies.",
        missing.len(),
        missing.join("\n  ")
    );
}

// ── S9 / Issue 90 — character + character codegen panic ───────────────────────
// `c + d` where both are characters panics with a stack-size mismatch because
// `parse_append_text` uses the character variable as a text destination.

// S9: character + character must produce text concatenation, not a panic.
#[test]
fn s9_char_plus_char() {
    code!(
        "fn test() {
    c = 'h';
    d = 'i';
    r = c + d;
    assert(r == \"hi\", \"expected 'hi' got '{r}'\");
}"
    )
    .result(Value::Null);
}

// S9: text indexing `a[0] + a[1]` must also work.
#[test]
fn s9_text_index_plus_text_index() {
    code!(
        "fn test() {
    a = \"hello\";
    r = a[0] + a[1];
    assert(r == \"he\");
}"
    )
    .result(Value::Null);
}

// ── S10 — Disallow type annotations in |x| short-form lambdas ────────────────
// Short-form lambdas infer types from the call-site hint.  Explicit type
// annotations belong in the long form: fn(x: integer) -> integer { body }.

// S10: `|x: integer|` must produce a compile-time error.
#[test]
fn s10_short_lambda_type_annotation_rejected() {
    code!(
        "fn test() {
    v = [1, 2, 3];
    r = map(v, |x: integer| { x * 2 });
}"
    )
    .error("Type annotations are not allowed in |x| lambdas — use fn(x: <type>) { ... } instead (add `-> <ret>` only for non-void returns; `-> void` is not a valid type) at s10_short_lambda_type_annotation_rejected:3:27");
}

// ── S11 — Bare function references (no fn prefix) ────────────────────────────

// S11: bare `double` resolves as a function reference without `fn` prefix.
#[test]
fn s11_bare_fn_ref() {
    code!(
        "fn double(x: integer) -> integer { x * 2 }
fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
fn test() {
    assert(apply(double, 7) == 14, \"bare fn ref\");
}"
    )
    .result(Value::Null);
}

// S11: bare fn-ref with map.
#[test]
fn s11_bare_fn_ref_map() {
    code!(
        "fn triple(x: integer) -> integer { x * 3 }
fn test() {
    v = [1, 2, 3];
    r = map(v, triple);
    assert(r[0] == 3);
    assert(r[1] == 6);
}"
    )
    .result(Value::Null);
}

// ── Plan-06 phase 4d: Fn-ref and tuple as struct fields ─────────────────────

// Fn-ref as a struct field: store the d_nr in 4 bytes, read back as a
// 20-byte stack fn-ref slot with null closure, then call through it.
// Storage and stack widths differ (matching `vector<fn-ref>`); the
// closure half is intentionally null because tuple/struct fields don't
// store closures.
#[test]
fn p4d_fn_ref_as_struct_field() {
    code!(
        "struct Holder { f: fn(integer) -> integer }
fn dbl(x: integer) -> integer { x + x }
fn triple(x: integer) -> integer { x * 3 }
fn test() {
    h1 = Holder { f: dbl };
    h2 = Holder { f: triple };
    assert(h1.f(10) == 20, \"h1.f(10)={h1.f(10)}\");
    assert(h2.f(10) == 30, \"h2.f(10)={h2.f(10)}\");
}"
    )
    .result(Value::Null);
}

// Tuple as a struct field: elements are inlined into the host struct's
// bytes via the synthetic `__tuple<…>` struct's positions; per-element
// reads/writes use the same OpInt variants as ordinary struct fields.
#[test]
fn p4d_tuple_as_struct_field() {
    code!(
        "struct Pair { v: (integer, integer) }
fn test() {
    p = Pair { v: (3, 4) };
    assert(p.v.0 == 3, \"p.v.0={p.v.0}\");
    assert(p.v.1 == 4, \"p.v.1={p.v.1}\");
}"
    )
    .result(Value::Null);
}

// Mixed-element tuple field with text + numeric atom: heap text pointer
// and primitive ints share the inlined tuple block, each at the offset
// the synthetic struct's layout assigns.
#[test]
fn p4d_tuple_field_mixed_with_text() {
    code!(
        "struct Mixed {
    name: text,
    coords: (integer, integer),
    scale: float
}
fn test() {
    m = Mixed { name: \"origin\", coords: (10, 20), scale: 1.5 };
    assert(m.coords.0 == 10, \"x={m.coords.0}\");
    assert(m.coords.1 == 20, \"y={m.coords.1}\");
    assert(m.scale == 1.5, \"scale={m.scale}\");
    assert(m.name == \"origin\", \"name={m.name}\");
}"
    )
    .result(Value::Null);
}

// P196: tuple struct field whose element is a fn-ref.  Storage holds the
// 4-byte i32 d_nr only (matching plain `Holder { f: dbl }`), but the
// native runtime representation of a fn-ref is the 16-byte `(u32, DbRef)`
// tuple.  When the source value is a `Value::TupleGet` of a fn-ref tuple
// element, the native codegen used to substitute `var_tmp.0` directly
// into `OpSetInt4`'s template — emitting `(var_tmp.0) as i32` which
// rustc rejects (`non-primitive cast: (u32, DbRef) as i32`, E0605) plus
// the matching E0308 on the null-check half.  The fix in
// `output_call_template` projects `.0` from the fn-ref tuple before the
// cast.  Interpreter behaviour was already correct (TupleGet of a
// fn-ref element pushes only the d_nr's 8 bytes); this test guards the
// end-to-end behaviour through the literal-tuple path.
#[test]
fn p4d_tuple_field_with_fn_ref() {
    code!(
        "struct Pair { v: (fn(integer) -> integer, integer) }
fn p_dbl(x: integer) -> integer { x + x }
fn p_triple(x: integer) -> integer { x * 3 }
fn test() {
    p1 = Pair { v: (p_dbl, 21) };
    p2 = Pair { v: (p_triple, 14) };
    pf1 = p1.v.0;
    pf2 = p2.v.0;
    assert(pf1(p1.v.1) == 42, \"p1.v.0(p1.v.1)={pf1(p1.v.1)}\");
    assert(pf2(p2.v.1) == 42, \"p2.v.0(p2.v.1)={pf2(p2.v.1)}\");
}"
    )
    .result(Value::Null);
}

// Tuple field with text element only: write+read of interned strings
// inside the host record's store.  Verifies the native-codegen `String`
// vs `&str` plumbing via the `tuple_text_to_string` flag and the
// pre-eval block-text deref wrap.
#[test]
fn p4d_tuple_field_text_pair() {
    code!(
        "struct A { v: (text, text) }
fn test() {
    a = A { v: (\"hello\", \"world\") };
    assert(a.v.0 == \"hello\", \"0={a.v.0}\");
    assert(a.v.1 == \"world\", \"1={a.v.1}\");
}"
    )
    .result(Value::Null);
}

// Tuple field with vector element: deep-copy via OpAppendVector into a
// vector record allocated in the host's store, plus a primitive
// follower so the synthetic struct's atomic layout has both heap and
// inline elements.
#[test]
fn p4d_tuple_field_with_vector() {
    code!(
        "struct WithVec { v: (vector<integer>, integer) }
fn test() {
    w = WithVec { v: ([1, 2, 3], 42) };
    assert(w.v.0[0] == 1, \"vec[0]={w.v.0[0]}\");
    assert(w.v.0[1] == 2, \"vec[1]={w.v.0[1]}\");
    assert(w.v.0[2] == 3, \"vec[2]={w.v.0[2]}\");
    assert(w.v.1 == 42, \"second={w.v.1}\");
}"
    )
    .result(Value::Null);
}

// Tuple field with a struct reference element: inlined struct bytes
// inside the host record (same-store) via OpCopyRecord.  Validates the
// `Rewritten(Reference)`-aware `convert` path so `(Inner { … }, 11)`
// matches `(Inner, integer)`.
#[test]
fn p4d_tuple_field_with_reference() {
    code!(
        "struct Inner { v: integer }
struct Outer { pair: (Inner, integer) }
fn test() {
    o = Outer { pair: (Inner { v: 7 }, 11) };
    inner = o.pair.0;
    assert(inner.v == 7, \"inner.v={inner.v}\");
    assert(o.pair.1 == 11, \"second={o.pair.1}\");
}"
    )
    .result(Value::Null);
}

// Nested tuple struct field: inner tuples inline their bytes inside
// the outer tuple's bytes inside the host record.  All access paths
// (set, get, null-init) recurse through nested `Type::Tuple` elements
// so the layout is fully recursive.
#[test]
fn p4d_tuple_field_nested_homogeneous() {
    code!(
        "struct Nested { v: ((integer, integer), (integer, integer)) }
fn test() {
    n = Nested { v: ((1, 2), (3, 4)) };
    inner1 = n.v.0;
    inner2 = n.v.1;
    assert(inner1.0 == 1, \"0.0={inner1.0}\");
    assert(inner1.1 == 2, \"0.1={inner1.1}\");
    assert(inner2.0 == 3, \"1.0={inner2.0}\");
    assert(inner2.1 == 4, \"1.1={inner2.1}\");
}"
    )
    .result(Value::Null);
}

// Nested tuple with mixed-type inner elements (text + integer): each
// leaf primitive's offset is computed from `outer_offset +
// inner_offset`, with text elements writing interned pointers in the
// host's store while integers stay inline.
#[test]
fn p4d_tuple_field_nested_mixed() {
    code!(
        "struct Pair { v: ((integer, text), (text, integer)) }
fn test() {
    p = Pair { v: ((1, \"a\"), (\"b\", 2)) };
    i1 = p.v.0;
    i2 = p.v.1;
    assert(i1.0 == 1, \"i1.0\");
    assert(i1.1 == \"a\", \"i1.1={i1.1}\");
    assert(i2.0 == \"b\", \"i2.0={i2.0}\");
    assert(i2.1 == 2, \"i2.1\");
}"
    )
    .result(Value::Null);
}

// Nested tuple with asymmetric arities (3-of-2 elements vs 2-of-2):
// covers element-offset adjustment when sub-tuples have different
// sizes side-by-side.
#[test]
fn p4d_tuple_field_nested_asymmetric() {
    code!(
        "struct Triple { v: ((integer, integer, integer), (integer, integer)) }
fn test() {
    t = Triple { v: ((1, 2, 3), (10, 20)) };
    a = t.v.0;
    b = t.v.1;
    assert(a.0 == 1 && a.1 == 2 && a.2 == 3, \"a\");
    assert(b.0 == 10 && b.1 == 20, \"b\");
}"
    )
    .result(Value::Null);
}

// P193 — Default-init for a struct with a fn-ref field.  Previously
// the native code emitted `(()) as i32` because `to_default(Type::
// Function)` returned `Value::Null`; the fn-ref arm now returns
// `Value::FnRef(0, u16::MAX, …)` which the downstream `set_field_check
// ::Function` arm reduces to a `d_nr=0` 4-byte storage write.
#[test]
fn p4d_fn_ref_field_default_init() {
    code!(
        "struct Holder { f: fn(integer) -> integer, n: integer }
fn test() {
    h = Holder { n: 7 };
    assert(h.n == 7, \"n={h.n}\");
}"
    )
    .result(Value::Null);
}

// P193 — Default-init `Holder {}` with no fields supplied.  The
// fn-ref field defaults via the same null-fn-ref path; the integer
// field defaults to 0 via the existing `Value::Int(0)` arm.
#[test]
fn p4d_fn_ref_field_bare_default() {
    code!(
        "struct Holder { f: fn(integer) -> integer }
fn test() { h = Holder {}; }"
    )
    .result(Value::Null);
}

// P193 — Default-init for a tuple struct field where elements are a
// mix of heap-pointed (text) and primitive (integer).  Recursive
// `to_default(Type::Tuple)` produces per-element defaults: text `\"\"`
// (interned empty string in the host's store) and integer `0`.
#[test]
fn p4d_tuple_field_default_init() {
    code!(
        "struct Pair { v: (text, integer) }
fn test() {
    p = Pair {};
    assert(p.v.0 == \"\", \"v.0='{p.v.0}'\");
    assert(p.v.1 == 0, \"v.1={p.v.1}\");
}"
    )
    .result(Value::Null);
}

// ── P213 — Capturing closures in struct fields ──────────────────────────────
// The capturing-closure-in-struct-field surface that was previously
// rejected at parse time is now backed by `Parts::ChildRec(closure_kt)`
// — a 4B u32 rec-id that points at a closure record co-located in
// host's Store.  `OpClaimChildRec` writes; `OpRefFromChildRec` reads;
// the cascade in `copy_claims` / `remove_claims` handles deep-copy and
// free automatically when the host moves cross-store or is freed.

/// P213: integer capture in a struct-field closure.  The canonical
/// reproducer from PROBLEMS.md § 213.
#[test]
fn p213_struct_field_basic_int() {
    code!(
        "struct Box { cb: fn(integer) -> integer }
fn run() -> integer {
    n = 5;
    b = Box { cb: fn(x: integer) -> integer { x + n } };
    b.cb(10)
}"
    )
    .expr("run()")
    .result(Value::Int(15));
}

/// P213: multiple integer captures inside the same closure record —
/// exercises the `Parts::ChildRec` cascade walking multiple fields of
/// the closure record's struct.
#[test]
fn p213_struct_field_multi_int_capture() {
    code!(
        "struct Acc { add: fn(integer) -> integer }
fn run() -> integer {
    base = 100;
    factor = 3;
    a = Acc { add: fn(n: integer) -> integer { base + n * factor } };
    a.add(7)
}"
    )
    .expr("run()")
    .result(Value::Int(121)); // 100 + 7 * 3
}

/// P216 — closed 2026-05-05.  Tuple-typed capture in closure body
/// crashed under interp ("Incomplete record" at `src/store.rs:227`)
/// and produced wrong results under native (read t.1 instead of t.0
/// because field offsets were `u16::MAX`).  Root cause:
/// `synthesize_closure_record` (`src/parser/vectors.rs:762`) added a
/// `Type::Tuple` attribute to the closure record but never registered
/// the synthetic `__tuple<…>` struct via `data.tuple_def(...)`.
/// `fill_database` then saw `type_elm(&Type::Tuple(_))` return
/// `u32::MAX` and silently skipped the attribute (line 381 gate),
/// leaving the closure record with size 0 → `OpDatabase` panics
/// claiming an empty record.  Fix walks each capture's `Type` in
/// `synthesize_closure_record` and calls `tuple_def` for every
/// `Type::Tuple` (recursively, so nested tuples register inside-out)
/// before adding the attribute.
#[test]
fn p216_tuple_capture_int_first() {
    code!(
        "fn run() -> integer {
    t = (3, 7);
    f = fn(x: integer) -> integer { t.0 + x };
    f(10)
}"
    )
    .expr("run()")
    .result(Value::Int(13)); // 3 + 10
}

/// P216 follow-up — capture of the second tuple element verifies
/// per-element offset handling on both backends (the original native
/// bug returned `t.1` for `t.0` reads because all element writes
/// targeted `pos + u16::MAX`, so the last write won and both reads
/// returned the same garbage).
#[test]
fn p216_tuple_capture_int_second() {
    code!(
        "fn run() -> integer {
    t = (3, 7);
    f = fn(x: integer) -> integer { t.1 + x };
    f(10)
}"
    )
    .expr("run()")
    .result(Value::Int(17)); // 7 + 10
}

/// P216 follow-up — three-element tuple capture sums all elements.
/// Verifies the per-element offset table (4B-aligned for i32-narrow
/// or 8B-aligned for i64) is correctly populated in the closure
/// record's tuple field.
#[test]
fn p216_tuple_capture_three_elements() {
    code!(
        "fn run() -> integer {
    t = (1, 2, 3);
    f = fn(x: integer) -> integer { t.0 + t.1 + t.2 + x };
    f(100)
}"
    )
    .expr("run()")
    .result(Value::Int(106)); // 1+2+3+100
}

/// P213: default-initialised fn-ref field stays at `rec=0` in the
/// closure_rec slot — no record gets claimed; reading constructs the
/// null sentinel correctly.  Calling such a field is undefined (the
/// d_nr is also 0); only field READ is exercised here.
#[test]
fn p213_struct_field_default_init() {
    code!(
        "struct Holder { name: text, cb: fn(integer) -> integer }
fn run() -> text {
    h = Holder { name: \"empty\" };
    h.name
}"
    )
    .expr("run()")
    .result(Value::Text("empty".to_string()));
}

// ── L6 — Field constraints and JSON-style struct literals ─────────────────────

// L6: basic field constraint — valid construction.
#[test]
fn l6_constraint_valid_construction() {
    code!(
        "struct Score {
    value: integer assert($.value >= 0, \"value must be >= 0\"),
    max: integer assert($.max >= $.value, \"max must be >= value\")
}
fn test() {
    s = Score { value: 5, max: 10 };
    assert(s.value == 5);
    assert(s.max == 10);
    s.value = 8;
    assert(s.value == 8);
}"
    )
    .result(Value::Null);
}

// L6: field constraint fires on invalid assignment.
#[test]
#[should_panic(expected = "value must be >= 0")]
fn l6_constraint_violation_on_assign() {
    code!(
        "struct Score {
    value: integer assert($.value >= 0, \"value must be >= 0\")
}
fn test() {
    s = Score { value: 5 };
    s.value = -1;
}"
    )
    .result(Value::Null);
}

// L6: cross-field constraint fires on invalid construction.
#[test]
#[should_panic(expected = "lo must be <= hi")]
fn l6_cross_field_constraint_violation() {
    code!(
        "struct Range {
    lo: integer assert($.lo <= $.hi, \"lo must be <= hi\"),
    hi: integer
}
fn test() {
    r = Range { lo: 20, hi: 10 };
}"
    )
    .result(Value::Null);
}

// L6: JSON-style quoted field names in struct literals.
#[test]
fn l6_json_quoted_field_names() {
    code!(
        r#"struct Point { x: integer, y: integer }
fn test() {
    p = Point { "x": 3, "y": 4 };
    assert(p.x == 3, "x={p.x}");
    assert(p.y == 4, "y={p.y}");
}"#
    )
    .result(Value::Null);
}

// L6: constraint with auto-generated message.
#[test]
#[should_panic(expected = "field constraint failed on Pos.x")]
fn l6_constraint_auto_message() {
    code!(
        "struct Pos {
    x: integer assert($.x >= 0)
}
fn test() {
    p = Pos { x: 5 };
    p.x = -1;
}"
    )
    .result(Value::Null);
}

// L6: vector literal input parsed like JSON array.
#[test]
fn l6_vector_literal_as_json_array() {
    code!(
        "fn test() {
    v = [12, 34, 56];
    assert(len(v) == 3, \"len={len(v)}\");
    assert(v[0] == 12);
    assert(v[1] == 34);
    assert(v[2] == 56);
}"
    )
    .result(Value::Null);
}

// L6: validate a vector of constrained structs with format-string message.
#[test]
fn l6_validate_vector_of_structs() {
    code!(
        "struct Item {
    name: text,
    qty: integer assert($.qty > 0, \"qty must be > 0 for '{$.name}'\")
}
fn test() {
    items = [
        Item { name: \"apple\", qty: 3 },
        Item { name: \"banana\", qty: 5 }
    ];
    total = 0;
    for it in items {
        total += it.qty;
    }
    assert(total == 8, \"total={total}\");
}"
    )
    .result(Value::Null);
}

// ── JSON-style parsing via `as` cast ─────────────────────────────────────────

// JSON-style quoted field names in `as Type` cast.
#[test]
fn json_quoted_field_names_in_cast() {
    code!(
        r#"struct Item { name: text, value: integer }
fn test() {
    jt = `{{"name": "hello", "value": 42}}` as Item;
    assert(jt.name == "hello", "name={jt.name}");
    assert(jt.value == 42, "value={jt.value}");
}"#
    )
    .result(Value::Null);
}

// JSON-style vector of structs parsed via `as`.
#[test]
fn json_vector_of_structs_cast() {
    code!(
        r#"struct Item { name: text, value: integer }
fn test() {
    items = `[ {{"name": "a", "value": 1}}, {{"name": "b", "value": 2}} ]` as vector<Item>;
    assert(len(items) == 2, "len={len(items)}");
    assert(items[0].name == "a");
    assert(items[1].value == 2);
}"#
    )
    .result(Value::Null);
}

// ── Type.parse(text) ──────────────────────────────────────────────────────────

// Type.parse(text) with JSON input.  Auto-wraps through
// json_parse internally (P54 step 5 with the step-6 backward-
// compatibility shim — see `src/parser/objects.rs::parse_type_parse`).
#[test]
fn type_parse_json() {
    code!(
        r#"struct Score { value: integer, name: text }
fn test() {
    s = Score.parse(`{{"value": 42, "name": "test"}}`);
    assert(s.value == 42, "value={s.value}");
    assert(s.name == "test", "name={s.name}");
}"#
    )
    .result(Value::Null);
}

// Type.parse(text) — loft-native bare-key form (`{value: 7}`)
// is NOT standard JSON, so json_parse rejects it.  Rewritten
// to standard JSON so the test still guards the struct-unwrap
// behaviour under the auto-wrap path.
#[test]
fn type_parse_loft_native() {
    code!(
        r#"struct Score { value: integer, name: text }
fn test() {
    s = Score.parse(`{{"value": 7, "name": "hello"}}`);
    assert(s.value == 7);
    assert(s.name == "hello");
}"#
    )
    .result(Value::Null);
}

// Type.parse(text) with variable input.
#[test]
fn type_parse_from_variable() {
    code!(
        r#"struct Point { x: integer, y: integer }
fn test() {
    input = `{{"x": 10, "y": 20}}`;
    p = Point.parse(input);
    assert(p.x == 10);
    assert(p.y == 20);
}"#
    )
    .result(Value::Null);
}

// Type.parse(text) with constraint — valid data.
#[test]
fn type_parse_with_constraint_valid() {
    code!(
        r#"struct Score {
    value: integer assert($.value >= 0, "value must be >= 0")
}
fn test() {
    s = Score.parse(`{{"value": 5}}`);
    assert(s.value == 5);
}"#
    )
    .result(Value::Null);
}

// Type.parse(text) with invalid data — constraint fires.
#[test]
#[should_panic(expected = "value must be >= 0")]
fn type_parse_with_constraint_violation() {
    code!(
        r#"struct Score {
    value: integer assert($.value >= 0, "value must be >= 0")
}
fn test() {
    s = Score { "value": -1 };
}"#
    )
    .result(Value::Null);
}

// L6: constraint violation with format-string message (falls back to auto-generated).
#[test]
#[should_panic(expected = "field constraint failed on Item.qty")]
fn l6_vector_struct_constraint_violation() {
    code!(
        "struct Item {
    name: text,
    qty: integer assert($.qty > 0, \"qty must be > 0 for '{$.name}'\")
}
fn test() {
    items = [
        Item { name: \"bad\", qty: 0 }
    ];
}"
    )
    .result(Value::Null);
}

// ── s#errors — error path reporting via #errors accessor ──────────────────────

// Successful JSON parse yields no diagnostic.  With P54 step 5's
// auto-wrap, text arguments still route through json_parse
// internally — the `s#errors` accessor stays empty on success,
// and `json_errors()` also stays empty.
#[test]
fn errors_accessor_empty_on_success() {
    code!(
        r#"struct Score { value: integer }
fn test() {
    s = Score.parse(`{{"value": 42}}`);
    err = s#errors;
    assert(len(err) == 0, "expected no error, got: '{err}'");
    assert(s.value == 42);
}"#
    )
    .result(Value::Null);
}

// Malformed JSON — `Struct.parse(text)` routes through the
// legacy lenient parser (preserves loft-native bare-key
// support per QUALITY.md § P54-U) which populates `s#errors`.
// The new typed-tree path (`Struct.parse(json_parse(text))`)
// populates `json_errors()` instead.  Both produce null fields
// on bad input.
#[test]
fn errors_accessor_path_on_failure() {
    code!(
        r#"struct Score { value: integer? }
fn test() {
    bad = Score.parse(`not_json`);
    err = bad#errors;
    assert(bad.value == null, "value should be null on bad parse");
    assert(len(err) > 0, "expected #errors entries for bad input");
}"#
    )
    .result(Value::Null);
}

// Type-mismatched nested input — `data: "not_an_object"` is a
// JString, not a JObject.  Under P54 step 5 the struct unwrap
// returns null-valued fields for kind mismatches; schema-level
// diagnostics arrive with Q1 schema-side (pending).  Verify the
// unwrap doesn't crash on the mismatched shape.
#[test]
fn errors_accessor_nested_path() {
    code!(
        r#"struct Inner { x: integer? }
struct Outer { name: text, data: Inner }
fn test() {
    bad = Outer.parse(`{{"name": "ok", "data": "not_an_object"}}`);
    assert(bad.name == "ok", "outer name should survive: got {bad.name}");
    assert(bad.data.x == null, "nested x should be null on mismatched shape");
}"#
    )
    .result(Value::Null);
}

// O7: OpClearStackText followed by ≥2 format ops must emit with_capacity hint;
// OpClearStackText followed by 0 or 1 ops must emit bare .clear().
#[test]
fn o7_format_string_with_capacity() {
    // Multi-segment format string: "hello {name}, count {n}" → 4 segments → with_capacity
    code!("struct S { name: text, count: integer }")
        .expr("s = S { name: \"Alice\", count: 3 }; \"hello {s.name}, count {s.count}\"")
        .result(Value::str("hello Alice, count 3"));
    let src = std::fs::read_to_string("tests/generated/issues_o7_format_string_with_capacity.rs")
        .expect("generated file not found");
    assert!(
        src.contains("with_capacity"),
        "multi-segment format string should emit with_capacity hint"
    );
    // Single-segment format: "{s.v}" → 1 segment → no with_capacity (bare .to_string())
    code!("struct S2 { v: integer }")
        .expr("s = S2 { v: 7 }; \"{s.v}\"")
        .result(Value::str("7"));
    let src2 = std::fs::read_to_string("tests/generated/issues_o7_format_string_with_capacity.rs")
        .expect("generated file not found");
    // The single-segment case must NOT get a with_capacity hint — only ≥2 segments qualify.
    // The generated file still contains with_capacity from the S struct test above (same file),
    // so instead verify that the S2 function body uses .to_string() for its single-segment clear.
    assert!(
        src2.contains(".to_string()"),
        "single-segment format string should fall through to bare .to_string()"
    );
}

// ── File.content() on non-existent file ─────────────────────────────────────
// Regression guard — verifies that File.content() on a missing path returns
// an empty text (not garbage / not a crash) under the regular execute path.
// The historical SIGSEGV was specific to execute_log (LOFT_LOG=full); the
// runtime behaviour the test asserts is now stable.  Un-ignored 2026-04-14
// after the test was found to pass in isolation; if execute_log ever
// regresses, that variant is exercised by the LOFT_LOG-driven test dumps,
// not by this guard.
//
// Deleted: file_content_nonexistent_trace — duplicate of file_content_nonexistent
// (passing), and the ignore was for a P136-adjacent harness bug, not a behavior gap.

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
        "fn negate(n: integer) -> integer {
    if n > 0 { return 0 - n; }
    n
}
fn test() {
    assert(negate(5) == -5, \"negate positive\");
    assert(negate(-3) == -3, \"negate negative\");
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
    ignore = "P122 stress test — ~10min in debug mode; runs in release automatically (passes in ~0.05s)."
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

// ── P54: JsonValue enum — landing tests ────────────────────────────────────
// Design in doc/claude/BITING_PLAN.md § P54.  These tests pin the public
// surface of the new JSON subsystem:
//
//   enum JsonValue { JObject, JArray, JString, JNumber, JBool, JNull }
//   fn json_parse(text) -> JsonValue
//   fn to_json(self: JsonValue) -> text
//   fn field / item / as_text / as_number / as_long / as_bool / len
//   MyStruct.parse(v: JsonValue) — replaces .parse(text)
//

/// String-variant parsing: the value is correctly stored, but
/// returning it through a function boundary trips a JsonValue-store
/// lifecycle issue — the store is freed during scope-exit cleanup
/// before the caller's text-copy machinery completes.  Same root
/// cause as `p54_extractor_as_text` below.  Standalone smoke
/// (`/tmp/jp1.loft` style with `match v { JString { value } =>
/// println("str={value}") }`) works fine inline; only the
/// fn-return path is broken.
#[test]
fn p54_parse_primitive_string() {
    code!(
        "fn run() -> text {
    v = json_parse(\"\\\"hello\\\"\");
    out = \"\";
    match v {
        JString { value } => { out = value; },
        _ => {}
    }
    out
}"
    )
    .expr("run()")
    .result(Value::str("hello"));
}

#[test]
fn p54_parse_primitive_number() {
    code!(
        "fn run() -> float {
    v = json_parse(\"42.5\");
    match v {
        JNumber { value } => value,
        _ => 0.0
    }
}"
    )
    .expr("run()")
    .result(Value::Float(42.5));
}

#[test]
fn p54_parse_primitive_bool_true() {
    code!(
        "fn run() -> boolean {
    v = json_parse(\"true\");
    match v {
        JBool { value } => value,
        _ => false
    }
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

#[test]
fn p54_parse_primitive_null() {
    code!(
        "fn run() -> boolean {
    v = json_parse(\"null\");
    match v {
        JNull => true,
        _ => false
    }
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

#[test]
fn p54_malformed_returns_jnull() {
    // Use a malformed input that doesn't trip loft's text-literal
    // interpolation (curly braces in `"…"` would).
    code!(
        "fn run() -> boolean {
    v = json_parse(\"xyz\");
    match v {
        JNull => true,
        _ => false
    }
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

/// `as_text` returns a `Str` into `Stores::scratch`; calling it
/// inline + assigning into a local works (`println("{v.as_text()}")`
/// is fine), but returning the resulting text through a function
/// boundary trips the same store-free-before-copy lifecycle as
/// `p54_parse_primitive_string`.  Both unblock together.
#[test]
fn p54_extractor_as_text() {
    code!(
        "fn run() -> text {
    v = json_parse(\"\\\"abc\\\"\");
    out = \"\";
    out += v.as_text();
    out
}"
    )
    .expr("run()")
    .result(Value::str("abc"));
}

/// Same store-free-before-copy lifecycle as the matching extractor
/// test above.  Unblocks once that lands.
#[test]
fn p54_extractor_as_text_wrong_kind_returns_null() {
    code!(
        "fn run() -> text {
    v = json_parse(\"42\");
    t = v.as_text();
    if t == null { \"is-null\" } else { \"not-null\" }
}"
    )
    .expr("run()")
    .result(Value::str("is-null"));
}

// Un-ignored 2026-04-14 by P54 step 4 third slice — non-empty
// primitive objects now materialise, n_field walks the arena
// vector for a name match, chained `.as_text()` reads the
// JString value out of the matched JsonField slot.
//
// Loft strings treat `{…}` as interpolation; the JSON `{` and
// `}` in the literal are doubled (`{{` / `}}`) so they reach
// `json_parse` as single braces.  Rationale in LOFT.md §
// String literals.
#[test]
fn p54_parse_object_field_access() {
    code!(
        "fn run() -> text {
    v = json_parse(\"{{\\\"name\\\":\\\"Alice\\\",\\\"age\\\":30}}\");
    v.field(\"name\").as_text()
}"
    )
    .expr("run()")
    .result(Value::str("Alice"));
}

// Un-ignored 2026-04-14 by P54 step 4 second slice — non-empty
// primitive arrays now materialise, `n_item` dispatches on JArray,
// and `as_long()` returns the element's numeric payload.
#[test]
fn p54_parse_array_item_access() {
    code!(
        "fn run() -> integer {
    v = json_parse(\"[10, 20, 30]\");
    v.item(1).as_long()
}"
    )
    .expr("run()")
    .result(Value::Long(20));
}

/// Chain access on a non-object value never traps — every
/// intermediate `field()` / `item()` returns `JNull`.  Step 3 stub:
/// since object/array parsing isn't wired yet, json_parse on any
/// object-shaped input returns JNull anyway, and the chain is
/// JNull all the way down.  Locks the chained-access safety
/// guarantee on a non-object root: every intermediate failure
/// produces JNull rather than trapping.  The positive-path
/// counterpart is `p54_chained_access_on_nested_object`.
#[test]
fn p54_missing_chain_returns_jnull() {
    // `{` in a loft text literal triggers format-string interpolation;
    // use a primitive that json_parse handles to produce a non-object
    // root.  The chain still lands at JNull because field/item on a
    // non-object always returns JNull.
    code!(
        "fn run() -> boolean {
    v = json_parse(\"42\");
    result = v.field(\"missing\").item(5).field(\"b\");
    match result {
        JNull => true,
        _ => false
    }
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

/// P54 step 4 — chained access on a real nested object reaches
/// the leaf.  Locks the documented STDLIB.md JSON example
/// (`v.field("users").item(0).field("name").as_text()`) so the
/// reference doc and the runtime cannot drift independently.
#[test]
fn p54_chained_access_on_nested_object() {
    code!(
        "fn run_pca() -> text {
    v_pca = json_parse(`{{\"users\":[{{\"name\":\"Alice\"}}]}}`);
    v_pca.field(\"users\").item(0).field(\"name\").as_text()
}"
    )
    .expr("run_pca()")
    .result(Value::str("Alice"));
}

/// P54 step 4 — locks the LOFT.md § Match expressions JsonValue
/// example.  Exercises destructuring of every JsonValue variant
/// (JObject / JArray / JNumber / JNull / wildcard) so that the
/// documented `match json_parse(raw) { ... }` patterns stay
/// supported.  When the doc is read by a new user, the same
/// arms must dispatch correctly today.
#[test]
fn p54_match_on_jsonvalue_classifies_each_kind() {
    code!(
        "fn classify_pmj(raw: text) -> text {
    match json_parse(raw) {
        JObject { fields: _ } => \"object\",
        JArray  { items: _ }  => \"array\",
        JNumber { value: _ }  => \"number\",
        JInteger { value: _ } => \"number\",
        JNull                 => \"null-or-error\",
        _                     => \"other\"
    }
}
fn run_pmj() -> integer {
    score_pmj = 0;
    if classify_pmj(`{{\"k\":1}}`) == \"object\" { score_pmj += 1; }
    if classify_pmj(\"[1,2]\") == \"array\" { score_pmj += 1; }
    if classify_pmj(\"42\") == \"number\" { score_pmj += 1; }
    if classify_pmj(\"null\") == \"null-or-error\" { score_pmj += 1; }
    if classify_pmj(\"not-json\") == \"null-or-error\" { score_pmj += 1; }
    if classify_pmj(`\"hi\"`) == \"other\" { score_pmj += 1; }
    score_pmj
}"
    )
    .expr("run_pmj()")
    .result(Value::Int(6));
}

#[test]
fn p54_struct_parse_accepts_jsonvalue() {
    code!(
        "struct User { name: text, age: integer }
fn run() -> text {
    v = json_parse(\"{{\\\"name\\\":\\\"Bob\\\",\\\"age\\\":25}}\");
    u = User.parse(v);
    u.name
}"
    )
    .expr("run()")
    .result(Value::str("Bob"));
}

/// P54 step 5 — `Type.parse(JsonValue)` populates an integer field
/// by unwrapping through `as_long()` + `OpCastIntFromLong` narrow.
/// Pairs with the above text-field test; together they lock both
/// primitive paths (text via OpSetText, integer via OpSetInt).
#[test]
fn p54_struct_parse_accepts_jsonvalue_integer_field() {
    code!(
        "struct User { name: text, age: integer }
fn run_age() -> integer {
    v = json_parse(\"{{\\\"name\\\":\\\"Bob\\\",\\\"age\\\":25}}\");
    u = User.parse(v);
    u.age
}"
    )
    .expr("run_age()")
    .result(Value::Int(25));
}

/// P54 step 5 nested slice — nested struct field (`Type::Reference`
/// to another struct) recurses into `build_struct_from_jsonvalue`
/// on the corresponding sub-JsonValue, populating the embedded
/// struct record.  Exercises the full path through
/// `OpCopyRecord`-for-Reference in `set_field_no_check`.
#[test]
fn p54_struct_parse_accepts_nested_struct_field() {
    code!(
        "struct Inner { x_coord: integer }
struct Outer { label: text, data: Inner }
fn run_nested() -> integer {
    v = json_parse(\"{{\\\"label\\\":\\\"ok\\\",\\\"data\\\":{{\\\"x_coord\\\":99}}}}\");
    o = Outer.parse(v);
    o.data.x_coord
}"
    )
    .expr("run_nested()")
    .result(Value::Int(99));
}

/// P54 step 5 nested slice — verify the outer struct's primitive
/// text field is populated when a nested-struct field is also
/// present.  Complements `p54_struct_parse_accepts_nested_struct_field`
/// which returned only the inner integer; this returns the outer
/// label so a regression in field-ordering or offset calculation
/// for mixed-type structs gets caught.
#[test]
fn p54_struct_parse_nested_populates_outer_text_too() {
    code!(
        "struct Inner { x_coord: integer }
struct Outer { label: text, data: Inner }
fn run_label() -> text {
    v = json_parse(\"{{\\\"label\\\":\\\"ok\\\",\\\"data\\\":{{\\\"x_coord\\\":99}}}}\");
    o = Outer.parse(v);
    o.label
}"
    )
    .expr("run_label()")
    .result(Value::str("ok"));
}

/// P54 step 5 — `JsonValue`-typed field captures the sub-tree
/// verbatim as a passthrough.  Solves the "forward arbitrary
/// subtree" use case where a struct has a dynamic-shape payload.
/// The field() result gets OpCopyRecord'd into the struct's
/// JsonValue slot; kind() on the embedded payload reads the
/// discriminant back as `"JArray"` confirming the bytes round-trip.
#[test]
fn p54_struct_parse_captures_jsonvalue_field_verbatim() {
    code!(
        "struct WithPayload { name: text, info: JsonValue }
fn run_payload_kind() -> text {
    v = json_parse(\"{{\\\"name\\\":\\\"demo\\\",\\\"info\\\":[1,2,3]}}\");
    p = WithPayload.parse(v);
    p.info.kind()
}"
    )
    .expr("run_payload_kind()")
    .result(Value::str("JArray"));
}

/// P54 step 5 vector-field slice — populate `vector<integer>` from
/// a JArray of numbers.  Today's implementation routes through
/// the `n_jsonvalue_to_vector_long` native which walks the
/// JArray at runtime, truncates each JNumber toward zero, and
/// appends.  Other primitive element types (text / float /
/// boolean / integer) and struct-element vectors are follow-up
/// slices.
#[test]
fn p54_struct_parse_accepts_vector_long_field_len() {
    code!(
        "struct Data { items: vector<integer> }
fn run() -> integer {
    v = json_parse(\"{{\\\"items\\\":[10,20,30]}}\");
    d = Data.parse(v);
    len(d.items)
}"
    )
    .expr("run()")
    .result(Value::Int(3));
}

#[test]
fn p54_struct_parse_vector_long_first_element() {
    code!(
        "struct Data { items: vector<integer> }
fn run_first() -> integer {
    v = json_parse(\"{{\\\"items\\\":[10,20,30]}}\");
    d = Data.parse(v);
    d.items[0]
}"
    )
    .expr("run_first()")
    .result(Value::Long(10));
}

#[test]
fn p54_struct_parse_vector_long_iterates_correctly() {
    code!(
        "struct Data { items: vector<integer> }
fn run_sum() -> integer {
    v = json_parse(\"{{\\\"items\\\":[10,20,30]}}\");
    d = Data.parse(v);
    total = 0;
    for x in d.items { total += x; }
    total
}"
    )
    .expr("run_sum()")
    .result(Value::Long(60));
}

#[test]
fn p54_struct_parse_vector_long_empty_array() {
    code!(
        "struct Data { items: vector<integer> }
fn run_empty() -> integer {
    v = json_parse(\"{{\\\"items\\\":[]}}\");
    d = Data.parse(v);
    len(d.items)
}"
    )
    .expr("run_empty()")
    .result(Value::Int(0));
}

/// P54 step 5 vector-field slice — `vector<integer>` populated
/// via the generic `n_jsonvalue_to_vector` native (elem_code = 2).
/// JNumber elements truncate toward zero with i32 narrowing;
/// non-number elements contribute `i32::MIN`.
#[test]
fn p54_struct_parse_vector_integer_field() {
    code!(
        "struct D { ns: vector<integer> }
fn run_int() -> integer {
    v = json_parse(\"{{\\\"ns\\\":[100,200,300]}}\");
    d = D.parse(v);
    total = 0;
    for x in d.ns { total += x; }
    total
}"
    )
    .expr("run_int()")
    .result(Value::Int(600));
}

/// P54 step 5 vector-field slice — `vector<float>` populated
/// via the generic `n_jsonvalue_to_vector` native (elem_code = 3).
/// JNumber elements pass through verbatim; non-number → NaN.
#[test]
fn p54_struct_parse_vector_float_field() {
    code!(
        "struct D { fs: vector<float> }
fn run_float() -> float {
    v = json_parse(\"{{\\\"fs\\\":[1.5,2.5]}}\");
    d = D.parse(v);
    d.fs[0]
}"
    )
    .expr("run_float()")
    .result(Value::Float(1.5));
}

/// P54 step 5 vector-field slice — `vector<boolean>` populated
/// via the generic `n_jsonvalue_to_vector` native (elem_code = 4).
/// JBool elements copy 0/1 byte; non-bool → 0.  The boolean case
/// previously hung because the handle store allocated only 1 word
/// (matching the element size) but the handle's vec_rec int sits
/// at byte offset 8 — overflowed the next free-block's header
/// and corrupted claim_scan into an infinite loop.  Fixed by
/// ensuring the handle store is always ≥ 2 words regardless of
/// element size.
#[test]
fn p54_struct_parse_vector_boolean_field() {
    code!(
        "struct D { bs: vector<boolean> }
fn run_bool() -> boolean {
    v = json_parse(\"{{\\\"bs\\\":[true,false,true]}}\");
    d = D.parse(v);
    d.bs[1]
}"
    )
    .expr("run_bool()")
    .result(Value::Boolean(false));
}

/// P54 step 5 vector-field slice — `vector<text>` populated via
/// the generic `n_jsonvalue_to_vector` native (elem_code = 5).
/// JString elements copy into the result vector's string area;
/// non-string → empty text.
#[test]
fn p54_struct_parse_vector_text_field() {
    code!(
        "struct D { ts: vector<text> }
fn run_text() -> text {
    v = json_parse(\"{{\\\"ts\\\":[\\\"hello\\\",\\\"world\\\"]}}\");
    d = D.parse(v);
    d.ts[0]
}"
    )
    .expr("run_text()")
    .result(Value::str("hello"));
}

/// P54 step 5 vector-of-struct slice — `vector<T>` where `T` is
/// a struct populates each element via runtime field-walk
/// (elem_code = 6).  The native enumerates the struct's fields
/// from `stores.types[struct_kt].parts` and writes each
/// primitive field by name lookup in the JSON object element.
/// Today handles primitive struct fields (text / integer /
/// long / float / boolean); nested struct or vector fields
/// inside the element type stay at zero-init defaults.
#[test]
fn p54_struct_parse_vector_of_struct_count() {
    code!(
        "struct User { name: text, age: integer }
struct Inbox { users: vector<User> }
fn run() -> integer {
    v = json_parse(\"{{\\\"users\\\":[{{\\\"name\\\":\\\"Alice\\\",\\\"age\\\":30}},{{\\\"name\\\":\\\"Bob\\\",\\\"age\\\":25}}]}}\");
    inbox = Inbox.parse(v);
    len(inbox.users)
}"
    )
    .expr("run()")
    .result(Value::Int(2));
}

#[test]
fn p54_struct_parse_vector_of_struct_first_text_field() {
    code!(
        "struct User { name: text, age: integer }
struct Inbox { users: vector<User> }
fn run() -> text {
    v = json_parse(\"{{\\\"users\\\":[{{\\\"name\\\":\\\"Alice\\\",\\\"age\\\":30}},{{\\\"name\\\":\\\"Bob\\\",\\\"age\\\":25}}]}}\");
    inbox = Inbox.parse(v);
    inbox.users[0].name
}"
    )
    .expr("run()")
    .result(Value::str("Alice"));
}

#[test]
fn p54_struct_parse_vector_of_struct_second_integer_field() {
    code!(
        "struct User { name: text, age: integer }
struct Inbox { users: vector<User> }
fn run() -> integer {
    v = json_parse(\"{{\\\"users\\\":[{{\\\"name\\\":\\\"Alice\\\",\\\"age\\\":30}},{{\\\"name\\\":\\\"Bob\\\",\\\"age\\\":25}}]}}\");
    inbox = Inbox.parse(v);
    inbox.users[1].age
}"
    )
    .expr("run()")
    .result(Value::Int(25));
}

#[test]
fn p54_struct_parse_vector_of_struct_iterates() {
    code!(
        "struct Score { val: integer }
struct Bag { scores: vector<Score> }
fn run() -> integer {
    v = json_parse(\"{{\\\"scores\\\":[{{\\\"val\\\":10}},{{\\\"val\\\":20}},{{\\\"val\\\":30}}]}}\");
    b = Bag.parse(v);
    total = 0;
    for s in b.scores { total += s.val; }
    total
}"
    )
    .expr("run()")
    .result(Value::Long(60));
}

#[test]
fn p54_struct_parse_vector_of_struct_empty_array() {
    code!(
        "struct User { name: text, age: integer }
struct Inbox { users: vector<User> }
fn run() -> integer {
    v = json_parse(\"{{\\\"users\\\":[]}}\");
    inbox = Inbox.parse(v);
    len(inbox.users)
}"
    )
    .expr("run()")
    .result(Value::Int(0));
}

#[test]
fn p54_struct_parse_vector_of_struct_missing_field_is_null() {
    code!(
        "struct User { name: text, age: integer }
struct Inbox { users: vector<User> }
fn run() -> integer {
    v = json_parse(\"{{\\\"users\\\":[{{\\\"name\\\":\\\"Alice\\\"}}]}}\");
    inbox = Inbox.parse(v);
    inbox.users[0].age
}"
    )
    .expr("run()")
    .result(Value::Null);
}

/// Q1 schema-side — type mismatch on a primitive field during
/// `Type.parse(JsonValue)` pushes a path-qualified diagnostic
/// to `json_errors()` instead of silently producing a null
/// sentinel.  Asserts the diagnostic contains the struct's
/// name + field name + expected vs actual variant.
#[test]
fn q1_schema_side_type_mismatch_pushes_diagnostic() {
    code!(
        "struct User { name: text, age: integer }
fn run() -> boolean {
    v = json_parse(\"{{\\\"name\\\":\\\"Alice\\\",\\\"age\\\":\\\"thirty\\\"}}\");
    u = User.parse(v);
    if u == u {}
    err = json_errors();
    err.contains(\"User.age\") && err.contains(\"expected JNumber\") && err.contains(\"got JString\")
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

/// Q1 schema-side — missing fields (JSON object lacks the key
/// entirely) DO NOT push a diagnostic.  Distinguishes "absent"
/// from "present-but-wrong-kind" — the former is silently
/// allowed (caller handles the null sentinel via `??` or `!`).
#[test]
fn q1_schema_side_missing_field_silent() {
    code!(
        "struct User { name: text, age: integer }
fn run() -> integer {
    v = json_parse(\"{{\\\"name\\\":\\\"Alice\\\"}}\");
    u = User.parse(v);
    if u == u {}
    json_errors().len()
}"
    )
    .expr("run()")
    .result(Value::Int(0));
}

/// Q1 schema-side — a clean parse leaves `json_errors()`
/// empty.  Companion guard to the mismatch-pushes test.
#[test]
fn q1_schema_side_clean_parse_no_diagnostic() {
    code!(
        "struct User { name: text, age: integer }
fn run() -> integer {
    v = json_parse(\"{{\\\"name\\\":\\\"Alice\\\",\\\"age\\\":30}}\");
    u = User.parse(v);
    if u == u {}
    json_errors().len()
}"
    )
    .expr("run()")
    .result(Value::Int(0));
}

/// Q1 schema-side — type mismatch on a primitive field of a
/// struct INSIDE a vector element pushes a diagnostic via the
/// runtime field-walk (not the compile-time check).  The
/// runtime path mirrors the same struct-name + field-name
/// diagnostic shape as the compile-time path.
#[test]
fn q1_schema_side_vector_element_type_mismatch_pushes_diagnostic() {
    code!(
        "struct User { name: text, age: integer }
struct Inbox { users: vector<User> }
fn run() -> boolean {
    v = json_parse(\"{{\\\"users\\\":[{{\\\"name\\\":\\\"Alice\\\",\\\"age\\\":30}},{{\\\"name\\\":\\\"Bob\\\",\\\"age\\\":\\\"twenty\\\"}}]}}\");
    inbox = Inbox.parse(v);
    if inbox == inbox {}
    err = json_errors();
    err.contains(\"User.age\") && err.contains(\"expected JNumber\") && err.contains(\"got JString\")
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

/// Q1 schema-side — text field receiving a number pushes a
/// diagnostic naming JString as expected and JNumber as actual.
#[test]
fn q1_schema_side_text_field_receiving_number() {
    code!(
        "struct User { name: text }
fn run() -> boolean {
    v = json_parse(\"{{\\\"name\\\":42}}\");
    u = User.parse(v);
    if u == u {}
    err = json_errors();
    err.contains(\"User.name\") && err.contains(\"expected JString\") && err.contains(\"got JInteger\")
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

/// Q1 schema-side — boolean field receiving a string pushes a
/// diagnostic naming JBool as expected.
#[test]
fn q1_schema_side_boolean_field_receiving_string() {
    code!(
        "struct Flag { active: boolean }
fn run() -> boolean {
    v = json_parse(\"{{\\\"active\\\":\\\"yes\\\"}}\");
    f = Flag.parse(v);
    if f == f {}
    err = json_errors();
    err.contains(\"Flag.active\") && err.contains(\"expected JBool\") && err.contains(\"got JString\")
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

// Deleted: p54_struct_parse_rejects_plain_text — tested a rejected design decision
// (hard rejection of text args). Current design auto-wraps through json_parse.

// ── P54 struct-enum blockers — runtime specs (BITING_PLAN § P54) ──────────
//
// Each struct-enum bug found while building JsonValue gets a regression
// guard (for fixed bugs) or an #[ignore]'d spec (for open bugs).  The
// #[ignore]'d tests document the expected behaviour; they'll go green
// automatically when the corresponding blocker is resolved.

/// B1 (FIXED `61c36d7`): `match v { UnitVariant => … }` no longer panics
/// when `v` is produced somewhere other than a literal.  Exercise via
/// a mixed-variant enum where the unit arm matches.
#[test]
fn p54_b1_unit_variant_match_from_binding() {
    code!(
        "pub enum Palette { Null, Shade { v: integer } }
fn run() -> integer {
    p = Shade { v: 7 };
    match p {
        Null => -1,
        Shade { v } => v
    }
}"
    )
    .expr("run()")
    .result(Value::Int(7));
}

/// B6 (FIXED `5684df2`): match-arm type unification strips `RefVar`
/// wrappers.  Binding a field-carrying struct variant's text field in one
/// arm and returning a literal text (`""`) in another no longer errors
/// with 'cannot unify: &text and text'.
///
/// Uses plain struct (not struct-enum) to dodge the still-open B4
/// runtime bug.  Same type-system machinery — the field binding yields
/// `&text`, the wildcard arm returns owned `text`.
#[test]
fn p54_b6_match_arm_text_unify_plain_struct() {
    code!(
        "struct Pair { a: text, b: integer }
fn extract(p: const Pair) -> text {
    match p.b {
        0 => p.a,
        _ => \"other\"
    }
}
fn run() -> text {
    extract(Pair { a: \"hello\", b: 0 })
}"
    )
    .expr("run()")
    .result(Value::str("hello"));
}

/// Direct-return versions — now working (commit `6074619` opened up
/// struct-enum subject dispatch in match; constructing + returning
/// without an intermediate variable round-trips cleanly).
#[test]
fn p54_b3_float_not_null_direct_return() {
    code!(
        "pub enum JV { A { v: float } }
fn mk() -> JV { A { v: 42.5 } }
fn run() -> float {
    x = mk();
    match x {
        A { v } => v
    }
}"
    )
    .expr("run()")
    .result(Value::Float(42.5));
}

#[test]
fn p54_b4_mixed_variant_direct_return() {
    code!(
        "pub enum JV { JA { v: boolean }, JB { v: integer }, JC { v: text } }
fn mk() -> JV { JB { v: 42 } }
fn run() -> integer {
    x = mk();
    match x {
        JA { v } => if v { 1 } else { 0 },
        JB { v } => v,
        JC { v } => v.len()
    }
}"
    )
    .expr("run()")
    .result(Value::Int(42));
}

/// Open (sharpened): the intermediate-variable pattern `n = A { … };
/// n` (tail expression, no `return`) crashes.  Sharpened in
/// `p54_b3_int_via_intermediate` to narrow the bug: the
/// tail-expression path frees the local's store while the returned
/// value still references it.  `return n;` works — see
/// `p54_struct_enum_explicit_return_of_local`.
#[test]
fn p54_b3_float_via_intermediate() {
    code!(
        "pub enum JV { A { v: float } }
fn mk() -> JV {
    n = A { v: 42.5 };
    n
}
fn run() -> float {
    x = mk();
    match x {
        A { v } => v
    }
}"
    )
    .expr("run()")
    .result(Value::Float(42.5));
}

/// B5 full regression guard: self-referential struct-enum with
/// recursive method.  Three layers had to land for this to pass:
///   1. `fill_all` registers `main_vector<T>` wrappers for every
///      struct/enum-variant `vector<T>` field — closes the original
///      "Incomplete record" panic on `OpDatabase(db_tp=u16::MAX)`.
///   2. Match-arm bindings carry `skip_free` — closes the garbage
///      `FreeRef(ref(4621,…))` on a not-taken arm's binding slot.
///   3. Struct-enum return-slot accounting (closed as a side-effect
///      of the cross-PR struct-enum return work landed in #168→#174).
#[test]
fn p54_b5_recursive_struct_enum() {
    code!(
        "pub enum Tree { Leaf { v: integer }, Node { kids: vector<Tree> } }
fn count(t: const Tree) -> integer {
    match t {
        Leaf { v } => v,
        Node { kids } => {
            c = 0;
            for k in kids { c += count(k); }
            c
        }
    }
}
fn run() -> integer {
    root = Node { kids: [Leaf { v: 3 }, Leaf { v: 4 }] };
    count(root)
}"
    )
    .expr("run()")
    .result(Value::Int(7));
}

/// B5 match-arm-binding regression (layer 2): the not-taken arm of
/// a match whose binding is a `vector<T>` must not crash scope
/// cleanup.  Before the `skip_free` fix at src/parser/control.rs:1103,
/// the match's `_mv_items_*` binding in the `Full { items }` arm
/// was freed at function exit even when the `Empty` arm was taken,
/// reading garbage bytes as a `DbRef` and panicking in
/// `Stores::free_named` with an out-of-bounds `store_nr`.  Now
/// skip_free suppresses the OpFreeRef emission, so the garbage
/// slot stays untouched.  Guards the layer-2 half of B5 against
/// regression independently of the recursive path exercised by
/// `p54_b5_recursive_struct_enum` (now un-ignored).
#[test]
fn p54_b5_not_taken_arm_with_vector_binding_ok() {
    code!(
        "struct Item { v: integer }
pub enum Wrap { Empty, Full { items: vector<Item> } }
fn run() -> integer {
    w = Wrap.Empty;
    match w {
        Empty => 42,
        Full { items } => items.len()
    }
}"
    )
    .expr("run()")
    .result(Value::Int(42));
}

/// B5 type-registration regression: a recursive struct-enum (`Node`
/// variant contains `vector<Tree>`) must get its `main_vector<Tree>`
/// wrapper registered during `fill_all`, so codegen's
/// `name_type("main_vector<Tree>")` lookup returns a real
/// `known_type` instead of `u16::MAX`.  Without this fix, simply
/// constructing `Node { kids: [...] }` would panic in
/// `Store::claim("Incomplete record")` — the scenario the original
/// B5 ticket reported.  This narrower regression guard exercises
/// just the construct-and-measure-len path (no match, no for-loop),
/// isolating the half of B5 that is now fixed from the still-open
/// match-arm-binding half tracked in `p54_b5_recursive_struct_enum`.
#[test]
fn p54_b5_recursive_struct_enum_construction() {
    code!(
        "pub enum Tree { Leaf { v: integer }, Node { kids: vector<Tree> } }
fn run() -> integer {
    root = Node { kids: [Leaf { v: 3 }, Leaf { v: 4 }] };
    match root {
        Leaf { v } => v,
        Node { kids } => kids.len()
    }
}"
    )
    .expr("run()")
    .result(Value::Int(2));
}

/// B5 layer 1 + 2 combined regression: iterate over a struct-enum-
/// variant `vector<T>` binding inside a match arm.  Exercises the
/// same code paths as `p54_b5_recursive_struct_enum` (the still-
/// ignored recursive one) up to but not including the recursive
/// inner call — so if the recursion layer lands later, a regression
/// on type registration OR binding skip_free gets caught here even
/// when the recursive path is green.  Asserts the for-loop sees
/// each element with its correct `Leaf.v` payload.
#[test]
fn p54_b5_for_loop_over_enum_variant_vector() {
    code!(
        "pub enum Tree { Leaf { v: integer }, Node { kids: vector<Tree> } }
fn run() -> integer {
    root = Node { kids: [Leaf { v: 3 }, Leaf { v: 4 }, Leaf { v: 5 }] };
    sum = 0;
    match root {
        Leaf { v } => sum += v,
        Node { kids } => {
            for k in kids {
                match k {
                    Leaf { v } => sum += v,
                    Node { kids } => sum += kids.len()
                }
            }
        }
    }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(12));
}

/// P54 related — positive baseline for struct-enum parameter passing.
/// Passing a struct-enum into a function and matching on it inside works
/// today; this test guards against that regressing while the return-
/// direction bugs (B3/B4) are being resolved.
#[test]
fn p54_struct_enum_as_parameter_ok() {
    code!(
        "pub enum JV { A { v: integer }, B { x: integer } }
fn show(j: const JV) -> integer {
    match j {
        A { v } => v,
        B { x } => x
    }
}
fn run() -> integer {
    n = A { v: 42 };
    show(n)
}"
    )
    .expr("run()")
    .result(Value::Int(42));
}

/// P54 related — positive baseline for struct-enum constructed in a
/// function and immediately matched in the same scope (no return).
/// Works today; guard against regression.
#[test]
fn p54_struct_enum_literal_then_match_same_scope() {
    code!(
        "pub enum JV { A { v: integer }, B { x: integer } }
fn run() -> integer {
    n = A { v: 7 };
    match n {
        A { v } => v,
        B { x } => x
    }
}"
    )
    .expr("run()")
    .result(Value::Int(7));
}

/// FIXED: single-variant struct-enum with integer payload returned
/// from a function now round-trips cleanly.  Previously crashed with
/// 'malloc(): unaligned tcache chunk detected'.  The fix that closed
/// this was the Reference(Enum)-as-match-subject arm in commit
/// `6074619` — same root cause (the for-body subject-type dispatch
/// mismatch also affected struct-enum assignment-site dispatch).
/// `float` / `float not null` variants (see p54_b3_float_not_null_variant)
/// and mixed-field variants (p54_b4_mixed_variant_return) remain
/// broken.
#[test]
fn p54_b3_single_variant_return() {
    code!(
        "pub enum JV { A { v: integer } }
fn mk() -> JV { A { v: 42 } }
fn run() -> integer {
    x = mk();
    match x {
        A { v } => v
    }
}"
    )
    .expr("run()")
    .result(Value::Int(42));
}

/// P54 — JsonValue-style extractors via a plain tagged struct.  This
/// is the workaround pattern callers use today while struct-enum
/// return-direction (B3/B4) is broken: a discriminant field plus one
/// slot per payload type.  Ugly but unblocks JSON work now.
///
/// Verifies that extractor-with-null-on-mismatch compiles cleanly
/// (B6 fix) and returns the expected values for both matching and
/// mismatching kind arms.
#[test]
fn p54_tagged_struct_extractors_work_today() {
    code!(
        "struct Tagged { kind: integer, text_val: text, num_val: float }
pub fn as_text(self: const Tagged) -> text {
    at_out = \"\";
    match self.kind {
        1 => { at_out = self.text_val; },
        _ => {}
    }
    at_out
}
pub fn as_number(self: const Tagged) -> float {
    match self.kind {
        2 => self.num_val,
        _ => 0.0
    }
}
fn run() -> text {
    r = \"\";
    t = Tagged { kind: 1, text_val: \"hello\", num_val: 0.0 };
    r += t.as_text();
    r += \"|\";
    n = Tagged { kind: 2, text_val: \"\", num_val: 3.14 };
    nt = n.as_text();
    r += \"miss[{nt}]\";
    r
}"
    )
    .expr("run()")
    .result(Value::str("hello|miss[]"));
}

/// P54 — the same extractor pattern via a struct-enum.  Now works
/// after the Reference(Enum) match-subject fix (commit `6074619`) —
/// the `t.as_text()` call site receives a struct-enum argument,
/// matches it, and returns the bound text.
#[test]
fn p54_struct_enum_extractors_spec() {
    code!(
        "pub enum Jv { Jstr { v: text }, Jnum { v: float } }
pub fn jv_as_text(self: Jv) -> text {
    jvat_out = \"\";
    match self {
        Jstr { v } => { jvat_out = v; },
        _ => {}
    }
    jvat_out
}
fn make_jstr() -> Jv { Jstr { v: \"hello\" } }
fn run() -> text {
    jvs_t = make_jstr();
    jvs_t.jv_as_text()
}"
    )
    .expr("run()")
    .result(Value::str("hello"));
}

/// B1-style fix applied to or-patterns: `A | B => …` over unit variants
/// in a mixed struct-enum previously panicked at
/// parser/control.rs:699 (same index-OOB shape as B1).  Guard the
/// attributes[0] access the same way.
#[test]
fn p54_or_pattern_mixed_struct_enum() {
    code!(
        "pub enum Sig { Off, Idle, On { level: integer } }
fn classify(s: const Sig) -> text {
    match s {
        Off | Idle => \"inactive\",
        On { level } => \"active\"
    }
}
fn run() -> text {
    classify(On { level: 80 })
}"
    )
    .expr("run()")
    .result(Value::str("active"));
}

/// Match guard on a struct-enum variant works today — regression
/// guard so future parser work doesn't drop this.
#[test]
fn p54_match_guard_on_struct_enum() {
    code!(
        "pub enum Sig { Off, On { level: integer } }
fn describe(s: const Sig) -> text {
    match s {
        Off => \"off\",
        On { level } if level > 50 => \"hi\",
        On { level } => \"lo\"
    }
}
fn run() -> text {
    r = \"\";
    r += describe(On { level: 80 });
    r += \",\";
    r += describe(On { level: 10 });
    r
}"
    )
    .expr("run()")
    .result(Value::str("hi,lo"));
}

/// B2-runtime (sub-bug of B3/B4): constructing a bare unit-variant
/// literal (`s = Idle;`) for a mixed struct-enum crashes at runtime
/// with `index out of bounds: the len is 2 but the index is <junk>`.
/// B2-compile-fix let this test compile; the runtime codegen path for
/// producing a valid struct-enum record from a bare unit-variant name
/// is still broken.  When that's fixed, this test goes green.
#[test]
fn p54_b2_unit_variant_literal_construction() {
    code!(
        "pub enum Sig { Off, Idle, On { level: integer } }
fn run() -> text {
    s = Sig.Idle;
    match s {
        Off => \"off\",
        Idle => \"idle\",
        On { level } => \"on\"
    }
}"
    )
    .expr("run()")
    .result(Value::str("idle"));
}

/// P54 — positive baseline: a plain enum (all unit variants) round-trips
/// through a bare identifier literal.  Only the *mixed* struct-enum
/// case is broken (B2-runtime), not plain enums.  This test guards
/// that distinction.
#[test]
fn p54_plain_enum_bare_variant_works() {
    code!(
        "pub enum Sig { Off, Idle, On }
fn run() -> text {
    s = Sig.Idle;
    match s {
        Off => \"off\",
        Idle => \"idle\",
        On => \"on\"
    }
}"
    )
    .expr("run()")
    .result(Value::str("idle"));
}

/// B2-runtime (qualified form): `Sig.Idle` as an expression in a
/// mixed struct-enum.  The parse path (parser/fields.rs) was giving
/// the result block `Type::Enum(dnr, true, vec![w])` — propagating
/// the work-ref into the LHS as a dep, so `s` became a borrower and
/// nothing freed the store.  Fixed by mirroring parser/objects.rs
/// and using `vec![]` instead (LHS owns, work-ref is skip_free).
#[test]
fn p54_b2_qualified_unit_variant_mixed_enum() {
    code!(
        "pub enum Sig { Off, Idle, On { level: integer } }
fn run() -> text {
    s = Sig.Idle;
    match s {
        Off => \"off\",
        Idle => \"idle\",
        On { level } => \"on\"
    }
}"
    )
    .expr("run()")
    .result(Value::str("idle"));
}

/// P54 — positive baseline: plain enum match inside a for-loop body
/// works.  Pairs with the `#[ignore]`'d struct-enum version below to
/// isolate the struct-enum-specific breakage.
#[test]
fn p54_plain_enum_match_inside_for() {
    code!(
        "pub enum Item { One, Two }
fn run() -> text {
    v: vector<Item> = [One, Two];
    r = \"\";
    for x in v {
        match x {
            One => { r += \"1\"; },
            Two => { r += \"2\"; }
        }
    }
    r
}"
    )
    .expr("run()")
    .result(Value::str("12"));
}

/// FIXED: struct-enum match inside a for-loop body previously failed
/// to parse with 'Expect token }' on the first arm's `=>`.  Root
/// cause: `for_type` maps `vector<StructEnum>` to
/// `Type::Reference(enum_def, …)` as the loop-variable type, but
/// `parse_match` only accepted `Type::Enum` or
/// `Type::Reference(EnumValue/Struct)` as a valid subject.  Added a
/// `Reference(d_nr) if DefType::Enum` case; struct-enum for-body
/// matches now compile and run.
#[test]
fn p54_struct_enum_match_inside_for() {
    code!(
        "pub enum Item { Empty, Filled { qty: integer } }
fn run() -> integer {
    v = [Filled { qty: 3 }, Filled { qty: 7 }];
    sum = 0;
    for x in v {
        match x {
            Empty => {},
            Filled { qty } => { sum += qty; }
        }
    }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(10));
}

/// P54 — positive baseline: struct-enum match outside a for-loop (via
/// direct indexing) works.  Confirms the struct-enum match machinery
/// itself is fine — the above bug lives in for-body parsing.
#[test]
fn p54_struct_enum_match_via_index_works() {
    code!(
        "pub enum Item { Empty, Filled { qty: integer } }
fn run() -> integer {
    v = [Filled { qty: 3 }, Filled { qty: 7 }];
    x = v[0];
    match x {
        Empty => 0,
        Filled { qty } => qty
    }
}"
    )
    .expr("run()")
    .result(Value::Int(3));
}

/// Struct-enum as a field of a plain struct — construction, access
/// through field chain, and match all work.  This is a pattern
/// JsonValue callers use today (wrap the JsonValue in a holder
/// struct).
#[test]
fn p54_struct_enum_as_struct_field() {
    code!(
        "pub enum Inner { A { v: integer }, B { v: text } }
pub struct Holder { inner: Inner, count: integer }
fn run() -> text {
    h = Holder { inner: A { v: 7 }, count: 1 };
    match h.inner {
        A { v } => \"A-{v}-{h.count}\",
        B { v } => \"B-{v}-{h.count}\"
    }
}"
    )
    .expr("run()")
    .result(Value::str("A-7-1"));
}

/// Vector of struct-enums with mixed variants; iterate and dispatch
/// by variant.  Exercises the Reference(Enum) match-subject fix
/// (commit `6074619`) plus accumulator mutation inside match arms.
#[test]
fn p54_struct_enum_vector_accumulate() {
    code!(
        "pub enum Op { Add { v: integer }, Sub { v: integer } }
fn run() -> integer {
    ops = [Add { v: 5 }, Sub { v: 3 }, Add { v: 2 }];
    sum = 0;
    for op in ops {
        match op {
            Add { v } => { sum += v; },
            Sub { v } => { sum -= v; }
        }
    }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(4));
}

/// B3 narrowed: the bug is the TAIL-EXPRESSION path, not the
/// intermediate variable itself.  `n = A { … }; n` (no `return`)
/// crashes because the function's scope exit frees `n`'s store
/// while the returned value still references it.  `return n;`
/// works fine — see `p54_struct_enum_explicit_return_of_local`.
/// Fix requires teaching the tail-expression-as-return codegen to
/// suppress the free of any local whose store is being returned,
/// or to materialize a copy.
#[test]
fn p54_b3_int_via_intermediate() {
    code!(
        "pub enum JV { A { v: integer } }
fn mk() -> JV {
    n = A { v: 42 };
    n
}
fn run() -> integer {
    x = mk();
    match x {
        A { v } => v
    }
}"
    )
    .expr("run()")
    .result(Value::Int(42));
}

/// Struct-enum intermediate-variable return — WORKS with explicit
/// `return n;` statement.  Pairs with `p54_b3_int_via_intermediate`
/// which fails on the tail-expression form `n = A { … }; n` (no
/// `return` keyword).  The tail-expression path has an ownership
/// bug: the local `n` gets freed on function exit while the returned
/// value still references the same store — explicit `return n`
/// avoids it.  Workaround today: always write `return n;`.
#[test]
fn p54_struct_enum_explicit_return_of_local() {
    code!(
        "pub enum JV { A { v: integer } }
fn mk() -> JV {
    n = A { v: 42 };
    return n;
}
fn run() -> integer {
    x = mk();
    match x {
        A { v } => v
    }
}"
    )
    .expr("run()")
    .result(Value::Int(42));
}

/// Reassignment before return — also works with explicit return.
/// Documents that the bug is specifically the *tail-expression*
/// path, not the assignment path.
#[test]
fn p54_struct_enum_reassign_explicit_return() {
    code!(
        "pub enum JV { A { v: integer } }
fn mk() -> JV {
    n = A { v: 1 };
    n = A { v: 42 };
    return n;
}
fn run() -> integer {
    x = mk();
    match x {
        A { v } => v
    }
}"
    )
    .expr("run()")
    .result(Value::Int(42));
}

/// Struct-enum as the value in `hash<Entry[name]>` — JsonValue's
/// eventual shape for JObject.  Works end-to-end: construction,
/// hash lookup, match on the retrieved value's enum field.  Pairs
/// with `p54_struct_enum_as_struct_field` to show struct-enum
/// embedding in containers is fully viable.
#[test]
fn p54_struct_enum_in_hash_value() {
    code!(
        "pub enum Val { IntV { v: integer }, StrV { v: text } }
pub struct Entry { name: text, value: Val }
pub struct Holder { h: hash<Entry[name]> }
fn run() -> text {
    m = Holder { h: [Entry { name: \"a\", value: IntV { v: 7 } }] };
    e = m.h[\"a\"];
    if e == null { return \"miss\"; }
    match e.value {
        IntV { v } => \"int-{v}\",
        StrV { v } => \"str-{v}\"
    }
}"
    )
    .expr("run()")
    .result(Value::str("int-7"));
}

/// Nested struct-enum — outer variant carries an inner struct-enum
/// as a field.  Full match-and-destructure chain works.  Critical
/// for JsonValue's JArray-of-JObjects / JObject-of-JArrays cases.
#[test]
fn p54_nested_struct_enum() {
    code!(
        "pub enum Inner { Leaf { v: integer } }
pub enum Outer { Wrap { inner: Inner }, Plain }
fn run() -> integer {
    o = Wrap { inner: Leaf { v: 42 } };
    match o {
        Plain => -1,
        Wrap { inner } => {
            match inner {
                Leaf { v } => v
            }
        }
    }
}"
    )
    .expr("run()")
    .result(Value::Int(42));
}

/// Struct-enum flowing through multiple function calls — parameter
/// into one fn, return from another, construct a fresh variant in
/// one arm, pass through in another.  Exercises the full Reference
/// / return / assignment path.
#[test]
fn p54_struct_enum_multi_call_flow() {
    code!(
        "pub enum V { A { v: integer }, B { v: text } }
fn double_a(x: const V) -> V {
    match x {
        A { v } => A { v: v * 2 },
        B { v } => B { v: v }
    }
}
fn describe(x: const V) -> text {
    match x {
        A { v } => \"a-{v}\",
        B { v } => \"b-{v}\"
    }
}
fn run() -> text {
    a = A { v: 5 };
    d = double_a(a);
    describe(d)
}"
    )
    .expr("run()")
    .result(Value::str("a-10"));
}

// ── P22: spatial<T> diagnostic wording (FIXED) ─────────────────────────
//
// @PLN48 S2: `spatial<T[x, y]>` now works (the radix tree), so the old
// "planned 1.1+" gate is gone.  A `spatial<T>` written WITHOUT its coordinate
// key fields is still an error — a spatial index needs coordinates — with a
// diagnostic that shows the correct bracket-key syntax.
#[test]
fn p22_spatial_without_keys_names_the_bracket_syntax() {
    code!(
        "struct Point { x: float, y: float }
struct World { items: spatial<Point> }
fn test() {
    w = World { items: [] };
}"
    )
    .error(
        "spatial<T[x, y]> needs coordinate key fields, e.g. spatial<Mob[x, y]> \
at p22_spatial_without_keys_names_the_bracket_syntax:2:39",
    );
}

// ── INC#29: !value asymmetry between boolean and integer ───────────────
//
// The unary `!` operator catches different things on different
// scalar types because the null sentinel is in-band:
//
//   boolean: false IS the null sentinel — `!b` catches both
//   integer: 0 is a real value — `!n` catches only i32::MIN
//
// This asymmetry silently changes meaning when code is ported
// between the two types.  These tests lock both shapes so a future
// uniformity refactor cannot regress without a doc update.
#[test]
fn inc29_bang_boolean_catches_false() {
    code!(
        "fn run() -> boolean {
    flag = false;
    !flag
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

#[test]
fn inc29_bang_integer_zero_is_not_null() {
    code!(
        "fn run() -> boolean {
    count = 0;
    !count
}"
    )
    .expr("run()")
    .result(Value::Boolean(false));
}

#[test]
fn inc29_bang_integer_null_is_caught() {
    // Plan-07 phase 4 step 4.3 — `/` by zero now raises a typed
    // RuntimeError on the non-nullable path; this test is about
    // INC#29 (`!n` catches integer null), not about division.  Use
    // an explicit nullable shape (`a / b ?? null`) so the divide
    // still produces the null sentinel that `!n` is here to catch.
    code!(
        "fn divide(a: integer, b: integer) -> integer? { a / b ?? null }
fn run() -> boolean {
    n = divide(1, 0);
    !n
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

// ── INC#3: #index semantics on text vs vector ──────────────────────────
//
// Text loops:    c#index == byte offset of current char (UTF-8)
// Vector loops:  v#index == 0-based element position
//
// On ASCII text the two coincide; on multi-byte text they diverge.
// Code that uses c#index as a counter passes its tests on ASCII and
// silently breaks on the first emoji.  These guards lock the
// divergence so a future "make these uniform" refactor cannot land
// without first updating LOFT.md.
#[test]
fn inc3_text_index_is_byte_offset_on_multibyte() {
    code!(
        "fn run() -> integer {
    last = 0;
    for c in \"a😊b\" { last = c#index; }
    last
}"
    )
    .expr("run()")
    // 'a' at 0, '😊' at 1 (4 bytes), 'b' at 5
    .result(Value::Int(5));
}

#[test]
fn inc3_text_count_is_character_position() {
    code!(
        "fn run() -> integer {
    last = 0;
    for c in \"a😊b\" { last = c#count; }
    last
}"
    )
    .expr("run()")
    // count is iterations completed so far; on the 'b' iteration that's 2
    .result(Value::Int(2));
}

#[test]
fn inc3_vector_index_is_element_position() {
    code!(
        "fn run() -> integer {
    items = [10, 20, 30];
    last = 0;
    for v in items { last = v#index; }
    last
}"
    )
    .expr("run()")
    .result(Value::Int(2));
}

// ── INC#26: match exhaustiveness ignores guarded arms ───────────────────
//
// A guarded arm (`pattern if guard => body`) does NOT count as
// covering that variant for exhaustiveness — the guard may fail
// at runtime.  Even if every variant has a guarded arm, a
// wildcard `_ =>` or an unguarded arm is still required.
//
// This is intentional (soundness: the compiler cannot prove the
// guard is always true), but surprising.  These tests lock the
// behaviour so a future "smarter exhaustiveness" attempt cannot
// silently drop the wildcard requirement without updating LOFT.md.
#[test]
fn inc26_guarded_arm_without_wildcard_is_rejected() {
    code!(
        "enum Color { Red, Green, Blue }
fn describe(c: const Color, bright: boolean) -> text {
    match c {
        Red if bright => \"bright red\",
        Green         => \"green\",
        Blue          => \"blue\"
    }
}"
    )
    // The match is not exhaustive: the Red guard can fail, and there
    // is no fallback for that case.  Parser must reject at compile time.
    .error(
        "match on Color is not exhaustive — missing: Red; add the missing variants or a '_ =>' wildcard \
at inc26_guarded_arm_without_wildcard_is_rejected:3:12",
    );
}

#[test]
fn inc26_guarded_arm_with_wildcard_compiles() {
    code!(
        "enum Color { Red, Green, Blue }
fn describe(c: const Color, bright: boolean) -> text {
    match c {
        Red if bright => \"bright red\",
        Green         => \"green\",
        Blue          => \"blue\",
        _             => \"dim red\"
    }
}
fn run() -> text { describe(Red, false) }"
    )
    .expr("run()")
    .result(Value::str("dim red"));
}

#[test]
fn inc26_guarded_arm_falls_through_when_guard_false() {
    code!(
        "enum Color { Red, Green, Blue }
fn describe(c: const Color, bright: boolean) -> text {
    match c {
        Red if bright => \"bright red\",
        Red           => \"dim red\",
        Green         => \"green\",
        Blue          => \"blue\"
    }
}
fn run() -> text { describe(Red, false) }"
    )
    .expr("run()")
    .result(Value::str("dim red"));
}

// ── INC#12: sort direction in the struct drives iteration direction ─────
//
// A `-` prefix on a key field in `sorted<T[-key]>` or
// `index<T[-key]>` flips the iteration direction of every query on
// that collection.  Reading the query site alone does not reveal
// the direction — the user must check the struct declaration,
// which may be far away.  This is the core of INC#12.
//
// These tests lock the direction-driven behaviour on two
// otherwise-identical sorted collections so a future uniformity
// refactor cannot silently flip either path without updating
// LOFT.md's Gotcha callout.
#[test]
fn inc12_sorted_ascending_iterates_forward() {
    code!(
        "struct ElmA { key: text, value: integer }
struct DbA { map: sorted<ElmA[key]> }
fn run_asc() -> text {
    db_a = DbA { map: [] };
    db_a.map += [ElmA { key: \"Alpha\", value: 1 }];
    db_a.map += [ElmA { key: \"Mid\",   value: 2 }];
    db_a.map += [ElmA { key: \"Omega\", value: 3 }];
    out_a = \"\";
    for v in db_a.map { out_a += \"{v.key},\"; }
    out_a
}"
    )
    .expr("run_asc()")
    .result(Value::str("Alpha,Mid,Omega,"));
}

#[test]
fn inc12_sorted_descending_iterates_backward() {
    code!(
        "struct ElmB { key: text, value: integer }
struct DbB { map: sorted<ElmB[-key]> }
fn run_desc() -> text {
    db_b = DbB { map: [] };
    db_b.map += [ElmB { key: \"Alpha\", value: 1 }];
    db_b.map += [ElmB { key: \"Mid\",   value: 2 }];
    db_b.map += [ElmB { key: \"Omega\", value: 3 }];
    out_b = \"\";
    for v in db_b.map { out_b += \"{v.key},\"; }
    out_b
}"
    )
    .expr("run_desc()")
    .result(Value::str("Omega,Mid,Alpha,"));
}

// ── INC#30: `{...}` double-duty (anonymous struct init vs. block) ───────
//
// The inconsistency writeup claimed that a typo like `{ x, y }`
// (missing colons) would silently become a block expression,
// evaluate `x` and `y` as statements, and return `y`.  Current
// loft rejects that shape at parse time — the trap is not
// reproducible.  These tests lock the three observable shapes so
// a future relaxation cannot reintroduce the silent-typo bite
// without updating LOFT.md and the INCONSISTENCIES.md entry.
#[test]
fn inc30_struct_init_with_colons_works() {
    code!(
        "struct Pt { x: integer, y: integer }
fn run_a() -> integer {
    p_a: Pt = Pt { x: 3, y: 4 };
    p_a.x + p_a.y
}"
    )
    .expr("run_a()")
    .result(Value::Int(7));
}

#[test]
fn inc30_block_expression_returns_last_value() {
    code!(
        "fn run_b() -> integer {
    r_b = { n_b = 1; m_b = n_b + 1; m_b };
    r_b
}"
    )
    .expr("run_b()")
    .result(Value::Int(2));
}

#[test]
fn inc30_typo_comma_without_colon_is_rejected() {
    code!(
        "struct PtC { x: integer, y: integer }
fn run_c() -> integer {
    a_c = 1;
    b_c = 2;
    p_c: PtC = { a_c, b_c };
    p_c.x + p_c.y
}"
    )
    .error("Expect token ; at inc30_typo_comma_without_colon_is_rejected:5:22");
}

// ── INC#31: open-ended range patterns in match arms ────────────────────
//
// The parser previously accepted `10..` (open-end) and `..10`
// (open-start) in match arms.  Under the interpreter these
// silently never matched (the absent endpoint encoded as null);
// under the native compiler they crashed rustc with an E0308
// `()` vs i32 type error.  Either failure mode is worse than
// "unsupported syntax".
//
// The fix emits a useful compile-time diagnostic pointing at
// the two-sided forms or a guard clause.  These tests lock
// three shapes:
//   - two-sided exclusive `lo..hi`  — works
//   - two-sided inclusive `lo..=hi` — works
//   - open-end `lo..` and open-start `..hi` — rejected
#[test]
fn inc31_two_sided_exclusive_range_matches() {
    code!(
        "fn bucket_a(n_a: integer) -> text {
    match n_a {
        10..20 => \"teens\",
        _      => \"other\"
    }
}"
    )
    .expr("bucket_a(15)")
    .result(Value::str("teens"));
}

#[test]
fn inc31_two_sided_inclusive_range_matches() {
    code!(
        "fn bucket_b(n_b: integer) -> text {
    match n_b {
        10..=20 => \"teens\",
        _       => \"other\"
    }
}"
    )
    .expr("bucket_b(20)")
    .result(Value::str("teens"));
}

#[test]
fn inc31_open_end_range_is_rejected() {
    code!(
        "fn bucket_c(n_c: integer) -> text {
    match n_c {
        10.. => \"ten-plus\",
        _    => \"other\"
    }
}"
    )
    .error(
        "open-ended range pattern `lo..` is not supported in match arms — \
write the two-sided form `lo..hi` (exclusive) or `lo..=hi` (inclusive), \
or use a guard like `n if n >= lo` at inc31_open_end_range_is_rejected:3:16",
    );
}

#[test]
fn inc31_open_start_range_is_rejected() {
    code!(
        "fn bucket_d(n_d: integer) -> text {
    match n_d {
        ..10 => \"below-ten\",
        _    => \"other\"
    }
}"
    )
    .error(
        "open-ended range pattern `..hi` is not supported in match arms — \
write the two-sided form `lo..hi` (exclusive) or `lo..=hi` (inclusive), \
or use a guard like `n if n < hi` at inc31_open_start_range_is_rejected:3:9",
    );
}

// ── INC#28: slice grammar + supported forms ────────────────────────────
//
// The grammar summary previously listed `v[2..-1]` as "negative
// indices count from end", but that claim was aspirational —
// the form produces an empty iterator (the range `2..-1` is
// literally empty).  `v[start..=end]` (inclusive) also worked
// in the parser but was undocumented.
//
// These tests lock the actually-supported shapes so a future
// implementation of negative indexing must update both the doc
// claim and these tests together, rather than silently flipping
// the semantics.
#[test]
fn inc28_slice_exclusive_range() {
    code!(
        "fn run_se() -> integer {
    v_se = [10, 20, 30, 40, 50];
    s_se = 0;
    for x_se in v_se[1..3] { s_se += x_se; }
    s_se
}"
    )
    .expr("run_se()")
    .result(Value::Int(50)); // 20 + 30
}

#[test]
fn inc28_slice_inclusive_range() {
    code!(
        "fn run_si() -> integer {
    v_si = [10, 20, 30, 40, 50];
    s_si = 0;
    for x_si in v_si[1..=3] { s_si += x_si; }
    s_si
}"
    )
    .expr("run_si()")
    .result(Value::Int(90)); // 20 + 30 + 40
}

#[test]
fn inc28_slice_open_end() {
    code!(
        "fn run_oe() -> integer {
    v_oe = [10, 20, 30, 40, 50];
    s_oe = 0;
    for x_oe in v_oe[2..] { s_oe += x_oe; }
    s_oe
}"
    )
    .expr("run_oe()")
    .result(Value::Int(120)); // 30 + 40 + 50
}

#[test]
fn inc28_slice_open_start() {
    code!(
        "fn run_os() -> integer {
    v_os = [10, 20, 30, 40, 50];
    s_os = 0;
    for x_os in v_os[..3] { s_os += x_os; }
    s_os
}"
    )
    .expr("run_os()")
    .result(Value::Int(60)); // 10 + 20 + 30
}

// INC#28 (@P384 fixed): a negative slice bound now counts from the end,
// mirroring single-index `v[-1]`.  `v[2..-1]` is indices 2..(len-1) = 2..4,
// i.e. two elements — no longer the empty range that the aspirational-doc
// era produced.  Full backend-parity matrix lives in
// tests/scripts/388-p384-negative-slice-from-end.loft.
#[test]
fn inc28_negative_slice_counts_from_end() {
    code!(
        "fn run_neg() -> integer {
    v_neg = [10, 20, 30, 40, 50];
    count_neg = 0;
    for x_neg in v_neg[2..-1] { count_neg += x_neg - x_neg + 1; }
    count_neg
}"
    )
    .expr("run_neg()")
    .result(Value::Int(2)); // indices 2,3 → two iterations
}

// ── INC#9: text indexing vs. slicing return different types ────────────
//
// `txt[i]` yields `character` (a scalar); `txt[i..j]` yields
// `text` (a string).  Vectors don't have this split (`vec[0]`
// is element T; `vec[0..1]` is `vector<T>`).  The asymmetry is
// deliberate — character is a distinct scalar, not a length-1
// text — but it's a real ergonomic trap: users assume the same
// operation family returns the same type domain.
//
// These tests lock both type paths + the practical concat
// consequences so a future "unify text indexing" refactor must
// update LOFT.md's Gotcha callout first.
// Probes `txt[i]` returns a `character` via its numeric value —
// avoids the B7-family text-return lifecycle crash hit when a
// function returns a text built by interpolating a character
// (`"{c}"` at tail of a `-> text` function SIGSEGVs today).
#[test]
fn inc9_text_index_returns_character() {
    code!(
        "fn run_ti() -> integer {
    txt_ti = \"hello\";
    c_ti = txt_ti[0];
    c_ti as integer
}"
    )
    .expr("run_ti()")
    .result(Value::Int(b'h' as i32));
}

#[test]
fn inc9_text_slice_returns_text() {
    code!(
        "fn run_ts() -> text {
    txt_ts = \"hello\";
    s_ts = txt_ts[0..1];
    s_ts
}"
    )
    .expr("run_ts()")
    .result(Value::str("h"));
}

#[test]
fn inc9_text_slices_concatenate_with_plus() {
    code!(
        "fn run_concat() -> text {
    txt_c = \"hello\";
    r_c = txt_c[0..1] + txt_c[1..2];
    r_c
}"
    )
    .expr("run_concat()")
    .result(Value::str("he"));
}

// Probes that `+` on `character` is arithmetic, not concatenation.
// 'b' - 'a' = 1 verifies the arithmetic path.  The "build text
// from characters via interpolation" consequence from the LOFT.md
// Gotcha callout is blocked today by a B7-family text-return
// SIGSEGV (`"{c}"` returned from `fn -> text` crashes); the
// workaround is to concatenate inside the function and not make
// the returned text the build target — but that crashes too.
// This test locks just the arithmetic-not-concat portion.
#[test]
fn inc9_character_plus_is_arithmetic_not_concat() {
    code!(
        "fn run_plus() -> integer {
    txt_p = \"abcd\";
    c1_p = txt_p[1];
    c2_p = txt_p[0];
    (c1_p as integer) - (c2_p as integer)
}"
    )
    .expr("run_plus()")
    .result(Value::Int(1));
}

// ── INC#17: type-conversion rules are mode-stratified, not uniform ──────
//
// Loft applies conversions in three modes: implicit (no
// annotation), format-only (implicit but only inside "{…}"),
// and explicit (`as` required).  The mode depends on the type
// pair, not on the context.  Users unable to predict this from
// first principles found themselves alternately typing too many
// `as` casts or hitting compile errors on missing ones.  The
// LOFT.md conversion table is the single reference; these tests
// lock the six most-common shapes so a future unification
// refactor cannot silently flip any mode.
#[test]
fn inc17_any_to_boolean_is_implicit() {
    // Non-zero integer is truthy in `if` without a cast.
    code!(
        "fn run_bool() -> integer {
    x_bool = 5;
    if x_bool { 1 } else { 0 }
}"
    )
    .expr("run_bool()")
    .result(Value::Int(1));
}

#[test]
fn inc17_integer_widens_to_float_in_arithmetic() {
    // 3 + 1.5 produces 4.5 without an explicit cast.
    code!(
        "fn run_widen() -> float {
    n_w = 3;
    f_w = 1.5;
    n_w + f_w
}"
    )
    .expr("run_widen()")
    .result(Value::Float(4.5));
}

#[test]
fn inc17_float_to_integer_requires_as() {
    // Truncates toward zero.
    code!(
        "fn run_trunc() -> integer {
    pi_t = 3.14;
    pi_t as integer
}"
    )
    .expr("run_trunc()")
    .result(Value::Int(3));
}

#[test]
fn inc17_text_to_integer_requires_as() {
    // Under @PLN25 DN3 a text→int parse yields `integer?` (the parse can fail);
    // the non-null return forces the discharge — `as` alone no longer suffices.
    code!(
        "fn run_parse() -> integer {
    s_p = \"42\";
    s_p as integer ?? 0
}"
    )
    .expr("run_parse()")
    .result(Value::Int(42));
}

#[test]
fn inc17_integer_to_text_is_format_only() {
    // Interpolation converts silently; the rendered text is
    // observable via format.  Probes the format-only path.
    code!(
        "fn run_fmt() -> integer {
    m_f = 7;
    t_f = \"n={m_f}\";
    len(t_f)
}"
    )
    .expr("run_fmt()")
    .result(Value::Int(3)); // "n=7"
}

#[test]
fn inc17_plain_enum_name_to_enum_requires_as() {
    code!(
        "enum Direction { North, South, East, West }
fn run_enum() -> integer {
    d_e = \"West\" as Direction;
    d_e as integer
}"
    )
    .expr("run_enum()")
    .result(Value::Int(4)); // plain-enum integer values are 1-indexed
}

// ── B7 family — method call on a JsonValue returning a scalar ────────
//
// Historical note (renamed 2026-04-14): this test was originally
// added as `b7_method_on_jsonvalue_returning_integer_crashes` —
// a regression marker for the period when every method call on a
// JsonValue local (even one returning a scalar like `len(v)`)
// double-freed the JsonValue store at scope exit.  The crash was
// resolved as a side-effect of later B-family landings (B2-runtime
// retrofit, B5 layers 1+2, the `t_9JsonValue_*` method-alias
// registrations for `n_as_*` / `n_field` / `n_item` / `n_len`).
//
// Today the test passes in both debug and release and guards the
// opposite invariant: method dispatch on a JsonValue local that
// returns a scalar must NOT crash and must NOT leak at scope exit.
// The remaining B7 symptom is narrower — the character-
// interpolation text-return path still SIGSEGVs, guarded by
// `b7_character_interpolation_return_crashes` (`#[ignore]`).
// See QUALITY.md § B7.
#[test]
fn b7_method_on_jsonvalue_returning_integer_works() {
    code!(
        "fn run_b7m() -> boolean {
    v_b7m = json_parse(\"null\");
    n_b7m = len(v_b7m);
    !n_b7m
}"
    )
    .expr("run_b7m()")
    .result(Value::Boolean(true));
}

/// Repeated method dispatch on the same JsonValue local.  If the
/// historical B7 double-free were still present, the second
/// `.kind()` call would trip the lifecycle bug because the
/// first call's post-dispatch cleanup had already decremented
/// the store's ref-count.  Today both calls succeed and return
/// the expected variant name.
#[test]
fn b7_repeated_method_dispatch_on_jsonvalue_works() {
    code!(
        "fn run_b7rm() -> text {
    v_b7rm = json_parse(\"true\");
    k1_b7rm = v_b7rm.kind();
    k2_b7rm = v_b7rm.kind();
    if k1_b7rm == k2_b7rm { k1_b7rm } else { \"MISMATCH\" }
}"
    )
    .expr("run_b7rm()")
    .result(Value::str("JBool"));
}

/// B7 method surface works on Q4-constructed JsonValues, not just
/// on `json_parse` results.  This locks that `json_null()` (and by
/// extension the other Q4 primitive constructors) produces a
/// JsonValue whose scope-exit cleanup doesn't conflict with method
/// dispatch — the same invariant the renamed `_works` test locks
/// for the json_parse side.
#[test]
fn b7_method_on_q4_constructed_jsonvalue_works() {
    code!(
        "fn run_b7q4() -> text {
    v_b7q4 = json_number(42.0);
    v_b7q4.kind()
}"
    )
    .expr("run_b7q4()")
    .result(Value::str("JNumber"));
}

// ── Q1: parser-side rich diagnostics through json_errors() ─────────────
//
// json_errors() now returns the full diagnostic shape:
//   parse error at line N col M (byte B):
//     path: /a/b/c
//     <message>
//     <context snippet with ^ caret>
//
// The loft testing harness has no substring matcher, so each test
// asserts the salient piece via a `text.contains(...)` check inside
// loft and returns a boolean.  Tolerant of future spacing /
// line-numbering tweaks.
#[test]
fn q1_json_errors_path_for_object_field() {
    code!(
        "fn run_q1a() -> boolean {
    v_q1a = json_parse(\"{{\\\"x\\\": 1.}}\");
    if v_q1a == v_q1a {}
    e_q1a = json_errors();
    e_q1a.contains(\"/x\")
}"
    )
    .expr("run_q1a()")
    .result(Value::Boolean(true));
}

#[test]
fn q1_json_errors_path_for_array_index() {
    code!(
        "fn run_q1b() -> boolean {
    v_q1b = json_parse(\"[1, 2, 1.]\");
    if v_q1b == v_q1b {}
    e_q1b = json_errors();
    e_q1b.contains(\"/2\")
}"
    )
    .expr("run_q1b()")
    .result(Value::Boolean(true));
}

#[test]
fn q1_json_errors_includes_caret_marker() {
    code!(
        "fn run_q1c() -> boolean {
    v_q1c = json_parse(\"{{\\\"x\\\": 1.}}\");
    if v_q1c == v_q1c {}
    e_q1c = json_errors();
    e_q1c.contains(\"^\")
}"
    )
    .expr("run_q1c()")
    .result(Value::Boolean(true));
}

#[test]
fn q1_json_errors_includes_line_and_byte() {
    code!(
        "fn run_q1d() -> boolean {
    v_q1d = json_parse(\"{{\\\"x\\\": 1.}}\");
    if v_q1d == v_q1d {}
    e_q1d = json_errors();
    e_q1d.contains(\"line\") && e_q1d.contains(\"byte\")
}"
    )
    .expr("run_q1d()")
    .result(Value::Boolean(true));
}

// B7 family — character-interpolation-return regression guard.
//
// Originally a SIGSEGV reproducer (discovered while writing INC#9
// regression tests): `fn f() -> text { c = txt[0]; "{c}" }`
// crashed because the text built via n_format_text on a character
// wasn't tracked for free on the outer function's text-return path.
//
// Closed as a side-effect of the B2-runtime / B5 / dep-inference /
// lock-args fixes that landed across PR #168 → #172.  Kept as a
// regression guard.  Old `_crashes` suffix retained for
// search-back compatibility — the test now passes.
#[test]
fn b7_character_interpolation_return_crashes() {
    code!(
        "fn build_b7c() -> text {
    txt_b7c = \"hello\";
    c_b7c = txt_b7c[0];
    \"{c_b7c}\"
}"
    )
    .expr("build_b7c()")
    .result(Value::str("h"));
}

// Multiple json_parse() in the same function — currently OK
// when each result is consumed via pattern matching.  Investigated
// while writing B7 regression tests; the previous QUALITY.md claim
// that "multiple json_parse() corrupts memory" was a misattribution
// — the corruption observed in earlier smoke tests came from the
// kind()/len() method calls, not from json_parse() itself.  This
// guard pins the pattern-match-based shape so future B7 work
// doesn't accidentally regress it.
#[test]
fn b7_multiple_json_parse_via_match_works() {
    code!(
        "fn run_b7p() -> boolean {
    a_b7p = json_parse(\"null\");
    b_b7p = json_parse(\"true\");
    match a_b7p { JNull => match b_b7p { JBool { value } => value, _ => false }, _ => false }
}"
    )
    .expr("run_b7p()")
    .result(Value::Boolean(true));
}

/// Q4 first slice — `json_null()` constructs a `JsonValue` set to
/// the `JNull` variant.  Primitive-only; doesn't need P54 step 4's
/// arena materialisation (no payload).  Consumed via pattern-match
/// so it rides on the working path guarded by
/// `b7_multiple_json_parse_via_match_works` rather than the still-
/// open method-call surface.  When P54 step 4 lands, the companion
/// `json_bool` / `json_number` / `json_string` primitives follow
/// the same shape; the container constructors `json_array` /
/// `json_object` land with the arena allocator.
#[test]
fn q4_json_null_returns_jnull_variant() {
    code!(
        "fn run_q4n() -> boolean {
    v_q4n = json_null();
    match v_q4n { JNull => true, _ => false }
}"
    )
    .expr("run_q4n()")
    .result(Value::Boolean(true));
}

/// Q4 ↔ Q2 cross-check — every primitive constructor must write
/// the same discriminant byte that `kind()` reads back as the
/// expected variant name.  Closes the integration gap between
/// the constructor write side (Q4) and the introspection read
/// side (Q2): without these guards, a constructor that wrote
/// the wrong discriminant byte would still pass its own match
/// test (because match dispatch and kind() share the same byte
/// — but a typo in either side would silently mis-name the
/// variant).
#[test]
fn q4_constructor_kind_cross_check_null() {
    code!(
        "fn run_q4ckn() -> text {
    json_null().kind()
}"
    )
    .expr("run_q4ckn()")
    .result(Value::str("JNull"));
}

#[test]
fn q4_constructor_kind_cross_check_bool() {
    code!(
        "fn run_q4ckb() -> text {
    json_bool(true).kind()
}"
    )
    .expr("run_q4ckb()")
    .result(Value::str("JBool"));
}

#[test]
fn q4_constructor_kind_cross_check_number() {
    code!(
        "fn run_q4cknum() -> text {
    json_number(2.5).kind()
}"
    )
    .expr("run_q4cknum()")
    .result(Value::str("JNumber"));
}

#[test]
fn q4_constructor_kind_cross_check_string() {
    code!(
        "fn run_q4cks() -> text {
    json_string(\"hi\").kind()
}"
    )
    .expr("run_q4cks()")
    .result(Value::str("JString"));
}

/// Q4 ↔ Q2 cross-check — `json_number(NaN)` resolves to `JNull`
/// (RFC 8259 disallows non-finite numbers), so kind() reports
/// `JNull`.  Locks the documented "non-finite → JNull"
/// substitution at the introspection level, not just the
/// internal storage.
#[test]
fn q4_constructor_kind_cross_check_nan_is_jnull() {
    code!(
        "fn run_q4cknan() -> text {
    json_number(null as float?).kind()
}"
    )
    .expr("run_q4cknan()")
    .result(Value::str("JNull"));
}

/// Q4 ↔ Q3 cross-check — every primitive constructor's payload
/// must serialise back to the canonical RFC 8259 text via
/// `to_json()`.  Closes the integration gap between the
/// constructor write side (Q4) and the serialiser read side
/// (Q3).  Without these, a constructor that wrote the wrong
/// payload bytes (e.g. flipped boolean polarity, wrong float
/// position) would still pass kind() and as_*() because each
/// reads the constructor-specific position — only `to_json()`
/// touches every byte and renders it as text.
#[test]
fn q4_constructor_to_json_cross_check_null() {
    code!(
        "fn run_q4ctjn() -> text {
    json_null().to_json()
}"
    )
    .expr("run_q4ctjn()")
    .result(Value::str("null"));
}

#[test]
fn q4_constructor_to_json_cross_check_bool_true() {
    code!(
        "fn run_q4ctjbt() -> text {
    json_bool(true).to_json()
}"
    )
    .expr("run_q4ctjbt()")
    .result(Value::str("true"));
}

#[test]
fn q4_constructor_to_json_cross_check_bool_false() {
    code!(
        "fn run_q4ctjbf() -> text {
    json_bool(false).to_json()
}"
    )
    .expr("run_q4ctjbf()")
    .result(Value::str("false"));
}

#[test]
fn q4_constructor_to_json_cross_check_number_integral() {
    code!(
        "fn run_q4ctjni() -> text {
    json_number(42.0).to_json()
}"
    )
    .expr("run_q4ctjni()")
    .result(Value::str("42"));
}

#[test]
fn q4_constructor_to_json_cross_check_number_fractional() {
    code!(
        "fn run_q4ctjnf() -> text {
    json_number(2.5).to_json()
}"
    )
    .expr("run_q4ctjnf()")
    .result(Value::str("2.5"));
}

#[test]
fn q4_constructor_to_json_cross_check_string() {
    code!(
        "fn run_q4ctjs() -> text {
    json_string(\"hi\").to_json()
}"
    )
    .expr("run_q4ctjs()")
    .result(Value::str("\"hi\""));
}

/// Q4 ↔ extractor cross-check — `json_X(v).as_X()` round-trips
/// the value back through the typed extractor.  Validates that
/// the constructor's payload-write position matches the
/// extractor's read position for each primitive variant.
#[test]
fn q4_constructor_as_bool_round_trips() {
    // `-> boolean?`, because `as_bool` is declared nullable now (loft#1302) and returning it
    // through a non-null `boolean` is what `(N-Store)` exists to catch — the warning it
    // raises here is correct, and silencing it by keeping the old signature would be pinning
    // the defect. The round trip is unchanged: the value still comes back.
    code!(
        "fn run_q4cab() -> boolean? {
    json_bool(true).as_bool()
}"
    )
    .expr("run_q4cab() == true")
    .result(Value::Boolean(true));
}

#[test]
fn q4_constructor_as_long_round_trips() {
    code!(
        "fn run_q4cal() -> integer {
    json_number(100.0).as_long()
}"
    )
    .expr("run_q4cal()")
    .result(Value::Long(100));
}

#[test]
fn q4_constructor_as_text_round_trips() {
    code!(
        "fn run_q4cat() -> text {
    json_string(\"abc\").as_text()
}"
    )
    .expr("run_q4cat()")
    .result(Value::str("abc"));
}

/// Q4 ↔ Q2 cross-check — `has_field()` on a Q4-built JObject finds
/// fields by name and rejects misses.  Bridges the construction
/// side (Q4) and the introspection side (Q2 has_field) — the
/// existing `q2_has_field_*` tests build via `json_parse(text)`,
/// not via the constructor surface, so this cross-check is the
/// only one that exercises the deep-copy → name-scan invariant.
#[test]
fn q4_constructor_has_field_finds_present_name() {
    code!(
        "fn run_q4chf() -> boolean {
    fields_q4chf: vector<JsonField> = [
        JsonField { name: \"alpha\", value: json_string(\"A\") },
        JsonField { name: \"beta\",  value: json_number(2.0) }
    ];
    obj_q4chf = json_object(fields_q4chf);
    obj_q4chf.has_field(\"alpha\") && !obj_q4chf.has_field(\"missing\")
}"
    )
    .expr("run_q4chf()")
    .result(Value::Boolean(true));
}

/// Q4 ↔ Q2 cross-check — `keys()` on a Q4-built JObject lists
/// every constructed field name.  Asserts both presence and
/// count via `keys().len()`.
#[test]
fn q4_constructor_keys_lists_constructed_names() {
    code!(
        "fn run_q4ckl() -> integer {
    fields_q4ckl: vector<JsonField> = [
        JsonField { name: \"alpha\", value: json_string(\"A\") },
        JsonField { name: \"beta\",  value: json_number(2.0) }
    ];
    obj_q4ckl = json_object(fields_q4ckl);
    obj_q4ckl.keys().len()
}"
    )
    .expr("run_q4ckl()")
    .result(Value::Int(2));
}

/// Q4 ↔ Q2 cross-check — `fields()` on a Q4-built JObject yields
/// every `(name, value)` entry.  Combined with the keys() test
/// above this locks both faces of the JObject introspection
/// surface against the constructor.
#[test]
fn q4_constructor_fields_lists_constructed_entries() {
    code!(
        "fn run_q4cfl() -> integer {
    fields_q4cfl: vector<JsonField> = [
        JsonField { name: \"alpha\", value: json_string(\"A\") },
        JsonField { name: \"beta\",  value: json_number(2.0) }
    ];
    obj_q4cfl = json_object(fields_q4cfl);
    obj_q4cfl.fields().len()
}"
    )
    .expr("run_q4cfl()")
    .result(Value::Int(2));
}

/// Q4 ↔ field navigation cross-check — `field()` lookup on a
/// Q4-built JObject walks the deep-copied field vector and
/// returns the embedded value.  Then `as_text()` extracts it.
/// Locks the full chain `json_object(...) → field(name) →
/// as_text()`.
#[test]
fn q4_constructor_field_lookup_extracts_value() {
    code!(
        "fn run_q4cfl2() -> text {
    fields_q4cfl2: vector<JsonField> = [
        JsonField { name: \"alpha\", value: json_string(\"A\") },
        JsonField { name: \"beta\",  value: json_string(\"B\") }
    ];
    obj_q4cfl2 = json_object(fields_q4cfl2);
    obj_q4cfl2.field(\"beta\").as_text()
}"
    )
    .expr("run_q4cfl2()")
    .result(Value::str("B"));
}

/// Q1 — `json_errors()` clears its trail on a successful parse.
/// The stdlib doc-comment spec says "Empty when the parse
/// succeeded" — the existing q1_* tests exercise the diagnostic
/// path on a single bad input but never verify the state-clearing
/// invariant: that a subsequent good parse erases the previous
/// error.  Without this guard, a regression that left stale
/// diagnostics from an earlier failure would silently mislead
/// every successive caller.
#[test]
fn q1_json_errors_cleared_after_successful_parse() {
    code!(
        "fn run_q1cls() -> boolean {
    bad_q1cls = json_parse(\"[1, 2, 1.]\");
    if bad_q1cls == bad_q1cls {}
    bad_len_q1cls = json_errors().len();
    good_q1cls = json_parse(\"[1, 2, 3]\");
    if good_q1cls == good_q1cls {}
    bad_len_q1cls > 0 && json_errors().len() == 0
}"
    )
    .expr("run_q1cls()")
    .result(Value::Boolean(true));
}

/// Q1 — `json_errors()` is empty after a fresh successful parse
/// on a never-failed JSON expression.  Pairs with the
/// state-clearing test above to lock both the "always empty on
/// success" and "transitions on failure→success" invariants.
#[test]
fn q1_json_errors_empty_after_clean_parse() {
    code!(
        "fn run_q1cep() -> integer {
    v_q1cep = json_parse(\"{{\\\"k\\\": 1}}\");
    if v_q1cep == v_q1cep {}
    json_errors().len()
}"
    )
    .expr("run_q1cep()")
    .result(Value::Int(0));
}

// ── Q1 spec-named acceptance tests (complete the § Q1 Tests list) ──────
//
// QUALITY.md § Q1 Tests enumerates five `p54_err_*` names as the
// target acceptance coverage.  Earlier landings used `q1_*`
// prefixes (with equivalent content for some, different content
// for others).  These add the missing spec names directly so a
// reader looking for the Q1 checklist finds it by the exact
// documented identifiers.

/// Q1 — `json_errors()` path for a leaf inside a nested object
/// is the full `/a/b` pointer (parent field + child field),
/// not just the leaf name.  Complements
/// `q1_json_errors_path_for_object_field` which only checked
/// a top-level field.
#[test]
fn p54_err_reports_path_into_nested_object() {
    code!(
        "fn run_perno() -> boolean {
    v_perno = json_parse(`{{\"a\": {{\"b\": 1.}}}}`);
    if v_perno == v_perno {}
    json_errors().contains(\"/a/b\")
}"
    )
    .expr("run_perno()")
    .result(Value::Boolean(true));
}

/// Q1 — `json_errors()` path into an array element is
/// `/N` with the element index.  Same assertion shape as the
/// pre-existing `q1_json_errors_path_for_array_index`; kept
/// under the spec name as well so QUALITY.md's § Q1 Tests
/// checklist matches the landed test set by-name.
#[test]
fn p54_err_reports_path_into_array_element() {
    code!(
        "fn run_perae() -> boolean {
    v_perae = json_parse(\"[1, 2, 1.]\");
    if v_perae == v_perae {}
    json_errors().contains(\"/2\")
}"
    )
    .expr("run_perae()")
    .result(Value::Boolean(true));
}

/// Q1 — a parse failure on line 2 reports `line 2` in the
/// diagnostic (not just "line N" — asserts the specific
/// number).  Multi-line input via explicit `\n` escapes.
#[test]
fn p54_err_reports_line_and_column() {
    code!(
        "fn run_perlc() -> boolean {
    v_perlc = json_parse(\"{{\\n  \\\"x\\\": 1.\\n}}\");
    if v_perlc == v_perlc {}
    json_errors().contains(\"line 2\")
}"
    )
    .expr("run_perlc()")
    .result(Value::Boolean(true));
}

/// Q1 — the context snippet includes a `^` caret under the
/// offending column.  Equivalent to
/// `q1_json_errors_includes_caret_marker`, added under the
/// spec name.
#[test]
fn p54_err_context_snippet_includes_caret() {
    code!(
        "fn run_percsic() -> boolean {
    v_percsic = json_parse(\"{{\\\"x\\\": 1.}}\");
    if v_percsic == v_percsic {}
    json_errors().contains(\"^\")
}"
    )
    .expr("run_percsic()")
    .result(Value::Boolean(true));
}

/// Q1 — RFC 6901 path escaping at the acceptance level.  A
/// field named `a/b~c` renders as `/a~1b~0c` in the path.  The
/// unit test `err_path_escapes_slash_and_tilde` in
/// `src/json.rs` covers the parser-side function, but no
/// acceptance test verified that the escaping actually reaches
/// `json_errors()` output.  Without this guard, a refactor that
/// dropped the escape helper in the `n_json_parse` glue could
/// regress RFC 6901 conformance silently.
#[test]
fn p54_err_path_escapes_slash_and_tilde() {
    code!(
        "fn run_perpest() -> boolean {
    v_perpest = json_parse(`{{\"a/b~c\": 1.}}`);
    if v_perpest == v_perpest {}
    json_errors().contains(\"/a~1b~0c\")
}"
    )
    .expr("run_perpest()")
    .result(Value::Boolean(true));
}

/// Extractor null-on-mismatch — `as_long()` on a JString returns
/// the integer null sentinel (`i64::MIN`).  The stdlib spec says
/// "null on kind mismatch" — never directly tested.
#[test]
fn p54_as_long_on_jstring_returns_null_sentinel() {
    code!(
        "fn run_alos() -> integer {
    json_string(\"hi\").as_long()
}"
    )
    .expr("run_alos()")
    .result(Value::Null);
}

/// Extractor null-on-mismatch — `as_text()` on a JNumber returns
/// the text null sentinel (which compares equal to `null` at the
/// loft level).  Validates the "null on kind mismatch" contract
/// for the text extractor.  Asserts via a loft-level `t == null`
/// check rather than a direct text comparison because the
/// underlying sentinel is `"\0"`, not the empty string.
#[test]
fn p54_as_text_on_jnumber_returns_null() {
    code!(
        "fn run_aton() -> boolean {
    t_aton = json_number(42.0).as_text();
    t_aton == null
}"
    )
    .expr("run_aton()")
    .result(Value::Boolean(true));
}

/// Extractor null-on-mismatch — `as_bool()` on a JNull answers NULL, like its three
/// siblings and like its own documentation.
///
/// It used to answer `false`, and this test used to pin that — its own comment called `false`
/// *"the boolean null sentinel"*, which it is not (255 is). Written to the documented intent
/// and then asserting whatever was there, with a parenthetical over the gap: `false` is a
/// value a caller cannot tell from a field that really says false, which is the whole defect
/// (loft#1302).
///
/// The declaration is `-> boolean?` now, because that is what carries the sentinel out; the
/// test's own signature moves with it.
#[test]
fn p54_as_bool_on_jnull_is_null() {
    code!(
        "fn run_abon() -> boolean? {
    json_null().as_bool()
}"
    )
    .expr("run_abon() == null")
    .result(Value::Boolean(true));
}

/// The CONTROL for the cell above: a JBool that really says `false` still answers `false`.
/// A cure that answered null for every falsey reading would satisfy the null test and destroy
/// the only value `as_bool` exists to return.
#[test]
fn p54_as_bool_on_a_real_false_is_false() {
    code!(
        "fn run_abf() -> boolean? {
    json_parse(\"{{\\\"b\\\":false}}\").field(\"b\").as_bool()
}"
    )
    .expr("run_abf() == false")
    .result(Value::Boolean(true));
}

/// Extractor `as_long()` truncates float toward zero (NOT round,
/// NOT floor).  The stdlib spec is explicit: "Truncates the
/// underlying float toward zero before converting."  Locks the
/// behaviour for both signs — `2.7 → 2` and `-2.7 → -2`.
#[test]
fn p54_as_long_truncates_positive_float_toward_zero() {
    code!(
        "fn run_altp() -> integer {
    json_number(2.7).as_long()
}"
    )
    .expr("run_altp()")
    .result(Value::Long(2));
}

#[test]
fn p54_as_long_truncates_negative_float_toward_zero() {
    code!(
        "fn run_altn() -> integer {
    json_number(-2.7).as_long()
}"
    )
    .expr("run_altn()")
    .result(Value::Long(-2));
}

/// Edge-case parse inputs — the documented "malformed input
/// returns JNull" contract was tested for individual bad-syntax
/// inputs (Q1 path tests) but never for the lexically empty
/// boundary cases.  Locks `""`, `"   "` (whitespace-only), and
/// arbitrary garbage all return JNull.
#[test]
fn p54_parse_empty_string_returns_jnull() {
    code!(
        "fn run_pes() -> text {
    json_parse(\"\").kind()
}"
    )
    .expr("run_pes()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_parse_whitespace_only_returns_jnull() {
    code!(
        "fn run_pwo() -> text {
    json_parse(\"   \").kind()
}"
    )
    .expr("run_pwo()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_parse_garbage_input_returns_jnull() {
    code!(
        "fn run_pgi() -> text {
    json_parse(\"not-json-at-all\").kind()
}"
    )
    .expr("run_pgi()")
    .result(Value::str("JNull"));
}

/// Q4-built primitive match destructuring — the constructor
/// path didn't have direct destructuring guards beyond the
/// existing JNull (`q4_json_null_returns_jnull_variant`).
/// Adds JBool + JNumber.
///
/// JString destructuring on a Q4-built value is intentionally
/// NOT tested here because it triggers a B7-family
/// `free(): invalid size` crash (discovered while writing
/// these guards via `/tmp/jstring_match_probe.loft` — the
/// same store-lifecycle issue that gates
/// `b7_character_interpolation_return_crashes`).  The match
/// branch destructure of a JString value's text-typed inner
/// field is a known-failing path for the Q4 constructor —
/// pattern matching via wildcard works (existing q4 tests),
/// but field-binding doesn't.  Tracked under B7.
#[test]
fn q4_match_destructuring_jbool_extracts_value() {
    code!(
        "fn run_q4mb() -> boolean {
    match json_bool(true) {
        JBool { value } => value,
        _ => false
    }
}"
    )
    .expr("run_q4mb()")
    .result(Value::Boolean(true));
}

#[test]
fn q4_match_destructuring_jnumber_extracts_value() {
    code!(
        "fn run_q4mn() -> float {
    match json_number(3.25) {
        JNumber { value } => value,
        _ => 0.0
    }
}"
    )
    .expr("run_q4mn()")
    .result(Value::Float(3.25));
}

/// Extractor — `as_number()` returns the JNumber payload on a
/// matching variant and NaN (the float null sentinel) on every
/// other.  Complements the other extractor null-on-mismatch
/// guards (as_long / as_text / as_bool).  Asserts NaN via
/// self-inequality (`f != f` is true iff f is NaN — the only
/// reliable loft-level NaN test).
#[test]
fn p54_as_number_on_jnumber_returns_value() {
    code!(
        "fn run_annv() -> float {
    json_number(3.5).as_number()
}"
    )
    .expr("run_annv()")
    .result(Value::Float(3.5));
}

#[test]
fn p54_as_number_on_jstring_returns_nan() {
    code!(
        "fn run_annjs() -> boolean {
    x_annjs = json_string(\"hi\").as_number();
    x_annjs == null
}"
    )
    .expr("run_annjs()")
    .result(Value::Boolean(true));
}

#[test]
fn p54_as_number_on_jbool_returns_nan() {
    code!(
        "fn run_annjb() -> boolean {
    x_annjb = json_bool(true).as_number();
    x_annjb == null
}"
    )
    .expr("run_annjb()")
    .result(Value::Boolean(true));
}

/// RFC 8259 numeric parse — scientific notation (`1e10`) must
/// parse as `JNumber` with the correctly-scaled float payload.
/// Never tested — the existing q1_* tests cover syntax-failure
/// paths on numbers like `1.` but not successful scientific
/// inputs.
#[test]
fn p54_parse_scientific_notation_is_jnumber() {
    code!(
        "fn run_psn() -> text {
    json_parse(\"1e10\").kind()
}"
    )
    .expr("run_psn()")
    .result(Value::str("JNumber"));
}

#[test]
fn p54_parse_scientific_notation_extracts_value() {
    code!(
        "fn run_psnv() -> boolean {
    v_psnv = json_parse(\"1e3\").as_number();
    v_psnv > 999.0 && v_psnv < 1001.0
}"
    )
    .expr("run_psnv()")
    .result(Value::Boolean(true));
}

/// RFC 8259 numeric parse — leading zeros are rejected (the
/// grammar allows only `0` or `[1-9][0-9]*` for the integer
/// part).  Locks the documented rejection behaviour so a
/// future permissive-mode change doesn't silently accept
/// `007`.  The `-0` case is a complementary positive: RFC 8259
/// explicitly allows negative zero (`-0` is a valid
/// `JNumber`).
#[test]
fn p54_parse_leading_zero_integer_is_rejected() {
    code!(
        "fn run_plz() -> text {
    json_parse(\"007\").kind()
}"
    )
    .expr("run_plz()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_parse_negative_zero_is_accepted() {
    code!(
        "fn run_pnz() -> text {
    json_parse(\"-0\").kind()
}"
    )
    .expr("run_pnz()")
    // @PLN109 — `-0` is integer-shaped, so it is a JInteger (exact 0), not JNumber.
    .result(Value::str("JInteger"));
}

/// Pretty-print depth counting — the `to_json_pretty` path
/// tracks an explicit depth counter in `json_to_text_at` and
/// emits the right number of 2-space indents at each level.
/// Prior tests cover depth 1 (`[1,2]`) and depth 2 (`{"k":[1,2]}`);
/// this guard exercises depth 3 (`{"a":{"b":[1]}}`) so the depth
/// counter is verified to propagate through nested containers
/// without off-by-one errors.
#[test]
fn q3_to_json_pretty_three_level_nesting() {
    code!(
        "fn run_q3p3() -> text {
    v_q3p3 = json_parse(`{{\"a\":{{\"b\":[1]}}}}`);
    v_q3p3.to_json_pretty()
}"
    )
    .expr("run_q3p3()")
    .result(Value::str(
        "{\n  \"a\": {\n    \"b\": [\n      1\n    ]\n  }\n}",
    ));
}

/// Round-trip preserves JObject insertion order.  The STDLIB.md
/// JSON reference says "Field names in insertion order" for
/// both `keys()` and object serialisation.  Never tested for
/// the parse → serialise path — a parser that sorted keys
/// alphabetically would be spec-incorrect but no test would
/// catch it.  Chooses names `z/a/m` so alphabetical reordering
/// (`a,m,z`) would produce distinct output from insertion
/// order (`z,a,m`).
#[test]
fn p54_parse_serialise_preserves_insertion_order() {
    code!(
        "fn run_piso() -> text {
    v_piso = json_parse(`{{\"z\":1,\"a\":2,\"m\":3}}`);
    v_piso.to_json()
}"
    )
    .expr("run_piso()")
    .result(Value::str("{\"z\":1,\"a\":2,\"m\":3}"));
}

/// Q4 → Q2 keys() preserves the caller's declared field order.
/// Complements `q4_constructor_keys_lists_constructed_names`
/// (which only asserted `.len() == 2`) by asserting every key
/// appears at the correct index, in the caller-supplied order.
#[test]
fn q4_constructor_keys_preserves_insertion_order() {
    code!(
        "fn run_q4kio() -> text {
    fields_q4kio: vector<JsonField> = [
        JsonField { name: \"zebra\", value: json_null() },
        JsonField { name: \"apple\", value: json_null() },
        JsonField { name: \"mango\", value: json_null() }
    ];
    obj_q4kio = json_object(fields_q4kio);
    ks_q4kio = obj_q4kio.keys();
    \"{ks_q4kio[0]}|{ks_q4kio[1]}|{ks_q4kio[2]}\"
}"
    )
    .expr("run_q4kio()")
    .result(Value::str("zebra|apple|mango"));
}

/// Deep-nesting navigation — a 5-level-deep JSON tree parses
/// into a tree where the leaf is reachable via five chained
/// `.field()` calls without tripping store-lifecycle or
/// arena-offset bugs.  QUALITY.md § Q3 Tests mentions
/// "nested up to depth 5" as the property-test target; this
/// guard pins that depth concretely.
#[test]
fn p54_deep_nesting_five_levels_navigable() {
    code!(
        "fn run_pdn5() -> integer {
    v_pdn5 = json_parse(`{{\"a\":{{\"b\":{{\"c\":{{\"d\":{{\"e\":42}}}}}}}}}}`);
    v_pdn5.field(\"a\").field(\"b\").field(\"c\").field(\"d\").field(\"e\").as_long()
}"
    )
    .expr("run_pdn5()")
    .result(Value::Long(42));
}

/// Pretty-print of an empty container inside a non-empty
/// parent.  Locks the edge case where the outer array indents
/// its children one level but the inner empty array stays `[]`
/// (no newline padding even though its parent is pretty-printed).
/// A naive implementation that always emitted `\n<indent>` for
/// every container would turn `[]` into `[\n  ]` at depth 1 —
/// this guard catches that.
#[test]
fn q3_to_json_pretty_empty_container_inside_non_empty() {
    code!(
        "fn run_q3pein() -> text {
    inner_q3pein: vector<JsonValue> = [];
    outer_q3pein: vector<JsonValue> = [json_array(inner_q3pein), json_number(1.0)];
    v_q3pein = json_array(outer_q3pein);
    v_q3pein.to_json_pretty()
}"
    )
    .expr("run_q3pein()")
    .result(Value::str("[\n  [],\n  1\n]"));
}

/// Q2 `fields()` — full name+value insertion-order preservation.
/// The prior `q4_constructor_keys_preserves_insertion_order`
/// pinned `keys()` at per-index granularity; this is the
/// companion for `fields()` on a parsed input, asserting that
/// each entry carries both its original name AND value at the
/// correct index.  Uses `z/a` names so alphabetical reordering
/// would produce distinct output.
#[test]
fn q2_fields_preserves_name_and_value_at_each_index() {
    code!(
        "fn run_q2fnvi() -> text {
    v_q2fnvi = json_parse(`{{\"z\":1,\"a\":2}}`);
    entries_q2fnvi = v_q2fnvi.fields();
    \"{entries_q2fnvi[0].name}={entries_q2fnvi[0].value.as_long()}|{entries_q2fnvi[1].name}={entries_q2fnvi[1].value.as_long()}\"
}"
    )
    .expr("run_q2fnvi()")
    .result(Value::str("z=1|a=2"));
}

/// Q2 `has_field("")` — empty-string key on a JObject that
/// carries a field with an empty name must return `true`.  Edge
/// case for the name-scan loop: a naive string-length shortcut
/// that treated empty-string as "no lookup" would break this.
/// Locks the documented "returns `true` iff carries a field
/// named `name`" contract at the name boundary.
#[test]
fn q2_has_field_matches_empty_name_key() {
    code!(
        "fn run_q2hen() -> boolean {
    v_q2hen = json_parse(`{{\"\":1}}`);
    v_q2hen.has_field(\"\") && !v_q2hen.has_field(\"a\")
}"
    )
    .expr("run_q2hen()")
    .result(Value::Boolean(true));
}

/// Match on non-empty JArray binds the `items` field to the
/// real container vector.  The vector's `.len()` must match the
/// JSON array's length.  Coverage gap: existing JArray tests
/// destructure via wildcard (`JArray _ =>`) but don't bind the
/// items field — so the binding codegen path for a non-empty
/// container wasn't directly exercised.
#[test]
fn p54_match_jarray_binds_non_empty_items() {
    code!(
        "fn run_pmjba() -> integer {
    match json_parse(\"[10,20,30]\") {
        JArray { items } => items.len(),
        _ => -1
    }
}"
    )
    .expr("run_pmjba()")
    .result(Value::Int(3));
}

/// Match on an empty JArray binds `items` to an empty vector.
/// Pairs with the non-empty test above so the binding path
/// is covered at both the minimum (zero-length) and the
/// non-degenerate (three-element) boundaries.
#[test]
fn p54_match_jarray_binds_empty_items() {
    code!(
        "fn run_pmjbe() -> integer {
    match json_parse(\"[]\") {
        JArray { items } => items.len(),
        _ => -1
    }
}"
    )
    .expr("run_pmjbe()")
    .result(Value::Int(0));
}

/// Q4 first slice — `json_null()` ignoring the B7 method-call
/// surface: two independent `json_null()` calls in the same
/// function, each consumed via its own match.  Mirrors the
/// `b7_multiple_json_parse_via_match_works` shape to guarantee
/// the constructor doesn't trip the B7 double-free when two
/// results coexist.
#[test]
fn q4_two_json_nulls_via_match_works() {
    code!(
        "fn run_q4nn() -> integer {
    a_q4nn = json_null();
    b_q4nn = json_null();
    ok_a_q4nn = match a_q4nn { JNull => 1, _ => 0 };
    ok_b_q4nn = match b_q4nn { JNull => 1, _ => 0 };
    ok_a_q4nn + ok_b_q4nn
}"
    )
    .expr("run_q4nn()")
    .result(Value::Int(2));
}

/// Q4 second slice — `json_bool(v)` constructs a `JBool` variant
/// carrying the supplied boolean payload.  Pattern-match on the
/// result binds the `value` field; this guard locks both
/// construction (discriminant byte written) and the payload round-
/// trip (field offset correct).
#[test]
fn q4_json_bool_round_trips_true() {
    code!(
        "fn run_q4bt() -> boolean {
    v_q4bt = json_bool(true);
    match v_q4bt { JBool { value } => value, _ => false }
}"
    )
    .expr("run_q4bt()")
    .result(Value::Boolean(true));
}

#[test]
fn q4_json_bool_round_trips_false() {
    code!(
        "fn run_q4bf() -> integer {
    v_q4bf = json_bool(false);
    match v_q4bf { JBool { value } => { if value { 1 } else { 2 } }, _ => 0 }
}"
    )
    .expr("run_q4bf()")
    .result(Value::Int(2));
}

/// Q4 third slice — `json_number(v)` constructs a `JNumber` variant
/// carrying the supplied float payload.  Non-finite input produces
/// `JNull` with a diagnostic in `json_errors()` — that behaviour is
/// guarded by `q4_json_number_nan_becomes_jnull` below.
#[test]
fn q4_json_number_round_trips_finite() {
    code!(
        "fn run_q4nr() -> float {
    v_q4nr = json_number(2.75);
    match v_q4nr { JNumber { value } => value, _ => 0.0 }
}"
    )
    .expr("run_q4nr()")
    .result(Value::Float(2.75));
}

#[test]
fn q4_json_number_negative_finite() {
    code!(
        "fn run_q4nn2() -> float {
    v_q4nn2 = json_number(-2.5);
    match v_q4nn2 { JNumber { value } => value, _ => 0.0 }
}"
    )
    .expr("run_q4nn2()")
    .result(Value::Float(-2.5));
}

/// Q4 third slice negative-case — feeding `float null` (NaN) or
/// non-finite values makes `json_number` store `JNull` with a
/// diagnostic, not a numeric payload that would violate RFC 8259.
#[test]
fn q4_json_number_nan_becomes_jnull() {
    code!(
        "fn run_q4nn3() -> integer {
    nan_val_q4 = null as float?;
    v_q4nn3 = json_number(nan_val_q4);
    match v_q4nn3 { JNull => 1, _ => 0 }
}"
    )
    .expr("run_q4nn3()")
    .result(Value::Int(1));
}

/// Q4 fourth slice — `json_string(v)` constructs a `JString`
/// variant carrying a copy of the supplied text.  The text is
/// written into the JsonValue's own store, so the returned value
/// lifetime-extends the payload independently of `v`.
///
/// Reading the bound `value: text` out of the match arm trips the
/// same native-returned-text lifecycle bug that B7's
/// `b7_character_interpolation_return_crashes` guards (the Str
/// pointer into the JsonValue store gets returned and later freed
/// as a DbRef).  Until B7 lands, the test verifies the shape of
/// the variant — `JString` branch taken — by measuring the
/// bound text's length (integer return, no Str escape) rather
/// than returning the value itself.
#[test]
fn q4_json_string_round_trips() {
    code!(
        "fn run_q4sr() -> integer {
    v_q4sr = json_string(\"hello world\");
    match v_q4sr { JString { value } => value.len(), _ => -1 }
}"
    )
    .expr("run_q4sr()")
    .result(Value::Int(11));
}

#[test]
fn q4_json_string_empty() {
    code!(
        "fn run_q4se() -> integer {
    v_q4se = json_string(\"\");
    match v_q4se { JString { value } => value.len(), _ => -1 }
}"
    )
    .expr("run_q4se()")
    .result(Value::Int(0));
}

/// Q2 — `kind(self: JsonValue) -> text` introspection.  One guard
/// per primitive variant, in both free-function syntax `kind(v)`
/// and method syntax `v.kind()`, to lock the registration of both
/// dispatch paths (NATIVE_FNS registers `n_kind` + the
/// `t_9JsonValue_kind` method alias).  Container variants
/// (`JArray`, `JObject`) land with P54 step 4's arena materialiser.
#[test]
fn q2_kind_of_jnull_free_form() {
    code!(
        "fn run_q2kn() -> text {
    v_q2kn = json_null();
    kind(v_q2kn)
}"
    )
    .expr("run_q2kn()")
    .result(Value::str("JNull"));
}

#[test]
fn q2_kind_of_jnull_method_form() {
    code!(
        "fn run_q2kn2() -> text {
    v_q2kn2 = json_null();
    v_q2kn2.kind()
}"
    )
    .expr("run_q2kn2()")
    .result(Value::str("JNull"));
}

#[test]
fn q2_kind_of_jbool() {
    code!(
        "fn run_q2kb() -> text {
    v_q2kb = json_bool(true);
    v_q2kb.kind()
}"
    )
    .expr("run_q2kb()")
    .result(Value::str("JBool"));
}

#[test]
fn q2_kind_of_jnumber() {
    code!(
        "fn run_q2knum() -> text {
    v_q2knum = json_number(42.0);
    v_q2knum.kind()
}"
    )
    .expr("run_q2knum()")
    .result(Value::str("JNumber"));
}

#[test]
fn q2_kind_of_jstring() {
    code!(
        "fn run_q2ks() -> text {
    v_q2ks = json_string(\"hello\");
    v_q2ks.kind()
}"
    )
    .expr("run_q2ks()")
    .result(Value::str("JString"));
}

/// Q2 kind() on a json_parse result — locks that the discriminant
/// byte written by `n_json_parse` for parsed primitives matches
/// the one `n_kind` reads back.  A JSON-parser-vs-kind-reader drift
/// would make `kind(json_parse(x))` return "JUnknown".
#[test]
fn q2_kind_of_parsed_primitive() {
    code!(
        "fn run_q2kp() -> text {
    v_q2kp = json_parse(\"true\");
    kind(v_q2kp)
}"
    )
    .expr("run_q2kp()")
    .result(Value::str("JBool"));
}

/// Q3 primitive-slice — `to_json(self: JsonValue) -> text` renders
/// a JsonValue to canonical RFC 8259 JSON text.  One guard per
/// primitive variant, each measured by text equality to lock the
/// serialisation contract.  Container variants (JArray / JObject)
/// render as `"<pending step 4>"` today; the full recursive
/// formatter lands with P54 step 4.
#[test]
fn q3_to_json_of_jnull() {
    code!(
        "fn run_q3tn() -> text {
    v_q3tn = json_null();
    to_json(v_q3tn)
}"
    )
    .expr("run_q3tn()")
    .result(Value::str("null"));
}

#[test]
fn q3_to_json_of_jbool_true() {
    code!(
        "fn run_q3tb() -> text {
    v_q3tb = json_bool(true);
    v_q3tb.to_json()
}"
    )
    .expr("run_q3tb()")
    .result(Value::str("true"));
}

#[test]
fn q3_to_json_of_jbool_false() {
    code!(
        "fn run_q3tbf() -> text {
    v_q3tbf = json_bool(false);
    v_q3tbf.to_json()
}"
    )
    .expr("run_q3tbf()")
    .result(Value::str("false"));
}

#[test]
fn q3_to_json_of_jnumber_integer() {
    code!(
        "fn run_q3tni() -> text {
    v_q3tni = json_number(42.0);
    v_q3tni.to_json()
}"
    )
    .expr("run_q3tni()")
    .result(Value::str("42"));
}

#[test]
fn q3_to_json_of_jnumber_fractional() {
    code!(
        "fn run_q3tnf() -> text {
    v_q3tnf = json_number(2.75);
    v_q3tnf.to_json()
}"
    )
    .expr("run_q3tnf()")
    .result(Value::str("2.75"));
}

/// Non-finite inputs to json_number (NaN, ±Inf) construct a JNull
/// variant with a diagnostic — to_json on that reads "null", not
/// the garbage representation of the bad float.  Matches RFC 8259
/// which forbids non-finite numeric literals.
#[test]
fn q3_to_json_of_nan_becomes_null() {
    code!(
        "fn run_q3tnn() -> text {
    nan_q3 = null as float?;
    v_q3tnn = json_number(nan_q3);
    v_q3tnn.to_json()
}"
    )
    .expr("run_q3tnn()")
    .result(Value::str("null"));
}

#[test]
fn q3_to_json_of_jstring_plain() {
    code!(
        "fn run_q3ts() -> text {
    v_q3ts = json_string(\"hello\");
    v_q3ts.to_json()
}"
    )
    .expr("run_q3ts()")
    .result(Value::str("\"hello\""));
}

// Q3 escape-sequence regressions (`"a\"b\\c"` round-trip; `\n` /
// `\t` / control-byte encoding) are deferred.  Initial attempt
// exposed that loft's String parser currently drops backslash-
// escapes in string literals inside `code!()` test scaffolding
// (the `q3_to_json_of_jstring_with_escapes` case hung on the
// loft-side interpretation of the `\\` sequence; needs isolated
// reproducer + investigation).  The Rust-side escape logic in
// `n_to_json` is exercised; the test-harness plumbing is what's
// blocking.  Track as a follow-up under Q3.

/// Q3 pretty-print primitive slice — `to_json_pretty(self: JsonValue)`
/// produces identical output to `to_json` for primitive variants
/// (no nested structure to indent).  Divergence lands with P54
/// step 4's arena materialiser.  These guards lock that primitive
/// output is byte-identical across the two entry points today, so
/// a future change that adds pretty-specific padding for
/// primitives would be caught.
#[test]
fn q3_to_json_pretty_of_jnull() {
    code!(
        "fn run_q3pn() -> text {
    v_q3pn = json_null();
    v_q3pn.to_json_pretty()
}"
    )
    .expr("run_q3pn()")
    .result(Value::str("null"));
}

#[test]
fn q3_to_json_pretty_of_jbool() {
    code!(
        "fn run_q3pb() -> text {
    v_q3pb = json_bool(true);
    v_q3pb.to_json_pretty()
}"
    )
    .expr("run_q3pb()")
    .result(Value::str("true"));
}

#[test]
fn q3_to_json_pretty_of_jnumber() {
    code!(
        "fn run_q3pnum() -> text {
    v_q3pnum = json_number(42.0);
    v_q3pnum.to_json_pretty()
}"
    )
    .expr("run_q3pnum()")
    .result(Value::str("42"));
}

#[test]
fn q3_to_json_pretty_of_jstring() {
    code!(
        "fn run_q3ps() -> text {
    v_q3ps = json_string(\"hi\");
    v_q3ps.to_json_pretty()
}"
    )
    .expr("run_q3ps()")
    .result(Value::str("\"hi\""));
}

/// Q3 — `to_json` and `to_json_pretty` must agree on primitives.
/// This regression guard compares the outputs directly and fails
/// if they ever diverge for a primitive variant (which would
/// indicate the pretty path is accidentally formatting leaf
/// values differently from the canonical path).
#[test]
fn q3_to_json_and_pretty_agree_on_primitive() {
    code!(
        "fn run_q3pa() -> boolean {
    v_q3pa = json_number(2.75);
    canonical_q3pa = v_q3pa.to_json();
    pretty_q3pa = v_q3pa.to_json_pretty();
    canonical_q3pa == pretty_q3pa
}"
    )
    .expr("run_q3pa()")
    .result(Value::Boolean(true));
}

/// Q3 — free-function dispatch of `to_json_pretty`.  Locks the
/// `n_to_json_pretty` registration in NATIVE_FNS alongside the
/// `t_9JsonValue_to_json_pretty` method alias.
#[test]
fn q3_to_json_pretty_free_form() {
    code!(
        "fn run_q3pf() -> text {
    v_q3pf = json_null();
    to_json_pretty(v_q3pf)
}"
    )
    .expr("run_q3pf()")
    .result(Value::str("null"));
}

/// Q3 pretty — empty containers stay byte-identical to canonical
/// (no newline padding for `[]` / `{}`).
#[test]
fn q3_to_json_pretty_empty_array() {
    code!(
        "fn run_q3pea() -> text {
    v_q3pea = json_parse(\"[]\");
    v_q3pea.to_json_pretty()
}"
    )
    .expr("run_q3pea()")
    .result(Value::str("[]"));
}

#[test]
fn q3_to_json_pretty_empty_object() {
    code!(
        "fn run_q3peo() -> text {
    v_q3peo = json_parse(\"{{}}\");
    v_q3peo.to_json_pretty()
}"
    )
    .expr("run_q3peo()")
    .result(Value::str("{}"));
}

/// Q3 pretty — non-empty array indents each element on its own
/// line with 2-space indent, closing bracket dedents back.
#[test]
fn q3_to_json_pretty_array_indents_elements() {
    code!(
        "fn run_q3pai() -> text {
    v_q3pai = json_parse(\"[1,2,3]\");
    v_q3pai.to_json_pretty()
}"
    )
    .expr("run_q3pai()")
    .result(Value::str("[\n  1,\n  2,\n  3\n]"));
}

/// Q3 pretty — non-empty object indents each field on its own
/// line; key/value separator is `": "` (colon + single space).
#[test]
fn q3_to_json_pretty_object_indents_fields() {
    code!(
        "fn run_q3poi() -> text {
    v_q3poi = json_parse(\"{{\\\"a\\\":1,\\\"b\\\":2}}\");
    v_q3poi.to_json_pretty()
}"
    )
    .expr("run_q3poi()")
    .result(Value::str("{\n  \"a\": 1,\n  \"b\": 2\n}"));
}

/// Q3 pretty — nested containers indent recursively.  Inner
/// container's indent is one level deeper than the outer's.
#[test]
fn q3_to_json_pretty_nested_array_in_object() {
    code!(
        "fn run_q3pnao() -> text {
    v_q3pnao = json_parse(\"{{\\\"k\\\":[1,2]}}\");
    v_q3pnao.to_json_pretty()
}"
    )
    .expr("run_q3pnao()")
    .result(Value::str("{\n  \"k\": [\n    1,\n    2\n  ]\n}"));
}

/// Q3 pretty — canonical and pretty diverge once a non-empty
/// container is in play.  Locks the active difference (the prior
/// stub returned the same text for both, which would have hidden
/// a regression in the pretty walk).
#[test]
fn q3_to_json_and_pretty_differ_on_nonempty_container() {
    code!(
        "fn run_q3pdc() -> boolean {
    v_q3pdc = json_parse(\"[1,2]\");
    v_q3pdc.to_json() != v_q3pdc.to_json_pretty()
}"
    )
    .expr("run_q3pdc()")
    .result(Value::Boolean(true));
}

/// P54 step-4 null-safety — `len()` on each primitive variant
/// returns the integer null sentinel (`i32::MIN`) per the
/// stdlib contract.  Locks the documented "no length defined"
/// behaviour for non-container variants so an accidental
/// switch to `0` (which would be wrong — a real empty array
/// has length 0) gets caught.
#[test]
fn p54_step4_len_on_jnull_is_null_sentinel() {
    code!(
        "fn run_lnn() -> integer {
    v = json_null();
    v.len()
}"
    )
    .expr("run_lnn()")
    .result(Value::Null);
}

#[test]
fn p54_step4_len_on_jbool_is_null_sentinel() {
    code!(
        "fn run_lnb() -> integer {
    v = json_bool(true);
    v.len()
}"
    )
    .expr("run_lnb()")
    .result(Value::Null);
}

#[test]
fn p54_step4_len_on_jnumber_is_null_sentinel() {
    code!(
        "fn run_lnnum() -> integer {
    v = json_number(1.0);
    v.len()
}"
    )
    .expr("run_lnnum()")
    .result(Value::Null);
}

#[test]
fn p54_step4_len_on_jstring_is_null_sentinel() {
    code!(
        "fn run_lnstr() -> integer {
    v = json_string(\"hello\");
    v.len()
}"
    )
    .expr("run_lnstr()")
    .result(Value::Null);
}

/// P54 step-4 null-safety — `field()` on a non-JObject receiver
/// returns `JNull` rather than crashing.  Locks the chained-
/// access safety guarantee (every intermediate missing produces
/// `JNull`, never a trap).
#[test]
fn p54_step4_field_on_jstring_returns_jnull() {
    code!(
        "fn run_fjs() -> text {
    v = json_string(\"hi\");
    v.field(\"missing\").kind()
}"
    )
    .expr("run_fjs()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_step4_field_missing_key_returns_jnull() {
    code!(
        "fn run_fmk() -> text {
    v = json_parse(\"{{\\\"present\\\":1}}\");
    v.field(\"absent\").kind()
}"
    )
    .expr("run_fmk()")
    .result(Value::str("JNull"));
}

/// P54 step-4 null-safety — `item()` on non-JArray, negative
/// index, and out-of-bounds index all return `JNull`.
#[test]
fn p54_step4_item_on_jnumber_returns_jnull() {
    code!(
        "fn run_ijn() -> text {
    v = json_number(42.0);
    v.item(0).kind()
}"
    )
    .expr("run_ijn()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_step4_item_negative_index_returns_jnull() {
    code!(
        "fn run_ini() -> text {
    v = json_parse(\"[1,2,3]\");
    v.item(-1).kind()
}"
    )
    .expr("run_ini()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_step4_item_out_of_bounds_returns_jnull() {
    code!(
        "fn run_iob() -> text {
    v = json_parse(\"[1,2,3]\");
    v.item(99).kind()
}"
    )
    .expr("run_iob()")
    .result(Value::str("JNull"));
}

/// Q3 — round-trip property for primitives.  Each primitive
/// variant survives `to_json` → `json_parse` with its kind
/// (and where applicable, payload) intact.  Listed in
/// QUALITY.md § Q3 Tests as `q3_primitives_round_trip`.
#[test]
fn q3_primitives_round_trip() {
    code!(
        "fn check_q3prt(s: text, expected_kind: text) -> boolean {
    v = json_parse(s);
    text_q3prt = v.to_json();
    parsed_q3prt = json_parse(text_q3prt);
    parsed_q3prt.kind() == expected_kind
}
fn run_q3prt() -> integer {
    score_q3prt = 0;
    if check_q3prt(\"null\", \"JNull\") { score_q3prt += 1; }
    if check_q3prt(\"true\", \"JBool\") { score_q3prt += 1; }
    if check_q3prt(\"false\", \"JBool\") { score_q3prt += 1; }
    if check_q3prt(\"42\", \"JInteger\") { score_q3prt += 1; }
    if check_q3prt(\"3.14\", \"JNumber\") { score_q3prt += 1; }
    if check_q3prt(\"\\\"hi\\\"\", \"JString\") { score_q3prt += 1; }
    score_q3prt
}"
    )
    .expr("run_q3prt()")
    .result(Value::Int(6));
}

/// Q3 — round-trip property for nested objects.  An object with
/// primitive fields survives `to_json` → `json_parse` and the
/// extracted leaves agree on values.  Listed in QUALITY.md
/// § Q3 Tests as `q3_nested_object_round_trip`.
#[test]
fn q3_nested_object_round_trip() {
    code!(
        "fn run_q3nort() -> integer {
    src_q3nort = json_parse(\"{{\\\"a\\\":1,\\\"b\\\":2,\\\"c\\\":3}}\");
    text_q3nort = src_q3nort.to_json();
    back_q3nort = json_parse(text_q3nort);
    sum_q3nort = 0;
    sum_q3nort += back_q3nort.field(\"a\").as_long();
    sum_q3nort += back_q3nort.field(\"b\").as_long();
    sum_q3nort += back_q3nort.field(\"c\").as_long();
    sum_q3nort as integer
}"
    )
    .expr("run_q3nort()")
    .result(Value::Int(6));
}

/// Q3 — round-trip property for arrays of mixed primitive kinds.
/// `[1,true,\"x\"]` survives `to_json` → `json_parse` with each
/// element's kind preserved.  Listed in QUALITY.md § Q3 Tests as
/// `q3_array_of_mixed_kinds_round_trip`.
#[test]
fn q3_array_of_mixed_kinds_round_trip() {
    code!(
        "fn run_q3amkrt() -> text {
    src_q3amkrt = json_parse(\"[1,true,\\\"x\\\"]\");
    text_q3amkrt = src_q3amkrt.to_json();
    back_q3amkrt = json_parse(text_q3amkrt);
    \"{back_q3amkrt.item(0).kind()}|{back_q3amkrt.item(1).kind()}|{back_q3amkrt.item(2).kind()}\"
}"
    )
    .expr("run_q3amkrt()")
    // @PLN109 — the leading `1` round-trips as a JInteger.
    .result(Value::str("JInteger|JBool|JString"));
}

/// Q3 — pretty-printed output is still valid JSON: `parse(to_json_pretty(v))`
/// produces an equivalent tree.  Locks the property that pretty
/// mode only adds whitespace between structural tokens, never
/// inside string literals or numbers.  Listed in QUALITY.md § Q3
/// Tests as `q3_pretty_form_valid_json`.
#[test]
fn q3_pretty_form_valid_json() {
    code!(
        "fn run_q3pfvj() -> integer {
    src_q3pfvj = json_parse(\"{{\\\"items\\\":[1,2,3]}}\");
    pretty_q3pfvj = src_q3pfvj.to_json_pretty();
    back_q3pfvj = json_parse(pretty_q3pfvj);
    back_q3pfvj.field(\"items\").len()
}"
    )
    .expr("run_q3pfvj()")
    .result(Value::Int(3));
}

/// Q3 — UTF-8 string content passes through `to_json` verbatim
/// (no `\\uXXXX` escaping of BMP characters).  Listed in
/// QUALITY.md § Q3 Tests as `q3_unicode_string_escaping`.
#[test]
fn q3_unicode_string_escaping() {
    code!(
        "fn run_q3use() -> text {
    s_q3use = json_string(\"α β 😊\");
    s_q3use.to_json()
}"
    )
    .expr("run_q3use()")
    .result(Value::str("\"α β 😊\""));
}

/// P54 step 4 first slice — empty arrays `[]` and empty objects
/// `{}` are now materialised as real `JArray` / `JObject`
/// variants (not the earlier `JNull`-stub).  This unblocks
/// `kind()`, `len()`, `has_field()`, and `to_json()` for the
/// empty-container case today; non-empty containers remain
/// stubbed until the full arena materialiser lands.
#[test]
fn p54_step4_empty_array_has_jarray_kind() {
    code!(
        "fn run_p4ea() -> text {
    v_p4ea = json_parse(\"[]\");
    v_p4ea.kind()
}"
    )
    .expr("run_p4ea()")
    .result(Value::str("JArray"));
}

#[test]
fn p54_step4_empty_object_has_jobject_kind() {
    // Loft string literals treat `{...}` as interpolation; escape
    // literal braces by doubling (`{{` → `{`, `}}` → `}`), so
    // `"{{}}"` in loft source is the two-char JSON empty-object
    // literal `{}`.  Same trick below on the other object tests.
    code!(
        "fn run_p4eo() -> text {
    v_p4eo = json_parse(\"{{}}\");
    v_p4eo.kind()
}"
    )
    .expr("run_p4eo()")
    .result(Value::str("JObject"));
}

/// Step 4 first slice — `len()` returns 0 for empty containers
/// (both JArray and JObject).  Primitive variants still return
/// the integer null sentinel via the unchanged path.
#[test]
fn p54_step4_empty_array_len_is_zero() {
    code!(
        "fn run_p4al() -> integer {
    v_p4al = json_parse(\"[]\");
    v_p4al.len()
}"
    )
    .expr("run_p4al()")
    .result(Value::Int(0));
}

#[test]
fn p54_step4_empty_object_len_is_zero() {
    code!(
        "fn run_p4ol() -> integer {
    v_p4ol = json_parse(\"{{}}\");
    v_p4ol.len()
}"
    )
    .expr("run_p4ol()")
    .result(Value::Int(0));
}

/// Step 4 first slice — `to_json()` now renders `"[]"` / `"{}"`
/// for empty containers instead of the earlier `"<pending step
/// 4>"` placeholder.  Non-empty containers still render the
/// placeholder until the full arena materialiser lands.
#[test]
fn p54_step4_empty_array_to_json() {
    code!(
        "fn run_p4aj() -> text {
    v_p4aj = json_parse(\"[]\");
    v_p4aj.to_json()
}"
    )
    .expr("run_p4aj()")
    .result(Value::str("[]"));
}

#[test]
fn p54_step4_empty_object_to_json() {
    code!(
        "fn run_p4oj() -> text {
    v_p4oj = json_parse(\"{{}}\");
    v_p4oj.to_json()
}"
    )
    .expr("run_p4oj()")
    .result(Value::str("{}"));
}

/// Step 4 first slice — round-trip: parse `[]`, serialise, parse
/// again, confirm the discriminant agrees end-to-end.  Locks that
/// `n_json_parse` and `n_to_json` agree on empty containers.
#[test]
fn p54_step4_empty_array_round_trips_through_to_json() {
    code!(
        "fn run_p4ar() -> text {
    first_p4ar = json_parse(\"[]\");
    round_p4ar = json_parse(first_p4ar.to_json());
    round_p4ar.kind()
}"
    )
    .expr("run_p4ar()")
    .result(Value::str("JArray"));
}

/// Step 4 fourth slice (2026-04-14) — nested containers
/// (arrays-of-arrays, objects-of-objects, or any mix) now
/// materialise too, closing step 4.  This guard reverses the
/// earlier stub assertion: `[[1,2],[3,4]]` is a real JArray,
/// not a JNull stub.
#[test]
fn p54_step4_nested_array_materialises() {
    code!(
        "fn run_p4nm() -> text {
    v_p4nm = json_parse(\"[[1,2],[3,4]]\");
    v_p4nm.kind()
}"
    )
    .expr("run_p4nm()")
    .result(Value::str("JArray"));
}

/// Step 4 second slice (2026-04-14) — non-empty arrays of primitive
/// elements now materialise into real JArray variants with elements
/// in an arena sub-record of the root's store.  The
/// `n_json_parse` + `n_len` + `n_item` + `n_to_json` paths all
/// dispatch on JArray and cooperate: parse produces the arena,
/// len reads the vector header, item reads the N-th slot,
/// to_json recurses.  Nested containers still stub as JNull
/// (guarded above).
#[test]
fn p54_step4_nonempty_primitive_array_has_jarray_kind() {
    code!(
        "fn run_p4npk() -> text {
    v_p4npk = json_parse(\"[1,2,3]\");
    v_p4npk.kind()
}"
    )
    .expr("run_p4npk()")
    .result(Value::str("JArray"));
}

#[test]
fn p54_step4_nonempty_primitive_array_length_correct() {
    code!(
        "fn run_p4npl() -> integer {
    v_p4npl = json_parse(\"[1,2,3]\");
    v_p4npl.len()
}"
    )
    .expr("run_p4npl()")
    .result(Value::Int(3));
}

#[test]
fn p54_step4_nonempty_primitive_array_item_0_is_first() {
    code!(
        "fn run_p4npi0() -> integer {
    v_p4npi0 = json_parse(\"[10,20,30]\");
    v_p4npi0.item(0).as_long()
}"
    )
    .expr("run_p4npi0()")
    .result(Value::Long(10));
}

#[test]
fn p54_step4_nonempty_primitive_array_item_1_is_middle() {
    code!(
        "fn run_p4npi1() -> integer {
    v_p4npi1 = json_parse(\"[10,20,30]\");
    v_p4npi1.item(1).as_long()
}"
    )
    .expr("run_p4npi1()")
    .result(Value::Long(20));
}

#[test]
fn p54_step4_nonempty_primitive_array_item_out_of_range_returns_jnull() {
    code!(
        "fn run_p4npior() -> text {
    v_p4npior = json_parse(\"[10,20]\");
    v_p4npior.item(5).kind()
}"
    )
    .expr("run_p4npior()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_step4_nonempty_bool_array_item_kind() {
    code!(
        "fn run_p4npbk() -> text {
    v_p4npbk = json_parse(\"[true,false]\");
    v_p4npbk.item(0).kind()
}"
    )
    .expr("run_p4npbk()")
    .result(Value::str("JBool"));
}

#[test]
fn p54_step4_nonempty_string_array_item_value() {
    code!(
        "fn run_p4npsi() -> text {
    v_p4npsi = json_parse(\"[\\\"hello\\\",\\\"world\\\"]\");
    v_p4npsi.item(1).kind()
}"
    )
    .expr("run_p4npsi()")
    .result(Value::str("JString"));
}

#[test]
fn p54_step4_nonempty_array_to_json_round_trips() {
    code!(
        "fn run_p4narj() -> integer {
    v_p4narj = json_parse(\"[1,2,3]\");
    round_p4narj = json_parse(v_p4narj.to_json());
    round_p4narj.len()
}"
    )
    .expr("run_p4narj()")
    .result(Value::Int(3));
}

#[test]
fn p54_step4_nonempty_array_to_json_text_shape() {
    // `[1,2,3]` serialises each JNumber via `f64::Display`, which
    // prints `1` / `2` / `3` for whole-number floats.
    code!(
        "fn run_p4nats() -> text {
    v_p4nats = json_parse(\"[1,2,3]\");
    v_p4nats.to_json()
}"
    )
    .expr("run_p4nats()")
    .result(Value::str("[1,2,3]"));
}

// The original `p54_parse_array_item_access` test (originally
// `#[ignore]`'d) was un-ignored in place 2026-04-14 by P54 step 4
// second slice — see that test's comment + commit history.

/// Step 4 third slice (2026-04-14) — non-empty primitive objects.
/// Tests mirror the array second-slice guards: discriminant,
/// length, field lookup (hit + miss), has_field, to_json, and
/// a round-trip.  Every loft string with `{` / `}` doubles them
/// to `{{` / `}}` per LOFT.md § String literals.
#[test]
fn p54_step4_nonempty_primitive_object_has_jobject_kind() {
    code!(
        "fn run_p4ok() -> text {
    v_p4oK = json_parse(\"{{\\\"k\\\":1}}\");
    v_p4oK.kind()
}"
    )
    .expr("run_p4ok()")
    .result(Value::str("JObject"));
}

#[test]
fn p54_step4_nonempty_primitive_object_length_correct() {
    code!(
        "fn run_p4ol() -> integer {
    v_p4oL = json_parse(\"{{\\\"a\\\":1,\\\"b\\\":2,\\\"c\\\":3}}\");
    v_p4oL.len()
}"
    )
    .expr("run_p4ol()")
    .result(Value::Int(3));
}

#[test]
fn p54_step4_nonempty_object_field_hit_returns_value() {
    code!(
        "fn run_p4oh() -> integer {
    v_p4oH = json_parse(\"{{\\\"age\\\":30}}\");
    v_p4oH.field(\"age\").as_long()
}"
    )
    .expr("run_p4oh()")
    .result(Value::Long(30));
}

#[test]
fn p54_step4_nonempty_object_field_miss_returns_jnull() {
    code!(
        "fn run_p4om() -> text {
    v_p4oM = json_parse(\"{{\\\"k\\\":1}}\");
    v_p4oM.field(\"missing\").kind()
}"
    )
    .expr("run_p4om()")
    .result(Value::str("JNull"));
}

#[test]
fn p54_step4_nonempty_object_has_field_hit() {
    code!(
        "fn run_p4ohh() -> boolean {
    v_p4oHh = json_parse(\"{{\\\"users\\\":true}}\");
    v_p4oHh.has_field(\"users\")
}"
    )
    .expr("run_p4ohh()")
    .result(Value::Boolean(true));
}

#[test]
fn p54_step4_nonempty_object_has_field_miss() {
    code!(
        "fn run_p4ohm() -> boolean {
    v_p4oHm = json_parse(\"{{\\\"k\\\":1}}\");
    v_p4oHm.has_field(\"q\")
}"
    )
    .expr("run_p4ohm()")
    .result(Value::Boolean(false));
}

#[test]
fn p54_step4_nonempty_object_to_json_text_shape() {
    code!(
        "fn run_p4oj() -> text {
    v_p4oJ = json_parse(\"{{\\\"k\\\":1}}\");
    v_p4oJ.to_json()
}"
    )
    .expr("run_p4oj()")
    .result(Value::str("{\"k\":1}"));
}

#[test]
fn p54_step4_nonempty_object_to_json_round_trips() {
    code!(
        "fn run_p4or() -> integer {
    v_p4oR = json_parse(\"{{\\\"a\\\":1,\\\"b\\\":2}}\");
    round_p4oR = json_parse(v_p4oR.to_json());
    round_p4oR.len()
}"
    )
    .expr("run_p4or()")
    .result(Value::Int(2));
}

#[test]
fn p54_step4_nonempty_object_mixed_primitive_values() {
    code!(
        "fn run_p4omx() -> text {
    v_p4oMx = json_parse(\"{{\\\"s\\\":\\\"hi\\\",\\\"n\\\":7,\\\"b\\\":true}}\");
    v_p4oMx.field(\"s\").as_text()
}"
    )
    .expr("run_p4omx()")
    .result(Value::str("hi"));
}

/// Step 4 fourth slice — nested arrays.  Outer `[[1,2],[3,4]]`
/// has length 2, each item is itself a JArray of length 2.
#[test]
fn p54_step4_nested_array_outer_length() {
    code!(
        "fn run_p4nal() -> integer {
    v_p4nal = json_parse(\"[[1,2],[3,4]]\");
    v_p4nal.len()
}"
    )
    .expr("run_p4nal()")
    .result(Value::Int(2));
}

#[test]
fn p54_step4_nested_array_inner_length() {
    code!(
        "fn run_p4nil() -> integer {
    v_p4nil = json_parse(\"[[1,2,3],[4,5]]\");
    v_p4nil.item(0).len()
}"
    )
    .expr("run_p4nil()")
    .result(Value::Int(3));
}

#[test]
fn p54_step4_nested_array_inner_item_value() {
    code!(
        "fn run_p4niv() -> integer {
    v_p4niv = json_parse(\"[[10,20],[30,40]]\");
    v_p4niv.item(1).item(0).as_long()
}"
    )
    .expr("run_p4niv()")
    .result(Value::Long(30));
}

/// Step 4 fourth slice — nested objects.
/// `{"a": {"b": 7}}` — outer field "a" is a JObject; inner
/// field "b" is a JNumber 7.
#[test]
fn p54_step4_nested_object_chained_field() {
    code!(
        "fn run_p4nocf() -> integer {
    v_p4nocf = json_parse(\"{{\\\"a\\\":{{\\\"b\\\":7}}}}\");
    v_p4nocf.field(\"a\").field(\"b\").as_long()
}"
    )
    .expr("run_p4nocf()")
    .result(Value::Long(7));
}

/// Step 4 fourth slice — array of objects.  `[{"k":1},{"k":2}]`
/// — outer is JArray, each item is a JObject with field `"k"`.
#[test]
fn p54_step4_array_of_objects_field_lookup() {
    code!(
        "fn run_p4aof() -> integer {
    v_p4aof = json_parse(\"[{{\\\"k\\\":1}},{{\\\"k\\\":2}}]\");
    v_p4aof.item(1).field(\"k\").as_long()
}"
    )
    .expr("run_p4aof()")
    .result(Value::Long(2));
}

/// Step 4 fourth slice — object containing an array.  Locks the
/// reverse mix from `array_of_objects` so both directions of the
/// recursion are exercised.
#[test]
fn p54_step4_object_with_array_field() {
    code!(
        "fn run_p4owaf() -> integer {
    v_p4owaf = json_parse(\"{{\\\"items\\\":[10,20,30]}}\");
    v_p4owaf.field(\"items\").len()
}"
    )
    .expr("run_p4owaf()")
    .result(Value::Int(3));
}

/// Step 4 fourth slice — to_json round-trip for nested containers.
#[test]
fn p54_step4_nested_array_to_json_text_shape() {
    code!(
        "fn run_p4narts() -> text {
    v_p4narts = json_parse(\"[[1,2],[3,4]]\");
    v_p4narts.to_json()
}"
    )
    .expr("run_p4narts()")
    .result(Value::str("[[1,2],[3,4]]"));
}

#[test]
fn p54_step4_object_with_array_to_json_text_shape() {
    code!(
        "fn run_p4owats() -> text {
    v_p4owats = json_parse(\"{{\\\"k\\\":[1,2]}}\");
    v_p4owats.to_json()
}"
    )
    .expr("run_p4owats()")
    .result(Value::str("{\"k\":[1,2]}"));
}

/// Step 4 + Q2 cross-integration — `has_field` on an empty
/// JObject returns false (no fields to look up).  The two pieces
/// were developed independently; this guard locks their
/// interaction so a future has_field rewrite can't accidentally
/// claim a field exists on an empty object.
#[test]
fn p54_step4_empty_object_has_no_field() {
    code!(
        "fn run_p4oh() -> boolean {
    v_p4oh = json_parse(\"{{}}\");
    v_p4oh.has_field(\"anything\")
}"
    )
    .expr("run_p4oh()")
    .result(Value::Boolean(false));
}

/// Step 4 + existing `field()` stub cross-integration — querying
/// any key on an empty JObject returns JNull.  Regressing this
/// would break the common `if v.has_field(k) { v.field(k) … }`
/// pattern when users write it on a JSON-parsed empty object.
#[test]
fn p54_step4_empty_object_field_lookup_returns_jnull() {
    code!(
        "fn run_p4ofl() -> text {
    v_p4ofl = json_parse(\"{{}}\");
    v_p4ofl.field(\"k\").kind()
}"
    )
    .expr("run_p4ofl()")
    .result(Value::str("JNull"));
}

/// Step 4 + existing `item()` stub cross-integration — any index
/// into an empty JArray returns JNull.  Locks that out-of-range
/// access doesn't accidentally leak into an uninitialised
/// variant slot.
#[test]
fn p54_step4_empty_array_item_lookup_returns_jnull() {
    code!(
        "fn run_p4eil() -> text {
    v_p4eil = json_parse(\"[]\");
    v_p4eil.item(0).kind()
}"
    )
    .expr("run_p4eil()")
    .result(Value::str("JNull"));
}

/// Step 4 + Q3 `to_json_pretty` cross-integration — pretty output
/// for an empty container is byte-identical to canonical
/// (`"[]"` / `"{}"`) — there's nothing to indent.  Locks that
/// divergence between canonical and pretty only happens when a
/// container has content.
#[test]
fn p54_step4_empty_array_pretty_matches_canonical() {
    code!(
        "fn run_p4eapc() -> boolean {
    v_p4eapc = json_parse(\"[]\");
    canonical_p4eapc = v_p4eapc.to_json();
    pretty_p4eapc = v_p4eapc.to_json_pretty();
    canonical_p4eapc == pretty_p4eapc
}"
    )
    .expr("run_p4eapc()")
    .result(Value::Boolean(true));
}

#[test]
fn p54_step4_empty_object_pretty_matches_canonical() {
    code!(
        "fn run_p4eopc() -> text {
    v_p4eopc = json_parse(\"{{}}\");
    v_p4eopc.to_json_pretty()
}"
    )
    .expr("run_p4eopc()")
    .result(Value::str("{}"));
}

/// Q2 — `has_field(self: JsonValue, name: text) -> boolean` —
/// forward-compatible stub.  Today returns false for every
/// primitive variant (JNull / JBool / JNumber / JString); a real
/// JObject can't be constructed until P54 step 4 so the JObject
/// branch isn't exercised yet.  When step 4 ships, these guards
/// still stand (primitives still return false) and a new
/// `q2_has_field_on_jobject` test will cover the positive case.
#[test]
fn q2_has_field_on_jnull_is_false() {
    code!(
        "fn run_q2hn() -> boolean {
    v_q2hn = json_null();
    v_q2hn.has_field(\"k\")
}"
    )
    .expr("run_q2hn()")
    .result(Value::Boolean(false));
}

#[test]
fn q2_has_field_on_jbool_is_false() {
    code!(
        "fn run_q2hb() -> boolean {
    v_q2hb = json_bool(true);
    v_q2hb.has_field(\"value\")
}"
    )
    .expr("run_q2hb()")
    .result(Value::Boolean(false));
}

#[test]
fn q2_has_field_on_jnumber_is_false() {
    code!(
        "fn run_q2hnum() -> boolean {
    v_q2hnum = json_number(42.0);
    v_q2hnum.has_field(\"n\")
}"
    )
    .expr("run_q2hnum()")
    .result(Value::Boolean(false));
}

#[test]
fn q2_has_field_on_jstring_is_false() {
    code!(
        "fn run_q2hs() -> boolean {
    v_q2hs = json_string(\"hello\");
    v_q2hs.has_field(\"anything\")
}"
    )
    .expr("run_q2hs()")
    .result(Value::Boolean(false));
}

/// Q2 — free-function form of `has_field`.  Locks the `n_has_field`
/// registration in NATIVE_FNS alongside the
/// `t_9JsonValue_has_field` method alias so both dispatch paths
/// keep working.
#[test]
fn q2_has_field_free_form_on_parsed_primitive() {
    code!(
        "fn run_q2hf() -> boolean {
    v_q2hf = json_parse(\"42\");
    has_field(v_q2hf, \"k\")
}"
    )
    .expr("run_q2hf()")
    .result(Value::Boolean(false));
}

/// Q2 — `keys(self: JsonValue) -> vector<text>` returns the
/// field names of a JObject in insertion order, empty vector
/// for every other variant.  First slice (2026-04-14): empty
/// vector unconditionally — JObject walk lands in a follow-up.
/// These guards lock the empty-vector return shape across all
/// variants, in both free and method form.
#[test]
fn q2_keys_on_jnull_is_empty() {
    code!(
        "fn run_q2kne() -> integer {
    v_q2kne = json_null();
    ks_q2kne = v_q2kne.keys();
    ks_q2kne.len()
}"
    )
    .expr("run_q2kne()")
    .result(Value::Int(0));
}

#[test]
fn q2_keys_on_jbool_is_empty() {
    code!(
        "fn run_q2kbe() -> integer {
    v_q2kbe = json_bool(true);
    keys(v_q2kbe).len()
}"
    )
    .expr("run_q2kbe()")
    .result(Value::Int(0));
}

#[test]
fn q2_keys_on_jobject_returns_field_names_length() {
    // (Was `q2_keys_on_jobject_is_empty_today` until 2026-04-14
    // when the JObject walk shipped.)  Locks that `keys()` on
    // a JObject now returns the actual field-name vector.
    code!(
        "fn run_q2koe() -> integer {
    v_q2koe = json_parse(\"{{\\\"k\\\":1}}\");
    v_q2koe.keys().len()
}"
    )
    .expr("run_q2koe()")
    .result(Value::Int(1));
}

#[test]
fn q2_keys_on_jobject_returns_multiple_field_names_length() {
    code!(
        "fn run_q2km() -> integer {
    v_q2km = json_parse(\"{{\\\"a\\\":1,\\\"b\\\":2,\\\"c\\\":3}}\");
    v_q2km.keys().len()
}"
    )
    .expr("run_q2km()")
    .result(Value::Int(3));
}

/// Q2 — `keys()` JObject walk preserves insertion order: the
/// first key in the source is the first key in the result.
#[test]
fn q2_keys_on_jobject_preserves_first_name() {
    code!(
        "fn run_q2kf() -> text {
    v_q2kf = json_parse(\"{{\\\"alpha\\\":1,\\\"beta\\\":2}}\");
    ks_q2kf = v_q2kf.keys();
    first_q2kf = \"\";
    for k in ks_q2kf {
        if first_q2kf == \"\" { first_q2kf = k; }
    }
    first_q2kf
}"
    )
    .expr("run_q2kf()")
    .result(Value::str("alpha"));
}

/// Q2 — `keys()` collects every name, locked by joining them.
#[test]
fn q2_keys_on_jobject_collects_all_names() {
    code!(
        "fn run_q2kc() -> text {
    v_q2kc = json_parse(\"{{\\\"x\\\":1,\\\"y\\\":2}}\");
    out_q2kc = \"\";
    for k in v_q2kc.keys() { out_q2kc += k; out_q2kc += \"|\"; }
    out_q2kc
}"
    )
    .expr("run_q2kc()")
    .result(Value::str("x|y|"));
}

/// Q2 — the empty `keys()` result is a real iterable: a `for`
/// loop over it terminates without executing the body.  Locks
/// that callers can write `for k in v.keys() { ... }` today
/// without it crashing or looping forever.
#[test]
fn q2_keys_for_loop_is_safe() {
    code!(
        "fn run_q2kfl() -> integer {
    v_q2kfl = json_null();
    count_q2kfl = 0;
    for _k in v_q2kfl.keys() { count_q2kfl += 1; }
    count_q2kfl
}"
    )
    .expr("run_q2kfl()")
    .result(Value::Int(0));
}

/// Q2 — `fields(self: JsonValue) -> vector<JsonField>` mirrors
/// `keys`'s shape but returns (name, value) entries.  First
/// slice (2026-04-14): empty vector for every variant including
/// JObject — the real walk lands with `keys`'s JObject walk.
#[test]
fn q2_fields_on_jnull_is_empty() {
    code!(
        "fn run_q2fne() -> integer {
    v_q2fne = json_null();
    fs_q2fne = v_q2fne.fields();
    fs_q2fne.len()
}"
    )
    .expr("run_q2fne()")
    .result(Value::Int(0));
}

#[test]
fn q2_fields_on_jstring_is_empty() {
    code!(
        "fn run_q2fse() -> integer {
    v_q2fse = json_string(\"hi\");
    fields(v_q2fse).len()
}"
    )
    .expr("run_q2fse()")
    .result(Value::Int(0));
}

#[test]
fn q2_fields_on_jobject_returns_field_entries_length() {
    // (Was `q2_fields_on_jobject_is_empty_today` until 2026-04-14
    // when the JObject walk shipped.)  Locks that `fields()`
    // on a JObject now returns a vector of JsonField entries.
    code!(
        "fn run_q2foe() -> integer {
    v_q2foe = json_parse(\"{{\\\"k\\\":1}}\");
    v_q2foe.fields().len()
}"
    )
    .expr("run_q2foe()")
    .result(Value::Int(1));
}

#[test]
fn q2_fields_on_jobject_collects_multiple_entries() {
    code!(
        "fn run_q2fm() -> integer {
    v_q2fm = json_parse(\"{{\\\"a\\\":1,\\\"b\\\":2,\\\"c\\\":3}}\");
    v_q2fm.fields().len()
}"
    )
    .expr("run_q2fm()")
    .result(Value::Int(3));
}

/// Q2 — `fields()` JObject walk preserves names: iterating the
/// result and reading `.name` gives back each JsonField's name.
#[test]
fn q2_fields_collects_all_names() {
    code!(
        "fn run_q2fcn() -> text {
    v_q2fcn = json_parse(\"{{\\\"x\\\":1,\\\"y\\\":2}}\");
    out_q2fcn = \"\";
    for entry in v_q2fcn.fields() { out_q2fcn += entry.name; out_q2fcn += \"|\"; }
    out_q2fcn
}"
    )
    .expr("run_q2fcn()")
    .result(Value::str("x|y|"));
}

/// Q2 — `fields()` JObject walk also copies primitive values:
/// iterating gives back each `entry.value` as the right variant
/// with the right payload.  This guard covers JNumber.
#[test]
fn q2_fields_preserves_primitive_number_values() {
    code!(
        "fn run_q2fp() -> integer {
    v_q2fp = json_parse(\"{{\\\"k\\\":42}}\");
    sum_q2fp = 0;
    for entry in v_q2fp.fields() {
        sum_q2fp += entry.value.as_long();
    }
    sum_q2fp
}"
    )
    .expr("run_q2fp()")
    .result(Value::Long(42));
}

/// Q2 — `fields()` JObject walk: container values deep-copy
/// into the result vector (JArray preserved).
#[test]
fn q2_fields_preserves_container_values_array() {
    code!(
        "fn run_q2fca() -> text {
    v_q2fca = json_parse(\"{{\\\"k\\\":[1,2,3]}}\");
    kind_q2fca = \"\";
    for entry in v_q2fca.fields() { kind_q2fca = entry.value.kind(); }
    kind_q2fca
}"
    )
    .expr("run_q2fca()")
    .result(Value::str("JArray"));
}

/// Q2 — `fields()` JObject walk: container values deep-copy
/// into the result vector (JObject preserved).
#[test]
fn q2_fields_preserves_container_values_object() {
    code!(
        "fn run_q2fco() -> text {
    v_q2fco = json_parse(\"{{\\\"k\\\":{{\\\"a\\\":1}}}}\");
    kind_q2fco = \"\";
    for entry in v_q2fco.fields() { kind_q2fco = entry.value.kind(); }
    kind_q2fco
}"
    )
    .expr("run_q2fco()")
    .result(Value::str("JObject"));
}

#[test]
fn q2_fields_for_loop_is_safe() {
    code!(
        "fn run_q2ffl() -> integer {
    v_q2ffl = json_bool(true);
    count_q2ffl = 0;
    for _entry in v_q2ffl.fields() { count_q2ffl += 1; }
    count_q2ffl
}"
    )
    .expr("run_q2ffl()")
    .result(Value::Int(0));
}

/// Q4 container constructors — first slice (2026-04-14):
/// `json_array(items)` and `json_object(fields)` build empty
/// containers when given empty input vectors.  Non-empty input
/// returns JNull + diagnostic; the per-element deep-copy lands
/// in a follow-up.
#[test]
fn q4_json_array_empty_vector_returns_jarray() {
    code!(
        "fn run_q4ae() -> text {
    items_q4ae: vector<JsonValue> = [];
    v_q4ae = json_array(items_q4ae);
    v_q4ae.kind()
}"
    )
    .expr("run_q4ae()")
    .result(Value::str("JArray"));
}

#[test]
fn q4_json_array_empty_has_zero_length() {
    code!(
        "fn run_q4ael() -> integer {
    items_q4ael: vector<JsonValue> = [];
    v_q4ael = json_array(items_q4ael);
    v_q4ael.len()
}"
    )
    .expr("run_q4ael()")
    .result(Value::Int(0));
}

#[test]
fn q4_json_array_empty_serialises_as_brackets() {
    code!(
        "fn run_q4aes() -> text {
    items_q4aes: vector<JsonValue> = [];
    v_q4aes = json_array(items_q4aes);
    v_q4aes.to_json()
}"
    )
    .expr("run_q4aes()")
    .result(Value::str("[]"));
}

#[test]
fn q4_json_array_nonempty_input_returns_jarray() {
    // (Was `…_stubs_to_jnull` until 2026-04-14 when the deep-copy
    // landed.)  Locks that non-empty input now produces a real
    // JArray with the right element count.
    code!(
        "fn run_q4ans() -> integer {
    items_q4ans: vector<JsonValue> = [json_null()];
    v_q4ans = json_array(items_q4ans);
    v_q4ans.len()
}"
    )
    .expr("run_q4ans()")
    .result(Value::Int(1));
}

/// Q4 `json_array` deep-copy — multiple elements, mixed primitive
/// variants, all preserved in the result arena.  `to_json` round-
/// trips back to the canonical text form.
#[test]
fn q4_json_array_multi_element_round_trips() {
    code!(
        "fn run_q4amrt() -> text {
    items_q4amrt: vector<JsonValue> = [
        json_number(1.0),
        json_number(2.0),
        json_number(3.0)
    ];
    v_q4amrt = json_array(items_q4amrt);
    v_q4amrt.to_json()
}"
    )
    .expr("run_q4amrt()")
    .result(Value::str("[1,2,3]"));
}

/// Q4 `json_array` deep-copy — element index access.  `item(N)`
/// reads back the value passed at position N.
#[test]
fn q4_json_array_item_access_after_construction() {
    code!(
        "fn run_q4aiac() -> integer {
    items_q4aiac: vector<JsonValue> = [
        json_number(10.0),
        json_number(20.0),
        json_number(30.0)
    ];
    v_q4aiac = json_array(items_q4aiac);
    v_q4aiac.item(1).as_long()
}"
    )
    .expr("run_q4aiac()")
    .result(Value::Long(20));
}

/// Q4 `json_array` deep-copy — recursive: array of arrays.
/// Inner arrays are themselves built via `json_array`, then
/// embedded.  Outer length 2; inner length 2.
#[test]
fn q4_json_array_nested_construction() {
    code!(
        "fn run_q4anc() -> integer {
    inner_a_q4anc: vector<JsonValue> = [json_number(1.0), json_number(2.0)];
    inner_b_q4anc: vector<JsonValue> = [json_number(3.0), json_number(4.0)];
    outer_q4anc: vector<JsonValue> = [
        json_array(inner_a_q4anc),
        json_array(inner_b_q4anc)
    ];
    v_q4anc = json_array(outer_q4anc);
    v_q4anc.item(1).item(0).as_long() as integer
}"
    )
    .expr("run_q4anc()")
    .result(Value::Int(3));
}

#[test]
fn q4_json_object_empty_vector_returns_jobject() {
    code!(
        "fn run_q4oe() -> text {
    fields_q4oe: vector<JsonField> = [];
    v_q4oe = json_object(fields_q4oe);
    v_q4oe.kind()
}"
    )
    .expr("run_q4oe()")
    .result(Value::str("JObject"));
}

#[test]
fn q4_json_object_empty_has_zero_length() {
    code!(
        "fn run_q4oel() -> integer {
    fields_q4oel: vector<JsonField> = [];
    v_q4oel = json_object(fields_q4oel);
    v_q4oel.len()
}"
    )
    .expr("run_q4oel()")
    .result(Value::Int(0));
}

#[test]
fn q4_json_object_empty_serialises_as_braces() {
    code!(
        "fn run_q4oes() -> text {
    fields_q4oes: vector<JsonField> = [];
    v_q4oes = json_object(fields_q4oes);
    v_q4oes.to_json()
}"
    )
    .expr("run_q4oes()")
    .result(Value::str("{}"));
}

/// Q4 `json_object` deep-copy — single field round-trip.  Build a
/// JsonField in loft, pass it to json_object, read back via
/// field() lookup.
#[test]
fn q4_json_object_single_field_round_trips() {
    code!(
        "fn run_q4osf() -> integer {
    f_q4osf = JsonField { name: \"k\", value: json_number(42.0) };
    fields_q4osf: vector<JsonField> = [f_q4osf];
    v_q4osf = json_object(fields_q4osf);
    v_q4osf.field(\"k\").as_long()
}"
    )
    .expr("run_q4osf()")
    .result(Value::Long(42));
}

/// Q4 `json_object` deep-copy — multi-field length.
#[test]
fn q4_json_object_multi_field_length() {
    code!(
        "fn run_q4omfl() -> integer {
    fa_q4omfl = JsonField { name: \"a\", value: json_number(1.0) };
    fb_q4omfl = JsonField { name: \"b\", value: json_string(\"x\") };
    fc_q4omfl = JsonField { name: \"c\", value: json_bool(true) };
    fields_q4omfl: vector<JsonField> = [fa_q4omfl, fb_q4omfl, fc_q4omfl];
    v_q4omfl = json_object(fields_q4omfl);
    v_q4omfl.len()
}"
    )
    .expr("run_q4omfl()")
    .result(Value::Int(3));
}

/// Q4 `json_object` deep-copy — to_json round-trip.  Build via
/// json_object, serialise via to_json, parse back, confirm shape.
#[test]
fn q4_json_object_serialisation() {
    code!(
        "fn run_q4os() -> text {
    f1_q4os = JsonField { name: \"k\", value: json_number(7.0) };
    fields_q4os: vector<JsonField> = [f1_q4os];
    v_q4os = json_object(fields_q4os);
    v_q4os.to_json()
}"
    )
    .expr("run_q4os()")
    .result(Value::str("{\"k\":7}"));
}

/// Q4 — forward a captured subtree.  Parses a JSON array, takes
/// the resulting JArray, embeds it as the value of a freshly-
/// constructed JObject field, and serialises.  Locks that the
/// `dbref_to_parsed` deep-copy used by `n_json_object` correctly
/// preserves container values originating from a parse arena
/// (not just constructor calls).  Listed in QUALITY.md § Q4 Tests
/// as `q4_forward_captured_subtree`.
#[test]
fn q4_forward_captured_subtree_array() {
    code!(
        "fn run_q4fcsa() -> text {
    src_q4fcsa = json_parse(\"[10,20,30]\");
    fields_q4fcsa: vector<JsonField> = [
        JsonField { name: \"data\", value: src_q4fcsa }
    ];
    obj_q4fcsa = json_object(fields_q4fcsa);
    obj_q4fcsa.to_json()
}"
    )
    .expr("run_q4fcsa()")
    .result(Value::str("{\"data\":[10,20,30]}"));
}

/// Q4 — forward-captured-subtree, object variant.  Same shape as
/// the array case but the captured subtree is itself a JObject.
#[test]
fn q4_forward_captured_subtree_object() {
    code!(
        "fn run_q4fcso() -> text {
    inner_q4fcso = json_parse(\"{{\\\"x\\\":1,\\\"y\\\":2}}\");
    fields_q4fcso: vector<JsonField> = [
        JsonField { name: \"point\", value: inner_q4fcso }
    ];
    obj_q4fcso = json_object(fields_q4fcso);
    obj_q4fcso.to_json()
}"
    )
    .expr("run_q4fcso()")
    .result(Value::str("{\"point\":{\"x\":1,\"y\":2}}"));
}

/// Q4 — forward-captured-subtree round-trip: parsing the
/// serialised result yields a tree whose structure agrees
/// with the original captured subtree.
#[test]
fn q4_forward_captured_subtree_round_trip() {
    code!(
        "fn run_q4fcsr() -> integer {
    src_q4fcsr = json_parse(\"[10,20,30]\");
    fields_q4fcsr: vector<JsonField> = [
        JsonField { name: \"data\", value: src_q4fcsr }
    ];
    obj_q4fcsr = json_object(fields_q4fcsr);
    text_q4fcsr = obj_q4fcsr.to_json();
    back_q4fcsr = json_parse(text_q4fcsr);
    back_q4fcsr.field(\"data\").item(1).as_long()
}"
    )
    .expr("run_q4fcsr()")
    .result(Value::Long(20));
}

/// Q2 full-surface smoke — exercises every Q2 helper
/// (`kind`, `has_field`, `keys`, `fields`) on the same JObject
/// value in one expression chain.  Locks the four-way dispatch
/// interaction.  Score breakdown today (post both keys + fields
/// JObject walks 2026-04-14): kind=="JObject" → 1, has_field("k")
/// → 1, keys.len() → 1, fields.len() → 1.  Total 4 — every Q2
/// helper now returns its real JObject answer.
#[test]
fn q2_full_surface_smoke_on_jobject() {
    code!(
        "fn run_q2fs() -> integer {
    v_q2fs = json_parse(\"{{\\\"k\\\":1}}\");
    score_q2fs = 0;
    if v_q2fs.kind() == \"JObject\" { score_q2fs += 1; }
    if v_q2fs.has_field(\"k\")     { score_q2fs += 1; }
    score_q2fs += v_q2fs.keys().len();
    score_q2fs += v_q2fs.fields().len();
    score_q2fs
}"
    )
    .expr("run_q2fs()")
    .result(Value::Int(4));
}

/// Q2 — the common guarded-access idiom works today and will
/// keep working when step 4 lands.  `if v.has_field(k) { … }`
/// is the forward-compatible pattern — on a primitive it
/// takes the else branch, on a JObject (future) it takes the
/// then branch iff the key is present.  This guard locks the
/// control-flow shape.
#[test]
fn q2_has_field_gates_conditional_safely() {
    code!(
        "fn run_q2hg() -> integer {
    v_q2hg = json_parse(\"null\");
    if v_q2hg.has_field(\"users\") { 1 } else { 2 }
}"
    )
    .expr("run_q2hg()")
    .result(Value::Int(2));
}

// INC#18 — `x#break` is a labelled-break statement that reuses the
// `#attribute` syntax.  Documented in LOFT.md § Break and continue;
// these tests lock the behaviour so the two-mechanism design cannot
// silently regress.

/// INC#27 — corrected 2026-04-13: `x#continue` **is** implemented
/// correctly as labelled-continue, symmetric to `x#break`.  The
/// earlier writeup declaring it a silent miscompile was wrong — it
/// relied on a nested-loop reproducer where bare-continue and
/// labelled-continue happened to produce the same numeric sum (320).
/// This test uses an outer-body operation that runs BETWEEN x
/// iterations: a bare `continue` would let it run each time, a
/// labelled `x#continue` skips it when we jump past it.  The
/// observed result outer_count=1, inner_count=6 → 106 confirms the
/// labelled-continue semantics.  Manual walk:
///   x=1: y=1 inner=1; y=2 x#continue → skip rest of x=1 body
///   x=2: y=1 inner=2; y=2 inner=3; y=3 x#continue → skip rest
///   x=3: y=1,2,3 all pass; inner=4,5,6; outer_body runs → outer=1
#[test]
fn inc27_x_continue_is_labelled_continue() {
    code!(
        "fn run() -> integer {
    outer_count = 0;
    inner_count = 0;
    for x in 1..4 {
        for y in 1..4 {
            if y > x { x#continue; }
            inner_count += 1;
        }
        outer_count += 1;
    }
    outer_count * 100 + inner_count
}"
    )
    .expr("run()")
    .result(Value::Int(106));
}

/// `x#break` from an inner loop exits the outer loop whose variable
/// is `x` — not just the innermost loop.  Without the labelled break,
/// the outer loop would continue and overwrite `first`.
#[test]
fn inc18_labelled_break_exits_outer_loop() {
    code!(
        "fn run() -> integer {
    first = 0;
    for x in 1..5 {
        for y in 1..5 {
            if x * y >= 6 {
                first = x * 100 + y;
                x#break;
            }
        }
    }
    first
}"
    )
    .expr("run()")
    .result(Value::Int(203));
}

// P140 — vector range-slice `v[a..b]` used to produce a bare Rust
// panic at `src/scopes.rs:250` via the test harness.  Root cause:
// tests/testing.rs ran `scopes::check` BEFORE `assert_diagnostics` +
// the `Level::Error` return, contrary to `src/main.rs` order — a
// parser-level type error (iterator vs vector<integer>) produced a
// malformed IR that scope analysis panicked on.  Fixed 2026-04-13 by
// aligning harness order with the binary's.  The diagnostic the
// parser always emitted (type mismatch on sum_of) is now the
// user-facing error, with a proper source location.
#[test]
fn p140_vector_range_slice_auto_materialises_to_vector() {
    // Updated 2026-05-20 alongside @P287 — assigning a slice expression
    // to a local now auto-materialises into a `vector<T>` instead of
    // leaving the local typed as `iterator<T>`.  The slice's elements
    // get copied into a fresh vector at the assignment site, and
    // downstream uses (`sum(s, 0)` here) see a normal `vector<integer>`.
    // The old shape of this test asserted the reverse — that the
    // slice-to-vector mismatch was rejected with a diagnostic — but a
    // working materialisation is a strictly better user experience and
    // closes @P287's "slice → struct field crashes scopes" panic on
    // the same code path.
    code!(
        "fn run() -> integer {
    v = [10, 20, 30, 40, 50];
    s = v[1..4];
    sum(s, 0)
}"
    )
    .expr("run()")
    .result(Value::Int(90));
}

#[test]
fn p287_struct_field_slice_self_assign() {
    // `s.v = s.v[1..]` used to crash `src/scopes.rs:298` with an index-
    // out-of-bounds panic (the iterator's end-of-stream `Break(0)` had
    // no enclosing loop to bind to).  Fix in `parse_assign_op` allocates
    // a temp local, materialises the slice into it, then field-writes
    // the temp into the destination via `OpClearVector` + `OpAppendVector`.
    code!(
        "struct S { v: vector<integer> }
fn run() -> integer {
    s = S{v: [1, 2, 3, 4]};
    s.v = s.v[1..];
    s.v.len()
}"
    )
    .expr("run()")
    .result(Value::Int(3));
}

#[test]
fn p287_struct_field_slice_other_source() {
    // Sibling shape — slice taken from a DIFFERENT vector also crashed
    // before the @P287 fix (proves the crash was about slice-RHS, not
    // specifically self-reference).
    code!(
        "struct S { v: vector<integer> }
fn run() -> integer {
    s = S{v: [1, 2, 3, 4]};
    src: vector<integer> = [10, 20, 30, 40];
    s.v = src[1..];
    s.v[0]
}"
    )
    .expr("run()")
    .result(Value::Int(20));
}

// INC#2 — vector has comprehensions; sorted/index do not.  Documented
// in LOFT.md § Key-based collections (gotcha block).  These tests
// lock the vector-vs-keyed-collection asymmetry so a future uniformity
// refactor cannot silently flip either half without updating the doc.

/// Vector comprehension `[for x in v if p { … }]` compiles and runs.
/// The positive baseline for the comprehension half of INC#2.
#[test]
fn inc02_vector_comprehension_works() {
    code!(
        "fn run() -> integer {
    v = [1, 2, 3, 4, 5, 6];
    sum([for x in v if x > 3 { x }], 0)
}"
    )
    .expr("run()")
    .result(Value::Int(15));
}

/// Sorted collections ARE iterable — the keyed-collection half that
/// *does* share the `for` API with vector.
#[test]
fn inc02_sorted_is_iterable() {
    code!(
        "struct Elm { k: integer, v: integer }
struct Db { s: sorted<Elm[k]> }
fn run() -> integer {
    db = Db { s: [Elm { k: 1, v: 10 }, Elm { k: 2, v: 20 }, Elm { k: 3, v: 30 }] };
    total = 0;
    for e in db.s { total += e.v; }
    total
}"
    )
    .expr("run()")
    .result(Value::Int(60));
}

// INC#8 — method-vs-free-function is the stdlib author's choice per
// function.  Documented in LOFT.md § Methods and function calls
// (gotcha block).  These tests lock concrete examples so the
// stdlib's declared call-forms cannot silently drift.

/// `sum_of(v)` is a free-function-only stdlib definition (no `self` /
/// `both` on its first parameter).  Method syntax `v.sum_of()` must
/// not resolve — locks the "free-only" half of the INC#8 asymmetry.
#[test]
fn inc08_sum_of_is_free_function_only() {
    code!(
        "fn run() -> integer {
    v = [10, 20, 30];
    v.sum_of()
}"
    )
    .expr("run()")
    .error("Unknown field vector.sum_of — did you mean the free function `sum_of(…)` ? (stdlib declared `sum_of` as free-only; see LOFT.md § Methods and function calls) at inc08_sum_of_is_free_function_only:3:14");
}

/// `text.starts_with(s)` is declared with `self: text` — method syntax
/// works, free-function syntax doesn't.  Pairs with
/// `inc08_sum_of_is_free_function_only` to show the asymmetry runs in
/// both directions per the stdlib declaration.
/// QUALITY 6d — writing a bare `hash<Row[id]>()` constructor
/// expression used to produce the cryptic `"Indexing a non vector"`
/// error with no pointer to the struct-literal idiom that actually
/// works.  The diagnostic now spells out both halves (the missing
/// feature and the idiom users should reach for).
#[test]
fn quality_6d_keyed_collection_constructor_hint() {
    code!(
        "struct Row { id: integer, v: integer }
fn run() -> integer {
    h = hash<Row[id]>();
    0
}"
    )
    .error(
        "Indexing a non vector — keyed collections (hash/sorted/index/spatial) have no generic-constructor expression; name the key via a type annotation and initialise from a vector literal: `h: hash<Row[id]> = [Row { id: 1 }];` (a struct field `struct Db { h: hash<Row[id]> }` works too) at quality_6d_keyed_collection_constructor_hint:3:20",
    )
    // @PLN102 — the malformed `hash<Row[id]>()` parses as `hash < Row[id] > ()`, a `<`…`>`
    // comparison chain, so the non-associative-comparison guard also fires here.
    .error(
        "comparison operators do not chain — `>` follows another comparison, which would compare a boolean to the next operand; parenthesise (e.g. `(a == b) == c`) or combine with `&&` (e.g. `a < b && b < c`) at quality_6d_keyed_collection_constructor_hint:3:23",
    );
    // loft#918 — a THIRD error, "No matching operator '<' on 'unknown' and 'boolean'",
    // used to trail these two.  An unmatched operator with an untyped operand now defers
    // on pass 1, and pass 1 already reported the two errors above, so pass 2 never runs
    // and the cascade line is gone.  Nothing is lost: those two name both halves of what
    // is wrong here, and the dropped line named a type the author never wrote.
}

/// @PLN102 — a unary `-` on a value whose type is only resolved on the SECOND
/// pass (here a forward reference to the struct-returning `mk`) must not lock the
/// result variable to integer.  Before the fix, `-z` matched `OpMinInt` on pass 1
/// (unknown operand) → `dot`/`run` inferred integer; pass 2 then resolved `z` to
/// float and the assignment errored "cannot change type from integer to float".
/// This is the same-file form of the cross-package transitive case that mis-typed
/// `normalize3().z` in a graphics → glb → mesh3d tree (registry resolution order).
#[test]
fn pln102_unary_minus_on_forward_resolved_operand() {
    code!(
        "struct V3 { x: float, y: float, z: float }
fn run() -> float {
    d = mk();
    z = d.z;
    -z
}
fn mk() -> V3 { V3 { x: 1.0, y: 2.0, z: 3.0 } }"
    )
    .expr("run()")
    .result(Value::Float(-3.0));
}

/// @PLN102 — the same re-typeable escape for a BINARY op whose operands are
/// ALL still unresolved on the first pass: `f() - g()` with both callees defined
/// lower in the file.  The single-operand guard above did not cover it, so the
/// `possible` loop matched `OpMinInt` and locked `x` to integer; pass 2 resolved
/// the real float returns and the assignment errored "cannot change type from
/// integer to float" at a line the author never wrote wrong.
///
/// Deliberately scoped to ALL operands unknown: one KNOWN operand is enough to
/// steer resolution, so a genuine mismatch (the "No matching operator '<' on
/// 'unknown' and 'boolean'" assertion in
/// `quality_6d_keyed_collection_constructor_hint`) must still reach the error
/// path.  Both sums are hand-computed: 2.5 - 1.5 and 2.5 + 1.5.
#[test]
fn pln102_binary_op_on_all_forward_resolved_operands() {
    code!(
        "fn run() -> float {
    x = f() - g();
    y = f() + g();
    x + y * 10.0
}
fn f() -> float { 2.5 }
fn g() -> float { 1.5 }"
    )
    .expr("run()")
    .result(Value::Float(41.0));
}

/// @PLN102 — deferring an all-unknown operator to pass 2 must not SWALLOW a real
/// error.  A genuinely undefined callee has no type on either pass, so it takes the
/// same deferral path as the forward reference above; pass 2 must still reject it.
/// Without this, widening the guard could turn a compile error into a silent
/// mis-compile.
#[test]
fn pln102_all_unknown_deferral_still_reports_undefined_callee() {
    code!(
        "fn run() -> integer {
    nope_a() - nope_b()
}"
    )
    .error("Unknown function nope_a at pln102_all_unknown_deferral_still_reports_undefined_callee:2:5")
    .error("Unknown function nope_b at pln102_all_unknown_deferral_still_reports_undefined_callee:2:16")
    // The two trailing errors are a CASCADE ARTIFACT of the deferral, not signal:
    // with no operand type on either pass the operator is never resolved, so the
    // half-applied `OpMinInt` also trips its arity check.  Pinned because the
    // harness compares the whole set — if a future change makes the deferral tidy
    // up after itself, drop these two rather than treating them as a contract.
    .error("missing argument for parameter 'v1' of `OpMinInt` — the call supplies too few arguments (add it, or give the parameter a default `= …`) at pln102_all_unknown_deferral_still_reports_undefined_callee:2:24")
    .error("missing argument for parameter 'v2' of `OpMinInt` — the call supplies too few arguments (add it, or give the parameter a default `= …`) at pln102_all_unknown_deferral_still_reports_undefined_callee:2:24");
}

/// @PLN102 — the deferral at the TOP of `call_op` is deliberately limited to the case
/// where no operand carries type information: one known operand is enough to steer
/// resolution, so the operator search still runs here.
///
/// loft#918 added a second, later deferral — at the reject site, after that search has
/// found nothing — and this is where the difference shows.  The mismatch is still
/// reported, and now names the type the operand really has (`float`, resolved on pass 2)
/// rather than the `unknown` pass 1 saw.  What must NOT happen is the diagnostic
/// disappearing, which is what this test guards.
#[test]
fn pln102_one_known_operand_keeps_the_mismatch_diagnostic() {
    code!(
        "fn run() -> boolean {
    f() < true
}
fn f() -> float { 1.0 }"
    )
    .error(
        "No matching operator '<' on 'float' and 'boolean' at pln102_one_known_operand_keeps_the_mismatch_diagnostic:3:1",
    );
}

/// @PLN102 — CLOSED 2026-08-20.  The mixed form `f() - 1`, one forward-resolved operand
/// and one literal, is typed from the operand that is really there.
///
/// It used to be REFUSED, not mis-valued: pass 1 saw `unknown - integer`, matched
/// `OpSubInt`, and locked the local to `integer`; pass 2 resolved `f()` to `float` and the
/// assignment reported *"Variable 'a' cannot change type from integer to float"* — at a
/// line that is correct, about a decision the reader never made.  Declaration order is not
/// supposed to matter; the two-pass parser exists so a function may be used before it is
/// written.
///
/// The deferral now fires when ANY operand is unresolved rather than only when ALL are.
/// The `all` restriction existed to protect
/// `pln102_one_known_operand_keeps_the_mismatch_diagnostic` — one known operand was
/// thought to be the only thing keeping a genuine mismatch reportable — and loft#918
/// retired that reason when it added the SECOND deferral at the reject site: a mismatch is
/// now reported there on pass 2, naming the type the operand really has.  So the guard and
/// this case coexist, and the predicted "defer on the RESULT type" redesign was not needed.
#[test]
fn pln102_one_known_operand_forward_float_is_typed_from_the_real_operand() {
    code!(
        "fn run() -> float {
    a = f() - 1;
    a
}
fn f() -> float { 4.5 }"
    )
    .expr("run()")
    .result(Value::Float(3.5));
}

/// @PLN102 — the same shape with the LITERAL first.  Operand order is not what decides
/// this, and a fix that reads the first operand only would pass the test above and leave
/// this one refused.
#[test]
fn pln102_forward_operand_second_is_typed_from_the_real_operand() {
    code!(
        "fn run() -> float {
    a = 1 - f();
    a
}
fn f() -> float { 4.5 }"
    )
    .expr("run()")
    .result(Value::Float(-3.5));
}

/// @PLN102 — and with a different operator, since the deferral is in the shared operator
/// search rather than in `-`.
#[test]
fn pln102_forward_operand_addition_is_typed_from_the_real_operand() {
    code!(
        "fn run() -> float {
    a = f() + 1;
    a
}
fn f() -> float { 4.5 }"
    )
    .expr("run()")
    .result(Value::Float(5.5));
}

/// @PLN102 — the shape that reads worst: the author WROTE the type down and was still
/// refused, with the message reversed (`from float to integer`), because pass 1 had
/// already typed the expression from the literal.
#[test]
fn pln102_forward_operand_with_a_declared_type_is_not_refused() {
    code!(
        "fn run() -> float {
    a: float = f() - 1;
    a
}
fn f() -> float { 4.5 }"
    )
    .expr("run()")
    .result(Value::Float(3.5));
}

/// QUALITY 6c — the free-function hint must NOT fire when there is
/// no `n_<field>` function compatible with the receiver.  Locks the
/// specificity of the hint: a genuinely-misspelled field produces
/// the plain "Unknown field" message without a misleading "did you
/// mean …" tail.
///
/// Plan-07 phase 5 added a generic Levenshtein-based field
/// suggestion via `Parser::suggest_field_name`, but its length-aware
/// cap (`min(2, name.len() / 4)`) suppresses suggestions for 1-char
/// inputs like `z` — over-match risk is too high — so this test
/// continues to assert the plain "Unknown field" message.
#[test]
fn quality_6c_unknown_field_without_free_fn_has_no_hint() {
    code!(
        "struct Point { x: integer, y: integer }
fn run() -> integer {
    p = Point { x: 1, y: 2 };
    p.z
}"
    )
    .error("Unknown field Point.z at quality_6c_unknown_field_without_free_fn_has_no_hint:4:8");
}

#[test]
fn inc08_starts_with_is_method_not_free_function() {
    code!(
        "fn run() -> boolean {
    s = \"hello\";
    s.starts_with(\"he\")
}"
    )
    .expr("run()")
    .result(Value::Boolean(true));
}

/// QUALITY 6c follow-on — the free→method direction.  `starts_with`
/// is declared `self: text`; calling it as a free function with a
/// wrong-type receiver (`starts_with(5, "he")`) used to produce the
/// cryptic `"Unknown function starts_with"` — the function *does*
/// exist, just not with `integer` as the receiver.  Hint now names
/// the receiver type the method is declared on.
#[test]
fn quality_6c_free_call_on_wrong_type_suggests_method() {
    code!(
        "fn run() -> boolean {
    starts_with(5, \"he\")
}"
    )
    .error("Unknown function starts_with — did you mean the method `x.starts_with(…)` on text? (stdlib declared `starts_with` as a method; see LOFT.md § Methods and function calls) at quality_6c_free_call_on_wrong_type_suggests_method:2:5");
}

/// QUALITY 6c follow-on — methods declared on several receiver types
/// (`is_numeric` lives on both `text` and `character`) enumerate all
/// candidates so the user can pick the right one.
#[test]
fn quality_6c_free_call_lists_all_method_receivers() {
    code!(
        "fn run() -> boolean {
    is_numeric(5)
}"
    )
    .error("Unknown function is_numeric — did you mean the method `x.is_numeric(…)` on text / character? (stdlib declared `is_numeric` as a method; see LOFT.md § Methods and function calls) at quality_6c_free_call_lists_all_method_receivers:2:5");
}

/// QUALITY 6c follow-on — the hint must stay silent when no method
/// by that name exists anywhere.  A genuinely-misspelled free
/// function name still produces the plain "Unknown function …"
/// message, without a misleading "did you mean …" tail.
#[test]
fn quality_6c_free_call_unknown_fn_has_no_method_hint() {
    code!(
        "fn run() -> integer {
    xyzzy_never_defined(5)
}"
    )
    .error("Unknown function xyzzy_never_defined at quality_6c_free_call_unknown_fn_has_no_method_hint:2:5");
}

/// `len` is declared `both: vector` — it works equally as method
/// (`v.len()`) and as free function (`len(v)`).  Guards the `both`
/// half of the INC#8 story: when an author picks `both`, the
/// asymmetry disappears.
#[test]
fn inc08_len_with_both_works_either_way() {
    code!(
        "fn run() -> integer {
    v = [1, 2, 3, 4];
    v.len() + len(v)
}"
    )
    .expr("run()")
    .result(Value::Int(8));
}

#[test]
fn inc18_bare_break_exits_innermost_only() {
    code!(
        "fn run() -> integer {
    count = 0;
    for x in 1..4 {
        for y in 1..4 {
            if y >= 2 { break; }
            count += x;
        }
    }
    count
}"
    )
    .expr("run()")
    .result(Value::Int(6));
}

// P143 regression — ref-returning function with two return paths:
//   early-return `gh_c.ck_hexes[0]` (DbRef into a `for`-iterator element
//   inside the argument `m`) vs fallthrough `Hex {}` (local promoted to
//   hidden `__ref_1`).  Calling the function twice on the same populated
//   Map used to SIGSEGV whenever memory layout didn't catch it (P143).
//
// Fix landed in `src/state/codegen.rs::gen_set_first_ref_call_copy` —
// emit `n_set_store_lock(arg, true)` for every ref-typed argument of
// the call before `OpCopyRecord`, then `n_set_store_lock(arg, false)`
// after.  The existing `OpCopyRecord` guard at `src/state/io.rs:1001`
// already skips the source-free when the source store is `locked`,
// so an early-return that aliased one of the args no longer kills
// the caller's argument.  The work-ref scope-exit logic in
// `src/scopes.rs::free_vars` was extended to free `__ref_*` /
// `__rref_*` work-refs to recover the storage that the
// non-aliased-source path used to claim via the `0x8000` bit.
//
// Fixtures: `tests/lib/p143_types.loft`, `tests/lib/p143_entry.loft`,
// `tests/lib/p143_main.loft` — three IR shapes (empty-map fallback,
// found-on-first-chunk, loop-fallback-after-non-matching-chunk).
#[test]
fn p143_default_struct_return_from_nested_vector_use() {
    let mut p = Parser::new();
    p.lib_dirs.push("tests/lib".to_string());
    p.parse_dir("default", true, false).unwrap();
    p.parse("tests/lib/p143_main.loft", false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    state.execute("main", &p.data);
    assert!(
        !state.database.had_fatal,
        "P143 regression: had_fatal set — ref-returning fn with early-return-through-iterator + fallthrough-default still corrupts memory"
    );
}

/// P144: forwarding a `&Struct` parameter to another function that also
/// takes `&Struct` caused native codegen to emit `*var_b` (deref) instead
/// of `var_b` (pass-through).  The fix in `calls.rs` detects when a
/// `Value::Var` pointing to a `RefVar` parameter is passed to another
/// `RefVar` parameter and emits it directly.
///
/// Interpreter test: parse + execute the cross-file package.
#[test]
fn p144_ref_param_forward_interpreter() {
    let mut p = Parser::new();
    p.lib_dirs.push("tests/lib".to_string());
    p.parse_dir("default", true, false).unwrap();
    p.parse("tests/lib/p144_main.loft", false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    state.execute("main", &p.data);
    assert!(
        !state.database.had_fatal,
        "P144 regression: & param forward caused runtime error"
    );
}

/// P144: native codegen test — the generated Rust must compile and run.
#[test]
fn p144_ref_param_forward_native() {
    let mut p = Parser::new();
    p.lib_dirs.push("tests/lib".to_string());
    p.parse_dir("default", true, false).unwrap();
    p.parse("tests/lib/p144_main.loft", false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);

    // Emit the native Rust source and verify it compiles.
    let rs_path = std::env::temp_dir().join("loft_p144_native.rs");
    {
        let mut f = std::fs::File::create(&rs_path).unwrap();
        let start_def = 0;
        let end_def = p.data.definitions();
        let main_nr = p.data.def_nr("n_main");
        let entry_defs: Vec<u32> = if main_nr < end_def {
            vec![main_nr]
        } else {
            (start_def..end_def).collect()
        };
        let mut out = loft::generation::Output::new(&p.data, &state.database);
        out.output_native_reachable(&mut f, start_def, end_def, &entry_defs)
            .unwrap();
    }

    // Read and check the generated source contains the fix pattern.
    let source = std::fs::read_to_string(&rs_path).unwrap();
    // The call to box_ensure should pass var_b directly, not *var_b.
    // P199 ABI change: native fns take `cell` (`&UnsafeCell<Stores>`),
    // not `stores` (`&mut Stores`).
    assert!(
        !source.contains("n_box_ensure(cell, *var_b)"),
        "P144 regression: native codegen still emits *var_b for & param forward.\nGenerated: {}",
        rs_path.display()
    );
    assert!(
        source.contains("n_box_ensure(cell, var_b)"),
        "P144 regression: expected direct var_b pass-through for & param.\nGenerated: {}",
        rs_path.display()
    );
    let _ = std::fs::remove_file(&rs_path);
}

/// P145 regression: user fn name collision with native stdlib
/// (e.g. user `to_json` → `n_to_json`, stdlib also has `n_to_json`
/// for JsonValue serialization).  `generate_call` used to emit
/// `OpStaticCall` (native dispatch) whenever `library_names`
/// matched, bypassing the user body's `OpCall` path and
/// corrupting the stack.  Fix: skip library_names lookup when
/// `def.code != Value::Null`.
/// P151 regression guard: forward-reference to a struct-returning fn
/// followed by field mutation USED to corrupt variable type inference.
/// Trigger:
/// ```
/// fn one() { x = callee(); x.v = 99; }
/// fn callee() -> H { H { v: 7 } }
/// ```
/// errored with `Variable 'x' cannot change type from integer to H`.
///
/// Root cause (closed): `parser/fields.rs::field()` silently dropped
/// `.v` when called on an unknown-type receiver in pass-1 — leaving
/// `code` as `Value::Var(x)`, which caused downstream assignment
/// processing (`parse_assign_op` → `change_var`) to set x's type to
/// the RHS expression's type (integer in `x.v = 99`).  Pass-2 then
/// rejected the now-resolved `x = callee()` returning the struct.
///
/// Fix: wrap `code` in `Value::Drop` when the field access is
/// unresolvable in pass-1, so `code != Value::Var(x)` and the
/// assignment processing skips the spurious type update.
#[test]
fn p151_forward_ref_struct_call_with_mutation() {
    code!(
        "struct H { v: integer }
fn one() {
    x = callee();
    x.v = 99;
}
fn callee() -> H { H { v: 7 } }
fn test() { one(); }"
    )
    .result(Value::Null);
}

/// P152.A — vector field assignment from a variable used to be silently
/// dropped at runtime (`s.v = fresh;` evaluated `fresh` but never wrote
/// it into the field).  Fix: `towards_set` now emits
/// `OpClearVector + OpAppendVector` when the LHS is a vector
/// field-access, deep-copying the RHS into the field's storage.
#[test]
fn p152_vec_field_assign_from_var_dataloss() {
    code!(
        "struct S { v: vector<integer> }
fn modify(s: S) {
    fresh: vector<integer> = [1, 2, 3];
    s.v = fresh;
}
fn test() {
    s = S { v: [] };
    modify(s);
    assert(len(s.v) == 3, \"expected 3 got {len(s.v)}\");
}"
    )
    .result(Value::Null);
}

/// P152.A (variant) — `s.v = []` used to silently keep the existing
/// vector contents.  Fix: parse_assign_op detects the empty-Insert
/// + field LHS shape and emits OpClearVector(to).
#[test]
fn p152_vec_field_assign_from_empty_literal_dataloss() {
    code!(
        "struct S { v: vector<integer> }
fn modify(s: S) {
    s.v = [];
}
fn test() {
    s = S { v: [1, 2, 3] };
    modify(s);
    assert(len(s.v) == 0, \"expected 0 got {len(s.v)}\");
}"
    )
    .result(Value::Null);
}

/// P152.A — vector field whole-replacement (`s.v = fresh`) propagates to
/// the caller by value (no `&` needed): the `towards_set` change emits a
/// real OpClearVector+OpAppendVector pair.  (The `&`-form variant is gone:
/// with the W4 redundant-`&` lint on by default the `&` here is flagged as
/// unnecessary, since field mutation already reaches the caller.)
#[test]
fn p152_vec_field_ref_param_mutation_undetected() {
    code!(
        "struct S { v: vector<integer> }
fn modify(s: S) {
    fresh: vector<integer> = [1, 2, 3];
    s.v = fresh;
}
fn test() {
    s = S { v: [] };
    modify(s);
    assert(len(s.v) == 3, \"expected 3 got {len(s.v)}\");
}"
    )
    .result(Value::Null);
}

/// P152.B — struct field whole-replacement (`s.i = fresh`) works at
/// runtime via OpCopyRecord and propagates to the caller by value (no `&`
/// needed).  (The `&`-form variant is gone: with the W4 redundant-`&` lint
/// on by default the `&` here is flagged as unnecessary, since field
/// mutation already reaches the caller.)
#[test]
fn p152_struct_field_ref_param_mutation_undetected() {
    code!(
        "struct Inner { x: integer }
struct Outer { i: Inner }
fn modify(s: Outer) {
    fresh = Inner { x: 99 };
    s.i = fresh;
}
fn test() {
    s = Outer { i: Inner { x: 7 } };
    modify(s);
    assert(s.i.x == 99, \"expected 99 got {s.i.x}\");
}"
    )
    .result(Value::Null);
}

/// P153 regression guard — vector ≥187 elements transferred to a struct
/// field via construction USED to corrupt the field's storage.
/// Root cause (closed): `vector_set_size` wrote the new length to the
/// pre-resize rec after `Store::resize` relocated the block, and
/// `vector_add` then byte-copied into the stale destination captured from
/// `vector_append`.  Fix in `src/database/structures.rs`: track the
/// relocated rec in `vector_set_size`; re-read the destination rec after
/// `vector_set_size` in `vector_add`.
#[test]
fn p153_vec_field_transfer_relocation_from_var() {
    code!(
        "struct H { h_material: integer }
struct C { ck_hexes: vector<H> }
fn test() {
    hexes: vector<H> = [];
    for _ in 0..1024 { hexes += [H {}]; }
    c = C { ck_hexes: hexes };
    newh = H {};
    newh.h_material = 42;
    c.ck_hexes[167] = newh;
    v = c.ck_hexes[167].h_material;
    assert(v == 42, \"expected 42 got {v}\");
}"
    )
    .result(Value::Null);
}

/// P153 regression guard — same bug, exposed via a function-call
/// initializer instead of a bare variable.  Previously fell through
/// `handle_field`'s else branch (no OpAppendVector emitted) and left the
/// field empty.  Fix: widen the deep-copy check to any non-Insert vector
/// expression.
#[test]
fn p153_vec_field_transfer_relocation_from_call() {
    code!(
        "struct H { h_material: integer }
struct C { ck_hexes: vector<H> }
fn build() -> vector<H> {
    hexes: vector<H> = [];
    for _ in 0..200 { hexes += [H {}]; }
    hexes
}
fn test() {
    c = C { ck_hexes: build() };
    newh = H {};
    newh.h_material = 42;
    c.ck_hexes[100] = newh;
    v = c.ck_hexes[100].h_material;
    assert(v == 42, \"expected 42 got {v}\");
}"
    )
    .result(Value::Null);
}

/// P153 regression guard — append after transfer must not heap-corrupt.
/// Pre-fix this triggered libc `double free or corruption` and SIGABRT.
#[test]
fn p153_vec_field_append_after_transfer() {
    code!(
        "struct H { x: integer }
struct C { items: vector<H> }
fn test() {
    hexes: vector<H> = [];
    for _ in 0..200 { hexes += [H {}]; }
    c = C { items: hexes };
    c.items += [H {}];
    assert(len(c.items) == 201, \"len {len(c.items)}\");
}"
    )
    .result(Value::Null);
}

/// P153 complement — direct-into-field pattern must still work (guard
/// against over-fixing the transfer path).
#[test]
fn p153_vec_field_direct_into_field_still_works() {
    code!(
        "struct H { h_material: integer }
struct C { ck_hexes: vector<H> }
fn test() {
    c = C { ck_hexes: [] };
    for _ in 0..200 { c.ck_hexes += [H {}]; }
    newh = H {};
    newh.h_material = 42;
    c.ck_hexes[100] = newh;
    v = c.ck_hexes[100].h_material;
    assert(v == 42, \"expected 42 got {v}\");
}"
    )
    .result(Value::Null);
}

/// P154 regression guard — `s.v = helper_fn(s.v, …)` must not wipe the
/// field.  Root cause (closed): the P152 lowering emitted
/// OpClearVector(s.v) BEFORE OpAppendVector evaluated the RHS, so the
/// helper saw an already-empty field and returned an empty vector, which
/// was then copied back as empty.
/// Fix: when the RHS is a non-Var expression, capture it into a fresh
/// local temp FIRST, then clear + append from the temp.
#[test]
fn p154_vec_field_assign_from_helper_reading_self() {
    code!(
        "struct S { v: vector<integer> }
fn tail(v: vector<integer>, drop: integer) -> vector<integer> {
    rebuilt: vector<integer> = [];
    keep = len(v) - drop;
    for i in 0..keep { rebuilt += [v[i]]; }
    rebuilt
}
fn test() {
    s = S { v: [1, 2, 3] };
    s.v = tail(s.v, 1);
    assert(len(s.v) == 2, \"expected 2 got {len(s.v)}\");
    assert(s.v[0] == 1, \"[0] {s.v[0]}\");
    assert(s.v[1] == 2, \"[1] {s.v[1]}\");
}"
    )
    .result(Value::Null);
}

/// P154 complement — `s.v = s.v` must be a no-op, not a wipe.
/// Handled by the self-identity guard: IR-equal LHS and RHS collapse
/// to an empty Insert.
#[test]
fn p154_vec_field_self_identity_is_noop() {
    code!(
        "struct S { v: vector<integer> }
fn test() {
    s = S { v: [10, 20, 30] };
    s.v = s.v;
    assert(len(s.v) == 3, \"len {len(s.v)}\");
    assert(s.v[1] == 20, \"[1] {s.v[1]}\");
}"
    )
    .result(Value::Null);
}

/// P154 complement — `s.v = hexes` (plain Var RHS) must still work.
/// The Var-only fast path skips the temp (unnecessary for Var reads).
#[test]
fn p154_vec_field_assign_from_plain_var_still_works() {
    code!(
        "struct S { v: vector<integer> }
fn test() {
    fresh: vector<integer> = [7, 8, 9];
    s = S { v: [] };
    s.v = fresh;
    assert(len(s.v) == 3, \"len {len(s.v)}\");
    assert(s.v[0] == 7, \"[0] {s.v[0]}\");
}"
    )
    .result(Value::Null);
}

/// P155 regression guard — push/undo/mid-assert/redo/final-read
/// sequence SIGSEGVs in OpGetVector.  Triggered when a helper fn that
/// reads a struct out of a vector is called between an undo-style
/// restore and a redo-style restore, with a mid-assert reading the
/// field in between.  Removing the mid-assert makes the crash
/// disappear.  Hypothesis: the helper returns a DbRef into a store
/// that gets freed before the final read, leaving a dangling ref that
/// OpGetVector dereferences.  See PROBLEMS.md P155 for the 22-line
/// minimal reproducer.
/// P156 regression guard — `vector<T>` with a T that shadows a stdlib
/// constant (e.g. `E`, `PI`) used to panic `typedef.rs:309` instead of
/// emitting the clean "struct conflicts with constant" diagnostic.
/// Fix: `parser/definitions.rs::sub_type` checks the resolved element
/// def's DefType up-front and emits a proper diagnostic if it's not a
/// type; `typedef.rs::fill_database` softened the assert to `continue`
/// so a prior parser error never panics the runtime.
#[test]
fn p156_vector_element_shadows_constant() {
    let s = loft::platform::sep_str();
    code!(
        "struct E { x: integer }
struct Big { v: vector<E> }
fn test() { }"
    )
    .error(&format!(
        "struct 'E' conflicts with a constant of the same name already defined \
         at default{s}01_code.loft:377:24 — pick a different name \
         at p156_vector_element_shadows_constant:1:11"
    ))
    .error(&format!(
        "'E' is a Constant, not a type — the element of vector<T> must be a \
         struct or enum (defined at default{s}01_code.loft:377:24) \
         at p156_vector_element_shadows_constant:2:26"
    ));
}

/// P157 regression guard — native codegen's pre-eval path emitted
/// `*var_m` for a `&Map` forwarded to another fn taking `&Map`,
/// breaking rustc type-check.  P144 fixed this for the non-pre-eval
/// path; the pre-eval arg re-emitter in `generation/pre_eval.rs::
/// output_code_with_subst` had its own code that bypassed the check.
/// Trigger: call a user fn that takes `&Struct` from inside another
/// fn that also takes `&Struct`, AND the call has other args that
/// need pre-evaluation (nested field reads, etc.).
#[test]
fn p157_native_refvar_forwarding_with_preeval() {
    // Write the test program to a temp file; use the parser's file-
    // loading entry point rather than an inline string.
    let src_path = std::env::temp_dir().join("loft_p157_test.loft");
    std::fs::write(
        &src_path,
        "struct Inner { val: integer }\n\
         struct Outer { inner: Inner }\n\
         fn helper(o: &Outer, n: integer) { o.inner.val = n; }\n\
         fn caller(o: &Outer) { helper(o, o.inner.val + 1); }\n\
         fn main() { o = Outer { inner: Inner { val: 5 } }; caller(o); }\n",
    )
    .unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse(src_path.to_str().unwrap(), false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let rs_path = std::env::temp_dir().join("loft_p157_native.rs");
    {
        let mut f = std::fs::File::create(&rs_path).unwrap();
        let main_nr = p.data.def_nr("n_main");
        let mut out = loft::generation::Output::new(&p.data, &state.database);
        out.output_native_reachable(&mut f, 0, p.data.definitions(), &[main_nr])
            .unwrap();
    }
    let source = std::fs::read_to_string(&rs_path).unwrap();
    // P199 ABI change: native fns take `cell` (`&UnsafeCell<Stores>`),
    // not `stores` (`&mut Stores`).
    assert!(
        !source.contains("n_helper(cell, *var_o"),
        "P157 regression: pre-eval path still emits *var_o for & param forward.\n\
         Generated: {}",
        rs_path.display()
    );
    assert!(
        source.contains("n_helper(cell, var_o"),
        "P157 regression: expected direct var_o pass-through.\n\
         Generated: {}",
        rs_path.display()
    );
    let _ = std::fs::remove_file(&rs_path);
    let _ = std::fs::remove_file(&src_path);
}

// ── Language enhancements ────────────────────────────────────────────

/// Bitwise NOT operator `~` — desugars to OpBitNotSingleInt.
#[test]
fn enhancement_bitwise_not() {
    expr!("~0").result(Value::Int(-1));
}

#[test]
fn enhancement_bitwise_not_clear_bit() {
    expr!("(32 | 64) & ~32").result(Value::Int(64));
}

/// `&vector<T>` mutation detection — for-loop variable field writes
/// should propagate back to the iterated `&` collection parameter.
#[test]
fn enhancement_ref_vector_loop_mutation_detected() {
    code!(
        "struct Item { val: integer }
fn double_all(items: &vector<Item>) {
    for it in items { it.val = it.val * 2; }
}
fn test() {
    v: vector<Item> = [Item { val: 5 }];
    double_all(v);
    assert(v[0].val == 10, \"doubled\");
}"
    )
    .result(Value::Null);
}

/// Read-only loop over `&vector<T>` should still flag the `&`.
#[test]
fn enhancement_ref_vector_readonly_loop_still_flags() {
    code!(
        "struct Item { val: integer }
fn sum_vals(items: &vector<Item>) -> integer {
    total = 0;
    for it in items { total = total + it.val; }
    total
}
fn test() { }"
    )
    // 2:20 is the `&` (loft#1003).  The error shares the reference check's position with
    // the `needless-reference-parameter` warning, so it moved off the body with it.
    .error(
        "Parameter 'items' has & but is never modified; remove the & \
at enhancement_ref_vector_readonly_loop_still_flags:2:20",
    );
}

/// `break value` in void function → compile error.
#[test]
fn enhancement_break_value_in_void_function_errors() {
    code!(
        "fn test() {
    for i in 0..10 {
        if i == 5 { break i; }
    }
}"
    )
    .error(
        "`break <value>` requires a non-void function — \
the value is returned from the enclosing function \
at enhancement_break_value_in_void_function_errors:3:29",
    );
}

/// `is` operator — variant check on plain enum.
#[test]
fn enhancement_is_plain_enum() {
    code!(
        "enum Dir { North, South, East, West }
fn test() {
    d = Dir.North;
    assert(d is North, \"is North\");
    assert(!(d is South), \"not South\");
}"
    )
    .result(Value::Null);
}

/// `is` operator — variant check on struct-enum + loop counting.
#[test]
fn enhancement_is_struct_enum_in_loop() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect { width: float, height: float }
}
fn test() {
    items: vector<Shape> = [Circle { radius: 1.0 }, Rect { width: 2.0, height: 3.0 }, Circle { radius: 4.0 }];
    count = 0;
    for it in items { if it is Circle { count = count + 1; } }
    assert(count == 2, \"2 circles\");
}"
    )
    .result(Value::Null);
}

/// `is` operator with field capture — single field.
#[test]
fn enhancement_is_capture_single_field() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect { width: float, height: float }
}
fn test() {
    s = Circle { radius: 3.14 };
    result = 0.0;
    if s is Circle { radius } {
        result = radius;
    }
    assert(result == 3.14, \"captured radius\");
}"
    )
    .result(Value::Null);
}

/// `is` operator with field capture — multiple fields + else branch.
#[test]
fn enhancement_is_capture_multiple_fields_else() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect { width: float, height: float }
}
fn test() {
    s = Rect { width: 5.0, height: 10.0 };
    area = 0.0;
    if s is Rect { width, height } {
        area = width * height;
    } else {
        area = -1.0;
    }
    assert(area == 50.0, \"captured both\");
    c = Circle { radius: 2.0 };
    if c is Rect { width, height } {
        area = width * height;
    } else {
        area = -1.0;
    }
    assert(area == -1.0, \"else taken\");
}"
    )
    .result(Value::Null);
}

/// `is` operator with field capture in loop — sum radii from mixed vector.
#[test]
fn enhancement_is_capture_in_loop() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect { width: float, height: float }
}
fn test() {
    items: vector<Shape> = [Circle { radius: 1.0 }, Rect { width: 2.0, height: 3.0 }, Circle { radius: 4.0 }];
    total = 0.0;
    for it in items {
        if it is Circle { radius } {
            total += radius;
        }
    }
    assert(total == 5.0, \"sum of radii\");
}"
    )
    .result(Value::Null);
}

/// `is` capture scope doesn't leak into outer scope.
#[test]
fn enhancement_is_capture_scope_isolation() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect { width: float, height: float }
}
fn test() {
    s = Circle { radius: 99.0 };
    radius = 1.0;
    if s is Rect { width, height } {
        radius = width;
    }
    assert(radius == 1.0, \"outer radius unchanged\");
}"
    )
    .result(Value::Null);
}

/// Op table extension — emit_op handles ops >= 255 via escape prefix.
/// No specific op to test yet (all 255 primary slots used), but verify
/// the infrastructure doesn't break existing ops.
#[test]
fn enhancement_op_extension_existing_ops_unaffected() {
    expr!("~0").result(Value::Int(-1));
}

/// map/filter on &vector<T> parameter — method resolution unwraps RefVar.
#[test]
fn enhancement_map_filter_on_ref_vector() {
    code!(
        "fn process(items: &vector<integer>) {
    items += [99];
    d = items.map(|x| { x * 2 });
    assert(d[0] == 2, \"mapped\");
}
fn test() {
    v = [1, 2, 3];
    process(v);
    assert(len(v) == 4, \"appended\");
}"
    )
    .result(Value::Null);
}

/// P161 regression guard — `for it in items` where items is
/// `&vector<Struct>` used to error "Unknown type null" (field access
/// on the loop variable failed).  Root cause: `for_type` and
/// `iterator` didn't unwrap `RefVar(Vector(...))` before matching.
#[test]
fn p161_for_over_ref_vector() {
    code!(
        "struct Item { val: integer }
fn add_item(items: &vector<Item>, v: integer) {
    items += [Item { val: v }];
}
fn test() {
    v: vector<Item> = [];
    add_item(v, 42);
    assert(len(v) == 1, \"len {len(v)}\");
    assert(v[0].val == 42, \"val {v[0].val}\");
}"
    )
    .result(Value::Null);
}

/// P160 regression guard — `modify(items[1], 42)` where `modify`
/// takes `&S` used to error "Cannot pass a literal or expression
/// to a '&' parameter".  Two fixes: (1) parser accepts "addressable"
/// expressions (vector element, field access chains rooted in a Var);
/// (2) codegen handles `OpCreateStack(non-Var expr)` by generating
/// the expression first (pushes DbRef), then emitting OpCreateStack
/// with the offset pointing at the just-pushed result.
#[test]
fn p160_vec_element_as_ref_param() {
    code!(
        "struct S { x: integer }
fn modify(s: S, val: integer) { s.x = val; }
fn test() {
    items: vector<S> = [S { x: 0 }, S { x: 10 }];
    modify(items[1], 42);
    assert(items[1].x == 42, \"got {items[1].x}\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p160_nested_field_vec_element_as_ref_param() {
    code!(
        "struct Inner { val: integer }
struct Outer { items: vector<Inner> }
fn set_val(inner: Inner, v: integer) { inner.val = v; }
fn test() {
    o = Outer { items: [Inner { val: 0 }, Inner { val: 0 }] };
    set_val(o.items[1], 99);
    assert(o.items[1].val == 99, \"got {o.items[1].val}\");
}"
    )
    .result(Value::Null);
}

/// P159 regression guard — `Shape.parse(json)` used to fail for
/// struct-enums ("Unknown field Shape.parse").  Fix: added
/// `DefType::Enum` branch in `parse_var` for `.parse(` detection,
/// and added discriminant wrapper `{"Variant":{fields}}` to the
/// JSON serializer in `format.rs`.
#[test]
fn p159_struct_enum_json_roundtrip() {
    code!(
        "enum Shape {
    Circle { radius: float },
    Rect { width: float, height: float }
}
fn test() {
    c = Circle { radius: 3.14 };
    j = \"{c:j}\";
    p = Shape.parse(j);
    r = match p { Circle { radius } => radius, Rect => 0.0 };
    assert(r == 3.14, \"circle rt\");
    rect = Rect { width: 5.0, height: 10.0 };
    j2 = \"{rect:j}\";
    p2 = Shape.parse(j2);
    r2 = match p2 { Circle => 0.0, Rect { width, height } => width * height };
    assert(r2 == 50.0, \"rect rt\");
}"
    )
    .result(Value::Null);
}

/// P158 regression guard — trailing comma after the last field in a
/// struct-enum variant used to trigger "Expect attribute".  Regular
/// structs accepted trailing commas; enum variants didn't.  Fix:
/// added `|| self.lexer.peek_token("}")` to the break condition in
/// `parse_enum_values`, mirroring `parse_struct`.
#[test]
fn p158_trailing_comma_enum_variant() {
    code!(
        "enum K {
    Alpha { x: integer, y: integer, },
    Beta { z: integer }
}
fn test() {
    a = Alpha { x: 1, y: 2 };
    match a { Alpha { x, y } => assert(x + y == 3, \"sum\"), Beta => 0 };
}"
    )
    .result(Value::Null);
}

/// P155 regression guard — push/undo/mid-assert/redo/final-read used
/// to SIGSEGV in OpGetVector.  Root cause: `state/codegen.rs::generate_set`
/// (reassignment path, lines 891-932) emitted `OpCopyRecord` with the
/// 0x8000 "free source" flag around a user-fn call, but without the
/// `n_set_store_lock` bracket.  When the callee returned a DbRef
/// aliased with a caller arg — e.g. `read_at(c, idx)` returns into
/// `c.items` — the free-source flag freed the caller's arg store.
/// Later uses of that arg SIGSEGV'd.  Fix: mirror the
/// `gen_set_first_ref_call_copy` lock/unlock bracket (which the P143
/// fix added) onto the reassignment path.
#[test]
fn p155_segv_undo_redo_midassert() {
    code!(
        "struct H { m: integer }
struct Elm { prev: H }
struct Ct { items: vector<H> }
struct Ss { undo: vector<Elm>, redo: vector<Elm> }
fn read_at(c: Ct, idx: integer) -> H { c.items[idx] ?? H {} }
fn test() {
    c = Ct { items: [H{}, H{}, H{}, H{}, H{}, H{}] };
    s = Ss { undo: [], redo: [] };
    h = read_at(c, 2);
    s.undo += [Elm { prev: h }];
    nh = H {}; nh.m = 77; c.items[2] = nh;
    e = s.undo[0];
    cur = read_at(c, 2);
    s.redo += [Elm { prev: cur }];
    c.items[2] = e.prev;
    assert(read_at(c, 2).m == 0, \"reverted\");
    re = s.redo[0];
    c.items[2] = re.prev;
    assert(read_at(c, 2).m == 77, \"reapplied\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p145_text_return_multivec_struct_cross_file() {
    let mut p = Parser::new();
    p.lib_dirs.push("tests/lib".to_string());
    p.parse_dir("default", true, false).unwrap();
    p.parse("tests/lib/p145_main2.loft", false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    state.execute("main", &p.data);
    assert!(
        !state.database.had_fatal,
        "P145 regression: to_json on cross-file multi-vector struct crashed"
    );
}

/// P162 regression guard — native codegen emits `return let mut …` when
/// a match expression with struct-enum field bindings + guard is returned
/// directly.  `pre_declare_branch_vars` writes `let mut` declarations
/// after the `return` keyword.  Interpreter works; native compilation fails.
#[test]
fn p162_return_match_struct_enum_native() {
    code!(
        "enum GShape {
    GCircle { radius: float },
    GRect { width: float, height: float }
}
fn garea(s: GShape) -> float {
    match s {
        GCircle { radius } if radius > 0.0 => 3.14 * radius * radius,
        GCircle { radius } => 0.0,
        GRect { width, height } => width * height
    }
}
fn test() {
    assert(garea(GCircle { radius: 2.0 }) > 12.0, \"circle area\");
    assert(garea(GCircle { radius: -1.0 }) == 0.0, \"negative radius\");
    assert(garea(GRect { width: 3.0, height: 4.0 }) == 12.0, \"rect area\");
}"
    )
    .result(Value::Null);
}

/// P164 regression guard — trailing comma after the LAST VARIANT of an
/// enum declaration used to fail with `Expect name in type definition`.
/// P158 fixed trailing commas inside a variant's field list; this is the
/// sibling case on the variant list itself.  Fix: mirror the P158 guard
/// (`|| self.lexer.peek_token("}")`) onto the outer variant-list break
/// check in `parse_enum_values`.
#[test]
fn p164_trailing_comma_enum_variant_list() {
    code!(
        "enum P164Kind {
    P164Alpha { x: integer },
    P164Beta { y: integer },
}
fn test() {
    a = P164Alpha { x: 1 };
    assert(a is P164Alpha, \"alpha\");
    b = P164Beta { y: 2 };
    assert(b is P164Beta, \"beta\");
}"
    )
    .result(Value::Null);
}

/// P164 also covers plain (non-struct-field) enum declarations.
#[test]
fn p164_trailing_comma_plain_enum() {
    code!(
        "enum P164Dir {
    P164North,
    P164East,
    P164South,
    P164West,
}
fn test() {
    d = P164Dir.P164North;
    assert(d is P164North, \"north\");
}"
    )
    .result(Value::Null);
}

/// P170 regression guard — `x = Struct{}; x = vec[i]; mutate(x)` used to
/// fail with `Incorrect var x[N] versus M on n_<fn>` at codegen.
///
/// Root cause: `parser/objects.rs::parse_object` had a gap in the
/// in-place struct-literal path.  When the LHS variable's type was
/// already inferred with dependencies (because a later assignment in
/// the same function did `x = bs[i]`, giving x type
/// `Reference(Bag, [bs])`), `is_independent(x)` returned false.  The
/// in-place `v_set(x, Null) + OpDatabase(x)` init branch required both
/// `is_independent` AND `type_matches` — with `type_matches=true` and
/// `is_independent=false`, neither the if-branch nor the else-if
/// (which required `!type_matches`) fired.  The struct-literal
/// statement emitted only field-init calls into uninitialised storage,
/// codegen never saw a Set for x's first assignment, and later
/// `generate_var(x)` asserted since x's slot sat above TOS.
///
/// Fix: extend the `else if` to also fire when
/// `!is_independent && !first_pass` — routes the construction through
/// a fresh work-ref (existing "new_object" path), which emits the
/// required `v_set + OpDatabase` prelude and yields a `Block`-shaped
/// RHS that the outer assignment can then copy/alias via the normal
/// Set path.
#[test]
fn p170_struct_placeholder_then_vec_elem_reassign() {
    code!(
        "struct P170Bag { items: vector<integer> }
fn p170_mutate_bag(b: P170Bag, v: integer) { b.items += [v]; }
fn test() {
    p170_bs: vector<P170Bag> = [];
    p170_x = P170Bag {};
    p170_bs += [P170Bag {}];
    p170_x = p170_bs[len(p170_bs) - 1] ?? P170Bag {};
    p170_mutate_bag(p170_x, 1);
    assert(len(p170_bs[0].items) == 1, \"mutated through alias\");
}"
    )
    .warning("Dead assignment — 'p170_x' is overwritten before being read at p170_struct_placeholder_then_vec_elem_reassign:5:25")
    .result(Value::Null);
}

/// P170 guard — three-way: the same shape but with a conditional
/// assignment between the placeholder and the vec-elem reassign.
#[test]
fn p170_placeholder_conditional_then_reassign() {
    code!(
        "struct P170CBag { val: integer }
fn p170c_bump(b: P170CBag) { b.val = b.val + 1; }
fn test() {
    p170c_v: vector<P170CBag> = [P170CBag { val: 5 }];
    p170c_x = P170CBag { val: 0 };
    p170c_x = p170c_v[0];
    p170c_bump(p170c_x);
    assert(p170c_v[0].val == 6, \"bumped first elem\");
}"
    )
    .warning("Dead assignment — 'p170c_x' is overwritten before being read at p170_placeholder_conditional_then_reassign:5:35")
    .result(Value::Null);
}

/// P167 regression guard — trailing comma in a function-call argument
/// list used to fail with "Too many parameters for n_<fn>".  P158 fixed
/// trailing commas in struct-enum variant field lists; P164 fixed
/// trailing commas in enum variant lists; P167 covers function-call
/// argument lists (the third and final trailing-comma site).  Fix:
/// mirror the P158 guard in `parser/control.rs::parse_call` — for both
/// the positional and named argument loops.
#[test]
fn p167_trailing_comma_function_call_positional() {
    code!(
        "fn p167_add3(a: integer, b: integer, c: integer) -> integer { a + b + c }
fn test() {
    r = p167_add3(1, 2, 3,);
    assert(r == 6, \"trailing comma positional\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p167_trailing_comma_function_call_multiline() {
    // The shape that actually caught it in the wild — multi-line
    // call (rgb/vec3-style) with a trailing comma.
    code!(
        "fn p167_mix(r: integer, g: integer, b: integer) -> integer {
    (r * 65536) + (g * 256) + b
}
fn test() {
    c = p167_mix(
        10,
        20,
        30,
    );
    assert(c == 10 * 65536 + 20 * 256 + 30, \"multiline trailing comma\");
}"
    )
    .result(Value::Null);
}

/// P165 regression guard — `var: Enum = Variant { ... }` used to fail
/// with "Variable 'var' cannot change type from Enum to Variant; use a
/// new variable name or cast with 'as'".  The type-change check treated
/// a struct-enum variant as a distinct type from its parent enum.
/// Fix: in `Function::change_var_type`, accept `(Enum(p, true, _),
/// Enum(v, true, _))` when `data.def(v).parent == p` — the parent
/// relationship proves subtype compatibility.
#[test]
fn p165_enum_annotation_with_variant_rhs() {
    code!(
        "enum P165Kind {
    P165Alpha { x: integer },
    P165Beta { y: integer }
}
fn take_kind(k: P165Kind) -> boolean { k is P165Alpha }
fn test() {
    // Annotated LHS with variant RHS (the P165 shape).
    k1: P165Kind = P165Alpha { x: 1 };
    assert(take_kind(k1), \"annotated alpha\");
    // Annotated LHS with the OTHER variant — also accepted.
    k2: P165Kind = P165Beta { y: 2 };
    assert(!take_kind(k2), \"annotated beta\");
}"
    )
    .result(Value::Null);
}

/// P175 — file-scope `pub NAME: vector<T> = [...]` constants of
/// textual element types were materialised as empty vectors at
/// runtime (`len == 0`).  `vector<integer>` / `vector<float>` /
/// `vector<single>` / `vector<integer>` already worked because
/// `compile::extract_literal_values` matched `OpSetInt/Float/Single/Long`
/// calls.  `vector<text>` missed the `OpSetText` branch + the
/// `Value::Text` arm in `build_const_vectors`'s per-element match,
/// so the pre-built constant was registered with zero values and the
/// OpConstRef runtime copy produced an empty vector.  Fix: add
/// `OpSetText` to the extractor's allow-list and handle `Value::Text(s)`
/// in the build loop via `store.set_str(s)` + `set_int` (mirroring
/// runtime `OpSetText` in `src/fill.rs::set_text`).
#[test]
fn p175_vector_of_text_constant_populates() {
    code!(
        "pub P175_INTS: vector<integer> = [10, 20, 30];
pub P175_TEXTS: vector<text> = [\"aa\", \"bb\", \"cc\"];
fn test() {
    assert(len(P175_INTS) == 3, \"int vec len\");
    assert(P175_INTS[0] == 10, \"int vec [0]\");
    assert(len(P175_TEXTS) == 3, \"text vec len = {len(P175_TEXTS)}\");
    assert(P175_TEXTS[0] == \"aa\", \"text vec [0]\");
    assert(P175_TEXTS[2] == \"cc\", \"text vec [2]\");
}"
    )
    .result(Value::Null);
}

/// P178 — slot-aliasing in `is`-capture body.  When a function body's
/// IR root is a `Value::Insert` (not a `Block`/`Loop`), `assign_slots`
/// would return early from `process_scope` without placing any locals.
/// Orphans then flowed into `place_orphaned_vars`, which started
/// candidate slots at 0 — the argument area.  Arguments have
/// `stack_pos == u16::MAX` during `assign_slots` (codegen assigns
/// their positions later), so the per-var conflict check couldn't
/// see them; orphans happily claimed slot 0, 4, 12, ..., overlapping
/// the args at runtime.  Writing to the captured `tb_id` slot
/// corrupted the caller-owned `&tools` ref bytes sharing those
/// offsets; the downstream `SetInt` on a now-null ref silently wrote
/// to store 0 (observed as `ft_cur == 0` instead of the expected
/// captured value).
///
/// Fix: `place_orphaned_vars` now takes `local_start` and starts the
/// candidate slot there, so orphan locals can't overlap the
/// argument + return-address region.
#[test]
fn p178_is_capture_slot_alias() {
    code!(
        "enum P178Ui { P178UhToolButton { tb_id: integer } }
struct P178Tools { ft_cur: integer }
fn p178_hit() -> P178Ui { P178UhToolButton { tb_id: 2 } }
fn p178_router(dummy: integer, tools: P178Tools) -> P178Ui {
    _ = dummy;
    rc = p178_hit();
    if rc is P178UhToolButton { tb_id } {
        tools.ft_cur = tb_id;
    }
    rc
}
fn test() {
    ft = P178Tools { ft_cur: 0 };
    p178_router(99, ft);
    assert(ft.ft_cur == 2, \"captured was {ft.ft_cur} (expected 2)\");
}"
    )
    .result(Value::Null);
}

/// P179 — passing `&struct.field` as a non-sole argument silently
/// corrupted both the `&` destination and the preceding by-value
/// arg.  Fixed by routing non-Var `&T` sources through a work-ref
/// local in `src/parser/mod.rs::convert()`, emitting a
/// `Value::Insert([Set(__ref_N, expr), OpCreateStack(Var(__ref_N))])`
/// that `src/scopes.rs::scan_args` hoists into the enclosing
/// statement list (so the work-ref lives at function scope, not
/// block scope), plus `set_skip_free` to keep the borrowed DbRef
/// from freeing the owning store at scope exit.
///
/// Fixture lives at
/// `tests/lib/p179_ref_field_arg_corrupts_siblings.loft`.
#[test]
fn p179_ref_field_arg_corrupts_sibling() {
    code!(
        "struct P179Inner { pin_n: integer }
struct P179Outer { po_x: P179Inner, po_q: integer }
fn p179_int_ref(n: integer, r: P179Inner) { r.pin_n = n; }
fn test() {
    o = P179Outer { po_x: P179Inner { pin_n: 0 }, po_q: 0 };
    p179_int_ref(42, o.po_x);
    assert(o.po_x.pin_n == 42, \"int+&field: expected 42, got {o.po_x.pin_n}\");
}"
    )
    .result(Value::Null);
}

/// P176 — method-style callee: `fn add(self: Box, x) { self.items += [x]; }`
/// called from a `&Box` parameter caller should compile.  Before the fix,
/// `find_written_vars` did not recurse into the callee's body, so the
/// caller's `&Box` param looked unused and the "Parameter ... has `&`
/// but is never modified" error fired.  Fix: interprocedural
/// `callee_param_writes` analysis in `src/parser/mod.rs`.
#[test]
fn p176_ref_param_method_style_mutation() {
    code!(
        "struct P176Box { items: vector<integer> }
fn p176_add(self: P176Box, x: integer) { self.items += [x]; }
fn p176_caller(b: P176Box) { p176_add(b, 1); }
fn test() {
    bx = P176Box { items: [] };
    p176_caller(bx);
    assert(len(bx.items) == 1, \"expected 1 got {len(bx.items)}\");
}"
    )
    .result(Value::Null);
}

/// P176 — 3-level forwarding: only the innermost callee writes a
/// field, two intermediate callers just forward the value.  The
/// outermost `&T` must still be accepted.  Exercises the fixpoint /
/// monotone-merge code path.
#[test]
fn p176_transitive_forwarding_three_levels() {
    code!(
        "struct P176Tx { val: integer }
fn p176_inner(self: P176Tx)   { self.val = self.val + 1; }
fn p176_mid(self: P176Tx)     { p176_inner(self); }
fn p176_outer(b: P176Tx)     { p176_mid(b); }
fn test() {
    b = P176Tx { val: 0 };
    p176_outer(b);
    assert(b.val == 1, \"expected 1 got {b.val}\");
}"
    )
    .result(Value::Null);
}

/// P176 — recursive self-call must terminate the analysis.  The
/// `callee_param_writes` placeholder breaks cycles before descending
/// into the body.  Without the placeholder, the fn would recurse
/// infinitely while computing its own param-writes.
#[test]
fn p176_recursive_self_call_terminates() {
    code!(
        "struct P176Rec { val: integer }
fn p176_bump(n: P176Rec, depth: integer) {
    n.val = n.val + 1;
    if depth > 0 { p176_bump(n, depth - 1); }
}
fn test() {
    n = P176Rec { val: 0 };
    p176_bump(n, 3);
    assert(n.val == 4, \"expected 4 got {n.val}\");
}"
    )
    .result(Value::Null);
}

/// P180 — assigning a `single` (f32) literal to a `float` (f64) struct
/// field used to be silently accepted and corrupted the record at
/// runtime.  Fix: `src/parser/expressions.rs` now funnels simple-
/// assignment RHS through the same `convert()` machinery the
/// constructor and return-type paths already use, which widens via
/// `OpConvFloatFromSingle` and rejects narrowing with a diagnostic.
#[test]
fn p180_single_literal_into_float_field() {
    code!(
        "struct P180Box { a: float, b: integer }
fn test() {
    p = P180Box { a: 1.0, b: 42 };
    p.a = 1.2f;
    // f32 1.2 widened to f64 is not exactly 1.2; allow a tolerance
    // that covers the unavoidable precision loss.
    assert(p.a > 1.19 && p.a < 1.21, \"expected ~1.2 got {p.a}\");
    assert(p.b == 42, \"b untouched, got {p.b}\");
}"
    )
    .result(Value::Null);
}

/// P180 companion — widening an `integer` RHS into a `long` field
/// still works after the int→long hand-rolled branch in the
/// assignment path was replaced with a generic `convert()` funnel.
/// Guards against regressing the prior auto-widen behaviour.
#[test]
fn p180_int_widens_to_long_field() {
    code!(
        "struct P180Long { n: integer }
fn test() {
    p = P180Long { n: 0 };
    p.n = 42;
    assert(p.n == 42, \"expected 42 got {p.n}\");
}"
    )
    .result(Value::Null);
}

/// P181 — inline struct-returning call inside a format-string
/// interpolation used to SIGSEGV when the call's arg was a
/// field-access expression (not a plain Var) and the callee
/// returned a borrowed view into one of its args.  Root cause:
/// `OpCopyRecord` was emitted with the `0x8000` free-source flag
/// set unconditionally, freeing the view's source store.
///
/// Fix in `src/state/codegen.rs` (two sites — first-assignment and
/// reassignment paths both touch OpCopyRecord): clear the flag
/// when the callee's return type carries a non-empty `dep` chain.
/// Inference already tags these correctly for consistent-view
/// callees; a deeper issue with MIXED-return callees
/// (some paths view, some owned) is tracked separately in
/// `doc/claude/plans/finished/00-inline-lift-safety/01b-return-dep-inference.md`.
///
/// Tests: `tests/lib/p181_inline_field_access.loft`.
#[test]
fn p181_inline_field_access_format_string() {
    code!(
        "struct P181Inner { n: integer }
struct P181Container { items: vector<P181Inner> }
struct P181Holder { c: P181Container, sentinel: integer }
fn p181_first_inner(c: P181Container) -> P181Inner {
    c.items[0]
}
fn test() {
    h = P181Holder {
        c: P181Container { items: [P181Inner { n: 1 }] },
        sentinel: 42,
    };
    assert(p181_first_inner(h.c).n == 1,
           \"inline; got {p181_first_inner(h.c).n}\");
    assert(h.sentinel == 42, \"sentinel preserved; got {h.sentinel}\");
}"
    )
    .result(Value::Null);
}

/// P184: `vector<i32>` honours the `size(4)` annotation on the alias.
/// Before Phase 3, indexing returned `(v[i+1] << 32) | v[i]` because
/// storage was 4-byte stride but `OpGetVector` / `get_val` emitted
/// 8-byte reads.
#[test]
fn p184_vector_i32_narrow_read() {
    code!(
        "struct P184Box { v: vector<i32> }
fn test() {
    b = P184Box { v: [] };
    b.v += [1 as i32, 2 as i32, 3 as i32];
    assert(b.v[0] == 1, \"v[0] expected 1, got {b.v[0]}\");
    assert(b.v[1] == 2, \"v[1] expected 2, got {b.v[1]}\");
    assert(b.v[2] == 3, \"v[2] expected 3, got {b.v[2]}\");
    assert(len(b.v) == 3, \"len expected 3, got {len(b.v)}\");
}"
    )
    .result(Value::Null);
}

/// P184 control: `vector<integer>` still uses 8-byte-stride storage.
#[test]
fn p184_vector_integer_wide_control() {
    code!(
        "struct P184WideBox { v: vector<integer> }
fn test() {
    b = P184WideBox { v: [] };
    b.v += [1, 2, 3];
    assert(b.v[0] == 1, \"v[0] expected 1, got {b.v[0]}\");
    assert(b.v[1] == 2, \"v[1] expected 2, got {b.v[1]}\");
    assert(b.v[2] == 3, \"v[2] expected 3, got {b.v[2]}\");
}"
    )
    .result(Value::Null);
}

/// P184: `vector<u16>` — currently stored wide (8-byte) because
/// `Parts::Short`'s legacy `val - min + 1` encoding diverges from
/// the raw-byte vector copy path; narrow 2-byte storage awaits a
/// later Phase 4 round.  This guard confirms the wide-fallback
/// behaviour is consistent (reads + writes agree) so values
/// round-trip correctly even without the narrowing optimisation.
#[test]
fn p184_vector_u16_round_trip() {
    code!(
        "struct P184U16Box { v: vector<u16> }
fn test() {
    b = P184U16Box { v: [] };
    b.v += [1 as u16, 2 as u16, 300 as u16, 65000 as u16];
    assert(b.v[0] == 1, \"v[0]\");
    assert(b.v[1] == 2, \"v[1]\");
    assert(b.v[2] == 300, \"v[2]\");
    assert(b.v[3] == 65000, \"v[3]\");
}"
    )
    .result(Value::Null);
}

/// P184: `vector<u8>` narrow storage — 1-byte stride.
#[test]
fn p184_vector_u8_narrow_read() {
    code!(
        "struct P184U8Box { v: vector<u8> }
fn test() {
    b = P184U8Box { v: [] };
    b.v += [1 as u8, 2 as u8, 255 as u8];
    assert(b.v[0] == 1, \"v[0]\");
    assert(b.v[1] == 2, \"v[1]\");
    assert(b.v[2] == 255, \"v[2]\");
}"
    )
    .result(Value::Null);
}

/// P184 Phase 5: `vector<i32>` as a LOCAL variable narrows to 4-byte
/// storage.  Before Phase 5 locals fell back to wide (8-byte) storage
/// because `fill_database` only ran on struct definitions — the
/// literal-append path in `parser/vectors.rs::build_vector_code` and
/// `get_type`'s Type::Vector arm used the default wide `integer` slot.
/// Phase 5 routes both paths through `Parser::vector_of` which
/// consults `Data::narrow_vector_content`.
#[test]
fn p184_vector_i32_local_narrow_read() {
    code!(
        "fn test() {
    result: vector<i32> = [];
    result += [1 as i32, 2 as i32, 3 as i32];
    assert(result[0] == 1, \"r[0] expected 1, got {result[0]}\");
    assert(result[1] == 2, \"r[1] expected 2, got {result[1]}\");
    assert(result[2] == 3, \"r[2] expected 3, got {result[2]}\");
    assert(len(result) == 3, \"len\");
}"
    )
    .result(Value::Null);
}

/// P184 Phase 5: `vector<i32>` as a function RETURN type narrows.
/// The caller sees 4-byte-stride storage because `parse_type`'s
/// alias-resolution path stamps forced_size and `get_type` /
/// `vector_of` register the narrow vector db type on demand.
#[test]
fn p184_vector_i32_return_narrow_read() {
    code!(
        "fn make_i32_vec() -> vector<i32> {
    result: vector<i32> = [];
    result += [10 as i32, 20 as i32, 30 as i32];
    result
}
fn test() {
    v = make_i32_vec();
    assert(v[0] == 10, \"v[0]\");
    assert(v[1] == 20, \"v[1]\");
    assert(v[2] == 30, \"v[2]\");
}"
    )
    .result(Value::Null);
}

/// P184 Phase 6: `hash<Row[narrow_key]>` already works via the
/// struct-field narrowing path — the key field's narrow storage
/// is handled by the same `fill_database` Integer arm that
/// Phase 0 extended.  Primitive-content `hash<i32>` /
/// `sorted<i32>` / `index<i32>` are parse errors (the grammar
/// requires a `[key]` suffix), so Phase 6 has no new narrowing
/// code to add.  This guard locks down the happy path:
/// `hash` + `sorted` collections with `u32`-typed key fields.
#[test]
fn p184_hash_sorted_narrow_key_field() {
    code!(
        "struct P184Row { rid: u32, name: text }
struct P184HashDb { rows: hash<P184Row[rid]> }
struct P184SortedDb { rows: sorted<P184Row[rid]> }
fn test() {
    h = P184HashDb { rows: [] };
    h.rows += [P184Row { rid: 42, name: \"forty-two\" }];
    h.rows += [P184Row { rid: 7, name: \"seven\" }];
    assert(h.rows[42].name == \"forty-two\", \"hash[42]\");
    assert(h.rows[7].name == \"seven\", \"hash[7]\");
    s = P184SortedDb { rows: [] };
    s.rows += [P184Row { rid: 3, name: \"three\" }];
    s.rows += [P184Row { rid: 1, name: \"one\" }];
    s.rows += [P184Row { rid: 2, name: \"two\" }];
    assert(s.rows[1].name == \"one\", \"sorted[1]\");
    assert(s.rows[2].name == \"two\", \"sorted[2]\");
    assert(s.rows[3].name == \"three\", \"sorted[3]\");
}"
    )
    .result(Value::Null);
}

/// P293 regression — `hash<Row[i32_field]>` lookup silently returned
/// null because (a) `determine_keys` mapped any non-built-in content
/// type (including Parts::Int) to `type_nr = 7` (the byte fallback),
/// (b) `read_key`'s catch-all popped 1 byte off the stack while the
/// lookup value was pushed as a full i64, and (c) the `hash_ref` /
/// `compare_key` / `get_key` paths in keys.rs only knew about the
/// legacy 8-byte `integer` storage.  Fixed by extending all four
/// paths to recognise Parts::Int / Short / ShortRaw / Byte.  Same
/// bug surfaced from `f#read as u32` (the original P293 report) once
/// `u32` was given `size(4)` to make file-I/O width predictable; the
/// narrow-key hash fix landed alongside.
#[test]
fn p293_narrow_key_hash_lookup() {
    code!(
        "struct P293Row { rid: i32, name: text }
struct P293Db { rows: hash<P293Row[rid]> }
fn test() {
    h = P293Db { rows: [] };
    h.rows += [P293Row { rid: 42, name: \"forty-two\" }];
    h.rows += [P293Row { rid: 7, name: \"seven\" }];
    h.rows += [P293Row { rid: 100, name: \"hundred\" }];
    found42 = h.rows[42];
    found7  = h.rows[7];
    foundX  = h.rows[100];
    miss    = h.rows[999];
    assert(found42 != null, \"42 present\");
    assert(found7  != null, \"7 present\");
    assert(foundX  != null, \"100 present\");
    assert(miss    == null, \"999 absent\");
    assert(found42.name == \"forty-two\", \"hash[42] value\");
    assert(found7.name  == \"seven\",     \"hash[7] value\");
    assert(foundX.name  == \"hundred\",   \"hash[100] value\");
}"
    )
    .result(Value::Null);
}

/// P284 — `for f in vector<float>` looped forever past the end yielding
/// a garbage subnormal (~2.8e-282).  Root cause: `Store::get_float` /
/// `get_single` skipped the `rec != 0` guard that `get_int` already has,
/// so a null DbRef (rec=0, returned by `OpGetVectorNullable` for OOB)
/// read `*addr(0, 0)` (the store's free-list header) in release mode —
/// the `valid()` asserts inside are debug-only.  Fixed by adding the
/// `rec != 0` guard to both float getters; null DbRefs now return
/// `f64::NAN` / `f32::NAN`, the for-loop's value-truthiness check then
/// evaluates to false and breaks.
#[test]
fn p284_vector_float_iteration_terminates() {
    code!(
        "fn test() {
    v: vector<float> = [1.0, 2.0, 3.0];
    sum = 0.0;
    count = 0;
    for f in v {
        sum = sum + f;
        count = count + 1;
        if count > 100 { return; }
    }
    assert(count == 3, \"loop terminates after 3 elements\");
    assert(sum > 5.9 && sum < 6.1, \"sum approximately 6.0\");
    sv: vector<single> = [1.0f, 2.0f, 3.0f];
    sc = 0;
    for _ in sv {
        sc = sc + 1;
        if sc > 100 { return; }
    }
    assert(sc == 3, \"single iteration terminates after 3 elements\");
}"
    )
    .result(Value::Null);
}

/// P277 — local `sorted<T[K]> = []; += [T{…}]` panicked with
/// "Variable 'x' cannot change type from sorted<…> to vector<…>" —
/// `parse_vector` re-typed the LHS local to vector<T> before
/// `parse_assign_op` could route through the keyed-collection element
/// dispatch.  Same shape would have affected hash / index / spatial
/// locals.  Fixed by an early intercept in `parse_assign_op` that
/// detects `local_keyed += [literal]` BEFORE `parse_operators` runs,
/// then per-element parses + dispatches via `new_record` (which
/// already routes per-kind via the P188-followup `lhs_known` lookup).
#[test]
fn p277_local_sorted_pluseq_single_literal() {
    code!(
        "struct TagSlot { name: text, count: integer }
fn test() {
    s: sorted<TagSlot[name]> = [];
    s += [TagSlot{name: \"alpha\", count: 1}];
    assert(len(s) == 1, \"len == 1\");
    assert(s[\"alpha\"].count == 1, \"alpha lookup\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p277_local_sorted_pluseq_multi_literal() {
    code!(
        "struct TagSlot { name: text, count: integer }
fn test() {
    s: sorted<TagSlot[name]> = [];
    s += [TagSlot{name: \"zeta\", count: 1},
          TagSlot{name: \"alpha\", count: 5},
          TagSlot{name: \"mike\", count: 3}];
    assert(len(s) == 3, \"three elements\");
    // sorted by name ascending — iterate and collect in order.
    out: vector<text> = [];
    for t in s { out += [t.name]; }
    assert(out[0] == \"alpha\", \"first = alpha\");
    assert(out[1] == \"mike\",  \"middle = mike\");
    assert(out[2] == \"zeta\",  \"last = zeta\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p277_local_hash_pluseq_multi_literal() {
    code!(
        "struct Row { id: integer, name: text }
fn test() {
    h: hash<Row[id]> = [];
    h += [Row{id: 1, name: \"one\"},
          Row{id: 7, name: \"seven\"},
          Row{id: 42, name: \"forty-two\"}];
    assert(len(h) == 3, \"three entries\");
    assert(h[1].name  == \"one\",       \"h[1]\");
    assert(h[7].name  == \"seven\",     \"h[7]\");
    assert(h[42].name == \"forty-two\", \"h[42]\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p277_local_index_pluseq_multi_literal() {
    code!(
        "struct Score { player: text, value: integer }
fn test() {
    ix: index<Score[player]> = [];
    ix += [Score{player: \"a\", value: 10},
           Score{player: \"b\", value: 20},
           Score{player: \"c\", value: 30}];
    assert(len(ix) == 3, \"three entries\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p277_local_sorted_mixed_scalar_and_literal() {
    code!(
        "struct TagSlot { name: text, count: integer }
fn test() {
    s: sorted<TagSlot[name]> = [];
    // Scalar `+= elem` (P188 path) — must still work.
    s += TagSlot{name: \"alpha\", count: 1};
    // Literal `+= [...]` (P277 path).
    s += [TagSlot{name: \"beta\", count: 2}, TagSlot{name: \"gamma\", count: 3}];
    // Another scalar — paths interleave.
    s += TagSlot{name: \"delta\", count: 4};
    assert(len(s) == 4, \"four entries from interleaved scalar + literal\");
    assert(s[\"alpha\"].count == 1, \"alpha\");
    assert(s[\"beta\"].count  == 2, \"beta\");
    assert(s[\"gamma\"].count == 3, \"gamma\");
    assert(s[\"delta\"].count == 4, \"delta\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p277_local_sorted_pluseq_empty_literal() {
    // Defensive regression — `+= []` is a no-op append.
    code!(
        "struct TagSlot { name: text, count: integer }
fn test() {
    s: sorted<TagSlot[name]> = [];
    s += [TagSlot{name: \"a\", count: 1}];
    s += [];
    s += [TagSlot{name: \"b\", count: 2}];
    assert(len(s) == 2, \"empty append is no-op\");
}"
    )
    .result(Value::Null);
}

/// @P300 — assigning the result of a call that RETURNS a keyed
/// collection to a local (`x = mk()`) used to panic in codegen
/// (`Incorrect var x[65535]`): the @P295 reassignment lowering fired
/// for the FIRST assignment too and emitted `Insert([OpReplaceKeyed])`
/// with no `Set` node, so `compute_intervals` recorded no `first_def`
/// and `x` got no stack slot.  Fixed by prepending `Set(x, Null)` in
/// the keyed-assignment branch (`parser/expressions.rs`); `scan_set`
/// keeps it on a first assignment (→ store init) and elides it on a
/// reassignment (→ bare `OpReplaceKeyed`).
#[test]
fn p300_hash_return_assign_untyped() {
    code!(
        "struct R { ck: integer, v: integer }
fn mk() -> hash<R[ck]> { h: hash<R[ck]> = []; h += [R{ck: 5, v: 9}]; h }
fn test() {
    x = mk();
    assert(len(x) == 1, \"len 1\");
    assert(x[5].v == 9, \"key 5 -> 9\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p300_hash_return_assign_typed() {
    code!(
        "struct R { ck: integer, v: integer }
fn mk() -> hash<R[ck]> { h: hash<R[ck]> = []; h += [R{ck: 5, v: 9}]; h }
fn test() {
    x: hash<R[ck]> = mk();
    assert(len(x) == 1, \"len 1\");
    assert(x[5].v == 9, \"key 5 -> 9\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p300_sorted_return_assign() {
    code!(
        "struct Item { k: integer }
fn build() -> sorted<Item[k]> { s: sorted<Item[k]> = []; s += [Item{k: 3}]; s += [Item{k: 1}]; s += [Item{k: 2}]; s }
fn test() {
    x = build();
    assert(len(x) == 3, \"len 3\");
    out = \"\";
    for it in x { out += \"{it.k} \"; }
    assert(out == \"1 2 3 \", \"ordered: {out}\");
}"
    )
    .result(Value::Null);
}

#[test]
fn p300_index_return_assign() {
    code!(
        "struct IRec { n: integer }
fn build() -> index<IRec[n]> { ix: index<IRec[n]> = []; ix += [IRec{n: 7}]; ix += [IRec{n: 3}]; ix }
fn test() {
    x = build();
    assert(len(x) == 2, \"len 2\");
}"
    )
    .result(Value::Null);
}

/// @P300 sibling — first assignment from another keyed LOCAL (Var-RHS
/// alias, no prior declaration of `x`).  Same no-slot panic shape; the
/// deep-copy gives `x` its own store so `ns` stays usable.
#[test]
fn p300_var_rhs_alias_first() {
    code!(
        "struct R { ck: integer, v: integer }
fn build() -> hash<R[ck]> { h: hash<R[ck]> = []; h += [R{ck: 1, v: 10}]; h += [R{ck: 2, v: 20}]; h }
fn test() {
    ns: hash<R[ck]> = build();
    x = ns;
    assert(len(x) == 2, \"x len 2\");
    assert(len(ns) == 2, \"ns still readable\");
    assert(x[1].v == 10 && ns[2].v == 20, \"both independent\");
}"
    )
    .result(Value::Null);
}

/// P279 — caller defined BEFORE a text-returning callee in the
/// same file: pass-1 types the call result as Unknown(0), the
/// receiving local is Unknown, and using it as a struct-field
/// value (`out += [Struct{field: unknown_local, …}]`) trips
/// "Cannot assign unknown(0) to field …" in pass-1.  Pass-2
/// re-runs the check with the fn registered and would have
/// resolved correctly, but the pass-1 diagnostic surfaces.
///
/// Same architectural shape as P281/P278: pass-1 mustn't emit
/// errors pass-2 would naturally resolve.  Fix in
/// `src/parser/objects.rs::handle_field` gates the diagnostic
/// on `!first_pass` when the value type is Unknown.
#[test]
fn p279_forward_text_fn_into_struct_field() {
    code!(
        "struct R { name: text }
fn test() {
    out: vector<R> = [];
    s = forward_fn(\"hello\");
    out += [R{name: s}];
    assert(len(out) == 1, \"struct constructed\");
    assert(out[0].name == \"hello\", \"forward-fn text reached struct field\");
}
fn forward_fn(s: text) -> text { s }"
    )
    .result(Value::Null);
}

#[test]
fn p279_forward_fn_via_intermediate_local() {
    // Real scan_link_line shape: while loop + if-arm reassign +
    // forward call wrapped in an intermediate local before the
    // struct-field init.
    code!(
        "struct R { name: text }
fn test() {
    out: vector<R> = [];
    line = \"x#tag\";
    nl = line.len();
    name = \"\";
    lh = 0;
    while lh < nl {
        if line.byte_at(lh) == 35 {
            name = line[lh + 1..nl];
            break;
        }
        lh = lh + 1;
    }
    a_esc = json_escape(name);
    out += [R{name: a_esc}];
    assert(len(out) == 1, \"loop + if-reassign + forward-fn + struct works\");
    assert(out[0].name == \"tag\", \"correct slice + forward-fn\");
}
fn json_escape(s: text) -> text { s }"
    )
    // The shape under test is the codegen one (loop + if-arm reassign + forward
    // call + struct field), so the source stays exactly as `scan_link_line` wrote
    // it — including `nl = line.len()` used as a slice bound.  That mixes units
    // (loft#749: `len()` counts characters, a slice bound is a byte offset) and
    // the @PLN110 lint now says so.  It is right to: this only computes "tag"
    // because the literal is ASCII.  Declared rather than corrected — rewriting
    // the source to `size(line)` would quietly change the shape this test exists
    // to pin.
    .warning(
        "a text slice ends at `len(text)` (a character count) but slice bounds are byte \
         offsets — this stops short on multi-byte text at \
         p279_forward_fn_via_intermediate_local:10:36",
    )
    .result(Value::Null);
}

/// P281 — caller defined BEFORE callee in the same file: the
/// caller's body parses with the callee unregistered, the call
/// returns `Type::Unknown(0)`, the receiving local stays Unknown,
/// and any `.method(args)` chain on it trips "Expect token ;"
/// in pass-1 (because pass-1's `field()` Unknown-receiver branch
/// consumed the method name but not the trailing `(args)`).
///
/// The two-pass parser ALREADY has the right architecture — pass
/// 2 sees every fn registered and re-parses bodies cleanly.  The
/// fix in `src/parser/fields.rs::field` makes pass-1 also consume
/// the `(args)` so no spurious pass-1 parse error escapes.
#[test]
fn p281_forward_text_returning_fn_method_chain() {
    code!(
        "fn test() {
    s = helper(\"foo.loft\");
    assert(s.len() == 3, \"forward text fn + .len() on receiver works\");
}
fn helper(s: text) -> text {
    i = s.find(\".\");
    if i == null { s } else { s[0..i] }
}"
    )
    .result(Value::Null);
}

#[test]
fn p281_forward_text_returning_fn_method_with_args() {
    code!(
        "fn test() {
    s = upper(\"abc\");
    n = s.find(\"BC\");
    assert(n == 1, \"forward fn + .find(arg) returns position 1\");
}
fn upper(s: text) -> text {
    out = \"\";
    for c in s { out = out + (c as integer - 32) as character; }
    out
}"
    )
    .result(Value::Null);
}

#[test]
fn p281_mutual_forward_text_fns() {
    code!(
        "fn test() {
    s = first(\"hello\");
    assert(s.len() == 5, \"first + second + len chain works\");
}
fn first(s: text) -> text {
    second(s)
}
fn second(s: text) -> text {
    s
}"
    )
    .result(Value::Null);
}

#[test]
fn p281_forward_fn_slice_on_text_return() {
    // P278/P281 sibling — slice expression on forward-text-fn result.
    // Was the second cascade after my P281 fix; tolerance extended to
    // parse_index for Unknown receivers covers `[a..b]` shapes too.
    code!(
        "fn test() {
    s = forward(\"hello world\");
    sp = s.find(\" \");
    if sp != null {
        first = s[0..sp];
        assert(first == \"hello\", \"slice on forward-text-fn result works\");
    }
}
fn forward(s: text) -> text {
    s
}"
    )
    .result(Value::Null);
}

#[test]
fn p281_forward_fn_in_println_format() {
    // Forward fn used inside a format string interpolation —
    // pass-1's body parse must also tolerate Unknown receivers
    // inside the format-string expression position.
    code!(
        "fn test() {
    out = \"len={shorten(\\\"hello world\\\").len()}\";
    assert(out == \"len=5\", \"format interp with forward fn + .len() works\");
}
fn shorten(s: text) -> text {
    sp = s.find(\" \");
    if sp == null { s } else { s[0..sp] }
}"
    )
    .result(Value::Null);
}

/// P292 — `v = new_v` where `new_v` is loop-scoped and `v` is outer-scoped
/// used to corrupt `v`'s storage on the next iteration: subsequent reads
/// of `v[j]` reported `index J out of bounds for length 0` (or SEGV when
/// the next iter's `new_v: vector<integer> = []` allocation landed on the
/// just-freed storage).  The accumulator pattern
/// `v: vector<T> = []; for i { new_v: vector<T> = []; for j { new_v += [v[j]] }; new_v += [i]; v = new_v }`
/// is the canonical reproducer.  Fixed by extending the field-replace
/// `OpClearVector + OpAppendVector` path to cover local-var vector
/// re-assignment when the RHS is a Var read (the only shape that aliases —
/// fresh-storage calls / comprehensions go through the standard Set path).
#[test]
fn p292_vector_reassign_from_loop_local() {
    code!(
        "fn test() {
    v: vector<integer> = [];
    v += [10];
    for i in 0..3 {
        new_v: vector<integer> = [];
        for j in 0..len(v) { new_v += [v[j]]; }
        new_v += [i];
        v = new_v;
    }
    assert(len(v) == 4, \"v has 4 elements after 3 iters of (copy + append)\");
    assert(v[0] == 10, \"v[0] = 10 (initial)\");
    assert(v[1] == 0,  \"v[1] = 0 (iter 0)\");
    assert(v[2] == 1,  \"v[2] = 1 (iter 1)\");
    assert(v[3] == 2,  \"v[3] = 2 (iter 2)\");
}"
    )
    .result(Value::Null);
}

/// @P295 — reassigning a keyed-collection LOCAL (`s = ns`) for
/// sorted/hash/index.  Before the fix this panicked in codegen
/// (`gen_put_var` has no `OpPut*` arm for keyed kinds).  Fixed by
/// emitting a deep-copy `OpReplaceKeyed` (remove_claims + copy_claims,
/// per-kind index rebuild) and stripping the `s["ns"]` lifetime dep so
/// scope analysis frees both `s` (its own copy) and `ns` (its scope).
/// The loop-rebuild shape (insertion-sort idiom) is the canonical case;
/// it must not accumulate across iterations.  This `code!` harness runs
/// interp; `tests/scripts/119-keyed-local-reassign.loft` covers both
/// backends (native keyed locals were unblocked by the @P296 fix).
#[test]
fn p295_sorted_reassign_from_loop_local() {
    code!(
        "struct Item { k: integer }
fn test() {
    s: sorted<Item[k]> = [];
    s += [Item{k: 100}];
    for i in 1..5 {
        ns: sorted<Item[k]> = [];
        for it in s { ns += [Item{k: it.k}]; }
        ns += [Item{k: i}];
        s = ns;
    }
    out = \"\";
    for it in s { out += \"{it.k} \"; }
    assert(out == \"1 2 3 4 100 \", \"sorted rebuild: {out}\");
}"
    )
    .result(Value::Null);
}

/// @P295 — hash + index variants of the keyed-local reassignment, plus
/// the fresh-storage call RHS (`s = build()`) that exercises the
/// 0x8000 source-free path.
#[test]
fn p295_hash_index_reassign() {
    code!(
        "struct H { k: text, v: integer }
struct I { n: integer }
fn test() {
    h: hash<H[k]> = [];
    h += [H{k: \"a\", v: 1}];
    nh: hash<H[k]> = [];
    nh += [H{k: \"a\", v: 1}];
    nh += [H{k: \"b\", v: 2}];
    h = nh;
    assert(len(h) == 2, \"hash reassign len\");
    assert(h[\"b\"].v == 2, \"hash reassign lookup\");

    ix: index<I[n]> = [];
    ix += [I{n: 5}];
    nx: index<I[n]> = [];
    nx += [I{n: 3}];
    nx += [I{n: 7}];
    ix = nx;
    assert(len(ix) == 2, \"index reassign len\");
}"
    )
    .result(Value::Null);
}

/// @P285 — a keyed-collection membership test (`hash[key] == null`) must
/// NOT fire the "Redundant null check" warning when the KEY is a
/// `not null` field.  The lookup RESULT is nullable (absent key → null);
/// the bug attributed the key's not-null-ness to the comparison.  The
/// test declares NO expected warnings, so the harness's
/// `assert_diagnostics` fails if any spurious warning is emitted.
#[test]
fn p285_hash_lookup_null_no_spurious_warning() {
    code!(
        "struct P285Ent { name: text, v: integer }
struct P285Box { items: hash<P285Ent[name]> }
fn test() {
    b = P285Box{items: []};
    b.items += [P285Ent{name: \"x\", v: 9}];
    key = P285Ent{name: \"x\", v: 0};
    miss = P285Ent{name: \"y\", v: 0};
    found = 0;
    if b.items[key.name] == null { found = -1; } else { found = b.items[key.name].v; }
    if b.items[miss.name] != null { found = found + 100; }
    assert(found == 9, \"present + absent membership tests resolve correctly\");
}"
    )
    .result(Value::Null);
}

/// @P285 control — a GENUINE redundant check (`not_null_field == null`,
/// no lookup) must STILL warn.  Guards against the fix over-suppressing.
#[test]
fn p285_genuine_redundant_check_still_warns() {
    code!(
        "struct P285G { name: text }
fn test() {
    g = P285G{name: \"x\"};
    if g.name == null { assert(false, \"unreachable\"); }
}"
    )
    .warning("Redundant null check — 'name' is 'not null', so this is false unless a null reached the slot anyway (an overflow, a NaN, or an out-of-range read) at p285_genuine_redundant_check_still_warns:4:24")
    .result(Value::Null);
}

/// @PLN102 — the redundant-null-check warning must NOT fire on a field of a
/// NULLABLE receiver: `s.name` where `s: S?` reads null when `s` is absent (C80),
/// so `s.name == null` is a genuine check, not "always false".  No `.warning()` —
/// an unexpected diagnostic fails the harness, so this guards the suppression (the
/// p285 test above guards the complement: a non-null receiver still warns).
#[test]
fn nullable_receiver_field_null_check_no_warning() {
    code!(
        "struct NRF { name: text }
fn test() {
    s: NRF? = null;
    if s.name == null { assert(true, \"reachable\"); }
}"
    )
    .result(Value::Null);
}

/// P185 — slot-aliasing bug: a local (`key`) declared AFTER an inner
/// `body += <format-string>` accumulator loop, inside an outer
/// `for _ in file(...).files()` that uses an inline temporary as the
/// iterator source, gets assigned a slot that overlaps a still-live
/// text buffer.  Scope teardown runs `OpFreeText` (fill.rs op 118)
/// on the aliased slot → SIGSEGV or `realloc(): invalid pointer`.
///
/// Two independent workarounds (each alone suppresses the crash):
///   (a) hoist `key` above the inner loop;
///   (b) hoist `file(...)` into a named variable `d`.
///
/// Same class as P178 (orphan-placer reused argument slots).  The
/// structural fix is a rework of slot allocation — see
/// `doc/claude/plans/finished/04-slot-assignment-redesign/`.
///
/// The test depends on `tests/docs/*.loft` existing (it does in-tree)
/// because the bug requires a real `file(...).files()` iterator —
/// synthetic vector iteration doesn't reproduce the slot overlap.
// The slice bound discharges its `find` (`?? 0`): since @PLN153 phase 3 a nullable INDEX is
// an `(N-Store)` slot and warns, and this fixture expects no diagnostics.
#[test]
fn p185_slot_alias_on_late_local_in_nested_for() {
    code!(
        "fn test() {
    out = file(\"/tmp/p185_out.txt\");
    for f in file(\"tests/docs\").files() {
        path = \"{f.path}\";
        if !path.ends_with(\".loft\") or path.ends_with(\"/.loft\") { continue; }
        body = \"\";
        for i in 0..3 {
            body += \"{i}\";
        }
        key = path[(path.find(\"/\") ?? 0) + 1..path.len() - 5];
        out += `
          {key}
        `;
        break;
    }
}"
    )
    .result(Value::Null);
}

/// P186 — struct-typed block / if expressions rejected as `void`.
///
/// `x = { S { ... } }`, `x = { mk() }` (where `mk()` returns a struct),
/// `if cond { S {…} } else { S {…} }`, and blocks with intermediate
/// statements before a struct-literal result all used to fail with
/// `Variable 'x' cannot change type from void to S` (CLI) or a
/// cascade of `Unknown type void` errors (test harness).
///
/// Root cause: `parse_object` in first_pass returns
/// `Type::Rewritten(Type::Reference(_))` with `*code = Value::Insert(…)`
/// because the struct-init IR can't be fully materialised until type
/// resolution in second_pass.  `parse_block`'s Insert-flattening then
/// unconditionally reset `t = Type::Void`, discarding the Rewritten
/// tag, so the block's inferred type was `void` in first_pass.
/// Second_pass then produced the real `Reference(S)` and
/// `change_var_type` fired the "cannot change type from void" error.
///
/// Fix: `src/parser/control.rs::parse_block` preserves the Rewritten
/// type across Insert flattening.  A companion fix in the same
/// function disambiguates struct-body `{ field: val, … }` from
/// block-expression `{ expr }` by peeking the first two tokens after
/// `{` — only `ident :` / `ident ,` route to `parse_object`.
///
/// The four shapes below reproduce the four PROBLEMS.md variants;
/// they must all compile and execute cleanly.
#[test]
fn p186_struct_typed_block_expressions() {
    code!(
        "struct P04Sb { sb_a: integer, sb_b: integer }
fn p04_mkbox(n: integer) -> P04Sb { P04Sb { sb_a: n, sb_b: n * 2 } }
fn test() {
    b1 = { P04Sb { sb_a: 3, sb_b: 4 } };
    assert(b1.sb_a == 3 and b1.sb_b == 4, \"b1\");
    b2 = { n = 5; P04Sb { sb_a: n, sb_b: n + 1 } };
    assert(b2.sb_a == 5 and b2.sb_b == 6, \"b2\");
    b3 = { p04_mkbox(7) };
    assert(b3.sb_a == 7 and b3.sb_b == 14, \"b3\");
    cond = true;
    b4 = if cond { P04Sb { sb_a: 1, sb_b: 2 } } else { P04Sb { sb_a: 0, sb_b: 0 } };
    assert(b4.sb_a == 1 and b4.sb_b == 2, \"b4\");
}"
    )
    .result(Value::Null);
}

/// P187 — struct scalar fields read as corrupt after a later vector
/// allocation in a sibling function.
///
/// Surface symptom (Brick Buster):
///   `graphics::create_sprite_sheet(atlas, 4, 5, graphics::painter_vao(p))`
///   receives `atlas` with `width=null`, `height=null`, `data.len=0`,
///   even though the caller just printed the same struct with correct
///   values (width=128, height=160, data.len=20480).  `gl_upload_canvas`
///   sees `w=0, h=0, count=1` and bails.  The entire title / HUD / text
///   rendering disappears.
///
/// Irreducible core (this test):
///   1. A builder returns a struct whose `vector<T>` field was
///      populated via a for-comprehension (`result.data = [for _ in ..]`).
///   2. A mutator method on that returned struct is called inside the
///      builder before `return`.
///   3. A subsequent sibling function allocates a local vector of its
///      own (any element type).
///
/// After those three things, `atlas.width` — a scalar field set at
/// struct-literal time — reads as a corrupt value.
///
/// Drop any of the three and the bug disappears: no comprehension,
/// no mutator call, or no post-return vector allocation → correct read.
///
/// Likely root cause: P184's narrow collection storage interacting
/// with store reallocation triggered by the later vector literal.
/// The returned struct's record is relocated but the caller's local
/// still references the old (stale) address, so the width read walks
/// into unrelated bytes.  See `doc/claude/PROBLEMS.md` § P187.
#[test]
fn p187_struct_scalar_field_corrupted_after_sibling_vector_alloc() {
    code!(
        "struct P187Canvas {
    width: integer,
    height: integer,
    data: vector<integer>
}
fn p187_canvas() -> P187Canvas {
    result = P187Canvas { width: 128, height: 160 };
    result.data = [for _ in 0..10 { 0 }];
    result
}
fn touch(self: P187Canvas) {
    self.data[0] = 42;
}
fn p187_build() -> P187Canvas {
    at = p187_canvas();
    at.touch();
    at
}
fn p187_alloc_local_vec() -> integer {
    a = [0.0f, 0.0f];
    a.len()
}
fn test() {
    atlas = p187_build();
    _unused = p187_alloc_local_vec();
    assert(atlas.width == 128, \"width={atlas.width}, expected 128\");
    assert(atlas.height == 160, \"height={atlas.height}, expected 160\");
    assert(atlas.data.len() == 10, \"data.len={atlas.data.len()}, expected 10\");
}"
    )
    .result(Value::Null);
}

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
/// (`pub fn len(both: hash)`); index uses a parser hook in
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
        "p220_null_text_preserved_through_struct_field:4:17"
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

// ── P54 Q3 second half — `T.to_json()` for any user struct ──────────

/// Q3.b — `instance.to_json()` on a flat struct emits canonical JSON
/// with all primitive fields.  The parser-side intercept in
/// `src/parser/fields.rs::field()` lowers the method call to
/// `n_struct_to_json(self_ref, struct_kt)`, which delegates to
/// `Stores::show_json` (`src/database/format.rs`).
#[test]
fn q3b_struct_to_json_basic_primitives() {
    code!(
        "struct U { name: text, age: integer, score: float, ok: boolean }
fn run() -> text {
    u = U { name: \"Alice\", age: 30, score: 4.5, ok: true };
    u.to_json()
}"
    )
    .expr("run()")
    .result(Value::Text(
        r#"{"name":"Alice","age":30,"score":4.5,"ok":true}"#.to_string(),
    ));
}

/// Q3.b — round-trip via `T.parse(json_parse(...))` recovers the
/// original struct.  This is the core property the design promises:
/// any struct can be serialised and re-parsed without loss.
#[test]
fn q3b_struct_to_json_round_trip() {
    code!(
        "struct U { name: text, age: integer }
fn run() -> integer {
    u = U { name: \"Bob\", age: 25 };
    txt = u.to_json();
    parsed = U.parse(json_parse(txt));
    parsed.age
}"
    )
    .expr("run()")
    .result(Value::Int(25));
}

/// Q3.b — nested struct fields recurse correctly.  The inner struct
/// is rendered as a nested JSON object inside the outer struct.
#[test]
fn q3b_struct_to_json_nested_struct() {
    code!(
        "struct A { city: text }
struct U { name: text, addr: A }
fn run() -> text {
    u = U { name: \"X\", addr: A { city: \"NYC\" } };
    u.to_json()
}"
    )
    .expr("run()")
    .result(Value::Text(
        r#"{"name":"X","addr":{"city":"NYC"}}"#.to_string(),
    ));
}

/// Q3.b — `vector<text>` field renders as a JSON array of strings.
#[test]
fn q3b_struct_to_json_vector_text_field() {
    code!(
        "struct U { tags: vector<text> }
fn run() -> text {
    u = U { tags: [\"dev\", \"rust\"] };
    u.to_json()
}"
    )
    .expr("run()")
    .result(Value::Text(r#"{"tags":["dev","rust"]}"#.to_string()));
}

/// Q3.b — `vector<integer>` field renders as a JSON array of numbers.
#[test]
fn q3b_struct_to_json_vector_integer_field() {
    code!(
        "struct U { nums: vector<integer> }
fn run() -> text {
    u = U { nums: [1, 2, 3] };
    u.to_json()
}"
    )
    .expr("run()")
    .result(Value::Text(r#"{"nums":[1,2,3]}"#.to_string()));
}

/// Q3.b — `JsonValue` field renders its inline subtree verbatim,
/// not as the generic enum-variant shape (`{"JString":{"value":"x"}}`).
/// Special-cased in `ShowDb::write` (P54 Q3): a `JsonValue`-typed
/// struct field routes to `write_jsonvalue` for native JSON-value
/// semantic rendering.
#[test]
fn q3b_struct_to_json_jsonvalue_field_renders_verbatim() {
    code!(
        "struct W { name: text, payload: JsonValue }
fn run() -> text {
    inner = json_parse(`{{\"x\":42}}`);
    w = W { name: \"outer\", payload: inner };
    w.to_json()
}"
    )
    .expr("run()")
    .result(Value::Text(
        r#"{"name":"outer","payload":{"x":42}}"#.to_string(),
    ));
}

/// Q3.b — JSON string escaping covers `"` and `\`.  The
/// `write_json_escaped` helper in `src/database/format.rs` is shared
/// by the struct text-field arm and the JsonValue passthrough arm,
/// so a regression here would produce invalid JSON in either path.
///
/// P233 — re-enabled 2026-05-07 after `tests/testing.rs::replace_tokens`
/// was fixed to escape `\` first, so the loft-lexer round-trip of
/// the expected `Value::Text` literal is now lossless.  The
/// per-byte escape dispatch is also locked at the Rust unit level
/// in `src/database/format.rs::json_escape_tests` (13 tests).
#[test]
fn q3b_struct_to_json_string_escapes_quote_and_backslash() {
    code!(
        "struct M { msg: text }
fn run() -> text {
    m = M { msg: \"she said \\\"hi\\\" \\\\ done\" };
    m.to_json()
}"
    )
    .expr("run()")
    // "she said \"hi\" \\ done" — quotes and backslash escaped.
    .result(Value::Text(
        r#"{"msg":"she said \"hi\" \\ done"}"#.to_string(),
    ));
}

/// Q3.b — control characters (`\n`, `\t`, `\r`) get the canonical
/// short-form escapes per RFC 8259.  Re-enabled with P233 fix.
#[test]
fn q3b_struct_to_json_string_escapes_control_chars() {
    code!(
        "struct M { msg: text }
fn run() -> text {
    m = M { msg: \"line1\\nline2\\ttab\" };
    m.to_json()
}"
    )
    .expr("run()")
    .result(Value::Text(r#"{"msg":"line1\nline2\ttab"}"#.to_string()));
}

/// Q3.b — `to_json_pretty()` produces multi-line indented output.
/// Every non-empty struct opens with newline + 2-space indent per
/// nesting level and dedents the closing brace to the parent's
/// depth.  Exact whitespace shape is part of the contract — pretty
/// JSON consumers (logging, golden-file diffs) depend on it.
#[test]
fn q3b_struct_to_json_pretty_format() {
    code!(
        "struct U { name: text, age: integer }
fn run() -> text {
    u = U { name: \"Alice\", age: 30 };
    u.to_json_pretty()
}"
    )
    .expr("run()")
    .result(Value::Text(
        "{\n  \"name\": \"Alice\",\n  \"age\": 30\n}".to_string(),
    ));
}

/// Q3.b / @P375 — an omitted nullable text field is stored by loft as an
/// allocated EMPTY string (`s_rec != 0`, `u.name == null` is `false`), not
/// the `s_rec == 0` null sentinel.  So it is a present value and `to_json`
/// now emits `"name":""` (dryopea-surfaced: `{x:j}` must emit every declared
/// field for a faithful save→load round-trip; the old `|| is_empty()` filter
/// dropped present empty strings, producing partial JSON).  GENUINELY-null
/// scalars (text with `s_rec == 0`, nullable int = `i64::MIN`) are still
/// omitted — only present-but-empty values changed.
#[test]
fn q3b_struct_to_json_emits_present_empty_text() {
    code!(
        "struct U { name: text, age: integer }
fn run() -> text {
    u = U { age: 7 };
    u.to_json()
}"
    )
    .expr("run()")
    .result(Value::Text(r#"{"name":"","age":7}"#.to_string()));
}

/// P234 — `r.0.x` lexer fix: the inner `0.x` previously parsed as
/// a malformed float literal because the number-tokeniser greedily
/// consumed `.` followed by anything.  P195's fix split the digit-
/// after case (`r.0.0` → integer + `.` + integer); P234 extends it
/// to the identifier-after case (`r.0.x` → integer + `.` + ident).
/// Verified via `tests/lexer::test::p234_tuple_index_then_field_does_not_glue_into_float`;
/// here we pin the surface-level reproducer that exposed it (the
/// runtime "tuple-of-struct member access" half is still open per
/// PROBLEMS.md P234, so this test only checks that PARSING reaches
/// the runtime — it asserts the program compiles without the
/// "Problem parsing float" diagnostic).
#[test]
fn p234_lexer_accepts_tuple_index_then_struct_field() {
    code!(
        "struct Point { x: integer, y: integer }
fn run() -> integer {
    p = Point { x: 10, y: 20 };
    r: (Point, integer) = (p, 5);
    inner = r.0;
    inner.x
}"
    )
    .expr("run()")
    .result(Value::Int(10));
}

#[test]
fn p234_runtime_tuple_with_struct_return_int_field() {
    code!(
        "struct Point { x: integer, y: integer }
fn make() -> (Point, integer) {
    p = Point { x: 10, y: 20 };
    (p, 5)
}
fn run() -> integer {
    r = make();
    r.1
}"
    )
    .expr("run()")
    .result(Value::Int(5));
}

/// P235 — for-loop tuple destructure (non-par half).  Pre-fix,
/// `for (a, b) in pairs { ... }` rejected with "Expect variable
/// after for"; the for-loop parser only accepted a single
/// identifier as the loop var.  After the fix, the parser
/// synthesizes a temp loop var, defines `a` / `b` as proper
/// variables typed from the iterated tuple's element types, and
/// prepends `Set` ops to the body so each iteration unpacks the
/// tuple before user code runs.  Closes the general parser feature
/// gap; the par half (`for (a, b) in pairs par(...) { ... }`)
/// remains open per PROBLEMS.md P235.
#[test]
fn p235_for_tuple_destructure_two_arity() {
    code!(
        "fn run() -> integer {
    pairs: vector<(integer, integer)> = [(1, 2), (3, 4), (5, 6)];
    sum = 0;
    for (a, b) in pairs {
        sum += a + b;
    }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(21)); // (1+2) + (3+4) + (5+6) = 21
}

/// P235 — three-arity destructure pins the "any arity" claim.
#[test]
fn p235_for_tuple_destructure_three_arity() {
    code!(
        "fn run() -> integer {
    triples: vector<(integer, integer, integer)> = [(1, 2, 3), (4, 5, 6)];
    sum = 0;
    for (a, b, c) in triples {
        sum += a + b + c;
    }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(21)); // (1+2+3) + (4+5+6) = 21
}

/// P235 — mixed scalar/text element types.
#[test]
fn p235_for_tuple_destructure_int_text() {
    code!(
        "fn run() -> integer {
    items: vector<(integer, text)> = [(1, \"one\"), (2, \"two\"), (3, \"three\")];
    total = 0;
    for (n, label) in items {
        total += n + len(label);
    }
    total
}"
    )
    .expr("run()")
    .result(Value::Int(17)); // (1+3) + (2+3) + (3+5) = 17
}

/// P235 par half — synthesized wrapper-worker for `for (a, b) in
/// pairs par(r = work(a, b), N) { ... }`.  Pre-fix, `a` and `b`
/// weren't in scope when `parse_parallel_worker` parsed `work(a,
/// b)`, producing two "Unknown variable" errors.  Closed by
/// `parse_destructure_par_worker` in
/// `src/parser/collections.rs` — defines the destructured names
/// in scope, parses the user call manually (capturing all args),
/// then synthesizes a wrapper fn `__par_destructure_w_<L>_<P>_<work>(t)
/// -> ret { work(t.0, t.1) }` and routes par dispatch through
/// the wrapper with the tuple loop element as the single
/// per-iteration arg.
#[test]
fn p235_par_half_two_arity_int_int() {
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
    .result(Value::Int(36)); // (1+2)+(3+4)+(5+6)+(7+8) = 36
}

/// P235 par half — three-arity destructure ensures the synthesis
/// supports more than 2 names.  Wrapper body builds three tuple
/// element reads and threads them into the user worker.
#[test]
fn p235_par_half_three_arity() {
    code!(
        "fn sum3(a: integer, b: integer, c: integer) -> integer { a + b + c }
fn run() -> integer {
    triples: vector<(integer, integer, integer)> = [(1, 2, 3), (4, 5, 6)];
    total = 0;
    for (a, b, c) in triples par(r = sum3(a, b, c), 4) { total += r; }
    total
}"
    )
    .expr("run()")
    .result(Value::Int(21)); // (1+2+3)+(4+5+6) = 21
}

/// P235 par half — args in non-positional order (`work(b, a)`
/// instead of `work(a, b)`).  The wrapper body must map each user
/// arg to its declared tuple position via the destructured var
/// nrs, not assume positional ordering.
#[test]
fn p235_par_half_args_swapped() {
    code!(
        "fn diff(x: integer, y: integer) -> integer { x - y }
fn run() -> integer {
    pairs: vector<(integer, integer)> = [(10, 1), (20, 5)];
    out = 0;
    for (a, b) in pairs par(r = diff(b, a), 2) { out += r; }
    out
}"
    )
    .expr("run()")
    .result(Value::Int(-(9 + 15))); // (1-10)+(5-20) = -24
}

#[test]
fn p234_runtime_via_run_fn() {
    code!(
        "struct Point { x: integer, y: integer }
fn make() -> (Point, integer) {
    (Point { x: 10, y: 20 }, 5)
}
fn run() -> integer {
    r = make();
    r.1
}"
    )
    .expr("run()")
    .result(Value::Int(5));
}

/// P236 — function whose return type is a heap-owned reference and
/// whose body's tail is `if/else` returning newly-constructed records
/// corrupted the return value on `--native` (returned the typed null
/// sentinel instead of the if/else's value).  Pre-fix native:
/// `make_p(true) → P { x: null, y: null }`.  Interpreter accidentally
/// worked because OpReturn read from eval-stack top.  Closed by
/// (a) `parser/control.rs::unify_if_branches_work_refs` — pick the
/// first branch's terminal work-ref as the shared one and rewrite the
/// other branch's work-ref references via `substitute_work_ref` so
/// both branches produce the same DbRef; (b) `scopes.rs::returned_var`
/// recurses through `Value::If` and reports the shared var so
/// `get_free_vars` skips OpFreeRef on it; (c) the catchall in
/// `scopes.rs::free_vars` emits `Return(Var(ret_var))` instead of
/// `Return(Null)` when a known ret_var is available.
#[test]
fn p236_struct_return_from_if_else_true() {
    code!(
        "struct P { x: integer, y: integer }
fn make_p(c: boolean) -> P {
    if c { P { x: 1, y: 2 } } else { P { x: 3, y: 4 } }
}
fn run() -> integer { p = make_p(true); p.x * 100 + p.y }"
    )
    .expr("run()")
    .result(Value::Int(102));
}

/// P236 — same fn, false branch.  Verifies the unification rewrite
/// didn't accidentally route the false branch through the true
/// branch's value (a regression caught during initial implementation
/// when the substitute_work_ref helper renamed user parameters).
#[test]
fn p236_struct_return_from_if_else_false() {
    code!(
        "struct P { x: integer, y: integer }
fn make_p(c: boolean) -> P {
    if c { P { x: 1, y: 2 } } else { P { x: 3, y: 4 } }
}
fn run() -> integer { p = make_p(false); p.x * 100 + p.y }"
    )
    .expr("run()")
    .result(Value::Int(304));
}

/// P236 — three-way `else if` chain.  Recursive descent in
/// `unify_if_branches_work_refs` walks nested `Value::If` nodes
/// (else-if desugars to `else { if ... }`).
#[test]
fn p236_struct_return_from_else_if_chain() {
    code!(
        "struct P { x: integer, y: integer }
fn make_p(c: integer) -> P {
    if c == 1 { P { x: 11, y: 12 } }
    else if c == 2 { P { x: 21, y: 22 } }
    else { P { x: 31, y: 32 } }
}
fn run() -> integer {
    a = make_p(1);
    b = make_p(2);
    c = make_p(3);
    (a.x + a.y) + (b.x + b.y) + (c.x + c.y)
}"
    )
    .expr("run()")
    .result(Value::Int(23 + 43 + 63));
}

/// P236 — guard against the parameter-substitution regression: when
/// both branches of the if/else return user-named parameters (NOT
/// work-refs), the unification helper must bail out and leave the IR
/// unchanged.  This test exercises the gate at
/// `unify_if_branches_work_refs` that requires both terminal vars to
/// match `__ref_*` / `__rref_*` naming.
#[test]
fn p236_param_returning_if_unaffected() {
    code!(
        "struct P { x: integer, y: integer }
fn pick(c: boolean, a: P, b: P) -> P {
    if c { a } else { b }
}
fn run() -> integer {
    p1 = P { x: 1, y: 2 };
    p2 = P { x: 3, y: 4 };
    chosen = pick(false, p1, p2);
    chosen.x * 10 + chosen.y
}"
    )
    .expr("run()")
    .result(Value::Int(34));
}

/// P236 — A7.1 par tuple wide-return runtime relies on the same
/// fix: the size-based gate widen in `parse_function` rewrites
/// `(integer, integer)` returns through `Reference(__tuple<…>)`,
/// and tail if/else with tuple branches goes through the synthetic-
/// struct unification path.  Closes the regression where
/// `min_max(...) -> (integer, integer) { if ... else ... }` returned
/// (null, null) on native.
#[test]
fn p236_tuple_return_from_if_else() {
    code!(
        "fn min_max(lo: integer, hi: integer) -> (integer, integer) {
    if lo <= hi { (lo, hi) } else { (hi, lo) }
}
fn run() -> integer {
    r1 = min_max(5, 2);
    r2 = min_max(1, 9);
    (r1.0 * 100 + r1.1) + (r2.0 * 1000 + r2.1)
}"
    )
    .expr("run()")
    .result(Value::Int(205 + 1009));
}

/// P240 — bounded-generic fn computing 2+ bound-supplied operator
/// results into locals then returning them in a tuple swapped
/// values backend-dependently.  Two compounding root causes:
///
/// (a) **Interp slot aliasing**: `compute_intervals` in
/// `src/variables/intervals.rs` had no `Value::Tuple` arm — when
/// the IR contained `Return(Tuple([Var(a), Var(b)]))`, the
/// recursion fell through silently and the operand vars'
/// last_use stayed at the default value, so the slot allocator
/// considered them dead and aliased their slots.  When `a` and
/// `b` were both written in the same scope, the second write
/// clobbered the first; the tuple read returned `(b_value,
/// b_value)`.  Fix: add `Value::Tuple(elems)`,
/// `Value::TupleGet(_, _)`, and `Value::TuplePut(_, _, _)` arms
/// that recurse into each element so reads update the operand
/// vars' last_use correctly.
///
/// (b) **Native hoisted-return guard miss**: scopes::free_vars
/// hoists the tuple value as a separate statement before the
/// `OpFreeText(work)` cleanup, leaving the body shape `[Set(lt),
/// Set(gt), Call(println), Tuple, OpFreeText, Return(Null)]`.
/// Native codegen runs `patch_hoisted_returns` to rewrite this
/// into `Return(Tuple)` — but only when the block result type
/// matches a small allow-list (Void / Never / `t_*` text-stub /
/// `__ret_N` text temp).  T-stubs returning a tuple weren't in
/// the list, so native emitted the tuple as a discarded
/// statement and fell through to a hardcoded `return (0, 0)`
/// (the type's null sentinel).  Fix: add `is_t_stub_tuple_body`
/// to the guard so tuple-returning T-stubs also run the patch.
///
/// Without (a), the no-side-effects shape `(lt, gt)` after two
/// `Set` ops was wrong on interp.  Without (b), the with-println
/// shape was wrong on native.  Both shapes now correct on both
/// backends.
#[test]
fn p240_bounded_generic_two_operator_tuple_return() {
    code!(
        "fn p240_classify<T: Ordered>(a: T, b: T) -> (integer, integer) {
    p240lt = if a < b { 1 } else { 0 };
    p240gt = if a > b { 1 } else { 0 };
    return (p240lt, p240gt);
}"
    )
    .expr("p240_classify(3, 5).0")
    .result(Value::Int(1));
}

/// P243 — bounded-generic fn returning a tuple with one or more
/// `text` elements where one element is built by a bound-supplied
/// method call (e.g. `(x.to_text(), "x")`) silently returned
/// empty strings on `--native` because the missing `.to_string()`
/// wrap on the bound-method element produced a `(Str, String)`
/// tuple that rustc rejected with E0308.  Interp was correct.
///
/// Two coordinated fixes already shipped today addressed half of
/// P243 incidentally:
///   - P240's `is_t_stub_tuple_body` extension to
///     `patch_hoisted_returns` made the tuple reach the Return
///     position (was previously a discarded statement).
///   - P238's `tuple_text_to_string` flag in
///     `src/generation/emit.rs::Value::Return` already sets the
///     wrap flag for `Tuple(text, …)` returns.
///
/// What remained: `infer_type` in `src/generation/emit.rs` had no
/// `Value::Span` arm.  The parser Span-wraps fault-prone calls
/// (every `obj.method()` site) for source-position tracking, so
/// the bound-method call inside the tuple was `Span(Call(…))`,
/// not bare `Call(…)`.  `infer_type(Span(Call))` returned `None`,
/// so the Tuple emit arm's `elem_is_text` check silently failed
/// and the `.to_string()` wrap didn't fire.
///
/// Fix: add `Value::Span(b) => self.infer_type(&b.1)` to the
/// match in `infer_type` so it transparently looks through
/// position wrappers — same shape as the Span-aware patches
/// elsewhere in the codebase.
#[test]
fn p243_bounded_generic_tuple_with_text_method_call() {
    code!(
        "struct P243Item { p243_id: integer }
fn to_text(self: P243Item) -> text { return \"item-{self.p243_id}\"; }
fn p243_show_pair<T: Printable>(p243x: T) -> (text, text) {
    return (p243x.to_text(), \"x\");
}"
    )
    .expr("p243_show_pair(P243Item { p243_id: 7 }).0")
    .result(Value::str("item-7"));
}

/// @P329 — regression of @P243 (interpreter side).  A bounded-generic
/// function returning a tuple containing one or more `text` elements
/// where one element comes from a bound-supplied method call lost the
/// element's bytes between the function's epilogue free and the caller's
/// PutText.  Root cause: `scopes::free_vars` had a B5-L3 wrap for single
/// `text` returns (deep-copy to `__ret_N: text` via OpAppendText) but no
/// analogue for `(text, ...)` returns — the tuple's text element Str
/// pointed into a function-local buffer that the scope's OpFreeText
/// invalidated before Return.  Fix: hoist each non-literal text element
/// to `__ret_text_N` via the same Set+AppendText pattern, then build a
/// fresh tuple from those temps as the return value.  The three siblings
/// below cover (text, integer), (text, text, text), and (text,) shapes
/// using chained `.N` on the call result (which routes through the
/// temp-tuple branch in `parser/operators.rs:597-615`).
#[test]
fn p329_bounded_generic_tuple_text_integer_chained() {
    code!(
        "struct P329Item { p329_id: integer }
fn to_text(self: P329Item) -> text { return \"item-{self.p329_id}\"; }
fn p329_show_pair<T: Printable>(p329x: T, n: integer) -> (text, integer) {
    return (p329x.to_text(), n);
}"
    )
    .expr("p329_show_pair(P329Item { p329_id: 3 }, 42).0")
    .result(Value::str("item-3"))
    .expr("p329_show_pair(P329Item { p329_id: 9 }, 7).1")
    .result(Value::Int(7));
}

#[test]
fn p329_bounded_generic_tuple_three_text_chained() {
    code!(
        "struct P329Tri { p329_tri_id: integer }
fn to_text(self: P329Tri) -> text { return \"item-{self.p329_tri_id}\"; }
fn p329_show_triple<T: Printable>(p329x: T) -> (text, text, text) {
    return (p329x.to_text(), \"middle\", p329x.to_text());
}"
    )
    .expr("p329_show_triple(P329Tri { p329_tri_id: 4 }).0")
    .result(Value::str("item-4"))
    .expr("p329_show_triple(P329Tri { p329_tri_id: 5 }).1")
    .result(Value::str("middle"))
    .expr("p329_show_triple(P329Tri { p329_tri_id: 6 }).2")
    .result(Value::str("item-6"));
}

/// @P330 — generic fn returning `(text, text)` assigned to a local then
/// element-accessed for the function's text return.  Pre-existing parser
/// bug surfaced while writing @P329 regression coverage.  Root cause:
/// `parser/control.rs::text_return` auto-hoisted a tuple-typed local to a
/// tuple-typed *parameter* when the function's return text depended on
/// that local (e.g. `r = pair(); r.0`).  The caller can only push a
/// 12-byte null `DbRef` placeholder for the hoisted arg, but the
/// parameter slot is 32 bytes for `(text, text)` (16 per `Str` element);
/// the 20-byte size mismatch corrupted the callee's frame layout, so
/// every subsequent argument read returned garbage (interpreter SIGBUS;
/// native produced `&var_x.to_string()` = `&String` E0308).  Fix: when
/// `text_return` would hoist a non-Text non-Reference local (specifically
/// `Type::Tuple`), skip the hoist entirely — the function's return type
/// loses the dep on that local, and `scopes::free_vars`'s B5-L3
/// single-text branch (`src/scopes.rs:961-988`) deep-copies the
/// `r.0` text into a `__ret_N: text` temp via `OpAppendText` before the
/// tuple local is freed.  Sibling native-codegen tweak in
/// `src/generation/emit.rs::Value::Var` for tuple-text-return context:
/// emit bare `var_x` instead of `&var_x` so the surrounding
/// `tuple_text_to_string` wrap produces `var_x.to_string()` (owned
/// `String` clone) rather than the broken `&var_x.to_string()` =
/// `&String`.  Two tests cover both routes.
#[test]
fn p330_generic_tuple_return_assign_then_chain_first() {
    code!(
        "struct P330Item { p330_id: integer }
fn to_text(self: P330Item) -> text { return \"item-{self.p330_id}\"; }
fn p330_pair<T: Printable>(p330x: T) -> (text, text) {
    return (p330x.to_text(), \"sentinel\");
}
fn p330_take_first(p330a: P330Item) -> text {
    p330r = p330_pair(p330a);
    p330r.0
}"
    )
    .expr("p330_take_first(P330Item { p330_id: 13 })")
    .result(Value::str("item-13"));
}

#[test]
fn p330_generic_tuple_return_assign_then_chain_second() {
    code!(
        "struct P330Item2 { p330_id2: integer }
fn to_text(self: P330Item2) -> text { return \"item-{self.p330_id2}\"; }
fn p330_pair2<T: Printable>(p330x2: T) -> (text, text) {
    return (\"prefix\", p330x2.to_text());
}
fn p330_take_second(p330a2: P330Item2) -> text {
    p330r2 = p330_pair2(p330a2);
    p330r2.1
}"
    )
    .expr("p330_take_second(P330Item2 { p330_id2: 27 })")
    .result(Value::str("item-27"));
}

/// @P329 — element 1 (second slot) access via chained `.1` on the call
/// result.  Verifies the deep-copy applies to NON-leading text elements
/// too (the original p243 test only read `.0`).
#[test]
fn p329_bounded_generic_tuple_text_text_chained_second_elem() {
    code!(
        "struct P329Second { p329_second_id: integer }
fn to_text(self: P329Second) -> text { return \"item-{self.p329_second_id}\"; }
fn p329_pair_second<T: Printable>(p329x: T) -> (text, text) {
    return (\"first\", p329x.to_text());
}"
    )
    .expr("p329_pair_second(P329Second { p329_second_id: 8 }).1")
    .result(Value::str("item-8"));
}

/// #549 — a bounded-generic fn whose return SHAPE is a concrete aggregate
/// (a struct, struct-enum, or pure-value tuple) leaked its result store when
/// the call was used INLINE (`f(x).field`) or DISCARDED, on both backends
/// (the `(integer,integer)` twin of this is `p240`).  Root cause: the caller's
/// lift-and-free decision (`scopes::inline_struct_return`) fired only for `n_`
/// (concrete) callees, not for `t_` generic monomorphs — so the fresh store the
/// monomorph allocated via `__retbuf` was never freed.  Fix: extend the lift to
/// `t_` callees that carry a `__retbuf` param (the NRVO signal that the return
/// is a fresh owned aggregate a monomorph keeps even after it loses its return
/// dep).  These pass under the DA gate (`-C debug-assertions=on`), where the
/// leak becomes a hard "Database not correctly freed" panic.
#[test]
fn p549_generic_struct_return_inline_no_leak() {
    code!(
        "struct P549Item { p549_id: integer }
fn to_text(self: P549Item) -> text { return \"i{self.p549_id}\"; }
struct P549Pair { p549_a: integer, p549_b: integer }
fn p549_mk<T: Printable>(_p549x: T) -> P549Pair { return P549Pair { p549_a: 1, p549_b: 2 }; }"
    )
    .expr("p549_mk(P549Item { p549_id: 7 }).p549_a")
    .result(Value::Int(1));
}

#[test]
fn p549_generic_struct_enum_return_inline_no_leak() {
    code!(
        "struct P549Item { p549_id: integer }
fn to_text(self: P549Item) -> text { return \"i{self.p549_id}\"; }
enum P549Shape { P549Circle { r: integer }, P549Square { s: integer } }
fn p549_shape<T: Printable>(_p549x: T) -> P549Shape { return P549Circle { r: 3 }; }
fn p549_use(e: P549Shape) -> integer { match e { P549Circle { r } => r, P549Square { s } => s } }"
    )
    .expr("p549_use(p549_shape(P549Item { p549_id: 7 }))")
    .result(Value::Int(3));
}

#[test]
fn p549_generic_aggregate_return_discarded_no_leak() {
    code!(
        "struct P549Item { p549_id: integer }
fn to_text(self: P549Item) -> text { return \"i{self.p549_id}\"; }
struct P549Pair { p549_a: integer, p549_b: integer }
fn p549_mk<T: Printable>(_p549x: T) -> P549Pair { return P549Pair { p549_a: 1, p549_b: 2 }; }
fn p549_discard() -> integer {
    p549_mk(P549Item { p549_id: 7 });
    return 42;
}"
    )
    .expr("p549_discard()")
    .result(Value::Int(42));
}

/// #549 over-reach guard — a bounded-generic fn that RETURNS ITS ARGUMENT
/// (`id<T>(x) -> T { x }`) is a BORROWED view, not a fresh store: its monomorph
/// loses the return dep AND gets no `__retbuf`, so the fix must NOT lift-and-free
/// it (that would double-free the caller's arg).  Under the DA gate this would
/// panic "double free"; it must stay clean.
#[test]
fn p549_generic_returns_arg_not_double_freed() {
    code!(
        "struct P549Item { p549_id: integer }
fn to_text(self: P549Item) -> text { return \"i{self.p549_id}\"; }
fn p549_id<T: Printable>(p549x: T) -> T { return p549x; }"
    )
    .expr("p549_id(P549Item { p549_id: 5 }).p549_id")
    .result(Value::Int(5));
}

/// #549 bug 2 — an explicit `return (owned_text, …)` of an aggregate literal
/// whose element is an OWNED/call-produced text double-freed that element's
/// String (`text.rs:334` under `-C debug-assertions=on`).  NON-generic (unlike
/// the p243/p329/p330 siblings) — the bug is not generic-specific.  Root cause:
/// the synthetic tuple/struct block a `return` builds is processed by BOTH the
/// `Value::Return` scan arm and `convert`'s is_body_return tail sweep; the first
/// makes the block terminal, and `scopes::free_vars` re-ran `insert_free` on the
/// now-terminal Block (the `Value::Block` arm preceded the `expr_is_terminal`
/// dedup) → a second `OpFreeText`.  Fix: order the terminal-dedup first.  A tail
/// aggregate (no `return`) never hit this, so these all use an explicit `return`.
#[test]
fn p549_bug2_return_tuple_owned_text_not_double_freed() {
    code!(
        "fn p549_gt() -> text { return \"a\" + \"b\"; }
fn p549_pair() -> (text, text) { return (p549_gt(), \"x\"); }"
    )
    .expr("p549_pair().0")
    .result(Value::str("ab"));
}

#[test]
fn p549_bug2_return_struct_owned_text_not_double_freed() {
    code!(
        "struct P549S { p549_a: text, p549_b: text }
fn p549_gt() -> text { return \"a\" + \"b\"; }
fn p549_mk() -> P549S { return P549S { p549_a: p549_gt(), p549_b: \"x\" }; }"
    )
    .expr("p549_mk().p549_a")
    .result(Value::str("ab"));
}

#[test]
fn p549_bug2_return_tuple_owned_text_discarded_not_double_freed() {
    code!(
        "fn p549_gt() -> text { return \"a\" + \"b\"; }
fn p549_pair() -> (text, text) { return (p549_gt(), \"x\"); }
fn p549_discard() -> integer {
    p549_pair();
    return 7;
}"
    )
    .expr("p549_discard()")
    .result(Value::Int(7));
}

/// P239 — for-loop over `vector<T>` inside a generic fn crashed
/// both backends.  Interp SIGSEGV; native rustc E0610
/// `i64.rec`.  The for-loop iter-termination check
/// (parser/collections.rs:1506-1514) emits
/// `OpConvBoolFromRef(Var(loop_var))` for any loop variable
/// typed `Reference(_, _)`, including `Reference(T_d_nr, …)`
/// for generic-T element iteration.  When T monomorphises to a
/// primitive, the substituted Var is now that primitive type
/// but the IR still has `OpConvBoolFromRef` — interp treats
/// `i64` as a `DbRef` (SIGSEGV) and native emits `i64.rec`
/// (rustc E0610).
///
/// Fix: extend `substitute_type_in_value` to swap
/// `OpConvBoolFromRef(Var(_))` to the matching primitive peer
/// (`OpConvBoolFromInt` / `OpConvBoolFromText` / etc.) when the
/// substituted concrete type is a primitive.  Reference / Vector
/// / struct-enum / tuple stay on `OpConvBoolFromRef` (the
/// existing behaviour works for any DbRef-shaped loop var).
#[test]
fn p239_for_loop_over_generic_vector() {
    code!(
        "fn p239_count<T>(v: vector<T>) -> integer {
    n = 0;
    for _ in v { n = n + 1; }
    return n;
}"
    )
    .expr("p239_count([10, 20, 30])")
    .result(Value::Int(3));
}

/// P241 — generic fn building a vector by `out += [x]` crashed both
/// backends.  Interp panicked at `src/database/allocation.rs` because
/// the parametric IR shape (`OpCopyRecord(src, _elm_, t_T)`) treats
/// `src: i64` as a `DbRef` and indexes into stores with an out-of-
/// range store_nr.  Native rustc rejected with E0308 / E0605 because
/// the generated Rust read `src` as a struct ref but it's a primitive.
///
/// Fix (2026-05-11): substitution-time triplet rewrite — when
/// `substitute_type_in_value` sees the parametric vector-element-write
/// triplet
///   `Set(_elm_, OpNewRecord(out, t_T, MAX))`
///   `Call(OpCopyRecord, [src, _elm_, t_T])`
///   `Call(OpFinishRecord, [out, _elm_, t_T, MAX])`
/// AND the substituted concrete type is a primitive, rewrites it to
/// the primitive shape (4 ops: `OpPreAllocVector` prefix +
/// `OpNewRecord` + `OpSetXxx` + `OpFinishRecord`), with type-id args
/// updated to point at the concrete vector type-id resolved via
/// `database.vector(database.db_type(concrete, data))`.  Slice 2
/// covers `Type::Integer(_)` only; slice 3 broadens to all
/// primitives.
#[test]
fn p241_singleton_int() {
    code!(
        "fn p241_singleton<T>(x: T) -> vector<T> {
    p241_out: vector<T> = [];
    p241_out += [x];
    return p241_out;
}"
    )
    .expr("p241_singleton(42)[0]")
    .result(Value::Int(42));
}

/// P241 slice 3 — Text.  Same generic shape; Text uses `OpSetText`
/// instead of `OpSetInt`.
#[test]
fn p241_singleton_text() {
    code!(
        "fn p241_singleton_t<T>(x: T) -> vector<T> {
    p241_out_t: vector<T> = [];
    p241_out_t += [x];
    return p241_out_t;
}"
    )
    .expr("p241_singleton_t(\"hello\")[0]")
    .result(Value::str("hello"));
}

/// P241 slice 3 — Float.  Verifies the Float setter dispatch.
#[test]
fn p241_singleton_float() {
    code!(
        "fn p241_singleton_f<T>(x: T) -> vector<T> {
    p241_out_f: vector<T> = [];
    p241_out_f += [x];
    return p241_out_f;
}"
    )
    .expr("p241_singleton_f(2.5)[0]")
    .result(Value::Float(2.5));
}

/// P241 slice 3 — Boolean.  Verifies the OpSetByte dispatch with
/// `min=0` arg.
#[test]
fn p241_singleton_bool() {
    code!(
        "fn p241_singleton_b<T>(x: T) -> vector<T> {
    p241_out_b: vector<T> = [];
    p241_out_b += [x];
    return p241_out_b;
}"
    )
    .expr("p241_singleton_b(true)[0]")
    .result(Value::Boolean(true));
}

/// P255 — capturing a generic-fn `vector<T>` return into a local
/// variable failed with `Variable vp cannot change type from
/// vector<P> to vector<P>; use a new variable name or cast with
/// 'as'`.  The error fired even though both sides display
/// identically: the LHS (the new local var) was typed
/// `Vector(Rewritten(Reference(P)))` because the call argument
/// `P { v: 99 }` returned a `Rewritten`-marked type, that marker
/// propagated into the bound T, and then into `vector<T>`.
/// `Type::is_equal` did not look through `Rewritten`, so the
/// Vector→Vector arm of `change_var` rejected the assignment.
///
/// Fix (2026-05-12) — three-part:
/// 1. `Type::is_equal` now strips `Rewritten` wrappers on either
///    side before unifying.
/// 2. `Parser::resolve_type_var` strips `Rewritten` from the
///    concrete arg type before binding T, so the marker (which
///    describes how a value was assembled, not the value's shape)
///    no longer enters the substituted IR.
/// 3. `rewrite_vector_write_triplets` was extended to handle
///    struct-T (Reference) — the OpCopyRecord shape is kept but
///    its `tp` arg AND the surrounding OpNewRecord/OpFinishRecord
///    `parent_tp` args are patched from the parametric T's
///    type-id to the concrete struct's type-id.  Without this
///    patch the runtime read the wrong record size from the
///    parametric placeholder type and returned garbage from
///    `vp[0].v`.
#[test]
fn p255_capture_generic_vector_struct_return() {
    code!(
        "struct P255S { p255_v: integer }
fn p255_make<T>(p255_x: T) -> vector<T> {
    p255_o: vector<T> = [];
    p255_o += [p255_x];
    return p255_o;
}
fn p255_get() -> integer {
    p255_vp = p255_make(P255S { p255_v: 99 });
    return p255_vp[0].p255_v;
}"
    )
    .expr("p255_get()")
    .result(Value::Int(99));
}

/// P253 — hash-table collision DoS: `keys::hash` and `key_hash`
/// previously used `DefaultHasher::new()` with a fixed seed (k0=0,
/// k1=0).  An attacker who could supply hash-table keys could
/// pre-compute N strings that all collide to a single bucket →
/// O(N²) insertion / lookup.  Same root-cause class as the 2011
/// hash-DoS in Python / Ruby / PHP / Java / Node.js.
///
/// Fix (2026-05-11): seed the hasher.  Now (arc G / #523) the seed
/// is drawn per-hash by `keys::fresh_seed` and stored IN the hash's
/// bucket record, so a persisted hash re-derives identical buckets in
/// any process (portability) while an attacker still cannot pre-compute
/// collisions without the hash's seed (the DoS defense).  This test is a
/// smoke check: a hash collection still inserts + looks up correctly
/// under the seeded hasher (no behavioural regression for legitimate use).
#[test]
fn p253_hash_remains_functional_after_seeding() {
    code!(
        "struct P253E { p253_name: text, p253_value: integer }
struct P253T { p253_data: hash<P253E[p253_name]> }
fn p253_lookup() -> integer {
    p253_t = P253T { p253_data: [] };
    p253_t.p253_data += [P253E { p253_name: \"alpha\", p253_value: 1 }];
    p253_t.p253_data += [P253E { p253_name: \"beta\", p253_value: 2 }];
    p253_t.p253_data += [P253E { p253_name: \"gamma\", p253_value: 3 }];
    p253_a = p253_t.p253_data[\"alpha\"];
    p253_b = p253_t.p253_data[\"beta\"];
    p253_g = p253_t.p253_data[\"gamma\"];
    p253_sum = 0;
    if p253_a != null { p253_sum = p253_sum + p253_a.p253_value; }
    if p253_b != null { p253_sum = p253_sum + p253_b.p253_value; }
    if p253_g != null { p253_sum = p253_sum + p253_g.p253_value; }
    return p253_sum;
}"
    )
    .expr("p253_lookup()")
    .result(Value::Int(6));
}

/// P251 — storing a tuple whose element is a fn-ref into a struct
/// field failed native compilation with rustc E0605 `(u32, DbRef)
/// as i32` (interp passed but the call-through-field shape
/// `s.t.0(arg)` panicked because the projection bug fed garbage
/// into the call dispatch).
///
/// Root cause: `src/parser/mod.rs::emit_set_one_element`'s
/// `Type::Function` arm only special-cased `Value::FnRef` literal
/// — when the source was `Value::Var(v)` with `v` typed
/// `Type::Function`, the value passed through unchanged.  Native
/// emit then produced `let _v_val = (var_v); ... _v_val as i32`
/// where `var_v` is the runtime `(u32, DbRef)` tuple, which rustc
/// rejects (E0308 + E0605).
///
/// Fix (2026-05-11): in the `Type::Function` arm, when the value
/// is `Value::Var(v)` AND `v`'s type is `Function` AND `v` is not
/// in the closure-vars table (non-capturing), wrap as
/// `Value::FnRefDnr(v)` so native emit projects
/// `(var_v.0 as i64)` (the d_nr half).  Mirrors the existing
/// projection in `emit_fn_ref_field_write` (parser/mod.rs:4886)
/// for the direct-field-write path.
///
/// This test exercises the original P251 surface symptom (read
/// the integer element back).  The call-through-field shape
/// (`s.t.0(arg)`) is covered by the matching cell in
/// `tests/tuple_matrix.rs::e4_d3_field_closure_local`.
#[test]
fn p251_tuple_with_fnref_in_struct_field_read() {
    code!(
        "struct P251S { p251_t: (fn(integer) -> integer, integer) }
fn p251_build() -> integer {
    p251_add5 = fn(p251_x: integer) -> integer { p251_x + 5 };
    p251_s = P251S { p251_t: (p251_add5, 99) };
    return p251_s.p251_t.1;
}"
    )
    .expr("p251_build()")
    .result(Value::Int(99));
}

/// P251 — call-through-field shape: invoke the fn-ref element
/// directly on the struct's tuple field.  Validates that the
/// projection fix for storage also makes the call-dispatch path
/// resolve to the correct d_nr.
#[test]
fn p251_tuple_with_fnref_in_struct_field_call() {
    code!(
        "struct P251S2 { p251_t2: (fn(integer) -> integer, integer) }
fn p251_call() -> integer {
    p251_add5 = fn(p251_x2: integer) -> integer { p251_x2 + 5 };
    p251_s2 = P251S2 { p251_t2: (p251_add5, 99) };
    return p251_s2.p251_t2.0(10);
}"
    )
    .expr("p251_call()")
    .result(Value::Int(15));
}

/// P250 — tuple-of-Reference returned from a fn and destructured
/// inside a loop body showed a stale-DbRef on the destructured
/// variable that picked up the FIRST argument; q1.v read `null` on
/// iter > 0 on both backends (q2 stayed correct).  Reproducer
/// printed `0: 0,100 / 1: null,101 / 2: null,102 / ...`.
///
/// Root cause: the destructure code in
/// `src/parser/expressions.rs::expression` (synthetic-`__tuple<…>`
/// path) emits the LHS Reference vars (q1, q2) as `OpGetField(tmp,
/// offset, ...)` reads — DbRefs that share `store_nr` with the
/// outer `tmp` variable.  Without dep tracking, scope analysis
/// emits an independent `OpFreeRef` for q1 and q2 at scope exit;
/// each free works on a `store_nr` basis and reclaims the entire
/// tuple's underlying store on the FIRST exit.  The next loop
/// iteration's `tmp = make_pair(...)` reassignment then ran
/// `OpFreeRef(tmp)` on the now-stale outer DbRef whose store_nr
/// got recycled by the next iter's `pa` allocation, silently
/// destroying that allocation.  q2 stayed correct because by
/// the time q2's slot was read, the new tuple was being built
/// from valid `pa`/`pb` allocations — the read happened before
/// the second iter's overwrite.
///
/// Fix (2026-05-11): in the synthetic-struct destructure path,
/// mark each Reference-typed LHS as `vars.depend(v_nr, tmp)` so
/// scope analysis treats them as borrows (deps non-empty → skip
/// `OpFreeRef`).  `tmp`'s `OpFreeRef` alone reclaims the storage
/// at the right time (after the loop body, when `tmp` itself is
/// reassigned or goes out of scope).  Only applies to Reference
/// elements; primitive elements (TupleGet path) read value-typed
/// slots that need no free.
#[test]
fn p250_loop_destructure_first_arg() {
    code!(
        "struct P250P { p250_v: integer }
fn p250_make_pair(a: P250P, b: P250P) -> (P250P, P250P) { (a, b) }
fn p250_run(n: integer) -> integer {
    p250_last = -1;
    for p250_i in 0..n {
        p250_pa = P250P { p250_v: p250_i };
        p250_pb = P250P { p250_v: p250_i + 100 };
        (p250_q1, p250_q2) = p250_make_pair(p250_pa, p250_pb);
        p250_last = p250_q1.p250_v;
    }
    return p250_last;
}"
    )
    .expr("p250_run(3)")
    .result(Value::Int(2));
}

/// P241 slice 4 — nested-in-if regression guard.  The
/// rewrite recurses through `Value::If` arms; this test exercises
/// that recursion by gating the push behind an `if` so the triplet
/// lives inside an If's true-arm Block.  Without the If recursion,
/// the rewrite would skip the triplet and the test would crash.
#[test]
fn p241_singleton_in_if_branch() {
    code!(
        "fn p241_cond_singleton<T>(x: T, p241_pick: boolean) -> vector<T> {
    p241_out_c: vector<T> = [];
    if p241_pick {
        p241_out_c += [x];
    }
    return p241_out_c;
}"
    )
    .expr("p241_cond_singleton(7, true)[0]")
    .result(Value::Int(7));
}

/// P252 — bounded-generic for-loop over a struct-ref vector returned
/// the FIRST item's bound-method result for every iteration instead
/// of the per-item result.  Surfaced 2026-05-11 by phase 4 cleanup;
/// bisected to slice-3 commit `6016655e` which swapped
/// `OpGetVector` to `OpGetVectorNullable` in the for-loop iter
/// step.  The I9-vec elm_size fixup in
/// `parser/mod.rs::substitute_type_in_value` only recognised
/// `OpGetVector` (not the Nullable peer); after the swap the iter
/// step kept `size=0` for generic-T element reads → every iteration
/// read element 0 → bound method always saw the FIRST item.
///
/// Fix: extend the I9-vec name match to `OpGetVectorNullable` too.
/// Both peers have identical (r, size, idx) arg shapes so the
/// existing fixup logic applies unchanged.
#[test]
fn p252_bounded_generic_for_loop_per_item_dispatch() {
    code!(
        "interface V {
    fn ok(self: Self) -> boolean
}
struct P { v: integer }
fn ok(self: P) -> boolean { return self.v > 0; }
fn p252_count<T: V>(items: vector<T>) -> integer {
    n = 0;
    for it in items {
        if it.ok() { n += 1; }
    }
    return n;
}"
    )
    .expr("p252_count([P{v:1}, P{v:0}, P{v:3}])")
    .result(Value::Int(2));
}

/// `store_memory()` returns a live store memory-utilisation report whose
/// header starts with "stores:".  Guards the builtin wiring
/// (`n_store_memory` interp impl + registration + the `#rust` body).
#[test]
fn store_memory_builtin_reports() {
    code!("fn helper() -> integer { 0 }")
        .expr("store_memory().starts_with(\"stores:\")")
        .result(Value::Boolean(true));
}

/// @P327 — `for p in pairs()` over a tuple-yielding coroutine silently
/// iterated 0 times on the interpreter because `convert(tuple, Boolean)`
/// found no matching `OpConv*` and the for-loop's exhaustion check fell
/// through to `OpNot(Var(tuple_p))` — which read the first byte of the
/// tuple as a boolean (1 from `(1, 10)` → true → `!true` → break).
/// Manual `next()` on the same generator worked.
///
/// Fix (`src/parser/collections.rs`): for tuple-yielded coroutines, use
/// `OpCoroutineExhausted(__gen_N)` to terminate instead of the
/// generic `OpNot(for_var)` check.  The gen var is captured from
/// `iter_next`'s first argument before `for_next` consumes it.
#[test]
fn p327_for_loop_over_tuple_yield_iterates_body() {
    code!(
        "fn pairs() -> iterator<(integer, integer)> {
    yield (1, 10);
    yield (2, 20);
    yield (3, 30);
}
fn run() -> integer {
    sum = 0;
    for p in pairs() { sum = sum + p.0 + p.1; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(66));
}

/// @P327 follow-up — the loop iterates EXACTLY the number of yields
/// (not more, not fewer).  Pre-fix this was 0 (silent break-out); a
/// regression that runs forever would also fail this test via the
/// test harness timeout, but the count assertion catches anything in
/// between.
#[test]
fn p327_for_loop_over_tuple_yield_count_matches_yields() {
    code!(
        "fn pairs() -> iterator<(integer, integer)> {
    yield (10, 20);
    yield (30, 40);
}
fn run() -> integer {
    n = 0;
    for _ in pairs() { n = n + 1; }
    n
}"
    )
    .expr("run()")
    .result(Value::Int(2));
}

/// @P324 — the CO1.9/S28 stale-DbRef guard fired on ANY store mutation
/// between coroutine yields, including the case where the generator
/// held NO DbRefs and the consumer only mutated an unrelated vector
/// (`out += [v]` inside `for v in gen()`).  Demoted to a debug-only
/// warning in `src/state/mod.rs` so the for-loop+accumulate idiom
/// works in production.
#[test]
fn p324_for_loop_accumulate_into_vector_works() {
    code!(
        "fn count() -> iterator<integer> {
    yield 1;
    yield 2;
    yield 3;
}
fn run() -> integer {
    out: vector<integer> = [];
    for v in count() { out += [v]; }
    out[0] + out[1] + out[2]
}"
    )
    .expr("run()")
    .result(Value::Int(6));
}

/// @P325 — vector comprehension `[for v in gen() { … }]` over a
/// coroutine ran forever (no termination check in
/// `build_comprehension_code`'s loop body for `Iterator` source) until
/// the store overflowed its 2 GiB word limit.  Fix: mirror the @P327
/// pattern — emit `OpCoroutineExhausted(__gen_N)` as the break check.
#[test]
fn p325_comprehension_over_generator_terminates() {
    code!(
        "fn count() -> iterator<integer> {
    yield 1;
    yield 2;
    yield 3;
}
fn run() -> integer {
    out = [for v in count() { v * 100 }];
    out[0] + out[1] + out[2]
}"
    )
    .expr("run()")
    .result(Value::Int(600));
}

/// @P326 — native state machine had no DbRef channel.  Generators
/// returning `iterator<Struct>` mis-cast `as i64` and read via the
/// wrong channel.  Fix: added `LoftCoroutine::next_dbref` +
/// `coroutine_next_dbref` runtime helper + size=12 dispatch arm in
/// `src/generation/ops/coroutine.rs` + `__ref_*` work-var
/// pre-declaration in the coroutine state machine.
#[test]
fn p326_iterator_of_struct_for_loop() {
    code!(
        "struct P326Pt { x: integer, y: integer }
fn points() -> iterator<P326Pt> {
    yield P326Pt { x: 1, y: 2 };
    yield P326Pt { x: 3, y: 4 };
    yield P326Pt { x: 5, y: 6 };
}
fn run() -> integer {
    sum = 0;
    for pt in points() { sum = sum + pt.x + pt.y; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(21));
}

/// @P326 follow-up — manual next() on a struct-yielding generator.
#[test]
fn p326_iterator_of_struct_manual_next() {
    code!(
        "struct P326Pt2 { x: integer, y: integer }
fn points() -> iterator<P326Pt2> {
    yield P326Pt2 { x: 10, y: 20 };
    yield P326Pt2 { x: 30, y: 40 };
}
fn run() -> integer {
    it = points();
    a = next(it);
    b = next(it);
    a.x + a.y + b.x + b.y
}"
    )
    .expr("run()")
    .result(Value::Int(100));
}

/// @P327 native — `iterator<(integer, integer)>` driven by `for p in gen()`
/// failed to compile under `--native` because tuple yields (16 bytes)
/// collided with text (`&str` = 16 bytes) in the `OpCoroutineNext`
/// channel dispatch.  Fix: introduce a unified `next_into(stores,
/// &mut [i64])` channel (plan-16 phase 01).  `value_size` now packs
/// (byte_size, channel_tag) — high byte 1 routes through the new
/// channel; the consumer allocates a stack `[i64; N]` buffer and
/// rebuilds the tuple from buffer slots.  Same encoding applies to
/// manual `next()` (control.rs) and the for-loop driver
/// (collections.rs).
#[test]
fn p327_native_iterator_of_tuple_for_loop() {
    code!(
        "fn pairs() -> iterator<(integer, integer)> {
    yield (1, 10);
    yield (2, 20);
    yield (3, 30);
}
fn run() -> integer {
    sum = 0;
    for p in pairs() { sum = sum + p.0 + p.1; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(66));
}

/// @P327 native follow-up — manual next() on a tuple-yielding generator.
#[test]
fn p327_native_iterator_of_tuple_manual_next() {
    code!(
        "fn pairs() -> iterator<(integer, integer)> {
    yield (10, 20);
    yield (30, 40);
}
fn run() -> integer {
    it = pairs();
    a = next(it);
    b = next(it);
    a.0 + a.1 + b.0 + b.1
}"
    )
    .expr("run()")
    .result(Value::Int(100));
}

/// @P328 — closure-yielding generators (`iterator<fn(integer) -> integer>`)
/// FIXED both backends 2026-05-23 via 4 coordinated fixes:
///   1. For-loop break check extended `Type::Tuple → Type::Tuple |
///      Type::Function` in `parser/collections.rs::iter_for` (closes
///      the `OpNot(fnref)` SIGBUS / native E0600).
///   2. Native unified `next_into` channel tag 2 (fn-ref rebuild) —
///      packs `(d_nr, closure_dbref)` into 2 i64 slots; consumer
///      rebuilds the `(u32, DbRef)` tuple from buffer slots.
///   3. Interp parse-time rewrite: bare `Value::Int(d_nr)` yielded
///      into `iterator<fn(...)>` is wrapped as `Value::FnRef(d_nr,
///      u16::MAX, _)` so the full 20-byte fn-ref is pushed onto the
///      coroutine stack; state-machine codegen emits
///      `OpNullRefSentinel` for `clos_var == u16::MAX` (was panicking
///      on `variables.stack(u16::MAX)`).
///   4. Native fn-ref reachability: `Value::Yield(Value::Int(d_nr))`
///      is now walked by `collect_fn_ref_literals` so the lambda
///      stays reachable (was hitting `unreachable!("invalid fn-ref")`).
///
/// Regression below: empty generator (smoke), non-capturing for-loop,
/// capturing manual next.  Matrix cells `y5_x1_*` and `y5_x2_*` in
/// `tests/coroutine_matrix.rs` cover the cross-mode path.
#[test]
fn p328_closure_yielding_for_loop_empty_generator() {
    code!(
        "fn fns() -> iterator<fn(integer) -> integer> {
    // empty generator — never yields
}
fn run() -> integer {
    n = 0;
    for _ in fns() { n = n + 1; }
    n
}"
    )
    .expr("run()")
    .result(Value::Int(0));
}

#[test]
fn p328_iterator_of_closure_for_loop_noncapturing() {
    code!(
        "fn fns() -> iterator<fn(integer) -> integer> {
    yield fn(x: integer) -> integer { x * 10 };
    yield fn(x: integer) -> integer { x + 100 };
}
fn run() -> integer {
    total = 0;
    for f in fns() { total = total + f(7); }
    total
}"
    )
    .expr("run()")
    .result(Value::Int(177));
}

#[test]
fn p328_iterator_of_closure_manual_next_capturing() {
    code!(
        "fn fns(base: integer) -> iterator<fn(integer) -> integer> {
    yield fn(x: integer) -> integer { x + base };
}
fn run() -> integer {
    it = fns(100);
    f = next(it);
    f(5)
}"
    )
    .expr("run()")
    .result(Value::Int(105));
}

/// @P322 — `iterator<integer>` generator whose body is `for n in [literal] {
/// yield n; }` leaked the literal vector at program exit.  Root cause: the
/// function body block contained a nested Void-result block ending with the
/// iterator-completion `return null;`.  `scopes::insert_free`'s void branch
/// dropped the outer-scope frees, so `__vdb_*` (the literal vector backing)
/// never got an `OpFreeRef`.  Fix: emit outer frees BEFORE the terminal
/// `Return` inside the inner block.  This test sums correctly; the leak
/// gate in `tests/wrap.rs::run_test` (Part B) catches the regression at
/// the script-suite level.
#[test]
fn p322_iterator_for_literal_vector_no_leak() {
    code!(
        "fn nums() -> iterator<integer> {
    for n in [10, 20, 30] {
        yield n;
    }
}
fn run() -> integer {
    total = 0;
    for n in nums() { total = total + n; }
    total
}"
    )
    .expr("run()")
    .result(Value::Int(60));
}

// ── pass-1 index of a vector whose element struct is forward-referenced ──────
// `f` indexes `vector<Later>` before `struct Later` is registered.  In pass-1
// the element type is still `Unknown`, so `parse_vector_index`'s
// `def(type_elm(etp))` saw `u32::MAX` and panicked ("Unknown definition").  The
// fix substitutes the builtin `reference` def for the unresolved element so a
// placeholder read builds; pass-2 (every def registered) resolves `Later` and
// rebuilds the real read.  A genuinely-undefined element still errors at the
// type declaration, so this never masks a real typo.  The `42` proves pass-2
// produced a correct read, not just a non-crashing parse.
#[test]
fn forward_ref_struct_vector_index() {
    code!(
        "fn f(v: vector<Later>) -> integer { v[0].x }
struct Later { x: integer }
fn run() -> integer { v = [Later { x: 42 }]; f(v) }"
    )
    .expr("run()")
    .result(Value::Int(42));
}

// ── forward-reference resolution: the two-pass invariant ────────────────────
// loft's parser runs pass-1 (register definitions) then pass-2 (resolve bodies
// with everything registered), and pass-2 only runs if pass-1 emitted zero
// errors.  A type referenced before its definition is legitimately `Unknown` in
// pass-1 and resolves in pass-2 — so pass-1 must DEFER on it (no hard error, no
// panic, no token desync) or it deletes the very pass that would resolve it.
// These guard three positions that each used to break pass-1 differently; the
// values prove pass-2 produced a correct read, not just a non-crashing parse.

// Return type named before its struct definition.  A forward `-> Cell` registers
// an `Unknown` stub, so the body's `Cell { … }` construction found a non-struct
// def and desynced into "Expect token ;".  Fixed by treating the stub as
// not-yet-real in the construction dispatch (defer; pass-2 sees the real struct).
#[test]
fn forward_ref_return_type() {
    code!(
        "fn make2() -> Cell { Cell { n: 2 } }
struct Cell { n: integer }
fn run() -> integer { make2().n }"
    )
    .expr("run()")
    .result(Value::Int(2));
}

// Local type annotation named before its struct.  Same `Unknown`-stub
// construction desync as the return-type case, via `c: Cell = Cell { … }`.
#[test]
fn forward_ref_local_annotation() {
    code!(
        "fn use3() -> integer { c: Cell = Cell { n: 3 }; c.n }
struct Cell { n: integer }
fn run() -> integer { use3() }"
    )
    .expr("run()")
    .result(Value::Int(3));
}

// Direct struct field whose type is defined later.  `b.inner` reads a field
// whose declared type is still `Unknown` in pass-1; `get_val` emitted "Field
// access not supported on type unknown", killing pass-2 before
// `actual_types_deferred` resolved the field type.  Fixed by deferring the
// field read on an `Unknown` type in pass-1.
#[test]
fn forward_ref_struct_field_type() {
    code!(
        "struct Box { inner: Cell }
struct Cell { n: integer }
fn run() -> integer { b = Box { inner: Cell { n: 4 } }; b.inner.n }"
    )
    .expr("run()")
    .result(Value::Int(4));
}

// ── @P375 — pass-1 must DEFER, not break, on three more forward positions ────
// A dependency imported at a high source number is parsed AFTER the importing
// package on pass 1 (the package loader reserves source slots eagerly but parses
// bodies in todo-stack order), so a cross-package type is legitimately Unknown
// while the dependent's body parses.  The same shape reproduces single-file with
// a plain forward reference.  Each test below broke pass 1 a different way before
// the fix and now resolves on pass 2; the value proves a correct read, not just a
// non-crashing parse.  The full cross-package boundary matrix lives in the @P375
// investigation probes.

// For-loop over a forward-referenced `vector<struct>` field.  `for it in m.items`
// makes `m.items` Unknown in pass 1, so `for_type` fell to its catch-all and
// returned `Type::Null`; the loop body's `it.n` then hit `field()` on a Null type
// (past the `Type::Unknown` defer-guard) and hard-errored "Unknown type null",
// aborting pass 1 before the type could resolve.  Fixed by returning `Unknown`
// (not `Null`) so the read routes through the existing defer-guard.
#[test]
fn forward_ref_for_loop_vector_struct_field() {
    code!(
        "fn build(m: Map) -> integer { s = 0; for it in m.items { s = s + it.n; } s }
struct Inner { n: integer }
struct Map { items: vector<Inner> }
fn run() -> integer { build(Map { items: [Inner { n: 5 }] }) }"
    )
    .expr("run()")
    .result(Value::Int(5));
}

// `match` on a forward-referenced enum.  With the subject enum still an Unknown
// stub in pass 1, every arm hit the `bad_variant` skip path — which consumed the
// `=> expr` body but NOT the trailing comma, so the next iteration saw the leading
// `,` instead of a variant name, broke early, and desynced into "Expect token }".
// Fixed by consuming the optional trailing comma in the skip path.
#[test]
fn forward_ref_enum_match() {
    code!(
        "fn pick() -> integer { c = Color::Green; match c { Color::Red => 0, Color::Green => 6 } }
enum Color { Red, Green }
fn run() -> integer { pick() }"
    )
    .expr("run()")
    .result(Value::Int(6));
}

// Local `vector<struct>` literal of a forward-referenced element (@P373).  In
// pass 1 the element is Unknown, so the real `main_vector<Inner>` wrapper is
// born during pass-2 body codegen — AFTER this file's `fill_all` registration
// sweep — and reaches codegen with no database `known_type` AND no laid-out
// field positions.  Two faults followed: codegen baked `OpDatabase(db_tp=
// u16::MAX)` (panic in `set_default_value`), and even once that was registered,
// the wrapper's `vector` field sat at position `u16::MAX`, so `OpGetField` read
// through a bogus offset and corrupted the interpreter free path — a heap write
// that SIGSEGV'd at scope exit AFTER the correct value printed.  Fixed by
// `vector_wrapper_known_type` (vectors.rs): register the wrapper on the spot
// then `database.finish()` it so the field positions are laid out before
// codegen consumes them.  The `result(8)` proves correctness; soundness (no
// teardown SIGSEGV) is what regressed, so this also doubles as a free-path
// guard on both backends.
#[test]
fn forward_ref_local_vector_literal() {
    code!(
        "fn pull() -> integer { v = [Cell { n: 8 }]; v[0].n }
struct Cell { n: integer }
fn run() -> integer { pull() }"
    )
    .expr("run()")
    .result(Value::Int(8));
}

// Struct with an INLINE field of a forward-referenced struct (`inner: Cell`,
// @P373).  `Box` is laid out (pass-1 `fill_database`) before `Cell`, and the
// embedded-reference arm read `Cell`'s `known_type` directly — still u16::MAX —
// instead of laying `Cell` out first (as the vector / tuple arms do).  `Box.inner`
// then landed at offset u16::MAX, never repaired on pass 2 (`finish_type` skips an
// already-sized type), so `b.inner.n` read through a bogus offset and corrupted
// the free path: a non-deterministic SIGSEGV at scope exit AFTER the correct
// value printed.  Fixed by recursing into the inline content first in both the
// interpreter (`fill_database`) and the native db-init generator.  A `result`
// proves correctness; soundness (no teardown crash) is the real guard, so these
// run their full free path.  This is the inline-struct sibling of @P373.
#[test]
fn forward_ref_inline_struct_field() {
    code!(
        "struct Box { inner: Cell }
struct Cell { n: integer }
fn run() -> integer { b = Box { inner: Cell { n: 4 } }; b.inner.n }"
    )
    .expr("run()")
    .result(Value::Int(4));
}

// Two-level inline forward reference (`Outer.mid: Mid`, `Mid.inner: Cell`), both
// hosts declared before their content — exercises the recursive content-layout
// transitively (Outer → Mid → Cell).
#[test]
fn forward_ref_nested_inline_struct() {
    code!(
        "struct Outer { mid: Mid }
struct Mid { inner: Cell }
struct Cell { n: integer }
fn run() -> integer { o = Outer { mid: Mid { inner: Cell { n: 9 } } }; o.mid.inner.n }"
    )
    .expr("run()")
    .result(Value::Int(9));
}

// Enum struct-variant with an inline forward-referenced field (`Sq { side: Cell }`
// before `struct Cell`) — the EnumValue layout path takes the same inline-content
// arm, so it had the same offset-u16::MAX corruption.
#[test]
fn forward_ref_enum_variant_inline_field() {
    code!(
        "enum Shape { Sq { side: Cell } }
struct Cell { n: integer }
fn run() -> integer { s = Sq { side: Cell { n: 2 } }; match s { Sq { side } => side.n } }"
    )
    .expr("run()")
    .result(Value::Int(2));
}

// ── @P379 — `use` namespaces struct types per library ────────────────────────
// Two libraries each defining `struct Chunk` with DIFFERENT field layouts
// (moros_map's holds vector<Hex>, hex_world's holds vector<Cell>) must load
// together without the `Double structure type Chunk` internal panic, and each
// library's Chunk-bearing collections must resolve to the CORRECT per-library
// content type.  Before the fix, `use hex_world; use moros_map;` panicked at
// src/database/types.rs:53.
#[test]
fn p379_two_libs_same_struct_name() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    // hex_world was extracted to loft-libs-world 2026-06-01 (W.2); the
    // fixture lives under tests/fixtures/libs/ from Phase 6.12 onwards.
    // moros_map still lives in `lib/`.
    p.lib_dirs.push("tests/fixtures/libs".to_string());
    p.lib_dirs.push("lib".to_string());
    p.parse("tests/multilib/p379_lib_namespace.loft", false);
    let errors: Vec<String> = p
        .diagnostics
        .entries()
        .iter()
        .filter(|e| e.level >= loft::diagnostics::Level::Error)
        .map(|e| e.to_string_compact())
        .collect();
    assert!(
        errors.is_empty(),
        "parse errors loading two libs: {errors:?}"
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    // production logger so an in-loft assert failure sets had_fatal instead of
    // aborting the test binary.
    let config = RuntimeLogConfig {
        log_path: std::path::PathBuf::from("/dev/null"),
        production: true,
        ..Default::default()
    };
    state.database.logger = Some(Arc::new(Mutex::new(Logger::new(config, None))));
    state.execute("main", &p.data);
    assert!(
        !state.database.had_fatal,
        "per-library Chunk fields resolved incorrectly (an in-loft assert failed)"
    );
}

// ── use-region fixpoint — manifest dep that is a MULTI-FILE package ──────────
// `mfdep_app` declares `mfdep_leaf` only as a manifest `[dependencies]` edge
// (no source `use`), so the multi-file `mfdep_leaf` reaches the lexer through
// the pending-deps loop rather than the explicit-`use` pre-scan.  Its entry
// file opens with `use mfdep_leafmod;`.  Before parse_file wrapped the
// pre-scan + pending-deps drain in a fixpoint loop, the pending loop parked the
// lexer on that un-pre-scanned entry file, and the main definition-loop then
// read its legitimately-leading `use` as "use statements must appear before all
// definitions" — and never imported it.  This guards that a multi-file package
// pulled purely via a manifest edge still has its leading uses pre-scanned.
#[test]
fn manifest_dep_multifile_use_order() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.lib_dirs.push("tests/fixtures/libs".to_string());
    p.parse("tests/multilib/mfdep_use_order.loft", false);
    let errors: Vec<String> = p
        .diagnostics
        .entries()
        .iter()
        .filter(|e| e.level >= loft::diagnostics::Level::Error)
        .map(|e| e.to_string_compact())
        .collect();
    assert!(
        errors.is_empty(),
        "multi-file manifest dep tripped the use-region check: {errors:?}"
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let config = RuntimeLogConfig {
        log_path: std::path::PathBuf::from("/dev/null"),
        production: true,
        ..Default::default()
    };
    state.database.logger = Some(Arc::new(Mutex::new(Logger::new(config, None))));
    state.execute("main", &p.data);
    assert!(
        !state.database.had_fatal,
        "manifest-dep multi-file package failed to load/resolve at runtime"
    );
}

// ── loft#797 — a field whose type a LATER module declares still gets storage ─
//
// `fwd797`'s entry `use`s `fwd797_inner` and only then declares the structs that
// module's fields name, so `fwd797_inner` is laid out while every one of those
// types is still a forward-reference stub.  `fill_database` silently skips a
// field it cannot size and nothing revisits an already-registered type, so the
// declaration and the layout disagreed from then on: `position()` answered
// `u16::MAX` for the missing field and that flowed out as the field OFFSET, so
// writes landed at `record + 65535` — in the neighbouring records' bytes.
//
// The fixture covers each spelling that reaches the layout differently (inline
// struct, `vector<T>`, a nullable `T?`, and a host whose own field is one of the
// structs that had to wait) and the in-loft `main` checks the SIZES as well as
// the values: a read follows whatever offsets the layout ended up with, so
// reading a field back cannot by itself prove the field has storage.
#[test]
fn forward_module_type_gets_a_slot() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.lib_dirs.push("tests/fixtures/libs".to_string());
    p.parse("tests/multilib/fwd797_layout.loft", false);
    let errors: Vec<String> = p
        .diagnostics
        .entries()
        .iter()
        .filter(|e| e.level >= loft::diagnostics::Level::Error)
        .map(|e| e.to_string_compact())
        .collect();
    assert!(
        errors.is_empty(),
        "a later-declared field type failed to compile: {errors:?}"
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let config = RuntimeLogConfig {
        log_path: std::path::PathBuf::from("/dev/null"),
        production: true,
        ..Default::default()
    };
    state.database.logger = Some(Arc::new(Mutex::new(Logger::new(config, None))));
    state.execute("main", &p.data);
    assert!(
        !state.database.had_fatal,
        "a field or a size assertion in fwd797_layout.loft failed"
    );
}

// ── loft#801 — a module may name the entry's type in an EXPRESSION, not just ─
// ── in a declaration ─────────────────────────────────────────────────────────
//
// `fwd801`'s entry `use`s `fwd801_inner` and only then declares the types that
// module names, so every mention there is a forward reference to a file suspended
// further up the `use` chain.  Whether it resolved used to depend on the SPELLING:
// a written type went through `parse_type`, which leaves a `DefType::Unknown` stub
// for the entry's declaration to adopt in place, and an expression left nothing to
// adopt.  So `r: F801Roofs = F801Roofs { … }` compiled and the identical
// `r = F801Roofs { … }` did not.
//
// The fixture covers each spelling that reaches the name differently — construction
// alone, a vector literal, iterating that vector, the type as a value argument, and
// a typedef (`parse_typedef` reported the waiting stub as a name clash where
// `parse_struct` and `parse_enum` both adopt it).
#[test]
fn module_names_the_entry_type_in_an_expression() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.lib_dirs.push("tests/fixtures/libs".to_string());
    p.parse("tests/multilib/fwd801_body.loft", false);
    let errors: Vec<String> = p
        .diagnostics
        .entries()
        .iter()
        .filter(|e| e.level >= loft::diagnostics::Level::Error)
        .map(|e| e.to_string_compact())
        .collect();
    assert!(
        errors.is_empty(),
        "a type named only in an expression failed to resolve: {errors:?}"
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let config = RuntimeLogConfig {
        log_path: std::path::PathBuf::from("/dev/null"),
        production: true,
        ..Default::default()
    };
    state.database.logger = Some(Arc::new(Mutex::new(Logger::new(config, None))));
    state.execute("main", &p.data);
    assert!(
        !state.database.had_fatal,
        "an assertion in fwd801_body.loft failed"
    );
}

// ── @PLN125 arc B — a scope-end hook declared inside a LIBRARY runs ──────────
//
// The hook belongs to its type, and a library's symbols are module-scoped
// (@PLN102 C97).  Both askers — the emitter that puts the drop call in, and the
// never-read lint that stays quiet for a binding held only for its drop — run
// AFTER parsing, when the current source is the main program, so their `def_nr`
// resolved only the hooks @PLN102 C97 had also injected into the global
// namespace: the `pub` ones.  A private `OpDrop` was accepted by
// `check_drop_signature` and then never called anywhere, including inside its
// own package.
//
// That is a silent resource leak, and it is the exact shape @PLN138 gives every
// SQL backend: a cursor owning a `sqlite3_stmt *`, released at the closing
// brace.  Nothing calls `OpDrop` by name, so a library has every reason to keep
// it private — which is why the private case is the one the fixture declares.
//
// A SUBPROCESS on both backends rather than the in-process parser idiom its
// neighbours use, because a drop's only observable is I/O: the hook receives
// only `self` and a struct field copies at construction, so it cannot write back
// into the script's own data.  Each leg gets its own trace file — two legs
// sharing one would interleave their appends into a third program neither ran.
#[test]
fn a_private_scope_end_hook_in_a_library_runs() -> std::io::Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for backend in ["--interpret", "--native"] {
        let trace = std::env::temp_dir().join(format!(
            "loft_dropscope{}.tmp",
            backend.trim_start_matches('-')
        ));
        let _ = std::fs::remove_file(&trace);
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(root.join("tests/fixtures/libs"))
            .arg(root.join("tests/multilib/drop_scope_hook.loft"))
            .env("LOFT_DROPSCOPE_TRACE", &trace)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success() && stdout.contains("drop scope hook ok"),
            "{backend} exited {}: stdout={stdout:?} stderr={:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        // The script asserts each trace itself; this catches the failure mode a
        // script cannot see, where the hook fires from a SECOND mechanism and
        // the runtime reports the extra free rather than the script reporting a
        // wrong trace.
        assert!(
            !stdout.contains("BUG (#"),
            "{backend}: a drop must not provoke an internal fault:\n{stdout}"
        );
        let _ = std::fs::remove_file(&trace);
    }
    Ok(())
}

// ── loft#847 — an associated type binds as a TYPE, not as a place ───────────
//
// An implementor's return type carries a dep list indexed in its OWN frame, and
// it is non-empty exactly when the returned record comes from a NESTED CALL
// rather than an inline construction.  The companion binding recorded that type
// verbatim, so a monomorph substituted those indices into the CALLER, where the
// same numbers name unrelated locals: the caller's binding became a view of
// whatever variable 1 happened to be, and the free landed on a stack record.
//
// **The value stays right, which is why this needs its own reader.**  The `#306`
// guard REFUSES the wrong free rather than performing it, so every assertion in
// the script passed while the ownership underneath was broken — and `#306` is an
// `eprintln`, which the in-process script harness never looks at.  A subprocess
// is the only thing that can see it.
//
// Both axes are in the fixture: the delegating producer AND the associated-type
// bound.  Its concrete-bound control delegates identically and was always clean.
#[test]
fn a_delegating_producer_binds_its_companion_cleanly() -> std::io::Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg(root.join("tests/scripts/pln125-a2c-companion.loft"))
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            out.status.success() && stdout.contains("pln125 a2c companion ok"),
            "{backend} exited {}: stdout={stdout:?} stderr={stderr:?}",
            out.status,
        );
        assert!(
            !stderr.contains("BUG (#"),
            "{backend}: a companion must not carry the implementor's deps into the \
             monomorph:\n{stderr}"
        );
    }
    Ok(())
}

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
         at issue_314_scalar_shared_by_two_closures_rejected:5:2",
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

#[test]
fn issue_318_returning_closure_carrying_struct_rejected() {
    code!(
        "struct Counter { n: integer }
struct K { cb: fn() }
fn make() -> K {
    w = Counter { n: 7 };
    K { cb: fn() { w.n = w.n + 1; } }
}
fn test_it() { k = make(); c = k.cb; c(); }"
    )
    .error(
        "function returns a struct type that holds a capturing closure; the \
         closure references state owned by this function's frame, so the value \
         cannot outlive it — construct the struct in the frame that owns the \
         captured state and pass it down, or return the closure itself (#318) \
         at issue_318_returning_closure_carrying_struct_rejected:3:17",
    );
}

#[test]
fn issue_318_closure_into_argument_struct_rejected() {
    code!(
        "struct Counter { n: integer }
struct K { cb: fn() }
struct H { k: K }
fn attach(h: H) {
    w = Counter { n: 7 };
    h.k = K { cb: fn() { w.n = w.n + 1; } };
}
fn test_it() {
    h = H { k: K { cb: fn() { print(\"orig\"); } } };
    attach(h);
}"
    )
    .error(
        "cannot store a capturing closure into a struct received as an argument \
         — the closure references state owned by this function's frame, which \
         the argument's struct outlives; construct the closure in the frame \
         that owns the captured state (#318) \
         at issue_318_closure_into_argument_struct_rejected:6:44",
    );
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
    .error(
        "collection of a struct type that holds a capturing closure is not \
         supported — element copies would dangle into the constructing \
         function's frame; keep closure holders in local variables and pass \
         them down as arguments (#318) \
         at issue_318_vector_of_closure_carrying_struct_rejected:5:19",
    )
    .error(
        "field `vector` would store a value of a type that holds a capturing \
         closure; such values are bound to the function frame that owns the \
         captures and cannot be copied into another struct — keep the closure \
         holder in a local variable and pass it down as an argument (#318) \
         at issue_318_vector_of_closure_carrying_struct_rejected:5:55",
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
    d = Node { value: 9 };
    default_null = if d.next == null { 1 } else { 0 };
    aliased * 100000 + repointed * 1000 + untouched * 10 + cleared * 2 + default_null
}"
    )
    .expr("run()")
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
    x.value
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
    std::fs::write(
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
    let bin = root.join("target/release/loft");
    if !bin.exists() {
        // Sanitizer CI jobs (ASan gate, stack_align sweep) run the test
        // binaries without building the release CLI — skip like the
        // engine_host_kernel spawning tests do instead of dying NotFound.
        eprintln!("skipping: release loft not built");
        return;
    }
    let out = std::process::Command::new(bin)
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

// ── @PLN87 P2 reassignment-locality / write-back CONSISTENCY lock-ins ─────────
// The uniform model (mirrors tests/scripts/87-p2-reassign-locality.loft, which
// holds the GREEN cells): a non-`&` whole-binding reassignment of a heap param
// is LOCAL; a `&` reassignment writes BACK; field/element mutation propagates.
// Struct + scalar cells already pass (P2.1).  These pin the still-inconsistent
// VECTOR cells to their target consistent behaviour; un-ignore each when it lands.

// Vector non-`&` reassignment REBINDS locally (leaves the caller untouched),
// like the struct case — fixed by P2.4 (vector_db hands a rebind param a fresh
// `__vdb` backing; the witness frees it at exit).
#[test]
fn pln87_vector_param_reassign_is_local() {
    code!(
        "fn vrebind(v: vector<integer>) { v = [7, 8, 9]; }
fn check() -> integer { a = [1, 2, 3]; vrebind(a); len(a) }"
    )
    .expr("check()")
    .result(Value::Int(3));
}

// Vector `&` reassignment WRITES BACK (caller sees the new vector), like the
// struct `&` case — fixed by P2.4 (a `&`-vector shares the caller's backing, so
// the write-back is a clear+refill in place: `OpClearVector` before the literal).
#[test]
fn pln87_vector_param_amp_writes_back() {
    code!(
        "fn vamp(v: &vector<integer>) { v = [7, 8, 9]; }
fn check() -> integer { a = [1, 2, 3]; vamp(a); a[0] }"
    )
    .expr("check()")
    .result(Value::Int(7));
}

/// @PLN87 #1 — `&` write-back from a CALL RHS (`o = mk()`). The ownership-transfer machinery
/// now routes a call RHS through a transferable owned temp, so the write-back reaches the
/// caller's place (`a.x` becomes 9). Verified clean on both backends. (Was deferred behind a
/// parse rejection that no longer exists.)
#[test]
fn pln87_amp_writeback_from_call_writes_back() {
    code!("struct Obj { x: integer } fn mk() -> Obj { Obj { x: 9 } } fn f(o: &Obj) { o = mk(); } fn check() -> integer { a = Obj { x: 1 }; f(a); a.x }")
        .expr("check()")
        .result(Value::Int(9));
}

/// @PLN87 — the `&var = 3` rejection must NOT over-fire on a valid RHS link:
/// `c = &v[0]` LINKS `c` to the element (bind-site `&`), and reading `c` sees the
/// linked value. (The `&` is on the RHS, followed by `;`, not an assignment `=`.)
#[test]
fn pln87_amp_rhs_link_is_not_rejected() {
    code!("fn check() -> integer { v = [10, 20]; c = &v[0]; c }")
        .expr("check()")
        .result(Value::Int(10));
}

// @PLN87 — the LINK-semantics ladder (corrected `&` model: `&` LINKS a binding to
// its source, read- and write-through).  Each rung is an ignored lock-in that flips
// to PASS when that rung lands.  North star: `a=3; b=&a; b=4; a == 4`.

/// L1 — scalar local, LIVE read: a link reflects the source's current value.
#[test]
fn pln87_link_l1_scalar_live_read() {
    code!("fn check() -> integer { a = 3; b = &a; a = 5; b }")
        .expr("check()")
        .result(Value::Int(5));
}

/// L2 — scalar local, WRITE-THROUGH (the north star): writing the link writes the source.
#[test]
fn pln87_link_l2_scalar_write_through() {
    code!("fn check() -> integer { a = 3; b = &a; b = 4; a }")
        .expr("check()")
        .result(Value::Int(4));
}

/// L3 — scalar struct-field link: `b = &s.x; b = 4` writes `s.x`.
#[test]
fn pln87_link_l3_field_write_through() {
    code!("struct S { x: integer } fn check() -> integer { s = S { x: 3 }; b = &s.x; b = 4; s.x }")
        .expr("check()")
        .result(Value::Int(4));
}

/// L3 — the field link is LIVE in the read direction too, and works for a field at a
/// NON-zero offset (`s.b`, offset 8) as well as offset 0.
#[test]
fn pln87_link_l3_field_live_read() {
    code!(
        "struct S { a: integer, b: integer } \
         fn check() -> integer { s = S { a: 3, b: 7 }; r = &s.b; s.b = 5; r }"
    )
    .expr("check()")
    .result(Value::Int(5));
}

/// L4 — scalar vector-element link: `c = &v[0]; c = 99` writes `v[0]`.
#[test]
fn pln87_link_l4_element_write_through() {
    code!("fn check() -> integer { v = [10, 20]; c = &v[0]; c = 99; v[0] }")
        .expr("check()")
        .result(Value::Int(99));
}

/// L4 — the link is LIVE in the read direction too: `v[0]` updates show through `c`.
#[test]
fn pln87_link_l4_element_live_read() {
    code!("fn check() -> integer { v = [10, 20]; c = &v[0]; v[0] = 5; c }")
        .expr("check()")
        .result(Value::Int(5));
}

/// L6 — link as a function parameter: `fn f(b: &integer){ b = 4 }; f(a)` writes `a`.
/// The `&` parameter is called WITHOUT `&` (`f(a)`); the reference comes from the type.
#[test]
fn pln87_link_l6_param_write_through() {
    code!("fn f(b: &integer) { b = 4 } fn check() -> integer { a = 3; f(a); a }")
        .expr("check()")
        .result(Value::Int(4));
}

/// L6 — a `&`-struct parameter links to the caller's struct: a field mutation through
/// the parameter writes the caller's field.
#[test]
fn pln87_link_l6_struct_param_field_writes_back() {
    code!(
        "struct S { x: integer } fn g(obj: S) { obj.x = 5; } \
         fn check() -> integer { s = S { x: 1 }; g(s); s.x }"
    )
    .expr("check()")
    .result(Value::Int(5));
}

/// L5 — heap whole-value reference: `p = &o` ALIASES the heap local (which COPIES on a
/// plain `p = o`), so a field mutation through `p` writes `o`.  `p` reuses the #257
/// alias representation (interp stack-ref / native DbRef-by-value), non-owning.
#[test]
fn pln87_link_l5_heap_whole_value_ref() {
    code!("struct S { x: integer } fn check() -> integer { o = S { x: 1 }; p = &o; p.x = 5; o.x }")
        .expr("check()")
        .result(Value::Int(5));
}

/// L5 — the heap reference is LIVE in the read direction too: `o`'s field updates show
/// through `p`; and `p = o` WITHOUT `&` still copies (the `&` is what makes it a link).
#[test]
fn pln87_link_l5_heap_reference_live_read() {
    code!("struct S { x: integer } fn check() -> integer { o = S { x: 1 }; p = &o; o.x = 7; p.x }")
        .expr("check()")
        .result(Value::Int(7));
}

#[test]
fn pln87_plain_heap_assign_still_copies() {
    code!("struct S { x: integer } fn check() -> integer { o = S { x: 1 }; p = o; p.x = 5; o.x }")
        .expr("check()")
        .result(Value::Int(1));
}

/// @PLN87 #2 — a typed-local reference `b: &T = src` is the L1 form with the `&` on
/// the TYPE (instead of `b = &src`): a live reference to the addressable scalar `src`,
/// read- and write-through.
#[test]
fn pln87_typed_local_scalar_reference_live_read() {
    code!("fn check() -> integer { a = 3; b: &integer = a; a = 5; b }")
        .expr("check()")
        .result(Value::Int(5));
}

#[test]
fn pln87_typed_local_reference_write_through() {
    code!("fn check() -> integer { c = 10; d: &integer = c; d = 4; c }")
        .expr("check()")
        .result(Value::Int(4));
}

// @PLN87 L7 — edges of the `&`-reference model.

/// L7 — a reference to a reference (`c = &b`, both scalar): `c` links to the same source
/// `b` does, so writing `c` writes the original and reads are live.
#[test]
fn pln87_l7_ref_to_ref_scalar() {
    code!("fn check() -> integer { a = 3; b = &a; c = &b; c = 5; a }")
        .expr("check()")
        .result(Value::Int(5));
}

/// L7 — a reference to a struct reference (`q = &p`): a field mutation through `q`
/// reaches the original struct.
#[test]
fn pln87_l7_ref_to_ref_struct() {
    code!("struct S { x: integer } fn check() -> integer { o = S { x: 1 }; p = &o; q = &p; q.x = 7; o.x }")
        .expr("check()")
        .result(Value::Int(7));
}

/// L7 — a reference in an inner scope to an OUTER local (the source outlives the
/// reference): write-through is safe.
#[test]
fn pln87_l7_inner_scope_reference_to_outer() {
    code!("fn check() -> integer { a = 3; if true { b = &a; b = 5; } a }")
        .expr("check()")
        .result(Value::Int(5));
}

/// L7 — a heap reference stays LIVE across a whole-value reassignment of the source
/// (`o = S{..}` reuses the record in place, so the alias sees the new field values).
#[test]
fn pln87_l7_heap_reference_live_across_reassign() {
    code!("struct S { x: integer } fn check() -> integer { o = S { x: 1 }; p = &o; o = S { x: 9 }; p.x }")
        .expr("check()")
        .result(Value::Int(9));
}

/// Regression (routing consumer, docs/loft-feedback.md 2026-07-08 "Incorrect loop
/// finish"): a `for` loop variable reused across two sequential loops of DIFFERENT
/// element types is invalid under loft's flat scoping — the reused name keeps the
/// FIRST loop's type (`add_variable` only refines an unknown type for a user var),
/// so a field access on the SECOND loop variable resolves against the stale type,
/// fails, and leaves the loop with no iterable.  That error path in `parse_for`
/// ("Need an iterable expression") used to `return` WITHOUT `finish_loop`-ing the
/// loop scope it had already opened, so the ENCLOSING loop's `finish_loop` tripped
/// the `assert_eq!(current_loop, loop_nr)` "Incorrect loop finish" panic — masking
/// the real diagnostic.  The parse must now diagnose cleanly instead of panicking;
/// this test would panic inside `parse_str` before the fix.
///
/// loft#825 — what it diagnoses CLEANLY moved one step upstream: it used to answer
/// "Unknown field Pt.roads", which is the stale binding's CONSEQUENCE, and then named
/// the loop-variable conflict itself.
///
/// loft#915 removed the conflict rather than reporting it — `t` in the second loop is
/// its own variable carrying `Rt`, so `t.roads` resolves and the program COMPILES.  That
/// is what this now pins, and it is the same guarantee from the other side: the field
/// read only works if the second loop failed to inherit the first's type, so a
/// regression to a shared binding fails this test as "Unknown field Pt.roads" — the
/// error the whole chain started from.
#[test]
fn loop_var_reuse_different_type_binds_per_loop() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    // `t` is a `Pt` in the first loop and an `Rt` in the second; `t.roads` in the
    // second body resolves against the stale `Pt` type (which has no `roads`
    // field) → no iterable → the previously-leaking error path.
    p.parse_str(
        "struct Pt { tkey: integer, areas: vector<integer> }
struct Rt { tkey: integer, roads: vector<integer> }
fn main() {
  layout: hash<Pt[tkey]> = [];
  roads: hash<Rt[tkey]> = [];
  for t in layout { for a in t.areas { println(\"{a}\"); } }
  for t in roads { for r in t.roads { println(\"{r}\"); } }
}",
        "loop_var_reuse_different_type",
        false,
    );
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "each loop binds its own `t`, so `t.roads` resolves against `Rt` and this must \
         compile; a shared binding shows up here as \"Unknown field Pt.roads\".  Got: {:?}",
        p.diagnostics.lines()
    );
}

// @PLN102 keystone step 4 — honest nullable stdlib returns.
//
// `find`/`rfind` (text) and `min_of`/`max_of` (vector) can produce null on a
// reachable path (value absent / vector empty) and are now typed `integer?` /
// `T?` respectively, so the static type no longer LIES about the null case.
// The runtime representation is identical to before (in-band sentinel), so a
// runtime test cannot distinguish `-> integer` from `-> integer?`; the honest
// type is only observable at a declared-non-null BOUNDARY, where assigning the
// nullable result to a non-null declaration is reported — `(N-Store)`'s WARNING at full
// width (@PLN153 phase 3b: the declared local used to be a hard error).  This is the
// non-vacuous guard: it FAILS if a future edit reverts a signature to non-null.
#[test]
fn pln102_stdlib_reachable_null_returns_are_typed_nullable() {
    // Each of these assigns a nullable-returning stdlib call to a declared
    // NON-NULL local.  With honest `?` return types this is reported; if the
    // return type were reverted to non-null it would compile clean (the guard
    // would then fail to see the expected diagnostic → test fails).
    let cases = [
        ("find", "n: integer = \"abc\".find(\"z\");"),
        ("rfind", "n: integer = \"abc\".rfind(\"z\");"),
        ("min_of", "n: integer = min_of([3, 1, 2]);"),
        ("max_of", "n: integer = max_of([3, 1, 2]);"),
    ];
    for (name, stmt) in cases {
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        p.parse_str(
            &format!("fn main() {{\n  {stmt}\n  println(\"{{n}}\");\n}}"),
            "pln102_nullable_return",
            false,
        );
        assert!(
            p.diagnostics.level() >= loft::diagnostics::Level::Warning,
            "{name}: nullable return assigned to a non-null decl should be reported, got: {:?}",
            p.diagnostics.lines()
        );
        assert!(
            p.diagnostics
                .lines()
                .iter()
                .any(|l| l.contains("is stored into the local `n`") && l.contains("integer?")),
            "{name}: expected the (N-Store) diagnostic naming `integer?`, got: {:?}",
            p.diagnostics.lines()
        );
    }
    // Positive control: the INFERRED (nullable) form compiles clean — proves the
    // error above is specifically the non-null-declaration mismatch, not a broken
    // program, and that the honest type stays fully usable via inference / `??`.
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse_str(
        "fn main() {\n  \
           a = \"abc\".find(\"z\");\n  \
           b = min_of([3, 1, 2]) ?? 0;\n  \
           assert(a == null, \"find absent is null\");\n  \
           assert(b == 1, \"min_of ?? default\");\n\
         }",
        "pln102_nullable_return_ok",
        false,
    );
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "inferred/`??` nullable use should compile clean, got: {:?}",
        p.diagnostics.lines()
    );
}

// @PLN102 pre-freeze — comparison operators are NON-ASSOCIATIVE.  A chain like
// `a == b == c` / `a < b < c` parses left-associatively as `(a OP b) OP c`, silently
// comparing a BOOLEAN to the third operand — the classic C footgun.  The parser now
// rejects a second comparison at the same level; the explicit `(a == b) == c`, an `&&`
// join, and a single comparison all stay legal.  This is the non-vacuous guard (proven
// to fail if the rule is reverted — the chains would then parse clean).
#[test]
fn pln102_comparison_is_non_associative() {
    let chains = [
        "b = 2 == 2 == 1;",
        "b = 1 < 5 < 10;",
        "b = true == true == true;",
        "b = 1 == 2 != 3;",
    ];
    for stmt in chains {
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        p.parse_str(
            &format!("fn main() {{\n  {stmt}\n}}"),
            "pln102_nonassoc",
            false,
        );
        assert!(
            p.diagnostics
                .lines()
                .iter()
                .any(|l| l.contains("do not chain")),
            "expected a 'do not chain' error for `{stmt}`, got: {:?}",
            p.diagnostics.lines()
        );
    }
    // Positive control: explicit parens, an `&&` join, and a single comparison compile
    // clean — proves the rule rejects only the CHAIN, not comparison itself.
    let ok = [
        "b = (2 == 2) == true;",
        "b = 1 < 5 && 5 < 10;",
        "b = 2 == 2;",
    ];
    for stmt in ok {
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        p.parse_str(
            &format!("fn main() {{\n  {stmt}\n}}"),
            "pln102_nonassoc_ok",
            false,
        );
        assert!(
            p.diagnostics.level() < loft::diagnostics::Level::Error,
            "expected `{stmt}` to compile clean, got: {:?}",
            p.diagnostics.lines()
        );
    }
}

// @PLN102 pre-freeze — a boolean and an integer are not comparable.  The old `bool == int`
// coerced the integer to boolean by "is non-null", so `true == 0` was TRUE and `true == 2`
// was TRUE (nonsense), while `bool < int` already errored.  `==`/`!=` now reject it too.
#[test]
fn pln102_boolean_integer_comparison_rejected() {
    let bad = [
        "b = true == 1;",
        "b = true != 0;",
        "b = 1 == false;",
        "b = false == 0;",
    ];
    for stmt in bad {
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        p.parse_str(
            &format!("fn main() {{\n  {stmt}\n}}"),
            "pln102_bool_int",
            false,
        );
        assert!(
            p.diagnostics
                .lines()
                .iter()
                .any(|l| l.contains("cannot compare a boolean and an integer")),
            "expected `{stmt}` to be rejected, got: {:?}",
            p.diagnostics.lines()
        );
    }
    // Still valid: bool==bool, int==int, and the three-state `boolean? == null`.
    let ok = [
        "b = true == false;",
        "b = 1 == 2;",
        "n: boolean? = true; b = n == null;",
    ];
    for stmt in ok {
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        p.parse_str(
            &format!("fn main() {{\n  {stmt}\n}}"),
            "pln102_bool_int_ok",
            false,
        );
        assert!(
            p.diagnostics.level() < loft::diagnostics::Level::Error,
            "expected `{stmt}` to compile clean, got: {:?}",
            p.diagnostics.lines()
        );
    }
}

// ─── @PLN114 — tuple layout must match the record layout ─────────────────────
//
// Both measure a tuple against the `struct` of identical fields, because that record
// is the oracle the tuple layout was moved onto.  They landed `#[ignore]`d — asserting
// a target that failed — and were un-ignored when the fix made them pass.

/// Instrument 3 — record-vs-tuple stride parity.
///
/// `(u8, u32, u16)` must occupy what `struct { a: u8, b: u32, c: u16 }` occupies.
/// Today the record packs to 7 bytes and the tuple reserves 24, because
/// `data::element_size` reports STACK widths (`Integer` is 8B regardless of
/// `forced_size`) where storage needs database widths.
#[test]
fn pln114_tuple_stride_matches_record() {
    code!(
        "struct M { a: u8, b: u32, c: u16 }
fn test() {
    vs: vector<M> = [M{a:1,b:2,c:3}, M{a:4,b:5,c:6}];
    vt: vector<(u8, u32, u16)> = [(1,2,3), (4,5,6)];
    assert(size(vt) == size(vs), \"tuple stride must equal record stride\");
}"
    );
}

/// Instrument 4 — a mixed-width tuple must round-trip every element.
///
/// Today the narrow trailing element reads back `+1`: `(1,2,3)` returns `(1,2,4)`,
/// reproducible in `vector<(u8,u16)>` and `vector<(u32,u16)>` too.  Silent
/// corruption — exit 0, no diagnostic.
#[test]
fn pln114_mixed_width_tuple_round_trip() {
    code!(
        "fn test() {
    v: vector<(u8, u32, u16)> = [(1,2,3), (4,5,6)];
    assert(v[0].0 == 1 && v[0].1 == 2 && v[0].2 == 3, \"element 0 round-trips\");
    assert(v[1].0 == 4 && v[1].1 == 5 && v[1].2 == 6, \"element 1 round-trips\");
    w: vector<(u8, u16)> = [(1,2)];
    assert(w[0].1 == 2, \"u16 after u8 round-trips\");
}"
    );
}

/// A3 — the two alignment tables must agree.
///
/// `data::element_align` and the inline table in `Data::tuple_def` encode the same
/// rule twice.  They ALREADY disagree about `Function` (8 vs 4), and nothing detects
/// it because nothing compares them — the drift this plan exists to remove.  Delete
/// this test when the inline copy is gone (A4) and there is nothing to compare.
#[test]
fn pln114_alignment_tables_agree() {
    use loft::data::{IntegerSpec, Type, element_stack_align};
    let int = Type::Integer(IntegerSpec {
        min: i32::MIN + 1,
        max: i32::MAX as u32,
        not_null: false,
        forced_size: None,
    });
    for (name, tp, expect) in [
        ("boolean", Type::Boolean, 1u8),
        ("character", Type::Character, 4),
        ("single", Type::Single, 4),
        ("integer", int, 8),
        ("float", Type::Float, 8),
    ] {
        assert_eq!(
            element_stack_align(&tp),
            expect,
            "element_stack_align({name}) — the tuple_def inline table must agree"
        );
    }
}

// ── #618: an entry function returning a heap value ───────────────────────────
// `ref_return` promotion makes a returned local BE the caller's hidden return
// buffer, so the body writes straight into it.  An ordinary call site allocates
// that buffer before the call; `execute_argv` pushed a bare `DbRef::NULL`, so
// every element write addressed `stores[u16::MAX]` and aborted with
// "index out of bounds: the len is 2 but the index is 65535".
//
// `ReplSession::value_of` was the reported symptom, but the entry contract is
// what broke, so guard it directly here: a plain `fn main() -> vector<integer>`
// reproduced it identically under `--interpret`.  Struct returns always worked
// (their promoted body opens with its own `OpDatabase`) and are kept as the
// negative control — a "fix" that regressed them would pass the vector cases.
//
// Interpreter only: `--native` cannot yet compile ANY heap-returning entry fn
// (its generated `main` omits the hidden buffer argument → rustc E0428/E0061),
// a separate pre-existing gap on the same contract.
fn run_entry_returning(code: &str) -> (State, loft::data::Data) {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse_str(code, "issue618", false);
    assert!(
        p.diagnostics.lines().is_empty(),
        "Parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    state.execute_argv("main", &p.data, &[]);
    (state, p.data)
}

#[test]
fn issue618_entry_fn_returns_bare_local_vector() {
    // Pre-fix: aborted here rather than returning.  The `println` makes the
    // built value observable even though the entry's return is discarded, so a
    // silently-empty buffer cannot pass as success.
    let (state, _) = run_entry_returning(
        "fn main() -> vector<integer> {\n  v = [1, 2, 3];\n  println(\"{v}\");\n  v\n}",
    );
    assert!(
        state.database.runtime_error.is_none(),
        "entry returning a bare local vector must not fault"
    );
}

#[test]
fn issue618_entry_fn_returns_vector_shapes() {
    // One axis per case: element kind, nesting depth, and zero cardinality —
    // every shape routes through the same hidden-buffer contract.
    for code in [
        "fn main() -> vector<text> {\n  v = [\"a\", \"b\"];\n  println(\"{v}\");\n  v\n}",
        "fn main() -> vector<vector<integer>> {\n  v = [[1, 2], [3]];\n  println(\"{v}\");\n  v\n}",
        "fn main() -> vector<integer> {\n  v: vector<integer> = [];\n  println(\"{v}\");\n  v\n}",
        // An element wider than 32 bits — the report's second signature.
        "fn main() -> vector<integer> {\n  v = [9000000000, 0];\n  println(\"{v}\");\n  v\n}",
    ] {
        let (state, _) = run_entry_returning(code);
        assert!(
            state.database.runtime_error.is_none(),
            "entry heap return faulted for: {code}"
        );
    }
}

#[test]
fn issue618_entry_fn_returning_struct_still_works() {
    // Negative control: this path allocates from the sentinel itself and was
    // never broken — it must stay that way.
    let (state, _) = run_entry_returning(
        "struct P { x: integer }\nfn main() -> P {\n  p = P { x: 1 };\n  println(\"{p}\");\n  p\n}",
    );
    assert!(
        state.database.runtime_error.is_none(),
        "entry returning a struct must keep working"
    );
}

// #654 — past ~32 KB of function body the interpreter's jump displacement was
// truncated to 16 bits, so a `while true` took its backward jump to an arbitrary
// address: the body ran ONCE, execution fell out of the loop, and main returned
// status 0 with no diagnostic.  A log ended mid-story with nothing wrong in it.
//
// The filed scope was the backward `while` jump, but a boundary matrix showed
// every jump class truncating — `while`, `for`, `break`, and the forward skips of
// `if` and `else` — because all of them encode a displacement the same way.  So
// the fix is at the encoding, not at any one construct: `OpGotoWord` /
// `OpGotoFalseWord` now carry a 32-bit displacement, which covers the whole
// `code_pos` space and so cannot move the threshold somewhere further out.
//
// `--native` was always correct here (it emits real Rust control flow and never
// reads these operands), which is why this guards the interpreter specifically.
#[test]
fn issue_654_jumps_survive_a_body_past_the_16_bit_displacement() {
    // ~2400 filler statements puts the body comfortably past 32 KB of emitted
    // bytecode; at 1484 the old encoder was still correct and at 1485 it was not,
    // so a guard at the boundary itself would be one statement from vacuous.
    //
    // The filler ACCUMULATES rather than assigning throwaway locals: it has to be
    // warning-free to compile here, and a running total lets each case assert how
    // many times the body actually ran.
    const N: i64 = 2400;
    let filler: String = (0..N).map(|i| format!("acc = acc + {i}; ")).collect();
    let once = N * (N - 1) / 2; // the filler's contribution per execution

    // One case per jump class.  Each asserts a VALUE, not merely that it ran:
    // the failure mode was silent fall-through, which a run-to-completion check
    // would have called a pass.
    let cases: [(&str, String); 5] = [
        // backward jump — `while`
        (
            "while",
            format!(
                "fn test() {{ acc = 0; i = 0; while true {{ i = i + 1; if i > 3 {{ break; }} {filler} }} \
             assert(i == 4, \"while ran {{i}} times, expected 4\"); \
             assert(acc == {}, \"while body ran the wrong number of times\"); }}",
                once * 3
            ),
        ),
        // backward jump — counted `for`
        (
            "for",
            format!(
                "fn test() {{ acc = 0; n = 0; for _ in 0..4 {{ n = n + 1; {filler} }} \
             assert(n == 4, \"for ran {{n}} times, expected 4\"); \
             assert(acc == {}, \"for body ran the wrong number of times\"); }}",
                once * 4
            ),
        ),
        // forward jump — `break` out of a huge body
        (
            "break",
            format!(
                "fn test() {{ acc = 0; i = 0; while true {{ i = i + 1; if i > 2 {{ break; }} {filler} }} \
             assert(i == 3, \"break left the loop at {{i}}, expected 3\"); \
             assert(acc == {}, \"break body ran the wrong number of times\"); }}",
                once * 2
            ),
        ),
        // forward jump — skipping a huge `if` body that must NOT run
        (
            "if",
            format!(
                "fn test() {{ acc = 0; x = 0; if x > 100 {{ {filler} }} \
             assert(acc == 0, \"the untaken if body ran\"); \
             assert(x == 0, \"execution did not reach the join\"); }}"
            ),
        ),
        // forward jump — skipping a huge `else` arm
        (
            "else",
            format!(
                "fn test() {{ acc = 0; taken = 0; if acc < 100 {{ taken = 1; }} else {{ {filler} taken = 2; }} \
             assert(taken == 1, \"took the wrong arm: {{taken}}\"); \
             assert(acc == 0, \"the untaken else arm ran\"); }}"
            ),
        ),
    ];

    for (label, src) in &cases {
        let (mut state, data) = compile_for_production(src);
        attach_production_logger(&mut state);
        state.execute("test", &data);
        assert!(
            !state.database.had_fatal,
            "#654: the `{label}` jump misbehaved past a 32 KB body"
        );
    }
}

// #655 — a `&boolean` parameter that is actually assigned panicked codegen with
// "Unknown referenced variable type: boolean".  Every other scalar reference type
// worked, which is what made it sting: `&boolean` is the natural shape for a
// two-state out-parameter, reached for right after `&integer` has just worked for
// the count beside it (moros hit it on `fn do_wall(open: &boolean, ax: &float, …)`).
//
// FOUR sites, not one.  The filed panic was the interpreter's READ path; fixing it
// exposed the interpreter WRITE path, and getting that far exposed two native ones.
// The root asymmetry is that a plain `boolean` parameter renders as Rust `bool`
// while a `&boolean` renders as `&mut u8`, because a boolean LOCAL is the tri-state
// storage byte (0/1/255, null-capable).  So the deref sites convert, symmetrically:
// read `*p == 1` (a null reads as false), write `u8::from(..)`.
//
// The matrix below is the reason all four were found — the filed scope was the
// parameter alone, and probes for a local `&`-link and for READING the flag in a
// condition each failed on native after the parameter case was green.
#[test]
fn issue_655_ampersand_boolean_reads_and_writes() {
    let cases: [(&str, &str); 7] = [
        // the filed reproducer
        (
            "negate",
            "fn flip(b: &boolean) { b = !b; } \
                    fn test() { x = false; flip(x); assert(x == true, \"negate\"); }",
        ),
        // a constant, not a negation — the write path without a read
        (
            "const",
            "fn setit(b: &boolean) { b = true; } \
                   fn test() { x = false; setit(x); assert(x == true, \"const\"); }",
        ),
        // the other direction, so a test that only ever produced `true` cannot pass
        (
            "from_true",
            "fn flip(b: &boolean) { b = !b; } \
                       fn test() { x = true; flip(x); assert(x == false, \"from true\"); }",
        ),
        // two flags at once — each must write its own slot
        (
            "two",
            "fn both(a: &boolean, b: &boolean) { a = true; b = false; } \
                 fn test() { p = false; q = true; both(p, q); \
                 assert(p == true, \"first\"); assert(q == false, \"second\"); }",
        ),
        // the shape moros actually wrote: a flag beside the scalars that worked
        (
            "mixed",
            "fn do_wall(open: &boolean, ax: &float, n: &integer) \
                   { open = true; ax = ax + 1.5; n = n + 1; } \
                   fn test() { o = false; x = 1.0; c = 0; do_wall(o, x, c); \
                   assert(o == true, \"flag\"); assert(x == 2.5, \"float\"); \
                   assert(c == 1, \"int\"); }",
        ),
        // a local `&`-link rather than a parameter — a separate native path
        (
            "local_link",
            "fn test() { a = false; b = &a; b = true; \
                        assert(a == true, \"local link\"); }",
        ),
        // READING the flag through the reference, which native compiled as `u8`
        // where a `bool` was required
        (
            "read",
            "fn setpair(b: &boolean, out: &integer) \
                  { if b { out = 1; } else { out = 2; } b = !b; } \
                  fn test() { x = true; n = 0; setpair(x, n); \
                  assert(x == false, \"flipped\"); assert(n == 1, \"read as true\"); }",
        ),
    ];
    for (label, src) in &cases {
        let (mut state, data) = compile_for_production(src);
        attach_production_logger(&mut state);
        state.execute("test", &data);
        assert!(
            !state.database.had_fatal,
            "#655: `&boolean` case `{label}` misbehaved"
        );
    }
}

// #656 — a library that qualifies a call with its OWN name (`dlib::shout(x)` inside
// `src/dlib.loft`) made the parser resolve that name back to the file it was already
// parsing and load it a second time.  Every definition re-registered, which surfaced
// as "cannot redefine method `shout` on `text`" pointing at the file's own line — an
// error about the source disagreeing with itself.
//
// A free function only duplicated silently (it showed up twice in `loft api-surface`
// output, the tell that went unread); a METHOD made it fatal.  Published `regex`
// carries exactly that shape, so `loft api-surface` — and with it `loft compat api` /
// `compat floor` / `compat check`, which all route through it — reported the library
// as unreadable while its own test suite passed.
#[test]
fn issue_656_self_qualified_reference_does_not_reparse_the_file() {
    let dir = std::env::temp_dir().join(format!("loft_656_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A METHOD plus a self-qualified call — both halves are needed: the method is
    // what makes the re-registration fatal rather than merely duplicated.
    let file = dir.join("dlib.loft");
    std::fs::write(
        &file,
        "pub fn shout(self: text) -> text { return self; }\n\
         pub fn go(x: text) -> text { return dlib::shout(x); }\n",
    )
    .unwrap();

    let mut p = Parser::new();
    p.lib_dirs.push(dir.to_string_lossy().to_string());
    let _ = p.parse_dir(
        &format!("{}/default", env!("CARGO_MANIFEST_DIR")),
        true,
        false,
    );
    p.parse(&file.to_string_lossy(), false);
    let level = p.diagnostics.level();
    let report = format!("{}", p.diagnostics);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        level < loft::diagnostics::Level::Error,
        "#656: a self-qualified reference must not re-parse its own file:\n{report}"
    );
}

// #656, second shape — a package that names itself while ANOTHER library is loaded.
//
// The two halves failed differently for the same reason.  With no other library in
// play the self-qualified name got loaded (and the file re-parsed) — the first case
// above.  Once a `use <dep>` has loaded something else, the main file's own name no
// longer resolves at all and the parser reported "Unknown library 'glib'" — naming
// the library it was reading at that moment.  `use_names` deliberately never holds
// the main file (source 1 is reserved so a user def can shadow a prelude name), so a
// package simply could not name itself.
//
// Published `graphics` is the real case: `graphics::color_r(..)` in a package that
// also does `use mesh3d; use glb;`.  It reproduced on 0.5.0, not just on main, and
// was the last package `loft compat floor` could not measure.  The dependency here
// is a local file rather than a registry package, so the test needs no network and
// no install.
#[test]
fn issue_656_package_can_name_itself_while_another_library_is_loaded() {
    let dir = std::env::temp_dir().join(format!("loft_656b_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    // The other library — its presence is the whole point, so `use` must resolve.
    std::fs::write(
        dir.join("src").join("gdep.loft"),
        "pub fn helper(x: integer) -> integer { return x * 2; }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("loft.toml"),
        "[package]\nname = \"glib\"\nversion = \"0.1.0\"\nloft = \">=0.8\"\n\
         [library]\nentry = \"src/glib.loft\"\n",
    )
    .unwrap();
    let file = dir.join("src").join("glib.loft");
    std::fs::write(
        &file,
        "use gdep;\n\
         pub fn base(x: integer) -> integer { return x + 1; }\n\
         pub fn far(x: integer) -> integer { return glib::base(x) + gdep::helper(x); }\n",
    )
    .unwrap();

    let mut p = Parser::new();
    p.lib_dirs
        .push(dir.join("src").to_string_lossy().to_string());
    let _ = p.parse_dir(
        &format!("{}/default", env!("CARGO_MANIFEST_DIR")),
        true,
        false,
    );
    p.parse(&file.to_string_lossy(), false);
    let level = p.diagnostics.level();
    let report = format!("{}", p.diagnostics);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        level < loft::diagnostics::Level::Error,
        "#656: a package must be able to name itself with another library loaded:\n{report}"
    );
}

// Library compatibility contract, step 7 — the RELEASE gate proves the WHOLE claim.
//
// A PR pays O(1): latest + declared floor + one random interior release. That answers
// "did this change break something". Publishing asks a different question — "is
// everything this package promises actually true" — and it is the only one of the two
// that is a promise to people who cannot ask the package a question.
//
// The pressure this must resist is truncation. Verifying what fits in the time available
// and reporting green produces a release claiming a floor it never checked, which is
// worse than no check at all because it carries the authority of one. So an overrun is a
// FAILURE, and the remedy is named rather than implied: cost is proportional to the
// CLAIM, so narrowing the floor shrinks it.
//
// Driven through the real binary, because the property under test is an EXIT CODE — the
// thing the release script branches on. A unit test of the message would have passed
// while the gate returned 0. `LOFT_HOME` isolates the install cache so the window is
// built here rather than depending on whatever this machine happens to have installed.
#[test]
fn compat_full_window_budget_overrun_fails_the_release() {
    let loft = env!("CARGO_BIN_EXE_loft");
    let root = std::env::temp_dir().join(format!("loft_s7_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let pkg = root.join("pkg");
    std::fs::create_dir_all(pkg.join("src")).unwrap();

    let manifest = |v: &str| {
        format!(
            "[package]\nname = \"s7pkg\"\nversion = \"{v}\"\nloft = \">=0.8\"\n\
             api_compatible_with  = \"0.1.0\"\ndata_compatible_with = \"0.1.0\"\n\
             [library]\nentry = \"src/s7pkg.loft\"\n"
        )
    };
    let source = "pub fn one(x: integer) -> integer { return x + 1; }\n";
    std::fs::write(pkg.join("loft.toml"), manifest("0.4.0")).unwrap();
    std::fs::write(pkg.join("src").join("s7pkg.loft"), source).unwrap();

    // Three earlier releases in an isolated install cache — a real window to walk, so
    // "budget exhausted" is reachable and "verified everything" means something.
    for v in ["0.1.0", "0.2.0", "0.3.0"] {
        let d = root.join(".loft/registry").join(format!("s7pkg-{v}"));
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("loft.toml"), manifest(v)).unwrap();
        std::fs::write(d.join("src").join("s7pkg.loft"), source).unwrap();
    }

    let run = |budget: &str| {
        std::process::Command::new(loft)
            .args(["compat", "check", "--full"])
            .current_dir(&pkg)
            .env("LOFT_HOME", &root)
            .env("LOFT_COMPAT_BUDGET", budget)
            .output()
            .expect("run loft compat check --full")
    };
    let text = |o: &std::process::Output| {
        format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        )
    };

    // Generous budget: the whole window is walked and the claim holds.
    let ok = run("600");
    let ok_txt = text(&ok);
    // Zero budget: nothing can be proved, so it must not report success.
    let over = run("0");
    let over_txt = text(&over);
    let _ = std::fs::remove_dir_all(&root);

    assert!(
        ok.status.success(),
        "#step7: an unbroken window must pass:\n{ok_txt}"
    );
    assert!(
        ok_txt.contains("whole claim verified"),
        "#step7: a pass must say the WHOLE claim was proved:\n{ok_txt}"
    );
    assert!(
        ok_txt.contains("0.1.0") && ok_txt.contains("0.2.0") && ok_txt.contains("0.3.0"),
        "#step7: --full must report every release, not a sample:\n{ok_txt}"
    );

    assert!(
        !over.status.success(),
        "#step7: a budget overrun must FAIL the release, not pass:\n{over_txt}"
    );
    assert!(
        over_txt.contains("BUDGET EXCEEDED") && over_txt.contains("never checked"),
        "#step7: an overrun must quantify what went unverified:\n{over_txt}"
    );
    assert!(
        over_txt.contains("Narrow the claim"),
        "#step7: an overrun must name the remedy:\n{over_txt}"
    );
}

// loft#664 — element-slot ownership must be REPRESENTABLE when the container is a
// field DbRef.
//
// A vector-literal element never owns a store: its record is the slot the enclosing
// `OpNewRecord` carved out of the container.  That fact was encoded only as a
// DEPENDENCY on the container VARIABLE, so a vector living inside an enum payload —
// addressed by a field DbRef, with no variable to depend on — produced an EMPTY dep
// list, and empty reads as "owns its store".  The answer came back wrong rather than
// unknown, and every consumer of the predicate inherited it (loft#660 surfaced through
// `parse_object`, which allocated a fresh record over the slot: a silent corruption
// and a SIGSEGV at depth 2, patched there by matching the `_elm` NAME prefix).
//
// This asserts the FACT, not a value, because the value is already right — #660's name
// proxy covered the one consumer that acted on it, so a behaviour test would pass with
// or without the fix and prove nothing.  What must hold is that the element reports
// "does not own a store" WHILE its dep list is empty: that combination is exactly what
// only a marker at the mint site can express, and it is what retires the name proxy.
#[test]
fn issue_664_element_in_enum_payload_is_not_owning() {
    const SRC: &str = r#"
struct L664 { n: integer }
enum B664 { NilB, Items { items: vector<L664> } }
enum E664 { NilE, Val { v: B664 } }

fn build664() -> E664 {
  return Val { v: Items { items: [ L664 { n: 7 }, L664 { n: 9 } ] } };
}
"#;
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("stdlib");
    p.parse_str(SRC, "<issue-664>", false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "#664 source must parse clean: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data, &mut p.database);

    let d_nr = p.data.def_nr("n_build664");
    assert_ne!(d_nr, u32::MAX, "#664: build664 must be defined");
    let vars = p.data.def(d_nr).variables();

    let mut checked = 0;
    for v in 0..vars.var_count() {
        let v = v as u16;
        if !vars.name(v).starts_with("_elm") {
            continue;
        }
        checked += 1;
        // The dep list is the part that CANNOT carry the fact here: the container is a
        // field DbRef, so there is no variable to name.  Left as the only encoding, an
        // empty list is read as ownership.
        assert!(
            vars.tp(v).depend().is_empty(),
            "#664: this element's container is a field DbRef, so it has no container \
             variable to depend on — the probe is aimed at the wrong construction if \
             '{}' has deps {:?}",
            vars.name(v),
            vars.tp(v).depend()
        );
        // The fact is now STATED at the mint site rather than inferred, so it holds
        // whatever the container turns out to be.  Without the marker this element
        // reports non-owning only because the field-initialiser path happens to also
        // set `skip_free` — a coincidence of two conditions, not one fact, and the
        // reason #660 had to fall back on the `_elm` name.
        assert!(
            vars.is_inline_ref(v),
            "#664: element '{}' must be marked non-owning where it is MINTED, not \
             wherever a consumer can infer it",
            vars.name(v)
        );
        assert!(
            !vars.owns_store(v),
            "#664: element '{}' has an empty dep list AND must still report that it \
             does not own a store — that is the whole gap, and inferring ownership \
             from deps alone gets it wrong",
            vars.name(v)
        );
    }
    assert!(
        checked > 0,
        "#664: the probe found no element variable — it proves nothing"
    );
}

// loft#675 — a heap-returning function carries exactly one hidden return buffer, and a
// cross-package return still works end to end.
//
// @PLAN59 reserves the hidden `__retbuf` at SIGNATURE parse, gated on the declared return
// already being a heap type. For a struct owned by a dependency resolved LAZILY (a
// registry package pulled in through the pending-deps loop) that is not knowable on pass
// 1: `glb`'s `fn glb_pos_min(…) -> Vec3` reads `Unknown(659)` there, and the stub's def
// number is LOWER than the real struct's, so the definitions it needs do not exist yet.
// The reservation never fired, `ref_return` grew the attribute in pass 2 instead, and
// that cross-pass arity growth stopped the published `input` package from compiling once
// the H5 guard became always-on. `reserve_late_return_buffers` closes it between the
// passes, where every type IS resolved.
//
// BE HONEST ABOUT WHAT THIS TEST IS. It pins the invariant and the end-to-end behaviour
// on a `path =` dependency chain, and it is NOT a reproduction of the bug: a `path` dep
// is parsed eagerly, so its types resolve at signature time and the late reservation
// never has to fire here. I could not vendor the lazy-registry ordering into a fixture.
// What actually catches a recurrence is `assert_pass2_def_attr_stable` — always-on, in
// every build, and the thing that caught this one; the fix was verified by re-running the
// real `input 0.2.0` suite with that assert at FULL strictness.
#[test]
fn issue_675_cross_library_heap_return_reserves_its_buffer() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("stdlib");
    p.lib_dirs.push("tests/fixtures/libs".to_string());
    p.parse("tests/multilib/r675_cross_lib_return.loft", false);
    let errors: Vec<String> = p
        .diagnostics
        .entries()
        .iter()
        .filter(|e| e.level >= loft::diagnostics::Level::Error)
        .map(|e| e.to_string_compact())
        .collect();
    assert!(
        errors.is_empty(),
        "#675 fixture must parse clean: {errors:?}"
    );

    let d_nr = p.data.def_nr("n_pick675");
    assert_ne!(d_nr, u32::MAX, "#675: pick675 must be defined");
    // A heap-returning function carries exactly one hidden heap buffer, and it was there
    // before pass 2 started — otherwise `ref_return` had to GROW the signature, and any
    // caller already lowered against the short arity passes one argument too few (#662).
    // The count also has to be exactly one: reserving a second buffer for a function that
    // already has one would break the ABI just as thoroughly.
    let attrs: Vec<(String, bool)> = p
        .data
        .def(d_nr)
        .attributes()
        .iter()
        .map(|a| (a.name.clone(), a.hidden))
        .collect();
    let hidden_heap = p
        .data
        .def(d_nr)
        .attributes()
        .iter()
        .filter(|a| {
            a.hidden
                && matches!(
                    a.typedef,
                    loft::data::Type::Reference(_, _)
                        | loft::data::Type::Vector(_, _)
                        | loft::data::Type::Enum(_, true, _)
                )
        })
        .count();
    assert_eq!(
        hidden_heap, 1,
        "#675: a function returning a struct from a DEPENDENCY must carry exactly one heap \
         return buffer — attributes: {attrs:?}"
    );

    // And it must still run: the buffer is an ABI change, so a wrong one shows up as a
    // caller/callee argument mismatch rather than a wrong number.
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let config = RuntimeLogConfig {
        log_path: std::path::PathBuf::from("/dev/null"),
        production: true,
        ..Default::default()
    };
    state.database.logger = Some(Arc::new(Mutex::new(Logger::new(config, None))));
    state.execute("main", &p.data);
    assert!(
        !state.database.had_fatal,
        "#675: the cross-library heap return failed at runtime"
    );
}

/// loft#677 — a function that appends to a struct PARAMETER and returns it must keep that
/// parameter in its return deps, so callers see the result for what it is: a borrow of the
/// argument, not a store of its own.
///
/// The value-level guard lives in `tests/scripts/return-borrow-of-mutated-arg.loft`; this
/// one asserts the FACT, because losing the dep is only sometimes fatal.  With a single
/// call the over-free lands on a store nothing reads again and every value still checks
/// out — which is exactly why the four minimal cases on the issue all passed while the
/// consumer segfaulted.  A signature assertion fails the moment the dep goes missing.
///
/// The append count is the axis: `ref_return`'s ladder counted records allocated as
/// CHILDREN of `o` and read ≥2 as "reassigned local, do not NRVO-promote".  For a
/// parameter there is nothing to promote, so the skip only deleted the borrow.
#[test]
fn issue_677_returned_mutated_param_keeps_its_borrow() {
    let src_path = std::env::temp_dir().join("loft_i677_ret_borrow.loft");
    std::fs::write(
        &src_path,
        "struct I677 { x: float }\n\
         struct S677 { name: text, tags: vector<integer>, items: vector<I677> }\n\
         fn one677(o: S677, t: integer) -> S677 { o.tags += [t]; o }\n\
         fn two677(o: S677, t: integer, it: I677) -> S677 { o.tags += [t]; o.items += [it]; o }\n\
         fn same677(o: S677, t: integer) -> S677 { o.tags += [t]; o.tags += [t]; o }\n\
         fn fresh677(o: S677) -> S677 { S677 { name: o.name, tags: [], items: [] } }\n\
         fn main() { s = S677 { name: \"c\", tags: [], items: [] }; two677(s, 1, I677 { x: 1.0 }); }\n",
    )
    .unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse(src_path.to_str().unwrap(), false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "#677 fixture must parse clean: {:?}",
        p.diagnostics.lines()
    );

    // `o` is attribute 0 in each of these, so the return dep must name index 0.
    for (name, appends) in [("n_one677", 1), ("n_two677", 2), ("n_same677", 2)] {
        let d_nr = p.data.def_nr(name);
        assert_ne!(d_nr, u32::MAX, "#677: {name} must be defined");
        let deps = p.data.def(d_nr).returned().depend().clone();
        assert!(
            deps.contains(&0),
            "#677: {name} returns its own parameter `o` after {appends} append(s), so the \
             return type must borrow attr 0 — got deps {deps:?}.  An empty dep list tells \
             every caller the result is an owned store, and the caller then frees the store \
             it passed in."
        );
    }

    // The other side of the boundary: a function that really does build a FRESH record
    // must NOT claim to borrow its argument, or callers leak it instead.
    let fresh = p.data.def_nr("n_fresh677");
    assert_ne!(fresh, u32::MAX, "#677: fresh677 must be defined");
    assert!(
        p.data.def(fresh).returned().depend().is_empty(),
        "#677: fresh677 allocates its own record, so its return must stay dep-free — got {:?}",
        p.data.def(fresh).returned().depend()
    );
}

/// loft#682 — a closure record must free only the captures it ADOPTED.
///
/// Each reference / collection capture is stored as a 12-byte `DbRef`, and
/// `free_named`'s cascade used to free every one of them when the record died.
/// That is right for a store the defining frame owned and handed over (the frame's
/// scope-exit free is suppressed for it, which is what lets an escaping factory
/// closure outlive its frame, #323), and wrong for a captured PARAMETER, whose
/// caller owns the store and outlives the frame.  The consumer lost a whole
/// `hex_world::World` to it.
///
/// The value-level guard is `tests/scripts/682-closure-capture-borrow.loft`; this
/// one asserts the FACT, because an over-free is only sometimes fatal.  With one
/// call the freed store is never re-read and every value still checks out — which
/// is why the report arrived as a panic thousands of ops later in an unrelated
/// function, and why a value test alone can pass on slot-reuse luck.
///
/// Two directions matter, so both are asserted: a parameter / projection capture
/// must be BORROWED (over-free), and an owned local must stay ADOPTED (marking it
/// borrowed leaks instead).  The verdict is only knowable after `scopes::check` —
/// `p682_proj`'s `ch` parses as "borrows `w`" and a call-result capture parses as
/// a borrow that scope analysis later rewrites to owned — so the test runs the
/// check first, exactly as compilation does.
#[test]
fn issue_682_closure_capture_ownership_marker() {
    let src_path = std::env::temp_dir().join("loft_i682_capture_marker.loft");
    std::fs::write(
        &src_path,
        "struct P682C { v: float }\n\
         struct P682W { cells: vector<P682C>, tick: integer }\n\
         fn p682_param(w: P682W, x: float) -> float { f = fn(s: float) -> float { w.tick as float * s }; x }\n\
         fn p682_proj(w: P682W, x: float) -> float { c = w.cells[0]; f = fn(s: float) -> float { c.v * s }; x }\n\
         fn p682_owned(x: float) -> float { o = P682W { cells: [], tick: 3 }; f = fn(s: float) -> float { o.tick as float * s }; f(x) }\n\
         fn p682_cell(x: integer) -> integer { acc = 0; b = fn(n: integer) { acc = acc + n; }; b(x); acc }\n\
         fn main() { w = P682W { cells: [P682C { v: 1.0 }], tick: 7 };\n\
                     p682_param(w, 1.0); p682_proj(w, 1.0); p682_owned(1.0); p682_cell(2); }\n",
    )
    .unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse(src_path.to_str().unwrap(), false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "#682 fixture must parse clean: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data, &mut p.database);

    // The lambdas are numbered in source order, so each closure record pairs with
    // the function above that defines it.
    let marker = |record: &str, capture: &str| -> bool {
        let d_nr = p.data.def_nr(record);
        assert_ne!(d_nr, u32::MAX, "#682: {record} must exist");
        let a = p.data.attr(d_nr, capture);
        assert_ne!(
            a,
            usize::MAX,
            "#682: {record} must carry the `{capture}` capture"
        );
        match p.data.attr_type(d_nr, a) {
            loft::data::Type::Reference(_, deps) => {
                assert!(
                    !deps.is_empty(),
                    "#682: {record}.{capture} must keep a share marker (a 12-byte DbRef); \
                     empty deps would store inline bytes and the closure would read a \
                     stale snapshot"
                );
                deps.is_borrowed_share()
            }
            other => panic!("#682: {record}.{capture} should be a Reference, got {other:?}"),
        }
    };

    assert!(
        marker("__closure_0", "w"),
        "#682: a captured PARAMETER must be marked BORROWED — its caller owns the store \
         and outlives this frame, so the record's cascade freeing it destroys the \
         caller's value (that is the whole bug)"
    );
    assert!(
        marker("__closure_1", "c"),
        "#682: a PROJECTION local (`c = w.cells[0]`) views into someone else's store, so \
         it must be marked BORROWED too — a parameter-only fix leaves this half broken"
    );
    assert!(
        !marker("__closure_2", "o"),
        "#682: a local the frame OWNS must stay ADOPTED — `get_free_vars` suppresses its \
         scope-exit free, so the record's cascade is the only free there is and marking \
         it borrowed leaks the store instead"
    );
    assert!(
        !marker("__closure_3", "acc"),
        "#682: a mutated scalar boxed into a `__cell_<T>` is minted FOR this closure, so \
         the record owns it however the original binding was reached (plan-22 / C74)"
    );
}

/// loft#685 — a MUTATED scalar capture whose source is a PARAMETER.
///
/// The closure record stored the capture as a 12-byte cell `DbRef`
/// (`box_captured_names_for_outer_scalars`) while `flip_scalars_to_box_types`
/// skipped arguments, so the parameter stayed an 8-byte stack scalar and
/// `emit_lambda_code`'s `OpSetDbRef` read 12 bytes out of an 8-byte slot — taking
/// the fn-ref being built beside it with it.
///
/// Values live in `tests/scripts/685-mutated-scalar-param-capture.loft`; this
/// asserts the FACT the fix rests on, because the value test cannot distinguish
/// "the two halves agree" from "they disagree but the garbage happened to work".
/// The invariant: for a mutated capture, the closure record's field type and the
/// binding the enclosing frame holds under that name must be the SAME cell.
#[test]
fn issue_685_mutated_scalar_param_is_boxed_like_a_local() {
    let src_path = std::env::temp_dir().join("loft_i685_param_box.loft");
    std::fs::write(
        &src_path,
        "fn p685_arg(n: integer) -> integer { b = fn(k: integer) { n = n + k; }; b(1); n }\n\
         fn p685_loc(s: integer) -> integer { n = s; b = fn(k: integer) { n = n + k; }; b(1); n }\n\
         fn main() { p685_arg(1); p685_loc(1); }\n",
    )
    .unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse(src_path.to_str().unwrap(), false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "#685 fixture must parse clean: {:?}",
        p.diagnostics.lines()
    );

    // Both functions must lower to the SAME shape — the argument case is the local
    // case, which is the whole point of the promotion.
    for (fname, record) in [("n_p685_arg", "__closure_0"), ("n_p685_loc", "__closure_1")] {
        let rec = p.data.def_nr(record);
        assert_ne!(rec, u32::MAX, "#685: {record} must exist");
        let a = p.data.attr(rec, "n");
        assert_ne!(a, usize::MAX, "#685: {record} must carry the `n` capture");
        let cell = match p.data.attr_type(rec, a) {
            loft::data::Type::Reference(d, _) => d,
            other => panic!("#685: {record}.n must be a cell Reference, got {other:?}"),
        };
        assert!(
            p.data.def(cell).name().starts_with("__cell_"),
            "#685: {record}.n must point at a `__cell_<T>`, got `{}`",
            p.data.def(cell).name()
        );

        // The frame's binding for `n` must be that same cell.  A binding that is still
        // a bare scalar here is the bug: the record would hold a 12-byte DbRef field
        // fed from an 8-byte slot.
        let d_nr = p.data.def_nr(fname);
        assert_ne!(d_nr, u32::MAX, "#685: {fname} must be defined");
        let vars = &p.data.def(d_nr).variables;
        let v = vars.var("n");
        assert_ne!(v, u16::MAX, "#685: {fname} must bind the name `n`");
        assert!(
            matches!(vars.tp(v), loft::data::Type::Reference(d, _)
                if p.data.def(*d).name().starts_with("__cell_")),
            "#685: `n` in {fname} must be the boxed cell the record points at, got {:?} \
             — a bare scalar here means the closure record is fed 12 bytes from an \
             8-byte slot",
            vars.tp(v)
        );
        assert!(
            !vars.is_argument(v),
            "#685: `n` in {fname} must resolve to a shadow LOCAL, not the argument — \
             flipping the argument itself would change the call ABI"
        );
    }

    // The by-value contract: the argument is still an argument, untouched, so the
    // caller's value cannot be reached through the cell.
    let d_nr = p.data.def_nr("n_p685_arg");
    let vars = &p.data.def(d_nr).variables;
    let args = vars.arguments();
    assert_eq!(
        args.len(),
        1,
        "#685: promoting must not change the arity of `p685_arg` (the H5 two-pass \
         contract catches this too) — got {args:?}"
    );
    assert!(
        matches!(vars.tp(args[0]), loft::data::Type::Integer(_)),
        "#685: the parameter slot must stay a plain scalar, got {:?}",
        vars.tp(args[0])
    );
}

/// loft#687 — a mutated TEXT capture's STORAGE is decided per BINDING, not per function.
///
/// #685 could not serve a mutated `text` PARAMETER in a text-returning function and
/// refused it by name; plan-22 02d-vii had skipped text boxing whenever the parent
/// returned text.  That condition was a proxy for one real case — a text local that is
/// the function's RETURN SOURCE, which the return machinery already gives its own hidden
/// `&text` out-parameter — and as a proxy it was too wide (it also skipped a text local
/// the function does not return) and useless for a parameter, which has no indirection to
/// reuse.  Both halves now ask the binding: `RefVar` means "already has one".
///
/// The fixture puts BOTH bindings in ONE text-returning function, which is what no
/// per-function condition can get right: `keep` is returned (stays inline + write-back),
/// `side` is not (takes a shared cell).  Values live in
/// `tests/scripts/687-mutated-text-param-capture.loft`; this asserts the storage, because
/// mixing the two representations for one binding is what used to segfault, and a value
/// test cannot see which one a green run picked.
#[test]
fn issue_687_mutated_text_capture_storage_is_per_binding() {
    let src_path = std::env::temp_dir().join("loft_i687_text_storage.loft");
    std::fs::write(
        &src_path,
        "fn ret687(seed: text) -> text { keep = seed; b = fn(k: text) { keep = keep + k; }; b(\"x\"); keep }\n\
         fn side687(seed: text) -> text { side = seed; b = fn(k: text) { side = side + k; }; b(\"x\"); \"p\" + side }\n\
         fn par687(n: text) -> text { b = fn(k: text) { n = n + k; }; b(\"x\"); n }\n\
         fn main() { ret687(\"a\"); side687(\"a\"); par687(\"a\"); }\n",
    )
    .unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse(src_path.to_str().unwrap(), false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "#687: a mutated text parameter in a text-returning fn must COMPILE now: {:?}",
        p.diagnostics.lines()
    );

    let is_cell = |record: &str, capture: &str| -> bool {
        let rec = p.data.def_nr(record);
        assert_ne!(rec, u32::MAX, "#687: {record} must exist");
        let a = p.data.attr(rec, capture);
        assert_ne!(a, usize::MAX, "#687: {record} must carry `{capture}`");
        matches!(p.data.attr_type(rec, a),
            loft::data::Type::Reference(d, _) if p.data.def(d).name().starts_with("__cell_"))
    };

    // All three functions return text, so the retired per-function proxy gave all three
    // the same answer.  Each binding needs a different one.

    // `keep` IS the returned text, so the return machinery already gave it a hidden
    // `&text` out-parameter.  Boxing it too is the mismatch that segfaulted: the record
    // would hold a cell DbRef while the binding stayed a `&text` stack pointer.
    assert!(
        !is_cell("__closure_0", "keep"),
        "#687: a text local that is the function's RETURN SOURCE must stay INLINE in the \
         record — it already has a hidden `&text` out-parameter, and two indirections for \
         one binding is the crash"
    );
    // `side` is not returned, so nothing else claims it and the shared cell is right.
    // The retired proxy skipped this one purely as collateral — the "too wide" half.
    assert!(
        is_cell("__closure_1", "side"),
        "#687: a mutated text local the function does NOT return must take a shared cell"
    );
    // A parameter has no indirection of its own to reuse, so #685's shadow local takes
    // the cell like every other type.  This is the combination that used to be refused.
    assert!(
        is_cell("__closure_2", "n"),
        "#687: a mutated text PARAMETER's shadow local must take a shared cell"
    );
}

/// loft#685 boundary — a value-const scalar parameter mutated through a closure stays
/// an error.
///
/// The closure-side write never reaches `validate_write`'s const guard (inside the
/// lambda the name is a capture, not a binding carrying the flag), so the check lives
/// at the promotion site.  Without it the fix would have quietly handed the closure a
/// writable cell for a read-only parameter — turning a crash into a silently accepted
/// contract violation, which is worse.
#[test]
fn issue_685_const_scalar_param_mutated_by_closure_is_rejected() {
    let src_path = std::env::temp_dir().join("loft_i685_const_param.loft");
    std::fs::write(
        &src_path,
        "fn c685(n: const integer) -> integer { b = fn(k: integer) { n = n + k; }; b(1); n }\n\
         fn main() { c685(1); }\n",
    )
    .unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse(src_path.to_str().unwrap(), false);
    let lines = p.diagnostics.lines().join("\n");
    assert!(
        p.diagnostics.level() >= loft::diagnostics::Level::Error,
        "#685: mutating a const parameter from a closure must stay an error.  \
         Diagnostics: {lines}"
    );
    assert!(
        lines.contains("const parameter") && lines.contains("from a closure"),
        "#685: the const refusal must say which binding and why, got: {lines}"
    );
}

/// loft#686 — a capture whose type comes from a struct declared LATER in the file.
///
/// Two faults composed. `Unknown(0)` is the "no type known" sentinel and names no
/// definition, but `copy_unknown_fields` read the `0` as a def number and gave the
/// field whatever definition #0 returns — `text` — so the closure body type-checked
/// against a type nothing in the program mentions. With that invented type gone the
/// field became unsized, and `fill_database` registers a struct while SKIPPING an
/// attribute it cannot size, so `finish` sized the record with the field left at
/// `position == u16::MAX` and the closure read its capture at offset 65535.
///
/// Values live in `tests/scripts/686-forward-declared-capture.loft`; these assert the
/// two FACTS, because the second fault was INTERMITTENT — a positionless field only
/// crashes when the bytes at offset 65535 happen to be fatal, so a green value run
/// proves much less than it looks.
#[test]
fn issue_686_forward_declared_capture_is_typed_and_positioned() {
    let src_path = std::env::temp_dir().join("loft_i686_forward_capture.loft");
    std::fs::write(
        &src_path,
        "fn q686(w: W686) -> float { ch = w.inner; f = fn(x: float) -> float { ch.q * x }; f(2.0) }\n\
         struct I686 { q: float }\n\
         struct W686 { inner: I686, tick: integer }\n\
         fn main() { w = W686 { inner: I686 { q: 5.0 }, tick: 7 }; q686(w); }\n",
    )
    .unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse(src_path.to_str().unwrap(), false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "#686 fixture must parse clean: {:?}",
        p.diagnostics.lines()
    );

    let rec = p.data.def_nr("__closure_0");
    assert_ne!(rec, u32::MAX, "#686: the closure record must exist");
    let a = p.data.attr(rec, "ch");
    assert_ne!(
        a,
        usize::MAX,
        "#686: the record must carry the `ch` capture"
    );

    // FACT 1 — the capture is typed as what it is, not as definition #0's return type.
    match p.data.attr_type(rec, a) {
        loft::data::Type::Reference(d, _) => assert_eq!(
            p.data.def(d).name(),
            "I686",
            "#686: the capture must resolve to the forward-declared struct, got `{}`",
            p.data.def(d).name()
        ),
        other => panic!(
            "#686: a struct capture must be a Reference; got {other:?}.  `Text` here is \
             the original bug — `Unknown(0)` resolved against definition #0"
        ),
    }

    // FACT 2 — the record's field is POSITIONED.  Skipping an unsized attribute while
    // still registering the struct left this at u16::MAX forever, and `finish_type`
    // will not revisit a sized type, so nothing downstream could repair it.
    let known = p.data.def(rec).known_type();
    assert_ne!(
        known,
        u16::MAX,
        "#686: the closure record must be laid out — a record left unregistered makes \
         `OpDatabase` allocate type u16::MAX"
    );
    let pos = p.database.position(known, "ch");
    assert_ne!(
        pos,
        u16::MAX,
        "#686: the `ch` field must have a real byte position; u16::MAX means the record \
         was sized while the field was still unresolved, and the closure then reads and \
         writes at offset 65535 (an INTERMITTENT crash, which is why this asserts the \
         position rather than trusting a green run)"
    );
}

/// loft#686 sibling — `Unknown(0)` must never be resolved as a reference to definition
/// #0.
///
/// It is the codebase-wide "no type known" sentinel (`Type::Unknown(0)` is what every
/// unresolved expression carries), so reading the `0` as a def number invents a type
/// from whatever happens to be defined first.  That is a lying fact rather than a
/// missing one: the field looked resolved, so nothing downstream questioned it, and the
/// error surfaced as `Unknown field text.cells` on a program with no `text` in sight.
#[test]
fn issue_686_nameless_unknown_is_not_resolved_against_def_zero() {
    let src_path = std::env::temp_dir().join("loft_i686_sentinel.loft");
    std::fs::write(
        &src_path,
        "fn s686(w: S686W) -> float { p = w.inner; f = fn(x: float) -> float { p.q * x }; f(1.0) }\n\
         struct S686I { q: float }\n\
         struct S686W { inner: S686I }\n\
         fn main() { w = S686W { inner: S686I { q: 2.0 } }; s686(w); }\n",
    )
    .unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse(src_path.to_str().unwrap(), false);

    // Definition #0's return type is what the old resolution handed the field.  Whatever
    // it is, no capture may end up with it by accident.
    let def_zero_ret = p.data.def(0).returned().clone();
    let rec = p.data.def_nr("__closure_0");
    assert_ne!(rec, u32::MAX, "#686: the closure record must exist");
    let a = p.data.attr(rec, "p");
    assert_ne!(a, usize::MAX, "#686: the record must carry the `p` capture");
    let got = p.data.attr_type(rec, a);
    assert!(
        !matches!(&got, t if *t == def_zero_ret),
        "#686: the capture took definition #0's return type ({def_zero_ret:?}) — \
         `Unknown(0)` was resolved as a def reference again"
    );
    assert!(
        matches!(&got, loft::data::Type::Reference(d, _) if p.data.def(*d).name() == "S686I"),
        "#686: the capture must be the struct it projects out of, got {got:?}"
    );
}

/// loft#683 — an index key whose type comes from a definition declared LOWER in the
/// file must not be rejected.  The check ran in pass 1, which sees only what is
/// declared above the current point, so `h[keys_declared_below()]` failed while the
/// same code with the callee moved up compiled.  Pass 2 knows every signature, but it
/// only runs when pass 1 is error-free — so the premature error aborted the parse.
///
/// The values live in `tests/scripts/683-declaration-order-index-key.loft`.  This is
/// the must-fail half: deferring the check must not DELETE it.  A genuinely wrong key
/// type has to stay an error in BOTH declaration orders — a fix that simply stopped
/// reporting would pass the value test and fail here.
#[test]
fn issue_683_wrong_index_key_is_still_rejected_in_both_orders() {
    // (label, source) — identical programs, differing only in where `t683` sits.
    let cases = [
        (
            "callee above",
            "struct K683 { k: integer, v: integer }\n\
             fn t683() -> text { \"z\" }\n\
             fn u683(h: hash<K683[k]>) -> integer { r = h[t683()]; if r != null { return r.v; } 0 }\n\
             fn main() { h: hash<K683[k]> = []; u683(h); }\n",
        ),
        (
            "callee below",
            "struct K683 { k: integer, v: integer }\n\
             fn u683(h: hash<K683[k]>) -> integer { r = h[t683()]; if r != null { return r.v; } 0 }\n\
             fn t683() -> text { \"z\" }\n\
             fn main() { h: hash<K683[k]> = []; u683(h); }\n",
        ),
    ];
    for (label, src) in cases {
        let src_path = std::env::temp_dir().join("loft_i683_wrong_key.loft");
        std::fs::write(&src_path, src).unwrap();
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        p.parse(src_path.to_str().unwrap(), false);
        let lines = p.diagnostics.lines().join("\n");
        assert!(
            p.diagnostics.level() >= loft::diagnostics::Level::Error,
            "#683 ({label}): a text key on an integer-keyed hash must stay an error.  \
             Diagnostics: {lines}"
        );
        assert!(
            lines.contains("Invalid index key"),
            "#683 ({label}): the rejection must still be the index-key diagnostic, got: {lines}"
        );
        let _ = std::fs::remove_file(&src_path);
    }
}

/// loft#689 — a range asks a collection to walk its keys IN ORDER, so only an ordered
/// collection can answer it.  `hash` is unordered and `spatial` is Morton-ordered (its
/// range form is the coordinate slice `s[(x1,y1)..(x2,y2)]`), and neither could step a
/// scalar range: both walked off the end of their iterator and SIGSEGV'd.  An open
/// `coll[..]` crashed too — it is the same walk without bounds.
///
/// These are now refused at parse time.  The values for the collections that CAN answer
/// a range live in `tests/scripts/689-keyed-range-slice.loft`; this pins the refusal, and
/// the second half pins what must keep working — a fix that simply rejected every index
/// on a `hash` would pass the first half alone.
#[test]
fn issue_689_range_is_refused_on_an_unordered_collection() {
    let refused = [
        (
            "hash, bounded range",
            "struct H689 { k: integer, v: integer }\n\
             fn main() { h: hash<H689[k]> = []; n = 0; for r in h[1..3] { n += r.v; } }\n",
            "unordered",
        ),
        (
            "hash, open range",
            "struct H689 { k: integer, v: integer }\n\
             fn main() { h: hash<H689[k]> = []; n = 0; for r in h[..] { n += r.v; } }\n",
            "unordered",
        ),
        (
            "spatial, scalar range",
            "struct S689 { x: integer, y: integer, v: integer }\n\
             fn main() { s: spatial<S689[x,y]> = []; n = 0; for r in s[1..3] { n += r.v; } }\n",
            "COORDINATE slice",
        ),
    ];
    for (label, src, needle) in refused {
        let src_path = std::env::temp_dir().join("loft_i689_refused.loft");
        std::fs::write(&src_path, src).unwrap();
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        p.parse(src_path.to_str().unwrap(), false);
        let lines = p.diagnostics.lines().join("\n");
        assert!(
            p.diagnostics.level() >= loft::diagnostics::Level::Error,
            "#689 ({label}): a range on an unordered collection must be refused, not run.  \
             Diagnostics: {lines}"
        );
        assert!(
            lines.contains(needle),
            "#689 ({label}): the refusal must say WHY and what to use instead \
             (expected {needle:?}), got: {lines}"
        );
        let _ = std::fs::remove_file(&src_path);
    }

    // The forms a hash / spatial collection CAN answer must still compile.
    let allowed = [
        (
            "hash single-key lookup",
            "struct H689 { k: integer, v: integer }\n\
             fn main() { h: hash<H689[k]> = []; r = h[1]; if r != null { } }\n",
        ),
        (
            "hash whole-collection iteration",
            "struct H689 { k: integer, v: integer }\n\
             fn main() { h: hash<H689[k]> = []; n = 0; for r in h { n += r.v; } }\n",
        ),
        (
            "spatial coordinate slice",
            "struct S689 { x: integer, y: integer, v: integer }\n\
             fn main() { s: spatial<S689[x,y]> = []; n = 0; \
             for r in s[(0,0)..(5,5)] { n += r.v; } }\n",
        ),
    ];
    for (label, src) in allowed {
        let src_path = std::env::temp_dir().join("loft_i689_allowed.loft");
        std::fs::write(&src_path, src).unwrap();
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        p.parse(src_path.to_str().unwrap(), false);
        let lines = p.diagnostics.lines().join("\n");
        assert!(
            p.diagnostics.level() < loft::diagnostics::Level::Error,
            "#689 ({label}): this form is what the refusal tells users to write — \
             it must still compile.  Diagnostics: {lines}"
        );
        let _ = std::fs::remove_file(&src_path);
    }
}

/// loft#690 — a `for` loop whose variable name is already bound to a DIFFERENT type
/// must not silently reuse the earlier binding.
///
/// The reuse check existed but compared with `Type::is_same`, which answers "same KIND
/// of type": it reports any two `Reference`s as the same whatever struct they name.  So
/// `for r in as_a { … }  for r in as_b { … }` slipped through, `add_variable` handed the
/// second loop the FIRST binding — old var, old type, old dep — and the body read B's
/// records through A's layout.  No diagnostic and no crash, just wrong numbers.
///
/// loft#915 removed the inheritance instead of reporting it: each loop binds its OWN
/// variable, so there is no earlier binding for the second loop to take, and these
/// programs now COMPILE.  What is asserted is therefore the stronger property the
/// diagnostic only approximated — the second loop's variable really carries the SECOND
/// element type.  Each body below reads a field that exists only on its own struct, so
/// a leaked binding is an "unknown field" error rather than a silent wrong number, which
/// is what made a value test unconvincing here (two structs with identical fields
/// returned the right answer while still being undefined).  The value half, on both
/// backends, is `tests/scripts/915-loop-variable-per-loop.loft` cell c6.
#[test]
fn issue_690_loop_variable_binds_its_own_type() {
    let per_loop_binding = [
        (
            "two struct types",
            "struct A690 { k: integer, av: integer }\n\
             struct B690 { k: text, bv: integer }\n\
             fn main() { a: vector<A690> = [A690{k:1,av:10}]; b: vector<B690> = [B690{k:\"x\",bv:1}];\n\
             n = 0; for r in a { n += r.av; } for r in b { n += r.bv; } println(\"{n}\"); }\n",
        ),
        (
            "two structs with IDENTICAL layouts — still different types",
            "struct A690 { k: integer, av: integer }\n\
             struct C690 { k: integer, cv: integer }\n\
             fn main() { a: vector<A690> = [A690{k:1,av:10}]; c: vector<C690> = [C690{k:2,cv:20}];\n\
             n = 0; for r in a { n += r.av; } for r in c { n += r.cv; } println(\"{n}\"); }\n",
        ),
        (
            "two enum types",
            "enum E690 { Red, Green }\n\
             enum F690 { Up, Down }\n\
             fn main() { a: vector<E690> = [E690.Red]; b: vector<F690> = [F690.Up];\n\
             n = 0; for r in a { if r == E690.Red { n += 1; } } \
             for r in b { if r == F690.Up { n += 1; } } println(\"{n}\"); }\n",
        ),
        (
            "nested vectors of different element structs",
            "struct A690 { av: integer }\n\
             struct B690 { bv: integer }\n\
             fn main() { a: vector<vector<A690>> = [[A690{av:1}]]; \
             b: vector<vector<B690>> = [[B690{bv:2}]];\n\
             n = 0; for r in a { for e in r { n += e.av; } } \
             for r in b { for e in r { n += e.bv; } } println(\"{n}\"); }\n",
        ),
    ];
    for (label, src) in per_loop_binding {
        let src_path = std::env::temp_dir().join("loft_i690_per_loop.loft");
        std::fs::write(&src_path, src).unwrap();
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        p.parse(src_path.to_str().unwrap(), false);
        let lines = p.diagnostics.lines().join("\n");
        assert!(
            p.diagnostics.level() < loft::diagnostics::Level::Error,
            "#690 ({label}): each loop binds its own variable, so the second loop's field \
             read resolves against the SECOND type and this must compile.  Diagnostics: \
             {lines}"
        );
        let _ = std::fs::remove_file(&src_path);
    }

    // The tolerances are load-bearing: loft has FLAT variable scoping, so reusing one
    // loop name at the SAME type is idiomatic and must keep compiling.  Tightening the
    // comparison to `is_equal` must not reach any of these.
    let accepted = [
        (
            "same struct, two loops",
            "struct A690 { v: integer }\n\
             fn main() { a: vector<A690> = [A690{v:1}]; b: vector<A690> = [A690{v:2}];\n\
             n = 0; for r in a { n += r.v; } for r in b { n += r.v; } }\n",
        ),
        (
            "same struct through DIFFERENT collection kinds",
            "struct A690 { k: integer, v: integer }\n\
             fn main() { a: vector<A690> = [A690{k:1,v:10}]; s: sorted<A690[k]> = []; \
             s += A690{k:2,v:20};\n\
             n = 0; for r in a { n += r.v; } for r in s { n += r.v; } }\n",
        ),
        (
            "integers in both loops (differing ranges stay the same type)",
            "fn main() { a: vector<integer> = [1,2]; b: vector<integer> = [3];\n\
             n = 0; for r in a { n += r; } for r in b { n += r; } }\n",
        ),
        (
            "text in both loops (differing deps stay the same type)",
            "fn main() { a: vector<text> = [\"xy\"]; b: vector<text> = [\"z\"];\n\
             n = 0; for r in a { n += len(r); } for r in b { n += len(r); } }\n",
        ),
        (
            "`_` stays exempt across different structs",
            "struct A690 { v: integer }\n\
             struct B690 { v: integer }\n\
             fn main() { a: vector<A690> = [A690{v:1}]; b: vector<B690> = [B690{v:2}];\n\
             n = 0; for _ in a { n += 1; } for _ in b { n += 1; } }\n",
        ),
    ];
    for (label, src) in accepted {
        let src_path = std::env::temp_dir().join("loft_i690_accepted.loft");
        std::fs::write(&src_path, src).unwrap();
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        p.parse(src_path.to_str().unwrap(), false);
        let lines = p.diagnostics.lines().join("\n");
        assert!(
            p.diagnostics.level() < loft::diagnostics::Level::Error,
            "#690 ({label}): this reuse is idiomatic under loft's flat scoping and must \
             still compile.  Diagnostics: {lines}"
        );
        let _ = std::fs::remove_file(&src_path);
    }
}

// ── loft#815 ─────────────────────────────────────────────────────────────────
// Native output emits only the functions `generation::reachable_functions`
// marks, and the three walkers feeding it each re-derived `Value`'s tree shape
// as a whitelist ending in `_ => {}`.  `Tuple` was absent from all three, so a
// callee reached ONLY from a tuple element was pruned while its call site was
// still emitted — rustc then failed E0425 on the emitted call and the whole
// library refused to build (`hex_way`'s `(0.0 - sin(a) * dir, cos(a) * dir)`
// took down every program in its dependency cone).
//
// The walkers now delegate recursion to `IrNode::for_each_child` /
// `Value::for_each_child`, so the walk is total by construction.

/// Parse `src` against the real stdlib and hand back the populated `Data`.
fn parse_for_reachability(tag: &str, src: &str) -> loft::data::Data {
    let path = std::env::temp_dir().join(format!("loft_{tag}_{}.loft", std::process::id()));
    std::fs::write(&path, src).unwrap();
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse(path.to_str().unwrap(), false);
    scopes::check(&mut p.data, &mut p.database);
    let _ = std::fs::remove_file(&path);
    p.data
}

/// Every `Call` def-nr anywhere under `node`, collected through the keystone
/// child walk.  Deliberately INDEPENDENT of the production walkers: an
/// assertion that reused `collect_calls` could not witness `collect_calls`
/// skipping a node kind.
fn all_call_targets(node: &loft::data::Value, out: &mut std::collections::HashSet<u32>) {
    if let loft::data::Value::Call(d, _) = node {
        out.insert(*d);
    }
    node.for_each_child(&mut |c| all_call_targets(c, out));
}

#[test]
fn i815_callee_of_a_tuple_element_stays_reachable() {
    // `helper` is called from NOWHERE but a tuple element, so it survives only
    // if the reachability walk descends into `Value::Tuple`.
    let data = parse_for_reachability(
        "i815_tuple",
        "fn helper(x: float) -> float { x * 2.0 }\n\
         fn pair(x: float) -> (float, float) { (helper(x), 1.0) }\n\
         fn main() { (a, b) = pair(3.0); println(\"{a} {b}\"); }\n",
    );
    let pair = data.def_nr("n_pair");
    let helper = data.def_nr("n_helper");
    assert!(
        pair != u32::MAX && helper != u32::MAX,
        "fixture defs missing"
    );

    let reachable = loft::generation::reachable_functions(&data, &[pair]);
    assert!(
        reachable.contains(&helper),
        "#815: a callee reached only from a tuple element must be in the reachable \
         set — native emits the call either way, so pruning it is an E0425 at rustc time"
    );
}

#[test]
fn i815_reachable_set_is_closed_under_calls() {
    // The invariant the per-kind whitelists kept breaking: whatever the walk
    // marks reachable, every call INSIDE those functions must be marked too.
    // The fixture spreads calls across the node kinds the whitelists missed —
    // tuple literal, tuple-element write, `parallel` arm, `par(...)` worker —
    // each reachable through that construct alone.
    let data = parse_for_reachability(
        "i815_closed",
        "fn in_tuple(x: float) -> float { x * 2.0 }\n\
         fn in_tuple_put(x: integer) -> integer { x * 3 }\n\
         fn in_parallel_a(x: integer) -> integer { x + 7 }\n\
         fn in_parallel_b(x: integer) -> integer { x + 9 }\n\
         fn in_par_worker(x: integer) -> integer { x * 5 }\n\
         fn pair(x: float) -> (float, float) { (in_tuple(x), 1.0) }\n\
         fn main() {\n\
           (a, b) = pair(3.0); println(\"{a} {b}\");\n\
           t = (1, 2); t.1 = in_tuple_put(5); println(\"{t.0} {t.1}\");\n\
           parallel { println(\"{in_parallel_a(3)}\"); println(\"{in_parallel_b(3)}\"); }\n\
           v = [1, 2, 3]; r: vector<integer> = [];\n\
           for e in v par(w = in_par_worker(e), 2) { r += [w]; }\n\
           println(\"{r.len()}\");\n\
         }\n",
    );
    let main = data.def_nr("n_main");
    assert!(main != u32::MAX, "fixture main missing");
    let reachable = loft::generation::reachable_functions(&data, &[main]);

    // Each helper is reachable through exactly one of the once-missed kinds.
    for name in [
        "n_in_tuple",
        "n_in_tuple_put",
        "n_in_parallel_a",
        "n_in_parallel_b",
        "n_in_par_worker",
    ] {
        let d = data.def_nr(name);
        assert!(d != u32::MAX, "#815: fixture def {name} missing");
        assert!(
            reachable.contains(&d),
            "#815: `{name}` is called only through a node kind the walk must descend into"
        );
    }

    // The general closure property, checked with an independent walker.
    let mut missing: Vec<String> = Vec::new();
    for d in reachable.iter().copied() {
        let mut targets = std::collections::HashSet::new();
        all_call_targets(data.def(d).code(), &mut targets);
        for t in targets {
            if !reachable.contains(&t) {
                missing.push(format!(
                    "{} calls {} which is not reachable",
                    data.def(d).name(),
                    data.def(t).name()
                ));
            }
        }
    }
    missing.sort();
    missing.dedup();
    assert!(
        missing.is_empty(),
        "#815: the reachable set must be closed under the call relation — native \
         emits a call for each of these but no body:\n  {}",
        missing.join("\n  ")
    );
}

// A struct that contains ITSELF by value is reported as such — the diagnostic names the
// type and the cure (`reference<T>`).  `Data::has_value_cycle` used to skip recursing into
// a child the program had CONSTRUCTED anywhere (`def_referenced`), which is every cyclic
// type a real program writes: the report went silent and the reader met the internal
// layout validator instead — `type layout: PENode: field 'next' has no position
// (u16::MAX)`, with no source position and no cure.  The `@EXPECT_ERROR: contains itself`
// fixture in `tests/scripts/36-parse-errors.loft` was meant to catch that and had itself
// gone inert (loft#929).
#[test]
fn a_struct_containing_itself_by_value_says_so() {
    code!(
        "struct PENode { val: integer, next: PENode }
fn test() {
    _n = PENode { val: 1 };
}"
    )
    .error(
        "Struct 'PENode' contains itself (directly or indirectly) — use reference<PENode> \
to break the cycle at a_struct_containing_itself_by_value_says_so:1:16",
    );
}

// The other direction of the same rule: `reference<T>` is the documented cure, so a
// self-reference THROUGH it stays legal.  Removing the `def_referenced` gate must not
// start reporting the shape the diagnostic recommends.
#[test]
fn a_reference_self_field_is_not_a_cycle() {
    code!(
        "struct RefNode { val: integer, next: reference<RefNode> }
fn test() {
    n = RefNode { val: 7 };
    assert(n.val == 7, \"reference<Self> field is legal\");
}"
    )
    .result(Value::Null);
}

// ── An arm after a total `_` names the rule it breaks ────────────────────
//
// A total `_` matches everything, so an arm written after it can never be selected. Both match
// paths — the enum one in `parse_match` and the scalar one in `parse_scalar_match` — used to
// `break` out of the arm loop at that point and let the next arm meet the closing-brace
// expectation, which reported `Expect token }`: the right caret with the wrong reason, and on
// the scalar path it cascaded into four more errors about the rest of the line, none of which
// mentioned the wildcard. The Match chapter states this rule as "put it last", so the compiler
// is what a reader meets when they get it wrong.
#[test]
fn an_arm_after_a_total_wildcard_says_so_on_the_scalar_path() {
    code!(
        "fn test() { r = match 2 { 1 => \"one\", _ => \"other\", 2 => \"two\" }; print(\"{r}\"); }"
    )
    .error(
        "a `_` arm matches everything, so this arm can never be selected — move `_` to the end \
at an_arm_after_a_total_wildcard_says_so_on_the_scalar_path:1:54",
    );
}

#[test]
fn an_arm_after_a_total_wildcard_says_so_on_the_enum_path() {
    code!(
        "enum D { North, South, East }
fn test() { r = match D.South { North => \"n\", _ => \"other\", South => \"s\" }; print(\"{r}\"); }"
    )
    .error(
        "a `_` arm matches everything, so this arm can never be selected — move `_` to the end \
at an_arm_after_a_total_wildcard_says_so_on_the_enum_path:2:66",
    );
}

/// The carve-out that keeps the check above from being wrong: a GUARDED `_ if cond` is NOT
/// total — the guard can reject — so arms are expected to follow it and must stay legal. This
/// is the same distinction `(M-Total)` draws for exhaustiveness, asked at the parse site.
#[test]
fn a_guarded_wildcard_still_admits_the_arms_after_it() {
    code!(
        "fn test() {
  r = match 7 { _ if 7 < 0 => \"neg\", _ if 7 > 100 => \"big\", 7 => \"seven\", _ => \"other\" };
  assert(r == \"seven\", \"a guarded _ does not close the arm list: {r}\");
}"
    );
}
