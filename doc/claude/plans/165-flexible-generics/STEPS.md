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
- **Built** (2026-09-21).  The rename claim measured 1621/1624 — the other three are the
  change the step is for, not a rename: a2's own guard; `1418`, where a later call at `i32`
  had reached the `integer` instance through that method lookup and now reaches its own
  width's instance through the template; and `1032`, whose generator STRUCT name is derived
  from the key.  Two things the watch did not name surfaced: `re_resolve_call` had found the
  instance a template-in-a-template call needed by that same method lookup (a call
  `outer<S>` aims at `inner`'s instance bound to `S`), now aimed through its template by
  `instantiate_nested_generics`; and six emitter/scopes sites recognised a monomorph by the
  `t_` prefix (`is_loft_defined`, the leaf and frameless-chain tests, the T-stub body
  patch, the monomorph lift), now asked of the key's shape (`Definition::is_instance` —
  the runtime's `i_parse_*` helpers share the letter).  `LOFT_NO_INSTANCE_KEY=1` is
  byte-identical to A7 over the corpus.

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

### A7 — a lambda written inside a template is instantiated with it  ·  M  ·  added while building A6

Not in the first cut of the plan: A6's cells put a lambda INSIDE a generic body and found it
was one definition typed by the template's own variable, shared by every instance — its
parameters, its return (a `-> T` lambda even took a record return buffer) and its closure
record were the placeholder's.  Silent-wrong on the interpreter (`x * x` answered
`[null,null,null]` for `[1,4,9]`), a panic or a refused crate on `--native`, in every cell but
the one whose lambda mentions no variable.  The design is `G-Mono` applied once more: a lambda
written inside a template is a template itself (`mark_template_lambda`, sharing its bounds);
the enclosing instance instantiates it with its own bindings (`instantiate_template_lambda`,
named `n___lambda_N_in_<instance>`); its closure record is re-laid for the instance's types
(`instantiate_closure_record`); a capture read is stamped `TV_CAPTURE` and re-lowered against
that record by name; a capturing lambda's record is re-built in the instance's frame
(`emit_lambda_code_in`, reusing the template's `__clos` slot); and `closure_parameter_last`
keeps `__closure` the last parameter after an instance's text return gains its buffer.

- **Red on its own:** `probes/lambdas/` (13 cells, both backends, strict stores and poison) and
  `tests/scripts/a-lambda-inside-a-generic-is-instantiated-with-it.loft`.
- **Compared against:** IDENTICAL over the corpus — no corpus file wrote a lambda inside a
  generic, which is why nothing caught it.

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
- **Built** (2026-09-21).  `Data::identity_spelling` is the one answer, read through
  `key_identity` by the free-overload key, the rank's `same`, the set-admission guard and
  both one-return-type checks.  Two more collisions than the plan named: a FUNCTION type
  keyed as the `i32` its value is stored as (`app(f: fn(…))` beside `app(n: i32)`), and a
  binding at `Pt?` sharing `Pt`'s instance.  Across widths an integer into a range that holds
  it ranks WIDENED (`(C-Int)`), or `u8` into `integer` would have dropped from EXACT to a
  conversion and tied with `float`.  A tuple keeps its synthetic struct's name, the one
  spelling a boxed and an unboxed tuple share.  Corpus with the switch: IDENTICAL; without:
  three files — two `f_` renames and the predicted wording change.

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
- **Built** (2026-09-21): measured IDENTICAL 1626/1626.  The alpha-normalised key needs each
  variable's bound set where `Data` can read it: a placeholder is minted per (spelling, bound
  set), and `Data::type_var_bound_keys` records the set beside it.  No emitter enumerates a
  set's members — every `Type::Routine` reader in `generation/` is type-level — so the
  accessor the plan named was not needed: selection is the one route to a member, and it
  skips a template until B3.

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
- **Built** (2026-09-21).  `template_ranks` in `parser/dispatch.rs`; `GENERIC` is apart from
  every sum of the concrete ranks, and `rank_no_worse` is the partial order.
  `check_satisfaction` is now `satisfaction_messages` (the one judgement) plus its diagnostic;
  `satisfies` asks it without reporting, and `infer_associated` is `associated_bindings`
  without its diagnostic.  A selected template takes the method-template route in `call`.
  Three findings:
  - the red cell's premise does not hold — selection's `can_convert` has NO integer-to-float
    step (a lone call converts; a set never did), so `f(x: float)` beside `f<T>(x: T)` at
    an `integer` reaches the template.  The incomparability is shown with a plain enum
    into an `integer` instead ([sets-refused/r05](probes/sets-refused/));
  - an argument typed by the CALLER's variable (a call inside another generic) does not
    rank a template: a set is not re-selected per instance, so the call stays refused as it
    is today for a set of concrete members — step B3b;
  - a variant decision (`Disp-Dynamic`) whose leaf is a template is refused naming it, never
    answered by the static selection for every variant — B7 builds that leaf.
  Cells: [sets/](probes/sets/) s03–s12 and [twins/t14](probes/twins/), green on both
  backends (twins under `LOFT_STRICT_STORES` + `LOFT_POISON`).  A refusal names a template by
  its header (`show<T: Named>(T)`).

