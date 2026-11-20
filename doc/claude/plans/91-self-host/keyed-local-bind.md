# @PLN91 — a keyed local bound from a call, a local, or a `?? []` (prepared subject)

**The family:** "a keyed local bound from something other than a local" (35 sites in 18
programs at try 521964bbe). The main sub-shapes: a keyed local bound from a CALL
(`src = mk_sorted(w)`, `seen = mkset([...])`), REBOUND from a call that takes it
(`s = grow_hash(s, 10)`, all five keyed kinds), and bound from a DISCHARGE (`hh = h ?? []`).
Joins of calls (`s = if c { f() } else { g() }`) are a different shape and aren't covered here.

**Judged right** on the interpreter (LOFT_POISON), --native-c (leak check) and --native, 200
passes. Hand-computed: `a = mk(3)` (v 0,10,20) then `a = grow(a, 7)` → 4 entries, total 37;
`src = mk(2); b = src; b += [...]` → src keeps 2, b holds 3 (a bind from a local COPIES);
`opt(mk(3))` with `hh = h ?? []` → 3030; `opt(null)` → 0.

## Rust's form

```
a = mk(n)                (a call: the copy RELEASES the call's store, the 0x8000 source-free bit)
    a: hash<E,["k"]> = null;
    OpReplaceKeyed(n_mk(n), a, 32869);              <- 32869 = 101 (hash<E[k]>) | 0x8000

a = grow(a, 7)           (a rebind from a call that TAKES the local: the local's store protected
                          from the call's frees for the duration)
    n_protect_store_frees(a);
    OpReplaceKeyed(n_grow(a, 7), a, 32869);
    n_unprotect_store_frees(a);

b = src                  (a local: a copy, NO source-free bit; src keeps its store)
    b: hash<E,["k"]> = null;
    OpReplaceKeyed(src, b, 101);

hh = h ?? []             (a discharge: the empty arm mints a keyed store of its own)
    __kvb_1: hash<E,["k"]> = null;                  <- hoisted to the function's top
    hh: hash<E,["k"]> = null;
    OpReplaceKeyed(if OpNot(OpVectorIsNull(h)) h else {#Vector:hash<E,["k"]>
        __kvb_1 = null;
        OpDatabase(__kvb_1, 101);
        __kvb_1 }, hh, 101);                         <- no source-free bit
```

**What to code:** every bind of a keyed local is `local = null; OpReplaceKeyed(<source>,
local, <keyed type>)`. The type carries `| 0x8000` exactly when the source is a CALL (its fresh
store is released by the copy). When the call is handed the bound local itself, the bind is
bracketed by `n_protect_store_frees(local)` and `n_unprotect_store_frees(local)`. A `?? []`
source is the `if` whose empty arm mints `__kvb_N` (null-init hoisted to the top) with
`OpDatabase(__kvb_N, <keyed type>)`.

## The sites (try 521964bbe)

