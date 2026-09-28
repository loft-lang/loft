<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Fix design — free a vector store's heap allocation at last-use (clusters I + III)

## Why this matters (the real stakes)

This is the implementation of **[GOALS.md Goal E — Predictable memory](../../GOALS.md#goal-e--predictable-memory-the-programmers-model-is-the-truth)**
(the programmer's model *is* the truth).  Not a watermark cosmetic.  A
block-scoped vector that lives until **function exit**
means a running program holds **more heap than the source implies** — and a memory
model the author can't reason about is a liability in itself, especially for a
data-handling language.  The goal is to make heap lifetime match the obvious mental
model: **a vector's store is freed as soon as its last use is past**, regardless of
how the stack slot is allocated.

This unifies **cluster I** (block-confined local pinned to function exit) and
**cluster III** (reassignment doesn't free the overwritten store) — they are the
**same fix surface**: *free a store at the last use of the data it holds.*

## Root cause (pinned, with sites)

The store backing a vector local is the `__vdb_N` work-ref allocated by
`vector_db` (`src/parser/vectors.rs:2137`).  Two mechanisms lift it **and** the
local to **function scope**:

1. `work_references()` **hoist** — the `__vdb_N` null-init is inserted at
   **function-body position 0** (`src/parser/expressions.rs:354`), so `scan_set`
   registers it at function scope.
2. `scan_if`'s **`small_both` pre-registration** (`src/scopes.rs:827-850`) lifts
   small block locals to the body scope.

Both are **load-bearing for the V1 slot allocator** (`src/scopes.rs:188` is
explicit: zone-1 pre-pass depends on outer-scope registration).  So the heap
store's lifetime is **coupled to the function-scoped stack slot**: the block-exit
free sweep (`free_vars`, `src/scopes.rs:937`) never sees the `__vdb` (it's at
function scope), and the store frees only at function teardown.

Loops escape the watermark not by freeing but by **reuse**: a loop body is scanned
once → one `__vdb` reused in place via `OpDatabase` clear+claim each iteration.

## The design principle

**Decouple HEAP-store lifetime from SLOT-scope lifetime.**

- The stack **slot** stays function-scoped — the allocator needs it; do **not**
  fight the lifting (it's load-bearing).
- The heap **store** is freed at the **last use of the local that points into it**
  (cluster I: the local's last read; cluster III: the point of the overwrite).

## Verification harness (already shipped: `0a45e880`)

`LOFT_STORE_GUARD=1` fires when a vector local's references are confined to one
non-loop nested block yet its `__vdb` store is scoped to an ancestor.  Built on the
`guard_refs` confinement walk (mirrors `compute_intervals`' traversal so no
Value-bearing variant is missed).  Read-only, gated, zero behaviour change.

- Fires on the cluster-I shape: `one_block`, `many_blocks` (5), `168` (7), `03-text` (6).
- **Silent on escape cases** (a local read after its block).
- **Does NOT fire on `11-vectors`** — that watermark is function-level sequential
  locals, a *distinct* mechanism (a local at function scope with no nested block to
  free it earlier; needs last-use freeing of *function-level* locals, a follow-on).

**Use it as the driving regression test:** fix the model until the guard is silent
suite-wide, then promote it to a `debug_assertions` assertion so the model can't
silently regress.

### Confinement = least-common-ancestor, NOT exact scope-match (probes 16-27)

A first cut compared every reference's *innermost* block scope and required them
**equal**.  Probes 16-27 proved that wrong — it UNDER-fires, missing the most
common shapes, which would make the fix skip them:

| Probe | Shape | exact-match | LCA (correct) |
|---|---|---|---|
| 20 | vector created in a block, **iterated** there (`for x in a`) | missed (0) | confined ✅ |
| 25 | vector **read inside a loop body** (declared outside the loop) | missed (0) | confined ✅ |
| 26 | vector created in a block, used in a **nested `if`** | missed (0) | confined ✅ |
| 17 | vector local in a **match arm** | confined | confined ✅ |
| 19 | distinct vectors in then / else branches | both confined | both confined ✅ |
| 18 | set in both branches, **read after** (escape) | silent | silent ✅ |
| 27 | reassigned in a branch, **read after** (escape) | silent | silent ✅ |

Root cause of the miss: a for-loop lowers to a nested `#For block`, a nested `if`
adds a sub-block — so the creating refs sit in block `B` and the using refs in a
**sub-block of `B`**.  Their innermost scopes differ, but the data is still
confined to `B`.

**The fix's confinement (and the guard) MUST track the full block/loop scope-PATH
at each reference and take the least-common-ancestor:** `LCA([B]) ⊔ LCA([B,sub]) =
[B]`.  The store frees at the LCA block's exit.  If the LCA's innermost element is
a **loop** scope, the local lives only inside that loop (per-iteration reuse) →
**not** relocatable.  `guard_refs` now does exactly this; probes 16-27 are its
catalogue (`probes/16_*.loft` … `probes/27_*.loft`).

> Probe hygiene note: avoid stdlib names in probe bodies (`sum`, `first`, …) — a
> name collision is a parse error, not a lifetime finding (probe 21 false alarm).

## THE CRUX — RESOLVED: the SLOT lifetime, via the body-0 null-init hoist

Three successive diagnoses were each *incomplete*; the experiments subtracted them
one by one (full ledger in
[`experiments/cluster-I-two-phase.md`](experiments/cluster-I-two-phase.md)) until
the real lever remained.  The progression is itself the record — each was held
confidently and each was wrong:

1. ~~**rc holds the store**~~ — ruled out by the `RC_OFF` flag: the stderr-only
   free order is byte-identical rc-on/off; one `free_named` per store at `rc=1`,
   no `dec_rc`.  rc=1 throughout.
2. ~~**register the `__vdb` at its block scope**~~ — this moves the IR `OpFreeRef`
   into the block but the **runtime free is unchanged** (reliable stderr-only order
   identical on/off).  *The IR free-position does not control the runtime free.*
3. **The runtime free follows the `__vdb`'s SLOT lifetime.**  A vector local `v`
   carries `dep=["__vdb_N"]` ⇒ `owns=false` ⇒ `v` is never freed; the `__vdb`
   (empty dep) is the sole owner.  Its slot is **function-scoped** because its
   null-init (`Set(__vdb, Null)`, the slot allocator's `first_def`) is **hoisted to
   body position 0** by `parse_code` (`expressions.rs:354-369`).  Codegen ties the
   free to that slot.  **Move the `first_def` into the confined block → the slot,
   and the free, become block-scoped.**  *Verified:* it works (see Fix approach).

**Why the hoist exists, and why it over-reaches.** The null-init only has to give
the slot allocator a `first_def` (without one, `assign_slots` skips the var and
codegen panics `"Incorrect var __ref_N[65535]"`, `variables/mod.rs:150-159`).
Body-0 is the *maximally broad* "definitely before first use" placement — **correct
without lifetime information**, but it function-scopes *every* work-ref's slot.  The
confinement analysis IS that missing lifetime information; with it, a confined
`__vdb`'s `first_def` can live in its real (block) scope.  This is the same shape as
the rc objection (a conservative mechanism that out-lived the information gap that
justified it) — and the same Goal E resolution (free at the real scope end once you
*know* it).

## Failed approaches — do NOT re-walk (the ruled-out path)

| Approach | Result |
|---|---|
| Relocate the `OpFreeRef` node as a post-pass (`sink_dead_store_frees`) | Ineffective (then mis-blamed on `compute_intervals` timing; the truer cause is the slot, below).  Reverted. |
| **Register the `__vdb` at its block scope** (move the IR `OpFreeRef`), slot left at body-0 | **Inert — the lever is the slot, not the IR free-position.** Stderr-only free order byte-identical.  Preserved as `experiments/cluster-I-two-phase`. |
| rc holds the store | Ruled out by the `RC_OFF` flag (free order identical rc-on/off). |
| `OpClearVector` store-reuse (cluster III) | Broke native (`var_v` out of scope — moved a *non-confined* declaration into the block) + runtime.  Reverted.  **N.B.** this is why "keep declaration at body-0" was believed — it was learned from a *non-confined* case (see Fix approach). |
| `OpFreeRef(old __vdb)` at the overwrite in `create_vector` (cluster III) | No-op — the dead-store free belongs in `scopes.rs`, not `create_vector`. |

## Fix approach — block-scope the SLOT (two-phase scan + null-init relocation)

**Foundation (`00dc10ae`):** `store_confinement` → `vdb → (local, block_scope)` for
every provably-confined store (adversarially hardened; `store_lifetime_guard` is now
a thin reporter over it).

**Cluster I — if-block case — VERIFIED on the interpreter** (working code in
`experiments/cluster-I-two-phase`).  Two steps, and **step 2 is the lever**:
1. gated two-phase scan: re-scan registering the confined `__vdb` at its block scope
   (`put_scope`) so `get_free_vars` emits `OpFreeRef` inside the block — *necessary
   but inert alone*;
2. **relocate the null-init** (`relocate_null_init`): move `Set(__vdb, Null)` from
   body-0 into the confined block, so the slot's `first_def` — and codegen's free —
   live in the block.

Result on `wm_nonconst` (sequential `if n>k` blocks): stores **interleave**
`+#2 -#2 +#2 -#2` (slot reused, max 1 vector live) vs batched without it.  Sound
(`172` green both backends), correct (`f(9)=13` interp + native), debug asserts pass.

**Native is fine when the `__vdb` is CONFINED** — its declaration, fill, read, and
free all live inside the block, so the emitted `let mut var___vdb` is in-scope.  The
"keep the declaration at body-0 for native" rule came from the *non-confined*
`OpClearVector` case (the var was used *after* the block); **confinement is exactly
the gate that makes moving the declaration safe.**

**Cluster I — top-level sequential case — NOT this fix.**  `11-vectors`/`07` locals
sit at *function* scope, not in a block, so `store_confinement` returns empty and the
guard is silent — block-scoping cannot free them earlier (watermark unchanged at 26,
*by design*).  That half needs **last-use freeing within a scope** — a distinct
mechanism, still open.  Cluster I is therefore *two* sub-problems; this fixes the
if-block one.

**Cluster III** (distinct, guard-silent): free the prior `__vdb` at each
unconditional overwrite in `scopes.rs`, guarded by `!is_captured(v)` — not
`create_vector`.

## Follow-up — the same hoist over-reaches for text (strings)

`parse_code` body-0-hoists `work_texts` / `promoted_text_args`
(`expressions.rs:347-352`) by the identical pattern, so confined `__work` *text*
stores pin to function exit the same way.  The same narrowing (a text-confinement
analysis + null-init relocation) applies once the vector case lands.  Less urgent —
recorded so it is not rediscovered cold.

## Soundness constraints (the guards)

- **Escape** — a local read *after* its block must keep its store function-lived
  (the `guard_confine` `Escaped` verdict; danger case `escape_after_block`).
- **Reassignment** (cluster III) — free the *old* store at the overwrite, only when the
  old mapping is dead (conditional reassignment in a branch keeps both stores live —
  see `escape_after_block`'s `v = [..]` in an `if`, read after).
- **Capture / `&vector` ref-params** — never free a store another live binding points
  into (`is_captured` guard; skip work-ref-aliased / ref-param cases).
- **Loops** — excluded; they already reuse in place.
- **LIFO** — `database::free` enforces reverse-alloc free order.

## Verification — status (2026-06)

**I-a (block-confined) verified general, not a one-repro.** A monotonic watermark field —
`Stores::peak` (`src/database/mod.rs`), the high-water of `max`, readable after a run — was
added so the watermark is a first-class number (no stderr-trace scraping, no buffering
hazard). Measured ON vs `CONF_OFF` across every confinement shape:

| shape (5× sequential) | ON | OFF |
|---|---|---|
| flat `if` | 3 | 7 |
| nested `if` | 4 | 8 |
| single `if/else` | 3 | 3 |
| `if/else` distinct vars | 3 | 10 |
| no-op `else` | 3 | — |
| `match` distinct vars | 3 | 8 |
| loop body | 3 | 3 |

`172` soundness boundary green on both backends; all main-having cluster-I probes exit
clean. The one residual — **shared variable reassigned across sibling blocks** (`if/else`
shared `z` stays 7; `match` shared `x`/`y` stays 8) — is **cluster III**, not a lexical-
scope gap; see [cluster-III-reassignment-pin.md § The single-valued-dep root](cluster-III-reassignment-pin.md).
Paused (overlaps III; do III first). `confine_reassign_safe` + a `multi_store` branch are
wired into `store_confinement` as the inert foundation.

## Verification plan (original)

1. `LOFT_STORE_GUARD` goes **silent** on `one_block` / `many_blocks` / `168` / `03-text`.
2. Watermark drops O(block-locals) → O(1) on `many_blocks` (peak 7 → ~2-3).
3. **Danger cases stay correct on both backends** — `escape_after_block`,
   `escape_skipped`, `dead_at_block_exit`, `loop_reuse_unaffected`
   (`/tmp/cl1/danger.loft`).
4. Concat / self-ref family unbroken — `167`, `168`, `11-vectors`, `135`, `136`, `95`.
5. Full `find_problems` clean; the exit leak gate (`tests/wrap.rs:276`) passes on both
   backends.
6. Promote `LOFT_STORE_GUARD` to a `#[cfg(debug_assertions)]` assertion (regression lock).

## Follow-on — the two open halves (both out of scope here)

Cluster I split in two once I-a (block-confined) landed; neither open half is reached by
`relocate_null_init`:

- **I-b — function-level sequential ("stringing").** Top-level locals (`11-vectors`,
  probe 07) have no nested block to relocate into; each is dead after last read but pins to
  scope exit. Needs **last-use freeing of function-scoped locals** — a separate, larger
  change. (Distinct from "stacking": these have non-overlapping sequential lifetimes that
  *should* reuse one slot, not nested coexisting ones.)
- **Shared-variable-across-blocks → cluster III.** A variable reassigned per sibling block
  shares III's root: **single-valued dep** drops the overwritten store's owning-variable
  link, so it cannot be freed at the overwrite. This overlaps I-b's last-use need and is
  taken **first** — see [cluster-III-reassignment-pin.md](cluster-III-reassignment-pin.md).

**Reorganized by fix mechanism (2026-06 investigation), not by cluster:**
- **Block-confinement** (relocate a store's slot into a narrower block): I-a (done) + the
  shared-block variant (one var reassigned across *sibling* blocks → Route 2 backer-recovery).
- **Last-use freeing** (emit `OpFreeRef` at a `__vdb`'s live-interval end, not scope-end):
  I-b *and* the canonical straight-line cluster III (`v=[a];v=[b]` — measured peak 5 ON==OFF,
  no sub-block to relocate into) **converge** here. `compute_intervals` already has the
  intervals.

Order: the **last-use mechanism** is the higher-value next step (closes canonical III + I-b
together); the shared-block Route-2 piece is smaller and builds on I-a. Full map:
[cluster-III-reassignment-pin.md](cluster-III-reassignment-pin.md).

## Tail-end experiment — disable store ref-counting once scoping is correct

**Goal (user, 2026-06):** after the scoping work lands and `LOFT_STORE_GUARD` is silent,
**turn the store ref-count OFF and see whether it is still needed.**

The primary objection is **NOT that rc is unsound** — it works.  It is that rc **glosses
over the lifetime details**: it is opaque machinery that decides *behind your back* when a
store really dies, so the actual memory model is hidden and can't be reasoned about
directly.  That is precisely the kind of system the user does not trust, and exactly what
[Goal E](../../GOALS.md#goal-e--predictable-memory-the-programmers-model-is-the-truth)
rejects (the C-vs-Rust transparency stance: the *programmer's* model is the truth, not a
counter's).  Removing rc is therefore a **transparency** goal first — make the lifetime
explicit (scope-based, source = truth) instead of abstracted away.

Two concrete consequences that follow:

1. **rc HIDES real lifetime bugs** (the diagnostic side of the same coin).  A ref-count
   masks an incorrect lifetime: an over-retained store (an extra rc holder) never crashes,
   it just lives too long, so a *wrong* free site silently looks fine.  Flipping rc off
   turns it into a **detector**: whatever breaks is a place the scoping is still incomplete
   (a real use-after-free / double-free rc was papering over).  *What breaks is the work.*
2. **Better parallel.** The shared rc counter is cross-thread contention; removing it
   helps parallel execution (already largely mitigated, but cleaner).

**Why this is the right tail-end, in this order:** the [rc crux](#the-crux--resolved-the-slot-lifetime-via-the-body-0-null-init-hoist)
already showed **no vector store needs rc** (`dec_rc=0` every shape — `inc_rc` fires only
for **closure capture**, `fill.rs:1967` / `allocation.rs:299`).  So once correct scoping
frees stores at scope end, the only remaining rc user is closure capture — and the
hypothesis is that scoping (or an explicit capture-copy) can cover that too, making the
whole `ref_count` field droppable.

**Method:** with the scoping fix in place, make `inc_rc` a no-op and `free_named` free at
rc≤0 always (or compile-out the `ref_count`/`OpIncRc`/`dec_rc` path), then run the full
suite + `LOFT_STORE_GUARD` on **both backends**.  Triage every new failure: each is
either (a) a scoping gap to fix (the bug rc hid), or (b) a genuine closure-capture
dependency to handle explicitly.  Outcome = either rc is droppable (delete the mechanism)
or the exact residual set of stores that still need it.  Own plan/branch — careful
refactor.  See [[project_drop_store_refcount]] + [[project_predictable_memory_model]].

### Diagnostic run — RC_OFF over the full suite (2026-06, gate now open)

Reclaim is the default and the Goal-E guard is silent, so the gate is met.  `RC_OFF=1`
(`free_named` frees at rc≤0 always) over the full **1923-test** suite, both backends, fails
**only 4 test groups** — confirming rc is droppable for essentially everything.  The residual
resolves to exactly **two root causes**:

1. **Closure capture (the genuine rc dependency).**  `p22_phase03_multi_factory` +
   `wrap loft_suite`: a `make() -> fn()` factory returns a closure capturing a mutable cell
   `n`; the cell must outlive `make()`'s frame.  rc keeps it alive (`inc_rc` on capture);
   `RC_OFF` frees it when the factory frame exits → the closure reads a freed cell
   (`store()` UAF, `allocation.rs:472`).  **This is the load-bearing rc use** — removing rc
   requires making a closure OWN its captured cells (explicit capture-cell lifetime tied to
   the closure value, not the defining scope).  The substantial sub-project.
2. **One text-vector scoping gap.**  `03-text.loft` leaks `kt=29 main_vector<text>×1` under
   `RC_OFF` — **no closures** (a plain `vector<text>` build/reassign does not repro), so a
   specific shape whose scope-free relied on rc.  An independent, smaller fix.

**So rc is NOT a simple delete.**  The path: (Phase A) locate + fix the `03-text` scoping
gap; (Phase B) closure-cell ownership so capture works without rc — the real blocker;
(Phase C) delete `ref_count` / `OpIncRc` / `inc_rc` / `dec_rc` and verify both backends.
Phase B is its own design (how a closure value owns its cells).

#### Refined by the probe corpus (2026-06) — the residual is TWO mechanisms

The probe corpus ([`probes/rc-removal/`](probes/rc-removal)) sharpened both root causes and
collapsed them with a surfaced stray:

- **Mechanism 1 — an unbound heap-returning-call temporary has no statement-end free.**  The
  `03-text` gap minimizes to `len("a,b".split(','))`: a native builtin's `vector<text>` temp,
  unbound, leaks on the interpreter under `RC_OFF` (a *user* fn's result goes through an sret
  `__ref_N` buffer that IS freed; the native builtin's does not).  **The same shape** is the
  stray `10_closure_passed_as_arg` leak (`apply(make())` — closure temp leaks with rc *on*).
  **One fix** (statement-end free for the unbound temp) covers Phase A AND that stray.
- **Mechanism 2 — the captured cell is freed at the DEFINING frame's exit.**  rc is needed
  ONLY for **≥2 coexisting** closures (store-trace verified: cell freed at frame exit, slot
  reused, coexisting closures alias it).  Single / sequential / read-only / in-frame / text /
  nested closures all survive `RC_OFF`.  Fix = closure-cell ownership.

The closure ↔ collection crashes the probing surfaced (capture a collection / store a
capturing closure) are a **separate, non-rc** closure-record-layout limitation (`P257`
family) — split to [`probes/closure-collection/`](probes/closure-collection), off this path.
