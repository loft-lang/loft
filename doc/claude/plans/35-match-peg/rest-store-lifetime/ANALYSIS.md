<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# `..rest` store-lifetime — situation analysis (probes + oracle) + the fixes

@PLN85-style attack: 29 probes across the axes, run on BOTH backends against the
interpreter leak-check ground truth (`MATRIX.txt`), plus a reporting-only OBSERVER
oracle (`LOFT_REST_ORACLE`, `ORACLE.txt`) that predicts the leak and names the exact
IR fact.

> **STATUS — both leak sub-classes FIXED + a third class (subject-corruption on REUSE)
> FIXED (2026-07-11).** The corpus is leak-clean and value-correct on both backends
> (`MATRIX.txt`, 0 mismatches, now incl. the **reuse axis** `a-*-reuse`); the oracle
> predicts 0 leaks (`ORACLE.txt`, 33/33). Fixes: in `scopes.rs::insert_free` — **A**
> extends the hoist to `Reference`/`Enum(_,true)`/`Vector` returns; **B** pushes the
> sibling store free into the `If` arms for a non-hoistable `&text` return
> (`push_frees_into_arms`). In `parser/expressions.rs::materialize_iterator` — **C** (the
> slice-materialise divergence, § below). The mapping below is the mechanism record.

## Class C — struct-enum subject-corruption on REUSE (`materialize_iterator`)

The corpus originally had NO **reuse axis** — every probe called the `..rest` fn once, so
it never re-read a subject after materialising a slice of it. Adding `a-*-reuse` (call the
fn TWICE on the SAME subject) exposed a **pre-existing, interpret-only miscompile of
`v[lo..hi]` itself** (confirmed on `main`; nothing to do with `..rest`, which just reuses
the proven `#Slice materialise` path):

> `#Slice materialise` read each element into an intermediate `comp_var = for_var`. The
> read (`for_var = subject[i]`) carries the borrow-dep `["subj"]`, but the `= for_var`
> assignment DROPPED it, so `comp_var` typed as a plain owned `ref(T)` looked owned to
> the free-analysis. It emitted `OpFreeRef(comp_var)` — a harmless no-op for an INLINE
> struct (`Parts::Struct`, raw `OpGetVectorNullable`), but for a STRUCT-ENUM (`is_base`
> via `Parts::Enum`, read via `OpGetVectorNullable`+`OpGetField`, which DEREFERENCES the
> record pointer) it freed the subject's OWN record. Single-slice hid it (corruption is
> only visible on a LATER read of the same range); slicing the same subject twice gave a
> corrupt (empty/short) second slice on the interpreter. **Native never corrupted** — it
> keeps `comp_var` as an owned copy (redundant copy+free), so the two backends diverged on
> the same IR (interp view vs native copy).

**Fix:** the borrow-dep IS the fact (loft-codegen step 2). `comp_var` was a redundant
intermediate — the element is materialised ONCE by `set_field` deep-copying into the fresh
`elm_var` record. Removing `comp_var` and deep-copying the **dep-carrying `for_var`**
directly keeps the borrow-dep, so the free-analysis never frees it (interp: no corruption)
and native emits a borrow, not an owned copy (no leak). One change fixes BOTH backends for
scalar/text/struct/large-struct/struct-enum (matrix reuse rows all `ok ok | ok ok`).
Guards: `tests/scripts/35d-slice-enum-reuse.loft` (direct `v[lo..hi]`, fails on `main`) +
the reuse asserts in `a-rest-is-a-fresh-capture-wherever-it-stands.loft`.

**Separate pre-existing crash the axis also surfaced:** `vector<vector<T>>` slicing
(`v[lo..hi]`) SIGSEGVs on `main` (both `..rest` and the direct slice) — a distinct nested-
vector-slice bug, NOT this corruption (file separately).

## Headline

The `..rest` leak is **NOT a confinement problem** (my earlier hypothesis, now
falsified). It is a **free-before-allocation ordering** problem:

> A `..rest` arm materialises a FRESH vector store (`__vdb`) INSIDE the match arm. The
> store is function-scoped, and its scope-exit `OpFreeRef(__vdb)` is placed **before**
> the `return` whose expression allocates it (`OpDatabase(__vdb)`) — so the free hits
> the null store (no-op) and the real allocation is never freed → **leak**.

