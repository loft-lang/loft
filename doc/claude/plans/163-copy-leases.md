<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 163 — Copy leases: a type with `OpDrop` says how a copy gets its own lease, or refuses the copy

## Status

Active — P0 done; P1 rewritten 2026-09-15 to a rule read off the line (owner); P2's report
reworked to that rule (P2r done); P3 — the report becomes a compile error — is next.  Tracked as
[`@PLN163`](https://github.com/loft-lang/plans/issues/163).  Decided with the owner on 2026-09-15
after the drop-release arc (`heap.md` D-heap-1, D-heap-7) kept meeting shapes where the compiler
could not tell which copy should release a resource.

## P0 findings

**Who declares `OpDrop`.**  42 files, all in this repository: `tests/scripts` (34), the feature
example `tests/docs/features/F115.loft`, one error-message case, the `dropscope` fixture library,
and five sqldb driver fixtures (`sqlite`, `postgres`, `maria`, `duckdb`, `registry`), each a
database cursor type.  **No published library, no consumer (`moros`, `dryopea`, `crawler`) and none
of the 187 packages in the registry cache declares one** — measured with a whole-tree grep for
`OpDrop` in every file type, controlled by a known hit in `139-drop-cascade.loft`.  So no type in
published code can be copied under the new rules, and none can be refused.

**Where the corpus copies one.**  `LOFT_DROP_COPY_CENSUS=1` over the 35 files that declare a hook:
576 copy sites, 362 of them from a variable the author wrote, in 27 files, plus 646 release
snapshots (question 8).  Every file printed the count line.

**What that decides for P3.**  loft is at contract 0, and `COMPATIBILITY.md` § The error surface is
one-directional says an error is added *before* the freeze ("be strict now, because you can always
relax later but never tighten").  With no published user, P3 needs no deprecation window.  What it
does need is converting this repository's own uses.

**The sqldb fixtures.**  `registry.loft` wraps a cursor with `out = RowsSqlite { rs: cs }; cs.disown()`
(eight sites), and each driver's `disown` zeroes the handle so the source's drop does nothing.
Under `(H-Copy-Refuse)`, `out = RowsSqlite { rs: cs }` writes a copy of the existing `cs`, so it is
refused; the rewrite builds the cursor where it belongs, `out = RowsSqlite { rs: sq.db_select(sql) }`,
and `disown` goes.  Measured on both backends against a database-free copy of the shape: with and
without `disown` the trace is `M7 R7 st=7 end Dany C7 Dcur7`, so `disown` is already redundant
today and the conversion changes no behaviour.

**The advice that counts a release as a use.**  `advice[avoidable-copy]: copy of Cur — cs is still
used after this point` fires on a copy with no later use, with or without `OpDrop`; the "later use"
it names is the function's last line.  The survival walk (`use_analysis.rs`, `last_use_pos` /
`last_use_loc`) counts the scope pass's release as a read.  That walk feeds `scopes::move_elide`, an
optimisation, so under the rules below it no longer bears on validity; it is recorded for whoever
next touches the copy advice.

## Goal

A type that declares `OpDrop` decides what a COPY of its value means — it makes a second lease
(`OpCopy`) or it refuses the copy at compile time — so every structure is dropped exactly once, no
rule has to guess where a release moved, and a programmer can judge from the code alone whether it
is valid.

## Effort + design

- **Effort:** H, cut into the phases below, each with its own comparison.
- **Design:** ~ — the rules are settled with the owner; the hook's exact shape and the refusal's
  error text are phase work.

## How it was decided — from the use cases, not the rule text

Each model below was replaced because a concrete program broke it:

| program | model tried | what the use case showed |
|---|---|---|
| `a = mk(); b = a; b.id = 4` | the release moves with the copy: `a`'s structure is never dropped | two structures, so two drops are RIGHT |
| `b = a` inside a block, `a` read after it | `b` releases while `a` is still in use | the copy needs its OWN lease |
| an outward network connection, a writable file | — | `dup` shares one stream and one position, so a second lease cannot exist: the copy must be REFUSED |
| `x = a; send(x)` beside `x = a; send(x); send(a)` | a copy of a value not used afterwards is a move (first version, 2026-09-15) | the first line compiles in one and fails in the other — a later line decides an earlier one's meaning.  **Rejected by the owner:** correct and incorrect code must not look alike to someone who does not know how loft reasons |
| `c = open_session().conn`, `return s.conn` | a member moves out of a container that dies at the copy (question 10) | sound, but the same pattern is valid or not depending on whether the container is used again — the same objection |
| a library holding a database connection | the rule read off the line | every use of `self.db` is a borrow in plain sight; the one line that would duplicate it is the line the error names, with a clean rewrite |

## The rules this plan introduces

Written into `doc/claude/formal/heap.md` § Drop and `binding.md` (P1), replacing `(H-Drop)`'s copy
clause ("the copy owns, the source stops dropping"):

- **(H-Lease)** a structure whose type owns a droppable holds its own lease and is dropped once, at
  its own death.  Two structures are two drops.
- **(H-Move)** a move is WRITTEN, never inferred: a fresh value placed where it is produced; a
  `return` whose every value is a variable the function owns or a fresh value; a block whose value
  is a variable it declares.
- **(H-Copy-Lease)** a copy of a type with `fn OpCopy(self: τ)` runs `OpCopy` on the new structure,
  which takes its own lease; a struct holding such members gets a synthesized copy cascade.
- **(H-Copy-Refuse)** a COPY of a type that owns a droppable without `OpCopy` is a compile-time
  error on the line that writes it, whatever follows: an existing value (a variable, parameter, loop
  variable, `match` binding, member, member of a call result) bound as a whole value, placed in a
  literal or appended, returned where `(H-Move)` does not allow it, or chosen by `??` or a join in
  one of those positions.  Legal: a fresh value anywhere, an argument, `&`, a view of a member.  The
  error names the copy and what to write instead.
- **(H-View-Drop)** a view of a droppable member never becomes a copy; disturbing its container
  while the view is used is the error, at the disturbance (`(B-Ref-Reshape)` for a view without `&`).
- **(H-Elide)** the compiler may elide a copy of a leasing type together with the matching drop.
  Liveness may decide such an optimisation, never validity.

What these retire: `(H-Drop)`'s copy clause, the first version's liveness `(H-Move)` and
`(H-Rebind-Self)`, `INTERFACES.md` § "Putting one in a container" ("a copy into a container is a
**move**", rewritten in P3), and the `double-move` warning's "released TWICE" framing.

## Composition matrix — Stage A

| axis | values |
|---|---|
| type | `OpDrop` + `OpCopy` · `OpDrop` only · plain struct holding each · nested two deep · struct-enum payload · tuple member · vector element |
| copy site | whole bind · reassignment · call return · into a field / enum payload / element / tuple member · a `??` operand · a branch arm · a loop body · a return |
| source | fresh · local · parameter · loop variable · `match` binding · member · member of a call result |
| non-copies | argument · `&` bind · view bind · `return` of the owner · a block yielding its own variable |
| backend | `--interpret` · `--native` (· `--native-wasm` for the hook calls) |

Each cell scores the compile verdict (and its named line), the drop COUNT and the `OpCopy` COUNT per
structure, and the value.  What the program does AFTER a copy is no longer an axis of validity.

## Sub-arcs

| Item | Source | Verify | Status |
|---|---|---|---|
| **P0** — census: every type with `OpDrop` in the corpus and the published libraries, and every place one is copied today | `LOFT_DROP_COPY_CENSUS` (`use_analysis::drop_copy_census`), the library and consumer trees, the registry cache | `ownership_drop_gate::the_census_names_the_copy_each_cell_makes`: every cell against a per-axis expectation, and the CROSS `local` cells with the hook removed report `0 sites`.  Falsified: disabling the bind arm fails 54 cells | Done |
| **P1** — the rules above in `formal/heap.md` + `binding.md`, with a deviation for every current behaviour that disagrees | this plan, C121 | **Rewritten 2026-09-15 to the rule read off the line.**  `lease_verdict` classifies every cell from its own lines: 201 of 275 `Refused` (all `D-heap-8`'s), the rest `Once`.  `every_cell_disagreeing_with_the_lease_rules_names_its_open_deviation` ties the one `Once` cell that fails in a baseline to `D-heap-7` (`q_present_local_var_ret`, `return a ?? b`); every other baseline line is a refusal.  `D-heap-1` and `D-heap-10` closed as reclassified; `D-heap-11` opened for `(H-View-Drop)`; `registers` reads `OPEN: 4`.  The mechanism was falsified in the first version (closing an entry, or removing a baseline line, turns it red) | Done |
| **P2** — the refusal as a REPORT (no error): at every copy site of a refusing type | `src/lease.rs`, `copy_manifest.rs` | the census refuses exactly the `Refused` cells; every emitted copy of a droppable carries a verdict | **P2a/P2b/P2c done under the first version:** a per-path liveness pass (`src/lease.rs`) printed as `lease=`, held to that version's verdicts (`liveness_verdict`, 111 refused); every copy either generator emits over the 275 cells and the 35 corpus files (408) carries a verdict.  The completeness half stands; the verdict half is P2r |
| **P2r** — the report reworked to the written-move rule | `src/lease.rs`, the census | the census refuses exactly the 201 `Refused` cells and no `Once` cell; the manifest check unchanged at zero; the corpus rows read again | **Done.**  The census prints two columns: `lease=` (`Frame::written_verdict`, the rule read off the line) and `liveness=` (the first version, kept for P6), each held to its own oracle on all 276 cells.  Sites added where no copy is emitted and the line still places an existing value: a tuple literal's `item`, the parser's erased `x = x` (noted in `copy_manifest`), a join arm that is a bare variable.  `p_t1` added for the read through a call result (`mk_s(130).h.id`), the one cell where the two readings disagree.  Falsified one decision at a time: the read-through placement (fails `p_t1`), the block-result move (`p_k4`), the self-bind note (`p_i1`), the join's view skip (4 field `arm` cells), the join's return placement (2 `local_var_ret` cells).  Corpus (35 files): 1248 sites, 646 of them snapshots; `lease=` refuses 355 (265 `copy`, 52 `caller`, 38 `container`), `liveness=` 112; the manifest check reads 250 emitted copies, 0 without a verdict |
| **P3** — the report becomes a compile error with the message the rule asks for; `(H-View-Drop)`; the corpus and fixtures converted | P2r | `tests/scripts` refusal cells (`@EXPECT_ERROR`) on both backends; `INTERFACES.md` § `OpDrop` rewritten | **`(H-View-Drop)` DONE 2026-09-17** — `heap.md` D-heap-11 closed, the register 3 → 2.  A view of a droppable member whose container is disturbed is refused instead of copied, raised from the walk that already refuses for `&` (`def_reshape_refusals`) with one added condition — the view's type owns a droppable — and its own message, because the `&` family's names a lost write that a plain view never had.  13 cells both backends, corpus 1590/0 changed, guards `a-view-of-a-droppable-member-stays-a-view.loft` + `parse_errors::h_view_drop_*`.  Boundary: a callee's GROWTH still copies (`disturbed: None` on the refusal walk; @PLN164 C3's separable widening).  **Still open in P3:** the D-heap-8 refusal itself (every written copy of a droppable) and the corpus/fixture conversion it needs |
| **P4** — `OpCopy`: signature check (mirror `check_drop_signature`), synthesized cascade (mirror `synth_drop_cascades`), a call at every copy site on both backends | P1 | drop gate re-baselined on the lease oracle, both backends, `LOFT_POISON`; the matrix's `OpCopy` count per cell | Open |
| **P5** — remove the machinery that moved a release across a copy, one decider at a time | `scopes::copy_moves_drop_from`, the per-path hand-off flags, the caller-record mark | drop gate and the matrix unchanged after each removal; `introspect_diff` names exactly the removed ops | Open |
| **P6** — `(H-Elide)`: an elided copy skips its `OpCopy` and the matching drop | C86, `alias-where-correct.md`, `src/lease.rs`'s liveness pass | matrix cells where elision fires count hooks consistently; `LOFT_LINK_WIDEN` on and off agree on every observable value | Open |

## P2 design — the report and its completeness

**Superseded for validity (2026-09-15).**  P2a built `src/lease.rs` as the one home for the first
version's question — is the copied value used after the copy? — as a backward liveness pass over the
post-scope IR: both arms of every `if`, loops to a fixed point, `break`/`continue`/`return`, the
scope pass's release scaffolding (release calls, `__disp_` snapshots, `__hoff_` flags) recognised by
markers the author cannot write, and a use as a read of the variable or of a variable that views it.
It was verified against that version's verdicts on all 275 cells and falsified twice.  Under the rule
read off the line, legality no longer asks that question; the pass stays for P6, where liveness may
decide an elision.

**What P2r keeps.**  The census's site enumeration (binds, copies, whole-tuple copies, the returned
tuple, returns of a parameter) and the manifest check (`copy_manifest::lease_uncovered`: every copy a
generator emits carries a census verdict, a call-result bind covered by the callee's `return`
verdict, a parser record whose destination the scope pass removed not counted).

**What P2r changes.**  The verdict: a site is refused when its SOURCE is an existing value (a user
variable, parameter, loop variable, `match` binding, member, or member of a call result, resolved
through compiler temps) and its position is not one `(H-Move)` names — fresh values, a `return` of
what the function owns, a block yielding its own variable.

## Phase ordering

1. **P0 before anything:** the census says how many programs and libraries this touches.
2. **P1 before code**, so every later phase closes a named deviation rather than inventing one.
3. **P2 and P2r before P3:** the refusal is proven complete as a report before it can fail a build —
   a whole-record bind and a call-return bind are minted at emission time (loft#774), so a refusal
   built on the IR alone would miss them.
4. **P4 after P3:** with refusal as the default, `OpCopy` is an opt-in the matrix can test type by
   type.
5. **P5 and P6 last**, as removals and an optimisation measured against the finished rules.

## Open design questions

1. **The hook's shape.**  In place on the new copy (`fn OpCopy(self: τ)`, fixing fields after the
   byte copy) is proposed; a constructor form (`-> τ`) would need a return ABI at every copy site.
   The name does not collide with the internal `OpCopyRecord` family (grep, 2026-09-15).
2. **The refusal's error text** names the copy and the rewrite: use the value where it is, pass it,
   build it where it belongs, return the owner.  It never names a later line, because no later line
   decides the verdict.
3. **A copy the compiler inserts on its own** — answered by the rules: a view of a droppable member
   never materialises (`(H-View-Drop)`), and a member of a call result bound or placed is a written
   copy the author sees on the line.
4. **Byte moves that are not copies**: a vector's growth, a keyed collection's rebalance, a store
   compaction.  None may run `OpCopy`; each is confirmed by a matrix cell.
5. **Across threads** (`par`): a value copied into a worker's store is a copy — `OpCopy` runs, or the
   type refuses.
6. **A non-droppable type that must not be copied** (a `unique struct` modifier, beside `value struct`):
   left out until a use case asks for it.
7. **SUPERSEDED — what `return s.h` and `return p` mean.**  Decided 2026-09-15 (owner) as refused,
   and confirmed for parameters ("refuse parameters in the callee"); the rule read off the line gives
   the same answer for both, without the liveness reasoning behind the first decision.
8. **The release snapshot.**  A reassignment of a droppable deep-copies the displaced record into
   `__disp_N` so its hook can run after the statement (`Scopes::displaced_drop`).  That is a runtime
   copy no rule asks for.  It must neither run `OpCopy` nor be refused; P5 decides whether it
   survives at all.
9. **DECIDED for droppables — a tuple literal copies its members in.**  `t = (tt.0, 1)` copies (and
   releases twice today) while `t = (s.h, 1)` is a view; under `(H-Copy-Refuse)` placing an existing
   droppable in a tuple literal is a copy, so both are refused.  Whether a tuple literal copies a
   plain member is a `tuples.md` `(T-Cons)` question outside this plan.
10. **DECIDED 2026-09-15 (owner) — a member does not move out of a container, dying or not.**  The
    compiler can move `open_session().conn` out statically (`OpDropAllExcept`), and a build of that
    move was requested; the owner then withdrew it: *"we are inching closer to rust here where correct
    and incorrect code starts to look almost identical to people without years of experience in the
    language … That people have to know how loft interprets code to be able to assess if what they
    wrote will be correct"* — and, on the corner case, *"there is almost always a clean workaround in
    user code … I still want to have libraries that are readable and can be understood from code."*
    So `c = open_session().conn` and `return s.conn` are written copies, refused with a message that
    shows the rewrite.
11. **A returned local tuple** lowers to a `synthetic_tuple_return` block (a hold `__ref_3 = t`, then
    one copy per member); the census recognises it as one whole-tuple site.  Under `(H-Move)` it is a
    `return` of the owner, legal.

## Cross-arc dependencies

- `heap.md` D-heap-1 (closed, reclassified) and D-heap-7 (one cell left): the rule read off the line
  turned nearly every shape they carried into a refusal.
- C86 and `plans/102-stability-contract/alias-where-correct.md`: P6 is the drop half of the
  transparent-link widening.
- The post-scope lint stage (`976680b3b`): the report runs there, on every path.

## See also

- `doc/claude/formal/heap.md` § Drop — `(H-Drop)`, `(H-Move)`, `(H-Copy-Refuse)`, `(H-View-Drop)`,
  D-heap-7, D-heap-8, D-heap-9, D-heap-11.
- `doc/claude/formal/binding.md` — `(B-Copy)`, `(B-View)`, `(B-Ref-Reshape)`;
  `doc/claude/formal/calls.md` — `(F-ParamHeap)`.
- `doc/claude/DESIGN_DECISIONS.md` C121 (this decision), C86 (whole-value binds copy), C111 (the
  drop cascade).
- `doc/claude/INTERFACES.md` § Running at scope end — `OpDrop`.
- `src/lease.rs`, `src/copy_manifest.rs` — the report and its completeness check.
- [`@PLN163`](https://github.com/loft-lang/plans/issues/163).
