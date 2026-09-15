<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 163 — Copy leases: a type with `OpDrop` says how a copy gets its own lease, or refuses the copy

## Status

Active — P0, P1 and P2 done; P3 (the refusal as an error) waits on open questions 9 and 10.  Tracked as
[`@PLN163`](https://github.com/loft-lang/plans/issues/163).  Decided with the owner on 2026-09-15
after the drop-release arc (`heap.md` D-heap-1, D-heap-7) kept meeting shapes where the compiler
could not tell which copy should release a resource.

## P0 findings so far

**Who declares `OpDrop`.**  42 files, all in this repository: `tests/scripts` (34), the feature
example `tests/docs/features/F115.loft`, one error-message case, the `dropscope` fixture library,
and five sqldb driver fixtures (`sqlite`, `postgres`, `maria`, `duckdb`, `registry`), each a
database cursor type.  **No published library, no consumer (`moros`, `dryopea`, `crawler`) and none
of the 187 packages in the registry cache declares one** — measured with a whole-tree grep for
`OpDrop` in every file type, controlled by a known hit in `139-drop-cascade.loft`.  So no type in
published code can be copied under the new rules, and none can be refused.

**Where the corpus copies one.**  `LOFT_DROP_COPY_CENSUS=1` over the 35 files that declare a hook:
576 copy sites, 362 of them from a variable the author wrote, in 27 files, plus 646 release
snapshots (below, question 8).  Every file printed the count line.  The census reads the IR after
the scope pass, which both backends share; P2 proves it against the copies each generator actually
emits (`copy_manifest.rs`).

**What that decides for P3.**  loft is at contract 0, and `COMPATIBILITY.md` § The error surface is
one-directional says an error is added *before* the freeze ("be strict now, because you can always
relax later but never tighten").  With no published user, P3 needs no deprecation window.  What it
does need is converting this repository's own uses.

**The sqldb fixtures spell a move by hand.**  `registry.loft` wraps a cursor with
`out = RowsSqlite { rs: cs }; cs.disown()` (eight sites), and each driver's `disown` zeroes the
handle so the source's drop does nothing.  Under `(H-Copy-Refuse)`, the `disown` call is a use of
the source after the copy, so the rule refuses the fixture's own workaround.  The rewrite is the move
the rule allows: drop the `disown` call and let `cs` die at the copy.  P3 converts these eight sites
along with the tests.

Measured on both backends against a database-free copy of the shape (a cursor type and a wrapper
enum, each with a hook that prints when it runs, and a `close` that prints only when it releases a
live handle): with and without `disown` the trace is `M7 R7 st=7 end Dany C7 Dcur7` — the cursor is
closed once, at the caller's drop, and never at the arm's end.  So `disown` is already redundant
today, and the conversion changes no behaviour.

**A first cell for P2.**  The same probe prints `advice[avoidable-copy]: copy of Cur — cs is still
used after this point` for the variant with NO use after the copy.  P2's refusal needs exactly this
fact (is the source used after the copy?), and here it already answers wrong.  Narrowed on
`--interpret`: it fires with no `OpDrop` at all, and the "later use" it names is always the
function's LAST line, wherever that is and whatever it reads.  The cause is the survival walk
(`use_analysis.rs`, the `Value::Var` arm that fills `last_use_pos` / `last_use_loc`), which counts
every read, including the scope pass's release of the source at the function's end.  That walk also
feeds `scopes::move_elide`, so excluding releases can change what is emitted, not only the advice.
That makes this P2's first cell rather than a side fix: P2 decides "used after the copy" once, and
this walk either becomes that home or stops being asked.

## Goal

A type that declares `OpDrop` decides what a COPY of its value means — it makes a second lease
(`OpCopy`) or it refuses the copy at compile time — so every structure is dropped exactly once and no
rule has to guess where a release moved.

## Effort + design

- **Effort:** H, cut into the phases below, each with its own comparison.
- **Design:** ~ — the rules are settled with the owner; the hook's exact shape and the refusal's
  completeness proof are phase work.

## How it was decided — from the use cases, not the rule text

Each model below was replaced because a concrete program broke it:

| program | resource model (`(H-Drop)` today) | what the use case showed |
|---|---|---|
| `a = mk(); b = a; b.id = 4` | one drop: the copy takes the release, `a`'s structure is never dropped | two structures, so two drops are RIGHT |
| `b = a` inside a block, `a` read after it | `b` releases while `a` is still in use | a drop rule cannot fix this: the copy needs its OWN lease |
| an outward network connection, a writable file | — | `dup` shares one stream and one position, so a second lease cannot exist: the copy must be REFUSED |
| `a = same(a)` | two drops (the displaced record, then the copy) | a value copied back into its own variable is one structure: one drop |

Measured on both backends before the decision; the probes are in the session record of the issue.

## The rules this plan introduces

Written into `doc/claude/formal/heap.md` and `binding.md` in phase 1, replacing `(H-Drop)`'s copy
clause ("the copy owns, the source stops dropping"):

- **(H-Lease)** a structure of a type that declares `OpDrop` holds a lease, and is dropped once, at
  its own death.  Two structures are two drops.
- **(H-Copy-Lease)** an implicit copy of a value whose type declares `fn OpCopy(self: τ)` copies the
  bytes and then runs `OpCopy` on the NEW structure, which takes its own lease.  A struct whose members
  have `OpCopy` gets a synthesized copy cascade, the mirror of the drop cascade (C111).
- **(H-Copy-Refuse)** a type with `OpDrop` and no `OpCopy` — or a struct holding such a member — can
  not be copied.  A copy while the source is still used is a compile error that names the later use.
  Legal, because no second lease exists: passing it as a parameter (`F-ParamHeap`: no copy), a
  `return`, `b = a` or a container literal when the source is not used afterwards (a move), and
  `b = &a`.
- **(H-Rebind-Self)** rebinding a variable to a copy of its own value (`a = same(a)`, `a = a`) is not
  a new structure.
- **(H-Elide)** the compiler may elide a copy together with the drop of the structure it would have
  made, where C86's transparent-link conditions hold.  `OpCopy` and `OpDrop` must not rely on running
  for an elided copy.

What these retire: `(H-Drop)`'s copy clause, `INTERFACES.md` § "Putting one in a container" ("a copy
into a container is a **move**"), the `double-move` warning's "released TWICE" framing (a live copy
of a refusing type is now an error, of a leasing type correct), and the drop gate's oracle, which
follows one id through copies (it becomes: each structure released once, each `OpCopy` paired with a
drop).

## Composition matrix — Stage A

The axes this plan crosses, each a `/tmp` probe on `--interpret` first, graduating to
`tests/scripts/`:

| axis | values |
|---|---|
| type | `OpDrop` + `OpCopy` · `OpDrop` only · plain struct holding each · nested two deep · struct-enum payload · tuple member · vector element |
| copy site | whole bind · reassignment · call return · into a field / enum payload / element / tuple member · a view materialised by a disturbed container (`B-View`) · a copied return · a `??` default · a branch arm · a loop body |
| source after the copy | not used (move) · read · written · used on one path only · captured |
| move-like uses | parameter · `return` · `&` bind · `a = same(a)` |
| backend | `--interpret` · `--native` (· `--native-wasm` for the hook calls) |

Each cell scores the drop COUNT and the `OpCopy` COUNT per structure, the compile verdict, and the
value; a refusal cell scores the error and its named line.

## Sub-arcs

| Item | Source | Verify | Status |
|---|---|---|---|
| **P0** — census: every type with `OpDrop` in the corpus and the published libraries, and every place one is copied today | `LOFT_DROP_COPY_CENSUS` (`use_analysis::drop_copy_census`), the library and consumer trees, the registry cache | `ownership_drop_gate::the_census_names_the_copy_each_cell_makes`: every CROSS and COALESCE cell against a per-axis expectation, and the CROSS `local` cells with the hook removed report `0 sites`.  Falsified: disabling the bind arm fails 54 cells; the join peel and the view-temp dependencies each failed it before they existed | Done |
| **P1** — the rules above in `formal/heap.md` + `binding.md`, with a deviation for every current behaviour that disagrees | this plan | Every gate cell has a lease verdict (`Once` / `Refused` / `Open`), and `every_cell_disagreeing_with_the_lease_rules_names_its_open_deviation` ties each `Once` cell that fails in a baseline to exactly one OPEN entry: `D-heap-1` (`p_o2`), `D-heap-7` (family 1, 19 cells), `D-heap-10` (`p_i2`).  All 111 `Refused` cells are `D-heap-8`'s, and `D-heap-9` is `OpCopy`.  `registers` reads `OPEN: 5`.  Falsified: closing `D-heap-10` in the register, or removing `p_i2` from a baseline, turns it red.  The census check asks a refused cell for a copy from a variable; marking a fresh-only cell refused turns it red.  Only `c_tuple_tuplem` is `Open` (question 9) | Done |
| **P2** — the refusal as a REPORT (no error): at every copy site of a refusing type whose source is used afterwards | `copy_manifest::Origin` (both backends' emitted copies) + `ParserMaterialise` | the manifest guard: every emitted copy of a refusing type is reported or proven a move; falsified by disabling one site | **P2a done:** `src/lease.rs` is the `(H-Move)` home, and the census prints its verdict as `lease=`.  `the_census_names_the_copy_each_cell_makes` refuses exactly the 111 `Refused` cells and no `Once` cell, with nothing unreached.  Falsified twice: without the loop fixed point `p_l1`/`p_l2` pass; with a release read as a use, 94 `Once` cells are refused.  Corpus (35 files): 448 moves, 113 refusals (52 caller, 45 container, 16 later, each `later` row read by hand), 0 unreached.  **P2b done:** every copy of a droppable the interpreter or the native generator emits over the 275 cells has a census verdict — `every_emitted_copy_of_a_droppable_has_a_lease_verdict`, keyed by function and destination, a call result's bind covered by the callee's `return` verdict.  It first found 10 unjudged binds of a callee's moved result, closed by judging every `return` of a droppable.  Falsified: without that, 28 call-result binds are unjudged on the two generators.  Corpus (35 files, both generators): 408 emitted copies of a droppable, 0 without a verdict.  The first run counted 2 more: the parser recorded a materialisation into `__ref_3` in `two_exits` (`1506b`, `1515`) while building the IR, and the scope pass then merged that work variable into the return buffer.  `__ref_3` occurs nowhere in the final body, and both copies into the buffer carry a verdict, so a parser record whose destination is gone names no emitted copy and is not counted.  **P2c:** the corpus rows read; two findings in the open questions (10: a member of a container that dies at the projection is already moved out statically; 11: a returned local tuple was refused, now judged whole) |
| **P3** — the report becomes a compile error; the published-library gate read row by row | P2 | `tests/scripts` refusal cells (`@EXPECT_ERROR`) on both backends; every library break is a real double release today, or the rule is wrong | Open |
| **P4** — `OpCopy`: signature check (mirror `check_drop_signature`, `definitions.rs`), synthesized cascade (mirror `synth_drop_cascades`), a call at every copy site on both backends | P1 | drop gate re-baselined on the lease oracle, both backends, `LOFT_POISON`; the matrix's `OpCopy` count per cell | Open |
| **P5** — remove the machinery that moved a release across a copy, one decider at a time | `scopes::copy_moves_drop_from`, the per-path hand-off flags, the caller-record mark | drop gate and the matrix unchanged after each removal; `introspect_diff` names exactly the removed ops | Open |
| **P6** — `(H-Elide)`: an elided copy skips its `OpCopy` and the matching drop | C86, `alias-where-correct.md` | matrix cells where elision fires count hooks consistently; `LOFT_LINK_WIDEN` on and off agree on every observable value | Open |

## P2 design — one home for "is the copied value used after the copy?"

**Invariant.** A copy of a type that owns a droppable without `OpCopy` is reported exactly when
`(H-Move)` says it is not a move, and the report names the use.  The question is answered in ONE
place, `src/lease.rs`, cited `@FR-H-Move`.

**Why a new home.**  Eight sites already answer a "last use" question.
- Five are sequential store-liveness walks, and they say so ("Sequential approximation across
  branches"): `store_liveness_walk`, `last_use_guard`, `lastuse_reclaim`, `store_dead_after_block`,
  `Variables::last_use`.
- One is pre-order positional: the survival walk's `last_use_pos`, which counts a release as a use
  and orders sibling arms by position, so `if c { x = a } else { y = a }` would read as used after.
- The codegen last-use move answers it for code generation.
- The double-move scan resets per arm.

None is per path, and the survival walk and the codegen move feed emission.  So P2 leaves them as
they are, and P5 moves the two that ask H-Move's own question onto the new home.

**The pass.**  A backward liveness pass for one variable over the post-scope structured IR.
- **Paths:** both arms of every `if` (value-position included), a `loop` iterated to a fixed point,
  `break`/`continue` taking their target's liveness, and `return` ending the path.
- **A use** is a read of the variable, or of a variable whose type depends on it.
- **A kill** is a rebind: a `Set` to a new value, or an in-place `OpDatabase` rebuild.
- **Not a use:** the scope pass's release scaffolding — a release call on the variable, the null
  check and `__hoff_` flag test that guard it, the sentinel written after it, and the `__disp_`
  snapshot a rebind takes.  It is recognised by the release call and those compiler names, which
  the author cannot write, never by `if` shape alone.

**Classifying a copy's source,** per arm of a join:
- a PROJECTION, or a variable whose assignment is one (a loop variable, a `match` binding) —
  refused: the container holds it;
- a parameter, or a local that holds the caller's record — refused: the caller holds it;
- otherwise — refused when the pass finds it live after the copy, and a move when it does not.

`return` of a parameter, or of a place reached through one, is a site of its own, because the
callee copies nothing there.

**Failure paths the cut has to catch.**
1. A scaffold read as a use: every `Once` cell refused.
2. A use missed on a path: a `Refused` cell passes (a copy in a loop, a use after `break`).
3. A member view read as a whole value: a loop variable copied is called a move.
4. A copy the IR walk never sees: a returned parameter, or a copy only a generator mints.

**Cut.**
- **P2a** — the pass and the classification, printed as a `lease=` column on each census line.
  Verified against `lease_verdict` on all 275 gate cells in BOTH directions: a refusal on every
  `Refused` cell and on no `Once` cell.  Falsified by removing the loop fixed point, and by reading
  one scaffold as a use.
- **P2b** — completeness: every copy `copy_manifest.rs` records for a refusing type (keyed by
  function and destination) matches a census site with a verdict.  Measured over the gate cells
  and the 35 corpus files.
- **P2c** — the corpus report: every refusal in the 35 files, read row by row.  This is what P3
  converts.

## Phase ordering

1. **P0 before anything:** the census says how many programs and libraries this touches, which
   decides whether P3's error needs a deprecation window (`COMPATIBILITY.md`).
2. **P1 before code**, so every later phase closes a named deviation rather than inventing one.
3. **P2 before P3:** the refusal is proven complete as a report before it can fail a build.  This is
   the load-bearing phase — `copy_manifest.rs` records that a whole-record bind and a call-return bind
   are minted at emission time and appear in no IR the analysis walks (loft#774), so a refusal built
   on the IR alone misses them.
4. **P4 after P3:** with refusal as the default, `OpCopy` is an opt-in the matrix can test type by
   type.
5. **P5 and P6 last**, as removals and an optimisation measured against the finished rules.

## Open design questions

1. **The hook's shape.**  In place on the new copy (`fn OpCopy(self: τ)`, fixing fields after the
   byte copy) is proposed; a constructor form (`-> τ`) would need a return ABI at every copy site.
   Confirm the name cannot collide with the internal `OpCopyRecord` family.
2. **The refusal's error text** has to name the later use that turned a move into a copy, the way
   Rust's "use of moved value" does, including a use on one branch only.
3. **A copy the compiler inserts on its own** — a view materialised because its container changed
   (`B-View`), a `??` default, a copied return: the error names the construct that forced it.  Is a
   refusal there acceptable, or does the construct need a non-copying lowering for refusing types?
4. **Byte moves that are not copies**: a vector's growth, a keyed collection's rebalance, a store
   compaction.  None may run `OpCopy`; each is confirmed by a matrix cell.
5. **Across threads** (`par`): a value copied into a worker's store is a copy — `OpCopy` runs, or the
   type refuses.
6. **A non-droppable type that must not be copied** (a `unique struct` modifier, beside `value struct`):
   left out until a use case asks for it.
7. **DECIDED 2026-09-15 (owner): refused** for a type without `OpCopy`, a second lease for a type
   with one.  The container's drop is a use of the member, so copying a member out of a container
   whose drop still runs is a copy while the source is used.  The author writes
   `c = acquire(); …; return c` instead.  Written into P1 as two more consequences, and the
   parameter one CONFIRMED by the owner 2026-09-15 ("refuse parameters in the callee").  A PARAMETER
   is never moved, because the caller holds it, so `return p` and `x = p` are refused too.  That
   narrows `(H-Rebind-Self)` to `a = a` and `a = a ?? d`, since `same(a)` is refused inside `same`.
   The builder idiom keeps a loft spelling: write through the aliasing parameter instead of
   returning it.  **Returning a member of a local that is about to die** (`return s.h`).  The census shows a copy
   into the return buffer (`kind=buffer from=s`), while `s`'s own drop still covers the member.
   `return a` of a whole local copies nothing, so "a `return` is legal" holds for a whole value only.
   For a member there are three readings: a second lease, a refusal, or a move OUT of the container.
   A move out needs the container's drop to skip the member, which is the per-element mark
   `heap.md` § Standalone right, encapsulated warned already declined.
8. **The release snapshot.**  A reassignment of a droppable deep-copies the displaced record into
   `__disp_N` so its hook can run after the statement (`Scopes::displaced_drop`).  That is a real
   runtime copy that no rule asks for.  It must neither run `OpCopy` nor be refused.  Under
   `(H-Lease)` the displaced structure is simply dropped, so P5 decides whether the snapshot
   survives at all.
9. **A tuple member placed in a tuple copies; a field placed in a tuple views.**  `t = (tt.0, 1)`
   emits `OpCopyRecord(tt.0, __ref_p2_1)` (`tuple_member_copy`) and releases id 1 twice
   (`M1 R1 D1 D1`), while `t = (s.h, 1)`, `t = (vs[0], 1)`, `t = (p, 1)` and `t = (a ?? d, 1)`
   are views typed with a dependency and release once.  By `(B-View-Base)` a struct projection off
   an owned base is a view, so the tuple-member spelling is the deviation candidate.  P1 classifies
   it against `tuples.md` `(T-Cons)`, which does not say whether a member is copied in.
10. **A member projected off a call result that dies in the statement.**  `mk_s(7).h.id` and
    `r = mk_dense().h` are refused today, as `container`, by the letter of `(H-Copy-Refuse)`.
    But the compiler already MOVES such a member out, statically: it copies `.h` into a buffer and
    releases the call result with `OpDropAllExcept(__lift_2, 0, …)`, which skips that member.
    There is no runtime mark (measured, post-scope IR, 2026-09-15).  Decision 7 declined a move out
    of a container because it would need "the per-element mark" — a premise that holds for a
    container that lives on, not for one that dies at the projection.  The same static skip could
    serve `return s.h` of a local that dies at the return.  Owner's call: is a move out of a
    container that dies at the move legal (one structure, one drop), with only a container that
    lives on refused?  Until decided, the report follows the rules as written.
11. **P2c finding — a returned local tuple is refused as `container`.**  `t = (s, 5); return t`
    lowers to a `synthetic_tuple_return` block: a hold `__ref_3 = t`, whose type depends on `t`'s
    member backing rather than on `t`, then a copy of each member into the return buffer.  The
    census did not see that as a whole-tuple copy; P2b's census recognises the block and resolves
    the hold, so the return is judged as `t` whole.

## Cross-arc dependencies

- `heap.md` D-heap-1 and D-heap-7: most remaining shapes become either correct (two structures) or a
  refusal; P1 re-classifies them rather than fixing them one by one.
- C86 and `plans/102-stability-contract/alias-where-correct.md`: P6 is the drop half of the
  transparent-link widening.
- The post-scope lint stage (`976680b3b`): P2's report runs there, on every path.

## See also

- `doc/claude/formal/heap.md` § Drop — `(H-Drop)`, `(H-Drop-Not)`, D-heap-1, D-heap-7.
- `doc/claude/formal/binding.md` — `(B-Copy)`, `(B-View)`; `doc/claude/formal/calls.md` —
  `(F-ParamHeap)`.
- `doc/claude/DESIGN_DECISIONS.md` C86 (whole-value binds copy), C111 (the drop cascade).
- `doc/claude/INTERFACES.md` § Running at scope end — `OpDrop`.
- `src/copy_manifest.rs` — the emitted-copy manifest P2 proves completeness against.
- [`@PLN163`](https://github.com/loft-lang/plans/issues/163).
