# @PLN91 — the record-return refusals, grouped by Rust's own verdict

The two largest refusal families of the port, *a function returning a record from something
other than a literal* and *a `return` of a record that is not a literal*, are where the port
still decides Rust's return selector by SHAPE (`rename_candidate`, `var_return`,
`build_into_buffer`).  Rust decides it by two verdicts per return site: the reference delivery
(`classify_reference_delivery`, `src/parser/control.rs`, `enum RefDelivery`) and the per-variable
promotion (`classify_ret_promotion`, `enum RetPromotion`).  Below, every refused function is
filed under the pass-2 verdicts Rust reaches at its return sites, so each group is one rule to
port, ordered by what it buys.

## The verdicts

| verdict | what Rust does at the site |
|---|---|
| promotion `MergeAttr` | the returned variable is already an attribute (a parameter, or a buffer renamed on pass 1): its index joins the return deps; nothing is placed |
| promotion `Bind` | the one-buffer rule: a named local keeps its own store and is deep-copied into `__retbuf` at the return; a parser work ref is substituted by the buffer instead |
| promotion `Rename` | the `__retbuf` attribute takes the local's name (pass 1), so the local IS the buffer |
| promotion `SkipReassigned`, `Grow` | the reassigned-local fall-through; a buffer that grows |
| delivery `Rename(ls)` | the tail's work ref(s) become `__retbuf` (`ref_return` + `nrvo_collapse_tail_set`) |
| delivery `MaterializeView` | the tail borrows a LOCAL's store: copied into an owned work ref first (`materialize_view_return`), then renamed |
| delivery `BuildIntoBuffer` | a fresh struct literal tail: its work ref is substituted by the buffer variable |
| delivery `AsIs` | the tail already delivers; nothing emitted |

Reading a row: `deliv:` is the delivery verdict and `promo:` each promotion verdict met at the
function's return sites on pass 2.  `(·)` stands for the variable names.  "no pass-2 route line"
are functions whose sites printed no verdict (7; read them one by one).

## Regenerating it

On the tester's side, `kr-tools/rr_verdicts.py <tip>` runs every program of the two families once
with `LOFT_NO_CACHE=1 LOFT_STATE_TRACE=1 LOFT_TRACE_RETPROMO=1` and maps the `route` lines to their
functions by line range (`LOFT_NO_CACHE` matters: a cached parse traces nothing).  One function by
hand: `LOFT_NO_CACHE=1 LOFT_TRACE_RETPROMO=1 loft --interpret p.loft 2>&1 | grep "fn=<name> "`.

## At 97e4cb064 (the port's tip, 2026-11-19)

