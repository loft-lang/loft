# @PLN91 — a vector of function values (prepared subject)

**The family:** "a vector of this element type: TyFunction", 28 sites in 12 programs at try
ca4065330 (1679's rebound-name cells, 1602, …): `fs: vector<fn(integer) -> integer> = [one1679]`,
`ts: vector<fn() -> text> = [text_one1679]`, then `+=`, `for f in fs { f(x) }`, `fs[i](x)`.
A CAPTURING closure cannot be stored in a collection; Rust refuses it by name ("a capturing closure
cannot be stored in a collection"), so only named functions and non-capturing lambdas occur.

**Judged right** on the interpreter (LOFT_POISON), --native-c (leak check) and --native, with 300
passes. Expected values computed by hand: `[one, dbl, fn(x) { x + 10 }]` summed over `f(3)` → 23,
`fs[1](4)` → 8, `ts[0]()` → `"t"`.

## Rust's form

```
fs: vector<function([integer…], integer…, deps [], constparams(0))>["__vdb_1"] = OpGetField(__vdb_1, 0, …);   <- after OpDatabase(__vdb_1, 102)
literal item / append (each):
    _elm_1: ref(boolean)["fs"] = OpNewRecord(fs, 101, 65535);       <- ref(boolean): unique_elm_var's Reference(0), a function type has no record def
    ___fn_ref_tmp_1: function(…) = 874;                              <- the function's DEFINITION NUMBER (a lambda's too: 880)
    OpSetInt4(_elm_1, 0, FnRefDnr(___fn_ref_tmp_1));                  <- the element is a 4-byte record holding it
    OpFinishRecord(fs, _elm_1, 101, 65535);
for f in fs:
    _vector_1: vector<function(…)>["fs"] = fs;  f#index = -1;
    f: function(…, deps [9]) = {#iter next  f#index = OpAddInt(f#index, 1);
          {#fn_ref_field_read: function(…)  OpGetInt4(OpGetVectorNullable(_vector_1, 4, f#index), 0);  OpNullRefSentinel(); } };
    if OpNot(OpLtInt(0, FnRefDnr(f))) break(0);                       <- the end test: a definition number of 0 is past the end
    s = OpAddInt(s, fn_ref[11](3));                                    <- the call through the value
```

Note `fn_ref_field_read`'s second statement, `OpNullRefSentinel()`, after the read. It is in Rust's
IR and the port mirrors it.

**What to code:** a vector whose element type is a function type has a 4-byte element (record
type 101 here, the int4 element vector). Each item is `___fn_ref_tmp_N = <def nr>` and
`OpSetInt4(_elm_N, 0, FnRefDnr(tmp))` between `OpNewRecord` / `OpFinishRecord`, with `_elm_N` typed
`Reference(0)`. A walk reads each element through the `fn_ref_field_read` block and ends on
`!(0 < FnRefDnr(f))`. A call through `f` or `fs[i]` is the fn-ref call the port already has for a
function-valued local.

## The sites (try ca4065330)

```
kr-suite/rules/binding/B-Ref-Lvalue/1602-a-ref-text-parameter-links-a-text-field-or-element.loft::n_test_through_a_function_value
kr-suite/rules/binding/B-Scope/1679-a-call-through-a-rebound-name-reaches-the-new-binding.loft::n_test_a_lambda_that_captures_the_new_binding
kr-suite/rules/binding/B-Scope/1679-a-call-through-a-rebound-name-reaches-the-new-binding.loft::n_test_a_link_naming_the_new_binding
kr-suite/rules/binding/B-Scope/1679-a-call-through-a-rebound-name-reaches-the-new-binding.loft::n_test_a_loop_variable_is_not_what_the_call_reaches
kr-suite/rules/binding/B-Scope/1679-a-call-through-a-rebound-name-reaches-the-new-binding.loft::n_test_a_text_returning_call_carries_its_work_buffers
kr-suite/rules/binding/B-Scope/1679-a-call-through-a-rebound-name-reaches-the-new-binding.loft::n_test_a_zero_argument_call_resolves_at_its_own_site
kr-suite/rules/binding/B-Scope/1679-a-call-through-a-rebound-name-reaches-the-new-binding.loft::n_test_the_call_may_stand_in_a_comprehension
kr-suite/rules/binding/B-Scope/1679-a-call-through-a-rebound-name-reaches-the-new-binding.loft::n_test_the_call_may_stand_in_a_nested_block_or_a_later_loop
kr-suite/rules/binding/B-Scope/1679-a-call-through-a-rebound-name-reaches-the-new-binding.loft::n_test_the_later_value_may_capture
kr-suite/rules/binding/B-Scope/1679-a-call-through-a-rebound-name-reaches-the-new-binding.loft::n_test_the_reads_that_were_never_wrong
kr-suite/rules/binding/B-Scope/1679-a-call-through-a-rebound-name-reaches-the-new-binding.loft::n_test_two_loops_over_one_name_then_a_rebind
kr-suite/rules/closures/L-Apply/a-function-value-that-returns-a-function-compiles-natively.loft::n_main
kr-suite/rules/closures/L-FnAbsent/1642-calling-an-absent-function-answers-null.loft::n_test_a_boolean_return
kr-suite/rules/closures/L-FnAbsent/1642-calling-an-absent-function-answers-null.loft::n_test_a_float_return
kr-suite/rules/closures/L-FnAbsent/1642-calling-an-absent-function-answers-null.loft::n_test_an_integer_return
kr-suite/rules/closures/L-FnAbsent/1642-calling-an-absent-function-answers-null.loft::n_test_arguments_are_evaluated_either_way
kr-suite/rules/closures/L-FnAbsent/1642-calling-an-absent-function-answers-null.loft::n_test_a_text_return
kr-suite/rules/closures/L-FnAbsent/1642-calling-an-absent-function-answers-null.loft::n_test_a_vector_return
kr-suite/rules/closures/L-FnAbsent/1642-calling-an-absent-function-answers-null.loft::n_test_a_void_function_is_skipped
kr-suite/rules/closures/L-FnAbsent/1642-calling-an-absent-function-answers-null.loft::n_test_in_a_loop
kr-suite/rules/coroutines/G-Call/a-lambda-that-yields-or-returns-an-iterator-is-a-generator-in-every-position.loft::n_test_an_absent_lambda_generator_yields_nothing
kr-suite/rules/rewrites/R-FnRefValue/a-fn-ref-dispatch-answers-the-tuple-its-arms-answer.loft::n_main
kr-suite/rules/types/N-Opt/has-null-is-owed-by-every-rule-that-constructs-an-optional.loft::n_main
kr-suite/rules/types/T-Chk-Lam/a-lambda-body-converts-into-its-expected-return.loft::n_main
kr-try/tests/scripts/2001-a-work-buffer-through-a-function-value-is-its-own-store.loft::n_main
kr-try/tests/scripts/507-par-vector-fnref.loft::n_main
kr-try/tests/scripts/a-library-function-is-a-value-under-its-qualified-name.loft::n_main
kr-try/tests/scripts/repro_p343.loft::n_main
```
