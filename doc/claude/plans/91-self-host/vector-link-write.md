# @PLN91 — a vector written through a `&` link to a vector local (prepared subject)

**The subject:** `e: vector<integer> = [1]; pe = &e; pe = [2, 2]`.  A whole vector written
through a link to a vector LOCAL.  The porter met it as text-link's last site (B-Ref-Intro/1372:82,
`pe = &e` on `e: vector<integer>?`) and kept it refused on purpose, because the write differs
even for a DENSE vector.  Probe: [probes/vector-link-write.loft](probes/vector-link-write.loft),
four cases with hand-computed answers, all of which hold on Rust's `--interpret` and `--native-c`
with `LOFT_POISON=1` and the leak check.  The port's verdicts on try 5d712c497: `dense_literal`
DIFFER, `dense_local` DIFFER, `dense_append` agree, and `nullable_literal` REFUSED ("a `&`
reference to something other than a scalar or a vector place: TyVector").

**The rule:** `@FR-B-Ref-Alias`: a link writes THROUGH to its source.  `pe = [2, 2]` replaces the
contents of `e`'s store, so `e` reads `[2, 2]` after it.  The link owns nothing and is never a
store of its own.

## Rust's form, measured on try 5d712c497

```
fn dense_literal() -> integer { e: vector<integer> = [1]; pe = &e; pe = [2, 2]; len(e) * 10 + e[1] }   (22)
  e: vector<integer> mutable nullable hidden work_buffer        <- a linked vector local becomes a WORK BUFFER
  __ref_1: vector<integer>["__vdb_1"] = null;  __vdb_2 = null;  __vdb_1 = null;
  OpPutRef(__ref_1, e);                                         <- its entry witness
  [2] if OpRefIsNull(e) { OpFreeRefIfDistinct(e, __ref_1)       <- e's first bind: minted when null,
                          OpDatabase(__vdb_1, 20)                  cleared in place when not
                          e: vector<integer>["__vdb_1"] = OpGetField(__vdb_1, 0, 7)
                          OpSetInt4(__vdb_1, 0, 0) }
      else { t_6vector_clear(e) };
      OpPreAllocVector(e, 1, 8);  OpPushInt(e, 1);
  [2] pe: vector<integer>["__vdb_2"] = e;                       <- the link
      OpClearVector(pe);                                         <- the write: in place, through the link
      OpPreAllocVector(pe, 2, 8);  OpPushInt(pe, 2);  OpPushInt(pe, 2);

fn dense_local() -> …  w: vector<integer> = [3, 4, 5]; pe = &e; pe = w;      (35)
  [3] pe: vector<integer>["w"] = e;
      OpClearVector(pe);
      OpAppendVector(pe, __refvec_rhs_1, 0);                     <- a local source staged in __refvec_rhs_1, appended
```

**The port's form** for `dense_literal` mints a NEW store for the link instead: `OpDatabase(__vdb_2,
20); pe = OpGetField(__vdb_2, 0, 7); OpSetInt4(__vdb_2, 0, 0)`.  `e` then never sees the write
(and has no `work_buffer` prelude).

Two details to copy as they stand: the link's TYPE takes the written value's deps (`["__vdb_2"]`
after the literal, `["w"]` after the local), and `e`'s `work_buffer` prelude (the `OpPutRef`
entry witness, then null-or-clear at the first bind).

## What to code

1. A vector local named by a `&` link is a `work_buffer` (hidden, with the entry witness and the
   null-or-clear first bind), as Rust marks it.
2. A whole-vector write through the link: `OpClearVector(<link>)`, then the literal's pushes or,
   for a vector source, `OpAppendVector(<link>, <staged source>, 0)`.  Never a fresh store.
3. Then admit the NULLABLE vector link (`e: vector<integer>?`, 1372:82), which today is refused
   by name.

## Probes

```
doc/claude/plans/91-self-host/probes/vector-link-write.loft   dense_literal 22, dense_local 35, dense_append 27, nullable_literal 3
kr-suite/rules/binding/B-Ref-Intro/1372-a-reference-links-a-nullable-slot.loft:82   pe = &e (nullable vector)
```
