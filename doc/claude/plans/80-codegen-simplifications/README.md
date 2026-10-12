<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN80 — Codegen simplifications (post-09 follow-ups)

## Status

**CLOSED 2026-07-09 — superseded (same disposition as @PLN79 / @PLN81).**  Tier 1
shipped (2026-05-02).  Tier 3 (template migration) = @PLN81, now **closed by
decision** — [DESIGN_DECISIONS.md § @C87](../../DESIGN_DECISIONS.md) keeps the
`#rust"..."` template path and **reverses** the direction (fold the ~5 emitters
INTO `#rust`, not the other way), so Tier 3 is dead.  Tier 2's arm migration is
already largely done: `src/generation/dispatch.rs::output_call_inner` is a clean
registry-first 3-way dispatch, phase 03's format/append Ops are registered
(`ops/text_ops.rs`) and phase 04's free Ops too (`ops/ref_ops.rs`, header:
"Migrated out of dispatch.rs::output_call_inner").  Only phase 05
(`narrow_int_cast` dual-role split) is undone, and this plan's own README marks
it a **driverless revisit-note** ("holds the design for the eventual split when a
future bug surfaces") — a revisit condition, not a paused deliverable.  Original
tier table follows.

| Tier | Phases | State |
|---|---|---|
| **1** — Correctness + cleanup | 01 (walker audit) + 02 (forwarding-smoke retire) | **SHIPPED 2026-05-02** on branch `plan-12-codegen-simplifications` (commits `c0c27e5` + `d446e5d`).  Reference for the walker-unspan convention lives in [NATIVE.md § Walker convention](../../NATIVE.md#walker-convention--always-unspan-before-matching-value).  The forwarding-smoke retirement is a one-time cleanup; the residual recipe stays in [NATIVE.md § Forwarding-first recipe](../../NATIVE.md#forwarding-first-recipe-verify-before-writing-real-emission). |
| **2** — Structural cleanup | 03 (format/append dispatch arms) + 04 (free/record dispatch arms) + 05 (`narrow_int_cast` split) | **DEFERRED.**  Phase 6 file relocated to [`../13-rust-template-migration/`](../81-rust-template-migration) (its parent). |
| **3** — Deep refactor | (template migration) | **DEFERRED to @PLN81** — see [`../13-rust-template-migration/`](../81-rust-template-migration). |

This plan stays in `deferred/` because Tier 2 phases 03-05 remain.

**Trigger to unpause Tier 2** (per
[`../../DEFERRED.md`](../DEFERRED.md)): same conditions as
@PLN81 — 3+ template-path bugs, major codegen evolution forcing
≥50 Op-annotation touches, or contributor appetite for a large
structural refactor.  Plan-12 Tier 2 is @PLN81's preamble; if
@PLN81 stays parked, Tier 2 doesn't earn its keep.

## Tier 1 outcome

### Phase 01 — Walker audit (closed 2026-05-02)

[`01-walker-audit.md`](01-walker-audit.md) — full implementation
record retained as historical archaeology.

Patched 16 walker sites across 3 files (`pre_eval.rs` + `emit.rs`
+ `coroutine.rs`) to call `.unspan()` before matching `Value::*`
variants.  All sites were latent — no in-tree miscompile
reproducer surfaced — but the byte-identical baseline confirmed
the unspan adds emit identically when input IR isn't Span-wrapped,
and kicks in correctly when it is.

The HIGH-severity site (`value_mentions_var`'s recursive walker
that propagates to `target_used_between`'s collapse-safety
decision) was patched defensively as the plan prescribed.  Pattern
is now @P204-style insurance, not bug fix.

Over-eager fixes reverted before commit (`needs_pre_eval`,
`create_stack_var`, `collect_pre_evals_inner` arg-handling sites,
`body_is_only_create_stacks` filter): each caused byte-identical
baseline to diverge because Span wrappers don't reach those leaf
sites in practice.  **Lesson**: only patch sites the plan
explicitly identifies; each `.unspan()` addition is a behaviour
change that must be validated against the byte-identical baseline.

Structural guard `pre_eval_walkers_unspan` in
`tests/codegen_emitter.rs:769` slices `patch_hoisted_returns` +
`value_mentions_var` and asserts every `matches!(op, Value::*)`
site is paired with `.unspan()`.  Prevents @P204-style regressions.

### Phase 02 — Retire `forwarding_smoke.rs` (closed 2026-05-02)

[`02-forwarding-smoke-retire.md`](02-forwarding-smoke-retire.md) —
full implementation record retained as historical archaeology.

Removed `src/generation/ops/forwarding_smoke.rs` and the 9
forwarded Op-name registrations in `build_registry`.  Plan-09 +
@PLAN11 shipped 5 production custom emitters
(`ParallelForEmitter`, `OpGetRecordEmitter`, `OpIterateEmitter`,
`ParallelQueueEmitter`, `ParallelBufRenameEmitter`,
`IntCompareEmitter`) which proved the dispatch path was exercised
end-to-end; the smoke-test forwarding entries became dead-weight.

Zero behavioural change — forwarding emitters delegated verbatim
to `DefaultEmitter`.  All gate suites stayed at their expected
counts: 540/540 issues, 43/43 threading, 35/35 threading_chars,
95/95 native, 18 codegen_emitter (the +1 is @PLN80 phase 01's
`pre_eval_walkers_unspan`).

The forwarding-first recipe itself stays valid — see
[NATIVE.md § Forwarding-first recipe](../../NATIVE.md#forwarding-first-recipe-verify-before-writing-real-emission)
for the residual one-shot pattern (write the forwarding emitter
inline as a verification one-shot when adding a new Op, then
swap in real logic).

## Tier 2 — Deferred phases

### Phase 03 — Migrate format / append dispatch arms

[`03-dispatch-format-append.md`](03-dispatch-format-append.md)

12 special-case match arms in `dispatch.rs::output_call_inner`
covering format-string concatenation + append targets that today
short-circuit through `dispatch.rs` rather than the per-Op
emitter registry.  Each arm migrates independently; per-arm
regression test in `tests/codegen_emitter.rs` catches any
emission divergence.

Open.  Independent of phases 04 and 05.

### Phase 04 — Migrate free / record dispatch arms

[`04-dispatch-free-record.md`](04-dispatch-free-record.md)

10 special-case match arms in `dispatch.rs::output_call_inner`
covering `OpFreeRef` (debug-name string + store_nr reset) and
record-construction shapes that today short-circuit through
`dispatch.rs`.  Same migration pattern as phase 03.

Open.  Independent of phases 03 and 05.

### Phase 05 — Split `narrow_int_cast` dual role

[`05-narrow-int-cast-split.md`](05-narrow-int-cast-split.md)

`src/generation/emit.rs::narrow_int_cast` serves two roles:
block-tail-expression coercion AND parameter narrowing.  Plan-09's
phase 02 was scoped to split it but DEMOTED via phase 00a's
finding that the actual @P200 bug was at the comparison level
(closed via `IntCompareEmitter`), so the split was no longer on
the critical path.  Phase 05 holds the design for the eventual
split when a future bug surfaces that genuinely needs the dual
role decoupled.

Open.  Independent of phases 03 and 04.

## Tier 3 — Relocated to @PLN81

Phase 06's stub
([`06-rust-template-migration-stub.md`](06-rust-template-migration-stub.md))
points at [`../13-rust-template-migration/`](../81-rust-template-migration),
which holds the deferred deep refactor (unify the
`#rust"…@v0…"` template path with the registered emitter path —
~200 Ops migrate; H effort).

## Sequencing (when Tier 2 unpauses)

Phases 03, 04, 05 are independent of each other and can land in
any order or in parallel.

## Acceptance gate (per phase commit)

```bash
cargo build --release --tests
cargo test --release --test issues 2>&1 | tail -3        # 540/540
cargo test --release --test threading 2>&1 | tail -3     # 43/43
cargo test --release --test threading_chars 2>&1 | tail -3  # 35/35
cargo test --release --test native -- --test-threads=1 2>&1 \
    | grep "native result"  # 95/95 (plan-09 + plan-11 floor)
cargo test --release --test codegen_emitter 2>&1 | tail -3  # ≥18
cargo fmt --all -- --check
cargo clippy --tests --release -- -D warnings
cargo check --no-default-features
scripts/p09_fast_gate.sh
```

Per
[`feedback_zero_regression_tolerance`](../../../../home/ubuntu/.claude/projects/-home-ubuntu-loft/memory/feedback_zero_regression_tolerance.md): <!--noindex-->
any regression aborts the commit.

## Risks

| Risk | Mitigation |
|---|---|
| Dispatch arm migration breaks emission for one Op | Each arm migrates independently; per-arm regression test in `tests/codegen_emitter.rs` catches it.  The forwarding-first recipe applies. |
| `narrow_int_cast` split surfaces a third role | Stop and document; phase 05 may need sub-phasing. |
| Plan-12 Tier 2 ships in parallel with @PLN81 (template migration) and they conflict | Plan-13 doesn't open until @PLN80 Tier 2 merges; sequence enforced via plans/README convention. |

## See also

- [NATIVE.md § Walker convention](../../NATIVE.md#walker-convention--always-unspan-before-matching-value)
  — the Tier 1 phase-01 convention extracted as contributor reference
- [NATIVE.md § Forwarding-first recipe](../../NATIVE.md#forwarding-first-recipe-verify-before-writing-real-emission)
  — the residual recipe after Tier 1 phase-02 retirement
- [`../../finished/09-native-runtime-rewrite/`](../finished/09-native-runtime-rewrite)
  — parent plan whose audit surfaced these simplifications
- [`../../finished/11-p204-ref-propagation/`](../finished/11-p204-ref-propagation)
  — surfaced the Span-miss walker pattern that phase 01
  generalised
- [`../13-rust-template-migration/`](../81-rust-template-migration)
  — Tier 3 deferred deep refactor
- [`../../DEFERRED.md`](../DEFERRED.md) — trigger row for
  Tier 2 unpause
- `tests/codegen_emitter.rs::pre_eval_walkers_unspan` — Tier 1
  structural guard
- `src/generation/ops/` — per-Op emitter implementations (Tier 2
  migration target)
