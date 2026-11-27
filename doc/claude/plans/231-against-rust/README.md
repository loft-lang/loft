<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 231 — Against Rust: each restriction's cause, whether it is one, and what we could do

[loft-lang/plans#231](https://github.com/loft-lang/plans/issues/231) · an investigation plan

## Status

| Stage | Status |
|---|---|
| A — Probe catalogue | ✅ 17 probes in [`probes/`](probes/), each with its expected answer, run on the interpreter and the C backend |
| B — Cause and verdict per restriction | 🟡 a first reading for all nine below; R3 and R6 still owe the probes named in their rows |
| C — The probes become tests | 🔴 not started |
| D — Cures | ⏸️ each cure the owner approves becomes its own plan or fix; none is built here |

## Goal

[VS_RUST.md](../../VS_RUST.md) lists nine things loft lacks against Rust.  For each one this plan
answers three questions:

1. **What causes it** — the decision (`@C…`), rule (`@FR-…`) or mechanism, by name.
2. **Is it a restriction at all** — a probe that tries to do the thing, on both backends.  A gap
   that a program can already close is a documentation or library gap, not a language one.
3. **What could we do** — the cure and what it costs, or why it stays.  A cure that reopens a
   decision is the owner's call: this plan states the trade and does not reopen anything.

And it turns the probes into tests (stage C), so a row of `VS_RUST.md` or of
`00-vs-rust.html` cannot go stale without a test going red.

## In-plan vs spinoff

A wrong claim in a doc found here is fixed here.  A cure is a spinoff: it gets its own plan or
fix once the owner picks it, because each one is a language or library change with its own
verification.

## The restrictions

Verdicts: **real** (a program cannot do it) · **partial** (it can, with a named workaround) ·
**not one** (the probe does it; the doc is wrong) · **by design** (a decision chose it, with a
revisit trigger).

| ID | Restriction | Verdict | Cause | Probes |
|---|---|---|---|---|
| R1 | No proof against aliasing or data races | partial; the data-race half is not one | @C79, @C86; `par` isolation | 03, 04, 13, 14, 16 |
| R2 | No moves, no consuming `self` | by design | @C86 | 03 |
| R3 | No dynamic dispatch on interfaces, no default methods | partial | INTERFACES.md § What is out of scope | 05, 06, 09 |
| R4 | No metaprogramming | real; no decision recorded | none written down | 17 |
| R5 | No lazy iterator chain | not one in the language; a library gap | the stdlib's `map`/`filter` are on `vector` | 10, 11, 12 |
| R6 | Closures: not in a collection; `par` takes no reference | partial | @C116, @C74, @C115; `par` isolation | 07, 08, 09, 14, 15 |
| R7 | Slower than `rustc -O` | real, and a defect past 3× | @C136; the keyed and alloc-temp classes | perf portal |
| R8 | No Rust ecosystem, a small own one | by design | @C170, @C78 (@PLN225) | — |
| R9 | The compatibility promise is not in force | real until 1.0 | contract 0 (@PLN102) | — |

### R1 — aliasing and data races

- **Cause.** Ownership is internal and there is no user-facing borrow checker (@C79).  A
  whole-value bind copies; sharing happens only through a parameter or an explicit `&` (@C86).
  `const` marks a name, not a value.
- **Is it one?**  The data-race half is not: a `par` worker runs on a clone whose parent stores it
  may only read (THREADING.md, `Stores::clone_for_light_worker`), and a captured reference is
  refused (probe 14).  So no two threads can write the same place.  The aliasing half is
  narrower than Rust's problem.  Two names write one place only when a parameter shares it, and
  the call site shows that.  Reshaping a container under a live `&` is a compile error (@C86).
- **Measured.** Two parameters of one call can name the same struct: `both(a, a)` writes
  `p.x = 10` and reads `q.x` as 10, on both backends (probe 16).  That is the one place where
  loft has aliasing that Rust would refuse, and no formal rule in `formal/binding.md` states
  what it answers.
- **What we could do.** (a) Refuse a call that passes one place to two heap parameters, at
  least one of them written; or (b) keep it, state the answer as a rule and make probe 16 its
  cell.  Either way the rule gets written.  There is also an upside to measure: where the
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
- **Still to probe.** Probe 15 shows a `par` worker may `println`, while `00-vs-rust.html` § 14
  says a worker may not touch shared state "(no println, no file access)".  Is printing from a
  worker a supported, ordered result on both backends and in a browser page, or does the rule
  hold and go unchecked?  The answer decides whether the page or the compiler changes.
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

## Stage C — the probes become tests

Each probe becomes a cell, so the comparison is checked on every run:

- A claim the user page makes (`00-vs-rust.html`) → a test in its `tests/comparisons/<subject>.loft`
  file, under the `@SUBJECT` it belongs to.
- A language rule a probe exercises → a cell in `../loft-suite` under that rule, with its header
  naming the `@FR-` rule, and a `@falsified-at` receipt (`make falsify`).
- A refusal (probes 04, 05, 06, 07, 14) → an `@EXPECT_ERROR` cell, so the restriction showing up
  or going away is a test change, not a surprise.

Probe 02 needs both runs (with and without `--production`); probe 10 asserts the count of
elements made, not only the sum, or it cannot tell lazy from eager.

## Documentation

- [VS_RUST.md](../../VS_RUST.md) — every verdict that moves a row, in the same commit.
- [`00-vs-rust.html`](../../../00-vs-rust.html) via `tests/comparisons/` — § 12 says there are no associated
  types (they shipped, @PLN125); § 14's "no println" depends on R6's probe.
- [SUBJECTS.md](../../SUBJECTS.md) — a row for metaprogramming once R4 has a decision.
- [INTERFACES.md](../../INTERFACES.md) § What is out of scope — if R3's default bodies are taken.
- [STDLIB.md](../../STDLIB.md) — if R5's lazy adapters are added.
- DESIGN_DECISIONS — a new `@C` entry for each restriction the owner keeps on purpose.

## See also

[STRONG_POINTS.md](../../STRONG_POINTS.md) · [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) ·
[THREADING.md](../../THREADING.md) · [PERF_PORTAL.md](../../PERF_PORTAL.md) ·
[COMPATIBILITY.md](../../COMPATIBILITY.md) · [bars/](../../bars/README.md)
