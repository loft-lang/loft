<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN85 — generic tuple-of-text return leak (p329/p330), the fix

The final 5 suite leakers. `fn pair<T: Printable>(x: T) -> (text, text)` returns a
BARE-VALUE tuple whose text element is an owned `__ret_text_N` String at a stack slot;
on the interpreter it orphans (native drops it via RAII). The non-generic twin does
NOT leak. This doc pins the invariant and the fix before the code.

## The construction, read off the proven-vs-broken diff

Captured both `loft introspect` IRs (design-protocol "capture the sibling and diff"):

PROVEN — non-generic `pair_ng(x: P) -> (text, text)`, leak=0:
```
fn n_pair_ng(x:P, __retbuf:__tuple<text,text>) -> __tuple<text,text> {
  {#synthetic_tuple_return
    OpDatabase(__ref_1, 65);                 // caller-owned heap __tuple struct
    OpSetText(__ref_1, 0, t_1P_to_text(x));  // COPY element 0 in
    OpSetText(__ref_1, 4, "m");              // COPY element 1 in
    return __ref_1;                          // caller frees the ref normally
  }
}
```
BROKEN — generic `pair<T>(x) -> (text, text)`, leak=1:
```
fn t_1P_pair(x:P) -> (text, text) {
  __ret_text_1: text = t_1P_to_text(x);   // owned String hoist (skip_free)
  return (__ret_text_1, "m");             // BARE tuple: elem 0 orphans on interp
}
```

## The invariant

> A generic monomorph whose return tuple has a **lifetime-bearing element**
> (text / vector / reference) must deliver through the `__tuple<…>` synthetic-struct
> `__retbuf` ABI — SIGNATURE, BODY, and CALLER prediction all agreeing — exactly as
> its non-generic twin does via `needs_tuple_rewrite`. The bare-value tuple ABI
> cannot own a heap element across the return on the interpreter.

Same predicate (`has_lifetime_concern`) the non-generic uses — this is making the
generic MATCH the proven sibling, not a wider rule.

## Why the prior attempt "broke p329/p330/p240/plan17"

The `from_tv` gate (`tuple_return_rewrite`, `parser/mod.rs`) blocks the __tuple rewrite
for LITERAL-tuple generic returns. Its doc-comment records that rewriting them broke
those tests. Root cause of that breakage (inferred from the mechanism): the rewrite
was applied to the SIGNATURE / caller-prediction only, while the monomorph BODY — a
substituted COPY of the template body, whose `block_result` tuple→struct rewrite is
GATED on the signature already being `__tuple` and so never fired for the generic
template — kept emitting a bare `return (a, b)`. Signature `__tuple` + body bare-tuple
= the "T compiled as a Reference dummy → garbage (interp) / E0308 (native)" mismatch
the `tuple_return_rewrite` doc-comment describes. **The missing site was the body.**

## Re-assertion sites (design-protocol step 2) — N = 3, silent if omitted

1. **Caller return-type prediction** — `predict_generic_return_type` (pass 1).
2. **Monomorph signature** — `new_returned` in `try_generic_instantiation` (pass 2).
3. **Monomorph body** — the tail tuple → `synthetic_tuple_return` rewrite.

Sites 1+2 already route through ONE function (`tuple_return_rewrite`) → collapsing the
gate there fixes both at once (chokepoint, pass-stable: both passes key on the same
concrete return type). Site 3 is the one the prior attempt missed; it gets its own
monomorph pass mirroring `promote_monomorph_text_return`. So the fix is:

- **`tuple_return_rewrite`**: also rewrite a `from_tv=false` literal tuple WHEN it
  `has_lifetime_concern` (leave pure-value literal tuples on their current bare ABI —
  minimize blast radius). Fixes sites 1 + 2.
