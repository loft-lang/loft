<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Course of action — handoff (2026-06-26)

Single entry point after a context clear. Read this first; it links everything else.

## Priority (decided 2026-06-26)

> **Brittleness > type-correctness.** The store-lifetime / ownership substrate (the code that
> re-derives ownership/lifetime per site and fails silently — the crash/corruption class:
> #462, the step-6 over-free, the leaks) is the **primary** concern. The nullability rewrite
> (dense default + `?` opt-in) is **designed, documented, and PARKED** — good, but secondary,
> because it polishes type-correctness on top of a substrate that still UAFs.

The key reason they're separable: the dense flip *fixed #462's symptom* (by removing the
tagged-element stores it fed on) but **did not fix the brittle ownership code** — it
re-exposed it as step 6. Fixing the brittleness fixes #462 *at the root*, without the flip.

---

## Current tree state — branch `fix-crawler`, everything UNCOMMITTED

Three threads coexist (see [implementation-steps.md § Reconciliation](../25-nullable-sequences/implementation-steps.md#reconciliation-with-the-in-flight-work-historical--origin-of-the-rewrite)):

| Thread | files | state |
|---|---|---|
| **#460 / #461** (crawler native/packaging bugs) | `src/main.rs`, `src/native_lib.rs`, `tests/n3_use_native.rs`, `tests/exit_codes.rs`, `tests/lib/{binwriter,selfpkg,typeshift}/` | ✅ **green, separable, landable** |
| **dense-vector flip** (#462 + nullability vectors-half) | `src/parser/definitions.rs`, `src/parser/vectors.rs` | ⚠️ **live, 7 tests failing** (4 = Phase-1 not done; 3 = step-6 ownership) |
| **design + investigation docs** | `doc/claude/formal/types.md`, `@PLN25/*`, `@PLN85/*` | ✅ no code risk |

Suite under the live tree: **2542 run, 7 failing** (`plan25_e2_json` ×3, `plan25_e2_hash` ×1,
`wrap loft_suite`, `native native_scripts` — the last two = `150-i306` + `85-borrowed-view`).

### Recommended tree actions before starting fresh

1. **Commit / PR #460 + #461** — they're green and independent. Lock the wins. (`Fixes #460`,
   `Fixes #461`; add the `tests/lib/*` fixtures.)
2. **Revert the dense-vector flip** (`git checkout origin/main -- src/parser/definitions.rs
   src/parser/vectors.rs`) → tree returns **green**, nullability fully parked. **#462 reopens**
   — that is intended: the brittleness work below kills it at the root, the proper fix.
   *(Alternative if you'd rather keep it: do Phase 1 + step 6 to reach green first — more work,
   entangles nullability. Revert is the clean fresh-start.)*
3. Keep all docs.

After (1)+(2): clean green `main`-based branch, nullability parked, ready for brittleness.

---

## STREAM A — the brittleness work (PRIORITY)

> **DONE (first wedge, 2026-06-26):** step-6 `150-i306` over-free fixed at the canonical
> Cluster-A.3 chokepoint, BOTH backends. Root: native's first-bind copy gate
> (`src/generation/dispatch.rs`) re-derived a coarse **type-shape proxy** ("callee has a
> visible `Reference`/`Enum` param") for the own-vs-borrow fact — which MISSED a borrowed
> **vector-element** view (`fn pick(t: vector<M>) -> M { t[i] }`: `t` is a `Vector` param,
> not `Reference`, yet the return aliases it). The bound local was adopted as owned and its
> scope-exit `OpFreeRef` whole-store-freed the caller's vector (recycled under allocation
> pressure → corruption). Interp was already correct — it reads the canonical
> `return_adopts_fresh_store()` fact (`state/codegen.rs` gen_set_first_at_tos); the fix routes
> native through the **same fact**, deleting the proxy (exactly the "read one carried answer,
> don't re-derive" remedy). Regression: `tests/scripts/85-store-lifetime-vector-view-call-bind-overfree.loft`
> (first-bind form; 150-i306 covers the reassignment form). Verified: full /tmp matrix +
> 150-i306 green both backends, leak-free, no suite regression.
>
> **Still open:** `85-…-enum-match-borrowed-view-overfree` fails, but its interp failure is a
> *read returns wrong value* (`found=false`, no crash) — a nullability-**access** correctness
> bug (Phase-1 / Stream B, PARKED), NOT an over-free. It cannot pass without the parked
> nullable-access glue, so it (and the 4 `plan25_e2_*` failures) are Stream-B, not brittleness.
> `#462` (slot-reuse UAF, needs the crawler corpus) remains the next ownership wedge.

**Thesis** (`OWNERSHIP_MODEL.md`): ownership/lifetime is *re-derived per site* — `copy_record`'s
free-source flag, scope frees, the adopt/free thicket, the borrowed-view deep-copy that fires for
one representation but not another. Every site re-decides "do I own / free / copy this store," and
the bugs are the sites that decide wrong (UAF, double-free, leak). The fix class: **compute
ownership once, have every site read it** — the same move that deleted Family N (one computed fact,
not a per-site re-derivation).

**Method** (mirror the nullability investigation that worked this session):
1. **Map the re-derivation** — an honest inventory of every site that re-derives an
   ownership/lifetime fact (own-vs-borrow, free-vs-keep, copy-vs-adopt), like
   [materialisation-site-inventory.md](materialisation-site-inventory.md) did for the type side.
   Start from its **Layer-3** row + `STABILITY_REDFLAGS.md` (the "non-local facts re-derived
   per-site" clusters).
2. **Find the one fact + chokepoint** — what single computed ownership fact would let each site
   *read* instead of *recompute* (the `OWNERSHIP_MODEL` north star: dep = the store a local owns).
3. **Wedge in via the live bugs** — **#462** (slot-reuse UAF) and **step 6** (dense borrowed-view
   over-free, `150-i306` / `85-borrowed-view`) are the same ownership-re-derivation class; use
   them as the concrete entry, matrix-first, then generalise to the chokepoint.
4. **Make violations loud** (`feedback`: brittleness = silent-failure invariants — remove the
   invariant or make its breach a hard error). The `LOFT_UAF` detector exists but only scans
   frame variables; **extending it to operand-stack + vector-element DbRefs** (cluster-462 § Tool
   gaps) is the instrument that turns silent UAFs into named ones.

**Entry docs:** `OWNERSHIP_MODEL.md` (north star) · `STABILITY_ROADMAP.md` / `STABILITY_METHOD.md`
/ `STABILITY_REDFLAGS.md` (the brittleness program) · [cluster-462](cluster-462-slot-reuse-uaf.md)
+ [materialisation-site-inventory.md](materialisation-site-inventory.md) (this session's maps).

**First action:** decide map-first (inventory the ownership re-derivation) vs wedge-first (drive
step-6 / #462 as the matrix-first entry into the substrate). Recommended: **wedge-first on step-6**
— it's a live, reproducible, well-localised instance of the exact class, and the fix will reveal
the chokepoint.

---

## STREAM B — the nullability rewrite (PARKED, fully designed)

Resume any time from these — nothing more to design, only build:

- **The model (contract):** [`formal/types.md` § Nullability](../../formal/types.md) — `τ?` optional
  former, dense storage default, `(N-Intro/Coal/Match/Store)`, `(N-Arith/Cast)` range-driven, the
  partial-vs-total line, `(N-Decl/Join)` declared-vs-inferred. Deviations **DN1–DN4** = the build.
- **The design + rationale:** [storage-vs-access-nullability.md](../25-nullable-sequences/storage-vs-access-nullability.md)
  (why default-nullable broke parametricity; the probe verdicts; #462/N fixed by dense).
- **The build plan:** [implementation-steps.md](../25-nullable-sequences/implementation-steps.md)
  — EXPAND→MIGRATE→CONTRACT→TIGHTEN→CLEANUP, per-site code changes (incl. the `as` two-form
  rework), and the merge discipline.

**Settled design decisions** (don't relitigate): dense-by-default everywhere; `T?` **postfix** the
only nullable marker; `not null` retired; `null` is the universal "doesn't fit / no result" value
(no wrap/saturate/UB); nullability range-driven so no `??` on the default i64 path; `as τ` requires
provable fit (else error → `as τ?`), `as τ?` is the checked cast; declared = commitment,
inferred = join-widened.

**When resumed:** Phase 1 first (annotate nullable sites `vector<S?>` — zero breakage), then the
rest. If the brittleness work fixes #462 at the root first, the dense flip becomes a *pure
ergonomics/parametricity* change (walker, Family N), no longer load-bearing for the crash.

---

## Doc index (everything from this session)

**@PLN85 (store-lifetime / brittleness):**
`COURSE-OF-ACTION.md` (this) · [cluster-462-slot-reuse-uaf.md](cluster-462-slot-reuse-uaf.md) ·
[nullable-materialization-field-map.md](nullable-materialization-field-map.md) ·
[materialisation-site-inventory.md](materialisation-site-inventory.md) · `probes/` (462-*, sib-*,
46A-*, 46N-* + the `## Crawler dogfood wave` table in `probes/README.md`).

**@PLN25 (nullability):**
[storage-vs-access-nullability.md](../25-nullable-sequences/storage-vs-access-nullability.md) ·
[implementation-steps.md](../25-nullable-sequences/implementation-steps.md) ·
`single-payload-refactor.md` (superseded-note added).

**Formal:** `doc/claude/formal/types.md` (§ Nullability rules + deviations DN1–DN4).

**Issues:** #460 (fixed), #461 (fixed), #462 (open — fixed-by-dense-flip-symptom; root is the
brittleness work). Crawler is the consumer (`../crawler`).

---

## TL;DR for the fresh session

1. Commit #460/#461; revert the dense flip (green tree, nullability parked, #462 reopens by design).
2. **Do the brittleness work** (Stream A) — ownership re-derivation is the root of the crash class;
   wedge in via step-6 / #462, matrix-first, drive toward the `OWNERSHIP_MODEL` chokepoint.
3. Nullability (Stream B) is fully designed and parked — resume after the substrate is sound.
