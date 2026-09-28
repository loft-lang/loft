<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 7 — Fused `for x in ls par(r = foo(x), 4) { … }`

**Status: open**

## Insight from phase 0a

The fused for-loop syntax **already exists in the parser today** —
`tests/scripts/22-threading.loft` and the new
`tests/threading_chars.rs` both exercise it.  Phase 7 is therefore
NOT about introducing the construction; it's about:

1. Making the fused form route through @PLAN06's typed pipeline
   (phases 1–5's runtime work).
2. Adding the desugar of the value-position call form
   `par(input, fn, threads)` so it produces the same `Value::ParFor`
   IR.
3. Adding `par_fold(...)` as a sibling sugar that auto-routes to
   `Stitch::Reduce`.
4. Removing `par_light` from the user surface entirely.
5. Auto-detecting pure-fold body in the fused for-loop and routing
   to `Stitch::Reduce` automatically.

The "Goal" section below kept its original wording for context;
read it as "the goal of @PLAN06 around this construction is..."
not "the goal of phase 7 is to build this".

## Goal

Replace the planned phase-7 three-variant API (`par_for_each`,
`par_fold`, `par_iter`) with a single syntactic construction that
covers all four "shapes" by writing normal loft code:

```loft
for x in ls par(r = foo(x), 4) {
    // body — runs sequentially in the parent thread,
    //         sees `x` and the parallel result `r`
}
```

Body discards `r` → for_each.  Body accumulates → fold.  Body
appends → collect.  Body yields → iterator.  **One primitive,
four use cases — the body picks the policy.**

The vector result becomes opt-in: users who want one allocate it
explicitly in the body; users who don't pay nothing.

## Why one construction beats three variants

The 4-variant approach (sketched in @PLAN06 README phase 7 placeholder)
exposes three named entry points: `par_for_each`, `par_fold`,
`par_iter`.  Each is fine in isolation but they fail to compose for
common patterns.  Specifically, all three lose the **(input, result)
pairing** — `par_iter(xs, fn)` yields `iterator<U>`, not
`iterator<(T, U)>`, so a body that needs both must build an explicit
tuple at extra cost.

The fused form pairs them naturally because the body is a
for-loop body that already binds `x`; adding `r` is one assignment
in the loop header.

| Capability | 4 variants | Fused form |
|---|---|---|
| Body sees both `x` and `r` | tuple workaround | native |
| Body mutates enclosing locals | partial (fold's combine fn) | native — body runs in parent |
| `break` / `continue` in body | needs `break_with` plumbing | native — for-loop body |
| Side effects on shared state | lock dance per worker | native — body sequential, no contention |
| Composes with future `yield` (CO1) | needs explicit chain | native — body can `yield r` |

The trade-off is parser work for the new syntax (modest) versus
three new stdlib fns + their type signatures + their docs.  The
runtime cost is identical — both shapes use @PLAN06's store-stitch
pipeline with a bounded queue.

## Surface

```loft
for <var> in <input_expr> par(<result_var> = <worker_expr>, <threads>) {
    <body>
}
```

Where:

- `<var>` is a fresh loft identifier bound to each input element in
  turn (same as a regular `for x in xs`).
- `<input_expr>` is any expression of type `vector<T>`,
  `sorted<T>`, `index<T>`, or `hash<T>`.
- `<result_var>` is a fresh identifier bound to the worker's
  per-element output, visible inside the body.
- `<worker_expr>` is an expression in scope of `<var>` that the
  worker evaluates in parallel.  Currently must be a function call
  or method call; arbitrary expressions are deferred to a follow-up.
- `<threads>` is an `integer` for the worker count.  Reuses the
  existing clamping logic from `par(...)` (clamps to rayon's
  available pool; under WASM single-threaded falls back to a
  sequential for-loop).
- `<body>` is a regular for-loop body — full mutable access to
  enclosing variables, supports `break` / `continue` / `return` /
  `yield`.

### Examples

```loft
// for_each — side effects only, no result vector
for x in items par(_ = process_async(x), 4) {
    log_info("done: {x}")
}

// fold — accumulate
total: float = 0.0
for x in items par(score = score_of(x), 4) {
    total += score
}

// collect — explicit vector allocation
results: vector<Score> = []
for x in items par(score = score_of(x), 4) {
    results += [score]
}

// streaming consumer with `break`
for x in items par(score = score_of(x), 4) {
    if score > threshold {
        notify(x, score)
        break               // signals workers to stop
    }
}

// pair-aware body
for x in items par(score = score_of(x), 4) {
    if score < 0.0 { log_warn("negative score for item {x}") }
    summary[x.id] = score
}
```

### `par(...)` becomes a parser-side desugaring

`par(input, fn, threads) -> vector<U>` and `par_light(input, fn, threads) -> vector<U>`
**stop being independent runtime entry points** and become a
parser-side desugaring of the fused form with an automatic
collect body.  Source-level behaviour is identical to today; the
implementation collapses to a single runtime path.

```loft
// User writes:
results = par(ls, foo, 4)

// Parser desugars to:
results = {
    __par_acc: vector<U> = vector_with_capacity(len(ls))
    for x in ls par(__par_r = foo(x), 4) {
        __par_acc += [__par_r]
    }
    __par_acc
}
```

`U` is inferred from `foo`'s return type via `Data::fn_return_type`
(per DESIGN.md D3).  The block expression is expression-positioned,
so every existing call site (argument position, tail expression,
`if`/`match` arms, return statement, struct-field initialiser)
keeps working unchanged.

### Where `__par_acc` and friends get their slots

Loft's parser runs in two passes; scope analysis (variable→slot
assignment in `src/variables/`) runs **after** parse pass 2 but
**before** codegen.  The desugar must produce IR that the existing
scope-analysis pass can process — it cannot pre-allocate slots
itself, because the surrounding fn's variable table is built later.

**The rule.**  The desugar runs in parse pass 2 and emits IR that
introduces `__par_acc`, `__par_r`, `__par_x` as **new local
declarations in the surrounding fn's `Function::variables`
table**.  Specifically:

1. The desugar uses the existing `Parser::fresh_var(prefix:
   "__par_acc")` helper (already used by format-string desugaring
   for its temporaries — `src/parser/expressions.rs`).
   `fresh_var` allocates a unique name AND registers it in the
   enclosing function's variable table with the right `Type` and
   `Scope`.
2. The synthesized `Value::Block` references the variables by
   name; scope analysis allocates the slots in the enclosing fn's
   frame at its normal pass.
3. Lifetime: the variables are scoped to the synthesized block;
   `get_free_vars` emits `OpFreeRef` at block exit using the
   existing scope-exit mechanism — no special-casing.

**Why this works.**  Format-string desugaring already creates
synthesized locals this way (e.g. for the temporary holding the
formatted result before assignment); the same mechanism handles
`par`.  The desugarer is a **producer** of variables, not a
**slot-allocator** — slot assignment stays with scope analysis as
the single owner.

**What does NOT work.**  Trying to desugar `par(...)` into a
`Value::Block` with locally-scoped variables that bypass the
enclosing fn's variable table would break: `Value::ParFor`'s body
references slots, and slots are only meaningful in the context of
a function's frame.  The desugar must thread its variables through
the same machinery normal user-written variables use.

**Verification.**  Phase 7c adds
`tests/issues.rs::par_call_desugar_slot_independence` —
a fixture that has 30 distinct `par(...)` calls in one fn (each
desugaring to its own `__par_acc`/`__par_r` pair); asserts
`fresh_var`'s monotonic counter produces unique slot allocations
and no slot collisions occur.

**Why a desugar, not a deprecation:** today's call form is the
ergonomic shape for "I need a vector"; phase 7's fused form is
the ergonomic shape for everything else.  Keeping both surfaces
while collapsing the runtime to one path gives the best of both —
no caller breaks, no two implementations to maintain, and the
sugar form gets `vector_with_capacity` pre-alloc for free
(impossible if users wrote the fused form by hand).

**`par_light` is removed from the user-visible surface entirely.**
The "light-path" execution strategy was a stopgap user-facing flag
because the runtime had no way to detect when scratch memory could
be skipped.  After @PLAN06 phase 5's auto-light heuristic, the
compiler picks the light path automatically — it's a property of
the worker fn, not a user choice.

Phase 7c removes `parallel_for_light` from `default/01_code.loft`
and renames the existing `par_light(...)` call sites in
`tests/scripts/`, `lib/`, and any internal docs to plain `par(...)`.
The auto-light heuristic produces the same execution profile.

Result: `par_light` ceases to exist in the language.  A user
writing `par_light(...)` after 0.9.0 gets a normal "unknown
function: par_light — did you mean par?" error from the parser,
same as any other typo.  The compiler still picks the light
internal path when applicable; users never need to know.

## Implementation

### Parser changes — two recognition rules

**Rule 1 — fused for-loop modifier.**  `src/parser/control.rs::parse_for_loop`
extended to recognise the `par(...)` modifier between the input
expression and the body.  Grammar fragment (in pseudo-EBNF):

```
for_loop := 'for' ident 'in' expr [par_clause] '{' body '}'
par_clause := 'par' '(' ident '=' expr ',' expr ')'
```

The keyword `par` is contextual — it's not reserved at expression
position, so existing `par(input, fn)` calls continue to parse as
function calls.  Inside `for ... in expr <here> { body }`, the
parser tries `par_clause` first and falls back to the brace-block.

**Rule 2 — value-position desugar.**  `src/parser/control.rs::parse_call`
recognises `par(input, fn, threads)` (and `par_light(...)`) at any
expression position and rewrites it to a `Value::Block` containing:
1. `Set(__par_acc, vector_with_capacity(len(input)))`,
2. `Value::ParFor { input, worker_fn, threads, x_var, r_var, body: AppendVector(__par_acc, Var(r_var)) }`,
3. `Var(__par_acc)` as the tail expression.

Both rules emit the same `Value::ParFor` IR node; codegen has one
target.

**Source-span preservation.**  Every synthesized IR node carries
the original `par(...)` call's source span (line, col, length)
threaded through `Definition::position`.  Diagnostics in the
desugared body refer to the user-written call site, not the
synthesized for-loop; the same mechanism format-string desugaring
and `?? return` already use.

**Method-form support.**  `par(ls, .my_method, 4)` desugars with
`r = x.my_method()` instead of `r = foo(x)`.  Detected by
inspecting `fn` token type at parse time; one extra branch in the
desugarer (~10 lines).

**Lambda-form support.**  `par(ls, |x| x * 2, 4)` works without
extra desugarer logic — lambdas are already callable values; the
desugar emits `r = (lambda)(x)` which goes through the existing
fn-ref dispatch.

Lowered IR: a new `Value::ParFor` variant carrying `(input,
worker_fn_d_nr, x_var, r_var, threads, body, src_span)`.  Codegen
emits a runtime call that wraps @PLAN06's store-typed pipeline
with a bounded-queue stitch policy.

### Codegen changes

After phase 1–5 land, the runtime exposes one polymorphic store-typed
parallel call that takes:

- input store + element type,
- worker fn def_nr,
- thread count,
- a stitch policy (concat | discard | reduce | queue).

`Value::ParFor` selects the **queue** policy.  The runtime allocates
a bounded queue Store sized `2 × threads`, spawns workers that push
`(x_idx, x_payload, r_payload)` tuples, and runs the body
sequentially in the parent thread by popping in order.

### Runtime changes

Add `src/parallel.rs::run_parallel_queue` (or a `Stitch::Queue`
variant of the polymorphic dispatcher built in phase 3).
Behaviour:

1. Allocate bounded queue store with capacity `2 × threads`.
2. Spawn `threads` worker threads, each in a loop:
   - claim next input index via shared atomic counter,
   - compute `r = foo(x)`,
   - push `(x, r)` onto the queue (blocks if full),
   - on shutdown signal: drain pending writes and exit.
3. Parent body loop:
   - pop next available tuple from queue (blocks if empty),
   - bind `x_var` and `r_var` in the body's scope,
   - execute body,
   - on `break`: signal shutdown to workers, drain queue.
4. Join workers; deallocate queue store.

**Completion-order delivery, not input-order** (per DESIGN.md
D1c).  The queue is FIFO over completion events; the parent body
sees `(x, r)` pairs in whatever order workers finish, not the
order they appeared in `input`.

This trades a small ergonomic surprise for substantial perf
freedom:
- No per-input-index synchronisation point — workers can post
  results as fast as they finish.
- No head-of-line blocking when one worker is slow.
- Aligns with the call-form `par(...)`'s contract: `vector<U>`
  is an unordered multiset.

Users who need ordered iteration should use a regular `for` loop
(no par); @PLAN06 is explicit that ordered parallel iteration is
out of scope.

### Test fixtures

`tests/scripts/22-threading.loft` already covers `par(...)` returning
a vector — those tests now exercise the **desugar path** (call form
→ `Value::ParFor` with auto-collect body) and must keep passing
unchanged.  Add five fixtures specifically for the new shapes:

| Fixture | Body shape | Asserts |
|---|---|---|
| `tests/scripts/par_for_each.loft` | side-effect only | output log lines; zero result-vector allocation (verify via `LOFT_STORES=warn`) |
| `tests/scripts/par_for_fold.loft` | accumulate to a single value via the auto-detect pattern | final accumulator equals serial fold; **`Stitch::Reduce` selected** (verify via `LOFT_LOG=static` showing no queue store allocated, just per-worker 1-element output stores) |
| `tests/scripts/par_for_break.loft` | `break` early on first match | workers stopped, no further `foo(x)` calls (verify via a counting `foo`) |
| `tests/scripts/par_for_pair_aware.loft` | body uses `x` and `r` together | per-pair correctness: every `(x, r)` pair seen by the body satisfies `r == foo(x)`.  **Order across pairs is not asserted** (per D1c — completion-order delivery; running the test multiple times typically shows order variation, which is direct evidence of parallelism) |
| `tests/scripts/par_call_desugars.loft` | call form `par(ls, foo, 4)` | `LOFT_LOG=static` dump shows synthesized `Value::ParFor` with `Stitch::Concat`; output identical to pre-desugar behaviour byte-for-byte |
| `tests/scripts/par_fold_sum.loft` | `par_fold(xs, 0, |a, b| a + b, 4)` | parallel reduction; result equals serial fold; bench shows ≥ 30 % faster than `par(xs, identity, 4).sum()` two-pass |
| `tests/scripts/par_fold_text_concat.loft` | `par_fold(strings, "", |a, b| a + b, 4)` | result preserves worker-id ordering; equal to serial concat |

Plus four unit tests in `tests/issues.rs`:

- `par_call_preserves_source_span` — diagnostics inside the
  desugared body cite the original `par(...)` call site.
- `par_call_with_method_form` — `par(ls, .my_method, 4)` desugars
  correctly.
- `par_call_with_lambda` — `par(ls, |x| x * 2, 4)` desugars correctly.
- `par_fold_auto_detect_pattern` — fused for-loop with
  `acc = combine(acc, r)` body emits `Stitch::Reduce` opcode;
  body with `acc = combine(acc, r) + 1` (extra computation) does
  NOT match the pattern and falls back to `Stitch::Queue`.

### Documentation

- `LOFT.md` § Control flow: a new "Parallel for-loop" subsection
  leading with the fused form (general primitive); the call form
  introduced afterward as "the value-position shortcut for the
  collect case, with auto-capacity pre-allocation".
- `THREADING.md`: replace the existing "par variants" section with
  one explanation centred on the fused construction.  Add a
  "How `par(...)` desugars" subsection showing the IR.
- `CHANGELOG.md`: user-facing entry framing it as "parallel
  for-loops" — the natural audience-facing name; mention that
  existing `par(input, fn, threads)` callers automatically benefit
  from the new runtime's memory profile.

## Acceptance criteria

- All five new fixtures + three unit tests pass on Linux x86_64,
  macOS aarch64, Windows MSVC.
- Existing `tests/scripts/22-threading.loft` passes byte-for-byte
  after the desugar lands (call form is still expression-positioned
  and produces the same vector).
- The @PLAN06 phase 0 baseline benchmarks rerun within ±5 % of
  baseline (no regression on existing `par(...) -> vector<U>`
  callers — the desugar path is the new hot path for them).
- A new microbench `bench_par_for_no_collect` measures the fused
  form's overhead vs. the call form on bench-1 (1 M `i64`):
  expected savings of ~3 ms and ~8 MB peak memory when the body
  drops `r` instead of collecting.
- Eight-line snippet in `LOFT.md` shows a complete fused for-loop
  par; reads like idiomatic loft.
- Source span fidelity: every diagnostic produced inside a
  desugared `par(...)` call cites the user-written `par(...)`
  source location, not `synthesized:0`.

## Sequencing

Phase 7 lands AFTER @PLAN06 phases 1–5 (the store-typed pipeline +
auto-light).  Phase 6 (cleanup) and phase 7 are independent — they
can land in either order.

Implementation order within phase 7:

1. **7a — parser + IR for the fused form.**  New `Value::ParFor`
   IR node carrying `(input, x_var, r_var, worker_fn, threads,
   body, src_span)`.  Parser recognises the `par(...)` modifier in
   for-loops; parse-error tests for malformed shapes.  Body falls
   through to today's `par(...)` runtime as a stopgap (no perf
   win yet — that arrives in 7b).