- **`promote_monomorph_tuple_return(d_nr)`** (new, hooked beside
  `promote_monomorph_text_return` in `try_generic_instantiation`): if the monomorph's
  now-rewritten return is `Reference(__tuple<…>)` but its body tail is still a bare
  tuple, run `rewrite_tail_tuple_to_synthetic_struct` on the body. Fixes site 3.

## Failure paths to probe (design-protocol steps 3–4)

- **p329/p330** (`(text,text)`, `(text,integer)`): the target — must go leak=0 +
  correct, both backends.
- **p240 / plan17** (the tests the from_tv gate named): must stay correct — the
  regression guard. Capture their return shapes; if pure-value, the scoped
  `has_lifetime_concern` guard leaves them untouched.
- **pure-value generic tuple** `-> (integer, integer)`: must be UNCHANGED (still bare
  ABI) — proves the relaxation is scoped to lifetime elements.
- **`-> T` where T=tuple** (from_tv=true): unchanged behavior.
- **tuple with a `Function` element**: excluded (as `needs_tuple_rewrite` excludes it).
- **bound vs discarded vs returned** monomorph tuple: all must be leak=0.
- Cross-pass: the pass-1 prediction and pass-2 instantiation must produce the SAME
  return type (else "variable changes type" / arity mismatch).

## Gate

`loft introspect` on the fixed generic must emit the SAME `synthetic_tuple_return`
shape as the proven non-generic (both backends). p329/p330/p240/plan17 + the matrix:
correct + leak=0 on interpret AND native. Full suite green.

## VERIFIED FINDINGS (2026-07-10 probe run) — the fix is 4 sites, not 2

Built the probe matrix (`probes/generic-tuple-return/`, `run.sh`) and drove a partial
implementation. Results reshape the plan:

**Harness lesson first (design-protocol "prove the harness can fail").** The first
`run.sh` had a hand-counted `../` chain to the suppressions file that silently
resolved to a nonexistent path → LSan errored → EVERY probe reported a false
`leak=0`. Nearly shipped a "fixed" verdict on a blind oracle. `run.sh` now resolves
the repo root via `git rev-parse` and runs a liveness proof (the unsuppressed
ir_read baseline MUST report a leak) before trusting any `0`.

**True baseline (trustworthy oracle):** owned-call tuple elements
(`(x.to_text(), …)`) leak — probes 01/02/03/05/07/08 = leak 1; arg-borrow (04) and
pure-value (06) = 0. All correct on both backends.

**The partial fix reached byte-identical body but exposed the real gap.** Relaxing
the template guard for CONCRETE lifetime tuples (definitions.rs) made the p329
monomorph body `loft introspect`-IDENTICAL to the proven non-generic (two
`OpFreeText(__work_1)`, no INSERT wrapper). Yet it STILL leaked — because the leak is
NOT in the body. The caller diff is decisive:

```
PROVEN (leak=0):  r = n_pair_ng({obj}, __ref_2);   OpFreeRefIfDistinct(r, __ref_2); … OpFreeRef(__ref_2)
GENERIC (leak=1): r = t_1P_pair({obj});            OpFreeRef(r)
```

The proven callee takes a hidden `__retbuf: __tuple` parameter (NRVO — caller owns
the buffer). The generic monomorph has **no `__retbuf` parameter**: it allocates the
struct internally and returns by value, and that non-NRVO return path orphans a text.
The `ref_return` promotion that ADDS the `__retbuf` param is skipped for generic
templates (control.rs:1180) and never re-run on the monomorph.

**So the real re-assertion sites are FOUR** (each silent if missed):
1. Return type → `__tuple` (definitions.rs `needs_tuple_rewrite`, concrete-tuple relax).
2. Body tuple → `synthetic_tuple_return` (block_result — fires once the sig is `__tuple`).
3. **`ref_return` adds the hidden `__retbuf` param to the monomorph** ← the missing site.
4. Caller passes `__retbuf` + `OpFreeRefIfDistinct` (falls out of 3 via the normal path).

