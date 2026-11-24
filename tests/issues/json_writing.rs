// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `T.to_json()` for any user struct, and the writing side of JSON.

use super::*;

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

/// @PLN165 E5–E7 — a program's own `<T>` reaches the stdlib's generics when the parser
/// CONTINUES another's Data (a prepared or cached stdlib, which is what this harness, and
/// `snapshot`/`ir_store`/`native_gate`, hand it).
///
/// `Parser::type_var_holders` — the map from `(spelling, bounds)` to the placeholder — belongs
/// to the parser that minted it, so a continuing parser fell back to the def the bare spelling
/// resolves to.  Since `sort<T: Ordered>` that def is the BOUNDED `T`, which an unbounded `<T>`
/// may not reuse, so this program minted `T#4` and its `vector<T#4>` would not convert to the
/// stdlib `reduce`'s `vector<T#3>`: *"expected vector<T>, got vector<T>"*, then a binding clash
/// on every call.  The whole corpus runner parses this way, so the E5–E7 guard was red there
/// while passing under the binary, which parses the stdlib itself.
#[test]
fn a_programs_type_variable_reaches_the_stdlib_generics_on_a_continued_parse() {
    code!(
        "fn fold<T, U>(v: vector<T>, init: U, f: fn(U, T) -> U) -> U { v.reduce(init, f) }
fn run() -> integer { fold([1, 2, 3], 0, |acc, n| { acc + n }) }"
    )
    .expr("run()")
    .result(Value::Int(6));
}

/// @PLN165 E5–E7 — a program may name a struct after a STDLIB type variable.
///
/// `map<T, U>` and `reduce<T, U>` made `U` one, and this harness (like the REPL and a
/// `<host>` string) parses at the stdlib's own source id on purpose, so the reader's `struct
/// U` and the stdlib's `<U>` competed for one `(name, source)` key: seven `struct U` tests
/// below stopped compiling with *"'U' is reserved as a generic type variable"*, a name the
/// same program compiles as a FILE.  The placeholder gives its flat name up to a declaration
/// from another file; every template that declared it holds it by number, so the generics
/// that name `U` still instantiate — which the `map` call here is for.
#[test]
fn a_struct_named_like_a_stdlib_type_variable_compiles() {
    code!(
        "struct U { a: integer }
fn run() -> integer {
    u = U { a: 5 };
    v = [1, 2].map(|n| { n * 3 });
    t = [1, 2].reduce(0, |acc, n| { acc + n });
    u.a + len(v) + t
}"
    )
    .expr("run()")
    // 5 + 2 + 3
    .result(Value::Int(10));
}

/// @FR-G-Gen-Scope — a declaration spelled like a STDLIB type variable compiles where the
/// parse shares the stdlib's source id (this harness, the REPL, a `<host>` string).
///
/// `sort_by<T, K: Ordered>` names `K` and `min_of<T>` names `T`; neither reserves the
/// letter, so an `enum K`, an `enum T` and a `type V` alias are the program's own, and the
/// stdlib generics that name those letters still instantiate beside them.
#[test]
fn a_declaration_spelled_like_a_stdlib_type_variable_compiles_in_source_0() {
    code!(
        "enum K { Ka, Kb }
enum T { Ta, Tb }
type U = integer;
struct P { n: integer }
fn run() -> integer {
    ps: vector<P> = [P { n: 3 }, P { n: 1 }, P { n: 2 }];
    ps.sort_by(|p| { p.n });
    u: U = 40;
    k = match K.Kb { Ka => 1, Kb => 2 };
    t = match T.Ta { Ta => 10, Tb => 20 };
    u + k + t + ps[0].n * 100 + (min_of([7, 5, 9]) ?? 0)
}"
    )
    .expr("run()")
    // 40 + 2 + 10 + 1 * 100 + 5
    .result(Value::Int(157));
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
operator to_text(self: P243Item) -> text { return \"item-{self.p243_id}\"; }
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
operator to_text(self: P329Item) -> text { return \"item-{self.p329_id}\"; }
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
operator to_text(self: P329Tri) -> text { return \"item-{self.p329_tri_id}\"; }
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
operator to_text(self: P330Item) -> text { return \"item-{self.p330_id}\"; }
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
operator to_text(self: P330Item2) -> text { return \"item-{self.p330_id2}\"; }
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
operator to_text(self: P329Second) -> text { return \"item-{self.p329_second_id}\"; }
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
operator to_text(self: P549Item) -> text { return \"i{self.p549_id}\"; }
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
operator to_text(self: P549Item) -> text { return \"i{self.p549_id}\"; }
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
operator to_text(self: P549Item) -> text { return \"i{self.p549_id}\"; }
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
operator to_text(self: P549Item) -> text { return \"i{self.p549_id}\"; }
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
