# @PLN91 — a `&`-linked vector rebound with `=`: refilled in place, through a staging store (prepared subject)

**Status: DONE** — measured at 0a1a8b43c: H-RootExtent a-vector-stores-extent… agrees in every layer.  Kept as the record of the subject.

**The subject:** the other half of "a vector local rebound from a call that takes it", which
loop-rebind-rotates-buffers.md left refused.  H-RootExtent/a-vector-stores-extent-is-its-collection-
through-swaps-and-placements now stops at :35 (after the loop half, c3f3e1328):
```
:34   l = &v;
:35   v = extend(v, 9);
:36   assert(len(l) == 6 && l[5].t == "e9" && l[0].t == "e0", "a link sees the exchanged store");
```
**The rule:** `@FR-B-Ref-Write` / `@FR-B-Ref-Uniform` (formal/binding.md; Rust's comment also names a deviation entry D-bind-43, no longer in the
working chapter): a vector
that a `&` link shares is written THROUGH by `=`.  The store keeps its identity and gets the new
contents, so the link sees them.  It is never rebound to a copy (loft#1836).  The clear runs
BEFORE the copy, so a right-hand side that may read the vector is materialised first
(`@FR-O-Proxy`).

## Rust's form

`Parser` in src/parser/expressions.rs ~13370–13470 (the `&vector` write).  For `=` on a
`RefVar(Vector)` or a linked plain local (`amp_vector_locals`), it is a three-way split:
```
v = v                         → nothing (an empty Insert): the store replaced by itself
v = <a dep-free local w>      → OpClearVector(v); OpAppendVector(v, w, <row>)
v = <anything else>           → a staging store, then the refill:
```
Measured on try 709915c85 (H-RootExtent :35):
```
  [35] OpDatabase(__vdb_2, 103);
       __refvec_rhs_1: vector<P>["__vdb_2"] = OpGetField(__vdb_2, 0, 102);  OpSetInt4(__vdb_2, 0, 0);   <- vector_db for the stage
       OpAppendVector(__refvec_rhs_1, n_extend(v, 9, __ref_2), 100);                                    <- the call copied into the stage
       OpClearVector(v);                                                                                <- then v cleared…
       OpAppendVector(v, __refvec_rhs_1, 100);                                                          <- …and refilled
```
The stage is `create_unique("_refvec_rhs", <the vector, dep-free>)` (so `__refvec_rhs_N`).  The
row is `append_elem_tp(elm)`, which is narrow-aware (@P314).

## Each form judged

- **The refill in place (clear, then append) for a linked vector:** a RULE (`@FR-B-Ref-Write`).
  The link must see the new contents.
- **The staging store whenever the right-hand side is not a dep-free local:** a RULE of order.
  The clear would otherwise wipe what the right-hand side reads (`v = tail(v)`, a call taking
  `v`).
- **`v = v` as nothing:** a RULE.  The clear would empty the source before the append.

## What to code

At el_ex_assign ~4255 (the `&`-linked half of the refusal), for `=` on a vector in
`vars.amp_vectors`, apply the three-way split above.  The staging road is `__refvec_rhs_N` with
its own `vector_db` store, the right-hand side appended in, then `OpClearVector(v)` and
`OpAppendVector(v, __refvec_rhs_N, <row>)`.  `+=` on a linked vector is the plain
`OpAppendVector(v, <rhs>, <row>)` and is probably carried already.

## Probe

```
kr-suite/rules/heap/H-RootExtent/a-vector-stores-extent-is-its-collection-through-swaps-and-placements.loft:35
```