Plus a pass-timing hazard: the concrete-tuple detection MISFIRED on a type-var
template (`both<T> -> (T,T)`) at pass-2, giving it a `__tuple` signature with a bare
body → native E0308 (probe 04). The detection must be pass-stable (key on the
concrete instantiated return, not the template shape mid-resolution).

**Next step:** a `promote_monomorph_tuple_return` that runs BOTH the body rewrite AND
`ref_return` (add `__retbuf`) on the monomorph — the tuple analogue of
`promote_monomorph_text_return` — gated on a pass-stable concrete-tuple test. The
probe matrix + trustworthy oracle are now in place to validate it cell-by-cell.

> **2026-09-05 — the deferred half landed** (QUALITY-history.md B7t, D-call-13).  The RESOLUTION below
> covered a template whose return SHAPE is concrete; a shape that depends on `T` (`-> (T, T)`,
> `-> T`) "deferred to instantiation as before", and nothing at instantiation received it.  It
> does now: `tuple_return_rewrite` boxes the lifetime-bearing literal tuple for both passes and
> `promote_monomorph_tuple_return` rewrites the body — site 3 of the four counted below — while
> the `__retbuf` (site 4) is still not minted for an instance; the boxed record is a work-ref the
> caller adopts, which is the same value the concrete twin delivers.

## RESOLUTION — the reorder collapsed the sites (FIXED, suite green)

Instead of re-deriving the 4 promotion sites on each monomorph (additive, fragile),
the sites were collapsed onto the ONE existing non-generic promotion flow by NARROWING
its guards — `is_generic_template` → `return_shape_depends_on_type_var`. A generic
template whose return SHAPE is already concrete (`-> (text, text)`) now rides the same
`definitions.rs` signature finalization (`__tuple` rewrite at site 1 + `__retbuf` at
site 3) and `block_result` body path the non-generic uses; the monomorph inherits a
fully-promoted signature+body for free. A return that still depends on `T` (`-> (T,T)`)
defers to instantiation as before.

The five changes:
1. `mod.rs::return_shape_depends_on_type_var` — the pass-stable predicate (keyed on the
   template's own type var via `extract_type_var`, NOT `DefType::Generic` — the type
   PARAMETER is not a generic FUNCTION def; that mistake let `(T,T)` slip through).
2. `definitions.rs` site 1 (`__tuple` rewrite) + site 3 (`__retbuf` param) — guard
   narrowed to `!is_generic_template || !return_shape_depends_on_type_var`.
