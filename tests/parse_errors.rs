// Copyright (c) 2022-2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

extern crate loft;

use loft::data::Value;

mod testing;

/// @PLN102 arc-E test-hygiene LOCK — the `code!` harness has NO silent-tolerance
/// filter: a fixture that emits a warning it does not `.warning(..)`-assert MUST fail.
/// This guards against re-introducing an `is_runtime_warning`-style tolerance (the one
/// this pass deleted). The probe emits a redundant-`&` warning and asserts nothing, so
/// evaluating it (on `Test` drop) must panic.
#[test]
fn harness_rejects_an_unasserted_warning() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {})); // silence the EXPECTED panic
    let outcome = std::panic::catch_unwind(|| {
        // `&S` param mutated by a field write → a redundant-`&` warning; asserting
        // nothing must be a failure now that the tolerance filter is gone.
        let _t = testing::testing_code(
            "struct S { x: integer } fn f(o: &S) { o.x = 1; } fn test() { s = S { x: 0 }; f(s); }",
            "harness_rejects_an_unasserted_warning_probe",
        );
    });
    std::panic::set_hook(prev);
    assert!(
        outcome.is_err(),
        "the harness must FAIL on an unasserted warning — silent tolerance is gone"
    );
}

#[test]
fn wrong_parameter() {
    code!("fn def(i: integer) { }\nfn test() { def(true); }")
        .error("expected integer, got boolean on argument 1 of call to def at wrong_parameter:2:17")
        .warning("Parameter i is never read at wrong_parameter:1:21");
}

#[test]
fn wrong_boolean() {
    code!("enum EType{ Val }\nfn def(t: EType) {}\nfn test() { def(true); }")
        .error("expected EType, got boolean on argument 1 of call to def at wrong_boolean:3:17")
        .warning("Parameter t is never read at wrong_boolean:2:19");
}

#[test]
fn default_arg_type_mismatch() {
    // @PLN102 arc-E E2 Tier-0: a wrong-typed default (`text = 42`) used to reach
    // runtime and SIGSEGV the interpreter (the int used as a text pointer). It
    // must be rejected at DEFINITION, exactly like a call-site argument mismatch.
    code!("fn f(x: text = 42) { print(\"{x}\"); }\nfn test() { f(); }")
        .error("expected text, got integer on default value at default_arg_type_mismatch:1:18");
}

// @PLN101 — a `value struct` is stored inline (no `store_nr` null sentinel), so `<value struct>?`
// has no representation and is rejected.
#[test]
fn value_struct_no_nullable() {
    code!("value struct P { x: integer }\nfn test() { q: P? = P { x: 1 }; }")
        .error("`P?` is not allowed — a `value struct` is stored inline and has no null; use a plain `P`, or a reference `struct` for nullability at value_struct_no_nullable:2:20");
}

#[test]
fn unknown_var() {
    code!("fn test() { a == 1 }").error("Unknown variable 'a' at unknown_var:1:13");
}

// A typed declaration that CONSTRUCTS a struct at module scope (`CFG p = P { … };`)
// used to ICE the parser: file scope has no function var table, so the construction
// destination is the `u16::MAX` "no slot" sentinel — which `Function::is_independent`
// indexed out of bounds and `parse_object_field`'s `depending()` asserted on. The
// malformed declaration must yield an ordinary diagnostic, never a panic. (Surfaced
// while probing @PLN116.)
#[test]
fn module_scope_struct_ctor_no_ice() {
    code!("struct P { x: integer }\nCFG p = P { x: 1 };\nfn test() { }")
        .error("Expect token = at module_scope_struct_ctor_no_ice:2:6");
}

/// S1: a misspelled variable name must produce a clear "Unknown variable" diagnostic
/// on the second pass without creating a ghost variable that could cause cascading errors.
#[test]
fn typo_var_name() {
    code!("fn test() { count = 0; cound + 1; }")
        .error("Unknown variable 'cound' — did you mean 'count'? at typo_var_name:1:24")
        .warning("Variable count is never read at typo_var_name:1:20");
}

// ── Plan-07 phase 5 — suggestion paths ──────────────────────────────────────

/// Function-name typo (Levenshtein 1) appends `did you mean` suffix.
/// Wires `Parser::suggest_function_name` at `mod.rs::call`'s
/// "Unknown function" diagnostic.
#[test]
fn p07_suggest_unknown_function() {
    code!(
        "fn double(x: integer) -> integer { x + x }
fn test() { doublet(5); }"
    )
    .error(
        "Unknown function doublet — did you mean 'double'? at p07_suggest_unknown_function:2:13",
    );
}

/// Type-name typo (Levenshtein 1) appends `did you mean` suffix.
/// Wires `Data::suggest_type_name` at `parser/mod.rs`'s deferred
/// "Undefined type" emitter.
#[test]
fn p07_suggest_undefined_type() {
    code!(
        "struct Counter { n: integer }
fn build() -> Conter { Counter { n: 0 } }
fn test() {}"
    )
    .error("Undefined type Conter — did you mean 'Counter'? at p07_suggest_undefined_type:2:23");
}

// ── T0.3 — cross-language type-name habits ───────────────────────
//
// A newcomer's first hour is full of `int` / `str` / `bool`.  Edit distance can
// reach NONE of these: `suggest_similar_capped` returns None at <= 3 characters
// (`int`, `str`, `i64`, `f64`) and caps distance at 2, while `bool`->`boolean` is
// 3, `char`->`character` is 5 and `string`->`text` is unrelated.  So the alias
// table in `data::builtin_type_alias` is the whole mechanism for this class.
//
// SUGGESTION ONLY — these names stay undefined.  Making them legal is a language
// question, declined in DESIGN_DECISIONS.md: loft's full-word type names are
// deliberate friction, and since inference means a type is rarely written at all,
// a cheap `int` would invite exactly the pointless annotations the language is
// shaped to discourage.

/// Every row of the cross-language alias table suggests the right loft type.
#[test]
fn t03_cross_language_type_aliases_suggest() {
    code!(
        "fn a(x: int) -> integer { 1 }
fn b(x: i64) -> integer { 1 }
fn c(x: u64) -> integer { 1 }
fn d(x: long) -> integer { 1 }
fn e(x: f64) -> integer { 1 }
fn g(x: double) -> integer { 1 }
fn h(x: f32) -> integer { 1 }
fn i(x: str) -> integer { 1 }
fn j(x: string) -> integer { 1 }
fn k(x: bool) -> integer { 1 }
fn l(x: char) -> integer { 1 }
fn test() {}"
    )
    .error("Undefined type int — did you mean 'integer'? at t03_cross_language_type_aliases_suggest:1:13")
    .error("Undefined type i64 — did you mean 'integer'? at t03_cross_language_type_aliases_suggest:2:13")
    .error("Undefined type u64 — did you mean 'integer'? at t03_cross_language_type_aliases_suggest:3:13")
    .error("Undefined type long — did you mean 'integer'? at t03_cross_language_type_aliases_suggest:4:14")
    .error("Undefined type f64 — did you mean 'float'? at t03_cross_language_type_aliases_suggest:5:13")
    .error("Undefined type double — did you mean 'float'? at t03_cross_language_type_aliases_suggest:6:16")
    .error("Undefined type f32 — did you mean 'single'? at t03_cross_language_type_aliases_suggest:7:13")
    .error("Undefined type str — did you mean 'text'? at t03_cross_language_type_aliases_suggest:8:13")
    .error("Undefined type string — did you mean 'text'? at t03_cross_language_type_aliases_suggest:9:16")
    .error("Undefined type bool — did you mean 'boolean'? at t03_cross_language_type_aliases_suggest:10:14")
    .error("Undefined type char — did you mean 'character'? at t03_cross_language_type_aliases_suggest:11:14");
}

/// The alias table did not cost the edit-distance path: a mistyped BUILTIN now
/// suggests too.  Before T0.3 `suggest_type_name` ranked only user structs/enums,
/// so a typo'd builtin had no candidates at all and suggested nothing.
#[test]
fn t03_mistyped_builtin_suggests() {
    code!(
        "fn a(x: intger) -> integer { 1 }
fn b(x: bolean) -> integer { 1 }
fn c(x: charater) -> integer { 1 }
fn test() {}"
    )
    .error("Undefined type intger — did you mean 'integer'? at t03_mistyped_builtin_suggests:1:16")
    .error("Undefined type bolean — did you mean 'boolean'? at t03_mistyped_builtin_suggests:2:16")
    .error(
        "Undefined type charater — did you mean 'character'? at t03_mistyped_builtin_suggests:3:18",
    );
}

/// An unrelated name still gets NO suggestion — the table must not make the
/// suggester fire on anything that merely looks unfamiliar.
#[test]
fn t03_unrelated_type_gets_no_suggestion() {
    code!("fn a(x: Nonsuch) -> integer { 1 }\nfn test() {}")
        .error("Undefined type Nonsuch at t03_unrelated_type_gets_no_suggestion:1:17");
}

// ── Plan-07 phase 5 anti-suggestion tests ────────────────────────
// Locks in the rule that suggestions DON'T fire when the candidate
// would be misleading.  Pre-fix the variable-suggestion site used
// the uncapped `suggest_similar` (distance ≤ 2) which over-matched
// 1-char names ("did you mean 'x'?" for typo `y`).  Phase-5 fix:
// skip suggestions for ≤1-char names where typos are too ambiguous
// to be meaningful.  Distance/scope filters cover the other cases.

/// Single-letter typo must NOT suggest the other 1-char name —
/// `x` vs `y` is a coin flip; the suggestion would be noise.
#[test]
fn p07_no_suggest_single_letter_typo() {
    code!("fn test() { x = 5; y == 1 }")
        .error("Unknown variable 'y' at p07_no_suggest_single_letter_typo:1:20")
        .warning("Variable x is never read at p07_no_suggest_single_letter_typo:1:16");
}

/// Distant name (`printbar` vs `foo`) must NOT suggest — Levenshtein
/// distance > 2 falls outside `suggest_similar`'s ceiling.
#[test]
fn p07_no_suggest_distant_name() {
    code!("fn test() { foo = 5; printbar == 1 }")
        .error("Unknown variable 'printbar' at p07_no_suggest_distant_name:1:22")
        .warning("Variable foo is never read at p07_no_suggest_distant_name:1:18");
}

/// Variable in a sibling fn must NOT be suggested — the candidate
/// set is function-scoped (`self.vars.iter()` only sees the current
/// fn's locals + arguments).  `cousin` defined in `other()` does not
/// leak into `test()`'s suggestion candidates for typo `cousn`.
#[test]
fn p07_no_suggest_sibling_fn_scope() {
    code!("fn other() { cousin = 99; }\nfn test() { cousn == 1 }")
        .warning("Variable cousin is never read at p07_no_suggest_sibling_fn_scope:1:22")
        .error("Unknown variable 'cousn' at p07_no_suggest_sibling_fn_scope:2:13");
}

// Field-suggestion paths (struct-literal + field-access) and the
// 1-char cap behaviour are end-to-end-validated by
// `quality_6c_unknown_field_without_free_fn_has_no_hint` in
// `tests/issues.rs` (the existing assertion of plain "Unknown field
// Point.z" depends on the length-aware cap suppressing the single-
// char suggestion).  Adding dedicated `p07_suggest_*` tests for
// these cases is blocked by `Variable v has unknown type` cascades
// when an unknown-field expression is bound to a local; those
// cascades are part of the parser's existing error-recovery shape
// and would dominate the suggestion-specific assertion.

#[test]
fn use_before_define() {
    code!("fn test() { if a == 1 { panic(); }; a = 1; }")
        .error("Unknown variable 'a' at use_before_define:1:16");
}

#[test]
fn wrong_text() {
    code!("fn rout(a: integer) -> integer {if a > 4 {return \"a\"} 2}\nfn test() {}")
        .error("expected integer, got text on return at wrong_text:1:51");
}

#[test]
fn empty_return() {
    code!("fn routine(a: integer) -> integer {if a > 4 {return} 1}\nfn test() {}")
        .error("Expect expression after return at empty_return:1:53");
}

#[test]
fn wrong_void() {
    code!("fn rout(a: integer) {if a > 4 {return 12}}\nfn test() {}")
        .error("Expect no expression after return at wrong_void:1:42");
}

#[test]
fn wrong_break() {
    code!("fn test() {break}").error("Cannot break outside a loop at wrong_break:1:18");
}

#[test]
fn wrong_continue() {
    code!("fn test() {continue}").error("Cannot continue outside a loop at wrong_continue:1:21");
}

#[test]
fn double_field_name() {
    code!("fn test(a: integer, b: integer, a: integer) { if a>b {} }")
        .error("Double attribute 'test.a' at double_field_name:1:35");
}

#[test]
fn incorrect_name() {
    code!("type something;\nfn something(a: integer) {}")
        .error("Cannot redefine 'something' (already defined at incorrect_name:1:16) at incorrect_name:2:27")
        .error("Expect type definitions to be in camel case style at incorrect_name:1:16");
}

#[test]
fn wrong_compare() {
    code!("enum EType{ V1 }\nenum Next{ V2 }\nfn test() { EType.V1 == Next.V2; }")
        .error("No matching operator '==' on 'EType' and 'Next' at wrong_compare:3:32");
}

#[test]
fn wrong_plus() {
    code!("fn test() {(1 + \"a\")}")
        .error("No matching operator '+' on 'integer' and 'text' at wrong_plus:1:20");
}

// @PLN102 pre-freeze (E2 Tier-1) — `==`/`!=` between incompatible types used to
// resolve through the boolean TRUTHINESS fallback (both operands coerced to "is
// truthy"), so `5 == "banana"` was `true == true` = **true**.  Reject them at
// compile time — the same "No matching operator" the ordering operators (`<` …)
// already give.  Same-type and numeric (int↔float, int↔char) comparisons and
// `x == null` null-checks stay valid (covered by the script suite).
#[test]
fn cross_type_eq_int_text() {
    code!("fn test() { b = 5 == \"x\"; }")
        .error("No matching operator '==' on 'integer' and 'text' at cross_type_eq_int_text:1:25");
}

#[test]
fn cross_type_eq_bool_text() {
    code!("fn test() { b = true == \"x\"; }")
        .error("No matching operator '==' on 'boolean' and 'text' at cross_type_eq_bool_text:1:28");
}

#[test]
fn cross_type_ne_int_text() {
    code!("fn test() { b = 5 != \"x\"; }")
        .error("No matching operator '!=' on 'integer' and 'text' at cross_type_ne_int_text:1:25");
}

#[test]
fn cross_type_eq_bool_float() {
    code!("fn test() { b = true == 5.0; }").error(
        "No matching operator '==' on 'boolean' and 'float' at cross_type_eq_bool_float:1:28",
    );
}

// @PLN102 pre-freeze (E2 Tier-1) — an enum vs a raw integer coerced the enum to
// its INTERNAL +1-biased discriminant, so `Color.Green == 1` leaked the encoding
// and read a confusing `false` (Green's disc is 2).  Reject it like every other
// cross-type compare; `enum == enum` and `enum == null` are unaffected.
#[test]
fn cross_type_eq_enum_int() {
    code!("enum Color { Red, Green, Blue }\nfn test() { c = Color.Green; b = c == 1; }")
        .error("No matching operator '==' on 'Color' and 'integer' at cross_type_eq_enum_int:2:40");
}

#[test]
fn cross_type_ne_int_enum() {
    code!("enum Color { Red, Green, Blue }\nfn test() { c = Color.Green; b = 0 != c; }")
        .error("No matching operator '!=' on 'integer' and 'Color' at cross_type_ne_int_enum:2:40");
}

// @PLN102 — a value-enum vector has no wired per-element null slot, so a `null`
// element in a `vector<Color?>` literal stays rejected even though a nullable-enum
// VARIABLE's `= null` now converts to the typed null (parse_errors is the guard so
// the scalar fix doesn't silently enable the unwired vector form).
#[test]
fn null_element_in_value_enum_vector_rejected() {
    code!("enum Color { Red, Green, Blue }\nfn test() { v: vector<Color?> = [Color.Red, null]; }")
        .error("cannot store null elements in a vector<Color> (would lose precision); declare the element nullable (`vector<Color?>`), or cast each element explicitly with 'as Color' at null_element_in_value_enum_vector_rejected:2:50");
}

// @PLN102 arc-E E2 (B) — a STATICALLY out-of-range constant operation is a
// COMPILE error (owner ruling 2026-07-19), not a silent null.  The runtime
// still nulls a *dynamic* out-of-range value (C80/C85) and the `?? d` / checked
// `as τ?` escapes stay valid (see tests/scripts/pln102-const-out-of-range.loft);
// only the BARE constant the developer can SEE is wrong is rejected.  These
// diagnostics carry E1 codes (`[shift-amount-out-of-range]` /
// `[cast-constant-out-of-range]`); the harness strips the tag before matching,
// so the pinned prose is the improvable text, the code is the stable handle.
#[test]
fn pln102_const_shift_over_range() {
    code!("fn test() { x = 1 << 100; }")
        .error("shift by 100 is out of the valid range 0..=63 — a constant out-of-range shift has no defined result at pln102_const_shift_over_range:1:26")
        .warning("Variable x is never read at pln102_const_shift_over_range:1:16");
}

#[test]
fn pln102_const_shift_negative() {
    code!("fn test() { x = 1 << -1; }")
        .error("shift by -1 is out of the valid range 0..=63 — a constant out-of-range shift has no defined result at pln102_const_shift_negative:1:25")
        .warning("Variable x is never read at pln102_const_shift_negative:1:16");
}

#[test]
fn pln102_const_cast_over_range() {
    code!("fn test() { x = 1e30 as integer; }")
        .error("the constant 1000000000000000000000000000000 is out of range for `integer` — a bare cast asserts the value fits at pln102_const_cast_over_range:1:33")
        .warning("Variable x is never read at pln102_const_cast_over_range:1:16");
}

// @PLN102 arc-E — `τ? ?? d` where the default `d` is NOT assignable to `τ` was unsound:
// the interpreter reinterpreted one representation as the other (SIGSEGV for `ref? ?? int`,
// silent corruption for `int? ?? float` / `int? ?? text`), while `--native` rejected it at
// rustc (E0308) — a backend divergence. Now a clean compile error on BOTH backends
// (qq-type-mismatch-fix.md). Assignability is ONE-directional: `float? ?? int` (int widens)
// stays valid, exercised by the script suite.
#[test]
fn qq_coalesce_ref_default_mismatch() {
    code!("struct Row { v: integer } fn test() { o: Row? = null; b = o ?? -1; }")
        .error("`??` default of type `integer` is not assignable to `Row` — a default must be usable where the value's type is expected at qq_coalesce_ref_default_mismatch:1:67")
        .warning("Variable b is never read at qq_coalesce_ref_default_mismatch:1:58");
}

#[test]
fn qq_coalesce_numeric_default_mismatch() {
    code!("fn test() { n: integer? = null; b = n ?? 2.5; }")
        .error("`??` default of type `float` is not assignable to `integer` — a default must be usable where the value's type is expected at qq_coalesce_numeric_default_mismatch:1:46")
        .warning("Variable b is never read at qq_coalesce_numeric_default_mismatch:1:36");
}

#[test]
fn qq_coalesce_crosstype_default_mismatch() {
    code!("fn test() { n: integer? = null; b = n ?? \"x\"; }")
        .error("`??` default of type `text` is not assignable to `integer` — a default must be usable where the value's type is expected at qq_coalesce_crosstype_default_mismatch:1:46")
        .warning("Variable b is never read at qq_coalesce_crosstype_default_mismatch:1:36");
}

#[test]
fn wrong_if() {
    code!("fn test() {if 1 > 0 { 2 } else {\"a\"}\n}")
        .error("expected integer, got text on else at wrong_if:1:34");
}

#[test]
fn wrong_assign() {
    code!("enum EType { V1 }\nfn test() {a = 1; a = EType.V1 }")
        .error("Variable 'a' cannot change type from integer to EType; use a new variable name or cast with 'as' at wrong_assign:2:33");
}

#[test]
fn mixed_enums() {
    code!("enum E1 { V1 }\nenum E2 { V2 }\nfn a(v: E2) -> E2 { v }\nfn test() { a(E1.V1) }")
        .error("expected E2, got E1 on argument 1 of call to a at mixed_enums:4:15");
}

#[test]
fn wrong_cast() {
    code!("enum E1 { V1 }\nfn test() { E1.V1 as float }")
        .error("Unknown cast from E1 to float at wrong_cast:2:29");
}

#[test]
fn field_type() {
    code!("struct Rec { v: u8 }\nfn test() { r = Rec { v: \"a\" }; assert(\"{r}\" == \"{{v:\\\"a\\\"}}\", \"Object\"); }")
        .error("Cannot assign text to field Rec.v of type integer(0, 255) at field_type:2:31");
}

