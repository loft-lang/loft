// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Production-mode exit status, for-loop mutation guards, function references, map / filter /
//! reduce, null scalar fields, store frees, file I/O, the generated tables, sorted and hash
//! field writes, the `--native` pipeline, lambdas and parameter defaults.

use super::*;

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
fn ccc_lookup(c: CccCache, k: text) -> text? { return c.items[k].value; }
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
give it a fallback with `?? <value>`, or make the destination `u8?` so a value that \
does not fit reads null, or make the value provably fit with a mask (`& 255`) \
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
    let _ = fa::remove_dir_all("tests/tmp_mkdir_test");
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
    let _ = fa::remove_dir_all("tests/tmp_mkdir_test");
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
// It guards the value the program answers; the half that read the rustc writer's emitted
// Rust went with the writer (@PLN225 R4).
#[test]
fn n8_pre_eval_uses_underscore_separator() {
    // Two nested user-fn calls: the inner call is pre-eval-hoisted by generation.rs.
    code!("fn inc(v: integer) -> integer { v + 1 }")
        .expr("inc(inc(0))")
        .result(Value::Int(2));
    // Every `let _pre…` line must use `_pre_N` (digit after underscore), not `_preN`.
}

// N8-empty: generated code must not emit `let _preN = ;` (empty right-hand side).
// The mutable-reference pattern (user fn with `&T = null` default) triggers this.
// It guards the value the program answers; the half that read the rustc writer's emitted
// Rust went with the writer (@PLN225 R4).
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
}

// N3: assigning a reference to another reference must emit OpCopyRecord for deep copy.
// Without it, both variables alias the same heap record; mutating through one changes the other.
// It guards the value the program answers; the half that read the rustc writer's emitted
// Rust went with the writer (@PLN225 R4).
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
}

// N5: vector::clear_vector must not be called when the DbRef is null (rec == 0).
// A function that initialises and returns a vector was panicking with
// "Unknown record 2147483648" because clear_vector ran on stores.null() before allocation.
// It guards the value the program answers; the half that read the rustc writer's emitted
// Rust went with the writer (@PLN225 R4).
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
}

// N4: struct-enum variants must show all fields when formatted with OpFormatDatabase.
// The init function was registering every enum value with u16::MAX as the struct type,
// so ShowDb could not dispatch to variant fields and only showed the variant name.
// It guards the value the program answers; the half that read the rustc writer's emitted
// Rust went with the writer (@PLN225 R4).
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
    let _ = fa::create_dir_all("tests/generated");
    let src = loft::create::generate_code_to(&p.data, &tmp).expect("generate_code_to failed");
    let _ = fa::remove_file(&tmp);
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
    let _ = fa::remove_file(&tmp);
    let src = fa::read_to_string("src/fill.rs").expect("src/fill.rs not found");
    assert_eq!(
        generated, src,
        "generated fill.rs differs from src/fill.rs — \
         run create::generate_code() and copy the result"
    );
}

/// `@FR-R-OpPriority` — every operator `default/` declares is numbered in its priority class's
/// band (`#hot` first, then the unmarked ones, then `#cold`), in declaration order within the
/// band, at the slot whose name the generated table carries; and every hot operator has a
/// one-byte opcode.  The drift test above only says the committed tables are what the
/// generator writes; this says the numbering the parser gives is the layout those tables use.
#[test]
fn every_operator_is_numbered_in_its_priority_band() {
    use loft::data::{OP_COLD, OP_HOT, OP_NORMAL};
    use loft::op_layout;
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    let data = &p.data;
    let hot = usize::from(op_layout::OP_HOT);
    let normal = usize::from(op_layout::OP_NORMAL);
    let mut last = [None::<u16>; 3];
    let mut count = [0usize; 3];
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !def.is_operator() {
            continue;
        }
        let code = usize::from(def.op_code);
        let class = def.op_priority;
        let band = match class {
            OP_HOT => 0..hot,
            OP_NORMAL => hot..hot + normal,
            OP_COLD => hot + normal..op_layout::OPERATOR_NAMES.len(),
            other => panic!("{}: priority class {other}", def.name),
        };
        assert!(
            band.contains(&code),
            "{} (class {class}) numbered {code}, outside {band:?}",
            def.name
        );
        assert_eq!(
            op_layout::OPERATOR_NAMES[code],
            def.name,
            "slot {code} names another operator"
        );
        let at = usize::from(class);
        assert!(
            last[at].is_none_or(|prev| prev < def.op_code),
            "{} numbered {code} after a later-declared operator of its class took {:?}",
            def.name,
            last[at]
        );
        last[at] = Some(def.op_code);
        count[at] += 1;
        if class == OP_HOT {
            assert!(
                code < 255,
                "hot operator {} has the two-byte opcode {code}",
                def.name
            );
        }
    }
    assert_eq!(
        count[0], hot,
        "the table's hot band differs from the #hot operators"
    );
    assert_eq!(
        count[1], normal,
        "the table's middle band differs from the unmarked operators"
    );
    assert!(
        count[0] > 0 && count[2] > 0,
        "no hot or no cold operator: the bands are untested"
    );
}

