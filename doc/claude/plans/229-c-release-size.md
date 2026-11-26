# @PLN229 — the C of a function's releases (design)

A track of @PLN229 (building a large program): the C a long function is written as is mostly
the releases at its exits.  This page measures it, lists the options with the invariant each
rests on, and recommends one.  Measured on the self-hosted elaborator's split build (porter
commit 36f3b0f19), the build B3 makes the gate's.

## What the C is made of

The largest unit is one file, `selfhost/src/el_ex_assign.loft` (2870 lines, under the 5000-line
ceiling): 19.9 MB of C.  87 % of it, 17.3 MB, is ONE function, `assignment_inner` (1703 lines),
which `(R-CSplit)` cuts into 127 parts.  That is 10 KB of C per source line, where the program
averages 0.7 KB (105 MB from 155 711 lines).  Of its 265 030 lines of C:

| What | Lines |
|---|---|
| release statements (`loft_free_var`, `…_distinct`, `loft_wpark_put`, `loft_text_free`) | 223 222 |
| everything else | 41 808 |

The function holds 1013 hidden result buffers (`__ref_N`, one per call that delivers into a
buffer), 85 `__ref_p` and 51 work texts, all living for the whole function.  The scope pass
frees each at EVERY exit ((H-Free)), so its 285 exits carry release runs of 140 to 1004
statements; `(R-CTail)` shares a tail only where exits end alike, and these do not.

**The premise every option below rests on, measured:** of the 197 185 exit releases that name a
buffer, 88 357 (45 %) free a buffer that nothing in the text BEFORE that exit assigns.  Each
such buffer is still its declaration's null reference there, and `loft_free_var` of a null
reference does nothing.  A path-sensitive count (a buffer assigned only in a branch the exit
does not come through) can only be higher; it is not measured yet (phase 0).  A two-function
probe shows both shapes: an early `return` frees a buffer first filled after it, and each exit
of an `if` frees the buffer of the branch that did not run.  The frees are in the IR
(`OpFreeRef` at the `return`), so the interpreter's bytecode and both native writers carry them.

Why it matters: gcc's time and memory follow the C's size, in the one-unit build and `--ship`
as much as in the split one.  This unit alone is most of a one-file rebuild (90 s) and sets the
split build's peak memory.

## The options

**A — release at an exit only what a path to it may have assigned** (the scope pass).
*Invariant:* an exit frees exactly the owned variables that some path from the function's entry
to that exit may have given a store; a variable no such path assigns is null there, and its free
is a no-op, so leaving it out changes no free that happens, no leak and no answer.  A forward walk
over the structured body (an `if`/`match` joins its arms, a loop counts its whole body) gives
each exit its may-be-assigned set.  *Removes:* at least 45 % of the release statements (the
measured bound), on every backend.  *Costs:* it changes the IR of every function with an early
exit, so gate E's image of nearly every program moves: the self-hosted scope pass must carry the
same rule before the port agrees again.  One site (the scope pass's exit frees); a miss there is
a leak, which the store-heap ceiling and the lifetime check report.

**B — a hidden buffer lives for the statement that fills it** (the scope pass).  Each
`__ref_N` is freed where its value's use ends, not at the function's exits, so an exit frees
only what is live at it.  *Invariant:* the memory model's own rule that a value dies with its scope
([LIFETIME.md](../LIFETIME.md)), extended to the compiler's temporaries.  *Removes:* more than A,
and memory is released earlier.  *Costs:* a change of WHEN frees happen — a store-lifetime
design, not a size fix — with every lowering that hands a buffer on (`bind`, a return buffer, a
copy-out) to re-check; and gate E's images move as in A.  An M+ plan of its own.

**C — one release epilogue per function** (the C writer).  Every exit stores its result in the
function's slot (`loft_rv`, as `(R-CTail)` does) and jumps to one tail that frees every owned
variable once.  *Removes:* almost all of it (1013 frees once instead of 223 222).  *Premise, not
held:* every variable is freed by one FORM at every exit (`loft_free_var_distinct(v, ret)` where
the exit returns a store, the plain form where not) and in one ORDER.  Measured on 2026-11-18 for
the scope-pass version (the tester handoff of that day, `plans/91-self-host/`): 323 of 434
multi-exit functions disagree, and the owner set it aside.
The C version meets the same disagreement.

**D — the release runs as data** (the C writer).  An exit's run becomes one call over a table
written once beside the function (`static const loft_rel loft_rel_<n>[] = { {kind, slot}, … }`:
the release kind and the variable's place in the frame struct), so the same statements run in
the same order, from 4 bytes of table per release instead of a line of C.  *Invariant:*
`(R-CTail)`'s own: the rule moves where a release is WRITTEN, never which releases run, in which
order.  *Removes:* nearly all of the release CODE (a table entry instead of a call), keeping
every free.  *Premises to check:* the variables must be addressable from the table, which holds
for a function `(R-CSplit)` parts (its locals live in `loft_fr`, so `offsetof` names them) and
not for an unparted one (it would keep its statements; those are the small ones); and the call
through the table must be as fast as the statements, measured on a release-heavy routine.
*Costs:* the C writer and `loft_rt.h` only: no IR change, so gate E, the interpreter and the
rustc writer do not move.

**Not options:** cutting a large file's unit into pieces cannot cut inside one function (87 % of
this unit); a line limit on files does not reach it either (the file is 2870 lines).  Splitting
`assignment_inner` in the port's source would help this program alone.

## Recommendation

**D first, then A.**

- D removes nearly all of the release code where it is large (the parted functions are exactly
  the long ones), changes no IR, and keeps every free in its order, so it is a C-writer change
  proven by the program's behaviour being unchanged (the suite, both C modes, ASan, the leak
  report) and by the elaborator's verdicts.
