<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 165 — the steps

The design is [DESIGN.md](DESIGN.md); read its invariant and its six findings first, because
the ORDER below comes from them.  Every step here passes two tests, and says how:

- **Compared against** — while the step is half done, the old path and the new one can both
  run and be compared *exactly*.  A step whose only check is "swap it and look" is too big.
- **Red on its own** — something fails if THIS step is done wrong, before the next one lands.
  A step that builds something nobody calls cannot fail, so it is not a step.

Two instruments carry most of the comparisons, and both already exist:
`scripts/introspect_diff.sh <before> <after>` (IR + bytecode + generated Rust + stderr,
byte-identical over every corpus file) and `scripts/probe-matrix <dir>` (cells that declare
their expected output, a control cell that must fail, a leak check).  "Both backends" below
means `--interpret` and `--native`, with `LOFT_STRICT_STORES=1` and `LOFT_POISON=1` armed.

## The arcs

Five arcs, each a PR that closes on its own.  Step C0 — the owner revising C110 — is done
(C126, 2026-09-21), so nothing below waits on a decision; the "Waits for C0" column records
which steps the revision unblocked.

| Arc | What a program can write afterwards | Steps | Waits for C0 |
|---|---|---|---|
| **A** | nothing new — three wrong refusals on `main` close | A0–A6 | no |
| **B** | a generic beside concrete definitions of its name, and beside other generics | B1–B7 | no |
| **C** | `fn hole<T>(n: integer, v: T)`; `fn map_grid<T, U>(…)` | C0–C4 | **all of it** |
| **D** | `struct Grid<T>`, `enum Shape<T>`, methods on them, the goal program | D1–D11 | **D9 and D11 only** |
| **E** | a program's own `insert` / `sort` / `reverse` / `reserve` / `filter` — then `map` / `reduce` — is one more member of the name's set | E1–E7 | **E6–E7** (they need C2) |

**How it lands** (DESIGN.md § How it lands): never a long-lived branch.  A0–A4 are
byte-identical and ride any PR.  The three steps marked **⚑ lands alone** move a surface a
freeze would pin — one step, one PR, a switch that restores the old form, the library lanes
run after it.  A step whose corpus gate does not hold yet lands opt-in and flips when it
does.  Steps marked **pre-freeze** cannot wait for contract 1; the rest only admit programs.

---

## Arc A — groundwork

**Nothing in A0–A4 changes what any program means**; each is proved by
`introspect_diff.sh` reading IDENTICAL.  A step here that changes emission is a bug in the
step.  A5 and A6 are the first changes a program can see, and each closes a wrong refusal.

### A0 — the twin matrix  ·  XS

`probes/twins/`, a `probe-matrix` directory: one cell per generic shape that works today (one
variable; bounded; a generic calling a generic; `vector<vector<T>>`; a tuple return; a struct
element; a nullable element; a `use`d library's generic; the stdlib's `sum` / `min_of`), each
printing the generic's answer BESIDE its hand-written twin's, with `@EXPECT` lines computed by
hand and one `@CONTROL` cell.

- **Red on its own:** the control cell must fail, or the run does; and this is the matrix
  phase 0's four fixes (loft#1536–#1539) are re-verified against.
- **Compared against:** hand-computed values — never "the two backends agree".
- **Why first:** `G-Mono` is *"an instance equals its twin"*, and every later step's
  acceptance is *"add its cells to this matrix"*.  Run `scripts/matrix_axes.py file` over it
  and record which composition axes it holds FIXED.

### A1 — one decoder for the key  ·  S  ·  pre-freeze

`Data::split_key(name) -> Option<KeyParts { kind, spelling, rest }>`, reading the length as
EVERY leading digit.  `Definition::original_name` (68 callers) and the 11 sites that strip `t_`
and split at the first underscore call it.  The encoder asserts what the decoder relies on: a
spelling never begins with a digit.

