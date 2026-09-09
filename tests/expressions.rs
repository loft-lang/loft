// Copyright (c) 2021-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Tests that require Rust-level type checking (.tp()) or native codegen.
// All other expression tests have moved to tests/scripts/*.loft.

extern crate loft;

mod testing;

use loft::data::{IntegerSpec, Type, Value};

const INTEGER: Type = Type::Integer(IntegerSpec::signed32());

#[test]
fn expr_add_null() {
    expr!("1 + null").tp(INTEGER);
}

#[test]
fn expr_zero_divide() {
    // Plan-07 phase 4 step 4.3 — `/` by zero now raises a typed
    // RuntimeError (DivideByZero) on the non-nullable path.  Use the
    // nullable `??` form to keep the original "is the type still
    // INTEGER under div-by-zero" probe without halting the test
    // harness.  The runtime-error path is covered separately by
    // `tests/runtime_errors.rs::kind_divide_by_zero_int_prints_pretty_error`.
    expr!("2 / (3 - 2 - 1) ?? null").tp(INTEGER);
}

#[test]
fn call_with_null() {
    // @PLN102 gate-2 (call-arg N-Store) — passing `null` into a param requires the param to be
    // nullable (`integer?`); into a non-null `integer` it is now the same violation as a nullable
    // into a non-null local. With `b: integer?`, `a + b` propagates (N-Prop) and `add(1, null)`
    // still yields null at runtime, typed `integer?`.
    code!("fn add(a: integer, b: integer?) -> integer? { a + b }")
        .expr("add(1, null)")
        .tp(Type::optional(INTEGER))
        .result(Value::Null);
}

#[test]
fn call_text_null() {
    // @PLN25 DN1 — a fn that can `return null` must declare a nullable result `text?`
    // (returning null into a non-null `text` is now an `(N-Store)` error); the call
    // then types `Optional(Text)` and still yields null at runtime.
    code!("fn routine(a: integer) -> text? { if a > 2 { return null }; \"#{a}#\"}")
        .expr("routine(5)")
        .tp(Type::optional(Type::Text(loft::data::Deps::none())))
        .result(Value::Null);
}

#[test]
fn call_int_null() {
    // @PLN25 DN1 — `return null` requires a nullable result `integer?`; the call types
    // `Optional(Integer)` and still yields null at runtime.
    code!("fn routine(a: integer) -> integer? { if a > 2 { return null }; a+1 }")
        .expr("routine(5)")
        .tp(Type::optional(INTEGER))
        .result(Value::Null);
}

#[test]
fn if_typing() {
    expr!("a = \"12\"; if a.len()>2 { null } else { \"error\" }").result(Value::str("error"));
    expr!("a = \"12\"; if a.len()==2 { null } else { \"error\" }")
        .tp(Type::Text(loft::data::Deps::none()))
        .result(Value::Null);
}

// N6 generated_code_compiles and native_test_suite moved to tests/native.rs

// ── T1.1 — Type::Tuple helpers ──────────────────────────────────────────────

#[test]
fn tuple_element_offsets() {
    use loft::data::{Type, element_stack_offsets, element_stack_size};
    let types = [
        Type::Integer(IntegerSpec {
            min: i32::MIN,
            max: i32::MAX as u32,
            not_null: false,
            forced_size: None,
        }),
        Type::Text(loft::data::Deps::none()),
        Type::Float,
    ];
    let offsets = element_stack_offsets(&types);
    // integer=8 at 0 (post-2c), text=Str size at 8, float=8 after text
    let text_sz = element_stack_size(&Type::Text(loft::data::Deps::none()));
    assert_eq!(offsets, vec![0, 8, 8 + text_sz]);
}

#[test]
fn tuple_owned_elements() {
    // owned_elements for [integer, text, reference<T>] should return text and ref entries
    use loft::data::{Type, owned_elements};
    let types = [
        Type::Integer(IntegerSpec {
            min: i32::MIN,
            max: i32::MAX as u32,
            not_null: false,
            forced_size: None,
        }),
        Type::Text(loft::data::Deps::none()),
        Type::Reference(0, loft::data::Deps::none()),
    ];
    let owned = owned_elements(&types);
    assert_eq!(owned.len(), 2);
}

// ── CO1.1 — CoroutineStatus enum ────────────────────────────────────────────
// Verify the CoroutineStatus enum from default/05_coroutine.loft.

#[test]
fn coroutine_status_construct() {
    code!(
        "fn check(s: CoroutineStatus) -> boolean {
               match s { Created => true, _ => false }
           }"
    )
    .expr("check(CoroutineStatus.Created)")
    .result(Value::Boolean(true));
}

#[test]
fn coroutine_status_ordering() {
    // Enum variant ordering: Created < Suspended < Running < Exhausted
    expr!("CoroutineStatus.Created < CoroutineStatus.Exhausted").result(Value::Boolean(true));
}

// ── TR1.3 — stack_trace() materialisation ────────────────────────────────────
// Verify that stack_trace() returns a vector of StackFrame.

#[test]
fn stack_trace_returns_frames() {
    // stack_trace() returns one frame per call-stack entry, including the synthetic
    // entry frame for n_test (Fix #88).  Named variable `frames` ensures OpFreeRef
    // is emitted at scope exit.
    code!(
        "fn inner(n: integer) -> integer { frames = stack_trace(); len(frames) + n - n }
         fn outer(n: integer) -> integer { inner(n) }"
    )
    .expr("outer(0)")
    .result(Value::Int(3)); // n_test(entry) + outer + inner
}

#[test]
fn stack_trace_function_names() {
    // Returns integer (1 if name matches) to avoid borrowing text from the vector,
    // which would suppress OpFreeRef and leak the database store.
    // frames = [n_test(entry), caller, check_caller_name]; "caller" is at index len-2.
    code!(
        "fn check_caller_name() -> integer {
            frames = stack_trace();
            if len(frames) > 1 && frames[len(frames) - 2].function == \"caller\" { 1 } else { 0 }
         }
         fn caller() -> integer { check_caller_name() }"
    )
    .expr("caller()")
    .result(Value::Int(1));
}

// ── TR1.4 — Call-site line numbers ───────────────────────────────────────────

#[test]
fn call_frame_has_line() {
    // Verify that stack_trace() reports a non-zero line for a known call site.
    // frames = [n_test(entry, line=0), check_line(line=call-site)].
    // Use frames[len(frames)-1] to access the innermost (check_line) frame.
    code!(
        "fn check_line(n: integer) -> integer {
            frames = stack_trace();
            if len(frames) > 0 { (frames[len(frames) - 1].line ?? 0) + n - n } else { -1 + n - n }
         }"
    )
    .expr("check_line(0)")
    .result(Value::Int(7)); // user code = 4 lines + 1 blank + "pub fn test() {" + call at line 7
}

// ── TR1.2 — StackFrame + ArgValue type declarations ─────────────────────────
// Verify the types from default/04_stacktrace.loft can be constructed and used.

#[test]
fn stacktrace_argvalue_construct() {
    // Verify ArgValue enum is visible: matching on a variant produces the expected type.
    code!(
        "fn check_arg(v: ArgValue) -> integer {
            match v { IntVal { n } => n, _ => -1 }
         }"
    )
    .expr("check_arg(IntVal { n: 42 })")
    .result(Value::Int(42));
}

#[test]
fn map_with_capturing_closure() {
    // C48: capturing closures work with map and filter.
    code!(
        "fn check() -> integer {
             factor = 3;
             scaled = map([1, 2, 3, 4, 5], fn(x: integer) -> integer { x * factor });
             threshold = 3;
             big = filter([1, 2, 3, 4, 5], fn(x: integer) -> boolean { x > threshold });
             scaled[2] + big[0]
         }"
    )
    .expr("check()")
    .result(Value::Int(13)); // scaled[2]=9, big[0]=4 → 13
}

#[test]
fn text_slot_reuse_sequential() {
    // C43.4: sequential text variables with non-overlapping lifetimes.
    code!(
        "fn check() -> text {
             a = \"hello\";
             b = a + \" world\";
             c = \"goodbye\";
             d = c + \" world\";
             d
         }"
    )
    .expr("check()")
    .result(Value::str("goodbye world"));
}

#[test]
fn struct_enum_local_freed() {
    // C41: creating a struct-enum as a local and returning a scalar must not leak.
    code!(
        "fn check() -> integer {
             v = IntVal { n: 42 };
             match v { IntVal { n } => n, _ => 0 }
         }"
    )
    .expr("check()")
    .result(Value::Int(42));
}

