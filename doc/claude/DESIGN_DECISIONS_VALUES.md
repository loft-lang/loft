<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Design decisions — values, null and arithmetic

The entries of the [decision register](DESIGN_DECISIONS.md) about what a value is: the null model, integer and float arithmetic, equality, literals and type names.  Each states
the decision, its reason and what would reopen it; its **record** link opens the question, the
trade-offs and the evidence in [DESIGN_DECISIONS-history.md](DESIGN_DECISIONS-history.md).

---

## C54.D — Rust-style numeric literal suffixes

**Decision.** No literal suffixes (`34u8`, `100i32`); the width comes from the binding or
parameter type, or from `34 as u8`.  **Why.** Context typing already covers every common case;
suffixes would add lexer ambiguity and put the intent on the literal instead of the binding.

**Revisit when.** A real program needs a literal-size distinction `as <T>` cannot express
reasonably ("Rust does it" is not evidence).  Decided 2026-04-13 — [record](DESIGN_DECISIONS-history.md#c54d--rust-style-numeric-literal-suffixes).
**Holds at:** `@C54.D` — `tests/scripts/a-declined-form-is-refused-with-its-cure.loft` (the refusal names the cure).
**Catalogue:** @F4 (width integers), @F5 (type conversions — `as`).

## C64 — Tuple struct-ref elements use MOVE semantics (not copy + null)

**Decision.** A destructured tuple member binds as its TYPE
binds: a plain struct member is a VIEW of the tuple's member (formal/binding.md `(B-View)` — a
struct-typed projection names an interior place), a collection or `text` member is a COPY
(`(B-Copy)`), and a `value struct` member a value.  The tuple stays whole; nothing moves.
**Why.** After `(a, _) = t`, `t` is still read and a write through `a` reaches `t.0`.

