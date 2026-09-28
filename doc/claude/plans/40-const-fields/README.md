<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN40 — `const` struct fields (write-once-at-construction)

Extend the existing `const` keyword from locals + parameters to
**struct fields**, giving loft a "frozen after construction" field
modifier.  Closes the locals-vs-fields asymmetry surfaced in
[INCONSISTENCIES.md § 33](../../INCONSISTENCIES.md#resolved-as-design-point-documented--regression-guarded).

## Status

**DONE — all 8 steps shipped (2026-07-16).**  `const` struct (and enum-variant)
fields are implemented, enforced, persisted, documented, and dogfooded.  A field
marked `const` is write-once at construction; a later `t.field = …` reassignment is
a compile error at the parse-time write guard, and construction is exempt for free
(it lowers via `Value::Insert`, a separate path).  const freezes the binding, not
contents.  No new opcodes, no runtime change; the only store change is one
round-trip bool.  Landed on `tuxedo-work` (steps 1–8), pending a PR.

One follow-up spun off (independent of const): the literal-field expected-type
enhancement — see [literal-expected-type.md](literal-expected-type.md).

The parser had **already recognised `const` in struct-field position and rejected
it** (`@P386` guard, `src/parser/definitions.rs`), so the feature was an additive,
opt-in change: no existing program could contain a `const` field, so enforcement
only ever fires on newly-written code.

## Goal

```loft
struct Token {
  const id:        integer,   // set once at construction, non-null by default
  const issued_at: integer,
  payload:         text       // mutable, as today
}

t = Token { id: 42, issued_at: 1_000_000, payload: "" };
t.payload = "hello";          // OK
t.id = 99;                    // ERROR: cannot reassign const field 'id'
```

The constraint is **purely static** — the runtime layout is unchanged, the field
read/write ops (`OpGetInt`, `OpSetInt`, …) are unchanged.  Enforcement is the
rejection of a field-write whose target attribute is `const`, at the same
parse-time hook that already rejects writes to key fields.

> **Syntax note (2026-07 refresh).**  `not null` is now **deprecated and a
> no-op** — a field type is non-null by default; write `T?` for a nullable field
> (`src/parser/definitions.rs:11`).  The examples below use the current syntax.
> The original plan (2026-05-11) was written before that change and used
> `const id: integer not null` throughout; that spelling still parses (the
> `not null` is a warned no-op) but is no longer the way to write it.

## The one design decision: a dedicated `const_field` flag

`Attribute` (`src/data.rs:2384`) already has two boolean flags that look reusable
but are **both taken**, and reusing either would break unrelated code:

| Flag | Current meaning | Why it can't carry const-field |
|---|---|---|
| `mutable` (default `true`) | `false` marks a **key field** (`set_mutable`, `typedef.rs:931`) *and*, on operator/method attributes, a bytecode-constant param that codegen **skips pushing** (`data.rs:4026` hazard note; `codegen.rs:2763`/`3020`; `create.rs:208`). | Setting `mutable=false` on a data field would (a) reuse the "key field, create a record instead" message and (b) risk the construction/codegen path skipping the field's value. |
| `constant` (default `false`) | `true` marks a **`virtual`/`computed` field** — "only return the default" (`definitions.rs:2880`). | Already means computed-no-storage; a `const` field has real storage written once. |

So add a new, orthogonal flag: **`Attribute.const_field: bool`** (default `false`).
It records exactly one fact — "this field is write-once" — read at exactly one
kind of site (the field-write guard).

## Where it plugs into the existing code

Four short touch-points, each already has a sibling doing almost the same thing:

1. **Representation** — `Attribute` in `src/data.rs:2384`; default set in
   `add_attribute` (`data.rs:3627`, where `mutable:true, constant:false` are set).
2. **Parse (struct)** — `parse_struct` field loop, `src/parser/definitions.rs:2461`.
   The `const` keyword is consumed today at line 2467.  Set the flag after
   `parse_field` returns, mirroring how the enum loop applies `#lexeme`
   (`definitions.rs:322`).
3. **Enforce** — `validate_write` (`src/parser/expressions.rs:3449`), called from
   the field-assignment path (`expressions.rs:1402`, guarded `var_nr == u16::MAX`).
   It already maps a write's byte-offset back to the field
   (`f.position == pos`) and rejects `!mutable` (key) fields.  A const-field check
   is one more `else if` there — it has the attribute identity the raw op-emit
   path (`call_to_set_op`, `operators.rs:397`) lacks (that path sees only
   `host_ref` + byte offset).
4. **Construction is automatically exempt** — a struct literal lowers via
   `Value::Insert` / `set_field_no_check` (`src/parser/objects.rs:2458`, `2831`),
   a **different** code path that never calls `validate_write`.  "Write-once-at-
   construction" falls out for free: the guard lives on the reassignment path only.
   (Step 4 still proves this with the matrix — it is a claim to verify, not assume.)

The precedent to copy is the existing const-**parameter** enforcement: the flag
`Variables::is_const_param` (`variables/mod.rs:1537`), the shared message
`"Cannot modify {} '{}'; remove 'const' or use a local copy"`, and — most
relevant — the constructor-into-const guard that branches on
`matches!(code, Value::Insert(_))` (`expressions.rs:2422`).  Const fields need the
field-keyed analogue of exactly that logic.

## Implementation — safe small steps

Each step compiles and keeps the full suite green on its own, so each is a
separate commit.  Steps 1–2 are inert (a flag nothing reads yet); step 3 only
*relaxes* an existing error; step 4 adds enforcement that can only fire on the new
syntax.  The risk is back-loaded and opt-in.

| # | Step | Where | Verify | Why safe / reversible |
|---|---|---|---|---|
| 0 | **Baseline the boundary.**  Write the positive + negative `.loft` probes (see Test strategy) and run them on **both backends**.  Today every `const`-field program fails at the declaration (`@P386`).  Record that — it proves the harness can fail. | `tests/scripts/40-const-fields.loft` (staging in `probes/` first) | All probes error at parse today | No code change |
| 1 ✅ | **Add the flag.**  `Attribute.const_field: bool`; default `false` in `add_attribute`; add to the `#[allow(clippy::struct_excessive_bools)]` note. **Note:** `Attribute` has no `Default` impl, so every `Attribute { … }` literal (9: `add_attribute`, `ir_read`, `ir_schema` JSON + 5 test constructors, `ir_store` test) must set the field or it won't compile. | `src/data.rs` | `make ci` green | A new `false` bool nothing reads is inert |
| 2 ✅ | **Round-trip the flag.**  The store schema is **generated** — edit the source `tools/ir_schema/ir.loft` AND hand-apply the matching `db.field(t139, "const_field", t4)` to the generated `src/ir_schema_gen.rs` (a wholesale regen drifts the `tN` ids — follow the `links` precedent already in that file).  Register it **LAST among the bools** so it packs at the next free offset (`ATTR_CONST_FIELD = 46`) and every prior offset stays put; bump `ATTRIBUTE_STRIDE` 46→47 (byte-offset constants + the `baked_layout_mirrors_loft_schema` assert live in `src/data_store.rs`).  Write in `write_attribute`; **read in `read_attribute`** (`src/ir_read.rs` — NOT `materialize_attributes`, which is the write side); JSON writer + tolerant parser + golden string in `src/ir_schema.rs`.  Make the round-trip test **non-vacuous** (build `const_field: true`, assert it survives). | `tools/ir_schema/ir.loft`, `src/ir_schema_gen.rs`, `src/data_store.rs`, `src/ir_store.rs`, `src/ir_read.rs`, `src/ir_schema.rs` | `baked_layout_mirrors_loft_schema` + stdlib store round-trips green; a save→reload keeps the bit | Round-trip of an always-`false` value is identity, so inert.  **No cache-format bump needed:** `BUILD_ID` (git HEAD) is in the cache key (`src/cache.rs`), so any rebuild invalidates stale caches — a stride change can't collide with old data.  Required so a **cached library struct** keeps its const-ness |
| 3 | **Accept + store the keyword.**  At `definitions.rs:2467` replace the `@P386` rejection with `let is_const = self.lexer.has_keyword("const");`, then after `parse_field` (`:2490`) set `attributes[idx].const_field = is_const`.  Reject `const virtual(...)` here (virtual already implies no-write).  Apply the same to enum-variant fields (`parse_enum_values` loop, `definitions.rs:286`). | `src/parser/definitions.rs` | `const x: integer` now parses and runs; the flag is set (unit assert on the `Attribute`); `const virtual(...)` errors | Only behaviour change is `const` no longer erroring — the intended direction.  Nothing reads the flag yet, so no write is rejected → zero false positives |
| 4 | **Enforce reassignment.**  Extend `validate_write` (`expressions.rs:3449`) with an `else if …const_field` arm emitting `"cannot reassign const field '{f}' of struct '{T}' — const fields are write-once-at-construction"`.  Use the **boundary matrix** (below) to find every write route that must be covered; add the field-keyed guard on any route `validate_write` doesn't reach (e.g. collection-field reassignment via the `towards_set` collection branch, `collections.rs:794`). | `src/parser/expressions.rs`, possibly `src/parser/collections.rs` | Full matrix: every negative cell errors, every positive cell (construction + read) still works, **both backends** | Fires only when `const_field==true`, which only step-3 code can set → no existing program affected.  This is the load-bearing step; gate it on the written matrix, not the suite alone |
| 5 ✅ | **Construction — keep loft's default-fill (decided: no init requirement).**  A `const` field follows the SAME construction rules as any field: an omitted field takes its default (an explicit `= expr`, else the type's zero value), a value in the literal overrides it, and the field is write-once from that point.  Probed (both backends): omitted `const x: integer` → `0`, `const x: integer = 8080` omitted → `8080` / overridden → `9000`.  **No `object_init` change** — the earlier plan assumed a "non-null field must be provided" check exists; it does not (loft zero-fills), and the owner's decision is to keep default-fill, NOT add a const-specific "must initialise" rule. | none (test + doc only) | `pln40_const_field_default_and_override` | Reuses loft's existing default-fill unchanged; const only adds the reassignment ban |
| 6 ✅ | **Document.**  `const` in the field-modifier list (a prefix modifier — freezes the binding, not contents; `const virtual` rejected). | `doc/claude/LOFT.md § Fields`, `.claude/skills/loft-write/SKILL.md` | drift check clean | Docs only |
| 7 ✅ | **Dogfood — `hex_world.Cell`** (`loft-libs-world`, the rebuild-via-construction grid).  Marked `c_color`/`c_height`/`c_age` `const`.  const immediately flagged `world_load`'s construct-then-fill (`.c_color = f#read` ×3) — the exact in-place-mutation footgun; collapsed it into one `Cell{…}` literal.  The tick loop's `icells[i] = Cell{…}` rebuild is unaffected.  All 16 lib tests (incl. save/load round-trip) green on **both backends**.  See the consumer survey below. | `loft-libs-world/hex_world/src/hex_world.loft` | 16/16 tests green, interp + native | Proves const catches real in-place mutation on a live tick loop; harvested a language lesson (below) |
| 8 ✅ | **Close the gap.**  INCONSISTENCIES § 33 flipped to resolved (Status marker + resolved-table row; `doc_hygiene` green).  Positive matrix graduated to `tests/scripts/40-const-fields.loft` (interp/native/wasm); negative cells are the `pln40_const_*` parse-error tests in `tests/issues.rs`. | `doc/claude/INCONSISTENCIES.md`, `tests/scripts/40-const-fields.loft`, `tests/issues.rs` | `doc_hygiene` + both-backend script run green | Closes the loop; regression guard lands with the feature |