#[test]
fn key_field() {
    code!(
        "struct Rec { n: text, v: u16 }
struct Coll { d: vector<Rec>, h: hash<Rec[n]> }
fn test() {
  s = Coll { d:[Rec {n: \"a\", v:12} ] };
  s.d[0].v = 13;
  s.d[0].n = \"b\";
}"
    )
    .error("Cannot write to key field Rec.n create a record instead at key_field:6:18");
}

#[test]
fn undefined() {
    code!("fn test(v: V) -> V { v }").error("Undefined type V at undefined:1:14");
}

#[test]
fn undefined_return() {
    code!("fn test(v: integer) -> V { v }").error("Undefined type V at undefined_return:1:27");
}

#[test]
fn undefined_as() {
    code!("fn test(v: integer) -> integer { v as V }")
        .error("Undefined type V at undefined_as:1:42");
}

#[test]
fn undefined_enum() {
    // V2 is undefined here — used as a stand-in to trigger the
    // "unknown variable" diagnostic.  The post-P246 warning sweep
    // also flags V2 as UPPER_CASE-without-const because the parser
    // synthesised a placeholder local for it during recovery.
    code!("enum E1 { V1 }\nfn test(v: E1) -> boolean { v > V2 }")
        .error("Unknown variable 'V2' — did you mean 'v'? at undefined_enum:2:33")
        .advice(
            "Variable 'V2' is UPPER_CASE — that style is reserved for constants \
             at undefined_enum:2:37",
        );
}

#[test]
fn unknown_sizeof() {
    // Nearly the shape of undefined_enum — `C` is undeclared, so the parser synthesises a
    // placeholder local for it.  The difference is that `sizeof` errors out before the
    // placeholder reaches the body, so nothing in the emitted code names it, and the
    // UPPER_CASE sweep no longer speaks about it (loft#921): the advice says "this is a
    // local variable", and a name that never became one is exactly what it must not claim.
    // The two errors below are the whole diagnosis of this line.
    code!("fn test() { sizeof(C); }")
        .error("Expect a variable or type after sizeof at unknown_sizeof:1:22")
        .error("Unknown variable 'C' at unknown_sizeof:1:20");
}

#[test]
fn index_non_indexable() {
    code!("fn test() { v = 5; v[1]; }").error("Indexing a non vector — keyed collections (hash/sorted/index/spatial) have no generic-constructor expression; name the key via a type annotation and initialise from a vector literal: `h: hash<Row[id]> = [Row { id: 1 }];` (a struct field `struct Db { h: hash<Row[id]> }` works too) at index_non_indexable:1:23");
}

#[test]
fn fn_name_as_param_type() {
    code!("fn helper() {}\nfn test(v: helper) {}")
        .error("Undefined type helper at fn_name_as_param_type:2:19");
}

#[test]
fn fn_name_as_typedef() {
    code!("fn helper() {}\ntype Alias = helper;\nfn test() { 1 }")
        .error("Undefined type helper at fn_name_as_typedef:2:21");
}

#[test]
fn missing_variant_impl() {
    // area() is only defined for Circle; Rect has no area() — expect a warning at Rect's definition.
    code!(
        "enum Shape {\n    Circle { r: float },\n    Rect { w: float, h: float }\n}\nfn area(self: Circle) -> float { self.r * self.r }\nfn test() { 1 + 1; }"
    )
    .warning("no implementation of 'area' for variant 'Rect' at missing_variant_impl:3:11");
}

#[test]
fn nullable_receiver_implements_its_variant() {
    // loft#1427 — a `self: Sq?` receiver IS an implementation of `Sq` (`@FR-F-Recv`), so the
    // warning must name only the variant that has none.  Asked bare, the scan reported `Sq`
    // too — a warning for an implementation written three lines above it — and this harness
    // fails on any warning the fixture does not assert, so the extra one goes red here.
    code!(
        "enum Sh {\n    Ci { r: integer },\n    Sq { s: integer },\n    Tr { t: integer }\n}\nfn ar(self: Ci) -> integer { self.r }\nfn ar(self: Sq?) -> integer { if self == null { 0 } else { self.s } }\nfn test() { 1 + 1; }"
    )
    .warning("no implementation of 'ar' for variant 'Tr' at nullable_receiver_implements_its_variant:4:9");
}

#[test]
fn stub_suppresses_missing_variant_warning() {
    // Rect has an empty-body stub — no warning should be emitted for either variant.
    code!(
        "enum Shape {\n    Circle { r: float },\n    Rect { w: float, h: float }\n}\nfn area(self: Circle) -> float { self.r * self.r }\nfn area(self: Rect) -> float { }\nfn test() { 1 + 1; }"
    );
    // no .warning() → assert_diagnostics expects an empty diagnostic set
}

// Direct call to stub (empty-body variant method) must not panic
#[test]
fn direct_call_to_stub() {
    // Calling r.area() where area is a stub for Rect must compile without panic.
    code!(
        "enum Shape { Circle { r: float }, Rect { w: float, h: float } }
fn area(self: Circle) -> float { self.r * self.r }
fn area(self: Rect) -> float { }
fn test() { r = Rect { w: 3.0, h: 4.0 }; r.area(); }"
    );
    // no .error() → compilation must succeed
}

// P257 (2026-05-12) — capturing a vector into a closure body used to
// crash both backends with no clean diagnostic: interp panicked with
// "Write to locked store at rec=N fld=M" (src/store.rs:963), native
// rejected with rustc E0308 + E0605 in the generated code, then (2026-05-12)
// rejected at parse.  @PLN93 (#511) implemented the deferred "option (b)":
// a collection is now captured by shared DbRef, so reading / index lookup
// through the capture works.  (`items[idx]` is a nullable vector read → `?? 0`.)
#[test]
fn p257_vector_capture_in_closure_works() {
    code!(
        "fn test() {
    items = [10, 20, 30];
    f = fn(idx: integer) -> integer { items[idx] ?? 0 };
    print(\"{f(1)}\\n\");
}"
    );
}

// @PLN93 (#511) Phase 6b — appending to a captured collection (`h += K{…}` inside a
// closure) is no longer rejected at parse: it lowers to an `OpNewRecord`/`OpFinishRecord`
// insert into the captured DbRef.  This locks in "parses clean" at the unit level; the
// both-backend runtime + leak proof lives in tests/scripts/505-collection-capture.loft.
#[test]
fn p511_bare_append_through_capture_parses() {
    code!(
        "struct K { id: integer, v: integer }
fn c0(cb: fn() -> integer) -> integer { cb() }
fn test() {
    h: hash<K[id]> = [];
    _ = c0(fn() -> integer { h += K { id: 1, v: 42 }; 0 });
    print(\"{len(h)}\\n\");
}"
    );
}

// P257 — the workaround the diagnostic recommends actually works:
// bind the element you need before the lambda, capture the bound
// value (a primitive or Reference) instead of the collection itself.
#[test]
fn p257_bind_before_lambda_workaround_works() {
    code!(
        "fn test() {
    items = [10, 20, 30];
    x = items[1];
    f = fn(dx: integer) -> integer { x + dx };
    print(\"{f(5)}\\n\");
}"
    );
}

// P213 — capturing closure stored in struct field used to panic at
// `src/store.rs:963` ("Write to locked store") in interp and emit
#[test]
fn p213_noncapturing_closure_in_struct_field_works() {
    // Non-capturing closures still parse and work — only the
    // capturing case is rejected.
    code!(
        "struct Box { cb: fn(integer) -> integer }
fn test() {
    b = Box { cb: fn(x: integer) -> integer { x + 1 } };
    print(\"{b.cb(10)}\\n\");
}"
    );
}

// Direct call to a method that exists on the enum but has no implementation for the variant
#[test]
fn direct_call_unimplemented_variant() {
    // r.area() where Rect has no area method at all must give an error, not a panic.
    code!(
        "enum Shape { Circle { r: float }, Rect { w: float, h: float } }
fn area(self: Circle) -> float { self.r * self.r }
fn test() { r = Rect { w: 3.0, h: 4.0 }; r.area(); }"
    )
    .error("Unknown field Rect.area at direct_call_unimplemented_variant:3:49")
    .warning(
        "no implementation of 'area' for variant 'Rect' at direct_call_unimplemented_variant:1:41",
    );
}

// --- parallel_for: extra context-argument count validation ---

#[test]
fn parallel_for_missing_context_arg() {
    // Worker expects 1 extra context arg (m) but none is provided.
    code!(
        "struct Item { v: integer } \
         fn scale(r: const Item, m: integer) -> integer { r.v * m } \
         fn test() { items = [Item{v:1}]; parallel_for(scale, items, 1); }"
    )
    .error("parallel_for: wrong number of extra arguments: worker expects 1, got 0 at parallel_for_missing_context_arg:1:150");
}

#[test]
fn parallel_for_unexpected_context_arg() {
    // Worker expects 0 extra args but 1 is provided.
    code!(
        "struct Item { v: integer } \
         fn id(r: const Item) -> integer { r.v } \
         fn test() { items = [Item{v:1}]; mult = 3; parallel_for(id, items, 1, mult); }"
    )
    .error("parallel_for: wrong number of extra arguments: worker expects 0, got 1 at parallel_for_unexpected_context_arg:1:144");
}

#[test]
fn parallel_for_too_many_context_args() {
    // Worker expects 1 extra arg but 2 are provided.
    code!(
        "struct Item { v: integer } \
         fn scale(r: const Item, m: integer) -> integer { r.v * m } \
         fn test() { items = [Item{v:1}]; a = 2; b = 3; parallel_for(scale, items, 1, a, b); }"
    )
    .error("parallel_for: wrong number of extra arguments: worker expects 1, got 2 at parallel_for_too_many_context_args:1:170");
}

// --- For-loop mutation guards ---

#[test]
fn add_to_iterated_vector() {
    // `v += elem` where v is currently being iterated is unsound: get_vector re-reads
    // the length each step, so new elements are visited — risking an infinite loop.
    code!("fn test() { v = [1, 2, 3]; for e in v { v += [4]; } }")
        .warning("Variable e is never read at add_to_iterated_vector:1:40")
        .error("Cannot add elements to 'v' while it is being iterated — use a separate collection or add after the loop at add_to_iterated_vector:1:47");
}

#[test]
fn remove_from_iterated_vector_is_allowed() {
    // `e#remove` adjusts the iterator position after removal — it is the designed,
    // safe way to remove the current element during iteration.  No error expected.
    code!("fn test() { v = [1, 2, 3]; for e in v if e > 1 { e#remove; } }");
}

#[test]
fn add_to_outer_loop_iterated() {
    // The guard catches mutations of a collection iterated by an *outer* loop too.
    code!(
        "fn test() { v = [1, 2, 3]; for e in v { for n in 1..3 { v += [n]; } } }"
    )
    .warning("Variable e is never read at add_to_outer_loop_iterated:1:40")
    .error("Cannot add elements to 'v' while it is being iterated — use a separate collection or add after the loop at add_to_outer_loop_iterated:1:63");
}

// T1-10: unused loop variable warning
#[test]
fn unused_loop_var_range() {
    // Loop variable never read in body — should warn.
    code!("fn test() { total = 0; for i in 0..3 { total += 1; } assert(total == 3, \"t\"); }")
        .warning("Variable i is never read at unused_loop_var_range:1:39");
}

#[test]
fn unused_loop_var_int_vector() {
    // Integer-element vector loop — should warn when element never read.
    code!(
        "fn test() {
  items = [1, 2, 3];
  total = 0;
  for item in items { total += 1; }
  assert(total == 3, \"t\");
}"
    )
    .warning("Variable item is never read at unused_loop_var_int_vector:4:22");
}

#[test]
fn unused_loop_var_suppressed_by_underscore() {
    // _ prefix suppresses the warning — consistent with other unused-variable rules.
    code!(
        "fn test() {
  items = [1, 2, 3];
  total = 0;
  for _item in items { total += 1; }
  assert(total == 3, \"t\");
}"
    );
}

#[test]
fn unused_loop_var_used_is_silent() {
    // No warning when the loop variable is actually read.
    code!(
        "fn test() {
  items = [1, 2, 3];
  total = 0;
  for item in items { total += item; }
  assert(total == 6, \"t\");
}"
    );
}

/// Unreachable code after return.
#[test]
fn unreachable_after_return() {
    code!(
        "fn compute() -> integer { return 1; x = 2; x }
fn test() { assert(compute() == 1, \"ok\"); }"
    )
    .warning("Unreachable code after return at unreachable_after_return:1:37");
}

/// Unreachable code after break.
#[test]
fn unreachable_after_break() {
    code!(
        "fn test() {
    for i in 1..5 {
        break;
        assert(false, \"unreachable\");
    };
}"
    )
    .warning("Variable i is never read at unreachable_after_break:2:20")
    .warning("Unreachable code after break at unreachable_after_break:4:9");
}

/// Unreachable code after continue.
#[test]
fn unreachable_after_continue() {
    code!(
        "fn test() {
    for i in 1..5 {
        continue;
        assert(false, \"unreachable\");
    };
}"
    )
    .warning("Variable i is never read at unreachable_after_continue:2:20")
    .warning("Unreachable code after continue at unreachable_after_continue:4:9");
}

/// No warning: return inside an if branch does not terminate the block.
#[test]
fn no_unreachable_after_branch_return() {
    code!(
        "fn compute(x: integer) -> integer {
    if x > 0 { return x };
    0
}
fn test() { assert(compute(5) == 5, \"ok\"); }"
    );
}

/// @PLN48 S2: `spatial<T[x, y]>` is now a working keyed collection (the radix
/// tree), so the old "planned for 1.1+" gate is gone.  What remains is the
/// arity check: a spatial index needs its coordinate key fields, so a bare
/// `spatial<T>` (no key-spec) is a helpful error rather than a silent empty key.
#[test]
fn spatial_needs_coordinate_keys() {
    code!("struct Point { x: integer, y: integer }\nfn test() { xs: spatial<Point> = []; }")
        .error("spatial<T[x, y]> needs coordinate key fields, e.g. spatial<Mob[x, y]> at spatial_needs_coordinate_keys:2:33");
}

/// @PLN48: the Morton code interleaves at most `radix_db::MAX_AXES` (3) axes; a wider
/// `spatial<T[a,b,c,d]>` key would index past the fixed `[u64; MAX_AXES]` code array
/// at runtime (a production panic).  The parser rejects it with a clean diagnostic.
#[test]
fn spatial_rejects_more_than_three_axes() {
    code!("struct P4 { a: integer, b: integer, c: integer, d: integer }\nfn test() { xs: spatial<P4[a, b, c, d]> = []; }")
        .error("spatial<T[…]> supports at most 3 coordinate axes, got 4 at spatial_rejects_more_than_three_axes:2:42");
}

/// loft#799: a `spatial` keyed on TEXT compiled, counted right, and then answered
/// NULL for a key just inserted — indistinguishable from "not found" at the call
/// site.  Refused at declaration now, naming the kind that does key on bytes.
#[test]
fn spatial_rejects_a_text_key() {
    code!("struct Word { w: text, n: integer }\nfn test() { ws: spatial<Word[w]> = []; }")
        .error("a spatial index interleaves its axes into a Morton code, which needs numbers, and `w` is text — use `trie<Word[w]>`, which keys on text and answers a prefix at spatial_rejects_a_text_key:2:35");
}

/// loft#799, the mirror: a `trie` walks one key's BYTES, so a numeric key is the
/// same mistake the other way round.
#[test]
fn trie_rejects_a_numeric_key() {
    code!("struct Pt { x: integer, y: integer }\nfn test() { ps: trie<Pt[x]> = []; }")
        .error("a trie keys on the BYTES of a text field, and `x` is integer — use `spatial<…>` for coordinates, or `sorted<…>` / `index<…>` to order on a number at trie_rejects_a_numeric_key:2:30");
}

/// A trie orders ONE key's bytes, so several keys have no order to share.
#[test]
fn trie_rejects_two_keys() {
    code!("struct Word { w: text, s: text }\nfn test() { ws: trie<Word[w, s]> = []; }")
        .error("trie<T[k]> keys on exactly ONE text field, got 2 — a trie orders one key's bytes, so several keys have no order to share; use `sorted<T[a, b]>` for a multi-field order at trie_rejects_two_keys:2:35");
}

/// A bare `trie<T>` has no key to walk, so it is a helpful error rather than a
/// silent empty key — the same shape `spatial<T>` gets.
#[test]
fn trie_needs_its_key_field() {
    code!("struct Word { w: text }\nfn test() { ws: trie<Word> = []; }").error(
        "trie<T[k]> needs its text key field, e.g. trie<Word[w]> at trie_needs_its_key_field:2:29",
    );
}

/// F57: write_file on a struct with a collection-type field must produce a compile error.
#[test]
fn write_file_collection_field() {
    code!(
        "struct Item { x: integer }\n\
         struct Record { items: sorted<Item[x]> }\n\
         fn test() {\n\
           f = file(\"out.bin\");\n\
           f#format = LittleEndian;\n\
           r = Record{};\n\
           f += r;\n\
         }"
    )
    .error("write_file: 'Record' has variable-width field 'items' (text/vector/collection) that binary I/O cannot round-trip; serialise a plain fixed-width struct at write_file_collection_field:7:8");
}

/// F57: read_file with `as T` where T has a collection-type field must produce a compile error.
#[test]
fn read_file_collection_field() {
    code!(
        "struct Item { x: integer }\n\
         struct Record { items: sorted<Item[x]> }\n\
         fn test() {\n\
           f = file(\"out.bin\");\n\
           f#format = LittleEndian;\n\
           _ = f#read(8) as Record;\n\
         }"
    )
    .error("read_file: 'Record' has variable-width field 'items' (text/vector/collection) that binary I/O cannot round-trip; serialise a plain fixed-width struct at read_file_collection_field:6:25");
}

/// T1-22: function with `not null` return type that may fall through warns.
/// This genuinely exercises the `not null` feature (the fall-through warning only
/// exists for a not-null return), so — unlike the incidental uses swept out in the
/// arc-E test-hygiene pass — it KEEPS `not null` and asserts BOTH warnings it emits.
#[test]
fn missing_return_not_null() {
    code!(
        "fn classify(n: integer) -> text not null {
    if n > 0 { return \"pos\" };
}
fn test() { classify(1); }"
    )
    .advice(
        "`not null` is deprecated and has no effect — a type is non-null by default now at missing_return_not_null:1:43",
    )
    .warning(
        "Not all code paths return a value — function 'classify' may return null at missing_return_not_null:3:2",
    );
}

/// T1-22: if/else where both branches return — no error, no warning.
/// (This currently produces a false-positive "void should be integer" error.)
#[test]
fn all_paths_return_if_else() {
    code!(
        "fn classify(n: integer) -> integer {
    if n > 0 { return 1 } else { return -1 }
}
fn test() { assert(classify(5) == 1, \"ok\"); }"
    );
}

/// T1-22: if/else both return with `not null` — no warning.
#[test]
fn all_paths_return_not_null() {
    code!(
        "fn classify(n: integer) -> integer {
    if n > 0 { return 1 } else { return -1 }
}
fn test() { assert(classify(5) == 1, \"ok\"); }"
    );
}

/// T1-22: function with `not null` return ending in a direct return — no warning.
#[test]
fn direct_return_not_null() {
    code!(
        "fn always() -> integer {
    return 42
}
fn test() { assert(always() == 42, \"ok\"); }"
    );
}

/// T1-22: last expression in block is non-void — counts as definitely-returns, no warning.
#[test]
fn implicit_return_not_null() {
    code!(
        "fn double(n: integer) -> integer {
    n * 2
}
fn test() { assert(double(3) == 6, \"ok\"); }"
    );
}

#[test]
fn shadow_different_type() {
    // A for-loop variable landing on a plain local is rejected whatever the two types
    // are — loft#915 folded the differing-type case into the one shadow diagnostic, since
    // a loop no longer inherits any binding and the type comparison had nothing left to
    // decide.  Reported on PASS 1, at the loop.
    code!(
        "fn test() {
    x = 1.5;
    v = [1, 2, 3];
    for x in v { }
}"
    )
    .error(
        "loop variable 'x' shadows a local named 'x' — rename the loop \
         variable (e.g. loop_x) or drop the outer `x` if it was a dead \
         placeholder; loft does not block-scope loop variables at \
         shadow_different_type:4:17",
    );
}

#[test]
fn shadow_same_type_ok() {
    // C61.local: same-type shadow of an outer local is now rejected at
    // parse time — renaming the loop variable or dropping the outer
    // local are the two documented fixes.  Previously this test was
    // named "_ok" because the reuse silently succeeded; it now pins the
    // rejection to prevent regression.
    code!(
        "fn test() {
    x = 10;
    v = [1, 2, 3];
    for x in v { }
    println(\"{x}\");
}"
    )
    .error(
        "loop variable 'x' shadows a local named 'x' — rename the loop \
         variable (e.g. loop_x) or drop the outer `x` if it was a dead \
         placeholder; loft does not block-scope loop variables at \
         shadow_same_type_ok:4:17",
    );
}

#[test]
fn if_expr_without_else() {
    // Using if as a value expression without else is a compile error.
    code!(
        "fn test() {
    x = if true { 42 };
    println(\"{x}\");
}"
    )
    .error("If-expression produces a value but has no else clause; add an else branch or make the body a statement at if_expr_without_else:2:24");
}

#[test]
fn if_expr_with_else_ok() {
    // If-expression with else is fine.
    code!(
        "fn test() {
    x = if true { 42 } else { 0 };
    assert(x == 42, \"ok\");
}"
    );
}

#[test]
fn if_statement_without_else_ok() {
    // If-statement (void body) without else is fine — no error.
    code!(
        "fn test() {
    x = 10;
    if x > 5 {
        println(\"{x}\");
    }
}"
    );
}

#[test]
fn type_cycle_self() {
    // Self-referential struct is a compile error.
    code!("struct Node { val: integer, next: Node }\nfn test() { }")
        .error("Struct 'Node' contains itself (directly or indirectly) — use reference<Node> to break the cycle at type_cycle_self:1:14");
}

#[test]
fn type_cycle_indirect() {
    // Mutually recursive structs are a compile error.
    code!(
        "struct A { val: integer, b: B }
struct B { val: integer, a: A }
fn test() { }"
    )
    .error("Struct 'A' contains itself (directly or indirectly) — use reference<A> to break the cycle at type_cycle_indirect:1:11")
    .error("Struct 'B' contains itself (directly or indirectly) — use reference<B> to break the cycle at type_cycle_indirect:2:11");
}

#[test]
fn non_cyclic_nested_struct_ok() {
    // Non-cyclic struct nesting is fine.
    code!(
        "struct Inner { x: integer }
struct Outer { i: Inner, y: integer }
fn test() {
    o = Outer { i: Inner { x: 1 }, y: 2 };
    assert(o.i.x == 1, \"nested\");
}"
    );
}

#[test]
fn keyword_sizeof_as_fn() {
    code!("fn sizeof() {}\nfn test() {}")
        .error("Expect name in function definition at keyword_sizeof_as_fn:1:10")
        .error("Syntax error: unexpected 'sizeof' at keyword_sizeof_as_fn:1:10");
}

// A10: `fields` is no longer a keyword — it can be used as a function name.

#[test]
fn keyword_debug_assert_as_fn() {
    code!("fn debug_assert() {}\nfn test() {}")
        .error("Expect name in function definition at keyword_debug_assert_as_fn:1:16")
        .error("Syntax error: unexpected 'debug_assert' at keyword_debug_assert_as_fn:1:16");
}

#[test]
fn keyword_assert_as_fn() {
    code!("fn assert() {}\nfn test() {}")
        .error("Expect name in function definition at keyword_assert_as_fn:1:10")
        .error("Syntax error: unexpected 'assert' at keyword_assert_as_fn:1:10");
}

#[test]
fn keyword_panic_as_fn() {
    code!("fn panic() {}\nfn test() {}")
        .error("Expect name in function definition at keyword_panic_as_fn:1:9")
        .error("Syntax error: unexpected 'panic' at keyword_panic_as_fn:1:9");
}

/// P5.3: operator on generic type T produces a generic-specific error.
#[test]
fn generic_operator_error() {
    code!("fn bad<T>(x: T, y: T) -> T { x + y }\nfn test() {}").error(
        "generic type T: operator '+' requires a concrete type at generic_operator_error:1:36",
    );
}

/// P5.3: field access on generic type T produces a generic-specific error.
#[test]
fn generic_field_error() {
    code!("fn bad<T>(x: T) -> integer { x.name }\nfn test() {}")
        .error("generic type T: field access requires a concrete type at generic_field_error:1:38");
}

// ── A5.1 — Closure capture analysis ─────────────────────────────────────────

/// A5.1: lambda referencing an outer variable is detected as a capture.
#[test]
fn capture_detected() {
    code!("fn test() {\n  count = 0;\n  f = fn(x: integer) { count += x; };\n  f(1);\n}");
}

/// A5.1: lambda that does NOT reference outer variables has no capture error.
#[test]
fn no_capture_no_error() {
    code!("fn test() {\n  f = fn(x: integer) -> integer { x + 1 };\n  assert(f(1) == 2);\n}");
}

/// A5.1: variable defined inside the lambda is not flagged as captured.
#[test]
fn local_not_captured() {
    code!(
        "fn test() {\n  f = fn(x: integer) -> integer { y = x + 1; y };\n  assert(f(1) == 2);\n}"
    );
}

// ── A5.2 — Closure record layout ────────────────────────────────────────────

/// A5.2: closure record is synthesized with the correct captured variable.
#[test]
fn closure_record_single_capture() {
    code!("fn test() {\n  count = 0;\n  f = fn(x: integer) { count += x; };\n  f(1);\n}");
}

/// A5.2: multiple captures produce a record with multiple fields.
#[test]
fn closure_record_multi_capture() {
    // A5.3: multi-capture — captured reads redirect to closure record fields.
    // No more "Unknown variable" errors thanks to the pre-has_var redirect.
    code!(
        "fn test() {\n  a = 1;\n  b = 2.0;\n  f = fn(x: integer) -> float { (a + x) as float + b };\n  assert(f(3) == 6.0);\n}"
    );
}

// ── CO1.5c — e#remove rejection on generator iterators ──────────────────────

#[test]
fn generator_remove_rejected() {
    code!(
        "fn gen() -> iterator<integer> { yield 1; yield 2; }
         fn test() { for n in gen() { n#remove; } }"
    )
    .error("'n#remove' is only valid on a loop iteration variable (e.g. 'for n in collection { n#remove }') at generator_remove_rejected:2:48");
}

// ── Fix #91 — Circular init detection ────────────────────────────────────────

// ── S23 — reject generator functions as par() workers ────────────────────────

/// S23: a worker function whose return type is iterator<T> must be rejected at
/// compile time.  Worker threads run inside par() and cannot advance coroutines
/// from the main thread — calling coroutine_next on an out-of-range index panics.
#[test]
fn par_worker_returns_generator() {
    code!(
        "fn gen_worker(x: integer) -> iterator<integer> { yield x; }
         fn test() {
             items = [1, 2, 3];
             for a in items par(b = gen_worker(a), 1) { assert(b > 0); }
         }"
    )
    .error("parallel worker 'gen_worker' returns iterator<integer> — generator functions cannot be used as parallel workers at par_worker_returns_generator:4:51");
}

/// loft#998 — `x#break` naming a declared NON-loop local was an internal compiler error.
///
/// `Variables::loop_nr` walked the enclosing-loop chain and exited on its CONDITION, so
/// falling off the end returned the chain length — one past the deepest valid level, with
/// no "not found" signal. `Scopes::scan` then indexed `loops.len() - lv - 1` and
/// underflowed a `usize`: *"internal compiler error … index is 18446744073709551615"*,
/// which names neither the mistake nor a cure. Both backends.
///
/// It now answers `Option`, because "not found" and "the outermost loop" are different
/// answers and one number cannot carry both, and the parser reports — which is where the
/// source position and the author's spelling are.
///
/// The message names what CAN be written, and the enclosing loops' variables are the whole
/// answer to that: innermost first, so a nested pair reads `j#break` before `i#break`.
#[test]
fn a_non_loop_variable_in_break_says_so() {
    code!("fn test() { k = 0; for j in 1..=3 { if j == 2 { k#break } } }").error(
        "`k` is not a loop variable — `k#break` names the loop to break by the variable that loop binds, and no enclosing loop binds `k`; write a plain `break` for the innermost loop, or `j#break` at a_non_loop_variable_in_break_says_so:1:58",
    );
}

/// The `continue` twin, in a `while` — which binds NO variable, so the only cure to offer
/// is the plain form. A cell that only ever saw `for` would not have found that.
#[test]
fn a_non_loop_variable_in_continue_says_so() {
    code!("fn test() { k = 0; while k < 3 { k#continue } }").error(
        "`k` is not a loop variable — `k#continue` names the loop to continue by the variable that loop binds, and no enclosing loop binds `k`; write a plain `continue` at a_non_loop_variable_in_continue_says_so:1:46",
    );
}

/// Nested loops list both, innermost first — the order the author reads them in.
#[test]
fn the_cure_lists_the_enclosing_loops_innermost_first() {
    code!("fn test() { z = 1; for i in 1..=2 { for j in 1..=2 { z#break } } }").error(
        "`z` is not a loop variable — `z#break` names the loop to break by the variable that loop binds, and no enclosing loop binds `z`; write a plain `break` for the innermost loop, or `j#break` or `i#break` at the_cure_lists_the_enclosing_loops_innermost_first:1:63",
    )
    .warning("Variable i is never read at the_cure_lists_the_enclosing_loops_innermost_first:1:36")
    .warning("Variable j is never read at the_cure_lists_the_enclosing_loops_innermost_first:1:53");
}

/// OUTSIDE a loop the pre-existing message is the right one, and stays the ONLY one —
/// adding "…and `k` is not a loop variable" says the same thing twice from further away.
#[test]
fn outside_a_loop_keeps_its_own_message() {
    code!("fn test() { k = 0; k#break }")
        .error("Cannot continue outside a loop at outside_a_loop_keeps_its_own_message:1:29");
}

/// loft#996 — a SLICE on a library type says what to write, instead of `Expect token ]`.
///
/// The composite `x[a, b]` now dispatches to `OpIndex` (its indices are arguments, which
/// is what the accepted declaration already means). The slice cannot: every built-in kind
/// lowers its own to a dedicated runtime call, and there is no range VALUE in the language
/// for a user method to receive — so `x[a..b]` needs a range type or an `OpSlice` of its
/// own, which is a language addition and not a parse. What it must not stay is
/// `Expect token ]` pointing at the `..`, beside the two messages this feature already
/// gets right.
///
/// ONE error, which is the second half of the fix: the refusal consumes the rest of the
/// bracket so the caller's `]` still matches. Returning with `..2` unread cascaded on
/// pass 1, and a pass-1 abort silences every pass-2 diagnostic — the first version of this
/// reported nothing at all and left `Expect token ]` standing.
#[test]
fn opindex_slice_says_what_to_write() {
    code!(
        "struct Ring996 { data: vector<integer> }
fn OpIndex(self: Ring996, i: integer) -> integer { return self.data[i] ?? 0; }
fn test() { r = Ring996 { data: [7, 8, 9] }; _x = r[0..2]; }"
    )
    .error(
        "`Ring996` defines `OpIndex`, which takes INDEX arguments — there is no range value to hand it, so `x[a..b]` has nothing to dispatch to; write the bounds as indices (`x[a, b]`, if `OpIndex` declares two) or give the type a method that slices (`x.slice(a, b)`) at opindex_slice_says_what_to_write:3:56",
    );
}

/// The arity mismatch the comma form makes reachable, and the name it uses.
///
/// A one-index `OpIndex` given two now earns the ordinary call diagnostic, reported
/// against the signature the author wrote. It used to name the STORAGE symbol —
/// `t_4Ring996_OpIndex`, which appears in no source file — and two fixtures in this suite
/// record that as a defect of its own (`tests/lib/dupmethod_a/…`,
/// `tests/scripts/850-…`). `Data::user_facing_name` renders the method as `Type.name`,
/// keeping the receiver visible because that is exactly what is ambiguous where these
/// messages fire.
#[test]
fn opindex_wrong_arity_names_the_method_not_the_symbol() {
    code!(
        "struct Ring996 { data: vector<integer> }
fn OpIndex(self: Ring996, i: integer) -> integer { return self.data[i] ?? 0; }
fn test() { r = Ring996 { data: [7, 8, 9] }; _x = r[0, 1]; }"
    )
    .error(
        "Too many parameters for Ring996.OpIndex at opindex_wrong_arity_names_the_method_not_the_symbol:3:58",
    );
}

/// The other caller of the same "unresolved worker" sentinel: a worker name that does
/// not exist. Both refusals answer `(u32::MAX, Unknown)` from `parse_parallel_worker`,
/// and `build_parallel_for_ir` has to read that ONE sentinel two opposite ways — on
/// pass 1 it means "not resolved yet" (a worker declared below the loop, loft#988, where
/// typing `b` as `integer` pins the slot and pass 2 can no longer refine it), on pass 2
/// it means "will never resolve, and the error is already reported", where `b` needs a
/// usable type or every use of it in the body earns a second diagnostic under the first.
///
/// The pass is what tells them apart. This cell holds the pass-2 half for the second
/// caller: exactly one error, with `b > 0` in the body silent (the `code!` harness fails
/// on any diagnostic not asserted here).
#[test]
fn par_worker_that_does_not_exist_reports_once() {
    code!(
        "fn test() {
             items = [1, 2, 3];
             for a in items par(b = no_such_worker(a), 1) { assert(b > 0); }
         }"
    )
    .error("Unknown function 'no_such_worker' at par_worker_that_does_not_exist_reports_once:3:55");
}

// ── T1.11 — Tuple type constraints ───────────────────────────────────────────

// T1.11a (Plan-06 phase 4d): the original rejection of tuple-typed struct
// fields ("tuples are stack-only values that cannot be heap-allocated")
// has been LIFTED.  Tuple fields now lay out their elements inline using
// the synthetic `__tuple<…>` struct's positions, mirroring how index
// bookkeeping triples are placed atomically.  See
// `parser/mod.rs::set_field_check`/`get_val` (Type::Tuple arms) and the
// `/tmp/tup_field*.loft` smoke tests for the working behaviour.

/// T1.11b: compound assignment on a tuple LHS must produce a clear diagnostic
/// instead of a generic internal error.
#[test]
fn tuple_compound_assign_rejected() {
    code!("fn test() { a = 1; b = 2; (a, b) += (1, 2); }")
        .error("compound assignment is not supported for tuple destructuring — use (a, b) = expr instead at tuple_compound_assign_rejected:1:36");
}

/// P206: a `match` arm written with `->` (the lambda return-arrow) instead
/// of the canonical `=>` separator must produce a clear diagnostic — and
/// MUST NOT hang the parser.  Before the fix, `lexer.token("=>")` failed
/// silently, the lexer never advanced, and the surrounding arm-loop spun
/// indefinitely consuming gigabytes of memory before OOM-kill.
#[test]
fn p206_match_arrow_rejected_scalar() {
    code!("fn test() { x = 1; match x { 0 => 0, _ -> 99 } }")
        .error("match arm separator is `=>`, not `->` at p206_match_arrow_rejected_scalar:1:42");
}

/// P206: same hazard inside a tuple match — the `parse_tuple_match` loop
/// shares the arrow-consume helper and must report the same diagnostic.
#[test]
fn p206_match_arrow_rejected_tuple() {
    code!("fn test() { t = (1, 2); match t { (0, _) => 0, (a, b) -> a + b } }")
        .error("match arm separator is `=>`, not `->` at p206_match_arrow_rejected_tuple:1:57");
}

/// plan-18/01: an or-pattern containing `@`-bindings (`x @ 1 | x @ 2 => …`)
/// previously hung the parser indefinitely.  `parse_match_pattern`
/// doesn't recognise `name @ pattern` inside the or-pattern loop — it
/// only handles literals, ranges, and bare expressions — so the
/// inner parse stopped without consuming `@`, and the outer
/// scalar/tuple/enum match loop re-entered pattern parsing on the
/// same unconsumed token (infinite loop, eventually OOM).
///
/// Fix: `expect_match_arm_arrow` now recovers via
/// `lexer.recover_to(&[",", "}", ";"])` after a missing `=>` so the
/// outer loop can pick up the next arm or exit cleanly.  The
/// diagnostic stays "Expect token =>".
#[test]
fn plan18_at_binding_in_or_pattern_does_not_hang() {
    code!("fn test() { n = 2; match n { x @ 1 | x @ 2 => 0, _ => 99 } }")
        .error("Expect token => at plan18_at_binding_in_or_pattern_does_not_hang:1:41");
}

// ── I1/I3 — Interface declarations ───────────────────────────────────────────

/// I3: a minimal empty interface declaration parses without error.
#[test]
fn interface_empty_parses() {
    code!("interface Foo {}\nfn test() {}");
}

/// I3: an interface with method signatures parses without error.
#[test]
fn interface_with_method_parses() {
    code!("interface Showable { fn display(self: Self) -> text }\nfn test() {}");
}

/// I3: a duplicate interface name is rejected with a "Redefined interface" diagnostic.
#[test]
fn interface_duplicate_name_rejected() {
    code!("interface Foo {}\ninterface Foo {}\nfn test() {}")
        .error("Cannot redefine interface 'Foo' at interface_duplicate_name_rejected:2:16");
}

// ── I3.1 — op-sugar in interface bodies ──────────────────────────────────────

/// I3.1: `op < (self: Self, other: Self) -> boolean` in an interface body is
/// syntactic sugar for a method named `OpLt` and must parse without error.
#[test]
fn interface_op_sugar_lt_parses() {
    code!("interface Rankable { op >= (self: Self, other: Self) -> boolean }\nfn test() {}");
}

/// I3.1: a multi-operator interface with `op +` and `op ==` desugars correctly.
#[test]
fn interface_op_sugar_multi_parses() {
    code!(
        "interface Combinable { op & (self: Self, other: Self) -> Self\n\
                                op ^ (self: Self, other: Self) -> Self }\nfn test() {}"
    );
}

// ── I4 — <T: Bound> bound syntax ─────────────────────────────────────────────

/// I4: `fn foo<T: Ordered>(x: T) -> T` with a valid interface bound parses
/// without error and stores the bound for later satisfaction checking.
#[test]
fn generic_fn_with_bound_parses() {
    code!("fn identity<T: Ordered>(x: T) -> T { x }\nfn test() {}");
}

/// I4: a bound name that does not exist must produce a clear diagnostic.
#[test]
fn generic_fn_unknown_bound_errors() {
    code!("fn foo<T: NonExistent>(x: T) -> T { x }\nfn test() {}")
        .error("'NonExistent' is not a known interface at generic_fn_unknown_bound_errors:1:32");
}

/// I4: a struct name used as a type bound must be rejected — only interfaces are valid bounds.
#[test]
fn generic_fn_struct_as_bound_errors() {
    code!("struct Point { x: integer }\nfn foo<T: Point>(x: T) -> T { x }\nfn test() {}")
        .error("'Point' is not an interface — bounds must be interface names at generic_fn_struct_as_bound_errors:2:26");
}

// ── I5 — Factory-method restriction ──────────────────────────────────────────

/// I5: a method that returns `Self` without a leading `self: Self` parameter
/// is a factory method and must be rejected in phase 1.
#[test]
fn interface_factory_method_rejected() {
    code!("interface Creatable { fn create() -> Self }\nfn test() {}")
        .error("factory methods not yet supported: 'create' returns Self without a 'self: Self' parameter at interface_factory_method_rejected:1:44");
}

// ── I6/I10 — Satisfaction checking diagnostics ───────────────────────────────

/// I6/I10: calling a bounded generic function with a type that does NOT implement
/// the required interface method must produce a clear "does not satisfy" diagnostic.
#[test]
fn satisfaction_check_fails_missing_method() {
    code!(
        "struct Thing { x: integer }
         fn pick_first<T: Ordered>(a: T, _b: T) -> T { a }
         fn test() { pick_first(Thing{x:1}, Thing{x:2}) }"
    )
    .error("'Thing' does not satisfy interface 'Ordered': missing OpLt at satisfaction_check_fails_missing_method:3:57");
}

/// @PLN125 A2a: an implementor whose method returns a DIFFERENT named type than
/// the interface declares does not satisfy it, and the message names both.
///
/// Before this, satisfaction checked only that a method of the right NAME
/// existed. What saved most programs was not a check but a coincidence: the
/// generic body is typed from the INTERFACE's return, so a field the interface's
/// type lacks fails later as `Unknown field Rows.m` — a message about the wrong
/// type, at the use site, blaming the caller. Give both structs the same field
/// name and even that went quiet while the call returned from the concrete
/// record.
#[test]
fn satisfaction_check_fails_on_a_different_return_type() {
    code!(
        "struct Rows { n: integer }
         struct ConnRows { pad: text, n: integer }
         interface Db { fn sel(self: Self) -> Rows }
         struct Conn { k: integer }
         fn sel(self: Conn) -> ConnRows { ConnRows { pad: \"p\", n: self.k } }
         fn go<D: Db>(d: D) -> integer { r = d.sel(); r.n }
         fn test() { go(Conn{k:1}) }"
    )
    .error(
        "'Conn' does not satisfy interface 'Db': 'sel' returns 'ConnRows' but the \
         interface declares 'Rows' at satisfaction_check_fails_on_a_different_return_type:7:36",
    );
}

/// @PLN125 A2a, the other side: `Self` in the interface's return substitutes to
/// the concrete type, so an implementor returning ITSELF satisfies `-> Self`.
/// A check that compared the two types literally would reject every such
/// interface, which is most of them.
#[test]
fn satisfaction_accepts_self_as_the_return_type() {
    code!(
        "interface Maker { fn mk(self: Self) -> Self }
         struct A { v: integer }
         fn mk(self: A) -> A { A { v: self.v + 1 } }
         fn bump<M: Maker>(m: M) -> M { m.mk() }
         fn test() { assert(bump(A{v:1}).v == 2, \"-> Self resolves per implementor\"); }"
    );
}

/// @PLN125 A2c: the bound on an associated type is a promise about the COMPANION,
/// and the only place it can be kept is the monomorph — the companion is per
/// implementor, so `type Rows: Cursor` cannot be checked until one is chosen.
///
/// The message names all four parties because three of them are invisible at the
/// call site: the reader wrote `use_it(s)` and is being told about a type they
/// never mentioned, so it has to say how that type was reached.
#[test]
fn associated_type_companion_must_satisfy_its_bound() {
    code!(
        "interface Cursor { fn width(self: Self) -> integer }
         interface Source { type Rows: Cursor
                            fn open(self: Self) -> Self.Rows }
         struct Bad { z: integer }
         struct S1 { t: text }
         fn open(self: S1) -> Bad { Bad { z: len(self.t) } }
         fn use_it<S: Source>(s: S) -> integer { r = s.open(); r.width() }
         fn test() { use_it(S1{t:\"x\"}) }"
    )
    .error(
        "'S1' binds 'Source.Rows' to 'Bad', which does not satisfy the declared bound \
         'Cursor': missing width at associated_type_companion_must_satisfy_its_bound:8:40",
    );
}

/// @PLN125 A2c: an implementor's methods must AGREE about what the companion is.
///
/// The companion is inferred from every position where the interface named the
/// placeholder — the return of `open`, the parameter of `feed`. An implementor
/// that answers two different types there does not have one companion, and
/// binding to whichever the walk reached first would make the monomorph depend on
/// declaration order rather than on what the author wrote.
#[test]
fn associated_type_companion_must_be_one_type() {
    code!(
        "interface Cursor { fn width(self: Self) -> integer }
         interface Source { type Rows: Cursor
                            fn open(self: Self) -> Self.Rows
                            fn feed(self: Self, r: Self.Rows) -> integer }
         struct RowsA { a: integer }
         struct RowsB { b: integer }
         fn width(self: RowsA) -> integer { self.a }
         fn width(self: RowsB) -> integer { self.b }
         struct S2 { t: text }
         fn open(self: S2) -> RowsA { RowsA { a: len(self.t) } }
         fn feed(self: S2, r: RowsB) -> integer { r.b + len(self.t) }
         fn use_it<S: Source>(s: S) -> integer { r = s.open(); r.width() }
         fn test() { use_it(S2{t:\"x\"}) }"
    )
    .error(
        "'S2' does not agree with itself about 'Source.Rows': 'open' and 'feed' name \
         different types for it at associated_type_companion_must_be_one_type:13:40",
    );
}

/// @PLN125 A2c: an associated type's name is a TYPE name and follows the same
/// rule as every other one.
///
/// It is also load-bearing rather than cosmetic: it becomes the
/// `t_<LEN><Interface>.<Name>_<method>` dispatch stub, whose LEN prefix is parsed
/// back to recover the method name — an underscore in it would split that name in
/// the wrong place and silently leave the call on the template stub.
#[test]
fn associated_type_name_must_be_camel_case() {
    code!(
        "interface Cursor { fn width(self: Self) -> integer }
         interface Source { type row_set: Cursor
                            fn open(self: Self) -> Self.row_set }
         fn test() {}"
    )
    .error(
        "Associated type 'row_set' must be CamelCase at \
         associated_type_name_must_be_camel_case:2:42",
    );
}

/// @PLN125 arc C: a type that has not defined `OpIndex` names the line to add.
///
/// The old answer was the keyed-collection message, which sent a reader chasing
/// a `hash<Row[id]>` constructor that has nothing to do with subscripting their
/// struct. Now that a library type CAN be indexed, "it did not define `OpIndex`"
/// is the actual cause and the fix is one signature.
#[test]
fn indexing_a_type_without_op_index_names_the_method() {
    code!(
        "struct Plain { v: integer }
         fn test() { p = Plain { v: 1 }; p[0] }"
    )
    .error(
        "`Plain` cannot be indexed — define `fn OpIndex(self: Plain, i: integer) -> \u{3c4}` \
         to give it `x[i]` at indexing_a_type_without_op_index_names_the_method:2:45",
    );
}

/// @PLN125 arc C: inside a generic, only the BOUNDS may be relied on — so an
/// unbounded type variable cannot be subscripted even when every type it is ever
/// instantiated with defines `OpIndex`.
///
/// This is a refusal that has to be enforced rather than fall out: a bound stub
/// is named for the HOLDER (`t_1I_OpIndex`), and holder names are shared across
/// generics, so a sibling `fn a<I: Indexable>` in the same program mints exactly
/// the name an unbounded `fn b<I>` would find. Without the check `b` compiled and
/// worked, promising nothing and delivering it.
#[test]
fn indexing_an_unbounded_type_variable_is_refused() {
    code!(
        "interface Indexable { op [] (self: Self, i: integer) -> integer }
         struct Bits { words: vector<integer> }
         fn OpIndex(self: Bits, i: integer) -> integer { self.words[i] ?? 0 }
         fn good<I: Indexable>(x: I) -> integer { x[0] }
         fn bad<I>(x: I) -> integer { x[0] }
         fn test() { good(Bits{words:[7]}) + bad(Bits{words:[7]}) }"
    )
    .error(
        "generic type I: `[\u{2026}]` needs a bound that declares it — add \
         `op [] (self: Self, i: integer) -> \u{3c4}` to an interface and bound `I` by it \
         at indexing_an_unbounded_type_variable_is_refused:5:42",
    );
}

/// @PLN125 arc C: `OpIndex` READS. `x[i] = …` is refused, and the message says so
/// in the author's terms.
///
/// A writing counterpart is a separate decision — it needs its own method, and a
/// decision about whether `x[i] += 1` may then read-modify-write — so this is a
/// refusal rather than a gap left silent. Before it, the assignment path reported
/// "Cannot assign to attribute on type 't_4Bits_OpIndex'", naming an internal
/// symbol the author never wrote.
#[test]
fn assigning_through_op_index_is_refused() {
    code!(
        "struct Bits { words: vector<integer> }
         fn OpIndex(self: Bits, i: integer) -> integer { self.words[i] ?? 0 }
         fn test() { b = Bits { words: [1,2] }; b[0] = 9; }"
    )
    .error(
        "`Bits` defines `OpIndex`, which READS — `x[i] = \u{2026}` has nothing to write \
         through; give the type a method that sets (`x.set(i, \u{2026})`) at \
         assigning_through_op_index_is_refused:3:58",
    );
}

/// @PLN125 arc B: `OpDrop` cannot return.
///
/// It runs at a closing brace with no caller left to tell, and loft has no
/// runtime errors (C80), so a result would go nowhere. That is a real semantic
/// weakening and it is the design: anything whose failure MATTERS stays an
/// explicit call — `tx.commit()` answers, the scope end does not. Saying so at
/// the declaration beats letting an author write a `-> boolean` nobody reads.
#[test]
fn op_drop_cannot_return() {
    code!(
        "struct Tx { t: text }
         fn OpDrop(self: Tx) -> boolean { self.t != \"\" }
         fn test() { x = Tx { t: \"a\" }; }"
    )
    .error(
        "`OpDrop` cannot return — it runs at scope end with no caller to answer; \
         anything whose failure matters stays an explicit call at op_drop_cannot_return:2:32",
    );
}

/// @PLN125 arc B: `OpDrop` takes only the receiver — the compiler calls it, so a
/// second argument has nowhere to come from.
///
/// The POSITION is part of the promise. Both checks run after the whole body is
/// parsed, so the lexer cursor has already reached the next declaration — these
/// used to assert `3:12`, which is `fn test`, a function neither message names.
#[test]
fn op_drop_takes_only_self() {
    code!(
        "struct Tx { t: text }
         fn OpDrop(self: Tx, extra: integer) { assert(extra > 0, self.t) }
         fn test() { x = Tx { t: \"a\" }; }"
    )
    .error(
        "`OpDrop` takes only `self` — the compiler calls it, so a second argument has \
         nowhere to come from at op_drop_takes_only_self:2:47",
    );
}

/// loft#845: `"{v}"` on an UNBOUNDED type variable is refused, and the message
/// names the bound that renders it.
///
/// A format string picks its op from the value's TYPE, and in a template the only
/// type available is the parameter's — an attribute-less struct, so a reference.
/// Substitution replaces types, not the ops chosen from them, so the monomorph
/// ran the RECORD formatter against a concrete value. Measured over every
/// argument kind, not one produced a right answer: `integer` / `float` /
/// `boolean` / `character` / an enum SIGSEGV'd on `--interpret` and were `E0308`
/// on `--native`; `text` and a struct rendered the literal `{}` on both. So this
/// refuses nothing that worked.
///
/// It is also the rule the rest of the language already applies — inside a
/// generic only the BOUNDS may be relied on, as a method call, a subscript
/// (@PLN125 arc C) and an operator all say. `Printable` is that bound, and the
/// bounded path renders correctly on both backends for every kind
/// (`tests/scripts/845-generic-format.loft`).
/// loft#1147 — and this test is LOAD-BEARING beyond its own subject.  Type-variable bounds
/// are keyed by NAME, so every generic in the program spelling its variable `T` shares one
/// `T` definition and one bounded generic anywhere mints the bound's stubs for all of them.
/// Adding an `Equatable + Printable` generic to the stdlib turned this refusal into an
/// ACCEPT, with nothing in the user's file changed — the format site was asking whether the
/// stub existed rather than whether this function's bounds declare it, which is the question
/// `call_op` already asks.  If this test ever goes green-by-accepting again, that leak is
/// back.
#[test]
fn formatting_an_unbounded_type_variable_is_refused() {
    code!(
        "fn show<T>(v: T) -> text { \"{v}\" }
         fn test() { assert(show(7) == \"7\", \"never reached\") }"
    )
    .error(
        "generic type T cannot be formatted — `\"{\u{2026}}\"` needs a bound that renders \
         it; write `<T: Printable>` (every built-in satisfies it, and a user type does by \
         defining `fn to_text(self: T) -> text`) at \
         formatting_an_unbounded_type_variable_is_refused:1:33",
    );
}

/// loft#1147 — the same leak, swept across every surface a bound can reach.  An UNBOUNDED
/// `<T>` must be refused each of these however many bounded generics the stdlib carries: a
/// stub minted for `min_of<T: Ordered>`, `sum<T: Addable>`, `tree_walk<T: Walkable>` or
/// `assert_eq<T: Equatable + Printable>` is that function's licence, not everyone's.
#[test]
fn an_unbounded_type_variable_reaches_no_bound_s_method() {
    code!("fn bad<T>(a: T, b: T) -> boolean { a == b }\nfn test() {}")
        .error("generic type T: operator '==' requires a concrete type at an_unbounded_type_variable_reaches_no_bound_s_method:1:43");
    code!("fn bad<T>(a: T, b: T) -> boolean { a != b }\nfn test() {}")
        .error("generic type T: operator '!=' requires a concrete type at an_unbounded_type_variable_reaches_no_bound_s_method:1:43");
    code!("fn bad<T>(a: T, b: T) -> boolean { a < b }\nfn test() {}")
        .error("generic type T: operator '<' requires a concrete type at an_unbounded_type_variable_reaches_no_bound_s_method:1:42");
    // loft#1151 — the SWAPPED spellings name the operator the author WROTE.  `handle_operator`
    // resolves `a > b` as `b < a` and `a >= b` as `b <= a`, and the refusal used to report the
    // rewritten one: a program that typed `>=` was told about `<=`.
    code!("fn bad<T>(a: T, b: T) -> boolean { a > b }\nfn test() {}")
        .error("generic type T: operator '>' requires a concrete type at an_unbounded_type_variable_reaches_no_bound_s_method:1:42");
    code!("fn bad<T>(a: T, b: T) -> boolean { a >= b }\nfn test() {}")
        .error("generic type T: operator '>=' requires a concrete type at an_unbounded_type_variable_reaches_no_bound_s_method:1:43");
    code!("fn bad<T>(a: T, b: T) -> boolean { a <= b }\nfn test() {}")
        .error("generic type T: operator '<=' requires a concrete type at an_unbounded_type_variable_reaches_no_bound_s_method:1:43");
    code!("fn bad<T>(v: T) -> text { v.to_text() }\nfn test() {}")
        .error("generic type T: field access requires a concrete type at an_unbounded_type_variable_reaches_no_bound_s_method:1:37");
}

/// loft#1147 — and the leak does not need the stdlib at all: a user's OWN bounded generic
/// mints the stub, and the unbounded one BESIDE IT in the same file used to borrow it.
/// Measured on the pre-fix build, this program compiled and printed `1 2`; `bad` reached
/// `good`'s `Printable` through nothing but the shared spelling of `T`.
#[test]
fn a_bounded_generic_does_not_lend_its_bound_to_an_unbounded_sibling() {
    code!(
        "fn good<T: Printable>(v: T) -> text { \"{v}\" }\n\
         fn bad<T>(v: T) -> text { \"{v}\" }\n\
         fn test() {}"
    )
    .error(
        "generic type T cannot be formatted \u{2014} `\"{\u{2026}}\"` needs a bound that renders \
            it; write `<T: Printable>` (every built-in satisfies it, and a user type does by \
            defining `fn to_text(self: T) -> text`) at \
            a_bounded_generic_does_not_lend_its_bound_to_an_unbounded_sibling:2:32",
    );
}

// ── fix-tvscope — Type variable namespace ────────────────────────────────────

/// fix-tvscope: defining a struct whose name clashes with a generic type variable
/// produces a clear diagnostic instead of the confusing "Redefined struct T".
#[test]
fn struct_name_clashes_with_type_variable() {
    code!("struct T { v: integer }\nfn test() {}")
        .error("'T' is reserved as a generic type variable \u{2014} choose a different struct name at struct_name_clashes_with_type_variable:1:11");
}

// ── Fix #91 — Circular init detection ────────────────────────────────────────

/// #91: two init fields referencing each other via $ should produce an error.
#[test]
fn circular_init_error() {
    code!("struct Bad {\n  a: integer init($.b),\n  b: integer init($.a),\n}\nfn test() {}")
        .error("circular init dependency: a -> b -> a at circular_init_error:2:3")
        .error("circular init dependency: b -> a -> b at circular_init_error:3:3");
}

// ── C42 — Unknown variable diagnostic ───────────────────────────────────────

/// C42: using an undefined variable name produces a clear error.
#[test]
fn unknown_variable_error() {
    code!("fn test() -> integer { reuslt = 42; result }")
        .error("Unknown variable 'result' — did you mean 'reuslt'? at unknown_variable_error:1:37")
        .warning("Variable reuslt is never read at unknown_variable_error:1:32");
}

// ── P128: File-scope constants accept type annotations (FIXED) ─────────────
//
// `parse_constant` (src/parser/definitions.rs:392) now consumes an optional
// `: type` annotation between the identifier and `=`. The annotation is
// parsed (so the parser accepts the form) but the literal's inferred type
// is the source of truth — a future enhancement could validate the two
// match after dep-list normalisation.
//
// Regression guard: the form must keep parsing without errors.
#[test]
fn p128_constant_with_type_annotation_parses() {
    code!("QUAD: vector<integer> = [1, 2, 3];\nfn test() {}");
    // No .error() calls — parses cleanly.
}

// ── P85b: User type/enum/struct shadowing a stdlib constant ─────────────────
//
// Defining a user type whose name collides with a stdlib constant (e.g.
// `enum E { ... }` collides with `pub E = OpMathEFloat()`) used to produce
// a compiler PANIC like `Cannot change returned type on [164]E to float
// twice was E`.  Both `enum` and `struct` now emit a clear, actionable
// diagnostic naming the conflicting definition's location.
#[test]
fn p85b_enum_shadowing_stdlib_constant_emits_diagnostic() {
    let s = loft::platform::sep_str();
    code!("enum E { Foo, Bar }\nfn test() {}").error(&format!(
        "enum 'E' conflicts with a constant of the same name already defined \
         at default{s}01_code.loft:377:24 — pick a different name \
         at p85b_enum_shadowing_stdlib_constant_emits_diagnostic:1:9"
    ));
}

#[test]
fn p85b_struct_shadowing_stdlib_constant_emits_diagnostic() {
    let s = loft::platform::sep_str();
    code!("struct E { n: integer }\nfn test() {}").error(&format!(
        "struct 'E' conflicts with a constant of the same name already defined \
         at default{s}01_code.loft:377:24 — pick a different name \
         at p85b_struct_shadowing_stdlib_constant_emits_diagnostic:1:11"
    ));
}

#[test]
fn p85b_type_shadowing_stdlib_constant_emits_diagnostic() {
    let s = loft::platform::sep_str();
    code!("type E = integer;\nfn test() {}").error(&format!(
        "type 'E' conflicts with a constant of the same name already defined \
         at default{s}01_code.loft:377:24 — pick a different name \
         at p85b_type_shadowing_stdlib_constant_emits_diagnostic:1:9"
    ));
}

#[test]
fn p85b_constant_shadowing_stdlib_constant_emits_diagnostic() {
    let s = loft::platform::sep_str();
    code!("E = 42;\nfn test() {}").error(&format!(
        "constant 'E' conflicts with a constant of the same name already defined \
         at default{s}01_code.loft:377:24 — pick a different name \
         at p85b_constant_shadowing_stdlib_constant_emits_diagnostic:1:8"
    ));
}

// ── P85c: file-scope-only declarations rejected with a clean diagnostic ─────
//
// Putting `struct`, `enum`, `type`, `interface`, `use`, `pub`, or a named
// `fn name(...)` inside a function body used to produce a cascade of
// confusing errors (`Expect token =`, `Expect constants to be in upper case`,
// `Syntax error: unexpected ...`).  parse_block now detects these keywords
// at the statement boundary and emits a single clear diagnostic.  Lambdas
// (`fn(args) { ... }`) are still allowed because they parse as expressions.
#[test]
fn p85c_struct_inside_fn_emits_diagnostic() {
    code!("fn test() {\n  struct Inner { v: integer }\n  x = 5;\n}").error(
        "'struct' definitions must be at file scope, not inside a function or block \
         at p85c_struct_inside_fn_emits_diagnostic:2:3",
    );
}

#[test]
fn p85c_enum_inside_fn_emits_diagnostic() {
    code!("fn test() {\n  enum Inner { A, B }\n  x = 5;\n}").error(
        "'enum' definitions must be at file scope, not inside a function or block \
         at p85c_enum_inside_fn_emits_diagnostic:2:3",
    );
}

#[test]
fn p85c_named_fn_inside_fn_emits_diagnostic() {
    code!("fn test() {\n  fn inner() -> integer { 5 }\n  x = 5;\n}").error(
        "'fn' definitions must be at file scope, not inside a function or block \
         at p85c_named_fn_inside_fn_emits_diagnostic:2:3",
    );
}

#[test]
fn c61_nested_same_name_loop_rejected() {
    // C61: nested `for i { for i { } }` silently aliases the outer
    // iterator's `#index` companion, causing wrong runtime results.
    // The parser now rejects it with an actionable rename hint.
    code!(
        "fn test() {\n  \
           for i in 0..3 {\n    \
             for i in 10..13 { }\n  \
           }\n\
         }"
    )
    .error(
        "loop variable 'i' shadows the enclosing loop's 'i' — \
         rename the inner loop variable (e.g. inner_i); loft does \
         not support nested same-name loops at c61_nested_same_name_loop_rejected:3:22",
    )
    .warning("Variable i is never read at c61_nested_same_name_loop_rejected:2:18");
}

#[test]
fn c61_local_shadow_rejected() {
    // C61.local: a for-loop variable that would silently clobber a
    // same-named outer local is rejected at parse time with a rename
    // hint.  Unblocked by PROBLEMS.md #139's OpReserveFrame fix, which
    // made it possible to rename stdlib docs without tripping the
    // slot-allocator TOS mismatch on layout changes.
    code!(
        "fn run() -> integer {\n  \
           x = 99;\n  \
           for x in 0..3 { }\n  \
           x\n\
         }"
    )
    .expr("run()")
    .error(
        "loop variable 'x' shadows a local named 'x' — rename the loop \
         variable (e.g. loop_x) or drop the outer `x` if it was a dead \
         placeholder; loft does not block-scope loop variables at \
         c61_local_shadow_rejected:3:18",
    );
}

#[test]
fn c61_local_shadow_renamed_ok() {
    // Regression guard: renaming the loop variable keeps the outer
    // local intact.
    code!(
        "fn run() -> integer {\n  \
           x = 99;\n  \
           for loop_x in 0..3 { x + loop_x; }\n  \
           x\n\
         }"
    )
    .expr("run()")
    .result(Value::Int(99));
}

#[test]
fn c61_local_dropped_outer_ok() {
    // Regression guard: dropping the dead outer placeholder is the
    // other documented fix.  `a` is live only inside the loop.
    code!(
        "fn run() -> integer {\n  \
           t = 0;\n  \
           for a in 1..6 { t += a; }\n  \
           t\n\
         }"
    )
    .expr("run()")
    .result(Value::Int(15));
}

#[test]
fn c61_nested_different_names_ok() {
    // Regression guard: nested loops with *different* names still parse.
    code!(
        "fn run() -> integer {\n  \
           total = 0;\n  \
           for i in 0..3 { for j in 10..13 { total += j + i; } }\n  \
           total\n\
         }"
    )
    .expr("run()")
    .result(Value::Int(108));
}

#[test]
fn c61_sequential_same_name_ok() {
    // Regression guard: sequential same-name loops (non-nested) remain
    // valid — only nested aliasing is rejected.
    code!(
        "fn run() -> integer {\n  \
           a = 0;\n  \
           for i in 0..3 { a += i; }\n  \
           b = 0;\n  \
           for i in 10..13 { b += i; }\n  \
           a + b\n\
         }"
    )
    .expr("run()")
    .result(Value::Int(36));
}

#[test]
fn p85c_lambda_inside_fn_still_works() {
    // Regression guard: lambda expressions (`fn(args) { ... }`) must not
    // trigger the file-scope-only diagnostic.
    code!(
        "fn test() {\n  f = fn(x: integer) -> integer { x * 2 };\n  \
         assert(f(5) == 10, \"lambda\");\n}"
    );
}

// ── L1: error recovery after token failures ─────────────────────────────────
//
// Missing `;` at end of a statement used to produce a cascade of four+
// errors ("Expect token ;", "Expect token }", "Expect constants to be in
// upper case style", "Syntax error: unexpected ..."). The parser now calls
// `Lexer::recover_to(&[";", "}"])` after a failed `token(";")` inside
// `parse_block`, resynchronising to the next statement boundary.
#[test]
fn l1_missing_semicolon_single_diagnostic() {
    // Missing `;` between `x = 1` and `y = 2;`. Should produce exactly one
    // error, not a cascade.
    code!("fn test() {\n  x = 1\n  y = 2;\n  assert(x + y == 3, \"\");\n}")
        .error("Expect token ; at l1_missing_semicolon_single_diagnostic:2:8");
}

#[test]
fn l1_missing_semicolon_in_body_single_diagnostic() {
    code!("fn foo(x: integer) -> integer {\n  y = x + 1\n  y * 2\n}\nfn test() {}")
        .error("Expect token ; at l1_missing_semicolon_in_body_single_diagnostic:2:12");
}

// ── Diagnostic position: the caret belongs to the construct, not to the cursor ──
//
// `Lexer::position` is the scan cursor — the END of the token the parser is HOLDING.
// A check that can only run once a construct is complete (a `const` write, a nullable
// reaching a non-null slot, a capture in a parallel arm) therefore raises with the
// cursor already on the NEXT token, and when that token is on a later line the caret
// lands on a line the construct does not occupy.  Measured before the fix: `a = 42`
// with `}` on its own line reported the const write AT the `}`, and blank lines
// between them carried it further still — the same statement, three different
// answers.  `Lexer::report_pos` attributes such a diagnostic to where the consumed
// source stops instead, so the three layouts below agree.
//
// The assertion is LAYOUT-INDEPENDENCE, not a hand-picked line: all three write the
// offending statement on line 2, so all three must report line 2 — and, since the const
// question became ONE check ahead of the lowering routes rather than one inside each of
// them, the same COLUMN too.  The write is `a = 42`, which ends at column 8, so the caret
// is column 9 whatever terminates the statement.  A hand-guessed
// expectation would only pin the layout it was written for.
#[test]
fn a_diagnostic_names_its_own_line_whatever_follows_it() {
    // Terminated on its own line — the `;` kept the cursor on line 2 by luck, which
    // is why this cell always passed and the two below did not exist.  It read 2:10 while
    // the per-route checks ran after the `;` had been consumed; asked before the routes,
    // the guard sees the same consumed source as the two unterminated layouts below.
    code!("fn f(a: const integer) {\n  a = 42;\n}\nfn test() { f(1); }")
        .error(
            "Cannot modify const parameter 'a'; remove 'const' or use a local copy \
             at a_diagnostic_names_its_own_line_whatever_follows_it:2:9",
        )
        .warning(
            "Parameter a is never read at a_diagnostic_names_its_own_line_whatever_follows_it:1:25",
        );
}

#[test]
fn a_diagnostic_names_its_own_line_with_no_terminator() {
    code!("fn f(a: const integer) {\n  a = 42\n}\nfn test() { f(1); }")
        .error(
            "Cannot modify const parameter 'a'; remove 'const' or use a local copy \
             at a_diagnostic_names_its_own_line_with_no_terminator:2:9",
        )
        .warning(
            "Parameter a is never read at a_diagnostic_names_its_own_line_with_no_terminator:1:25",
        );
}

#[test]
fn a_diagnostic_names_its_own_line_across_blank_lines() {
    // The distance is unbounded: whatever the parser must skip to find the next token
    // is how far the caret used to drift.
    code!("fn f(a: const integer) {\n  a = 42\n\n\n}\nfn test() { f(1); }")
        .error(
            "Cannot modify const parameter 'a'; remove 'const' or use a local copy \
             at a_diagnostic_names_its_own_line_across_blank_lines:2:9",
        )
        .warning(
            "Parameter a is never read at a_diagnostic_names_its_own_line_across_blank_lines:1:25",
        );
}

// A diagnostic that IS about the token the parser is holding keeps that token's
// position, and says so at the site.  These two are the opt-outs, pinned here so a
// later widening of the consumed-source default cannot take them silently: the
// keyword is still the current token when the refusal is raised, and the first token
// of the unreachable statement is what "unreachable code" is about.
#[test]
fn a_current_token_diagnostic_still_names_the_current_token() {
    code!("fn test() {\n  struct Inner { a: integer }\n}").error(
        "'struct' definitions must be at file scope, not inside a function or block \
             at a_current_token_diagnostic_still_names_the_current_token:2:3",
    );
}

// ── A whole-CONSTRUCT check still names the part it is about ────────────────────
//
// These checks can only run once the construct is complete, so `report_pos`'s
// consumed-source default lands on its closing brace — right, but not useful: a
// struct has many fields and "somewhere in this struct" is not an answer. Each site
// holds a better position and now uses it. `scripts/diag_position_audit.py --brace`
// went 19 → 4 → 0 over the corpus across the two passes.
#[test]
fn each_circular_init_cycle_names_its_own_field() {
    // The axis is field COUNT: the cycle runs a -> b -> c -> a among four fields that
    // are not in it, so a caret that merely picked the first field, or the struct, or
    // its brace would be visible here. Each message names its own head field first,
    // and lands on that field's line.
    code!(
        "struct Many {
  p: integer = 1,
  a: integer = $.b,
  q: integer = 2,
  b: integer = $.c,
  r: integer = 3,
  c: integer = $.a,
}
fn test() { m = Many {}; assert(m.p == 1, \"\"); }"
    )
    .error("circular init dependency: a -> b -> c -> a at each_circular_init_cycle_names_its_own_field:3:3")
    .error("circular init dependency: b -> c -> a -> b at each_circular_init_cycle_names_its_own_field:5:3")
    .error("circular init dependency: c -> a -> b -> c at each_circular_init_cycle_names_its_own_field:7:3");
}

#[test]
fn a_generator_tail_value_names_the_tail() {
    code!(
        "fn gsrc(n: integer) -> iterator<integer> { yield n; }
fn gtail() -> iterator<integer> {
  gsrc(1)
}
fn test() { for v in gtail() { assert(v == 1, \"\"); } }"
    )
    .error(
        "a generator's body produces values only through `yield`, so this tail value is \
         discarded — `for v in <generator>() { yield v; }` forwards another generator's \
         values, and a bare statement ends the body \
         at a_generator_tail_value_names_the_tail:3:9",
    );
}

// All four narrowing POSITIONS, in one fixture, because three of them were already
// right and only the return tail was not — a fixture with the broken one alone would
// not show that the fix left the working three alone.
#[test]
fn a_narrowing_names_its_own_line_in_every_position() {
    code!(
        "struct NBox { f: i32 }
fn takes(v: i32) -> integer { 1 }
fn ret_tail(n: integer) -> i32 {
  n
}
fn test() {
  n = 5000000000;
  a: i32 = n;
  b = takes(n);
  c = NBox { f: n };
  d = ret_tail(n);
  assert(a == 0 && b == 0 && c.f == 0 && d == 0, \"\");
}"
    )
    // the return tail — the one that landed on the closing brace
    .error("cannot implicitly narrow integer to i32 (may lose data) — give it a fallback with `?? <value>`, take the checked cast `as i32?` (value or null), or make the value provably fit (a mask, or an `if` range check) at a_narrowing_names_its_own_line_in_every_position:4:3")
    // the three that always worked, pinned so the fix cannot quietly move them
    .error("cannot implicitly narrow integer to i32 (may lose data) — give it a fallback with `?? <value>`, take the checked cast `as i32?` (value or null), or make the value provably fit (a mask, or an `if` range check) at a_narrowing_names_its_own_line_in_every_position:8:14")
    .error("cannot implicitly narrow integer to i32 (may lose data) — give it a fallback with `?? <value>`, take the checked cast `as i32?` (value or null), or make the value provably fit (a mask, or an `if` range check) at a_narrowing_names_its_own_line_in_every_position:9:16")
    .error("cannot implicitly narrow integer to i32 (may lose data) — give it a fallback with `?? <value>`, take the checked cast `as i32?` (value or null), or make the value provably fit (a mask, or an `if` range check) at a_narrowing_names_its_own_line_in_every_position:10:20")
    .warning("Parameter v is never read at a_narrowing_names_its_own_line_in_every_position:2:30");
}

// ── P54 struct-enum blockers (BITING_PLAN § P54) ─────────────────────────
//
// Regression guards for the struct-enum compiler bugs surfaced while
// building JsonValue.  Each bug is tracked as B1..B7 in BITING_PLAN.md.
// Fixed bugs pin the diagnostic-or-success behaviour; open bugs land as
// `#[ignore]`'d with the expected future state so the test goes green
// automatically when the fix lands.

/// B2 was originally: `fn mk() -> Shade { Shade.N }` for a mixed-kind
/// enum (unit + struct-field variants) errored with 'Shade should be
/// Shade on return from block' because the unit variant's declared
/// `Type::Enum(d, false, _)` didn't unify with the struct-enum-upgraded
/// parent's `Type::Enum(d, true, _)`.
///
/// Full fix now lands (`parse_enum_values` post-pass syncs every
/// variant's type to the final parent type), so the compile passes
/// cleanly with no diagnostic.  Runtime use of the returned
/// struct-enum is still blocked by B3/B4 — tracked separately in
/// `tests/issues.rs::p54_b3_single_variant_return`.
#[test]
fn p54_b2_unit_variant_return_compiles() {
    // No .error() or .fatal() — the test passes when compilation
    // produces no diagnostics.
    code!(
        "pub enum Shade { N, V { v: integer } }
fn mk() -> Shade { Shade.N }
fn test() {}"
    );
}

/// `--features emit-repro` writes the assembled test source to
/// `/tmp/loft-repro/<name>.loft` before executing, with a thin
/// `fn main() { test(); }` tail appended so the file is directly
/// runnable via `target/release/loft <path>`.  Test name MUST match
/// the generated filename — `Test::drop` uses `stdext::function_name!()`.
#[cfg(feature = "emit-repro")]
#[test]
fn emit_repro_produces_runnable_loft_file() {
    let path = "/tmp/loft-repro/emit_repro_produces_runnable_loft_file.loft";
    let _ = std::fs::remove_file(path);

    code!(
        "fn run() -> integer {
    1 + 2
}"
    )
    .expr("run()")
    .result(Value::Int(3));

    let contents = std::fs::read_to_string(path).unwrap_or_else(|e| {
        panic!("emit-repro: expected {path} to be written but read failed: {e}")
    });
    assert!(
        contents.contains("fn run() -> integer {"),
        "emit-repro: body missing from {path}:\n---\n{contents}"
    );
    assert!(
        contents.contains("pub fn test()"),
        "emit-repro: test() wrapper missing from {path}:\n---\n{contents}"
    );
    assert!(
        contents.contains("fn main() {") && contents.contains("test();"),
        "emit-repro: runnable `fn main() {{ test(); }}` tail missing from {path}:\n---\n{contents}"
    );
}

// ── Binary-format lint (Phase 2c post-migration hardening) ────────────
//
// `f += <integer>` without an explicit width cast writes 8 bytes post-2c,
// which silently breaks pre-2c binary writers expecting 4-byte fields.
// The parser warns when it sees a bare integer write to a file variable.
// Explicit width casts (`as i32`, `as u8`, `as integer`, etc.) silence
// the warning.

#[test]
fn binary_write_bare_integer_warns() {
    code!("fn test() {\n  f = file(\"/tmp/lint_a.bin\");\n  f += 42;\n}").warning(
        "`f += <integer>` without a width cast writes 8 bytes, and a binary file \
             (BigEndian / LittleEndian) usually wants an exact width at \
             binary_write_bare_integer_warns:3:11",
    );
}

#[test]
fn binary_write_narrow_cast_silent() {
    // `as i32` gives an explicit 4-byte width → no warning.
    code!("fn test() {\n  f = file(\"/tmp/lint_b.bin\");\n  f += 42 as i32;\n}");
}

#[test]
fn binary_write_integer_cast_silent() {
    // `as integer` documents an intentional 8-byte write → no warning.
    code!("fn test() {\n  f = file(\"/tmp/lint_c.bin\");\n  f += 42 as integer;\n}");
}

/// @P315 — a float literal in a `vector<single>` must be a COMPILE ERROR (no
/// silent float→single truncation; the old behaviour silently promoted the
/// declared element type and wrote 8-byte values into 4-byte slots → heap
/// corruption).  The user writes a `single` literal (`1.0f`) or `as single`.
#[test]
fn p315_float_literal_into_single_vector() {
    // One diagnostic per offending element (clean parse recovery, like the
    // sibling "No common type" path) — both float literals are rejected.
    // loft#1146 — the SUFFIX is named first: it is the cure `LOFT.md`'s own example
    // writes and the one that costs no conversion.  The cast stays for non-literals.
    code!("fn test() { v: vector<single> = [1.0, 2.0]; }")
        .error(
            "cannot store float elements in a vector<single> (would lose precision); \
             write `single` literals with the `f` suffix (`[1.0f, 2.0f]`), or cast each \
             element with 'as single' at \
             p315_float_literal_into_single_vector:1:38",
        )
        .error(
            "cannot store float elements in a vector<single> (would lose precision); \
             write `single` literals with the `f` suffix (`[1.0f, 2.0f]`), or cast each \
             element with 'as single' at \
             p315_float_literal_into_single_vector:1:43",
        );
}

/// loft#1146 — every refusal a `single` earns names the `f` literal suffix, which
/// `LOFT.md` calls the FIRST cure and states three times.  Before this, all three
/// prescribed `as` and none of them mentioned the suffix, so a reader who did not
/// already know it could not reach it from the message; the scalar one additionally
/// offered "use a new variable name", which is actively wrong here — the name is not
/// the problem and a second variable earns the same rejection.
#[test]
fn single_refusals_name_the_f_suffix() {
    // The scalar store.  "use a new variable name" is GONE, not merely reordered.
    code!("fn test() { x: single = 1.5; }").error(
        "Variable 'x' cannot change type from single to float; a bare decimal literal is \
         `float` — write it with the `f` suffix (`1.5f`), or cast the value with \
         `as single` at single_refusals_name_the_f_suffix:1:29",
    );
}

/// loft#1146 — a suffix that is not `f` used to be lexed as a separate identifier, so
/// `1.5s` reported "Expect token ;" — a missing semicolon, which names nothing the
/// author did.  The run is consumed, so this is ONE diagnostic and not that one plus a
/// parse error behind it.
#[test]
fn a_bad_numeric_suffix_names_the_suffix_set() {
    code!("fn test() { x = 1.5s; }").error(
        "`s` is not a numeric suffix; write `f` for a `single` literal (`1.5f`) — a bare \
         decimal literal is a `float` at a_bad_numeric_suffix_names_the_suffix_set:1:21",
    );
}

/// GitHub #256 — SUPERSEDED by @PLN17 (three-state boolean).  A boolean now has a
/// real null sentinel (byte 255), so `??` works: `null ?? x` discharges to `x`, and
/// `false` (which is NOT null) stays `false` — no silent fall-through.  The
/// null-check is `lhs == null` (raw `== 255`), not the value's truthiness.
#[test]
fn gh256_bool_null_coalesce_supported() {
    // `false` is not null, so `false ?? true` keeps `false` (the #256 footgun is gone).
    expr!("false ?? true").result(Value::Boolean(false));
}

/// GitHub #253 — `!` on a `not null` non-boolean is always false (`!x` tests
/// "is x null", and a `not null` value is never null).  Warn rather than error:
/// nullable operands are a legitimate null test and boolean `!` is ordinary
/// negation.
#[test]
fn gh253_bang_on_not_null_warns() {
    // Genuinely exercises `not null` (the `!x`-is-always-false diagnostic depends on the
    // value being non-null), so it KEEPS `not null` and asserts the deprecation too.
    code!("fn test() { h: integer not null = 3; if !h { h = 4; } }")
        .advice(
            "`not null` is deprecated and has no effect — a type is non-null by default now at gh253_bang_on_not_null_warns:1:34",
        )
        .warning(
            "'!' on a 'not null' integer is always false — '!x' tests whether x \
             is null, and a 'not null' value is never null at \
             gh253_bang_on_not_null_warns:1:45",
        );
}

/// GitHub #253 companion — `!` on a *nullable* operand is the sanctioned null
/// test (stdlib `min`/`max` use `!both`), so it must NOT warn.
#[test]
fn gh253_bang_on_nullable_is_quiet() {
    code!("fn test() { n: integer = 3; if !n { n = 4; } assert(n == 3, \"ok\"); }");
}

/// A collection element is a plain fn-ref (DESIGN_DECISIONS.md C116, loft#247): a
/// CAPTURING closure headed into one is refused at compile time, with the struct-field
/// route named.  Covers the detectable shapes — a direct capturing lambda, and a call to
/// a function that RETURNS one (`[make(1)]`, a closure factory).
#[test]
fn gh247_capturing_closure_in_vector_rejected_direct() {
    code!("fn test() { k = 3; fs: vector<fn() -> integer> = [fn() -> integer { k * 2 }]; }").error(
        "a capturing closure cannot be stored in a collection: a collection has one element \
         layout and each capture set is its own record shape — hold it in a struct field, or \
         store a non-capturing fn that reads the state from a struct field \
         at gh247_capturing_closure_in_vector_rejected_direct:1:77",
    );
}

#[test]
fn gh247_capturing_closure_in_vector_rejected_call() {
    code!(
        "fn make(k: integer) -> fn() -> integer { fn() -> integer { k * 2 } }
fn test() { fs: vector<fn() -> integer> = [make(1)]; }"
    )
    .error(
        "a capturing closure cannot be stored in a collection: a collection has one element \
         layout and each capture set is its own record shape — hold it in a struct field, or \
         store a non-capturing fn that reads the state from a struct field \
         at gh247_capturing_closure_in_vector_rejected_call:2:52",
    );
}

/// Non-capturing closures and named fn-refs in a vector still work (not rejected).
#[test]
fn gh247_noncapturing_closure_in_vector_ok() {
    code!(
        "fn dbl(x: integer) -> integer { x * 2 }\nfn test() { fs: vector<fn(integer) -> integer> = [dbl, fn(y: integer) -> integer { 7 }]; }"
    );
}

// ── #302 — expression diagnostics point at the offending token's start ───────
// Before the fix the caret landed on the statement terminator (the `;` / past
// the closing `)`); the lexer's current position at detection time, not the
// offending sub-expression's start.  Each case pins the corrected column.

/// An unknown variable as a call argument points at the variable, not the `;`.
#[test]
fn p302_unknown_var_arg_column() {
    code!("fn test() { print(undefinedvar); }")
        .error("Unknown variable 'undefinedvar' at p302_unknown_var_arg_column:1:19");
}

/// A type-mismatched argument points at the argument's start, not the `;`.
#[test]
fn p302_type_mismatch_arg_column() {
    code!("fn test() { print(1 + 2); }")
        .error("expected text, got integer on argument 1 of call to print at p302_type_mismatch_arg_column:1:19");
}

/// An undefined type in a struct literal yields `unknown type '…'` at the type
/// name's start — not a misleading `Expect token ;` mid-`{`.
///
/// @P376 — a genuinely-undefined struct construction (`NoSuchType { … }`) is
/// reported as ONE clean error.  Pass 1 defers the unknown construction (so a
/// real forward / cross-package `Cell { … }` resolves silently in pass 2); when
/// the name is still undefined in pass 2 it is a typo, and the assigned variable
/// is POISONED with `Type::Never` so every downstream read (`print(x)`, field
/// access, the post-parse unknown-type sweep) is silenced — no cascade.
#[test]
fn p302_unknown_type_struct_literal() {
    code!("fn test() { x = NoSuchType { a: 1 }; print(x); }")
        .error("unknown type 'NoSuchType' at p302_unknown_type_struct_literal:1:17");
}

/// @P376 (sibling) — an undefined VARIABLE on an assignment RHS, then used in a
/// format string, used to produce the same 8-error cascade + nested-string
/// fatal as the unknown-struct path.  The assignment poisons the variable to
/// `Never` (pass 2), so only the root "Unknown variable 'qqq'" remains.
#[test]
fn p376_undefined_var_rhs_no_cascade() {
    code!("fn test() { p = qqq; print(\"{p.name}\"); }")
        .error("Unknown variable 'qqq' at p376_undefined_var_rhs_no_cascade:1:17");
}

/// @P376 (sibling) — an undefined FUNCTION call on an assignment RHS.  The
/// failed call leaves its discarded target `Var(p)` as the RHS, so the cascade
/// tail used to be a spurious "Unknown variable 'p'"; the poison + the
/// `code == target` skip leave only the root "Unknown function".
#[test]
fn p376_undefined_fn_rhs_no_cascade() {
    code!("fn test() { p = nofn(1); print(\"{p.name}\"); }")
        .error("Unknown function nofn at p376_undefined_fn_rhs_no_cascade:1:17");
}

/// @P376 (sibling) — a directly interpolated undefined variable.  Returning
/// `Void` from the format expression used to abort mid-placeholder, cascading
/// "Expect token )" + "expected text, got void" + a nested-string fatal; the
/// format expression now poisons to `Never` and parses the `{…}` cleanly.
#[test]
fn p376_undefined_var_in_format_no_cascade() {
    code!("fn test() { print(\"{zzz}\"); }")
        .error("Unknown variable 'zzz' at p376_undefined_var_in_format_no_cascade:1:21");
}

// @PLN87 — `&` is a BIND-SITE link marker ("link this binding instead of copying
// it"), not an assignment target.  `&var = …` / `&o.f = …` is invalid; a linked
// binding is reassigned with a plain `var = …` (no `&`), and a valid RHS link
// (`c = &v[0]`) is unaffected.
#[test]
fn pln87_amp_on_assignment_target_is_error() {
    code!("fn test() { x = 5; print(\"{x}\\n\"); &x = 3; print(\"{x}\\n\"); }")
        .error("`&` cannot appear on the left of an assignment — it marks a binding as a link to its source at the binding site (`x = &src`), not an assignment target; drop the `&` (the binding is already linked) at pln87_amp_on_assignment_target_is_error:1:40");
}

#[test]
fn pln87_amp_on_field_target_is_error() {
    code!("struct O { y: integer } fn test() { o = O { y: 1 }; print(\"{o.y}\\n\"); &o.y = 3; }")
        .error("`&` cannot appear on the left of an assignment — it marks a binding as a link to its source at the binding site (`x = &src`), not an assignment target; drop the `&` (the binding is already linked) at pln87_amp_on_field_target_is_error:1:77");
}

#[test]
fn pln87_amp_on_compound_assign_target_is_error() {
    code!("fn test() { x = 5; print(\"{x}\\n\"); &x += 3; print(\"{x}\\n\"); }")
        .error("`&` cannot appear on the left of an assignment — it marks a binding as a link to its source at the binding site (`x = &src`), not an assignment target; drop the `&` (the binding is already linked) at pln87_amp_on_compound_assign_target_is_error:1:41");
}

// @PLN87 #1 — `&`'s operand must be a PLACE (variable / struct field / vector element),
// never a temporary (literal, computed value, or call result).
#[test]
fn pln87_amp_on_temporary_paren_is_error() {
    code!("fn test() { b = &(1 + 2); print(\"{b}\\n\"); }")
        .error("`&` requires an addressable operand — a variable, struct field, or vector element — not a temporary (a literal, computed value, or call result) at pln87_amp_on_temporary_paren_is_error:1:26");
}

#[test]
fn pln87_amp_on_call_result_is_error() {
    code!("fn mk() -> integer { 5 } fn test() { b = &mk(); print(\"{b}\\n\"); }")
        .error("`&` requires an addressable operand — a variable, struct field, or vector element — not a temporary (a literal, computed value, or call result) at pln87_amp_on_call_result_is_error:1:48");
}

#[test]
fn pln87_amp_on_literal_is_error() {
    code!("fn test() { b = &123; print(\"{b}\\n\"); }")
        .error("`&` requires an addressable operand — a variable, struct field, or vector element — not a temporary (a literal, computed value, or call result) at pln87_amp_on_literal_is_error:1:22");
}

// @PLN87 — `&` is NOT a general operator: valid only as the whole RHS of a binding
// (`a = &b`).  As a call argument or a sub-expression it is an error — a `&` parameter
// is called WITHOUT `&` (the reference comes from the parameter's type).
#[test]
fn pln87_amp_as_call_arg_is_error() {
    code!("fn f(o: &integer) { o = o + 1; } fn test() { x = 5; f(&x); print(\"{x}\\n\"); }")
        .error("`&` is not a general operator — it binds a reference only as the whole right-hand side of an assignment (`a = &b`). Pass a `&` parameter WITHOUT `&` (`f(x)`, the reference comes from the parameter type); do not use `&` in an argument or sub-expression at pln87_amp_as_call_arg_is_error:1:58");
}

#[test]
fn pln87_amp_in_subexpr_is_error() {
    code!("fn test() { a = 5; b = &a + 1; print(\"{b}\\n\"); }")
        .error("`&` is not a general operator — it binds a reference only as the whole right-hand side of an assignment (`a = &b`). Pass a `&` parameter WITHOUT `&` (`f(x)`, the reference comes from the parameter type); do not use `&` in an argument or sub-expression at pln87_amp_in_subexpr_is_error:1:28");
}

// @PLN87 B-Ref-AnnotationOnly — the SWEEP over `&`-position, not one cell.  The guard
// used to decide by peeking the token AFTER the `&`-operand: a `;`/`}` there meant "the
// whole RHS".  That proves nothing FOLLOWS the `&`, never that nothing PRECEDED it, so
// every shape where the `&` is the LAST operand of a larger expression compiled — incl.
// the rule's own named example, `x + &y`.  `pln87_amp_in_subexpr_is_error` missed them
// all because it puts the `&` at the HEAD (`b = &a + 1`), the one sub-expression
// position the peek did catch.  These five cover the positions it did not.
#[test]
fn pln87_amp_as_tail_operand_is_error() {
    code!("fn test() { a = 5; b = 1 + &a; print(\"{b}\\n\"); }")
        .error("`&` is not a general operator — it binds a reference only as the whole right-hand side of an assignment (`a = &b`). Pass a `&` parameter WITHOUT `&` (`f(x)`, the reference comes from the parameter type); do not use `&` in an argument or sub-expression at pln87_amp_as_tail_operand_is_error:1:31");
}

// A COMPOUND assignment is not a bind site: `b += &a` mutates `b`, it does not give `b`
// a reference type, so the `&` has no binding to annotate.
#[test]
fn pln87_amp_as_compound_assign_rhs_is_error() {
    code!("fn test() { a = 3; b = 1; b += &a; print(\"{b}\\n\"); }")
        .error("`&` is not a general operator — it binds a reference only as the whole right-hand side of an assignment (`a = &b`). Pass a `&` parameter WITHOUT `&` (`f(x)`, the reference comes from the parameter type); do not use `&` in an argument or sub-expression at pln87_amp_as_compound_assign_rhs_is_error:1:35");
}

// A block-final `1 + &a` — a tail VALUE, not a binding.  (A bare block-final `&a` keeps
// its own D-bind-7 message below; here the `&` is an operand, so it is the general ban.)
#[test]
fn pln87_amp_in_block_final_expression_is_error() {
    code!("fn tv(a: integer) -> integer { 1 + &a } fn test() { print(\"{tv(5)}\\n\"); }")
        .error("`&` is not a general operator — it binds a reference only as the whole right-hand side of an assignment (`a = &b`). Pass a `&` parameter WITHOUT `&` (`f(x)`, the reference comes from the parameter type); do not use `&` in an argument or sub-expression at pln87_amp_in_block_final_expression_is_error:1:40");
}

// A struct-literal FIELD value — the L7 concern for a record rather than a vector: a
// closing `}` follows the operand, which is exactly what the old peek read as "the
// whole RHS terminates here".
#[test]
fn pln87_amp_in_struct_literal_field_is_error() {
    code!("struct S { x: integer } fn test() { a = 3; s = S { x: &a }; print(\"{s.x}\\n\"); }")
        .error("`&` is not a general operator — it binds a reference only as the whole right-hand side of an assignment (`a = &b`). Pass a `&` parameter WITHOUT `&` (`f(x)`, the reference comes from the parameter type); do not use `&` in an argument or sub-expression at pln87_amp_in_struct_literal_field_is_error:1:59");
}

// A `return &a;` — the `;` terminates, but a return is not a binding either.  It already
// rejected before the head gate, via the D-bind-7 message rather than the general ban:
// the returned expression is parsed as its own statement, and that statement DOES begin
// with `&`, so the bare-reference guard is the one that fires.  "Discards the reference"
// is the accurate reading of a returned `&a`.  Pinned so the sweep stays a sweep.
#[test]
fn pln87_amp_in_return_statement_is_error() {
    code!("fn tr(a: integer) -> integer { return &a; } fn test() { print(\"{tr(5)}\\n\"); }")
        .error("`&` is not a general operator — it binds a reference only as the whole right-hand side of an assignment (`a = &b`); a bare `&a` discards the reference. Drop it, or write `name = &a` to bind one at pln87_amp_in_return_statement_is_error:1:39");
}

// @PLN87 L7 — a reference cannot be smuggled into a data-structure literal: a `&` in a
// collection element hits the general-operator ban, keeping references from outliving
// their source.  (A `&T` struct-FIELD type is likewise rejected — "Attribute … needs
// type or definition" — so a reference cannot be stored in a heap record either.)
#[test]
fn pln87_l7_ref_in_vector_literal_is_error() {
    code!("fn test() { a = 3; v = [&a]; print(\"{v[0]}\\n\"); }")
        .error("`&` is not a general operator — it binds a reference only as the whole right-hand side of an assignment (`a = &b`). Pass a `&` parameter WITHOUT `&` (`f(x)`, the reference comes from the parameter type); do not use `&` in an argument or sub-expression at pln87_l7_ref_in_vector_literal_is_error:1:28");
}

// @PLN87 D-bind-7 — the LAST position the VITAL rule (binding.md B-Ref-AnnotationOnly)
// did not cover: a bare `&a;` STATEMENT.  `&` binds a reference only as an assignment
// RHS; standing alone (or as a block-final value) it discards the reference, which the
// rule forbids — `&` may appear ONLY at a binding.  Caret points at the `&`.
#[test]
fn pln87_d_bind_7_bare_amp_statement_is_error() {
    code!("fn test() { a = 5; &a; print(\"{a}\\n\"); }")
        .error("`&` is not a general operator — it binds a reference only as the whole right-hand side of an assignment (`a = &b`); a bare `&a` discards the reference. Drop it, or write `name = &a` to bind one at pln87_d_bind_7_bare_amp_statement_is_error:1:20");
}

#[test]
fn pln87_d_bind_7_bare_amp_field_statement_is_error() {
    code!("struct O { y: integer } fn test() { o = O { y: 1 }; &o.y; print(\"{o.y}\\n\"); }")
        .error("`&` is not a general operator — it binds a reference only as the whole right-hand side of an assignment (`a = &b`); a bare `&a` discards the reference. Drop it, or write `name = &a` to bind one at pln87_d_bind_7_bare_amp_field_statement_is_error:1:53");
}

// A block-final `&a` (a function/block tail value) is likewise outside a binding —
// rejected, not silently a no-op return.
#[test]
fn pln87_d_bind_7_block_final_amp_is_error() {
    code!("fn mk() -> integer { a = 5; &a } fn test() { print(\"{mk()}\\n\"); }")
        .error("`&` is not a general operator — it binds a reference only as the whole right-hand side of an assignment (`a = &b`); a bare `&a` discards the reference. Drop it, or write `name = &a` to bind one at pln87_d_bind_7_block_final_amp_is_error:1:29");
}

// D-key-1 — a keyed range / partial-key subscript yields a `for`-only iterator
// (`Value::Iter`).  In a VALUE position (`x = coll[lo..hi]`) it used to panic — at
// parse time in `set_loop` (range) or at codegen "Iter should have been rewritten"
// (partial key).  Both now emit a clean diagnostic that aborts the compile.
#[test]
fn keyed_range_slice_in_value_position_is_error() {
    code!("struct Item { nr: integer, val: integer } struct DB { idx: index<Item[nr]> } fn test() { db = DB { idx: [ Item{nr:10,val:1} ] }; x = db.idx[10..=30]; }")
        .error("a keyed range slice is a `for`-loop iterator, not a value — iterate it directly (`for x in coll[lo..hi] { … }`) or materialise a vector with a comprehension (`[for x in coll[lo..hi] { x }]`) at keyed_range_slice_in_value_position_is_error:1:148")
        .warning("Variable x is never read at keyed_range_slice_in_value_position_is_error:1:133");
}

#[test]
fn keyed_partial_key_in_value_position_is_error() {
    code!("struct Item { nr: integer, label: text, val: integer } struct DB { idx: index<Item[nr, label]> } fn test() { db = DB { idx: [ Item{nr:10,label:\"a\",val:1} ] }; x = db.idx[10]; }")
        .error("a keyed partial-key match is a `for`-loop iterator, not a value — iterate it directly (`for x in coll[key] { … }`) or give every key field for a single-record lookup at keyed_partial_key_in_value_position_is_error:1:174")
        .warning("Variable x is never read at keyed_partial_key_in_value_position_is_error:1:163");
}

// @PLN35 Phase 2 (F6 / M-Total): a slice pattern is length-constrained, hence non-total.
// A vector match is exhaustive only if its final arm is total (a `_` or a bare binding);
// a slice-only match with no such arm must be a static error, not a silent typed-null.
#[test]
fn vector_match_not_exhaustive() {
    code!("fn f(v: vector<integer>) -> integer { match v { [a] => a, [a, b] => a + b } }\nfn test() { f([1]); }")
        .error("match on vector is not exhaustive — a slice pattern can fail (a length no arm matches); add a '_ =>' or a bare-binding final arm at vector_match_not_exhaustive:1:78");
}

// @PLN35 Phase 3 (L3.7, P-Multi): comma-separated multi-pattern arms bind the SAME
// captures (D-simple) from whichever variant matched.  The guards below pin the
// D-simple boundary: mismatched capture types, partial name overlap, and the
// combinations deferred to Phase 4 (a guard / a field sub-pattern on such an arm).

// A same-named capture must have the SAME type in every listed pattern.
#[test]
fn multi_pattern_capture_type_mismatch() {
    code!("enum Rec { Ri { k: integer }, Rt { k: text } }\nfn f(r: Rec) -> integer { match r { Ri { k }, Rt { k } => k, _ => 0 } }")
        .error("multi-pattern arm: capture 'k' is text in this pattern but integer in the first — every listed pattern must bind the same captures at the same type at multi_pattern_capture_type_mismatch:2:55");
}

// Partial name overlap (a capture in only some patterns → option<T>) is Phase 4.
#[test]
fn multi_pattern_partial_overlap() {
    code!("enum Rec { Ra { u: integer }, Rb { w: integer } }\nfn f(r: Rec) -> integer { match r { Ra { u }, Rb { w } => u, _ => 0 } }")
        .error("multi-pattern arm: capture 'w' is not bound by the first pattern (partial overlap → option<T> is Phase 4) at multi_pattern_partial_overlap:2:55")
        .error("multi-pattern arm: every listed pattern must bind the same captures (u) at multi_pattern_partial_overlap:2:58");
}

// A guard on a multi-pattern arm (must hold for whichever pattern matched) is Phase 4.
#[test]
fn multi_pattern_guard_deferred() {
    code!("enum Rec { Ra { k: integer }, Rb { k: integer } }\nfn f(r: Rec) -> integer { match r { Ra { k }, Rb { k } if k > 0 => k, _ => 0 } }")
        .error("a guard is not yet supported on a multi-pattern arm (Phase 4) at multi_pattern_guard_deferred:2:67");
}

// A field sub-pattern inside a non-first listed pattern is Phase 4.
#[test]
fn multi_pattern_subpattern_deferred() {
    code!("enum Sub { Sp, Sq }\nenum Rec { Ra { i: Sub }, Rb { i: Sub } }\nfn f(r: Rec) -> integer { match r { Ra { i }, Rb { i: Sp } => 0, _ => 1 } }")
        .error("a field sub-pattern is not yet supported in a multi-pattern arm (Phase 4) at multi_pattern_subpattern_deferred:3:57");
}

// Union exhaustiveness: the listed variants are ALL covered by the multi-pattern
// arm, so a still-missing variant must be reported (not silently accepted).
#[test]
fn multi_pattern_union_not_exhaustive() {
    code!("enum Rec { Ra { k: integer }, Rb { k: integer }, Rc { k: integer } }\nfn f(r: Rec) -> integer { match r { Ra { k }, Rb { k } => k } }")
        .error("match on Rec is not exhaustive — missing: Rc; add the missing variants or a '_ =>' wildcard at multi_pattern_union_not_exhaustive:2:34");
}

// @PLN35 Phase 4 (L3.2, P-Alt): a single-element alternation `( V1 { f } | V2 { f } )`
// in a slice element position.  A same-named capture must unify across branches; a
// name in only some branches promotes to `option<T>` (Phase 4.2 — valid, not an
// error).  So the only capture-level errors are a type clash and an unknown variant.

// A same-named capture must unify across branches.
#[test]
fn alternation_capture_type_mismatch() {
    code!("enum Tk { Ai { n: integer }, At { n: text } }\nfn f(t: vector<Tk>) -> integer { match t { [ (Ai { n } | At { n }) ] => 0, _ => 1 } }")
        .error("alternation capture 'n' is text in one branch but integer in another at alternation_capture_type_mismatch:2:69");
}

// An unknown variant in an alternation branch is a clean error (no panic).
#[test]
fn alternation_bad_variant() {
    code!("enum Tk { Id { n: text }, Str { n: text } }\nfn f(t: vector<Tk>) -> text { match t { [ (Id { n } | Nope { n }) ] => n, _ => \"y\" } }")
        .error("'Nope' is not a variant of Tk at alternation_bad_variant:2:61");
}

// @PLN35 Phase 6.3 — a literal slice element on a struct-enum with no `#lexeme` field is a
// clean error (not a panic): mark a field `#lexeme` or write the variant pattern.
#[test]
fn lexeme_missing() {
    code!("enum Token { Kw { name: text }, Num { value: integer } }\nfn f(v: vector<Token>) -> integer { match v { [ \"x\" ] => 1, _ => -1 } }")
        .error("Token has no `#lexeme` field a text literal can match — mark a field `#lexeme` or write the variant pattern at lexeme_missing:2:54");
}

// A literal whose type cannot match the (scalar) slice element type is rejected.
#[test]
fn slice_literal_type_mismatch() {
    code!("fn f(v: vector<integer>) -> integer { match v { [ \"x\" ] => 1, _ => -1 } }").error(
        "a text literal cannot match a integer slice element at slice_literal_type_mismatch:1:56",
    );
}

// An unknown `#`-annotation on an enum field is a clean error.
#[test]
fn unknown_field_annotation() {
    code!("enum Tok { V { #bogus f: text } }").error(
        "unknown field annotation `#bogus` (expected `#lexeme`) at unknown_field_annotation:1:24",
    );
}

// @PLN35 slice 1 — a scalar repetition `name:Type*` whose Type is not the vector's element
// type is rejected.
#[test]
fn scalar_rep_type_mismatch() {
    code!("fn f(v: vector<integer>) -> integer { match v { [ xs:text* ] => xs.len(), _ => -1 } }")
        .error("a scalar repetition `xs:text*` must match the vector's element type integer at scalar_rep_type_mismatch:1:59");
}

// @PLN35 slice 1 — a `..rest` after a scalar repetition is not yet supported (a clean error,
// not a silent mis-parse).
#[test]
fn scalar_rep_rest_unsupported() {
    code!("fn f(v: vector<integer>) -> integer { match v { [ xs:integer*, .. ] => xs.len(), _ => -1 } }")
        .error("a `..rest` after a scalar repetition `xs:integer*` is not yet supported at scalar_rep_rest_unsupported:1:66");
}

// @PLN35 slice 1 — a non-literal element after a scalar repetition is rejected (recovers to
// `]` so this is the primary error, not a cascade).
#[test]
fn scalar_rep_nonliteral_tail() {
    code!("fn f(v: vector<integer>) -> integer { match v { [ xs:integer*, y ] => xs.len(), _ => -1 } }")
        .error("only literal elements are supported after a scalar repetition `xs:integer*` at scalar_rep_nonliteral_tail:1:65");
}

// @PLN35 slice 2 — a per-iteration capture of a NON-scalar field `( V { heap } )*` is deferred
// (only scalar/text fields project into a vector today).
#[test]
fn field_capture_nonscalar_deferred() {
    code!("enum Box { B { items: vector<integer> } }\nfn f(v: vector<Box>) -> integer { match v { [ ( B { items } )* ] => 1, _ => -1 } }")
        .error("per-iteration capture of the non-scalar field `items` is not yet supported (only scalar/text fields project into a vector) at field_capture_nonscalar_deferred:2:63");
}

// @PLN35 slice 2 — a `{ field }` naming something that is not a field of the run variant.
#[test]
fn field_capture_unknown_field() {
    code!("enum Tok { Num { n: integer } }\nfn f(v: vector<Tok>) -> integer { match v { [ ( Num { nope } )* ] => 1, _ => -1 } }")
        .error("`nope` is not a field of Num at field_capture_unknown_field:2:64");
}

// @PLN35 slice 3 — a fixed (non-`..rest`) tail after a repetition and a `..rest` are still
// mutually exclusive.
#[test]
fn tail_and_rest_rejected() {
    code!("enum Tok { Num { n: integer }, End { e: integer } }\nfn f(v: vector<Tok>) -> integer { match v { [ (Num)*, End { e }, ..rest ] => e + rest.len(), _ => -1 } }")
        .error("a fixed tail after a repetition cannot combine with `..rest` (yet) at tail_and_rest_rejected:2:74");
}

// @PLN35 Phase 7 — streaming `match` over an unsupported element type (a tuple; scalar / text /
// struct-enum DO work) is deferred with a clean error pointing at the collect idiom.
#[test]
fn stream_match_complex_deferred() {
    code!("fn g() -> iterator<(integer, integer)> { yield (1, 2); }\nfn f() -> integer { match g() { [ _ ] => 1, _ => -1 } }")
        .error("streaming `match` over an `iterator<(integer, integer)>` is not yet supported (only scalar, text, or struct-enum element types) — collect it first: `match [for x in <iter> { x }] { … }` at stream_match_complex_deferred:2:32");
}

// @PLN35 PC2 — a sub-rule invocation `[ name: rule ]` in a cursor match must be the WHOLE slice
// pattern for now; mixing it with fixed elements (the running-pos + revert) is deferred.
#[test]
fn subrule_mixing_deferred() {
    code!("enum Tok { Id { x: integer }, LP { x: integer } }\nstruct Cur { src: vector<Tok>, pos: integer }\nstruct N { v: integer }\nfn parse_id(c: Cur) -> N { match c { [ Id { x } ] => N { v: x }, _ => null } }\nfn f(c: Cur) -> integer { match c { [ e: parse_id, y ] => e.v, _ => -1 } }")
        .error("a sub-rule element `e: rule` must currently be the whole slice pattern (mixing a sub-rule with fixed elements is deferred to a follow-up) at subrule_mixing_deferred:5:51");
}

// @PLN35 PC3 — a left-recursive sub-rule grammar (a cycle in the invocation graph) is a COMPILE
// error, not a runtime hang: every cursor sub-rule invocation is at position 0, so a cycle cannot
// consume and would recurse forever.
#[test]
fn subrule_left_recursion() {
    code!("enum Tok { Num { x: integer } }\nstruct Cur { src: vector<Tok>, pos: integer }\nstruct N { v: integer }\nfn expr(c: Cur) -> N { match c { [ Num { x } ] => N { v: x }, [ e: expr ] => e, _ => null } }\nfn f(c: Cur) -> integer { r = match c { [ x: expr ] => x.v, _ => -1 }; r }")
        .error("sub-rule `expr` is left-recursive (expr -> expr): a cursor `match` invokes a sub-rule before consuming any input, so this cycle would recurse forever at subrule_left_recursion:4:68");
}

// @PLN35 PC4 — an invoked sub-rule must be pure: a cursor `match` hoists its call unconditionally
// (runs even when the arm is not taken) and may backtrack over it, so any observable effect leaks.
#[test]
fn subrule_impure_rejected() {
    code!("enum Tok { Id { x: integer }, LP { x: integer } }\nstruct Cur { src: vector<Tok>, pos: integer }\nstruct N { v: integer }\nfn noisy(c: Cur) -> N { print(\"hi\"); match c { [ Id { x } ] => N { v: x }, _ => null } }\nfn f(c: Cur) -> integer { r = match c { [ n: noisy ] => n.v, _ => -1 }; r }")
        .error("sub-rule `noisy` is not pure — a cursor `match` may invoke it speculatively (even when its arm is not taken) and backtrack over it, so its side effects would be observable; a sub-rule must only advance the cursor and return (no I/O, host mutation, or randomness) at subrule_impure_rejected:5:46");
}

// A user type may be named `T` (a name the stdlib uses as a generic type variable):
// verified as a real user program in tests/scripts/generic-typevar-name-usable.loft.
// The fix keys vector types by their element (not by a display name two distinct
// `T`s share), so a user `vector<T>` no longer reuses the marker's size-0 entry.

// Regression: the typedef keyword is `type`, not `typedef`. A typed declaration with an
// UNKNOWN type (`Foo x = 5`, or the common `typedef T = integer` typo) once PANICKED in
// `change_var_type` (`index 65535 into empty variables` — the no-variable sentinel) instead
// of diagnosing. The sentinel guard in `Function::change_var_type` makes it diagnose.
#[test]
fn unknown_type_typed_decl_diagnoses_not_panics() {
    code!("Foo x = 5")
        .error("Expect token = at unknown_type_typed_decl_diagnoses_not_panics:1:6")
        .error(
            "Expect constants to be in upper case style at \
             unknown_type_typed_decl_diagnoses_not_panics:1:6",
        )
        .error("Expect token ; at unknown_type_typed_decl_diagnoses_not_panics:1:10");
}

// @PLN — routing-feedback finding 2: a struct-valued file-scope constant is rejected
// (its record can't be materialised at each use → null on interp, E0308 on native);
// the diagnostic points at the zero-arg-fn idiom that works on both backends.
#[test]
fn struct_valued_constant_rejected() {
    code!("struct Point { x: integer } POINT_NONE = Point { x: 1 }; fn test() { a = POINT_NONE; }")
        .error(
            "a struct-valued constant ('POINT_NONE') is not supported — a record cannot be \
             materialised at each use site (it reads `null` on --interpret and fails to \
             compile on --native).  Wrap it in a zero-argument function instead: \
             `fn point_none() -> Point { … }`, then call `point_none()` at \
             struct_valued_constant_rejected:1:57",
        );
}

// loft#1090 — a vector constant the constant store cannot pre-build is refused at the
// declaration.  It has no fallback: the use site references the pre-built store
// (`OpConstRef`) rather than re-running the initialiser, so an unreported failure reads
// `null` at every reference instead of merely being slower.  A call is the residue the
// fold cannot reduce — `BASE + 1` and `"a" + "b"` are folded and build normally.
#[test]
fn unbuildable_vector_constant_rejected() {
    code!(
        "fn seven() -> integer { 7 }\nC = [seven(), 42];\nfn test() { assert(len(C) == 2, \"n\"); }"
    )
    .error(
        "constant 'C' is built from an element value that is only known at run time, or a \
         field of a kind a constant cannot hold — its elements are pre-built in the \
         constant store from their literal fields, and anything they do not describe reads \
         back empty.  Use a zero-argument function instead: `fn c() -> vector<integer> \
         { … }`, then call `c()` at unbuildable_vector_constant_rejected:2:19",
    );
}

// A text constant builds its value in ONE work buffer, and a constant is pasted at every
// reference — so the paste re-points that buffer onto one the using function owns
// (`rebind_constant_buffer`).  An initialiser that needs more than one buffer, such as a
// constant referenced twice, has no single buffer to re-point; before this it panicked the
// parser inside a formatted string and returned a doubled value (`q2qq2q`) through a local.
// Refuse it at the declaration and name the idiom that does work.
#[test]
fn multi_buffer_text_constant_rejected() {
    code!("B = \"q\"; A = B + \"{1 + 1}\" + B; fn test() { a = A; }").error(
        "text constant 'A' is assembled in a way that cannot be pasted at a use site — a \
         constant is inlined at every reference, and this initialiser builds its value \
         across more than one buffer.  Use a zero-argument function instead: \
         `fn a() -> text { … }`, then call `a()` at multi_buffer_text_constant_rejected:1:32",
    );
}

// A pass-1 LEFTOVER binding must not be reported as an unread variable.
//
// Pass 1 could not resolve the match subject — a FORWARD-declared callee is enough, no
// recursion needed — so it bound the arm to a plain variable named after the field. Pass
// 2, with the callee resolved, bound the arm to its `_mv_<field>` variable and read THAT.
// Variables persist across passes, so the abandoned pass-1 binding survived with
// `uses == 0` and warned about a name the program plainly does read. loft#661 fixed the
// half where the leftover kept an `Unknown` type; this is the half where it got a real
// one, so that test could not see it.
//
// It matters beyond noise: `LOFT_DENY_WARNINGS=1` gates library CI, so a false warning
// fails a build for code that is correct.
#[test]
fn pass1_leftover_binding_is_not_reported_unread() {
    code!(
        "enum LB { LbNil, LbHolder { items: vector<u8> } }
fn fwd() -> LB {
    e = seed();
    match e { LbHolder { items } => { items += [121]; assert(len(items) == 2, \"x\"); }, _ => { } };
    return e;
}
fn seed() -> LB { return LbHolder { items: [120] }; }
fn test() { match fwd() { LbHolder { items } => { assert(len(items) == 2, \"read\"); }, _ => { } }; }"
    );
}

// The other side of the same predicate: a local the body really never reads still warns,
// and so does an unread PARAMETER. A parameter is declared in the SIGNATURE, so "never
// appears in the body" is exactly what an unread one looks like — the leftover exemption
// must not swallow it.
#[test]
fn genuinely_unread_local_and_parameter_still_warn() {
    code!(
        "fn takes(a: integer, b: integer) -> integer { return a; }
fn test() {
    unused = 42;
    used = takes(1, 2);
    assert(used == 1, \"used\");
}"
    )
    .warning("Parameter b is never read at genuinely_unread_local_and_parameter_still_warn:1:46")
    .warning(
        "Variable unused is never read at genuinely_unread_local_and_parameter_still_warn:3:13",
    );
}

// formal/binding.md B-Ref-Reshape — removing from a container while a reference into it is
// OPEN is REFUSED AT COMPILE TIME (maker, 2026-08-05: "The removal of anything from a
// structure (vector for example) that has an open & relation (for us an edge case) should be
// forbidden on compile time").  Together with B-Ref-Alias — a `&` writes through in ALL cases
// — that makes the pair total with no runtime machinery: the one shape where a reference could
// not write through is rejected before it runs.  loft#779; the boundary these are drawn from
// is doc/claude/plans/130-element-view-invalidation/probes/40-reshape-refusal/.
//
// Every REFUSED case below compiled and silently dropped the write before the fix; the
// POSITIVE cases at the end are the ones the refusal must not swallow.

/// B-Ref-Reshape (a) — the removal does not even move the linked element, and is still refused:
/// the rule is about an OPEN reference, not about whether this particular removal would have
/// invalidated it.  Deciding otherwise needs the removal index and the view index, which are
/// usually only known at run time.
#[test]
fn b_ref_reshape_removal_under_open_amp_link_is_error() {
    code!(
        "struct Box { n: integer } \
         fn test() { v = [Box { n: 11 }, Box { n: 22 }, Box { n: 33 }]; \
           c = &v[0]; v.remove(2); c.n = 99; print(\"{v[0].n}\\n\"); }"
    )
    .error(
        "cannot remove from `v` while `c` references a place inside it — a removal renumbers \
         the remaining elements, so a write through `c` would no longer reach the element it \
         names. Move it after the last use of `c`, or bind without `&` to work on a copy at \
         b_ref_reshape_removal_under_open_amp_link_is_error:1:1",
    );
}

/// B-Ref-Reshape (b) — the linked element MOVES (index 2 → 1).  Refused for the same reason,
/// which is why no follow-the-element machinery is needed.
#[test]
fn b_ref_reshape_removal_moving_linked_element_is_error() {
    code!(
        "struct Box { n: integer } \
         fn test() { v = [Box { n: 11 }, Box { n: 22 }, Box { n: 33 }]; \
           c = &v[2]; v.remove(0); c.n = 99; print(\"{v[1].n}\\n\"); }"
    )
    .error(
        "cannot remove from `v` while `c` references a place inside it — a removal renumbers \
         the remaining elements, so a write through `c` would no longer reach the element it \
         names. Move it after the last use of `c`, or bind without `&` to work on a copy at \
         b_ref_reshape_removal_moving_linked_element_is_error:1:1",
    );
}

/// B-Ref-Reshape (c) — the removal is in the CALLEE, through a `&` parameter: loft#779's own
/// repro, and @PLN130 probe 26, carried as a known-FAIL through the whole plan.  It was the
/// cell with no diagnostic of any kind.
#[test]
fn b_ref_reshape_callee_removal_under_amp_param_is_error() {
    code!(
        "struct Box { n: integer } \
         fn shift(target: &Box, all: &vector<Box>) { all.remove(0); target.n = 99; } \
         fn test() { v = [Box { n: 11 }, Box { n: 22 }, Box { n: 33 }]; \
           shift(v[2], v); print(\"{v[1].n}\\n\"); }"
    )
    .error(
        "cannot pass both `v` and a reference into it to `shift` — `shift` removes from `all`, \
         which renumbers the remaining elements while `target` still references one, so a write \
         through `target` would be lost. Pass the INDEX instead and read the element again \
         after the removal at b_ref_reshape_callee_removal_under_amp_param_is_error:1:1",
    )
    .advice(
        "`&` on parameter `target` only slows it down here — a `&`-reference is \
         double-indirect on every access, and field mutation already propagates to \
         the caller without it at b_ref_reshape_callee_removal_under_amp_param_is_error:1:70",
    );
}

/// B-Ref-Reshape (d) — the same call with the `&` DROPPED from the view parameter.
///
/// Refused too, and that is measured rather than assumed: a plain struct parameter aliases the
/// caller's element exactly as a `&` one does, so this loses the same write.  loft's own
/// `warn_redundant_amp` advice tells authors the `&` here is pointless — if only the `&`
/// spelling were refused, taking that advice would trade a compile error for a silent lost
/// write.  (Probe 40 cell X9 is the measurement: `fn w(t: Box) { t.n = 99 }` called as
/// `w(v[2])` writes 99 into the caller's `v`.)
#[test]
fn b_ref_reshape_callee_removal_under_plain_param_is_error() {
    code!(
        "struct Box { n: integer } \
         fn shift(target: Box, all: &vector<Box>) { all.remove(0); target.n = 99; } \
         fn test() { v = [Box { n: 11 }, Box { n: 22 }, Box { n: 33 }]; \
           shift(v[2], v); print(\"{v[1].n}\\n\"); }"
    )
    .error(
        "cannot pass both `v` and a reference into it to `shift` — `shift` removes from `all`, \
         which renumbers the remaining elements while `target` still references one, so a write \
         through `target` would be lost. Pass the INDEX instead and read the element again \
         after the removal at b_ref_reshape_callee_removal_under_plain_param_is_error:1:1",
    );
}

/// B-Ref-Reshape (e) — the removal is TWO calls down.
///
/// `removed_ref_params` closes over the call graph rather than looking one frame deep, so
/// extracting the removal into a helper does not make the refusal disappear.  Without the
/// closure this is the shape that would let a refactor silently reinstate the lost write.
#[test]
fn b_ref_reshape_removal_two_frames_down_is_error() {
    code!(
        "struct Box { n: integer } \
         fn inner(all: &vector<Box>) { all.remove(0); } \
         fn shift(target: &Box, all: &vector<Box>) { inner(all); target.n = 99; } \
         fn test() { v = [Box { n: 11 }, Box { n: 22 }, Box { n: 33 }]; \
           shift(v[2], v); print(\"{v[1].n}\\n\"); }"
    )
    .error(
        "cannot pass both `v` and a reference into it to `shift` — `shift` removes from `all`, \
         which renumbers the remaining elements while `target` still references one, so a write \
         through `target` would be lost. Pass the INDEX instead and read the element again \
         after the removal at b_ref_reshape_removal_two_frames_down_is_error:1:1",
    )
    .advice(
        "`&` on parameter `target` only slows it down here — a `&`-reference is \
         double-indirect on every access, and field mutation already propagates to \
         the caller without it at b_ref_reshape_removal_two_frames_down_is_error:1:117",
    );
}

/// B-Ref-Reshape (f) — a `&` link in this frame, and the removal one call down.
///
/// Neither half is visible to the other on its own: the caller sees no removal, the callee sees
/// no link.  The fact that closes the gap is *"this callee removes from `&` parameter k"*.
#[test]
fn b_ref_reshape_callee_removal_under_local_amp_link_is_error() {
    code!(
        "struct Box { n: integer } \
         fn drop_last(all: &vector<Box>) { all.remove(2); } \
         fn test() { v = [Box { n: 11 }, Box { n: 22 }, Box { n: 33 }]; \
           c = &v[0]; drop_last(v); c.n = 99; print(\"{v[0].n}\\n\"); }"
    )
    .error(
        "cannot call `drop_last` while `c` references a place inside `v` — `drop_last` would \
         remove from `v`, and a removal renumbers the remaining elements, so a write through \
         `c` would no longer reach the element it names. Move the call after the last use of \
         `c`, or bind without `&` to work on a copy at \
         b_ref_reshape_callee_removal_under_local_amp_link_is_error:1:1",
    );
}

/// B-Ref-Reshape (g) — writing a KEY field through a `&` reference into a keyed collection.
///
/// The third disturbance, and the odd one out: there is no liveness question (the key write IS
/// the use) and no "move it later" remedy, because the write itself is what destroys the place.
/// Changing a key would leave the element reachable by no key, so the write cannot reach the
/// collection — and a `&` may not be quietly turned into a copy, so it is refused.
#[test]
fn b_ref_reshape_rekey_through_amp_link_is_error() {
    code!(
        "struct Elm { key: integer, tag: integer } \
         fn test() { s: sorted<Elm[key]> = [Elm { key: 10, tag: 111 }, Elm { key: 30, tag: 333 }]; \
           c = &s[30]; c.key = 5; print(\"{s[5].tag}\\n\"); }"
    )
    .error(
        "cannot write the key field `key` through `c` — `key` is one of `s`'s keys, and changing \
         it would leave the element reachable by no key, so the write cannot reach `s`. \
         Re-insert with `s[key] = value`, or bind without `&` to work on a copy at \
         b_ref_reshape_rekey_through_amp_link_is_error:1:155",
    );
}

/// B-Ref-Reshape (h) — REPLACING the container while a `&` reference into it is live.
///
/// Nothing about the container's shape changed; the NAME stopped meaning the store, so the
/// reference points at a place that is no longer anybody's. Same verdict, different remedy
/// wording from the removal cause.
#[test]
fn b_ref_reshape_container_reassign_under_amp_link_is_error() {
    code!(
        "struct Box { n: integer } struct Mid { inner: Box } \
         fn test() { bx = Mid { inner: Box { n: 11 } }; c = &bx.inner; \
           bx = Mid { inner: Box { n: 22 } }; c.n = 99; print(\"{bx.inner.n}\\n\"); }"
    )
    .error(
        "cannot give `bx` a new value while `c` references a place inside it — `c` names a place \
         inside `bx`, and replacing `bx` leaves that place with nothing to point at. Move it \
         after the last use of `c`, or bind without `&` to work on a copy at \
         b_ref_reshape_container_reassign_under_amp_link_is_error:1:1",
    );
}

/// The POSITIVE cell for the re-key arm: a NON-key field written through the same `&` reference
/// is untouched. The trigger is the key, not the collection — widening it would take
/// write-through away from keyed elements generally.
#[test]
fn b_ref_reshape_non_key_field_through_amp_link_still_writes_through() {
    code!(
        "struct Elm { key: integer, tag: integer } \
         fn check() -> integer { \
           s: sorted<Elm[key]> = [Elm { key: 10, tag: 111 }, Elm { key: 30, tag: 333 }]; \
           c = &s[30]; c.tag = 99; s[30].tag }"
    )
    .expr("check()")
    .result(Value::Int(99));
}

/// The POSITIVE cell the refusal must not swallow: a link that is DEAD before the removal is
/// no conflict.  Liveness is the condition, not existence — the rustc rule.
#[test]
fn b_ref_reshape_dead_amp_link_before_removal_still_compiles() {
    code!(
        "struct Box { n: integer } \
         fn check() -> integer { v = [Box { n: 11 }, Box { n: 22 }, Box { n: 33 }]; \
           c = &v[0]; c.n = 99; v.remove(2); v[0].n }"
    )
    .expr("check()")
    .result(Value::Int(99));
}

/// The second POSITIVE cell: the callee removes, but from a DIFFERENT container than the one
/// the reference names.  Relating the two arguments is the whole precision of the check — a
/// rule keyed on "a callee that removes" alone would reject this.
#[test]
fn b_ref_reshape_reference_into_other_container_still_compiles() {
    code!(
        "struct Box { n: integer } \
         fn shift(target: &Box, all: &vector<Box>) { all.remove(0); target.n = 99; } \
         fn check() -> integer { v = [Box { n: 11 }, Box { n: 22 }]; \
           w = [Box { n: 44 }, Box { n: 55 }]; shift(w[1], v); w[1].n }"
    )
    .expr("check()")
    .advice(
        "`&` on parameter `target` only slows it down here — a `&`-reference is \
         double-indirect on every access, and field mutation already propagates to \
         the caller without it at b_ref_reshape_reference_into_other_container_still_compiles:1:70",
    )
    .result(Value::Int(99));
}

/// The third POSITIVE cell: the callee removes from a container it OWNS, so nothing the caller
/// holds is renumbered.  Guards the `RefVar` test in the call-graph closure — dropping it would
/// propagate a local's reshape up to every caller.
#[test]
fn b_ref_reshape_callee_local_removal_still_compiles() {
    code!(
        "struct Box { n: integer } \
         fn own(target: &Box) { mine = [Box { n: 44 }, Box { n: 55 }]; mine.remove(0); \
           target.n = 99 + mine[0].n; } \
         fn check() -> integer { v = [Box { n: 11 }, Box { n: 22 }]; own(v[1]); v[1].n }"
    )
    .expr("check()")
    .advice(
        "`&` on parameter `target` only slows it down here — a `&`-reference is \
         double-indirect on every access, and field mutation already propagates to \
         the caller without it at b_ref_reshape_callee_local_removal_still_compiles:1:49",
    )
    .result(Value::Int(154));
}

/// The POSITIVE cell the refusal must not swallow: a link that is DEAD before the removal is
/// no conflict.  Liveness is the condition, not existence — the rustc rule.
///
/// FIXED (F9 step 1).  It was a REGRESSION @PLN130 F2 introduced on this branch rather than a
/// pre-existing gap: the installed mainline binary (which has no materialise) answers 99, and
/// F2 answered 11.  F2's reshape path keyed on the CONTAINER and was order-blind, so it
/// materialised any view of a container reshaped ANYWHERE in the function — even one dead long
/// before the removal.  Two things followed, both on @PLN130's own closure bar: a write that
/// used to land was LOST, and the advice claimed "`v` is modified while `c` is in use" when
/// `c` was not in use at all.
///
/// `collect_views_to_materialise` now walks both causes in source order and condemns a view
/// only where it is USED after the disturbance.  The boundary is
/// `probes/39-f2-liveness-boundary.loft`; the CI form is
/// `tests/scripts/148-view-liveness-across-reshape.loft`.
#[test]
fn f2_dead_view_before_removal_keeps_writing_through() {
    code!(
        "struct Box { n: integer } \
         fn check() -> integer { v = [Box { n: 11 }, Box { n: 22 }, Box { n: 33 }]; \
           c = v[0]; c.n = 99; v.remove(2); v[0].n }"
    )
    .expr("check()")
    .result(Value::Int(99));
}

/// loft#874 — a key field the ELEMENT type does not have used to be an ICE.
///
/// `Data::attr` answers `usize::MAX` for a name it cannot find and `set_mutable`
/// handed that straight to `attributes[…]`, so an ordinary typo panicked with a Rust
/// source location and a caret on a line that was correct as written. The way in that
/// was reported is the `hash<K, V>` spelling every other language uses, but a plain
/// misspelling reaches it too — the same defect, and the far more likely one.
#[test]
fn keyed_collection_unknown_key_field_is_a_diagnostic_not_an_ice() {
    code!("struct At { ca_key: integer, ca_at: integer }\nstruct W { idx: hash<At[ca_kye]> }")
        .error(
            "Field `idx`: `ca_kye` is not a field of `At`, so it cannot be a key — did you \
             mean `ca_key`? A keyed collection names its keys as FIELDS OF ITS ELEMENT — \
             write `hash<Element[key_field]>`, not `hash<key, Element>` at \
             keyed_collection_unknown_key_field_is_a_diagnostic_not_an_ice:2:11",
        );
}

/// The same guard for a name too far off to suggest: the message lists what IS
/// available rather than going quiet, because the user's model of the syntax is what
/// is wrong and a bare "unknown field" would leave it intact.
#[test]
fn keyed_collection_unknown_key_field_lists_the_fields_when_it_cannot_suggest() {
    code!("struct At { ca_key: integer, ca_at: integer }\nstruct W { idx: sorted<At[zzzzzz]> }")
        .error(
            "Field `idx`: `zzzzzz` is not a field of `At`, so it cannot be a key — the fields \
             of `At` are: ca_key, ca_at. A keyed collection names its keys as FIELDS OF ITS \
             ELEMENT — write `hash<Element[key_field]>`, not `hash<key, Element>` at \
             keyed_collection_unknown_key_field_lists_the_fields_when_it_cannot_suggest:2:11",
        );
}

/// loft#923 — a KEYED collection cannot be a vector ELEMENT, and the refusal
/// lands where it is declared.
///
/// Every route to using one failed. A literal element types as the CONTENT
/// struct, so `vh += [[E{…}]]` was a type error; appending a keyed LOCAL walked
/// off the type table; and `--native` never got that far, because the element
/// type was never created and the generated `init()` named a binding no line
/// made (rustc E0425). So the type could be declared, and only declared — which
/// is why it is refused rather than given storage, the same call
/// DESIGN_DECISIONS C113 makes for two `index` members over one key.
///
/// The message names the KIND (`a `hash` cannot be…`) rather than the full type.
/// The keys are not what is wrong here — no keyed collection of any shape has an
/// element form — so naming them would only add width to a refusal that applies
/// whatever they are.
///
/// It used to have a second reason: `Type::name` rendered a keyed type's key list
/// in the schema's debug spelling (`sorted<E,[("k", true)]>`) rather than the
/// source's (`sorted<E[k]>`). That is fixed (loft#956 carries it, where the same
/// string reached a `reduce` refusal), so the workaround is no longer load-bearing
/// — the choice above is.
#[test]
fn keyed_collection_as_a_vector_element_is_refused() {
    code!("struct Ent { k: integer, v: integer }\nfn test() { vh: vector<hash<Ent[k]>> = []; }")
        .error(
            "a `hash` cannot be a vector ELEMENT — a keyed collection has no element form \
             anything can write, so `vector<hash<…>>` could only ever be declared and stay \
             empty. Hold it in a struct and make a vector of THAT: the extra record is what \
             the element would have been anyway. at \
             keyed_collection_as_a_vector_element_is_refused:2:39",
        );
}

/// loft#1275 — a NAMED method required at two arities by one bound set, which stays refused
/// after the stub key gained the arity.
///
/// The key carries the arity now, so two signatures of one name are two stubs and an OPERATOR
/// reaches both: `-` is unary or binary by its SYNTAX, so `call_op` asks for the exact one.
/// A method call cannot: it resolves its RECEIVER before its arguments are parsed, so
/// `x.sizer()` has no arity to ask with. Refusing at the declaration names both arities and
/// the cure; letting it through would resolve one of them by accident.
#[test]
fn one_bound_set_cannot_require_two_signatures_of_one_method() {
    code!(
        "interface HasSize1 { fn sizer(self: Self) -> integer }\n\
         interface HasSize2 { fn sizer(self: Self, scale: integer) -> integer }\n\
         struct A1301 { v: integer }\n\
         fn sizer(self: A1301) -> integer { self.v }\n\
         fn one<T: HasSize1 + HasSize2>(x: T) -> integer { x.sizer() }\n\
         fn test() { }"
    )
    .error(
        "the bounds on 'T' require two different 'sizer' — one taking 1 parameter(s) and one \
         taking 2 — and a method call resolves its receiver before its arguments, so only one \
         of them can be reached. Bound 'T' by the interface that declares the one this body \
         calls, and give the other its own generic at \
         one_bound_set_cannot_require_two_signatures_of_one_method:5:40",
    );
}

/// loft#1275's own reproducer, and the case that CLOSED `formal/interfaces.md` `D-gen-4`.
///
/// `(G-Iface)` calls an interface a set of SIGNATURES, so declaring `-` at both arities asks
/// for two requirements and not one. `-` desugars to `OpMin` either way, and while the stub
/// key spelled only the NAME the second declaration was silently dropped — the interface
/// carried one `OpMin`, and `a - b` under any bound was refused with *"operator 'Min' requires
/// a concrete type"*. The key carries the arity now, so both are reachable and this compiles
/// and runs.
#[test]
fn one_interface_may_declare_both_arities_of_minus() {
    code!(
        "interface SubNeg { op - (self: Self, other: Self) -> Self\n\
         op - (self: Self) -> Self }\n\
         fn both<T: SubNeg>(a: T, b: T) -> T { -(a - b) }"
    )
    .expr("both(10, 3)")
    .result(Value::Int(-7));
}

/// loft#1301 / loft#1300 — two generic headers that both write `T` bind two DIFFERENT
/// variables, and each is judged against its own bounds.
///
/// `formal/interfaces.md` `(G-Gen)` says a header *introduces* its type variable, so the
/// binding is per-header. The implementation gave the spelling file scope: one placeholder
/// stood for every `T`, and the bound-method stubs hang off that placeholder. A stub carries a
/// SIGNATURE, so whichever header was declared first owned it and the second's calls were
/// checked against a parameter list its author never wrote — order-dependently. Renaming one
/// variable to `U` cured it, which is what named the axis.
///
/// The pair below is the reported reproducer; the full matrix — the two arities of `-`, a
/// same-arity different-parameter-type pair, a same-parameters different-return pair, and the
/// sharing that must survive — is
/// `tests/scripts/1300-a-generic-header-binds-its-own-type-variable.loft`.
#[test]
fn two_headers_writing_the_same_type_variable_are_two_variables() {
    code!(
        "interface HasSize1 { fn sizer(self: Self) -> integer }\n\
         interface HasSize2 { fn sizer(self: Self, scale: integer) -> integer }\n\
         struct A1301 { v: integer }\n\
         struct B1301 { v: integer }\n\
         fn sizer(self: A1301) -> integer { self.v }\n\
         fn sizer(self: B1301, scale: integer) -> integer { self.v * scale }\n\
         fn one<T: HasSize1>(x: T) -> integer { x.sizer() }\n\
         fn two<T: HasSize2>(x: T) -> integer { x.sizer(10) }\n\
         fn test() -> integer { return one(A1301 { v: 7 }) + two(B1301 { v: 7 }); }"
    )
    .result(Value::Int(77));
    // The declaration ORDER was what decided which header owned the stub, so the swap is the
    // cell that says the cure is not order-sensitive either.
    code!(
        "interface HasSize1 { fn sizer(self: Self) -> integer }\n\
         interface HasSize2 { fn sizer(self: Self, scale: integer) -> integer }\n\
         struct A1301 { v: integer }\n\
         struct B1301 { v: integer }\n\
         fn sizer(self: A1301) -> integer { self.v }\n\
         fn sizer(self: B1301, scale: integer) -> integer { self.v * scale }\n\
         fn two<T: HasSize2>(x: T) -> integer { x.sizer(10) }\n\
         fn one<T: HasSize1>(x: T) -> integer { x.sizer() }\n\
         fn test() -> integer { return one(A1301 { v: 7 }) + two(B1301 { v: 7 }); }"
    )
    .result(Value::Int(77));
}

/// The CONTROLS, and they are what two earlier attempts at this diagnostic failed.
///
/// Sharing a stub is the NORM: same interface twice, or two interfaces declaring the same
/// method the SAME way, legitimately share one placeholder and one stub. A predicate that
/// compared the STUB against an interface method rather than child-to-child reported a
/// conflict on `default/01_code.loft` itself — `assert_eq` and `assert_ne` share
/// `AssertValue: Equatable + Printable` — and refused every program. Different method NAMES
/// never collide at all.
///
/// The third cell below was the WORKAROUND while two headers writing `T` shared a stub; it
/// now answers the same as writing `T` twice, and stays as the control that says a renamed
/// variable did not become a second class of thing.
#[test]
fn legitimate_stub_sharing_is_not_a_conflict() {
    // Same method name, same signature, two generics both using `T`.
    code!(
        "interface Q1 { fn gamma(self: Self) -> integer }\n\
         interface Q2 { fn gamma(self: Self) -> integer }\n\
         struct B1301 { v: integer }\n\
         fn gamma(self: B1301) -> integer { self.v }\n\
         fn one<T: Q1>(x: T) -> integer { x.gamma() }\n\
         fn two<T: Q2>(x: T) -> integer { x.gamma() + 1 }\n\
         fn test() -> integer { return one(B1301 { v: 7 }) + two(B1301 { v: 7 }); }"
    )
    .result(Value::Int(15));
    // Different method names, two generics both using `T`.
    code!(
        "interface P1 { fn alpha(self: Self) -> integer }\n\
         interface P2 { fn beta(self: Self, n: integer) -> integer }\n\
         struct C1301 { v: integer }\n\
         fn alpha(self: C1301) -> integer { self.v }\n\
         fn beta(self: C1301, n: integer) -> integer { self.v * n }\n\
         fn one<T: P1>(x: T) -> integer { x.alpha() }\n\
         fn two<T: P2>(x: T) -> integer { x.beta(10) }\n\
         fn test() -> integer { return one(C1301 { v: 7 }) + two(C1301 { v: 7 }); }"
    )
    .result(Value::Int(77));
    // And the cure the message names actually works.
    code!(
        "interface R1 { fn delta(self: Self) -> integer }\n\
         interface R2 { fn delta(self: Self, n: integer) -> integer }\n\
         struct D1301 { v: integer }\n\
         fn delta(self: D1301) -> integer { self.v }\n\
         fn one<T: R1>(x: T) -> integer { x.delta() }\n\
         fn two<U: R2>(x: U) -> integer { x.delta(1) }\n\
         fn test() -> integer { return one(D1301 { v: 7 }); }"
    )
    .result(Value::Int(7));
}

/// loft#1298 — the same refusal for the spelling that writes NO type.
///
/// The declaration check above was described as *"the one chokepoint every `vector<…>`
/// element passes through"*, and it is not: an INFERRED literal never writes the type, so
/// `hs = [mk(1), mk(2)]` where `mk` returns a `hash<E[k]>` reached no check at all. It
/// panicked the interpreter instead — the element copy landed on `u16::MAX`, which the
/// source-free bit masks to `0x7FFF`, an index into an 85-row type table.
///
/// Refused rather than made to work, and that was MEASURED rather than assumed: giving the
/// element the collection's own registered row gets past the type table and then corrupts
/// the block chain (*"block chain is malformed … a block owning no words"*, then a divide by
/// zero), so the storage has no room for a keyed element either. The message's own cure —
/// hold it in a struct — is verified to work.
#[test]
fn an_inferred_keyed_vector_element_is_refused_too() {
    code!(
        "struct Ent { k: integer, v: integer }\n\
         fn mk() -> hash<Ent[k]> { h: hash<Ent[k]> = []; h }\n\
         fn test() { hs = [mk(), mk()]; }"
    )
    .error(
        "a `hash` cannot be a vector ELEMENT — a keyed collection has no element form \
         anything can write, so `vector<hash<…>>` could only ever be declared and stay \
         empty. Hold it in a struct and make a vector of THAT: the extra record is what \
         the element would have been anyway. at \
         an_inferred_keyed_vector_element_is_refused_too:3:30",
    );
}

/// The CONTROL for the refusal above: a keyed literal built THROUGH a keyed destination is
/// a different construct (its elements are the CONTENT struct, not collections) and must
/// still build, as must a nested VECTOR literal beside it. A refusal that fired on either
/// would satisfy the test above and take a working spelling with it.
#[test]
fn a_keyed_destination_literal_and_a_nested_vector_still_build() {
    code!(
        "struct Ent { k: integer, v: integer }\n\
         fn test() -> integer { h: hash<Ent[k]> = [Ent { k: 1, v: 7 }, Ent { k: 2, v: 8 }]; \
         vv = [[1, 2], [3, 4]]; return len(h) + len(vv); }"
    )
    .result(Value::Int(4));
}

/// Every keyed kind, not just the one that was reported: `spatial` and `trie`
/// reach the same dead end, and a rule that names three of five kinds is how
/// loft#922's field-replace path left two of them broken.
#[test]
fn every_keyed_kind_is_refused_as_a_vector_element() {
    code!("struct Ent { k: integer, v: integer }\nfn test() { vh: vector<sorted<Ent[k]>> = []; }")
        .error(
            "a `sorted` cannot be a vector ELEMENT — a keyed collection has no element form \
             anything can write, so `vector<sorted<…>>` could only ever be declared and stay \
             empty. Hold it in a struct and make a vector of THAT: the extra record is what \
             the element would have been anyway. at \
             every_keyed_kind_is_refused_as_a_vector_element:2:41",
        );
    code!("struct Ent { k: integer, v: integer }\nfn test() { vh: vector<index<Ent[k]>> = []; }")
        .error(
            "a `index` cannot be a vector ELEMENT — a keyed collection has no element form \
             anything can write, so `vector<index<…>>` could only ever be declared and stay \
             empty. Hold it in a struct and make a vector of THAT: the extra record is what \
             the element would have been anyway. at \
             every_keyed_kind_is_refused_as_a_vector_element:2:40",
        );
    code!(
        "struct Pt { x: integer, y: integer }\nfn test() { vh: vector<spatial<Pt[x, y]>> = []; }"
    )
    .error(
        "a `spatial` cannot be a vector ELEMENT — a keyed collection has no element form \
             anything can write, so `vector<spatial<…>>` could only ever be declared and stay \
             empty. Hold it in a struct and make a vector of THAT: the extra record is what \
             the element would have been anyway. at \
             every_keyed_kind_is_refused_as_a_vector_element:2:44",
    );
    code!("struct Wd { nm: text, v: integer }\nfn test() { vh: vector<trie<Wd[nm]>> = []; }")
        .error(
            "a `trie` cannot be a vector ELEMENT — a keyed collection has no element form \
             anything can write, so `vector<trie<…>>` could only ever be declared and stay \
             empty. Hold it in a struct and make a vector of THAT: the extra record is what \
             the element would have been anyway. at \
             every_keyed_kind_is_refused_as_a_vector_element:2:39",
        );
}

/// loft#1266's sibling from the Safety-chapter review — the enum variant limit.
///
/// A plain enum is one byte and both ends of it are reserved: `0` is the undefined value
/// the variants are numbered away from, and `255` is the null sentinel every scalar type
/// has (`OpConvBoolFromEnum` is `@v1 != 255 && @v1 != 0`).  So 254 variants is the whole
/// domain, and the guard used to fire one variant too late in a way that had no upper
/// bound on the damage: 255 variants COMPILED and the last one answered `null` while
/// `match` sent it to the wildcard; 256 variants overflowed the `u8` — an internal
/// compiler error under debug assertions, a wrap to the reserved `0` without them, and the
/// ICE named a position inside `default/05_coroutine.loft` rather than the enum.
///
/// The reading half — that 254 variants still WORK and the last one is an ordinary value —
/// is `tests/scripts/the-reference-safety-traps-are-what-it-catalogues.loft`, which cannot
/// hold this cell because the fixed compiler refuses to parse it.
#[test]
fn enum_variant_limit_refuses_the_255th() {
    let variants: Vec<String> = (0..255).map(|i| format!("  V{i},")).collect();
    let src = format!("enum Wide {{\n{}\n}}\n", variants.join("\n"));
    code!(&src).error(
        "Too many enum variants — an enum holds at most 254, because a variant is one byte \
         and 0 and 255 are reserved at enum_variant_limit_refuses_the_255th:255:8",
    );
}

// ── generic bounds refuse exactly what they do not guarantee (Generics-chapter review) ──
//
// The reference's table of built-in interfaces was a copy of INTERFACES.md's DESIGN rather
// than of `default/01_code.loft`, so it promised `-` under `Addable` and all four operators
// under `Numeric`.  The permitted half is
// `tests/scripts/the-reference-bounds-permit-what-it-lists.loft`; these are the refusals,
// as Rust tests rather than `@EXPECT_ERROR` cells so that every one is matched instead of
// only the first (loft#1261).

/// `Addable` is `+` alone — its declaration carries one method and `-` is not it.
#[test]
fn generic_bound_addable_refuses_subtraction() {
    code!("fn probe<T: Addable>(a: T, b: T) -> T { a - b }\nfn test() { }").error(
        "generic type T: operator '-' requires a concrete type at \
         generic_bound_addable_refuses_subtraction:1:47",
    );
}

/// `Numeric` is `*` and the unary `-`; `+` belongs to `Addable` and is refused here.
#[test]
fn generic_bound_numeric_refuses_addition() {
    code!("fn probe<T: Numeric>(a: T, b: T) -> T { a + b }\nfn test() { }").error(
        "generic type T: operator '+' requires a concrete type at \
         generic_bound_numeric_refuses_addition:1:47",
    );
}

/// …and division, which no bound offers at all.
#[test]
fn generic_bound_numeric_refuses_division() {
    code!("fn probe<T: Numeric>(a: T, b: T) -> T { a / b }\nfn test() { }").error(
        "generic type T: operator '/' requires a concrete type at \
         generic_bound_numeric_refuses_division:1:47",
    );
}

/// An UNBOUNDED T carries no comparison either — the chapter's "you can only assign and
/// return T" is about `==` as much as about arithmetic.
#[test]
fn an_unbounded_generic_refuses_equality() {
    code!("fn probe<T>(a: T, b: T) -> boolean { a == b }\nfn test() { }").error(
        "generic type T: operator '==' requires a concrete type at \
         an_unbounded_generic_refuses_equality:1:45",
    );
}

/// A field read needs a concrete type: the bound says which OPERATIONS exist, never which
/// fields.
#[test]
fn a_generic_refuses_field_access() {
    code!("struct P { f: integer }\nfn probe<T>(a: T) -> integer { a.f }\nfn test() { }").error(
        "generic type T: field access requires a concrete type at \
         a_generic_refuses_field_access:2:37",
    );
}

/// The type variable has to be reachable from the FIRST argument, because that is what the
/// call site infers it from.
#[test]
fn a_type_variable_must_reach_the_first_parameter() {
    code!("fn probe<T>(tag: text, x: T) -> T { x }\nfn test() { }")
        .error(
            "Type variable T must appear in the first parameter — move T to the first parameter \
         position at a_type_variable_must_reach_the_first_parameter:1:32",
        )
        .warning(
            "Parameter tag is never read at a_type_variable_must_reach_the_first_parameter:1:36",
        );
}

/// A user type that has not defined the bound's operator is refused at the CALL, which is
/// where the concrete type is known — and the message names the function to write.
#[test]
fn a_user_type_missing_the_operator_is_refused_at_the_call() {
    code!(
        "struct Pri { value: integer }\n\
         fn biggest<T: Ordered>(a: T, b: T) -> T { if a > b { a } else { b } }\n\
         fn test() { r = biggest(Pri{value: 3}, Pri{value: 7}); }"
    )
    .error(
        "'Pri' does not satisfy interface 'Ordered': missing OpLt at \
         a_user_type_missing_the_operator_is_refused_at_the_call:3:54",
    )
    .warning(
        "Variable r is never read at a_user_type_missing_the_operator_is_refused_at_the_call:3:16",
    );
}
/// A bound is satisfied by a SIGNATURE, not by a name (loft#1274).
///
/// `formal/interfaces.md` (G-Sat) says a type satisfies an interface when a function with the
/// interface's signature is visible — and `Numeric` declares `op - (self: Self) -> Self`, the
/// UNARY negation. A binary `a - b` in a `<T: Numeric>` body desugars to the same `OpMin`, so
/// a name-only test said the bound covered it: the call bound one operand too many, dropped
/// `b`, and computed `-a` on both backends with no diagnostic. The float case answered an
/// integer, so the result type was wrong too.
///
/// It now takes the refusal `Addable` already gave the same expression. No built-in bound
/// offers binary subtraction; whether one SHOULD is a separate question about letting the two
/// arities of `OpMin` coexist in one interface.
///
/// The reading half — that the operators the bound DOES declare still work, and that the
/// `!=` / `<=` / `>=` derivations are untouched — is
/// `tests/scripts/1274-a-bound-is-satisfied-by-signature-not-name.loft`, which cannot hold
/// this cell because the fixed compiler refuses to parse it.
#[test]
fn binary_minus_under_numeric_is_refused_not_bound_to_the_unary_op() {
    code!("fn gsub<T: Numeric>(a: T, b: T) -> T { a - b }\nfn run() -> integer { gsub(9, 4) }")
        .error(
            "generic type T: operator '-' requires a concrete type at \
             binary_minus_under_numeric_is_refused_not_bound_to_the_unary_op:1:46",
        );
}

/// A closure may not replace the whole value of a captured heap PARAMETER (loft#1281).
///
/// `(F-ParamRebind)` makes that rebind local to the callee and the capture has no route back
/// to the parameter's SLOT, so before the refusal the write went to the store the closure
/// record's DbRef copy names — which `(F-ParamHeap)` makes the CALLER's. `fn f(p:
/// vector<integer>) { g = fn() { p = [7,7]; }; g(); }` left the caller's `[1,2]` as `[7,7]`
/// on both backends with nothing reported.
///
/// Refusing is the call the `&`-scalar shape takes one rule over (DESIGN_DECISIONS C115
/// carries both halves): making it
/// mean what it says needs the binding reachable from the closure plus a write-back, which
/// the cell machinery cannot supply for a heap value.
///
/// The shapes it must NOT reach — a mutation through the capture, a captured LOCAL, a
/// captured scalar parameter, a `&` parameter — are
/// `tests/scripts/1281-a-closure-cannot-replace-a-captured-parameter.loft`, which cannot
/// hold these cells because the fixed compiler refuses to parse them.
#[test]
fn a_closure_cannot_replace_a_captured_heap_parameter() {
    // The cure the message names is the whole of the diagnostic's value, so it is matched in
    // full rather than by a prefix: a reader who is told only "you cannot" rewrites the
    // wrong half.
    let msg = |col: &str| {
        format!(
            "Cannot replace the whole value of the captured parameter 'p' from a closure — \
             the closure shares the caller's storage, so the replacement would reach the \
             caller, where a rebind of a parameter is local to this function. Change it in \
             place if the caller should see it (`p.clear()`, `p += [...]`, `p[i] = ...`), or \
             build a local and use that. at \
             a_closure_cannot_replace_a_captured_heap_parameter:{col}"
        )
    };
    // every right-hand side, since the defect was in the destination and not the source
    code!("fn f(p: vector<integer>) { g = fn() { p = [7, 7]; }; g(); }\nfn run() -> integer { 1 }")
        .error(&msg("1:27"));
    code!(
        "fn mk() -> vector<integer> { v: vector<integer> = []; v += [7]; v }\n\
         fn f(p: vector<integer>) { g = fn() { p = mk(); }; g(); }\nfn run() -> integer { 1 }"
    )
    .error(&msg("2:27"));
    code!(
        "fn f(p: vector<integer>) { o: vector<integer> = []; o += [7]; \
         g = fn() { p = o; }; g(); }\nfn run() -> integer { 1 }"
    )
    .error(&msg("1:27"));
    // and every heap KIND, since the leak was not vector-specific
    code!(
        "struct R { k: integer, v: integer }\n\
         fn mkh() -> hash<R[k]> { h: hash<R[k]> = []; h += [R { k: 9, v: 9 }]; h }\n\
         fn f(p: hash<R[k]>) { g = fn() { p = mkh(); }; g(); }\nfn run() -> integer { 1 }"
    )
    .error(&msg("3:22"));
    code!(
        "struct S { n: integer }\n\
         fn f(p: S) { g = fn() { p = S { n: 9 }; }; g(); }\nfn run() -> integer { 1 }"
    )
    .error(&msg("2:13"));
}
