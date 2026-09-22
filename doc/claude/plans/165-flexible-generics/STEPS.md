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

- **Built** (2026-09-21).  The library cell ([boundary/](probes/boundary/), and
  `tests/lib/genericlib.loft` behind `a-library-carries-generics-across-the-boundary`) answers
  on the interpreter, native and `--native-release`; `ir_schema_roundtrip` 8/8.  The GitHub
  gate on C2 found what the local runs could not: the test HARNESS starts each program from
  a prepared stdlib, with a parser that has no record of the stdlib's type-variable
  placeholders, and `bind_header_var` then reused a placeholder by its spelling alone — a
  `<T>` took a stdlib `<T: Ordered>`'s `T`.  Harmless while bounds lived on the template;
  once C2 put them on the placeholder, an unbounded `a == b` compiled and a `boolean` was
  refused as not `Ordered` (`parse_errors`, `issues`, `tests/docs/25-generics` in CI).  Where
  the parser has no record, the placeholder's own recorded bounds now decide reuse
  (`placeholder_bounds_key` — the `bounds` field travels with the Data and through the IR
  codec).  Corpus: IDENTICAL 1646/1646 against C3 (the CLI parse keeps its record).
  `G-Key` is written into `formal/interfaces.md` and cited at the key's mint and decoder.

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
- **Built** (2026-09-21).  The check sits where a type name has resolved
  (`parse_type_inner`): a definition in `Data::type_var_bound_keys` — every header variable is
  recorded there, which `Self`, an interface's associated types and an empty user
  `struct Marker {}` (a placeholder's shape) are not — that is not one of the current
  header's variables is refused where it is written, on either pass (a struct's fields are
  laid out at the end of pass 1).  `parse_struct` / `parse_enum` / `parse_typedef` /
  `parse_interface` clear the previous function's header, which a struct declared after a
  generic function otherwise still saw.  A4's refused `struct Box<T>` header lets its own
  fields name `T` (`refused_header_vars`), so the one refusal stands alone.  Count, with
  `loft --check` over all 7 101 `.loft` files in the repository: four hits — the plan's d1,
  d2, d3 probes and A4's guard (the cascade just named) — none unexpected.  Corpus: only
  the two new guards differ from C4.

### D2 — a struct may declare a variable  ·  M  ·  pre-freeze

A new definition kind for a type template (`D-Template`): never laid out, never emitted.  The
header is parsed by the function header's parser (A3), so `<T>` and `<T: Printable>` mean
what they mean on a function; a list of several is D9's.  The IR codec gains the kind, and
the cache version moves with it.

- **Red on its own:** a program that only DECLARES `struct Box<T> { v: T }` compiles and runs
  `main` — three syntax errors today; `ir_schema_roundtrip` over a file holding a template;
  a template's field typed `T` never reaches layout (F15).
