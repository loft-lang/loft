# 57 — Vector store-lifetime watermark (@P393)

> ✅ **CLOSED 2026-06** ([`@PLN2`](https://github.com/loft-lang/plans/issues/2)).
> Thesis delivered: @P393's watermark warnings gone (cluster II), the I/III
> straight-line watermark fixed (last-use freeing is the default), and the
> rc-removal tail-end complete (Phases A/B/C — `ref_count`/`inc_rc`/`dec_rc`/
> `OpIncRc` deleted; `Store.pinned` for const/global).  Tracked elsewhere:
> Cluster III Route 2 → [QUALITY-history.md § Open work](../../QUALITY-history.md); sibling crash
> → [#258](https://github.com/loft-lang/loft/issues/258) (capturing closure into a
> collection); `vv[0]+=[x]` codegen → [#246](https://github.com/loft-lang/loft/issues/246);
> parallel-capture feature → `lib_plans` 08/10; nightly parity sweep → TESTING.md.
> B.1 (tuple+vector return) was already fixed (regression `tests/scripts/169`).
> Per the new tracking model the dir stays put; state lives on the board.

## Status (REQUIRED)

| Stage | Status |
|---|---|
| A — Probe catalogue | ✅ complete (15 baseline + 38 cluster-I edge probes, both backends) |
| B — Mechanism investigation | 🟢 cluster II FIXED; I + III root-caused + **unified** (same fix surface = free a store at its data's last use) |
| C — Fix design | ✅ **written** — [`fix-design-store-lifetime.md`](fix-design-store-lifetime.md) (clusters I + III): decouple heap-store lifetime from slot scope.  **rc crux DISSOLVED** — no vector store holds a ref past rc=1 (`dec_rc=0` every shape; `probes/cluster-I/00_rc_trace`), so the fix is simply "emit the free at the confined last use", no rc surgery.  Confinement analysis **adversarially hardened** (3 probe rounds; sound vs return/yield/break, block-result, tuple-element, dep-aliasing, borrow chains; loop-internal excluded). |
| D — Implementation | 🟢 cluster II shipped (`ff8b0730`); **cluster I-a (block-confined) SHIPPED + verified (`3ec7d94c`)** — `relocate_null_init` block-scopes the confined `__vdb`'s slot (the crux was the **body-0 null-init hoist**, not rc/timing).  **Cluster I-b + III straight-line FIXED — last-use freeing is the DEFAULT (Phase 5, 2026-06).**  See [`fix-design-last-use-freeing.md`](fix-design-last-use-freeing.md): Phase 1 diagnostic (`store_liveness_walk`) → Phase 2 spike (refuted free-alone: the watermark was **locked at body-0** by the `__vdb` null-init hoist) → **real lever = relocate the null-init AND early-free** (`lastuse_reclaim`) → Phase 3 soundness gates (`reclaim_safe`/`contains_alloc_unconditional`/`holder_retained` — dominance + escape/alias/capture/retention) → Phase 4 Goal-E enforcement assert (`reclaim_unfreed_eligible`) → Phase 2.5 tag/verify gate (interpreter **and** `--native`, scoped to reclaim-eligible stores).  **Measured (both backends):** reassign 11× 13→4, distinct-locals 37→3, comprehension 12→3, all outputs identical.  Watermark regression guard in `tests/watermark.rs`; `Stores::peak` field; **full suite 1923✅ with reclaim DEFAULT + the Goal-E assert active** (`LASTUSE_RECLAIM_OFF` restores the un-reclaimed baseline for A/B).  **Open:** the **shared-variable-across-blocks residual = cluster III Route 2** (shared `z` across `else`-blocks / `x` across `match` arms — a multi-store local whose single-valued dep drops the overwritten store's owner; `confine_reassign_safe` + `multi_store` are the inert foundation). |

> **Reframed (not benign) — this is [GOALS.md Goal E](../../GOALS.md#goal-e--predictable-memory-the-programmers-model-is-the-truth).** A
> block-scoped vector living to function exit means a program holds **more heap
> than the source implies** — an unpredictable-memory liability, not a watermark.
> The fix design + the `LOFT_STORE_GUARD` detector live in
> [`fix-design-store-lifetime.md`](fix-design-store-lifetime.md); the probe
> landmarks (incl. 2 sibling crash bugs) in
> [`probes/cluster-I/`](probes/cluster-I) + [`probes/bugs/`](probes/bugs).

**Cluster II fix (`ff8b0730`) resolved @P393's user-visible symptom.** The 2× literal-init
double-allocation was the dominant watermark contributor; removing it dropped
`11-vectors`'s watermark from **44 → 26**, below the hardcoded `LOFT_STORES=warn` floor of
30 — so the ~14 "possible leak" warnings that prompted @P393 are **gone**. Clusters I
(scope-end pinning) and III (reassignment leak) still make the watermark O(locals) but no
longer cross the threshold; both are benign (exit-safe) and left as a design call.

**What triggered this.** Filed as [@P393](../../PROBLEMS.md) on 2026-06-01 while
verifying the @P390 self-slice fix: `LOFT_STORES=warn loft --tests tests/scripts/11-vectors.loft`
emits ~14 `possible leak at alloc #N` warnings as the active-store count climbs to
~40 (max=40), yet the test **passes the exit leak gate** (`tests/wrap.rs:276`). So
this is a *during-run high-watermark*, not an exit leak. The investigation's job:
characterise whether the climb is a real per-operation delayed-free or benign
function-scope accumulation, and across which vector shapes.

**Stage-A verdict (verified by `LOFT_STORES=log` traces — see [RESULTS.md](RESULTS.md)):**
the `possible leak` warnings are a **false positive for an actual leak** — every store
frees at function-scope exit (11-vectors trace is `aaaa…(44 allocs)…ffff…(42 frees)`,
**zero interleaving**; exit gate passes). The high watermark is real and has three
verified contributing mechanisms (clusters I + II + III below). It is a **memory-watermark
/ heuristic-false-positive** concern, **not** a correctness/leak concern.

## Goal (REQUIRED)

Reduce the function-local vector-store watermark by fixing the three clusters in-plan:
(I) free dead store-backed locals at last-use instead of scope-end, (II) stop the
literal/comprehension init-temp from pinning, (III) free the overwritten store on
reassignment — plus a heuristic fix so `LOFT_STORES=warn` stops flagging legitimately-long
functions. The "do nothing, it's benign" outcome is on the table for clusters I/III (they
share the aliasing-guard surface) and must be argued explicitly, not assumed; cluster II
is a clear redundancy worth removing regardless.

## In-plan vs spinoff policy (default: in-plan)

All three clusters stay **in-plan** and are fixed here — this plan exists to fix @P393,
not merely characterise it. Findings discovered during the investigation are **not** filed
as separate P-issues; the cumulative probe suite is the regression guard.

**Two orthogonal codegen bugs surfaced during the edge probing and are FILED separately**
(a different subsystem — slot / stack-position assignment, not store lifetime). Both turned
out scope-pinned + root-caused, so per the spin-out rule they need **no plan** — they are
tracked as standalone P-issues and fixed directly:

- **[@P394](../../PROBLEMS.md)** — `b = a` (new local ← bare vector var) → LHS on slot
  `u16::MAX` → `codegen.rs:2669` panic / hang / silent-empty. Root: the `uses>0` guard at
  `expressions.rs:1333`.
- **[@P395](../../PROBLEMS.md)** — `(v+[x]).len()` (concat-temp method receiver) in an
  assignment RHS → 8-byte stack drift → silent garbage / panic. Root: incomplete concat-temp
  guard at `vectors.rs:39`.

Full edge matrices in [RESULTS.md](RESULTS.md) § Orthogonal codegen bugs. The aliasing
answer that fell out of the @P394 probing (vectors copy on expression-assignment) resolved
cluster I/III fix-safety.

## Cluster catalogue (REQUIRED)

| ID | Cluster | Severity (corruption / leak) | Backend asymmetry | Probes | Doc |
|---|---|---|---|---|---|
| I | Store-backed locals pin to **function exit**, not last-use. Only LOOP bodies reuse their store in-place; `if`/non-loop-block locals pin like top-level statements (probe 15) | none / none (exit-safe) — watermark O(distinct bindings) | both (interp 44, native 42) | 02, 07, 09, 15, 11-vectors | [cluster-I-scope-end-batching.md](cluster-I-scope-end-batching.md) |
| II | ✅ **FIXED** (`ff8b0730`) — `local = [literal]` / comprehension / struct-vector init double-allocated a scope-pinned store (2×/local): the literal body allocated v's store, then `create_vector`'s `=` `vector_db` allocated a second, orphaned one. Fix: `create_vector` skips its `vector_db` when the body already allocated (head `Set(v, OpGetField)`). Now 1×, like concat/slice. **11-vectors watermark 44 → 26 (below the warn floor → @P393's warnings gone).** | none / none (exit-safe) — was watermark ×2 | both | 07, 09, 10, 11, 12, 13 | [cluster-II-literal-init-double-alloc.md](cluster-II-literal-init-double-alloc.md) |
| III | Reassigning `v = [new]` does **not** free the previous store; every overwrite pins the now-unreachable old value to scope exit (probe 14) | none / none (exit-safe) — watermark O(reassignments) | both | 14 | [cluster-III-reassignment-pin.md](cluster-III-reassignment-pin.md) |

No cluster is a leak (every store frees at scope exit). Transient *unbound* temps
(slice-in-format, probe 08) free/reuse correctly at statement-end — **not** part of the
problem. Clusters I and III share a root cause (store-free anchored only at scope exit,
never at last-use or overwrite) and a fix surface (dead-store freeing with the PLAN51/52
aliasing guard); cluster II is an independent init-codegen redundancy.

## Probe suite (REQUIRED)

| File | Shape | Cluster | Status |
|---|---|---|---|
| `01-baseline-single-vector.loft` | one small vector, build+read | reference | passes — baseline (watermark ~2) |
| `02-loop-local-integer.loft` | fresh `vector<integer>` local per loop iter | I (contrast) | passes — store **reused in-place**, watermark flat (4 allocs) |
| `03-loop-local-text.loft` | per-iter `vector<text>` | I (contrast) | passes — flat |
| `04-loop-nested-vector.loft` | per-iter `vector<vector<integer>>` | I (contrast) | passes — flat |
| `05-multi-fn-returns.loft` | helper builds+returns vector value ×20 | I (contrast) | passes — per-call store freed at return |
| `06-slice-ops-loop.loft` | per-iter slice into a new local | I (contrast) | passes — flat |
| `07-sequential-named-locals.loft` | 35 **typed** named vector locals | I + II | **watermark 72** (≈2× per local), all freed at scope-end |
| `08-sequential-transient-temps.loft` | 35 unbound slice-in-format temps | — (control) | passes — watermark **4**; temps free/reuse at statement-end |
| `09-untyped-named-locals.loft` | 35 **untyped** named vector locals | II | **watermark 72** — identical to 07; annotation is *not* the cause |
| `10-struct-vectors.loft` | 10 `vector<Item>` (struct) locals | II | **2×/local** (22 allocs) — elements inline, no per-element store |
| `11-comprehension-init.loft` | 10 `c = [for…]` locals | II | **2×/local** (22) — comprehension init also doubles |
| `12-concat-init.loft` | 10 `c = a + b` locals | II (reference) | **1×/local** (26 at N=20) — concat result *becomes* the local; no double |
| `13-slice-init.loft` | 10 `t = base[a..b]` locals | II (reference) | **1×/local** (14) — slice materialises into the local; no double |
| `14-reassignment.loft` | 1 local reassigned 10× | III | **~1×/assign, all pinned** (14, `aaa…fff`) — old store not freed on overwrite |
| `15-if-block-locals.loft` | 10 locals in separate `if` blocks | I | **2×/local, all pinned to fn exit** (22) — non-loop blocks do *not* free at block-end |
| `16-generality-struct-text.loft` | 10 struct + 10 text locals | I (scope) | **12 allocs** — scalars/structs/text are *inline*; cluster I is collection-specific |
| `17-assignment-aliasing.loft` | `b = a` then mutate | (fix-safety) | tripped **@P394**; via `b = a[..]`: independent **copy** (a=3, b=4) |
| `18-parse-init.loft` | 10 `as vector<T>` parse locals | II (reference) | **1×/local** — parse result becomes the local |
| `19-while-loop.loft` | `while` loop, per-iter local | I (reference) | store **reused** — flat; all loops reuse |

`tests/scripts/11-vectors.loft` is the field repro (not copied into `probes/`): 44 allocs,
watermark 44 interp / 42 native, all freed at scope exit.

## Reference ↔ problem pairings (REQUIRED if probes ≥ 5)

| Problem | Reference | What the diff reveals |
|---|---|---|
| 07 (35 named locals, watermark 72) | 02 (loop local, watermark 4) | A loop body is a scope that exits each iteration → store **reused**; 35 sibling statements in one scope each allocate + pin. The watermark is driven by *distinct scope-lived bindings*, not by total allocations. |
| 07 (watermark 72) | 08 (35 unbound temps, watermark 4) | Binding to a named local is what pins the store to scope-end; the same value produced as an *unbound* temp frees at statement-end. Cluster I is specifically about **named** locals. |
| 09 (untyped, 72) | 07 (typed, 72) | The `: vector<integer>` annotation does **not** change the count → cluster II's doubling comes from the literal-init temp, not the type annotation. |
| 07/11 (literal/comprehension, 2×) | 12/13 (concat/slice, 1×) | The diff isolates cluster II to *which* init form: `+`/slice emit the result store *as* the local (1×); literal/comprehension build a fresh temp then bind (2×). The fix target is the literal/comprehension materialise-then-copy path, not all inits. |
| 15 (`if`-block locals, all pinned to fn exit) | 02 (loop body, reused) | Both are nested blocks, but only the loop reuses its store; the `if`-block local pins to function exit. So cluster I is **not** "blocks free at block-end" — it is "only loops reuse, everything else pins." |
| 14 (reassignment, old store pinned) | 02 (loop reassign, reused) | A loop's per-iteration `v = [..]` reuses one store; a straight-line `v = [..]; v = [..]` pins each overwritten store to scope exit (cluster III). The divergence is loop-reuse vs overwrite-pin. |

## Tool gaps (OPTIONAL but recommended)

| Tool | Status | Used for |
|---|---|---|
| `LOFT_STORES=log` | Verified-essential | Full alloc/free/dec_rc trace (`src/database/allocation.rs:131`). The alloc/free *interleaving* (`aaaa…ffff…` vs `afafaf…`) is the whole diagnosis. |
| `LOFT_STORES=warn` | Verified-suitable but **coarse** | Threshold hardcoded at `active > 30` (`allocation.rs:142`) — not configurable, and fires on any legitimately-long function. A Stage-C tool fix should make it watermark-relative or raise the floor. |
| Per-alloc source label | **Gap** | Interpreter allocs log an empty `name` field, so the trace can't say which alloc is which local/temp. Native attaches names via `OpFreeRef(var_name)` only on free. Labelling interp allocs would let a future probe attribute the watermark per-binding. |
| `LOFT_TIMEOUT=<secs>` (+ `LOFT_TIMEOUT_GRACE`) | Verified-essential | Watchdog (`src/timeout.rs`). On a hang it prints `phase=… fn=… last op=…` — `last op:(none — crash outside interpreter)` localized @P394's hang variant to the **compile/codegen** phase (not the bytecode loop), which tied it to the `codegen.rs:2669` slot bug. Far better than an external `timeout` SIGTERM. |

## Status & next-session roadmap (REQUIRED)

| Cluster | Mechanism status | Action needed | Effort |
|---|---|---|---|
| I | 🟢 Root-caused + **rc-misdiagnosis RESOLVED**; foundation landed (`00dc10ae`), fix mapped, not yet implemented. **Mechanism (corrected by the investigation agent):** the store backing a vector local is the `__vdb_N` work-ref, null-init **hoisted to body position 0** (`expressions.rs:354`) → registered at **function scope** (`scan_set`, `scopes.rs:635`) → the sole owner freed at function exit (the local has `dep=[__vdb]` so `owns=false`, never freed itself). The earlier `sink_dead_store_frees` relocation was **ineffective for a TIMING reason, not rc** — it ran *after* `compute_intervals`, so the slot interval was already long; the move was cosmetic. **rc=1 throughout** (`dec_rc=0`); a mid-program `OpFreeRef(__vdb)` genuinely frees (proven). | **Scope-registration change, not node relocation, not rc surgery.** Register the confined `__vdb` at its LCA block scope so the existing block-exit `free_vars` frees it there. The chicken-and-egg (free emitted during `scan`, before the confining block is reached; `set_scope` asserts-once) is resolved by a **gated two-phase scan** (`scan` is fresh per fn, `scopes.rs:99`): scan → `store_confinement` map → if non-empty, re-scan with the map so `scan_set`/`scan_if` register the confined `__vdb`(+local) at the block scope. Keep the `Set(__vdb,Null)` declaration at body 0 (native). Full plan: [`fix-design` § Fix approach](fix-design-store-lifetime.md#fix-approach--block-scope-the-slot-two-phase-scan--null-init-relocation). | M+ (mapped) |
| II | ✅ **FIXED `ff8b0730`** — root cause was the redundant `create_vector` `=` `vector_db` (the literal body already allocated v's store via `build_vector_list`). `create_vector` now skips it when the body has the head `Set(v, OpGetField)` repoint. 1× now; literal-init watermark halved. | — | done |
| III | 🟢 Characterised; **fix ATTEMPTED + reverted** — two naive `create_vector` approaches fail: (a) `OpFreeRef(old __vdb)` at the overwrite didn't take **at the `create_vector` site** — the correct site is the `scopes.rs` dead-store path (rc=1, so a single free there suffices — *not* an rc problem); (b) `OpClearVector` + body-refill works for *literal* reassign AND stays correct, but **broadly breaks** native (`var_v` out of scope — moves the *declaration*) + runtime (11-vectors `+=`, text iteration, keyed concat, a SIGSEGV) for non-literal bodies. Benign + below the warn floor (cluster II resolved @P393), so deferred. Distinct surface from I — **guard-silent** (`v` read at fn end ⇒ LCA *is* the function scope). | **scope-analysis-level dead-store freeing**: at each **unconditional** overwrite, free the *prior* `__vdb` in `src/scopes.rs` (the dead-store path), guarded by `!is_captured(v)` — NOT a `create_vector` clear/free. Second commit, after cluster I lands the scope plumbing. | M (mapped) |

**Recommended sequence.** Stage B reading (cluster II first — smallest surface, clear
redundancy, and the concat/slice path is a ready-made reference for the fix) → clusters
III then I (shared dead-store-free surface; do them together with the aliasing guard) →
decide Stage C only if the I/III fix needs option comparison. The **quickest user-visible
win** is the `LOFT_STORES=warn` heuristic floor (raise/relativise the threshold) — it
removes the false-positive noise regardless of whether the watermark itself is touched,
and is XS. Fix in-plan, one cluster per commit (§ template Fix-application discipline).

**Open design question for the user (Stage C).** loft's scope-based freeing
([LIFETIME.md](../../LIFETIME.md)) frees locals at scope exit by design. Clusters I and
III ask whether store-backed locals should additionally free at last-use (I) and on
overwrite (III). That trades a smaller watermark for added free-emission complexity (and
interacts with dep-tracking / aliasing — the same surface as PLAN51/52: a local whose store
is aliased by a still-live binding must not be freed early). This is a deliberate
language-design call, not a mechanical fix — left for the user. Cluster II is independent of
this question (an init redundancy) and is worth fixing regardless.

## Deferred follow-ups (sequenced)

Items this plan's work justifies but defers — recorded here, picked up **in
order**, each gated so it lands clean (the same "come back to it" model as the
rc-removal tail-end).  None is folded into the cluster I/III store-lifetime fix;
the `parallel {}` items in particular are a **sibling discovery**, not part of
this plan's thesis
([plans/README.md § Sibling bugs are discoveries](../README.md#sibling-bugs-are-discoveries-to-record-not-cases-to-fix-in-place)).

1. **Disable store ref-counting** — Goal E continuation.  ✅ **DONE (2026-06)** —
   the Stores ref-count is FULLY REMOVED.  Driven through the `RC_OFF` experiment +
   a rigorous probe corpus ([`probes/rc-removal/`](probes/rc-removal)) in three
   phases: **A** Mechanism 1 — free unbound heap-returning-call temps
   (`efdf8a1c`; closure-temp + de-NRVO'd vector, [`probes/nrvo-inline-leak/`](probes/nrvo-inline-leak));
   **B** Mechanism 2 — closure-cell ownership = the closure-record cascade
   (`328fc80d`); **C** delete `ref_count`/`inc_rc`/`dec_rc`/`OpIncRc`
   (`5745a2c2` + `80208ab1`), the const/global pin retargeted to a `Store.pinned`
   flag, `free_named` now unconditional.  The memory model is now single-ownership
   free + cascade-owned closure cells + `pinned` const stores — no hidden counter.
   (A pre-existing lib-test fixed-filename concurrency bug surfaced during the
   Phase C run and was fixed alongside: [`probes/lib-test-file-concurrency/`](probes/lib-test-file-concurrency),
   `8041d743` + `db3c588a`.)

2. **`parallel {}` capture — soundness floor LANDED; feature deferred.**  Goal A +
   Goal D; **its own scoped case**, not this plan's store-lifetime work.  A
   67-probe battery mapped it fully ([`probes/bugs/`](probes/bugs)): native
   silently no-ops arm bodies; interpret has ~5 deterministic failure modes
   (scalar/text write silent-loss, heap-mutation crash on the read-only worker
   clone, cross-scope SIGSEGV, param SIGSEGV).  **Built:** a precise compile-time
   diagnostic (`reject_unsound_parallel_captures`) that rejects every unbuilt
   capture (writes/mutation/param) on both backends while leaving reads legal —
   regression `tests/scripts/170`.  **Deferred:** the capture *feature* itself
   (channels/coordination), whose real use case is **server/client async I/O** and
   must be driven by that consumer (`lib_plans` 08-server / 10-game-client), not
   designed abstractly.  Residual sibling discovered: `vv[0] += [2]` hits a
   `data.rs:3036` codegen assertion *outside* `parallel {}` too — a separate
   nested-vec element-compound-assign bug.

3. **Nightly differential backend-parity sweep** — the **Goal D standing
   detector** (A has the sanitizer, E has `LOFT_STORE_GUARD`, D has none).  *Gate:*
   after #2, so it is **born green**, not red on the known divergences.  Oracle-free:
   run a curated *deterministic* corpus on `--interpret` and `--native`, compare
   observable output (stdout + crash signature), flag any disagreement — the
   backends are each other's oracle, which catches the self-satisfying-assert blind
   spot (test-80/81 "pass" on native *because* it no-ops the arms).  Nightly because
   `--native` shells to `rustc` per file; complements the curated-deep `cross_mode!`
   matrices.  Hard part = output normalization (ordering / RNG / addresses) → start
   small + curated, expand as the normalizer matures.  Design rationale in
   [GUARDS.md § Backend divergence](../../GUARDS.md#testing-race-prone-and-backend-divergent-mechanics).

## Relationship to the store-lifetime/aliasing class

Kindred to (all closed/finished) [PLAN51 hidden-buffer-aliasing](../finished/51-hidden-buffer-aliasing),
[PLAN52 value-block-borrow-cleanup](../finished/52-value-block-borrow-cleanup),
@P383, @P390, @P391 — the recurring store-lifetime / DbRef-lifetime family that maps onto
soundness-floor A in [GOALS.md](../../GOALS.md). Distinct from those: @P393 is a *watermark/
efficiency* finding with **no correctness failure**, where the others were corruption or
dangling-ref bugs. It is filed as its own plan (not folded into PLAN51) because PLAN51 is
finished.
