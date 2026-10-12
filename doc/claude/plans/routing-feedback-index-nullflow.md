<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Routing-consumer feedback — index / null-flow / `??` correctness

**Source:** the `routing` consumer agent (dogfood loop; `../routing/docs/loft-feedback.md`,
"finding 1–3").  **Status: TRIAGE — intended changes recorded, NOT yet implemented.**
Each finding is reproduced on the current loft (both backends where relevant); the repros
are in the session scratchpad and inlined below.  This doc says *what we want to change*;
it does not change it.

Two of the three are genuine bugs (a lint false-negative and a `??` codegen fault); the
third is intentional behaviour that is under-documented.  Severity order below.

---

## Finding 2 — struct constant as a `??` fallback miscompiles (HIGH: silent wrong value + build break)

**Repro** (both backends wrong):
```loft
struct Point { x: integer, y: integer }
POINT_NONE = Point { x: 0 - 1, y: 0 - 1 };   // a struct file-scope constant
fn g(p: Point?) -> Point { p ?? POINT_NONE }
fn main() { println("{g(null).x}"); }         // want -1
```
- `--interpret`: prints `null` — the `??` fallback did NOT materialise the struct constant
  (should be `-1`).  Silent WRONG value, and the interpreter suite goes green.
- `--native`: `error[E0308]: mismatched types` — codegen emits ill-typed Rust; `make test`
  fails (the consumer hit **17 rustc errors** from this).
- Workaround the consumer shipped: a **zero-arg fn** `point_none()` returning the struct
  works on both backends.  So the fault is specific to a struct *constant* in the `??`
  fallback position.

**Root cause (CONFIRMED, broader than `??`):** a struct file-scope constant
(`P = Point { … }`) is not just broken in `??` — it is broken in *every* position.  Its
stored value is the constructor's field-writes with **no allocated record** (the IR shows
`else { INSERT OpSetInt(null, …) OpSetInt(null, …) }` — dest = `null`).  A plain
`a = POINT_NONE` panics codegen (`Incorrect var a[65535]`); `?? POINT_NONE` reads null
(interp) / E0308 (native).  Scalars inline fine; scalar-element **vector** constants ride
the `OpConstRef` const-store path (`objects.rs:1232`, built by `build_const_vectors` in
`compile.rs:111`) — a heap record has neither.  A zero-arg fn works because it materialises
the record through the ordinary return-buffer call path.

**Change — two phases:**
- **SHIPPED (safe immediate fix):** *reject* a struct-valued constant at
  `parse_constant` (`definitions.rs`) with a diagnostic that points at the working
  zero-arg-fn idiom — turning a silent `null` / `E0308` / codegen-panic into a clear
  compile error on both backends.  No stdlib/lib uses struct constants, so nothing breaks.
  Guard: `tests/parse_errors.rs::struct_valued_constant_rejected`.
- **FOLLOW-UP (full support, own plan):** route struct (`Reference`) constants through the
  const-store like vectors — a `build_const_records` sibling that materialises the record
  from the constructor's field literals into `CONST_STORE`, plus emit `OpConstRef` for a
  `Reference` constant at `objects.rs:1232` (`OpConstRef` + `OpCopyRecord` already
  deep-copy a record).  This is the loft-codegen "prove the working bytecode on both
  backends" gate — the vector-constant path is the working reference to mirror.  Likely
  extends to enum-payload constants (the same materialisation gap).

**Why it matters:** a sentinel constant is the obvious idiom; it passes the interpreter and
then breaks the native build — the worst failure shape (green locally, red on `make test`).

**Steps (small, safe, each proven on BOTH backends before the next):**

| # | Step | Verify |
|---|---|---|
| 0 | **Boundary probe (throwaway).** `x ?? STRUCT_CONST` beside its working twin `x ?? struct_fn()`, × {present, null} × {interp, native}.  Hand-compute every cell (present→field; null→the fallback's field, e.g. `-1`).  Snapshot CURRENT (interp `null`, native E0308) beside it — this is the spec. | spec recorded |
| 1 | **Prove the working bytecode first** (loft-codegen gate).  `loft introspect` the fn-fallback form (WORKS) and the const-fallback form (BROKEN) on both backends; the IR + native-Rust diff at the `??` fallback operand IS the fix target.  No compiler edit yet. | diff captured, pair saved |
| 2 | **Carry the missing fact, then emit.**  Whatever the fn form carries that the const form drops (record materialisation / owned-vs-value delivery — read it off step 1, do NOT re-derive in codegen), make the const-fallback path carry it.  Smallest change at the one emit site. | struct-const form == fn form, IR + native Rust |
| 3 | **Validate + graduate.**  Value == the constant's field on interp AND native; native compiles clean; no leak (`LOFT_STORES=warn` / `LOFT_NATIVE_LEAK_CHECK`).  Regression: a `cross_mode!` test asserting `x ?? STRUCT_CONST` value-parity + a native-compile check. | `make ci` |

---

## Finding 1 — the index-fit lint's loop-var carry ignores the range's source vector (MEDIUM: lint false-negative)

> **SUPERSEDED by @PLN102 #583's strict-index lint (2026-07-16).**  The tuxedo fix
> (`6ba336ec`) typed the mismatched `w[i]` NULLABLE + a same-base-struct SoA carve-out.
> Mainline #583 chose the opposite, canonical design — `w[i]` stays NON-NULL (reads
> @C80-null on overrun) with an opt-in `LOFT_LINT_STRICT_INDEX` audit that fires even on
> same-base fields (see `102-stability-contract/strict-index-corpus.loft` FIRE 3).  On the
> `tuxedo-work`↔`origin/main` merge the tuxedo f1 fix was REVERTED in favour of #583 (the
> two typings are incompatible).  The finding's concern is now addressed by that lint.

