# @PLN91 — a record literal whose vector field holds items that are no literals (prepared subject)

**The family:** "a vector field of records written from a literal: an item that is no literal",
25 sites in 20 programs at try a623de4a5. A record construction whose vector field's literal
holds a call's answer (`Box { items: [mk(3), mk(4)] }`), a local (`JShapes { items:
[JCircle { r: 1 }, dot] }`), or nested vectors and tuples (`G { tp: [[42], [7, 8]] }`,
`rows: [(1, 2), (3, 4)]`).  This page is the RECORD items: a call or a local.

**Judged right** on the interpreter (LOFT_POISON), --native-c (leak check) and --native, with 300
passes. Expected values computed by hand: `Box { items: [mk(3), mk(4)], n: 2 }` → `2:4:p3`;
`[P { id: 1, t: "lit" }, d, mk(4)]` then `d.id = 99` → the item is a COPY (`9:4:99`);
`G { tp: [[42], [7, 8]] }` → `tp[1][1] + len(tp)` = 10.

## Rust's form: the two pages you already carry, combined

```
b = Box { items: [mk(i), mk(i + 1)], n: 2 }
    b: ref(Box) = null;  OpDatabase(b, 102);  OpSetInt4(b, 8, 0);          <- the record, its vector field empty
    _elm_1: ref(P)["b"] = OpNewRecord(b, 102, 0);                          <- minted on the PARENT: (b, Box's type, field 0)  — vector-field-from-a-literal.md
    __ref_p2_1: ref(P) = n_mk(i, _elm_1);                                  <- a call's item: the element IS the callee's buffer — keyed-append-of-a-record.md
    if OpDistinctStore(__ref_p2_1, _elm_1) OpCopyRecord(__ref_p2_1, _elm_1, 49253) else null;   <- 49253 = P(101) | COPY_FRESH_DEST | source-free
    OpFinishRecord(b, _elm_1, 102, 0);
    … the second item, `__ref_p2_2` …
    OpSetInt(b, 0, 2);                                                      <- then the scalar field
[P { id: 1, t: "lit" }, d, mk(4)]
    a literal item: OpNewRecord(b, 102, 0); OpSetInt(_elm_1, 0, 1); OpSetText(_elm_1, 8, "lit"); OpFinishRecord(…)
    a LOCAL item:   OpNewRecord(b, 102, 0); OpCopyRecord(d, _elm_1, 16485); OpFinishRecord(…)   <- 16485 = P | COPY_FRESH_DEST, no source free
```

**What to code:** in a record construction's vector-field literal, mint each item on the parent
(`OpNewRecord(parent, <parent type>, <field number>)` / `OpFinishRecord` the same), and fill it as
the item's kind does in a vector append. A literal is written field by field. A call takes the
element as its buffer, with the `OpDistinctStore`-guarded copy and the source-free bit. A local is
copied in (`COPY_FRESH_DEST`, no source free). The `_elm_1` name is reused for every item. The
nested-vector and tuple items (`[[42]]`, `[(1, 2)]`) are this family's other shapes, not on this
page.

## The sites (try a623de4a5)

```
kr-suite/rules/binding/B-Ref-Lvalue/a-stored-tuple-is-a-place-a-link-names.loft::n_test_an_element_of_a_field_vector_links
kr-suite/rules/binding/B-Ref-Lvalue/a-stored-tuple-is-a-place-a-link-names.loft::n_test_a_struct_field_links_three_ways
kr-suite/rules/calls/F-Ret/a-nullable-return-hands-out-a-fresh-owned-value-as-its-dense-twin-does.loft::n_mk
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_test_an_element_of_a_field
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_test_an_element_of_a_literal
kr-suite/rules/formatting/F-Render-Fields/57-json.loft::n_test_json_enum_in_vector
kr-suite/rules/heap/H-CopySelf/a-value-delivered-onto-or-from-inside-its-own-place-keeps-its-value.loft::n_test_two_and_three_steps_deep
kr-suite/rules/heap/H-Drop/1597-a-vector-of-vectors-releases-its-inner-elements.loft::n_c2
kr-suite/rules/heap/H-Free/a-call-frees-the-record-it-made-for-an-argument-however-it-was-built.loft::n_pw
kr-suite/rules/heap/H-Free/a-call-frees-the-record-it-made-for-an-argument-however-it-was-built.loft::n_test_literal_field_and_block_arguments
kr-suite/rules/heap/H-Free/a-literal-field-read-before-its-element-is-rewritten-is-a-view.loft::n_test_each_pass_rewrites_the_element
kr-suite/rules/heap/H-Free/an-early-exit-frees-what-the-frame-owns-whoever-would-fill-it-later.loft::n_test_an_arm_ending_in_a_literal
kr-suite/rules/heap/H-LongRef/1974-a-nested-vector-of-a-linked-type-appends-to-its-element.loft::n_field
kr-suite/rules/heap/H-Removed/a-cleared-vector-releases-the-records-a-reference-named.loft::n_deeper
kr-suite/rules/operational/E-Asgn-Compound/2426-a-tuple-member-of-a-call-indexed-element-is-one-place.loft::n_test_a_call_indexed_tuple_member
kr-suite/rules/ownership/O-Detach/2397-a-literal-reads-its-field-under-another-name.loft::n_test_a_nested_vector_element_through_a_view
kr-suite/rules/rewrites/R-CopyView/2265-a-loop-variable-copy-made-a-view-is-bound-in-the-loop.loft::n_mk
kr-suite/rules/tuples/T-Cons/1698-a-tuple-held-as-a-vector-element-is-a-place.loft::n_main
kr-suite/rules/tuples/T-Cons/2253-a-compound-member-copies-the-local-it-answers.loft::n_test_a_nested_vector_and_a_field_default
kr-suite/rules/types/N-Opt/has-null-is-owed-by-every-rule-that-constructs-an-optional.loft::n_main
kr-suite/rules/types/T-Chk-Vec/a-comprehension-body-is-checked-against-the-declared-element.loft::n_test_a_field_reading_itself_with_an_empty_default
kr-suite/rules/types/T-Chk-Vec/a-comprehension-body-is-checked-against-the-declared-element.loft::n_test_view_a_literal_default_then_another_field
kr-suite/rules/types/T-Chk-Vec/a-comprehension-body-is-checked-against-the-declared-element.loft::n_test_view_an_empty_default_then_another_field
kr-try/tests/scripts/24-field-overlap-enum-struct.loft::n_main
kr-try/tests/scripts/25-sorted-enum-variant-range.loft::n_main
```
