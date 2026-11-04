<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Facts across calls — one routine, parameterised by data and reach

What the compiler may know about a call from OUTSIDE the callee's body: what the call can
write, what it answers, what its parameters hold on entry.  Every rewrite that relies on such
a fact (`rewrites.md`: `(R-Inputs)`, `(R-NonSentinelParam)`, `(R-RangedCall)`, `(R-PureReuse)`,
`(R-SharedCall)`, `(R-RetAdopt)`, `(R-Destination)`, `(R-AppendTwin)`, `(R-Transient)`, the
header verdict's callee clause, …) reads it from ONE generic routine.  This chapter says what
that routine computes, which way each fact may err, how far it follows calls, and how a fact
proven at a call site specialises the callee.

**OPEN: 3** — the register at the end.  The rewrites themselves stay in `rewrites.md`; this
chapter is the fact source they cite.

## The routine

```
  (Fact-Engine)  one routine answers every question about a call from outside the
                 callee's body:  facts(kind, reach) → a SUMMARY per function (what a
                 call of it does and answers) and, for a downward kind, an ENTRY FACT per
                 parameter (what every call hands it).  `kind` names the expected data
                 (Fact-Kind), `reach` how far the routine follows calls (Fact-Reach).
                 The routine owns the call graph, the order it is walked in, the cycles,
                 the binding of a summary to the arguments of a call, the memo and the
                 trace; a kind owns only its data and what one IR node does to it.  An
                 answer is a pure function of (program, kind, reach): it is computed once
                 per compilation and shared by every writer — the interpreter's generator,
                 the native writer and the C writer read the same answers.
```

**In words.**  Today each rewrite that looks into a callee walks that callee with its own
code, its own cycle rule and its own idea of what a function value or a native does (the table
in § The analyses today).  A fact computed twice can disagree with itself, and a writer that
lacks one analysis lacks the rewrite: the C writer has no `(R-RangedCall)` because its interval
facts are computed inside native's writer.  The routine is the one place the walk, the cycles
and the boundaries are decided; a new analysis is a new `kind`, not a new walk.

## The expected data

```
  (Fact-Kind)    a kind declares, and nothing else:
                 (a) its DOMAIN — the values a fact takes, their order, a bottom and a
                     top; an infinite domain (an interval) also declares its WIDENING;
                 (b) its DIRECTION — MAY (an over-approximation: everything the call might
                     do is in it) or MUST (an under-approximation: everything in it holds on
                     every call);
                 (c) its KEY — what a fact is about: the function, a parameter, a PATH
                     from a parameter (fields and elements, to a declared depth), the hidden
                     result buffer, the result, or a call site;
                 (d) its FLOW — UP (a callee's summary read by its callers) or DOWN (what
                     every caller hands a parameter, read by the callee);
                 (e) its TRANSFER — what one IR node does to the fact, a call excepted: the
                     routine applies a call by binding the callee's summary (UP) or by
                     reading the argument's fact (DOWN), never the kind;
                 (f) its SEED — for a DOWN kind, the assumption a twin may be built on
                     (Fact-Twin);
                 (g) its CYCLE SCOPE — the kinds whose questions share one guard, so a
                     question about a function another kind of the scope is evaluating
                     gets the cycle answer — and its MEMO POLICY, which answers are kept
                     (a conclusive one always; an inconclusive one only where it cannot
                     depend on the asker).
```

```
  (Fact-Direction) a MAY fact is used only to prove an ABSENCE (nothing writes this
                 vector, so its header stays) and a MUST fact only to prove a PRESENCE
                 (this parameter is never the sentinel, so `%` is plain).  Neither is used
                 the other way.  So a fact the routine fails to establish costs a rewrite
                 and never a value: a MAY fact errs toward top (more effects), a MUST fact
                 toward bottom (fewer guarantees).
```

**In words.**  The kinds the rewrites need, with their direction and key:

| kind | direction, flow | key | domain | read by |
|---|---|---|---|---|
| `writes` | MAY, UP | parameter path, result buffer, any other store | none < in-place scalar write < element write < length change (push, clear, truncate) < rebind or free | the header verdict, `(R-Inputs)`, `(R-TextBorrow)`'s tested clause, `(R-StreamWalk)` |
| `mints` | MAY, UP | function | the stores a call creates: kept local, freed, answered | `(R-RetAdopt)`, `(R-MoveAppend)` |
| `observable` | MAY, UP | function | output, file, socket, clock, random, an impure native, a yield, a `par` | `(R-PureReuse)`, `(R-SharedCall)` |
| `fault` | MAY, UP | function | may report: checked overflow, an index fault, `??`'s panic, the call-depth cap | `(R-SharedCall)` totality, `@C120` |
| `reads` | MAY, UP | parameter path | the vectors a call indexes, the fields it reads | `(R-Inputs)`'s `__inv` |
| `provenance` | MUST, UP | result | fresh (minted, held by no one else) · delivered in the buffer · a view into parameter p at a path · a copy of such a field | `(R-Destination)`, `(R-Transient)`, view returns, `pa_get`'s field answer |
| `value` | MUST, UP | result | non-sentinel; an integer interval | callers' `(R-Range)` |
| `entry` | MUST, DOWN | parameter | non-sentinel; an integer interval; a header held by the caller | `(R-NonSentinelParam)`, `(R-RangedCall)`, the twins |
| `use` | MUST, at a call site | the call's result | only read · stored · escapes | producer twins: `(R-Transient)`, `(R-AppendTwin)` |

`effect-free` (no `writes` outside its own mints and buffer, no `observable`) and `total`
(effect-free and no `fault`) are derived from the rows, not kinds of their own.

## The reach

```
  (Fact-Reach)   reach is a set of parameters, each with a default, and an answer
                 states the reach it was computed under:
                 DEPTH   — `own` (the body alone, a call answers top), `n` (n calls
                           deep, a deeper call answers top) or `all` (the whole call graph);
                           default `all`, because summaries compose: depth costs nothing
                           per query once each function is summarised.
                 CYCLES  — `cut` (a call into a function under evaluation answers the
                           kind's conservative fact: top for MAY, bottom for MUST) or `solve`
                           (the target): solved together, a strongly connected set at a time,
                           a MAY kind from bottom upward (a cycle adds no effect its bodies
                           do not have), a MUST kind from top downward (every candidate
                           assumed, each round dropping what one site does not prove — the
                           `(R-NonSentinelParam)` scheme, sound by induction on call
                           depth); an infinite domain widens to its declared bound after
                           two rounds.  A call on a cycle may reach the call-depth cap, so
                           the `fault` kind records it there.
                 FNREFS  — `opaque` (a call through a function value answers top, and a
                           function whose value is taken has no `entry` facts: its callers
                           are not all known) or `signature` (the join over every function
                           of that signature whose value the program takes; a closed program
                           only).  Default `opaque`.
                 NATIVES — `declared`: a native answers what its declaration says (`#pure`,
                           `#impure(category)`, a declared fact) and top for the rest; its
                           Rust or C body is never read.
                 UNITS   — `load`: every function compiled in this load, libraries
                           included; a PLACED library (PLACEMENT.md: a worker or another
                           machine) is a native at its boundary, its own functions analysed
                           inside its own unit.
                 SWAP    — in an `open_world` program (the debugger, a live swap) a
                           function that can be replaced is `opaque` to every caller and has
                           no `entry` facts, as `(R-InlineLeaf)` declines there.
                 TWINS   — the specialisation budget (Fact-Twin): twins per function and
                           the depth a twin's own calls are routed to twins.
