<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# STABILITY_REDFLAG_REMEDIATION.md — collapse the complex code, the bugs fall out

The executable plan behind [STABILITY_REDFLAGS.md](STABILITY_REDFLAGS.md) (the
5-cluster map). Each cluster is **complex code that re-derives one missing fact
across N sites**. The job is **not** to patch the N sites — it is to **land the
fact once**: the N-site code collapses to simple reads, and **the bug family
dissolves with it, by construction.** Sites confirmed against the loft tree on
2026-06-21; re-`grep` before editing.

## Worked precedent (this repo — the template for every step)

The cluster-A `match`-return-over-borrowed native E0308 was fixed by landing **one
fact**: `infer_type` now answers for `Value::Insert` (its tail-item type) — a
6-line case in `src/generation/emit.rs`. That single read made the existing
typed-null handler fire for *every* Insert-arm value-if, so the special-case gap
closed AND the bug auto-fixed (commit `6b29fe30`, regression
`tests/scripts/85-store-lifetime-match-return-borrowed.loft`). **That is the shape
of every step below: a missing fact, computed once, at the chokepoint — the bug is
a side effect of the collapse, not a separate fix.**

## The discipline every step obeys (the loft-codegen gate — non-negotiable)

1. **Prove the working-vs-broken bytecode/source STANDALONE on BOTH backends**
   (`--interpret` AND `--native`) *before* editing the generator. (probe-04 is the
   anti-example.)
2. **Build the boundary matrix** for the fact's class; hand-compute each cell's
   expected value; keep one deliberately-broken control cell red (prove it can fail).
3. **Land at the chokepoint, enforce exactly the invariant** — no narrower (a
   per-site patch leaves the siblings), no wider (a false unification).
4. **Verify the full matrix on BOTH backends + the suite**; graduate a guard to
   `tests/scripts/`. Interp-vs-native divergence IS the failure mode for A and C.

## Leverage order: A → C → B → D

A and C collapse the bulk; B's siblings and E dissolve behind A; D is S-sized
cleanup. Each numbered step is independently shippable + gated; the `prereq` lines
are the only hard ordering constraints.

---

## Cluster A — carry the return/bind ownership dep

**Complex code to delete (loft):** the **11 `has_ref_params` decision sites**
(`src/scopes.rs` ×7, `src/state/codegen.rs` ×4), the **8 `is_borrowed_view`
derivations** (interp `state/codegen.rs` vs native `generation/dispatch.rs` —
computed twice, divergently), the `block_result`/`ref_return` return thicket
(`src/parser/control.rs`), and the `scan_set` runtime store-nr witness
(`src/scopes.rs`).

**Fact to land** (OWNERSHIP_MODEL rows 99/102/104): a heap binding reads ONE
carried answer — *return-dep empty ⇒ **adopt**; `{Attr(src)}` ⇒ **copy** on
escape* (102), the *return-source SET over arms* (99), *one funnelled return path*
(104). **Prereq:** typed `Deps` carries the dep (H2).

**Steps:**
1. **A.1 — compute the dep once.** In the callee analysis, compute return-ownership
   as a *set* of sources (∅⇒Adopt, `{Attr(s)}`⇒CopyFrom(s), mixed-arms⇒union); store
   on `Deps`. *Gate:* the value matches the hand-computed matrix per arm-shape, both
   backends, before any codegen reads it.
2. **A.2 — funnel the return path.** `block_result`/mid-return/native-forwarder
   converge on ONE emit that *reads* A.1 — delete the per-shape adopt/copy/borrow
   arms + the `RetSite` fork. *Gate:* the @PLN85 probe suite green both backends.
3. **A.3 — replace the 11 `has_ref_params` sites with the dep read.** ✅ DONE
   (the 3 live sites — `scopes.rs`, `codegen.rs` `gen_set_first_at_tos` + `gen_set_first_ref_copy`)
   read `Definition::return_adopts_fresh_store()`. *Gate met:* `grep -c has_ref_params`
   → 0 in decision positions; matrix green both backends. See the A.3 progress note
   for the over-unification finding (the dep is broader than `returns_borrowed_view`).
4. **A.4 — unify `is_borrowed_view`** (compute once, shared interp+native) and
   **delete the `scan_set` runtime witness**. *Gate:* the OOB/hidden-only edge agrees
   both backends; a `--static` dump shows the witness ops gone.

