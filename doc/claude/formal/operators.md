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

  (Op-Def)     an `operator` definition is checked where it is written, and refused there
               unless its name is a form in (Op-Back) — a form of the table not built yet is
               refused by name, saying what it will back — its first parameter is `self` of a
               user type (Op-Home), and its shape is the form's (Op-Shape).  So a definition
               that compiles backs its form.

  (Op-Home)    the `self` type of an `operator` definition is declared in the same source as
               the definition; on a built-in type it is refused.

  (Op-Shape)   `compare` takes `self` and one more parameter and answers `Ordering`.

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

  (Op-Result)  the form is the call: its type is the method's return type, and an operand of
               nullable type is discharged — and warned about — exactly as the same argument of
               `a.compare(b)` is.

  (Op-Bound)   `Ordered`'s `op <` is met by `operator compare(self: C, other: C) -> Ordering`,
               and a monomorph's `<` at such a `C` is (Op-Order)'s `a.compare(b) == Less`.
```

**In words.**  `operator compare(self: Date, other: Date) -> Ordering` makes `<`, `<=`, `>` and
`>=` work on `Date`, each as one call of that method with the operands in the order they are
written; `sort` and every generic bounded by `Ordered` read it too.  The keyword is what opts a
type in: the same method declared with `fn` is just a method, and `<` on that type is refused
saying so.

## Where each rule is kept

| rule | site | guard |
|---|---|---|
| Op-Def, Op-Home, Op-Shape | `Parser::check_operator_definition` (`parser/definitions.rs`) | `tests/parse_errors.rs` (`operator_*`) |
| Op-Mark, Op-Left, Op-Back | `Parser::operator_compare` (`parser/operators.rs`) | `tests/parse_errors.rs` (`a_plain_compare_does_not_make_less_than_work`), `tests/scripts/an-operator-compare-gives-a-type-every-order-form.loft` |
| Op-Order, Op-Result | `Parser::order_through_compare` | the same script |
| Op-Bound | `Data::operator_compare_for`, `Parser::satisfaction_failures`, `substitute_type_in_value` | the same script (`sort`, `min_of`, `max_of`, a user generic) |
| Op-Name | the definition is an ordinary method | the same script |

## Deviations

**OPEN: 0.**
