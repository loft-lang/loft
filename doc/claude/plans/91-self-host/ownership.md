<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 — ownership in the rewrite: what the Rust code does, and the shape to aim for

## Status

**Inspection done, direction set, NO step designed (owner, 2026-10-27).**  The owner wants
an elegant version of loft's ownership logic in the rewrite.  This file records what the
current Rust implementation does (measured on `origin/main` @ `c3ea8977d`; every path below is
relative to that tree) and the shape the rewrite aims for.  Following the plan's working norm
([1b-interpreter.md](1b-interpreter.md) § The working norm), the first ownership step is
designed only when the rewrite reaches it; § When it enters the plan names that point.

## What the Rust implementation does

### Size and spread

| where | lines | role |
|---|---|---|
| `src/scopes.rs` + `src/scopes/` (39 files) | ~33 400 | the scope pass: free placement, witnesses, hand-offs, materialisation |
| `src/use_analysis.rs` | ~9 700 | the authoritative oracle `ownership_of` (`:5033`) and return delivery |
| `src/ownership_cfg.rs` | ~1 700 | a flow-sensitive CFG dataflow, an OBSERVER only (`LOFT_OWN_ORACLE`) |
| `src/variables/mod.rs` | ~6 300 | deps on types, ownership flags, ~18 side tables per function |
| `src/parser/*` | ~400 sites | copy/view at binds, return delivery (`control.rs`: 392 buffer mentions), hidden buffers |
| runtime (`database/allocation.rs`, `fill.rs`, `native.rs`) | — | identity-guarded frees, a call bracket, a double free silently ignored |

### The finding: one question, answered in many places

**"Does this local own its store here?" has five representations.**  `formal/ownership.md`
(`:330–395`) names four of them itself:

1. the deps proxy on the variable's type (an empty list means "owns"), which the spec calls
   *"UNSOUND ALONE"*;
2. the `skip_free` veto that exists because the proxy is unsound (~70 parser writers, ~16 in
   scopes);
3. the oracle `use_analysis::ownership_of` (`Own::{Owned, Borrowed, Join, Unknown}`, `:1874`),
   derived from the IR;
4. the scope pass's `owned_refs` memo (latest assignment, loop depth);
5. `ownership_cfg`'s flow-sensitive `OFact` (`:452`), with the same lattice as the oracle,
   computed and never consumed.

The composite predicates combine two or three of them: `owns_store` (`variables/mod.rs:5813`),
`proxy_says_owned` (`:5559`), `owns_displaced_store` (`:5391`).  The documented defects come
from them disagreeing: loft#723, D-own-16, and `state/codegen.rs:~2670`, which calls itself
*"a SECOND home for the question … and they do [disagree]"*.

**Per-path facts are done by hand.**  Six maps (`owned_refs`, `view_backing`,
`construction_backing`, `bind_backing`, `tuple_call_mint`, `tuple_member_now`) are each
snapshotted and intersected at `if` (`scopes/branches.rs:210–296`), at loops (`scan.rs:315–347`,
one pass, no fixpoint) and at a third join (`scan.rs:840–852`).  `join_rewrite`, `arm_lift`,
the second and third rescans of a body (`scopes.rs:690–743`) and the branch pre-inits exist
largely to rewrite the IR until each arm has one fact.

**Where the static fact is missing, a runtime witness stands in.**  There are about 15 kinds,
each a hidden local:
- booleans: `__lbo_`, `__rbo_`, `__hoff_`, `__tmov_`;
- store-identity snapshots: `__own_`, `__rbw_`, `__vrb_`, `__wit_`, `__disp_`, `__vdisp_`,
  `__vsnap_`, `__fdisp_`, `__hdsp_`, `__fkeep_`.

