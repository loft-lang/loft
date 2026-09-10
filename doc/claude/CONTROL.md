<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# CONTROL.md — where the programmer is, and is not, in control

loft deliberately hides bookkeeping: ownership is internal ([C79](DESIGN_DECISIONS.md)),
a fault degrades to null and the run continues ([C80](DESIGN_DECISIONS.md)), freeing
happens at owner death. That is the design, and it is not what this document is about.

This document is the **census of the remainder** — the places where loft decides
something the source does not say, ranked by whether the programmer can find out and
whether they can say otherwise. The principle it serves is
[GOALS.md § Legible cost](GOALS.md): *a performance-critical decision is never abstracted
away; what is automated stays deterministic.* Hidden is fine. **Hidden, unmeasured, and
growing** is not, which is why this is a census with a number rather than a list of
complaints.

## The test

Two questions, asked of every mechanic:

1. **Can the programmer SEE it?** Is there a signal at the site where the decision lands?
2. **Can the programmer SAY it?** Is there a source-level way to demand the other outcome?

Neither = a real gap. SEE only = legible, usually fine. **SAY only = the dangerous one** —
steerable, but only by someone who already knows it exists. A third axis decides how fast
a gap can grow: is the decision **derived from the source** (inspectable by reading the
program) or from the **environment / data** (not reproducible by reading it)?

## The census

### Neither SEE nor SAY

| # | Mechanic | Where | Note |
|---|---|---|---|
| 1 | **A fault erases its own cause.** null is contagious and silent by design; the value tells you where it *surfaced*, never where it *arose*. | `formal/operational.md` `(E-Uncomp-NN)`, C80, C85 | The largest single control loss, and it is now smaller by exactly one half. The SECOND half of this row — *"a non-nullable width takes the type's default instead, so the fault becomes indistinguishable from an answer"* — is closed: `@FR-E-Uncomp-Seen` makes that store testable at the site, `x += 10; if !x { … }`, with the value unchanged ([@PLN152](plans/152-validity-flag-null-model/README.md)). What survives is PROVENANCE — a null still says where it surfaced and never where it arose, which is **P1** below. loft#1296, loft#1297 |
| 2 | **Native optimisations gated only by env vars** — vector-header hoist, element fuse, sibling-block store confinement. Worth ~2–3× on a loop and 4× on the store watermark; the hoist gate is an allow-list, so an op missing from it silently costs the optimisation. | `src/generation/hoist.rs`, `recover_backer` | `LOFT_NO_VECTOR_HOIST` / `LOFT_NO_ELEM_FUSE` / `LOFT_NO_CONF_RECOVER` are process-wide, not program properties. Nothing at the loop says whether it applied. |
| 3 | **A copy can never fail a build.** `avoidable-copy` is `advice`, and advice has no deny switch by design — so *"this hot loop must not copy"* is unwritable and un-gateable in CI. | [DIAGNOSTICS.md](DIAGNOSTICS.md), [COPY_DIAGNOSTICS.md](COPY_DIAGNOSTICS.md) | |
| 4 | **Bucket 5 hides most copies.** Copies whose source is a compiler temp route to `Internal` and leave the user report — 139 of 173 on the first 371-script survey. The promised attribution back to the user value behind the temp is not built. | `src/use_analysis.rs` | |
| 5 | **No capacity control for four of six collection kinds.** `reserve` takes `vector` and `hash`; `sorted` / `index` / `spatial` / `trie` are refused *"has no capacity to set"* — but they sit on an arena that grows by 7/3 and never shrinks. | [STDLIB.md](STDLIB.md) § `reserve` | Measured: a `sorted` of 20 000 records reads 57 % used, **43 % inner slack**. The refusal's wording is also wrong about the backing store. |
| 6 | **Declaration order is promised irrelevant and never tested.** [LOFT.md](LOFT.md) says a file may hold its declarations in any order; permuting top-level items across 200 test scripts produced three live defects on `main`. | [STABILITY_REDFLAGS.md](STABILITY_REDFLAGS.md) § Result 5 | Control silently removed from something the reader was told they had. |

### SAY but no SEE — the class most likely to grow quietly

