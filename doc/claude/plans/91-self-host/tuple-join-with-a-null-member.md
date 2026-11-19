# @PLN91 — a join of tuples where one arm's member is `null` (prepared subject)

**The family:** "`if` arms of different types: TyUnknown, TyTuple", 15 sites in 5 programs at try
a623de4a5 (1682's cells, 1380, 1350, a-record-a-value-branch). One arm builds a tuple with a bare
`null` member, so its member type is unknown until the join: `if z == 1 { (null, 3) } else if
z == 2 { (4, 5) } else { (6, 7) }` under `-> (integer?, integer)`, or `t = if c == 0 { (null, 1) }
else { (T9 { a: 40 }, 2) }`.

**Judged right** on the interpreter (LOFT_POISON), --native-c (leak check) and --native.
Expected values computed by hand: `chain(1)` → `(null, 3)` (`.0 ?? -9` → -9); `chain(2)` → `4/5`;
`first_null(0)` → `(null ?? T9 { a: -1 }).a + 1` = 0; `first_null(1)` → 42; a struct-enum member
joined with `null` (`match k { 1 => (null, 11), 2 => (Circle {…}, 22), _ => (Sq {…}, 33) }`) →
`none11 some22 some33`.  (`(call().0 ?? D).a` over such a call is loft#2577, a released
defect found preparing this; it is not part of this subject.)

## Rust's form

```
fn chain(z) -> (integer?, integer) { if z == 1 { (null, 3) } else if z == 2 { (4, 5) } else { (6, 7) } }
    if OpEqInt(z, 1) {#block: (integer?, integer)  (OpConvIntFromNull(), 3) }        <- the null arm: the JOINED type, `null` as the member type's null
    else if OpEqInt(z, 2) {#block: (integer, integer)  (4, 5) }                      <- later arms keep their OWN type at a function's tail chain
    else {#block: (integer, integer)  (6, 7) }
t = if c == 0 { (null, 1) } else { (T9 { a: 40 }, 2) }
    t: (ref(T9)?, integer) = if OpEqInt(c, 0) {#block: (ref(T9)?, integer)  (OpNullRefSentinel(), 1) }
                             else {#block: (ref(T9)?, integer)  ({#Object __ref_p2_1 …OpDatabase(__ref_p2_1, 102)…}, 2) };
                                                                                       <- in a BIND both arms carry the joined type
```

**What to code:** type a tuple arm's bare `null` member from the join: the other arms' member, made
nullable. A nullable integer is `OpConvIntFromNull()` (each scalar its own `…FromNull`), a record
`OpNullRefSentinel()`, a struct-enum its enum null. The arm holding the null is typed with the
joined tuple type. The other arms keep their own type at a function's tail `else if` chain, and
take the joined type under a bind, as measured above. Check a `match` join and the struct-enum
case (1682 `enum_member`, a-record-a-value-branch `pick`) with `elab_trace.py` before porting:
they were judged right but their arm typing was not read here.

## The sites (try a623de4a5)

```
kr-suite/rules/calls/F-Default/1758-a-text-tuple-default-answers-what-its-argument-would.loft::
kr-suite/rules/calls/F-Ret/1350-a-lifetime-tuple-result-joins-a-tuple-literal.loft::n_test_join_directions
kr-suite/rules/heap/H-Drop/a-record-a-value-branch-or-a-field-literal-builds-is-released-once.loft::n_pick_if
kr-suite/rules/types/N-Decl/1380-an-else-if-chain-answers-in-its-first-arms-type.loft::n_test_a_lifetime_tuple_then_arm_boxes_the_chains_stack_tuple
kr-suite/rules/types/T-Chk/1682-a-null-member-in-the-first-tuple-arm-does-not-pin-the-join.loft::n_chain
kr-suite/rules/types/T-Chk/1682-a-null-member-in-the-first-tuple-arm-does-not-pin-the-join.loft::n_enum_member
kr-suite/rules/types/T-Chk/1682-a-null-member-in-the-first-tuple-arm-does-not-pin-the-join.loft::n_float_member
kr-suite/rules/types/T-Chk/1682-a-null-member-in-the-first-tuple-arm-does-not-pin-the-join.loft::n_main
kr-suite/rules/types/T-Chk/1682-a-null-member-in-the-first-tuple-arm-does-not-pin-the-join.loft::n_narrow_member
kr-suite/rules/types/T-Chk/1682-a-null-member-in-the-first-tuple-arm-does-not-pin-the-join.loft::n_nested_first
kr-suite/rules/types/T-Chk/1682-a-null-member-in-the-first-tuple-arm-does-not-pin-the-join.loft::n_null_text_member
kr-suite/rules/types/T-Chk/1682-a-null-member-in-the-first-tuple-arm-does-not-pin-the-join.loft::n_scalar_first
kr-suite/rules/types/T-Chk/1682-a-null-member-in-the-first-tuple-arm-does-not-pin-the-join.loft::n_second_member
kr-suite/rules/types/T-Chk/1682-a-null-member-in-the-first-tuple-arm-does-not-pin-the-join.loft::n_struct_member
kr-suite/rules/types/T-Chk/1682-a-null-member-in-the-first-tuple-arm-does-not-pin-the-join.loft::n_text_member
```