They are consumed by the conditional free operators `OpFreeRefIfDistinct`,
`OpFreeRefOrHandUp`, `OpFreeRefUnlessEntry` and `OpBindOrCopy` (`default/01_code.loft`,
`fill.rs:3601–3640`), and by the call bracket `n_protect_store_frees` (`native.rs:1660`).  That
bracket is how the runtime tells "the callee returned my argument" from "the callee minted a
store", which no static bit records.  `scopes/phase.rs:320–329` notes that `__lbo_` answers the
same question as the identity route.

**Liveness is re-derived at least twelve times**, by private walks:
`confinement::store_dead_after_block`, `last_use::store_liveness_walk` (documented as a
*"sequential approximation across branches"*), `convert::locals_read_after`,
`construct_elide::references_var_after`, `move_elide::mentions_var`,
`arm_lift::mentions_var_outside`, `branches::mentions_of`, `buffers::names_outside_free`,
`phase::var_mentions_in`, `capture_builds::subtree_has_return`, and others.

**Return delivery has about 40 verdicts across five enums**, one enum per return kind:
`classify_vector_delivery` (`parser/control.rs:3025`), `classify_reference_delivery` (`:3209`),
`classify_text_return` (`:18063`), `classify_ret_promotion` (`:19444`, 10 verdicts), and
`TextDep`.  The parser also changes a function's signature while parsing it (hidden
`__retbuf`/`&text` parameters, `ref_return` `:19903`).

