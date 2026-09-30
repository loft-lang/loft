<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN94 Check C — under-free / leak detection (design, measured before committing)

> **STATUS (2026-07-07):** the definite-leak scan (`run_leak_scan`) is PROMOTED onto `check`; the
> over-free **Check B** (`run_over_free_check`) is ALSO promoted (its true-positive is
> `LOFT_OWN_INJECT_FREE_BORROWED`). `check-dev` now retains only the exit-state Check C as a second
> opinion. The "promotion waits" verdicts below are the historical record of the journey, superseded
> by the promotions — kept for the measurement trail.

Design Protocol 1: a throwaway gated prototype (`LOFT_OWN_UNDERFREE`) was built and **measured
against real corpora BEFORE the real design** — store-lifetime is an exact-invariant domain, so the
numbers, not intuition, fix the scope. The prototype has been reverted; its measurements are the
foundation below.

## The scope the measurements forced (read this first — it is narrower than it looks)

Under-free is **not** the mirror image of over-free. Two obstacles a static check faces that the
over-free checks (A/B) do not:

1. **Runtime-path leaks are statically invisible.** `LOFT_NO_JOIN_OWN` leaks a store on `local_source`
   (`leak=1`) — but the prototype flagged the SAME count (12) on the correct plan and the leaky one.
   Its leak is a `Join` store freed **conditionally** (`OpFreeRefIfDistinct`): the free op is
   statically PRESENT, it just does not execute on the leaking path. A static "no free op exists"
   check cannot see that. **This class is structurally the runtime leak-check's** (`check_store_leaks`).
2. **Transfer-completeness governs the false-positive rate.** A leak is "an owned store never freed
   AND never transferred out." Miss a transfer path → cry wolf on a moved value.

So Check C's honest target is the **DEFINITE leak**: an `Owned` heap-store local with **no free op on
any path** and no transfer out — a store leaked on *every* execution. Sound for that subclass (a flag
is a real leak, modulo transfer-completeness); it does **not** catch conditional/`Join` leaks.

## C.0 — the enabling PREREQUISITE (found while building C.1; it is a BLOCKER)

Building the transfer-out set (below) drove the false-positive rate to 0–4/file — then the residuals
exposed a deeper constraint. `n_main`'s free set came back **empty** (`freed={}`) even though its
compiled IR has 7 `OpFreeRef`s. Cause: **the oracle observes the PRE-codegen IR.** `oracle(data)` is
the third line of `scopes::check` (`src/scopes.rs:1554`); `check` ends at line 1762; `get_free_vars`
(which inserts the frees) runs at ~1877, during codegen, **after** `check`. So at oracle time a
freshly-checked USER function's `def.code` has **no frees yet** — they are appended later.

Consequences:
- **Check C cannot run at the current call site** — its whole premise is reading the emitted frees.
- **Check B is near-VACUOUS on user functions today** — the same reason. Its 0-RED across 377 scripts
  is partly because it had no user-function frees to inspect (it meaningfully checks only pre-cached
  stdlib, whose `def.code` carried frees from an earlier compile). This is a real limit of the
  shipped check, surfaced here — worth widening when C.0 lands.
- **Check A is unaffected** — it is fact-based (needs no frees), which is why the A1b catch works.

**C.0 = give the free-based checks a POST-codegen view. ✅ BUILT (dev tier, 2026-07-07).**
`ownership_cfg::oracle_free_checks` runs at the END of `scopes::check` (after `get_free_vars`),
gated on `LOFT_OWN_ORACLE=check-dev`; Check A stays at the pre-codegen site. It is a pure observer
(SI-1 held) and NEVER on the default `check` path.

**C.0 immediately exposed the REAL blocker (the 377-script sweep, not the tiny corpus).** With frees
now visible, Checks B + C produce **153 findings across `tests/scripts`** — and they are dominated by
ONE root cause: **the ownership fact is not materialisation-aware.** A struct copy `r1 = a` (a param)
reads `Borrowed(a)`, but the shipped code COPIES it and frees the owned copy — so:
- Check B fires: `free-of-borrowed r1 is Borrowed(a)` (it IS freed, legitimately, as an owned copy);
- Check C fires: loop/iteration/container temps whose owned-copy frees the fact does not see as owning.
This is the `n_choose` gap, pervasive. **The work Check C needs is not more transfer-set tuning — it
is a materialisation-aware FACT** (classify a copied `x = borrowed-source` as `Owned`, matching what
codegen emits). That is a Phase-3-level precision upgrade.

