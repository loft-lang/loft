<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 231 — the restrictions, one by one

The companion of [README.md](README.md): for each restriction, whether it bites, its cause,
what was measured and whether it can be lifted.  Probe numbers refer to [probes/](probes/).

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
- **Can we lift it?  Feasible — every part but the return already exists.**  The probes use the
  shape the lift would hand a caller: a local link to the same place (`e = &s.enemies[0]`), which
  is what `first(s)` would return.  All ten answer as computed by hand, on both backends:

  | Probe | Use of the link | Answer |
  |---|---|---|
  | 20 | write a field through it | lands (90) |
  | 21 | pass it to a writing callee | lands (90) |
  | 22 | a link rooted in a PARAMETER, made in a callee | lands, the sibling untouched (90 / 50) |
  | 23 | grow the container while the link is used after | refused at compile time |
  | 24 | grow it after the link's last use | runs (90 / 3) |
  | 25 | remove an element while a link to another is used after | refused at compile time |
  | 26 | link to an element that does not exist | no write, the program runs on (100 / 2) |
  | 27 | link into a `hash` element | lands (40) |
  | 28 | replace the whole value through the link | lands, the sibling untouched (7 / 50) |
  | 29 | append to a text field through a link | lands (`s!`) |

  The return itself needs only the two facts that already meet nowhere (probe 30,
  `LOFT_VAR_TABLE`): `first` is typed `Reference(…, Deps { items: [0] })` — the return records
  that it borrows parameter 0, `(O-Move)`'s `{Attr(param)}` — and a link binds with
  `deps=[st]`, the dependency `src/scopes/reshape_refusals.rs` reads for probes 23 and 25.
  Today the caller binds `first(st)` as `OWNS`, a copy.  The lift is the call site of a `-> &T`
  function binding its result with the return's dependency mapped onto the argument, exactly
  as `l = &st.enemies[0]` binds; and the callee refusing a `-> &T` whose returned place depends
  on no parameter (a local, a fresh value).
- **Still open — the design questions the lift has to answer before it is built:**
  1. An argument that is a temporary (`first(mk())`): the place dies at the end of the
     statement.  Refuse the call, or materialise the value as a copy (what happens today)?
  2. A `const` argument: a link into it would allow a write the `const` forbids.  Refuse, or
     hand back a read-only view?
  3. A callee that returns a link it received (`fn pick(a: &T, b: &T) -> &T`): the dependency
     is on BOTH parameters, so the caller's reshape check must hold both arguments.
  4. The rule text: `(F-Ret)` says "an addressable input" — the lift above is that rule, so the
     rule needs no change; LOFT_DECLARATIONS.md's "no `&T` return type" does.
- **Verdict.** Liftable, narrowly, and the formal rule already describes it.  Out of the plan as
  [loft#2659](https://github.com/loft-lang/loft/issues/2659) (the probe table and the four
  questions) and the deviation **D-call-32** in `formal/calls.md`.

### R11 — restrictions consumers still work around, which loft has lifted

- **Does it bite?**  Yes, and it is the cheapest bite to remove: a consumer that still believes a
  restriction pays for it in every file it writes.  dryopea's `CLAUDE.md` § Loft language
  gotchas and `docs/LOFT_GOTCHAS.md` steer its code by thirteen loft issues — loft#873, #876,
  #877, #880, #894, #914, #915, #918, #935, #955, #962, #969, #974 — and **all thirteen are
  closed**.  hexbody's `CLAUDE.md` carries the loop-variable rule too.  crawler's `CLAUDE.md`
  § "the traps that will bite you today" cites loft#338, #392 and #854, all three closed, and
  calls its `ortler` tools unusable because of #854.
- **Measured on main** (probes 42–52, both backends):

  | Workaround the consumer still writes | Issue | On main |
  |---|---|---|
  | loop variables prefixed per function (`tslr_w`), because a name keeps one type per function | #915 | two loops reuse `e` at two types: `t=6`; a name rebinds after its loop |
  | a negative number kept out of a file-scope `const vector` | #955 | `len 3`, `B[1] == -5` |
  | a struct always built from `*_empty()`, never a partial literal; fields named so that zero is the safe value (`top_removed` for `has_top`) | #914, #876 | a declared default is honoured (`200`); an undeclared one gets `advice[omitted-field-zero]` |
  | a call bound to a local before it is indexed in tail position | #877 | `steps(a)[0] ?? -1` answers `5` |
  | an accessor never written through | #894 | still lost, and now warned — that one is R10 |
  | a struct with a `hash` field never interpolated | #873 | prints `{h:[{k:"a",v:1}],n:3}` |
  | an inner tail call bound to a local before returning | #880 | two nested tail calls keep the loop's 4 cells |
  | a helper declared above its caller before a local binds its call | #918 | a call to a lower function binds: `later 6` |
  | a keyed record read inline, never through an accessor | #974 | three accessor reads with a struct made between: `2 2 2` |
  | crawler: never swap struct elements through a held link | #338 | the swap answers `2 1` (probe 52) |
  | dryopea: never bind a `FlowField` to a local in a per-enemy path (*"2250×"*) | — | no gap against indexing inline, both backends (probes 55, 56) |
  | routing: walk a collection by index, because `for x in t.routes` copies the records | — | the `for` is the faster one on C (probes 57, 58) |
  | crawler: the `ortler` tools are unusable, an 86 400-element literal takes 18 min to parse | #854 | parses and runs in 1.8 s on the interpreter, `last 657` as computed (the file is generated, not kept: `const BIG: vector<integer> = [ … ]` holding `i % 997` for `i` in `0..86400`, written out by a script) |