2. **7b — runtime queue store.**  `run_parallel_queue` in
   `src/parallel.rs` (or the polymorphic dispatcher's queue policy
   from phase 3).  Codegen routes `Value::ParFor` to the queue
   policy.  Bench shows the expected ~3 ms / 8 MB win on bench-1
   when the body doesn't collect.
3. **7c — desugar the call form + remove `par_light`.**  Three sub-steps:
   - `parse_call` rewrites `par(input, fn, threads)` into
     `Value::Block` containing a `Value::ParFor` with an
     auto-collect body using `vector_with_capacity(len(input))`.
     Source spans threaded through.
   - Delete `parallel_for_light` from `default/01_code.loft`.
   - Rename every `par_light(...)` call site in
     `tests/scripts/`, `tests/docs/`, `lib/`, and any tutorial /
     example files to `par(...)`.  The auto-light heuristic from
     phase 5 produces the same execution profile.

   Existing `tests/scripts/22-threading.loft` proves byte-equivalent
   behaviour; the underlying runtime is now uniform.  After 7c
   lands, `par_light` is not a name in the language — a user
   writing it gets `unknown function: par_light — did you mean
   par?` from the regular parser-side undefined-name diagnostic.
4. **7d — `par_fold` surface + auto-detect pure-fold body.**  Two
   pieces:

   *Surface.* Declare in `default/01_code.loft`:

   ```loft
   pub fn par_fold<T, U>(input: vector<T>, init: U,
                         fold: fn(U, T) -> U,
                         threads: integer) -> U;
   ```

   Lowers to `Value::ParFor` with `Stitch::Reduce { fold_fn }` and
   no body.  `init` is encoded as the `Reduce` policy's seed value
   on the opcode payload (or in a dedicated fixed slot for non-
   primitive `U`).  Document the monoid requirement: `fold` must
   be associative with `init` as identity for parallel results to
   match a serial fold.  Common cases (sum / max / min / boolean
   AND / boolean OR / text concat) qualify.

   *Auto-detect.* Scope analysis recognises a fused for-loop body
   of the shape:

   ```loft
   acc: U = <init>
   for x in xs par(r = foo(x), 4) {
       acc = combine(acc, r)            // or `acc op= r` for op ∈ {+, *, |, &, +=, ...}
   }
   ```

   When the body matches, codegen rewrites to `Stitch::Reduce`
   instead of `Stitch::Queue`.  The user wrote a fused for-loop;
   the compiler emits a parallel reduce.  Both forms (`par_fold(...)`
   directly, or the fused loop matching the pattern) compile to
   identical bytecode.

   Pattern criteria for auto-detect:
   - Body is a single `Set(acc, fold_fn(Var(acc), Var(r)))` or a
     primitive compound assignment (`acc += r` etc.).
   - `acc` is declared immediately before the for-loop, with a
     simple initial value.
   - `acc` is not read or written elsewhere in the surrounding
     scope between declaration and loop.
   - `acc` and the fused for-loop's tail expression match (i.e.
     `acc` is the value the surrounding code expects after the
     loop).

   Conservative — false negatives are fine (user gets queue policy
   instead of reduce; correct output, smaller perf win).  False
   positives must be impossible: any body access outside the
   single accumulator update disqualifies the pattern.

   Bench on `total = par_fold(values, 0, |a, b| a + b, 4)` for
   1 M i64 inputs: expected near-linear speedup vs. serial fold;
   ≥ 30 % faster than today's `par(values, identity, 4).sum()`
   two-pass shape (which allocates 8 MB for the intermediate vector
   that par_fold skips).

