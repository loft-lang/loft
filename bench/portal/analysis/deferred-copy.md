# The deferred copy — a move from a dead view, guarded by an index compare

`(R-DeferredCopy)` in `doc/claude/formal/rewrites.md` is the rule; this file is its design:
the form the owner chose, why it is sound, the code points, the cells that guard it, and the
steps that build it.  C only (`src/c/body/deferred.rs`, `loft_rt.h`).

## The shape it is for

The self-host interpreter (`selfhost/src/run.loft` on `91-self-host`), once per interpreted call:

```loft
callee_prog = progs[callee] ?? panic("lowered above");  // a VIEW of the element (B-View)
...
prog = callee_prog;              // a whole-value bind: (B-Copy) owes a copy of a Lowering
...                              // next pass round the loop:
progs[k] = lower::lower(b, k);   // the only write that can change what `prog` reads
op = prog.ops[pc] ?? panic(…);
```

Taking the copy at the bind costs ~46 % of the run (`loft_copy_claims`).  The copy is only
observable where something changes the place `callee_prog` views, so it can be taken there.

## The decided form (owner, 2026-10-28)

**3c — the bind is a MOVE.**  `x = s` gives `x` the reference `s` holds, instead of a copy.
When `s` is a VIEW (`callee_prog` views `progs[callee]`), `x` then views that place too, and
the copy (B-Copy) owes is still owed.  It is taken only if the place changes while `x` still
reads it, and 3b says when that is.  For a view source, being dead after the bind is not
needed: a later read of `s` is a read of the place, which is what `s` means, and the guard
protects `x` alone.  The liveness condition belongs to an OWNER source (§ Not in this design),
where it is what makes a move possible at all.  In the interpreter `callee_prog` is dead after
the bind either way.

**3b — at a write the view lives across, compare the indexes.**  A write rooted at element `k`
of the vector `V` that `x` views an element of emits:

```c
if (loft_dcv<x> && loft_dc_is_elem(<x>, <V>, <size>, <k>)) { <materialise x> }
<the write>
```

`loft_dc_is_elem` resolves `k` exactly as `loft_vec_get` does (a negative index counts from the
end). It then compares the element's reference with `x`'s.  The compare is on POSITIONS, not
on the raw integers, because `progs[-1]` can be the element `x` views.  A write that cannot be
named this way gets the **flag fallback**: `if (loft_dcv<x>) { <materialise x> }`, the form
built first.  The owner accepted the fallback: it is one static flag per local, not a run-time
registry of views (3a, declined).

**The flag** `loft_dcv<x>` is up exactly while `x` holds a reference taken by a deferred bind
and no copy has been taken since.  Any other bind of `x` (a view bind such as
`prog = progs[def] ?? …`, or a copy) lowers it.  The guard needs it: when `x` views
`progs[def]` through an ordinary view bind, a write to `progs[def]` must reach `x`
((B-Disturb): overwriting a place does not end a view of it).

## Why the index compare is sound

1. **Single ownership.**  Every record reachable from `V[j]` is owned through `V[j]` and
   through nothing else.  A write rooted at `V[k]` changes the bytes of element `k`. It may
   also release or claim records owned through `V[k]`.  It changes no record that a view of
   `V[j]`, `j ≠ k`, can reach.
2. **`x` views a whole element.**  The rule admits 3b guards for `x` only when every deferred
   bind's source is bound as `V[e]` alone (a `??` null discharge allowed, no field projection).
   Then `x`'s reference IS the element's: same store, record and position ⟺ same element.  A
   view of an inline sub-record, or of a record reached through a vector field, would make the
   position compare unsound, so such a source keeps the flag fallback.
3. **The element has not moved.**  A growth or removal of `V`, or a rebind of `V`, is a write
   no index names, so it takes the flag fallback.  The copy is taken before it and the flag is
   down by the time any later indexed write is tested.  So whenever the compare runs with the
   flag up, `V`'s element positions are the ones `x` was bound to.
4. **What a write is.**  A write is an IR call that `call_writes_store` answers yes for and
   whose store operands are not proven apart (`dc_writes_source`).  A statement is an indexed
   write when the operand it WRITES is a path rooted at `OpGetVector(V, size, k)`, and nothing
   else the statement writes is unproven.  `V` is a pure path (`vector_path`); `k` is a `Var`
   or an integer literal, so it is evaluated twice.  The written operand is:
   - the first operand of a field setter (`OpSet<K>(P, off, value)`), and
   - the second operand of `OpCopyRecord(src, P, tp)`.  Its source is only read.  Under
     `COPY_FREE_SOURCE` the source is released too, but by that bit's contract it is a
     callee's fresh temporary in a store of its own, so the release reaches nothing `x` views.

   The path's root is the OUTERMOST `OpGetVector` whose vector operand is a pure path:
   `ps[1].ops[0].a = 9` is rooted at `ps[1]`.

## Moving the views — only the live ones