**Total effort:** M.  No new opcodes and no runtime/codegen change; the only
store change is the one round-trip bool in step 2 (a stride bump on the
generated IR schema, safe because `BUILD_ID` invalidates stale caches).

## Enforcement surface — the boundary matrix for step 4

The check must fire on **reassignment** and stay silent on **construction and
read**, for every field type and every write route.  Build this in throwaway
`/tmp` probes on `--interpret` first, hand-computing each expected cell, then
verify on `--native`:

| Field type | Construct (must PASS) | Read (must PASS) | Reassign field (must ERROR) | Mutate contents (out of scope — must PASS) |
|---|---|---|---|---|
| `const x: integer` | `T{ x: 1 }` | `t.x` | `t.x = 2` | — |
| `const s: text` | `T{ s: "a" }` | `t.s` | `t.s = "b"` | — |
| `const r: OtherStruct` | `T{ r: O{…} }` | `t.r.f` | `t.r = O{…}` | `t.r.f = 5` (f not const) |
| `const v: vector<integer>` | `T{ v: [1] }` | `t.v[0]` | `t.v = [2]` (reassign only) | `t.v[0] = 9` AND `t.v += [9]` (append — both contents) |
| `const x: integer?` (nullable) | `T{}` (takes its default) | `t.x` | `t.x = 1` | — |

