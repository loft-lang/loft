// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Hash iteration, const fields and variants, loop write-through, hash value fields, short-form
//! lambdas, text-returning natives, characters, and function references and tuples as struct
//! fields, including capturing closures.

use super::*;

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
    );
    // And nothing follows it.  There were two recovery lines: *"Need an iterable expression
    // in a for statement"* (the `for`'s fallback, which now speaks only when nothing else
    // did) and *"Expect token ;"* (the `for` bailed without consuming its body, and its
    // `Null` code asked for a `;` a loop never needs).  The harness matches the diagnostics
    // EXACTLY, so the refusal standing alone is the assertion (loft#1453).
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

/// @FR-Const-VirtualReject (@PLN40): `const virtual(…)` is rejected — a virtual field is already computed
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
        "non-null type `integer` — it becomes null there; an index loft cannot prove in range ",
        "may be past the end: index with a shape it proves (`v[-1]`, the loop's own element, ",
        "`v[x#index]`, an index tested against `len(v)`), or write `!` after the read when the ",
        "program guarantees the element at issue_84_merge_sort_too_few_parameters:8:34"
    ))
    .warning(concat!(
        "a nullable `integer?` is stored into element 0 of this vector literal of the ",
        "non-null type `integer` — it becomes null there; an index loft cannot prove in range ",
        "may be past the end: index with a shape it proves (`v[-1]`, the loop's own element, ",
        "`v[x#index]`, an index tested against `len(v)`), or write `!` after the read when the ",
        "program guarantees the element at issue_84_merge_sort_too_few_parameters:9:39"
    ))
    .warning(concat!(
        "a nullable `integer?` is stored into element 0 of this vector literal of the ",
        "non-null type `integer` — it becomes null there; an index loft cannot prove in range ",
        "may be past the end: index with a shape it proves (`v[-1]`, the loop's own element, ",
        "`v[x#index]`, an index tested against `len(v)`), or write `!` after the read when the ",
        "program guarantees the element at issue_84_merge_sort_too_few_parameters:10:47"
    ))
    .warning(concat!(
        "a nullable `integer?` is stored into element 0 of this vector literal of the ",
        "non-null type `integer` — it becomes null there; an index loft cannot prove in range ",
        "may be past the end: index with a shape it proves (`v[-1]`, the loop's own element, ",
        "`v[x#index]`, an index tested against `len(v)`), or write `!` after the read when the ",
        "program guarantees the element at issue_84_merge_sort_too_few_parameters:11:27"
    ))
    .warning(concat!(
        "a nullable `integer?` is stored into element 0 of this vector literal of the ",
        "non-null type `integer` — it becomes null there; an index loft cannot prove in range ",
        "may be past the end: index with a shape it proves (`v[-1]`, the loop's own element, ",
        "`v[x#index]`, an index tested against `len(v)`), or write `!` after the read when the ",
        "program guarantees the element at issue_84_merge_sort_too_few_parameters:19:35"
    ))
    .result(Value::Null);
}

// N7: OpFormatFloat must generate ops::format_float(...), not OpFormatFloat(stores, ...).
// OpFormatStackLong must generate ops::format_long(var_, ...) without stores or &mut.
// It guards the value the program answers; the half that read the rustc writer's emitted
// Rust went with the writer (@PLN225 R4).
#[test]
fn n7_format_ops_generate_correct_rust() {
    // Float formatting
    code!("struct Flt { v: float }")
        .expr("f = Flt { v: 3.14 }; \"{f.v}\"")
        .result(Value::str("3.14"));
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
    let generated = loft::create::generate_code_to(
        &p.data,
        &std::env::temp_dir()
            .join(format!("loft_fill_check_{}.rs", std::process::id()))
            .to_string_lossy(),
    )
    .expect("generate_code_to failed");
    let current = fa::read_to_string("src/fill.rs").expect("cannot read src/fill.rs");
    assert_eq!(
        current, generated,
        "src/fill.rs is out of date — run: cargo test regen_fill_rs -- --ignored --nocapture"
    );
}

// Assert that src/op_layout.rs — the slot names and class sizes fill.rs re-exports — matches
// what generate_layout_to would produce.  If this fails, run the same regen_fill_rs.
#[test]
fn op_layout_rs_up_to_date() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    scopes::check(&mut p.data, &mut p.database);
    let generated = loft::create::generate_layout_to(
        &p.data,
        &std::env::temp_dir()
            .join(format!("loft_op_layout_check_{}.rs", std::process::id()))
            .to_string_lossy(),
    )
    .expect("generate_layout_to failed");
    let current = fa::read_to_string("src/op_layout.rs").expect("cannot read src/op_layout.rs");
    assert_eq!(
        current, generated,
        "src/op_layout.rs is out of date — run: cargo test regen_fill_rs -- --ignored --nocapture"
    );
}

// @I67 — opcode implementations (this generates the @generated src/fill.rs from the default `#rust` bodies)
// Regenerate src/fill.rs from the default library definitions.
// Run with: cargo test regen_fill_rs -- --ignored --nocapture
#[test]
#[ignore = "maintenance: regenerates src/fill.rs — run manually when default/*.loft changes"]
fn regen_fill_rs() {
    let mut p = Parser::new();
    // Unchecked: the stdlib check compares `default/` against the table this run REGENERATES,
    // so after an operator is added or removed it would refuse the repair it asks for.
    p.parse_dir_unchecked("default", true, false).unwrap();
    scopes::check(&mut p.data, &mut p.database);
    loft::create::generate_code_to(&p.data, "src/fill.rs").expect("generate_code_to failed");
    loft::create::generate_layout_to(&p.data, "src/op_layout.rs")
        .expect("generate_layout_to failed");
    println!("src/fill.rs and src/op_layout.rs regenerated");
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
    let native_src = fa::read_to_string("src/native.rs").expect("cannot read src/native.rs");
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

// S10 (@C62): `|x: integer|` must produce a compile-time error.
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
    .warning("Variable h is never read at p4d_fn_ref_field_bare_default:2:16")
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
// host's Store.  `OpChildRec` builds it in place; `OpRefFromChildRec` reads;
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