```

```
  (Fact-ReachMin) a rewrite asks the SMALLEST reach that admits its sites.  Widening a
                 rewrite's reach is a change of its own, measured by the rewrite census
                 (`make rewrite-census`: the admissions it gains, by rule and bench) and
                 guarded by the falsifier of Fact-Oracle.
```

**In words.**  The reach parameters are the decisions every current analysis makes privately
and differently: whether a function value is followed, what a native is assumed to do, what a
cycle answers.  Stating them as parameters makes two analyses that ask the same question
under the same reach share one answer, and makes a rewrite's reach visible where it is asked.

## Twins

```
  (Fact-Twin)    a TWIN is a function written again under a set of ENTRY ASSUMPTIONS —
                 per parameter, a seed its kind declares (non-sentinel, an interval, a
                 header held by the caller, the caller's buffer as its result buffer) — and
                 called only from a site that PROVES each assumption.  Its body is
                 analysed with the assumptions seeded; its own calls are routed by the same
                 proofs, so a chain specialises end to end, to the reach's twin depth.  A
                 twin's KEY is (function, assumptions); equal keys are one twin, named and
                 emitted once, by one registry both writers read.  A twin is made only when
                 the assumptions change its emission (the gain test: a twin that would
                 emit the general body is not made), and at most the reach's budget of
                 twins per function; past it a site calls the admitted twin whose
                 assumptions are the largest subset of what it proves, or the general body.
```

**In words.**  `__nn` (`(R-NonSentinelParam)`'s twin clause), `__rg` (`(R-RangedCall)`),
`__inv` (`(R-Inputs)`), `__kh`, `__ap`, `__d` and `__apr` / `__apv` are each this rule with one
assumption kind, and each carries its own key: a bit in the C writer's mask
(`src/c/mod.rs` `INV_BIT` … `DS_BIT`, `NN_MASK`), a name suffix in native's.  One key makes
two assumptions on one twin an ordinary case (`__rg__kh` exists by hand today) and the
budget one number.

## One home and the oracle

```
  (Fact-One)     a fact about a callee is computed by the routine and nowhere else: a
                 rewrite reads the answer and never walks the callee's body itself.  An
                 analysis moved onto the routine is moved with the census unchanged — every
                 rule admits exactly the sites it admitted — before its reach changes.
```

```
  (Fact-Oracle)  every kind has a falsifier that runs the program: under
                 `LOFT_FACT_VERIFY=1` the interpreter checks a MUST `entry` fact on entry, a
                 MUST result fact at return, and a MAY `writes` / `mints` summary as a
                 superset of what the call actually wrote and minted, recorded per call
                 while the switch is on.  A failed check names the function, the kind, the
                 reach and the fact.
```

**In words.**  The census is the parallel run that makes moving an analysis safe (the old walk
and the routine admit the same sites, or the move is not done); the oracle is what makes a
widened reach safe, since the census can only count what is admitted, not whether it was right.

## Relations

```
  (Fact-Relational) a fact relating a parameter to something the callee reads — `x <
                 self.width`, `i < len(v)` — is not a fact of the routine.  A call whose
                 callee's branches depend on such a relation reaches the caller's
                 analysis by inlining the callee (`(R-InlineLeaf)`; its accessor clause,
                 PROPOSED, admits a leaf that reads a record parameter's fields and one
                 vector element), where the relation is an ordinary fact of one body.
```

**In words.**  `draw_quads`' texel loop (@PLN210 P5) is the case: `get_pixel` and `set_pixel`
test `gx` against `self.width` and the index against `len(self.data)`.  An interval domain
cannot carry `gx < self.width`; a relational domain could, at the cost of a symbolic lattice in
the routine.  Inlining the accessor makes the tests intraprocedural, where loop versioning and
`(R-Range)` already reason, and it composes with every other intraprocedural rule (`(R-Fill)`
then sees a per-pixel `set_pixel` loop as a fill).  The routine stays non-relational.

## The kinds

Each kind and how the routine answers it.  Every kind of the routine treats a call through a
function value as top (FNREFS `opaque`) and a native by its declaration.

| kind | data | cycles | memo policy | where |
|---|---|---|---|---|
| `StoresWritten` | may write a store (bool) | `cut`, the writes scope: yes | a no always; a yes only with the emitter's context | `opt::writes` |
| `InPlaceOnly` | writes only in place (bool) | `cut`, the writes scope: no | a body's yes always, its no only with context; never a native's or a cycle's | `opt::writes` |
| `Writes` | what a call writes (`WriteSet`) | `cut`, the writes scope: none | every answer, counted by the census (`R-Callee`) | `opt::body_writes` |
| `Inputs` | the invariant inputs a call reads | `cut`, its own scope: none | every answer | `opt::hoist` |
| `BufferOnly` | every write lands in the handed buffer (bool) | `cut`, its own scope: yes | nothing kept | `opt::store_text_borrow` |
| effect-free | the functions whose only effect is their result | `solve`, greatest | — | `pure_reuse::effect_free` |
| total | effect-free and unable to report a fault | `solve`, least | — | `shared_call::total` |
| non-sentinel entry | per (function, parameter) | `solve`, greatest | — | `opt::non_sentinel` |

Outside the routine (D-fact-1, D-fact-2): `view_return::callee` (whether a call answers a view
— its own body only, memoised privately by native's writer), `ret_adopt::callee_reinits_buffer`
(its own body only), `stream_walk::written_params` (the signature only), and the interval
`entry` facts of `generation::ranged_call` (native only).

## Deviations

### D-fact-1 (OPEN, @PLN210) — three callee facts keep their own walk or memo ([#2351](https://github.com/loft-lang/loft/issues/2351))
- **Violates:** `Fact-One`
- **Where:** `lower::view_return::callee` (native's `vr_memo`), `opt::ret_adopt::callee_reinits_buffer`,
  `lower::stream_walk::written_params`
- **Effect:** none of them looks past the callee's own body, so no answer depends on a cycle
  rule; a memo of a callee fact still lives outside the handle.
- **Removal:** one long-lived handle per writer (today a writer builds a handle per question
  over the memos it keeps), the three as kinds of reach `own` on it.

### D-fact-2 (OPEN, @PLN210) — interval entry facts exist only in native's writer ([#2351](https://github.com/loft-lang/loft/issues/2351))
- **Violates:** `Fact-Engine` (every writer reads the same answers)
- **Where:** `src/generation/ranged_call.rs` (the seeding, the guarded copy, `__rg`)
- **Effect:** the C writer has no `(R-RangedCall)`: 384 native admissions on the benches, none
  on the C (`bench/portal/analysis/c-libraries.md` § The census against rustc).
- **Removal:** the `entry` kind's interval domain in the routine; `__rg` a twin key of
  `Fact-Twin`, emitted by both writers.

### D-fact-3 (OPEN, @PLN210) — each twin kind carries its own key ([#2351](https://github.com/loft-lang/loft/issues/2351))
- **Violates:** `Fact-Twin` (one key, one registry)
- **Where:** `src/c/mod.rs` (`INV_BIT` … `VF_BIT`, `NN_MASK`); native's name suffixes and its
  bit keys OR'd into a definition number (`DV_BIT`, `src/lower/decode_view.rs`; `KH_BIT`,
  `src/generation/kept.rs`); the IR's `_tuple`, `__given_` and `@st` names
- **Effect:** a twin under two assumptions is a hand-made combination; the C's mask has eight
  named bits and 24 parameter bits left, guarded by a compile-time assert.
- **Removal:** a key of (function, assumptions) and one registry, the existing twins re-keyed
  with their emission unchanged — designed in `plans/211-rule-phases/design.md` § D-fact-3.