- **Cause.**  Nothing in loft: the notes were written when the defects were live, and nothing
  tells a consumer that a cited issue closed.
- **Can we lift it?**  The restrictions are already lifted; what is left is the consumers'
  notes.  Their trees are read-only to this stream, so the report lands here, for their agents:
  each row above is a line in their notes to retire, and the probe beside it is the evidence.
  Nine of the thirteen issues are re-measured, and every one holds.  Four are closed and not yet
  re-run, because their shapes need more than one file: #935 (a very large function), #962 (a
  const read through an aggregator module), #969 (two library functions over one field), and
  #876's JSON-cast half.

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

### R5 — `map`, `filter` and `reduce` take only a vector

- **Does it bite?**  No consumer has hit it: nothing outside loft's own tests and docs calls
  them (the `graphics` hits are an unrelated `Texture.filter`).  The owner's reason to keep and
  fix them rather than remove them: a newcomer reaches for them out of habit, and that first
  use should work.
- **Cause.**  The formal rules define all three as a rewrite over ANY source a `for` takes —
  `(I-Map)` `src.map(f) ≡ [ for x in src { f(x) } ]`, `(I-Filter)`, `(I-Reduce)` in
  `formal/iteration.md`.  The compiler parses them on their own (`parse_map`, `parse_filter`,
  `parse_reduce`, `reduce_is_special` in `src/parser/`, about 730 lines) and refuses every
  source but a vector.  That refusal is a deviation the rules do not record.
- **Measured** (probes 31–41, both backends):

  | Source | The comprehension | `map` / `filter` / `reduce` |
  |---|---|---|
  | vector | works | works, method form and fn-refs included |
  | hash, sorted | works (probes 38, 40) | refused: *"first argument must be a vector"* (33, 35) |
  | text | works (39) | refused (34) |
  | generator | works (37) | refused (32) |
  | range | works inside `for` | a range is not a value at all (41) — a separate limit |

  With a lambda the real refusal is hidden behind *"Cannot infer type for lambda parameter"*
  (31, 36): the user is first told to type the lambda, then refused once they have.
- **Two layers, kept apart.**  The rules say what a call MEANS: a fresh vector (`(I-Comp)`),
  `filter` finished before `map` starts.  The rewrites decide what the C DOES (`formal/rewrites.md`).
  Read from `--native-emit` (probes 53, 54):
  - `map` and `filter` are already lowered to the comprehension's loop, and each loop gets the
    same rewrites: a push window (`(R-Push)`) and a hoisted vector header (`(R-Header)`).  What
    is separate is the parsing and the vector-only source check — not the code they produce.
  - `map(filter(v, p), f)` is two loops with an intermediate buffer claimed, filled and freed;
    `[for x in v if p(x) { f(x) }]` is one loop with none.  No rewrite fuses the stages.
  - The lambdas stay calls (`static inline` C functions); inlining them is the C compiler's.
- **Can we lift it?  Feasible.**  The target is the right-hand side of each rule, and that
  already works for every source on both backends.  The lift is to parse the three calls (and
  their method forms) as exactly that rewrite; the element type then types the lambda as it does
  in a comprehension, and the code produced stays what it is today, rewrites included.
  Laziness is not part of it: the rules give a vector result, which is also what a Rust user's
  `.collect()` gives.
- **And an optimisation, not a restriction:** fusing `map(filter(…))` into one loop is a
  rewrite the layer could admit wherever the difference cannot be observed — where both lambdas
  are pure — as the other `R-` rules carry their admission conditions.  It is not part of the
  lift, and it is OWED all the same: measured, the chain is 4–5× the fused comprehension on C
  (README § The natural form owes its speed, probes 59–60), and goal F gives the natural
  spelling the efficient code.
- **Verdict.**  Liftable, and the rules already say how.  Out of the plan as
  [loft#2658](https://github.com/loft-lang/loft/issues/2658) and the deviation **D-iter-18** in
  `formal/iteration.md`.

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
  layout argument is the whole cause, and it needs the owner's call.  For `par`: no copy is
  needed.  `(C-Shared)` gives a worker its starter's stores shared and read-only, and
  `(C-Det)` admits context arguments; the refusal's reason, *"a par worker runs on an isolated
  store clone"*, predates @PLN226, which made the stores shared.  The loop element is already a
  struct read that way.  A struct context argument is the same read — a representation-layer
  refusal with an out-of-date reason, recorded here and not filed, for want of a consumer.

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