**The workflow (so the checks are never reverted again): a gated dev tier + a FP RATCHET.** The dev
checks live in the code behind `check-dev`; `tests/ownership_oracle.rs::oracle_dev_free_check_ratchet`
(`#[ignore]`) counts their findings over `tests/scripts` and asserts `≤ DEV_FP_BASELINE`. Every
improvement LOWERS the baseline; a regression fails; at 0 they promote into `check`.

**RETIRED (2026-09-29, owner).** `oracle_dev_free_check_ratchet` is removed: nothing ran it after
July, it had drifted to 449 unexamined findings, and the checker it counted is superseded on `check`
by the leak scan.  The leak scan's own ratchet (`oracle_leak_scan_ratchet`) is the one kept, at 0, on
every PR (ci.yml's advisory `leak-scan` job).

**RESULT — ratchet 153 → 0 (2026-07-07).** The "materialisation-aware fact" is: use the POST-codegen
**type dep** as the ownership signal, not the flow-sensitive fact. The borrow-elision decision is
usage-dependent (VH to replicate independently) but post-codegen it is baked into the dep — a copied
`ac_copy = f(a, __ref_2)` has an EMPTY dep (owns). So Check C tests `tp(v).depend().is_empty()`;
Check B flags a freed var with a NON-empty dep. Drove 153 → 0 over 377 scripts + 54 fuzz + 7 probes
via: the type-dep signal (under-free 120→4); `transferred_out += OpSetDbRef` capture (up_a/uv_a); and
narrow documented exclusions — `__ncc_*` (stale present-arm dep after default-arm materialisation),
self-dep work-refs (@P302), freed params (retbuf displacement), closure bodies. `DEV_FP_BASELINE=0`.

**HONEST characterisation (the tradeoff the type dep buys).** Using the dep makes B/C **CONSISTENCY
checks** — free-placement vs the ownership dep, catching a `get_free_vars`/dep DIVERGENCE — NOT
*independent* cross-checks. `LOFT_NO_JOIN_OWN` is NOT caught by B/C (its dep and frees are
consistently wrong — that stays Check A's independent job and the runtime leak-check's). The
independence in @PLN94 lives in Check A (the fact vs `ownership_of`); B/C are the free-placement
consistency layer beside it.

**PROMOTION ATTEMPT (2026-07-07) — blocked on Check C's true-positive; no-false-positive gate PASSED.**
- **No crying wolf ✓** — 0 across `tests/scripts` + `tests/docs` + `tests/lib` + `examples` (521
  files) + 54 fuzz + 7 probes. Broad and clean.
- **True-positive ✗ (the blocker)** — a genuine injected leak (drop the scope-exit free of an
  `OpDatabase` db-var like `__vdb_1`) is NOT caught by the exit-state-scoped Check C: a db-var's only
  owns-entry is `= null`, so it is absent from the fixpoint state. Iterating ALL vars
  (`snapshot_names`) catches it — verified via a `get_free_vars` fault-injection hook — but
  over-approximates leaks: backing stores of returned values (`buf["__vdb_1"]` carries `__vdb_1` out),
  `par` materialise temps, retbufs, `__ncc` in the leak direction. Closing those by name-exclusion is
  whack-a-mole (the design-protocol tell); the sound fix is **comprehensive transfer/free tracking**
  (a returned value's transitive backing is transferred; `par`/coroutine frees; the retbuf return) —
  a VH increment. Reverted the all-vars change + the fault-injection hook (a per-free env read in the
  hot path; re-add read-once with the all-vars work).

**Verdict:** Check B/C stay CLEAN + gated in the dev tier; promotion waits on the all-vars leak scan +
comprehensive transfer tracking. The fact-precision win (153→0) and the dev-tier/ratchet workflow
stand; the true-positive is the next real piece.

**UPDATE (2026-07-07) — the leak scan is now a CLEAN, SOUND definite-leak check (its own `check-leak`
tier).** Applying the "raise-it-→-flag-it, never revert" rule, the all-vars scan was parked behind
`LOFT_OWN_ORACLE=check-leak` with its own ratchet (baseline 927), then chipped down:
- **The MINTED recognizer** (only an `OpDatabase` target owns a store; a type-dep-only phantom like
  `__retbuf` has none) dropped **927 → 0** — `__retbuf` alone was ~889, and the rest were non-minted
  temps.
- **The transfer split** (the bug the true-positive exposed): RETURN seeds close transitively through
  the dep; consume/capture seeds do NOT (an element absorbed into a LOCAL container leaves the
  container leak-checked). + the shipped `skip_free`/`caller_hidden_buf` flags.
- **True-positive fires:** a cached test-only `get_free_vars` hook `LOFT_OWN_INJECT_DROP_FREE` drops a
  named owned var's free; `oracle_leak_scan_flags_an_injected_leak` asserts `check-leak` RED on
  `__vdb_1` (probe 07) — not vacuous. 0 FP across scripts+docs+lib+examples (~521 files).
- **KNOWN GAPS (documented, not FPs):** conditional/`Join` leaks (`LOFT_NO_JOIN_OWN` — the runtime
  leak-check's class, BY DESIGN) and closure bodies (frees on a different codegen clock). The
  adopted-owned non-`OpDatabase` class is now CLOSED — see the update below.

So the under-free direction the user asked for is REAL now — a sound definite-leak scan PROMOTED onto
`check` over two owned-store classes (`OpDatabase`-minted + adopted NRVO buffers), `LEAK_SCAN_BASELINE=0`
re-armed as a regression guard.

> **UPDATE (2026-07-07) — adopted-owned class CLOSED + promoted.** A work-ref (`__ref_*`/`__rref_*`)
> NRVO return buffer a function owns + frees but never `OpDatabase`-mints was the demonstrable gap: with
> its free dropped, the runtime leak-check flags it but the OpDatabase-only scan missed it. Recognizer:
> a work-ref passed as a `Var` argument to a call (the real-vs-`__retbuf`-phantom discriminator). The
> `!caller_hidden_buf` exclusion had to be DROPPED — that flag is the NRVO-buffer codegen tag, not a
> "freed elsewhere" semantic; correct code still excludes the buffer via `!freed`/`!closed`, so the drop
> is FP-safe (verified 0 FP across 829 files). Folded into `run_leak_scan` (no gate); true-positive
> `oracle_adopt_leak_flags_an_injected_leak` (probe 09). Conditional/`Join` stays the runtime check's;
> closures stay blocked (frees on a different clock).

The false-positive MACHINERY already works: the heap-type filter (70–124 → 9–35/file) and the
transfer-out set (returned ∪ consumed-into-container, → 0–4/file on the small corpus) are in
`run_free_checks`; the residual 153 is the fact-precision blocker above, not the machinery.

## Coexistence — what it adds, what it does NOT replace

Check C runs BESIDE the shipped **runtime** leak-check, catching a **complementary** sub-class:

| | catches | misses |
|---|---|---|
| **runtime leak-check** (`check_store_leaks`, `LOFT_STORES=warn`) | any leak on an EXECUTED path (incl. conditional/`Join`) | leaks on paths a given run does not take |
| **Check C** (static) | DEFINITE leaks on ALL paths — incl. a deleted free the tests never execute | conditional/runtime-path leaks (the `Join` class) |

Neither replaces the other; the pair covers more. This EXTENDS the oracle from the over-free class
(A/B) to the under-free class — the direction the user asked for — without pretending to subsume the
runtime detector.

## The invariant

> **An `Owned`, HEAP-typed, LOCAL var (not a parameter) that appears in NO free op anywhere in the
> function body and is not transferred out (returned, moved into a container, or adopted by a callee)
> leaks its store — RED.**

## What the prototype measured (the failure paths, in order of magnitude)

Gated prototype on correct corpora (06-capture, 505, fuzz cells):

- **Raw: 70–124 false positives / file.** Dominated by SCALARS classified `Owned` (loop counters
  `k#index`, `i`, `p`; ints `np`, `depth`) — a scalar has no heap store to leak.
  → **Fix 1 (heap filter): `func.tp(v).heap_dep().is_some()`.** Drops FPs to **9–35 / file**. Cheap,
  done in the prototype.
- **Residual 9–35 / file: ONE class — element/scratch temps consumed into a container.** `_elm_1`,
  `_elm_2` (`OpNewRecord` → `OpFinishRecord(container, _elm_N)`), `_hash_scratch_1`, `_reduce_acc_1`.
  These are **moved** into a collection / reduction, not leaked; the prototype's return-only transfer
  set misses them.
  → **Fix 2 (consume-tracking): the real work — extend the transfer-out set.**

## The transfer-out set (Fix 2 — the load-bearing piece)

A var is transferred out (⇒ NOT a leak even though this frame does not free it) iff it is:

- **returned** — appears in a `Value::Return(…)` (prototype has this), OR its store backs the return
  (named in `def.returned` deps);
- **consumed into a container** — the element/source arg of `OpFinishRecord(container, v, …)`,
  `OpAppendVector(container, v, …)`, or `OpCopyRecord(v, dst, …)` with the source-free bit;
- **adopted by a callee** — passed as an argument to a call whose callee frees that parameter (read
  the callee's own free set / `deps`; conservative default: a heap arg to a non-native call is
  assumed possibly-adopted → excluded, biasing toward no-cry-wolf).

**Bias, stated:** when unsure whether a path transfers, EXCLUDE (treat as transferred). That
under-approximates the leak set — Check C may MISS a leak — which is SAFE for a gate (the runtime
leak-check is the completeness backstop) and keeps it from crying wolf. Mirrors Check B's conservatism
(unconditional frees only).

## Re-assertion sites — N = 1

One pass over the exit-state fact + the (freed ∪ transferred-out) set. No spray; a new container/consume
op is one more entry in the transfer-out set, `log()`ged if unmodeled.

## Steps (each independently committable, each states its gate)

- **C.0 — the free-based checks get a POST-codegen view (BLOCKER, see above).** Add a second oracle
  entry point after codegen (or thread the pre-codegen fact to a post-codegen free-walk); prove Check
  B now sees user-function frees (its coverage was vacuous there). **Gate:** on the A1b test,
  `collect_free_targets(n_h)` is non-empty; SI-1 still holds (still an observer). Everything below
  stacks on this.
- **C.1 — heap filter + returned + consume-into-container.** Re-add the check (gated
  `LOFT_OWN_UNDERFREE` first) with Fix 1 + the return and `OpFinishRecord`/`OpAppendVector`/
  `OpCopyRecord` transfer set. **Gate:** FP on the correct corpus (7 probes + 505 + 54 fuzz cells)
  drops from 9–35/file to a small hand-verifiable set; adjudicate each residual (a real gap → extend
  the set; a real leak → keep).
- **C.2 — callee-adoption.** Add the "heap arg to a call whose callee frees it" exclusion (conservative
  default: exclude). **Gate:** FP → 0 across the correct corpus + fuzzer; SI-1 holds.
- **C.3 — true-positive (the 4.3 injected fault this finally enables).** Hand-delete one `OpFreeRef`
  from a corpus program's emitted plan (or a test fixture) → Check C RED, naming the exact store +
  function; the un-mutated program clean. Document that `LOFT_NO_JOIN_OWN` (a CONDITIONAL leak) is
  NOT caught here — it is the runtime leak-check's, by design.
- **C.4 — promote off the gate + land.** Fold Check C into `check` mode (ungated), add
  `oracle_flags_a_deleted_free` + extend `oracle_clean_on_correct_corpus` to assert 0 under-free REDs
  in `tests/ownership_oracle.rs`. **Gate:** the binary is green.

## Failure paths (enumerated)

- **Transfer set incomplete → false positive** (crying wolf on a moved value). The 9–35 residual is
  exactly this; C.1/C.2 close the known shapes; the conservative-exclude bias caps the blast radius.
- **Transfer set too broad → missed leak** (false negative). Accepted by design — the runtime
  leak-check backstops; a static gate that cries wolf is worse than one that occasionally defers.
- **Conditional/`Join` leak → structurally out of scope** (the `NO_JOIN_OWN` measurement). Not a bug
  in Check C; `log()` nothing — it is simply the other detector's class.
- **Text vars** — freed via `OpFreeText`; include it in the free set (unlike Check B, which excluded
  it — there the issue was over-free of a copied sub; here we need the free to COUNT so text locals are
  not spurious leaks).

## Relation to the formal skeleton

Check C proves the DUAL of Check B's over-free lemma: *every `Owned` heap local is freed exactly once
on every path (no leak)* — the O-Derived "free … once, at scope exit" half. It is the
**freed-exactly-once sub-invariant** the `formal/ownership.md` obligation ledger lists as OUT OF SCOPE
for the over-free proof; Check C brings the DEFINITE-leak fragment of it in scope, the conditional
fragment staying with the runtime witness. Update the ledger when C.4 lands.
