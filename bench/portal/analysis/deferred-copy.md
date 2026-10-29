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

Five copy points were emitted in `run`, all flag tests.  Under this design there are none:

| point | before | now |
|---|---|---|
| 1 | `n_lower(b, callee, __ref_3)` | no point: `prog` is bound again (`prog = callee_prog`) before its old value is read on every path, and so are its views |
| 2 | `OpCopyRecord(__lift_2, progs[callee], …)` | index guard on `progs` admitted; no point, for the same reason |
| 3–5 | the `Outcome` writes of three `return`s | no point: nothing is read after a `return` |

Two precisions in the rule's own apart test made the cells' first point disappear as well: a
buffer that is null at every mention (`__ref_N`, which a callee mints into) names no store
that exists, and a scalar read out of a record (`callee = op.a`) is not a view of it, so it
does not keep `op` live (`lease::Frame::readers_of`).

## Built (2026-10-28)

Measured on the real self-host interpreter (`target-exp/wt-merge`), `-O3`, pinned to one
core, the minimum of five runs:

| kernel | gcc off → on | clang off → on |
|---|---|---|
| fib(25) | 149.9 → 47.2 ms (−69 %) | 90.1 → 30.9 ms (−66 %) |
| mix(500000, 1) | 157.4 → 158.8 ms (+0.9 %) | 85.2 → 84.1 ms |
| collatz(1, 20000) | 821.6 → 823.3 ms (+0.2 %) | 450.0 → 443.5 ms |

Every answer agrees with the rule off.  The prediction held: with no view remapped on the hot
path, `mix` and `collatz` are within noise under gcc, and fib gained more than the flag form
did (−53 %), because no copy point is left at all.

Falsified by mutation, each on the cells named: the guard never firing fails overwrite, from
the end, field write, index moves, carried view and source live; a raw-integer compare fails
from the end alone; no view moved fails overwrite, from the end and carried view; a root taken
at the nested element fails field write (1145).  Dropping a point when only a view of the local
is read cannot fail any cell, because the language never produces that shape: a local rebound
while a view of it is read later makes the view its own copy at its bind (`(B-View)`).

Found on the way, fixed beside it: the view walk (`src/scopes/view_walk.rs`) missed that
materialisation in a LOOP when the local VIEWS its container on the way in and holds its own
copy from the second pass on — both backends then read a released record, right by luck on a
plain run and the poison pattern under `LOFT_POISON=1`.  Guard:
`tests/scripts/a-view-taken-through-a-local-that-copies-on-a-later-pass-materialises.loft`.

### Re-measured on the self-host after S3 (2026-10-29)

`run.loft` after S3 (owner table, record and image ops) has ten flag points in its op arms:
calls handed the bundle `b` (`n_load_table`, `n_default_image`, `n_database`), which cannot be
proven apart from `progs` because `progs`'s record is placed in the caller's return-buffer
store.  `mix` and `collatz` never run those arms, yet ran 22 % and 16 % slower under gcc
(loft-91's `measure.py`: +19 % and +14 %).  Priced on the emitted C, each variant against the
rule off: points removed 0.997 / 0.976; points kept without any remap 0.986 / 0.966; the
helper cold and not inlined 1.216; the remap through a pointer 1.237; only `op`'s remaps kept
1.019 / 1.004; only the `??` temporary's kept 1.224 / 1.165.  The cost was the remap of the
`??` discharge temporary (`__ncr_5`), which is never read after the point: its later reads
are `op`'s, a separate variable moved on its own.  The point now moves a view only where the
view ITSELF is read again (`lease::Frame::read_itself_after`); the local still counts its
views' reads.  Fixed: mix 1.021, collatz 1.004 (gcc), 1.023 / 1.014 (clang); fib 0.293 / 0.335.
The ~2 % left is the eight live `op` remaps; proving `b` apart would remove them (0.986).

loft-91 measured the fix through `measure.py` (`--native-c --ship`): gcc matched, clang still
read mix 1.068, collatz 1.040.  The ship path emits WITHOUT `LOFT_RELEASE_PASS_PROBE`, the C
priced above; on that C under clang the points themselves cost what the `op` remaps did not
(all points removed: 0.977 / 0.960).  So the points had to go, which is an apart question:
- `progs` is placed in the caller's return-buffer store by `(R-ExitVector)` (admitted with no
  move — it is never returned).  Declining that admission removed the points but dropped
  `R-ExitVector`, `R-DecodeView`, `R-TextRun`, `R-Header`, `R-Base` and `R-Range` across eleven
  benches (`scripts/rewrite_census.py`), so the placement stays.
- A plain argument (`b`) whose record type reaches no `reference<τ>` field owns a tree
  disjoint from records this activation placed or minted.  Building that clause found a
  SOUNDNESS hole in what had shipped: `distinct` called a fresh store apart from an argument
  whose pointer field pointed into it (`h.link = &ps[1]; poke(h)` read 55 for 10 on C).  A
  value whose type reaches a pointer is now apart from nothing (guard cell `linked`).  The
  loop-header hoist was probed on the same shape and declines it (a growth through a pointer
  path is not admitted).
- The promoted work buffer (`__ref_15`) and a destination temporary (`__ref_4 = images[t]…`)
  got their own clauses.

`run` now has no copy point at all.  `measure.py`'s path, on/off: gcc fib 0.295, mix 1.009,
collatz 0.990; clang fib 0.335, mix 0.977, collatz 0.959.

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
| field write | `prog = copy of ps[1]; ps[1].ops[0].a = 9; ps[1].k = 55` → `prog.k*100 + prog.ops[0].a*10 + ps[1].k` | 1065 | the root is taken at the wrong `OpGetVector` (the nested write comes first, so it alone must take the copy) |
| index moves | `i = 1; prog = copy of ps[i]; i = 2; ps[i] = mk(7); i = 1; ps[i] = mk(8)` → `prog.k` | 10 | the index is captured at the bind |
| carried view | `prog = copy of ps[1]; op = prog.ops[0]`; loop n=2: `sum += op.a*10 + op.b + prog.k; if step == 0 { ps[1] = mk(9) }` | 42 | liveness misses the loop's back edge |
| source live | `prog = callee; ps[1] = mk(7)` → `prog.k * 100 + callee.k` | 1070 | the guard materialises the source too, or skips `x` |
| outlives | `prog = copy of ps[1]; o = prog.ops[2]; ps[1] = mk(7); prog = ps[0]` → `o.a*10 + o.b + prog.k` | 13 | never on the copy point: `(B-View)` makes `o` its own copy, since `prog` is rebound while `o` is read |

## Steps (all built 2026-10-28)

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
