<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN182 — draft rules: operator syntax on a user type

**Status: DRAFT (P0); order and `+ - *` are BUILT (P1, P2).**  Op-Def, Op-Home, Op-Shape,
Op-Mark, Op-Name, Op-Left, Op-Order, Op-Result, Op-Compound and Op-Bound, for `compare`,
`plus`, `minus` and `times`, are in force and live in
[formal/operators.md](../../formal/operators.md); the rest below are proposed.  Each moves into
`doc/claude/formal/` (an `interfaces.md` section, `(Op-…)` tags) in the phase that implements it,
with its guard; until then nothing in `src/` may cite them.  The plan: [README.md](README.md).

A **user type** is a `struct`, `value struct` or `enum` declared outside `default/`.  An
**operator definition** is written with the keyword `operator` in place of `fn` — `pub operator
plus(self: Money, other: Money) -> Money { … }` — with a name from the table of `(Op-Back)`.  It
is a function in its definition and an operator in its use (owner, Q1).

## The table

```
  (Op-Back)   form on a user type τ          is the call                    the operator definition's shape
              a < b, a <= b, a > b, a >= b   a.compare(b)  (Op-Order)       compare(self: τ, other: U) -> Ordering
              a + b                          a.plus(b)                       plus(self: τ, other: U) -> V
              a - b                          a.minus(b)                      minus(self: τ, other: U) -> V
              -a                             a.negate()                      negate(self: τ) -> V
              a * b                          a.times(b)                      times(self: τ, other: U) -> V
              a / b                          a.divided_by(b)                 divided_by(self: τ, other: U) -> V
              a % b                          a.remainder(b)                  remainder(self: τ, other: U) -> V
              a ⊕= b   (⊕ any of + - * / %)  a = a.<⊕'s method>(b)  (Op-Compound)
              x[i₁, …, iₙ]                   x.at(i₁, …, iₙ)                 at(self: τ, i₁: K₁, …) -> V
              x[i₁, …, iₙ] = v               x.set_at(i₁, …, iₙ, v)          set_at(self: τ, i₁: K₁, …, v: V)
              x[a..b], x[a..], x[..b]        x.slice(a?, b?)   (Op-Slice)    slice(self: τ, from: K?, till: K?) -> R
              for e in x[lo..hi]             x.key_range(lo?, hi?)           PLACED, not allowed (Op-KeyRange)
              a ** b                         a.power(b)                      PLACED, not allowed (Op-Bits)
              a & b, a | b, a ^ b, ~a,       a.bit_and(b), a.bit_or(b),      PLACED, not allowed (Op-Bits)
              a << b, a >> b                 a.bit_xor(b), a.bit_not(),
                                             a.shift_left(b), a.shift_right(b)
              a == b, a != b                 a.equals(b), !a.equals(b)       equals(self: τ, other: τ) -> boolean   (Op-Eq)
              x as T                         x.to_t()      (Op-Conv)       to_t(self: τ) -> T, T the type `to_` names
              !a                             —  the presence test, always  (Op-Not)
              xs[(x1,y1)..(x2,y2)], xs[(x,y)..], xs[(x,y)..:n]
                                             —  built-in `spatial` only  (Op-Spatial)
              for e in x                     x.next()       (Op-Fold)       next(self: τ) -> E?
              "{x}", "{x:spec}"              x.to_text(), x.to_text(spec) (Op-Fold)  to_text(self: τ) -> text, to_text(self: τ, spec: text) -> text
```

**In words.**  An operator applied to a user type is a call of ONE `operator` definition on that
type, and the table is the whole list: the name says what the symbol does, the keyword says that
the definition is reached by syntax, and nothing is spelled `Op`.

## The rules