At a copy point each reference derived from `x` (`op = prog.ops[pc]`) is moved into the copy
(`loft_dc_remap`).  It must move only if it is READ again before it is bound again.  A view
that is dead at the point needs no remap.  In the interpreter's loop, `op` and its `??` temp
are both bound again at the top of the loop before any read, so the `Call` arm's copy points
remap nothing.  The liveness is `lease::Frame`'s per-path pass: both arms of an `if`, a loop to
its fixed point, `break`/`continue` to their targets.  That pass was written for `(H-Elide)`,
where liveness may decide an optimisation; this is a second such reader.  A site the pass does
not reach (a statement the C writer built itself) counts as live.

The remaps are what cost gcc: with the rule on, `mix` ran in 128 ms against 95 ms with it off.
Without the `op` remap it ran in 97 ms, and without the temp's remap in 103 ms.  Prediction: with only live
views remapped, `mix` and `collatz` under gcc run within noise of the rule off, and fib(25)
keeps its gain (gcc 151 → 71 ms, clang 92 → 36 ms).  That is what turns the rule on by default.

## What the self-host gets

Five copy points are emitted in `run` today, all flag tests:

| point | before | form under this design |
|---|---|---|
| 1 | `n_lower(b, callee, __ref_3)` | flag fallback (`b`, `__ref_3` not proven apart from `progs`) |
| 2 | `OpCopyRecord(__lift_2, progs[callee], …)` | index guard on `progs[callee]` |
| 3–5 | the `Outcome` writes of three `return`s | flag fallback; `x` is dead after them, which `dead_after` should already have seen (step 1 finds why not) |

Point 1 still comes first, so the copy is still taken once per function lowered, as it is
today.  The gains on this shape come from the views no longer remapped and the return points
dropped.  The index guard is what keeps a write to ANOTHER element from copying.  On the
self-host that waits on point 1 being proven apart, which is a `StoreFacts` question for later.

## Not in this design

- **A dead OWNER source.**  `x = s` where `s` owns its record and is dead after is a move that
  needs no copy at all: ownership passes to `x`.  That needs `s`'s own release suppressed. It
  is a separate clause and no consumer has shown the shape yet.
- **A view of an inline sub-record** (`s = V[e].inner`): sound with a range compare (`x` lies
  inside element `k`'s bytes). It is left out until a consumer shows the shape.

## Cells

Guard: `loft-suite/rules/binding/B-Copy/a-deferred-copy-keeps-the-value-it-was-bound-to-whatever-the-source-does.loft`.
Its five cells stand (overwrite, grow, callee, clear, walk).  Added, each falsified on the
build it guards (`mk(i) = P { ops: [Op{i,1}, Op{i,2}, Op{i,3}], k: 10 * i }`):

| cell | shape | value | fails when |
|---|---|---|---|
| other element | `prog = copy of ps[1]; ps[2] = mk(7)` → `prog.k * 100 + ps[2].k` | 1070 | never on values; it checks that skipping the copy is right |
| from the end | `prog = copy of ps[2]; op = prog.ops[1]; ps[-1] = mk(7)` → `prog.k*100 + op.a*10 + op.b` | 2022 | the guard compares raw integers |
| field write | `prog = copy of ps[1]; ps[1].k = 55; ps[1].ops[0].a = 9` → `prog.k*100 + prog.ops[0].a*10 + ps[1].k` | 1065 | the root is taken at the wrong `OpGetVector` |
| index moves | `i = 1; prog = copy of ps[i]; i = 2; ps[i] = mk(7); i = 1; ps[i] = mk(8)` → `prog.k` | 10 | the index is captured at the bind |
| carried view | `prog = copy of ps[1]; op = prog.ops[0]`; loop n=2: `sum += op.a*10 + op.b + prog.k; if step == 0 { ps[1] = mk(9) }` | 42 | liveness misses the loop's back edge |
| source live | `prog = callee; ps[1] = mk(7)` → `prog.k * 100 + callee.k` | 1070 | the guard materialises the source too, or skips `x` |

## Steps

1. **Liveness of the derived views** (`dc_materialise`): remap a derived view only where
   `lease::Frame` says it is live after the point; drop the point where `x` and every view
   are dead.  Find why points 3–5 survive `dead_after`.  Cells: carried view, plus the five.
   Measure `mix`/`collatz`/`fib` under gcc and clang against the prediction above.
   3c needs no step of its own: the deferred bind already takes the source's reference.
2. **3b**: the element-shaped source check in `dc_binds`; the indexed-write recogniser and its
   guard in `dc_before`/`dc_materialise`; `loft_dc_is_elem` in `loft_rt.h`.  Cells: other
   element, from the end, field write, index moves, source live.
3. **On by default**: remove the `LOFT_DEFERRED_COPY` opt-in and add a `LOFT_NO_DEFERRED_COPY`
   switch; `c_sweep.py` default and poison; guard falsified; tell `loft-91` to rerun
   `selfhost/bench/measure.py`.
