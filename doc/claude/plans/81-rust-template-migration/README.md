# @PLN81 — `#rust"..."` template migration

**Status: CLOSED — decided against (won't-do) 2026-07-08.** The premise inverted since this stub
was opened. When filed (2026-05-02), `#rust"..."` looked like a redundant second emission path to
delete (migrate all ~200 stdlib sites to hand-written `OpEmitter`s, then retire
`output_call_template` + `Value::RawExpr`). Since then `#rust` **inline** became a first-class,
*recommended* library-authoring mechanism — the loft-ship **Tier-1** path ("prefer `#rust` inline
over `#native` external whenever the Rust is small"), ✓ across all four targets in
[PACKAGES.md](../../PACKAGES.md). So it's a **kept public feature**, not debt: deleting the template
path would break the documented `#rust` inline library route, and this doc's own "cost" section
names the regression — a new Op is a one-line `#rust` annotation today vs a struct + impl + register
call after. The real concern (one less-bug-prone emission path — the @P203 double-substitution
class) is better served by HARDENING the template path (the differential oracle + regression guards,
2026-07) and keeping `#rust` as the co-located library-facing path. If consolidation is ever wanted,
the correct direction is the REVERSE (fold the ~5 emitters into `#rust`), which is a fresh plan, not
this one. See [DESIGN_DECISIONS.md](../../DESIGN_DECISIONS.md). The design sketch below is retained
as a historical record.

**Amended 2026-09-29 (C87):** the library half of that premise never held — the parser accepted
`#rust` only in `default/`, and no library used it.  `#rust` is now stated and enforced as the
standard library's template (refused elsewhere, pointing at `#native`); the reason to keep it —
one line per stdlib operator — stands.

---

## Goal

Migrate every `#rust"..."` annotation in `default/*.loft` to a
hand-written runtime fn + registered emitter under
`src/generation/ops/`.  After the migration, Op emission has a
single source of truth: the `OpEmitter` registry.  The template-
substitution path in `src/generation/calls.rs::output_call_template`
deletes; `Value::RawExpr` deletes; `default/*.loft` keeps Op
signatures but no Rust bodies.

## Current state (2026-05-02)

Two emission paths coexist:

- **Template path:** ~200 Ops have `#rust"..."` annotations in
  `default/01_code.loft`, `default/02_images.loft`, and
  `default/03_text.loft`.  Codegen substitutes placeholders at emit
  time.
- **Emitter path:** ~5 Ops have hand-written Rust emitters
  registered in the `OpEmitter` registry from @PLAN09 phase 00 +
  @PLAN09 phases 03/04/06/10 + @PLAN11 @P204.

The dual path is functional today but has caused real bugs (@P203
was a template double-substitution; the let-bind-on-repeat fix is
a hack the emitter path wouldn't need).

## Why deferred

No active driver.  The most recent codegen-evolution work
(@PLAN09, @PLAN11) shipped, and the template path is currently
stable.  Plan 13's payoff is a future investment in codegen
maintainability, not a fix for an open problem.

The trigger to unpause:

- **Multiple template-path bugs accumulate** — if 3+ P-issues over
  a few months trace back to template substitution edge cases,
  the per-Op emitter path's "won't bite us next time" value
  compounds.
- **Major codegen evolution lands** — adding a new type system
  feature (e.g. interfaces, generics extensions, new operators)
  forces touching ~50+ Op annotations.  Doing that across the
  dual path costs more than unifying first.
- **Contributor appetite for H-effort structural work** — plan
  13 is large-effort (H) structural work with low user-visible payoff.

Without one of those triggers, plan 13 is busywork.

## What plan 12 owes plan 13

Plan 12 Tier 2 (phases 03-05) is the necessary preamble:

- Phase 03 retires format/append dispatch arms
- Phase 04 retires free/record dispatch arms
- Phase 05 splits `narrow_int_cast`'s dual role

After plan 12 Tier 2, `dispatch.rs::output_call_inner`'s special
match shrinks toward zero, and the migration target shape becomes
uniform.  Without plan 12 Tier 2, plan 13 hits a worse code state
and runs longer.

So if plan 13 ever opens, plan 12 Tier 2 must land first.

## Sketch (for @PLN81's eventual design)

### Phase 13.1 — Catalog all `#rust` annotations

Survey `default/*.loft` for every `#rust"..."` annotation.  Group
by:
- Op family (arithmetic / text / vector / hash / etc.)
- Annotation complexity (single-substitution vs multi-line vs
  conditional-branch)
- Whether the annotation calls a helper fn (`ops::op_add_int`,
  `vector::get_vector`, etc.) or inlines logic

### Phase 13.2 — Auto-translate trivial annotations

The simple annotations (single-substitution to a helper fn) can
auto-migrate via a script:

- Generate a stub runtime fn calling the existing helper.
- Register a passthrough emitter.
- Run byte-identical baseline.

### Phase 13.3 — Hand-port complex annotations

Multi-line annotations and conditional-branch ones (e.g. the
let-bind-on-repeat ones from @PLAN09 phase 00 step 0.7b) need
hand-ported emitters.

### Phase 13.4 — Remove `#rust` parsing path

Once all annotations are migrated, the `#rust"..."` parsing code
in `src/parser/` and the template-substitution code in
`src/generation/calls.rs::output_call_template` can be retired.
`Value::RawExpr` can be retired alongside.

### Phase 13.5 — Validate parity

Native suite stays at 95/95 (or whatever the current PR-ready
floor is).  Per zero-regression rule.

## What plan 13 would deliver

- One emission code path, not two.
- Type-checked, IDE-navigable Op definitions (rename / find-
  references / type-check now work; `#rust` strings are opaque).
- Debuggable codegen: step through emitter functions in `gdb` /
  `rust-lldb`; today you debug template substitution by reading
  generated source + grep'ing `default/*.loft`.
- Eliminates one verified bug class (@P203-style template double-
  substitution).

## What plan 13 would cost

- Large effort (H) — structural refactor across ~200 sites.
- ~2000-6000 lines of new emitter code (templates were dense;
  emitters are verbose).
- Onboarding regression: adding a new Op currently requires a
  one-line `#rust` annotation in `default/01_code.loft`; after
  plan 13, contributors write a struct + impl + register call.
- ~200 migration steps with a long tail of "this template
  behaved differently than my emitter by 0.1%" subtleties.

## Memory candidate

If @PLN81 ever opens, save:

```
project_template_emitter_unification.md — Plan-13 unifies the
`#rust"..."` template path with the registered emitter path.
~200 Op annotations migrate to runtime fns + emitters.  Single
source of truth for Op emission.  Multi-week effort; opens
after plan-12 merges.
```

This stub itself is documentation — no code.
