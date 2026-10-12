<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 90 — copy diagnostics: make every silent structure copy visible

Tracker: [@PLN90](https://github.com/loft-lang/plans/issues/90).
Full design + failure-path enumeration: [COPY_DIAGNOSTICS.md](../../COPY_DIAGNOSTICS.md).
**Remaining open work before release (issues + optimisations, prioritised): [REMAINING.md](REMAINING.md).**
**Concrete close-out plan — the verified remainder decomposed into landable increments
(W1–W9, sequenced, with per-step file·fn·line): [CLOSEOUT.md](CLOSEOUT.md).**
**The user-facing lint recipe (the bound-vs-unbound survival split, flag-gated report, staged
to close): [unbound-copy-lint.md](unbound-copy-lint.md).** Phase-A survey + the 4 landed items:
[survey-findings.md](survey-findings.md).
**Phase B scope (drain the elidable copies — the @C86 last-use MOVE-elision, NOT the Avoidable
`&`-hints): [phase-b-scope.md](phase-b-scope.md).**
**Phase B DESIGN — grounded in captured IR (the invariant · current→target IR · the one
chokepoint read by 3 emit sites · both-generator plan · boundary matrix · failure modes ·
falsification probes · verifiable slices): [phase-b-design.md](phase-b-design.md)** (captures:
[bytecode-comparisons/phaseB-captures.txt](bytecode-comparisons/phaseB-captures.txt)).

## Status

**Phase 1 COMPLETE; phase 2 started (avoidable-vs-forced classification).**

**Phase B (the @C86 last-use MOVE-elision) — B1.1/B1.2 done; B1.3 RECORD + B1.3b/B1.3c CONSTRUCT
LANDED.** Dead-after owned sources are now lowered by building DIRECTLY into the destination instead
of copy-then-free — `src/scopes.rs`, pure IR rewrites (no new op, so BOTH backends from one pass),
gated `LOFT_MOVE_ELIDE` (OFF = byte-identical). Covered: **Record** (`v[i]=e` / `o.f=src`,
`move_elide`); **Construct field-append** (`x.field += src` into an existing container,
`construct_move_rewrite`, reorder-free); and **Construct fresh construction**
(`a = Bag{items:base}`, `construct_fresh_rewrite` — hoist a's alloc + retarget base's build onto
a.field, conservatively guarded). Matrix-validated (value + poison + leak, interp + native);
survivors still copy; non-provably-safe fresh constructs (param field value, >1 per fn) stay copies.
**B1.4** widened the fresh-construction guards: a **never-written parameter** as a hoisted field
value (`Bag { id: n, items: base }`, sound on the interprocedural `find_written_vars`), and
**multiple fresh constructs per fn**. **B1.3d** added the `a.field = base` whole-vector replacement —
the `__p154_rhs` DOUBLE copy (`base → __p154_rhs → a.field` + `OpClearVector`), detected structurally (not a
MovePlan) and lowered to build `base` directly into the cleared field, eliminating BOTH copies (incl.
the old-content-free of a heap-text field). **Nested bodies** now work too — both reorder-based rewrites walk EVERY block, so a construct or
`a.field = base` inside an `if`/loop body is elided (per-iteration correct in a loop). The
move-elision now covers all four copy shapes in flat AND nested positions. Guards:
`tests/use_analysis.rs::move_elide_{record,construct,fresh,param,multiple,whole_vector,nested}_*`.

**B1.5 FLIPPED — the move-elision is DEFAULT ON — MERGED to main 2026-07-06 (PR #514, squash
`46ecd3dc`).** `LOFT_NO_MOVE_ELIDE` opts out; the `MOVE-PLAN` dump is split onto its own opt-in
`LOFT_MOVE_ELIDE`. Getting there required a **flag-ON exposure sweep** (running the whole corpus with
the rewrite on — the ~30 hand-probes never did): it found ~12 corpus bugs, all "retargeted into a
non-stable slot / source read between build and copy," all fixed via a guard layer (`bad_containers`,
`def_order`, `source_escapes`, self-read, replace-vs-append). Behavioural corpus green default-on
(issues 748, leak 49, native, wrap, native_scripts, loft_suite); CI green incl. the ASan UAF/OOB
gate. Detail: [phase-b-design.md § B1.5](phase-b-design.md).