**Revisit when.** `(B-View)` changes.  Decided 2026-05-11, superseded 2026-09-28 — [record](DESIGN_DECISIONS-history.md#c64--tuple-struct-ref-elements-use-move-semantics-not-copy--null).
**Holds at:** `@C64` — `tests/scripts/a-tuple-member-destructures-as-its-type-binds.loft`; `tests/tuple_matrix.rs` (the `e5_*` struct-ref cells).
**Catalogue:** @F11 (tuples).

## C65 — Tuple "structure value" element type folded into reference (E5 = E6)

**Decision.** A tuple element of a plain struct type is a reference to a record (E5), and of a
`value struct` type a value (E6): destructuring, projecting or copying the tuple gives an
independent value, on both backends.  **Why.** `value struct` (@PLN101) is loft's by-value struct.

**Revisit when.** Never for the fold; it no longer applies.  Decided 2026-05-11, superseded
2026-09-28 — [record](DESIGN_DECISIONS-history.md#c65--tuple-structure-value-element-type-folded-into-reference-e5--e6).
**Holds at:** `@C65` — `tests/scripts/a-tuple-member-destructures-as-its-type-binds.loft`.
**Catalogue:** @F11 (tuples).

## C69 — `!x` on a non-boolean is a null test, not logical-not

**Decision.** `!x` on a non-boolean asks "is `x` null?", so `!0` is `false`; boolean `!` negates,
with `!null == true`.  `!x` on a statically non-null operand warns that it is always false.
**Why.** Null is in-band and the null-test idiom carries load (`!both` in the stdlib's
`min`/`max`/`clamp`); C-style coercion would invert it, a compile error reject it.

**Revisit when.** The idiom is shown to cause a recurring class of bugs the warning and the
documentation miss, AND a replacement is prototyped that does not churn the stdlib ("I expected
C semantics" is not evidence).  Decided 2026-06-03 — [record](DESIGN_DECISIONS-history.md#c69--x-on-a-non-boolean-is-a-null-test-not-logical-not).
**Holds at:** `tests/parse_errors.rs::gh253_bang_on_not_null_warns`, `gh253_bang_on_nullable_is_quiet`.
**Catalogue:** @F37 (operators — unary `!`), @F1 (null model).

## C73 — `boolean` is three-state (false / true / null); `==` is raw, truthiness coerces

**Decision.** A `boolean` is one byte: `false` = 0, `true` = 1, `null` = 255, kept distinct
everywhere a boolean lives.  `==` and `!=` compare the raw value, so `null == false` is `false`
and `b == null` is the null test; `if`, `while`, `!`, `&&` and `||` treat null as false; `??`
tests for null, so `false ?? x` stays `false`.  **Why.** This is exactly what `integer` already
does (`0 == null` is `false`); coercing inside `==` would make boolean the one inconsistent type.

**Revisit when.** Never, short of a change to the in-band null model.  Decided 2026-06-10 — [record](DESIGN_DECISIONS-history.md#c73--boolean-is-three-state-false--true--null--is-raw-truthiness-coerces).
**Holds at:** `tests/scripts/292-pln17-three-state-boolean.loft`.
**Catalogue:** @F3 (scalar types — boolean), @F1 (null model).

## @PLN116 — the `x?` default-fallback operator + enum-field non-null soundness

**Decision.** Postfix `x?` (tightest precedence) yields `x`, or the type's default when `x` is
null; `a ?? b?` parses as `a ?? (b?)`.  One predicate decides "T has a default" for both `x?`
and an omitted field in `S{}`.  A non-optional enum field with no `= expr` has no default, so
`x?` on it and an `S{}` that omits it are compile errors; `x?` on an `E?` value gives the first
variant, and an optional enum field (`Color?`) defaults to null.  **Why.** An enum's 0 is its
null, so zero-filling a non-null enum field would put null into a non-null slot.

**Revisit when.** An enum can nominate its own default variant — an additive extension, not a
change to this rule.  Decided before contract 1 — [record](DESIGN_DECISIONS-history.md#pln116--the-x-default-fallback-operator--enum-field-non-null-soundness).
**Holds at:** [plans/116-default-fallback-operator/](plans/116-default-fallback-operator/README.md).

## C85 — Overflow arithmetic types NON-null; the game keeps running (don't force `integer?` on every `*`/`+`/`-`)

**Decision.** `a*b`, `a+b` and `a-b` on `integer` type non-null; an overflow yields the null
sentinel at run time and execution continues — no trap, no wrap.  The reachable-fault operations
(`/`, `%`, `v[i]`, `s[i]`, text→integer parse) stay `τ?`.  A value equal to the sentinel
(`i64::MIN`, computed or read as data) reads as null; that collision is accepted.  **Why.**
`integer?` on the ubiquitous `*`/`+`/`-` would tax every expression for a fault players never hit; a
detectable null beats a plausible wrapped value.

**Revisit when.** No trigger recorded; the wide-`integer` sentinel is part of the contract-1
freeze, and narrow declared ranges follow [C127](#c127--a-narrow-type-without--has-no-null-an-unfitting-value-takes-the-types-default-and-says-so).  Ratified 2026-07-13 — [record](DESIGN_DECISIONS-history.md#c85--overflow-arithmetic-types-non-null-the-game-keeps-running-dont-force-integer-on-every--).
**Catalogue:** @F38 (arithmetic safety), @F1 (null model). Refines [C80](DESIGN_DECISIONS_FAILURE.md#c80--the-spreadsheet-fault-model-nothing-stops-a-running-calculation) and the @PLN25 `(N-Div)`/`(N-Arith)` rules (formal/types.md § DN3).

## C90 — Each nullable scalar reserves ONE bit-pattern for null (the in-band sentinel residual; accepted, frozen)

**Decision.** Each nullable scalar keeps one in-band reserved value, frozen with contract 1:
`integer` `i64::MIN`; narrow widths their top stored value (`u8?` → `255`); `boolean` `255`;
`character` codepoint `0`; `float`/`single` a reserved `NaN` (`±inf` are real values).  References
(`nullref`) and structs in a vector (`__nullable<S>`) are out-of-band.  A `τ?` is as wide as `τ`;
its reserved value cannot be stored as data.  **Why.** A tagged representation would widen every
`τ?`, un-densify `vector<τ?>` and reopen the layout to win back one extreme value per type; the
collision sites are guarded.

**Revisit when.** No trigger recorded; frozen with contract 1.  Decided 2026-07-13 — [record](DESIGN_DECISIONS-history.md#c90--each-nullable-scalar-reserves-one-bit-pattern-for-null-the-in-band-sentinel-residual-accepted-frozen).
**Catalogue:** @F1 (null / Optional), @F3 (scalar types), @F4 (width integers). Closes the @PLN102 pre-freeze [null-model keystone](plans/102-stability-contract/keystone-null-model.md) (option B); the sibling of [C85](#c85--overflow-arithmetic-types-non-null-the-game-keeps-running-dont-force-integer-on-every--) (overflow yields the sentinel) and [C80](DESIGN_DECISIONS_FAILURE.md#c80--the-spreadsheet-fault-model-nothing-stops-a-running-calculation) (the spreadsheet model).
**Holds at:** `tests/scripts/pln102-null-residual-golden.loft`.

## C91 — `==` compares content for every type, cycles included; `&a == &b` asks identity

**Decision.** `==`/`!=` compare CONTENT for every type — scalars, `text`, `struct`, `value struct`,
vectors, tuples, keyed collections — recursively through what the value owns and through
`reference<T>`, and terminate on cyclic values (a pair met again while it is being compared counts
as equal).  Identity is asked in the source: `&a == &b` / `&a != &b` compare whether both name the
same record; `&` on only one side is refused, and a scalar or `text` has no record to name.  That
two operands are one record is only the fast path of content equality, never its meaning, and a
`value struct` is a layout choice with the same `==`.  Key comparison and hashing in keyed
collections agree with `==`.  No `===`.  **Why.** Every variable is its own value (C86), so an
identity `==` would answer by where a value is stored.  What `==` means is
decided by what is written; how fast it is, by the compiler.

**Holds at:** `@C91` — `./scripts/idx tag:@C91`: the `tests/scripts/c91-*.loft` guards (content,
collections, vectors, identity, keys, generics) and `tests/store_content_eq.rs`, both backends.

**Revisit when.** A consumer's content comparison costs more than it can carry and it truly asks
identity — the cure is `&a == &b`, not a cheaper `==`.  Decided 2026-07-13, content everywhere
and `&a == &b` 2026-09-28 (owner) — [record](DESIGN_DECISIONS-history.md#c91---is-value-by-value--reference-by-identity-bounded-never-a-reference-chase--reserved-for-opt-in-deep-equality).
**Catalogue:** @F1 (null / equality), @F2 (operators). Sibling of [C86](DESIGN_DECISIONS_OWNERSHIP.md#c86--whole-value-heap-binds-copy-aliasing-is-a-last-use-elision-the-rustc-rule) (whole-value copy) and [C90](#c90--each-nullable-scalar-reserves-one-bit-pattern-for-null-the-in-band-sentinel-residual-accepted-frozen) (the in-band sentinels).

## C94 — Integer `/` truncates toward zero and `%` takes the dividend's sign; `floor_mod` is the wrap-around helper

**Decision.** Integer `/` truncates toward zero (`-7 / 2 == -3`) and `%` takes the dividend's
sign (`-7 % 2 == -1`), so `a == (a / b) * b + a % b`; division by zero is null.  The wrap-around
case is `x.floor_mod(n)`, landing in `[0, n)` for a positive `n` (`(-1).floor_mod(3) == 2`).
**Why.** `/` and `%` are a matched pair, so the choice is which `/` is least surprising; cyclic
indexing is a distinct operation, and naming it makes the wrap visible at the call site.

**Revisit when.** No trigger recorded; the operator convention is frozen with contract 1.
Decided 2026-07-15 — [record](DESIGN_DECISIONS-history.md#c94--integer--truncates-toward-zero-and--takes-the-dividends-sign-floor_mod-is-the-wrap-around-helper).
**Catalogue:** @F2 (operators) / math. Fixes the sign convention for negative integer division at contract 1 — the one place a language must *pick* (C, Rust, Go, Java, JS truncate; Python, Ruby floor). Both are legitimate; the freeze needs one named default.

## C103 — `int` / `str` / `bool` are suggested, never legal (no cross-language type aliases)

**Decision.** `int`, `str`, `string`, `bool`, `i64`, `u64`, `f64`, `f32`, `char`, `double` and
`long` stay undefined; the error names the loft type (`data::builtin_type_alias`, a table, since
edit distance cannot reach these).  **Why.** Full-word type names mean fewer acronyms for a
newcomer and a small nudge against annotations inference makes pointless; an alias is permanent
surface after contract 1; and the lookalikes are not identities (`integer` excludes `i64::MIN`, the
null sentinel), which a suggestion tolerates and an alias would make silent.

**Revisit when.** Evidence that the suggestion is not landing — people hitting it repeatedly, not
once.  Decided 2026-07-24 — [record](DESIGN_DECISIONS-history.md#c103--int--str--bool-are-suggested-never-legal-no-cross-language-type-aliases).
**Holds at:** `tests/parse_errors.rs::t03_cross_language_type_aliases_suggest`.

## C119 — a tuple type is never nullable; a tuple that arrives absent is a present tuple of null members

**Decision.** A written `(τ, τ)?` is refused, naming its cures.  Wherever absence is synthesised
for a tuple, the answer is the present tuple of null members:
`optional((τ₁, …, τₙ)) ≡ (τ₁?, …, τₙ?)`.  **Why.** A tagged tuple layout would be re-asserted by
every tuple function and emitter, each a place to misread bytes silently; a tuple has no faithful
document form, so "every member null" is the natural reading of absence.

**Revisit when.** A consumer shows "absent tuple" and "tuple of nulls" carry different information.
Decided 2026-09-07 (loft#1423, loft#1451) — [record](DESIGN_DECISIONS-history.md#c119--a-tuple-type-is-never-nullable-a-tuple-that-arrives-absent-is-a-present-tuple-of-null-members).
Holds at `(T-Absent)`, [formal/tuples.md](formal/tuples.md).
**Catalogue:** @F1 (null model), @F (tuples).

## C120 — Integer arithmetic on native stays sentinel-aware after a fault; the non-null proof does not close over `+`, `-`, `*`

**Decision.** `+`, `-` or `*` over non-null operands is not proven non-null, so after an overflow
native carries null exactly as the interpreter does.  A plain operator is emitted only where a fact
established BEFORE the arithmetic rules out a fault (`(R-Range)`, `(R-GuardedChain)`,
`(R-BoundedNest)`).  A published library always keeps the checks; a check-free release pass for a
game with fault-free checked runs may come, never as a default.  **Why.** A rewrite must leave every
observable value as the interpreter answers it, faults included.

**Revisit when.** Nothing reopens it; the successor is a range proof.  Decided 2026-09-15 — [record](DESIGN_DECISIONS-history.md#c120--integer-arithmetic-on-native-stays-sentinel-aware-after-a-fault-the-non-null-proof-does-not-close-over----).
Holds at [NATIVE.md § Optimisation tiers](NATIVE.md#optimisation-tiers--semantics-runs-performance-lanes-shipped-binaries).

## C127 — A narrow type without `?` has no null: an unfitting value takes the type's DEFAULT, and says so

**Decision.** A declared narrow range without `?` (`limit(lo, hi)` and the aliases
`u8`/`i8`/`u16`/`i16`/`u32`) has no null: an unfitting value takes the type's DEFAULT — zero where
the range holds it, else the bound nearest zero — for a local, field, element, parameter and return
alike.  Where the author can be made to choose, the narrowing is refused (`?`, `?? d` or `as T?`
cures it) — a local, a parameter, a field, an element, a cast; the one shape it cannot ask about,
a compound step (`x += 10`), carries the advice `narrow-fallback` — advice, because the default IS
the promised answer (the two-tier rule).  Plain `integer` and `i32` keep C85's sentinel — a
PRAGMATIC exemption, not a line to extend.  **Why.** Null-on-overflow depended on whether the range
left a spare code, so two non-nullable declarations behaved oppositely.

**Revisit when.** The `i32` exemption's cost changes (a program needs the full 32-bit range); that
clause is one predicate.  Decided 2026-09-23 — [record](DESIGN_DECISIONS-history.md#c127--a-narrow-type-without--has-no-null-an-unfitting-value-takes-the-types-default-and-says-so).
**Catalogue:** @F4 (width integers), @F1 (null model). Refines [C85](#c85--overflow-arithmetic-types-non-null-the-game-keeps-running-dont-force-integer-on-every--) at the narrow end and settles `formal/types.md` `(N-Reserve)` against `(E-Uncomp-NN)`.
**Holds at:** `@C127` — `./scripts/idx tag:@C127`; `IntegerSpec::non_null_reads_null`;
`tests/scripts/1615-every-narrow-slot-answers-the-types-default-for-an-unfitting-value.loft`.

## C129 — No opt-in to the processor's arithmetic (no machine-dependent scope or type)

**Decision.** No declaration, scope, type or mode licenses wrapping integers or IEEE `inf`/`NaN`
as values; the sentinel checks are the semantics.  A check is retired only where the compiler
PROVES the value cannot be the sentinel.  The one opening kept is C120's evidence-licensed
release tier.  **Why.** Machine-dependent arithmetic makes one program answer differently per
target, silently; a value-range proof is portable by construction.

**Revisit when.** Never for the machine-dependent form.  Decided 2026-09-08, amended 2026-09-15
(renumbered from a duplicate C67) — [record](DESIGN_DECISIONS-history.md#c129--no-opt-in-to-the-processors-arithmetic-no-machine-dependent-scope-or-type).
**Catalogue:** @F38 (arithmetic safety).
