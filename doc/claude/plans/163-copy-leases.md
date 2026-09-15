<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 163 — Copy leases: a type with `OpDrop` says how a copy gets its own lease, or refuses the copy

## Status

Active — P0 and P1 done; P2 (the refusal as a report) next.  Tracked as
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
| **P2** — the refusal as a REPORT (no error): at every copy site of a refusing type whose source is used afterwards | `copy_manifest::Origin` (both backends' emitted copies) + `ParserMaterialise` | the manifest guard: every emitted copy of a refusing type is reported or proven a move; falsified by disabling one site | Open |
| **P3** — the report becomes a compile error; the published-library gate read row by row | P2 | `tests/scripts` refusal cells (`@EXPECT_ERROR`) on both backends; every library break is a real double release today, or the rule is wrong | Open |
| **P4** — `OpCopy`: signature check (mirror `check_drop_signature`, `definitions.rs`), synthesized cascade (mirror `synth_drop_cascades`), a call at every copy site on both backends | P1 | drop gate re-baselined on the lease oracle, both backends, `LOFT_POISON`; the matrix's `OpCopy` count per cell | Open |
| **P5** — remove the machinery that moved a release across a copy, one decider at a time | `scopes::copy_moves_drop_from`, the per-path hand-off flags, the caller-record mark | drop gate and the matrix unchanged after each removal; `introspect_diff` names exactly the removed ops | Open |
| **P6** — `(H-Elide)`: an elided copy skips its `OpCopy` and the matching drop | C86, `alias-where-correct.md` | matrix cells where elision fires count hooks consistently; `LOFT_LINK_WIDEN` on and off agree on every observable value | Open |

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
   `c = acquire(); …; return c` instead.  Written into P1 as two more consequences.  A PARAMETER
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