```
kr-suite/rules/binding/B-Copy/1319-a-nullable-whole-value-bind-copies-like-its-dense-twin.loft::n_b19_the_keyed_kinds_are_the_control:150
kr-suite/rules/binding/B-Copy/a-keyed-local-rebound-from-a-call-that-reads-it-keeps-its-records.loft::n_main:47
kr-suite/rules/calls/F-Ret/2562-a-returned-keyed-field-of-a-local-record-is-its-own-store.loft::n_test_fr2562_a_parameters_field_stays_the_callers:101
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_loop_pass:162
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_once_join:152
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_once_self:155
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_twice_self:158
kr-suite/rules/collections/Col-Insert-Absent/an-instantiated-collection-replaces-a-repeated-key.loft::n_test_absent_in_loop_rebuilt:98
kr-suite/rules/collections/Col-Insert-Absent/an-instantiated-collection-replaces-a-repeated-key.loft::n_test_hash_absent_then_duplicate:33
kr-suite/rules/collections/Col-Insert-Absent/an-instantiated-collection-replaces-a-repeated-key.loft::n_test_sorted_and_index_absent_then_duplicate:42
kr-suite/rules/collections/Col-Insert-Absent/an-instantiated-collection-replaces-a-repeated-key.loft::n_test_spatial_absent_keeps_both:65
kr-suite/rules/collections/Col-Insert-Absent/an-instantiated-collection-replaces-a-repeated-key.loft::n_test_trie_absent_then_duplicate:57
kr-suite/rules/heap/H-SwapIn/a-rebind-from-a-call-may-exchange-the-stores-and-answers-what-the-copy-answers.loft::n_test_a_keyed_local_rebound_from_a_call_takes_the_store_only_when_it_is_give:84
kr-suite/rules/heap/H-SwapKeyed/a-keyed-local-rebound-from-a-call-exchanges-its-store-and-answers-what-the-copy-answers.loft::n_test_a_callee_handing_back_another_locals_collection:286
kr-suite/rules/heap/H-SwapKeyed/a-keyed-local-rebound-from-a-call-exchanges-its-store-and-answers-what-the-copy-answers.loft::n_test_rebound_from_a_join:310
kr-suite/rules/heap/H-SwapKeyed/a-keyed-local-rebound-from-a-call-exchanges-its-store-and-answers-what-the-copy-answers.loft::n_walk_hash:83
kr-suite/rules/heap/H-SwapKeyed/a-keyed-local-rebound-from-a-call-exchanges-its-store-and-answers-what-the-copy-answers.loft::n_walk_index:153
kr-suite/rules/heap/H-SwapKeyed/a-keyed-local-rebound-from-a-call-exchanges-its-store-and-answers-what-the-copy-answers.loft::n_walk_sorted:118
kr-suite/rules/heap/H-SwapKeyed/a-keyed-local-rebound-from-a-call-exchanges-its-store-and-answers-what-the-copy-answers.loft::n_walk_spatial:223
kr-suite/rules/heap/H-SwapKeyed/a-keyed-local-rebound-from-a-call-exchanges-its-store-and-answers-what-the-copy-answers.loft::n_walk_trie:188
kr-suite/rules/iteration/I-For/2573-a-for-evaluates-its-sorted-or-index-source-once.loft::n_main:44
kr-suite/rules/layout/L-Null/a-nullable-keyed-collection-crosses-a-call-as-its-dense-twin.loft::n_test_a_keyed_return_that_can_be_absent_hands_back_what_it_built:286
kr-suite/rules/layout/L-Null/a-nullable-keyed-collection-crosses-a-call-as-its-dense-twin.loft::n_test_a_returned_nullable_keyed_parameter_is_still_the_callers:141
kr-suite/rules/ownership/O-Complete/1142-a-keyed-return-through-a-join-frees-the-arm-that-did-not-run.loft::n_main:135
kr-suite/rules/ownership/O-Complete/a-coalesce-fallback-read-inline-is-released.loft::n_keyed_bound:38
kr-suite/rules/ownership/O-Deps/1335-a-fn-ref-return-of-any-shape-maps-its-deps-into-the-caller.loft::n_test_keyed_in_a_loop:72
kr-suite/rules/ownership/O-Deps/1335-a-fn-ref-return-of-any-shape-maps-its-deps-into-the-caller.loft::n_test_keyed_joins:61
kr-suite/rules/ownership/O-Move/a-keyed-collection-returned-borrowed-is-the-callers-copy.loft::n_test_a_borrowed_return_of_every_keyed_kind_is_a_copy:45
kr-suite/rules/ownership/O-NoDiverge/1328-a-fn-ref-rebind-frees-the-store-it-displaces-a-records-or-a-vectors.loft::n_test_a_keyed_local_rebound_from_a_fn_ref:143
kr-suite/rules/ownership/O-Owner/2258-a-keyed-collection-from-a-fn-ref-has-one-owner-per-pass.loft::n_test_a_rebound_owner_and_a_coalesce_default:56
kr-suite/rules/ownership/O-Proxy/1320-a-branch-joined-binding-frees-the-arm-that-minted.loft::n_test_a_keyed_branch_reaches_the_op_that_materialises_it:82
kr-suite/rules/ownership/O-Proxy/1320-a-branch-joined-binding-frees-the-arm-that-minted.loft::n_test_the_borrow_arm_leaves_the_callers_collection_intact:138
kr-try/tests/scripts/157-mint-hoist.loft::n_test_mint_hoist:122
kr-try/tests/scripts/a-keyed-bind-from-a-call-on-a-vector-literal-compiles-natively.loft::n_count:27
kr-try/tests/scripts/a-keyed-bind-from-a-call-on-a-vector-literal-compiles-natively.loft::n_main:33
```
