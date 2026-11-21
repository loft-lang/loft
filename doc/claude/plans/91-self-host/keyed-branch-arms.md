# @PLN91 — a keyed-collection result from a branch: a vector-literal arm is built as the hash (prepared subject)

**The family:** "`if` arms of different types" (32 programs at try 0ca590df5, 2 held by it
alone).  This page takes the other sole program, F-Ret/2562-a-returned-keyed-field-of-a-local-
record-is-its-own-store.  Its `fr2562_arm` answers a `hash` from a branch whose arms are a hash
FIELD of a local and a VECTOR LITERAL:
```
fn fr2562_arm(n: integer) -> hash<Fr2562E[fr_k]> {
  b = Fr2562Src {};  b.fr_h += [Fr2562E { fr_k: 2, fr_v: n }];
  if n > 5 { b.fr_h } else { [Fr2562E { fr_k: 2, fr_v: 1 }] } }
```
The port refuses it ("`if` arms of different types: TyHash, TyVector").  The other seven
functions the gate lists as DIFFER in this file differ only in their definition NUMBER (`#887`
against `#889`), a knock-on of the refused function, not a lowering difference.

**The rule:** `@FR-I-Lit` / `@FR-F-Ret`: a branch is checked against its expected type, and the
expected type reaches every arm.  A vector literal checked against a keyed collection BUILDS
that collection.  A keyed field of a local returned is handed out as a store of its own.

## Rust's form, measured on try 4ba39108e

```
  __kvb_1: hash<Fr2562E,["fr_k"]> = null;                               <- head
  … b built, its hash filled through OpNewRecord(b, 103, 0) / OpFinishRecord …
  [46] if OpLtInt(5, n) {#block: hash<…>["b"]
          { !! INSERT  ___blk_val_1: hash<…> = null
                       OpReplaceKeyed(OpGetField(b, 0, 102), ___blk_val_1, 102)     <- the field handed out
                       ___blk_val_1 } }
       else {#block: hash<…>
          {#Vector: hash<…>                                                <- the literal AS the hash
            __kvb_1 = null;  OpDatabase(__kvb_1, 102);
            _elm_2: ref(Fr2562E)["__kvb_1"] = OpNewRecord(__kvb_1, 102, 65535);
            OpSetInt(_elm_2, 0, 2);  OpSetInt(_elm_2, 8, 1);
            OpFinishRecord(__kvb_1, _elm_2, 102, 65535);
            __kvb_1 } }
```

The field arm is the hand-over the port already makes at a plain tail (`fr2562_tail`, which
agrees).  What is missing is the expected type reaching the arms.  With `hash<…>` expected,
the literal arm is built in a keyed buffer `__kvb_N` (`OpDatabase(__kvb_N, <hash tp>)`, one
`OpNewRecord(__kvb_N, <hash tp>, 65535)` per element), typed `hash<…>`, and the join is a hash.

## What to code

In the branch's elaboration, hand the expected type (the function's result, here a keyed
collection) to each arm.  A vector literal arm checked against a keyed collection becomes the
`#Vector`-named block that builds the collection in `__kvb_N`.  A keyed-field arm takes the
tail's hand-over (`OpReplaceKeyed` into `___blk_val_N`).

**Check first:** the port may not build a keyed literal in a `__kvb_N` anywhere yet.  Its only
mentions of `__kvb` (el_ex_assign ~1369, ~3466) note that route as not carried and refuse it by
name.  If so, the `__kvb_N` build is the larger half of this subject, and `x: hash<…> = [ … ]`
is the place to start it.

## Probe (the sole program; `main` asserts each function's answer)

```
kr-suite/rules/calls/F-Ret/2562-a-returned-keyed-field-of-a-local-record-is-its-own-store.loft:46   n_fr2562_arm
```