**@PLN90 — ALL 9 work items DONE or RESOLVED (2026-07-06).** Phase B (the *elimination* half)
shipped. **Built/closed (6):** **W1 = A1b** (the temp-subject borrow-return UAF — the wide-release
blocker, default on) · **W6** (par-native E0308, P0) · **W5** (the enforced copy-lint channel, gated
`LOFT_WARN_COPIES`) · **S5.2** (precise lint locations) · **W8** (empty-`[]` arm now materialises a
real empty vector in both formattings — the fold-to-null fragility is gone; suite green, guard
`508`) · **W4** (O-Complete — already CLOSED 2026-07-04 as formal **D-own-2**: the ownership fact is
TOTAL, oracle green locally). **Resolved (3):** **W2/A2 — WON'T DO** (three matrix-guarded
prototypes proved a struct-field return **must copy** per the #415/C86 value-semantics contract; the
copy is FORCED, not avoidable) · **W3** MOOT (rode A2) · **W9** no safe drain (remaining Avoidable
are @C86-contract-owned) · **W7** DEFERRED — the plan's final phase, correctly not-yet-active (its
lint-silencing premise is undermined by the same A2/#415/C86 finding, and reserving `copy` is a
design decision awaiting demonstrated need). Detail per item: [CLOSEOUT.md](CLOSEOUT.md).

**Phase 1** — the decision covers every structure-copy emission ([phase1-inventory.md](phase1-inventory.md)).
`LOFT_COPY_DUMP` is the runtime ground truth; the verdict (`use_analysis`, **route 1** —
extend its domain, since the facts live only in the post-parse pass) classifies all four
copy idioms: var-buffer · construction/field-append · return-buffer · `OpCopyRecord`. Parity
proven on the corpus; all diagnostic-only `Copy` rows (no `ElidePlan`, no codegen change).

**Phase 2** — each `Copy` row carries a 4-way `VerdictRow.class` (the warning bucket):
**Eliminated** (`Borrow`) · **Avoidable** (warn — a borrow would be sound, blocked only by
analysis weakness; the north-star worklist) · **Implicit** (SILENT — the copy is inherent to
the model: a struct/enum field or vector slot *owns* its data, e.g. `S { f: src }`, `v[i] =
e`) · **Forced** (informational — owned by circumstance: temporary source, later mutation).
`LOFT_MATERIALIZE_DUMP` shows `bucket=…` per row + a `MAT-WORKLIST avoidable=N implicit=N
forced=N` tally. On the corpus: `field_return` → AVOIDABLE (its elimination IS the @PLN85 P4
borrow-correctly fix); construction / `OpCopyRecord` → implicit; `assign_field` → eliminated.
**North-star primary:** the warning serves elimination — drain bucket 2 (avoidable) into
bucket 1 (auto-elide), stay quiet on bucket 3, never copy "just because"
([COPY_DIAGNOSTICS.md § North-star](../../COPY_DIAGNOSTICS.md)). Suite byte-identical (issues
746, use_analysis 16).