**Bug family that auto-fixes:** #405/#406/#409/#410 class, the field-read/`return v`
aliasing, the `infer_type(Insert)` E0308 (already done — one site of this cluster),
and Cluster E's manifestation guards.

---

## Cluster C / H10 — fold `copy_claims` source enumeration onto the keystone

> **Scope corrected 2026-06-22, coordinates re-measured 2026-07-10.**  The original
> framing below (C.0–C.5: fold copy *and* validate *and* construct *and* the keyed
> re-dispatch) was **falsified by a design probe**.  Only `copy_claims` folds.  This
> section is Cluster C's canonical home — the executable plan lives here, not in the
> closed @PLN85 plan directory.  Register entry:
> [STABILITY_HOTSPOTS.md § H10](STABILITY_HOTSPOTS.md); tracking row:
> [STABILITY_ROADMAP.md](STABILITY_ROADMAP.md).  This is a **work item under the light
> flow**, not a plan — the design is settled, so a plan issue would be a pointer.

**Fact to land:** one carried walk — `for_each_owned_child(rec, tp) -> OwnedWalk`
(`src/database/allocation.rs:95`), the per-`Parts` source descriptor.  `remove_claims`
(`:2024`) already reads it and is the model thin-visitor.  `copy_claims` (`:1711`) does
not: its four per-kind helpers each re-roll the same source walk by hand.  That
divergence is the densest historical bug cluster in the tree (@P290 SIGSEGV, @P306/@P318
hash slot-drift, @P309 missing length header, #260/#330).

**What folds and what does NOT.**  The walk has two halves.  **Source enumeration**
("list this record's child slots") is the shared fact and folds.  **Destination build**
("allocate the copy into `to`") is genuinely per-kind and **stays** — unifying it is how
@P318/@P309 come back.  Two paths are ruled OUT of this fold:

- **`validate_claims` (`:1268`) does NOT fold.**  It is a separate *defensive* family: it
  runs on suspected-corrupt heaps (the @P306 `LOFT_TRACE_CR` pre-walk before
  `OpCopyRecord`), so it bounds-checks each pointer *before* following it and does not
  recurse into the per-element-record kinds at all — whereas the keystone **trusts** its
  pointers (`debug_assert!` on a freed/out-of-range record).  Folding it would turn "name
  the broken edge" back into "fault on it".  Boundary pinned in the keystone's
  `OwnedChild` doc comment.
- **`record_new` / `record_finish` (`structures.rs`) do NOT fold.**  A WRITE/build path,
  not a read-walk.  If they share a per-`Parts` *layout* fact (strides/positions), that is
  a separate refactor.

`copy_claims_hash_body` (`:1572`) is the worked template — it already reads
`for child in self.for_each_owned_child(rec, tp).children`, takes `child.owning_elem` as
the source element record, and pairs each with a freshly-claimed destination slot.  The
other three copy that shape with a hand-rolled source loop.

| helper (line) | source walk to replace | destination build that STAYS |
|---|---|---|
| `copy_claims_index_body` (`:1640`) | `collect_index_nodes(rec, left)` — the **same call** the keystone's Index arm makes (`:180`) | `tree::add` re-insert; already mirrors `hash_body` |
| `copy_claims_array_body` (`:1501`) | `for i in 0..length { elm = get_u32_raw(cur, 8+4*i) }` | @P309 length-header `set_u32_raw(into, 4, length)`; per-element slot-copy |
| `copy_claims_seq_vector` (`:1457`) | `for i in 0..length { pos: 8 + size*i }` | one bulk `copy_block(length*size+4)`; positional slot-copy |

Each source walk is ~3–6 lines and matches the keystone position-by-position (verified:
Vector `8+size*i`, Array `8+4*i`, Index the identical `collect_index_nodes`).  So the fold
is mechanical, with **one wrinkle**: `array_body` and `seq_vector` are called with the
*content* type (`*v`, call sites `:1753` and `:1801`), but the keystone wants the
*container* type — folding them needs a small call-site/signature change, not a pure body
edit.  `index_body` already takes the container type, so it is a near drop-in.

### The verifiable phased plan

One helper per phase, each independently shippable.  Prove green on **both backends**
before editing and after each phase; on any red, revert that one site and diagnose before
continuing (bisect-by-site).  `B=./target/release/loft`,
`T=tests/scripts/85-store-lifetime-claims-keystone.loft`.

- **Phase 0 — baseline (no edits).**  Confirm the tree is green so any later red is
  unambiguously the fold: `$B --interpret --tests $T` → ok · `$B --native --tests $T` → ok
  · `LOFT_COPY_CHECK=1 $B --interpret $T` → no mismatch warning ·
  `cargo test --release --test leak` → pass.
- **Phase 1 — fold `index_body` (lowest risk).**  Replace the `collect_index_nodes` source
  walk with the keystone children, reading `child.owning_elem` as the source node; keep the
  `tree::add` destination body unchanged.  *Verify:* `$T` both backends → ok; leak gate →
  pass; `LOFT_COPY_CHECK=1` → clean; `62-index-range-queries.loft` +
  `129-sorted-index-field-deepcopy.loft` both backends → ok.
- **Phase 2 — fold `array_body`.**  Change the call site / signature to pass the container
  type; replace the `8+4*i` source loop with keystone children.  **Keep** the @P309
  length-header write.  *Verify:* `$T` both backends; leak gate; `LOFT_COPY_CHECK=1` clean;
  `374-vector-hash-sibling-dup-key.loft` → ok.
- **Phase 3 — fold `seq_vector`.**  Same container-type adjustment; replace the `8+size*i`
  source positions with keystone children.  **Keep** the single bulk `copy_block` (iterate
  the keystone alongside it — accept the double pass; do not merge them).  *Verify:* `$T`
  both backends; leak gate; `LOFT_COPY_CHECK=1` clean; `182-deep-nested-vector-copy.loft`,
  `183-nested-single-vector.loft`, `152-i319-i320-field-vectors.loft`,
  `163-plan53-cross-store-vector-add.loft` → ok on both backends.
- **Phase 4 — full verification + docs.**  `./scripts/find_problems.sh --bg` then `--wait`
  → nothing new.  `cargo fmt --check` + `cargo clippy --release --lib` clean.  Update the
  keystone `OwnedChild` comment (the three helpers "are a mechanical source-fold away" →
  "now fold") and the H10 register.

**Done when:** `for_each_owned_child` is the single source enumeration for `remove_claims`
and all four `copy_claims` kinds; the keystone guard, the leak gate and `LOFT_COPY_CHECK`
are green on interp **and** `--native`; the suite shows no new failures.  The three
divergent re-encodings of the cascade walk are gone, with destination build correctly left
per-kind.

**Guards.**  `tests/scripts/85-store-lifetime-claims-keystone.loft` covers every axis
(vector, hash, sorted/ordered, index + sibling-sorted = the @P309 axis, multi-heap struct,
inline enum) under the store-leak gate.  `cargo test --release --test leak` catches a
dropped or double-freed element record.  `LOFT_COPY_CHECK=1` (or `LOFT_LOG=copy_check`) walks
source and destination in parallel and warns on a nested-collection mismatch, hooked at
`OpCopyRecord` / `OpReplaceKeyed` / `set_keyed`.

**Tripwire calibration (2026-07-10) — read this before trusting its silence.**  Two faults
were injected into `copy_claims_array_body` and the results differ, which changes how the
phases below must be read:

| injected fault | `LOFT_COPY_CHECK` | guard values |
|---|---|---|
| short length **header** (`length-1`) | **FIRES** — `MISMATCH copy_record.s: src_len=3 dst_len=2`, on BOTH backends | fails |
| truncated element **loop** (`0..length-1`), header still correct | **SILENT** | fails (3/7) |

So `LOFT_COPY_CHECK` detects a *length* divergence, not "right length, wrong or missing
element" — which is exactly the shape an off-by-one **source fold** produces.  It is a
secondary net here, not the primary one.  The primary net for these phases is the guard's
**value assertions** plus the leak gate; both caught the truncation the tripwire missed.
Its silence is only evidence for the length class.  (Earlier text called it "the in-process
tripwire for an off-by-one source fold" — that overstated it.)

---

## Cluster B — stack-delta signal (the wrong-signal siblings)

**Fact:** "did this branch leave a value, how many bytes?" is the runtime net stack
delta (`true_stack != stack_pos`), never the last-expr/function-return *type*. The
`gen_if null_else` (#405) fix is the template.

**Status:** **deferred — currently UNVERIFIABLE.** B.0 (2026-06-21) could not unmask
B5 (`codegen.rs` rebalance, `size(returned())` vs `size(tp)`) — the divergent-arm
precondition doesn't fire for reasonable shapes (loft equalizes arm stack levels
before the join), so a fix has no RED probe and the gate forbids patching it. Pick
up **only when a real trigger appears**, or behind Cluster A (the carried fact makes
the signal explicit). `size_code` (`stack.rs`) is the same family, same disposition.

---

## Cluster D — converge the typed-null encoders  *(S-sized, any gap)*

**Complex code to delete (loft):** the drifting null encoders —
`write_typed_null` (`emit.rs:1029`, native), `emit_typed_null` (interp,
`state/codegen.rs`), `STRING_NULL`/`init_ref_sentinel` (`state/mod.rs`).

**Fact to land:** one `sentinel(tp)` encode/decode table (H6) that every site reads.

**Steps:**
1. **D.1 — make `sentinel(tp)` the single source**; point the native + interp +
   string-null + ref-sentinel sites at it.
2. **D.2 — delete the local re-encodings.** *Gate:* a null round-trip matrix
   (per width / per kind) byte-identical both backends.

---

## Progress (this repo)

- [x] **Cluster A · one site** — `infer_type(Insert)` value-position typed-null gap
      (`emit.rs`). Commit `6b29fe30`; bug auto-fixed: match-return-over-borrowed
      native E0308. *Demonstrates the model: land the fact → bug falls out.*
- [x] **A.4 — unify `is_borrowed_view`** (the divergence fix). The
      `is_borrowed_view` fact was derived THREE times, structurally identically
      but separately (interp `state/codegen.rs` ×2, native `generation/dispatch.rs`
      ×1) — the H4 drift risk. Collapsed onto ONE method,
      `Definition::returns_borrowed_view()`, read by both backends (commit
      `20610eaf`, net −56/+72). Behaviour-preserving (the A.0 return-ownership
      boundary matrix is identical before/after on both backends); full suite
      clean both backends; regression
      `tests/scripts/85-store-lifetime-borrowed-view-query.loft`.
      **`scan_set` runtime witness DELIBERATELY retained** (the over-unification
      guard): `paired_witness`/`OpFreeRefIfDistinct` guards a *statically-
      unresolvable* adopt-vs-orphan case (a callee like `map_from_json` that
      adopts-or-allocates at RUNTIME on its input) — a genuinely different fact
      the single static return-dep cannot carry; forcing it onto the dep regresses
      that case.
- [ ] A.1 — **RE-ATTEMPT 2026-06-22 (after the substrate fix `782937e9` landed):
      the bind-site copy is NOT a bug fix — it CONTRADICTS the documented language
      design. Routed as a DESIGN decision; no code landed (behaviour-preserving).**
      The store-reuse-after-free 3-deep substrate IS fixed and IS unblocking, but it
      was never the real blocker for the bind-site cases (A/C).
      **#426A/C (`a = vv[0]`, `c = o.inner.v`) "must COPY" is FALSE per the docs.**
      LOFT_DATA.md § Vectors/§ Variables (#338) documents vector-ELEMENT reads as
      dep-tracked VIEWS: `a = vv[0]` for `vector<vector<T>>` is a VIEW on base
      (`vv[0] += [9]` ⇒ `len(a) == 4`), CONSISTENTLY with the struct-element view that
      `tests/scripts/294-vector-element-view-semantics.loft` PINS and that #415
      DELIBERATELY excluded index reads to preserve (OWNERSHIP_MODEL row 102: "a
      vector INDEX read keeps its existing nested-stride path"). The probe's "observed
      4: aliases" reads the deliberate VIEW as a bug. Worse, copying the nested-field
      base REGRESSED a real consumer (`p379`, hex_world `set_cell`): `cells =
      chunk.ck_cells; cells[i] = v` RELIES on the field-read alias to write through —
      copying it makes `self: &World` "never modified" (parse error / data loss). The
      carried borrow dep CANNOT distinguish a read-only copy from a write-through
      alias; that is the `find_field_written_vars` mutation analysis (the
      OWNERSHIP_MODEL borrow checker), not a dep-driven codegen patch. So A/C are a
      copy-vs-view LANGUAGE design reconciliation, routed forward. (Validated
      matrix-first, both backends; over-unification guard fired on p379 + test 294.)
      **#426B (return path) is a SEPARATE store-lifetime SUBSTRATE bug (NOT design,
      NOT the design conflict above).** Routing `{ w[0] }` through
      `copy_borrow_tail_into_retbuf` DOES copy into `__retbuf`, but the helper's
      `OpFreeRef(__fwd)` frees a store the freelist recycles into the NEXT allocation,
      corrupting a subsequent borrowing-read copy (`b = idx0(ww); c = by.v` ⇒
      `len(c) == 0`; getv's struct-field tail does not hit it — its `__fwd` views the
      arg base, not a nested element store). The a7-class return-buffer substrate.
      Repro banked `/tmp/p_followups/p426B_returnbuf_store_reuse.loft`.
      **Earlier-attempt notes RE-VERIFIED on this base (still accurate):**
      **(i) Return-source SET — hypothesis FALSIFIED by the leak suite.** Dropping
      the `ret_var==MAX` gate (make the union-of-arms SET primary) over-suppresses a
      free even WITH the companion `skip_free` de-conflation (`1ff929f5`), LEAKING:
      `repro_p365` (`main_vector<integer>`) for the vector widening, and
      `25-nullable-sequences` (`__nullable<NRow>::Some`) for the struct-enum
      widening (both caught by `wrap::loft_suite`'s strict per-script leak
      accounting; the authoritative `leak` + `leak_cases` suites and standalone
      `LOFT_STORES=warn` pass — it's an aggregate-only leak the gate was protecting
      against).  The `ret_var==MAX` gate is LOAD-BEARING for free-suppress
      correctness — it confines the set-path to the multi-arm case `returned_var`
      can't see; single-arm returns already free correctly via `in_ret`, and the
      set-path's `skip_free` mark leaks them.  The companion fix unblocked only the
      keyed ALLOCATION crash, not this broader free-suppress correctness.  Kept the
      original gated `free_vars` (`scopes.rs`).
      **(ii) #426 bind-site/return copy — store-reuse-after-free substrate.** The
      dep-driven copy ("any `OpGetField` whose result type carries a borrow dep ⇒
      allocate + `OpAppendVector`") is CORRECT per case (A `a=vv[0]`, C `c=o.inner.v`
      copy cleanly in ISOLATION, both backends), BUT it makes the source store DEAD
      at the read → FREED at the read site, and a subsequent NESTED 3-deep vector
      build (`vector<vector<vector<T>>>`) into the recycled store corrupts (`len 0`).
      The EXISTING #415 struct-field copy already hits this latently (no test
      followed it with a 3-deep append; 2-deep is fine, only 3-deep corrupts);
      widening to the index / nested reads turned the latent corruption into a real
      regression (`185-nested-boolean-vector`).  The return-path (B `b=idx0(w){w[0]}`)
      is the same substrate — the index-read tail gets the `["??"]` buffer ABI but
      never copies `w[0]` into it (the a7 class).  So #426 stays ALIASED and the
      #415 `expressions.rs` special-case is RETAINED (NOT deleted).
      **TWO substrate bugs to file (pre-existing on `main`):** (1) the
      store-reuse-after-free 3-deep corruption (repro
      `/tmp/p_followups/p426_store_reuse_3deep.loft`); (2) the return-buffer index-tail
      aliasing (#426B, the a7 sibling).  Probe
      `probes/07-borrowing-read-aliasing.loft` documents the residual (RED until the
      substrate fix).  Effort to UNBLOCK A.1: the store-reuse / return-buffer
      substrate (its own investigation — the a7 sibling — NOT a localized
      dep-driven copy).
      **WAY OUT (the unblocking sequence — substrate FIRST, then A.1 lands clean):**
      1. **Fix the store-reuse-after-free substrate** (the deeper root). A
         borrowing-read copy frees its source store at the read, the freed `store_nr`
         is recycled, and a later 3-deep nested build into it corrupts (`len 0`, both
         backends — `p426_store_reuse_3deep.loft`). Matrix-first on the allocator
         recycle path; two candidate chokepoints: (a) the allocator must not hand
         back a `store_nr` while a live downstream build still targets it (a
         use-after-recycle guard on the free-list), or (b) the borrowing-read copy
         must keep the source alive past the read (defer its free to true scope-exit).
         #415's existing copy hits this latently, so the fix is a net stability win
         beyond A.1.
      2. **Fix the free-suppress correctness** (the return-source SET). The
         `ret_var==MAX` gate is load-bearing because the SET marks EVERY source
         `skip_free`, suppressing the free even for a single-arm / owned return that
         SHOULD free. The dep must carry, per source, *owned-and-returned (suppress
         free)* vs *owned-and-freed (don't)* — not just source identity. Then the
         set-path can replace `in_ret` without the `p365` / `25-nullable-sequences`
         leaks.
      3. **THEN A.1 lands clean:** drop the `ret_var==MAX` gate (part i) and the
         dep-driven bind-site/return copy (part ii) read the now-trustworthy dep —
         closing #415 + the #425 inline-call sibling + #426 and DELETING the #415
         `expressions.rs` special-case (the original A.1 goal). probe 07 + the
         row-99/100 matrix are the both-backends gate.
      Track as a **store-allocator substrate investigation** (its own plan); the two
      substrate bugs are its entry points, repros banked in `/tmp/p_followups/`.
- [~] A.2 (funnel the return path) — **a2 LANDED; a7 localized + routed.** The
      implicit-tail whole-arg vector return (`fn idv(v) -> vector { v }`, matrix a2)
      now funnels through the SAME copy-into-`__retbuf` the struct-field tail (#415)
      and the explicit `return v` (`parse_return`) use: one shared helper
      `copy_borrow_tail_into_retbuf`, reached via a new `tail_whole_arg_vector`
      predicate gated to `context == "return from block"` (the one funnelled return
      path).  a2 is RED→GREEN on BOTH backends through bytecode (signature
      `-> vector["__retbuf"]`, not the old `["v"]` borrow); regression
      `tests/scripts/85-store-lifetime-implicit-param-return-copy.loft`.  The #415
      inline copy collapsed into the shared helper (net: 2 borrow-tail cases, 1
      copy site).  The earlier "fires in IR, bytecode reverts" symptom was the leaf
      fix landing in `ref_return` (which records a param `ls` as a borrow dep); the
      funnel sidesteps `ref_return` for the borrow-tail case entirely.  **a7 and a10
      are NOT the row-104 funnel** (see the A.0 table) — a10 was already fixed by the
      #409 forwarder branch on the A.4 floor (green both backends); a7 is a distinct
      *if-return buffer-model* substrate bug (the function buffer is named `__vdb_1`
      and the `if` true arm reuses it as its own build target via
      `OpGetField(__vdb_1, 0)`, then clears+frees the buffer mid-arm).  Its root is
      the `if`/`else` arm `result`-type asymmetry (the ELSE arm alone gets the
      concrete heap type at `parse_if` ~1404, so it runs per-arm buffer delivery;
      the true arm + every `match_arm` get `Unknown` and skip it), which a clean fix
      must reconcile — out of the row-104 funnel's tight scope, routed forward.
- [x] **A.3 — replace the 11 `has_ref_params` sites with the carried adopt-vs-copy
      fact.** The 3 LIVE decision sites (`scopes.rs:946`, `state/codegen.rs:2066`
      + `:2201`; codegen2201 is dead — the whole-suite usage sentinel found zero
      fires across `tests/scripts` + `tests/docs`, kept consistent for revival)
      now read ONE accessor `Definition::return_adopts_fresh_store()` (`data.rs`):
      **dep empty OR the `["??"]` one-buffer marker ⇒ adopt; any real attr index
      ⇒ copy**. `grep -c has_ref_params` in decision positions: 3 → 0.
      **Over-unification guard (the load-bearing finding):** the A.3 fact is
      STRICTLY BROADER than A.4's `returns_borrowed_view()` — that method checks
      only VISIBLE attrs, but the adopt-vs-copy decision must ALSO copy a HIDDEN
      `ref_return` work-ref return (dep `["cv"]`, the caller-reused `__ref_N`
      buffer). The first collapse onto `returns_borrowed_view` REGRESSED
      `143-plan51-cluster3` (cross-iteration `kt=66 Canvas` alias, interp-correct
      / native-leak — caught by the both-backends gate); the broader
      `return_adopts_fresh_store` fact splits `["??"]` (adopt) from `["cv"]`
      (copy) and fixes it. The refinement flips copy→adopt ONLY for the
      genuinely-fresh case (a ref param whose return is a fresh literal). The
      return-ownership boundary matrix (15 cells + control, both backends) is
      green before/after; full `cargo test` clean (the known stale-cdylib
      registry fixtures `p310`/`p171`/`imaging`/`v2_*` are unrelated — they pass
      on a fresh worktree build). `scan_set`/`OpFreeRefIfDistinct` +
      `paired_witness` adopt-vs-orphan witness DELIBERATELY retained (A.4 guard).
      Regression `tests/scripts/85-store-lifetime-return-ownership-adopt.loft`.
- [x] **C.0–C.3 — `for_each_owned_child` keystone** (commit `4ff673f8`): `remove_claims`
      collapsed 9 arms/174 lines → 2 arms/41 lines onto one carried heap-cascade walk;
      `copy_claims_hash_body` reads the keystone spine. Over-unification guard:
      `validate_claims` (defensive mirror, bounds-checks before deref) + `copy_claims`
      destination construction (genuinely per-kind) left separate. Matrix byte-identical
      both backends; @P290/@P306/@P318/@P309 repros green. C.4/C.5 (construction/null-init
      + keyed re-dispatch) routed forward.
- [x] **D.1–D.2 — typed-null encoders converged onto `DbRef::NULL`** (commit `526cac22`
      → cherry-picked `83d0484e`): every heap-ref null encoder reads one `DbRef::NULL`
      const (`keys.rs`) instead of re-spelling `DbRef { store_nr: MAX, … }`; the
      `pos:0`/`pos:8` byte-drift is gone. **Bonus H4 fix:** `fn f() -> character { return
      null }` was native-E0308 / interp-tolerant (the `character` null was routed to the
      integer sentinel) — now `OpConvCharacterFromNull`, byte-identical both backends.
      Over-unification guard: `STRING_NULL` (text `Str` sentinel) + interp `database.null()`
      (allocates a real null *store*, a different mechanism) left distinct. Regression
      `tests/scripts/407-cluster-d-null-sentinel-roundtrip.loft`.
- [ ] B (deferred — unverifiable until a trigger appears)

### A.0 boundary-matrix RED findings (pre-existing return-ownership bugs)

The A.0 matrix (`/tmp` corpus, both backends) surfaced three live, pre-existing
return-ownership bugs — the bug family A.2/A.3 will dissolve, precisely localized:

| Cell | Shape | Symptom (interp / native) | Localization / status |
|---|---|---|---|
| a2 | implicit-tail whole-arg `{ v }` borrow-return | ALIASES (`a.len 4` / 4) | ✅ **FIXED (A.2):** funnelled to the shared `copy_borrow_tail_into_retbuf` (the #415 / explicit-`return v` copy), `context == "return from block"` gate; GREEN both backends |
| a7 | `if`-return over owned literal arms | corrupt (`p0=null(oob)`, both backends agree wrong) | ⏳ **localized, routed** — NOT the funnel: the function return buffer is named `__vdb_1`, the `if` true arm reuses it as its own build target (`OpGetField(__vdb_1, 0)`) then clears+frees it mid-arm; root is the `if`/`else` arm `result`-type asymmetry (`parse_if` ~1404: only the ELSE arm gets the concrete heap type → runs per-arm delivery; true arm + `match_arm` get `Unknown`). A separate if-return-buffer-model fix. |
| a10 | forwarder `return mk(n)` | (on the A.4 floor) no leak, correct value | ✅ already GREEN — the #409 `native_forwarder` branch (`block_result`) copies the forward into `__retbuf`; the A.0 "interp LEAK ×2 kt=19" finding predates that landing |

The explicit `return v` (a1), `match`-return owned arms (a6), struct param return
(a8), field-of-param return (a5), and now the implicit-tail whole-arg return (a2)
all PASS both backends — the working templates the funnel converges onto.  Matrix
corpus reconstructed at `/tmp/clusterA_matrix` (13 cells + control), baseline-
classified against the A.4-floor binary: only a2 flipped FAIL→PASS, no green cell
regressed, both backends.

## Tracking

No bugs filed — open items map to existing `OWNERSHIP_MODEL` holes (99/102/103/104)
and H-rows (H2/H3/H4/H6); the genuinely-NEW ones (C keystone, B siblings) get a
forward H-row when picked up. Each landed step updates OWNERSHIP_MODEL's hole table.
