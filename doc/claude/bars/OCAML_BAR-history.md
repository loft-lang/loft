<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# OCAML_BAR history

The measurements [OCAML_BAR.md](OCAML_BAR.md) no longer carries, oldest first. The working doc
holds the current Evaluation only.

## Evaluation — measured 2026-09-21

Every probe was run on both backends against `9f5cf6a96` (origin/main) plus the loft#1572 fix,
from a scratch directory; no probe file or harness is committed yet. **The two backends agreed on
every cell**, so each row below is one answer. Where the probe's spelling was wrong for a
capability loft already has, the row scores the capability in its real spelling and says so. The
document's own rule — *a probe that cannot parse is a FAIL* — would otherwise have recorded four
false FAILs (A7, B5, C8, F2).

| entry | measured | what answered |
|---|---|---|
| A1 map, two variables | FAIL | `<T, U>` does not parse — @PLN165 arc C |
| A2 fold | FAIL | same — arc C |
| A3 variable not in first param | FAIL | same — arc C |
| A4 generic struct | FAIL | `struct Pair<` does not parse — @PLN165 arc D |
| A5 generic recursive enum | FAIL | arc D, and see A5b |
| A5b recursive enum, monomorphic | FAIL | refused: *"Enum 'Expr' contains itself … use reference<Expr> to break the cycle"* — and a `reference<Expr>` field then refuses `&e` (**defect 3**, loft#1579). A `struct Box { e: Expr }` pointed at through `reference<Box>` builds and evaluates a tree on both backends |
| A6 user `Result` | FAIL | arcs C + D |
| A7 user-declared interface | **PASS** | the spelling is `fn area(self: Self) -> float` (INTERFACES.md); the probe's `fn area(self)` was the defect |
| A8 associated type | PARTIAL | the companion works inside a generic (`type Rows: Cursor`, `Self.Rows`, @PLN125 — PASS); naming it in the generic's OWN signature (`-> S.Rows`) does not parse, as INTERFACES.md documents |
| B1 closures in a vector | FAIL | named refusal: *"a capturing closure cannot be stored in a collection: a collection has one element layout and each capture set is its own record shape"* |
| B2 handler table | FAIL | B1, and `hash<(…)>` has no tuple element type |
| B3 two closures, one written scalar | FAIL | named refusal: *"mutated through a closure and captured by 2 closures"* — design-negotiable as the entry says |
| B4 local recursive function | FAIL | *"'fn' definitions must be at file scope"*; the self-referencing-lambda spelling is an **internal compiler error** (**defect 2**) |
| B5 capture a `&` parameter | **PASS as `pass`** | capturing and writing through a `&` parameter WORKS on both backends (`[1,10]` → `[2,11]`); the refusal the probe expects does not exist and 26-closures is stale (**doc bug 5**). The probe as written was also invalid (`bump_all(&xs)`, an untyped `\|i\|`) |
| B6 compose | FAIL | arc C |
| C1 nested constructor pattern | **PASS** | `W { i: A { v: 0 }, k } => …` over a NON-recursive nesting, both backends; the recursive shape is blocked on A5b only |
| C2 literal in field position | **PASS** | `Circle { r: 0.0 } => "point"` works (the reference page does not show it) |
| C3 field rename + as-binding | FAIL | `Rect { w: width, h }` → *"Unknown variable 'width'"*; `whole @ Rect {…}` → *"'whole' is not a variant"* |
| C4 tuple of variants | **PASS** | `(Playing { hp }, Damage { amount }) if hp <= amount => …` works on both backends; the probe failed only on its arm BODY `Playing { hp }` — field-init shorthand, which loft does not have (`Playing { hp: hp }`) |
| C5 or-pattern with bindings | FAIL | `Circle { r } \| Sphere { r }` does not parse |
| C6 exhaustiveness through nesting | PARTIAL | over a non-recursive nesting the hole IS refused, but named by its outer variant (*"not exhaustive — missing: W"*), not as `W { i: B }` |
| C7 scalar-match hole diagnosed | FAIL | silent, both backends |
| C8 head/tail pattern | PASS with `_` | slice patterns exist (`[]`, `[x, ..rest]`); `[]` + `[x, ..rest]` is not seen as total (*"a slice pattern can fail … add a '_ =>'"*). The probe's own `fn sum` collides with the reserved stdlib name |
| D1 generic return smuggles null | FAIL — a DECIDED edge | `formal/types.md` (N-Index) trusts a constant index *by contract* (C80, @PLN102 D1): `v[0]` reads `T` non-null and faults to null at run time. Moving this bar reopens that decision; it is not an unregistered hole |
| D2 empty-stub default | FAIL | silent `0.0`, both backends |
| E1, E2 structural sharing | BLOCKED | on A5b (no recursive enum can be built); `memory_used()` does not exist — `store_memory()`'s record count is the instrument the corpus uses |
| F1 missing combination named | PARTIAL | refused at compile time: *"no definition of `beats` takes (Hand, Hand) — declared: beats(Rock, Scissors), …"* — names the declared set, not the missing pair |
| F2 enum-level fallback | FAIL — **defect 1** | on a PLAIN enum every one of the nine combinations answers the fallback, silently; the same program over struct-enum variants dispatches correctly (`R>S`, `P>R`, `S>P`) |
| G1–G13 regression floor | covered | every cited page IS a test: `tests/docs/*.loft` generate the reference pages and run in `make ci`, so the floor is already gated |

**Score (capability, both backends):** A 1/8 + 1 partial · B 1/6 · C 4/8 + 1 partial · D 0/2 ·
E blocked · F 0/2 + 1 partial.

### Defects the measurement surfaced (filed)

1. **loft#1577 — multiple dispatch ignores per-value definitions on a plain enum** — `silent-wrong`. `fn beats(a: Rock, b: Scissors)` on `enum Hand { Rock, Paper, Scissors }` is accepted and never chosen: with an enum-level `beats(a: Hand, b: Hand)` all nine combinations take it; without one, the call is refused although definitions match the runtime values. Struct-enum variants are unaffected. Either the declaration is refused or the dispatcher reads the value.
2. **loft#1578 — a self-referencing lambda is an ICE** — `go = fn(acc: integer, i: integer) -> integer { … go(…) }` inside a function: *"var_pos underflow in fn 'n___lambda_0': variable 'go' … has no assigned slot"*.
3. **loft#1579 — the cycle refusal's own cure does not work.** The real boundary is wider than recursion: ANY struct or variant field typed `reference<E>` over an ENUM refuses `&x` (*"Cannot assign ref(Shape)["c"] to field SH.s of type ref(Shape)["??"]"*), while a reference to a struct works everywhere and a LOCAL `reference<E>` works. So no recursive struct-enum is buildable the way the compiler suggests, which blocks A5, the recursive forms of C1 and C6, and tier E; the struct-wrapper above is the workaround.

Two more surfaced while correcting the reference pages against these results: **loft#1580** —
`==` on a `value struct` compares identity, not content (DESIGN_DECISIONS C91); **loft#1581** — a
concrete `!=` ignores a user-defined `OpEq`, so `a == b` and `a != b` are both true. Both
`silent-wrong`.

### The document's own claims, corrected

- **Doc bugs 1 and 2 held and are fixed** (`stdlib-interfaces.html` rendered only its heading because the renderer skipped every interface; `09-enum`'s `opposite()` is now a `match`). **3 changed**: D1 is a decided edge, and `25-generics` already states the behaviour and both remedies accurately, so it needs no change. **4 is fixed** ("Match inside an arm"), and `29-match` now also shows the patterns C1, C2 and C4 measured working. **5 was new and is fixed**: `26-closures` said *"A '&' parameter cannot be captured at all, in any shape"*; it now documents (L-CapRef).
- **There is no `loop` keyword** (LUA_BAR's erratum, measured): A8's probe uses `loop { … }`, so rewrite it with `while true` — a `while` GENERATOR is lazy on `--native` when its one `yield` is on the loop body's straight line (loft#1586; COROUTINE.md CL-9).
- **The expectation line cannot be `# bar: …`.** `#` opens a loft annotation (`#rust`, `#cwd`), not a comment. Use `// @BAR: pass` / `// @BAR: refuse "…"` / `// @BAR: measure …`, or reuse the corpus's `@EXPECT_ERROR:` and `@EXPECT_WARNING:` (C7's "a warning counts" is exactly `@EXPECT_WARNING`).
- **A parse failure is a FAIL only when no current spelling expresses the capability.** Rewrite a probe into the existing spelling first (A7, B5, C8, F2 above) — the "syntax is a placeholder" rule cuts both ways.
- **WONTFIX belongs in DESIGN_DECISIONS.md**, the declined-features register. COMPATIBILITY.md is the breaking-change policy; it is the right home only for the separate decision to make a tier a promise.
- **Tier A's order is @PLN165's.** Arc C (several variables, a variable in any parameter — A1–A3, B6), arc D (generic structs and enums — A4–A6), arc E (`map`/`reduce` as library generics). Its STEPS.md already sequences them after the C110 → C126 revision; this file should track that plan, not set a second order.
- **Tier G needs no lifted copies.** Point each row at its `tests/docs/*.loft` file; a copy would be a second home for the same example.

## Per-entry statuses carried by OCAML_BAR.md until 2026-10-05

Each entry's line from the documentation pass and from the first measurement, as the entry held them.

### A1_map_two_type_vars

Measured 2026-09-21: FAIL — `<T, U>` does not parse; @PLN165 arc C.

Documented 2026-09-21 (before measuring): **FAIL** — `<T, U>` does not parse (documented).

### A2_fold_accumulator_type

Measured 2026-09-21: FAIL — arc C.

Documented 2026-09-21 (before measuring): **FAIL** — same root cause as A1.

### A3_type_var_not_in_first_param

Measured 2026-09-21: FAIL — `<T, U>` does not parse; arc C.

Documented 2026-09-21 (before measuring): **FAIL** — "Type variable T must appear in the first
parameter" (documented).

### A4_generic_struct

Measured 2026-09-21: FAIL — `struct Pair<` does not parse; @PLN165 arc D.

Documented 2026-09-21 (before measuring): **FAIL** — generic structs do not exist (documented).

### A5_generic_recursive_enum

Measured 2026-09-21: FAIL — arc D, and A5b: a direct self-reference is refused naming `reference<Expr>` as the cure, which then refuses an `Expr` value (defect 3).

Documented 2026-09-21 (before measuring): **FAIL** (A4 root cause). Also unverified: whether a
*non-generic* struct-enum may hold its own type directly in a field (not via
`vector<Self>`). If it may not, add probe `A5b_recursive_enum_mono` as a
prerequisite with `enum Expr { Lit { v: integer }, Add { l: Expr, r: Expr } }`.

### A6_user_result_type

Measured 2026-09-21: FAIL — arcs C + D.

Documented 2026-09-21 (before measuring): **FAIL** (A1 + A4 root causes). Note that `E` appears only
in the enum, never in the function's first parameter directly — A3 must hold.

### A7_user_declared_interface

Measured 2026-09-21: **PASS** both backends, spelled `fn area(self: Self) -> float`. Doc bug 1 (the empty Interfaces page) stands.

Documented 2026-09-21 (before measuring): **UNKNOWN**. First action: determine the truth, then either
(a) mark PASS and *fill the empty Interfaces reference page*, or (b) mark FAIL.
Either outcome fixes a doc bug.

### A8_associated_type

Measured 2026-09-21: PARTIAL — the companion works inside a generic (`type Rows: Cursor`, `Self.Rows`, @PLN125); `-> S.Item` in the generic's own signature does not parse (documented).

Documented 2026-09-21 (before measuring): **UNKNOWN** (feature listed, no reference page found).
Depends on A3 (`S.Item` in the return type).

### B1_vector_of_capturing_closures

Measured 2026-09-21: FAIL — named refusal, as documented.

Documented 2026-09-21 (before measuring): **FAIL** — "a capturing closure cannot be stored in a
collection" (documented). This blocks handler tables, behaviour lists,
combinator alternatives and thunk queues; it is the single most consequential
closure limit.

### B2_handler_table

Measured 2026-09-21: FAIL — B1, and `hash<(…)>` has no tuple element type.

Documented 2026-09-21 (before measuring): **FAIL** (B1 root cause). Keyed-collection syntax for a
`(text, fn())` entry is illustrative.

### B3_two_closures_share_written_scalar

Measured 2026-09-21: FAIL — named refusal (*"captured by 2 closures"*).

Documented 2026-09-21 (before measuring): **FAIL** (documented refusal). Negotiable because the
struct workaround is one line and the restriction has a clear no-GC rationale.
If the decision is "never", record it in DESIGN_DECISIONS.md and mark this entry
`WONTFIX` rather than deleting it.

### B4_recursive_local_function

Measured 2026-09-21: FAIL — *"'fn' definitions must be at file scope"*; the self-referencing lambda is an ICE (defect 2).

Documented 2026-09-21 (before measuring): **UNKNOWN** — no local `fn` or self-referencing lambda
appears in the reference. If refused, the criterion can also be met by a
self-referencing lambda; either spelling passes.

### B5_closure_captures_ref_param

Measured 2026-09-21: **works** — capture and write-through succeed on both backends, so the expectation flips to `pass` and 26-closures is stale (doc bug 5). The probe as written was invalid (`bump_all(&xs)`, untyped `|i|`).

Documented 2026-09-21 (before measuring): **PASS as a refusal** (documented). Flip to `# bar: pass`
only if the design changes.

### B6_compose

Measured 2026-09-21: FAIL — arc C.

Documented 2026-09-21 (before measuring): **FAIL** (A1/A3 root causes; also `V` appears only in the
second parameter and the return type).

### C1_nested_constructor_pattern

Measured 2026-09-21: **PASS** over a non-recursive nesting, both backends; this recursive form is blocked on A5b.

Documented 2026-09-21 (before measuring): **FAIL** — no nested constructor in field position exists.
Prerequisite: a struct-enum field of its own enum type (see A5b note).

### C2_literal_in_field_position

Measured 2026-09-21: **PASS** both backends.

Documented 2026-09-21 (before measuring): **FAIL** (documented: bindings only).

### C3_field_rename_and_as_binding

Measured 2026-09-21: FAIL — neither the rename nor `whole @` parses.

Documented 2026-09-21 (before measuring): **FAIL** (documented: names must equal field names).

### C4_tuple_of_variants

Measured 2026-09-21: **PASS** both backends in the real spelling — the probe's arm body `Playing { hp }` uses field-init shorthand, which loft does not have; `Playing { hp: hp }` passes every assertion.

Documented 2026-09-21 (before measuring): **FAIL** (tuple elements documented as scalars/`_` only).
Note @F122 multiple dispatch covers the *dispatch* half of this; it does not
give one arm access to both sides' fields plus a guard over both.

### C5_or_pattern_with_bindings

Measured 2026-09-21: FAIL — does not parse.

Documented 2026-09-21 (before measuring): **UNKNOWN** — or-patterns are documented only over bare
variants and scalars.

### C6_exhaustiveness_through_nesting

Measured 2026-09-21: PARTIAL — non-recursively nested, the hole is refused but named by its outer variant (*"missing: W"*); this recursive form is blocked on A5b.

Documented 2026-09-21 (before measuring): **FAIL** (depends on C1; once nested patterns exist, the
checker must see through them — this probe ensures the two land together).

### C7_scalar_match_hole_is_diagnosed

Measured 2026-09-21: FAIL — silent on both backends.

Documented 2026-09-21 (before measuring): **FAIL** (documented silence). Treat `refuse` here as
"emits a diagnostic containing the substring"; a warning that still compiles
counts, and `bar.loft` should accept warnings for this entry.

### C8_vector_head_tail_pattern

Measured 2026-09-21: PASS with a `_` arm (the probe's `sum` is a reserved name); `[]` + `[x, ..rest]` is not seen as total.

Documented 2026-09-21 (before measuring): **UNKNOWN** — verify against the @F99 syntax and rewrite the
probe in it if it differs.

### D1_generic_return_cannot_smuggle_null

Measured 2026-09-21: FAIL — a decided edge: (N-Index) trusts a constant index by contract (C80, @PLN102 D1).

Documented 2026-09-21 (before measuring): **FAIL**. Acceptable fixes: (a) constant-index reads are
typed `T?` like computed ones; (b) a returned expression whose static type is
`T?` cannot satisfy a declared `T` without `??` or `?`; (c) a flow proof of
`len(v) > 0`. Any of the three passes.

### D2_empty_stub_default_is_explicit

Measured 2026-09-21: FAIL — silent `0.0` on both backends.

Documented 2026-09-21 (before measuring): **FAIL**. A warning at the *read site* (the result is used)
satisfies this; an unused stub may stay silent.

### E1_persistent_cons_is_O1

Measured 2026-09-21: BLOCKED on A5b; `memory_used()` does not exist — use `store_memory()`'s record count.

Documented 2026-09-21 (before measuring): **UNKNOWN** (recursive enum prerequisite; metric name
illustrative).

### E2_persistent_tree_insert_is_Olog

Measured 2026-09-21: BLOCKED on A5b.

Documented 2026-09-21 (before measuring): **BLOCKED**.

### F1_missing_combination_named

Measured 2026-09-21: PARTIAL — refused, naming the declared set rather than the missing pair.

Documented 2026-09-21 (before measuring): **UNKNOWN** — verify the diagnostic exists and names the
pair (either order of the two names is acceptable in the substring check;
adjust the expectation to the real wording once known, but it must name both).

### F2_enum_level_fallback_covers_combinations

Measured 2026-09-21: FAIL — defect 1: on a plain enum all nine combinations take the fallback; struct-enum variants dispatch correctly.

Documented 2026-09-21 (before measuring): **UNKNOWN**.

Measured 2026-09-21: covered — each cited page is generated from a `tests/docs/*.loft` file that `make ci` runs.

## Documentation bugs found while measuring (all four closed; § Evaluation — measured 2026-09-21 records each)

File these regardless of any implementation decision.

1. `stdlib-interfaces.html` renders only its heading. Either the generator
   dropped the body or the page was never written. It is the page A7/A8 need.
2. `09-enum.html` implements `opposite()` with an if-chain where the page's own
   later section shows `match`; the example teaches the pattern the page argues
   against. Rewrite with `match`.
3. `25-generics.html` states the `T`-typed `v[0]` null leak plainly and offers
   only "check the length before calling". That is a documented soundness hole
   (D1); the doc should link to the tracking issue.
4. `29-match.html` "Nested match" heading describes nested *expressions*. Rename
   to "Match inside an arm" so that "nested patterns" is not a term the docs
   appear to already own.

## Evaluation rows changed at `58601f08a` (as measured at `b194678c`)

| entry | measured | what answered |
|---|---|---|
| A3 variable not in first param | FAIL | `zip<T, U>` building a `vector<(T, U)>`: the interpreter skips `main`, panics or segfaults, native fails `E0308` — [loft#1868](https://github.com/loft-lang/loft/issues/1868) (`silent-wrong`); a generic struct element (`vector<Two<T>>`) works.  `empty_of<T>() -> vector<T>` is refused: *"Generic function must have at least one parameter of type T"* <!-- doc-lint: ok --> |
| C1 nested constructor pattern | PARTIAL | non-recursive nesting PASS; the recursive shape through a `reference<Expr>` field never matches on the interpreter and fails `E0605` on native — [loft#1870](https://github.com/loft-lang/loft/issues/1870) (`silent-wrong`) <!-- doc-lint: ok --> |
| C5 or-pattern with bindings | **PASS** | the spelling is `,`: `Circle { r }, Sphere { r } => r` (`@FR-P-Multi`, LOFT_CONTROL.md § Match expressions).  `\|` joins variant names only; between patterns that bind it is refused (*"Expect token =>"*) |

## Evaluation rows changed after `58601f08a` (as measured there)

| entry | measured | what answered |
|---|---|---|
| C3 field rename + as-binding | PARTIAL | rename PASS (`Rect { w: width, h }`); `whole @ Rect {…}` → *"'whole' is not a variant"* |
