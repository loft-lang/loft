<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Design — the coherent immutability model

**Status: Phase 1 IN PROGRESS on `tuxedo-work` (no PR).**  Clean-slate spec for
loft's immutability, coherence first — the shipped @PLN40 field-const and the ad-hoc
const-param/local behaviour all become one quadrant of this model; the libs get
re-annotated to match.  No new keyword.

## Implementation progress (Phase 1)

- **Step 0 (matrix probe) — DONE.**  Target = the 4×2 table below × {local,field,param}.
  Reusable probe recreatable from the "Implementation" section.
- **Step 1 (const-local = binding-const, allows append) — DONE** (commit `13d625df`).
  Unified the 5 scattered `is_const_param` write-guards into one parser helper
  `const_write_blocked(nr, op) = is_const_param && (is_argument || op == "=")`
  (`src/parser/operators.rs`).  const-local now rejects only rebind; const-param
  unchanged.  Verified both backends; guard `test_const_local_binding_allows_append`
  in `tests/scripts/90-immutability.loft`.
- **Step 2 (two flags: binding-const vs value-const) — DONE.**  Renamed
  `Variable.const_param → const_binding` (binding-const, `const` PREFIX) and added a
  NEW `value_const` (value-const, `const` before the TYPE); axis-neutral guards (d#lock
  unlock, text-arg auto-promotion, dead-store + UPPER_CASE lints) route through the new
  `is_const_any`.  Parses `x: const T` local (was a parse error).  Inert on its own.
  `value_const` rides on `Variable` **only** — see the FIELD scope note below.
- **Step 3 (value-const enforcement via base-resolution) — DONE (load-bearing).**
  Param `p: const T` now sets `value_const` (was binding); `const_write_blocked` is
  two-flag + op-based (binding rejects `=`, value rejects `+=`); `validate_write`
  resolves a component write's BASE variable (`lhs_base_var`, walks `args[0]` to the
  leaf `Var`) and rejects any mutation through a value-const binding.  NO `Type::Const`
  wrapper (ripples through every Type match).  Full 23-cell matrix green on BOTH
  backends; positives in `tests/scripts/90-immutability.loft`, negatives in
  `tests/scripts/102-expected-errors.loft`.
  - **Rule 1 (scalar collapse) folded in here** — required for step-3 coherence: once a
    scalar `const` param is value-const, a bare "rebind allowed" would make
    `const integer` params reassignable, so a by-value scalar (`integer / float /
    single / boolean / character`) and a `& const T` reference are FULLY immutable under
    either axis (reject `=` AND `+=`).  Text stays COMPOUND (append-allowed for
    binding-const), matching the shipped const-text-field behaviour — the design's
    "text is a by-value scalar" reading is left as an open question.
  - **FIELD value-const deferred to Phase 2:** `v: const T` on a struct (the record-type
    deep freeze) needs `Attribute.value_const` + IR-serialisation + type-carried
    transitivity through stores/returns/generics — all Phase 2.  Phase 1 ships the
    lib-relevant case: value-const **params + locals** (the honest read-only borrow).
    A field's binding-const half (`const v: T`) already works.