3. `control.rs:1180` — the block_result promotion guard, narrowed to also allow a
   generic template already returning a `__tuple<…>` (kept to `__tuple` ONLY: enabling
   the general text_return path for concrete `-> text` generics panics on a template
   whose var table isn't promotion-ready — `plan17_b`).
4. `scopes.rs` caller pairing — the `adopts_fresh_store` / `OpFreeRefIfDistinct` pairing
   fired only for `n_`-named callees; broadened to `t_` (generic monomorphs / methods),
   so the caller frees the aliased return with `OpFreeRefIfDistinct`, not a leaking
   `OpFreeRef`. This was the last residual and a LATENT gap for any `t_` method
   returning an owned ref/tuple.
5. `corpus.loft` — `f_tuple_ctor`/`f_tuple_pair`/`f_tuple_literals` now deliver via
   `__tuple` (no `TupleElement` text-return verdict); annotations updated.

Gate met: all 8 `probes/generic-tuple-return/` cells leak=0 + correct on interp AND
native; `g3_tuple_return` leak 1→0; p329/p330/p240/plan17 + the text_return corpus +
the FULL suite (2738) green. `loft introspect` of the fixed monomorph is byte-identical
to the proven non-generic.

## Status: FIXED (p329/p330 tuple class → 0). The separate `-> text` generic-monomorph
## return (`g1b`, the forward-ref/pre-pass class) is untouched by this and remains open.

## #549 follow-up — INLINE / DISCARD aggregate-return leak (bug 1, FIXED 2026-07-10)

The RESOLUTION above closed the BOUND case (`t = f(x)`; caller pairing broadened to
`t_` in `get_free_vars`). A sibling leak survived for the INLINE (`f(x).0`) and
DISCARDED (`f(x);`) cases of a generic monomorph returning ANY concrete aggregate —
pure-value tuple (`p240`), plain struct, or struct-enum. Those route through
`scopes::inline_struct_return`, whose first block (the `Reference` / struct-`Enum` /
`Function` arms) was gated on `n_` ONLY, so a `t_` monomorph's fresh `__retbuf` store
was never lifted-and-freed → orphaned on BOTH backends (`Database N not correctly
freed` under the DA gate).

Fix: extend that block's gate to a `t_` callee that carries a `__retbuf` param. The
`__retbuf` NRVO parameter is the reliable "delivers a fresh owned aggregate" signal a
monomorph keeps even though it LOSES its return dep during specialization — so it
lifts the fresh-aggregate returns (`p240`, `p549_*`) while leaving a borrowed-arg
return (`id<T>(x) -> T { x }`, no `__retbuf`, empty dep) unlifted (lifting it would
double-free the caller's arg — the `p549_generic_returns_arg_not_double_freed` guard).
Verified: the 10-cell boundary matrix + `p240` + the four `p549_*` guards, correct +
leak-free on interpret AND native, under the DA gate.

The **`text.rs:334` double-free** (bug 2 in #549 — a `(text, …)` return double-frees an
owned/call-produced text element; `p243`/`p329`/`p330`, DA-only) is a SEPARATE, older
bug — fixed next (below), NOT by the bug-1 lift change.

## #549 bug 2 — the explicit-`return` aggregate double-free (FIXED 2026-07-10)

Boundary (matrix, both backends): double-free ⟺ an explicit `return (…)` of an aggregate
LITERAL (tuple OR struct) containing an OWNED text element (call-produced / `+`-concat /
a variable holding one). A TAIL aggregate (no `return` keyword) and an all-literal
aggregate are clean; position / container / `(text,int)` don't matter.

Root cause (in `scopes::free_vars`, NOT the parser): an explicit `return (owned_text, …)`
at a body tail builds a `synthetic_tuple_return` Block and is processed by `free_vars`
TWICE — once by the `Value::Return` scan arm, once by `convert`'s `is_body_return` tail
sweep (the tail-no-`return` form has no Return node, so it runs once → clean). The first
pass runs `insert_free`, which appends the owned-element free + the return, making the
Block TERMINAL. The second pass hit the `Value::Block` arm — which precedes the
`expr_is_terminal` dedup — and re-ran `insert_free`, emitting a SECOND `OpFreeText` on the
same element (and a `return return`). The `expr_is_terminal` dedup already existed for an
`Insert`-ending-in-Return; a terminal *Block* just never reached it.

Fix: order the `expr_is_terminal` dedup BEFORE the `Value::Block` insert_free arm, so a
Block that already ends in a Return propagates as-is instead of being re-freed. The fixed
`return`-path IR is byte-identical to the clean tail path (one `OpFreeText`, one `return`).
Verified: 13-cell matrix + `p243`/`p329`/`p330` (DA gate 6 fails → 0) + three non-generic
`p549_bug2_*` guards, on interpret AND native; full `issues` DA gate 755/0; full suite green.

Two UNRELATED pre-existing DA-only failures surfaced while verifying (both red on
`origin/main`, both orthogonal to this fix, both OUT OF SCOPE): `156-plan52-chained-
coalesce` (a `??`-coalesce double-free, a DIFFERENT `text.rs:334` path — not a return) and
`audience_crystal/03-crystal-incr` (`mod.rs:4014`). Worth a follow-up.
