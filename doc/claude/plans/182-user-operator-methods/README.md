<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN182 — operator syntax for user types, through named methods

**Status: P0 (design + falsification) written.  Decided by the owner: Q1 (names, the `operator`
keyword), Q3 (`operator equals` allowed), Q10 (the subset is what is in use), the `to_<type>`
conversions.  Recommended here, for the owner: Q12 (callable by name), Q13 (interface spelling),
Q14 (the key-field rule), Q15 (conversion details).**  The plan
itself — goal, constraints, scope, phases — is the tracker issue,
[loft-lang/plans#182](https://github.com/loft-lang/plans/issues/182); this directory holds P0's
deliverables:

| file | what |
|---|---|
| [RULES.md](RULES.md) | the draft rules, one per form, and the form → rule table |
| [PROGRAMS.md](PROGRAMS.md) | `time`, a `Grid`, a multi-key lookup and a keyed slice in the proposed spelling, each use tagged with its rule |
| [PAGE.md](PAGE.md) | the user-facing page draft (the @F entry P6 publishes) |
| [CENSUS.md](CENSUS.md), [census.py](census.py) | the name census over the stdlib and every library's public API |
| [by-functions.patch](by-functions.patch) | `sort_by` / `min_by` / `max_by` / `sum_by` for `default/01_code.loft`, verified (Q11); applies with `git apply` |

## The design in one paragraph

An operator on a user type is a call of ONE `operator` definition on that type — a method written
with the keyword `operator` in place of `fn` — from a fixed table:
`compare` (all four order operators, through a stdlib `Ordering`), `plus`, `minus`, `negate`,
`times`, `divided_by`, `remainder`, `at` / `set_at` for `[]`, and the existing `next` and
`to_text`.  The left operand's type decides; nothing commutes; compound assignment is `a =
a.plus(b)`.  `==`, `!=` and `!` keep their built-in meaning for every type.  Slices, keyed slices,
`**` and the bit operators have reserved names and are refused until someone needs them.  Only the
type's own package can define them.

## Answers to the plan's questions

**Q1 — naming.  DECIDED (owner; recorded on the issue).**  The names stand: `compare` (answering
`Ordering`), `plus`, `minus`, `negate`, `times`, `divided_by`, `remainder`, `at`, `set_at`; placed:
`slice`, `key_range`, `power`, `bit_and`, `bit_or`, `bit_xor`, `bit_not`, `shift_left`,
`shift_right`.  The census ([CENSUS.md](CENSUS.md)) decided two of them: `add` is a method on
`tween`'s `Track` and `web`'s `WsGroup` meaning "insert", and `get` on `arguments` and `random`.
**The opt-in is explicit**: the keyword `operator` REPLACES `fn` — `pub operator plus(self:
Money, other: Money) -> Money { … }` — "a function in definition but not in its use".  A
same-named plain `fn` is ordinary, and the operator on that type is refused naming it (Op-Mark).
The definition is checked where it is written: name in the list, shape per form, own package,
`self` first (Op-Def).  `operator` becomes a keyword; no loft or library source uses it as a
name.  `next` and `to_text` fold into the table as `operator next` / `operator to_text` AT ONCE
(owner correction: no client project depends on them yet) — an unmarked one is an ordinary method
from then on, and `for` / `"{…}"` on a type with only an unmarked one is refused naming it
(Op-Fold).  Re-counted with the census: the stdlib's 7 (six `to_text` on built-in types and
`Printable`'s member), the libraries' 4 (`time`'s two `to_text(self, spec: text)` — the
`"{x:spec}"` form, which (Op-Shape) now admits — and `server`'s two `next`), and 139 definitions in
73 in-tree files (tests, fixtures, docs) that migrate in the same change.

**Q2 — derivation.**  Only the order: `< <= > >=` all read one `compare` call, and the page shows
each derivation as a formula.  Nothing else derives — `-` is not `+` of `negate`, `!=` is the
built-in complement of the built-in `==`.  A type defines any subset of the arithmetic methods.
`compare` answers an `Ordering`, not an integer: a sign-integer invites `self.ms - other.ms`, which
overflows to null and orders silently wrong (E-Uncomp); the stdlib gives every built-in ordered
type a `compare` (`self.ms.compare(other.ms)`), and `Ordering.then(next)` chains fields.

**Q3 — equality.  DECIDED (owner): structural by default, `operator equals` opts in**
(Op-Eq): `a == b` is `a.equals(b)`, `a != b` its negation; `compare` and `equals` independent;
`!x` stays the presence test.  What remained for P0 is **Q14 — the key field**: see below.

**Q4 — mixed types and direction.**  `a ⊕ b` calls the left operand's method, chosen by the right
operand's type among its overloads (`DateTime.minus(DateTime) -> Duration` beside
`DateTime.minus(Duration) -> DateTime`).  The reverse is never implied: `3 * w` is refused, and
since `integer` is not the user's type (Op-Home), it cannot be made to work.  The page says
`w * 3`.

**Q5 — compound assignment.**  `a ⊕= b` is `a = a.m(b)`, `a` evaluated once as a place.  No
in-place method.

**Q6 — null and failure.**  The form's type and value are the method's (Op-Result); a method
answering `V?` makes the form `V?`.  An out-of-range `x[i]` is the type's answer, visible in
`at`'s return type; the page recommends `V?`, matching the built-in collections.  No fault is
added (C80).

**Q7 — `[]` forms.**  The brackets' SHAPE picks the method: no `..` → `at` (read) or `set_at`
(write), and element and keyed lookup are ONE rule for a user type, since the brackets hold the
method's parameters; `..` → `slice` (a value) or `key_range` (a `for`-only iterator), and a type
defines at most one of the two.  `..=` and the spatial forms are refused on a user type.

**Q8 — bounds.**  (Op-Bound): an interface's `op ⊕` member is met by a stdlib operator
definition or by ⊕'s backing method at the signature (G-Sat) asks — `Ordered` by `compare`,
`Addable` by `plus`, `Subtractable` by `minus`, `Numeric` by `times` + `negate`, `op []` by `at`.
`Equatable` stays met by every type.

**Q9 — discoverability.**  [PAGE.md](PAGE.md) is the entry; its lookup keys are every operator
token and every method name in the table.  The refusal for an undefined form names the method and
its exact signature, and `--explain` opens the entry (@PLN183 delivers it).

**Q10 — the first subset.  DECIDED (owner): "we implement what is in use at least for now".**
`compare`, `plus`, `minus`, `times`, the `to_<type>` conversions, `next` and `to_text` — what
`time`, `server` and the stdlib define today; the census confirms nothing else is in use as an
operator (`time`'s `negate` and `minus(DateTime, Duration)` are called by name; its `OpEq` /
`OpNe` are structural).  Everything else is placed and refused with the `--explain` line
(RULES.md § The subset); `OpIndex` retires with no shipped replacement.

**Q11 — `sort_by` / `min_by` / `max_by` / `sum_by`.**  Keep them, as key-based helpers: `compare`
gives a type ONE order, while `sort_by` sorts by whatever key one call needs (dates by weekday,
fares by distance) without defining an order on the type.  A version is written and
matrix-verified on both backends — inline and linked element layouts, ties, empty and one-element
vectors, text / float / integer keys — and waits for P1 with the rest of loft#1833.  That run found
and fixed a silent-wrong it would have inherited: a generic read `vector<T>`'s element as the id's
address whenever `T` had a keyed collection over it (loft commit c0cab3e7b).

**Q12 — is an `operator` definition callable by name (`m.plus(n)`)?  Recommended: yes**
(Op-Name), for the owner.  It is a function in its definition, so it is called like one, by both
spellings (F-OneBody), and may be passed where a function value is expected.  Three things depend
on it: `next` and `to_text` are called by name today (`x.to_text()`), so the folded pair must stay
callable and one rule for the whole table beats an exception for two; `time`'s callers of
`d.minus(span)` and `w.negate()` keep working, so `time` breaks once (the `fn Op…` names), not
twice; and a generic body bounded by `Addable` may write `a.plus(b)` as readily as `a + b`.  The
cost is two spellings of one operation, which `next` / `to_text` already have.

**Q13 — how is an interface member spelled?  Recommended: as the definition that meets it**
(Op-Iface), for the owner.  `interface Printable { operator to_text(self: Self) -> text }` is met
only by an `operator to_text`; `fn size(self: Self)` only by a plain `fn size`.  The operator
interfaces follow: `Ordered` declares `operator compare(self: Self, other: Self) -> Ordering`,
`Addable` `operator plus(…)`, and the stdlib meets them for the built-in types with `operator`
definitions of its own (P1 adds `compare` on them anyway).  So every bound is met one way, by a
built-in type and a user type alike, and the symbol member `op ⊕` retires with `fn Op…` — 6 uses
in the stdlib, 8 outside it.  `Equatable` keeps no member to define: every type meets it (Op-Eq).

**Q14 — a key field whose type has `operator equals`.  Recommended: (a), refuse it at the
declaration** (Op-Key), for the owner.  Measured on today's tree with a user `fn OpEq` standing in
for `operator equals` (`Dt` with `==` meaning "same last digit", so `Dt{1} == Dt{11}`):

| cell | (a) refuse the key | (b) keys compare structurally |
|---|---|---|
| `a == b` | true | true (measured) |
| `hash<E[d]>` given `a`, then `b` | refused at the declaration | **2 entries** for keys `==` calls one (measured) |
| `h[b]` with only `a` inserted | refused | not run (#1845); by construction a miss for a key `==` says is there |
| `sorted<E[d]>` given `a`, `b` | refused | not run (#1845); by construction 2 entries, as `hash` |
| a key `W { d: Dt }` (holds a `Dt`) | refused (Op-Key looks through fields) | not run (#1845); `W{a} == W{b}` true by (E-Eq) through `Dt`'s `==` |
| `hash<E[n]>` keyed on a scalar beside a `Dt` field | allowed: the key is `n` | allowed |
| content `==` of two such hashes | records compared by `equals` | the same |

The measured cell already shows (b)'s disagreement: one collection holding two entries for keys
`==` calls one.  The unmeasured ones follow from it — (b) leaves a lookup answering "absent" for a key `==` says is present, with no diagnostic: a
silent disagreement, which (E-Eq-Key) exists to forbid.  (a) refuses at compile time (C80 permits
that) and names the cure.  The matrix also found that ANY struct-typed key field — no `OpEq` at
all — crashes the interpreter at its first lookup and does not compile with `--native`: loft#1845,
filed, so the (b) lookup cells could not be run past the first.  Op-Key holds whichever way #1845
is decided.

**Q15 — conversion details.**  Recommended, for the owner (Op-Conv):
- `x as text` calls `to_text(self)` if defined, else `to_text(self, "")` — so `time`, which
  defines only the spec form, gets `dt as text` without a second definition.
- Snake case: `DateTime` → `to_date_time`; an all-capital run is one word (`HTTPRequest` →
  `to_http_request`); a digit stays with the word before it (`Vec2` → `to_vec2`).  A generic
  instance and a `type` alias have no `to_` name: converting to one is refused.
- The built-in conversions appear on the page as the same family, each listed with its result
  type (`integer as float`, `text as integer` …), provided by the stdlib; whether they also become
  callable methods (`5.to_float()`) waits for a user (Q10).

## What P0 found

- **Overload selection had no rule in `formal/`.**  (Op-Left) chooses among `minus`'s overloads
  by the right operand's type, as a method call does.  The rules existed, in @PLN162's plan
  directory, never moved when that plan finished; P1's first step moved them to
  [formal/dispatch.md](../../formal/dispatch.md), and (Op-Left) cites `Disp-Select`.
- **`compare` needs stdlib support first.**  `Ordering`, `then`, and `compare` on `integer`,
  `float`, `single`, `text` and `character` are P1's first step: without them a user's `compare`
  is a three-way `if`.
- **The opt-in is visible at the definition** (Q1, decided): `operator` says that syntax reaches
  the method.  At the use, a reader of `a + b` on a user type finds it through the page, the
  refusal and the IDE hover (@PLN183).
- **Interfaces need one spelling** (Q13).  `Printable` declares `fn to_text(self: Self)`, and the
  operator interfaces declare symbol members (`op <`, `op +`; 6 in the stdlib, 8 outside it).
- **No form went without a rule or took two** (RULES.md § Every form, one rule; PROGRAMS.md
  § The count), and every `time` operation reads at least as well as today.

## Next

Q12–Q15 are with the owner.  P1 starts after them, and begins with: the stdlib's `Ordering` / `compare`,
the overload rule, then (Op-Order) / (Op-Bound) for `Ordered`, with `time`'s parallel run (both
spellings, one matrix of dates).
