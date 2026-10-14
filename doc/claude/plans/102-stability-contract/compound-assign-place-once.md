<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN102 F2 — Compound assignment evaluates its place once (fix scope)

> **Status: IMPLEMENTED + VALIDATED + LANDED (2026-07-16); GAP FOUND + CLOSED (2026-07-17).**
> A code-eval sweep found the original hoist fired ONLY when the field offset was the literal
> `Int(0)` — so a compound write to a struct field at a NONZERO offset (`w[idx()].y += 5`) still
> emitted the index call twice and, with a divergent index, wrote one element's field from a value
> read at another (the exact @C92 corruption, both backends). A second facet: the hoist rewrote the
> place to a temp BEFORE `validate_write`, silently bypassing #584's value-const / const-field
> enforcement (`r.v[idx()] += 9` on a `const` field was accepted). Both closed: the hoist now fires
> for ANY offset (rebuilding `OpGet(elem_ref, offset)` on a once-evaluated element ref for the
> nonzero case), and it runs `validate_write` on the ORIGINAL place first (`parse_assign_op` gains a
> `skip_validate` flag to avoid a double-fire). Regression extended with a struct-field-at-offset
> case. Verified both backends.
> Owner ruling: evaluate the place
> exactly once. Decision record: [DESIGN_DECISIONS.md @C92](../../DESIGN_DECISIONS.md). Fix in
> `src/parser/expressions.rs` (`ir_has_user_call` + the `_place` hoist in `parse_assign`, now
> offset-general + const-check-preserving);
> regression `tests/scripts/pln102-f2-place-once.loft`. Validated per the loft-codegen method:
> the double-eval reproduces pre-fix on both backends (instrument-can-fail); post-fix the boundary
> matrix (count + value + divergence + nested + all five ops + Op-wrapped calls) passes on both
> backends; the no-user-call corpus is byte-identical (IR + bytecode + native Rust); no leak
> (`LOFT_STORES=warn` / `LOFT_NATIVE_LEAK_CHECK`); full suite green (only the 3 known heavy-serial
> flakes + the pre-existing viewer `content()`→`text?` consumer break, both outside F2's `op != "="`
> path). On `tuxedo-f2-impl-steps`, stacked on arc-D → PR #583; no PR. (The `formal-audit.md`
> reconciliation table once marked F2 "✅ single-eval" — that was **wrong**: it double-evaluated,
> re-verified 2026-07-14 on both backends, now genuinely fixed.)

## The behaviour (verified 2026-07-14, both backends)

A compound assignment desugars `place op= rhs` → `place = <read place> op rhs`, and the read and
the write each emit the place's **addressing sub-expressions**, so any dynamic one runs twice.

| Case | `idx()` calls | Note |
|---|---|---|
| `w[idx()] += 5` | **2** | index expression double-evaluated |
| `m[i()][j()] += 5` (nested) | **4** | each index level × read+write |
| `s.v += 5` (field on a plain var) | 0 dynamic | no addressing sub-expression — unaffected |
| `w[1] += 5` / `w[i] += 5` (const / var index) | idempotent | reads twice but same value — **behaviourally safe** |

The live hazard is not the wasted read but **divergence**: `w[next()] += 5` where `next()`
returns `1` then `2` **reads `w[1]` and writes `w[2]`** — lands on the wrong slot, no error. Plus
any place-side effect (`w[log()] += 5`) fires twice.

## The invariant (one rule)

> A compound assignment evaluates the **addressing sub-expressions of its place** — index
> expressions and container-producing calls — **exactly once**; the read and the write then
> operate on the **same** located slot.

Field/offset selection carries no evaluation (it is a static offset); only the *dynamic* parts
(an `[expr]` index, a call that produces the container) need hoisting. A constant/variable index
is already idempotent, so the lowering must be **byte-identical** there — the fix is a no-op for
every program that doesn't put a side effect in its place.

## The chokepoint

