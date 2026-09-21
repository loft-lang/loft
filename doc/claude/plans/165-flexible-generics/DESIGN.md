<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 165 — Design: generic parameters in full

The plan is [README.md](README.md); the ordered work is [STEPS.md](STEPS.md).  This file is the
hypothesis those steps test: the invariant, the ways it breaks, and the decisions a step cannot
take for itself.  Every claim about today's compiler below was **run**, on `9f5cf6a96`, and its
program is in [`probes/`](probes/) with the answer it gave written at the top.

## The invariant

> **A generic never survives the parser.  Every use of a generic definition — a function or a
> type — names a concrete INSTANCE.  The instance's identity is a function of its template and
> of ALL its bound types, and the instance answers exactly what a hand-written concrete twin
> answers.**

Three clauses, and each has one home:

| Clause | Says | Home |
|---|---|---|
| `G-Mono` *(exists)* | an instance equals its hand-written twin — value, length and leak, on both backends | `formal/interfaces.md`; `Parser::instantiate_template` |
| `G-Key` *(new)* | an instance's key is an injective function of (template, every bound type) — ONE encoder, ONE decoder | `Data::mangle_method` + the decoder step A1 adds |
| `G-Select` *(new)* | a generic is a member of its name's overload set, and which definition a call reaches is a function of the argument types alone | `parser/dispatch.rs` — @PLN162's invariant, unchanged, asserted at one more kind of member |

`G-Mono` is why this is affordable: a backend never sees a generic.  The interpreter, `--native`
and wasm compile ordinary functions and ordinary structs, so every step below is a PARSER step
and its proof is a comparison of emitted IR against a twin written by hand.

## What is already there

Measured, not read.  The owner's hypothesis — *"more flexible generics need almost no extra
infrastructure"* — holds further than the plan's own table says, because two siblings it did
not count already do the hard half of generic TYPES.

| Need | The sibling that already does it | Where |
|---|---|---|
| select by every parameter's type | @PLN162: rank vector per definition, minimal elements of the pointwise order | `dispatch.rs:50` `dispatch_rank`, `:273` `select_overload` |
| a key carrying several types | `f_10Rock#Paper_beat` — types joined with `#` inside the length-counted part | `data.rs:7306` `full_spelling`, `:7494` `mangle_free_overload` |
| bind a variable under any type former | `resolve_type_var` descends `Type::zip_children`, so `T` under `vector<T>`, `(T, U)`, `fn(T) -> T`, `T?` is found one way | `parser/mod.rs:8446` |
| substitute a LIST of holders | `bindings: Vec<(u32, Type)>` — the type variable first, each associated type after (@PLN125 A2c) | `parser/mod.rs:7600`, `substitute_all` `:9213` |
| sites whose lowering depends on `T` | six `TV_*` stamped blocks, re-lowered per instance; five of the six read their concrete type off the block's OWN `result` | `parser/mod.rs:9605` `rewrite_generic_type_defaults` |
| a method TEMPLATE keyed on a type constructor | `fn head<T>(self: vector<T>)` is `t_6vector_head` and instantiates from a method call | loft#1539, `instantiate_template` |
| **a struct minted per type-argument tuple, by name, idempotently, deferred while a member is unresolved, registered for every source** | `Data::tuple_def` → `__tuple<integer,text>`; `vector_def` → `main_vector<τ>`; `__nullable<S>` | `data.rs:8648`, `:8536` |

The last row is the plan's *"generic structs: **no**"*.  A generic struct instance
`Pair<integer,text>` is what `tuple_def` already builds, with the field names and types taken
from a declared template instead of from `_0`, `_1`.  The lessons that function paid for —
loft#944 (a pass-1 name differing from the pass-2 name is an H5 contract failure), loft#1503
(the name and the layout must erase the same things) — are the first two failure paths of
phase D, already written down.

## Five findings that shape the steps

