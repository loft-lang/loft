<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN85 / D-own — collapse the adopt/free re-derivation thicket (driven by #457)

> **The goal is NOT to fix [#457](https://github.com/loft-lang/loft/issues/457).** It is to
> close the *class* by completing one ownership fact, so this shape and every adopt-across-arms
> sibling close **by construction** and the per-site free-derivation thicket shrinks (net code
> DOWN). #457 is the **driving diagnostic** for this slice — exactly what #448 was for the
> [D-own-1 delivery collapse](D-own-1-return-delivery-collapse.md). Patching #457 by adding a
> shape-condition is the anti-pattern this slice exists to delete
> ([[evolve-data-structures-when-burdened]]).

This is the **adopt/free** sibling of D-own-1's **delivery** collapse — the same beacon
([OWNERSHIP_MODEL.md § ACTIVE](../../OWNERSHIP_MODEL.md#active--the-simplification-exploration-next-days-exploratory--revertable)),
the next per-site thicket. Branch: `pln85-ownership-collapse`. Exploratory + revertable,
oracle-guarded.

---

## 1. The diagnostic — #457, fully root-caused

**A `vector<text>` is corrupted: its length reads garbage (wrong result) and the process
SIGSEGVs non-deterministically.** Filed as a 2026.6.0 regression that turned a green RFC-6962
(zero-trust) `loft test` red with no app-code change. Pure loft (crypto incidental).

**Minimal repro (both backends print `len(p) == 2`, want 1):**
[`probes/457-adopt-free-min.loft`](probes/457-adopt-free-min.loft) — 12 lines, also inlined here:

```loft
fn b2b(s: text) -> vector<u8> { r: vector<u8> = []; for c in s { r += [1]; } return r; }
fn mleaf(x: text) -> text { b: vector<u8> = [0]; b += b2b(x); return "h"; }
fn path(m: integer, lo: integer, hi: integer) -> vector<text> {
  out: vector<text> = []; n = hi - lo;
  if n <= 1 { return out; }
  k = 1;
  if m < k { out = path(m, lo, lo + k); out += [ mleaf("a") ]; }
  else     { out = path(m - k, lo + k, hi); out += [ mleaf("b") ]; }
  return out;
}
fn main() { p = path(0, 0, 2); r = mleaf("e"); assert(len(p) == 1, "..."); }  // len == 2
```

**Exact mechanism (read off the `loft introspect` IR of `n_path`):**

- `out` is the function's NRVO return buffer (backed by `__vdb_1`). Each arm reassigns it to a
  recursive return delivered into a per-arm buffer: `out = n_path(…, __ref_1)` (if-arm),
  `out = n_path(…, __ref_2)` (else-arm). `out` **adopts** that buffer's store.
- At function exit the codegen **unconditionally frees BOTH** buffers —
  `OpFreeRef(__ref_1); OpFreeRef(__ref_2);` — *then* `return out`. So the buffer `out` adopted
  (whichever arm ran) is **freed before it is returned**.
- The caller's `p` therefore holds a **dangling store**. `mleaf`'s `b = [0]; b += b2b("e")` =
  `[0,1]` (len **2**) allocates into the freed slot; `len(p)` reads `b`'s length → **2**. On
  larger trees the reuse lands elsewhere → the non-deterministic SIGSEGV.

**Bisection (necessary + sufficient — re-check before narrowing):**

| probe | change | result |
|---|---|---|
| M | the full thing minus `verify` — just `p = path(); r = mleaf(); len(p)` | **repros** (verify/for-loop/multi-arg all unneeded) |
| Q | strip to if/else + `out = path()` both arms + trailing `mleaf` | **repros** (the clean minimal) |
| R | only the **if** arm reassigns `out` | clean — **both arms is necessary** |
| S | Q without the trailing `r = mleaf("e")` | clean — **a slot-reuse build is necessary** |
| A–P | param-only, inline build, single-assign, no-recursion, no-else | clean — none alone triggers it |

So the trigger is precisely: **a returned local reassigned to a recursive return in ≥2
branches** (the per-arm `__ref` buffers) **+ a later heap allocation to reoccupy the freed
slot.**

---

## 2. The class + the one invariant (design-protocol step 4)

This is the open OWNERSHIP_MODEL hole — *"`returned_var` collapses `match`/`if` → a return-source
**set** (union of arms), not one var"* ([OWNERSHIP_MODEL.md § holes](../../OWNERSHIP_MODEL.md#the-migration-backlog-provenance-record--live-status-is-formalownershipmd),
row "returned_var") — in its **adopt/free** facet, and cluster-V's invariant
([cluster-V-nrvo-adopt-ownership.md](cluster-V-nrvo-adopt-ownership.md)):

> **The invariant to complete:** *a vector local's `dep` = the store it owns.* When `out`
> adopts a buffer (in **any** arm), `out` **owns** that store; the buffer is **moved**
> (emptied), not separately freed. The exit `OpFreeRef(__ref_N)` is a no-op **by the fact**, not
> by a per-shape condition.

When it holds, the per-`__ref`-per-shape free decisions collapse to **one ownership read** —
"does `out` own this store? then don't free it separately" — and no arm/shape can be missed
(the fact covers all of them). #457 becomes a *cell* of the fact, not a special case.

The deviation (the complexity to delete): codegen re-derives "adopt-vs-free this buffer" per
return *shape* (the `__ref` free placement + `return_adopts_fresh_store` + the witness-free),
and #457 is a shape it gets wrong.

---

## 3. The seam — why this is almost certainly D-own-1 fallout (check first)

#457 is a **regression**, and the strong hypothesis is that the **D-own-1 delivery collapse
introduced it**: D-own-1 collapsed the return-*delivery* thicket and was "swept dry" across
~41 probes — but that sweep covered **if/match TAILS** (the function tail is a branch), **not
this shape**: a **mid-body** `out = recursive()` in both arms, then `out += [...]`, then a
*separate* `return out`. The delivery collapse and the adopt/free collapse are **not yet one
fact**, and #457 is the seam between them.

**First concrete step (NOT bisect — it destroys WIP):** `git log --oneline -- src/parser/control.rs
src/scopes.rs` + `git show <commit>` on the recent D-own-1 / return-source / `__ref`-free
commits (the `cc69101b` / `c9b8f154` / `0f79737b` / `0fcf66fa` family and any `OpFreeRef`/
`collect_return_sources` change). Confirm whether the unconditional `OpFreeRef(__ref_1);
OpFreeRef(__ref_2)` at the multi-arm-adopt exit was introduced there. That tells us if this is a
delivery/adopt seam (most likely) or an older latent shape.

---

## 4. The method (design-protocol — instrument before building)

1. **Count the re-assertion sites.** Find every place that decides adopt-vs-free for a
   `__ref`/`__vdb` buffer (the `OpFreeRef(__ref_N)` emission at return, `return_adopts_fresh_store`,
   the witness-free at `scopes.rs:86–90` / `:930–946`, the ref_return adopt). That count is the
   instrument — the thicket to collapse.
2. **Probe deps-sufficiency.** Is the fact ALREADY there but unread at these sites (like
   D-own-1's delivery branch — the fix is "read it once"), or is there a genuine **D-own-2 gap**
   (the multi-arm adopt isn't representable in `deps` and must be completed first)? Use
   `probes/457-adopt-free-min.loft` + the `LOFT_LOG=fn:n_path` IR. The collapse cannot be wider
   than the fact is complete.
3. **Complete the fact at the chokepoint.** When `out` (the return source) adopts a buffer,
   move it (empty the `__ref`) so the exit free is a no-op — driven by the ownership dep, not a
   shape classifier. **Delete** the per-site condition(s) it replaces in the same step. Net code
   DOWN.
4. **Small step + both backends green at every commit** (the migration discipline). If it can't
   be small, the fact isn't isolated — split it.

---

## 5. Fix site (where the chokepoint lives)

- **`src/scopes.rs`** — the adopt/free placement: `return_adopts_fresh_store`, the witness-free
  (comment at `:86–90`: "it adopts the buffer's store … witness's free — a no-op in the adoption
  case"), `collect_return_sources`. The multi-arm adopt must put **each arm's** adopted buffer
  into `out`'s return-source set so it is not freed.
- **`src/parser/control.rs`** — `block_result` / the `ref_return` promotion + the `__ref` free
  emission (the D-own-1 `Delivery` selector + `fresh_owned_vector_deps` live here; the adopt/free
  decision is the sibling of the delivery decision).
- The IR to target: in `n_path`, the exit emits `OpFreeRef(__ref_1); OpFreeRef(__ref_2); return
  out;` where `out` adopted one of them. The fix makes the adopted buffer's free a no-op.

---

## 6. Safety net + landing rule

- **Matrix-first** (CLAUDE.md): value **+ length + leak** on **both backends** (a leak-only probe
  misses a double-append/reuse — a delivery that doubles a vector is leak-free). The minimal repro
  asserts length; extend to value + `LOFT_NATIVE_LEAK_CHECK` / `LOFT_STORES=warn`.
- **Sweep the shape-space dry** before declaring the class closed — the axes D-own-1's sweep
  missed: `match` arms (not just `if/else`), nested `if`, `out = adopt()` then `+=` vs then
  another `adopt()`, struct/text/Reference/enum element types (not just `vector<text>`), 3+ arms,
  recursion depth, the buffer-taken cell. Each green on both backends.
- **Graduate** `probes/457-adopt-free-min.loft` → `tests/scripts/85-*.loft` (a `85-store-lifetime-…`
  regression) so CI catches the next sibling by construction. Add a `tests/oracle/` guard (@PLN89).
- **End-to-end:** confirm the zero-trust RFC-6962 package's `loft test` goes back green,
  leak-clean both backends. **ZT is checked out beside loft:**
  `/home/jurjens/workspace/zero-trust-shared-files/`.
- **Revert rule:** if a step regresses, bisect by SITE (apply one site, re-run the matrix) and
  revert that site. The win is measured in **deleted** per-site conditions + shrunk line count,
  with zero behaviour change on valid programs.

---

## 7. Status

- [x] #457 reproduced (pure loft), minimized to 12 lines, mechanism root-caused, class named.
      Repro: [`probes/457-adopt-free-min.loft`](probes/457-adopt-free-min.loft).
- [x] Regression-origin read (§3) — the `Type::Reference | Enum` gate on the witness-pairing
      predates D-own-1 (since #137, `5afb054c`).  So #457 is the **seam**, not D-own-1 fallout:
      D-own-1 made the vector return-delivery *adopt* `__ref_N` (like Reference), but the
      adopt/free protection was never widened to vectors.
- [x] Instrument + deps-sufficiency (§4.1–4.2) — **fact-unread, not a D-own-2 gap**.  The
      witness-pairing (`paired_witness` → `OpFreeRefIfDistinct(__ref_N, witness)`) already exists;
      it was just type-restricted to Reference/Enum.  `vector_adopts` at scan time reads
      `returned().depend() == [hidden_buffer_attr]` (NOT the `u16::MAX` marker — that's promoted
      later, so `return_adopts_fresh_store()` reads `false`).
- [x] **PARTIAL fix landed** (`47b30a53`, `scopes.rs`): extend the witness-pairing to vector
      adopters + an explicit `OpFreeRef(v)` for a non-return-source adopter (it keeps its dep, so
      no null-init-materialisation orphan).  No new opcodes.  **#457 inclusion-proof subject FIXED**
      (ZT `test_directory` + issue repro pass, baseline SIGSEGVs); full loft suite **2538/2538**,
      leak-clean both backends.
- [x] Sweep + graduate — [`probes/457-shape-sweep.loft`](probes/457-shape-sweep.loft) (if/else ·
      match-STATEMENT · nested-if · 3-arm · int/struct/text elems · recursion depth, value+length+
      leak both backends) → `tests/scripts/85-store-lifetime-457-vector-adopt-free.loft`.
- [x] **CLOSED — the delivery root fix landed; #457 fully fixed, both backends, ZT green, no #306.**

## 8. Resolution — the aliasing-safe delivery, and the thicket DELETED

The free-side patch (`47b30a53`) was abandoned: it grew the thicket and left two residuals because
it patched around the **dep-mismatch root** — a vector adopter's *static* dep says "`v` borrows
`__ref_N`" while its *runtime* store is a deeper/adopted store.  The real fix moves to the **return
delivery**, in two small commits:

1. **`cd66579b` — aliasing-safe delivery (`OpReplaceVector`).**  The return machinery delivered a
   `return out` into the buffer with `OpClearVector(buf); OpAppendVector(buf, out)`.  When `out`
   ALIASES `buf` (the NRVO case), the clear emptied `out` before the append read it — a **self-copy
   that returned EMPTY**.  This was **R2**: NOT a `verify_consistency` bug as first guessed, but
   subproof's `m == n` base case (`out += [mth]; return out`, `out` aliasing the buffer) coming back
   empty (and `f(0)` returned len 0 on `main`).  Fix: one op — `vector_replace` no-ops when dest and
   src are the same backing vector, else clears+appends — replacing the clear+append pair in
   `deliver_mid_vector_returns`.  Removes the "callers must guarantee non-aliasing" burden.
2. **`<this commit>` — the adopt delivers; the thicket is DELETED.**  With delivery aliasing-safe,
   the implicit-tail adopt (`cv = recurse(.., __ref_N)`, then implicit `cv` tail) delivers `cv` into
   the buffer via `OpReplaceVector` and returns the buffer.  So the fn ALWAYS returns its buffer, the
   dep is accurate, and the per-arm `__ref_N` free is the plain `OpFreeRef` again.  `src/scopes.rs`
   reverts to **origin/main** — the `vector_adopters` set, the explicit free, and the witness-pairing
   extension are all gone.  **R1** (the #306 stack-store-free noise) vanishes with the explicit free.

**Net for #457:** the fix is one aliasing-safe op + a tail delivery in `control.rs`, with `scopes.rs`
UNCHANGED from origin/main — the free-side derivation is GONE, not grown.  The earlier failed
free-side approaches (pairing-only leaks; strip-the-dep orphans a null-init store; explicit-free
hits #306) all stemmed from fighting the dep-mismatch on the free side; delivering into the buffer
makes the dep true by construction so none of them are needed.

**Verified:** full loft suite **2538/2538** both backends; `457-min` + `457-shape-sweep` leak-clean
both backends; the self-copy guard `tests/scripts/85-store-lifetime-mid-return-aliasing-deliver.loft`;
ZT `directory` (inclusion + consistency + §8.3) + `fedops` + `membership` + `records` all pass with
**zero #306**.

## Anchors

- Sibling slice: [D-own-1-return-delivery-collapse.md](D-own-1-return-delivery-collapse.md) (the
  delivery collapse — done; this is its adopt/free seam).
- Invariant: [cluster-V-nrvo-adopt-ownership.md](cluster-V-nrvo-adopt-ownership.md) ("a local's
  dep = the store it owns").
- Beacon + holes: [OWNERSHIP_MODEL.md § ACTIVE](../../OWNERSHIP_MODEL.md#active--the-simplification-exploration-next-days-exploratory--revertable)
  and § The current holes (the "returned_var collapses match/if" row).
- Method: [CODEGEN_METHOD.md](../../CODEGEN_METHOD.md) · the `design-protocol` skill · CLAUDE.md § matrix-first.
- Issue: [#457](https://github.com/loft-lang/loft/issues/457). Consumer: `/home/jurjens/workspace/zero-trust-shared-files/`.
