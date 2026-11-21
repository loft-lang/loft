# @PLN91 — a record literal written into a `reference<T>` field re-points it (prepared subject)

**The family:** "a record literal written into a field or an element" (15 programs, 6 held by it
alone).  This page takes its "a place it is built apart for" sub-shape as one sole program has
it: R-InPlaceLiteral/a-literal-assigned-to-a-reference-field-repoints-it (lines 44 and 53).  A
literal is written into a POINTER field (`struct H { r: reference<P>, … }`, `h.r = P { … }`,
`v.hs[0].r = P { … }`).  The place is the pointer read `OpGetDbRef(h, 8)`, which is neither the
plain field read nor the vector element that `literal_into_place` accepts, so the port refuses
at `el_ex_place.loft` ~1244.

**The rule:** `@FR-H-RefSet` / `@FR-R-InPlaceLiteral`: assigning a `reference<T>` field RE-POINTS
it.  The literal is a new record, built apart, and the field is set to name it.  The record the
field named before is left alone (`p1` keeps `x: 1`).

## Rust's form, measured on try 68bb3548b

```
struct P { x: integer, t: text }   struct H { r: reference<P>, k: integer }
  p1 = P { x: 1, t: "one" };  h = H { r: p1, k: 0 };
:44  h.r = P { x: h.r.x + 10, t: "{h.r.t}!" };
  [44] OpSetDbRef(h, 8, {#Object  __ref_p2_1: ref(P) = null;  OpDatabase(__ref_p2_1, 100);
          OpSetInt(__ref_p2_1, 0, OpAddInt(OpGetInt(OpGetDbRef(h, 8), 0), 10));   <- reads the OLD target
          OpSetText(__ref_p2_1, 8, {#Formatted string … OpGetText(OpGetDbRef(h, 8), 8) … "!"});
          __ref_p2_1 })                                                      <- the field re-pointed

:53  v.hs[0].r = P { x: 20, t: "new" };                                     (the pointer field of an element)
  [53] OpSetDbRef(OpGetVector(OpGetField(v, 0, 105), 24, 0), 8, {#Object __ref_p2_1 … OpSetInt(…, 0, 20);
          OpSetText(…, 8, "new"); __ref_p2_1 })
```

The literal is built in its own `__ref_p2_N`, its fields are free to read the field's OLD target
(`OpGetDbRef(owner, pos)`), and `OpSetDbRef(<owner place>, <field pos>, <the record>)` re-points
the field.  Nothing is copied into the old record.

## What to code

At el_ex_place ~1244, before the refusal: when the place is a pointer read (`OpGetDbRef(<owner>,
<pos>)`, where `<owner>` is a local, a field, or an element `OpGetVector(…)`), build the literal
apart (the port's `object_fresh`, as the copy road above it does) and emit
`OpSetDbRef(<owner>, <pos>, <built>)` instead of copying into the place.

## Probe (the sole program; each assert hand-computed)

```
kr-suite/rules/rewrites/R-InPlaceLiteral/a-literal-assigned-to-a-reference-field-repoints-it.loft:44, :53
   (its later cells, an element written WHOLE with a pointer field and an append, are
    `v.hs[i] = H { r: P { … }, … }` and `v.hs += [H { r: P { … } … }]`: check them after)
```

Not on this page: the family's other "a place it is built apart for" sites (164-in-place-literal
:90/:110/:127, Col-Group:201/:425/:430, Col-Insert/1572:170, H-WriteOOB:178/:212).  Their place
shapes are not measured here.