**1. The plan's headline cell cannot be keyed today, and no generic is involved.**
`fn first(v: vector<integer>)` beside `fn first(v: vector<text>)` is refused *"Cannot
redefine"* ([b1](probes/b1-vector-element-overload.loft)).  `full_spelling`'s own doc says why:
*"Two `vector<τ>` spell alike — the element type is not in a key today either."*  So admitting
a `Generic` into a set — the plan's phase 1 as written — would still refuse
`first<T>(v: vector<T>)` beside `first(v: vector<integer>)`, as one key written twice.  The key
and the rank must read a type's FULL identity first (step B1).  The monomorph namer already
learned this twice (loft#1024 for collections, loft#1383 and loft#1418 for integer widths): it
is the second speller of one question, and B1 gives the question one home.

**2. "Concrete beats bound" works today by a key collision, not by a rank.**
`fn describe(self: Sq)` beside `fn describe<T: Shape>(x: T)` answers `SQUARE 1|some tri 2`
([b4](probes/b4-concrete-method-beside-bounded-generic.loft)) because the method is keyed
`t_2Sq_describe` and the ladder finds it by the receiver ALONE.  Give the method a second
parameter and the one-argument call is captured by it and refused — *"missing argument for
parameter 'n'"* — while the generic takes exactly that call
([b5](probes/b5-method-arity-captures-the-call.loft)).  `Disp-Applicable` says the method does
not apply; nothing asked.  A wrong refusal, on `main`, closed by B4.

**3. The key has one encoder and twelve decoders.**  @PLN162 step 1 gave the spelling one home
(`mangle_method`).  Reading it back did not follow: `Definition::original_name` has 68 callers
and reads a **two-digit** length, and 11 more sites strip `t_` and split at the first
underscore.  A 121-character key dispatches correctly on both backends
([a1](probes/a1-key-over-99-chars.loft)) and `original_name` answers garbage for it.  Keys that
carry every bound type reach 100 characters easily, so the decoder is step A1 — before anything
makes keys longer.

**4. A type variable is a global definition.**  The placeholder for `<T>` is an attribute-less
`Struct` named `T`, visible to every later signature: `fn g(x: T)` compiles with no header
([d2](probes/d2-type-variable-leaks-its-header.loft)), and `struct Holder { v: T }` reaches the
layout pass and fails there naming `__typevar_T`, at the file's last line
([d3](probes/d3-struct-field-typed-by-a-leaked-variable.loft)).  Harmless while only function
headers name variables; load-bearing the moment a struct has its own `<T>` (step D1).

**5. An instance is keyed as a METHOD, and takes a real method for itself.**
`count_of<T>(v: vector<T>)` at `T = Cat` is keyed `t_3Cat_count_of` — also the key of a method
`count_of` on `Cat`.  The instantiation asks *"does this key exist?"*, finds the method, and
answers it as its own instance: two definitions that are each legal, and legal together, are
refused *"expected Cat, got vector<Cat>"* in both declaration orders
([a2](probes/a2-instance-key-meets-a-method-key.loft)).  A wrong refusal on `main`, and the
FOURTH repair of one key: loft#1024 (collections erased), loft#1383 and loft#1418 (integer
widths erased), loft#1539 (a `__self_` marker to keep two templates apart).  Four patches to one
rule is the sign the rule is wrong: an instance is not a method on its first bound type, and its
key should stop saying so (step A5).  This probe was written to attack this design's own
cleanest claim — *"with one variable the key is today's, byte for byte"* — and falsified it.

## Failure paths

Written before any code, because this list is where the invariant was found.  Each is closed
by a named step and watched by a named cell.