**Copy against view is decided, then overridden.**  The parser decides at the bind
(`classify_vec_bind`, `expressions.rs:2008–2110`, reading the deps proxy because *"the
oracle's whole-body walk does not exist mid-parse"*, `:555`).  The oracle's elision plans may
reverse it later (`view_elided`, `move_plans`), and both emitters re-decide adopt against copy
(`generation/dispatch.rs:1548–1646`).

**The runtime forgives what the compiler gets wrong.**  `Stores::free_named` treats a double
free as a silent no-op (`database/allocation.rs:~1843`, a stated departure from
`@FR-H-FreeTwice`), so a defect in the static system is invisible until a store is reused.

### What is general and worth keeping

- **The `Own` lattice and its interprocedural fixpoint** (`use_analysis.rs:1874`,
  `return_ownership` `:3159`).
- **`ownership_cfg`'s CFG builder, reaching definitions and checks**: definite leak
  (`run_leak_scan` `:1213`), free of a borrowed value (`:1347`), free of a vetoed binding
  (`:1407`), with fault injection to falsify them.  It is already proved over-free-sound
  (`formal/ownership.md:564`).
- **The place and disturbance family**: `scopes/places.rs`, `disturbance.rs` (an
  interprocedural closure, `:720`), `view_walk.rs`, `reshape_refusals.rs`.  Borrow validity
  is a separate question from ownership, and this is a coherent answer to it.
- **The semantic rules.**  The formal chapter mixes rules that define what programs mean
  with rules that describe the fact machinery:
  - Semantic, kept as the rewrite's specification: `(B-Copy)`, `(B-View*)`, `(B-Ref-*)`,
    `(B-Disturb)`, `(H-Materialise)`, `(O-Owner)`, `(O-Move)`, `(O-Derived)`,
    `(O-Complete)`, `(O-Detach)`, `(H-FreeAll)`, the drop and lease rules, `(O-One-Kind)`.
  - Machinery, which describes this implementation: `(O-Proxy)`, `(O-Override)`,
    `(O-Latest)`, `(O-Witness)`, most of `(O-Buffer)`.
  - Optimisation, values unchanged and switchable per `(R-Switch)`: every `(R-*)` placement
    rule, buffer reuse, confinement, last-use frees.

## The shape to aim for (direction, not a design)

Four pieces.  Each replaces a family above, and each answers its question in exactly one place.

1. **The parser records only what the source says.**  A bind is emitted as what it means:
   copy of a place, view of a place, `&` link to a place, result of a call.  The syntax-only
   facts (`&`, `const`, captured, user-named) travel with it.  No deps, no `skip_free`, no
   hidden buffers, no signature changes while parsing.  This removes the parser's ~400 sites
   and the pass-1/pass-2 disagreements (`pass2_rebuilt`, `__ref_p2_N`).
2. **One forward dataflow per function decides ownership.**  It runs over a control-flow
   graph: the existing `ownership_cfg` design, promoted from observer to authority, as @C88
   already recommends.
   - Each local, and each place key, holds one of `Null`, `Owns(s)`, `Views(place)`,
     `Moved` or `Maybe(s)`, where `s` is an abstract store.
   - Joins are the lattice join, and loops run to a fixpoint.
   - Frees are read off the result. A displaced `Owns` is released at its rebind. At scope
     exit `Owns` is freed, `Maybe` is freed behind **one** runtime flag, and anything else
     is left alone.
   - `Maybe` is the only runtime witness kind.  It replaces the five representations, the
     six hand-intersected maps, the rescans and the fifteen witness kinds.
3. **Function summaries carry what a call needs**, closed over the call graph: what the
   result is (fresh, or a view of parameter *i*), which parameters the body disturbs or
   writes, and which references it stores into other arguments.  **Hypothesis to
   falsify:** a function whose result is fresh on one path and a borrow on another copies
   on the borrow path, so every summary is one of the two.  Values are unchanged under
   `(O-Move)` (the caller receives an independent value either way).  If it holds, the
   conditional free operators and the call bracket disappear.  Its cost is one copy per
   such return, and it gets measured before it is adopted.
4. **One backward liveness pass, consumed by every optimisation.**  Last-use moves,
   confinement, lazy buffers and placement become rewrites over SETTLED facts, each
   switchable, none re-deriving liveness.  They come after the semantics are right, not
   with them.

**Borrow validity** (places, disturbance, materialisation, the refusals) stays a separate check
over places, using the same summaries.  **The checks become theorems:** `leak_check`,
`text_return_check` and `ownership_cfg`'s scans turn into assertions on the dataflow's
result.  And a double free is a fault (`@FR-H-FreeTwice`), never a silent no-op, so a defect
cannot hide.

## Its own track, started early (owner, 2026-10-27)

**Ownership has always been loft's biggest hurdle, so it is vital to get right.**  It is
therefore not deferred to the store phase.  It is a track of its own, **O**, beside the
interpreter phases, and it needs nothing from them except S0 (the bundle opens in loft).  The
analysis reads IR records and runs no program, so it can start the moment S0 lands.

How the track makes getting it right likely, rather than hoped for:

- **The acceptance corpus already exists, and it is the history of the hurdle.**  Measured
  2026-10-27: 446 language-suite cells in the `ownership`, `heap`, `binding`, `closures` and
  `tuples` chapters; 105 `tests/scripts` guards citing an `(O-*)`, `(H-*)` or `(B-*)` rule;
  56 `D-own` entries in `formal/ownership-history.md`; ~208 closed issues on leaks, double
  frees and use-after-free.  Every one of them is a program the new analysis must judge
  correctly.  "Correct on the corpus" is the bar, not "looks elegant".
- **The design answers the defect classes on paper before any code.**  The inspection above
  found fifteen classes (`ownership-history.md`): joins, proxy confusion, buffer ownership,
  ordering, latest assignment, interprocedural translation, opaque calls, backend
  divergence, stale views after growth, and the others.  The first step's design walks each
  class through the five-state lattice and the summaries, and says what the analysis
  concludes.  A class the lattice cannot express stops the design, not the code.
- **It is a checker before it is an authority.**  First it judges the frees the Rust scope
  pass already put into the IR.  It reports every store freed twice, never freed, or freed
  while borrowed, and it must agree with `ownership_cfg` (the Rust observer with the same
  lattice) on every program.  Only after it is silent and agreeing over the whole corpus
  does it place frees itself, in 1a.
- **Every disagreement is resolved by the semantic rules**, never by either implementation.
  The rule says which side is wrong.  A Rust defect found this way is fixed in Rust first
  (the dogfood rule).
- **Falsify the cheap claims first.**
  - Does a `Maybe` ever need more than one runtime flag, for example in a loop?
  - What does the callee-copies hypothesis cost on the corpus?
  - Do places inside records, tuple members, closure captures and generator handles fit
    the place key, or do they need a sixth state?
  Each claim gets a probe before the step that relies on it.

### Fast as well as right (owner, 2026-10-27)

Inferred ownership is loft's answer to the borrow checker that slows rustc down: the
programmer writes no lifetimes, and the compiler works ownership out.  That answer holds only
if working it out is CHEAP.  So compile time is a gate of the O track, beside correctness.

**Baseline, measured 2026-10-27** (`LOFT_TIMING=1 loft --interpret --check`, installed loft =
`origin/main`).  `scopes=` is `scopes::check` alone (`src/main.rs:~9820`); the ownership work
the parser does inline (deps, flags, return delivery) is inside `parse_user=` and cannot be
separated today.

| program | lines | `parse_user` | `scopes` |
|---|---|---|---|
| `tests/scripts/10-match.loft` | 396 | 17.0 ms | 10.5 ms |
| `tests/scripts/164-view-field.loft` | 674 | 26.0 ms | 24.6 ms |
| `tests/scripts/a-generic-instance-returns-what-its-concrete-twin-returns.loft` | 610 | 19.7 ms | 15.4 ms |

On a program of a few hundred lines, the scope pass alone takes 60–95 % as long as all of
parsing.

Side observation, to check rather than claim: `close_program`'s `value_record` reports
`rounds=1001` on all three programs, and 247 ms on `164-view-field`.  That looks like a
fixpoint stopping at a cap rather than converging.

**What fast requires of the design** (constraints on every O step, not a design):
- **Linear per function.**  One forward pass over the CFG, and loops iterate to a fixpoint
  bounded by the lattice's height (five states), not by a round cap.  One backward
  liveness pass.  No rescans of a body.
- **Modular.**  Summaries are computed once per function, callees first (strongly connected
  components of the call graph together), and cached by the function's body.  An edit
  re-analyses the changed function, plus its callers only when its summary changed.  This
  is what makes the edit-to-probe cycle (@PLN194) cheap.
- **No whole-program fixpoint over bodies.**  The interprocedural facts are the summaries,
  nothing else.
- **Measured, not argued.**  The O checker reports its own time per function.  The gate:
  the loft-written analysis, compiled `--native-release`, costs less than `scopes=` on the
  same programs, and its time grows linearly on generated functions of growing size.  The
  claim is checked against loft's own timings, never against folklore about rustc.

The track's first step, **O0**, is designed after S0.1 lands.  It is expected to cover the CFG
over `ir.loft` records and the lattice as a checker, with the defect-class walk-through as its
design.  Only O0 is designed next; later steps are not.  The interpreter and the front end meet
the track at two points:

- **In S3** the interpreter's leak check and the O checker must agree.
- **In 1a** the checker becomes the authority that places frees.

⚠ **This meets the "identical IR" constraint.**  An elegant pass will not reproduce the Rust
pass's fifteen witness kinds, so the IR after the scope pass cannot be byte-identical.
Recommended: identical IR up to the scope pass's INPUT, and from there equivalence judged by
the rules (exactly-once frees, no free of a borrow, the leak check, the suite).  The owner
decides.  For that boundary to be checkable, the bundle has to be saved before
`scopes::check` as well as after (today `startup_cache::save_program` runs after it,
`src/main.rs:~9322`; `scopes::check` is `src/scopes.rs:584`).

## Open questions for the owner

1. **The identical-IR boundary**: up to the scope pass's input (recommended), or through it.
2. **The callee-copies hypothesis** (piece 3): adopt it if the measured cost is small, and
   otherwise keep `Maybe` for results too.
3. **The double-free no-op** in `free_named`: make it a fault in the Rust runtime now (a
   dogfood fix, it hides defects), or leave it until the checker exists.
