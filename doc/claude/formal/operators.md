<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/operators.md — operator syntax on a user type (strict)

**Catalogue:** @PLN182 (the plan that builds it; its draft rules for the forms not built yet are
[plans/182-user-operator-methods/RULES.md](../plans/182-user-operator-methods/RULES.md)), C132.

> **Rules then deviations** (see [README](README.md)).  A user type — a `struct`, `value
> struct` or `enum` declared outside `default/` — reaches an operator form through ONE method
> on that type, written with the keyword `operator` in place of `fn`.  This chapter holds the
> rules for the forms that are built; each further form moves here from the plan's draft in
> the phase that builds it.  It extends [dispatch.md](dispatch.md) (which member a call
> reaches) and [interfaces.md](interfaces.md) (`G-Sat`).

## The rules

```
  (Op-Back)    form on a user type τ         is the call          the operator definition
               a < b, a <= b, a > b, a >= b  a.compare(b)          operator compare(self: τ, other: U) -> Ordering
               a + b                         a.plus(b)             operator plus(self: τ, other: U) -> V
               a - b                         a.minus(b)            operator minus(self: τ, other: U) -> V
               a * b                         a.times(b)            operator times(self: τ, other: U) -> V
               a / b                         a.divided_by(b)       operator divided_by(self: τ, other: U) -> V
               a % b                         a.remainder(b)        operator remainder(self: τ, other: U) -> V
               -a                            a.negate()            operator negate(self: τ) -> V
               x as T                        x.to_t()              operator to_t(self: S) -> T   (Op-Conv)
               for e in x                    x.next()              operator next(self: τ) -> E?
               "{x}", "{x:spec}"             x.to_text(), x.to_text(spec)
                                                                   operator to_text(self: τ) -> text,
                                                                   operator to_text(self: τ, spec: text) -> text

  (Op-Def)     an `operator` definition is checked where it is written, and refused there
               unless its name is a form in (Op-Back) — a form of the table not built yet is
               refused by name, saying what it will back — its first parameter is `self` of a
               user type (Op-Home), and its shape is the form's (Op-Shape).  So a definition
               that compiles backs its form.

  (Op-Home)    the `self` type of an `operator` definition is declared in the same source as
               the definition; on a built-in type it is refused.

  (Op-Shape)   `compare` takes `self` and one more parameter and answers `Ordering`; `plus`,
               `minus`, `times`, `divided_by` and `remainder` take `self` and one more parameter
               and answer a value; `negate` takes `self` alone and answers a value; `next` takes
               `self` alone and answers the item, null when the walk is done; `to_text` takes
               `self`, or `self` and `spec: text`, and answers `text`.

  (Op-Mark)    only an `operator` definition backs a form.  A plain `fn` of the same name is
               an ordinary method; a use of the form on a type that has only such a method,
               and no other way to answer it, is refused naming the method and the keyword.

  (Op-Name)    an `operator` definition is also an ordinary method: `a.compare(b)` and
               `compare(a, b)` call it (F-OneBody).

  (Op-Left)    `a ⊕ b` calls the method of the LEFT operand's type, chosen among that name's
               members by the right operand's type as the call `a.m(b)` chooses (Disp-Select).
               The right operand's type is never asked and no form is commuted; each operand
               is evaluated once, left to right.

  (Op-Order)   the four order forms read ONE `compare` call:
                 a < b  ≡ a.compare(b) == Less        a <= b ≡ a.compare(b) != Greater
                 a > b  ≡ a.compare(b) == Greater     a >= b ≡ a.compare(b) != Less

  (Op-Result)  the form is the call: its type is the method's return type — no wrap is added for
               a nullable operand — and an operand of nullable type is discharged, and warned
               about, exactly as the same argument of `a.m(b)` is.

  (Op-Compound) `a ⊕= b` for ⊕ in `+ - * / %` is `a = a.m(b)` for ⊕'s method m, on any place — a
               variable, a field, an element — with the place read once.

  (Op-Eq)      `==` and `!=` are (E-Eq), structural, on every type at every depth, and no type
               defines them: `operator equals` is refused at the definition, naming the cure — a
               named method (`same_second(self, other)`) for a comparison of the type's own.  A
               plain `fn equals` is an ordinary method.  `!a` is the presence test on every type.
               `Equatable` declares nothing; every type meets it.  @C134.

  (Op-Std)     a type declared in `default/` keeps the stdlib's operator definitions, and the
               stdlib's own bodies reach no user `operator` definition.  The stdlib marks its own
               `operator to_text` on the built-in types; (Op-Home) binds the programs.  The
               internal names (`OpLt`, `OpAdd`, `OpIndex`, `OpConv…`) back an operator only as
               the stdlib's own definitions: a program's `fn OpAdd` is an ordinary function,
               callable by its name and reached by no operator, `[…]`, `as` or bound (@C132).  A
               definition spelled so is said where it is written — a WARNING for `OpEq`, `OpNe`
               and `OpNot`, whose forms keep answering (by the fields, and as the presence
               test), an advice for the rest, whose every use is refused.  `[…]` belongs to the
               built-in collections: a program's type is not subscripted, and an interface
               cannot require `op []`.

  (Op-Fold)    `for e in x` reaches only an `operator next`, and `"{x}"` only an `operator
               to_text`; a plain `fn next` / `fn to_text` is an ordinary method, and the form on
               a type with only such a method is refused naming it.

  (Op-Conv)    `x as T` calls `x.to_t()` where `t` is T's name in snake case (`DateTime` →
               `date_time`; an all-capital run is one word, `HTTPRequest` → `http_request`; a
               digit stays with the word before it, `Vec2` → `vec2`), and the definition answers
               exactly T.  T is one of the six base types or a non-generic struct or enum; a
               generic instance and a narrowed integer have no `to_` name.  The one exception to
               (Op-Home): a conversion INTO a type of this source may take a foreign `self`
               (`operator to_date_time(self: text) -> DateTime`), so no two packages claim one
               conversion.  `x as text` calls `to_text`, with an empty spec where the type
               defines only the spec form.  A plain `fn to_<t>` is an ordinary method; an `as` it
               would have answered is refused naming it.

  (Op-Iface)   an interface member written `operator` (`Printable`'s `operator to_text`) is met
               only by an `operator` definition; a member written `fn` by a plain method.

  (Op-Bound)   `Ordered`'s `op <` is met by `operator compare(self: C, other: C) -> Ordering`,
               `Addable`'s `op +` by `operator plus(self: C, other: C) -> C`, `Subtractable`'s
               binary `op -` by `operator minus(…) -> C`, and `Numeric`'s `op *` by `operator
               times(…) -> C`; a monomorph's operator at such a `C` is that one call (`<` read
               as `a.compare(b) == Less`).  `Numeric` also asks for unary `-`, which a user type
               cannot define yet, so a user type does not meet `Numeric`.
```

