<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/interfaces.md — semantics for interfaces & generics (strict)

**Catalogue:** @F25 (generics), @F26 (interfaces & bounds), @PLN89 (differential oracle).
Reference: [INTERFACES.md](../INTERFACES.md), [TYPING_RELATION.md](../TYPING_RELATION.md).

> **Rules then deviations** (see [README](README.md)). This is the relation for loft's
> **compile-time polymorphism**: an `interface` (a set of method signatures), **structural
> satisfaction** (a type meets an interface by having the methods — no `impl`), a **generic**
> function bounded by interfaces, and **monomorphization** (one specialised copy per concrete
> type). It is primarily a *static* area (extends [types.md](types.md)); dispatch is a call
> ([calls.md](calls.md)). Every rule is a **user-visible contract** verified on both backends.

## Notation

Uses [types.md](types.md)'s `Γ ⊢ e ⇒ τ` (e *has* type τ). An **interface** `I` is a finite set of
method signatures `fn m(self: Self, p̄) -> R`. A **type variable** `T` is a name bound by a
generic header; a **bound** `T: I₁ + … + Iₖ` constrains it. `C ⊨ I` reads "concrete type `C`
**satisfies** interface `I`". `[T ↦ C]` is the substitution of a concrete type for a type variable.

---

## Rules

### Interface declaration — signatures only, `self: Self`

```
  (G-Iface)   interface I { fn m₁(self: Self, p̄₁) -> R₁  …  fn mₙ(self: Self, p̄ₙ) -> Rₙ }
              declares I as the set {m₁ … mₙ} of method SIGNATURES.  Each method's first
              parameter is `self: Self` (Self = the implementing type, filled in per instance);
              the interface has NO bodies.  An operator method may use sugar
              `op <tok> (self: Self, …) -> R`, which names the method `OpCamelCase` (e.g. `<` ⟶ OpLt).
```

**In words.** An interface is a named list of method shapes a type must provide — for example
`interface Ordered { op < (self: Self, other: Self) -> boolean }`. It states *what*, never *how*
(no method has a body). `Self` is a placeholder standing for whatever concrete type ends up
satisfying it. Operator requirements are written with `op <` and desugar to the canonical operator
method name.

### Satisfaction — structural, at the use site

```
  (G-Sat)   C ⊨ I   iff for every  fn m(self: Self, p̄) -> R  in I,  a concrete function m with
            receiver type C and signature [Self ↦ C](p̄ -> R) is VISIBLE at the point of use.
            No `impl` declaration is written or needed — having the methods IS satisfying.
```

**In words.** A type satisfies an interface exactly when the required methods exist for it — loft
reads the functions in scope, it does not want an `impl` block. A `struct Box` with
`fn size(self: Box) -> integer` automatically satisfies `interface Sizable { fn size(self:
Self) -> integer }`. Built-in types satisfy the stdlib interfaces (`Ordered`, `Equatable`,
`Addable`, `Numeric`, `Scalable`, `Printable`) through their existing operators. Satisfaction is judged with
the functions visible *where the generic is used*, not where the interface was declared.

### Generic functions — a bounded type variable

```
  (G-Gen)   fn f<T>(x: …T…) -> …T…            introduces an UNBOUNDED type variable T.
            fn f<T: I₁ + … + Iₖ>(…)           bounds T: only a type C with C ⊨ Iⱼ for every j may
                                              instantiate f, and inside f the body may call exactly
                                              the methods the bounds Iⱼ provide on a T value.
            fn f<T>(h: hash<T[k]>)            REFUSED at the declaration, for every keyed former
                                              (hash, sorted, index, spatial, trie) in a parameter
                                              or the return: a key names a field, and T has none
                                              until it is instantiated.
            fn m<T>(self: vector<T>)          a method whose receiver is BUILT over T, keyed on the
                                              receiver's former; `self: T` itself is REFUSED at the
                                              declaration — a method is found on its receiver's
                                              type, and T is not one.
```

**In words.** `fn total<T: Sizable>(xs: vector<T>) -> integer` is generic over any element type
that is `Sizable`; the body may call `.size()` on a `T` because the bound guarantees it. An
unbounded `<T>` may only move values around (store, pass, return) — with no bound there is no
method it is allowed to call on a `T`. Multiple bounds combine with `+`.

### Monomorphization — one specialised copy per concrete type, in the parser

```
  (G-Mono)   a call f(ā) with concrete argument types C̄ SPECIALISES f: the parser produces a
             per-C̄ copy of f with [T ↦ C] applied throughout (attribute, return, and body types;
             every CONSTANT derived from a type — a schema row, an element width, the op a
             builtin lowers to; and every method call re-resolved to C's concrete function).
             A method template `x.m()` specialises the same way.  This happens ONCE, in
             the parser, before backend selection — so the interpreter and `--native` receive the
             SAME specialised IR.  There is NO runtime interface value and NO dynamic dispatch.
```