#[test]
fn stacktrace_arginfo_field() {
    // Verify ArgInfo struct is visible and fields are accessible.
    code!("fn get_name(info: ArgInfo) -> text { info.name }")
        .expr("get_name(ArgInfo { name: \"x\", type_name: \"integer\", value: IntVal { n: 7 } })")
        .result(Value::str("x"));
}

#[test]
fn stacktrace_frame_field() {
    // Verify StackFrame struct is visible and fields are accessible.
    code!("fn get_fn(f: StackFrame) -> text { f.function }")
        .expr("get_fn(StackFrame { function: \"main\", file: \"test.loft\", line: 1 })")
        .result(Value::str("main"));
}

// ── TR1.1 — Shadow call-frame vector ────────────────────────────────────────
// Verify that function calls still work after the OpCall bytecode format change
// (d_nr + args_size operands added for the shadow call-frame vector).

#[test]
fn call_stack_nested_calls() {
    code!(
        "fn add(a: integer, b: integer) -> integer { a + b }
         fn double(x: integer) -> integer { add(x, x) }
         fn quad(x: integer) -> integer { double(double(x)) }"
    )
    .expr("quad(3)")
    .result(Value::Int(12));
}

#[test]
fn call_stack_fn_ref() {
    code!(
        "fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
         fn inc(n: integer) -> integer { n + 1 }"
    )
    .expr("apply(inc, 41)")
    .result(Value::Int(42));
}

#[test]
fn call_stack_recursive() {
    code!(
        "fn fib(n: integer) -> integer {
            if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
         }"
    )
    .expr("fib(10)")
    .result(Value::Int(55));
}

// ── T1.2 — Tuple parser (notation, literals, destructuring) ─────────────────

#[test]
fn tuple_type_return() {
    // A function returning a tuple type should parse and compile.
    code!(
        "fn pair(a: integer, b: integer) -> (integer, integer) {
            (a, b)
         }"
    )
    .expr("pair(3, 7).0")
    .result(Value::Int(3));
}

#[test]
fn tuple_literal_basic() {
    // A tuple literal assigned to a variable; element access via .0 / .1.
    expr!("t = (10, 20); t.0 + t.1").result(Value::Int(30));
}

#[test]
fn tuple_element_access_three() {
    // Three-element tuple with mixed types — access each element.
    expr!("t = (1, 2, 3); t.0 + t.1 + t.2").result(Value::Int(6));
}

#[test]
fn tuple_destructure_basic() {
    // LHS destructuring: (a, b) = expr.
    code!("fn pair(x: integer) -> (integer, integer) { (x, x * 2) }")
        .expr("(a, b) = pair(5); a + b")
        .result(Value::Int(15));
}

#[test]
fn tuple_element_assign() {
    // Assigning to an individual tuple element: t.0 = expr.
    expr!("t = (1, 2); t.0 = 10; t.0 + t.1").result(Value::Int(12));
}

#[test]
fn tuple_type_annotation() {
    // Explicit tuple type annotation on a variable.
    expr!("t: (integer, integer) = (3, 4); t.0 + t.1").result(Value::Int(7));
}

#[test]
fn tuple_parameter() {
    // Tuple type as a function parameter.
    code!("fn sum_pair(p: (integer, integer)) -> integer { p.0 + p.1 }")
        .expr("sum_pair((10, 20))")
        .result(Value::Int(30));
}

#[test]
fn tuple_with_text() {
    // Tuple containing a text element — verify text is accessible.
    code!("fn greet(name: text) -> (integer, text) { (len(name), name) }")
        .expr("greet(\"hello\").0")
        .result(Value::Int(5));
}

// ── T1.5 — Reference-tuple parameters ────────────────────────────────────────

#[test]
fn ref_tuple_param_swap() {
    // &(integer, integer) parameter — swap elements via reference.
    code!(
        "fn swap(pair: &(integer, integer)) {
            tmp = pair.0;
            pair.0 = pair.1;
            pair.1 = tmp;
         }"
    )
    // In loft, ref args are passed by variable name — no & prefix at call site.
    .expr("p = (3, 7); swap(p); p.0 * 10 + p.1")
    .result(Value::Int(73));
}

// ── T1.6 — Tuple-aware mutation guard ────────────────────────────────────────

#[test]
fn ref_tuple_unused_mutation_error() {
    // &(integer, integer) parameter that is never mutated — should produce a warning.
    // Column 20 is the `&` itself (loft#1003).  This pinned 1:53 — a position inside the
    // BODY — because the check ran after the body and fell back to the variable's source.
    code!("fn read_only(pair: &(integer, integer)) -> integer { pair.0 + pair.1 }")
        .expr("p = (3, 7); read_only(p)")
        .warning("Parameter 'pair' does not need to be a reference at ref_tuple_unused_mutation_error:1:20")
        .result(Value::Int(10));
}

// ── A5.3 — Closure capture at call site ─────────────────────────────────────

#[test]
fn closure_capture_integer() {
    // A lambda captures an integer from the enclosing scope.
    expr!("x = 10; f = fn(y: integer) -> integer { x + y }; f(5)").result(Value::Int(15));
}

#[test]
fn closure_capture_after_change() {
    // A5.6-2: closure is allocated at definition time, so x=10 is captured into
    // the closure record when `f = fn(...)` is evaluated.  x=99 is a later
    // reassignment that does not affect the closure → f(5) = 10 + 5 = 15.
    //
    // And NO dead-assignment warning: the capture READS the 10, which the result
    // above proves (15 is 10 + 5).  This case used to assert the warning — the
    // capture is deliberately not counted in `uses`, so the check read it as a
    // write overwritten before any read, and advertised deleting the line that
    // supplies the answer.  A lint that must never make a program wrong stays
    // silent on a captured variable (`Variables::track_write`).
    expr!("x = 10; f = fn(y: integer) -> integer { x + y }; x = 99; f(5)").result(Value::Int(15));
}

#[test]
fn closure_capture_multiple() {
    // A lambda captures two variables from the enclosing scope.
    expr!("a = 3; b = 7; f = fn(x: integer) -> integer { a + b + x }; f(10)")
        .result(Value::Int(20));
}

#[test]
fn closure_capture_text() {
    // Captured text is deep-copied — independent of the original after capture.
    code!(
        "fn make_greeter(prefix: text) -> fn(text) -> text {
            fn(name: text) -> text { \"{prefix} {name}\" }
         }"
    )
    .expr("make_greeter(\"Hello\")(\"world\")")
    .result(Value::str("Hello world"));
}

#[test]
fn closure_capture_text_integer_return() {
    // Same-scope text capture: lambda reads captured text, returns integer.
    // A5.6b.1: zero-param fn-ref fast path now injects __closure arg; text_return
    // no longer adds captured vars as spurious RefVar(Text) work-buffer arguments.
    expr!("prefix = \"hello\"; f = fn() -> integer { len(prefix) }; f()").result(Value::Int(5));
}

// A5.6b.2: re-enabled after generate_call_ref work-buffer push fix.
#[test]
fn closure_capture_text_return() {
    // Same-scope text capture: lambda reads captured text, returns text.
    expr!(
        "greeting = \"hello\"; f = fn(name: text) -> text { \"{greeting}, {name}!\" }; f(\"world\")"
    )
    .result(Value::str("hello, world!"));
}

// ── A5.6e — Closure capture coverage ────────────────────────────────────────

