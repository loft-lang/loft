# @PLN91 — a tuple with a tuple member (prepared subject)

**The family:** "a tuple with a member of this type: TyTuple", 26 sites in 15 programs at try
ca4065330: `a = ((1, "a1"), 2)`, `n = (("in", 4), "out")`, `v: vector<((integer, text),
integer)> = [((1, "x"), 10), mk(2)]`, a call returning one, a member write `b.0.1 = …`.

**Judged right** on the interpreter (LOFT_POISON), --native-c (leak check) and --native, with 300
passes. Expected values computed by hand: `b = a; b.0.1 = "zz"; b.1 = 9` leaves `a` at `1/a1/2`
(the copy is whole) and gives `b` `zz/9`; a vector of three such tuples (a literal, two from
`mk(i) -> ((i, "s{i}"), i * 10)`) → `v[2].0.1 = "s3"`; the sum of `e.0.0 + e.1` → 66.

## Rust's form

```
a = ((1, "a1"), 2)                 a: ((integer, text), integer) = ((1i32, "a1"), 2i32);      <- a nested Tuple literal, as is
b = a                              b = ({#tuple_member_move: (integer, text)
                                          _tuphold_1: (integer, text["a"]) = a.0;
                                          (_tuphold_1.0, _tuphold_1.1) }, a.1);             <- a TUPLE member moved through a hold; a scalar member read
b.0.1 = "zz"                       __work_p2_1: text = "zz";                                    <- the text value first (a pass-2 work text)
                                   {#nested_tuple_assign  __ref_1: (integer, text["b"]) = b.0;
                                                          __ref_1.1 = __work_p2_1;  b.0 = __ref_1 }   <- read the member, write the leaf, write back
b.0.0 = 5                          {#nested_tuple_assign  __ref_2 = b.0;  __ref_2.0 = 5i32;  b.0 = __ref_2 }
b.1 = 9                            b.1 = 9i32;                                                  <- depth 1: a plain TuplePut
vector element, a literal          _elm_1: ref(boolean)["v"] = OpNewRecord(v, 103, 65535);
                                   {#tuple_field_set OpSetInt(_elm_1, 0, 1); OpSetText(_elm_1, 8, "x") };  OpSetInt(_elm_1, 16, 10);
                                   OpFinishRecord(v, _elm_1, 103, 65535);                        <- the inner tuple FLATTENED at its offsets
vector element, a call             {#tuple_elem_set
                                     __ref_4: ((integer, text), integer) = {#tuple_unbox
                                        __ref_3: ref(__tuple<(integer, text),integer>) = n_mk(2, __ref_2);
                                        ((OpGetInt(__ref_3, 0), OpGetText(__ref_3, 8)), OpGetInt(__ref_3, 16)) };
                                     __ref_5: (integer, text) = __ref_4.0;
                                     OpSetInt(_elm_1, 0, __ref_5.0);  OpSetText(_elm_1, 8, __ref_5.1);  OpSetInt(_elm_1, 16, __ref_4.1) }
```

`build_nested_tuple_assign` (expressions.rs:370) is the member write: the chain of member reads
into work vars, `TuplePut` of the leaf into the deepest, then each intermediate written back up
to the root in reverse.

**What to code:** keep a nested tuple a nested `Tuple` value in locals. Copy a tuple member
through a `_tuphold_N` in a `tuple_member_move` block. Write a member at depth 2+ by
`nested_tuple_assign`. In a vector element, the tuple is flattened at the element's field
offsets: a literal by `tuple_field_set` for the inner tuple, then the outer scalar. A call's
answer is unboxed from its `__tuple<…>` record (`tuple_unbox`, the call's buffer `__ref_N`), then
set field by field (`tuple_elem_set`). Text values bound for a member are hoisted to a
`__work_p2_N` first.

## The sites (try ca4065330)

```
kr-suite/rules/binding/B-Copy/a-copy-of-a-compound-value-copies-every-member.loft::n_test_a_nested_tuple_with_a_text_leaf
kr-suite/rules/calls/F-ParamTuple/a-by-value-tuple-parameter-is-the-callees-own-value.loft::n_test_every_member_kind
kr-suite/rules/ownership/O-Detach/2261-a-tuple-rebuilt-from-its-own-members-reads-them-first.loft::n_test_every_member_kind_over_four_passes
kr-suite/rules/tuples/T-Absent/an-absent-tuple-meets-the-type-its-author-declares.loft::n_c5
kr-suite/rules/tuples/T-Cons/1590-a-tuple-member-string-may-open-a-hole.loft::n_main
kr-suite/rules/tuples/T-Cons/1698-a-tuple-held-as-a-vector-element-is-a-place.loft::n_main
kr-suite/rules/tuples/T-Record/a-link-to-a-text-tuple-member-writes-the-member.loft::n_nested
kr-suite/rules/tuples/T-Record/a-tuple-member-holds-its-value-everywhere-a-narrow-one-or-a-text.loft::n_test_a_text_tuple_member_holds_its_value_everywhere
kr-suite/rules/types/C-Var/sibling-variants-join-to-their-enum.loft::n_test_nested_null_and_enum_first_members_join
kr-suite/rules/types/N-Coal/1425-a-nested-tuple-finds-its-own-null-sentinel.loft::n_a_twice_nested_tuple_discharges_too
kr-suite/rules/types/N-Domain/1478-a-text-tuple-member-fits-its-owned-slot.loft::n_main
kr-suite/rules/types/N-Shape/a-tuple-member-releases-once-whatever-it-is-declared-and-beside.loft::n_c_both_in_one
kr-suite/rules/types/N-Shape/a-tuple-member-releases-once-whatever-it-is-declared-and-beside.loft::n_c_declared_nested
kr-suite/rules/types/N-Shape/a-tuple-member-releases-once-whatever-it-is-declared-and-beside.loft::n_c_depth3
kr-suite/rules/types/N-Shape/a-tuple-member-releases-once-whatever-it-is-declared-and-beside.loft::n_c_inner
kr-suite/rules/types/N-Shape/a-tuple-member-releases-once-whatever-it-is-declared-and-beside.loft::n_c_nested
kr-suite/rules/types/N-Shape/a-tuple-member-releases-once-whatever-it-is-declared-and-beside.loft::n_c_nested_nullable
kr-suite/rules/types/N-Shape/a-tuple-member-releases-once-whatever-it-is-declared-and-beside.loft::n_c_no_copy
kr-suite/rules/types/N-Shape/a-tuple-member-releases-once-whatever-it-is-declared-and-beside.loft::n_c_nullable_inner
kr-suite/rules/types/N-Shape/a-tuple-member-releases-once-whatever-it-is-declared-and-beside.loft::n_c_outer_and_inner
kr-suite/rules/types/N-Shape/a-tuple-member-releases-once-whatever-it-is-declared-and-beside.loft::n_c_two_inner
kr-suite/rules/types/N-Shape/a-tuple-member-releases-once-whatever-it-is-declared-and-beside.loft::n_c_vector_member
kr-suite/rules/types/N-Shape/a-tuple-member-releases-once-whatever-it-is-declared-and-beside.loft::n_s3_c_chain
kr-suite/rules/types/P-Alt-Diff/an-absent-tuple-capture-of-an-alternative-is-the-all-null-tuple.loft::n_test_a_tuple_alternative_absent_tuple_capture_is_all_null
kr-try/tests/scripts/1512-a-tuple-argument-call-member-releases-once.loft::n_nested
kr-try/tests/scripts/an-arrived-absence-keeps-a-tuples-text-frees.loft::n_main
```
