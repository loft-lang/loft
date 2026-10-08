<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN85 / D-own-2 — complete the ownership fact for every binding/path — **CLOSED 2026-07-04**

The completeness deviation ([formal/ownership.md D-own-2](../../formal/ownership.md)):
*not every binding/path has a computed ownership fact; uncovered paths fall back to
a heuristic/stopgap, and a divergence hides until a test hits the path.*

**CLOSED 2026-07-04 (@PLN90).**  The ownership fact is now TOTAL: (1) `ownership_of`
computes an `Own` for every value (the `_ => Owned` tail covers only fresh-owned /
scalar / payload-less values — not a hole); (2) the free side reads that one fact
(`ref_rhs_ownership` is a pure oracle read, `RefRhs::Unknown` deleted); (3) the
inherently-runtime JOIN is completed per-path by the `_own_store` witness (loft#495,
commits 44fd7d72 + a4bcad5b).  The transition class was swept dry over 6 shapes (2 live
over-frees fixed, 4 safe); the value-vs-bind + deps-carried-join residuals are computed
+ safe (probed both backends, `probes/d-own-2/value-vs-bind-inert.loft`,
`call-owned-arm-join-safe.loft`); validated by the full suite 2600/2600 + native_scripts
+ poison + native leak-check + DA + differential oracle + the `program_ownership` fuzzer
(3108 execs, 0 findings).  The single-fact UNIFICATION (three cooperating mechanisms →
one `deps` read) rides D-own-1; the adopt-vs-view OPTIMISATION is @PLN90's copy-lint.

This doc records the 2026-07-04 measurement + the concrete live bugs it surfaced.

## Measurement 1 — the free-side classifier vs the oracle (latent, not a bug)

