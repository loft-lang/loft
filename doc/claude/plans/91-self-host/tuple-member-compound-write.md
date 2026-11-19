# @PLN91 — a compound write to a tuple member (prepared subject)

**The family:** "a compound assignment to a place that is not a known local" (50 sites in 45
programs at try 431d5ddc5). 36 of those sites, in 14 programs, write a TUPLE MEMBER: `q.0 += e`,
`t.1 += 5` on a tuple parameter, `t.0 += "!"`, `a.0 += [k]`, `v[1].1 += 3`, `w[0].1.1 += 7`.
The others (`tags += [2]` through a `&vector` alias, `c: &text = t; c += "!"`, `s.m? += 1`)
are other shapes and aren't part of this page.

**Judged right** on the interpreter (LOFT_POISON), --native-c (leak check) and --native, 300
passes. Values hand-computed (k = 3): `q = (k, "a"); q.0 += k * 10; q.1 += "b"` → `33 ab`;
`v = [(1, 2), (3, 4)]; v[1].1 += 3; v[0].0 -= 1` → `7 0`; `w[0].1.1 += 7` over `(2, 3)` → 10;
`a = ([1], 5); a.0 += [k]` → `[1, 3]`; a tuple PARAMETER is the callee's copy:
`number_beside_text(("x", 1))` answers 6 and the caller's tuple still reads 1. And
`v[tick(c)].1 += 5` runs `tick` ONCE per write (`(E-Asgn-Compound)`): two calls → `14`, count 2.

## Rust's form, by sub-shape

```
(a) a local tuple, scalar member          q.0 += k * 10
    q.0 = OpAddInt(q.0, OpMulInt(k, 10));

(b) a tuple PARAMETER: copied once at entry into __tp_<name>, every later use reads the copy
    fn f(t: (text, integer)) -> integer { t.1 += 5; t.1 }
    __tp_t: (text, integer) = t;                  <- first statement of the body
    __tp_t.1 = OpAddInt(__tp_t.1, 5);
    __tp_t.1;
    a heap member is deep-copied into a fresh store by a #tuple_member_move block:
    __vdb_1: ref(main_vector<integer>) = null;
    __tp_t: (vector<integer>["_tupcopy_1"], integer) = ({#tuple_member_move
        OpDatabase(__vdb_1, 20); _tupcopy_1 = OpGetField(__vdb_1, 0, 7); OpSetInt4(__vdb_1, 0, 0);
        OpClearVector(_tupcopy_1); OpAppendVector(_tupcopy_1, t.0, 0); _tupcopy_1 }, t.1);

(c) a text member: built in a work text, then stored
    q.1 += "b"
    OpClearText(__work_1); OpAppendText(__work_1, q.1); OpAppendText(__work_1, "b");
    q.1 = __work_1;
    (each += on the same member takes its own __work_N: two writes, __work_1 and __work_2)

(d) an append to a vector member: the ordinary append block over the member
    a.0 += [k]
    {#Vector:vector<integer>["a"]
      _vec_2: vector<integer>["a"] = a.0;
      OpPreAllocVector(_vec_2, 1, 8); OpPushInt(_vec_2, k);
      if OpVectorIsNull(a.0) a.0 = _vec_2 else null;
      _vec_2 }

(e) a member of a vector ELEMENT: written in place through the element
    v[1].1 += 3              (literal index: addressed inline, twice)
    OpSetInt(OpGetVector(v, 16, 1), 8, OpAddInt(OpGetInt(OpGetVector(v, 16, 1), 8), 3));
    w[0].1.1 += 7            (nested: one offset, 16 = 8 + 8, element size 24)
    OpSetInt(OpGetVector(w, 24, 0), 16, OpAddInt(OpGetInt(OpGetVector(w, 24, 0), 16), 7));
    v[tick(c)].1 += 5        (an index that runs a call: bound ONCE to a place view)
    __place_1: ref(__tuple<integer,integer>)["v"] = OpGetVector(v, 16, n_tick(c));
    OpSetInt(__place_1, 8, OpAddInt(OpGetInt(__place_1, 8), 5));
```

**What to code**, by sub-shape: (a) a `TupleSet` of the member with the operator over a
`TupleGet` of the same member; (b) a tuple parameter written anywhere in the body gets the
`__tp_<name>` copy as the body's first statement, every use is renamed to it, and a heap member
is copied through `#tuple_member_move`; (c) a text member goes through a fresh `__work_N`
(clear, append the member, append the right side) and is set back; (d) the ordinary
append-to-a-vector block with the member as its destination; (e) an element's member is an
`OpSet<width>` at the member's offset in the element, and the element is addressed inline
when its index is free of calls, else bound once to a `__place_N` view typed
`ref(__tuple<…>)["<vector>"]`.

**Checked with the slot matrix** (`slot_matrix.py cell '<stmt>' --type int`, 35 integer
fillers against their bind-first twins, interpreter and C): `tq.0 += <S>`, `tv[1].1 += <S>`
and `ta.0 += [<S>]` hold for every filler. **One oracle defect not to mirror:** an element
member whose INDEX calls a function value (`tv[fnv(…) % 2].1 += 5`) loses the write on
loft30-gate (`s=6` where the twin leaves 11). That's @PLN221 found.md row 1, loft#2578, fixed
on tuxedo-pr2574-next and on loft30-gate with the next join. Port the form for a plain call
index (the `__place_N` view) and let a function-value index take the same route.

## The sites (try 431d5ddc5)