- **Red on its own:** a unit test — `original_name` of [a1](probes/a1-key-over-99-chars.loft)'s
  121-character key is `pick`.  Red today.
- **Compared against:** IDENTICAL over the corpus; every corpus key is under 100 characters,
  so nothing may move.
- **Why separate:** B1, A5 and C2 all make keys longer.  The decoder has to be right before
  anything leans on it.

### A2 — every deferred site reads its own type  ·  S

Five of the six `TV_*` blocks already read their concrete type from the block's own `result`.
`TV_DEFAULT_BLOCK` reads the function-wide `concrete` instead; give it a `result` like its
siblings, then drop the eleven `concrete: &Type` parameters that only pass it along.

- **Red on its own:** `tests/scripts/1016-generic-null-default-instantiation.loft` and its
  neighbours pin the default's value per instance.
- **Compared against:** IDENTICAL.
- **Why separate:** with two variables there is no *"the"* concrete type.  Removing the
  question is safer than answering it twice.

### A3 — a template's variables have one home  ·  S

`Parser::template_vars(g_nr)` — the distinct placeholders over ALL declared parameters, in
first-appearance order — and `Parser::bind_template(g_nr, types) -> Option<Vec<(u32, Type)>>`,
which runs `resolve_type_var` for each variable over each (parameter, argument) pair.  The
five `extract_type_var(attributes()[0])` sites call them; `cur_type_var: u32` becomes
`cur_type_vars`, a list holding exactly one.

- **Red on its own:** a release-build check that `template_vars` has length one for every
  template the corpus declares — real, because `debug_assert` is compiled out of this crate.
- **Compared against:** IDENTICAL.
- **Why separate:** the list is the new concept.  Introducing it while it always has one
  element keeps the concept and the behaviour change out of each other's diff — the move
  @PLN162's step 2 made for `candidates()`.

### A4 — each refused shape says so once, where it is written  ·  S

`<K, V>` (three parse errors today), `struct Box<T>` (three), a generic as a fn-ref
(*"Unknown variable"*), `first<integer>(a)` (*"comparison operators do not chain"*).  Each
becomes ONE diagnostic at the construct, stating the rule as it stands — never "not yet",
never a plan tag.  The fn-ref and the explicit argument keep their message for good
(DESIGN.md § Out of scope, `D-Infer`) and name the cure; the other two are replaced when C2
and D2 land.

- **Red on its own:** one `@EXPECT_ERROR` script per shape.
- **Compared against:** `introspect_diff.sh` names exactly the files refused for these shapes
  today (their stderr changed) and no other.

### A5 — an instance has its own key  ·  M  ·  pre-freeze  ·  ⚑ lands alone

`Data::identity_spelling(tp)` is extracted from the namer inside `instantiate_template`
(collections keep their element, integers their range and forced width), and the instance key
becomes `i_<LEN><τ>_<template key>` (`D-Key`).  The `__self_` marker goes: the template's key
already tells a method template from a free one.  The character map that makes a key a Rust
identifier moves into `generation::sanitize`, and the emitter refuses two definitions that
sanitise to one identifier.

- **Red on its own:** [a2](probes/a2-instance-key-meets-a-method-key.loft) answers `3|107` on
  both backends — refused on `main` today.
- **Compared against:** the step prints its rename map (`t_7integer_idf → i_7integer_n_idf`)
  under an environment switch; `introspect_diff.sh` over the corpus with that map applied to
  the BEFORE side reads IDENTICAL.  A rename and nothing else, proved rather than asserted.
- **Switch:** parse-time `LOFT_NO_INSTANCE_KEY=1` mints the `t_…` key again — the rollback,
  and the first bisect step for a generic call that reaches the wrong definition.
- **Watch:** the ladder no longer finds an instance as a "method", so a repeat call reaches
  it through the template and the `existing` lookup.  [x1](probes/x1-monomorph-is-not-a-method.loft)
  must stay refused in all three orders, and the unknown-function suggestions must stop
  offering instances as methods of a type.

