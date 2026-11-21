# @PLN91 — a field append whose item is built apart and appended whole (prepared subject)

**The subject:** the own-buffer road the porter named as the open half of
field-append-reading-the-field.md.  Two sites:
- sub-case (C), R-Base/158-a-base-survives-a-growth-of-another-store.loft:218, a VECTOR field
  (`m.chunks += [Chunk { cx: m.chunks[0]?.cx + i, hexes: [] }]`);
- R-KeyedEntry/a-keyed-insert-writes-the-entry-its-key-names.loft:83, a KEYED field
  (`c.h += [C { k: 9, n: (c.h[9]?.n ?? 0) + 1 }]`).

In both, the item's value builds a record of its own while reading the field: the `?? default`
of a `?` read makes a fresh `C`, and the item holds a vector literal.  So Rust does not stage
it.  It builds the whole item list apart and appends it whole.

**The rule:** `@FR-E-Asgn-Compound` / `@FR-I-Comp`: every part of the item is read before the
field changes.  An item that cannot be staged as plain values is built in a vector of its own,
then appended.  This is `parse_vector`'s own-buffer arm (loft#2397).

## Rust's forms, measured on try 5d712c497

```
VECTOR field   R-Base/158-a-base-survives-…:218   m.chunks += [Chunk { cx: m.chunks[0]?.cx + i, hexes: [] }];
  OpAppendVector(OpGetField(m, 8, 106), {#Vector  OpDatabase(__vdb_1, 115);
       _vec_1 = OpGetField(__vdb_1, 0, 106);  OpSetInt4(__vdb_1, 0, 0);  OpPreAllocVector(_vec_1, 1, 16);
       _elm_2: ref(Chunk)["_vec_1"] = OpNewRecord(_vec_1, 106, 65535);  OpSetInt4(_elm_2, 8, 0);
       OpSetInt(_elm_2, 0, OpAddInt(OpGetInt({#ncc … OpGetVectorNullable(OpGetField(m, 8, 106), 16, 0) … else {#Object …}}, …), i));
       OpFinishRecord(_vec_1, _elm_2, 106, 65535);  _vec_1 }, <tp>)

KEYED field    R-KeyedEntry/…-its-key-names.loft:83   c.h += [C { k: 9, n: (c.h[9]?.n ?? 0) + 1 }];
  OpFillKeyed(c, {#Vector: vector<ref(C)["c"]>["__vdb_1"]
       OpDatabase(__vdb_1, 116);  _vec_1 = OpGetField(__vdb_1, 0, 117);  OpSetInt4(__vdb_1, 0, 0);
       OpPreAllocVector(_vec_1, 1, 4);
       _elm_6: ref(C)["_vec_1"] = OpNewRecord(_vec_1, 117, 65535);
       OpSetInt(_elm_6, 0, 9);
       OpSetInt(_elm_6, 8, OpAddInt({#ncc … OpGetRecord(OpGetField(c, 0, 105), 105, 1, 9) … else {#Object __ref_p2_1 …} …}, 1));
       OpFinishRecord(_vec_1, _elm_6, 117, 65535);  _vec_1 },
     105, 104, 0)                                         <- OpFillKeyed(parent, src, keyed tp, parent tp, field)
```

`OpFillKeyed(parent, src, tp, parent_tp, field)` (default/01_code.loft ~2263) fills the keyed
collection with a deep copy of each record of the plain vector `src`, each placed by its key.

## What to code

In the field-append road (el_ex_place ~1830–1885), where an item's value builds a record (a
`?? <record>` default, or a vector or record literal inside the item) and so cannot be staged:
- build the whole item list in its own `#Vector` (`__vdb_N` / `_vec_N`, elements made by
  `OpNewRecord(_vec_N, <elem tp>, 65535)`);
- then, for a vector field, `OpAppendVector(<field>, <that vector>, <tp>)`;
- for a keyed field, `OpFillKeyed(<owner>, <that vector>, <keyed tp>, <owner tp>, <field pos>)`.

The port already emits `OpFillKeyed` for a struct literal's keyed field given a vector value
(el_ex_place ~3645), so the op is known to its writer.

## Probes (each with hand-computed answers)

```
kr-suite/rules/rewrites/R-Base/158-a-base-survives-a-growth-of-another-store.loft:218            (sole)
kr-suite/rules/rewrites/R-KeyedEntry/a-keyed-insert-writes-the-entry-its-key-names.loft:83       (sole; also :88 after A)
```