5. **7e — doc + CHANGELOG.**  Replace `THREADING.md`'s "par variants"
   section; new `LOFT.md` subsection covering both the fused for-
   loop and `par_fold`; CHANGELOG entry.  CHANGELOG notes:
   - `par(items, fn, threads)` keeps working (now sugar for the
     fused for-loop with auto-collect body).
   - `par_fold(items, init, fold, threads)` is new — single-pass
     parallel reduction with no intermediate vector.
   - `par_light` was an internal flag that no longer exists at the
     user surface; users who hand-typed it receive an "unknown
     function" error and rename to `par`.
   - The fused for-loop with `acc = combine(acc, r)` body is
     auto-detected as a parallel reduce — no surface choice
     needed; same bytecode as `par_fold(...)`.

Each commit lands with `make ci` green.

## Caveats and mitigations

Each caveat below is a real concern surfaced during the design;
the mitigation is the concrete answer that lands in the phase
implementation, not a future "we'll handle this later" promise.

### C1 — Parser ambiguity between modifier and call

**Concern.** `for x in ls par(r = foo(x), 4) { … }` versus
`for x in get_data() { … }` where `get_data()` happens to be named
`par`.  The parser must decide which shape it's looking at without
backtracking through arbitrary expressions.

**Mitigation.** `par` is recognised as the modifier **only** in
the specific position `for ident 'in' expr <here> '(' …`.  The
parser at this point has already consumed the input expression;
the next token decides:
- If the next token is `par` AND the token after is `(` AND the
  token after that is an identifier followed by `=` → parse as
  `par_clause`.