| # | How it breaks | Closed by | Cell |
|---|---|---|---|
| F1 | two overloads differ only inside a collection or by integer width, and get one key | B1 | b1 |
| F2 | a generic and a concrete definition of one name are a redefinition | B2–B3 | b2, b3 |
| F3 | the ladder picks a method by receiver key and never asks `Disp-Applicable` | B4 | b5 |
| F4 | two generics apply and neither is ranked (`vector<T>` vs `T`; `<T: A + B>` vs `<T: A>`) | B5–B6 | b6 |
| F5 | a generic member reaches codegen through its `Dynamic` dispatcher's attribute list | B2 | emission diff: no template symbol |
| F6 | a dynamic (`Disp-Dynamic`) leaf or an open-world stub is a template | B7 | oracle twin |
| F7 | a key of 100+ characters decodes to the wrong name | A1 | a1 + unit test |
| F7b | an instance key equals a concrete method's key, and the method is taken for the instance | A5 | a2 |
| F7c | two templates of one name, bound alike, mint ONE instance | A5 (the key names its template) | b6 |
| F7d | two keys differ only in characters the native emitter maps to `_` | A5 (the emitter refuses a duplicate identifier) | a generation-time check |
| F8 | `K, V` bound to `(integer, text)` and `(text, integer)` share an instance | C3 | c1 |
| F9 | one variable named by two parameters binds two types | C1 | c3 |
| F10 | a variable named by NO parameter cannot be inferred | C2 | refusal cell |
| F11 | an instance name minted on pass 1 differs on pass 2 (H5; loft#944's class) | C3, D3 | a forward-declared argument type |
| F12 | the instance name and the instance layout erase different things (loft#1503's class) | D3 | two instances whose arguments differ only in deps |
| F13 | an associated type's binding assumes the variable is `bindings[0]` | C4 | @PLN125's generic over an associated type, at two variables |
| F14 | a type variable is visible outside its header | D1 | d2, d3 |
| F15 | a template's field typed `T` reaches layout (zero width — loft#1536's class) | D2 | d1 |
| F16 | `struct Bad<T> { n: Bad<vector<T>> }` mints instances without end | D7 | refusal cell |
| F17 | `struct Node<T> { next: Node<T>? }` asks for its own instance while it is being built | D7 | recursion cell |
| F18 | native's `init()` replays parse-time type order and an instance is minted out of order | D3, D8 | `LOFT_STRICT_SCHEMA_IDS=1` |
| F19 | a cached IR image or a library's `api_surface` has no spelling for a type template | D2 | `ir_schema_roundtrip`, `make api-compat` |
| F20 | a diagnostic or the debugger shows `Pair_integer_text_` where the author wrote `Pair<integer, text>` | D3 | `@EXPECT_ERROR` cells read the message |
| F21 | `first<integer>(a)` is read as a chained comparison | decision `D-Infer` | e1 |

## The re-assertion count

`N × silence` is the brittleness, known before any code.

| Invariant | Sites that must agree today | After |
|---|---|---|
| how a key is read back | 12 (`original_name` + 11 inline) — omission is **silent** (a wrong name, no error) | 1 (A1) |
| what a type's identity is in a key | 2 (`type_spelling`; the monomorph namer in `instantiate_template`) — silent: one key for two types | 1 (A5 extracts it, B1 adopts it) |
| how a key becomes a Rust identifier | 2 (the namer's `< > , ( )` → `_` at mint time; `generation::sanitize`'s `#` → `__`) — silent: two keys, one symbol | 1, in the emitter (A5) |
| "which is THE type variable" | 5 `extract_type_var(attributes()[0])` + 17 `cur_type_var` reads — silent: binds the wrong variable | 1 accessor (A3) |
| "the concrete type" of an instantiation | 11 `concrete: &Type` parameters; ONE real reader (`TV_DEFAULT_BLOCK`) | 0 — every site reads its own `result` (A2) |
| every member of a set is callable | `Type::Routine` read at ~50 sites across 15 files — most never ask | 1 accessor that says `Function` or `Template` (B2) |

## Decisions

Each takes the refusing side where the language could either refuse or choose, for the reason
@PLN162's RULES.md gives: a refusal can be broadened later without moving any program; a choice
cannot.  `CONTRACT_VERSION` is 0, so this is the one era in which the distinction is free.

**`D-Key`.**  An instance has a key kind of its own: `i_<LEN><τ₁>#<τ₂>…_<template key>`.
The length-counted part holds every bound type's identity spelling, in the order the variables
FIRST APPEAR in the parameter list, joined with `#`; what follows is the TEMPLATE's own key
(`n_idf`, or its `f_…` key once it is a member of a set), so the pair (template, bound types)
is spelled whole and two templates of one name cannot share an instance.  First appearance, not
header order, because it is derivable from the parameters alone — no new field on a definition,
no IR schema change.  The key keeps its readable characters; making it a Rust identifier is the
emitter's job, in one place.  This renames every existing instance once (`t_7integer_idf`
becomes `i_7integer_n_idf`), which step A5 proves is a rename and nothing else.

**`D-Rank`.**  A parameter that names a type variable ranks `GENERIC` where it binds.  `GENERIC`
is worse than `EXACT`, `WIDENED` and `LOSSY` — a concrete definition that takes the argument as
it is wins, which is RULES.md's *"a concrete type is more specific than a bound that admits
it"*.  `GENERIC` against `CONVERTED` is **incomparable**: `f(x: float)` and `f<T>(x: T)` called
with an `integer` is refused as ambiguous, naming both.  One needs an implicit conversion, the
other an instantiation, and neither is derivable as "more specific" from the signatures.