### A6 — a lambda argument is typed under the bindings made so far  ·  S

A short lambda takes its parameter type from the parameter it is handed to (`self.expected`).
For a generic callee that type is the template's — `fn(T) -> T` — with the variable still in
it.  Substitute the bindings A3's `bind_template` makes from the arguments ALREADY parsed,
so `my_map(a, |x| { … })` hands the lambda `fn(integer) -> integer`.  A variable no earlier
argument binds stays as it is, and the lambda says it cannot infer — the message a lambda
with no context already gives.

- **Red on its own:** [e3](probes/e3-short-lambda-under-a-generic.loft) answers
  `[10,20,30] [10,20,30] [10,20,30]` — the third is refused on `main` today.
- **Compared against:** IDENTICAL over every file that compiles today; and the generic's
  answer equals the CONCRETE twin's (`twice`), which is the same program with `T` written
  out.
- **Why here:** E5–E7 turn `filter`, `map` and `reduce` into library generics, and every call
  of those passes a short lambda.  This is a one-variable defect and needs nothing from
  arc C.

---

## Arc B — a generic is a member of its name's set

### B1 — the key and the rank read a type's full identity  ·  M  ·  pre-freeze  ·  ⚑ lands alone

`full_spelling` (free-overload `f_` keys) and `dispatch_rank`'s `same` test call
`identity_spelling`.  METHOD keys keep the bare constructor — `t_6vector_len` is found ON
`vector`, and that does not change.

- **Red on its own:** [b1](probes/b1-vector-element-overload.loft) answers `ints|texts`; a
  pair differing only by integer width (`u8` / `u16`) answers apart.
- **Compared against:** parse-time switch `LOFT_NO_ELEMENT_KEY=1` restores the erasing
  spelling.  Switch ON against before: IDENTICAL.  Switch OFF against before: DIFFERENT only
  in files that declare a free overload over a collection or a sized integer, and there only
  in `f_…` names — plus one expected change of WORDING, not of verdict: a `vector<text>`
  handed to a lone `vector<integer>` overload is refused by selection now rather than by the
  argument check after it.
- **Why before B2:** finding 1.  Without it `first<T>(v: vector<T>)` and
  `first(v: vector<integer>)` are one key, and B2 would refuse them for a new reason.

### B2 — a template may be a member  ·  S

`add_fn`'s admission gate (`data.rs:7825`) takes `Function | Generic`.  A template in a set is
keyed by its ALPHA-NORMALISED spelling — each variable written as its position plus its sorted
bounds (`$0:Ordered+Printable`) — so `f<T>(v: vector<T>)` and `f<U>(v: vector<U>)` are the
redefinition they are, and `f<T: A>` beside `f<T: B>` are two members.  One accessor,
`Data::set_members(main)`, answers each member as `Function` or `Template`; every emitter
iterates functions only.  Selection still skips templates in this step.

- **Red on its own:** [b2](probes/b2-generic-beside-concrete-free.loft) cut down to its
  `integer` call answers `999` on both backends, and its emission carries no symbol for the
  template (F5).
- **Compared against:** IDENTICAL over every file that compiles today — a template beside a
  same-named definition is refused today, so no compiling file has one.

### B3 — selection ranks a template, and instantiates what it selects  ·  M