- Else fall through to the brace-block.

The three-token lookahead is bounded; no left-recursion.  Existing
`par(...)` calls at expression position are unaffected because the
modifier rule only activates inside the for-loop header.

**Verification.** Add a parse-error test
(`par_modifier_not_recognised_outside_for`) confirming
`par(input, fn, 4)` outside a for-loop header still parses as a
regular call.

### C2 — Body-in-parent-thread surprise

**Concern.** Users may assume the body parallelises too, leading
to "why is my hashmap insert losing entries?" race-condition bugs.

**Mitigation.**
- LOFT.md's parallel-for subsection leads with the rule in bold:
  **"the body runs sequentially in the parent thread; only the `r =`
  expression is parallel."**
- `THREADING.md` shows the data-flow diagram: workers compute → bounded
  queue → parent body.
- A new lint warning fires when the body contains another `par(...)`
  call (allowed because nested fused loops are valid, but flagged
  for review): `warning: nested par() inside parallel for-loop body;
  the outer body runs sequentially — confirm this is intended`.
- The DAP debugger (LSP.3) labels the parent body's frame as
  "parent (sequential)" and the worker frames as "par worker N" so
  users see the threading model at runtime.

### C3 — `break` deadlock on full queue

**Concern.** Worker holds a result; queue is full; consumer stops
reading because of `break`.  Without care, worker blocks forever.

