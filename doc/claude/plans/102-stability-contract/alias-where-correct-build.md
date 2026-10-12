<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Building + validating the transparent-link widening — safe small steps

> **Status: build plan (2026-07-16).** The *how* for [alias-where-correct.md](alias-where-correct.md)
> (the *why*): widen the copy-elision so the compiler realizes a bind as a shared-store **link**
> wherever a link is **safe** (no UAF) **and unobservable** (identical result), keeping copy as the
> semantics. Read the design doc first. This plan is instrument-first: **every oracle is proven
> report-only against a hand-computed matrix before it touches codegen**, and the codegen change is
> held to **byte-value-identity** on both backends. Nothing here changes what a program observes.

## The correctness bar (the whole build serves this)

Two invariants, both hard, both testable:

1. **Byte-VALUE-identity.** With the widening ON, every program produces the **identical observable
   result** (values + exit + output) it produced with copy-everywhere, on **both** backends. The
   emitted IR *changes* (a copy becomes a borrow); the runtime value must not. This is *not* Mode-B
   byte-identical IR — it is value-identity across a deliberate IR change. Gate-OFF stays
   byte-identical (the change is inert when disabled).
2. **#415-safety.** The widening never realizes an **unsafe** link (a borrow whose source store is
   freed / reassigned / escapes while the borrow is live) — the exact UAF that made field-read binds
   copy in the first place. A single unsafe link is a UAF regression, caught by the leak gates.

The design is **sound by conservatism** (`use_analysis`'s standing rule: *"can only lose an elision,
never produce a wrong borrow"*): if safety-and-unobservability is not *proven*, it materializes the
copy. So the failure surface is one-sided — a **missed** link (a real copy that was safe to share) is
acceptable; a **wrong** link is the only bug, and it shows up as a value divergence or a leak.

## What already exists — do NOT rebuild

The widening extends a shipped, validated analysis; most of the machinery is present:

- **The current elision** already links on **last-use** (source provably dead after the bind) —
  `use_analysis`'s verdict → `scopes::elide_borrows` (the `ElidePlan`) / `move_elide` (`MovePlan`).
- **The position-aware mutation fact** the widening needs is already computed: `mut_max_pos` — *"the
  source is mutated after the copy"* (`use_analysis.rs:119`, built from `mark_write` /
  `find_written_vars`). The widening reuses it and adds the symmetric *"the local is mutated after the
  bind"* query from the same access data.
- **The safety fact:** `Ownership::Borrowed { base }` (the source var backing a bound value) +
  `warn_dead_stores`' non-escape check + `reclaim_safe`.
- **The oracle test harness:** `tests/ownership_oracle.rs`, `tests/use_analysis.rs` — where the
  report-only oracle verdicts are pinned against hand-computed expectations.
- **The instruments:** `--report-copies` / `LOFT_MATERIALIZE_DUMP` (copy-count + which binds link),
  `LOFT_POISON` (arena poison-on-free), `LOFT_NATIVE_LEAK_CHECK`, `LOFT_STORES=warn` (the leak gates),
  and the `#415` baseline `tests/scripts/85-store-lifetime-field-read-copy.loft`.

So the build is **"extend `mut_max_pos`'s consumer + the elision verdict, behind a gate,"** not a new
subsystem.

## The validation methodology (the load-bearing half)

- **Instrument-first: validate each oracle BEFORE wiring it to codegen.** Build the safety and
  observability queries **report-only** (a dump / a test-only accessor), and pin them against a
  **hand-computed** boundary matrix in `tests/ownership_oracle.rs`. An oracle that is only checked
  *through* the codegen change cannot be debugged when a value diverges — you would not know if the
  bug is the oracle or the wiring. Prove the oracle in isolation first (engineering-rigor §
  calibration: the instrument is validated before it is trusted).
- **Capture-and-diff the answer.** The current copy-behavior **is** the answer to reproduce. Capture
  the full-suite result + leak-state GATE-OFF as the baseline; the GATE-ON run must reproduce it
  **exactly**. A value diff is the spec of a bug, read off the diff — not theorized.
