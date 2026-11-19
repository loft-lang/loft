# @PLN91 — a vector field bound whole from a literal (prepared subject)

**The family:** "a vector field bound whole from this value", 36 sites in 22 programs at try
ca4065330. Its right-hand sides: a vector LITERAL in 21 sites (`w.a = [S { n: 901 }]`,
`h.data = [7, 8, 9]`, `s.v = [1, 2]`), a slice in 7 (`vars.present[0..dp]`), a concatenation
in 5, plus a `??`. This page is the literal, the 21.

**Judged right** on the interpreter (LOFT_POISON), --native-c (leak check) and --native, with
1000 passes and no leak (each pass replaces a field that held records, integers and texts).
Expected values computed by hand: `w.a = [S { n: 901 + i, t: "new" }]` → `1:904:new` for i = 3;
`w.d = [7, 8, 9]` then `w.d = []` → 0; `w.ts = ["p", "q{i}"]` → `["p","q3"]`; the scalar field
untouched → 5.

## Rust's form (route `parse_vector` per statement)

```
w.a = [S { n: 901 + i, t: "new" }]          a RECORD element
    OpClearVector(OpGetField(w, 8, 102));                     <- the field emptied first (its records released)
    _elm_4: ref(S)["w"] = OpNewRecord(w, 101, 0);             <- the new element on the PARENT record: (w, its type, the FIELD NUMBER)
    OpSetInt(_elm_4, 0, OpAddInt(901, i));  OpSetText(_elm_4, 8, "new");
    OpFinishRecord(w, _elm_4, 101, 0);
w.d = [7, 8, 9]                              an integer element
    OpClearVector(OpGetField(w, 12, 7));
    OpPushInt(OpGetField(w, 12, 7), 7);  OpPushInt(…, 8);  OpPushInt(…, 9);    <- no OpPreAllocVector here
w.ts = ["p", "q{i}"]                         a text element
    OpClearVector(OpGetField(w, 16, 44));
    _elm_6: ref(boolean)["w"] = OpNewRecord(w, 101, 2);       <- Reference(0) again; same (parent, type, field) triple
    OpSetText(_elm_6, 0, "p");  OpFinishRecord(w, _elm_6, 101, 2);
    _elm_6 = OpNewRecord(w, 101, 2);  OpSetText(_elm_6, 0, {#Formatted string __work_1 = "q"; OpFormatInt(__work_1, i, …); __work_1});  OpFinishRecord(…);
w.d = []                                     the empty literal
    OpClearVector(OpGetField(w, 12, 7));                       <- alone
```

**What to code:** a whole vector literal assigned to a FIELD clears the field in place
(`OpClearVector` of the field read) and appends each item INTO it. The difference from a local's
literal: the element is minted on the PARENT record, `OpNewRecord(parent, <parent record type>,
<field number>)`, with `OpFinishRecord` taking the same triple. No `__vdb` is minted and no copy
is made. A scalar element pushes through the field read (`OpPushInt(OpGetField(…), x)`). The
element temp follows `unique_elm_var`: the record's type for a record element, `Reference(0)` for
a text. The slice and concatenation right-hand sides (14 sites) are separate shapes; see their
sites before porting.

## The sites (try ca4065330)

