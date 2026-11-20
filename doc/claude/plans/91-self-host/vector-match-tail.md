# @PLN91 — a vector `match` tail (prepared subject)

**The family:** "a function returning a vector from this tail: null" has 55 sites at try
4b17a78b7. Thirty of them are one shape: a function whose TAIL is a slice-pattern `match`
answering a vector. The arms are a rest capture (`[_h, ..t] => t`), a whole-subject binding
(`whole => whole`) or a literal default (`_ => []`). This is the handoff's item 3 ("tails that are a
`match` answering a view"), larger than the eight sites listed there.

**Judged right** on the interpreter (LOFT_POISON), --native-c (leak check) and --native, 1000
passes with no leak: `tail([1,2,3]) += [9]` → `[2,3,9]`; `tail([])` → `[]`; `tail_w([4,5])` →
`[5]`; and `e = tail_w(src); e += [1]` leaves `src` at `[6,7]`. The rest is a COPY, never a view of
the caller's vector (`a-rest-is-a-fresh-capture-wherever-it-stands`).

## Rust's form

`fn tail(v: vector<integer>) -> vector<integer> { match v { [_, ..rest] => rest, _ => [] } }`:

```
  rest#1: vector<integer> mutable nullable hidden      <- the return buffer, RENAMED onto the first arm's rest capture
  {#block: vector<integer>["__retbuf"]
    {#vector_match: vector<integer>["rest#1"]
      if OpLeInt(1, OpLengthVector(v)) {#slice_binding ["rest#1"]
        { OpClearVector(rest#1)
          { _rest_lo_1 = 1; _rest_hi_1 = OpMinInt(OpLengthVector(v), 0); _rest_idx_1 = OpConvIntFromNull() }
          loop {#Slice materialise  _slice_elm_1 = OpGetInt(OpGetVectorNullable(v, 8, {#Iter range …}), 0);
                 _elm_1 = OpNewRecord(rest#1, 7, 65535); OpSetInt(_elm_1, 0, _slice_elm_1); OpFinishRecord(rest#1, _elm_1, 7, 65535) } };
        rest#1 }
      else {#Vector ["__vdb_1"]  OpDatabase(__vdb_1, 20); _vec_1 = OpGetField(__vdb_1, 0, 7); OpSetInt4(__vdb_1, 0, 0);
        { OpClearVector(rest#1); OpAppendVector(rest#1, _vec_1, 0); OpFreeRef(__vdb_1); rest#1 } } } }
```

A `whole => whole` arm (`tail_w`): `whole#1: vector<integer>["v"] = v; { OpClearVector(t#1); OpAppendVector(t#1, whole#1, 0); t#1 }`,
and the match's deps name both captures (`["t#1", "whole#1"]`).

**What to code:** for a vector-returning function whose tail is a slice `match`, rename the return
buffer onto the FIRST arm's rest capture (the hidden parameter takes its name, `rest#1`). The rest
is materialised straight into it (`OpClearVector`, then the slice loop pushing each element). Every
other arm's value is copied in (`OpClearVector(<buf>); OpAppendVector(<buf>, <value>, 0); <buf>`),
with `OpFreeRef` of a literal default's `__vdb_N`. A captured head (`_h#1 = OpGetInt(OpGetVector(v, 8, 0), 0)`)
reads the subject in place.

## The thirty sites (try 4b17a78b7)

