<!-- reference for the loft-write skill: read for struct-enum dispatch, operators on your own type, format specs, or a match beyond a plain enum -->
# loft-write: enums, operators, format specs and match

The expression half of the [loft-write skill](../SKILL.md).  Read it when you declare a
struct-enum with per-variant methods, put an operator on your own type, compare for identity,
tune a `{…}` format spec, or write a `match` on a tuple, a payload or a slice.

## Struct-enums and per-variant dispatch

Each variant has fields; polymorphic dispatch via methods:

<!-- from tests/reference/skill-declarations.loft -->
```loft
enum Shape {
    Circle { radius: float },
    Rect   { w: float, h: float },
}
fn area(self: Circle) -> float { 3.14159 * self.radius * self.radius }
fn area(self: Rect)   -> float { self.w * self.h }
```
<!-- from tests/reference/skill-declarations.loft -->
```loft
s = Circle { radius: 2.0 };
a = area(s);   // dispatches to correct variant
```

A plain enum takes methods too (`fn label(self: Color) -> text { match self { … } }`);
struct-enum variants add per-variant dispatch.  Trailing commas in variant field lists are
accepted: `Circle { radius: float, }`.

JSON round-trip: `"{shape:j}"` produces `{"Circle":{"radius":3.14}}`; `Shape.parse(json)`
reconstructs the correct variant.

## Operators on your own type

Write `operator` in place of `fn`, `self` first, in the type's own package only:
`compare` → `< <= > >=` (answers `Ordering`); `plus minus times divided_by remainder negate` →
`+ - * / %` and unary `-` (`a += b` is `a = a.plus(b)`); `to_<type>` → `as`; `to_text` /
`next` → `"{x}"` / `for`.  `==` / `!=` stay structural for every type, and a plain `fn` named
like one drives nothing — nor does a user `fn Op…`: `OpXxx` names are the stdlib's operator
table only (@C132).  `[…]` is the built-in collections' (a type reads through a named method,
`x.at(i)`).  Full list and rules: LOFT.md § Operators on your own type, `formal/operators.md`,
feature @F124.

## `==` compares content; `&a == &b` asks identity

**`==` / `!=` compare CONTENT for every type** — structs, vectors, tuples, struct-enum values,
keyed collections, recursively through fields and `reference<T>`, cycles included
(DESIGN_DECISIONS_VALUES.md § @C91).  Two separately built records with equal fields are `==`;
`[P{x:1}] == [P{x:1}]` is `true`.  To ask whether two names are ONE record, write `&a == &b` —
`&` on both sides; on one side only it is a compile error.  There is no `===`.  Keyed
collections hash and compare keys the way `==` does (`0.0` and `-0.0` are one key).

## Format specs

**A format spec tunes what the value RENDERS as, so it works on every type**, not just numbers
and text: `{c:>5}` pads a character, `{v:>12}` a vector, `{p:*^16}` a struct.  The flags that
choose the rendering (`#`, `:j`) are separate from width and alignment and combine with them.
Zero-padding is the exception: `0N` pads a NUMBER (`{-1:04}` is `-001`) and is a compile error
on anything else — use a fill instead (`{b:*>6}`).  Inside a `<T: Printable>` generic a hole
renders as the instance's type (`{x:05}` on an integer `T` is `00042`).

## Match beyond a plain enum

A name after `:` in a `match` field pattern BINDS (`Circle { radius: r } => r`), never compares
with a variable in scope; `is` takes bare names only (`if s is Circle { radius } { … }`).

<!-- from tests/reference/skill-is-and-match.loft -->
```loft
said = match (3, 7) {
    (0, _)   => "zero",
    (n, m)   => "{n},{m}",
};
```
<!-- from tests/reference/skill-is-and-match.loft -->
```loft
// An enum element names a VARIANT — by tag, or with its payload bound.  A record payload
// bound this way is a view of the MATCHED VALUE; a scalar payload is a copy.  `a` and `b`
// are PARAMETERS here, which keep aliasing the caller's values inside the tuple
// (B-Ref-Alias), so `status.hp -= 3` reaches the caller's `b`.  A tuple of LOCALS copies
// them in (T-Cons) and the write would stay in that copy — match a local alone to write
// through to it.
match (a, b) {
    (Fireball, IceWall { status }) => { status.hp -= 3; },
    (Arrow { id }, Crate)          => { world.removed += [id]; },
    _ => {},
}
```

**Slice arms that together cover every length need no `_`** — `[]` with `[x, ..rest]` is the
list recursion; a rest arm alone, or a set that leaves a length out, is refused (*"a slice
pattern can fail"*):

<!-- from tests/reference/skill-is-and-match.loft -->
```loft
fn total(v: vector<integer>) -> integer {
  match v {
    [] => 0,
    [x, ..rest] => x + total(rest),
  }
}
```

**A scalar `match` with no `_` and no bare-binding arm selects nothing for a value its arms do
not name** — the match answers null (`τ?`); a statement does nothing — and the
`match-scalar-hole` advice says so at the match.  Add the `_` arm, or set `LOFT_NO_SCALAR_HOLE`
for a deliberate hole.
