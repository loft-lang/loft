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
| type a short lambda from the parameter it is passed to | works for a concrete callee, and for the built-ins through a by-name table (`("map", 1)`); only the generic callee is missing | `parser/control.rs:17348`; e3 |
| a method TEMPLATE keyed on a type constructor | `fn head<T>(self: vector<T>)` is `t_6vector_head` and instantiates from a method call | loft#1539, `instantiate_template` |
| **a struct minted per type-argument tuple, by name, idempotently, deferred while a member is unresolved, registered for every source** | `Data::tuple_def` → `__tuple<integer,text>`; `vector_def` → `main_vector<τ>`; `__nullable<S>` | `data.rs:8648`, `:8536` |

The last row is the plan's *"generic structs: **no**"*.  A generic struct instance
`Pair<integer,text>` is what `tuple_def` already builds, with the field names and types taken
from a declared template instead of from `_0`, `_1`.  The lessons that function paid for —
loft#944 (a pass-1 name differing from the pass-2 name is an H5 contract failure), loft#1503
(the name and the layout must erase the same things) — are the first two failure paths of
phase D, already written down.

## Six findings that shape the steps

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

**6. A lambda passed to a generic is typed from the template's `T`, not from what `T` is
bound to.**  A short lambda takes its parameter type from the parameter it is handed to: the
built-in `map(a, |x| { x * 10 })` answers `[10,20,30]`, and so does a CONCRETE function
declared `f: fn(integer) -> integer`.  A generic declared `f: fn(T) -> T` hands the lambda the
variable itself, although the first argument has already bound it: *"No matching operator '*'
on 'T' and 'integer'"* ([e3](probes/e3-short-lambda-under-a-generic.loft)).  A third wrong
refusal on `main`, and the reason `map` cannot yet be an ordinary library generic even at one
variable (step A6).

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
| F7e | a lambda argument is typed from the template's parameter, not from the bindings the earlier arguments made | A6 | e3 |
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
(loft#1538's message).  C110's record-set decision stands whole: this plan proposes no
`hash<K, V>`, and does not offer a hand-built `Map<K, V>` as a reason for anything — a keyed
lookup is what a record set is for.  That answers open question 4.

## C110, evaluated

The material for step C0, which is done: the owner revised C110 as C126 on 2026-09-21, and
the register entry is the decision — this section is the measurement it rests on.  Each of
C110's reasons, run against `9f5cf6a96`, and what the measurement leaves standing.

| C110's reason | Verdict | Measured |
|---|---|---|
| The monomorph's name is its identity, built *"from the FIRST argument's type"*, and four readers must follow a second type: `find_fn`, native, `original_name`, the H5 guard | **no longer holds** | The name is built from what `T` BINDS to: `first<T>(v: vector<T>)` at `integer` is `t_7integer_first`.  `find_fn` looks under the first argument (`vector`) and never finds it — `LOFT_TRACE=call` shows both calls of `first(a)` resolving through the template.  All three stdlib generics have that shape.  Where `find_fn` DOES find an instance, that is finding 5's defect.  `h5_split_mangled` reads every leading digit and only the function-name half; native emits whatever the key is.  Three readers are indifferent to the key's content, and the fourth is a bug |
| *"The cost is understood — it has simply never been worth paying"* (C95: *"when mangling extends beyond the first parameter…"*) | **overtaken** | @PLN162 extended it on 2026-09-14: every overload key carries every parameter's type (`f_10Rock#Paper_beat`) and the emitter maps `#` |
| The sole named consumer is HARMED: *"a generic `hole<T>` accepts every type by construction"*, deleting the per-kind opt-in | **false as stated; stands on narrower ground** | `hole<T: SqlHole>(v: T)` called with a type that has not opted in is refused *"'Raw' does not satisfy interface 'SqlHole': missing sql_hole"* — a compile error naming the method to add, the property C110 says a generic deletes.  A BOUND is a per-kind opt-in.  What does change is who owns the list: the target's author today, anyone who implements the interface under a bound.  For a surface whose audit is *"read one target's method list"* that is a real difference, so the `hole_*` family stays per-kind.  It was a reason for one consumer not to use the feature, never a reason against the feature |
| `hash<K, V>` would be a second spelling of a keyed collection, and a worse one | **holds in full** | Nothing has changed, and `D-Keyed` keeps it |

**The consumer C110 asked for is the language itself.**  `map` is
`vector<T> × fn(T) -> U → vector<U>`, and `reduce` is `vector<T> × U × fn(U, T) -> U`: two
type variables each, shipped as compiler special forms (`parse_map`, `parse_reduce`) because a
library cannot say `U`.  The H5 guard carries an exemption for exactly this — *"For `map` the
OUTPUT element wrapper is unknowable in pass 1"*.  Measured: a program can write the `T → T`
half as a library generic today (`[10,20,30]`) and cannot write `T → U`, which the built-in
answers (`["<1>","<2>","<3>"]`).  The owner's rule of 2026-09-15 is *no observable special
names*; arc E's first list (`insert`, `sort`, `reverse`, `reserve`) is the ONE-variable
built-ins, and `map` and `reduce` are missing from it because they need two.  That is new
evidence in C110's own sense — a use, not a shape that reads familiar from another language.
No LIBRARY consumer was found: the published libraries declare zero generics.

**What C110 never decided.**  Its reasons against *"multiple type variables"* are all about
keyed collections; its decision says a case for multi-parameter generics *"must argue for
itself"* — they were set aside, not evaluated.  And a generic struct with ONE variable
(`struct Grid<T>`) is named nowhere in the register: it is unbuilt, not declined.  So arc D
needs no revisit until its one step that gives a type several variables (D9).

**Several variables on a TYPE.**  C110's reasons do not reach it, and its bar — *"a use that a
record set genuinely cannot express"* — is not met either: a tuple already is `Pair<K, V>`,
`τ?` already is an option, and a record set already is a map.  It is in the plan on the
owner's first reason below: `instance_def` takes an argument LIST, so the capability arrives
with one-variable structs, and a rule that stops at the second variable is one no reader
could derive from anything.

**What gates what.**

| Needs the revision | Does not |
|---|---|
| C1 — a variable in any parameter (C110 (a)) | arcs A and B |
| C2–C4 — several variables on a function (C110 (b), for functions) | D1–D8, D10 — one-variable generic types |
| D9 — several variables on a type | E1–E5 — the one-variable built-ins |
| E6–E7 — `map`, `reduce` (they need C2) | |

**The owner's reasons for the revision (2026-09-21)** — and they are not C110's bar, which
asked for a consumer.  (1) *No random-feeling restrictions*: "one type variable, in the first
parameter" is a restriction a reader meets with no way to derive it, and the measurement
above says nothing in the compiler needs it any more.  (2) *Before stable*: the change is
allowed later, the work under it is not — § Why before contract 1 says which steps.
(3) *The fallout must not become a long-lived branch* — § How it lands.

**The decision** is [C126](../../DESIGN_DECISIONS.md) — its text lives in the register and
nowhere here, so the two cannot disagree.

## Why before contract 1

*"This change is allowed later"* is true of half the plan.  A step that only ADMITS programs
can land after the freeze.  A step that renames a key, changes the IR, moves which definition
a call reaches, or adds a refusal cannot — and those are the steps under everything else.

| Surface a freeze would pin | Steps that move it | Why it cannot wait |
|---|---|---|
| definition keys — they are native symbols, and the library path decodes them (`native_lib.rs`, `lib_placement/dispatch.rs` call `original_name`) | A1, A5, B1 | a key is how a library's function is FOUND |
| the IR schema and every cached image of it | D2 (a new definition kind), D3 (an instance records its template and arguments) | a published library's cached IR is read by later compilers |
| which definition a call reaches | B4 — selection by rank where a receiver key decided | a program that compiles today may become ambiguous: an error-ADD |
| the set of refused programs | D1 (`T` outside its header), `D-Regular`, `D-Rank`'s ambiguity | C95's rule: the error surface can only shrink after contract 1 |

Pure broadening, safe at any time: A6, B2–B3, B5–B7, all of C, D4–D11 — and arc E, wherever a
step's emission gate holds (a built-in that becomes a library function also renumbers
`index/target_surface.json`, which is regenerated, not frozen).