**Mitigation.** Bounded queue uses an mpsc-style shutdown sentinel
(matching idiomatic rust):
1. `break` in the body sets `shutdown.store(true, Release)`.
2. Each worker's queue-write checks the shutdown flag inside the
   blocking-write retry loop; if set, the worker drops the
   in-flight result and exits cleanly.
3. Parent body drains the queue before the join — pending writes
   are popped and discarded after `break` triggers shutdown, so no
   producer is left blocked.

**Verification.** Fixture `par_for_break_no_deadlock.loft` runs a
4-worker fused loop with a body that breaks after the first result;
asserts every worker exited and the test completes within a 1 s
timeout.

### C4 — Completion-order delivery (resolved by D1c)

**Concern (originally).** In-order delivery serialises slot
writes per input index; a slow worker on element N stalls workers
on element N+1 even if their results are ready.

**Resolution.** Plan-06 DESIGN.md D1c drops the input-order
guarantee entirely.  Workers post results in completion order via
a shared atomic write-cursor; no head-of-line blocking, no
per-input-index synchronisation.

User contract for the fused for-loop body: `(x, r)` pairs arrive
in **completion order**, not input order.  Per-pair correctness
holds (`r == foo(x)`); cross-pair ordering does not.  Documented
in LOFT.md's parallel-for subsection in bold:

> **The body sees `(x, r)` pairs in the order workers finish,
> not the order `x` appears in `input`.**  If you need ordered
> processing, use a regular `for` loop.

This change is the load-bearing perf decision — without it, par
would have to either pre-allocate even chunks (bad for unbalanced
workloads) or serialise stitch-time writes (defeats parallelism).

**Side benefit for testing**: completion-order delivery makes
**run-to-run order variation a direct parallelism proof** (see
phase 8f.2 / DESIGN.md D8.2 for the test gate).

Out of scope for @PLAN06 phase 7; flagged as future work in the
CHANGELOG note.

### C5 — Worker panic mid-iteration

**Concern.** `foo(x)` panics for some `x`; what happens to the
parent body and other workers?

**Mitigation.** Same as today's `par(...)`: the panic is caught at
the join point and propagated to the parent thread, which aborts
the loop.  The body sees no further `(x, r)` pairs after the
panicked element.  Documented in `THREADING.md`'s "panic semantics"
subsection.

**Future improvement.** Once L1 (error recovery) lands, the
worker's result type can become `Result<U, Error>`; the body then
sees the error as a normal value (`if r is Err(e) { … }`) instead
of an aborting panic.  Tracked as a 1.0+ enhancement, not a
phase-7 commitment.

### C6 — Source span fidelity for desugared call form