- **Compared against:** IDENTICAL over the corpus.
- **Built** (2026-09-21).  `DefType::TypeTemplate` (codec code 11, `CACHE_FORMAT_VERSION` 8 → 9);
  the six exhaustive `DefType` sites map it (IR store/read, the schema's names both ways, the
  LSP's symbol kind and the API surface read it as a struct).  `parse_struct` binds a header
  (`bind_type_header`: each variable to its placeholder, each bound set to its placeholder and
  stubs) and marks the definition a template; a list of several parses already (C2's header),
  its cells are D9's.  A bare template name in type position is refused naming the cure
  (`Box<integer>`), its type poisoned so nothing reports it twice.  The enum header stays
  refused until D8 (A4's guard narrowed to it).  Switch `LOFT_NO_GENERIC_TYPES=1`.  A program
  declaring `struct Box<T>` and `struct Pair<K: Printable, V>` runs on both backends and
  emits no template; `ir_schema_roundtrip` 8/8 over a script holding them.  Corpus: only the
  two new guards differ from D1.

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
- **Built** (2026-09-21).  `Data::instance_def` beside `tuple_def`; `Definition` gains
  `type_params` (a template's variables, header order), `instance_of` and `instance_args` —
  the IR schema source (`tools/ir_schema/ir.loft`), the regenerated schema, the pinned
  positions (stride 167 → 183), both codecs, and `CACHE_FORMAT_VERSION` 9 → 10.  Type
  substitution moved onto `Type` (`substitute`, `substitute_all`) so `Data` can instantiate
  without the parser; the parser's two helpers delegate.  `Box<…>` parses before the
  collection sub-type dispatch; a collection's element takes a template (and its `?` after
  the arguments); a wrong argument count is refused saying how many the template takes.  A
  literal takes its instance from the expected type — `seeds_instance_hint`, added to the
  leaving-value hint and the four argument admission lists — and without one is refused for
  now (D4 infers).  The strongest gate: `loft introspect` of a program over `Box<integer>`
  against its twin over `struct BoxInteger { v: integer }` — IDENTICAL once the one name is
  mapped and numbers masked (the template's `__typevar_T` store registration aside, which
  any generic program carries).  F18 clean on native; F12 one `Box<text>` for three
  spellings with different deps; F20 reads `Box<integer>`.  Cells
  [generic-types/](probes/generic-types/) t01–t06 green on both backends under
  `LOFT_STRICT_STORES` + `LOFT_POISON`.  Corpus against D2: one file differs, the
  `field_value` message the respelling fix changed (the snapshot predates it).

### D4 — a literal infers its argument  ·  S

`Box { v: 1 }`: `resolve_type_var` over each (field type, value type) pair — the function
calls already use.  A variable no field value binds is refused naming the variable and the
cure, which is D3's annotation.

- **Red on its own:** [d1](probes/d1-generic-struct.loft) answers `1|a`; an empty `Box {}`
  with no annotation is refused.
- **Built** (2026-09-22).  `Parser::literal_instance`: with no expected instance the field
  values are read once (`expression()`, nothing expected) to learn their types and the lexer
  is put back — the abandon path `parse_object`'s hint retry already takes — then each header
  variable binds through `resolve_type_var` over (declared field type, value type), the first
  field that relates deciding, and the literal is built against the instance exactly as an
  annotated one.  The first read's diagnostics are REWOUND (`Diagnostics::mark`/`rewind`,
  new), so a value's warning is said once.  Refused on pass 2 at the literal's name: a
  variable no value binds (`null`, a void call and a poisoned value bind nothing) — *"`Box
  { … }` cannot tell what T is — no field value names it; give the binding its type"* — and
  one variable given two types by two fields, named both (C1's predicate,
  `convert_admitting`); a value that itself reported adds nothing (@P376).  An instance whose
  argument is only typed on pass 2 (a function or struct declared BELOW the use) is minted
  after the file's layout ran: H5 admits it as the seventh legal pass-2 append (a
  `__tuple`'s shape), and the parser's `instance_def` wrapper registers and lays it out on
  the spot as the closure record's registration does — this also closes the same hole in
  D3's type position (`x: Box<Q>` above `struct Q` answered *"field has no storage"*).
  `instance_def` refuses a `never` argument.
  Found on the way and fixed (pre-existing on main, `hit-by:loft`): a struct field's value
  and a vector literal's element are parsed through `parse_operators`, which never met
  `expression()`'s check that a name READ resolves — a bare unknown name there read
  *"Cannot assign unknown(0)"*, *"`main_vector<unknown>` never resolved"*, or NOTHING for a
  collection field, whose in-place append then reached codegen (*"Incorrect var
  undefined[65535]"*, an internal compiler error).  `known_var_or_type` now answers whether
  it reported, both sites check and poison, `expression()` answers `never` for a name it
  reported, and four cascades behind it are closed: a call on a poisoned argument
  (*"Unknown function len"*), `+=` of a poisoned source, a vector field assigned a reported
  name (said twice), a format width.  Exact-list tests in `tests/parse_errors.rs`.
  Cells [inferred/](probes/inferred/) i01–i12 green on both backends under
  `LOFT_STRICT_STORES` + `LOFT_POISON`, refusals [inferred-refused/](probes/inferred-refused/)
  x01–x06 read by hand.  Corpus against D3: the new guard alone differs.

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
- **Built** (2026-09-22).  An open instance is a def `instance_def` now mints for arguments
  that mention a variable (`Data::is_open_instance`).  It gets a ROW and no layout —
  `fill_database` registers it fieldless under the placeholder's internal prefix, and the
  native `init()` replays it fieldless (a replayed field minted `vector<__typevar_T>` and
  shifted every later id; `LOFT_STRICT_SCHEMA_IDS=1` caught it).  Binding: `resolve_type_var`
  takes `&Data` and pairs an open instance's recorded arguments with a concrete one's;
  `template_vars`, the every-variable check and the type-variable questions read through
  open instances (`type_mentions`, `placeholders_in`).  Substitution: a monomorph's bindings
  gain `(open ↦ concrete)` pairs (`open_instance_bindings`) — the plan's *"`substitute_all`
  over an open instance answers `instance_def` of the substituted arguments"*, reached by the
  substitution the variable already takes — so signatures, locals, body types, predicted
  returns and selection ranks see the concrete instance; `retarget_parametric_type_rows`
  maps the open row to the concrete one, and a monomorph still naming an open row after it
  is an internal error (the alarm).  Deferred sites for what the layout decides:
  `TV_FIELD` (read, at `get_field`), `TV_FIELD_SET` (write, at `towards_set`), `TV_OBJECT`
  (a literal), each lowered per instance by the ordinary helper; an element stride over a
  `vector<Box<T>>` names the open instance.  A `-> Box<T>` template declares the twin's
  `__retbuf` (the shape is a record whatever `T` is, so `return_shape_depends_on_type_var`
  stays structural), and the monomorph replays the twin's tail delivery
  (`promote_monomorph_record_return`: `BuildIntoBuffer`, and literal mid-body exits).  A
  literal passed to a generic infers its own instance rather than the parameter's open one;
  messages show an instance as written (`Box<T>`, not the `Box<T#5>` key).
  Twins: `get`, `put`, `wrap` (literal return) and `bump` (an offset after a `T` field) are
  byte-identical to their hand-written twins over `BoxInteger` / `PairText`.  Cells
  [open-instances/](probes/open-instances/) e01–e10 green on both backends under
  `LOFT_STRICT_STORES` + `LOFT_POISON`, and on native under `LOFT_STRICT_SCHEMA_IDS`.
  Corpus against D4: IDENTICAL but for the new guard.
  Found on the way, and fixed after it (a D2 gap): a field `assert(…)` on a generic struct
  was refused *"Unknown variable 'n'"* where the twin's check holds, and a `$`-reading
  default with it.  In a template's own declaration a field name now reads through a
  deferred `TV_FIELD` naming the template (its offsets depend on the arguments), and every
  place the code is replayed — a literal, a field write, `object_init`'s default, a
  literal a generic builds — binds it to the instance (`bind_instance_code`), reading the
  TEMPLATE's check (`field_check`: the copy an instance took on pass 1 predates the
  template's pass-2 parse, which is when a check is stored).  A literal a generic builds
  reports its failed check at the literal, as the twin does.  Cells
  [field-checks/](probes/field-checks/) k01–k06 green on both backends.

### D6 — a method on a generic struct  ·  S

`fn at<T>(self: Grid<T>, i: integer) -> T?` is a method TEMPLATE keyed on the template's
name, `t_4Grid_at` — loft#1539's mechanism with `Grid` where `vector` stood.

- **Red on its own:** `g.at(2)` and `at(g, 2)` answer alike and equal the twin's, the
  out-of-range read included.
- **Built** (2026-09-22).  Three homes answered "which receiver is this?" and each named the
  instance: the KEY (`key_type_name` — now the template's name for any instance, as every
  `vector<τ>` keys on `vector`), MEMBERSHIP (`add_fn` put the method's `Routine` attribute on
  the open instance; now on the template, `Data::method_family`, and a field lookup on an
  instance falls back to its template's method members — an instance minted before the
  method is declared has no copy), and the loft#850 foreign-receiver check
  (`method_receives`, which rejected the concrete instance against the open one; now one
  family).  A concrete method on one instance beside the template shares the key and forms
  one overload set; its pass-2 lookup tried the full-spelling key only for two or more
  parameters ("with one parameter the two keys are one"), which is false once a key names a
  family — so the template re-parsed as the concrete member.  Asked whenever the two differ,
  which also fixed one-parameter `vector` method overloads (`fn head(self: vector<integer>)`
  beside `fn head<T>(self: vector<T>)`), broken the same way on this branch and refused on
  main.  Twins: `at` and `widen` are byte-identical to `GridInteger`'s.  Cells
  [struct-methods/](probes/struct-methods/) m01–m09 green on both backends under
  `LOFT_STRICT_STORES` + `LOFT_POISON`; corpus against D5 IDENTICAL.

### D7 — a template that mentions itself  ·  S

Regular recursion works because `instance_def` registers the name before it fills the fields,
as `tuple_def` does.  An irregular mention has no finite set of instances and is refused at
the declaration (`D-Regular`).

- **Red on its own:** `struct Node<T> { v: T, next: Node<T>? }` — push three, read back,
  length, leak; `struct Bad<T> { n: Bad<vector<T>>? }` is refused, and the compiler
  terminates (the cell runs under `LOFT_TIMEOUT`).
- **Built** (2026-09-22).  The plan's example was measured against its twin first: an INLINE
  `next: NodeInt?` is refused (*"contains itself — use reference<NodeInt>"*); the list is
  `reference<Node<T>>?` and the tree `vector<Tree<T>>`.  So the red cells became those two
  shapes plus the refusals.  An instance's field types are its template's with the bindings
  applied and each open instance the field WRITES closed (`Data::close_open`; closing ones a
  binding brought in recursed without end on `Box<Box<T>>`); regular recursion ends because
  the name is registered before the fields.  An instance the template's own declaration
  mints copies only the fields parsed so far, so the declaration's end refreshes them
  (`refresh_instances`) — FIELDS only, methods stay on the template (copying one on pass 2
  broke H5).  `D-Regular` refuses an irregular self-mention on the pass that parses it
  first and marks the template so no instance is minted; a nesting bound in `instance_def`
  keeps compilation finite regardless.  An inline self field is each instance's own cycle,
  reported in the instance's words (`Node<integer>`) as the twin's is — the cycle check
  skips open instances and also runs for an instance first laid out on pass 2.
  `Type::substitute` keeps a `reference<…>` pointer marker, and a template's `reference<…>`
  field is a pointer field.  A mutual pair needs a FORWARD reference to a generic struct,
  which was a syntax error: pass 1 now reads and sets aside the arguments of a name still a
  stub; the stub adoption and `resolve_adopted_stubs` no longer point it at the bare
  template (`names_unresolved` counts a bare template as unresolved); a template's or a
  concrete struct's field and a parameter are retyped on pass 2 (a concrete struct laid out
  as soon as its declaration completes, `lay_out_late`); a RETURN is resolved between the
  passes (`resolve_forward_generic_returns`) so its `__retbuf` is reserved in time; a
  literal lays out the expected instance it takes.  Cells [self-reference/](probes/self-reference/)
  s01–s07 green on both backends under `LOFT_STRICT_STORES` + `LOFT_POISON`; refusals
  x01–x04 read by hand; four guards falsified at f5db133f1.  Corpus against D6: one file
  differs, the D6 guard, whose `init()` no longer replays method names as fields.

### D8 — a generic enum  ·  M

`enum Shape<T> { Dot { at: T }, Line { from: T, to: T } }`: an instance is the enum AND each
variant, minted together.  `match` over an instance; `Disp-Dynamic` over an instance's
variants (B7).

- **Red on its own:** construct, match, a `vector<Shape<integer>>`, a nullable payload; twin;
  an oracle twin for the dispatcher.
- **Built** (2026-09-22).  `parse_enum` reads a header as `parse_struct` does and makes the
  enum a `TypeTemplate`; its variants are template parts (`Data::is_template_part`), never laid
  out, wrapped or cycle-checked.  `instance_def` on an enum template mints the enum and each
  variant together (`fill_enum_instance`): the variant list with its discriminants, and one
  variant definition per template variant with its payload closed to the instance — the
  variants keep their bare names first-wins, as every `__nullable<S>`'s `Null` and `Some` do,
  and are reached through their enum.  In type position an enum instance is `Enum(inst, …)`,
  and `Shape<integer>::Dot` names an instance's variant (a bare `Dot` is the template's).  A
  variant literal takes its instance from an expected `Enum(instance)` or infers it from its
  payload (D4's reader, generalised to fields of one def and variables of another), then
  builds the instance's own variant; a unit variant resolves through its expected instance
  as any variant in context does.  `match`, vectors and a nullable payload needed nothing
  further.  Found on the way and fixed (on main): a method on an enum answered `v.m()` for a
  variant-typed `v` but not `m(v)` — `Data::candidates` now falls back from a variant that
  declares no `m` (in either nullability spelling — asked of the dense one alone, an enum's
  generated dispatcher called itself, which the corpus diff caught on `1427b`) to its enum.
  Twins: `show`/`mk`/`main` over `Shape<integer>` are identical to `ShapeI`'s with type ids
  masked.  The `Disp-Match-Equiv` pair `tests/oracle/37-dispatch-over-a-generic-enums-variants*`
  agrees across the interpreter, native and wasm.  Cells [generic-enums/](probes/generic-enums/)
  g01–g06 green on both backends under `LOFT_STRICT_STORES` + `LOFT_POISON`.  D2's guard that
  refused an enum's variables is replaced by `a-generic-enum-is-an-enum-per-instance`.  Corpus
  against D7: only the two new guards differ.

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
- **Built** (2026-09-22).  The cells found no one-argument assumption in D3–D6.  They found a
  literal RECEIVER taking its statement's expected type: in `s = Pair { k: 1, v: "a" }.swap()`
  the expected type is `swap`'s result (`Pair<text, integer>`, from pass 1) and the literal
  was built as that — loft#1304's postfix class, for the instance a literal chooses.  A
  literal followed by `.` or `[` now infers from its payload (`literal_is_a_receiver`, one
  balanced look-ahead the literal's own parse then re-walks).  `swap` is its twin over
  `PairIT`/`PairTI` with numbers masked.  Cells [several-on-a-type/](probes/several-on-a-type/)
  p01–p05 green on both backends.  Corpus against D8: the new guard, and D6's guard, whose
  receiver literal `Grid { cells: ["a"], w: 1 }.with_w(20)` now infers — through D4's
  discovery read, which leaves a dead temporary behind for a collection value (declared null,
  freed, never read).  That is a difference in FORM from the twin, not in value; its cure is
  a variable-table rollback for a discarded read, which touches every side map keyed by a
  variable number — built in D10, where the temporary turned out to break programs too.

### D10 — across the boundary  ·  S

A library exports a generic struct and a consumer instantiates it at its own type;
`api_surface` golden; `ir_schema_roundtrip`; `--native-release`; the debugger and the LSP
show `Grid<integer>`.

- **Found on the way** (2026-09-22).  The first consumer cell failed: `t: Grid<text> =
  grid(["a", "b", "c"], 3)` after `m = Grid { cells: [Mine { n: 5 }], w: 1 }` was refused,
  `_vec_2` changing type from `vector<Mine>` to `vector<text>`.  No library was needed — D4's
  first read (the values read to learn the instance) is not repeated on pass 2 where a bound
  variable's pass-1 type is the expected instance, so whatever it NUMBERED renumbered what
  followed: its `_vec_N` temporary took the name the next vector wanted, and a lambda value's
  definition shifted every later lambda (`x` met `s: text`).  The matrix
  ([inferred/](probes/inferred/) i13–i21, [x07](probes/inferred-refused/)) found two more:
  a format string in a generic literal failed outright (`Expect token }`, inferred since D4
  and annotated since D9), and D9's receiver look-ahead was a walk over tokens.  One cause
  under both: `Lexer::revert` replayed tokens but not the state around them — the cursor a
  caret is read from, the end of the consumed source, the mode a string literal leaves, and
  the rest of a string a hole's `}` resumes — and a token walk records what no parse reads.
  So the replay now carries all four (`Recorded`), the first read puts the variable table
  back whole and defines no lambda (a long one answers its header's type; a `|…|` one takes
  its types from the field and binds nothing — refused alone, naming both cures), and the
  receiver question is the one token after that read's closing brace.  Every generic literal
  is read that way now, so the answer is one per program, not one per pass.  A caret raised
  during a replay moved to the parse's own position: three corpus pins moved (the INC#30
  typo, two slice-pattern refusals), each onto the construct it names or the token the parse
  is stuck on; the unknown-name arm of `parse_var` reports at the name, as its siblings do.
  Corpus against D9: the new guard, dead temporaries and two orphan lambdas gone from the
  D4–D7 guards, and the three moved carets.
- **Built** (2026-09-22).  [boundary-types/](probes/boundary-types/) b01–b08 over
  `tests/lib/gridlib.loft`, green on both backends and under `--native-release`: the
  consumer's own instances, the library's enum, field check, default, self-naming struct and
  two-variable struct, a qualified `gridlib::Grid<integer>`, and the consumer's generics
  (a function, a struct) over the library's.  The widened cells found five defects, none of
  them the boundary's:
  - a generic naming one variable in two parameters through a generic struct
    (`widths<T>(a: Grid<T>, b: Grid<T>)`) was refused, `T` "to text and to text": the clash
    check compared the second argument with the OPEN `Grid<T>`.  It now closes it
    (`Parser::close_open`, which lays out what it mints on pass 2) — probes o20, x01.
  - D8 never put a generic enum in template code, and every shape failed: a layout alarm on
    the open instance's variants, `Enum` forms missed by the pairing, the substitution, the
    variable table, the stride and the nested-call instantiation (the last ran a template
    with no code and ended the program with no output and exit 0), and a variant literal
    or unit variant that built a record at the open row.  An open instance's variant is now
    open too, mapped per monomorph to its instance's variant (`bound_instance`) — probes
    [generic-enums/](probes/generic-enums/) g07–g12.  `generate_call` refuses a call to a
    template (`@FR-G-Mono`), which turns that silent exit into an internal compiler error:
    falsified by removing the variable table's `Enum` arm.
  - a generic `match` answering `T` fell back to a REFERENCE null: native refused an
    `integer` instance (E0308).  On main too, for any enum subject.  The fallback is now asked
    of the concrete type per monomorph (`TV_NULL_VALUE`), a type with no null value of its own
    keeping the reference sentinel — probes [type-var-nulls/](probes/type-var-nulls/).
  - `Pair<V, K>` inside `Pair<K, V>` had fields `k: K, v: K`: an instance's bindings were
    applied one after another.  `close_open` substitutes them at once
    (`Type::substitute_simultaneous`); a template reading its swapped instance was refused
    ("expected V, got K") — probe p06.
  - `api-surface` listed the variable placeholders, every instance and the `main_vector<τ>`
    wrapper of a vector parameter (the last on main too, for any library).  It lists the
    template now (`Grid<T>`, `Slot<T>` as an enum), no method as a field, and spells a
    signature's generic types from source — golden tests in `tests/api_surface.rs`.  The
    LSP outline shares `classify` and drops them too; hover already named the template.
  - the debugger showed `Box<integer(0, 255)s1>` and could not evaluate an expression over a
    generic local: its seed `g = Grid<integer>{…}` does not parse (DESIGN keeps type
    arguments out of expressions).  A literal now names the template (`Stores::shown`, a
    display alias filled when bytecode is generated) and the pause line and every seed carry
    the source type (`g: Grid<integer> = Grid{…}`); a stdlib alias reads as itself in source
    spelling (`u8`, not the unparseable `integer(0, 255)`); an instance's frame is named by
    its function (`peek`).
  Guards: `a-library-carries-generic-types-across-the-boundary` (`--lib tests/lib`),
  `a-generic-function-over-a-generic-enum-equals-its-twin`,
  `a-generic-match-falls-back-to-its-variables-null`,
  `two-parameters-at-one-variable-through-an-instance`.  `ir_schema_roundtrip` green.
  Corpus against the first half: IDENTICAL — every change above is to programs the corpus
  did not have.

### D11 — the goal program  ·  XS  (waits for arc C)

README.md § Goal as ONE guard in `tests/scripts/`: `struct Grid<T>`, a two-variable
`map_grid<T, U>` called with a short lambda, and a generic `show` beside a concrete one —
every arc in one file.  Closes the plan.

- **Built** (2026-09-22).  [goal/](probes/goal/) g01–g12, green on both backends; the guard is
  `the-goal-program-of-flexible-generics`.  The goal program was refused — "expected U, got
  integer on return from block": the callback's hint (`callee_param_hint`) handed the short
  lambda `fn(Tile) -> U` with `U` bound by no argument yet.  A return naming such a variable
  is left open now, so the body declares it (loft#945's rule for `map`'s callback) and the
  call binds `U` from the lambda's type; the hint is also closed (`close_open`), so a
  callback typed by the generic struct itself (`fn(Grid<T>) -> integer`) is handed the call's
  instance — it "bound T to T".  The cells found two more:
  - an instance at `text` answered WRONG on the interpreter: `acc = f(acc)` in `fold<T, U>`
    at `U = text` gave `c` for `abc` (native `abc`), since A6 let a short lambda reach a
    generic.  The parse stages an assignment that reads its destination through a work text
    when the variable is text (P223); a template's `acc: T` is not, so each instance where a
    variable BECAME text now stages it (`stage_text_self_reads`) — guard
    `an-instance-at-text-stages-a-self-reading-assignment`.
  - a lambda answering only `null` bound `U` to `null` and died minting `Grid<null>` (an ICE).
    `null`, `void` and a poisoned value bind nothing in a call either, as in a literal (D4);
    the refusal names the variable ("`map_grid` cannot tell what U is") and the clash check
    no longer reads an unbound variable's parameter as a clash on another — probe
    [goal-refused/](probes/goal-refused/) x01, a `parse_errors` test.
  The axes the guards held fixed (`matrix_axes.py`: nullable, narrow integer, enum, tuple,
  an `if` arm) were crossed in g13–g17 and found three more, two of them on main:
  - a template building a `vector<T>` (a literal `[x]` or an append) at `T = integer?` or at a
    tuple kept the template's record copy at the variable's row — a SIGSEGV, a store panic or
    an internal error on both backends, on main too.  The element write peels its binding
    (`τ?` is `τ`'s shape, @FR-N-Shape) and writes a tuple member by member through the
    concrete append's own `emit_tuple_set_ops`, lowered in the instance's frame
    (`TV_TUPLE_ELEM`) — guard `a-generic-builds-a-vector-of-a-nullable-or-a-tuple`.
  - a tuple's instance key read the registry: spelled `(integer, text)` before its
    `__tuple<…>` struct existed and by that name after, so one type minted two
    `Grid<(integer, text)>` and the binding "changed type" to itself.  `identity_spelling`
    names a tuple as `tuple_def` will, registered or not.
  - a self-reading `acc = if … { f(acc) } else { acc }` at a text instance was assigned whole:
    native met `String` against `&String` (E0308, before any staging too), and staging it
    appended the whole `if`.  A branch now delivers per arm first, as the parse binds one
    (`try_branch_text_bind`), and each arm is staged on its own.
  Corpus against D10: IDENTICAL.  Arc D closes here: `G-Type` (an instance is an ordinary
  type, an open instance is re-read per monomorph, how a literal and a call bind) and
  `G-Regular` are written into `formal/interfaces.md` and cited at their sites.

---

## Arc E — the by-name built-ins become library generics

Its first half is on `main` already: a by-name special case in `dispatch_call` is taken only
when no definition the program declares applies.  One built-in per step, each a parallel run
(the method as E1 built it; the plan as first written is kept below for its measurement):

1. declare the built-in in `default/*.loft` as a generic METHOD on `vector`, marked `#builtin`
   — a stdlib method reserves its name for its receiver type alone;
2. compare `loft introspect` of the corpus against the OLD `default/`, masked: the special
   form stays the lowering, so every call site must read identical;
3. the method spelling reaches the same lowering (`builtin_method_call`).

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
- **E1 first measured** (2026-09-22), as a free generic.  `pub fn reverse<T>(v:
  vector<T>) { reverse(v) }` in `default/` (its body reaches the special form, which the stdlib
  never pre-empts) behind `LOFT_BUILTIN_GENERICS=reverse`, with a call to an instance of a stdlib
  generic whose body is one op inlined in the IR (native's `@FR-R-Wrapper`, done for both
  backends): every corpus CALL SITE is identical on both backends; what differs is the instances
  themselves, minted and emitted but never called.  But the step cannot land: a standard-library
  FREE function's name is RESERVED (`redefinition_text`, loft#863/C95 — *"a program cannot define
  its own `reverse`"*), so declaring `reverse<T>` in `default/` refuses every program that
  defines its own `reverse` — `a-builtin-call-name-never-hides-a-programs-own-function` among
  them, the guard of the special-names half this arc stands on (*"never limit what a user can
  define"*, 2026-09-15).  Arc E as written reserves exactly the names that half freed.  The
  choice is the owner's: (a) a built-in's stdlib generic does not reserve its name — a
  program's definition joins the set and outranks it, as it outranks the special form today;
  (b) no stdlib free name is reserved any more — every program definition beside a stdlib one
  is a member of the set (`G-Select`), the program's ranking first on a tie; (c) the special
  forms stay.  The measured code is [probes/e1-library-reverse.patch](probes/e1-library-reverse.patch)
  (the switch, the IR inline, `Data::instance_template`, `one_op_wrapper` taking an instance of
  a stdlib template).
- **Owner's decision (2026-09-22): the built-ins become stdlib generic METHODS on `vector`.**
  A stdlib method reserves its name only for its own receiver type (measured on `clear`): a
  program's `fn sort(self: Roster)` or `fn sort(r: Roster)` is a member of the set beside it,
  and only a program's own `sort` for `vector` itself is refused — *"allow users to create
  their own versions on their own defined structures"*.
- **E1 Built** (2026-09-22), as a `#builtin` METHOD.  `pub fn reverse<T>(self: vector<T>)
  { reverse(self) }` in `default/01_code.loft`, followed by `#builtin`: the declaration is the
  built-in's SIGNATURE — its place in the name's set (`@FR-G-Select`), its method spelling,
  what hover and the published Standard Library read — and a call that selects it lowers
  through the special form of its name, as the bare call always has.  The marker is a stored
  field (`Definition::builtin`, `DEF_BUILTIN`; stride 183 → 184, `CACHE_FORMAT_VERSION` 11),
  never a name test, and only `default/` may write it (`parse_errors` pins the refusal).
  Nothing about the bare spelling moves: a stdlib definition never pre-empts the special form
  (`program_definition_applies`), so `reverse(v)` is lowered exactly as before; `v.reverse()`
  selects the method and `builtin_method_call` hands it to the same lowering.  So no instance
  is minted and nothing is inlined — the free-generic design above needed an IR inliner of
  one-op instances, which `insert`, `sort`, `map`, `filter` and `reduce` (blocks, temporaries,
  lambdas) could not use; this one is the same for all seven.  A program's own `reverse` for
  its own type works in both spellings; one for `vector` is refused as for any stdlib method
  (`parse_errors`).  Corpus, masked (numbers, the stdlib's path, the schema's registration
  lines), the before side on the OLD `default/`: six generic files lose an unused `__ref_p`
  declaration in their instances (a program's unbounded `T` now shares the stdlib's
  placeholder), and `a-program-generic-named-like-a-builtin-is-what-its-call-reaches` moved its
  first cell from `reverse` to `any` — a value the control's builtin answers in silence.
  Guard: `a-built-in-is-a-method-a-program-extends-for-its-own-types`.
  Measuring note: every binary reads `default/` from the tree, so a corpus diff of a
  `default/` change needs the OLD `default/` on the before side — two step binaries over one
  tree read the same stdlib and reported IDENTICAL while the guard above was refused.
- **Owed by E2–E7 from E1's design.**  The special-names guard above still defines `sort<T:
  Named>(v: vector<T>)` and `reserve<T>(v: vector<T>)`; E2 and E4 refuse both, so their cells
  move to a name that stays a special form (`all`, `count_if`).  The METHOD spelling of `map`,
  `filter` and `reduce` is recognised by name today (`parse_vector_method`, which also hints a
  const receiver's element `const` — loft#1540); once each is a `#builtin` method the call
  finds it as an attribute instead, so the argument hints come from its declared signature
  and the `const` hint has to come with them.

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
- E6's pass-1 return prediction (above).  Arc E's emission equality holds by construction
  for a `#builtin` method (the special form stays the lowering) and is measured per step.
- Nothing here was built.  Every *"works because"* in arcs C and D is a prediction, and the
  step's own gate is what tests it.
