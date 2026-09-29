<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 22 — Enum-scoped variants, prelude shadowing, and `use … as …`

> **CLOSURE RECORD — DONE 2026-06-14.**  All four phases built and validated on
> both backends; issue [@PLN22](https://github.com/loft-lang/plans/issues/22) is
> `status:finished` (closed).  **User-facing reference moved to its home:**
> [LOFT_DECLARATIONS.md § Library imports / Shadowing / Enum-scoped variants](../../LOFT_DECLARATIONS.md#library-imports)
> + [CHANGELOG_TECHNICAL.md](../../CHANGELOG_TECHNICAL.md) (technical) and
> [CHANGELOG.md](../../../../CHANGELOG.md) (user-facing).  This file is retained
> as the design + build record (the chokepoint-first build order, the
> design-protocol rounds, and the as-built per-phase notes) — read LOFT.md for
> how it works today, this file for how it was built.

Tracker: [@PLN22](https://github.com/loft-lang/plans/issues/22).  Standard plan
(design-before-build).  Every M+ phase runs the
[design-protocol](../../DESIGN_PROTOCOL.md); every fix runs matrix-first.

## Status (REQUIRED)

**ALL PHASES BUILT (2026-06-14) — two enums may share a variant name (P1); user
defs shadow stdlib/prelude names while `std::Name` still reaches the prelude (P2);
`use … as …` aliasing for libraries, types, and functions (P3); grouped
`use lib::(a as x, b);` selective imports, flat comma-list dropped (P4).  Matrix +
suites green on both backends bar known-environmental (WASM rlib / port-bind).**

The build landed via the chokepoint-first order.  **Final design (a deliberate
choice with the project owner):** a bare variant used as a VALUE resolves ONLY via
context (match subject, typed decl, typed reassignment / `rec.field`, parameter,
return incl. an `if`-branch tail, struct-field type & default, `==` LHS,
`Enum::`/`Enum.` qualifier).  A bare variant used to DEFINE a new untyped variable
(`x = Red`) is a **hard error — even when the name is currently unique** — so that
adding a second enum with that variant name can never silently re-point an existing
bare assignment (the friction the owner asked to eliminate).  The variant name
stays usable as a TYPE / constructor (`Circle { … }`, `s: Circle`,
`fn f(self: Circle)`), so struct-variant construction is unaffected.

This SUPERSEDES an interim "ambiguity-tracking" build (which preserved no-context
*unique* `s = Idle`) — the owner chose the stricter rule on 2026-06-14.

**Phase 1 design round done (2026-06-13). Phases 2–4 drafted.**
An earlier draft framed this as
"loft has a single flat namespace / `match North` resolves by global lookup."
**That was wrong** — validated against the current tree (probes, 2026-06-13).
loft's *resolution* is already scope- and context-aware; the residual problem is
narrow and lives on the **definition** side.  This plan fixes exactly that.

### What already works (VERIFIED — do not rebuild)

| Capability | Evidence |
|---|---|
| `::` path separator | `consts::E`, `char::from`, `f32::from_bits` in the stdlib |
| Library-qualified names + **auto-`use`** + use-region pre-scan | `lib::fn`, `objects.rs:27` (`get_source`), `mod.rs:4475` |
| `use lib;` (wildcard), `use lib::*;`, `use lib::name;` | `imports.rs` (5 pass); `ImportSpec` at `mod.rs:4479` |
| **Context-directed bare variants** (the "context not namespace" model) — resolve against the known type in match arms, typed locals (`d: Dir = Up`), comparisons (`d == Up`), function args (`pick(Down)`) | probe `g1` → `typed=1 arg=2`; `tests/lib/match_lib_enum_main.loft` (C53) |
| `Enum::Variant` and `lib::Variant` qualification | C53 test: `Status::Yay`, `enumlib::Yay` |

### The actual gaps (VERIFIED)

1. **Variant definitions are keyed by BARE name** (`add_def` keys `(name, source)`,
   `data.rs:3001`), so two enums that share a variant name **panic at parse time**:
   `enum A { Red }` + `enum B { Red }` → `Dual definition of Red` (`data.rs:3003`).
   Resolution is context-aware, but *storage* is flat-per-source. (probe `g2`)
2. **No prelude shadowing**: a user definition that collides with an imported /
   stdlib name is rejected — `enum E` → "conflicts with a constant … pick a
   different name" (the stdlib `E`). (probe `g3`)
3. **No `as` aliasing**: the `use` grammar has no `as` clause — so two libraries
   (or two enums) exporting the same name have no disambiguation escape hatch.
4. **The selective-import syntax reads poorly**: `use lib::a, b, c;` — a flat
   top-level comma list where `b`/`c` don't visually bind to `lib::`.

## Goal (REQUIRED)

Variant names stop being global; a user name may shadow the prelude; collisions
have an `as` escape hatch — *without* regressing the context-directed bare-variant
resolution that already works.

## The load-bearing invariant

> A definition is keyed by its **scope path**, not a bare name: a variant by
> `(enum, variant)`, a top-level name by `(source, name)` with **user defs
> shadowing the prelude**.  Two definitions collide **iff** their full scope
> paths are equal — so two enums may both have `Red`, and a user `enum E` is
> legal.  Resolution is unchanged where it already works (context + `::`).

## Phases

### Phase 1 — enum-scoped variant definitions (M; the core fix)

**Design round DONE** (design-protocol steps 1–4, 2026-06-13); build pending.

**Invariant:** a variant is identified by `(enum, variant_name)` and found *only*
via its enum's member index — never a bare global `def_nr`. Two variants collide
iff their `(enum, name)` paths are equal.

**Probed and confirmed — the mechanism already half-exists.** Resolution already
routes through the enum's members in both places that matter: the value path uses
`def(enr).attr_names.get(name)` (`objects.rs:301`), and the match path falls back
to `children_of(e_nr).find(name)` (`control.rs:1414`, the C53 fix). The variant
*def* legitimately coexists for its substructure (struct-enum fields, discriminant,
codegen) — so this is NOT "variants stop being defs"; it's "variants stop being
**globally keyed**." The collision is one redundant thing: `add_def(variant, …,
EnumValue)` (`definitions.rs:216`) *also* inserts the bare `(name, source)` key
into `def_names` — that key is what panics and what every site tries *first*.

**The fix (3 moves):**
1. **Chokepoint** — `Data::variant_of(enum, name) -> Option<def_nr>`, unifying the
   two existing enum-member lookups (`attr_names.get` + `children_of.find`). Every
   resolution site consults it with the contextual enum.
2. **De-globalize** — `add_def` for `EnumValue` stops inserting the bare
   `(name, source)` key (the def still exists, reachable via the enum). Panic gone;
   two enums share `Red`.
3. Drop the `def_nr(name)`-first try for variants at the resolution sites
   (`control.rs:1411/1451/1536/1548/2987/3017/3028`, `objects.rs:290`,
   `fields.rs:491`); `Enum::V` / `lib::V` resolve the same member path; a
   no-context bare variant stays an error (demand qualification).

**Brittleness (step 2): low by construction.** N ≈ 9 sites, but **omission is
loud, not silent** — once variants aren't globally registered, a missed site gets
`def_nr(variant)==MAX` and fails to resolve (a hard error a test catches), never a
silently-wrong variant. The C53 `children_of` fallback already exists at the main
sites, so de-globalizing mostly *promotes the fallback to primary*.

**Matrix:** two-enums-sharing-a-variant × {match, typed local, `==`, fn arg, `is`,
`Enum::V`, `lib::V`} × both backends; the C53 tests (`tests/lib/match_lib_enum_main.loft`)
stay green; probe `g2` (the panic) flips to passing. (`g3` `enum E` is the
prelude-shadow case → **Phase 2**, not Phase 1.)

**Build spike (2026-06-13) — scope VALIDATED; de-globalize is necessary but NOT
sufficient.** A throwaway spike (the `variant_of` chokepoint + de-globalizing
`add_def` for `EnumValue` + the second-pass re-resolve `definitions.rs:220` →
`variant_of`) confirmed the easy half: the `Dual definition of Red` panic is gone,
two enums share `Red`, and the stdlib still loads. Loud-omission then pinpointed
the residual (exactly as the brittleness argument predicted):

- **Value-position bare variants** (`f(Red)`, `l: Light = Red`, `l == Red`) fail
  with `Unknown variable 'Red'`. They resolve through `parse_var`'s `parent_tp`
  context branch (`objects.rs:301`), which (a) checks `Type::Enum` while the
  typed-declaration path threads `Type::Reference(enum)` (`objects.rs:1673`) — so
  it misses — and (b) is never populated with the expected enum at the
  comparison-RHS and call-arg sites.
- The match / `is` paths already carry the enum (the C53 `children_of` fallback),
  so they are fine.

So Phase 1's true cost = de-globalize + `variant_of` (small) **plus threading the
expected enum to the value-position resolver**: normalize `Reference(enum)`↔`Enum`
at `objects.rs:301`, and set `parent_tp` at the `==` / arg / `return` / field-init
sites. This is a design-protocol step-6 ESSENTIAL divergence (a real domain axis:
value-position variant resolution genuinely needs expected-type plumbing — the
original "resolution already has the context" claim held only for match).

**Build order next session:** (1) `variant_of` + de-globalize + `definitions.rs:220`;
(2) `objects.rs:301` Reference-enum normalization; (3) thread the expected enum
INTO `parse_var` at the value-position sites; (4) verify the matrix + C53 +
`loft_suite`/`native_scripts` both backends. Spike matrix: two enums sharing
`Red` across match / arg / typed-local / `==` / `Enum::V`.

**Confirmed (2026-06-13, 2nd spike):** steps 1+2 *alone* break `loft_suite` —
existing code resolves value-position bare variants — so **step 3 is mandatory,
not feature-only**. And step 2 alone is insufficient: `parent_tp` is **not set to
the expected enum** at the typed-local-declaration / `==`-RHS / call-arg sites
(my Reference-enum branch never fired there), so step 3 is the **bulk** of Phase 1
— locate and thread the expected type *into* `parse_var` at each value-position
parser site (decl initializer, comparison RHS, call arg, `return`, struct-field
init), after which the context branch resolves the variant. A focused
session-sized parser change; do NOT interleave with other work (it leaves the
tree red until step 3 lands). Both spikes reverted; `2026-07` clean.

**Resume artifact (SUPERSEDED — Phase 1 is built):** the old `phase1-step1-2.patch`
encoded the de-globalize-FIRST approach the build-order correction reversed; it was
removed once Phase 1 landed. The as-built record is *Phase 1 — BUILT* above.

**Step 3 exploration (2026-06-13) — the `var_tp` unifying lever is FALSIFIED.**
Tried: add an `expected_tp` param to `parse_var` + a `variant_ctx(name, parent_tp,
expected_tp)` helper (resolve a variant against either context enum), and pass
`var_tp` (the operand's expected type, already threaded through `parse_operators`)
as `expected_tp` from `parse_single` (vectors.rs:400). Result on the four
isolation probes:
- `p_arg` (`classify(Red)`) and `p_ret` (`-> Light { Red }`): **still fail** —
  `var_tp` is NOT the param / return type at those leaves; the call-arg parser
  and the return-body parser don't thread their expected type as `var_tp`.
- `p_decl` (`l: Light = Red`): now resolves but `l == Light::Red` is **false** —
  a **bare-vs-qualified variant-value representation mismatch** (the bare
  `attr_value` form ≠ the qualified `Light::Red` form). A correctness bug, not
  just a resolution gap.

So step 3 is **genuinely per-site**, not one lever: at each value-position site
find where its expected type actually lives and thread it as the variant context
— call arg (param type in `parse_call`), `return` (fn return type at the body
parse), comparison RHS (the LHS `current_type` at `operators.rs:1295`, NOT the
outer `parent_tp`), typed-local decl — **and** reconcile the bare vs `Enum::V`
variant value representations so `==` agrees. Each is its own probe + targeted
edit; watch the silent create-placeholder-var fallback (it turns a missed site
into a wrong result, not a loud error). The `var_tp` attempt was reverted (not in
the patch); the helper idea may still seed step 3, but with the *correct*
per-site type, not `var_tp`.

**BUILD-ORDER CORRECTION (2026-06-13, 3rd probe) — de-globalize LAST, not first.**
The `p_decl` "wrong value" was a symptom of a bigger problem: de-globalization
regresses **qualified `Enum::Variant` too**. `Light::Red == Light::Red` is `true`
on clean `main` but **`false` under steps 1+2** — the qualified path
(`parse_var`'s `::` branch → `parse_constant_value`) *also* relied on the global
`def_nr(variant)`. So de-globalize-FIRST makes the tree red until EVERY
variant-resolution path is rewired at once — it cannot land incrementally (each
"continue" surfaced another broken path: bare value, then qualified, …).

**The right order is CHOKEPOINT-FIRST.** Refactor every variant-resolution path
to route through `variant_of(context_enum, name)` *while variants stay globally
keyed* — each refactor is then non-breaking (variant_of finds the same variant)
and independently landable on a green tree. De-globalize is the **final flip**.
Paths and where each gets its enum context:
- **match arm** — the subject type (the `children_of` fallback already exists).
- **`Enum::Variant` qualified** — the qualifier `name` IS the enum; rewire the
  `::` path so when the qualifier resolves to an enum, the variant is
  `variant_of(qualifier_enum, nm)` (not a global/source lookup).
- **`is`** — the subject type.
- **bare value-position** (arg / decl / `return` / `==`-RHS) — the *expected*
  type, which is NOT threaded today (the per-site work; do these incrementally
  too, each landable while still global).

**Revised build order:** (1) `variant_of` + `definitions.rs:220` [in the patch,
non-breaking]; (2) route match / qualified / `is` through `variant_of` [land each];
(3) thread the expected enum at the bare-value sites [per-site, land each]; (4)
de-globalize `add_def` [the flip — now safe, all paths already route through the
chokepoint]; (5) verify matrix + C53 + suites both backends. The step-1+2 patch's
de-globalize half moves to step 4; its `variant_of` + `definitions.rs:220` halves
stay at step 1.

### Phase 1 — BUILT (2026-06-14)

Built in the chokepoint-first order above.  The boundary matrix (`/tmp/claude/pln22`,
graduated to [`tests/scripts/369-pln22-shared-enum-variants.loft`](../../../../tests/scripts/369-pln22-shared-enum-variants.loft)
for the working cases + a `@EXPECT_ERROR` case in
[`102-expected-errors.loft`](../../../../tests/scripts/102-expected-errors.loft))
is green on **both** backends.

**The rule (owner decision, 2026-06-14):** a bare variant resolves as a VALUE only
from CONTEXT; defining a new untyped variable from a bare variant is a hard error
even when unique.  This SUPERSEDES the interim "ambiguity-tracking" build — which
kept no-context *unique* `s = Idle` working — because that very inference is the
friction the owner wanted gone (add a second enum with the name later → the bare
assignment silently re-points or breaks).  An enum VARIANT keeps a first-wins flat
`(name, source)` key so it stays reachable as a TYPE / constructor, but a bare
variant VALUE never resolves through that key.

**As-built sites:**
- `data.rs` — `variant_of(enum, name)` + `variant_in_source(source, name)` +
  `enums_with_variant(name)` chokepoints; `add_def` keeps EnumValue as a FIRST-wins
  flat key (no panic on a shared name); `rebuild_indices` mirrors it.
- `parser/objects.rs` — `parse_constant_value` resolves a qualified `Enum::Variant`
  via `variant_of(qualifier_enum)` and a `lib::Variant` via `variant_in_source`,
  but DEFERS a bare variant VALUE (no `{`-construction) to context; the flat
  `def_nr` branch excludes `EnumValue`; the context branches + a new
  `Type::Reference(enum)`↔`Type::Enum` branch resolve via `emit_variant_value`
  (the single emitter, plain discriminant or mixed-enum allocation); the resolver's
  error path emits a targeted "bare variant … qualify it as `Enum.X`" diagnostic and
  RECOVERS by resolving against the enum so no placeholder var poisons the second
  pass (without this, one bad site cascades into dozens of "Unknown variable").
- `parser/control.rs` — match arm / or-pattern / `is` route through `variant_of`;
  call-arg sets `enum_hint` from the param enum (BOTH passes); `parse_block` sets
  `enum_hint` from the block's expected type and SAVES/RESTORES it (so every
  `if`-branch tail of a typed-return fn sees it, not just the first).
- `parser/vectors.rs` — `parse_single` seeds the variant context from `var_tp` else
  `enum_hint` whenever `parent_tp` is not itself an enum (covers typed decl, `==`,
  call arg, return, struct-field init); `enum_context` helper.
- `parser/definitions.rs` — enum second-pass re-resolve via `variant_of`; struct
  field DEFAULT (`level: Level = Warning`) sets `enum_hint` from the field type.
- `parser/mod.rs` — `enum_hint` parser field (mirrors `lambda_hint`).

**Field-default work-ref aliasing — FIXED.** A mixed struct-enum unit-variant used
as a struct field DEFAULT (`sig: Sig = Idle`) is a self-contained `EnumUnitLit`
allocation block whose `Var(0)` is its own work-ref, not the record placeholder.
`object_init`'s `replace_record_ref(_, code)` was rewriting that `Var(0)` to the
struct's own ref, so the default's `OpDatabase` re-allocated the struct variable and
clobbered an explicitly-provided sibling field (`Widget { color: Red }` → wrong
`color`).  Fixed by re-homing such a default to a FRESH work-ref in the construction
context (regression: the `Cfg` case in 369).

### Phase 2 — prelude shadowing (M)

**BUILT (2026-06-14).**  Matrix + regression
([`tests/scripts/370-pln22-prelude-shadowing.loft`](../../../../tests/scripts/370-pln22-prelude-shadowing.loft);
the built-in-keyword reject in `102-expected-errors.loft`) green on both backends;
full suite green bar the 3 known-environmental.  As-built notes: **(a)** the
`fill_database` loop guarded struct/enum DB registration on the *bare name*, so a
shadowing `struct File` (second def of the name) was skipped and crashed with a
`u16::MAX` type-id at runtime — fixed to a PER-DEF guard (`known_type == MAX`), so
the P379 source-qualified registration actually runs for the second definer.
**(b)** `prelude_shadowed` must test namespace membership by NAME
(`source_nr(cur, name)`), NOT the found def's physical `.source`: a cross-file
forward-ref type (p173) lives in another file's source yet is imported into this
one's namespace, so the `.source`-based check false-positived and double-defined
it.  A user definition shadows a
stdlib / wildcard-imported name of the same key instead of being rejected, so
`enum E` / `struct File` / `enum Format` become legal — removing the "pick a
different name" wall — while the shadowed name stays reachable via qualification
(`std::E`).

**Guiding principle (the WHY): a simple script is namespace-less; a complex
program may redefine everything.**  A throwaway script writes bare `E`, `File`,
`sqrt(…)` with zero ceremony — it never writes `std::` and never thinks about
sources — because bare names resolve current-source-first with a **fallback to the
stdlib (`source 0`)**.  A larger program may define its own `File` / `Format` /
`E`; bare names then resolve to *its* definitions (current source wins), and
`std::Name` is the explicit escape hatch back to the prelude.  The SAME lookup
serves both — the only difference is whether you defined the name yourself — which
is exactly why the `source 1` model is chosen: it makes "redefine everything" free
(a fresh `(name, source)` key) and "namespace-less" free (the `source 0` fallback),
with no mode switch.

**The collision surface (measured).**  Top-level stdlib *constants* are only two,
both mathematical: `PI` and `E` (`E` = Euler's number, used by the stdlib's own
`exp`/`ln`; those bind it at stdlib-parse time so shadowing never affects them).
The larger, more valuable surface is stdlib **struct / enum** names a domain type
wants to reuse — `File`, `Format`, `JsonValue`, `ArgValue`, `StackFrame`, … .
Phase 2 is **scoped to const / struct / enum** shadowing; the **typedef
type-keywords** (`integer`, `text`, `vector`, `hash`, `i32`, `reference`, …) stay
a hard error — shadowing a built-in type is never intended and is a footgun.

**The root clash (the load-bearing finding).**  The stdlib *and the user's main
file are both `source 0`*, and `def_names` maps `(name, source) → one def_nr`.
Bare `E` (`def_nr`) and `std::E` (`source_nr(0, …)`) therefore resolve through the
**same `("E", 0)` slot** — it can hold the stdlib constant *or* the user enum, not
both, and `add_def` panics on the duplicate `(E, 0)` key.  That single shared slot
is the entire problem; nothing else clashes.

**Chosen fix — give the main file its own source (`source 1`; stdlib stays
`source 0`).**  The two `E`s then land in different slots, so bare and qualified
naturally diverge — no extra machinery, and it matches the plan invariant
(`(name, source)` key, current-source-first resolution):

| lookup | keys | result |
|---|---|---|
| bare `E` | `("E", 1)` then `("E", 0)` | user enum |
| `std::E` | `source_nr(0, "E")` → `("E", 0)` | stdlib constant |
| `add_def("E")` in main | inserts a fresh `("E", 1)` | no dual-key panic |

Two required changes: **(1)** assign the main file a non-zero source (after
`reset()` sets `source = 0`, set it for the first `!default` parse); **(2)** make
the four conflict checks **same-source** (`def(existing).source == self.data.source`)
— `parse_enum`, `parse_struct`, `parse_constant`, the typedef path
(`definitions.rs` ~351 / ~1845 / ~498 / ~413) — otherwise `def_nr` still falls
through to the stdlib `E` at `source 0` and rejects it.

**Rejected alternative — a shadow-map** (keep `("E", 0)` = stdlib; a separate
`shadows` table consulted only by *bare* resolution, bypassed by `std::`).  It
preserves `std::` too and is more localised, but it adds a parallel resolution
table every bare-name site must consult and a stdlib/user boundary
(`def_nr < first_user_def`) to tell "shadowing the prelude" from "redefining my
own def".  `source 1` is preferred: less standing complexity, same guarantee.

**Ripple risk to verify (why this is M, not S).**  Much of the compiler assumes
`source 0 = user code`.  Moving main to `source 1` must be verified against:
- **function overload resolution** (`find_fn`) — a user `fn sqrt` becomes
  `(n_sqrt, 1)` vs stdlib `(n_sqrt, 0)`; confirm dispatch still collects stdlib
  candidates;
- **native symbol naming** (`n_<name>`, the P379 source-qualified rename) and
  **database type registration** (`qualified_type_name`, which keys on
  `def.source`) — user types' generated symbols / DB keys shift source;
- **import source numbering** — libraries currently number up from 0; main taking
  1 means libs start at 2.

None look broken by construction, but these are where a subtle regression hides —
so the build verifies the Phase-2 matrix (`/tmp/claude/p2`, graduate to
`tests/scripts/`) **plus the full suite on both backends** (native / DB / import
tests especially), not just the enum matrix.

- **Shadow direction:** user def wins in its own source; `std::Name` reaches the
  shadowed stdlib one.  (Open question #3 is settled by the scope above.)

### Phase 3 — `use … as …` aliasing — BUILT (2026-06-14)

Added the `as` clause to the `use` grammar.  Three forms, all green on both
backends (fixture [`tests/lib/p3_alias_main.loft`](../../../../tests/lib/p3_alias_main.loft),
test `imports::pln22_phase3_use_as_aliasing`):

- **library alias** `use enumlib as el;` → `el::make()` — a qualifier alias.
- **type alias** `use enumlib::Status as St;` → bare `St` (and `St.Yay`).
- **function alias** `use enumlib::make as mk;` → `mk()`.

This is the escape hatch Phases 1/2 leave open (two libs export `parse`; a name
you want bare under a non-colliding spelling).

**As-built semantics + sites:**
- `ImportSpec::Names` carries `(name_in_library, bind_name)` pairs (`bind == name`
  unless `as`).  `import_name` / `import_name_overwrite` gained a `bind` param: the
  lib LOOKUP uses `name` (+ `n_name`), the BIND uses `bind` (+ `n_bind`).
- `use lib as m;` registers the alias via the new `Data::use_alias` (a qualifier
  alias on `use_names`, no new source) — and is **qualified-only**: unlike plain
  `use lib;` it does NOT wildcard-import (so bare names aren't polluted; that is
  the disambiguation point).  An explicit `::` spec is still honoured.
- Parsing (`parser/mod.rs` use-region): `as` after the lib id → library alias;
  `as` after a `::`-spec name → per-name bind alias.  Both register/bind in the
  two-phase load (first-load + the `use_exists` re-parse).

### Phase 4 — selective-import syntax — BUILT (2026-06-14)

Dropped the flat top-level comma list (`use lib::a, b, c;`), which read poorly
(`b`/`c` didn't visually bind to `lib::`).  Now:

- **Single**: `use lib::name [as bind];` — unchanged.
- **Grouped**: `use lib::(a, b, c);` — `()` (loft's existing arg-list delimiter)
  binds the names to `lib::`; per-name `as` aliases work inside (Phase 3 reused):
  `use lib::(a as aa, b, c as cc);`.
- **Flat list dropped**: `use lib::a, b;` errors ("import multiple names with
  parentheses: `use lib::(a, b, …)`") and still binds the names (recovery), so the
  rest of the file parses.
- `use lib;` (wildcard) and qualified `lib::fn()` (auto-`use`) still cover the
  common cases.

**As-built:** one parsing change in the `use` region — after `::`, an optional `(`
marks the group; the existing Phase-3 name loop parses `name [as bind]` entries; a
grouped spec requires the closing `)`, an un-grouped spec with >1 name is the
dropped flat list (error + recovery).  Migrated the sole flat-list `.loft` site
(`tests/fixtures/libs/game_protocol/tests/protocol.loft`).  Tests:
`tests/lib/p4_group_main.loft` (grouped + aliases, both backends),
`imports::pln22_phase4_grouped_import` + `pln22_phase4_flat_list_rejected`.

## Open questions

1. **Variant storage shape** — reuse `attr_names` (variants as enum attributes),
   or a dedicated `variant_names` map keyed `(enum_def, name)`? Settle in the
   Phase-1 design round; `attr_names` reuse is the leaner hypothesis.
2. **Bare variant with NO context** (`x = Up`, untyped) — RESOLVED (Phase 1
   build, owner decision): it is ALWAYS a hard error, even when `Up` is currently
   unique → qualify it (`x = Dir.Up` / `x: Dir = Up`).  The original premise
   ("already an error") was inaccurate (it used to infer the enum, and ~16 tests +
   2 doc/lib fixtures relied on that), but the owner chose to make it an error so
   adding a second enum with the variant name can never silently re-point an
   existing bare assignment.  Those fixtures were migrated to the qualified form.
3. **Shadowing scope** — SETTLED (Phase 2 design, 2026-06-14): shadow stdlib /
   wildcard-imported **const / struct / enum** names; built-in type-keywords
   (`integer`, `vector`, …) stay a hard error.  Mechanism: give the main file its
   own source (`source 1`) so `(name, source)` keys no longer collide with the
   stdlib's `source 0`.  See Phase 2.
4. **1.0 sequencing** — the `(enum, variant)` key change is observable (two-enum
   programs that panic today start compiling); land before the 1.0 stability
   contract.

## Cross-arc dependencies

- **[PACKAGES.md](../../PACKAGES.md)** — Phase 3/4 touch the `use` grammar; keep
  the library-author docs in step.
- **1.0 stability contract** ([ROADMAP.md](../../ROADMAP.md)) — Phase 1 before 1.0.

## See also

- DESIGN_DECISIONS.md — record the selective-import syntax decision (drop flat
  comma list; one-per-line idiom + optional `::(…)`) and the shadow direction.
- C53 (`tests/lib/match_lib_enum_main.loft`) — the existing bare/`Enum::`/`lib::`
  variant-resolution coverage Phase 1 must keep green.