**Concern.** A type error inside a desugared `par(...)` call
points users at synthesized IR they never wrote, not at the call
site they wrote.

**Mitigation.** `Value::ParFor`'s `src_span` field carries the
original `par(...)` token range.  The diagnostic emitter uses
`Value::source_span()` (the existing accessor for desugared
nodes — already used by format strings and `?? return`) to surface
the user-facing location.

**Verification.** Unit test `par_call_preserves_source_span`
deliberately constructs a `par(ls, broken_fn, 4)` where `broken_fn`
has a wrong-type return; asserts the diagnostic message contains
the file/line of the `par(...)` call, not `synthesized:0`.

### C7 — Method-form (`par(ls, .my_method, 4)`)

**Concern.** Method-bound callables today need `self` injected at
the call site; the fused form's worker expression `r = foo(x)`
doesn't naturally accommodate this.

**Mitigation.** The desugarer inspects the `fn` argument's token
type at parse time:
- Plain identifier → emit `r = foo(x)`.
- `.method_name` (method-ref token) → emit `r = x.method_name()`.
- Lambda → emit `r = (lambda)(x)` (lambdas are callable values).

One extra branch in the desugarer (~10 lines).  All three forms
covered by the existing test matrix in
`tests/scripts/22-threading.loft`; phase 7c adds explicit
desugar-IR fixtures (`par_call_with_method_form`,
`par_call_with_lambda`).

### C8 — Inspecting the desugared IR

**Concern.** Users debugging unexpected behaviour need to see what
the parser produced from their `par(...)` call.

**Mitigation.** The existing `LOFT_LOG=static` dump already prints
post-desugar IR.  Phase 7c ensures the synthesized `Value::ParFor`
carries a `// desugared from: par(ls, foo, 4) at file.loft:42`
comment in the dump, matching the existing convention for format
strings and `?? return`.

### C9 — Synthesised variable name collisions and slot allocation

**Concern.** The desugarer names its accumulator `__par_acc` and
its loop variables `__par_x` / `__par_r`; a user with `__par_acc`
in the enclosing scope would shadow it.  Separately, the
synthesized variables need slot numbers in the enclosing fn's
frame — but the desugarer runs during parse pass 2, before scope
analysis runs.

**Mitigation.** Use the parser's existing fresh-var generator
(`Parser::fresh_var(prefix)`), which (a) produces collision-free
names by appending a monotonic counter, and (b) registers the
new variable in the enclosing `Function::variables` table so
scope analysis allocates a slot for it at its normal pass.  Same
mechanism format-string desugaring already uses for its
temporaries.

See "Where `__par_acc` and friends get their slots" earlier in
this doc for the full ownership story.

### C10 — Empty input vector