**Repro** (`sound.loft` warns 0×, `sound2.loft` warns 1×; both print `s=null`):
```loft
// range derived from v (len 3) but the index goes into w (len 1)
for i in 0..len(v) { s = s + take(w[i]); }      // FOR — emits NO warning (bug)
while i < len(v) { s = s + take(w[i]); i += 1; } // WHILE — emits the warning (correct)
```
The `@PLN25` null-flow analysis marks a scalar `v[i]` read non-null (drops the `τ?`) when
the index is "provably fit".  For a `for`-range index it trusts the loop variable
UNCONDITIONALLY — it never checks that the loop's range source is the SAME vector being
indexed — so `w[i]` inside `for i in 0..len(v)` is wrongly treated as in-bounds, the read
loses its `?`, no discharge is required, and an OOB access silently returns null.  The
`while` form gets no carry, so `w[i]` stays `τ?` and the lint fires.  **Perverse
consequence:** rewriting `while` → `for` (the cheapest way to silence the lint) is exactly
the refactor that *hides* the bug.

**Root cause (located):** `Parser::index_provably_fit` — `src/parser/fields.rs:774`.
```rust
Value::Var(v) => {
    if self.vars.is_active_loop_var(*v) {
        return true;          // <-- BUG: no check that the loop's range == len(vec)
    }
    // the `if idx < len(vec)` guard path DOES pair (idx, vec) via vec_key + index_bounded:
    vec_key(vec, &self.data).is_some_and(|vk|
        self.index_bounded.iter().any(|(iv, ik)| *iv == *v && *ik == vk))
}
```
The `if idx < len(vec)` guard path already does the right thing — it pairs the index var
with the indexed vector's `VecKey` (`self.index_bounded: Vec<(u16, VecKey)>`, set in
`control.rs:2531`).  The loop-var path needs the **same pairing**.

**Intended change:** narrow the `is_active_loop_var` arm to require the loop var's
range-source vector to equal `vec`.  Concretely: when `parse_for` registers a range loop
whose bound is `len(V)` (`for i in 0..len(V)`), record the pair `(loop_var, VecKey(V))`
(a per-active-loop-var map, mirroring `index_bounded`); then in `index_provably_fit` return
`true` only when that recorded source matches `vec_key(vec)`.  A `for i in 0..N` (constant
or non-`len` bound) proves nothing for any specific vector → stays `τ?`.  Loop vars are the
common correct idiom (`for i in 0..len(v) { v[i] }`), so this must keep the true-positive
suppression while killing the cross-vector false-negative.

**Steps (small, safe, each verifiable):**

| # | Step | Verify |
|---|---|---|
| 0 | **Boundary matrix (throwaway).**  `for i in 0..len(v) { v[i] }` (fit), `for i in 0..len(v) { w[i] }` (NOT fit), the two `while` twins, and `if i < len(w) { w[i] }` inside the `v`-ranged loop (re-fits).  Hand-write the warn / no-warn verdict per cell; snapshot CURRENT (the `w[i]` for-cell wrongly silent) beside it. | spec recorded |
| 1 | **Record the range source (INERT).**  In `parse_for`, when the range bound is `len(V)`, record `(loop_var, VecKey(V))` into a NEW per-active-loop-var map (mirror `index_bounded`; push on entry, pop on exit).  Nothing reads it yet. | parses; map populated (white-box); suite green (inert) |
| 2 | **Gate the carry on the source (load-bearing).**  In `index_provably_fit` (`fields.rs:774`) replace `is_active_loop_var(*v) → true` with: fit only if the loop var's recorded source `VecKey` equals `vec_key(vec)`.  A `for i in 0..N` (non-`len` bound) records nothing → never fits.  Flips `w[i]`→`τ?`, keeps `v[i]` fit. | matrix cells match; `v[i]` still fit (no new false-positives); both backends |
| 3 | **Graduate.**  The matrix's warn/no-warn cells into a warning-count guard (the verdict is parse-time; `parse_errors.rs` / a `#warn`-annotated script fits).  Keep the `while`/`if`-guard twins as the true-positive + re-fit controls. | `make ci` |

Contained: step 1 only adds an unread map (inert); step 2 only *narrows* an
over-broad `true` (can add `τ?`, i.e. more warnings — never removes a real one); the
true-positive `for i in 0..len(v) { v[i] }` idiom must stay wart-free (guard it in step 0).

