# @PLN91 — a field append whose item reads the field (prepared subject)

**The family:** "a field append whose item reads the field" (5 programs at try 5953e2c2b, 4 of
them held by it alone).  `o.f += [R { … }]` where an item's value reads `o.f` itself
(`len(t.rows)`, `m.chunks[0]?.cx`, `hd.h[3].a`) or hands `o` to a call.  The port already has a
staged arm for this (el_ex_place ~1830–1885).  It refuses three sub-cases by name, at ~1868
and ~1880.  Each has a measured Rust form below.

**The rule:** `@FR-E-Asgn-Compound` / `@FR-R-InPlaceLiteral`'s staging clause: every value an
item reads is read BEFORE the element is made in the field.  Reading after would see the
element half-made, or a store the append has grown.  Rust stages, or builds the item apart and
appends it.

## The three sub-cases, Rust's form measured on try 5953e2c2b

```
A. "through a call that may write it"   158-mint-window.loft:106 (n_m5); R-KeyedEntry/a-keyed-insert-writes-the-entry-its-key-names.loft:88
   w.xs += [P { x: m5_side(w, i), y: i }];
     OpDatabase(__vdb_1, 111);  _build_src_1: vector<ref(P)>["__vdb_1"] = OpGetField(__vdb_1, 0, 105);
     OpSetInt4(__vdb_1, 0, 0);
     OpAppendVector(_build_src_1, OpGetField(w, 0, 105), 101);   <- the field snapshotted first
     __ref_1: integer = n_m5_side(w, i);                         <- every value staged, the call too
     __ref_2: integer = i;
     _elm_1: ref(P)["w"] = OpNewRecord(w, 108, 0);
     OpSetInt(_elm_1, 0, __ref_1);  OpSetInt(_elm_1, 8, __ref_2);
     OpFinishRecord(w, _elm_1, 108, 0);

B. "of this element type"   a-keyed-insert-stages-its-values-before-the-entry-is-found.loft:74;
                            R-KeyedEntry/…-its-key-names.loft:83; 158-keyed-fast-paths.loft:334
   hd.h += [P { k: 3, a: (hd.h[3].a ?? 0) + 1, b: hd.h[3].a ?? 0 }];       (a KEYED field)
     __ref_5: integer = OpAddInt({#ncc … OpGetRecord(OpGetField(hd, 0, 105), 105, 1, 3) …}, 1);
     __ref_6: integer = {#ncc … };                               <- staged, no vector built at all
     _elm_11: ref(P)["hd"] = OpNewRecord(hd, 104, 0);
     OpSetInt(_elm_11, 0, 3);  OpSetInt(_elm_11, 8, __ref_5);  OpSetInt(_elm_11, 16, __ref_6);
     OpFinishRecord(hd, _elm_11, 104, 0);

C. "beside a value built in place"   R-Base/158-a-base-survives-a-growth-of-another-store.loft:218
   m.chunks += [Chunk { cx: m.chunks[0]?.cx + i, hexes: [] }];            (the item holds a vector literal)
     OpAppendVector(OpGetField(m, 8, 106), {#Vector  OpDatabase(__vdb_1, 115);
          _vec_1 = OpGetField(__vdb_1, 0, 106);  OpSetInt4(__vdb_1, 0, 0);  OpPreAllocVector(_vec_1, 1, 16);
          _elm_2: ref(Chunk)["_vec_1"] = OpNewRecord(_vec_1, 106, 65535);  OpSetInt4(_elm_2, 8, 0);
          OpSetInt(_elm_2, 0, OpAddInt(OpGetInt({#ncc … OpGetVectorNullable(OpGetField(m, 8, 106), 16, 0) …}, …)));
          … }, …)                                                <- built apart in its own vector, appended whole
```

## What to code

- **B** is the port's staged road, minus the refusal.  The refusal is the `vector_rows` guard
  ("Asked only where the element's vector type is declared already").  For a KEYED field Rust
  builds no vector: it stages the values and makes the record through the owner.  So for a
  keyed field the guard does not apply.
- **A** adds two steps to B: the field is snapshotted into `_build_src_N` first, through a
  fresh `__vdb_N` filled by `OpAppendVector(_build_src_N, <field>, <row>)`.  A call's answer is
  staged like any value.
- **C** is the own-buffer arm (`parse_vector`'s, loft#2397) the port already takes for an item
  that reads the record under another name (el_ex_place ~1692, `alias_reads`).  Take it also
  when the item holds a value built in place, such as a vector literal field.

## Probes (the sole programs, each with hand-computed answers)

```
A  kr-try/tests/scripts/158-mint-window.loft:106                                       n_m5
B  kr-try/tests/scripts/a-keyed-insert-stages-its-values-before-the-entry-is-found.loft:74
C  kr-suite/rules/rewrites/R-Base/158-a-base-survives-a-growth-of-another-store.loft:218   n_c17
A+B kr-suite/rules/rewrites/R-KeyedEntry/a-keyed-insert-writes-the-entry-its-key-names.loft:83, :88
B (not sole: two other families also refuse it) kr-try/tests/scripts/158-keyed-fast-paths.loft:334
```