The single fact that discriminates leak from clean is the ORACLE's
`free/alloc order` — **`FREE-before-ALLOC` ⇔ leak**, on all 29 probes (0 mismatches).

## Why the free lands before the allocation

The free-analysis (`scopes.rs::insert_free`) HOISTS a value-returning block's tail
into a `__ret` temp before the frees (`__ret = if{…alloc…}; free; return __ret`), so
the store frees after its allocation → clean. It does **not** hoist when the block's
result type is not a value type — then it emits `free; return if{…alloc…}` and the
free precedes the allocation. Two sub-classes, both caught by the oracle:

| Sub-class | Return type | Oracle `ret=` | Fix lever |
|---|---|---|---|
| **A** | `Reference` / struct / **enum** (`ref(LetS)`) | `NO-hoist` | extend the hoist to Reference/Enum/Vector/Struct returns — a DbRef hoist is native-safe (unlike `&text`) |
| **B** | **`&text`** promoted out-buffer (the captured field returned directly) | outer block `hoists`, but the inner `vector_match` block result is `&text` (`RefVar`), which is deliberately NOT hoisted (native `Str::new(&local)` dangle — proven) | place the sibling store's free INSIDE the allocating arm (after materialisation), not at the outer scope; do NOT hoist the `&text` |

## The leak class, precisely (from `MATRIX.txt`)

**LEAKS** — a variant-sub-pattern head captures a FIELD that ESCAPES into the return,
`..rest` present, and the return is a non-value type:
`b-varhead-escape-enum-used`, `b-varhead-escape-struct-used`,
`b-varhead-escape-text-unused`, `d-namepat-plus-rest-escape`.

**CLEAN** — everything else: return rest / `len(rest)` / pass rest to a fn / rest
unused (all element types, all VALUE returns hoist); a name:pat ELEMENT capture that
escapes (`c-namepat-escape` — the element read is not a promoted `&text`); a FORMATTED
text return (`"{word}{len(rest)}"` is an owned new text, hoists, not the `&text`
buffer).

## Why my earlier "confinement" fix was wrong

The oracle shows every `..rest` store is `REJECT(ambiguous)` — dep-backed by BOTH the
user `rest` local AND the per-element `_elm` temp — so `store_confinement` never
confines it, in leaking AND clean cases alike. Confinement is orthogonal to the leak.
Excluding `_`-temps from the ambiguity gate (the attempted fix) CONFINES the store in
ALL cases, which over-frees a still-used `rest` (regressed `p1` = `b-varhead-escape-
enum-used`: `nt(rest)` read empty → wrong value, worse than a leak). Confirmed: the
lever is the RETURN-hoist, not confinement.

## Separate pre-existing bug the sweep isolated

`e-empty-heap-vec-match`: an empty heap-vector `[]` as a match `_` arm fails NATIVE
codegen (`expected DbRef, found ()`); the scalar control (`e-empty-scalar-vec-match`)
is clean. No `..rest` involved — a distinct native-codegen bug (file separately).

## Fix direction (next phase — NOT done here)

1. **Sub-class A** — in `insert_free`, extend the hoist gate so a `Reference` / `Enum`
   / `Vector` / struct block result hoists like a value (a DbRef `__ret` is
   native-safe). Verify: `b-varhead-escape-enum-used` + `-struct-used` flip to clean,
   the whole matrix stays green on both backends, full suite green.
2. **Sub-class B** — for a `&text` (RefVar-text) inner block that cannot be hoisted,
   move the sibling store free INTO the allocating arm (so it runs after
   `OpDatabase`). Verify: `b-varhead-escape-text-unused` + `d-namepat-plus-rest-escape`
   flip to clean.
3. Re-run `run_matrix.sh` + `run_oracle.sh` — the oracle must report
   `alloc-before-free → clean` for all, and predict 0 leaks.

## Reproduce

    doc/claude/plans/35-match-peg/rest-store-lifetime/gen_probes.py   # regenerate probes
    .../run_matrix.sh     # ground-truth value×leak matrix, both backends  → MATRIX.txt
    .../run_oracle.sh     # oracle leak-prediction vs ground truth (29/29) → ORACLE.txt

The oracle (`LOFT_REST_ORACLE=1`, needs `LOFT_NO_CACHE=1` to bypass the compiled-program
cache) lives in `src/scopes.rs::rest_store_oracle` — an OBSERVER (no IR change), the
`@PLN94 ownership_cfg::oracle` pattern.