| # | Mechanic | Note |
|---|---|---|
| 7 | **The backend is chosen by the environment.** `loft p.loft` compiles via rustc *when available, otherwise interprets*. That changes speed by orders of magnitude, changes the `par` path, decides whether the profiler works, and occasionally changes semantics. | `--interpret` / `LOFT_REQUIRE_NATIVE=1` are process flags. **A program cannot declare "I require native"** in `loft.toml`. |
| 8 | **Placement.** One manifest line turns a call into IPC or a network round trip, deliberately indistinguishable at the call site. | [PLACEMENT.md](PLACEMENT.md). The invariant is well defended; it is still the largest cost-per-call swing in the system, and the line belongs to the library author, not the consumer. |
| 9 | **Lazy stores.** `persons[42]` performs a SELECT or an HTTP range read on a miss; only `store_lazy_faults()` reports it, as a whole-program counter. | [LAZY_STORES.md](LAZY_STORES.md). This is exactly the N+1 pattern [GOALS.md](GOALS.md) names as the generating failure — *correct at low scale, fatal at production* — with no site-level signal. |

### Bounded, recorded for completeness

10. **Closure capture mode is not selectable** — scalar by value, heap shared, decided by
    type (`formal/closures.md` `(L-CapScalar)` / `(L-CapHeap)`). Consistent with the
    parameter rule, so learnable; there is no way to say "snapshot this struct".
11. **`MAX_CALL_DEPTH = 10 000`, fixed** (`src/state/mod.rs`). Not settable from source or CLI.
12. **The live/debug tier ships in every `--native` artifact** unless `--lean`; a
    `--native-release` program and a library cdylib are lean by default (no live channel,
    depth-only frames) — the shipped tier, NATIVE.md § Optimisation tiers.
13. **`par` over a `hash` is non-deterministic in result order** — documented, and `sorted`
    is the cure. Handled correctly; listed so the axis is not re-discovered.

## The number to track

> **Mechanics with neither SEE nor SAY: 6** (items 1–6, measured 2026-09-02).
> **SAY-but-no-SEE: 3** (items 7–9).

Re-measure per release. The first number is the one that must not grow; the second is the
one that grows quietly, because each of its members individually looks like a deployment
convenience.

**What the number is not.** 53 diagnostic codes ship (27 warning / 15 advice / 11 error),
and a large share exist precisely to make a hidden mechanic visible — `omitted-field-zero`,
`variant-field-unchecked`, `shadowed-by-method`, `undeclared-dependency`, `lost-write`,
`linked-group-*`. That count going up is the SEE side working. **Nothing measures the SAY
side**, and several cures are themselves invisible mechanics wearing a better label — an
`omitted-field-zero` answered by a declared field default moves the silence from the
literal to the declaration. The census counts mechanics, not diagnostics, for that reason.

## The work queue — what to do about all of it

Designed 2026-09-02 after @PLN152, which closed having found that the null capability was
never missing (see its [MEASUREMENTS.md](plans/152-validity-flag-null-model/MEASUREMENTS.md)).
Routed by the lightest workflow that holds each item, not swept into one plan.

### Now — small, self-contained, each one commit

| | item | why now |
|---|---|---|
| ~~**N1**~~ **DONE** | The implicit-narrow store message sent the author to a refusal. `v[0] = (v[0] ?? 0) + 10` says *"cast explicitly with `as u8`"*; following it gets a SECOND refusal saying *"use `u8?` for a checked cast"*. Two hops to a cure the first message could have named. | The commonest way an author meets narrow widths, and it currently teaches the wrong move. One message, one test. |
| ~~**N2**~~ **DONE** | A `??` that discharges the READ read as if it should have helped. In the shape above the author wrote a `??` and it did not apply, because the store's root is `+`. Either say so, or fold it into N1's wording. | Same site as N1; decide together or the two messages will disagree. |
| ~~**N3**~~ **DONE** | `declared_range` cannot see narrow aliases — it returns `None` the moment `forced_size` is set, so a guard reaching for it silently claims nothing. Cost me a wrong no-op in @PLN152 step 2. | A doc comment on both functions naming which spellings each answers. Pure prevention. |
| ~~**N4**~~ **DONE** | `git checkout --theirs` takes the WHOLE file, not the conflicted hunk — it reverted 101 lines of a chapter during this session's join, the second time that has happened here. | One paragraph in [DEVELOPMENT.md](DEVELOPMENT.md)'s join guidance. |
| ~~**N6**~~ **NOT small — moved out** | loft#1308 and loft#1309 are the only issues with no fix anywhere, and neither belongs in this table. **#1308 is actively in progress in `../loft`** (uncommitted `scopes.rs` + `parser/vectors.rs` + a new `tests/escaping_closure_capture.rs`, touched minutes ago) — do not take it. **#1309 is not a small fix**: a repro of the general `!call()`-in-argument shape produces IR identical to the working spelling, so the 24 B push is specific to `exhausted` on a coroutine handle, and observing it at all needs a build with `debug-assertions` flipped back on. It wants the matrix discipline and its own session. | Both were found by review guards rather than by a consumer, and both are invisible to every gate that runs by default — which is the fact worth keeping here even though the fixes are not. |
| ~~**N5**~~ | ~~The sibling branch is missing four stdlib section markers~~ — **already resolved**: the restored markers went to main in the #1307 squash; all four read 1 there. | — |