**The window is open now, and it is measured.**  The published libraries declare **0**
generics and **0** free overload sets, so today A5 renames no library symbol and B1 re-keys
no library definition — the fallout is this repository's corpus and nothing else.  @PLN162's
overloads are a week old.  Every library that adopts them, or a generic, widens what A5 and
B1 must carry.

## How it lands — never a long-lived branch

The fallout is worked on `main`, behind a switch, not on a branch.

1. **A0–A4 cannot have fallout.**  Their gate is byte-identical emission over the corpus, so
   they ride any PR.
2. **The three steps that move a frozen surface land ALONE** — A5, B1, B4: one step, one PR,
   nothing batched with it.  Each carries a parse-time switch that restores the old form and
   an exact gate (A5: identical under the printed rename map; B1 and B4: identical with the
   switch set).  After each, the library lanes run (`revalidate-libs`, `lib-main-health`) —
   `make ci` says nothing about the shipped libraries.
3. **A step whose corpus gate does not hold yet lands OPT-IN, and flips when it does.**  The
   repository's standing pattern — `LOFT_NO_VALUE_RECORD` was opt-in for two days because its
   call-site gate did not hold over the script corpus.  The step's own cells run under the
   switch, so it is exercised, never dormant; `main` stays releasable; and the residue is
   fixed in small PRs against `main` rather than accumulating beside it.
4. **Everything else only admits programs**, so a defect in it cannot reach a program that
   compiled before.

An arc is sized to close inside the one-or-two-PRs-a-day cadence, and a step that does not
fit is cut again rather than carried.

## Out of scope

A generic as a fn-ref value ([e2](probes/e2-generic-as-fn-ref.loft) — it has no single address
before it is instantiated; refused with its reason in step A4); explicit type application in
expressions; default type arguments; a bound on a generic TYPE's variable that adds fields
(a field-bearing bound is a later plan, per `D-Keyed`); variance; specialising a generic type
per argument.