**In words.**  `operator compare(self: Date, other: Date) -> Ordering` makes `<`, `<=`, `>` and
`>=` work on `Date`, each as one call of that method with the operands in the order they are
written; `sort` and every generic bounded by `Ordered` read it too.  `operator plus`, `minus` and
`times` do the same for `+`, `-` and `*`, and `a += b` is `a = a.plus(b)`; the left operand's
type decides, so `w * 3` can work where `3 * w` is refused.  The keyword is what opts a
type in: the same method declared with `fn` is just a method, and `<` on that type is refused
saying so.

## Where each rule is kept

| rule | site | guard |
|---|---|---|
| Op-Def, Op-Home, Op-Shape | `Parser::check_operator_definition` (`parser/definitions.rs`) | `tests/parse_errors.rs` (`operator_*`) |
| Op-Mark, Op-Left, Op-Back | `Parser::operator_compare` (`parser/operators.rs`) | `tests/parse_errors.rs` (`a_plain_compare_does_not_make_less_than_work`), `tests/scripts/an-operator-compare-gives-a-type-every-order-form.loft` |
| Op-Order, Op-Result | `Parser::order_through_compare` | the same script |
| Op-Back (arithmetic), Op-Compound | `Parser::operator_member`, `arith_through_operator`, the arithmetic and compound sites (`parser/operators.rs`) | `tests/scripts/an-operator-plus-minus-or-times-is-one-call-of-the-left-operand.loft`, `tests/parse_errors.rs` (`operator_plus_*`, `a_plain_plus_does_not_make_plus_work`) |
| Op-Bound | `Data::operator_member_for`, `Parser::satisfaction_failures`, `substitute_type_in_value` | both scripts (`sort`, `min_of`, `max_of`, `sum`, user generics) |
| Op-Name | the definition is an ordinary method | the same script |
| Op-Fold | the `next` lookups (`parser/collections.rs`, `parser/control.rs`) and the format route (`try_bound_to_text_call`, the spec reader in `parser/objects.rs`) | `tests/scripts/a-programs-own-iterator-works-like-a-built-in-one.loft` and the format guards, migrated; refusals in `tests/parse_errors.rs` |
| Op-Conv | `Data::conversion_name`, `Parser::check_conversion_definition`, `Parser::operator_conversion` (the `as` site) | `tests/scripts/operator-next-to-text-and-to-type-drive-for-format-and-as.loft`, `tests/parse_errors.rs` (`operator_conversion_*`, `a_plain_to_integer_does_not_drive_as`) |
| Op-Iface | the interface body (`parser/definitions.rs`), `Parser::satisfaction_failures` | `tests/scripts/845-generic-format.loft` (Printable through a user type) |
| Op-Std | `Parser::operator_member` (no lookup while `default/` is parsed), `Data::backs_operator` (the stdlib's `Op…` only: `user_op_method`, `call_op`, `re_resolve_call`, satisfaction, the conversion registry), `Parser::report_retired_operator_function`, the subscript refusal (`parser/fields.rs`) | `tests/frontend_counts.rs` (the cold stdlib parse holds its pin), `tests/scripts/a-fn-named-for-an-operator-is-an-ordinary-function.loft`, `1580` / `1581` (the warning tier), `996-opindex-composite-subscript.loft` and `tests/parse_errors.rs` (`assigning_through_op_index_is_refused`, `an_interface_cannot_require_a_subscript`) |

## Deviations

**OPEN: 0.**