The compound desugar in `src/parser/expressions.rs` (`parse_assign` / `parse_assign_op`, the
`op != "="` path) forms `place = <place-read> op rhs` by reusing the parsed place `Value` (which
embeds the raw index/receiver sub-expressions) on both sides. The fix hoists those sub-expressions
to temps **at the point the place is parsed for a compound op**, so both the read and the write
reference the temp `Var`, not the original call.

Two viable shapes (pick at implementation, guided by `loft introspect`):

1. **Hoist during place parse.** When parsing a place for `op != "="`, replace each dynamic
   addressing sub-expression `e` with a fresh `__place_N` temp and prepend `Set(__place_N, e)`.
   The place `Value` then holds only `Var`s, so cloning it for the read re-evaluates nothing.
   Most localized; matches the existing hoisted-temp convention (`__ncc_*`, `__lift_*`).
2. **Hoist at the desugar.** Keep the place parse as-is but, before duplicating it, walk the
   place `Value`, extract non-idempotent sub-expressions (`Call`/`CallRef`/index args that are not
   `Var`/`Int`) into `Set`s, and substitute (the existing `substitute_value`, `expressions.rs:224`,
   is the substitution primitive).

Idempotence test for "needs hoisting": the sub-expression is **not** a bare `Var`, `Int`, or other
const — i.e. anything that could call a function or read mutable state. Bare `Var`/const are left
inline (byte-identical).

## Gate (loft-codegen method — both backends)