Write routes that must all hit the ERROR cells (the plan's "fires regardless of
access route"): direct (`t.x = …`), nested (`box.t.x = …`), through a `&`
parameter (`fn f(t: &T){ t.x = … }`), and a closure capture (`|| t.x = …`).  Each
lowers the LHS through the same field-assignment path, so `validate_write` should
see them all — **confirm, don't assume**.  A no-error cell is vacuous unless a
sibling cell proves the probe can error.

## What `const` does NOT cover

- **Contents through a const collection/struct field.**  `const v: vector<integer>`
  freezes the *binding* (`t.v = […]` reassign rejected) but not the contents:
  `t.v[0] = 5` (element write) AND `t.v += […]` (append) are both allowed — a `+=`-grown
  accumulator field IS const-able (implemented via const-append-contents.md; a const
  *scalar* still rejects `+=` since it has no contents).  Do **not**
  put the check in `set_field_check` (`mod.rs:5227`): that chokepoint is shared by
  construction, element deep-copies, and ~20 compiler-synthesised loop/vector
  writes, so `emit_check` is not a clean "user re-assignment" signal.
- **Mutation through `Reference<T>`** — `Reference<T>` bypasses normal field-write
  checks (it points into the heap).  Const stays a static, zero-cost check; it does
  not extend to reference writes.  Document as an explicit limitation.
- **Enum-variant match bindings** (`if shape is Circle { radius }`) are already
  read-only locals — const-by-construction, no annotation needed.

## Decisions

- **Must a const field be initialised at construction?**  **No (owner decision,
  step 5).**  A `const` field follows loft's standard default-fill: omitted → its
  default (explicit `= expr`, else the type's zero value); a value in the literal
  overrides it; the field is write-once from that point.  loft keeps its default-fill
  semantics unchanged — `const` adds only the reassignment ban, no construction-time
  "must initialise" rule.  (The earlier design proposed requiring init; probing showed
  loft zero-fills *all* omitted fields, and the owner chose to keep that.)
- **Should a const field auto-imply non-null?**  No — orthogonal.  Fields are
  already non-null by default; `const x: integer` is non-null + write-once,
  `const x: integer?` is nullable + write-once.  No coupling.

## Open questions (not blocking)

- **`pub const field`?**  Struct fields have no visibility modifiers today (`pub`
  on a field is consumed and ignored, `definitions.rs:2462`).  Out of scope.
- **Const on enum-variant fields?**  In scope — step 3 covers the variant loop
  alongside the struct loop.

## Test strategy

Positive shapes (run + assert), negative shapes (`@EXPECT_ERROR`), plus the real
consumer from step 7.  Current syntax (no `not null`):

```loft
// p386_const_field_constructed_and_read
struct Token { const id: integer }
t = Token { id: 42 };
assert(t.id == 42, "read after construction");

// p386_const_field_with_default
struct Cfg { const port: integer = 8080 }
c1 = Cfg {};
c2 = Cfg { port: 9000 };
assert(c1.port == 8080 && c2.port == 9000, "default + override");
```

```loft
// p386_const_field_reassign_rejected   @EXPECT_ERROR
struct Token { const id: integer }
fn main() { t = Token { id: 1 }; t.id = 99; }

// p386_const_field_reassign_via_ref     @EXPECT_ERROR
struct Token { const id: integer }
fn touch(t: &Token) { t.id = 99; }

// p386_const_virtual_rejected           @EXPECT_ERROR
struct Bad { const v: integer virtual($.x * 2), x: integer }
```

## Consumer survey — where const fields apply

Surveyed every known library + consumer (`loft-libs-core`/`game`/`graphics`/`world`/`net`,
`crawler`, `routing`, and the in-repo `audience_crystal`/`engine_host`) for fields that are
set at construction and never reassigned via `.field =`.  Strong prior signal: `const` is
**already a parameter qualifier** in shipping libs (`imaging`'s `save_png(self: const Image)`,
crawler's `const Region`/`const Hydro`) — const fields complete the immutable-value story.

Five recurring situations that call for const fields:

1. **Pure value / record types → const the whole struct.**  Geometry (`Vec2/3/4`, `Mat4`,
   `Rect`, `Circle`, `Pixel`, `GeoPoint`, `BBox`), content/DB rows (`MonsterDef`, `ItemDef`,
   `TerrainType`, `Way`, `GEdge`), result records (`HttpResponse`, `MatchResult`, `Decoded`,
   `HpkeSealed`, `Event`).  Built once, only read.
2. **Identity fields on otherwise-mutable "god objects" → where const catches the most bugs.**
   `Sim.wseed`, `Enemy.key`/`maxhp`, `Server.handle`, `WebSocket.ws_id`,
   `GameEnvelope.sequence`, `Chunk.ck_cx`/`ck_cz`, `TerrainParams.tp_seed`.  (The entire
   `loft-libs-net` src has zero field mutations — risk-free.)
3. **Rebuild-via-construction records → const enforces "rebuild, never mutate in place."**
   `hex_world.Cell` (the step-7 dogfood), `audience_crystal.ChunkMeshEntry`.
4. **Fixed configuration set once.**  `CrystalIncr.group_dim`, `TerrainParams` (16 config
   fields), `Renderer` dims/shaders, `Args.name`/`version`.
5. **Container-binding const** (vector/map grown via `+=`/`[i]=` but never rebound — const the
   binding, contents still mutate).  `Scene.meshes`, `Graph.nodes`/`edges`, `ck_cells`.  Lower urgency.

**Friction cases** (const forces a small refactor — the "earns its keep" test): construct-then-fill
idioms that must collapse into a literal — `hex_world.world_load` (done, step 7), `Canvas.data`,
`routing_kernel.Graph.build_adj` (rebinds `adj_*`).  Genuine mutation to leave non-const: `World.tick`,
`Material.set_color`, `Heap.size`, RNG state (`RandStream`, `Rng`).

### Harvested lesson (step-7 dogfood)

Collapsing `hex_world.world_load`'s per-field fills into a `Cell{…}` literal exposed an
**expected-type asymmetry**: an assignment LHS pushes the field's declared type into its RHS
(`cell.c_color = f#read` reads a `u8`), but a **struct-literal field position does not** — a bare
`Cell{ c_color: f#read }` infers `text` and errors.  Worked around with explicit `f#read as u8`/`as u16`.
The underlying gap — propagate a literal field's declared type as the value expression's expected type —
is a language-enhancement candidate that const adoption surfaces (const removes the LHS-driven escape hatch).
Designed (root cause + safe small steps) in
[literal-expected-type.md](literal-expected-type.md); it is independent of `const`
and should become its own plan / `loft-lang/features` issue before build.

## See also

- [INCONSISTENCIES.md § 33](../../INCONSISTENCIES.md#resolved-as-design-point-documented--regression-guarded) — the gap this closes
- `@P386` / `src/parser/definitions.rs:2467` — the existing parser guard that is the landing site
- `@F18` — the const-**parameter** feature (`Argument.constant`) whose enforcement shape this mirrors
- [LOFT.md § Field modifiers](../../LOFT.md) — current modifier list (to be extended in step 6)
- `.claude/skills/loft-write/SKILL.md` — user-facing field-modifier reference (step 6)
- A `lib/*` grid/cell consumer with the rebuild-via-construction tick loop — the step-7 dogfood target (pick the current one at build time; the historic pointers `lib/world` and `lib/hex_world` have both moved)