```
  (Op-Def)     an `operator` definition is checked where it is written, and refused there unless
               ALL of: its name is in (Op-Back)'s list and not a reserved one (Op-Bits,
               Op-KeyRange, Op-Slice while placed); its first parameter is `self`, of a user type
               (Op-Home); and its shape is the form's (Op-Shape).  So a definition that compiles
               backs its form — there is no `operator` that silently backs nothing.

  (Op-Home)    an `operator` definition's `self` type is declared in the same package.  An
               `operator` on another package's type, or on a built-in type, is refused.

  (Op-Shape)   the form's arity: `negate`, `next`, `to_text` with `self` alone (`to_text` also with
               `self` and `spec: text`, the `"{x:spec}"` form); `compare`, `plus`,
               `minus`, `times`, `divided_by`, `remainder` with `self` and ONE more parameter; `at`
               with `self` and n ≥ 1; `set_at` with `self`, n ≥ 1 and the value LAST; `compare`
               answering `Ordering`, `to_text` answering `text`.

  (Op-Mark)    only an `operator` definition backs a form.  A plain `fn` of a table name is an
               ordinary method, and a use of the form on that type is refused, naming the unmarked
               method and the `operator` keyword it lacks.

  (Op-Name)    an `operator` definition is also an ordinary method: `m.plus(n)` and `plus(m, n)`
               call it (F-OneBody), and it may be passed where a function is expected.  PROPOSED
               (README Q12), for the owner.

  (Op-Fold)    `next` and `to_text` join the table AT ONCE (owner): from the release that ships
               (Op-Mark), a plain `fn next` / `fn to_text` is an ordinary method, and `for` /
               `"{…}"` on a type with only an unmarked one is refused naming it.  The same change
               marks the stdlib's own definitions and the published libraries' (`time`, `server`),
               which are republished with it.

  (Op-Iface)   an interface member is spelled as the definition that meets it: `operator
               compare(self: Self, other: Self) -> Ordering` is met only by an `operator compare`,
               `fn size(self: Self) -> integer` by a plain `fn size`.  PROPOSED (README Q13).

  (Op-Left)    a binary form `a ⊕ b` calls the method of the LEFT operand's type, chosen among
               that name's overloads by the RIGHT operand's type, as the method call `a.m(b)`
               chooses (Disp-Select, formal/dispatch.md).  The right operand's
               type is never asked, and no form is commuted: `2 * d` is refused where `d * 2` is
               not.  Each operand is evaluated once, left to right.

  (Op-Order)   `compare` answers `Ordering` (`Less`, `Equal`, `Greater`; stdlib).  The four order
               forms read it, each from ONE call:
                 a < b  ≡ a.compare(b) == Less        a <= b ≡ a.compare(b) != Greater
                 a > b  ≡ a.compare(b) == Greater     a >= b ≡ a.compare(b) != Less
               `compare` answering `Equal` does not make `a == b` true (Op-Eq).

  (Op-Compound) `a ⊕= b` is `a = a.m(b)` for ⊕'s method m, with `a` evaluated once as a place.
               There is no in-place method to define.

  (Op-Result)  the form's type is the method's return type, its value the method's value: a
               method answering `V?` makes the form `V?`.  An operand of nullable type is
               discharged exactly as the receiver or argument of the same call would be.
               The language adds no fault (C80).

  (Op-Eq)      `==` / `!=` are (E-Eq), structural, on every type at every depth; no type defines
               them, and `operator equals` is refused naming a named method as the cure (@C134,
               README Q3).  `compare` answering `Equal` does not make `==` true.  No key rule
               follows: every key compares structurally (Q14 closed).

  (Op-Conv)    `x as T` on a user type calls `x.to_t()`, where `t` is T's name in snake case
               (`DateTime` → `date_time`, `HttpRequest` → `http_request`, an all-capital run is one
               word: `HTTPRequest` → `http_request`; a digit stays with the word before it: `Vec2`
               → `vec2`), and the definition must answer exactly T.  T is one of the base types
               (`integer`, `float`, `single`, `text`, `boolean`, `character`) or a non-generic
               `struct` / `value struct` / `enum`; a generic instance or a `type` alias has no
               `to_` name.  The one exception to (Op-Home)'s own-type rule: a conversion INTO the
               package's own type may take a foreign `self` (`operator to_date_time(self: text)
               -> DateTime`), so only `DateTime`'s package can define `to_date_time` and no two
               packages claim one conversion.  `to_text` is a member of this family; `x as text`
               calls `to_text(self)` if the type defines it, else `to_text(self, "")` (Op-Fold's
               spec form).  A plain `fn to_<name>` (`to_uppercase`, `to_json`, `to_millis`) is an
               ordinary method.

  (Op-Not)     `!x` on a user type is the presence test and cannot be defined.

  (Op-Slice)   `x[a..b]` / `x[a..]` / `x[..b]` on a user type calls `slice` with each written end
               and null for an omitted one; the range is half-open.  `x[a..=b]` is refused on a
               user type (an inclusive end means "one past", which only an integer has).

  (Op-KeyRange) a `for`-only keyed slice on a user type (`Slice-KeyedIter`'s shape) is backed by
               `key_range(self, lo: K?, hi: K?)` answering an iterator.  PLACED: refused until a
               user type needs it and user-written iterators exist.  A type defines `slice` or
               `key_range`, not both: one meaning per shape.

  (Op-Spatial)  the spatial forms (`Slice-Box`, `Slice-Open`, `Slice-Cap`) belong to the built-in
               `spatial` collection; on a user type they are refused.

  (Op-Bits)    `**`, `& | ^ ~ << >>` have their backing names reserved and are refused on a user
               type until a user needs one: an `operator` of a reserved name is refused at its
               definition, and a plain `fn` of one is an ordinary method.

  (Op-Bound)   under (Op-Iface) the stdlib's operator interfaces declare `operator` members, met by
               the stdlib's `operator` definitions on built-in types and by a user type's own
               (Op-Def), at the signature (G-Sat) asks:
               `Ordered` by `compare(self: Self, other: Self) -> Ordering`; `Addable` by
               `plus(Self) -> Self`; `Subtractable` by `minus(Self) -> Self`; `Numeric` by
               `times(Self) -> Self` and `negate() -> Self`; `op []` by `at`.  `Equatable` is met
               by every type (G-Sat-Eq), structurally.

  (Op-Std)     a type declared in `default/` keeps the stdlib's operator definitions; a user
               `fn Op…` is an ordinary function (C132, phase P5).
```