```
kr-suite/rules/binding/B-Scope/a-patterns-names-end-with-its-arm.loft::n_tail  line 32
kr-suite/rules/binding/P-Cap-View/a-pattern-capture-is-a-view-of-the-subjects-place-on-b-views-terms.loft::n_r_vec  line 120
kr-suite/rules/heap/H-FreeNull/freeing-null-releases-nothing-on-every-path.loft::n_field  line 156
kr-suite/rules/matching/P-Cap/a-capture-in-or-of-a-group-binds-what-the-group-matched.loft::n_grab  line 66
kr-suite/rules/matching/P-Rest/a-binding-arm-answering-the-subject-leaves-the-callers-vector.loft::n_head3  line 27
kr-suite/rules/matching/P-Rest/a-binding-arm-answering-the-subject-leaves-the-callers-vector.loft::n_only  line 29
kr-suite/rules/matching/P-Rest/a-binding-arm-answering-the-subject-leaves-the-callers-vector.loft::n_rebound  line 32
kr-suite/rules/matching/P-Rest/a-binding-arm-answering-the-subject-leaves-the-callers-vector.loft::n_tail_i  line 25
kr-suite/rules/matching/P-Rest/a-binding-arm-answering-the-subject-leaves-the-callers-vector.loft::n_tail_p  line 26
kr-suite/rules/matching/P-Rest/a-binding-arm-answering-the-subject-leaves-the-callers-vector.loft::n_tail_t  line 24
kr-suite/rules/matching/P-Rest/a-binding-arm-answering-the-subject-leaves-the-callers-vector.loft::n_tail_v  line 28
kr-suite/rules/matching/P-Rest/a-named-rest-leaves-room-for-the-fixed-elements-after-it.loft::n_struct_mid  line 131
kr-suite/rules/matching/P-Rest/a-rest-is-a-fresh-capture-wherever-it-stands.loft::n_drop2_i  line 53
kr-suite/rules/matching/P-Rest/a-rest-is-a-fresh-capture-wherever-it-stands.loft::n_tail  line 175
kr-suite/rules/matching/P-Rest/a-rest-is-a-fresh-capture-wherever-it-stands.loft::n_tail_i  line 50
kr-suite/rules/matching/P-Rest/a-rest-is-a-fresh-capture-wherever-it-stands.loft::n_tail_p  line 72
kr-suite/rules/matching/P-Rest/a-rest-is-a-fresh-capture-wherever-it-stands.loft::n_tail_t  line 58
kr-suite/rules/types/N-Shape/1493-a-branch-arm-yielding-a-locals-field-delivers-a-copy.loft::n_bound_then_yielded  line 155
kr-suite/rules/types/N-Shape/1493-a-branch-arm-yielding-a-locals-field-delivers-a-copy.loft::n_first_arm  line 76
kr-suite/rules/types/N-Shape/1493-a-branch-arm-yielding-a-locals-field-delivers-a-copy.loft::n_from_call  line 101
kr-suite/rules/types/N-Shape/1493-a-branch-arm-yielding-a-locals-field-delivers-a-copy.loft::n_literal_filled  line 84
kr-suite/rules/types/N-Shape/1493-a-branch-arm-yielding-a-locals-field-delivers-a-copy.loft::n_nested  line 97
kr-suite/rules/types/N-Shape/1493-a-branch-arm-yielding-a-locals-field-delivers-a-copy.loft::n_outer_local  line 89
kr-suite/rules/types/N-Shape/1493-a-branch-arm-yielding-a-locals-field-delivers-a-copy.loft::n_parameter_field  line 143
kr-suite/rules/types/N-Shape/1493-a-branch-arm-yielding-a-locals-field-delivers-a-copy.loft::n_records  line 93
kr-suite/rules/types/N-Shape/1493-a-branch-arm-yielding-a-locals-field-delivers-a-copy.loft::n_second_arm  line 160
kr-suite/rules/types/N-Shape/1493-a-branch-arm-yielding-a-locals-field-delivers-a-copy.loft::n_views_a_parameter  line 147
kr-suite/rules/types/P-Rep-Ty/a-repetition-capture-is-a-dense-vector-of-its-element.loft::n_pt_grab  line 37
kr-suite/rules/types/P-Rep-Ty/a-repetition-capture-is-a-dense-vector-of-its-element.loft::n_pt_rest  line 41
kr-suite/rules/types/P-Rep-Ty/a-repetition-collects-a-heap-field-per-element.loft::n_escaped  line 47
```
