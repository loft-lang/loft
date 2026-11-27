<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 231 — Restrictions that bite: what causes each, and can we lift it

[loft-lang/plans#231](https://github.com/loft-lang/plans/issues/231) · an investigation plan

## Status

| Stage | Status |
|---|---|
| A — Which restrictions bite | 🟡 the loft tracker and four consumers' loft notes read (dryopea, crawler, moros, routing); R10 bites, the rest show no consumer evidence yet |
| B — Cause and lift per biting restriction | 🟡 R10 read against the formal rules; the others carry a first reading |
| C — Lifts | ⏸️ each lift the owner approves becomes its own plan or fix |

## Goal

Find the restrictions that BITE us — the ones a real program written in loft runs into — and,
for each, investigate whether we can lift it.  A restriction that only reads badly on a
comparison page is not this plan's work.

Per restriction:

1. **Does it bite?**  Evidence from a consumer: a workaround in its code, an entry in its loft
   notes, an issue it filed.  No evidence, no work.
2. **What causes it** — the decision (`@C…`), rule (`@FR-…`) or mechanism, by name.
3. **Can we lift it** — what the lift costs, and what it would break.  A lift that reopens a
   decision is the owner's call.

The probes in [`probes/`](probes/) are the evidence for the verdicts, not tests; tests are
written where a lift needs them.  A lift that removes a restriction also removes its refusal and
the doc lines that describe it.

## The restrictions

Verdicts: **real** (a program cannot do it) · **partial** (it can, with a named workaround) ·
**not one** (the probe does it; the doc is wrong) · **by design** (a decision chose it, with a
revisit trigger).

| ID | Restriction | Bites? | Verdict | Cause | Probes |
|---|---|---|---|---|---|
| R10 | A function cannot return a place (`-> &T`) | **yes** — dryopea's lost writes through an accessor (loft#894, LOFT_GOTCHAS.md § Values, copies and tail calls) | real, and the rules allow it | the refusal at the `&` return type; `(F-Ret)` names a `&T` return as its exception | 18, 19 |
| R1 | No proof against aliasing or data races | one loft-found case (loft#1605); no consumer | partial; the data-race half is not one | @C79, @C86; `par` isolation | 03, 04, 13, 14, 16 |
| R2 | No moves, no consuming `self` | through R10: copies are what accessors lose writes to | by design | @C86 | 03 |
| R3 | No dynamic dispatch on interfaces, no default methods | no consumer evidence | partial | INTERFACES.md § What is out of scope | 05, 06, 09 |
| R4 | No metaprogramming | no consumer evidence | real; no decision recorded | none written down | 17 |
| R5 | No lazy iterator chain | no consumer evidence | not one in the language; a library gap | the stdlib's `map`/`filter` are on `vector` | 10, 11, 12 |
| R6 | Closures: not in a collection; `par` takes no reference | loft-found only (loft#1358, #2492) | partial | @C116, @C74, @C115; `par` isolation | 07, 08, 09, 14, 15 |
| R7 | Slower than `rustc -O` | yes — every consumer's speed; @C136 owns it | real, and a defect past 3× | @C136; the keyed and alloc-temp classes | perf portal |
| R8 | No Rust ecosystem, a small own one | not measured | by design | @C170, @C78 (@PLN225) | — |
| R9 | The compatibility promise is not in force | not a code restriction | real until 1.0 | contract 0 (@PLN102) | — |

### R10 — a function cannot return a place

- **Does it bite?**  Yes.  dryopea's notes: *"A struct RETURNED from a function is a COPY, so
  mutating it is a silent no-op"* — `hurt(first(state), 10)` lands no damage, while indexing
  inline or writing through a `for` loop variable lands it (loft#894).  A one-line "get me the
  element" accessor is the most ordinary helper there is, and it can be read through but never
  written through.  crawler met the bind form of the same family (`vv = s.vis` mutating a copy).
- **Measured on main** (probe 18): the write through the accessor is still lost on both
  backends, but `warning[lost-write]` now names it, which is what loft#894 asked for.  The
  cure the warning offers is to index inline, which is the workaround and not a lift.
- **Cause.** `-> &Enemy` is refused: *"a return type, a field, an element or a tuple member
  holds a value and cannot be a link"* (probe 19); LOFT_DECLARATIONS.md states it as an edge of
  the `&` ladder (a reference can't escape its source).  But `formal/calls.md` `(F-Ret)` names
  *"a `&T` return … an explicit borrow of an addressable input"* as its one exception.  So the
  rules allow what the compiler refuses — by the doctrine, the code is the deviation, unless the
  owner rules the exception out of the rule.
- **Can we lift it?**  The lift the rule describes is narrow: a `&T` return whose place is
  rooted in a PARAMETER (an addressable input), never in a local.  That is the one case a
  lifetime is not needed for, because the place outlives the call by construction; the caller's
  `&` parameter machinery already links a place.  What must be settled first: what the caller
  may do with the returned link (bind it with `&`, pass it on, write through it), and that a
  reshape of the container while the link lives stays a compile error (@C86).  Effort M; the
  first step is a probe matrix over those three uses on both backends, written before any code.

### R1 — aliasing and data races

- **Cause.** Ownership is internal and there is no user-facing borrow checker (@C79).  A
  whole-value bind copies; sharing happens only through a parameter or an explicit `&` (@C86).
  `const` marks a name, not a value.
- **Is it one?**  The data-race half is narrow: a `par` worker runs on a clone whose parent stores
  it may only read (THREADING.md, `Stores::clone_for_light_worker`), and a captured reference is
  refused (probe 14), so no two workers write one store.  What is left is impurity: a worker
  that prints or touches a file has an UNDEFINED result (`@FR-C-Impure`), and loft does not
  check for it (probe 15 prints, on both backends).  The aliasing half is
  narrower than Rust's problem.  Two names write one place only when a parameter shares it, and
  the call site shows that.  Reshaping a container under a live `&` is a compile error (@C86).
- **Measured.** Two parameters of one call can name the same struct: `both(a, a)` writes
  `p.x = 10` and reads `q.x` as 10, on both backends (probe 16).  That is the one place where
  loft has aliasing that Rust would refuse, and no formal rule in `formal/binding.md` states
  what it answers.
- **What we could do.** (a) Refuse a call that passes one place to two heap parameters, at
  least one of them written; or (b) keep it and state the answer as a rule.  Either way the rule gets written.  There is also an upside to measure: where the
  compiler knows a parameter is unaliased, the C backend could mark it `restrict`, a speed
  lever for R7.  That is a hypothesis, not a promise.

### R2 — moves

- **Cause.** "Every variable is its own value" (@C86): a bind copies, and the copy is skipped
  when the source is provably dead afterwards (the last-use elision).  So a move already happens
  wherever it is safe.  The programmer just cannot ask for one.
- **Is it one?**  For speed, no: a dead source costs nothing to copy.  What it takes away is
  typestate: a consuming method that leaves the old value unusable, such as a builder that
  cannot be finished twice.
- **What we could do.** Nothing, unless a consumer shows a typestate need.  The revisit trigger
  in @C86 names bind copies dominating a profile, and says that argues for widening the
  elision, never for moves.

### R3 — dynamic dispatch

- **Cause.** An interface constrains a type; it is not a type (probe 05's refusal).  Generics
  are monomorphised, so every call site knows the concrete type.  Default bodies and interface
  inheritance are listed as out of scope "for now" in INTERFACES.md, not refused by a decision.
- **Is it one?**  Partly.  A closed set of types dispatches through an enum's per-variant
  functions.  An open set — a library user adding a type — can be built by hand as a struct of
  `fn` fields, and those fields may capture (probe 09).  What is missing is the convenience: the
  compiler building that struct from an interface, and default bodies (probe 06).
- **Still to probe.** The enum route when a library's enum is extended by its user: can it be,
  or is the open-set case only the `fn`-field form?
- **What we could do.** Default method bodies are the cheapest step: the body is an ordinary
  generic function, so no new runtime mechanism.  An interface value (`x: Named`) is a struct of
  fn-refs the compiler fills from the concrete type.  That is a real feature, M or larger, and
  it needs a decision.  Both are the owner's call.

### R4 — metaprogramming

- **Cause.** None recorded.  There is no macro, `derive` or compile-time code facility, and no
  `@C` entry says why.  SUBJECTS.md would show such a gap as `— none recorded`.
- **Is it one?**  Yes.  Rust uses `derive` for printing, comparing and serialising.  loft covers
  those without it: `"{x}"` and `:j` print any struct, `==` is structural for every type (@C134),
  and JSON reads into a struct by its fields.  So most of what `derive` is used for is built in.
  What remains is code generation: repetitive declarations, domain-specific syntax.
- **What we could do.** First write the decision down, whichever way it goes.  The options in
  order of cost: no macros, with the reason recorded; compile-time functions (there is no `const fn`, probe 17;
  what exists is a literal-bodied function treated as a constant, `@FR-R-Const`, and field
  defaults folded at compile time); a hygienic macro system, which is H and would add a second
  language to learn.

### R5 — lazy chains

- **Cause.** `map`, `filter` and `reduce` in `default/01_code.loft` take and return `vector`.
- **Is it one?**  Not in the language.  A generic generator adapter is four lines and lazy on
  both backends: probe 10 stops after three of a million elements, probe 11 is the generic form.
- **What we could do.** Add lazy adapters over `iterator<T>` to the stdlib (`map`, `filter`,
  `take`, `zip`), and measure them against the `vector` forms on both backends.  S.  Then
  `VS_RUST.md` row 5 moves to "a library gap" or goes away.

### R6 — closure limits

- **Cause.** A collection has one element layout, and each capture set is its own record shape
  (@C116).  A shared mutable scalar cell has no owner in the store model (@C74).  A closure holds
  a copy of the value, not of the binding (@C115).  A `par` worker reads an isolated clone, so a
  captured reference would point into a store it does not have.
- **Is it one?**  Partly.  The cure @C116 names works: a vector of structs whose `fn` fields
  capture runs on both backends (probe 09).  So the restriction is one wrapper struct deep.  For
  `par`, a scalar or the loop element may be passed; a struct cannot (probe 14).
- **Settled by the rules.** Probe 15 has a `par` worker `println`.  `@FR-C-Impure` makes an impure
  worker's result undefined and says loft does not check it, which is what `00-vs-rust.html` § 14
  says.  So the page is right, and the open choice is whether to CHECK purity: refuse a worker
  that reaches I/O, or keep it a documented programmer obligation.
- **What we could do.** For collections: let the compiler box a capturing closure into the
  wrapper struct by itself (the shape probe 09 builds by hand).  That is cheap if @C116's
  layout argument is the whole cause, and it needs the owner's call.  For `par`: pass a struct
  argument by copying it into the worker's clone.  That costs one copy per worker, not per
  element.

### R7 — speed

- **Cause.** Measured by class (PERF_PORTAL.md): keyed collections and short-lived temporaries
  are the slowest.  The C backend's median is 1.43× the Rust reference, the `--ship` build's
  1.30× (`bench/portal/results/laptop.tsv`).
- **Is it one?**  Yes, and @C136 already makes a routine past 3× a defect.  No new decision is
  needed.
- **What we could do.** Nothing new in this plan: the work is the perf portal's worklist.  This
  plan's one addition is R1's `restrict` hypothesis, as a measured experiment.

### R8 — ecosystem

- **Cause.** A backend stays only while it answers every program as the others do (@C170), so
  the rustc writer and the Rust-crate path went (@PLN225).  A library's native part is a C shim
  bound by `#c`.  Above the engine there are no black boxes (@C78).
- **Is it one?**  It is a choice with a cost: C libraries are reachable, Rust crates are not.
- **What we could do.** Measure what consumers ask for that only exists as a crate, then
  decide case by case: a C library to bind, or a loft library to write.  No reopening of @C170.

### R9 — the compatibility freeze

- **Cause.** loft is at contract 0, the era with no promise (COMPATIBILITY.md); the soft
  freeze records every break.  The 1.0 freeze is @PLN102's.
- **Is it one?**  Yes, and it is a timing question, owned by @PLN102.  This plan only keeps the
  row true.

## Documentation

- [VS_RUST.md](../../VS_RUST.md) — every verdict that moves a row, in the same commit.
- [`00-vs-rust.html`](../../../00-vs-rust.html) via `tests/comparisons/` — § 12 says there are no associated
  types (they shipped, @PLN125); § 14's "no println" is right (`@FR-C-Impure`).
- [SUBJECTS.md](../../SUBJECTS.md) — a row for metaprogramming once R4 has a decision.
- [INTERFACES.md](../../INTERFACES.md) § What is out of scope — if R3's default bodies are taken.
- [STDLIB.md](../../STDLIB.md) — if R5's lazy adapters are added.
- DESIGN_DECISIONS — a new `@C` entry for each restriction the owner keeps on purpose.

## See also

[STRONG_POINTS.md](../../STRONG_POINTS.md) · [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) ·
[THREADING.md](../../THREADING.md) · [PERF_PORTAL.md](../../PERF_PORTAL.md) ·
[COMPATIBILITY.md](../../COMPATIBILITY.md) · [bars/](../../bars/README.md)