- **The full suite GATE-ON is the soundness check.** A false *"unobservable"* verdict produces a
  different value → the suite reds on that program. A false *"safe"* verdict produces a UAF/leak →
  the leak gates red. So running the suite + leak gates gate-on, on both backends, **is** the
  validation that the oracles are sound — every divergence is an oracle bug. The targeted matrices
  (below) make the specific hazards non-vacuous.
- **Positive controls (no vacuous green).** Each matrix carries an **injected** cell the oracle MUST
  flag `unsafe` / `observable`; a green run that never exercised a fail cell proves nothing.
- **Both backends, always.** Interp and native are separate generators reading one plan; a link
  correct on one can UAF on the other. Every gate runs `--interpret` AND `--native`.
- **The gate is the safety valve.** `LOFT_LINK_WIDEN` (opt-in) lets the widening be **measured**
  gate-on across the whole corpus before it is ever default — and instantly disabled if a divergence
  appears. Default-on is earned only after a clean gate-on suite.

## The two boundary matrices (hand-computed; the falsification targets)

Build these as `/tmp` probes first (engineering-rigor), then graduate the surviving cells to
`tests/scripts/` + `tests/ownership_oracle.rs`. Every cell: **value AND length AND leak**, both
backends.

**Matrix S — safety (`link_is_safe`).** Each row is a `local = <source>` bind; the verdict must be
`unsafe` (⇒ copy) unless the source provably outlives the local:

| Shape | Must be | Why |
|---|---|---|
| `a = s.v; read a` (source lives) | **safe** | the field's store outlives `a` |
| `a = s.v; free/last-use s; read a` | **unsafe** | `#415` — a link dangles when `s` is freed |
| `a = s.v; s = other; read a` | **unsafe** | `base` reassigned — the old store may be reclaimed |
| `a = s.v; return a` (a escapes) | **unsafe** | `a` outlives the callee frame that owns `s` |
| `a = mkv()` (owned source) | n/a (no `base`) | an owned value has no source to link to — copy/own path unchanged |

**Matrix O — observability (`link_is_unobservable`).** Each row is a `local = <source>` bind that is
already `safe`; the verdict must be `observable` (⇒ copy) unless copy and link compute the same
result:

| Shape | Must be | Why |
|---|---|---|
| `a = s.v; read a; read s.v` (both read-only) | **unobservable** | no write to diverge them |
| `a = s.v; a[i] = x; read s.v` | **observable** | link writes through to `s.v`; copy does not |
| `a = s.v; s.v[i] = x; read a` | **observable** | link reflects the source write; copy does not |
| **`a = s.v; b = &s.v; b[i] = x; read a`** | **observable** | ⚠️ the source is mutated through **another alias** — the oracle must be alias-aware and treat any write that can reach `s.v`'s store as a mutation, else it links and silently reflects `b`'s write |
| `a = s.v; a[i] = x` (`a` discarded, dead store) | **observable** | this is the lint-to-`&` case — a copy (write lost); NOT linked (that link is observable) |

The starred row is the design-protocol **over-unification guard**: the tempting `link_is_unobservable`
= "the source var is not written after the bind" is *wrong* — it misses a write through a sibling
alias. The oracle must be conservative (over-report `observable`) whenever it cannot prove no write
reaches the source store. This cell is the falsification target that proves it.

## The build ladder (commit-by-commit; oracle-first, gated, each validated)

