# @PLN91 — a generic instantiated at a tuple with a text member, or at a hash (prepared subject)

**The family:** "a generic instance at this type" (19 programs at try 0ca590df5, 2 held by it
alone).  Both sole programs call an `Equatable` generic (`fn same<T: Equatable>(a: T, b: T) ->
boolean { a == b }`) at a type the port does not bind yet: a TUPLE WITH A TEXT MEMBER
(`same((1, "a"), (1, "a"))`) and a HASH (`same(a, b)` with `a, b: hash<E[k]>`).  The port
refuses at `el_ex_call.loft` ~143, and `bound_refused` names the kind.  Its `scalar_tuple`
test admits a tuple of plain scalars only (a text member fails it), and it has no arm for a
`hash`.

**The rule:** `@FR-G-Mono`: a generic is instantiated once per binding, its body elaborated with
the type variable replaced.  `@FR-G-Sat-Eq`: every type is equatable, and `==` on the binding
compares CONTENT as the type defines it.

## Rust's instances, measured with the tested tree's Rust (b7e35bef5's loft; the parser's lowering, before the scope pass)

```
G-Sat-Eq/an-equatable-bound-compares-content-in-the-monomorph.loft:37
  fn i_21__tuple<integer,text>_n_same(a: (integer, text), b: (integer, text)) -> boolean
     [21] return if OpEqInt(a.0, b.0) OpEqText(a.1, b.1) else false;     <- member by member, `&&`

G-Sat-Eq/c91-every-type-is-equatable.loft:57
  fn i_13hash<E,["k"]>_n_same(a: hash<E,["k"]>["??"], b: hash<E,["k"]>["??"]) -> boolean
     [29] return OpEqContent(a, b, 107);                                 <- content equality, the hash's known type
```

The instance NAME is `i_<LEN><type name>_<template>`.  The type name is the display form:
`__tuple<integer,text>` (21 characters) and `hash<E,["k"]>` (13).  The hash parameters carry
deps `["??"]`.

## What to code

In `bound_refused`, admit a tuple whose members are plain scalars OR texts, and a keyed
collection (`hash<…>`).  Then:
- In the instance body, `a == b` on the tuple binding lowers member by member, as an `&&` chain
  of each member's equality (`OpEqInt` / `OpEqText`), which the port's tuple `==` may already
  do outside generics.
- `==` on the hash binding becomes `OpEqContent(a, b, <the hash's known type>)`.
- The instance name uses the display type name with its character count.

## Probes (the two sole programs, each with hand-computed answers)

```
kr-suite/rules/interfaces/G-Sat-Eq/an-equatable-bound-compares-content-in-the-monomorph.loft:37   the tuple
kr-suite/rules/interfaces/G-Sat-Eq/c91-every-type-is-equatable.loft:57, :64                       the hash; a tuple with text
```