**In words.** When `total` is called with a `vector<Box>`, loft builds a `Box`-specialised copy of
`total` and dispatches `.size()` to `Box`'s concrete method. Because specialisation is a parser
step feeding one shared IR to both backends, generics behave identically under `--interpret` and
`--native` — the two cannot drift. This is static monomorphization, like Rust's, not a v-table.

### Satisfaction is checked at instantiation — a miss does not compile

```
  (G-Check)  at each instantiation f[T ↦ C], the checker verifies C ⊨ Iⱼ for every bound.  A
             missing method is a STATIC error — `'C' does not satisfy interface 'I': missing m` —
             and the program does NOT compile.  The check is at the USE, not the interface
             declaration (an unused interface constrains nothing).
```

**In words.** If you call a `T: Sizable` generic with a type lacking `size`, you get a compile
error naming the type, the interface, and the missing method — never a runtime failure. The check
fires where the generic is instantiated, so the same interface can be satisfied by different sets
of visible functions at different call sites.

### Scope — compile-time polymorphism only (decided boundaries)

```
  (G-Scope)  interfaces are COMPILE-TIME bounds, not runtime types.  Out of scope by design
             (each a decided edge, not a deviation): an interface-typed VALUE / variable
             (`x: I = …` — dynamic dispatch); interface INHERITANCE (`interface A extends B`);
             ASSOCIATED types; DEFAULT method bodies; a GENERIC method inside an interface; a
             FACTORY method (a `Self` return with no `self` parameter).
```

**In words.** You cannot store a value at its interface type and dispatch on it at runtime
(`x: Sizable = …` is rejected) — an interface only ever appears as a generic *bound*. Inheritance,
associated types, default bodies, generic interface methods, and no-`self` factory methods are
deliberately not in the language (see [INTERFACES.md § out of scope](../INTERFACES.md)); each is a
decided boundary, so it belongs here as a scope rule, not as a deviation to close.

---

## Deviations

**OPEN: 0.**  Every deviation this doc has carried is closed; the record, and the four closed
deviations, are in the companion [interfaces-history.md](interfaces-history.md).

## Conformance

- **Declare + structurally satisfy (`G-Iface` / `G-Sat`)** — `interface Sizable { fn size(self:
  Self) -> integer }` with `fn size(self: Box) -> integer` makes `Box ⊨ Sizable` — no `impl`.
- **Bounded generic dispatch (`G-Gen` / `G-Mono`)** — `fn total<T: Sizable>(xs: vector<T>) ->
  integer { s=0; for x in xs { s += x.size() } s }` over `[Box{2,3}, Box{4,5}]` is `26`, identical
  on both backends.
- **Satisfaction failure is static (`G-Check`)** — calling a `T: Sizable` generic with a `struct
  Bare` lacking `size` fails to compile: `'Bare' does not satisfy interface 'Sizable': missing size`.
- **Satisfaction is per SIGNATURE, not per name (`G-Sat`)** — `(G-Sat)` judges against
  `[Self ↦ C](p̄ -> R)`, and the check asked `find_fn`, which takes a name and a receiver and no
  arity.  While no interface could declare one name twice that gap could not be reached; the
  moment `Subtractable` asked for a two-operand `OpMin`, a type providing only the UNARY one
  answered the name, satisfied the bound, and the monomorph called it with one operand too many
  and dropped the second — `diff(a, b)` computed `-a` on both backends with no diagnostic, which
  is loft#1274's defect at the satisfaction site.  The comparison is the VISIBLE parameter count
  on both sides (a struct return carries a hidden buffer an interface declaration does not), and
  the re-ask goes through `possible_with_signature`, the resolver monomorphisation already uses,
  so the two cannot disagree about which definition a signature names.  Oracle:
  `tests/scripts/1275-a-bound-offers-both-arities-of-minus.loft`.
- **No dynamic dispatch (`G-Scope`)** — `x: Sizable = Box{…}` is rejected; an interface names a
  generic bound, never a variable's type.
- **A header binds its OWN variable (`G-Gen`)** — `fn one<T: HasSize1>(x: T)` beside
  `fn two<T: HasSize2>(x: T)`, where the two interfaces declare `sizer` with different
  signatures, compiles and each call resolves against its own bound.  The spelling is shared;
  the variable is not.  (Until 2026-09-02 one placeholder — and so one bound-method stub —
  stood for every `T` in the program, and the second header's calls were checked against the
  first's signatures; `D-gen-3`.)

D-op-1's falsifier applies: any program a monomorphized generic evaluates differently on the two
backends — or that one driver accepts and another rejects — is the definitional error this doc names.