- A removes the releases that do nothing, on every backend, which is right on its own terms (the
  interpreter's bytecode shrinks too), but it moves every gate-E image, so it waits for the
  porter's scope-pass port and is sequenced with them, not now.
- B is a memory-model plan, not a size fix; C rests on a premise measured false.

## Phases (D)

| Phase | Deliverable | Validated by |
|---|---|---|
| D0 probe | the release share of every parted function in the elaborator and in the suite's C, and the per-release cost of a table walk against the statements on one release-heavy routine | numbers written here; D is dropped if the walk costs more than 2 % on that routine |
| D1 the table | a parted function's exit run of N ≥ 8 releases written as `loft_release(cx, loft_fr, loft_rel_<n>, N)`; the kinds the runs hold (`free_var`, `free_var_distinct` with its slot, `text_free`, `wpark_put`) | the suite in C and split C, `c_emit`, ASan and the leak report unchanged; the elaborator's gate verdicts identical; its C size and gcc time measured |
| D2 the receipts | `(R-CTable)` stated beside `(R-CTail)` in `formal/rewrites.md`, its switch `LOFT_NO_C_RELTABLE=1`, a guard that a parted function's release runs leave the code | the guard FAILS with the switch set |

## D0 — measured

**Where the release code is** (the elaborator's 111 MB of C in functions, `scripts/c_release_share.py`
over the split units):

| Functions | C | of it, release | in runs of 8 or more |
|---|---|---|---|
| `(R-CSplit)` parts (locals in `loft_fr`) | 55.3 MB | 36.8 MB (66 %) | 36.1 MB — 32 % of all the C |
| unparted | 49.5 MB | 15.9 MB (32 %) | 14.7 MB — 13 % of all the C |
| parted functions' own bodies | 6.2 MB | 0.4 MB | 0.2 MB |

Release code is 48 % of the program's C.  D1 reaches the parts' 36.1 MB: their variables have
a static place in the frame struct.  The unparted functions' 14.7 MB would need their owned
locals gathered into a struct first (an open question below).  By form, in the runs D1 takes:
`loft_free_var_distinct` 196 488, `loft_free_var` 161 230, `loft_wpark_put` 46 797,
`loft_text_free` 36 004 — so an entry carries its kind, its slot, and for the distinct form
the slot of the value the exit returns.

**What a table costs** (a standalone replica of the two inline release forms, 5 % of the
buffers holding a store, `-O1` unless named; times per exit, best of three):

| Shape | Statements | Table |
|---|---|---|
| 40 exits × 1013 releases (`assignment_inner`'s scale) | 7.9 µs; gcc 27.8 s, 1.84 MB of object (`-O3`: 51.8 s) | 1.4 µs; gcc 0.17 s, 0.33 MB (`-O3`: 0.19 s) |
| 4 exits × 256 releases | 396 ns | 378 ns |
| 4 exits × 64 | 77 ns | 89 ns |
| 4 exits × 8 | 8.4 ns | 12.2 ns |

At the scale of a parted function the table is five times faster: the straight-line releases
miss the instruction cache at every exit, the loop stays in it.  Where the code is small and
hot, the statements win (1.4x per release up to 64).  So D1 takes the release runs of PARTED
functions only — large by construction, so cold — of eight releases or more, and leaves every
other function's releases as statements.  The assumption a replica cannot settle (how many
buffers hold a store at a real exit) is D1's to measure, on the gate's own run time.

## Open questions

- Phase 0 of A, whenever it is taken up: the path-sensitive share, from the IR.
- Whether a table run can carry `(R-CTail)`'s sharing (two exits naming one table) or makes it
  unnecessary.
- The unparted functions' 14.7 MB: gather a long function's owned locals into a struct so a
  table can name them, or leave them — measured after D1, by what remains.