This is a behaviour-preserving change for the common case AND a behaviour fix for the side-effect
case, so it runs **both** codegen gates (see the loft-codegen skill § Mode B "surfaces a real
change"):

1. **Byte-identical corpus (the untouched paths).** `loft introspect` before/after on a corpus of
   const- and var-indexed compound assigns (`w[1] += 5`, `w[i] += 5`, `s.v += 5`, `+=`/`-=`/`*=`/
   `/=`/`%=`/`&=`/`|=`/`^=`/`<<=`/`>>=`, keyed-collection `s[k] += …`) → **empty diff** on interp
   IR + native Rust. Proves the no-side-effect majority is unchanged.
2. **Boundary matrix (the path we fix), value + count, both backends.** One probe per row of the
   table above, each asserting the **call count** (via a printing place-fn) AND the **stored
   value**: `w[idx()] += 5` → `idx` once, `w[1]==25`; nested `m[i()][j()] += 5` → per-level once;
   the **divergence proof** `w[next()] += 5` with `next()` returning `1` then `2` → after the fix
   both target `w[1]` (read and write agree), before the fix they diverge. Graduate to
   `tests/scripts/`.
3. **Full suite** on both backends + `LOFT_STORES=warn` / `LOFT_NATIVE_LEAK_CHECK` (a hoisted temp
   holding a heap container must not leak or double-free — the place temp is a borrowed view of an
   existing store, not an owned alloc; confirm it is not freed).

## Conversion set

Measured at implementation by the byte-identical corpus + full suite: any in-tree
`place-with-side-effect op= rhs` whose behaviour *changes* (a place fn that was called twice now
once). Expected ≈ **zero** — side effects in a place expression are rare and mostly unintentional;
the divergent case is a latent bug the fix removes, not a behaviour a program relies on. `log()`
the exact set if non-empty (no silent conversion).

## Scope / effort

**S.** One chokepoint, one hoist, gated by an idempotence test; no new op, no runtime change. Not
a null/ownership-substrate change. Sibling F4 (assignment place-vs-RHS *order*) is adjacent and
could be settled in the same pass, but is a distinct decision (order, not count) — keep separate
unless the owner rules F4 too.

## See also

- [DESIGN_DECISIONS.md @C92](../../DESIGN_DECISIONS.md) — the decision.
- [formal-audit.md](formal-audit.md) F2 / F4 — the audit rows.
- [COMPATIBILITY.md](../../COMPATIBILITY.md) § the error surface — "a would-be-error is first a
  rewrite-to-correct-function," the principle this fix instances.

## Implementation steps (each with its verification)

Grounded in the actual emitted IR (`loft introspect`, 2026-07-14). Current `w[idx()] += 5`:

```
OpSetInt( OpGetVector(w, 8, n_idx()), 0,                              // write target
          OpAddInt( OpGetInt(OpGetVector(w, 8, n_idx()), 0), 5 ) )    // read — n_idx() AGAIN
```

`n_idx()` is embedded in BOTH `OpGetVector` calls. Target after the fix:

```
__place_1 = n_idx();                                                  // once
OpSetInt( OpGetVector(w, 8, __place_1), 0,
          OpAddInt( OpGetInt(OpGetVector(w, 8, __place_1), 0), 5 ) )
```

**Step 0 — baseline instruments (before touching the compiler).**
- 0a. Byte-identical corpus `f2-untouched.loft`: one fn per path that must NOT change — `w[1] += 5`
  (const index), `w[i] += 5` (var index), `s.v += 5` (field), each op `+= -= *= /= %= &= |= ^= <<=
  >>=`, keyed `h[k] += v`; a `main` runs all. *Verify:* clean on both backends; capture
  `loft introspect f2-untouched.loft > before.txt`.
- 0b. Boundary matrix `f2-matrix.loft`: `w[idx()] += 5` (idx prints), nested `m[i()][j()] += 5`, and
  the divergence probe `w[next()] += 5` where `next()` returns 1 then 2. *Verify (CURRENT/broken):*
  idx 2×, nested 4×, divergence reads `w[1]` / writes `w[2]` — the recorded before-state.

**Step 1 — prove the target IR standalone (the spec).** The once-eval form is exactly what the
hand-written `p = idx(); w[p] += 5` already emits. *Verify:* `loft introspect` that source →
`n_idx()` bound once, both `OpGetVector` read `p`; runs correct on both backends. That is the
working bytecode proven beside the broken one.

**Step 2 — locate the duplication point.** In `parse_assign_op` (`src/parser/expressions.rs`, the
`op != "="` compound path), the place `to` (which embeds the `n_idx()` `Call`) is used for the write
target AND cloned into the read `OpGetInt(to.clone(), …)`. *Verify:* one env-gated `eprintln` at the
construction fires for `w[idx()] += 5` and shows `to` containing `n_idx()`.

**Step 3 — hoist non-idempotent addressing sub-expressions once.** Before the place is duplicated,
walk `to`; for each addressing arg (the index arg of `OpGetVector`, a container-producing
`OpGetField`/call) that is NOT a bare `Var`/`Int`/const, create `__place_N`, prepend
`Set(__place_N, expr)`, and `substitute_value` (`expressions.rs:224`) the arg → `Var(__place_N)` in
BOTH the write and read copies. Idempotent args (bare `Var`/`Int`) are left inline. *Verify:*
`introspect` on `w[idx()] += 5` matches the Step-1 target IR (one `__place_1 = n_idx()`, both
`OpGetVector` use it).

**Step 4 — byte-identical gate (untouched paths).** `loft introspect f2-untouched.loft > after.txt;
diff before.txt after.txt`. *Verify:* EMPTY diff — interp IR AND native Rust — for const/var index,
field, keyed. (Re-run after `cargo fmt`.)

**Step 5 — boundary matrix, value + count, both backends.** Re-run `f2-matrix.loft`. *Verify on
`--interpret` AND `--native`:* `w[idx()] += 5` → idx ONCE, `w[1]==25`; nested → each level once;
divergence `w[next()] += 5` → read and write hit the SAME slot (`w[1]`), value correct.
`LOFT_STORES=warn` / `LOFT_NATIVE_LEAK_CHECK` clean — a container-producing-call place binds a heap
VIEW to `__place_N`; confirm it is a borrow (not owned), so not freed / not double-freed.

**Step 6 — conversion set + full suite + graduate.** *Verify:* full `cargo nextest` green on both
backends; `log()` any in-tree program whose place-fn call count changed (expected ≈ 0 — a
side-effecting place is rare/unintentional). Graduate the matrix to `tests/scripts/pln102-f2-place-once.loft`.

**Done when:** Steps 4 (empty diff) AND 5 (matrix correct, both backends, leak-clean) both hold, and
the full suite is green. Effort **S** — one hoist at one chokepoint, no new op, no runtime change.