#[test]
fn closure_capture_struct_ref() {
    // A5.6e scenario 3: capture a struct record (12-byte DbRef).
    // Verifies the base A5.6b.1 DbRef copy path for non-text captures.
    code!(
        "struct Item { value: integer }
fn test() {
    it = Item { value: 10 };
    add = fn(x: integer) -> integer { it.value + x };
    assert(add(5) == 15, \"expected 15, got {add(5)}\");
}"
    )
    .result(loft::data::Value::Null);
}

#[test]
fn closure_capture_vector_elem() {
    // A5.6e scenario 4: capture a value obtained by indexing a vector.
    // Verifies that the element is materialised into a local before capture
    // and the closure correctly reads it at call time.
    code!(
        "fn test() {
    nums = [10, 20, 30];
    chosen = nums[1];
    pick = fn() -> integer { chosen };
    assert(pick() == 20, \"expected 20, got {pick()}\");
}"
    )
    .result(loft::data::Value::Null);
}

#[test]
fn closure_capture_text_loop() {
    // A5.6f: work buffer must be cleared before each call so loop iterations
    // don't accumulate text from previous calls.
    code!(
        "fn test() {
    for i in 0..5 {
        prefix = \"hello\";
        f = fn(name: text) -> text { \"{prefix}, {name}!\" };
        result = f(\"world\");
        assert(result == \"hello, world!\", \"iter {i}: expected hello, world!, got {result}\");
    }
}"
    )
    .result(loft::data::Value::Null);
}

// ── CO1.2 — OpCoroutineCreate + OpCoroutineNext ─────────────────────────────

#[test]
fn coroutine_create_basic() {
    // A generator function should return an iterator without executing the body.
    code!(
        "fn count() -> iterator<integer> { yield 1; yield 2; yield 3; }
         fn test_count() -> integer {
            gen = count();
            next(gen)
         }"
    )
    .expr("test_count()")
    .result(Value::Int(1));
}

#[test]
fn coroutine_next_sequence() {
    // Successive next() calls advance the generator.
    code!(
        "fn count() -> iterator<integer> { yield 10; yield 20; yield 30; }
         fn sum_three() -> integer {
            gen = count();
            a = next(gen);
            b = next(gen);
            c = next(gen);
            a + b + c
         }"
    )
    .expr("sum_three()")
    .result(Value::Int(60));
}

#[test]
fn coroutine_exhausted() {
    // After all yields + one more advance, exhausted() returns true.
    code!(
        "fn one_val() -> iterator<integer> { yield 42; }
         fn check() -> boolean {
            gen = one_val();
            next(gen);
            next(gen);
            exhausted(gen)
         }"
    )
    .expr("check()")
    .result(Value::Boolean(true));
}

#[test]
fn coroutine_for_loop() {
    // Generator consumed by a for loop.
    code!(
        "fn range3() -> iterator<integer> { yield 1; yield 2; yield 3; }
         fn sum_gen() -> integer {
            total = 0;
            for n in range3() { total += n; }
            total
         }"
    )
    .expr("sum_gen()")
    .result(Value::Int(6));
}

// ── CO1.3e — Nested yield (generator calls helper function) ─────────────────

#[test]
fn coroutine_call_helper_between_yields() {
    // A generator calls a regular function between yields.
    // The call frame is saved/restored across the yield/resume cycle.
    code!(
        "fn double(x: integer) -> integer { x * 2 }
         fn gen() -> iterator<integer> {
            yield double(5);
            yield double(10);
         }"
    )
    .expr("total = 0; for n in gen() { total += n; }; total")
    .result(Value::Int(30));
}

// ── CO1.3d — Text serialisation across yield/resume ─────────────────────────

#[test]
fn coroutine_text_param_survives_yield() {
    // A generator that takes a `text` parameter and yields `len(text)`.
    // The text value must survive the yield/resume cycle without dangling pointers.
    code!(
        "fn gen_len(s: text) -> iterator<integer> {
            yield len(s);
            yield len(s);
         }
         fn sum_lens() -> integer {
            total = 0;
            for n in gen_len(\"hello\") { total += n; }
            total
         }"
    )
    .expr("sum_lens()")
    .result(Value::Int(10));
}

// P2-R3: text LOCAL survives yield — CO1.3d serialise_text_slots implemented
#[test]
fn coroutine_text_local_survives_yield() {
    // P2-R3: a generator that builds a text LOCAL (not a parameter) and yields.
    // CO1.3d must serialise the local String to text_owned at yield and restore
    // the pointer on resume; until then the raw bytes path leaves a dangling ptr.
    code!(
        "fn gen_words() -> iterator<text> {
            word = \"hello\";
            yield word;
            word = \"world\";
            yield word;
         }
         fn joined() -> text {
            result = \"\";
            for w in gen_words() { result += w; result += \" \"; }
            result
         }"
    )
    .expr("joined()")
    .result(Value::str("hello world "));
}

// ── CO1.4 — yield from delegation ───────────────────────────────────────────

// ── T1.7 — `integer not null` annotation for tuple elements ─────────────────

#[test]
fn not_null_element_assignment() {
    // `integer not null` element in a tuple type — basic assignment compiles and runs.
    code!("fn count_pair() -> (integer, integer) { (1, 2) }")
        .expr("p = count_pair(); p.0 + p.1")
        .result(Value::Int(3));
}

// ── CO1.4 — yield from ───────────────────────────────────────────────────────

#[test]
fn coroutine_yield_from() {
    // yield from delegates to a sub-generator.
    code!(
        "fn inner() -> iterator<integer> { yield 10; yield 20; }
         fn outer() -> iterator<integer> { yield 1; yield from inner(); yield 2; }
         fn sum_all() -> integer {
            total = 0;
            for n in outer() { total += n; }
            total
         }"
    )
    .expr("sum_all()")
    .result(Value::Int(33));
}

// ── S23 — runtime guard in coroutine_next ─────────────────────────────────────

/// S23/P1-R2 runtime guard: coroutine_next must bounds-check idx before indexing
/// self.coroutines.  When a COROUTINE_STORE DbRef escapes into a worker thread its
/// rec value indexes the WORKER's (empty) coroutines table → out-of-bounds panic.
/// After the fix a clear attributed message fires instead of a bare index panic.
#[test]
fn coroutine_next_bounds_guard() {
    // S23 is complete: the compiler rejects iterator<T> function calls inside par()
    // bodies, and coroutine_next has a runtime bounds guard as defence-in-depth.
    // The compiler check is exercised by par_worker_returns_generator in parse_errors.rs.
    // Nothing to assert here; this test exists as a named placeholder.
}

// ── S26 — exhausted coroutine frames freed on return ──────────────────────────

/// S26: after a for-loop exhausts a generator, coroutine_return frees the slot.
/// Running many generators in succession must not grow State::coroutines unboundedly.
#[test]
fn coroutine_frame_freed_after_exhaustion() {
    code!(
        "fn up_to_two() -> iterator<integer> { yield 1; yield 2; }
         fn sum_many() -> integer {
             total = 0;
             for _ in 0..1000 { for n in up_to_two() { total += n; } }
             total
         }"
    )
    .expr("sum_many()")
    .result(Value::Int(3000));
}

// ── S27 — text_positions save/restore across yield/resume ─────────────────────

/// S27: text_positions entries for suspended generator locals are removed at
/// yield and restored at resume.  Consumer text locals no longer conflict with
/// the suspended generator's tracked positions in debug builds.
/// The observable fix is the absence of spurious double-free panics; we verify
/// the generator + consumer text combination produces the correct integer sum.
#[test]
fn coroutine_text_positions_save_restore() {
    // Consumer allocates text while iterating a generator; S27 ensures
    // text_positions entries from the suspended generator don't mask missing
    // OpFreeText calls in the consumer (or cause false double-free panics).
    code!(
        "fn gen_ints() -> iterator<integer> { yield 1; yield 2; yield 3; }
         fn sum_with_label(label: text) -> integer {
             total = 0;
             for n in gen_ints() { total += n; }
             len(label) + total
         }"
    )
    .expr("sum_with_label(\"hi\")")
    .result(Value::Int(8)); // len("hi")=2, sum=6, total=8
}

// ── S29 — parallel store: thread::scope + skip claims ─────────────────────────

/// S29/P1-R2+P1-R3: run_parallel_direct uses thread::scope and claims-free
/// worker clones; observable results are identical to the old thread::spawn path.
#[test]
fn parallel_for_thread_scope_results() {
    code!(
        "struct Num { value: integer }
         struct NumList { items: vector<Num> }
         fn doubled(n: const Num) -> integer { n.value * 2 }
         fn run_par() -> integer {
             lst = NumList {};
             lst.items += [Num { value: 1 }, Num { value: 2 }, Num { value: 3 }, Num { value: 4 }, Num { value: 5 }];
             total = 0;
             for a in lst.items par(b = doubled(a), 2) { total += b; }
             total
         }"
    )
    .expr("run_par()")
    .result(Value::Int(30));
}

// ── A14 — par_light auto-selection ───────────────────────────────────────────

/// A14: par() with a simple integer worker automatically uses the light path.
#[test]
fn par_light_auto_selected() {
    code!(
        "struct Num { value: integer }
         struct NumList { items: vector<Num> }
         fn tripled(n: const Num) -> integer { n.value * 3 }
         fn run_par() -> integer {
             lst = NumList {};
             lst.items += [Num{value:1}, Num{value:2}, Num{value:3}, Num{value:4}, Num{value:5},
                           Num{value:6}, Num{value:7}, Num{value:8}, Num{value:9}, Num{value:10}];
             total = 0;
             for a in lst.items par(b = tripled(a), 4) { total += b };
             total
         }"
    )
    .expr("run_par()")
    .result(Value::Int(165));
}

/// A14: par with extra args uses the light path too.
#[test]
fn par_light_extra_args() {
    code!(
        "struct Num { value: integer }
         struct NumList { items: vector<Num> }
         fn scaled(n: const Num, factor: integer) -> integer { n.value * factor }
         fn run_par() -> integer {
             lst = NumList {};
             lst.items += [Num { value: 2 }, Num { value: 5 }];
             total = 0;
             for a in lst.items par(b = scaled(a, 10), 2) { total += b };
             total
         }"
    )
    .expr("run_par()")
    .result(Value::Int(70));
}

// ── S28 — debug generation counter for stale DbRef across coroutine yield ─────

/// S28: mutating a struct store between coroutine `next()` calls trips the
/// debug-mode generation-counter guard.  @P324/#218 DEMOTED that guard from a
/// panic to a non-fatal `eprintln!` warning (the snapshot over-approximates —
/// it flags every mutated store, not just ones the suspended frame's DbRefs
/// reach — so a panic false-positived on plain `out += [v]` integer-generator
/// idioms).  So under debug_assertions the stale-store shape now WARNS on stderr
/// and RUNS TO COMPLETION rather than panicking; this pins that the DA-armed
/// build reaches the correct result (no UAF/store-corruption panic).  The
/// release twin `p324_integer_generator_with_concurrent_store_mutation_no_longer_panics`
/// covers the same shape with the guard compiled out.
#[test]
#[cfg(debug_assertions)]
fn coroutine_stale_store_guard() {
    // The generator count_up has no DbRef parameters; the stale-store check is a
    // heuristic that fires on ANY store mutation between yields.  We pre-create a
    // struct store before the loop so it is included in the yield snapshot, then
    // claim a new record (Box{}) inside the loop to increment its generation —
    // which emits the S28 warning without halting execution.
    code!(
        "struct Box { val: integer }
         struct BoxList { items: vector<Box> }
         fn count_up() -> iterator<integer> { yield 1; yield 2; yield 3; }
         fn run_stale() -> integer {
             lst = BoxList {};
             lst.items += [Box { val: 0 }];
             total = 0;
             for n in count_up() {
                 lst.items += [Box { val: n }];
                 total += n;
             }
             total
         }"
    )
    .expr("run_stale()")
    .result(Value::Int(6));
}

// ── S22 — claim/delete on a locked store panics in all build profiles ─────────

#[test]
#[should_panic(expected = "Claim on read-only store")]
fn claim_on_locked_store_panics() {
    // S22: store.claim() panics when the store is HARD-locked (read_only)
    // in all build profiles.  Pre-P290 a single `locked` flag covered
    // both the hard read-only case AND the soft "don't-free" case; this
    // test exercises the hard case only — Store::lock() sets read_only.
    let mut stores = loft::database::Stores::new();
    let db = stores.null();
    stores.store_mut(&db).lock();
    stores.claim(&db, 4); // must panic — read-only store may not be extended
}

#[test]
#[should_panic(expected = "Delete on locked store")]
fn delete_on_locked_store_panics() {
    // S22: store.delete() now panics when locked in all build profiles.
    // Before the fix, release builds silently ignored the delete.
    let mut stores = loft::database::Stores::new();
    let db = stores.null();
    let cr = stores.claim(&db, 2);
    stores.store_mut(&db).lock();
    stores.store_mut(&db).delete(cr.rec); // must panic
}

// ── S25.1 — text arg `Str` serialised at coroutine_create ────────────────────

/// S25.1 (P2-R1): text args are passed as `Str { ptr, len }` — a borrowed reference
/// into the caller's owned `String`.  After `coroutine_create`, the caller's String
/// may be freed by `OpFreeText` before the generator is first resumed.  The `Str`
/// in `stack_bytes` then holds a dangling pointer.
/// Fix: `serialise_text_slots` at `coroutine_create` converts every dynamic text arg
/// to an owned `String` in `frame.text_owned`.  See SAFE.md § P2-R1.
#[test]
fn coroutine_text_arg_dynamic_serialised() {
    // gen_label receives a format-string arg (dynamic heap String, not a static literal).
    // After coroutine_create, the Str in stack_bytes must point to a serialised owned
    // String — not the caller's allocation which OpFreeText may free immediately after.
    code!(
        "fn gen_label(prefix: text) -> iterator<integer> {
             yield len(prefix);
             yield len(prefix);
         }
         fn sum_dynamic_lens(n: integer) -> integer {
             total = 0;
             for v in gen_label(\"item {n}\") { total += v; }
             total
         }"
    )
    .expr("sum_dynamic_lens(3)")
    .result(Value::Int(12)); // len("item 3") = 6, two yields: 6 + 6 = 12
}

// ── S25.2 — `String` locals freed before coroutine_return rewinds stack ──────

/// S25.2 (P2-R2): when a generator has a `text` local variable, the `String` heap
/// allocation lives on the coroutine's live stack.  `coroutine_return` calls
/// `frame.text_owned.clear()` and `frame.stack_bytes.clear()`, but neither path
/// calls `String::drop()` for stack-resident Strings → heap leak.
/// Fix: drain `text_positions` entries in the live frame region before rewinding.
/// See SAFE.md § P2-R2.
#[test]
fn coroutine_text_arg_freed_at_return() {
    // gen_len_twice takes a dynamic text arg, yields its length twice, then exhausts.
    // At coroutine_return, frame.text_owned must be drained so the owned String
    // allocated by S25.1 at create time is freed.  Without the drain, every
    // generator exhaustion with a text arg leaks a String heap allocation.
    // The test verifies correct values; Valgrind is needed to confirm the drain.
    // Note: single-yield generators have a separate pre-existing bug (return 0).
    code!(
        "fn gen_len_twice(prefix: text) -> iterator<integer> {
             yield len(prefix);
             yield len(prefix);
         }
         fn sum_twice(n: integer) -> integer {
             total = 0;
             for v in gen_len_twice(\"item {n}\") { total += v; }
             total
         }"
    )
    .expr("sum_twice(3)")
    .result(Value::Int(12)); // len("item 3") = 6, two yields: 6 + 6 = 12
}

// ── S37 — abandoned coroutine frame freed on early break ─────────────────────

/// S37: when a `for` loop exits early (break), `OpFreeRef` calls `free_ref` on
/// the coroutine DbRef.  Before the fix, `database.free()` was a no-op for
/// `COROUTINE_STORE` (store_nr == u16::MAX), so the frame's `text_owned` buffers
/// and `stack_bytes` were never freed — a memory leak on every early-break path.
/// Fix: `free_ref` checks `db.store_nr == COROUTINE_STORE` and calls
/// `free_coroutine(db.rec)` explicitly.
/// The test verifies that the generator function produces correct values when
/// used normally (post-break code still executes correctly).
#[test]
fn coroutine_early_break_frame_freed() {
    // count3 yields three values; the consumer breaks after the first.
    // After the break, free_ref must release the coroutine frame without panicking.
    // We confirm correctness by verifying the consumer returns 1 (the first yield only).
    code!(
        "fn count3() -> iterator<integer> { yield 1; yield 2; yield 3; }
         fn take_first() -> integer {
             for v in count3() {
                 return v;
             }
             0
         }"
    )
    .expr("take_first()")
    .result(Value::Int(1));
}

// ── S25.3 — text local `String` dropped on early break ───────────────────────

/// S25.3 (C24 / Step 2): when a generator with a text LOCAL is abandoned via an
/// early `break`, `free_coroutine` must drop the `String` objects embedded in
/// `frame.stack_bytes`.  Without the fix, every early break from such a generator
/// leaks a String heap allocation per live text local at the last yield point.
///
/// The test verifies correct values; run under Miri or Valgrind to confirm that
/// no String heap buffer is leaked.
#[test]
fn coroutine_text_local_early_break() {
    // gen_greet yields one greeting then another; the consumer breaks after the first.
    // At break, the text local `greeting` is live in frame.stack_bytes and its String
    // heap buffer must be freed by drop_text_locals_in_bytes in free_coroutine.
    code!(
        "fn gen_greet() -> iterator<text> {
             greeting = \"hello\";
             yield greeting;
             greeting = \"world\";
             yield greeting;
         }
         fn take_first_len() -> integer {
             for g in gen_greet() {
                 return len(g);
             }
             0
         }"
    )
    .expr("take_first_len()")
    .result(Value::Int(5)); // len("hello") = 5
}

/// S25.3 (C24 / Step 1): when a generator has a text local declared AFTER the
/// first yield point, its slot is uninitialised at first yield.  The Zone-2
/// zeroing at first resume establishes the null-ptr invariant so that
/// `drop_text_locals_in_bytes` safely skips the uninitialised slot.
///
/// Without Step 1, the raw store garbage in the slot would appear as a non-null
/// pointer and `drop_in_place` would dereference garbage → UB / crash.
#[test]
fn coroutine_text_local_declared_after_first_yield() {
    // gen_late_text yields an integer, then creates a text local and yields it.
    // The consumer breaks after the first (integer converted to text) yield.
    // At break, the text local `label` slot is uninitialised (null-zeroed by
    // Step 1) and must be skipped by drop_text_locals_in_bytes.
    code!(
        "fn gen_late_text() -> iterator<integer> {
             yield 1;
             label = \"ignored\";
             yield len(label);
         }
         fn take_first_int() -> integer {
             for n in gen_late_text() {
                 return n;
             }
             0
         }"
    )
    .expr("take_first_int()")
    .result(Value::Int(1));
}

// ── T1.10 — Homogeneous-type tuple coverage ───────────────────────────────────

/// T1.10-1: homogeneous (text, text) tuple — both slots live and freed correctly.
#[test]
fn tuple_homogeneous_text() {
    code!(
        "fn make_pair(first: text, last: text) -> (text, text) { (first, last) }
         fn test() {
             (g, s) = make_pair(\"Hello\", \"World\");
             assert(g == \"Hello\", \"first\");
             assert(s == \"World\", \"second\");
         }"
    );
}

/// T1.10-2: text fields from a struct record into a tuple — field text into
/// tuple element does not produce a dangling reference.
#[test]
fn tuple_store_text_fields() {
    code!(
        "struct Label { name: text }
         fn label_pair(a: Label, b: Label) -> (text, text) { (a.name, b.name) }
         fn test() {
             la = Label { name: \"alpha\" };
             lb = Label { name: \"beta\" };
             (n1, n2) = label_pair(la, lb);
             assert(n1 == \"alpha\", \"first\");
             assert(n2 == \"beta\", \"second\");
         }"
    );
}

/// T1.10-3: two struct-reference elements — adjacent DbRef slots in a tuple.
#[test]
fn tuple_struct_refs() {
    code!(
        "struct Point { x: integer, y: integer }
         fn two_points(a: Point, b: Point) -> (Point, Point) { (b, a) }
         fn test() {
             p1 = Point { x: 1, y: 2 };
             p2 = Point { x: 3, y: 4 };
             (q1, q2) = two_points(p1, p2);
             assert(q1.x == 3, \"q1.x\");
             assert(q2.x == 1, \"q2.x\");
         }"
    );
}

/// T1.10-4: tuple elements sourced from indexed vector reads.
#[test]
fn tuple_from_vector_elements() {
    code!(
        "fn first_two(v: vector<integer>) -> (integer, integer) { (v[0], v[1]) }
         fn test() {
             nums = [10, 20, 30];
             (a, b) = first_two(nums);
             assert(a == 10, \"first\");
             assert(b == 20, \"second\");
         }"
    );
}

// ── T1.9 — Tuple destructuring in `match` ────────────────────────────────────

/// T1.9-1: wildcard arm `_` in a tuple match should evaluate to the arm body.
#[test]
fn tuple_match_wildcard() {
    code!("fn pick_wildcard(t: (integer, integer)) -> integer { match t { _ => 42 } }")
        .expr("pick_wildcard((1, 2))")
        .result(Value::Int(42));
}

/// T1.9-2: literal pattern arms — match on both element values.
#[test]
fn tuple_match_literal() {
    code!(
        "fn classify(t: (integer, integer)) -> integer {
             match t {
                 (0, 0) => 0,
                 (1, _) => 1,
                 _      => 99,
             }
         }"
    )
    .expr("classify((0, 0))")
    .result(Value::Int(0))
    .expr("classify((1, 5))")
    .result(Value::Int(1))
    .expr("classify((2, 3))")
    .result(Value::Int(99));
}

/// T1.9-3: binding variables in a tuple arm — bound names usable in arm body.
#[test]
fn tuple_match_binding() {
    code!("fn sum_pair(t: (integer, integer)) -> integer { match t { (a, b) => a + b } }")
        .expr("sum_pair((3, 4))")
        .result(Value::Int(7));
}

// ── @P324 — S28 guard narrowed to debug-only ─────────────────────────────────

/// @P324: the S28 store-mutation guard was previously always-on (CO1.9), but the
/// snapshot is over-broad — it captures EVERY live store at yield, not just
/// stores the suspended frame's DbRef locals point at.  This false-positives on
/// the most basic iterator-consumer idiom: `out += [v]` inside
/// `for v in gen()` over an integer generator (the consumer mutates `out`'s
/// store; the generator holds no DbRefs).  Until a precise narrowing lands the
/// guard is demoted to a debug-only `eprintln!` warning so release builds can
/// run this shape.  This test exercises that shape and must NOT panic on
/// release — paired with the debug-only `coroutine_stale_store_guard` above
/// which still verifies the guard's diagnostic surface in debug builds.
#[test]
fn p324_integer_generator_with_concurrent_store_mutation_no_longer_panics() {
    code!(
        "struct Box { val: integer }
         struct BoxList { items: vector<Box> }
         fn count_up() -> iterator<integer> { yield 1; yield 2; yield 3; }
         fn run_stale() -> integer {
             lst = BoxList {};
             lst.items += [Box { val: 0 }];
             total = 0;
             for n in count_up() {
                 lst.items += [Box { val: n }];
                 total += n;
             }
             total
         }"
    )
    .expr("run_stale()")
    .result(Value::Int(6));
}

// ── I6 — Satisfaction checking at generic instantiation ──────────────────────

/// I6: calling a bounded generic function with a type that satisfies the interface
/// (i.e. implements the required operator) must compile and return the correct value.
#[test]
fn satisfaction_check_passes_with_implementing_type() {
    code!(
        "struct Score { value: integer }
         fn OpLt(self: Score, other: Score) -> boolean { self.value < other.value }
         fn pick_first<T: Ordered>(a: T, _b: T) -> T { a }"
    )
    .expr("pick_first(Score{value:3}, Score{value:7}).value")
    .result(Value::Int(3));
}

// ── I7 — Bounded method calls on T ───────────────────────────────────────────

/// I7: calling an interface method on a T-typed receiver inside a bounded generic
/// function body must compile and produce the correct result at the call site.
#[test]
fn bounded_method_call_in_generic_body() {
    code!(
        "interface HasValue { fn get_value(self: Self) -> integer }
         struct Point { x: integer }
         fn get_value(self: Point) -> integer { self.x }
         fn extract<T: HasValue>(v: T) -> integer { v.get_value() }"
    )
    .expr("extract(Point{x:42})")
    .result(Value::Int(42));
}

// ── I8.1 — T op T via bound ──────────────────────────────────────────────────

/// I8.1: a same-type binary operator (`T < T`) inside a bounded generic function
/// body must compile when the bound declares the operator via `op <`.
#[test]
fn bounded_operator_in_generic_body() {
    code!(
        "struct Score { value: integer }
         fn OpLt(self: Score, other: Score) -> boolean { self.value < other.value }
         fn pick_min<T: Ordered>(a: T, b: T) -> T { if a < b { a } else { b } }"
    )
    .expr("pick_min(Score{value:7}, Score{value:3}).value")
    .result(Value::Int(3));
}

// ── I8.2 — Return-type propagation from interface signature ──────────────────

/// I8.2: a bounded operator whose return type is `Self` must propagate the
/// correct concrete return type — here `pick_max` returns `T` (resolved to `Score`)
/// whose `.value` field must be accessible on the result.
/// Uses stdlib `Ordered` (`op <`) so the return type `Self` is tested end-to-end.
#[test]
fn bounded_operator_self_return_type() {
    code!(
        "struct Score { value: integer }
         fn OpLt(self: Score, other: Score) -> boolean { self.value < other.value }
         fn pick_max<T: Ordered>(a: T, b: T) -> T { if a < b { b } else { a } }"
    )
    .expr("pick_max(Score{value:3}, Score{value:9}).value")
    .result(Value::Int(9));
}

// ── I8.3 — Mixed-type binary operators (T op concrete) ──────────────────────

/// I8.3: an operator with a concrete second parameter (`T * integer`) must
/// compile inside a bounded generic body and produce the correct result.
/// The interface declares a mixed-type signature: `self: Self, factor: integer -> integer`.
#[test]
fn bounded_mixed_type_operator() {
    code!(
        // @PLN25 DN3 — `self.value / divisor` types `integer?` (a variable divisor can be
        // 0); discharge with `?? 0` so the `op / -> integer` body returns a non-null value.
        "interface Divisible { op / (self: Self, divisor: integer) -> integer }
         struct Score { value: integer }
         fn OpDiv(self: Score, divisor: integer) -> integer { self.value / divisor ?? 0 }
         fn halve<T: Divisible>(v: T, n: integer) -> integer { v / n }"
    )
    .expr("halve(Score{value:42}, 6)")
    .result(Value::Int(7));
}

// ── I8.4 — Unary operators on T ─────────────────────────────────────────────

/// I8.4: a unary operator (`-T`) inside a bounded generic body must compile
/// when the bound declares the unary operator.  Uses `op -` (negation) which
/// returns `integer` to avoid struct-return allocation tracking issues.
#[test]
fn bounded_unary_operator() {
    code!(
        // @PLN25 DN3 — `self.value % modulus` types `integer?`; discharge with `?? 0`.
        "interface Modular { op % (self: Self, modulus: integer) -> integer }
         struct Score { value: integer }
         fn OpRem(self: Score, modulus: integer) -> integer { self.value % modulus ?? 0 }
         fn mod_measure<T: Modular>(v: T, m: integer) -> integer { v % m }"
    )
    .expr("mod_measure(Score{value:42}, 10)")
    .result(Value::Int(2));
}

// ── I9 — Standard library interface: Ordered ────────────────────────────────

/// I9: the `Ordered` interface from the standard library enables bounded-generic
/// functions that use `<` on user-defined types.
#[test]
fn stdlib_ordered_interface() {
    code!(
        "struct Score { value: integer }
         fn OpLt(self: Score, other: Score) -> boolean { self.value < other.value }
         fn pick_min<T: Ordered>(a: T, b: T) -> T { if a < b { a } else { b } }"
    )
    .expr("pick_min(Score{value:7}, Score{value:3}).value")
    .result(Value::Int(3));
}

// ── I9-prim — Built-in types satisfy interfaces ─────────────────────────────

/// I9-prim: built-in `integer` satisfies `Ordered` via the stdlib `OpLtInt`.
#[test]
fn builtin_integer_satisfies_ordered() {
    code!("fn pick_min<T: Ordered>(a: T, b: T) -> T { if a < b { a } else { b } }")
        .expr("pick_min(7, 3)")
        .result(Value::Int(3));
}

/// I9-prim: built-in `float` satisfies `Ordered` via the stdlib `OpLtFloat`.
#[test]
fn builtin_float_satisfies_ordered() {
    code!("fn pick_min<T: Ordered>(a: T, b: T) -> T { if a < b { a } else { b } }")
        .expr("pick_min(3.14, 2.72)")
        .result(Value::Float(2.72));
}

// ── I9-Eq — Equatable interface ─────────────────────────────────────────────

/// I9-Eq: `Equatable` enables bounded-generic equality checks on built-in types.
#[test]
fn stdlib_equatable_interface() {
    code!("fn are_equal<T: Equatable>(a: T, b: T) -> boolean { a == b }")
        .expr("are_equal(42, 42)")
        .result(Value::Boolean(true));
}

// ── I9-Add — Addable interface ──────────────────────────────────────────────

/// I9-Add: `Addable` enables bounded-generic addition on built-in types.
#[test]
fn stdlib_addable_interface() {
    code!("fn add_pair<T: Addable>(a: T, b: T) -> T { a + b }")
        .expr("add_pair(10, 32)")
        .result(Value::Int(42));
}

// ── I9.1 — Generic min_of on built-in vectors ──────────────────────────────

/// I9.1: a bounded-generic function with `Addable` bound works on built-in integers
/// — verifying that `sum_pair` with built-in `+` produces the correct result.
#[test]
fn generic_sum_pair_on_integers() {
    code!("fn sum_pair<T: Addable>(a: T, b: T) -> T { a + b }")
        .expr("sum_pair(10, 32)")
        .result(Value::Int(42));
}

/// I9.1: a bounded-generic function with `Addable` bound works on float types.
#[test]
fn generic_sum_pair_on_floats() {
    code!("fn sum_pair<T: Addable>(a: T, b: T) -> T { a + b }")
        .expr("sum_pair(1.5, 2.5)")
        .result(Value::Float(4.0));
}

// ── I9-vec — vector<T> element access in generic specialization ─────────────

/// I9-vec: generic function with `vector<T>` parameter — element access `v[0]`
/// must return the correct value after specialization for integer.
#[test]
fn generic_vector_element_access() {
    code!("fn first_of<T: Ordered>(v: vector<T>) -> T { v[0] }")
        .expr("first_of([7, 3, 9])")
        .result(Value::Int(7));
}

// ── I9.1 — Generic min/max on integer vectors ──────────────────────────────

/// I9.1: bounded-generic `min_of_pair` selects the smaller of two `vector<T>` elements.
#[test]
fn generic_min_of_vector_elements() {
    code!(
        "fn smaller<T: Ordered>(v: vector<T>) -> T {
           if v[0] < v[1] { v[0] } else { v[1] }
         }"
    )
    .expr("smaller([7, 3])")
    .result(Value::Int(3));
}

// ── I9.2 — Generic sum using Addable ────────────────────────────────────────

/// I9.2: bounded-generic sum of vector elements with `Addable` bound.
#[test]
fn generic_sum_on_integer_vector() {
    code!(
        "fn vec_sum3<T: Addable>(v: vector<T>, init: T) -> T {
           v[0] + v[1] + v[2] + init
         }"
    )
    .expr("vec_sum3([10, 20, 12], 0)")
    .result(Value::Int(42));
}

// ── I9+ — Numeric interface ─────────────────────────────────────────────────

/// I9+: `Numeric` interface with `op *` and `op -` (separate from Addable's `op +`).
#[test]
fn stdlib_numeric_interface() {
    code!("fn square<T: Numeric>(v: T) -> T { v * v }")
        .expr("square(6)")
        .result(Value::Int(36));
}

// ── I9-var — Intermediate variables in generic bodies ───────────────────────

/// I9-var: a generic function body can assign a vector element to a local
/// variable and return it.  Previously, ref_return promoted the local to a
/// hidden parameter (because T looked like a Reference), causing a codegen crash
/// when T was specialized to a value type.
#[test]
fn generic_intermediate_variable() {
    code!(
        "fn first_of<T: Ordered>(v: vector<T>) -> T {
           result = v[0]; result
         }"
    )
    .expr("first_of([7, 3, 9])")
    .result(Value::Int(7));
}

/// I9-var: a for-loop accumulator pattern inside a bounded-generic body.
#[test]
fn generic_for_loop_accumulator() {
    code!(
        "fn find_min<T: Ordered>(v: vector<T>) -> T {
           result = v[0];
           for i in 1..v.len() { if v[i] < result { result = v[i] } };
           result
         }"
    )
    .expr("find_min([7, 3, 9, 1, 5])")
    .result(Value::Int(1));
}

// ── I9.1 — Generic min_of/max_of ───────────────────────────────────────────

/// I9.1: a generic `find_max` using a for-loop accumulator on `vector<T>`.
#[test]
fn generic_max_on_integer_vector() {
    code!(
        "fn find_max<T: Ordered>(v: vector<T>) -> T {
           result = v[0];
           for i in 1..v.len() { if result < v[i] { result = v[i] } };
           result
         }"
    )
    .expr("find_max([3, 9, 1, 7, 5])")
    .result(Value::Int(9));
}

// ── I9.2 — Generic sum with identity ────────────────────────────────────────

/// I9.2: a generic `vec_sum` with caller-supplied identity element, using
/// a for-loop accumulator on `vector<T>`.
#[test]
fn generic_sum_with_identity() {
    code!(
        "fn vec_sum<T: Addable>(v: vector<T>, init: T) -> T {
           result = init;
           for i in 0..v.len() { result = result + v[i] };
           result
         }"
    )
    .expr("vec_sum([10, 20, 12], 0)")
    .result(Value::Int(42));
}

// ── I9-Sc — Scalable interface ──────────────────────────────────────────────

/// I9-Sc: `Scalable` interface with `fn scale(self, factor) -> integer`.
/// Uses a method-based interface to avoid stub-name collision with `Numeric`.
/// The method returns integer to avoid struct-return allocation issues.
#[test]
fn stdlib_scalable_interface() {
    code!(
        "struct Weight { grams: integer }
         fn scale(self: Weight, factor: integer) -> integer {
             self.grams * factor
         }
         fn scaled<T: Scalable>(v: T, n: integer) -> integer { v.scale(n) }"
    )
    .expr("scaled(Weight{grams: 21}, 2)")
    .result(Value::Int(42));
}

// ── I9-stub — Interface stub naming collision fix ───────────────────────────

/// I9-stub: two interfaces can declare the same operator without conflicting.
/// Previously, both `Addable { op + }` and `Numeric { op * ; op - }` worked, but
/// defining a third interface with `op +` would fail on "Cannot redefine OpAdd".
/// Tests with integer (no struct allocation issues).
#[test]
fn two_interfaces_same_operator_no_conflict() {
    code!(
        "interface Summable { op + (self: Self, other: Self) -> Self }
         fn total<T: Summable>(a: T, b: T) -> T { a + b }"
    )
    .expr("total(10, 32)")
    .result(Value::Int(42));
}

// ── I9.1 — Replace min_of/max_of with bounded generics ─────────────────────

/// I9.1: stdlib `min_of` is now a bounded generic that works on any Ordered type.
#[test]
fn stdlib_min_of_generic() {
    expr!("min_of([7, 3, 9, 1, 5])").result(Value::Int(1));
}

/// I9.1: stdlib `max_of` is now a bounded generic that works on any Ordered type.
#[test]
fn stdlib_max_of_generic() {
    expr!("max_of([3, 9, 1, 7])").result(Value::Int(9));
}

// ── I9.2 — Generic sum with identity ────────────────────────────────────────

/// I9.2: stdlib `sum` function with caller-supplied identity element.
#[test]
fn stdlib_sum_generic() {
    expr!("sum([10, 20, 12], 0)").result(Value::Int(42));
}

// ── I9-Pr — Printable interface ─────────────────────────────────────────────

/// I9.1: generic min_of/max_of work on float vectors — the key benefit of generifying.
#[test]
fn stdlib_min_of_float() {
    expr!("min_of([3.14, 2.72, 1.41])").result(Value::Float(1.41));
}

/// I9.1: generic max_of on float vectors.
#[test]
#[allow(clippy::approx_constant)]
fn stdlib_max_of_float() {
    expr!("max_of([3.14, 2.72, 1.41])").result(Value::Float(3.14));
}

// ── I9-text — Text-returning interface methods ──────────────────────────────

/// I9-text: a text-returning method in an interface works when the concrete
/// implementation returns a text field (not a format string — format strings
/// with text-return generics need additional stack-layout work).
#[test]
fn generic_text_returning_method() {
    code!(
        "struct Tag { label: text }
         fn to_text(self: Tag) -> text { self.label }
         fn show<T: Printable>(v: T) -> text { v.to_text() }"
    )
    .expr("show(Tag{label: \"hello\"})")
    .result(Value::Text("hello".to_string()));
}

// ── I9-Pr — Printable interface ─────────────────────────────────────────────

/// I9-Pr: `Printable` interface used with a text-field return.
#[test]
fn stdlib_printable_interface() {
    code!(
        "struct Tag { label: text }
         fn to_text(self: Tag) -> text { self.label }
         fn show<T: Printable>(v: T) -> text { v.to_text() }"
    )
    .expr("show(Tag{label: \"world\"})")
    .result(Value::Text("world".to_string()));
}

// ── CO1.7 — Coroutine yield from for-loops ──────────────────────────────────

/// CO1.7: yield from inside a range-based for-loop.
#[test]
fn coroutine_yield_from_range_loop() {
    code!(
        "fn yield_range() -> iterator<integer> {
           yield 1;
           for i in 0..3 { yield i * 10 };
           yield 99
         }"
    )
    .expr(
        "{
        total = 0;
        for x in yield_range() { total = total + x };
        total
    }",
    )
    .result(Value::Int(130));
}

/// CO1.7: yield from inside a vector for-loop.
#[test]
fn coroutine_yield_from_vector_loop() {
    code!(
        "fn yield_items(v: vector<integer>) -> iterator<integer> {
           yield -1;
           for e in v { yield e };
           yield -2
         }"
    )
    .expr(
        "{
        total = 0;
        for x in yield_items([10, 20, 30]) { total = total + x };
        total
    }",
    )
    .result(Value::Int(57));
}

/// CO1.7-text: yield from inside a text character for-loop.
/// Previously infinite-looped because the character null sentinel (i32::MIN)
/// was not recognised by `op_conv_bool_from_character`.
#[test]
fn coroutine_yield_from_text_loop() {
    code!(
        "fn yield_chars(s: text) -> iterator<character> {
           yield ' ';
           for c in s { yield c }
         }"
    )
    .expr(
        "{
        count = 0;
        for _ch in yield_chars(\"ab\") { count = count + 1 };
        count
    }",
    )
    .result(Value::Int(3));
}

