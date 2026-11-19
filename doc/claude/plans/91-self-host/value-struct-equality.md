# @PLN91 — `==` on two value structs, field by field (prepared subject)

**The family:** "a comparison of two value structs (field by field)", 19 sites in 8 programs at
try c268bf4b9, and the ONLY refusal in 2. `a == b` / `a != b` where both are a `value struct`
(`@FR-E-Eq`, @C91: a value compares by content, a reference by identity).

**Judged right** on the interpreter (LOFT_POISON), --native-c (leak check) and --native. Expected
values computed by hand: equal fields → true, one differing field → false; a copy equals its
source; `!=` on texts built differently with equal characters → false; a nested value struct
compared by content (`O { p: a, k: 5 } == O { p: b, k: 5 }` with `a`, `b` equal → true; with `p`
differing → false); 1000 comparisons against `P { x: 1, y: i % 3 }` → 333; a call operand
`mkp(1) == a` → true / false.

## Rust's form: `value_struct_eq` (compare.rs:398)

```
fn eq_p(a: P, b: P) -> boolean { a == b }          P { x: integer, y: integer }
    if OpEqInt(OpGetInt(a, 0), OpGetInt(b, 0)) OpEqInt(OpGetInt(a, 8), OpGetInt(b, 8)) else false

fn ne_t(a: T, b: T) -> boolean { a != b }          T { s: text, n: integer }   (s at offset 8, n at 0)
    OpNot(if OpEqText(OpGetText(a, 8), OpGetText(b, 8)) OpEqInt(OpGetInt(a, 0), OpGetInt(b, 0)) else false)

fn eq_o(a: O, b: O) -> boolean { a == b }          O { p: P, k: integer }
    if (if OpEqInt(OpGetInt(OpGetField(a, 0, 100), 0), OpGetInt(OpGetField(b, 0, 100), 0))
           OpEqInt(OpGetInt(OpGetField(a, 0, 100), 8), OpGetInt(OpGetField(b, 0, 100), 8)) else false)
       OpEqInt(OpGetInt(a, 16), OpGetInt(b, 16)) else false

fn ec(a: P) -> boolean { mkp(1) == a }             an operand that runs a user call
    __ref_1: ref(P) = null;                                       <- the call's return buffer, in the head
    {#value_eq:boolean  ___veq_1: ref(P) = n_mkp(1, __ref_1);
                        if OpEqInt(OpGetInt(___veq_1, 0), OpGetInt(a, 0)) OpEqInt(OpGetInt(___veq_1, 8), OpGetInt(a, 8)) else false }
```

**What to code:** for `==` / `!=` with both operand types the same value struct, fold over its
FIELDS IN DECLARATION ORDER, not offset order (`T` compares `s` at offset 8 before `n` at 0).
Skip a `Routine`-typed member (a method is no field). Each field is that field type's `==`
resolved as an ordinary `==` call (`call_op`), so a nested value struct recurses into this same
rule, a text gives `OpEqText`, a float its float `==`. Fold the results left to right as
`acc = if acc cmp else false`: the first field alone, no `if`; no fields at all → `true`. `!=`
wraps the whole fold in ONE `OpNot`. An operand that `ir_has_user_call` is set ONCE into
`__veq_N: ref(<struct>)` (no deps; the variable prints `___veq_1`, from the `__` prefix plus
create_unique's `_`), and the fold plus the sets go in a `v_block` labelled `value_eq`. Any other
operand is read in place per field. If any field's `==` resolves to unknown or null, decline back
to the ordinary resolution (`None`). A reference field compares by identity (deviation D-op-16).

## The sites (try c268bf4b9)

```
kr-suite/rules/calls/F-Block/a-match-at-a-tail-answers-its-arm-not-a-temporarys-free.loft::n_bound
kr-suite/rules/calls/F-Block/a-match-at-a-tail-answers-its-arm-not-a-temporarys-free.loft::n_if_tail
kr-suite/rules/calls/F-Block/a-match-at-a-tail-answers-its-arm-not-a-temporarys-free.loft::n_pick
kr-suite/rules/calls/F-Block/a-match-at-a-tail-answers-its-arm-not-a-temporarys-free.loft::n_unequal
kr-suite/rules/calls/F-Recv/1817-an-operator-reaches-every-overload-of-its-second-operand.loft::n_main
kr-suite/rules/operational/E-Eq/1580-a-value-struct-compares-by-content.loft::n_c1
kr-suite/rules/operational/E-Eq/1580-a-value-struct-compares-by-content.loft::n_g1
kr-suite/rules/operational/E-Eq/1580-a-value-struct-compares-by-content.loft::n_g2
kr-suite/rules/operational/E-Eq/1580-a-value-struct-compares-by-content.loft::n_g3
kr-suite/rules/operational/E-Eq/1580-a-value-struct-compares-by-content.loft::n_g4
kr-suite/rules/operational/E-Eq/1580-a-value-struct-compares-by-content.loft::n_g5
kr-suite/rules/operational/E-Eq/1580-a-value-struct-compares-by-content.loft::n_g6
kr-suite/rules/operational/E-Eq/1580-a-value-struct-compares-by-content.loft::n_g7
kr-suite/rules/operational/E-Eq/1580-a-value-struct-compares-by-content.loft::n_g8
kr-suite/rules/operational/E-Eq/1581-a-concrete-not-equal-derives-from-a-user-op-eq.loft::n_g2
kr-suite/rules/operational/E-Eq/a-value-struct-with-methods-compares-by-its-fields.loft::n_main
kr-suite/rules/operational/E-Eq/c91-a-struct-compares-by-content-through-its-references.loft::n_test_a_value_struct_follows_its_reference
kr-suite/rules/operators/Op-Back/every-operator-form-reaches-the-definition-that-compiles.loft::n_test_negate_divided_by_and_remainder_are_operator_methods
kr-suite/rules/operators/Op-Std/a-fn-named-for-an-operator-is-an-ordinary-function.loft::n_main
```