### B3b — a set is re-selected per instance  ·  M  ·  added while building B3

`(G-Mono)`: a call written inside a generic, whose argument is typed by the generic's own
variable, reaches in each instance what the instance's concrete twin reaches.  Today a set
called that way is refused (*"generic type U: method call requires a concrete type"*), for a
set of concrete members as much as for one holding a template, and a bounded generic cannot
even call a lone bounded generic whose variable is spelled differently (`outer<U: Named>`
calling `inner<T: Named>` is *"'U' does not satisfy interface 'Named'"*, on `main`).  The
call site is stamped as a deferred site (the `TV_*` family) and lowered again per instance
through `call`, with the instance's argument types.

- **Red on its own:** [sets-in-generic/g01](probes/sets-in-generic/) answers
  `special cat 1|any dog 2`; the lone bounded pair at two spellings.  Twins beside each.
- **Compared against:** IDENTICAL over every file that compiles today.
- **Built** (2026-09-21).  `Parser::defer_set_call` stamps `TV_SELECT` (with `TV_SELECT_ARG` /
  `TV_SELECT_NAMED` carrying each argument and its static type, and the source the generic
  was written in); `lower_set_call` lowers it in each instance through `call`.  Switch
  `LOFT_NO_SET_RESELECT=1`.  What the cells found:
  - the lone bounded pair was a separate defect, fixed first (`142e35694`): a text-returning
    interface method's bound stub did not mark its hidden `&text` buffer hidden, so `(G-Sat)`
    at a variable read it as one parameter too many;
  - only a TEMPLATE body defers — an instance bound to a variable (`i_1V_n_wrap`) is never
    instantiated from, and native emits it, so its call is made the ordinary way (g04);
  - a re-decided call that returns a record mints its buffer after the parse; the monomorph
    declares it with the preamble's own predicate, `work_ref_takes_preamble`, extracted to one
    home — and only those buffers, or a deferred default's buffer moves (three corpus files
    did, caught by the corpus diff);
  - a set of CONCRETE members at a variable stays refused (`D-Rank` gives nothing to rank at a
    variable but a template): no duck typing.  The refusal names the set now, not a "method
    call";
  - a member reached in an instance whose return differs from what the body was typed with is
    refused naming it ([sets-refused/r06](probes/sets-refused/)).
  Measured: IDENTICAL 1629/1629 against `142e35694`; g01–g10 (g10 through a library) and twin
  t15 green on both backends under `LOFT_STRICT_STORES` + `LOFT_POISON`.

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
- **Built** (2026-09-21).  `Data::add_fn`, both declaration orders: a free template beside
  `self`/`both` methods of its name in one source admits them to the dispatcher
  (`admit_overload_set` — a method keeps its `t_` key, so `x.m(…)` and `m(x, …)` still reach
  one definition) and joins under its `f_` key; a method declared after a free template
  admits the template.  The stdlib never joins.  The count came first: over every `.loft`
  file in the repository, two corpus files declare the pair (`an-instance-is-not-a-method-on-
  its-bound-type`, and `a-method-and-a-function-of-one-name-on-one-type-are-refused`, whose
  pair is on ONE receiver pattern and stays `(F-OneBody)`'s refusal, reported before any join).
  Measured: DIFFERENT 1 of 1633 against `c243bd4e6` — the first file, and there a pure rename
  (`i_3Cat_n_count_of` → `i_3Cat_f_10vector<$0>_count_of`) plus def renumbering, proven with
  the rename map applied and numbers masked; no file became refused (error-adds: 0).  Cells
  [method-sets/](probes/method-sets/) m01–m10 (m10 through a library) and
  [sets/s13](probes/sets/), which covers the argument spellings and element types the first
  cells held fixed, green on both backends under `LOFT_STRICT_STORES` + `LOFT_POISON`.  A
  free generic answers the METHOD spelling too where the receiver has a method of the name
  (`q.describe()` reaches `describe<T>` when only a two-parameter method is on `Sq`), which is
  `(F-OneBody)` read from the other side.

### B5 — a stronger bound is more specific  ·  S

Bound-set inclusion at `GENERIC` positions (`D-Specific`).

- **Red on its own:** `<T: Ordered + Printable>` beside `<T: Ordered>` beside `<T>`: a type
  with both bounds reaches the first, one with `Ordered` alone the second, one with neither
  the third; `<T: A>` beside `<T: B>` at a type with both is refused naming both.
- **Built** (2026-09-21).  `Parser::strictly_better` replaces the closure: where both ranks
  are `GENERIC` at a position, `generic_position_order` answers — bound-set inclusion
  (`position_bounds`, over every variable the parameter mentions), asked only between
  patterns that are the same up to renaming (`pattern_instance` both ways), so B5 makes no
  choice B6 would take back.  The cells use two user interfaces rather than `Ordered` /
  `Printable` (the order is the same; a user type satisfying one and not the other is
  clearer to write).  Cells [specific/](probes/specific/) p01–p05 (p04 re-decided per
  instance) green on both backends under `LOFT_STRICT_STORES` + `LOFT_POISON`;
  [sets-refused/r08](probes/sets-refused/) refused naming both.  Corpus: only the new guard
  differs from B4 (refused there).

### B6 — a narrower pattern is more specific  ·  S

Pattern P is more specific than Q when P is a substitution instance of Q and not the reverse.

- **Red on its own:** [b6](probes/b6-two-generics-one-name.loft) answers `vec|any`;
  `g<T>(p: (T, integer))` beside `g<T>(p: (integer, T))` at `(1, 2)` is refused naming both.
  Both instances exist side by side at `T = integer` — F7c, which A5's key makes possible.
- **Built** (2026-09-21).  `generic_position_order` reads two orders that must agree: the
  pattern (`pattern_instance` both ways — one-way matching, the specific side's variables
  opaque atoms, a variable met twice meeting one type) and the bound set (B5).  One saying
  fewer and the other more, or either saying neither, leaves the pair unranked
  ([sets-refused/r10](probes/sets-refused/): `vector<T>` against `<U: Named>` at a Named
  `vector<C>`).  Cells [patterns/](probes/patterns/) q01–q05 (q05 re-decided per instance;
  a two-variable tuple pattern waits for C2 in [arc-c/](probes/arc-c/)) green on both
  backends under `LOFT_STRICT_STORES` + `LOFT_POISON`; b6 answers `vec|any`.  Corpus: only
  the new guard differs from B5 (refused there).

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
- **Built** (2026-09-21).  `build_specialisation` instantiates each template leaf at its variant
  tuple's types (the null leaf at the call's static types) before reading the dispatcher's
  return, the way a static call instantiates; a leaf that cannot instantiate refuses the call.
  Three defects found on the way, each fixed where it lives:
  - a variant's own methods did not satisfy a bound at that variant (`190c4d0d5`, on main);
  - the dispatcher forwarded its leaves' hidden buffers under the FIRST leaf's names, and an
    instance's `___tret_1` (a `unique`-minted name — so is `___acc_1`) was the very name the
    dispatcher's own text-return promotion then minted: one variable, every result assigned
    into the buffer it had just filled (SIGSEGV / E0308).  The buffers now go by position
    under the dispatcher's own `__fwd_{k}` names; making `unique` skip taken names was tried
    and is wrong — pass 2 reuses pass 1's variables by name on purpose;
  - an instance dropped its template's HIDDEN flags (`Argument` has none), so its `__retbuf`
    read as a declared parameter: native kept it where the twin's value record drops it — a
    `G-Mono` divergence of its own — and the dispatcher minted an undeclared buffer for it.
  `Disp-World`'s stated limit is written into [@PLN162 RULES.md](../162-multiple-dispatch/RULES.md):
  an add to a set that holds a generic is refused whole (`live_reload::add_fn_block`), since
  its static sites have no stub to rebuild.  Cells [dynamic/](probes/dynamic/) d01–d05 green
  on both backends under `LOFT_STRICT_STORES` + `LOFT_POISON`; the `Disp-Match-Equiv` pair
  `tests/oracle/36-dispatch-set-with-a-generic*` agrees across the interpreter, native and
  wasm; `tests/live_world.rs` gains the refused add.  Corpus against `190c4d0d5`: DIFFERENT
  10 of 1639 — every difference read: dispatchers' forwarded-buffer rename; instances now
  carrying their twins' hidden parameters (native signature without `__retbuf`, its buffer
  witness, one more fn-ref arm whose type now matches).  `G-Select` is written into
  `formal/interfaces.md` and cited at its four sites.

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
- **Built** (2026-09-21).  The first-parameter rule is `D-Every-Var` in `parse_function`, and a
  template refused by it answers its declared return at a call (loft#1538's shape, as the
  several-variables refusal does).  Three places assumed the FIRST parameter: the pass-2
  "cannot infer" precheck in `instantiate_template` and its twin in `predict_template_return`
  (now asked only when the first parameter carries a variable — `first_param_binds`), and
  `instantiate_nested_generics`, which bound a nested generic from its call's first argument
  alone and now takes the rest from the callee's parameters under the instance's bindings (it
  takes the whole binding list, and the stale-monomorph record drops its `concrete`).  F9 is
  `binding_clash`, asked with the argument check's own predicate (`convert_admitting`) — a
  first cut with `can_convert` refused the stdlib's `sum(v, 0)`, since selection's
  applicability is narrower than the argument check.  Found on the way: a call refused by
  selection answered `unknown`, and the call it was an argument of then reported it again as
  a MISSING argument; it answers `never` now (@P376's poison), for the ambiguity and the
  variant-decision refusals too.  Cells [any-param/](probes/any-param/) v01–v05 green on both
  backends under `LOFT_STRICT_STORES` + `LOFT_POISON`; r11, r12 refused with one message
  each.  Corpus: IDENTICAL 1640/1640 against B7.  `parse_errors`' pinned first-parameter test
  is rewritten for the new rule, and LOFT.md / `tests/docs/25-generics.loft` say it.

### C2 — a header declares a list  ·  S

C110 (b), for functions.  `<K, V>` and `<K: Ordered, V: Printable>` parse into
`cur_type_vars` — the one header parser, which D2 already uses for `struct` at one variable.  Nothing else changes: A3's binding loop and A5's key were
written over a list and have been running with one element since.

- **Red on its own:** [c1](probes/c1-two-type-variables.loft) answers `1:a|b:2`, with two
  instances whose keys differ (F8) — read off `loft introspect`, not assumed.
- **Compared against:** IDENTICAL over every file that compiles today.
- **The alarm:** if this step needs more than the parser, A3 or A5 missed a single-variable
  assumption.  Find it and fix it there, not here.
- **Built** (2026-09-21).  The alarm rang twice, both in the parser, each fixed where it
  lives: (1) a template bakes an element read of `vector<V>` with stride `0` (no width yet),
  and instantiation rewrote EVERY stride-0 read with the binding it was substituting — with
  two variables `K`'s pass took `V`'s reads and the program read `text` at `integer`'s width
  (a store-bounds panic).  The read now bakes a marker naming its variable
  (`Parser::type_var_stride`, negative, which no width is), rewritten only by that variable's
  pass; a legacy `0` stays the one variable's.  (2) Satisfaction read the TEMPLATE's bound
  list, which is its first variable's: `V: Named` at a `text` passed unchecked, the stub
  resolved to nothing and the call answered `cat 1` with no diagnostic — a silent wrong
  answer on this branch, never on `main` (a second variable could not be declared there).
  Each variable's bounds are now on its placeholder (`has_bound_for_method` already read
  them there) and `satisfaction_messages` judges every binding against its own
  (`var_bounds`), for the instantiation and for selection's `satisfies` alike.  F8 read off
  `loft introspect`: `i_12integer#text_n_pair_up` and `i_12text#integer_n_pair_up`.  Switch
  `LOFT_NO_SEVERAL_VARS=1`.  Cells [two-vars/](probes/two-vars/) w01–w06 (w06: a
  two-variable tuple pattern in a set) green on both backends under `LOFT_STRICT_STORES` +
  `LOFT_POISON`, and every earlier matrix unchanged; A4's refusal guard is replaced by the
  positive one.  Corpus against `7e1e85c0a`: only the two new guards differ (refused there).