/// CO1.7-char: a plain character iterator (no text loop) must also exhaust
/// correctly — the character null sentinel fix covers this case too.
#[test]
fn coroutine_character_iterator_exhausts() {
    code!(
        "fn gen_chars() -> iterator<character> {
           yield 'x';
           yield 'y'
         }"
    )
    .expr(
        "{
        count = 0;
        for _c in gen_chars() { count = count + 1 };
        count
    }",
    )
    .result(Value::Int(2));
}

/// CO1.7-store: yield from inside a vector-of-structs for-loop.
#[test]
fn coroutine_yield_from_struct_vector_loop() {
    code!(
        "struct Node { value: integer }
         fn yield_values(ns: vector<Node>) -> iterator<integer> {
           yield 0;
           for n in ns { yield n.value }
         }"
    )
    .expr(
        "{
        total = 0;
        for v in yield_values([Node{value:10}, Node{value:20}, Node{value:30}]) {
            total = total + v
        };
        total
    }",
    )
    .result(Value::Int(60));
}

/// CO1.7-field-text: yield characters from a struct's text field.
#[test]
fn coroutine_yield_from_field_text_loop() {
    code!(
        "struct Item { name: text }
         fn yield_name_chars(it: Item) -> iterator<character> {
           yield ' ';
           for c in it.name { yield c }
         }"
    )
    .expr(
        "{
        count = 0;
        for _ch in yield_name_chars(Item{name: \"hi\"}) { count = count + 1 };
        count
    }",
    )
    .result(Value::Int(3));
}

// ── CO1.8 — Multi-text parameters + nested-block safety ─────────────────────

/// CO1.8a: a generator with two text parameters must serialise both at create.
#[test]
fn coroutine_multi_text_params() {
    code!(
        "fn join_chars(a: text, b: text) -> iterator<character> {
           for c in a { yield c };
           for c in b { yield c }
         }"
    )
    .expr(
        "{
        count = 0;
        for _c in join_chars(\"he\", \"lo\") { count = count + 1 };
        count
    }",
    )
    .result(Value::Int(4));
}

