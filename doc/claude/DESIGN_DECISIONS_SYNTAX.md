<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Design decisions — syntax, names and calls

The entries of the [decision register](DESIGN_DECISIONS.md) about the surface grammar, how a name resolves (imports, methods, shadowing) and how generics are written.  Each states
the decision, its reason and what would reopen it; its **record** link opens the question, the
trade-offs and the evidence in [DESIGN_DECISIONS-history.md](DESIGN_DECISIONS-history.md).

---

## C62 — No type annotations in `|x|` shorthand lambdas

**Decision.** `|x: T| { … }` is a compile error that points at `fn(x: T) -> R { … }`.  The
shorthand is for inferred parameters; the `fn(…)` form is the typed one.  **Why.** Typed
shorthand would make the two forms mean the same thing, turn the choice into a coin-flip, and
add parser branches for no capability the `fn` form lacks.

**Revisit when.** Never, short of a rewrite that removes the inferred/explicit distinction.
Decided 2026-04-17 — [record](DESIGN_DECISIONS-history.md#c62--no-type-annotations-in-x-shorthand-lambdas).
**Catalogue:** @F22 (closures & lambdas).

## C63 — No nested `fn` definitions inside fn bodies

**Decision.** A `fn` definition inside a function or block is refused (`'fn' definitions must be
at file scope, not inside a function or block`); a local helper is a lambda,
`let helper = fn(x: integer) -> integer { x * 2 };`.  **Why.** A nested `fn` either captures
(closures under a second syntax) or does not (surprising against every mainstream language);
both are sugar with no capability closures lack.

**Revisit when.** A real workflow shows the typed-lambda form costing more developer time than
implementing nested `fn` would.  Decided 2026-05-04 — [record](DESIGN_DECISIONS-history.md#c63--no-nested-fn-definitions-inside-fn-bodies).
**Catalogue:** @F16 (functions & declarations).

## C76 — Selective imports group with `()`, not Rust-style `{}`; flat comma list dropped

**Decision.** Import several names as `use lib::(a [as x], b, c);`; a single `use lib::name [as
bind];` is unchanged.  The flat list `use lib::a, b;` is an error ("import multiple names with
parentheses"), and `{}` is not accepted.  **Why.** `()` is loft's grouping delimiter and reads as
"these belong to `lib::`"; `{}` stays reserved for blocks and struct literals.

**Revisit when.** Nested path grouping (`use a::(b::c, d)`) is needed and parses without
colliding with the struct-literal or call grammar.  Decided 2026-06-14 — [record](DESIGN_DECISIONS-history.md#c76--selective-imports-group-with--not-rust-style--flat-comma-list-dropped).
**Holds at:** `tests/imports.rs::pln22_phase4_grouped_import`, `::pln22_phase4_flat_list_rejected`.
**Catalogue:** @F47 (library imports / module system).

## C81 — `&` stays one token, disambiguated by position (bitwise-and vs reference)

**Decision.** One `&` token: infix is bitwise-and, prefix is the reference annotation.  A prefix
`&` is a parse error anywhere but a binding, so position always decides.  **Why.** The rule is
total, Rust makes the same call, and a second sigil would add surface for no semantic gain.

**Revisit when.** A generated parser that cannot reproduce the positional rule hits a real
ambiguity.  Decided 2026-06-24 — [record](DESIGN_DECISIONS-history.md#c81---stays-one-token-disambiguated-by-position-bitwise-and-vs-reference).
**Holds at:** @FR-B-Ref-AnnotationOnly.
**Catalogue:** @F21 (references `&T`), @F37 (operators — bitwise `&`).

## C82 — loft's surface is deliberately not context-free

**Decision.** loft keeps constructs that need backtracking or lexer modes — type versus
variable, `S { … }` versus a block, `"{e}"` interpolation — and does not pursue a context-free
grammar; the hand-written parser is the grammar, and tooling reuses it.  **Why.** Those points buy
real ergonomics, and nothing has needed a generated parser.

**Revisit when.** A third-party grammar-based tool genuinely cannot reuse loft's parser AND the
context-sensitive points can be expressed without unbounded backtracking.
Decided 2026-06-24 — [record](DESIGN_DECISIONS-history.md#c82--lofts-surface-is-deliberately-not-context-free).

## C89 — No tuple-style enum variants; a matcher reads like grammar and is never forced

**Decision.** Enum payloads are always NAMED fields, read as `e.field`; positional variants
(`Ok(a)`) are permanently declined.  A structural matcher (the @PLN35 PEG patterns) stays, on two
conditions: it is never the only way to read data, and its surface reads like grammar notation,
not regex.  **Why.** A wrapped payload forces a matcher just to read a value, and that grows the
whole mitigation layer (`?`, `unwrap`, `if let`, combinators); named fields and `τ?` with `??`
remove the need for it.

**Revisit when.** Never for tuple variants.  The "reads like grammar" bar applies to each PEG
syntax choice.  Decided 2026-07-10 — [record](DESIGN_DECISIONS-history.md#c89--no-tuple-style-enum-variants-a-matcher-reads-like-grammar-and-is-never-forced).
**Holds at:** `@C89` — `tests/scripts/a-declined-form-is-refused-with-its-cure.loft` (the refusal names the cure).

## C92 — Compound assignment evaluates its place expression exactly once

**Decision.** `place op= rhs` (and the keyed-collection forms) evaluates the addressing
sub-expressions of `place` — index expressions, container-producing calls — exactly once, and
reads and writes through the same values.  Part of the contract-1 freeze.  **Why.** A second
evaluation of a side-effecting index reads one slot and writes another, a silent wrong answer;
C, Rust and Python all evaluate the place once.

**Revisit when.** No trigger recorded; frozen with contract 1.  Decided 2026-07-14 — [record](DESIGN_DECISIONS-history.md#c92--compound-assignment-evaluates-its-place-expression-exactly-once).
**Catalogue:** @F2 (operators). Closes the @PLN102 pre-freeze **F2** ("compound-assign double-evaluates its place"). Same class as the F4 assignment eval-order item — an evaluation count/order that would otherwise freeze as impl-defined.

## C95 — A definition a same-named method would silently shadow is a compile error (no silent redefinition)

**Decision.** A free function `foo(x: T, …)` is a compile error at the definition when a method
`foo` for first-argument type `T` exists — calls would dispatch to the method and the function could
never run.  The test is exact on the first-argument type, so sharing a name with a method on another
receiver stays legal.  Frozen with contract 1.  **Why.** An uncallable definition is a latent bug;
letting the user's definition win would let a program silently re-point a stdlib call.

**Revisit when.** No trigger recorded; the predicate follows the mangling if it ever keys on more
than the first parameter.  Decided 2026-07-15 — [record](DESIGN_DECISIONS-history.md#c95--a-definition-a-same-named-method-would-silently-shadow-is-a-compile-error-no-silent-redefinition).
**Catalogue:** @F2 (operators) / naming. Instances the platform rule *no runtime errors, ever* ([C80](DESIGN_DECISIONS_FAILURE.md#c80--the-spreadsheet-fault-model-nothing-stops-a-running-calculation)) at its **compile-time valve**: what cannot work is *disallowed*, not run into a silent-wrong result. Surfaced by C94 (adding stdlib `floor_mod` collided with an existing library helper).
**Holds at:** `src/data.rs::add_fn`.

## C97 — A library's public symbols live under its module, not the global namespace (so the stdlib can grow without breaking a shipped lib)

**Decision.** A library's public symbols are `lib::name`, registered scoped to the library's source;
the stdlib is the only global namespace.  A library may define a free function with a stdlib name
(C95 guards a program's own definitions); a library `self:`/`both:` METHOD named like a stdlib
method still errors — accepted.  **Why.** Global library symbols let any stdlib addition break a
published, immutable library, which absolute compatibility forbids.

**Revisit when.** No trigger recorded; frozen with contract 1.  Decided 2026-07-15 — [record](DESIGN_DECISIONS-history.md#c97--a-librarys-public-symbols-live-under-its-module-not-the-global-namespace-so-the-stdlib-can-grow-without-breaking-a-shipped-lib).
**Catalogue:** @F2 (operators) / modules + naming. A contract-1 compat decision — the precondition for a stdlib that is *both* absolute-compat *and* still growable.
**Holds at:** `src/data.rs::add_fn`; `tests/imports.rs` `pln102_c97_library_may_define_a_stdlib_name`;
`.github/workflows/revalidate-libs.yml`.

## C98 — `use lib;` binds only the `lib` namespace; unqualified access is an EXPLICIT `use lib::*` / `use lib::(…)`, where the imported name wins

**Decision.** `use lib;` binds only the `lib::` qualifier; `use lib::*;` and `use lib::(a, b)`
bind names unqualified for the importing file only; `pub use lib::*;` / `pub use lib::(a, b);`
also pass them on to whoever imports that file (the rustc rule — a package entry offers its
modules this way), and a bare `pub use lib;` is refused, as is a bare `use self::m;` (a package's own
module has no qualifier to bind).  An explicitly imported name wins over a
same-spelled stdlib name, and `as` aliases resolve any clash.  A name that does not resolve
because of its import form says so and names both cures.  **Why.** A qualifying program cannot
collide with stdlib or library growth, an explicit import keeps its binding forever, and a
library's own imports do not leak into its consumers' namespace.

**Revisit when.** A wildcard import against the program's own top-level definition is refused
today (Rust lets the local shadow a glob); relaxing it is open.  Decided 2026-07-15, `pub use`
and the rustc rule 2026-09-28 (owner) — [record](DESIGN_DECISIONS-history.md#c98--use-lib-binds-only-the-lib-namespace-unqualified-access-is-an-explicit-use-lib--use-lib-where-the-imported-name-wins).
**Holds at:** `@C98` — `./scripts/idx tag:@C98`.
**Catalogue:** @F2 (operators) / modules + naming. The name-resolution HOW that [C97](#c97--a-librarys-public-symbols-live-under-its-module-not-the-global-namespace-so-the-stdlib-can-grow-without-breaking-a-shipped-lib) deferred — the rule that makes "the stdlib can grow without breaking a program" actually hold.

## C100 — `print` stays text-only (no bare `print(value)` or variadic `print`)

**Decision.** `print`/`println` take `text` only; any value, or several, print through the format
string (`print("{x}")`, `print("{a} {b}")`).  No bare `print(value)`, no variadic `print`.
**Why.** The capability already ships; the three-character sugar would force `print` to become an
overloaded method, breaking REPL completion and the poisoned-argument error recovery, and a
variadic `print` hides its separator where the format string writes it.

**Revisit when.** The owner wants the bare call enough to accept a print-specific argument
coercion (auto-insert `.to_text()` in the call checker); variadic `print` would also need the
no-variadics stance reversed.  Decided 2026-07-20 — [record](DESIGN_DECISIONS-history.md#c100--print-stays-text-only-no-bare-printvalue-or-variadic-print).
**Catalogue:** @PLN13 (beginner-friendly scripts — step 5).

## C101 — `std`/`core` are reserved package names; `std::name` is the stdlib's qualified form (the shadow escape hatch)

**Decision.** `std::name` is the stdlib's permanent qualified form and reaches a stdlib symbol
that a user definition or an import shadows; it is never required.  `std` and `core` are
reserved package names, refused by `loft new` and by the registry.  **Why.** The resolution rule
freezes with contract 1, so the escape hatch and the names that must never be a library are
fixed now, while no published package uses them.

**Revisit when.** No trigger recorded; frozen with contract 1.  Decided 2026-07-24 — [record](DESIGN_DECISIONS-history.md#c101--stdcore-are-reserved-package-names-stdname-is-the-stdlibs-qualified-form-the-shadow-escape-hatch).
**Catalogue:** @F2 (operators) / modules + naming; completes the C97/C98 namespace model.
**Holds at:** `src/libscan.rs` `RESERVED_PACKAGE_NAMES` / `is_reserved_package_name`; the guard in
`tests/imports.rs`.

## C110 — a generic's type variable stays in the FIRST parameter, and a keyed collection stays a record set

**Decision.** Superseded in part by
[C126](#c126--a-generics-type-variables-are-unrestricted-a-keyed-collection-stays-a-record-set)
(type variables in any parameter; several per generic).  Still holding: keyed collections stay
record sets keyed on a named field (no `hash<K, V>`), and `hole_*` stays per-kind.  **Why.** A
record set carries its own key, so key and value cannot desync and one model covers all five keyed
kinds; a generic `hole<T>` would drop the per-kind opt-in keeping non-literal values out of SQL.

**Revisit when.** A multi-parameter keyed type has a use a record set cannot express.  Decided
2026-08-11, revised 2026-09-21 by C126 — [record](DESIGN_DECISIONS-history.md#c110--a-generics-type-variable-stays-in-the-first-parameter-and-a-keyed-collection-stays-a-record-set).
**Holds at:** `@C110` — `tests/scripts/a-declined-form-is-refused-with-its-cure.loft` (the refusal names the cure).
**Catalogue:** @F26 (interfaces & bounded generics) · @F94 (type-directed interpolation)

## C112 — a binding position mints a local whatever else carries that name; the function stays reachable as a call

**Decision.** A name at any binding position — assignment, typed local, tuple-destructuring element,
parameter, `for` variable, struct field — mints a local whatever function carries that name; the
function stays callable in the same scope (`chr = 65` beside `chr(65)`): parentheses pick the
namespace.  **Why.** Otherwise every short verb a library exports is a word its consumers may not
use.  This does not reopen C95 (a local re-points no call site).

**Revisit when.** No trigger recorded.  Decided 2026-08-11 — [record](DESIGN_DECISIONS-history.md#c112--a-binding-position-mints-a-local-whatever-else-carries-that-name-the-function-stays-reachable-as-a-call).
**Holds at:** `tests/scripts/852-local-shadows-a-function-name.loft`;
`tests/imports.rs::pln102_c98_a_local_may_shadow_a_library_function`.
**Catalogue:** @F2 (operators) / modules + naming · [C98](#c98--use-lib-binds-only-the-lib-namespace-unqualified-access-is-an-explicit-use-lib--use-lib-where-the-imported-name-wins)'s consumer-facing half · loft#852, loft#756

## C123 — one name has one body per receiver type; `both` is how a function takes both spellings

**Decision.** A method and a plain-parameter function of one name whose first parameter has the
same type are refused at the second declaration, in either order and at any arity; a function on
another type is an overload.  `self` and `both` now mean the same, so `both` is deprecated
(warning `both-receiver-deprecated`).  **Why.** When `x.doit()` and `doit(x)` can differ, reading
a line requires knowing how loft resolves it.

**Revisit when.** Never for two behaviours under one name on one type — the cure is a second name.
Remove `both` once no published version declares it.  Decided 2026-09-15 — [record](DESIGN_DECISIONS-history.md#c123--one-name-has-one-body-per-receiver-type-both-is-how-a-function-takes-both-spellings).
**Catalogue:** @F16 (method and function calls) · INC#8 · `formal/calls.md` (F-OneBody)

## C126 — a generic's type variables are unrestricted; a keyed collection stays a record set

**Decision.** A type variable may appear in any parameter, and a function, struct or enum may
declare several; one that appears in no parameter or field is refused.  Kept from C110: keyed
collections stay record sets (no `hash<K, V>`), and the SQL `hole_*` family stays per-kind.
**Why.** "One variable, first parameter only" is a restriction no reader can derive, and `map` /
`reduce` need a second variable to stop being special forms.

**Revisit when.** A keyed type has a use a record set cannot express, or a target wants an OPEN
set of hole kinds.  Decided 2026-09-21 — [record](DESIGN_DECISIONS-history.md#c126--a-generics-type-variables-are-unrestricted-a-keyed-collection-stays-a-record-set).
**Catalogue:** @F25 (generics) · @F26 (bounded generics) · @F94 · revises C110.
