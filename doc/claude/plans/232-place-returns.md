<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 232 — A function returns a place: `-> &T` accessors

Tracker: [@PLN232](https://github.com/loft-lang/plans/issues/232) (closes loft#2659 and `D-call-32`).

> **Status — DONE 2026-11-28.**
> Reference for what shipped lives in [LOFT_DECLARATIONS.md](../LOFT_DECLARATIONS.md) and
> [LOFT.md](../LOFT.md); the formal rules are `(B-RefType-Return)` in
> [formal/binding.md](../formal/binding.md) and `(F-Ret)` in [formal/calls.md](../formal/calls.md);
> the ownership amendment is [@C77](../DESIGN_DECISIONS_OWNERSHIP.md); the diagnostic update is
> `lost-write` in [DIAGNOSTICS.md](../DIAGNOSTICS.md).  This file is a closure record only.

## Headline invariant

A call of a `-> &T` (or `-> const &T`) function IS the place its body returns, each parameter
replaced by its argument — in every position a place can stand (`first(s).hp = 3`,
`hurt(first(s), 10)`, `e = first(s)`, `e = &first(s)`).  Only shapes that would violate place
soundness are refused: returning a local, literal or fresh temporary, returning places rooted in
two different parameters' lifetimes, returning a mutable `&T` from a value-`const` parameter,
declaring a generator `-> &iterator<T>`, or returning conflicting `&text` kinds.

## What shipped, per phase

| Phase | Step | Outcome | Guard |
|---|---|---|---|
| **0** | Silent lost writes on `-> T` call results | `first(s).hp = 3`, `s.first().hp -= 10` and `hurt(s.first(), 10)` emit `lost-write` even without `-> &T` | `loft-suite rules/calls/F-Ret/a-write-into-a-call-result-is-lost-and-says-so.loft` |
| **A** | Spec cells under `@FR-B-RefType-Return` | 13 cells (4 positive cross-mode guards + 9 refusal guards) covering records, tuples, generics, interfaces, fn-refs, scalars, text, lints and every refusal | `loft-suite/rules/binding/B-RefType-Return/*.loft` |
| **B1** | Parse `-> &T` / `-> const &T` and check callee | `parse_return_type` accepts return `&` on functions, methods, interfaces, lambdas and `fn(…) -> &T` types while keeping generator, field, element and tuple-member `&` refused; `finalize_place_return_body` enforces one-parameter lifetime and `const` propagation | `place-return-*-refused.loft`, `tests/scripts/a-link-type-outside-a-variable-is-refused.loft` |
| **B2** | Call in place positions | Field writes, compound assignments, collection indexing, slice/append mutations and passing to `&` callees mutate the underlying element on interpreter and C backends | `place-return-records.loft` |
| **B3** | Binds, reshape and disturb | `e = first(s)` binds as a view for records/collections (`FirstArm::Plain` when `!tp.depend().is_empty()`, materialising under `@FR-B-Disturb`) and `e = &first(s)` links with container-level `@FR-B-Ref-Reshape` enforcement | `place-return-records.loft`, `place-return-reshape-refused.loft` |
| **B4** | Lints and store records | `lost-write`, `double-move` and unused-`&`-parameter lints treat `-> &T` callees as place borrows; `lost-write` offers `-> &T` as a conditional fix naming its caller-wide view cost | `place-return-lints.loft`, `tests/e1_code_set.rs` `lost_write_offers_place_return_fix_on_value_accessor` |
| **B5** | Fn-ref types, generics and interfaces | `ConstParams` encodes `ret_ref` + `ret_const` (`ret_ref_param` on `Definition`); generic monomorphs, interface dispatch and `fn(S) -> &E` values preserve and check place returns | `place-return-generics-interfaces-fnrefs.loft` |
| **C** | Scalar and text places (`-> &integer`, `-> &text`) | `Type::RefVar` returns emit `OpReturn` of the 12-byte `DbRef` on the interpreter and `loft_ref` / `loft_str_ref` in C; `&text` return kind (`TextLinkKind`) is tracked per definition and checked across return sites | `place-return-scalars-and-text.loft`, `place-return-text-kind-refused.loft` |
| **D** | Self-hosted compiler port | `selfhost/src/` (`syntax`, `ir`, `el`, `el_types`, `el_definitions`, `el_expressions`, `el_ex_call`, `el_retpromo`, `lower`, `bgen`, `cgen_bind`) mirrors Phases B1–B5 and C; Gate E reports 0 diffs | `cargo test --test selfhost`, `selfhost/front/elab_gate.py` |

## Owner rulings (2026-11-27)

- **Q1 — Lift `(F-Ret)`'s `&T` return:** lift `-> &T` rather than dropping `(F-Ret)`'s carve-out.
- **Q2 — Plain bind `e = first(s)`:** follows the place — a view for records/collections (`@FR-B-View`) and a copy for scalars/text (`@FR-B-Copy`), with `e = &first(s)` linking in every case.
- **Q3 — Temporary root `first(mk())`:** follows the place — reading and passing on work, `&first(mk())` is refused, and `e = first(mk())` materialises its own value (`@FR-O-Proxy`).
- **Q4 — Branches:** accepted when every returned place lives in one parameter's lifetime (the caller pins the union of reached containers); refused when return paths span two parameters (`Deps [0, 1]`).
- **Q5 — Scalar and text places:** built in Phase C alongside records; conflicting `&text` kinds (stack `text` variable vs store field/element) are checked at compile time.
- **Q6 — Rule home:** `(B-RefType-Return)` in [formal/binding.md](../formal/binding.md), cited from `(F-Ret)` and `@C77`; `&` appears in the signature only, never in `return &place` (`@FR-B-Ref-AnnotationOnly`).
- **Q7 — Declarations:** named functions, methods, generics, interface methods and `fn(…) -> &T` function-reference types all participate; generators stay refused (`@FR-G-NoRefParam`).
- **Q8 — `lost-write` cure:** offered as a conditional fix naming its caller-wide effect (*"a plain bind of its result then views the element instead of copying it"*).

## Documentation shipped

- [LOFT_DECLARATIONS.md](../LOFT_DECLARATIONS.md), [LOFT.md](../LOFT.md), [tests/docs/08-struct.loft](../../../tests/docs/08-struct.loft) (regenerated via `gendoc` + `build-playground-examples.loft`).
- [formal/binding.md](../formal/binding.md) `(B-RefType-Return)`, [formal/calls.md](../formal/calls.md) `(F-Ret)` and [formal/calls-history.md](../formal/calls-history.md) `D-call-32`.
- [DESIGN_DECISIONS_OWNERSHIP.md](../DESIGN_DECISIONS_OWNERSHIP.md) `@C77` and [DESIGN_DECISIONS-history.md](../DESIGN_DECISIONS-history.md).
- [DIAGNOSTICS.md](../DIAGNOSTICS.md) `lost-write` conditional fix, `.claude/skills/loft-write/SKILL.md`, `@F21` (`loft-lang/features#21` + generated shadow), and [CHANGELOG.md](../../../CHANGELOG.md).

## See also

- [formal/binding.md](../formal/binding.md) `(B-RefType-Return)`, `(B-RefType-OfVar)`, `(B-View)`, `(B-Ref-Reshape)`, `(B-Disturb)`.
- [formal/calls.md](../formal/calls.md) `(F-Ret)`; [@C77, @C79, @C86](../DESIGN_DECISIONS_OWNERSHIP.md).
- [@PLN231](https://github.com/loft-lang/plans/issues/231); [@PLN233](233-settled-return-protocols.md); loft#2659, loft#894.