/// CO1.8b: a text local created after the first yield must survive resume.
#[test]
fn coroutine_text_local_after_yield() {
    code!(
        "fn lazy_labels() -> iterator<integer> {
           yield 1;
           label = \"second\";
           yield len(label)
         }"
    )
    .expr(
        "{
        total = 0;
        for v in lazy_labels() { total = total + v };
        total
    }",
    )
    .result(Value::Int(7));
}

/// CO1.8c: a text local inside a nested for-loop block must be freed correctly.
#[test]
fn coroutine_text_local_nested_block() {
    code!(
        "fn text_lens(v: vector<text>) -> iterator<integer> {
           for item in v {
             s = \"{item}!\";
             yield len(s)
           }
         }"
    )
    .expr(
        "{
        total = 0;
        for n in text_lens([\"hi\", \"bye\"]) { total = total + n };
        total
    }",
    )
    .result(Value::Int(7));
}

// ── A8.1 — Open-ended bounds on sorted/index ────────────────────────────────

/// A8.1: `col[lo..]` iterates from `lo` to the end of a sorted collection.
#[test]
fn sorted_open_end_range() {
    code!(
        "struct Elm { key: integer, val: integer }
         struct Db { map: sorted<Elm[key]> }
         fn sum_from(db: Db, lo: integer) -> integer {
           total = 0;
           for e in db.map[lo..] { total = total + e.val };
           total
         }"
    )
    .expr("sum_from(Db{map:[Elm{key:1,val:10}, Elm{key:2,val:20}, Elm{key:3,val:30}]}, 2)")
    .result(Value::Int(50));
}