```
kr-suite/rules/binding/B-Disturb/a-disturbance-ends-the-places-inside-its-container-and-only-those.loft::n_test_a_rebuilt_field
kr-suite/rules/binding/B-Disturb/a-disturbance-ends-the-places-inside-its-container-and-only-those.loft::n_test_the_statement_and_the_spelling_of_the_view
kr-suite/rules/binding/B-Disturb/a-disturbance-ends-the-places-inside-its-container-and-only-those.loft::n_view_of_a_rebuilt_field
kr-suite/rules/binding/B-Ref-Write/a-vector-link-follows-a-rebind-of-its-source.loft::n_test_a_link_to_a_field_follows_its_rebind
kr-suite/rules/calls/F-Ret/each-result-is-a-fresh-record-written-to-its-defaults.loft::n_test_a_borrowing_result_is_copied_at_its_bind
kr-suite/rules/collections/Col-Concat/a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-stands.loft::n_test_a_field
kr-suite/rules/collections/Col-Group/a-groups-members-are-found-by-their-element-type-whatever-the-order-or-the-nullability.loft::n_test_a_group_forms_whichever_member_is_declared_first
kr-suite/rules/collections/Col-Group/a-write-through-any-member-of-a-linked-group-reaches-every-member.loft::n_test_a_vector_value_into_a_group_reaches_every_member
kr-suite/rules/collections/Col-Group/a-write-through-any-member-of-a-linked-group-reaches-every-member.loft::n_test_linked_group_fill
kr-suite/rules/collections/Slice-Value/a-slice-is-a-fresh-vector-at-every-vector-position.loft::n_test_either_side_of_the_plus
kr-suite/rules/heap/H-Free/each-pass-frees-the-answer-it-copied.loft::n_walk
kr-suite/rules/heap/H-Removed/a-cleared-vector-releases-the-records-a-reference-named.loft::n_one_field
kr-suite/rules/iteration/I-Comp/a-build-that-reads-its-destination-reads-what-it-held.loft::n_plain_field
kr-suite/rules/iteration/I-Comp/a-build-that-reads-its-destination-reads-what-it-held.loft::n_struct_and_scalar
kr-suite/rules/layout/L-Null/bind-copies-or-views-the-whole-boundary.loft::n_test_collection_field_off_a_borrowed_base_is_a_view
kr-suite/rules/layout/L-Null/bind-copies-or-views-the-whole-boundary.loft::n_test_collection_field_off_an_owned_base_copies
kr-suite/rules/ownership/O-Complete/2408-a-store-rebound-away-on-some-paths-lives-to-its-scope-exit.loft::n_in_a_loop
kr-suite/rules/ownership/O-Complete/2408-a-store-rebound-away-on-some-paths-lives-to-its-scope-exit.loft::n_in_a_loop_branch
kr-suite/rules/ownership/O-Complete/2408-a-store-rebound-away-on-some-paths-lives-to-its-scope-exit.loft::n_in_a_match_arm
kr-suite/rules/ownership/O-Complete/2408-a-store-rebound-away-on-some-paths-lives-to-its-scope-exit.loft::n_in_an_if
kr-suite/rules/ownership/O-Complete/2408-a-store-rebound-away-on-some-paths-lives-to-its-scope-exit.loft::n_of_texts
kr-suite/rules/ownership/O-Complete/2408-a-store-rebound-away-on-some-paths-lives-to-its-scope-exit.loft::n_on_the_straight_line
kr-suite/rules/ownership/O-Complete/2408-a-store-rebound-away-on-some-paths-lives-to-its-scope-exit.loft::n_to_a_literal
kr-suite/rules/ownership/O-Detach/2397-a-literal-reads-its-field-under-another-name.loft::n_test_control_a_write_through_the_view
kr-suite/rules/ownership/O-Detach/2402-a-chain-into-a-field-reads-the-field-before-its-build.loft::n_main
kr-suite/rules/ownership/O-Detach/a-rebind-or-a-build-reads-its-destination-through-any-view-first.loft::n_inside_the_arm
kr-suite/rules/ownership/O-Detach/a-rebind-or-a-build-reads-its-destination-through-any-view-first.loft::n_test_a_loop_variables_field
kr-suite/rules/ownership/O-Detach/a-vector-literal-reads-what-its-destination-held.loft::n_test_a_struct_field
kr-suite/rules/ownership/O-Move/a-join-arm-call-that-may-answer-its-argument-frees-it-once.loft::n_ad
kr-suite/rules/rewrites/R-ForwardWalk/a-walk-over-a-field-reads-its-length-where-the-body-can-change-it.loft::n_test_a_walk_over_a_scalar_field_keeps_its_length_read_where_the_body_can_re
kr-suite/rules/types/N-Coal/2302-a-default-after-a-field-operand-is-not-reported-redundant.loft::n_test_a_default_after_a_field_operand_is_used
kr-suite/rules/types/T-Chk-Vec/a-comprehension-body-is-checked-against-the-declared-element.loft::n_test_view_a_literal_default_then_another_field
kr-suite/rules/types/T-Chk-Vec/a-comprehension-body-is-checked-against-the-declared-element.loft::n_test_view_an_empty_default_then_another_field
kr-try/tests/scripts/164-buffer-is-the-place.loft::n_d5
kr-try/tests/scripts/164-callee-disturb.loft::n_c_rebuild_inline
kr-try/tests/scripts/164-callee-disturb.loft::n_rebuild_field
```