`definition_ranks` for a template: a position naming a variable ranks `GENERIC` where
`bind_template` binds it and the bounds hold — asked of a `satisfies()` that reports nothing,
with `check_satisfaction` becoming `satisfies` plus its diagnostic, one home.  `dominated`
compares positions by a partial order in which `GENERIC` and `CONVERTED` are incomparable
(`D-Rank`).  `Selection::One(template)` takes the route `call` already has for a method
template (`method_template`, loft#1539): predict the return on pass 1, instantiate on pass 2.

- **Red on its own:** b2 answers `999|p`; [b3](probes/b3-bounded-generic-beside-concrete-free.loft)
  answers `special cat 1|any dog 2`; `f(x: float)` beside `f<T>(x: T)` called with an
  `integer` is refused naming both.  Twins added to A0's matrix.
- **Compared against:** IDENTICAL over every file that compiles today; parse-time switch
  `LOFT_NO_GENERIC_MEMBER=1` restores the `Function`-only gate and is the first bisect step
  for a call that reaches the wrong definition where a generic shares the name.

### B4 — a generic beside a same-named METHOD is a set too  ·  S  ·  pre-freeze  ·  ⚑ lands alone

Today the method is found by its receiver key and the generic never gets asked (finding 2).
Both join the bare dispatcher, as a `self` name's overloads always have, so `Disp-Applicable`
reads arity and every parameter.

- **Red on its own:** [b5](probes/b5-method-arity-captures-the-call.loft) answers
  `some square 1|SQUARE 1 x3` — a wrong refusal on `main`.
  [b4](probes/b4-concrete-method-beside-bounded-generic.loft) still answers
  `SQUARE 1|some tri 2`, now by rank.
- **Switch:** parse-time `LOFT_NO_METHOD_IN_SET=1` leaves the pair on the receiver key.  A
  program that compiles today and becomes AMBIGUOUS under rank (a method that needs a
  conversion where the generic binds exactly — `D-Rank`) is an error-add: count those over
  every `.loft` file before the default flips.
- **Compared against:** count first which corpus files declare a generic beside a same-named
  method — they are the ONLY files whose emission may differ, and only in key names.  The
  59 corpus files that use a generic are the watch set.

### B5 — a stronger bound is more specific  ·  S

Bound-set inclusion at `GENERIC` positions (`D-Specific`).

- **Red on its own:** `<T: Ordered + Printable>` beside `<T: Ordered>` beside `<T>`: a type
  with both bounds reaches the first, one with `Ordered` alone the second, one with neither
  the third; `<T: A>` beside `<T: B>` at a type with both is refused naming both.

### B6 — a narrower pattern is more specific  ·  S

Pattern P is more specific than Q when P is a substitution instance of Q and not the reverse.

- **Red on its own:** [b6](probes/b6-two-generics-one-name.loft) answers `vec|any`;
  `g<T>(p: (T, integer))` beside `g<T>(p: (integer, T))` at `(1, 2)` is refused naming both.
  Both instances exist side by side at `T = integer` — F7c, which A5's key makes possible.

### B7 — a template as a dynamic leaf  ·  M

`build_specialisation` reaches its leaves through the route B3 built, so a variant tuple that
selects a template calls that template's instance for the variant's type.  `stub_admissible`
keeps declining the open profile's stub for a set with a template member — a stated limit of
`Disp-World`, written down as one.

- **Red on its own:** a `Disp-Match-Equiv` twin in `tests/oracle/` — the dispatcher against
  the `match` written by hand, over an enum one of whose variants satisfies a bound.  And a
  live-reload cell adding an overload to a set that holds a template.
- **Closes the arc:** every B cell graduates to `tests/scripts/` with its `@falsified-at:`
  receipt; `G-Select` is written into `formal/interfaces.md` with its citations — not before,
  because a rule with no implementation is a deviation.

---

## Arc C — several variables, in any parameter

### C0 — the owner revises C110  ·  DONE 2026-09-21

Recorded as [C126](../../DESIGN_DECISIONS.md) — *a generic's type variables are
unrestricted; a keyed collection stays a record set* — with C110 carrying a revision note at
its top.  Arc C, step D9 and steps E6–E7 are unblocked.  What stays decided: no
`hash<K, V>`, and the `hole_*` family stays per-kind.

### C1 — one variable, any parameter  ·  S

C110 (a).  `D-Every-Var` replaces the first-parameter rule.  `bind_template` gains its consistency check:
a variable that two parameters bind to two types makes a template inapplicable in a set, and
is refused naming both parameters where it stands alone (F9).

- **Red on its own:** [c2](probes/c2-variable-outside-first-parameter.loft) answers
  `1:a|2:7`; [c3](probes/c3-one-variable-bound-twice.loft) stays refused;
  `fn bad<T>(n: integer) -> T` is refused at its declaration (F10).
- **Compared against:** IDENTICAL over every file that compiles today.

### C2 — a header declares a list  ·  S

C110 (b), for functions.  `<K, V>` and `<K: Ordered, V: Printable>` parse into
`cur_type_vars` — the one header parser, which D2 already uses for `struct` at one variable.  Nothing else changes: A3's binding loop and A5's key were
written over a list and have been running with one element since.

- **Red on its own:** [c1](probes/c1-two-type-variables.loft) answers `1:a|b:2`, with two
  instances whose keys differ (F8) — read off `loft introspect`, not assumed.
- **Compared against:** IDENTICAL over every file that compiles today.
- **The alarm:** if this step needs more than the parser, A3 or A5 missed a single-variable
  assumption.  Find it and fix it there, not here.

### C3 — two variables through everything that assumed one  ·  S

`associated_bindings` (it reads the variable as `bindings[0]` — F13), `re_resolve_call` and
`instantiate_nested_generics` per variable, the stale-instance re-derivation (loft#1023) over
a list.

- **Red on its own:** `outer<A, B>(a, b)` calling `inner(b, a)` — the variables cross; bound
  methods called on both variables; @PLN125's associated-type generic at two variables; a
  template declared BELOW its caller.  All with twins.

### C4 — across the boundary  ·  S

Two variables in a `use`d library, as a member of an overload set (arc B),
`cargo test --release --test ir_schema_roundtrip`, `--native-release`.  Closes the arc:
cells graduate, `G-Key` is written into `formal/interfaces.md`.

---

## Arc D — generic structs and enums

One variable throughout, until D9.  A one-variable generic struct is declined nowhere in
`DESIGN_DECISIONS.md`, so D1–D8 and D10 do not wait for C0.

### D1 — a type variable is scoped to its header  ·  S  ·  pre-freeze

`parse_type` resolves a variable's spelling only through `cur_type_vars`.

- **Red on its own:** [d2](probes/d2-type-variable-leaks-its-header.loft) and
  [d3](probes/d3-struct-field-typed-by-a-leaked-variable.loft) are refused where the name is
  written, in the author's words — no `__typevar_T`, no last-line position.
- **Compared against:** first COUNT the `.loft` files anywhere in the repository — corpus,
  docs, probes, fixture libraries — that name a leaked variable.  The expected count is zero;
  any hit is a program this step newly refuses, and is looked at by hand.

### D2 — a struct may declare a variable  ·  M  ·  pre-freeze

A new definition kind for a type template (`D-Template`): never laid out, never emitted.  The
header is parsed by the function header's parser (A3), so `<T>` and `<T: Printable>` mean
what they mean on a function; a list of several is D9's.  The IR codec gains the kind, and
the cache version moves with it.

- **Red on its own:** a program that only DECLARES `struct Box<T> { v: T }` compiles and runs
  `main` — three syntax errors today; `ir_schema_roundtrip` over a file holding a template;
  a template's field typed `T` never reaches layout (F15).
- **Compared against:** IDENTICAL over the corpus.

### D3 — an instance in type position  ·  M  ·  pre-freeze

`Data::instance_def(template, args)`, written beside its sibling `tuple_def` and keeping its
four properties: named from the arguments' identity spellings (`Box<integer>`); idempotent;
deferred while an argument is unresolved (F11); registered for every source.  The instance
records `(template, args)` on its definition — a stored fact, because reading it back out of
the NAME is finding 3 again.  `b: Box<integer> = Box { v: 1 }` takes its instance from the
annotation, the way `v: vector<integer> = []` takes its element type.

- **Red on its own:** construction, field read and write, `vector<Box<integer>>` — value AND
  length AND leak, both backends.
- **Compared against:** the hand-written twin `struct BoxInteger { v: integer }`:
  `loft introspect` of the two programs is identical once the one type name is mapped.  This
  is the strongest gate in the plan — an instance IS its twin, byte for byte.
- **Also:** `LOFT_STRICT_SCHEMA_IDS=1` clean on `--native` (F18); two instances whose
  arguments differ only in deps are ONE instance (F12); an `@EXPECT_ERROR` cell reads
  `Box<integer>` in its message, not `Box_integer_` (F20).

### D4 — a literal infers its argument  ·  S

`Box { v: 1 }`: `resolve_type_var` over each (field type, value type) pair — the function
calls already use.  A variable no field value binds is refused naming the variable and the
cure, which is D3's annotation.

- **Red on its own:** [d1](probes/d1-generic-struct.loft) answers `1|a`; an empty `Box {}`
  with no annotation is refused.

### D5 — a generic function over a generic struct  ·  M

Inside a template, `Box<T>` is an OPEN instance — its argument mentions a variable — and is
never laid out.  `Type::zip_children` pairs two instances of one template argument by
argument; `substitute_all` over an open instance answers `instance_def` of the substituted
arguments.

- **Red on its own:** `fn get<T>(b: Box<T>) -> T`;
  `fn map_box<T>(b: Box<T>, f: fn(T) -> T) -> Box<T>` called with a short lambda (A6); a
  concrete `fn show(b: Box<integer>)` beside `fn show<T: Printable>(b: Box<T>)`, the concrete
  one winning for a `Box<integer>` (arc B).  Twins for each.
- **The alarm:** an open instance reaching layout is loft#1536's class (a zero-width
  `__typevar_T`).  Make it loud: the layout pass refuses a definition whose recorded
  arguments mention a variable.

### D6 — a method on a generic struct  ·  S

`fn at<T>(self: Grid<T>, i: integer) -> T?` is a method TEMPLATE keyed on the template's
name, `t_4Grid_at` — loft#1539's mechanism with `Grid` where `vector` stood.

- **Red on its own:** `g.at(2)` and `at(g, 2)` answer alike and equal the twin's, the
  out-of-range read included.

### D7 — a template that mentions itself  ·  S

Regular recursion works because `instance_def` registers the name before it fills the fields,
as `tuple_def` does.  An irregular mention has no finite set of instances and is refused at
the declaration (`D-Regular`).

- **Red on its own:** `struct Node<T> { v: T, next: Node<T>? }` — push three, read back,
  length, leak; `struct Bad<T> { n: Bad<vector<T>>? }` is refused, and the compiler
  terminates (the cell runs under `LOFT_TIMEOUT`).

### D8 — a generic enum  ·  M

`enum Shape<T> { Dot { at: T }, Line { from: T, to: T } }`: an instance is the enum AND each
variant, minted together.  `match` over an instance; `Disp-Dynamic` over an instance's
variants (B7).

- **Red on its own:** construct, match, a `vector<Shape<integer>>`, a nullable payload; twin;
  an oracle twin for the dispatcher.

### D9 — several variables on a type  ·  S  (waits for C0 and C2)

`struct Pair<K, V> { k: K, v: V }`, `fn swap<K, V>(self: Pair<K, V>) -> Pair<V, K>`.
`instance_def` has taken an argument LIST since D3 and C2 parses the header's list, so this
step is cells and whatever they find.

- **Red on its own:** `Pair { k: 1, v: "a" }` names `Pair<integer,text>`; `swap` answers
  `Pair<text,integer>`, a DIFFERENT instance; both spellings of the method; twins.
- **The alarm:** if this step needs more than cells, D3 or D5 assumed one argument
  somewhere.  Fix it there.
- **Why it is in:** no consumer asks for it — a tuple already covers `Pair` (DESIGN.md §
  C110, evaluated).  It is here because stopping at the second variable would be a
  restriction no reader could derive.

### D10 — across the boundary  ·  S

A library exports a generic struct and a consumer instantiates it at its own type;
`api_surface` golden; `ir_schema_roundtrip`; `--native-release`; the debugger and the LSP
show `Grid<integer>`.

### D11 — the goal program  ·  XS  (waits for arc C)

README.md § Goal as ONE guard in `tests/scripts/`: `struct Grid<T>`, a two-variable
`map_grid<T, U>` called with a short lambda, and a generic `show` beside a concrete one —
every arc in one file.  Closes the plan.

---

## Arc E — the by-name built-ins become library generics

Its first half is on `main` already: a by-name special case in `dispatch_call` is taken only
when no definition the program declares applies.  One built-in per step, each a parallel run:

1. write the `default/*.loft` generic beside the special case, under a switch;
2. compare `loft introspect` of every corpus call site, special case against instance;
3. delete the entry from `dispatch_call` only when they are equal.

| Step | Built-in | Variables | Waits for |
|---|---|---|---|
| E1 | `reverse<T>` | one | — |
| E2 | `reserve<T>` | one | — |
| E3 | `insert<T>` | one | — |
| E4 | `sort<T: Ordered>` | one | — |
| E5 | `filter<T>(v: vector<T>, f: fn(T) -> boolean)` | one | A6 |
| E6 | `map<T, U>(v: vector<T>, f: fn(T) -> U) -> vector<U>` | **two** | A6, C2 |
| E7 | `reduce<T, U>(v: vector<T>, init: U, f: fn(U, T) -> U) -> U` | **two** | A6, C2 |

E6 and E7 are the consumer C110 asked for (DESIGN.md § C110, evaluated): the language's own
two-variable generics, which the no-observable-special-names rule cannot retire without arc C.

- **Red on its own:** the comparison in 2, per built-in.
- **Not verified, and it may not hold — emission.**  A special case lowers to ops in place
  where an instance is a CALL.  If 2 reads different, that is the step's finding —
  `--native`'s one-op wrapper inlining (`LOFT_NO_WRAPPER_INLINE`) may close the gap there; the
  interpreter has no such pass.
- **Not verified, and it is the hard part of E6 — pass 1.**  `U` is bound from the lambda's
  RETURN type, and the H5 guard already records that for `map` this is *"unknowable in pass
  1"*: the built-in answers a result type early and desugars on pass 2 only.
  `predict_template_return` has to do the same for a variable bound from a lambda.  E6's
  first act is to measure what pass 1 knows of a short lambda's return, before anything is
  written.
- Each step also runs `make surface-gen`: a new builtin renumbers
  `index/target_surface.json`.

---

## Owed by every step

- the cells on BOTH backends, strict stores and poison armed; value AND length AND leak;
- `scripts/matrix_axes.py file <guard>` — which axes the guard holds fixed, written down;
- `make falsify GUARD=… REF=…` for each new `tests/scripts/` file (`@falsified-at:`);
- for a step with a switch: the suite once with the switch set;
- `Fixes #N` and a `Contract:` trailer where a step closes an issue.  Three defects on `main`
  are closed here and are not filed yet: **a2** (A5), **e3** (A6) and **b5** (B4).

## What was not verified

- `--native` was run for the probes that compile (a1, b0, b4).  Every refusal above is the
  parser's, measured on `--interpret`; none was re-measured on `--native`.
- The count of call sites that read a set's members (`Type::Routine`, ~50 across 15 files) is
  a grep, not an audit.  B2's accessor is where the real number is found.
- No LIBRARY wants several type variables: the published libraries declare zero generics.
  The consumer is the stdlib's own `map` and `reduce`.
- Arc E's emission equality, and E6's pass-1 return prediction (above).
- Nothing here was built.  Every *"works because"* in arcs C and D is a prediction, and the
  step's own gate is what tests it.
