<!-- reference for the loft-write skill: read when a struct field carries a modifier -->
# loft-write: field modifiers reference

The field-modifier half of the [loft-write skill](SKILL.md).

## Field modifiers (struct fields only)

<!-- from tests/reference/skill-declarations.loft -->
```loft
struct Point {
    const id: integer,           // binding-const PREFIX: write-once at construction
    const cells: vector<Cell>,   // BUILDER: slot write-once, `cells += […]` still grows it
    coord: const Coord,          // value-const on the TYPE: frozen value (contents read-only)
    x: float,                    // non-null by default (DN1) — no `not null` needed
    r: integer limit(0, 255),
    label: text = "default",
    area: float virtual($.x * 2.0),
    note: text?,                 // append `?` to ALLOW null
}
```

Modifiers: `limit(min, max)`, `default(expr)` / `= expr`, `virtual(expr)`.

**Two const axes** (@PLN40) — `const` before the NAME freezes the *binding*; `const` before the
TYPE freezes the *value*.  They are opposites and compose:

| declaration | rebind `t.v=` | append `t.v+=` | element `t.v[i]=` | use |
|---|---|---|---|---|
| `v: T` | ✓ | ✓ | ✓ | plain |
| `const v: T` | ✗ | ✓ | ✓ | **builder** (grow in place) |
| `v: const T` | ✓ | ✗ | ✗ | **frozen value** (rebindable slot) |
| `const v: const T` | ✗ | ✗ | ✗ | fully immutable |

- **binding-const** (`const v: T`, prefix): write-once slot, contents stay mutable — the builder
  shape.  Reach for it when a field is assembled once then grown/edited in place (`const cells:
  vector<Cell>` with `cells += […]`).  `const virtual(...)` is rejected; works on enum-variant fields.
- **value-const** (`v: const T`, on the type): the value is read-only — append / element / nested
  writes (`t.v+=`, `t.v[i]=`, `t.r.x=` through it) are rejected; a whole-value rebind `t.v = other`
  is allowed.  Reach for it on a genuine value/record (shared config, a compound key, a `par()`-shared
  value).  A write through a local bound to it (`w = t.v; w.x = 5`) is refused too.
- **scalar collapse**: a by-value scalar (`integer/float/…`) freezes fully under EITHER axis —
  `const n: integer` and `n: const integer` both reject `t.n=` and `t.n+=`.

**Owner policy** (libraries): const every never-reassigned field.  Use `const v: T` for builder fields
you still append to; `v: const T` for true value/record fields; whole-value keys/config → both.