/// A8.1: `col[..hi]` iterates from start to `hi` (exclusive).
#[test]
fn sorted_open_start_range() {
    code!(
        "struct Elm { key: integer, val: integer }
         struct Db { map: sorted<Elm[key]> }
         fn sum_to(db: Db, hi: integer) -> integer {
           total = 0;
           for e in db.map[..hi] { total = total + e.val };
           total
         }"
    )
    .expr("sum_to(Db{map:[Elm{key:1,val:10}, Elm{key:2,val:20}, Elm{key:3,val:30}]}, 3)")
    .result(Value::Int(30));
}

// ── A8.2 — Range slicing on sorted ──────────────────────────────────────────

/// A8.2: `sorted[lo..hi]` range iteration works on sorted collections.
#[test]
fn sorted_range_iteration() {
    code!(
        "struct Elm { key: integer, val: integer }
         struct Db { map: sorted<Elm[key]> }
         fn sum_range(db: Db, lo: integer, hi: integer) -> integer {
           total = 0;
           for e in db.map[lo..hi] { total = total + e.val };
           total
         }"
    )
    .expr("sum_range(Db{map:[Elm{key:1,val:10}, Elm{key:2,val:20}, Elm{key:3,val:30}, Elm{key:4,val:40}]}, 2, 4)")
    .result(Value::Int(50));
}