**Concern.** `par([], foo, 4)` — desugared body never executes;
queue store never used; need to confirm no allocation goes to
waste.

**Mitigation.** The runtime checks `len(input) == 0` before
spawning workers; returns an empty result store immediately.
Auto-capacity pre-alloc passes 0 to `vector_with_capacity`, which
the existing implementation handles cleanly (no allocation).

**Verification.** Phase-0 characterisation suite already covers
this (`par_empty_input`); it runs against the desugar path
unchanged after 7c lands.

### C11 — Memory overhead of the queue store

**Concern.** The bounded queue allocates `2 × threads ×
size_of<(T, U)>` bytes; for large element types and high thread
counts this could be non-trivial.

**Mitigation.** Quantified bounds:
- 8-byte primitives, 4 threads: 64 bytes peak.
- Struct-heavy workloads (1 KB elements, 16 threads): 32 KB peak.
- Pathological (10 KB elements, 32 threads): 640 KB peak.

All trivial vs. the today-vector's MB-scale peak.  If a user
genuinely needs to tune the queue depth (e.g. extremely uneven
worker latency), a future `par(r = foo(x), 4, queue_depth: 16)`
modifier can be added; not in phase 7's scope.

### C12 — `par_light` removal user impact

**Concern.** `par_light(...)` exists today as a user-visible
function in `default/01_code.loft` and is used in tests/lib.
Removing it could break user programs.

**Mitigation.** The "light-path" execution was always a runtime
optimisation that leaked into the surface because we had no way to
detect it automatically.  After phase 5's auto-light heuristic the
distinction is purely internal — the compiler picks the light
path when the worker doesn't allocate, regardless of what the user
typed.

Phase 7c performs the cleanup in three steps:
1. Delete `parallel_for_light` from `default/01_code.loft`.
2. Rename every internal call site (`tests/scripts/22-threading.loft`,
   any `lib/` examples, any tutorial code) to `par(...)`.
3. Verify the renamed callers produce identical output and the
   auto-light heuristic from phase 5 picks the light path for
   them.

User-program impact: any third-party loft program that hand-typed
`par_light(...)` after 0.9.0 receives a regular parser
`unknown function: par_light` error — the same diagnostic that
fires for any unknown name, with the standard "did you mean par?"
suggestion already in the unknown-fn path.  The fix is a one-token
rename.

This is intentional surface reduction, not a deprecation cycle.
The internal optimisation continues to fire automatically; users
never need to know it exists.

## What this replaces in the @PLAN06 README

The README's phase-7 line currently reads:

> | 7 | (placeholder for variant entry points) |

Replace with:

> | 7 | [07-fused-for-par.md](07-fused-for-par.md) | open | M | Fused `for x in ls par(r = foo(x), 4) { … }` construction.  One primitive covers for_each / fold / collect / iter — body picks the policy.  Replaces the earlier 3-variant idea (`par_for_each` / `par_fold` / `par_iter`). |

## Out of scope

Deferred to follow-ups (post-@PLAN06):

- **Out-of-order delivery** (`par_unordered(...)` modifier).
- **Worker pool reuse across calls** — today every fused for-loop
  spawns its own pool.  A workspace-wide pool is a separate
  optimisation (related to A14 `par_light` and W1.14 WASM workers).
- **Nested fused loops** — semantics are clear (outer body runs in
  parent, inner spawns a sub-pool) but worth a fixture once 7b lands.
- **`yield` in the body** — interacts with CO1 coroutines (1.1+).
  Fused form is forward-compatible: when CO1 lands, body can
  `yield r` and the construction becomes a parallel iterator factory.

## Cross-references

- [README.md](README.md) — @PLAN06 ladder; phase 7 sits at the
  end after the store-stitch runtime + auto-light land.
- [00-baseline-and-bench.md](00-baseline-and-bench.md) — phase 0's
  bench harness gates phase 7's perf claim.
- [../../THREADING.md](../../../THREADING.md) — current par design;
  phase 7d rewrites the "variants" section.
- [../../LOFT.md](../../../LOFT.md) — phase 7d adds a "Parallel
  for-loop" subsection.
- [../../ROADMAP.md § 1.1+ I13](../../../ROADMAP.md) —
  iterator protocol; future-compatible with body `yield`.
