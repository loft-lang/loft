# @PLN91 — a record element written from a call (prepared subject)

**The family:** "an assignment to a place that is not a plain local", 38 sites in 23 programs at
try a623de4a5. The sub-shape here: a vector-of-records ELEMENT written whole from a call,
`ps[1] = mk(9)`, `ps[-1] = mk(7)` (B-Copy's deferred-copy cells, among others). Its other
spellings (`u.0.1 = "c0"`, a tuple member) belong to the tuple subjects.

**Judged right** on the interpreter (LOFT_POISON), --native-c (leak check) and --native, with 300
passes. Expected values computed by hand under the rules: `ps[1] = mk(14)` → `14:r14:[14,28]`;
`ps[-1] = mk(24)` → the last element 24; `keep = ps[0]; ps[0] = mk(30); keep.id = 99` → BOTH read
99. **That last one is by the rule, not a defect:** binding.md `(B-Copy)` says *"the SHAPE
decides between this rule and (B-View): a STRUCT-typed projection views, every other plain bind
copies"*. `keep = ps[0]` is a struct projection, so `keep` is a VIEW of the slot. `prog = callee`
of a whole local is a copy, and the B-Copy cells use that.

## Rust's form

```
ps[1] = mk(10 + i)
    OpCopyRecord(n_mk(OpAddInt(10, i), __ref_4), OpGetVector(ps, 16, 1), 32868);    <- 32868 = R(100) | source-free 0x8000
ps[-1] = mk(20 + i)
    OpCopyRecord(n_mk(OpAddInt(20, i), __ref_5), OpGetVector(ps, 16, OpMinSingleInt(1)), 32868);
keep = ps[0]
    keep: ref(R)["ps"] = OpGetVector(ps, 16, 0);                                       <- a VIEW (B-View), no copy
```

The call builds into a buffer of its own (`__ref_N`, one per call site, numbered in order), and
`OpCopyRecord` copies it into the element. The source-free bit releases the buffer's store after
the copy; the element's old heap members are released by the copy, as `(H-ClearRelease)` asks.
No `_elm_N` is minted, and the element is addressed by `OpGetVector` at the copy itself.

**What to code:** an element write `v[i] = f(…)` of a record type is one `OpCopyRecord(<the call
into its own work buffer>, OpGetVector(v, <elem size>, <index>), <record type> | 0x8000)`. A
negative literal index stays `OpMinSingleInt(n)`; the runtime counts it from the end. A bind
`x = v[i]` of a struct element is a view typed `ref(R)["v"]`, not a copy.

## The sites (try a623de4a5)

```
kr-suite/rules/binding/B-Copy/a-copy-of-a-compound-value-copies-every-member.loft::n_test_a_nested_tuple_with_a_text_leaf
kr-suite/rules/binding/B-Copy/a-deferred-copy-keeps-the-value-it-was-bound-to-whatever-the-source-does.loft::n_carried
kr-suite/rules/binding/B-Copy/a-deferred-copy-keeps-the-value-it-was-bound-to-whatever-the-source-does.loft::n_from_end
kr-suite/rules/binding/B-Copy/a-deferred-copy-keeps-the-value-it-was-bound-to-whatever-the-source-does.loft::n_live
kr-suite/rules/binding/B-Copy/a-deferred-copy-keeps-the-value-it-was-bound-to-whatever-the-source-does.loft::n_other
kr-suite/rules/binding/B-Copy/a-deferred-copy-keeps-the-value-it-was-bound-to-whatever-the-source-does.loft::n_outlives
kr-suite/rules/binding/B-Copy/a-deferred-copy-keeps-the-value-it-was-bound-to-whatever-the-source-does.loft::n_overwrite
kr-suite/rules/binding/B-Copy/a-deferred-copy-keeps-the-value-it-was-bound-to-whatever-the-source-does.loft::n_plain_arg
kr-suite/rules/binding/B-Copy/a-deferred-copy-keeps-the-value-it-was-bound-to-whatever-the-source-does.loft::n_walk
kr-suite/rules/binding/B-Ref-Alias/a-link-composes-with-tuples-repoints-and-ref-params.loft::n_test_a_link_to_a_tuple_member
kr-suite/rules/binding/B-Ref-Lvalue/a-stored-tuple-is-a-place-a-link-names.loft::n_test_an_element_of_a_field_vector_links
kr-suite/rules/binding/B-Ref-Lvalue/a-stored-tuple-is-a-place-a-link-names.loft::n_test_a_struct_field_links_three_ways
kr-suite/rules/binding/B-Ref-Lvalue/a-stored-tuple-is-a-place-a-link-names.loft::n_test_a_vector_element_links_three_ways
kr-suite/rules/binding/Const-Value/a-link-a-view-and-a-discharge-of-a-const-value-read-as-the-value.loft::n_test_a_discharge_target_writes_the_place
kr-suite/rules/calls/F-ParamScalar/a-compound-write-to-a-tuple-parameters-member-is-the-callees-copy.loft::n_plain
kr-suite/rules/calls/F-ParamTuple/a-by-value-tuple-parameter-is-the-callees-own-value.loft::n_c_rebind
kr-suite/rules/calls/F-ParamTuple/a-by-value-tuple-parameter-is-the-callees-own-value.loft::n_c_scalar
kr-suite/rules/collections/Col-Group/a-write-through-any-member-of-a-linked-group-reaches-every-member.loft::n_test_a_group_element_written_through_the_vector_member_reaches_every_member
kr-suite/rules/collections/Col-Vec/a-vector-literal-compiles-whatever-it-reaches-is-declared-below.loft::n_retype
kr-suite/rules/coroutines/G-Own/a-link-in-a-generator-is-a-link-and-yielding-it-yields-its-value.loft::n_tuples
kr-suite/rules/heap/H-CopySelf/a-value-delivered-onto-or-from-inside-its-own-place-keeps-its-value.loft::n_peel_param
kr-suite/rules/heap/H-CopySelf/a-value-delivered-onto-or-from-inside-its-own-place-keeps-its-value.loft::n_test_a_record_written_from_inside_its_own_place_keeps_its_value
kr-suite/rules/interfaces/G-Mono/a-generic-over-a-vector-of-tuples-of-its-type-variables.loft::n_replace
kr-suite/rules/interfaces/G-Mono/a-generic-over-a-vector-of-tuples-of-its-type-variables.loft::n_setfirst
kr-suite/rules/iteration/I-Comp/a-comprehension-yields-a-tuple-element-whole.loft::n_test_a_nested_or_heap_member_is_copied_deep
kr-suite/rules/layout/L-Narrow/a-stored-tuple-member-is-written-in-its-own-width.loft::n_put_first
kr-suite/rules/layout/L-Narrow/a-stored-tuple-member-is-written-in-its-own-width.loft::n_test_an_element_assigned_in_the_middle
kr-suite/rules/layout/L-Narrow/a-stored-tuple-member-is-written-in-its-own-width.loft::n_test_elements_assigned_in_a_loop
kr-suite/rules/layout/L-Null/bind-copies-or-views-the-whole-boundary.loft::n_test_collection_tuple_element_off_a_borrowed_base_is_a_view
kr-suite/rules/layout/L-Null/bind-copies-or-views-the-whole-boundary.loft::n_test_struct_tuple_element_off_a_borrowed_base_is_a_view
kr-suite/rules/layout/L-Tuple/a-stored-tuple-is-a-record-packed-like-a-struct-keeping-its-written-positions.loft::n_test_a_four_byte_aligned_element_keeps_every_member
kr-suite/rules/layout/L-Tuple/a-stored-tuple-is-a-record-packed-like-a-struct-keeping-its-written-positions.loft::n_test_a_nested_tuple_element_keeps_every_member
kr-suite/rules/layout/L-Tuple/a-stored-tuple-is-a-record-packed-like-a-struct-keeping-its-written-positions.loft::n_test_a_two_byte_aligned_element_keeps_every_member
kr-suite/rules/layout/L-Tuple/a-stored-tuple-is-a-record-packed-like-a-struct-keeping-its-written-positions.loft::n_test_positions_survive_every_boundary
kr-suite/rules/operational/E-Left/1987-an-earlier-operand-is-evaluated-before-a-lifted-call.loft::t_7OrdWrap_reset
kr-suite/rules/operational/E-Seq/a-block-at-statement-start-before-an-operator.loft::n_test_a_void_block_still_ends_its_statement
kr-suite/rules/ownership/O-Detach/a-rebind-or-a-build-reads-its-destination-through-any-view-first.loft::n___lambda_test_a_captured_collection_0
kr-suite/rules/ownership/O-Detach/a-rebind-or-a-build-reads-its-destination-through-any-view-first.loft::n___lambda_test_a_captured_collection_in_a_loop_0
kr-suite/rules/ownership/O-Detach/a-rebind-or-a-build-reads-its-destination-through-any-view-first.loft::n___lambda_test_a_comprehension_through_a_capture_0
kr-suite/rules/rewrites/R-InPlaceLiteral/1571-a-literal-written-in-place-reads-the-old-value-through-any-view.loft::n_chosen
kr-suite/rules/rewrites/R-InPlaceLiteral/1571-a-literal-written-in-place-reads-the-old-value-through-any-view.loft::n_doubled
kr-suite/rules/rewrites/R-InPlaceLiteral/1571-a-literal-written-in-place-reads-the-old-value-through-any-view.loft::n_from_other
kr-suite/rules/rewrites/R-InPlaceLiteral/1571-a-literal-written-in-place-reads-the-old-value-through-any-view.loft::n_renamed
kr-suite/rules/rewrites/R-InPlaceLiteral/1571-a-literal-written-in-place-reads-the-old-value-through-any-view.loft::n_rewrap
kr-suite/rules/tuples/T-Cons/1689-a-tuples-heap-member-is-its-own-copy.loft::n_main
kr-suite/rules/tuples/T-Cons/1698-a-tuple-held-as-a-vector-element-is-a-place.loft::n_main
kr-suite/rules/tuples/T-Proj/1698b-a-stored-tuples-text-and-scalar-members-are-written-in-place.loft::n_main
kr-suite/rules/tuples/T-Ref/a-destructure-unpacks-a-reference-tuple.loft::n_test_a_reference_tuple_local
kr-suite/rules/types/C-Var/a-tuple-member-is-typed-by-its-declaration-a-variant-one-included.loft::n_test_a_local_link_writes_either_member
kr-suite/rules/types/N-Store/1720-a-nullable-written-to-an-enum-text-or-record-slot-is-reported.loft::n_test_field_enum
kr-suite/rules/types/N-Store/1720-a-nullable-written-to-an-enum-text-or-record-slot-is-reported.loft::n_test_field_variant
kr-suite/rules/types/N-Store/a-null-variant-handle-written-into-a-nullable-slot-clears-it.loft::n_c4
kr-try/tests/docs/28-tuples.loft::n_doubled_first
kr-try/tests/scripts/1326-a-captured-keyed-collection-rebind-replaces.loft::n___lambda_test_a_comprehension_over_its_own_destination_reads_its_source_first_0
kr-try/tests/scripts/1326-a-captured-keyed-collection-rebind-replaces.loft::n___lambda_test_the_captured_vector_spelling_is_unmoved_0
kr-try/tests/scripts/1331-a-repointed-literal-accumulator-is-detached.loft::n___lambda_test_a_captured_vector_is_replaced_0
kr-try/tests/scripts/1331-a-repointed-literal-accumulator-is-detached.loft::n___lambda_test_a_comprehension_over_its_own_source_is_unmoved_0
```
