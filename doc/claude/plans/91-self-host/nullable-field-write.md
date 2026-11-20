# @PLN91 — `null`, or a variant literal, written into a nullable field (prepared subject)

**The family:** "an assignment to a nullable place" (31 programs at try 775c31471, 3 held by it
alone).  This page takes two shapes, which cover 2 of the 3 sole programs: a NULLABLE
STRUCT-ENUM field given `null` or a variant literal (`r.cond = null`, `r.cond = NdCall { … }`
with `cond: Nd?`), and a NULLABLE VECTOR field given `null` (`o.xs = null` with
`xs: vector<integer>?`).  The port refuses at `el_ex_place.loft` ~726, after its nullable-place
arms for a text field (`OpSetTextReplace` / `OpConvTextFromNull`) and a record element
(`record_place_write`).

**The rule:** `(L-Null)`: a nullable field holds its absence in its own bytes.  Writing `null`
releases what the field held and stores the sentinel.  A value is written as into the dense
field.

## Rust's form, measured on try 775c31471

```
F-Ret/2443-a-record-copied-out-of-a-kept-buffer-releases-it.loft   struct VArm { cond: Nd?, … }   Nd: a struct enum
  r.cond = null;                        [33] OpSetInt4(r, 0, 0);        <- tag 0 at the field: no variant
  r.cond = NdCall { d: k, args: [NdInt { n: k * 10 }] };
     [36] OpCopyRecord({#Object __ref_p2_1 … OpSetEnum(__ref_p2_1, 0, 3) … __ref_p2_1 },
                       OpGetField(r, 0, 100), 100);                  <- built apart, copied into the field place

a-pooled-buffers-vectors-are-emptied-in-place.loft:66   struct O { h: integer, xs: vector<integer>? }
  if k == 1 { o.xs = null; }
     OpClearVector(OpGetField(o, 8, 7));                             <- release the elements
     OpSetInt4(o, 8, -1);                                            <- the vector field's null sentinel
```

The two sentinels differ by kind: a struct enum's absent value is tag 0 (`OpSetInt4(rec, pos,
0)`), and a vector field's is the header `-1`.

## What to code

At el_ex_place ~726, add two arms beside the text one:
1. A nullable struct-enum field: `null` writes `OpSetInt4(owner, pos, 0)`; a variant literal is
   built apart (the port's record-literal road, `__ref_p2_N`) and copied with
   `OpCopyRecord(<literal>, OpGetField(owner, pos, enum_tp), enum_tp)`.
2. A nullable vector field given `null`: `OpClearVector(OpGetField(owner, pos, vec_tp))`, then
   `OpSetInt4(owner, pos, -1)`.

## Probes (the two sole programs, each with hand-computed answers)

```
kr-suite/rules/calls/F-Ret/2443-a-record-copied-out-of-a-kept-buffer-releases-it.loft:33, :36   r.cond = null / = NdCall {…}
kr-try/tests/scripts/a-pooled-buffers-vectors-are-emptied-in-place.loft:66                      o.xs = null
```

Not on this page: the third sole program's `os[k].f = It { … }` (reported as "an element of
TyReference given TyReference", operands-reduce-left-to-right…:45), and the family's other
field kinds given a value of a converted type.