- **Steps 4–5** — the `&`-borrow interaction (a value-const value may not be passed to a
  `&` param; rule 4) + docs graduate to `tests/scripts/40-const-fields.loft` +
  `tests/issues.rs` and the LOFT.md / loft-write 4-quadrant table.  The const-suggest
  lint (const-suggest-lint.md) is now well-defined as a DUAL lint (binding-const on a
  never-rebound field, value-const on a never-mutated field/param) — its own follow-up.
  - **Scope correction (measured 2026-08-18, @PLN141 C2 slice 5): rule 4 names the wrong
    spelling.**  It closes `const` → `&` param, but `&` is not what lets a callee write:
    a PLAIN struct parameter names the caller's record too (@F106 / @PLN141 `@FTR-003`).
    Measured on both backends, a value-const parameter passed to a plain parameter is
    accepted and the callee's field write lands in the ORIGINAL record:

    ```loft
    fn bump(a: Account) { a.balance = 999; }
    fn describe(acct: const Account) -> text { bump(acct); "…" }   // accepted today
    // caller's balance: 500 -> 999
    ```

    So steps 4–5 must gate every argument position that can be written — plain struct /
    vector / value-struct parameters as well as `&` — or the rule closes the rare
    spelling and leaves the common one open.  A direct write through the parameter is
    already refused (`Cannot modify const parameter 'a'`), which is what makes the hole
    invisible: the guarantee reads as enforced right up to the first call.
  - **Built 2026-09-15 (loft#1540): rule 2 for views, and rule 4 for `&`.**  A view of a
    value-const value — a loop variable over its elements, an index / field projection or `&`
    link bound to a local, a loop over such a view, a read off a value-const FIELD — is marked
    value-const at its bind (`Parser::mark_const_view`), so every existing write guard refuses a
    write through it, `#remove` included, and the refusal names the view and the value.  A
    value-const value or view handed to a `&` parameter is refused at the call.  Guards:
    `tests/scripts/a-view-of-a-const-value-is-read-only.loft` (12 refusals) and
    `…-leaves-its-copies-writable.loft` (the copies, whole-value binds and `const` hand-offs that
    must stay legal).  Known over-approximation: the mark follows the BIND, not the flow, so a
    view local rebound to a fresh value is still refused.
  - **Built 2026-09-15 — the plain-parameter half, by SIGNATURE (owner, DESIGN_DECISIONS.md C124),
    landing as the warning `const-to-plain-parameter` until the 17 library helpers C124 § Rollout
    lists declare `const`.**
    A value-const value reaches a record or collection parameter only when that parameter is
    declared `const`; the body is never read (C121), and a proof that a callee does not write stays
    an optimisation's (C122).  The parameter's `const` is carried on the definition's attribute
    (`Attribute.value_const`, serialised in the IR store — the Phase 2b deferral this needed) and
    kept by every copy of a signature: generic instances, interface and bound-method stubs,
    default-value functions, overload dispatchers.  The stdlib's read-only heap parameters are
    `const`; `Op*` primitives are exempt (only stdlib bodies call them, and `const` there means an
    immediate operand).  **Open:** a function type cannot spell `const`, so a value-const value
    handed through a function reference is unchecked (formal/binding.md D-bind-45).

## First principle — two orthogonal facts, and loft already has one of them

Immutability is really **two independent questions**:

1. **Can the slot be re-pointed?**  (`x = otherValue`) — a property of the
   *binding* (the variable / field slot).
2. **Can the value be mutated through this name?**  (`x.f = …`, `x[i] = …`,
   `x += …`) — a property of the *access* to the value.

These map cleanly onto machinery loft **already** has:

- Question 2 is exactly the **borrow mode**.  loft already has `&T` = a *mutable*
  borrow (write-through).  Its missing counterpart is the *immutable* borrow — a
  read-only view.  **That is what value-`const` is** (`&T` : `const T` :: Rust
  `&mut T` : `&T`).
- Question 1 is the **binding mode** — `let` vs `let mut` in Rust.  loft's default
  binding is mutable; binding-`const` is the immutable binding.

So the model is not a bolt-on: it completes the borrow system (adds the read-only
borrow) and adds an immutable binding.  One keyword, positioned to say *which fact*:

## The model

- **`const` before the name → binding-const.**  The slot never re-points.
  `const x = …` (local), `const v: T` (field), `const p: T` (param).
  Rejects `x = …`.  Says nothing about the value.
- **`const` before the type → value-const (a read-only borrow of the value).**
  `x: const T`, `v: const T`, `p: const T`.  The value cannot be mutated through
  this name: no `x.f = …`, no `x[i] = …`, no `x += …`.  **Transitive** — reading a
  field/element of a `const` value yields a `const` view, so immutability can't be
  laundered by extracting a part.
- **Both compose:** `const v: const T` — a slot that never re-points, holding a
  value that can't be mutated = fully immutable.

| declaration | `x = …` (rebind) | `x.f=` / `x[i]=` / `x += …` (mutate) |
|---|---|---|
| `x: T` (default) | ✓ | ✓ |
| `const x: T` (binding) | ✗ | ✓ |
| `x: const T` (value) | ✓ | ✗ |
| `const x: const T` (both) | ✗ | ✗ |

### Coherence rules (what makes it total, not ad-hoc)

1. **Scalars collapse.**  A scalar (`integer`, `float`, `boolean`, `character`,
   `text` when used by value) has no "contents" distinct from its binding, so
   binding-const and value-const coincide — `const x: integer` is the whole story;
   `x: const integer` is accepted as the same thing (no separate meaning).
2. **`const` is transitive on a value.**  `const S` on a struct freezes every field
   recursively; `const vector<T>` freezes the vector *and* makes each element
   `const T`.  A read (`c.inner.x`, `v[i]`) is always allowed and yields a `const`
   view; a write anywhere under a `const` is rejected.
3. **It composes into generics.**  `vector<const Point>` = a *mutable* vector whose
   *elements* are frozen (you can append/replace elements, but not mutate a Point in
   place); `const vector<Point>` = a frozen vector of (transitively frozen) Points.
   This nested control is the payoff of putting value-const in the type.
4. **`const T` is the immutable borrow.**  As a parameter it is the read-only
   counterpart to `&T`: `f(p: const T)` promises not to mutate the caller's value;
   `f(p: &T)` may.  A `const` value may be passed to a `const` param but not a `&`
   param (can't hand a read-only value to something that will mutate it).
5. **No laundering via return.**  A function returning a borrow of a `const`
   parameter/field returns a `const` borrow.

## Where the four quadrants are used (guidance)

- **`const x: const T` (fully immutable)** — value / record types: `Pixel`, `Vec3`,
  `Rect`, `GeoPoint`, `Event`, `HttpResponse`, DB rows.  "It is a value; freeze it."
- **`const v: T` (binding-const)** — builder/accumulator fields grown then read:
  `Mesh.verts`, `Scene.meshes`, `Args.options`, `World.chunks`, `CellSnap.*`.  The
  slot is fixed; the container still grows via `+=`/element writes.
- **`p: const T` (value-const)** — read-only parameters (the shared borrow): the
  `const Image` / `const Region` accessor style, made honest (truly no mutation).
- **`p: &T` (unchanged)** — the mutable borrow, for write-through params.

## Implementation — safe small steps

**The load-bearing constraint (from the code):** value-const must **not** be a
`Type::Const(Box<Type>)` wrapper.  `Type` (`src/data.rs`) is pattern-matched in
hundreds of sites across the compiler; a wrapper variant would ripple through every
one (unwrap-or-break) — the exact codegen change that regresses the suite (the
loft-codegen skill's probe-04 anti-example).  So the plan is **phased**:

- **Phase 1 — flags on the binding + base-resolution** (this plan).  Both axes ride
  on the existing per-binding flag mechanism (`Attribute.const_field` for fields,
  `Variable.const_param` for vars), enforced at the write chokepoints by resolving a
  write's *base binding* and checking its flags.  No `Type` change → no ripple.
  Delivers the model for **direct writes through a binding** — the common case, and
  everything the libs need.
- **Phase 2 — type-carried const** (a separate, larger plan).  Thread a `const` bit
  through `Type` so immutability survives store round-trips, function returns
  (no-laundering), and composes into generics (`vector<const T>`).  Phase 1's
  base-resolution covers direct writes without it.

**Representation (phase 1):**
- **binding-const** = `const` PREFIX → the shipped `const_field` (fields) + a
  `const_binding` flag on `Variable` (vars; the existing `const_param` renamed/reused).
- **value-const** = `const` before the TYPE → a NEW `value_const` bool on `Attribute`
  and `Variable`.  Both may be set (`const v: const T`).

Both axes enforce at the two chokepoints loft already has: `validate_write`
(`expressions.rs:3453`, fields) and the `is_const_param` sites
(`operators.rs:20`/`116`/`302`, `collections.rs:794`/`865`, vars).

| # | Step | Verify (both backends) |
|---|---|---|
| 0 | **Matrix probe** (throwaway): the 4×2 table × {local, field, param} × {scalar, struct, vector, hash, text, nested} — hand-computed target verdicts.  This is the spec; snapshot CURRENT behaviour beside it. | spec recorded |
| 1 | **Binding-const, unified append.**  const-local (`const x`, prefix) allows `+=` (contents), matching const-field; const-*param* (`p: const T`, type-qual) does NOT — it is heading to value-const.  Distinguish via `const_kind` (argument flag).  A contained relax of the `is_const_param` append guards. | binding-const row identical for local + field; param unchanged |
| 2 | **Parse the two positions into two flags.**  `const` prefix → binding flag; `const` before the type → `value_const` on the `Attribute`/`Variable`.  Inert until step 3 (nothing reads `value_const` yet). | parses; flags set (white-box); suite green (inert) |
| 3 | **Value-const enforcement — the base-resolution (load-bearing).**  For any write, resolve the *base binding* of the target chain (`a.b.c[i] = x`, `a += …`, `a = …` → base `a`); if its `value_const` flag is set, reject ALL of rebind / append / element / field / nested.  Extends `validate_write` (already resolves a field's parent) and the `is_const_param` sites with base-chain resolution.  Transitivity for direct writes falls out (any write under a value-const base is caught). | value-const row + `p.inner.x=` transitivity, both backends; gate on the written matrix, not just the suite |
| 4 | **Scalars collapse + borrow interaction.**  For a scalar binding, binding-const == value-const (rule 1).  A `value_const` binding cannot be passed to a `&` (mutable-borrow) param; `p: const T` is the read-only borrow (rule 4). | param matrix + the `&`-vs-const cases |
| 5 | **Docs + graduate.**  The full matrix into `tests/scripts/40-const-fields.loft` + `tests/issues.rs` negatives; LOFT.md / loft-write carry the 4-quadrant table + the `&`/`const` borrow analogy. | `make ci` |

Each step is contained: step 1 only turns a const-local `+=` error into a success;
step 2 is inert; step 3 only fires on the new `value_const` flag (nothing sets it
before step 2).  No env-gate needed.  Phase 2 (type-carried) is deferred to its own
plan and is the only part that touches `Type`.

## The library-scoped const-suggest lint

Suggests the RIGHT quadrant, reusing the two write chokepoints as the oracle:
- **binding-const** (`const` prefix) on a field never *rebound*;
- **value-const** (`: const T`) on a field/param never *mutated at all* (only read /
  passed to `const` params) — the value/record types;
- library-scoped (fires on library packages, quiet on app code); advisory (a `pub`
  field's or an exported fn's writers may be unseen — author judges).

## Re-annotating the libs

"We update the libs ourselves" — the classification from the uptake survey drives it:
- upgrade the pure value/record types to `const x: const T` (Vec/Mat/Pixel/Rect,
  Cbor/Hpke/GameEnvelope/Event, MonsterDef/Way/GEdge/SubPath/TerrainSample, the
  `value struct` DateTime/Duration);
- keep the builders as `const v: T` (Mesh/Scene/Args/World/PTile/EdgeCosts/CellSnap…);
- turn the read-only accessor params (`const Image`/`Region`/`Hydro`) into honest
  value-const `p: const T`.

## See also

- [README.md](README.md) — @PLN40 field-const, now the `const v: T` quadrant
- [const-suggest-lint.md](const-suggest-lint.md) — the lint (subsumed above)
- `src/parser/expressions.rs:3453` (`validate_write`) + `parser/operators.rs:20` /
  `collections.rs:794` (`is_const_param`) — the binding chokepoints
- THREADING.md / the `&`-borrow model — value-const is its read-only sibling