### C3 — two variables through everything that assumed one  ·  S

`associated_bindings` (it reads the variable as `bindings[0]` — F13), `re_resolve_call` and
`instantiate_nested_generics` per variable, the stale-instance re-derivation (loft#1023) over
a list.

- **Red on its own:** `outer<A, B>(a, b)` calling `inner(b, a)` — the variables cross; bound
  methods called on both variables; @PLN125's associated-type generic at two variables; a
  template declared BELOW its caller.  All with twins.
- **Built** (2026-09-21).  Two silent wrong answers, both on this branch only: (1)
  `associated_bindings` read the template's bounds (the first variable's) against the first
  binding (F13), so `K.Item` of `<S: Source, K: Sink>` was never bound and a call through its
  bound read garbage — `infer_associated` now walks every variable, its own bounds against
  its own binding, and the companion-bound message names that variable's implementor; (2)
  `instantiate_nested_generics` fell back to the FIRST binding for a nested call whose first
  argument is not a plain variable (`inner(bs[i], a)` inside `outer<A, B>` bound `inner` at
  `A` and read a text as an integer) — such an argument takes the callee's parameter under the
  instance's bindings.  `re_resolve_call` needed nothing: substitution runs per binding, and
  a bound stub's receiver is its first parameter.  The stale re-derivation already carried
  the list (C1).  Cells [two-vars-through/](probes/two-vars-through/) y01–y05 with twins,
  green on both backends under `LOFT_STRICT_STORES` + `LOFT_POISON`;
  [sets-refused/r14](probes/sets-refused/) names the right implementor.  Corpus: IDENTICAL
  1644/1644 against C2.

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