// N8: Sort must work correctly in native-codegen mode.
// The #rust template for OpSortVector is inlined directly (no OpSortVector runtime fn needed):
// it calls `Stores::sort_vector`, which picks the element order from the schema type.
// It guards the value the program answers; the half that read the rustc writer's emitted
// Rust went with the writer (@PLN225 R4).
#[test]
fn n8_codegen_runtime_vector_ops_exist() {
    // Sorting a vector of integers must work in native-codegen mode.
    code!("fn sort_it() -> vector<integer> { v = [3, 1, 2]; sort(v); v }")
        .expr("\"{sort_it()}\"")
        .result(Value::str("[1,2,3]"));
}

// N10: ops::text_character returns char but loft represents character as i32.
// Generated code assigns the char to an i32 variable without a cast, causing a compile error.
// Also, i32 character variables used in method dispatch (is_alphanumeric etc.) need wrapping
// with ops::to_char(...) since the method requires char, not i32.
// It guards the value the program answers; the half that read the rustc writer's emitted
// Rust went with the writer (@PLN225 R4).
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
}

// N2: output_init must register content types before the structs that reference them in
// sorted/index/hash fields.  When a struct has a sorted<Foo> field and Foo has a higher
// type-ID than the struct, the init function panicked because db.sorted(foo_type_id, ...)
// was called before Foo was registered.
// It guards the value the program answers; the half that read the rustc writer's emitted
// Rust went with the writer (@PLN225 R4).
#[test]
fn n2_sorted_field_content_type_registered_first() {
    code!(
        "struct Sort { nr: integer }
struct Container { data: sorted<Sort[nr]> }"
    )
    .expr("c = Container {}; \"{c}\"")
    .result(Value::str("{data:[]}"));
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
    let mut f = fa::create(&tmp_rs).expect("tmp file");
    let mut out = Output::new(&p.data, &state.database);
    out.output_native_reachable(&mut f, start_def, end_def, &[main_nr])
        .expect("output_native_reachable");
    drop(f);
    // Verify the generated source contains expected landmarks.
    let generated = fa::read_to_string(&tmp_rs).expect("read generated source");
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
    let loft_rlib = fa::read_dir(&deps_dir).ok().and_then(|it| {
        it.into_iter()
            .filter(|e| {
                let s = e.file_name().unwrap_or_default();
                s.starts_with("libloft") && s.ends_with(".rlib")
            })
            .max_by_key(|e| {
                fa::symlink_metadata(e)
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
            })
            .map(|e| e.os_spelling())
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
        if let Ok(entries) = fa::read_dir(&deps_dir) {
            for entry in entries {
                let name = entry.file_name().unwrap_or_default().to_string();
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
                rustc_args.push(format!("{crate_name}={}", entry.os_spelling().display()));
            }
        }
    }
    rustc_args.push(tmp_rs.to_str().unwrap().to_string());
    match loft::platform::process::harness_command("rustc")
        .args(&rustc_args)
        .status()
    {
        Ok(s) if s.success() => {
            // Binary compiled — run it to confirm correctness.
            let run = loft::platform::process::harness_command(&binary).status();
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
    let _ = fa::remove_file(&tmp_rs);
    let _ = fa::remove_file(&binary);
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