// ── A8.4 — Comprehensions on key ranges ─────────────────────────────────────

/// A8.4: `[for v in col[lo..hi] { expr }]` builds a vector from a range.
#[test]
fn sorted_range_comprehension() {
    code!(
        "struct Elm { key: integer, val: integer }
         struct Db { map: sorted<Elm[key]> }
         fn vals_in_range(db: Db, lo: integer, hi: integer) -> vector<integer> {
           [for e in db.map[lo..hi] { e.val }]
         }"
    )
    .expr("vals_in_range(Db{map:[Elm{key:1,val:10}, Elm{key:2,val:20}, Elm{key:3,val:30}]}, 1, 3).len()")
    .result(Value::Int(2));
}

// ── A8.6 — Match on collection results ──────────────────────────────────────

/// A8.6: nullable collection lookup — `if !col[k]` checks for missing keys.
#[test]
fn sorted_nullable_lookup() {
    code!(
        "struct Elm { key: integer, val: integer }
         struct Db { map: sorted<Elm[key]> }
         fn lookup_val(db: Db, k: integer) -> integer {
           if !db.map[k] { -1 } else { db.map[k].val }
         }"
    )
    .expr("lookup_val(Db{map:[Elm{key:1,val:10}, Elm{key:2,val:20}]}, 2)")
    .result(Value::Int(20));
}