```
kr-suite/rules/binding/B-Ref-Lvalue/a-stored-tuple-is-a-place-a-link-names.loft::n_test_a_vector_element_links_three_ways:40
kr-suite/rules/binding/B-Ref-Lvalue/a-stored-tuple-is-a-place-a-link-names.loft::n_test_a_struct_field_links_three_ways:53
kr-suite/rules/binding/B-Ref-Lvalue/a-stored-tuple-is-a-place-a-link-names.loft::n_test_a_link_is_forwarded_and_a_local_still_links:87
kr-suite/rules/binding/B-Ref-Lvalue/a-stored-tuple-is-a-place-a-link-names.loft::n_test_links_in_a_loop:96
kr-suite/rules/binding/B-View-Depth/a-tuple-copies-what-a-plain-bind-would-view.loft::n_test_a_tuple_off_a_borrowed_base_copies:22
kr-suite/rules/binding/B-View-Depth/a-tuple-copies-what-a-plain-bind-would-view.loft::n_test_an_index_read_views_and_a_tuple_copies:37
kr-suite/rules/calls/F-ParamScalar/a-compound-write-to-a-tuple-parameters-member-is-the-callees-copy.loft::n_appended:17
kr-suite/rules/calls/F-ParamScalar/a-compound-write-to-a-tuple-parameters-member-is-the-callees-copy.loft::n_number_beside_text:19
kr-suite/rules/calls/F-ParamScalar/a-compound-write-to-a-tuple-parameters-member-is-the-callees-copy.loft::n_both:21
kr-suite/rules/calls/F-ParamTuple/a-by-value-tuple-parameter-is-the-callees-own-value.loft::n_c_append:29
kr-suite/rules/calls/F-ParamTuple/a-by-value-tuple-parameter-is-the-callees-own-value.loft::n_c_hash:37
kr-suite/rules/calls/F-ParamTuple/a-by-value-tuple-parameter-is-the-callees-own-value.loft::n_c_text:41
kr-suite/rules/calls/F-ParamTuple/a-by-value-tuple-parameter-is-the-callees-own-value.loft::n_c_field2:95
kr-suite/rules/iteration/I-Comp/a-comprehension-yields-a-tuple-element-whole.loft::n_test_a_nested_or_heap_member_is_copied_deep:50
kr-suite/rules/layout/L-Tuple/a-stored-tuple-is-a-record-packed-like-a-struct-keeping-its-written-positions.loft::n_test_a_record_backed_text_integer_member:112
kr-suite/rules/layout/L-Tuple/a-stored-tuple-is-a-record-packed-like-a-struct-keeping-its-written-positions.loft::n_test_a_two_byte_aligned_element_keeps_every_member:176
kr-suite/rules/layout/L-Tuple/a-stored-tuple-is-a-record-packed-like-a-struct-keeping-its-written-positions.loft::n_test_a_four_byte_aligned_element_keeps_every_member:185
kr-suite/rules/layout/L-Tuple/a-stored-tuple-is-a-record-packed-like-a-struct-keeping-its-written-positions.loft::n_test_a_nested_tuple_element_keeps_every_member:192
kr-suite/rules/operational/E-Asgn-Compound/2426-a-tuple-member-of-a-call-indexed-element-is-one-place.loft::n_bump:43
kr-suite/rules/operational/E-Asgn-Compound/2426-a-tuple-member-of-a-call-indexed-element-is-one-place.loft::n_test_a_call_indexed_tuple_member:48
kr-suite/rules/operational/E-Asgn-Compound/2426-a-tuple-member-of-a-call-indexed-element-is-one-place.loft::n_test_the_controls:78
kr-suite/rules/rewrites/R-InPlace/a-link-is-rebound-and-hoisted-around-as-its-place-allows.loft::n_test_a_link_to_a_plain_record_blocks_no_hoist:135
kr-suite/rules/tuples/T-Cons/1689-a-tuples-heap-member-is-its-own-copy.loft::n_main:93
kr-suite/rules/tuples/T-Cons/2253-a-compound-member-copies-the-local-it-answers.loft::n_test_every_spelling_of_a_vector_member:36
kr-suite/rules/tuples/T-Cons/2253-a-compound-member-copies-the-local-it-answers.loft::n_test_a_nested_vector_and_a_field_default:61
kr-suite/rules/tuples/T-Cons/2253-a-compound-member-copies-the-local-it-answers.loft::n_test_a_struct_and_a_hash_member:77
kr-suite/rules/tuples/T-Cons/2253-a-compound-member-copies-the-local-it-answers.loft::n_test_a_loop_alternating_a_fresh_arm_and_a_local:100
kr-suite/rules/tuples/T-Cons/2253-a-compound-member-copies-the-local-it-answers.loft::n_test_arms_that_build_beside_an_arm_that_names_a_local:116
kr-suite/rules/tuples/T-Cons/1698-a-tuple-held-as-a-vector-element-is-a-place.loft::n_through_link:34
kr-suite/rules/tuples/T-Cons/1698-a-tuple-held-as-a-vector-element-is-a-place.loft::n_bind_off_link:37
kr-suite/rules/tuples/T-Cons/1698-a-tuple-held-as-a-vector-element-is-a-place.loft::n_destructure_off_param:42
kr-suite/rules/tuples/T-Cons/1698-a-tuple-held-as-a-vector-element-is-a-place.loft::n_main:50
kr-suite/rules/tuples/T-Proj/1698b-a-stored-tuples-text-and-scalar-members-are-written-in-place.loft::n_main:22
kr-suite/rules/tuples/T-Record/a-tuple-member-holds-its-value-everywhere-a-narrow-one-or-a-text.loft::n_bump_copy:63
kr-suite/rules/tuples/T-Record/a-tuple-member-holds-its-value-everywhere-a-narrow-one-or-a-text.loft::n_test_a_text_tuple_member_holds_its_value_everywhere:91
kr-suite/rules/types/C-Var/a-tuple-member-is-typed-by-its-declaration-a-variant-one-included.loft::n_test_a_local_link_writes_either_member:158
```
