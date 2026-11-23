# @PLN91 — a vector local rebound from a call that takes it, inside a loop: the buffer rotation (H7) (prepared subject)

**Status: DONE** — measured at 0a1a8b43c: its refusal no longer occurs.  Kept as the record of the subject.

**The family:** "a vector local rebound from a call that takes it" (9 sites; 2 programs held by
it alone):
```
H-RootExtent/a-vector-stores-extent-is-its-collection-through-swaps-and-placements.loft:32    for i in 0..5 { v = extend(v, i); }
B-Copy/a-keyed-local-rebound-from-a-call-that-reads-it-keeps-its-records.loft:68              for x in [3, 1, 2] { d = addv(d, x); }
```
The port refuses at el_ex_assign.loft ~4255.  Its comment names two forms it does not carry: a
rebind inside a loop rotates its buffers (`OpPutRef`), and a `&`-linked target is staged through
`__refvec_rhs_N`.  This page takes the LOOP form, which is what both sole sites are.

**The rule:** H7 (`Parser::rotate_loop_retbufs`, src/parser/expressions.rs ~1125–1215; its doc
comment is the spec).  A function returning a heap value CLEARS the caller's return buffer on
entry.  Inside a loop, the target holds the buffer the call site handed last time, so the next
call would clear its own argument.  The site therefore gets a SECOND buffer, and the pair
ping-pongs: the call never writes into the store the live target holds.

## Rust's form, measured on try 71e086496

```
H-RootExtent:32   v: vector<P> = [];  for i in 0..5 { v = extend(v, i); }
  head:  __vdb_2 = null;  __vdb_1 = null;  __ref_2 = null;  __ref_1 = null;  __ref_3 = null;   <- __ref_3, the partner, right after __ref_1
  [31] OpDatabase(__vdb_1, 103);  v: vector<P>["__ref_2"] = OpGetField(__vdb_1, 0, 102);  OpSetInt4(__vdb_1, 0, 0);
  [32] … loop { … {#block
           v: vector<P>["__ref_2"] = n_extend(v, i, __ref_1);       <- the call writes __ref_1
           OpPutRef(__ref_1, __ref_3);                              <- the site's next call writes the OTHER store
           OpPutRef(__ref_3, v) } }                                  <- which parks the live one

B-Copy:68   for x in [3, 1, 2] { d = addv(d, x); }
  … {#block  d: vector<integer>["__ref_1"] = n_addv(d, x#4, __ref_1);  OpPutRef(__ref_1, __ref_2);  OpPutRef(__ref_2, d) }
```
The partner is a fresh `__ref_N` minted per call site.  Its null-init goes right after the
buffer's own null-init (rotate_loop_retbufs: "so the two stay adjacent in `var_order`").  The
two ops are spliced in right after the self-feeding call's statement.  The rewrite runs over
the whole body after it is parsed, on the SECOND pass only (expressions.rs ~1017, just before
`own_fresh_binds`).

## Each form judged

- **The second buffer and the two `OpPutRef`s after each self-feeding call in a loop:** a RULE,
  H7.  Without it, every append but the last is lost silently.
- **The partner's null-init placed beside its buffer:** a RULE of scope order.  Exit frees run in
  reverse declaration order, and both buffers are ordinary work refs freed once.
- **The target's declared dep (`v: …["__ref_2"]` in H-RootExtent, where the loop's call hands
  `__ref_1`):** an internal name (@C168).  Carry it as measured, and measure what sets it: it is
  not the loop's buffer.

## What to code

Port `rotate_loop_retbufs` as a pass over the elaborated body: in a loop, after each statement
`t = f(…, t, …, __ref_N)` whose target is an argument of the call and whose last argument is a
return buffer, splice in `OpPutRef(__ref_N, <partner>); OpPutRef(<partner>, t)`.  Mint the
partner, and put its null-init right after `__ref_N`'s.  Lift the refusal at ~4255 for the loop
case; the `&`-linked staging stays refused.

## Probes (the two sole programs)

```
kr-suite/rules/heap/H-RootExtent/a-vector-stores-extent-is-its-collection-through-swaps-and-placements.loft:32
kr-suite/rules/binding/B-Copy/a-keyed-local-rebound-from-a-call-that-reads-it-keeps-its-records.loft:68
```
