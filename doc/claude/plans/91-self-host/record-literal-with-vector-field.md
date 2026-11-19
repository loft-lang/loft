# @PLN91 — a record literal with a vector field written to a place (prepared subject)

**The family:** "a record literal written into a field or an element: a vector literal field",
15 sites at tip e43620208, and the ONLY refusal in 3 programs. A record literal that holds a
vector literal field, assigned to an existing field or element:
`vb.inner = Vf { v: [7, 8, 9], n: 3 }`, `vs.els[i] = Vf { v: [len(vs.els[i].v), 5], n: 2 }`,
`b.c = C2 { items: [9], o: b.c.items }`.

**Judged right** on the interpreter (LOFT_POISON), --native-c (leak check) and --native, with
1000 passes and no leak. Expected values computed by hand: the field write → sum 24, n 3; an
element literal reading its own place's vector length → `[3,5]`, which repeated settles at
`[2,5]`; `b.c = C2 { items: [9], o: b.c.items }` → items `[9]`, o the OLD `[4,5]`.

**⚠ Superseded for FIELD places by main 21992d3e3 (#2574), measured at loft30-gate 06d02f19c:**
`vb.inner = Vf { v: [7, 8, 9], n: 3 }` is now built IN PLACE (`OpSetInt4(OpGetField(vb, 0, 100),
8, 0)`, then `OpPushInt` into the field's vector, then `OpSetInt` of `n`, with `_elm_1: ref(boolean) = null`
left in the body). `vs.els[i] = Vf { v: [len(vs.els[i].v), 5], n: 2 }` is still built apart and
copied, as below. That cell is both an element place AND a literal reading its place, so which of
the two keeps it apart is not separated here; read `@FR-R-InPlaceLiteral` in formal/rewrites.md
before porting the decision.

**Not in place, by the rule (as it read before 21992d3e3):** `(R-InPlaceLiteral)` (formal/rewrites.md) declines an element
whose type owns a collection field, and Rust declines the field form too. A declined rewrite takes
the plain path: the literal is BUILT APART and COPIED in. This is not a Rust defect; the port
mirrors the copy.

## Rust's form

```
fn f_vec(vb: VBox) { vb.inner = Vf { v: [7, 8, 9], n: 3 }; }
    __ref_p2_1: ref(Vf) = null;
    _elm_1: ref(boolean) = null;                     <- a head null-init of the vector literal's pass-1 element temp; nothing uses it
    OpCopyRecord({#Object ["__ref_p2_1"]
        __ref_p2_1 = null; OpDatabase(__ref_p2_1, 100);
        OpSetInt4(__ref_p2_1, 8, 0);                  <- the vector field starts empty
        { OpPushInt(OpGetField(__ref_p2_1, 8, 7), 7); OpPushInt(… 8); OpPushInt(… 9) };   <- one push per item, into the field
        OpSetInt(__ref_p2_1, 0, 3);                   <- then the scalar fields, in declaration order
        __ref_p2_1 }, OpGetField(vb, 0, 100), 100);   <- copied into the place (the place is the 2nd argument)

fn e_vec(vs: VSc, i: integer) { vs.els[i] = Vf { v: [len(vs.els[i].v), 5], n: 2 }; }
    __ref_p2_2: ref(Vf) = null;  __ref_p2_1: ref(Vf) = null;      <- TWO minted, in this order; only _2 is used
    OpCopyRecord({#Object ["__ref_p2_2"] … OpPushInt(OpGetField(__ref_p2_2, 8, 7), t_6vector_len(OpGetField(OpGetVector(OpGetField(vs, 0, 103), 16, i), 8, 7))) … },
                 OpGetVector(OpGetField(vs, 0, 103), 16, i), 100);
```

The items read the OLD place directly (no staging): the record is built apart, so nothing is
written until `OpCopyRecord`, which releases the place's old vector.

**What to code:** for a record literal assigned to a field or element, when the record type owns
a vector field that the literal gives as a vector literal, decline the in-place write. Build the
literal as an `#Object` in a fresh `__ref_p2_N`: `OpDatabase`, then the vector field `OpSetInt4(…, 0)`
and one `OpPushInt` per item through `OpGetField(buf, <offset>, <vector type>)`, then the scalar
fields. Then `OpCopyRecord(built, <place>, <record type>)`. Mirror the head exactly: the unused
`_elm_1: ref(boolean)` in the field form, and the unused extra `__ref_p2_1` minted AFTER `__ref_p2_2`
in the element form. Trace both with `LOFT_STATE_TRACE` `var` rows for where pass 1 mints them.

## The sites (tip e43620208)

```
kr-suite/rules/binding/B-Disturb/a-disturbance-ends-the-places-inside-its-container-and-only-those.loft::n_g2
kr-suite/rules/binding/B-Disturb/a-disturbance-ends-the-places-inside-its-container-and-only-those.loft::n_j2
kr-suite/rules/binding/B-Disturb/a-disturbance-ends-the-places-inside-its-container-and-only-those.loft::n_k2
kr-suite/rules/collections/Col-Insert/1572-an-ordered-collection-displaces-a-repeated-key.loft::n_relink1572
kr-suite/rules/heap/H-ClearRelease/a-reused-vector-or-place-releases-what-it-held.loft::n_test_an_in_place_literal_in_a_loop
kr-suite/rules/heap/H-WriteOOB/a-write-to-a-place-that-names-nothing-lands-nowhere.loft::n_records
kr-suite/rules/ownership/O-One-Kind/a-removed-temporary-store-answers-as-the-store-it-stands-for.loft::n_test_a_literal_written_into_its_element
kr-suite/rules/rewrites/R-InPlaceLiteral/a-literal-reading-its-places-own-collection-keeps-it.loft::n_test_a_field_literal_stages_copies_of_its_heap_reads
loft-keyed-ranges/tests/scripts/132-vector-elemset-inline-literal-uaf.loft::n_test_p313_field_and_append_unaffected
loft-keyed-ranges/tests/scripts/164-in-place-literal.loft::n_e_computed_index
loft-keyed-ranges/tests/scripts/164-in-place-literal.loft::n_e_effect_index
loft-keyed-ranges/tests/scripts/164-in-place-literal.loft::n_e_heap
loft-keyed-ranges/tests/scripts/164-in-place-literal.loft::n_e_negative
loft-keyed-ranges/tests/scripts/164-in-place-literal.loft::n_e_vec
loft-keyed-ranges/tests/scripts/164-in-place-literal.loft::n_f_vec
```