| # | Commit | What lands | Validation (the gate for THIS step) | E |
|---|---|---|---|---|
| 1 | **✅ DONE (2026-07-16).** Baseline capture + the two probe matrices. Matrix S (S2 source-dead, S4 escaping copy) + Matrix O (O1 read-only-both, O2 write-copy, O3 write-source, O4 sibling-`&`-alias trap) authored as `tests/scripts/link-widen-baseline.loft` (self-asserting, hand-computed copy-semantics values) + the driver `tests/alias_link_baseline.rs` (both backends · `LOFT_POISON` + `LOFT_NATIVE_LEAK_CHECK` clean · a `harness_can_fail` positive control proving a wrong-link value reds). No product change; `loft_suite` green. | ✅ every cell's hand-computed value matches copy-behaviour both backends; leak/poison-clean; the harness is proven non-vacuous | S |
| 2 | **✅ DONE (2026-07-16).** `link_is_safe` — report-only, matrixed. `use_analysis::link_safety_of` reuses the shipped copy-fill facts (extracted `collect_uses` so the shipped elision stays byte-identical — proven: `use_analysis`/`ownership_oracle`/`loft_suite` green): per copy-fill bind `a=s.v`, `safe` = single-def ∧ source outlives `a` (param, or a non-reassigned local whose last-use ≥ `a`'s) ∧ `a` non-escaping (`∉ ineligible`). Surfaced by `LOFT_DUMP_LINK_SAFE` (`link-safe-dbg:` lines), NO codegen. SOUND BY CONSERVATISM (favours unsafe; last-use under-approximates store lifetime). Escaping returns take the return-buffer path → not a candidate at all. | ✅ `tests/link_safe_oracle.rs` pins Matrix S both backends: S1 safe, S2 (dead) / S3 (reassigned) unsafe, S4 (escape) never-safe; non-vacuous (emits both verdicts); native≡interp | M |
| 3 | **✅ DONE (2026-07-16).** `link_is_unobservable` — report-only, matrixed, ALIAS-AWARE. `use_analysis::link_observability_of` (reuses `collect_uses`, shipped elision untouched): per copy-fill bind `a=s.v`, `unobs` = local `a` not mutated after the bind (`∉ ineligible`) ∧ source store stable after the fill — `other_max_pos[base] < fill` AND **every var aliasing the base** (`tp(b).depend().contains(base)`) is stable after the fill. The alias clause is what catches `a=s.v; b=&s.v; b[i]=x` (a write through the sibling reaches `s.v`'s store). Surfaced by `LOFT_DUMP_LINK_OBS`; no codegen. SOUND BY CONSERVATISM (a set-based over-count only misses a link). | ✅ `tests/link_obs_oracle.rs` pins Matrix O both backends: O1 unobs, O2 (local) / O3 (source) / O4 (sibling-`&`) observable; **O5 = the load-bearing proof** — the `&` precedes the bind so only the alias clause can flag it (isolated, verified observable); non-vacuous; native≡interp | M |
| 4 | **✅ DONE (2026-07-16).** Wired the widening into `ElidePlan` — GATED `LOFT_LINK_WIDEN` (default off). `analyze_fn`'s verdict gains a Borrow branch when `bind_link_safe && bind_link_unobservable` (the extracted per-bind predicates the oracles use, so codegen ≡ the validated oracle); the existing `ElidePlan` production + its borrower gate run unchanged. **The gate earned its keep:** the first gate-on run STOPPED at a codegen panic (`Incorrect var v[65535]` on a **self-append** `v += v` — `append_src[v]=[v]`, a link would borrow itself); fixed at the ORACLE (`bind_link_safe` excludes `src == v`), guarded by Matrix-S cell **S5**. | ✅ **gate-OFF:** dead branch ⇒ byte-identical (loft_suite/issues/wrap green, default). **gate-ON:** `a=s.v`'s copy elided to a borrow (introspect diff); **byte-value-identical + leak/UAF-clean** — issues (764) ✓ + under `LOFT_POISON` ✓, loft_suite ✓, native_scripts ✓, leak (49) ✓ + poison ✓, strings/frame_vars/expressions/wrap ✓, both backends. (Pre-existing `35c-rest-capture`-under-poison failure is not ours — fails gate-off+poison too.) | M |
| 5 | **⏸ MEASURED, DEFAULT-ON DEFERRED (2026-07-16).** The flip was implemented + the full suite proved gate-on byte-value-identical (2966 tests, only the 3 known flakes; slots/layout/poison clean) — but the **copy-count measurement is a stop signal**: the win is **~0 in real code**. The read-only-both field-bind pattern the widening targets is essentially absent (you read `s.v[i]` directly; an `a=s.v` bind is nearly always for mutation/escape → oracle-excluded; source-dead is already the move-elision) — **0 copies eliminated** across the vector-heavy scripts AND the ~2000-line `viewer/main.loft` (42 copy ops, 0 elided). Defaulting on adds per-bind analysis cost (an O(n) alias scan) for no benefit. @C86's revisit trigger — *a profiler shows bind-copies dominating a real consumer* — is **NOT met**, so it **stays opt-in** (`LOFT_LINK_WIDEN`), ready for when a consumer demonstrates the win. | ✅ gate-on byte-value-identical (steps 1–4 correctness proven both backends); ✅ win measured ≈ 0 → flip deferred per @C86 | S |
| 6 | **✅ DONE (2026-07-16).** Pointed the dead-store lint at `&`. `warn_dead_stores`' message on an `Owned` non-escaping `reads==0 & write_targets>0` local now steers straight to the fix — *"…the mutation lands in the copy, not the source. For write-through, bind a live reference with `&` (`x = &…`). If a copy was intended, read `x` after the mutation."* The write-through is always the programmer's explicit `&`, never inferred (copy stays the semantics). No behaviour change (still a copy). | ✅ the hex_terrain shape warns with the `&`-steer; VALUE unchanged (`t.tr_h[0]`==7, write still lost); `tests/dead_code_lint.rs` guards the steer content (`&` + read-back) + its existing alias/escape/read-back controls stay silent, both backends; `runtime_warnings` (45) green; doc (107 README) updated | S |

**Shape:** the risk is concentrated in the two oracles (steps 2–3), proven **in isolation** against
hand-computed matrices *before* any codegen change, so a step-4 value divergence points at a known
place. Steps 4–5 are the wiring + the byte-value-identity proof (the "everything the same" guarantee).
Step 6 is the ergonomic close. The safety oracle (step 2) is the keystone that guarantees `#415`
never returns; the alias-aware observability oracle (step 3, the ⚠️ cell) is the subtle one — over-
report `observable` whenever a write might reach the source.

## Stop conditions (revert, don't push through)

- A **gate-on value divergence** on any program → an oracle over-claimed `unobservable`. STOP; fix
  step 3; never "adjust the codegen to match". The value is the spec.
- A **gate-on leak / poison / UAF** → an oracle over-claimed `safe`. STOP; fix step 2; the `#415`
  class is back.
- **The widening keeps needing another special case** in the elision verdict → the fact belongs in
  the oracle (step 2/3), not in codegen (design-protocol / loft-codegen: facts in the analysis,
  translation in codegen).
- **One backend passes, the other diverges/leaks** → not landable; the plan is not a subset until
  both agree.

## See also

- [alias-where-correct.md](alias-where-correct.md) — the design + the crystallized principle (copy is
  the semantics; links are transparent-or-explicit).
- [DESIGN_DECISIONS.md § @C86](../../DESIGN_DECISIONS.md) — the contract this optimizes under (its
  "revisit when: widen `ElidePlan`").
- The loft-codegen skill (Mode B byte-identity + the both-backends rule) and the engineering-rigor
  skill (the boundary matrix, calibrate-the-instrument, the ratchet + positive controls).
- Code-points: `src/use_analysis.rs` (`Ownership::Borrowed`, `mut_max_pos`, `mark_write`,
  `find_written_vars`, `warn_dead_stores`, the elision verdict) · `src/scopes.rs`
  (`elide_borrows` / `move_elide`) · `tests/ownership_oracle.rs` / `tests/use_analysis.rs` (the oracle
  harness) · `tests/scripts/85-store-lifetime-field-read-copy.loft` (the `#415` baseline).