Instrumented every `scan_set` #316 site to log where `ref_rhs_ownership` (free-side)
and the `ownership_of` oracle would drive a DIFFERENT `owned_refs` action, swept all
359 `tests/scripts`: **8997 divergences, 100% one-directional** —
`free=Unknown(don't-track)` vs `oracle=Owned(track)`, ALL on compiler-gen temps
(`_elm_N`/`__vdb_N`/`__ref_N`).  ZERO dangerous-direction divergences (the free side
never says Owned where the oracle says Borrowed).  So the free-side classifier is
*incomplete but SAFE*: it conservatively leaves owned temps untracked (freed at scope
exit instead of at the #316 transition), and those temps don't reach the transition-
free's firing condition anyway.  No live bug; unifying the two onto one three-valued
fact (Owned / Borrowed / **Unknown**, each consumer rounding Unknown its own safe
way) is the clean completion — a semantic oracle change, @PLN90's charter.

## Measurement 2 — a LIVE bug: struct whole-value copy-return leaks on native

Probing arbitrary borrowing returns surfaced a real one
([`probes/d-own-2/`](probes/d-own-2/)):

```loft
fn idcopy(x: Box) -> Box { r = x; r }          // struct — LEAKS Box×1 on native
fn vcopy(x: vector<int>) -> vector<int> { r = x; r }   // vector — CLEAN both backends
fn choose(cond, x: Box) -> Box { r = x; if cond { r = Box{..}; } r }  // the JOIN face — LEAKS
```

- **Symptom:** native leaks the returned store; interp is clean (a D-op-2 backend
  divergence).  `idcopy`'s return type is `Box["r", "x"]`; `vcopy`'s is
  `vector<int>["??"]` (owned).  The struct return carries the VISIBLE param `x`, so
  the caller's `returns_borrowed_view()` reads *true* → it treats the result as a
  borrow of `x` → never frees the owned copy.
- **Root (exhaustively traced with `LOFT_TRACE_RR`):** a CROSS-PASS staleness.
  Per @C86, `r = x` COPIES (confirmed: mutating `r` doesn't touch `x`; native emits
  `OpDatabase`+`OpCopyRecord`), so `r` OWNS its store and must not carry `x`.  The
  @C86 whole-value copy dep-strip is implemented for VECTORS (`classify_vec_bind` →
  `make_independent`, `parser/expressions.rs`) but there is NO struct equivalent —
  AND the var-copy strip is deliberately pass-2-only (to avoid the vector elm-var
  counter drift).  So in **pass 1** `r`'s dep is still `[x]`; `ref_return` pulls `x`
  into the return dep (`v=x verdict=MergeAttr{a:0}`).  In **pass 2** `r` is stripped
  to owned (`Deps{items:[]}`), but `ref_return` seeds `dep = cur.clone()` from the
  pass-1 return type — carrying the stale `x` (`ret=Reference(621, [1, 0])`).  The
  vector path does not hit this (its `ref_return` handling yields the `["??"]`
  one-buffer owned marker); the struct/Reference path does.
- **The fact to complete (D-own-2):** `r = x` on a struct is a COPY → the return
  OWNS `r`'s store and does NOT borrow `x`.
- **FIX — LANDED (2026-07-04, commit `3f0330c1`, locus 2):** in `ref_return`, before
  the return-dep finalization, PRUNE any VISIBLE-attr return dep whose var is NOT in
  `expanded` (the pass-2 transitive return-source set).  A visible attr the caller
  reads through `returns_borrowed_view()` MUST name a real pass-2 borrow source; the
  stale pass-1 `x` (carried by `dep = cur.clone()`) names none, so it drops.
  `expanded` already includes transitive deps, so a GENUINE borrow (`fn id(x)->Box{x}`
  keeps `Box["x"]`; a returned view of `x` keeps `x`) is preserved; hidden buffer
  attrs are never pruned.  Chose locus 2 over locus 1 (a pass-1 struct copy-strip)
  because it is surgical (touches only the return-dep, not the bind-site / pass
  counter) and provably behaviour-preserving where there is no stale dep.
- **Guard:** `tests/scripts/85-struct-copy-return-owned.loft` (graduated — copy /
  copy+mutate / genuine borrow / nested-field / the JOIN `choose` / loop).  Root
  probes retained under `probes/d-own-2/`.

## Measurement 3 — the return adopt-vs-copy class SWEPT (the finish)

Swept the return adopt-vs-copy class across the composition axes (the D-own-1
"swept-dry" method), BOTH backends, value + leak:

- **Types:** struct (Reference), enum, vector, sorted (keyed), tuple, text.
- **Bindings:** copy (`r = x`), direct borrow (`x`), field-read (`o.f`),
  method-result, call-result, deep projection (`o.m.i`).
- **Control:** straight-line, `if`-JOIN (borrow-arm / owned-arm), `match`-arm,
  call-chain, reassign-then-return.

**All CLEAN** (leak-free, value-correct) EXCEPT one facet: a JOIN **vector** return
whose owned arm is a `=` literal reassignment delivers the WRONG VALUE — the literal
APPENDS to the buffer the borrow-arm's `r = x` filled instead of REPLACING it (returned
`len` 5, not 2; `len(r)` computed INSIDE the fn is correct).  Root: `build_vector_list`
(`vectors.rs:1848`) gates the clearing `vector_db` on `dep.is_empty()` with no clear
fallback, so a `=` reassign on a vector that already OWNS content (the NRVO buffer)
appends.  This is the DELIVERY-value facet (distinct from the caller-free/leak facet
fixed above), needing the `=`-vs-`+=` clear routing, not a return-dep change.  **Filed
[loft#492](https://github.com/loft-lang/loft/issues/492)** (sev:medium, root-caused);
probe `probes/d-own-2/join-vector-return-append.loft`.

## Status

- [x] Measured the free-side/oracle incompleteness — latent + safe (one-directional).
- [x] Root-caused + **FIXED** the live struct-copy-return native leak (locus 2 —
      `ref_return` visible-attr prune; `idcopy` `Box["r","x"]`→`Box["r"]`, `iddirect`
      stays `Box["x"]`; 0-diff on 8 corpora; full DA + suite + oracle + poison green).
- [x] Swept the return adopt-vs-copy class dry (all types × bindings × control) — one
      residual facet found: the JOIN-vector-return DELIVERY value bug, filed as #492.
- [x] **loft#492 FIXED** (commit `f88833c2`): `create_vector`'s `=` path emits
      `OpClearVector` when `vector_db` no-ops on an argument buffer for a non-empty
      literal — the buffer can't get a fresh backing, and `=` is a replace, so the clear
      is correct on a fresh buffer (no-op) and a filled one (JOIN arm / loop re-pass).
      `+=` untouched.  Guard `tests/scripts/85-join-vector-return-replace.loft`; 7/8
      corpora 0-diff (promotion adds one clear, behaviour-neutral); full DA + suite +
      oracle green.
- [ ] Free-side/oracle unification (Measurement 1) — the three-valued-fact completion,
      latent, folds into @PLN90.

**D-own-2 return adopt-vs-copy class: CLOSED.**  Both live facets fixed — the caller-free
leak (`3f0330c1`) and the delivery value (`f88833c2`, #492); the class swept dry across
all types × bindings × control.  The only remaining D-own-2 item is the latent + safe
free-side/oracle three-valued-fact unification → @PLN90.

## @PLN90 slice — the runtime-Join witness (loft#495, 2026-07-04) — **FIXED**

Probing the free-side completeness on the @PLN90 branch surfaced a LIVE native
miscompile (filed **loft#495**, **FIXED commit 44fd7d72**), the concrete first
slice of the runtime-Join witness residual forward-homed at @PLN85 close-out.

**The fix — a native runtime owned-store witness** (`_own_store_<name>: DbRef`,
prologue-declared per runtime-Join local). `output_set` (generation/dispatch.rs)
routes such vars through the tracker: an OWNED assign points it at r's fresh
store; a BORROW reassign frees the tracked owned store it displaces (never r's new
view) and NULLs it; the scope-exit `OpFreeRef` (generation/ops/ref_ops.rs) frees
`_own_store_<name>`. So the copy is freed on the empty-loop path and the view is
never freed on the loop-ran path. `collect_witness_vars` (generation/mod.rs)
scopes it to `owned >= 1 && borrow >= 1` — 4 scripts trigger it, all genuine
ownership-transition cases; interp is untouched (was already correct). Validated:
dn2batch 6/6, guard `tests/scripts/85-runtime-join-loop-copy-view.loft`, full
suite 2600/2600 + native_scripts + poison + native leak-check + @C86 mutation.

### Sweep of the transition class (commit a4bcad5b + probes)

Probed six shapes on both backends; found + fixed **two** live native over-frees,
confirmed **four** safe:

| shape | owned/borrow | result |
|---|---|---|
| `r=x; for{ r=v[i]??x }` (loop) | 1 / ≥1 | LIVE → fixed (#495, 44fd7d72) |
| `r=x; for{ r=v[i]??x; r=Box{..} }` (mixed owned reassign) | ≥2 / ≥1 | LIVE → fixed (a4bcad5b) |
| `first = items[0]??d; first.v` (borrow-only) | 0 / ≥1 | SAFE — borrow-TYPED, never a candidate |
| `r=x; for{ r=pick(v,i) }` (borrow via call) | 1 / ≥1 | clean |
| `r=x; if c { r=v[0]??x }` (conditional, non-loop) | 1 / ≥1 | clean |
| `r=x; for{ r=pick(v,i) }` with pick's OWNED arm running | 1 / ≥1 | clean (no leak, correct value) |

The **owned>=2 fix** (a4bcad5b): the 2nd owned assign, on the reassign path, ran
`owned_ref_reassign` / `OpDatabase(var_r)` in-place reuse against a var_r HOLDING A
BORROWED VIEW. `output_set_witnessed` now resets var_r to the null sentinel before
the value so it allocates fresh (never reusing/freeing the view), frees the tracked
owned store it displaces, and tracks the new owned store. Probes:
`probes/d-own-2/loop-copy-view-native-divergence.loft` (owned==1),
`owned-reassign-mixed-native-divergence.loft` (owned>=2).

RESIDUAL (latent, safe): a call whose return is a runtime owned/borrow Join
(`pick` = `v[i] ?? Box{..}`) is classified conservatively as borrow; the OWNED-arm
store is not tracked, but empirically neither leaks nor corrupts (materialised via
the return-buffer machinery) — the deps-carried-join, unfuzzed axis, still open.

Original root-cause below.

Shape: `r = x (@C86 copy);  for i { r = v[i] ?? x }` — a Reference local that starts
OWNED then is reassigned to a **view** each iteration. `ownership_of(v[i] ?? x)` =
`Join{base=v}` (owned on neither arm). Native over-frees the borrowed views at BOTH
free-sites: (1) the in-loop `owned_ref_reassign` displaced-free (`dispatch.rs`)
fires every iteration and releases `v[i-1]` (a live element store); (2) the
scope-exit `OpFreeRef(var_r)` releases r's final view. Both gates key on
`tp(r).depend().is_empty()` (r is typed owned). Interp stays clean (no in-loop free
+ lenient interior-ref free). Heisenbug: a `println` in the loop shifts allocation
order and hides it; needs a 3-function program (allocation-order sensitive).

Empirically proven (both attempts land in the plan, both fail):
- a store-aliasing witness guard against the Join base is INEFFECTIVE —
  `vector<ref(T)>` elements are separate stores from the vector, so `_old.store_nr
  != var_v.store_nr` is always true;
- suppressing ONLY the in-loop free still panics 5/5 — the scope-exit free alone
  corrupts.

Why static cannot close it: r's ownership is a genuine runtime JOIN (owned copy on
the empty-loop path, borrow on the loop-ran path). The correct fix is a **runtime
owned-witness** (per-var flag: r owns iff its last executed assignment produced an
owned store) gating BOTH native free-sites — the inline-ncc-reassign twin of the
`OpBindOrCopy` witness that already covers the CALL-result bind (`g = pick(t,i)`).
Repro: `probes/d-own-2/loop-copy-view-native-divergence.loft`.
