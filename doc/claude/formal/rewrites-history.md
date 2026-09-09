<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Native rewrites — the record of attempts, and where each invariant is held

The companion of [rewrites.md](rewrites.md), in the shape of
[ownership-history.md](ownership-history.md): a rule says what must hold; this file says
which site actually holds it, what was tried against it, and what a sabotage measured.  Read
it before attacking a rule with a sabotage — two of the three sabotages of 2026-09-09
stayed GREEN, and each time the reason was that the invariant was held by a site other
than the one attacked.  A green sabotage is a finding about WHERE the invariant lives, and
this is where it is written down.

## Where each invariant is held (2026-09-09)

| rule | held by | a sabotage that turns it red | a sabotage that stays green, and why |
|---|---|---|---|
| R-Switch | every hoisted read's `VERIFY` monomorph (`get_elem_hoisted`, `vec_set_hoisted_or_raise_runtime`, `hoisted_scalar_verify`, `push_hoisted`) | any stale holder: the first read panics under `LOFT_HOIST_VERIFY=1` | — |
| R-Header | `hoist::vector_candidates` + `blocks_header_hoist`'s allow-list | shift a fused write's field offset (P4b's falsifier: writes land in neighbours) | — |
| R-Scalar | `hoist::WriteSet::evicts` at ANALYSIS time (caller side) | `evicts` answering `false`: eight P4c cells red | the CALLEE-side filter in `callee_inputs_inner` skipped: green, because the caller's write set already carries the callee's writes through `callee_writes` — the callee-side filter is a second assertion, kept to keep a twin's signature free of inputs no caller can hold |
| R-Inputs | the twin's parameter WIRING (`push_twin_frames` naming `__is_k` / `__ih_k` in the order `twin_call_inputs` passes them) | rotate a callee's two same-typed inputs: the composite cell red, the verifier panics `hoisted 3, now 4` | the rebound test on the input candidates skipped: green, because a loop that re-points a link is declined by the GATE (`OpCreateStack` takes a reference operand and is not store-free) before the rebound test is asked; reversing ALL inputs does not compile (c10's twin takes a `u8` and an `f64`) — a refusal, not a measurement |
| R-Push / R-Refresh | `Stores::push_hoisted`'s fast path: the length written to the header AND the record | skip the record write-back: the constant-fill cell red (`len(lay.best)` reads the record), the verifier panics at the next push | — |
| R-State | the ORDER of `hoist::hoistable`'s collectors (reads, callee inputs, then pushes removing themselves from the read list) | run the push collector first: a pushed path gets a plain header too, and a twin handed the plain one reads one push behind (the 2026-09-09 emission, caught by the verifier) | held by order, not by construction; the closure is the state builder (below) |
| R-Alias | `hoist::owned_local` / `hoist::retbuf_var` in `hoistable`'s admission block | drop a view candidate's exclusion: an alias of a pushed local reads a moved record | — |
| R-Wrapper | `hoist::one_op_wrapper`'s ORIGIN test (`def.source() == STD_SOURCE`) | remove it: a user one-op function inlined, the wasm live-dispatch probe counts 0 dispatches (D-rw-1, the gate that found it) | — |

## Attempts and closures

- **D-rw-1 — a user one-op function inlined as a stdlib wrapper (2026-09-09, CLOSED).**  The
  recogniser asked the body's SHAPE and not the definition's ORIGIN.  Found by the GitHub
  gate's wasm live-dispatch probe on the rebased tree; the local curated set never runs it.
  Closed by the origin test; cell c11 of `V-o-wrapper-op-cells.loft` pins the user call.
  The third form of one lesson: a peephole matching an op shape must also ask what the shape
  stands for — the TYPE (P4c), the CONTAINER (§ V-m), the ORIGIN (here).
- **One path, two holders (2026-09-09, CLOSED by ordering; the structural closure is
  queued).**  The first push loop beside a twin call: `hoistable` collected the push paths
  BEFORE § V-p's callee inputs, the input collector added the pushed path again as a plain
  header, `begin_vector_hoist` bound both, and the twin was handed the plain one — one push
  behind.  No value cell saw it; `LOFT_HOIST_VERIFY=1` did, on the callee-inputs guard
  (`hoisted {len 12}, now {len 13}`).  Closed by running the push block after every
  collector that can add a read candidate.  This is the defect R-State was written for, and
  the reason the chapter now has a hoist-state section: the per-rewrite rules were each
  right and together said nothing about the same path being claimed twice.  The closure by
  construction — `hoistable` as ONE state builder — and the emission audit that checks the
  emitted routines against R-State are both queued in @PLN157's README.
- **Two green sabotages (2026-09-09).**  Recorded in the table above (R-Scalar's callee-side
  filter, R-Inputs' rebound test).  Each was a claim in DESIGN.md's `@falsified-at` draft
  that the measurement refused; the guards carry the sabotage that measured instead.
- **A rewrite this chapter does not own (2026-09-09).**  A push whose VALUE reads the pushed
  vector (`px += [px[len(px) - 1]? + d]`) is lowered by the PARSER as `OpDatabase ·
  OpAppendVector` — a whole-vector COPY per iteration — before the push, so the push tier
  never sees it (cell c3 of `V-q-hoisted-push-cells.loft`).  `lock_ribbons`' running sum is
  quadratic in the point count through it.  A lowering shared by both backends belongs to
  [collections.md](collections.md), not here; noted so the next reader does not look for it
  in the emitter.

## Shipped: the emission audit (2026-09-09).  Queued: the state builder

1. **The emission audit** (@PLN157 § V-r, SHIPPED 2026-09-09): `scripts/emission_audit.py
   <emitted.rs>` checks R-State, R-Refresh and R-Inputs structurally over a `--native-emit`
   output — the prelude lines bind holders keyed by their path EXPRESSION text, the reads,
   writes, pushes, `.len` uses and twin calls name them, and a template append on a held
   path is a violation; `tests/emission_audit.rs` runs it over every cell corpus and the
   in-repo bench in the gate.  Falsified against the collector order that produced the
   double holder: flagged at emission, with no run.  This is the plan's validation goal: as
   the emitted routines grow, their assumptions stay checkable.
2. **The state builder** (M): `hoist::hoistable` today is three collectors and two `retain`s
   whose ORDER holds R-State.  A `LoopState` with `hold(path, Holder)` as the only insert (a
   second holder for a path is an error, not a shadow), `evict(scalar)`, and
   `admit_mover(path)` applying R-Alias; the gate, the read collectors, the callee inputs
   and the pushes each call it; `begin_vector_hoist` iterates the map.
   Behaviour-preserving (the loft-codegen skill's Mode B: byte-identical emission over the
   cell corpora before and after), and the precondition for the next mover — the
   self-reading push once the parser stops copying, a bulk fill for a constant
   comprehension.