**Phase 2 REFINEMENT — the report + survival split BUILT (#510); the enforced warning is not
([unbound-copy-lint.md](unbound-copy-lint.md), CLOSEOUT W5).** `report_copies` (the user-facing
`--report-copies`) and the bound-vs-unbound `survival_class` split now ship (gated on
`report_copies_enabled()`); what remains is routing Avoidable rows through the `Level::Warning`
diagnostics channel as a default lint (blocked on draining the Avoidable set — W1/W2 — first).
The blanket "construction / `OpCopyRecord` → Implicit" above is **too coarse**: it silences a
construction / slot-set that **duplicates a still-live source** — a genuine *unbound* copy the
user must see. The silent/indicate line is **bound vs unbound**, keyed on the copy's *source
fate*, not the emitting op: **bound** (scalar · move — source consumed here · literal source)
→ Implicit/silent; **unbound** (a live source is duplicated) → indicated (Avoidable/Forced).
Delivered as a **flag-gated report** (off by default) that answers two questions — *what still
copies that we can fix* and *where the hidden cost is in libs/programs* — and staged to
**close**: survey → drain the Avoidable set → accept the forced remainder behind a **sparse
per-site opt-out annotation** (so libraries PR copy-clean) → promote to an enforced gate. A
longer trajectory, tracked as phases A–D.

**Phase 2 next — draining bucket 2 (the north-star), starting with field-return = the
@PLN85 P4 borrow-correctly fix.** Design written:
[borrow-return/DESIGN.md](borrow-return/DESIGN.md). Invariant: a borrowed-view return
returns the aliased `DbRef` directly (no return buffer, no copy); the copy **moves to the
caller**, emitted only where the borrow is unsound (the subject does not out-live the
result — a caller-side `deps` decision). This eliminates the field-return copy AND fixes
P4's borrowed-yield crash (the same shape, compiled as a borrow). Slices: (1) callee
direct-alias-return ABI, (2) caller materialise-on-demand (coupled with 1 — gated until
both land), (3) retire the callee copy. Each matrix-validated both backends, gated. NEXT:
capture the target alias-return bytecode (loft-codegen gate), then slice 1.

**North-star (do not lose it):** the warning is the *instrument*, not the goal — the goal is
the compiler **automatically not copying** when it can prove a borrow is safe (we never copy
"just because"). loft already has the engine: a `Borrow` verdict → `ElidePlan` → the borrow
rewrite elides the copy (var-buffer ships today). Every copy sorts into three buckets:
**(1) auto-eliminated** (no warning — grow this), **(2) avoidable** (warned — and this set
IS the worklist for bucket 1), **(3) forced** (warned, informational). See
[COPY_DIAGNOSTICS.md § North-star](../../COPY_DIAGNOSTICS.md). Notably, **field-return is
bucket 2** — eliminating it is fixing @PLN85 P4 (borrow-correctly), which closes the loop
back to that work.

NEXT — **phase 2**: classify each Copy row avoidable vs forced (bucket 2 vs 3 — the
elimination worklist), then emit the user-facing lint off it (with the `&` / restructure
hint, opt-in first). Coarse var/source attribution in some rows is to be sharpened there.

Wanted **before @PLN85 closes**: we often miss that a copy is happening, and that blind
spot is shaping what we build (the @PLN85 owned-copy match-return synthesis manufactures
copies on a near-universal pattern). Surfacing copies first changes those decisions.

## Goal

Make **every deep copy of a heap structure visible** — never silent. loft aliases heap
values by default; a deep copy of a record / vector-of-records is the fallback the
compiler emits when it cannot prove an alias is safe, and it is invisible today. Surface
it, with its reason, classified avoidable vs forced.

**Invariant:** every emitted structure copy is decided at one place (the copy-vs-borrow
verdict) and surfaced there with its reason — so a copy is never silent and a warning
never fires without an actual copy.

It is a **perf/behaviour lint, not a borrow checker** — never an error, never an
annotation, lowering unchanged; compatible with the decided "internal, invisible
ownership" model ([OWNERSHIP_MODEL.md § Internal and invisible](../../OWNERSHIP_MODEL.md)).
Three reasons to warn on *every* structure copy regardless of static size: the cost is
runtime-sized and unbounded ("hundreds of MB just to be sure"); conservative copies are
invisible and often avoidable; a copy silently changes behaviour (independent value breaks
the alias-default). It also doubles as **our** worklist — each avoidable copy is a
currently-invisible borrow-analysis gap we might still fix.

## Phases

1. **Coverage (load-bearing first).** Make the copy-vs-borrow decision the *sole arbiter
   consulted by every structure-copy emission*. Today `use_analysis::Verdict` (with a
   `reason`) decides only vector-copy bindings, while `OpCopyRecord` / copying
   `OpAppendVector` are emitted at ~20 scattered sites (struct construction, pass-by-value,
   assignment) that bypass it. Extend the verdict's domain, or route all emission through
   one `emit_structure_copy` chokepoint that consults it. *A warning is only as complete as
   the decision — so this is the first deliverable, not the diagnostic.*
2. **Diagnostic.** Emit the lint off the decision: avoidable vs forced, with the verdict's
   reason and an existing-lever hint (`&` / restructure). Opt-in lint first; promote to a
   default warning once borrows are the norm.
3. **Explicit copy-intent.** The surface form to opt into an independent copy and silence
   the lint — the inverse of `&`.

## Cross-arc dependencies

- **@PLN85 (ownership / over-free).** Soundness rides on the `deps` / `use_analysis`
  verdict. Prerequisite is **borrow-correctly, not the owned-copy synthesis**: the borrowed
  match-return should compile as a true borrow (no return-buffer ABI), so borrows are the
  default and a copy is the rare, warned exception. The @PLN85 `jo_copy_borrowed_arm_yield`
  synthesis is the *wrong* tool here (it manufactures the very copies this plan warns on);
  keep it only as the fallback for the genuine forced case (a temporary subject that cannot
  outlive the return).

## Open questions

- Coverage-first vs scoped-warning-first (recommendation: coverage first; a partial warning
  that looks complete is worse than none).
- Default severity: opt-in lint vs default warning.
- Explicit-copy surface syntax (phase 3).

## See also

- [COPY_DIAGNOSTICS.md](../../COPY_DIAGNOSTICS.md) — the design.
- [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) — the decision substrate + the
  "internal, invisible, no borrow checker" position.
- `src/use_analysis.rs` — the copy-vs-borrow `Verdict` + `reason` this hangs off.
