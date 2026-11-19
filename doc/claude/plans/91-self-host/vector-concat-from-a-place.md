# @PLN91 — a vector concatenation whose operands are places (prepared subject)

**The family:** "a vector concatenation of this shape: a first operand that is no other local,
or elements that are no plain scalars" (44 sites in 10 programs at try 0169c36eb). The main
sub-shape is a first operand that is a PLACE or a call, not a local: `r = h[0] + [1]`,
`r = w.v + [1]`, `r = g.tp[0] + [1]`, `r = first(h) + [1]`, `r = h[0] + h[1]`, and a
discharged element `(h[i] ?? []) + [i]`. Element types other than plain scalars (text, records)
take the same form at their own type numbers.

**Judged right** on the interpreter (LOFT_POISON), --native-c (leak check) and --native, 300
passes. Values hand-computed (k = 3, `h = [[1, 2], [3]]`, `w.v = [7]`): `h[0] + [k]` then
`+= [100]` → `[1,2,3,100]`; `w.v + [k, 9]` → `[7,3,9]`; `first(h) + [5]` → `[1,2,5]`;
`h[0] + h[1]` → `[1,2,3]`; `(h[k % 2] ?? []) + [k]` → `[3,3]`. Afterwards `h` and `w.v` are
unchanged: the concatenation copies, never views. Note: `h[i] + [i]` with a COMPUTED index is a
nullable element, and Rust refuses it ("cannot concatenate a nullable `vector<integer>?`"). A
corpus site with it holds an `h` whose element type makes the read dense, or is refused by both.

## Rust's form

```
r = A + B + …        (bound to a new local; one pattern whatever the operands are)
    OpDatabase(__vdb_N, 20);                               <- a fresh store for r (vector<integer> record 20)
    r: vector<integer>["__vdb_N"] = OpGetField(__vdb_N, 0, 7);
    OpSetInt4(__vdb_N, 0, 0);
    OpAppendVector(r, <A as it stands>, 0);                <- one append per operand, source order
    OpAppendVector(r, <B as it stands>, 0);

the operand forms:
    h[0]          OpGetField(OpGetVector(h, 4, 0), 0, 7)   <- the element's vector field, no copy first
    w.v           OpGetField(w, 0, 7)
    first(h)      n_first(h, __ref_1)                      <- the call with its work buffer
    [k, 9]        {#Vector:vector<integer>["__vdb_M"]      <- a literal is its own store block
                    OpDatabase(__vdb_M, 20); _vec_K = OpGetField(__vdb_M, 0, 7); OpSetInt4(__vdb_M, 0, 0);
                    OpPreAllocVector(_vec_K, 2, 8); OpPushInt(_vec_K, k); OpPushInt(_vec_K, 9); _vec_K }
    (h[i] ?? [])  {#ncc:vector<integer>["h"]  __ncc_1 = OpGetField(OpGetVectorNullable(h, 4, i), 0, 7);
                    if OpNot(OpVectorIsNull(__ncc_1)) __ncc_1 else {#Vector … an empty store … } }
```

Numbering: an operand literal's `__vdb` is minted BEFORE the result's, so `r1 = h[0] + [k]` has
the literal's `__vdb_2` and the result's `__vdb_3`. The `_vec_K` counter runs in source order.

**What to code:** a bind whose right side is `+` over vectors lowers to the three-statement
fresh-store prologue for the result, then one `OpAppendVector(result, operand, 0)` per operand
left to right. Each operand is emitted as the expression it is, a place, a call or a literal
block, with no temp and no copy of its own. The final `0` is the append's element type argument
for `vector<integer>`; for other elements it is the element's store type, as in the plain-local
form you already emit.

## The sites (try 0169c36eb)

```
kr-suite/rules/closures/L-CapKeep/each-run-of-a-build-in-a-loop-is-its-own-record-and-a-kept-closure-keeps-its-pass.loft::n_one_loop:400
kr-suite/rules/closures/L-CapKeep/each-run-of-a-build-in-a-loop-is-its-own-record-and-a-kept-closure-keeps-its-pass.loft::n_two_loops:392
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_escape:204
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_test_a_closure_kept_past_the_loop:294
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_test_a_closure_rebuilt_each_pass_reads_its_own_build:280
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_test_a_concat_rebind_leaves_the_record_its_store:210
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_test_a_rebind_in_a_loop:233
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_test_text_elements_and_a_call_before:250
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_test_without_a_record_the_concat_is_in_place:261
kr-suite/rules/collections/Col-Concat/2406-a-literal-after-a-site-only-the-second-pass-builds-keeps-its-elements.loft::n_c_arg:41
kr-suite/rules/collections/Col-Concat/2406-a-literal-after-a-site-only-the-second-pass-builds-keeps-its-elements.loft::n_c_text:34
kr-suite/rules/collections/Col-Concat/2406-a-literal-after-a-site-only-the-second-pass-builds-keeps-its-elements.loft::n_c_twice:62
kr-suite/rules/collections/Col-Concat/2406-a-literal-after-a-site-only-the-second-pass-builds-keeps-its-elements.loft::n_c_two:47
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_append_record:161
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_first_only:492
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n___lambda_test_inside_other_expressions_0:231
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_last_only:498
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_of_param:340
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_records:531
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_sorted_groups:485
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_test_a_borrowing_call_an_if_and_a_block:377
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_test_a_field:198
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_test_a_literal_of_an_element:289
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_test_a_loop_turn:186
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_test_an_element_of_a_field:352
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_test_an_element_of_a_literal:421
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_test_an_element_of_a_local:346
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_test_a_postfix:248
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_test_a_reassignment_and_an_append:409
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_test_a_record_field:368
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_test_a_return_and_a_head:220
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_test_enum_and_narrow_elements:449
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_test_in_a_loop_and_an_if_arm:436
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_test_texts_and_records:388
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_test_the_controls:457
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_test_the_right_operand:397
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_test_two_levels_down:362
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_texts:529
kr-suite/rules/collections/Slice-Value/a-slice-is-a-fresh-vector-at-every-vector-position.loft::n_test_in_other_positions:182
kr-suite/rules/heap/H-Free/each-pass-frees-the-answer-it-copied.loft::n_sv:105
kr-suite/rules/iteration/I-Comp/a-build-that-reads-its-destination-reads-what-it-held.loft::n_sorted_insert:145
kr-suite/rules/ownership/O-Detach/2402-a-chain-into-a-field-reads-the-field-before-its-build.loft::n_main:40
kr-suite/rules/tuples/T-Cons/1590-a-tuple-member-string-may-open-a-hole.loft::n_main:71
kr-suite/rules/tuples/T-Cons/1590-a-tuple-whose-first-member-holds-a-string-binds.loft::n_main:47
```
