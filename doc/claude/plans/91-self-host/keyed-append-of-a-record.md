# @PLN91 — a keyed append of a record that is not a literal (prepared subject)

**The family:** "a keyed collection item that is not a literal of its record", 9 programs at
try c268bf4b9, and the ONLY refusal in 3 of them. A `hash` / `sorted` / `index` append whose item
is a call's answer (`h += [mk(i)]`, `frag += [make(g)]`, the store_* scripts), a local
(`b += [big]`), or a view from another collection (`for e in rows { h += [e] }`).

**Judged right** on the interpreter (LOFT_POISON), --native-c (leak check) and --native, with 300
passes. Expected values computed by hand:
- call items, keys 0..3, then `mk(2)` again → `len 4`, `h[3].v 30`, `h[2].t "t2"`: a duplicate
  key REPLACES, `@FR-Col-Insert`;
- a local `r` appended, then `r.v = 99` → the collection keeps 50: the append COPIES;
- views of `[mk(1), mk(7), mk(1)]` into an index → `len 2`, `ix[7].t "t7"`, source untouched.

## Rust's form: the vector append's, into the keyed collection

```
call item   h += [mk(i)]:
    _elm_1: ref(KV)["h"] = OpNewRecord(h, 103, 65535);
    __ref_p2_1: ref(KV) = n_mk(i, _elm_1);                  <- the element IS the callee's return buffer
    if OpDistinctStore(__ref_p2_1, _elm_1) OpCopyRecord(__ref_p2_1, _elm_1, 49252) else null;   <- 49252 = KV(100) | COPY_FRESH_DEST 0x4000 | free-source 0x8000
    OpFinishRecord(h, _elm_1, 103, 65535);                   <- the keyed insert (replaces a held key)
local item  s += [r]:          _elm_1: ref(KV)["s"] = OpNewRecord(s, 105, 65535); OpCopyRecord(r, _elm_1, 16484); OpFinishRecord(s, _elm_1, 105, 65535);
view item   ix += [e]:         _elm_2: ref(KV)["ix"] = OpNewRecord(ix, 104, 65535); OpCopyRecord(e, _elm_2, 16484); OpFinishRecord(ix, _elm_2, 104, 65535);
                                (16484 = KV | COPY_FRESH_DEST, keys.rs:1361; the source is not freed)
```

The call form also mints one `__ref_p2_N: ref(KV) = null` per call site in the head, numbered in
source order and listed in reverse (`__ref_p2_4 … __ref_p2_1`). The `_elm_N` counter is per
collection append site, as for vectors.

**What to code:** the vector append of a non-literal record item, which the port already carries
(the `src = [mk(1), mk(7), mk(1)]` vector literal in the same probe uses it, byte for byte), taken
for a `hash` / `sorted` / `index` / `spatial` / `trie` destination. Use the collection's own
database type in `OpNewRecord` / `OpFinishRecord`; the element deps name the collection. The
duplicate-key replacement lives in `OpFinishRecord`, not in the IR.

## The sites (try c268bf4b9)

```
kr-suite/rules/binding/B-Copy/1612-a-coalesce-chain-copies-the-operand-it-chooses.loft::n_c28
kr-suite/rules/collections/Col-Group/2563-a-keyed-value-into-a-group-member-reaches-every-member.loft::n_cg2563_keep
kr-suite/rules/types/N-Match/a-null-arm-matches-a-null-subject-of-any-kind.loft::n_test_a_nullable_keyed_collection
kr-try/tests/scripts/136-persist-copy-morton-order.loft::n_main
kr-try/tests/scripts/802-lazy-refusal-visible.loft::n_main
kr-try/tests/scripts/a-plain-vector-append-without-a-header-lands-in-its-slot.loft::n_b7
kr-try/tests/scripts/store_compact_b2.loft::n_main
kr-try/tests/scripts/store_digest_b0.loft::n_main
kr-try/tests/scripts/store_rebuild_b1.loft::n_main
```
