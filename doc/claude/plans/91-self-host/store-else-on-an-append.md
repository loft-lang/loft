# @PLN91 — a store `else` on an append, `xs += [v] else { … }` (prepared subject)

**Status: DONE** — measured at 0a1a8b43c: its refusal no longer occurs.  Kept as the record of the subject.

**The family:** "a store `else` on an append" (4 sites, 2 programs held by it alone):
```
H-Write-Else/2549-an-append-is-a-store-whose-place-is-its-collection.loft:30, :54   d.xs += [4] else { out += "X" };  ds[3].hs += [E { k: 6, v: 60 }] else { out += "A" };
a-discarded-text-write-is-discarded-whole.loft:31                               h.hs += [E { k: 6, v: 60 }, E { k: 7, v: 70 }] else { out += "H" };
```
The port's `store_else` (el_ex_object.loft ~2277) carries a write whose PLACE may be missing,
and a narrow write whose VALUE may not fit.  It refuses any statement with an append step
(~2289): its comment says an append's target, the collection, is not carried yet.

**The rule:** `@FR-H-Write-Else` with `@FR-H-WriteNull` / `@FR-H-WriteLocked` (loft#2549): an
APPEND is a store too.  Its place is the COLLECTION it claims in.  An absent or locked collection
answers the reported null, so the claim, push or filing lands nowhere, and the arm runs exactly
then.

## Rust's form

src/parser/store_else.rs: `store_shape` ~214 and `is_append_op` ~281.  An append step is a call
whose op starts with `OpPush`, or is `OpNewRecord`, `OpFinishRecord`, `OpAppendVector`,
`OpAppendCopy`, `n_keyed_entry_ints` or `n_keyed_entry_text`.  It is also a `Set` of an element
to such a call (`_elm = OpNewRecord(…)`).  Its target is ARGUMENT 0.  The field writes into the
claimed element (`OpSet…(_elm, …)`) are NOT a second target: they skip through the null element.
Every append step of the statement must name the same target, as for any store.  The target
is then bound once in `_store_target_N`, as `parse_store_else` ~117 does for a place.

Measured on try cf9d865fe (2549 :30, :31):
```
:30  d.xs += [4] else { out += "X" };
  [30] _store_target_1: ref(D)["d"] = OpGetField(d, 0, 7);                       <- the COLLECTION, bound once
       if if OpNot(OpConvBoolFromRef(_store_target_1)) true else n_get_store_lock(_store_target_1)
         _store_target_1 = OpNullReported() else null;                            <- absent or locked → the reported null
       OpPushInt(_store_target_1, 4);                                             <- the push through it
       if OpNot(OpConvBoolFromRef(_store_target_1)) {#block  OpAppendText(out, "X") } else null;   <- the arm

:31  d.hs += [E { k: 6, v: 60 }] else { out += "H" };
  [31] _store_target_2: ref(D)["d"] = OpGetField(d, 0, 103);   (the same guard)
       _elm_3: ref(E)["d"] = OpNewRecord(_store_target_2, 103, 1);                <- the claim through the target…
       OpSetInt(_elm_3, 0, 6);  OpSetInt(_elm_3, 8, 60);                          <- …its field writes unchanged
       OpFinishRecord(_store_target_2, _elm_3, 103, 1);
       if OpNot(OpConvBoolFromRef(_store_target_2)) {#block  OpAppendText(out, "H") } else null;
```

## Each form judged

- **The collection as the store's place, bound once, the guard and the arm as for any store:** a
  RULE (`@FR-H-Write-Else`, loft#2549).
- **The claimed element's field writes left as they are:** a RULE (`@FR-H-WriteNull`).  Through a
  null collection the claim answers a null element, and its writes skip.
- **`_store_target_N` typed `ref(D)["d"]` for a vector field** (`store_target_type` over the
  target): an internal type name (@C168).  Carry it as measured.

## What to code

In `store_else`, treat an append step as a store step: its target is argument 0 of the append
op (or of the call a `Set` of the element holds).  Skip the field writes into an element that
a claim set.  Every target must be the same, then go through the existing
`_store_target_N` road.  Lift the refusal at ~2289.

## Probes (the two sole programs)

```
kr-suite/rules/heap/H-Write-Else/2549-an-append-is-a-store-whose-place-is-its-collection.loft:30–34, :54
kr-suite/rules/heap/H-WriteLocked-Whole/a-discarded-text-write-is-discarded-whole.loft:31    (a keyed filing of two records)
```