---

## Finding 3 — scalar negative indexing counts from the end, documented only for slices (DOC gap + footgun)

**Behaviour** (confirmed, and IDENTICAL on interpreter + `--native`):

| `v = [10,20,30]` | `v[-1]` | `v[-3]` | `v[-4]` | `v[3]` |
|---|---|---|---|---|
| result | `30` | `10` | `null` | `null` |

So scalar indexing is: `i ∈ [0,len)` → element · `i ≥ len` → null · `i ∈ [-len,-1]` →
element counted from the end · `i < -len` → null.  This is **intentional** — the
`index_provably_fit` comment calls `v[-1]` "the Python-style last-element idiom" — and it
mirrors the negative *slice* bounds documented at LOFT.md:1269 (@P384).

**The gap:** LOFT.md:1251 documents scalar indexing as only `// index (null if out of
bounds)`; the negative-from-the-end half is written down **only for slices**.  A reader
(correctly) treats `v[i]` as "null when out of range" and collapses `if ei >= 0 { e =
edges[ei]; … }` into a bare null-guard — but `edges[-1]` is the LAST edge, so a stray
negative index (e.g. a "not found" `-1`) silently attributes a real element instead of the
intended miss.  This quietly **defeats the `if v[i] { … }` guard the finding-1 lint itself
recommends**: high OOB nulls, negative OOB wraps, and only one is written down.

**Intended change (docs only — the behaviour is intentional and backend-consistent, so
NOTHING in the compiler changes; we are writing down what already happens):**

1. **LOFT.md § indexing (line ~1251).**  Replace the terse `v[i]  // index (null if out of
   bounds)` with the full scalar rule, and cross-link the slice rule so scalar + slice
   negative-indexing read as one model.  Proposed wording:
   > `v[i]` — element at `i`.  `i ∈ [0, len)` → the element; `i ≥ len` → `null`.  A
   > **negative** `i ∈ [-len, -1]` counts **from the end** (`v[-1]` is the last element,
   > `v[-len]` the first) — the same rule as negative slice bounds (@P384); `i < -len` →
   > `null`.  So a *computed* index that can go negative does NOT null-guard: `v[-1]`
   > returns a real element, not `null`.  Guard with `if i >= 0` before the read (or only
   > `?? d` after a `>= 0` check) when `i` may be a "not-found" `-1` / an underflow.
2. **loft-write skill** (`.claude/skills/loft-write/…`, the "known bugs / gotchas" +
   error→fix reference authors read before writing `.loft`).  Add a gotcha row:
   > **`v[i]` with a possibly-negative index does not null-guard.**  `v[-1]` is the last
   > element (Python-style), NOT `null`.  `if v[i] { … }` / `v[i] ?? d` only catch
   > `i ≥ len`, not a negative `i` (a `-1` sentinel, a `a - b` underflow).  Fix: test
   > `if i >= 0` first.  (Only slices advertised negative-from-end before — @P384; scalar
   > indexing does it too.)
3. **INCONSISTENCIES.md / CAVEATS.md.**  One row: "scalar negative index counts from the
   end (documented only for slices pre-2026-07); high OOB nulls, negative-in-range wraps —
   asymmetric, defeats a naive `if v[i]` guard."
- **Consider (separate, lower priority):** a lint that flags a `v[i]` whose index is a
  subtraction / a known-can-be-`-1` value sitting behind an `if v[i]`-style null guard — the
  guard is a lie for negative `i`.  The doc fixes above close the correctness gap on their
  own; the lint is a nicety.

**Steps (small, safe — pure docs, no compiler change, no matrix needed):**

| # | Step | Verify |
|---|---|---|
| 1 | LOFT.md:1251 indexing table (wording above). | `make gendoc` clean; reads right |
| 2 | loft-write skill gotcha row (wording above). | skill still lints/loads |
| 3 | INCONSISTENCIES.md / CAVEATS.md one-line entry. | doc-hygiene gate |
| 4 | *(optional, separate)* the negative-index-behind-null-guard lint. | its own matrix |

Do steps 1–3 together (they're the same edit in three docs); step 4 is a follow-up.

**SHIPPED (steps 1–3, docs only — no behaviour change).**  LOFT.md § indexing carries
the full `i≥len`→null / `i∈[-len,-1]`→from-end / `i<-len`→null table + the null-guard
footgun, cross-linked to the slice rule (@P384); the loft-write skill has the
"`v[i]` with a possibly-negative index does NOT null-guard — test `if i >= 0` first"
gotcha; CAVEATS.md § Accepted trade-offs records the asymmetry.  Step 4 (a lint on a
subtraction/`-1`-able index behind an `if v[i]` guard) remains an optional follow-up.

---

## Routing / sequencing

- Finding 2 first (correctness + build break, both backends) — loft-codegen gate.
- Finding 1 next (lint precision; contained parser change mirroring `index_bounded`).
- Finding 3 is a doc fix (+ an optional lint) — cheap, do alongside.

Each can graduate to its own `loft-lang/plans` issue; this doc is the triage that feeds them.
