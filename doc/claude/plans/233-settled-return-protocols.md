<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 233 — Settled return protocols: no pass-1 borrow survives into pass 2

## Status

Active ([@PLN233](https://github.com/loft-lang/plans/issues/233), from loft#2660).  The direction
is the owner's (loft#2660): *settle the analysis in pass 2, not pass 1*.  Phases 0, 1 and 3 are
built: pass 2 starts from clean dependency lists (`(O-Borrow-Final)`), which cures loft#2660's
program.  Phase 2 (settled RETURNS, Q1) is next: a callee declared below its caller still hands
pass 2 its pass-1 return.

## Goal

Every function's return protocol (which store its result lives in: a borrow of parameter *i*,
or its own return buffer) is ONE fact, settled before any caller's final pass reads it, and no
variable carries a dependency from pass 1 into pass 2.

## The invariant

> **A caller's final pass reads its callees' FINAL return protocols, and a variable's
> dependencies are the ones its final pass computed — nothing older.**

**The class today.**  loft parses every program twice.  Pass 1 types a call against the callee's
return as known at that moment.  A recursive call is typed before its own function's return is
known, and a call to a function declared below has only its pass-1 shape.  Pass 2 knows more.
`f`'s return can change from "borrows `n`" (`Deps [0]`) to "its own buffer" (`Deps [1]`) when
`materialize_view_return` copies a view at the tail.  But `Function::append`
(`src/variables/mod.rs`) unions each variable's pass-1 dependencies into pass 2, so a caller
typed against the old answer keeps a borrow of a store it actually owns.  The variable reads
as a view, and its store is never freed (loft#2660, a leak on both backends).

**Measured on main @ 9e84211ef**, 2,557 corpus programs (the firewall02 stream, on loft#2660):

| What | Count |
|---|---|
| functions whose `returned().depend()` differs between the passes | 471 names in 216 programs |
| … the harmful direction (pass 1 records a borrow that pass 2 drops) | 77 instances, 62 names, 43 programs |
| … which are recursion | a minority: most are `[0]` → `[1]`/`[2]` (the hidden return buffer) |
| programs that change when every pass-1 dependency is cleared at pass 2's start | 26 of 2,553 |
| … real regressions among them (double free, failed assert, leak) | about 18 |

The 18 are why the union exists: pass 2 still leans on pass-1 dependencies for match and tuple
pattern captures, and for views a later disturbance materialises.  Callers repair the drift
shape by shape today: `loft-suite rules/ownership/O-Witness/1466-a-call-result-takes-the-callees-pass-two-borrow-list.loft`
is one repair, and `change_var_type`'s `adopt_fnref_ret` is another (a fn-ref slot adopts a
refined return dependency).  loft#2660 is a shape nobody repaired yet.  A rule that collects
per-shape repairs while its defect list re-cuts is the sign to stop patching (DESIGN_PROTOCOL.md
§ When the patches point the other way), so this plan replaces the union, not the shapes.

## Phases

| Item | What | Verify (goes red if done wrong) | Effort |
|---|---|---|---|
| **0** | The census as a tool: per program, each function whose return dependency differs between the passes, and the harmful-direction count, as a ratchet (`index/`), with `LOFT_LOG` naming the function and both answers | it reproduces the measured counts on main 9e84211ef (77 / 62 / 43, 471 / 216), and loft#2660's `f` is in its list | S |
| **1** | The cells: loft#2660's program; one cell per harmful-direction SHAPE class the census finds (recursion in a match arm, a callee declared below its caller, …); the ~18 programs the clear-everything experiment broke, re-measured and kept as cells (each correct today, a guard for phase 3) | every cell is green today except loft#2660's shape cells, marked expected-to-fail; each fails for the named reason | S |
| **0** status | **Done** (62c8870930): `LOFT_RETURN_DRIFT`, `scripts/return_drift.py`, baseline 229 harmful instances / 195 names / 98 programs over 3502 programs (this census counts every function where a stale borrow COULD survive) | — | — |
| **1** status | **Measured 2026-11-27** (`LOFT_NO_PASS1_DEPS=1` strips the pass-1 dependency lists at each function's pass-2 start, `Function::strip_carried_deps`): loft#2660's program is cured, and 25 suite cells fail on both backends (49 of 3352 runs) — the set phase 3 must make pass with the switch on: `binding/B-Copy/1752-a-coalesce-over-a-whole-local-copies.loft`, `binding/B-Copy/a-re-bind-of-a-whole-local-copies-in-every-spelling.loft`, `binding/B-Disturb/a-view-is-copied-only-where-it-is-live-across-a-disturbance.loft`, `binding/B-View/1184-a-view-assigned-back-onto-its-own-source.loft`, `calls/F-Block/a-match-at-a-tail-answers-its-arm-not-a-temporarys-free.loft`, `closures/L-CapHeap/1085b-a-nullable-local-frees-what-it-displaces.loft`, `dispatch/Disp-Match-Equiv/a-set-with-a-resolving-member-and-its-match-agree-on-every-tuple.loft`, `heap/H-CopySelf/a-value-delivered-onto-or-from-inside-its-own-place-keeps-its-value.loft`, `heap/H-Free/an-early-return-frees-a-literal-once-its-adopter-is-rebound-later.loft`, `matching/P-Alt/a-tuple-element-lists-alternatives.loft`, `matching/P-Cap/a-capture-and-a-plain-struct-stand-at-every-pattern-position.loft`, `matching/P-Guard/a-guard-is-asked-of-each-listed-pattern.loft`, `matching/P-Multi/a-tuple-or-pattern-arm-links-its-bindings.loft`, `matching/P-Point/a-struct-point-pattern-reads-any-declaration-order-and-a-null-field.loft`, `ownership/O-Borrow/2338-a-view-through-a-match-capture-outlives-the-match.loft`, `ownership/O-Detach/2397-a-literal-reads-its-field-under-another-name.loft`, `ownership/O-Proxy/an-is-payload-binding-borrows-its-subject-like-its-match-twin.loft`, `ownership/O-Witness/1510-a-mixed-own-view-locals-record-hooks-once.loft`, `ownership/O-Witness/a-mixed-local-viewing-a-hash-element-releases-through-its-witness.loft`, `ownership/O-Witness/a-mixed-local-viewing-an-index-element-releases-through-its-witness.loft`, `ownership/O-Witness/a-mixed-local-viewing-a-sorted-element-releases-through-its-witness.loft`, `ownership/O-Witness/a-mixed-local-viewing-a-spatial-element-releases-through-its-witness.loft`, `types/C-Var/a-tuple-member-is-typed-by-its-declaration-a-variant-one-included.loft`, `types/N-Shape/1493-a-branch-arm-yielding-a-locals-field-delivers-a-copy.loft`, `types/N-Shape/a-record-local-reassigned-after-a-literal-build-releases-what-it-displaces.loft` | — | — |
| **2a** status | **Probed 2026-11-27** (`LOFT_PASS2_REPEAT=1`): a repeated pass 2 cures loft#2660's leak (7, no store left), but the premise FAILS as built — the repeat raises 373 false `dead-assignment` warnings across the stdlib (the variable tables carried from the passes before confuse the lints) and on C one return changed again.  So the repeat needs phase 3's clean tables, and the diagnostics of the FINAL pass only: phase 2 waits on 3, or takes both together. | — | — |
| **3** status | **Built 2026-11-27**: `strip_carried_deps` runs on every pass 2 (no switch).  Three shapes leaned on the carried lists, each now pass 2's own: (1) a temp re-minted at pass 1's site keeps pass 1's TYPE, so `unique` / `match_subject` adopt pass 2's deps at the same shape (`adopt_remint_deps`: the match-subject borrows, 11 cells); (2) a LATER bind that makes a local a view is a lookahead, kept apart as `pass1_deps` and asked by `viewed_in_pass_one` at the literal-in-place site (13 cells); (3) a pass-1-only inline-ref temp took a preamble init after a tail match (`reads_var` skips a temp the body names nowhere; N-Shape 1493 on C); (4) a `par` worker's return deps are its ATTRIBUTE indices and the result binding read them as frame variables (`[0]` named whatever variable 0 was), so `par_result_borrows` maps them onto the element's vector and the extra arguments (tests/scripts/1044's pass-through worker, a double free under poison).  Beside (4), loft#2670 (on main, not this plan's): a whole bind of a par element or result did not copy (`lower_whole_binds`).  The store records follow a `par` row of an element-answering worker, so (4)'s double free reads CAUGHT with its cure off.  The suite: 3356 of 3356 on both backends; the full gate green.  Guard: `ownership/O-Borrow-Final/2660-a-callers-borrow-list-is-the-final-passs.loft` (red on main, both backends).  What stays: `probe_below` — loft#2660's program with `f` declared BELOW `probe` — still leaks one store; phase 2's cell. | — | — |
| **2** status | **Built behind `LOFT_PASS2_REPEAT=1`, 2026-11-28; not the default.**  `Parser::settle_returns`: after pass 2 and its closing step (`close_pass2`: `after_pass2`'s promotions settle a return too), pass 2 repeats while a function whose protocol changed is called from a body the parse reached BEFORE it (`body_order` — not definition numbers: a call to a function declared below mints its definition at the call), at most twice, each run rewinding the diagnostics to pass 2's mark; `Function::append` now clears the dead-store tracking, which removes 2a's 373 false warnings.  Cures loft#2692 (`[4,6] [107,109]`, both backends).  Two findings keep it off: (1) a RECURSIVE callee's protocol does not settle — loft#2660's program with `g` declared below goes `[0]` → `[1]` → `[1, 0]` on the repeat (its recursive call, read as buffer-delivering, re-adds the parameter borrow), so 2a's monotone hypothesis is false for recursion; (2) with the switch on, three suite cells over a function declared later fail on both backends (F-Ret text-match tail, F-Render 1270, M-Match over a later function's value).  loft#2693 is not cured either way.  Next: why the repeat re-derives a borrow for a recursive tail (the return's deps are merged into the previous pass's, not recomputed?), then the three cells. | — | — |
| **2a** | Probe Q1's premise: run pass 2 a second time on loft#2660's program and one census program, and read whether the second pass reaches the settled returns with nothing else changed | the probe's IR equals pass 2's for every function whose callees did not change; the census reads 0 harmful for the probed programs | XS |
| **2** | Settle the returns (Q1's mechanism): pass 2 is repeated while a function's return protocol changed between two passes, bounded, with the bound's reason stated (phase 2a measures whether the sequence is monotone — the hypothesis is that a return only gains a buffer or loses a borrow; a cycle that does not settle is reported, never looped) | the census ratchet reads 0 harmful; `introspect_diff.sh` (both writers) differs ONLY on the census programs; `ownership_drop_gate` on both backends; the drop/poison/stack sweeps; compile time of the corpus measured before and after | M |
| **3** | Stop the union: `Function::append` no longer carries variable dependencies; each of the ~18 shapes computes its own in pass 2 (pattern captures, materialised views) — one shape per step, its cell the gate | phase 1's 18 cells green at each step; loft#2660's cells green; `own_sweep.py` reads CAUGHT on each cell with the cure switched off (the store-records rule: compiler, records and checker together) | M–MH |
| **4** | Retire the per-shape repairs the settlement makes dead (`adopt_fnref_ret`, 1466's site, the others the census names): each removed only when its cell still passes and the emission is byte-identical without it | per repair: its cell green, `introspect_diff.sh` IDENTICAL | S |
| **5** | The self-hosted front end follows (`selfhost/src/scope.loft`, `el_*`): the same settled protocol and no carried dependencies | gate E agrees on phase 1's cells; no agreeing program lost; the scope survey's agree count does not fall | S–M |

Each phase keeps the old answer beside the new one where it can (the census, the byte-identical
emission diff), so a phase that changes more than its census predicts goes red before it lands.

## Design questions

1. **How pass 2 reads settled returns.**  The returns are only known after pass 2's body
   analysis (`materialize_view_return`, `ref_return`), so `between_passes` alone cannot settle
   them.  Three mechanisms:
   - **Repeat pass 2 until the returns are stable** (recommended).  `parse_file` already re-runs
     from `data.reset()`, so a third pass is the existing code path.  It runs only for a program
     whose census is non-empty (43 of 2,557 today), so the other programs cost nothing.  Its
     premise, that a repeated pass 2 is idempotent where nothing changed, is phase 2a's probe.
     The cost is compile time on those programs, measured in phase 2 (the owner's rule: shipped
     code speed first, build cost measured).
   - **Callees first over the call graph**, iterating each recursive cycle to a fixed point.
     It is the exact form of the direction, but pass 2 is a whole-file parse in source order
     today; per-function ordering would restructure the parser.  It takes over only if 2a's
     probe falsifies the repeat.
   - **Re-run only the callers of a changed function.**  It is the cheapest at run time, but the
     parser cannot re-parse one function body on its own, so it has the same restructuring cost.
2. **The rule's home.**  Recommended: a rule in `formal/ownership.md` beside `(O-Move)`: *"a
   function's return protocol is one fact; every call reads its final value, and a variable's
   dependencies are those of its final pass"*.  It is cited at `between_passes`, at the repeat,
   at `Function::append`, and at each phase-3 shape site; its first guard is loft#2660's cell.

## Cross-arc dependencies

- loft#2660 closes with phase 3, and its cell flips there.
- The self-host port (@PLN91) owns phase 5.  Its scope pass ports the same analysis, so the
  stream that owns the scope-pass port is the natural builder of 2–4 too.
- @PLN232 (`-> &T` returns) reads `returned().depend()` to decide that a return names one
  parameter (its Q4).  It builds on settled answers, so this plan landing first removes a class
  of wrong `Deps` it would otherwise inherit.
- The store-records checker (`STORE_RECORDS.md`, @PLN228): each phase-3 shape updates the
  records and must read CAUGHT with its cure off.

## Documentation

- `formal/ownership.md`: the rule (Q2); `OPEN:` and the D-own entry for loft#2660's class.
- `COMPILER.md` / `INTERNALS.md`: the passes, and when pass 2 repeats.
- `STORE_RECORDS.md`: what a call's summary reads once returns are settled.
- `OWNERSHIP_MODEL.md`: the pass-1 union retired.
- `CHANGELOG.md`: Bug fixes (the leak class).
- The `loft-codegen` skill: "a return protocol is settled; do not repair a caller for a stale
  one" in § Zeroth.

## See also

- loft#2660 (the diagnosis and the corpus measurements), loft#1466, loft#1181 (`adopt_fnref_ret`).
- [formal/ownership.md](../formal/ownership.md) `(O-Move)`, `(O-Borrow)`;
  [STORE_RECORDS.md](../STORE_RECORDS.md); [DESIGN_PROTOCOL.md](../DESIGN_PROTOCOL.md).
- [@PLN233](https://github.com/loft-lang/plans/issues/233); [@PLN232](232-place-returns.md).