### Next — an `## Open work` row in the doc that owns it

| | item | home |
|---|---|---|
| **O1** | `D-op-5` — two spellings of a following null-check still report differently. The last null-specific open deviation; `types.md` is at OPEN: 0. | [formal/operational.md](formal/operational.md) |
| ~~**O2**~~ **DONE** | Is `u32` a third case? No — deliberate, and now stated once instead of derived. Its spare code sits at the TOP, where no non-null read tests for it, so an overflow answers `0` like the other four; `IntegerSpec::non_null_reads_null` is the one home that says so, and `@PLN152` step 5 fuses `u32` with the four on that answer rather than on a variant list. | [formal/types.md](formal/types.md) |
| **O3** | No capacity control for `sorted`/`index`/`spatial`/`trie` — item 5 of the census, and the refusal message claims they have *"no capacity to set"* while their arena grows 7/3 and holds 43 % slack. | [STDLIB.md](STDLIB.md) § `reserve` |

### The one plan-sized item

**P1 — null provenance.** Item 1 of the census, unchanged all session and the largest control
gap that survives. C80 makes null contagious and silent by design, so a null tells you where it
SURFACED and never where it AROSE; `--dev-soft-halt` names fault sites but nothing links an
observed null back to the fault that produced it. Everything else on this page is a message, a
row, or a doc comment — this is the one that needs a design.

Its shape is also now better understood than when this page was written: @PLN152 measured that
a per-value bit is affordable only where it is *asked for*, and that ordinary arithmetic
(`integer`/`float`/`single`) keeps a sentinel and needs nothing. A provenance design should
inherit that constraint rather than rediscover it.

### Deliberately NOT queued

Census items 2, 3, 4, 6 and 7–9 stay as measurements, not tasks. Each is a policy question the
owner has not asked for — whether a copy should be able to fail a build, whether a native
optimisation should be expressible in source, whether a program may demand a backend — and
turning a measurement into a task before that call is made is how a census becomes a backlog
nobody agreed to.

## How to re-measure

The census is meant to be re-derived, not trusted:

```bash
# item 3/4 — the copy report and what it suppresses
grep -n "Internal" src/use_analysis.rs

# item 5 — capacity refusals, and the slack a real collection carries
#   a probe that fills a sorted, then prints store_memory()
loft --interpret <probe>.loft

# item 6 — permute a file's top-level items and diff both runs
#   (compare stderr as a sorted multiset: diagnostics emit in declaration order)

# item 7 — what a bare run actually did
loft --help | head -12
```

Items 1 and 2 are read off the formal rules and `src/generation/hoist.rs` respectively;
neither has an instrument yet, which is what puts them in the first table.

## See also

- [GOALS.md](GOALS.md) § Legible cost — the principle this document measures against.
- [STRONG_POINTS.md](STRONG_POINTS.md) § 12 (null provenance) and § 13 (layout control) —
  the two turn-offs items 1 and 5 belong to.
- [plans/152-validity-flag-null-model](plans/152-validity-flag-null-model/README.md) — the
  plan that closed the SECOND half of item 1 (the substituted default is now testable at the
  store); the provenance half is P1 above.
- [COPY_DIAGNOSTICS.md](COPY_DIAGNOSTICS.md) · [PLACEMENT.md](PLACEMENT.md) ·
  [LAZY_STORES.md](LAZY_STORES.md) · [DIAGNOSTICS.md](DIAGNOSTICS.md)