```
92 refused functions in the record-return families at 97e4cb064, by Rust's pass-2 verdicts

## promo:MergeAttr   — 17 functions in 13 programs
   kr-suite/rules/binding/B-Copy/a-binding-from-a-value-branch-copies-the-arm-it-takes.loft::n_early   [block EnumUnitLit]
   kr-suite/rules/calls/F-ParamTuple/a-by-value-tuple-parameter-is-the-callees-own-value.loft::n_pick   [other]
   kr-suite/rules/calls/F-Rec/a-recursive-call-gets-its-own-frame.loft::n_passdown   [if]
   kr-suite/rules/calls/F-Return/a-function-answers-the-value-its-return-or-its-tail-names.loft::n_rec_if   [call n_same]
   kr-suite/rules/formatting/F-Target-Pos/every-position-with-an-expected-type-builds-the-target.loft::n_build   [block Formatted string]
   kr-suite/rules/ownership/O-Move/a-member-of-a-tuple-argument-handed-back-is-the-callers.loft::n_first   [other]
   kr-suite/rules/ownership/O-Proxy/2314-a-vector-bound-from-an-owned-match-subject-is-copied-out.loft::n_set_value   [block match]
   kr-suite/rules/rewrites/R-TailSelf/a-result-held-in-a-store-is-delivered-as-the-call-would.loft::n_early   [call n_early]
   kr-suite/rules/types/N-Store/2006-a-returned-field-of-a-record-that-is-null-is-null.loft::n_b2   [call OpGetField]
   kr-suite/rules/types/N-Store/2006-a-returned-field-of-a-record-that-is-null-is-null.loft::n_b6   [call OpGetField]
   loft-keyed-ranges/tests/scripts/2016-a-chain-site-whose-callee-returns-its-parameter-copies-what-it-hands-up.loft::n_c3   [call n_id2]
   loft-keyed-ranges/tests/scripts/2016-a-chain-site-whose-callee-returns-its-parameter-copies-what-it-hands-up.loft::n_w2   [call n_j]
   loft-keyed-ranges/tests/scripts/2016-a-chain-site-whose-callee-returns-its-parameter-copies-what-it-hands-up.loft::n_w3   [call n_id2]
   loft-keyed-ranges/tests/scripts/2016-a-chain-site-whose-callee-returns-its-parameter-copies-what-it-hands-up.loft::n_w4   [call n_id2]
   loft-keyed-ranges/tests/scripts/2561-a-record-copied-out-of-a-view-is-absent-where-its-source-is.loft::n_mid_call   [call OpVectorRef]
   loft-keyed-ranges/tests/scripts/978-branch-join-carries-both-arms-borrows.loft::n_i978_tail   [if]
   loft-keyed-ranges/tests/scripts/a-chain-exit-hands-the-placed-buffer-through.loft::n_chain_view   [call n_view_exit]

## promo:Bind   — 11 functions in 8 programs
   kr-suite/rules/calls/F-ParamHeap/a-heap-argument-shares-the-callers-store-and-nothing-in-the-frame-releases-it.loft::n_head_from_parameter   [block vector_match]
   kr-suite/rules/heap/H-Free/an-early-exit-frees-what-the-frame-owns-whoever-would-fill-it-later.loft::n_pc   [block scalar_match]
   kr-suite/rules/layout/L-Null/a-nullable-pointer-or-handle-keeps-its-representation-and-its-sentinel.loft::n_head   [block vector_match]
   kr-suite/rules/ownership/O-Complete/2384-a-record-copied-into-its-buffer-releases-the-buffer-when-the-arm-returns-early.loft::n_by_match   [if]
   kr-suite/rules/ownership/O-Complete/2384-a-record-copied-into-its-buffer-releases-the-buffer-when-the-arm-returns-early.loft::n_no_early   [block scalar_match]
   kr-suite/rules/ownership/O-Deps/1414-a-nullable-element-binding-borrows-its-subject.loft::n_first_n   [block vector_match]
   kr-suite/rules/ownership/O-Unknown/1659-an-opaque-fn-ref-record-result-is-owned-once.loft::n___lambda_test_a_bound_capture_written_through_is_a_copy_0   [call OpGetDbRef]
   kr-suite/rules/ownership/O-Unknown/1659-an-opaque-fn-ref-record-result-is-owned-once.loft::n___lambda_test_a_capture_or_mint_bound_leaves_the_capture_0   [call OpGetDbRef]
   kr-suite/rules/ownership/O-Unknown/1659-an-opaque-fn-ref-record-result-is-owned-once.loft::n___lambda_test_a_rebound_capture_is_a_copy_0   [call OpGetDbRef]
   kr-suite/rules/types/T-Chk-Lam/a-lambda-body-converts-into-its-expected-return.loft::n_app_s   [other]
   loft-keyed-ranges/tests/scripts/1990-a-nullable-result-that-joins-a-dense-leaf-returns-in-registers.loft::n_j20   [if]

## deliv:Rename(·) promo:MergeAttr   — 10 functions in 8 programs
   kr-suite/rules/binding/B-Copy/a-binding-from-a-value-branch-copies-the-arm-it-takes.loft::n_folded   [if]
   kr-suite/rules/binding/B-Copy/a-binding-from-a-value-branch-copies-the-arm-it-takes.loft::n_rebound_after   [block EnumUnitLit]
   kr-suite/rules/calls/F-Ret/a-part-of-a-calls-result-returned-is-the-callers-own-copy.loft::n_elem   [block ncr]
   kr-suite/rules/heap/H-Free/a-call-frees-the-record-it-made-for-an-argument-however-it-was-built.loft::n_first   [if]
   kr-suite/rules/matching/P-Rule/a-sub-rule-run-releases-its-answers-and-never-the-subjects.loft::n_mixed   [block cursor match]
   kr-suite/rules/types/N-Store/2006-a-returned-field-of-a-record-that-is-null-is-null.loft::n_b1   [call OpGetField]
   loft-keyed-ranges/tests/scripts/1990-a-nullable-result-that-joins-a-dense-leaf-returns-in-registers.loft::n_j10   [block scalar_match]
   loft-keyed-ranges/tests/scripts/1994-a-null-stored-into-a-dense-record-slot-reads-back-null.loft::n_w5   [block scalar_match]
   loft-keyed-ranges/tests/scripts/2016-a-chain-site-whose-callee-returns-its-parameter-copies-what-it-hands-up.loft::n_c2   [call n_id2]
   loft-keyed-ranges/tests/scripts/2016-a-chain-site-whose-callee-returns-its-parameter-copies-what-it-hands-up.loft::n_w1   [call n_id2]

## deliv:MaterializeView promo:Bind   — 9 functions in 5 programs
   kr-suite/rules/binding/B-Copy/a-binding-from-a-value-branch-copies-the-arm-it-takes.loft::n_ch_in_a_loop   [block EnumUnitLit]
   kr-suite/rules/binding/B-Copy/a-binding-from-a-value-branch-copies-the-arm-it-takes.loft::n_pick   [if]
   kr-suite/rules/heap/H-Free/a-tail-match-answer-materialised-into-a-work-ref-is-freed-once.loft::n_held_bang   [if]
   kr-suite/rules/heap/H-Free/a-tail-match-answer-materialised-into-a-work-ref-is-freed-once.loft::n_held_plain   [if]
   kr-suite/rules/heap/H-Free/a-tail-match-answer-materialised-into-a-work-ref-is-freed-once.loft::n_held_two   [if]
   kr-suite/rules/ownership/O-Complete/2488-a-branch-that-can-leave-copies-each-arm-into-the-return-buffer.loft::n_c6   [if]
   kr-suite/rules/ownership/O-Deps/1414-a-nullable-element-binding-borrows-its-subject.loft::n_first_d   [block vector_match]
   loft-keyed-ranges/tests/scripts/2561-a-record-copied-out-of-a-view-is-absent-where-its-source-is.loft::n_call_hash   [call OpGetRecord]
   loft-keyed-ranges/tests/scripts/2561-a-record-copied-out-of-a-view-is-absent-where-its-source-is.loft::n_enum_call   [call OpGetField]

## deliv:Rename(·) promo:Bind   — 8 functions in 5 programs
   kr-suite/rules/formatting/F-Target-Pos/every-position-with-an-expected-type-builds-the-target.loft::n_branch   [if]
   kr-suite/rules/ownership/O-Complete/2384-a-record-copied-into-its-buffer-releases-the-buffer-when-the-arm-returns-early.loft::n_by_block   [block block]
   kr-suite/rules/ownership/O-Complete/2384-a-record-copied-into-its-buffer-releases-the-buffer-when-the-arm-returns-early.loft::n_dense   [block scalar_match]
   kr-suite/rules/ownership/O-Complete/2384-a-record-copied-into-its-buffer-releases-the-buffer-when-the-arm-returns-early.loft::n_null_early   [block scalar_match]
   kr-suite/rules/ownership/O-Complete/2488-a-branch-that-can-leave-copies-each-arm-into-the-return-buffer.loft::n_c4   [if]
   kr-suite/rules/rewrites/R-InPlaceLiteral/1571-a-literal-written-in-place-reads-the-old-value-through-any-view.loft::n_doubled   [an owned local]
   loft-keyed-ranges/tests/scripts/2561-a-record-copied-out-of-a-view-is-absent-where-its-source-is.loft::n_call_index   [call OpGetRecord]
   loft-keyed-ranges/tests/scripts/2561-a-record-copied-out-of-a-view-is-absent-where-its-source-is.loft::n_call_sorted   [call OpGetRecord]

## (no pass-2 route line)   — 7 functions in 4 programs
   kr-suite/rules/matching/M-Exhaust/2390-a-match-whose-every-arm-returns-answers-on-every-backend.loft::n_as_record   [if]
   kr-suite/rules/tuples/T-Cons/1675-a-keyed-tuple-member-filled-from-a-vector-holds-its-records.loft::n_ret_dupes   [other]
   kr-suite/rules/tuples/T-Cons/1675-a-keyed-tuple-member-filled-from-a-vector-holds-its-records.loft::n_ret_hash   [other]
   kr-suite/rules/tuples/T-Cons/1675-a-keyed-tuple-member-filled-from-a-vector-holds-its-records.loft::n_ret_index   [other]
   kr-suite/rules/tuples/T-Cons/1675-a-keyed-tuple-member-filled-from-a-vector-holds-its-records.loft::n_ret_sorted   [other]
   kr-suite/rules/types/N-Shape/a-tuple-member-releases-once-whatever-it-is-declared-and-beside.loft::n_c_from_call_mk   [other]
   loft-keyed-ranges/tests/scripts/1511-a-call-result-in-a-tuple-member-releases-once.loft::n_g   [other]

## deliv:MaterializeView promo:MergeAttr   — 6 functions in 3 programs
   kr-suite/rules/ownership/O-Deps/1414-a-nullable-element-binding-borrows-its-subject.loft::n_head_n   [block vector_match]
   loft-keyed-ranges/tests/scripts/2561-a-record-copied-out-of-a-view-is-absent-where-its-source-is.loft::n_enum_local   [call OpGetField]
   loft-keyed-ranges/tests/scripts/2561-a-record-copied-out-of-a-view-is-absent-where-its-source-is.loft::n_local_hash   [call OpGetRecord]
   loft-keyed-ranges/tests/scripts/2561-a-record-copied-out-of-a-view-is-absent-where-its-source-is.loft::n_local_index   [call OpGetRecord]
   loft-keyed-ranges/tests/scripts/2561-a-record-copied-out-of-a-view-is-absent-where-its-source-is.loft::n_local_sorted   [call OpGetRecord]
   loft-keyed-ranges/tests/scripts/882-keyed-element-read-borrows-its-container.loft::n_i882_nullable   [call OpGetRecord]

## promo:Bind promo:MergeAttr   — 5 functions in 4 programs
   kr-suite/rules/formatting/F-Target-Pos/every-position-with-an-expected-type-builds-the-target.loft::n_early   [block Formatted string]
   kr-suite/rules/formatting/F-Target-Pos/every-position-with-an-expected-type-builds-the-target.loft::n_early   [block Formatted string]
   kr-suite/rules/heap/H-CopySelf/a-value-delivered-onto-or-from-inside-its-own-place-keeps-its-value.loft::n_pick   [if]
   kr-suite/rules/ownership/O-Buffer/a-return-buffer-is-the-callers-through-every-hand-on.loft::n_rw   [if]
   kr-suite/rules/rewrites/R-HostStore/a-result-built-in-its-hosts-store-is-adopted-and-released-once.loft::n_simplify   [block Object]

## deliv:BuildIntoBuffer(__ref_p2_1,__retbuf)   — 4 functions in 4 programs
   kr-suite/rules/calls/F-Ret/a-part-of-a-calls-result-returned-is-the-callers-own-copy.loft::n_nullable   [if]
   kr-suite/rules/ownership/O-Complete/2488-a-branch-that-can-leave-copies-each-arm-into-the-return-buffer.loft::n_c1   [if]
   kr-suite/rules/ownership/O-Deps/2376-an-element-read-off-a-borrowing-calls-field-names-no-callee-dep.loft::n_pick   [if]
   kr-suite/rules/ownership/O-Witness/2372-a-witnessed-local-bound-from-an-unwrapped-block-local-optional-copies-it.loft::n_body   [an owned local]

## deliv:AsIs   — 3 functions in 3 programs
   kr-suite/rules/binding/B-Copy/a-binding-from-a-value-branch-copies-the-arm-it-takes.loft::n_fold   [a borrowing local]
   kr-suite/rules/matching/P-Rule/a-sub-rule-reads-between-fixed-elements.loft::n_paren   [block cursor match]
   kr-suite/rules/matching/P-Rule/a-sub-rule-run-releases-its-answers-and-never-the-subjects.loft::n_view   [block cursor match]

## deliv:Rename(·) promo:Bind promo:MergeAttr   — 3 functions in 3 programs
   kr-suite/rules/calls/F-Ret/a-stored-reference-returned-hands-back-a-fresh-copy-however-it-is-spelled.loft::n_rf_ret   [call OpGetDbRef]
   kr-suite/rules/heap/H-Free/a-tail-match-answer-materialised-into-a-work-ref-is-freed-once.loft::n_held   [if]
   kr-suite/rules/ownership/O-Complete/2384-a-record-copied-into-its-buffer-releases-the-buffer-when-the-arm-returns-early.loft::n_param_early   [block scalar_match]

## deliv:BuildIntoBuffer(__ref_p2_1,__retbuf) promo:MergeAttr   — 2 functions in 1 programs
   kr-suite/rules/calls/F-Return/a-function-answers-the-value-its-return-or-its-tail-names.loft::n_rec_fresh   [call n_same]
   kr-suite/rules/calls/F-Return/a-function-answers-the-value-its-return-or-its-tail-names.loft::n_rec_loop   [call n_same]

## deliv:Rename(a#1) promo:Bind   — 1 functions in 1 programs
   kr-suite/rules/calls/F-ParamHeap/a-heap-argument-shares-the-callers-store-and-nothing-in-the-frame-releases-it.loft::n_head_from_nullable_parameter   [block vector_match]

## deliv:Rename(b,t,__retbuf) promo:Bind promo:MergeAttr   — 1 functions in 1 programs
   kr-suite/rules/heap/H-Free/each-pass-frees-the-answer-it-copied.loft::n_sv   [if]

## promo:Grow   — 1 functions in 1 programs
   kr-suite/rules/layout/L-Null/a-nested-block-that-binds-the-buffer-from-a-call-delivers-into-it.loft::n_c_record   [block block]

## deliv:BuildIntoBuffer(__ref_p2_2,__retbuf)   — 1 functions in 1 programs
   kr-suite/rules/ownership/O-Derived/a-call-argument-a-recursive-join-answers-is-not-freed-before-the-return.loft::n_classify   [if]

## promo:Rename   — 1 functions in 1 programs
   kr-suite/rules/rewrites/R-FnRefValue/a-fn-ref-dispatch-answers-the-tuple-its-arms-answer.loft::n_first_of   [other]

## deliv:Rename(·) promo:SkipReassigned   — 1 functions in 1 programs
   kr-suite/rules/rewrites/R-InPlaceLiteral/1571-a-literal-written-in-place-reads-the-old-value-through-any-view.loft::n_fresh   [an owned local]

## deliv:AsIs promo:MergeAttr   — 1 functions in 1 programs
   loft-keyed-ranges/tests/scripts/2016-a-chain-site-whose-callee-returns-its-parameter-copies-what-it-hands-up.loft::n_w5   [call n_id2]
```
