<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 85 — Retire the store-lifetime bug class

Plan id: [@PLN85](https://github.com/loft-lang/plans/issues/85) · investigation-style
(reading order: Status → Probes → Cluster docs → Roadmap).

> # Status — CLOSED 2026-07-10 (store-lifetime bug class retired).
> Every arc of this investigation has shipped: the store-lifetime clusters, the
> `@PLN54` text-tail-return leak, the write+read / move-on-block-return residuals,
> and — finally — the CORPUS text-leaker second wave (below). The enforcing
> nightly leak gate is GREEN (full `tests/scripts` + `tests/docs` corpus + `lib/`
> `library_suite` Direct-leak=0 on both backends). The cluster docs + `probes/`
> below stay as the mechanism-understanding record. One out-of-gate-scope
> follow-up remains (`web/byte_at`, a NATIVE-EXTENSION interop leak, a different
> subsystem) — see [`NEXT-SESSION-corpus-leakers.md`](NEXT-SESSION-corpus-leakers.md).
> The finishing PR closes the tracker issue (`Closes @PLN85`).

> ## ✅ CORPUS text-leakers → 0 (2026-07-10) — the enforcing leak gate is GREEN.
> The second wave the nightly `asan-leak-gate` uncovered (9 `tests/scripts`
> leakers beyond the issues suite) is CLOSED. All `append_text` owned-text-return
> orphans on the interpreter; four principled fixes, each proven on a boundary
> matrix + ASan oracle and graduated to focused regressions in `tests/scripts/`
> (`85-text-optional-null-return`, `85-ncc-container-text-return`,
> `85-generic-forward-borrow-text-return`, `85-field-iter-text-payload-free` — plus
> the 9 real-world corpus leakers themselves), on BOTH backends:
> 1. **null-text sentinel early return** (`-> text?` with a work-buffer) — the
>    `return null` hoisted `OpConvTextFromNull()` into an owned skip_free temp; it
>    is a borrowed static, so return it directly like a text literal (`scopes.rs`
>    B2 free-safe rule). Fixes `85-text-optional-null-return`.
> 2. **`??`/ncc over a container** — the coalesce block wraps its selecting `if`,
>    which `if_tail_yields_text` didn't see through, so the ncc missed the per-arm
>    `__acc` buffer promotion and orphaned the owned copy (`control.rs`). Fixes
>    `500`, `155`, `157`, `199`, `repro_p356`.
> 3. **generic forward-borrow return** — a monomorph returning `x.method()` where
>    the method borrows `self` keeps the bare declared `text` (the arg-dep the
>    non-generic derives is lost) → owned copy orphaned. `promote_monomorph_text_return`
>    now also promotes a `ForwardArg` call tail AND non-tail early returns
>    (`control.rs`). Fixes `repro_p205`, `86-interfaces`.
> 4. **`#fields` text payload binding** — the unrolled field-iteration shared ONE
>    `_mv_<f>` payload binding across clones, freed only at the last (conditional)
>    use; each field-block now gets its own copy (`collections.rs`). Fixes
>    `45-field-iter`.
>
> Full `tests/scripts` + `tests/docs` corpus (436) and the `lib/` `library_suite`
> are Direct-leak=0; full suite 2738/2738; fmt/clippy clean. See
> [`NEXT-SESSION-corpus-leakers.md`](NEXT-SESSION-corpus-leakers.md) for the fix
> map + the one out-of-gate-scope follow-up (`web/byte_at` native-extension interop).

> ## 🔺 REOPENED then RE-CLOSED (2026-07-09) — two residual clusters fixed.
> `@PLN47` surfaced residuals the earlier close had missed. Both now FIXED:
> - **move-on-block-return** — `x = { z = P{…}; z }` borrowed instead of moving
>   (leak + slot-reuse corruption). Fixed unconditionally; suite 2721/2721.
>   **[block-return-move.md](block-return-move.md)**.
> - **write+read struct (`p9`)** — interp-only struct binary-I/O corruption+leak
>   (`f += s; f#read as S`): both write & read mis-targeted the eval stack
>   instead of the record. Fixed by the Vector-style deref in both paths,
>   validated by a `LOFT_POISON` + cross-mode oracle (13/13 probes, both
>   backends). **[writeread-slot-leak.md](writeread-slot-leak.md)**.
> The prior CLOSED record below stands as the earlier investigation history.

> ## 🔺 REOPENED (2026-07-09) — one residual: text tail-return leak (@PLN54 S4).
> The @PLN54 LeakSanitizer sweep on macOS-ARM surfaced a REAL, GROWING production
> leak in the store-lifetime class this plan owns:
> - **text-tail-return** — a native text-dest call in **implicit tail-return**
>   position (`fn f() -> text { …; u.to_json() }`) leaks ~2 allocations per call
>   and GROWS. `wrap_value_text_dest` wraps the tail native into a `work_text`
>   (`__work_1`), the tail delivery **copies** it into the return (`AppendText`)
>   instead of **moving** it, and the copied-then-dead `__work_1` is never freed
>   (a copy-vs-move / skip-free misclassification — the D-own-2 adopt-vs-copy
>   class). Rebinding to a local (`r = u.to_json(); return r;`) promotes/moves it
>   and is leak-free. **The `-> text?` (optional) variant is a USE-AFTER-FREE**
>   (the source is freed then read), not just a leak — a correctness/safety bug.
>   Boundary matrix + oracle harness (`probes/text-tail-return/run_matrix.sh`):
>   any native text-dest CALL delivered directly as a fn's return value
>   leaks (or UAFs when optional); rebind / interpolation / caller-bound are clean.
>   Fix site: the text tail-return delivery
>   (`text_return`/`classify_text_dep` in `control.rs`) needs the fresh-owned-text
>   move the **vector** side already has (`fresh_owned_vector_deps`). Proven
>   broken/working bytecode pair + repro:
>   **[text-tail-return-leak.md](text-tail-return-leak.md)**. NOT yet fixed —
>   the chokepoint edit is a focused both-backends pass (blocks @PLN54 S4's
>   `detect_leaks=1` flip).

> ## ✅✅ CLOSED (2026-07-04) — the store-lifetime bug class is retired.
> **Charter met:** outcome (b) — the clusters are independent, each invariant enforced at
> its chokepoint, with a standing instrument (fuzz + `LOFT_POISON` + DA-calibration +
> leak-gate + ASan/Miri) that keeps the class closed *by construction*.  **All live bugs
> fixed:** the 5 DA-inventory leak cells (t1–t4 + p188), the `skip_free` pass-poison class,
> and both D-own-2 return adopt-vs-copy facets (the struct copy-return leak + the
> JOIN-vector delivery, #492).  **The ownership model:** D-own-1's delivery/reassign
> re-derivation thicket collapsed behind pure `classify_X`/`dispatch_X` selectors with the
> `ownership_of` oracle default-on (0/54 over-free); D-own-2's return adopt-vs-copy class
> swept dry.  D-own-3/4/5 + the types.md deviations already CLOSED.
> **Forward-homed** (per the closure policy): the free-side/oracle three-valued-fact
> unification (latent + SAFE — measured 100% one-directional, no miscompile) and the
> runtime `??`-JOIN witness → **[@PLN90](https://github.com/loft-lang/plans/issues/90)**
> (copy-diagnostics); the DA-only codegen-slot class (`t_4File_lines` slot-width,
> `generate_set/call`, `wrap::dir` — not leaks, no shipped-build impact) → a `loft` issue.
> The detail below is retained as the investigation + close-out record.

> **▶ WIDE-RELEASE GATE 1 — the fuzz-proof:** [fuzz-proof-gate.md](fuzz-proof-gate.md).
> The "CLOSED" below is the *investigation* + per-cluster fixes — each known shape fixed and
> guarded. The remaining gate before loft goes to many people is proving the **class closed
> *by construction*** (a standing fuzz/sanitizer instrument over the ownership invariant, not
> anecdotal silence). **The instrument now EXISTS and is live (2026-07):** the standing
> `tests/ownership_fuzz_gate.rs` job, the in-process libfuzzer target
> (`cargo +nightly fuzz run program_ownership`, H5 + poison fully live — it caught F1+F2 in
> its first five minutes; both FIXED), `LOFT_POISON=1 cargo test` green (the @PLN54-S3
> criterion), and the debug-assertions calibration run
> ([DEBUG.md](../../DEBUG.md#the-debug-assertions-calibration-run-target-da)).  What remains
> in the gate doc: the open DA-inventory cells (§ final honest DA map) and the unfuzzed
> axes list — see [STABILITY_ROADMAP.md § the wide-release bar](../../STABILITY_ROADMAP.md).

> **▶▶ DA-CALIBRATION LEAK CELLS — ALL CLOSED (2026-07-04).**
> [NEXT-SESSION-da-leak-cells.md](NEXT-SESSION-da-leak-cells.md) — the five DA-inventory
> leak cells (t1 closure / t2 `&`-default null-init / t3/p179 skip_free poison / t4/pln87
> displaced-record writeback / p188 discarded keyed-collection return) are each FIXED on
> BOTH backends, verified under the debug-assertions gate, with a graduated
> `tests/scripts/85-*` guard.  The `skip_free` pass-poison class residual is RESOLVED
> (per-site `!first_pass` gates for the two H5-counter-shift instances; the pass-boundary
> bit-clear DECLINED — variable tables persist across the stdlib↔user parse boundary).
> Dev suite 2603/2603, `LOFT_POISON=1` gate green.  The **store-lifetime LEAK class is
> retired**; the residual DA cells (stdlib slot-width, `generate_set/call`, `wrap::dir`)
> are a SEPARATE codegen-slot-correctness class, DA-only, NOT this plan's charter.
> Instrument = `LOFT_SKIPFREE_TRACE=<var>` (now prints the `#[track_caller]` location).
> (The 2026-06-29 join-ownership handoff below stays valid for the D-own-1/2
> deps-carried-join completion — the F2 residual.)
>
> Older handoff (2026-06-29):
> [NEXT-SESSION-join-ownership-analysis.md](NEXT-SESSION-join-ownership-analysis.md). The over-free
> class is now FULLY PROBED and the fix is scoped: it needs ONE ownership fact (Owned / Borrowed /
> **Join** = the `v[i] ?? d` runtime owned-or-borrow), which IS @PLN25's copy-vs-borrow analysis
> (`src/use_analysis.rs`) — extend it, don't duplicate. Build it **inert → test separately → use
> optionally** (the @PLN25 pattern). That handoff has the 3 chokepoints, the corrected root-cause,
> the acceptance repros (`bytecode-comparisons/462-*`), the fuzz/poison tools, and the staged plan.
> (The older 2026-06-26 leak handoff — `NEXT-SESSION-leak-fix.md` — is superseded: that adopt-re-return
> leak is now FIXED; the live class is the borrowed-view over-free mapped here.)

> **▶ ACTIVE D-own slice (2026-06-25): the adopt/free re-derivation collapse — driven by
> [#457](https://github.com/loft-lang/loft/issues/457).** A `vector<text>` corrupted (wrong len +
> SIGSEGV) because a returned local adopted across `if/else` arms is freed before `return`.
> **CLOSED** — fixed in the **return delivery**, not the free side.  The free-side patch
> (`47b30a53`) was abandoned; the real fix is an **aliasing-safe vector delivery** (`OpReplaceVector`,
> a no-op when source aliases dest) plus delivering the implicit-tail adopt into the buffer, so the
> fn ALWAYS returns its buffer and the dep is accurate.  `src/scopes.rs` reverts to **origin/main** —
> the whole free-side thicket (pairing + explicit free + `IfDistinct`) is **deleted**.  Fixes the
> consistency residual (the `clear+append` self-copy that emptied an aliasing `return out`) and the
> #306 noise by construction; loft suite 2538/2538, ZT directory/fedops/membership/records green with
> zero #306.  Full resolution in §8 of **[adopt-free-collapse.md](adopt-free-collapse.md)** (the
> adopt/free sibling of the [D-own-1 delivery collapse](D-own-1-return-delivery-collapse.md)).
> The *simplification* (D-own) continues per [OWNERSHIP_MODEL.md § ACTIVE](../../OWNERSHIP_MODEL.md#active--the-simplification-exploration-next-days-exploratory--revertable).

> ## ✅ CLOSED — outcome (b): the clusters are independent, each invariant named + enforced at its own chokepoint, with a standing instrument that keeps the class shut.
>
> The investigation's central single-root hypothesis was **falsified** (Stage B): the
> store-lifetime bugs are *independent mechanisms*, not one shared slot-init root. Each is now
> fixed at its own chokepoint and guarded:
>
> | Cluster | Invariant (the one fact) | Chokepoint | Guard |
> |---|---|---|---|
> | **II** NRVO return-buffer double-free | a returned local's buffer is delivered into `__retbuf` once, freed once | `scopes.rs` `collect_return_sources` + `control.rs` `materialize_vector_arms_into` | `85-store-lifetime-vector-match-return.loft` + 4 more |
> | **III** enum-discriminant corruption | `copy_claims` reads an initialised discriminant from every producer | #412 (enum-field producer) | probes 02/03 |
> | **V** NRVO adopt/append ownership-dep (the #437 regression, ex-plan-90) | **a vector local's `dep` = the store it owns** | `control.rs` ref_return adopt-promotion + `vectors.rs`/`operators.rs` concat-adopt | `437-nrvo-return-aliasing.loft` |
> | **I** FFI foreign-store vector return | a local that adopts a callee store frees it once | producer chokepoints (#409/#410) | — (vector instances fixed; struct-return latent, forward-homed) |
>
> **Standing instrument (keeps the class shut):** the `wrap` program-exit **leak-gate** over the
> whole `tests/scripts/` corpus + `leak_cases` / `leak_cross_mode` + **ASan** (UAF/OOB) + **Miri**
> (hard-UB) in CI, plus **20 graduated `85-*` regressions** and the `437` guard. A new sibling that
> leaks/UAFs in any covered shape fails CI by construction (cluster V was exactly such a
> re-discovered sibling — now closed, and its shapes are in the corpus).
>
> **Residuals forward-homed** (per the closure policy → [QUALITY-history.md § Store-lifetime cluster](../../QUALITY-history.md#store-lifetime-cluster)): cluster I's **latent** FFI struct-return read gap (unreachable until an `alloc_struct` helper) and cluster IV's **@PLN51 latent edge leaks** (re-probed at closure — native-mostly leaks on tuple/lambda/operator/capture-heap shapes, **pre-existing, identical on `origin/main`**, below the graduated-corpus gate).
>
> The original detail below is retained as the investigation record.

## Status

| Stage | Status |
|---|---|
| A — Probe catalogue (extract from the real recent bugs + a real consumer) | ✅ probes 01–04 (evaluation in [recent-bugs.md](recent-bugs.md)); of 3 predicted siblings: #1 latent, #2 closed, #3 LIVE |
| B — Mechanism investigation (shared-root vs N-independent) | ✅ **CLOSED via debugger** — real mechanism: `ki` is a borrowed alias of the NRVO return buffer `__ref_1`; under conditional×unused, scope analysis emits a per-iteration `OpFreeRef(ki)` that double-frees the return buffer via a stack-ref ([cluster-II](cluster-II-slot-init-dominance.md) § CLOSED OUT). NOT the slot-init story. |
| C — Fix design | ✅ **DESIGNED** — the structural fix is a **type-system change**: make heap-return ownership a computed type fact (the return type's `Deps`: owned/transferred vs borrows-attr), so codegen (caller adopt-vs-copy; callee free-or-not) collapses to mechanical reads — per [CODEGEN_METHOD.md](../../CODEGEN_METHOD.md), the `has_ref_params`/`returned_var` complexity is the symptom of this missing fact. Type design: **[type-ownership-design.md](type-ownership-design.md)**; target bytecode + rungs: [stage-c-move-convention-design.md](stage-c-move-convention-design.md). |
| D — Implementation | ✅ **CLUSTER II SHIPPED** — the cluster-II owned/borrow vector-`match`-return is correct AND leak-free on BOTH backends; probe 05 graduated to `tests/scripts/85-store-lifetime-vector-match-return.loft`; full suite clean. Two landed changes: (1) **scopes.rs** `collect_return_sources` + narrowed skip_free (the SET that fixes `returned_var`'s match-collapse); (2) **control.rs** `materialize_vector_arms_into` — per-arm NRVO delivery into `__retbuf` (each arm copies its elements into the caller's buffer, frees the dead local backing, yields `__retbuf`), closing the interp work-ref leak (root-caused to `init_ref`'s eager `null()` store) while staying native-compilable. Plus a real native-generator bracing-bug fix (emit.rs). Full mechanism: [type-ownership-design.md § 6h](type-ownership-design.md). **Also fixed + guarded:** #405 (probe 04, `gen_if` stack-delta gate) and the `return v` borrow-return aliasing (param→buffer copy). **→ the cluster-II CRASH/CORRUPTION class is retired on both backends** (5 graduated regressions in `tests/scripts/85-*`). **The two ownership-COMPLETENESS residuals are now ALSO fixed + guarded:** **loft#415** field-read adopt-vs-copy (`a = x.v` and `a = getv(b){ b.v }` now COPY — bind-site + implicit-tail struct-field copy; guard `85-store-lifetime-field-read-copy.loft`; OWNERSHIP_MODEL row 103 CLOSED) and **loft#416** implicit-tail `{ match }` interp leak (the materialize gate now fires for `t = Vector` branch tails, with a nullable-arm guard so a reachable `null` arm is left alone; guard `85-store-lifetime-implicit-match-return.loft`). **All known cluster-II shapes — explicit & implicit match-return, #405 conditional-loop, param-return & field-read aliasing — are correct + leak-free on both backends, each with a graduated regression. Plan ready to close (outcome b).** |

**What triggered this:** the *known* store-lifetime mechanisms are already hardened and
shipped — H1 (fn-arity freeze), **H2 (typed `Deps` newtype + value-tagged spaces — see
[DEPS_INVENTORY.md](../../DEPS_INVENTORY.md))**, H3 (ownership carried, not re-derived), and the
finished @PLN51 hidden-buffer-aliasing investigation. **Yet the class still produced four
sev:high bugs this cycle** (#405/#406/#409/#410), in mechanisms the prior work did not cover.
So the open question is not "apply the known fix" (done) but: *is there a deeper shared root the
hardening missed, or are these genuinely independent mechanisms?*

## Goal

Ship one of two provable outcomes: **(a)** a structural change that retires the class (a single
chokepoint/invariant the residual clusters all violate), **or (b)** the verified finding that the
clusters are independent, with each one's irreducible invariant named + enforced at its own
chokepoint AND a standing instrument (corpus / sanitizer / fuzz) that keeps the class closed so
the next cycle stops re-discovering siblings. Either way: a probe suite that proves the class
closed on both backends.

## Central hypothesis (HYPOTHESIZED — Stage B must verify or kill)

Refined by the recent-bug evaluation ([recent-bugs.md](recent-bugs.md)) from "value-vs-dep
desync" to the broader **slot-initialisation-before-lifetime-op** invariant:

> **Every slot a lifetime operation (free / `copy_claims` / in-place vector rebuild) reads must
> be initialised — a real store/discriminant or a recognisable sentinel — by EVERY construction
> path that produces the record.**

All recent bugs fit as *different producers violating this one invariant*: uninitialised dep
slot (#405), empty dep buffer vs a foreign value (#409/#410), uninitialised enum discriminant
(#406), dangling cross-iteration slot (@PLN51-II — and **#405 is an uncovered sibling of that
already-"closed" cluster**). Key structural fact the evaluation verified: **corruption manifests
concentrated** in `src/database/allocation.rs` (`free_named`, `copy_claims`), while **roots
scatter** across producer codegen paths — so per-producer fixes (control.rs, expressions.rs) and
defensive-at-manifestation patches (#405's OOB-refuse) each leave the class open.

**Stage-B update:** this slot-init framing was **FALSIFIED for the live cluster II** — its real
mechanism is an **NRVO return-buffer double-free** (`ki` borrows `__ref_1`; debugger-verified), not
a producer slot-init gap. The clusters are turning out *independent*, not one shared root:
#1 latent FFI gap, #2 copy_claims (closed by #412), #3=II NRVO-alias ownership. So the plan is
trending toward outcome (b) — per-mechanism invariants — over a single chokepoint.

## In-plan vs spinoff policy (default: in-plan)

Findings during this investigation are fixed in-plan and recorded in the cluster catalogue, not
double-filed (the probes + cluster docs are the record). On closure the rule inverts: any still-open
residual gets a forward home (PROBLEMS.md / QUALITY.md `## Open work`) citing its cluster doc.
Full policy: [`_INVESTIGATION_TEMPLATE.md`](../_INVESTIGATION_TEMPLATE.md).

## Cluster catalogue (seeded — mechanisms verified in [recent-bugs.md](recent-bugs.md); to be confirmed/split by probes)

| ID | Cluster | Severity | Backends | Status of the instance | Doc |
|---|---|---|---|---|---|
| I | FFI foreign-store `vector` return delivered to a local; in-place `+=` drops it (local borrows foreign store, dep buffer empty) | high | both | #409 (wrapper) + #410 (direct) FIXED at producer chokepoints — mechanism cluster open | cluster-I (todo) |
| II | **NRVO return-buffer double-free** (was mis-described as a dep-slot init bug): `ki` aliases the NRVO buffer `__ref_1`; under conditional × **unused**, scope analysis emits a per-iteration `OpFreeRef(ki)` that whole-store-frees the stack-alias (#405/#306) while `__ref_1` also frees it at fn exit | high | **interp SIGSEGV / native completes** | 🔴 **LIVE on main** (probe 04); mechanism VERIFIED via debugger. THE live cluster. | [cluster-II](cluster-II-slot-init-dominance.md) § CLOSED OUT |
| III | enum-discriminant corruption: struct-with-enum-fields-from-a-variable appended to a vector; `copy_claims` reads a -1 discriminant | high | both | ✅ **CLOSED on main** (probes 02/03): #412 fixed the enum shape; `copy_claims` breadth (vector/sub-struct fields, nested) verified clean | — |
| IV | @PLN51 hidden-buffer-aliasing residuals (siblings not re-probed since closure) | tbd | both | re-extract from `finished/51` probes | cluster-IV (todo) |
| V | **NRVO adopt/append ownership-dep** (the #437 regression, was the standalone plan-90): a vector local's `dep` diverges from the store it actually holds — `buf += call()` (append-source orphaned), `buf = head()` via a `match` wrapper (mixed-arm NRVO), `a = call() + …` (concat backing orphaned) → wrong free → leak on escape / corruption | high | both | ✅ **FIXED + REDUCED** — one invariant (*dep = owned store*) at 3 sites; the 4-site dep thicket reduced 4→3 (I-c witness-pairing DELETED as subsumed; the `+=` backing-preserve **retained — load-bearing on native**, an interp-only subsumption check had wrongly dropped it). Full matrix CLEAN both backends (values + leaks); suites green. Guard `tests/scripts/437-nrvo-return-aliasing.loft` | [cluster-V](cluster-V-nrvo-adopt-ownership.md) |
| 462 | **stale-DbRef-after-slot-reuse UAF**: `vector<__nullable<S>>` += `[fn()->struct]` in a large accumulating fn; a prematurely-freed struct store's slot is reused, a live stale DbRef (on the operand stack / in an element) corrupts the new occupant → `copy_record` reads a freed store | high | **interp SIGSEGV** (crawler `questtest`) | 🔴 **LIVE on main** — regression from the #457/#459 D-own delivery rework; `LOFT_NO_SLOT_REUSE=1` proves slot-reuse; does NOT shrink (needs ~190-store interleave). Coexisting massive leak. | [cluster-462](cluster-462-slot-reuse-uaf.md) |

> **▶ REOPENED (2026-06-25): the crawler dogfood wave.** The outcome-(b) closure held for the
> probed shapes, but the crawler consumer surfaced a fresh wave at real-consumer scale —
> [#462](https://github.com/loft-lang/loft/issues/462) (cluster-462 above) is the live store-lifetime
> SIGSEGV. The standing instruments **missed it** because the survivor reference is not a frame
> variable (so `LOFT_UAF` is blind) and the minimal shapes don't trip the CI leak-gate. See
> cluster-462 § "Why the standing instruments missed it" + § "Tool gaps". The **sibling sweep**
> (probes `46x-*`) hunts the same class at the other recently-fixed sites.
>
> **▶ FIELD MAP (~95-shape sweep): [nullable-materialization-field-map.md](nullable-materialization-field-map.md).**
> Beyond #462 the sweep found **two crisp, minimal bug families** in the @PLN25 nullable layer —
> **A** (`?? <vector-literal>` default not materialised) and **N** (a vector-literal element that
> isn't a struct-literal is over-promoted to `__nullable`) — plus the safe region (all delivery +
> append shapes clean). The doc documents each cell's complexity and names the unifying pattern: a
> **materialisation hole for freshly-constructed vector/struct values at the nullable boundary**
> (one predicted fix locus, not three). Curated probes: `46A-*`, `46N-*`, `sib-nullcoalesce-*`.

## Probe suite

`probes/` (to be created in Stage A). Per the investigation method: **write probes before reading
source**, liberally; extract at least one from a REAL consumer (crypto/imaging FFI returns, moros),
not only the synthetic repros; run every probe on `--interpret` AND `--native` and record the
matrix. A probe graduates to `tests/scripts/85-*.loft` only when it passes assertions + clean exit +
no leak (`LOFT_STORES=warn`) + bounded runtime.

| File | Shape | Cluster | Status |
|---|---|---|---|
| `01-native-struct-return.loft` | direct `#native` struct (non-vector heap) return + read/mutate | I (sibling) | read-only RED but **NOT a confirmed sibling** — isolated to an FFI-layout gap (no `alloc_struct` helper; `alloc_record` doesn't lay out a loft-readable struct ref). Sibling #1 is **latent/unreachable**, gated on a future struct-return helper — see [recent-bugs.md](recent-bugs.md) |
| `02-enum-fields-in-vector.loft` | struct w/ 1 & 2 variable-sourced enum fields, appended to a vector | III (#406) | ✅ **PASS on current main** — #412 fully closed the #406 shape (the "still corrupt" in its thread was the pre-merge WIP). Control (direct read) proves non-vacuous. |
| `03-copy-claims-breadth.loft` | struct w/ vector field, sub-struct field; nested `vector<vector>` appended | II/III breadth | ✅ **PASS** — `copy_claims` deep-copies these field kinds correctly; sibling #2 (copy_claims family) closed for reachable shapes |
| `04-slot-init-405.loft` | heap local conditional × unused × nested loop (#405 shape) | II (#405) | ✅ **FIXED — BOTH backends, graduated** to `tests/scripts/85-store-lifetime-405-conditional-loop.loft`. ROOT (different from the earlier caller-side-NRVO theory — that was WRONG): `gen_if`'s `null_else_value` gate keyed on the true-branch RETURN TYPE (`tp != Void`), but a block ending in `OpAppendVector` reports a non-Void type while pushing NOTHING — so the else path emitted a spurious `ConstInt(i64::MIN)`, leaking 8 stack bytes per false-branch iteration → stale `DbRef` → SIGSEGV at `OpFreeRef`. Fix (`src/state/codegen.rs`): gate on the actual stack delta `true_stack != stack_pos`. Found by a loft-codegen-**skilled** eval agent that proved working-vs-broken bytecode on both backends first (the method this plan exists to enforce); a no-skill baseline on the same bug did not fix it. |
| `05-enum-arg-vector-return-aliasing.loft` | **REAL-CONSUMER (cbor):** `fn(enum) -> vector` called N times, results held live | II | ✅ **FIXED — BOTH backends, graduated.** Correct independent values + no leak on `--interpret` and `--native`; graduated to `tests/scripts/85-store-lifetime-vector-match-return.loft`. Root cause of the cbor map-encode corruption. Fix: per-arm NRVO delivery into `__retbuf` ([type-ownership-design.md § 6h](type-ownership-design.md)). |
| `06-field-read-adopt-vs-copy.loft` | binding a vector FIELD: `a = x.v` (no fn) and `a = getv(x)` (fn returns `b.v`) | adopt-vs-copy ([OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) holes 102/103) | 🟠 **FILED → [loft#415](https://github.com/loft-lang/loft/issues/415)** (sev:medium, `wa:clean` — explicit `af=[]; af+=x.v` copies, verified both backends). LIVE on main: `af`/`ag` len == 4 (alias) vs control `a = x` (len 3). Silent value-semantics violation (no crash/leak); the bare `a = x.v` aliases with NO function — distinct from the CLOSED `return <param>` hole (row 101). A multi-cycle ownership-completeness rung (the adopt-vs-copy fact extended to field reads), not a cluster-II crash. Not graduated (LIVE until the fact is completed). |

## Roadmap (next session)

1. **Stage A** — port the four repros (#405/#406/#409/#410) into `probes/` as assertion-bearing
   `.loft` files; add a real-consumer FFI-return extraction; run both backends; record the matrix.
2. Pull the @PLN51 probe set forward and re-run for live siblings (cluster IV).
3. **Stage B** — for each cluster, a `cluster-<id>.md` with a verified-vs-hypothesised table; test
   the central hypothesis (shared desync root) by checking whether one chokepoint invariant covers
   I/II and whether III is the same or independent. Instrument (one env-flag trace) rather than
   theorise when a mechanism resists two reads.
4. **Stage C/D** — either the chokepoint fix (if shared) or per-mechanism invariants + a standing
   instrument; graduate probes per cluster as each fix lands.

## Anchors

- Design: [DEPS_INVENTORY.md](../../DEPS_INVENTORY.md) (typed `Deps`, done) · [LIFETIME.md](../../LIFETIME.md) (dep model) · [STABILITY_HOTSPOTS.md](../../STABILITY_HOTSPOTS.md) (H-register) · [STABILITY_METHOD.md](../../STABILITY_METHOD.md)
- Evidence: loft#405, #406, #409, #410 · finished @PLN51 (`plans/finished/51-hidden-buffer-aliasing/`)
- Method: `CLAUDE.md` § matrix-first · the `design-protocol` skill (one invariant, or N?)
