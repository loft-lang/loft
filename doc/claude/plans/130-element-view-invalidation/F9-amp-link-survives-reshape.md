<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# F9 — a `&` link always writes through; reshaping under one is REFUSED (D-bind-8)

**Status — SHIPPED (2026-08-05), all five steps.** Tracker:
[loft#779](https://github.com/loft-lang/loft/issues/779) · rule
[`formal/binding.md` B-Ref-Reshape](../../formal/binding.md), which replaced deviation D-bind-8
(OPEN is back to **0**) · lock-ins `b_ref_reshape_*` in `tests/parse_errors.rs` ·
`tests/scripts/149-reference-survives-callee-reshape.loft` · boundary
[`probes/40-reshape-refusal/`](probes/40-reshape-refusal/README.md).

## The rule

Two halves. The first is already in the spec; the second is the maker's decision of 2026-08-05
and is a spec ADDITION.

1. **B-Ref-Alias, unchanged and unconditional** — *"the `&τ` annotation makes ANY binding a live
   LINK to the source"*, so every write through a `&` reaches the source. Restated by the maker:
   *"A reference to any structure variable or otherwise should allow for writing it too. In all
   cases."*
2. **NEW — removing from a container while a `&` link into it is open is REFUSED AT COMPILE
   TIME.** Maker, 2026-08-05: *"The removal of anything from a structure (vector for example)
   that has an open `&` relation (for us an edge case) should be forbidden on compile time."*

Together these are total and need **no runtime machinery at all**: a `&` always writes through,
because the one program shape where it could not is rejected before it runs. This is the rustc
bargain in loft's spelling — where rustc refuses a mutation while a borrow is live, loft refuses
the removal; and it is affordable precisely because the maker classes it an **edge case**, so no
real program pays for it.

**What does NOT change: @PLN130 F2 stays exactly as shipped for a PLAIN bind.** `c = v[0]` copies
(or mimics a copy), so materialise-plus-advice remains right for it — maker: *"When we copy data
… then we obviously do not want write through."* The whole arc is about `&` only, and the two
paths must end up disjoint:

| binding | container reshaped while it is live | outcome |
|---|---|---|
| plain `c = v[0]` | yes | materialise + advice (F2, shipped, unchanged) |
| `c = &v[0]` | yes | **compile-time error** |
| `c = &v[0]` | no | live link, writes through (already correct) |

## Root cause — the `&` is thrown away before any decision can use it

`c = &v[0]` and `c = v[0]` compile to **byte-identical IR**:

```
c(1):ref(Box)["v"] = OpGetVector(v(1), 16i32, 1i32);      // both spellings
```

(verified with `loft introspect`; the only diff is line numbers). For a **struct-typed**
projection the two were equivalent under B-View — both alias — so the parser drops the `&` as
redundant, silently. Harmless until F2 made a view materialise on a reshape: from that moment the
spellings stopped being equivalent, and the discarded `&` became load-bearing information that no
longer exists.

D-bind-0 closed on *"`&τ` is now `Type::RefVar`, a reference type the variable carries"*. True for
**parameters** (they render `&vec<ref(N)>`); not true for a local struct projection.

**Measured today** (probe 38 / `LOFT_VAR_TABLE`), all three silently dropping the write:

| | shape | today | why |
|---|---|---|---|
| **(a)** | `c = &v[0]; v.remove(2)` — element does NOT move | 11, write dropped | materialise fires on a `&`, nothing marks it as one |
| **(b)** | `c = &v[2]; v.remove(0)` — element moves 2→1 | 33, write dropped | same |
| **(c)** | `&` param, callee does the `remove` | 33, **no diagnostic at all** | materialise does not fire (a `&` param IS distinguishable); nothing adjusts the link |

Under the decided rule **all three become compile-time errors**, so the fix direction is one
mechanism (detect + refuse) rather than three.

## What the refusal decision removes from this design

An earlier draft of this file had a *"make the link follow its element"* step — emit
`if link.pos > removed.pos { link.pos -= elem_size }` at each removal. It is deleted. It was the
only step with real risk (runtime arithmetic per link, a disjointness requirement against the
plain path, and an undecided sub-case for a link to the *removed* element). The refusal answers
all of that by construction, and the undecided sub-case evaporates: there is no "link to a
removed element" if the removal is rejected.

Recorded because the reasoning is worth not re-deriving: following was *feasible* — a vector stays
DENSE (F3 decision 1), so the new position is arithmetic rather than a lookup, and the live-link
set is already computed. It is simply not worth it for an edge case.

## Steps

Each is independently landable, has its own gate, and leaves the tree green.

### Step 1 — make F2's materialise LIVENESS-aware ✅ DONE (2026-08-05)

**Shipped.** `collect_views_to_materialise` now answers both causes and the flat
`reshaped_containers` set is gone; `tests/scripts/148-view-liveness-across-reshape.loft` is the
CI guard, `probes/39-f2-liveness-boundary.loft` the 12-cell boundary. The merge blocker is
cleared. Details in § Step 1, as shipped, below.

Before any `&` work: F2's reshape path keyed on the **container** and was **order-blind**. It
materialised any view of a container reshaped anywhere in the function, even one that is dead
long before the removal. Measured, plain bind, both backends:

```loft
c = v[0];
c.n = 99;          // the view is DEAD from here
v.remove(2);
// v[0].n == 11 on this branch;  == 99 on the installed mainline binary
```

**This is a regression F2 introduced on this branch**, not a pre-existing gap — mainline has no
materialise and lands the write. Two closure-bar violations at once: a **lost write**, and
**untrue advice** (*"`v` is modified while `c` is in use"* when `c` is not in use). F2 is not on
`main`, so nothing has shipped — which is exactly why this must be fixed before the branch merges.

The fix is available and already built: **F8's `collect_views_to_materialise` is keyed on the
VIEW and is liveness-aware** (it walks blocks in order and marks only a view live across the
event). The reshape cause simply does not route through it — it still consults the flat
`reshaped_containers` set. Route it through the same walk, treating a `remove` / `#remove` of the
container as the event instead of a re-establishment.

- **Fixes:** the lost write and the false advice for plain binds; reduces over-copying.
- **Also the prerequisite for step 3**, whose refusal condition is *"a `&` link LIVE across the
  removal"* — the same liveness question.
- **Gate:** `f2_dead_view_before_removal_keeps_writing_through` (lock-in, `tests/parse_errors.rs`)
  flips to PASS; `145`, `146`, `201`, `774`, `147` unchanged; the advice fires only where the view
  really is live; both backends; `LOFT_STRICT_STORES` clean; full suite green. Re-run probe 03–07
  and 29, which are F2's own evidence — they must stay green, and they are the cells that prove
  the liveness walk did not go too far the other way.
- **Risk: low-medium.** It narrows when a copy happens, so the failure direction is a stale view
  coming back — which probes 03–07 and 29 are exactly the guard for.

#### Step 1, as shipped

**The design was right about the destination and wrong about the vehicle.** F8's walk is keyed
on the VIEW and walks blocks in order, but *order is not liveness*: it marks a view bound before
the event, which is exactly what cell L1 (`c = v[0]; c.n = 99; v.remove(2)`) is. Routing the
reshape cause through it unchanged would have fixed L5 and L8 and left the headline regression
in place. So the walk gained the missing half rather than merely a second caller:

> A disturbance only **SHAKES** the open views of that container. A later read or write of a
> shaken view is what **condemns** it. A view whose last use precedes the disturbance keeps its
> alias and writes through.

That is one rule for both causes, so `reshaped_containers` is deleted as a field and the
per-statement helper feeds the same walk; `views_to_materialise` became a
`HashMap<u16, ViewCause>` so the two advice lines still read differently (`Reshaped` wins when
both apply). The cause split is now carried as data instead of re-derived at the strip site from
*"is the container in the reshaped set"*, which was the thing that could disagree with itself.

**One measurement decided the shape of the walk, and code-reading would not have found it.**
The first version shook and then read uses over the WHOLE statement before recursing into it —
so for the function's top-level block it re-scanned every statement inside, and `c.n = 99`
(which runs *before* the removal) counted as a use *after* it. L1 stayed broken with the
analysis "working". One env-gated print of the condemning statement named the block instantly;
the fix is to handle uses at LEAF granularity and let the recursion carry order. Kept as the
reason `walk_stmt` looks the way it does: the whole-statement shake survives only for `Loop`
(where the next iteration really does put the disturbance before the body's uses) and for forms
the walk cannot descend into (a `match` arm), where coarse-in-both-directions is the safety net.

| | before | after |
|---|---|---|
| L1 dead before the removal | 11, write lost | **99** |
| L5 bind in a block that closed | 11 native / 99 interp — a backend split | **99** both |
| L8 removal before the bind | 11, write lost | **99** |
| L10 one dead + one live view | 11022, both stripped | **99022** |
| L2/L3/L4/L6/L7/L9 (F2's own evidence) | unchanged | unchanged |
| advice lines emitted | 13, four of them untrue | **9**, all on views genuinely in use |

Probes 03–07, 26, 28, 29, 30, 35, 36, 37 and 38 re-run clean on both backends;
`LOFT_STRICT_STORES` reports nothing on probe 39, so no cell passes by leaking.

**Known lower bound, recorded rather than papered over:** a view bound inside a nested block and
used on a LATER iteration of an enclosing loop is not tracked, because the frame closes with the
block. A disturbance anywhere inside a loop shakes every view held from *outside* it before the
body is walked, so the ordinary loop shapes are covered; this one keeps today's behaviour.

### Step 2 — give `&` a representation that survives to the check (INERT) ✅ DONE (2026-08-05)

**Shipped.** `Variable::amp_link` + `set_amp_link`/`is_amp_link`, set in
`parse_assign_op` at the point the `&` previously evaporated — the branch where neither the
scalar-stack nor the heap-ref lowering claimed it and the source type is a struct. Rendered as
an `amplink` column by `LOFT_VAR_TABLE`, which is the only way to tell the two spellings apart.
Still INERT: no decision reads it.

**Calibrated in both directions, because a marker that has never been made to fire and to stay
quiet is an unread dial** — and without this the gate below would have proved only that dead
code changes nothing:

| shape | `amplink` |
|---|---|
| `c = &v[0]` · `c = &o.inner` · `c = &v[0].inner` | **set** |
| `c = v[0]` · `c = o.inner` (plain projections) | quiet |
| `b = &a` (scalar), `p = &o` (whole value), `d = &v` (vector) — the `&`s that already lower | quiet |

**Gate: both halves byte-identical**, on a corpus carrying all three groups plus `&` parameters
(`scratchpad/amp-corpus.loft`) — emitted IR + bytecode, and generated native Rust. Each was
first shown DETERMINISTIC across two runs of the same binary, so the comparison means something;
that check earned its keep, because the first attempt diverged on nothing but the program cache
serving run 2 (`LOFT_NO_CACHE=1` is required) and the run before that compared a binary which,
copied out of `target/release/`, could find neither the stdlib nor its deps directory and so
compiled nothing at all. Behaviour identical on both backends.

Mark the binding when the parser sees `&` at a projection bind — a flag on the variable, **not** a
type change. Nothing reads it yet.

- **Why a flag, not `Type::RefVar` for locals.** Making a local RefVar would re-route every read
  and write through the double indirection params use — slower on every access, which is exactly
  what loft's own advice warns about for a redundant `&` param — and would touch every codegen
  dispatch site. A marker bit keeps today's single-indirection representation and informs only the
  new check. Cheapest thing that carries the fact.
- **Gate (behaviour-preserving refactor, per the loft-codegen skill):** emitted IR **and**
  generated native Rust **byte-identical** before/after, on a corpus with `&` locals, `&` params
  and plain projections. The flag is observable only via `LOFT_VAR_TABLE` / `LOFT_DEBUG_F8`. Full
  suite green.
- **Risk: low.** Adds a fact, changes no decision.

### Step 3 — refuse a removal while a `&` link into that container is LIVE ✅ DONE (2026-08-05)

At each `remove` / `#remove` site, if any `&`-marked link into that container is live across it,
report a compile-time error naming both the removal and the link.

- **Liveness is the condition, not existence** — the rustc rule. A `&` link that is dead before
  the removal is no conflict, and `c = &v[0]; c.n = 1; v.remove(0);` must keep compiling. The F2
  liveness walk (`collect_views_to_materialise`, added for F8) already computes exactly
  "is this binding live across this operation", so this reuses it rather than inventing one.
- **Fixes:** (a) and (b).
- **Risk: medium**, and it is a COMPATIBILITY risk rather than a correctness one — see below.

#### Step 3, as shipped — and the ordering constraint DISSOLVED

Shipped as producer 1 of `scopes::reshape_refusals`: the same `ViewWalk`, filtered to
`is_amp_link` bindings condemned by a `Reshaped` cause. Both same-function lock-ins report with
the caret on the removal; `145`, `146`, `201`, `774`, `147` unchanged.

**The design's "single most important ordering constraint" turned out not to exist.** It said
exempting `&` from the materialise is safe only in combination with the refusal, because on its
own it would turn case (b)'s dropped write into a write to the vacated slot. True — but the
exemption is not needed at all. The refusal fires on exactly the set the materialise would have
copied (same walk, same cause, filtered by the same flag), so every program that would have been
exempted is a program that no longer compiles. The materialise is left untouched, which is the
smaller change and removes the pairing hazard by construction rather than by sequencing.

### Step 4 — the cross-frame case (c) ✅ DONE (2026-08-05)

The callee removes from a `&vector` param while the caller holds a reference into that container.
Checked **at the call site**, which is the only place the two are known to name the same store:
inside the callee they are two unrelated parameters, and refusing there would reject sound
programs. The per-definition fact it needs is `removed_ref_params` — *"this callee removes from
`&` param k"*, the mirror of F8's `reassigned_ref_params`.

#### Step 4, as shipped — two things the measurement changed

**The `&` is not the carrier, and loft#779's own boundary table said it was.** The issue's row A2
reads *"same as A1 with the `&` dropped — plain param copies (@C86), so nothing to lose"*. It does
not copy. Probe 40 cell **X9** is one line — `fn w(t: Box) { t.n = 99 }` called as `w(v[2])` —
and it writes **99 into the caller's `v`**. A plain struct parameter aliases the caller's element
exactly as a `&` one does, which is precisely what loft's own `warn_redundant_amp` advice tells
authors (*"field mutation already propagates to the caller without it"*). So the cross-frame
lost write happens with or without the `&`, and refusing only the `&` spelling would have meant
an author who takes loft's advice and drops it trades a compile error for a silent lost write —
the worst possible pairing. The call-site check therefore keys on the ALIASING relation.

That is not inconsistent with step 3 being `&`-only: a plain LOCAL bind does not alias across a
reshape, because F2 materialises it. At a PARAMETER there is no bind site to materialise at.
One sentence covers both: **the refusal fires where the binding still aliases the container.**

**One frame deep was a scoping choice, and the closure is cheap, so it is gone.** The design
planned to leave a removal two calls down alone with an `#[ignore]`d lock-in. That hole is one an
author would trip over by REFACTORING — extract `all.remove(0)` into a helper and the refusal
disappears along with the diagnostic. `removed_params_map` now closes over the call graph with a
worklist built in the same pass as the direct removals (*"caller `c` passes its own `&` parameter
`s` as callee `e`'s parameter `k`"* edges), so cell X7 is refused too. Cost is one walk of each
body plus the propagation. Only a callee reached through a runtime fn-ref is still invisible.

**Where the check lives, and the trap in getting there.** `Parser::check_reshape_under_reference`,
post-pass-2 beside `check_subrule_wellformedness` and for the same reason: the question is asked
of a *callee's* body, which is only complete once the file is parsed. The first cut filtered the
definitions by `source != STD_SOURCE`, to avoid re-walking the stdlib on every load —
**and that made the check a silent no-op for the entire Rust test suite**, because
`Parser::parse_str` never leaves `STD_SOURCE`. It fired on a file and not on a `code!`, which
reads exactly like "the feature works" until a lock-in disagrees. Every definition is checked
now; measured cost is in the noise (~8 ms on a hello-world parse, unmeasurable on a real one)
and it means the stdlib is checked too.

- **Fixes:** (c), probe 26, probe 38 cell A1; closes loft#779.
- **Gate:** all six refusal lock-ins report; the three positive lock-ins and
  `tests/scripts/149-…` compile and write through on both backends; probe 40's 25 cells match
  their hand-computed values; `LOFT_STRICT_STORES` clean; full suite green (3733).

### Step 5 — close the deviation and state the rule ✅ DONE (2026-08-05)

- **B-Ref-Reshape** is now a rule in `formal/binding.md` beside B-Ref-Alias, and D-bind-8 is
  closed with its evidence; `OPEN` is **0** once step 6 below lands.
- [OWNERSHIP_MODEL.md § A view lasts as long as the thing it names](../../OWNERSHIP_MODEL.md) and
  the @C86 lifetime paragraph in [DESIGN_DECISIONS.md](../../DESIGN_DECISIONS.md) carry the shipped
  two-row rule (plain → materialise; reference → refused) instead of the *"known gap"* wording.
- `LOFT.md`'s user-facing paragraph gains the error and the index workaround.
- Three gaps the formal validation surfaced and closed: `collections.md` had no removal rule at
  all (`Col-Remove` now states the density that this whole rule rests on), and `binding.md`
  B-View / `heap.md` H-View both asserted an UNCONDITIONAL alias that @PLN130 F2 had already
  made conditional — the rules were incomplete, not the code, so both gained the materialise
  qualifier (`H-Materialise`).

### Step 6 — the other two disturbances (D-bind-9) ✅ DONE (2026-08-05)

**The rule had three producers and the first cut enforced one.** B-Ref-Reshape was written from
the maker's sentence, which named REMOVAL. `B-Disturb` names three events that end the place a
reference points at, and the other two still silently downgraded a `&` to a copy. Closing
D-bind-8 while they held was an accounting error: the deviation named all three mechanisms of one
rule and the sign-off covered one.

**What found them was a sweep, not a cell.** 14 shapes of `&` — whole struct, whole vector,
element, field, nested, keyed non-key, keyed key, local reassign, callee reassign, `&` param
mutate, `&` param rebind, loop, branch, overwrite-in-place — each asserting the single thing `&`
promises: that the write reaches the source. Twelve honoured it; two did not, identically on both
backends and each with a *"copied out of"* advice line:

```loft
c = &s[30];  c.key = 5;                                        // s[5] ABSENT
c = &bx.inner;  bx = Mid { inner: Box { n: 22 } };  c.n = 99;   // bx.inner.n == 22
```

The generalisable point: **a rule with more than one producer needs a sweep over the producers,
not a boundary around the one that was reported.** The removal producer had a 25-cell boundary
(probe 40) and the other two had nothing, because the issue only named the removal.

Both now refuse. The reassignment arm is the same liveness walk with the cause filter dropped —
`ViewWalk` already returned `Reassigned` with its container, line and callee, so it was a filter
change and a second message. The re-key arm refuses at `note_key_field_write` where the base
`is_amp_link`; it needs no liveness question, because the key write IS the use, and it needs a
different remedy in the message, because there is no "move it later" — the write itself is what
destroys the place, so the way out is to re-insert.

**Neither needed a new decision**, and that is what the maker's 2026-08-05 principle bought:
*"we can decline code where we cannot create a safe implementation"* covers all three producers
at once, where the original sentence covered one. The earlier scoping note in this file —
*"a re-key is a write, not a removal … needs its own decision"* — is answered by it.

- **Gate:** the 12 honouring shapes still honour on both backends with `LOFT_STRICT_STORES`
  clean; lock-ins `b_ref_reshape_rekey_through_amp_link_is_error` and
  `b_ref_reshape_container_reassign_under_amp_link_is_error` plus their positive twins (a NON-key
  field still writes through, a reference dead before the reassignment still does); probe 40's 25
  cells unchanged; full suite green. D-bind-9 opened and closed the same day; `OPEN` is **0**.

## The compatibility question — ANSWERED 2026-08-05: cleared, and the doc urges it

**Adding an error can stop compiling a program that compiles today** —
[COMPATIBILITY.md](../../COMPATIBILITY.md), and *"compatibility is ABSOLUTE"*. Checked against
the freeze machinery rather than assumed, as this section demanded:

**`manifest::CONTRACT_VERSION == 0` — the freeze has NOT happened.** Its own doc comment calls
0 the pre-freeze baseline, and `tests/layout_golden.rs` keeps the flip-gate inert while it holds.
So the refusal lands **unconditionally** and needs no @PLN113 edition valve. That was the branch
this section was written to discover, and it is the cheap one.

**The stronger finding: post-freeze this becomes impossible, so it is now or never.**
[COMPATIBILITY.md § The error surface is one-directional](../../COMPATIBILITY.md) is explicit —
loft may always *drop* an error and, after the freeze, may **never add one**. That *inverts* the
usual pre-freeze disposition: *"be strict now, because you can always relax later but never
tighten,"* and every place loft is too permissive is a **last-chance-to-add**. A silently
dropped write through a `&` is exactly the "silently accepts something dubious" shape it names.
So step 3 is not merely permitted, it is on the pre-freeze audit's own list.

**The one caveat that genuinely applies, and why it does not block.** The same section says the
*first* resolution of a would-be-error is **a rewrite to correct function, not an error** (the
@C80 model: give it a sane defined behaviour instead of rejecting). For this shape that rewrite
exists and is named above — make the link FOLLOW its element, which a dense vector makes
arithmetic rather than a lookup. It was considered and **declined by the maker** as not worth
runtime arithmetic per link for an edge case. That is the required "consciously accept" step,
taken by the person entitled to take it, and recorded here so the freeze audit does not have to
re-derive it.

Two supporting arguments, unchanged and still true:

- Every program the error rejects is **already silently wrong** — it loses a write with no
  diagnostic. Refusing it converts silent breakage into a compile-time message, which is the
  direction the closure bar demands, not a regression.
- It is an **edge case** by the maker's own classification, so the blast radius is small.

**Measured, after the fact: the blast radius is zero on this repo.** The full suite (3733 tests,
including every library fixture) is green with the refusal active, and the stdlib does not trip
it. The one program shape that lost something it had is probe 40 cell **X3** — a callee removing
an element BELOW the reference, where the element never moves and the write lands today. It is
refused anyway, because the rule is about an OPEN reference and not about whether this particular
removal would have invalidated it; deciding otherwise needs the removal index and the reference's
index, which are usually only known at run time. Its same-function twin (cell S1) is already
broken today, so keeping the two answers consistent is worth more than saving X3.

## Deliberately out of scope

**A `&` link across a keyed-collection RE-KEY.** F4 treats a key-field write as a reshape and
materialises the view; for a `sorted` the record genuinely moves. Same class, and under the
decided rule it should presumably be refused too — but a re-key is a *write*, not a removal, so
the maker's sentence does not literally cover it. Needs its own decision. Recorded so nobody
assumes step 3 covered it.

## Where the shipped rule lives

1. `formal/binding.md` § B-Ref-Reshape — the rule; `formal/binding-history.md` carries D-bind-8's closure record.
2. `probes/40-reshape-refusal/README.md` — the 25-cell boundary, with the *before* column measured
   on the pre-fix binary and the three cells (X9, S4, X3) that decided the shape of the fix.
3. `tests/parse_errors.rs::b_ref_reshape_*` — six refused shapes and three positive ones;
   `tests/scripts/149-reference-survives-callee-reshape.loft` — the runnable positive cells.
4. `scopes::reshape_refusals` — the analysis (two producers);
   `Parser::check_reshape_under_reference` — the reporting.