**`D-Specific`.**  Between two generic positions, more specific means *admits strictly fewer
types* — the same inclusion that already orders a variant under its enum:

- a bound set that is a strict superset is more specific (`<T: Ordered + Printable>` beats
  `<T: Ordered>` beats `<T>`); two sets neither of which contains the other are incomparable;
- a pattern that is a substitution instance of the other is more specific (`vector<T>` beats
  `T`); two patterns neither of which is an instance of the other are incomparable.

**`D-Kind`.**  A bound against an enum stays incomparable — RULES.md decided it, this plan
does not reopen it.  That answers the plan's open question 3.

**`D-Infer`.**  Type arguments are written in TYPE position only — `p: Pair<integer, text>`,
where `<` already means what it means for `vector<integer>` — and inferred everywhere else: a
call from its arguments, a literal from its field values or from the binding's annotation (the
way `v: vector<integer> = []` already types an empty literal).  `first<integer>(a)` stays
refused, with a message that names the cure.  No expression grammar changes.  That answers open
question 2.

**`D-Every-Var`.**  Every declared variable must appear in a parameter (a function) or a field
(a struct).  The first-parameter rule becomes this rule; it is refused at the declaration.

**`D-Scope`.**  A type variable is a type only inside the definition whose header declares it.

**`D-Template`.**  A generic struct or enum is its own definition kind.  Most sites test
`def_type == Struct`, and to every one of them a template then reads as *not a struct* — inert
by default, which is the safe direction, where a flagged `Struct` would be laid out by the
first site that forgot the flag.

**`D-Regular`.**  A template may mention itself only at its own variables, unchanged
(`Node<T>` inside `Node<T>`).  `Bad<vector<T>>` inside `Bad<T>` is refused at the declaration:
it has no finite set of instances.

**`D-Keyed`.**  A keyed collection over a type variable stays refused at the definition
(loft#1538's message).  C110 (b) stands whole: this plan proposes no `hash<K, V>`, and a
user-written `Map<K, V>` is a struct over two vectors — a different construct.  That answers
open question 4.

## The C110 revisit — what the owner is asked to record

Phases C and D need it; A, B and E do not touch it.  A plan cannot supersede a register entry,
so step C0 is the owner writing the revisit into `DESIGN_DECISIONS.md`, and this section is the
material for it — including what argues against.

**What changed since C110 closed.**  Its cost argument: *"the name must encode a second type
and every back-parsing site follows."*  @PLN162 has since put several types inside the key's
length-counted part for every overload set, and the emitter maps `#`.  The cost C110 priced is
largely paid — what remains is step A1, which `main` needs anyway (finding 3).

**What did not change.**  C110's bar is *"a real consumer, not a shape that reads more familiar
from another language."*  Measured against that bar: the published libraries declare **zero**
generics; the stdlib declares three (`min_of`, `max_of`, `sum`); 59 corpus files use one.  Five
library files hold paired parallel vectors (the `Map<K, V>` shape) and none was examined for
whether a record set serves it better, which for a keyed lookup C110 (b) says it does.  The
evidence for phases C and D is the owner's direction and the infrastructure now being cheap.
It is not a consumer asking.

**The `hole_*` safety refusal survives.**  C110's named harm was a generic `hole<T>` deleting
@PLN124's per-kind opt-in.  Nothing here touches that family: `D-Every-Var` admits
`hole<T>(self: Acc, v: T)` as a shape, and whether a library USES it is that library's decision,
which C110's third bullet already took.

**Proposed wording** — *"Revised by @PLN165: (a) a type variable may appear in any parameter
and a generic may declare several, keyed by `D-Key`; (b) a struct or enum may be generic.
C110 (b)'s record-set decision stands — no `hash<K, V>` — and so does the `hole_*` family's
per-kind form.  Recorded on the owner's direction; no consumer had asked."*

## Out of scope

A generic as a fn-ref value ([e2](probes/e2-generic-as-fn-ref.loft) — it has no single address
before it is instantiated; refused with its reason in step A4); explicit type application in
expressions; default type arguments; a bound on a generic TYPE's variable that adds fields
(a field-bearing bound is a later plan, per `D-Keyed`); variance; specialising a generic type
per argument.