## Every form, one rule

| form | rule | in the first subset |
|---|---|---|
| `< <= > >=` | Op-Order | yes (P1) |
| `+ - *` | Op-Back + Op-Left | yes (P2) |
| `/ %`, unary `-` | Op-Back + Op-Left | placed |
| `+= -= *=` | Op-Compound | yes (P2) |
| `/= %=` | Op-Compound | placed, with `/` `%` |
| `x[i…]` read | Op-Back (`at`) | placed (P3 designed, not built) |
| `x[i…] = v` | Op-Back (`set_at`) | placed |
| `x[i…] ⊕= v` | Op-Compound over `at` + `set_at`, indices evaluated once | P3 decides |
| `x[a..b]` and open ends | Op-Slice | placed (P4 if a user asks) |
| `x[a..=b]` | Op-Slice (refused) | refused |
| keyed `for` slice | Op-KeyRange | placed, refused |
| spatial slices | Op-Spatial | refused |
| `** & \| ^ ~ << >>` | Op-Bits | placed, refused |
| `== !=` | Op-Eq | structural, always; `operator equals` refused (@C134) |
| `x as T` | Op-Conv | yes |
| `!` | Op-Not | presence |
| bounds | Op-Bound | with P1/P2/P3 |
| who may define | Op-Def, Op-Home, Op-Shape, Op-Mark | with P1 |
| calling it by name | Op-Name (proposed) | with P1 |
| `for e in x`, `"{x}"`, `"{x:spec}"` | Op-Fold | with P1, at once |
| an interface member | Op-Iface (proposed), Op-Bound | with P1 |
| what the form yields | Op-Result | with P1 |

**The subset (owner, Q10: "we implement what is in use at least for now").**  Built in P1–P2:
`compare`, `plus`, `minus`, `times`, the `to_<type>` conversions, `next` and `to_text` — what
`time`, `server` and the stdlib define today.  Every other row is PLACED: designed here, reserved,
and refused with the page's `--explain` line — `at` / `set_at`, the slices, `**` and the bit
operators.  `negate`, `divided_by` and `remainder` were built in P5b; `equals` is refused for
good (@C134).  `OpIndex` retires in P5
with no shipped replacement.
