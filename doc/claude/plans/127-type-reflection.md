<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->
# @PLN127 — Type reflection from loft code

## Status

**DONE — shipped 2026-08-05.**  Arcs A–E built, both backends.

`type_of(x)` and `type_named(name)` hand a loft program the declared shape of a type as
data.  The user-facing reference is [STDLIB_RUNTIME.md § Reflection](../STDLIB_RUNTIME.md); the catalogue
entry is [`@F107`](https://github.com/loft-lang/features/issues/107).  The API is declared
in [`default/07_reflect.loft`](../../../default/07_reflect.loft), filled by
`native::reflect_type_into` and `native::type_named_in`, and gated by
`tests/scripts/pln127-reflect.loft` (the type-space matrix — hand-checked byte offsets, an
enum whose tags start at 1 because 0 is how the store spells ABSENT, a struct-enum variant,
a nested record, a vector's element, all five scalars) and
`tests/scripts/pln127-reflect-consumer.loft` (an ORM's schema half, written only through
the API).

**Issue:** [loft-lang/plans#127](https://github.com/loft-lang/plans/issues/127).

One structural decision carried the whole plan: **one filler serves both backends** — the
interpreter through `src/native.rs`, `--native` through a `codegen_runtime` wrapper onto the
SAME function.  A second implementation there would have been exactly the drift this plan
existed to prevent, because reflection must not become a second, disagreeing description of
a layout @PLN97 already pins.

---

## Closure record

The reference content lives in STDLIB.md.  What follows is what the build SETTLED — the
decisions and the measured surprises, which is the part no reference doc records.

### Arc D — nullability yes, `const` no

The plan asked whether `LayoutDesc` should grow `const`-ness and non-narrow nullability, or
read them from a second source.  Measuring found neither was the real choice, because
**neither is a storage fact**: `text?` and `text` share a content type and spell absence
with a SENTINEL, so nothing in the stored bytes implies either.  (A NARROW int is the
exception — it registers a distinct content type per nullability, which is why the
descriptor already reported nullable for those and only those.)  So D was really: *does
reflection report facts that exist only in the source?*

**The line drawn: what a VALUE can be, yes; what CODE may do to it, no.**  Nullability is
the first kind and is load-bearing — arc E's generator was complete, correct, and could not
emit `NOT NULL`, which does not make a DDL less detailed, it makes it accept rows the loft
type would refuse.  `const` is the second kind: it constrains loft code, not data, so an ORM
has no use for it, and admitting it would make reflection a mirror of the source text.

Built as a deposit at the one parse-time site that knows (`typedef.rs`, where `Optional(τ)`
is peeled), **replayed by the native generator**, carried by `LayoutField` and *not*
rendered.

Three things this cost that reading the code would not have shown:

- **`--native` reported `nullable=false` for every field** until the generator emitted the
  deposit too, because it rebuilds the schema by REPLAYING `init()`.  The parity probe
  caught it; the fix wraps `emit_field` rather than sitting in either caller, because there
  are two call sites and the one that mattered was not the obvious one.
- **The IR-store round trip had to carry it too**, or a schema read back from a store
  answered "not nullable" for every field.  That grew `DbField` by a byte, which needed
  `CACHE_FORMAT_VERSION` bumped — the stdlib cache key does not fold in the binary's mtime,
  so a cache written at the old stride was read at the new one and panicked.  25 LSP tests
  failed on that one cause.
- **The @PLN97 layout identity is unchanged, and that is measured rather than argued.**  A
  store written by the pre-arc-D binary loads under the arc-D binary through both the
  whole-image and keyed paths with `ok=true`, and the same gate still REFUSES a genuinely
  reshaped layout.  `layout_hash` hashes `render_dump()`, and nullability is carried there
  but never rendered.

### Reflection inside a generic is not reachable this way

A generic body is parsed ONCE against its type variable, so `type_of(v)` there answers
`__typevar_T`.  The same mechanism makes `"{v:j}"` in a generic body render `{}`.  Reaching
it needs the body parsed per instantiation, which is a different plan — so the doc comment
states the limit rather than implying otherwise.  Call reflection where the concrete type is
known.

### Three more the build settled that the design had not

- **Q1 dissolves for `type_of`.**  The type id is resolved where the call is WRITTEN, so it
  is a parse-time constant — the mechanism `to_json` already uses, and `--native` replays
  the type table rather than minting it, so a parse-time id is replayed with it.  Q1 was
  only ever load-bearing for a name given at RUNTIME, which is arc C's question, and arc C
  answered it: `Stores::name` is a TOTAL lookup that reports absent rather than minting a
  type for a typo.
- **Q3 answers itself for two scalars, and only two.**  `get_type` — the one existing
  storage derivation — reports `integer` for a `character` and has no entry at all for a
  `boolean` (`#65535`).  Reflection names those two directly and keeps the single derivation
  for everything else, because a second derivation is a second thing to drift.  Narrow ints
  still report storage, and `size` is where that shows.
- **Arc A was a prerequisite in fact, not just in order.**  A `TypeInfo` holds an enum in a
  struct field, which is precisely the shape that made `json_parse` reject a whole document.
  `"{t:j}"` on a `TypeInfo` renders complete JSON only because arc A landed.

### Arc A — the workaround was unavailable, not merely awkward

Both defects were WHOLE-DOCUMENT failures, so a struct holding either shape could not be
read back at all.  Fixed in `src/database/format.rs`, seven cells in
`tests/scripts/57-json.loft` on both backends, each proven able to fail first.

- **[loft#768](https://github.com/loft-lang/loft/issues/768)** — an enum-TYPED position
  wrote its tag bare, which is not JSON.  Two writers render an enum and only one knew about
  JSON: `Parts::EnumValue` wrapped as `{"Circle":{…}}`, `Parts::Enum` did not.  The typed
  position now wraps the same way, so writer and reader name ONE shape between them.
- **[loft#769](https://github.com/loft-lang/loft/issues/769)** — an absent `text?` is stored
  as the sentinel `"\0"`, not a null pointer, so it reached the escaper and came back as a
  present one-character string.

### Left out on purpose

**Write access.**  Reflection describes a TYPE; there is no way to read a VALUE's field by
name.  Arc E found this from the read side — it had to be a schema generator rather than the
"generic serialiser" the plan imagined, because a serialiser needs both halves.
Constructing or mutating a value by field name is a separate and larger question.

## See also

- [STDLIB_RUNTIME.md § Reflection](../STDLIB_RUNTIME.md) — the API reference.
- [`src/database/descriptor.rs`](../../../src/database/descriptor.rs) — `LayoutDesc`, the
  substrate, shipped with @PLN105; this plan is its loft-side sibling.
- [BROWSER_INTEROP.md § The binary bridge](../BROWSER_INTEROP.md) — the JS reader that
  consumes the same descriptor.
- [`default/04_stacktrace.loft`](../../../default/04_stacktrace.loft) — the precedent for
  the answer's shape (`@F88`).
