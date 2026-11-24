// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The language inconsistencies (INC#), operator and quality cases, and the vector, struct and
//! native regressions next to them.

use super::*;

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
    // At the `{` the statement is stuck on: the literal is abandoned and the lexer put back.
    // It read 5:22 while a revert left the cursor where the abandoned read had stopped.
    .error("Expect token ; at inc30_typo_comma_without_colon_is_rejected:5:17");
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

// INC#18 — `x#break` is a labelled-break statement that reuses the
// `#attribute` syntax.  Documented in LOFT_CONTROL.md § Break and continue;
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
// in LOFT_DATA.md § Key-based collections (gotcha block).  These tests
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
// function.  Documented in LOFT_DATA.md § Methods and function calls
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
    .error("Unknown field vector<integer>.sum_of — did you mean the free function `sum_of(…)` ? (stdlib declared `sum_of` as free-only; see LOFT_DATA.md § Methods and function calls) at inc08_sum_of_is_free_function_only:3:14");
}

/// `text.starts_with(s)` is declared with `self: text` — method syntax
/// works, free-function syntax doesn't.  Pairs with
/// `inc08_sum_of_is_free_function_only` to show the asymmetry runs in
/// both directions per the stdlib declaration.
/// QUALITY-history.md 6d — writing a bare `hash<Row[id]>()` constructor
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
    .error("Unknown function nope_b at pln102_all_unknown_deferral_still_reports_undefined_callee:2:16");
    // The half-applied `OpMinInt` no longer adds its own two arity errors behind these: a
    // reported call is poisoned to `never`, and the operator skips a slot filled by one
    // (loft#1719), the tidy-up this test anticipated.
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

/// QUALITY-history.md 6c — the free-function hint must NOT fire when there is
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

/// QUALITY-history.md 6c follow-on — the free→method direction.  `starts_with`
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
    .error("Unknown function starts_with — did you mean the method `x.starts_with(…)` on text? (stdlib declared `starts_with` as a method; see LOFT_DATA.md § Methods and function calls) at quality_6c_free_call_on_wrong_type_suggests_method:2:5");
}

/// QUALITY-history.md 6c follow-on — methods declared on several receiver types
/// (`is_numeric` lives on both `text` and `character`) enumerate all
/// candidates so the user can pick the right one.
#[test]
fn quality_6c_free_call_lists_all_method_receivers() {
    code!(
        "fn run() -> boolean {
    is_numeric(5)
}"
    )
    .error("Unknown function is_numeric — did you mean the method `x.is_numeric(…)` on text / character? (stdlib declared `is_numeric` as a method; see LOFT_DATA.md § Methods and function calls) at quality_6c_free_call_lists_all_method_receivers:2:5");
}

/// QUALITY-history.md 6c follow-on — the hint must stay silent when no method
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

/// `len` is declared `self: vector` — it works equally as method
/// (`v.len()`) and as free function (`len(v)`).  Guards the `self`
/// half of the INC#8 story (C123 deprecated `both`): when an author picks `self`, the
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
        let mut f = fa::create(&rs_path).unwrap();
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
    let source = fa::read_to_string(&rs_path).unwrap();
    // The call to box_ensure should pass var_b directly, not *var_b.
    // P199 ABI change: native fns take `cell` (`&UnsafeCell<Stores>`),
    // not `stores` (`&mut Stores`).
    // `@FR-R-Header`'s kept clause may call the `__kh` twin, which takes the held header
    // after the ordinary arguments: the parameter is forwarded the same way in both.
    assert!(
        !source.contains("n_box_ensure(cell, *var_b)")
            && !source.contains("n_box_ensure__kh(cell, *var_b,"),
        "P144 regression: native codegen still emits *var_b for & param forward.\nGenerated: {}",
        rs_path.display()
    );
    assert!(
        source.contains("n_box_ensure(cell, var_b)")
            || source.contains("n_box_ensure__kh(cell, var_b,"),
        "P144 regression: expected direct var_b pass-through for & param.\nGenerated: {}",
        rs_path.display()
    );
    let _ = fa::remove_file(&rs_path);
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
    fa::write(
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
        let mut f = fa::create(&rs_path).unwrap();
        let main_nr = p.data.def_nr("n_main");
        let mut out = loft::generation::Output::new(&p.data, &state.database);
        out.output_native_reachable(&mut f, 0, p.data.definitions(), &[main_nr])
            .unwrap();
    }
    let source = fa::read_to_string(&rs_path).unwrap();
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
    let _ = fa::remove_file(&rs_path);
    let _ = fa::remove_file(&src_path);
}
