# @PLN91 — `+=` into a field that is a keyed collection or a file (prepared subject)

**The family:** "an operator with no builtin candidate" (21 programs at try 4bff1c6cc, 6 held
by it alone).  The family is mixed.  This page takes its most coherent part: `+=` whose left
side is a FIELD holding a keyed collection (`hash`, `sorted`, …) or a `File`.  That covers
4 of the 6 sole programs.  The port refuses at `el_ex_base.loft` ~2608 with `+ on TyHash,
TyReference`, `+ on TySorted, TyReference` and `+ on TyReference, TyText` (a File is a
reference).

**The rule:** `@FR-R-InPlaceLiteral` and the file write: `c.f += R { … }` builds the record
in the collection's store, as an element of a vector field is built; `b.h += "ab"` writes the
text to the file the field holds, as `h += "ab"` does for a file local.  The left side is a
PLACE, read where it lies, never copied.

## Rust's form, measured on try 4bff1c6cc

```
hash field += record literal   (Col-Bucket/135-reserve-hash.loft:94)
  for i in 0..500 { box.data += RRec { k: i, v: i * 4 }; }
     _elm_8: ref(RRec)["box"] = OpNewRecord(box, 103, 0);      <- the OWNER and the field's position
     OpSetInt(_elm_8, 0, i);  OpSetInt(_elm_8, 8, i * 4);
     OpFinishRecord(box, _elm_8, 103, 0);

sorted field += record literal   (store_load_field_refusal.loft:37)
  s.items += FRec { id: 10, n: 100 };
     _elm_2: ref(FRec)["s"] = OpNewRecord(s, 103, 0);
     OpSetInt(_elm_2, 0, 10);  OpSetInt(_elm_2, 8, 100);
     OpFinishRecord(s, _elm_2, 103, 0);

file field += text   (H-Handle/a-file-field-takes-an-append-as-a-file-local-does.loft:30)
  b.h += "ab";
     _wf_1: text = "ab";
     OpWriteFile(OpGetField(b, 0, 48), OpCreateStack(_wf_1), 5);
  (a file LOCAL `h += …` is the same with `h` for the place: OpWriteFile(h, OpCreateStack(_wf_3), 5))
```

The record append names the field's OWNER (`box`, `s`) and the field's position (`0`), not
a read of the field.  The element temp's deps are the owner.

## What to code

At the refusal (el_ex_base ~2608), route `+=` on a field before operator lookup:
1. A keyed-collection field given a record literal: the element road the port already builds
   for a vector element (`OpNewRecord(owner, tp, pos)`, the field writes, `OpFinishRecord(owner,
   elm, tp, pos)`; el_ex_call ~752–907 has the template form).
2. A `File` field given a text: the file-local road (el_ex_vector ~32–74, `_wf_N` and
   `OpWriteFile`), with the place `OpGetField(owner, pos, file_tp)` as the target.

## Probes (the four sole programs, each with hand-computed answers)

```
kr-suite/rules/collections/Col-Bucket/135-reserve-hash.loft:94                       hash field += record
kr-try/tests/scripts/store_load_field_refusal.loft:37                                sorted field += record
kr-suite/rules/heap/H-Handle/a-file-field-takes-an-append-as-a-file-local-does.loft:30    file field += text
kr-suite/rules/heap/H-Handle/a-file-field-not-yet-written-holds-no-descriptor.loft:32, :46  file field += text
```

Not on this page: 1159's `+=` of a VECTOR into each keyed kind (`hash`, `sorted`, `index`, `trie`,
`radix`), 505's hash `+=` in a capture, and the family's other operators (`==` and `-` on mixed
types), which are separate refusals.