// ── A8.5 — Reverse range iteration ──────────────────────────────────────────

/// A8.5: `rev(col[lo..hi])` iterates a range in reverse key order.
#[test]
fn sorted_reverse_range() {
    code!(
        "struct Elm { key: integer, val: integer }
         struct Db { map: sorted<Elm[key]> }
         fn rev_sum(db: Db) -> integer {
           result = 0;
           for e in rev(db.map[1..3]) { result = result * 100 + e.val };
           result
         }"
    )
    .expr("rev_sum(Db{map:[Elm{key:1,val:10}, Elm{key:2,val:20}, Elm{key:3,val:30}]})")
    .result(Value::Int(2010));
}

/// C99 (@PLN102 H8): a sorted's subscript is uniformly KEY-addressed, never
/// positional — the single lookup `s[k]` AND the range `s[lo..hi]` both address
/// by key.  Keys are deliberately placed far from positions (10,20,30,40) so a
/// positional misread yields a wildly different number:
///   key-addressed: range[15..35)=keys 20,30 (val 500); s[20]=key20 (val 200);
///                  s[1]=null (no key 1, flag 0)  ->  500*1000 + 200 + 0 = 500200
///   positional:    range[15..35)=empty (0); s[20]=pos 20=null (-1);
///                  s[1]=pos 1=key20 present (flag 1)  ->  0 - 1 + 1 = 0
#[test]
fn sorted_subscript_is_key_addressed_not_positional() {
    code!(
        "struct Elm { key: integer, val: integer }
         struct Db { map: sorted<Elm[key]> }
         fn probe(db: Db) -> integer {
           range_sum = 0;
           for e in db.map[15..35] { range_sum = range_sum + e.val };
           hit = db.map[20];
           hit_val = if hit { hit.val } else { -1 };
           miss = db.map[1];
           miss_flag = if miss { 1 } else { 0 };
           range_sum * 1000 + hit_val + miss_flag
         }"
    )
    .expr("probe(Db{map:[Elm{key:10,val:100}, Elm{key:20,val:200}, Elm{key:30,val:300}, Elm{key:40,val:400}]})")
    .result(Value::Int(500200));
}

/// @PLN109 pre-step: loft's number lexer accepts the full IEEE/JSON exponent
/// forms — uppercase `E` and a `+` sign — not just `e` / `e-`. This lets loft's
/// own lexer parse valid JSON numbers when the hand-rolled JSON scanner is
/// retired, and is harmless for normal loft (no `<digit>E<digit>` literal existed
/// before; hex `0x1E` is consumed separately).
#[test]
fn number_exponent_accepts_uppercase_e_and_plus_sign() {
    code!(
        "fn ok() -> boolean {
           1E5 == 1e5 && 1e+5 == 1e5 && 1.5E3 == 1500.0 && 2.5e-2 == 0.025 && 1E+2 == 100.0
         }"
    )
    .expr("ok()")
    .result(Value::Boolean(true));
}

// ── A8.3 — Partial-key match iterator ───────────────────────────────────────

/// A8.3: `idx[k1]` on a multi-key index iterates all elements matching k1.
#[test]
fn index_partial_key_match() {
    code!(
        "struct Elm { nr: integer, key: text, val: integer }
         struct Db { idx: index<Elm[nr, -key]> }
         fn sum_by_nr(db: Db, n: integer) -> integer {
           total = 0;
           for e in db.idx[n] { total = total + e.val };
           total
         }"
    )
    .expr("sum_by_nr(Db{idx:[Elm{nr:1,key:\"a\",val:10}, Elm{nr:1,key:\"b\",val:20}, Elm{nr:2,key:\"c\",val:30}]}, 1)")
    .result(Value::Int(30));
}

// ── A8.1-idx — Open-ended bounds on index ───────────────────────────────────

/// A8.1-idx: open-ended bounds work on index collections too.
#[test]
fn index_open_end_range() {
    code!(
        "struct Elm { nr: integer, val: integer }
         struct Db { idx: index<Elm[nr]> }
         fn sum_from(db: Db, lo: integer) -> integer {
           total = 0;
           for e in db.idx[lo..] { total = total + e.val };
           total
         }"
    )
    .expr("sum_from(Db{idx:[Elm{nr:1,val:10}, Elm{nr:2,val:20}, Elm{nr:3,val:30}]}, 2)")
    .result(Value::Int(50));
}

// ── A8.5-idx — Reverse range on index ───────────────────────────────────────

/// A8.5-idx: `rev(idx[lo..hi])` works on index collections.
#[test]
fn index_reverse_range() {
    code!(
        "struct Elm { nr: integer, val: integer }
         struct Db { idx: index<Elm[nr]> }
         fn rev_sum(db: Db) -> integer {
           result = 0;
           for e in rev(db.idx[1..3]) { result = result * 100 + e.val };
           result
         }"
    )
    .expr("rev_sum(Db{idx:[Elm{nr:1,val:10}, Elm{nr:2,val:20}, Elm{nr:3,val:30}]})")
    .result(Value::Int(2010));
}
