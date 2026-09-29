
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Native Rust Code Generation

Plan for making the existing Rust code generation backend (`src/generation/`) produce
compilable, runnable code. The generated code must produce the same results as the
bytecode interpreter for every loft program.

---

## Goals

### Primary goal
Make `src/generation/` produce Rust source files that compile and run correctly —
producing identical output to the bytecode interpreter for every loft program.

### Interpreter safety invariant
The bytecode interpreter is the production execution engine.  **Every step in this
plan must leave it fully functional.**  Concretely:

1. **`cargo test` must pass after every commit.** All 400+ existing tests exercise
   the bytecode interpreter.  A red test means the interpreter is broken.
2. **Never modify `src/fill.rs` or `src/state/` for native codegen purposes.**
   These files are the interpreter core.  Native codegen is a parallel backend,
   not a replacement.
3. **`default/01_code.loft` templates are shared.**  The `#rust` annotations are
   read by both `src/create.rs` (bytecode → fill.rs) and `src/generation/`
   (native codegen).  Any template change must be validated against both paths:
   - `create.rs` applies `stores.` → `s.database.` before writing fill.rs
   - `generation/` must apply `s.database.` → `stores.` (the inverse) when
     emitting native code
   - Templates that already say `s.database.*` pass through create.rs unchanged
     and **must not be changed to `stores.*`** — that would break fill.rs
4. **New files only.**  Steps that add code (N3: `codegen_runtime.rs`, N6: compile
   test, N7: CLI flag) create new files or add `pub mod` lines.  They do not
   modify interpreter logic.
5. **Test both backends after template changes.**  After any change to
   `default/01_code.loft`:
   - Run `cargo test` (validates bytecode interpreter)
   - Run `make gtest` or equivalent (regenerates fill.rs; confirms templates
     still produce valid operator code)

### Verification checklist (run after every N-step)
```bash
cargo test                              # all interpreter tests pass
cargo clippy --tests -- -D warnings     # no new warnings, including test code
cargo fmt -- --check                    # formatted
```

---

## Optimisation tiers — semantics runs, performance lanes, shipped binaries

Three tiers, and the split is deliberate (owner's rule, 2026-09-08):

| tier | what runs it | prelude | rustc |
|---|---|---|---|
| **semantics** | `--native`, the test runner's native corpus and fixtures | named frames, live/debug channel | `-O` or none — the compile is the cost that matters |
| **performance** | `scripts/native_ratio.sh`, `bench/run_bench.sh`, a consumer's `--native-release` bench | lean | `-C opt-level=3 -C codegen-units=1` |
| **shipped** | `--native-release` programs, every library cdylib (`native_lib.rs`) | lean | `-C opt-level=3 -C codegen-units=1` |

A performance lane measures exactly what a shipped binary gets, and never the semantics
build.  The numbers behind the split, drawing pass consumer lane, 2026-09-08: the lean
tier alone took `hash` 4.7M → 1.2M ns/op, `hair` −55 %, `wide_line` −32 %, `composite`
−16 %, `lock` −13 %, `smooth` −19 %; `codegen-units=1` on top −3–5 % broadly and −15–20 %
on `hash`; opt-level 3 alone and `-C target-cpu=native` moved nothing.  The runtime rlib
itself stays on cargo's default release profile: rebuilt with one codegen unit it moved
neither `lock` nor `hash` (its hot accessors are `#[inline]` already), so the cost to
`make ci`'s build is not paid for a gain that did not appear.  `--lean` remains the way
to strip the tier from a `--native` build; `--html` keeps its named frames (the browser
panic hook's frame block is a pinned contract).

**The tier that does not exist yet (owner, 2026-09-15).**  Every tier above keeps the
null-sentinel and overflow checks that make an integer fault a reported null rather than
a wrapped number (DESIGN_DECISIONS.md C129, C120): the checks are the semantics, and they
stay the default because a starting programmer must never be handed a random-looking
number they cannot debug.  What may come, and is deliberately not built: an OPT-IN
"proven program" tier for a game that has already run fine with the checks on — licensed
by the evidence of its own fault-free runs under the checked tier, never by a
declaration — that emits the plain arithmetic the processor does.  Its shape (owner,
the same day): a RELEASE BUILD PASS FOR GAMES.  A library published to the registry
always carries the checks; a game that has passed extended in-house testing under the
checked build is compiled once more from every source it uses — its libraries recompiled
at that level inside the game's build, never taken as published binaries — into the
less safe artefact that ships to Steam or the browser.  A build, not a switch, and
licensed by the testing's fault-free ledger.  Until it exists, the only way a check is
retired is a PROOF that the value cannot be the sentinel (`@FR-R-Counter`, the range
proofs @PLN157 queues), which is portable by construction and — the point — retires the
check inside a library's ordinary build, so every consumer gets it with the checks on.

## Current State

**Updated 2026-03-23 — Full native test parity achieved.**

`src/generation/` translates the loft IR tree into Rust source files.  The original 6
root-cause error categories (totalling ~1500 errors) are resolved by the completed N-steps.
The `codegen_runtime.rs` module is in place; templates are corrected; stdlib inclusion works.
`src/fill.rs` is now auto-generated: `create.rs::generate_code()` runs `rustfmt` after
writing and the `n9_generated_fill_matches_src` test enforces byte-exact match.

**Test parity (2026-03-23):**
- All 24 `tests/docs/*.loft` files compile and run natively (0 failures).
- All 35 non-error `tests/scripts/*.loft` files compile and run natively (0 failures).
- `loft --tests --native tests/scripts` passes 305 tests across 39 files — identical
  to the interpreter.
- CI (`make ci`) now fails on any native compile or runtime failure.

**Key fixes in 0d15114 (2026-03-23):**
- **Issue #77 (fn-ref dispatch):** Conditional fn-refs like `if flag { fn a } else { fn b }`
  now generate correct match-dispatch arms.  Root cause: `collect_fn_ref_literals` only
  extracted `Int(n)` from direct `Set(var, Int(n))`, missing `Int` inside `If`/`Block`.
  Fix: recursive `collect_int_fn_refs` helper.
- **Issue #80 (LIFO store-free):** Recursive functions caused use-after-free because native
  codegen allocates stores at call time (not pre-allocated like the interpreter).
  Fix: `allocation.rs::free_named` now allows non-LIFO frees by cascading `max` downward;
  `generation/` resets `store_nr` to `u16::MAX` after `OpFreeRef`.
- **Pre-eval extension:** `needs_pre_eval` now covers `Value::Insert` and `Value::Iter`;
  `collect_pre_evals_inner` handles `Value::Return`.

### Loop-invariant vector headers (loft#885)

A loop the emitter can prove writes **no store** derives each vector's
`(store_nr, record, length)` once, immediately before the loop, and reads its elements
against that triple instead of re-deriving all three per element. Worth ~2× on an
indexed-read kernel; the measurement, the design and the reason `rustc` cannot do it for us
are in [PERFORMANCE-history.md § Design: P2](PERFORMANCE-history.md) → *Shipped: the NATIVE half*.

Where it lives: `src/generation/hoist.rs` (the gate), `src/generation/ops/vector_ops.rs`
(the two emitters, both falling through to the `#rust` template when the gate declined),
`Output::begin_vector_hoist` (the prelude), and `vector::vec_header` /
`vector::get_vector_hoisted` / `Stores::vec_get_hoisted_or_raise_runtime` (the runtime).
Interpreter emission is untouched.

Two switches, both read at GENERATION time:

* **`LOFT_HOIST_VERIFY=1`** emits the checking form of every hoisted read — it re-derives
  the header and panics if the loop moved the vector under it. This is the gate on the
  gate; run it over the suite after touching `hoist.rs`.
* **`LOFT_NO_VECTOR_HOIST=1`** emits the pre-loft#885 form, so one binary carries both
  halves of an A/B. It is also the first bisect step for a `--native`-only wrong answer in
  a loop that indexes a vector.

### Constant tables are data, not statements (loft#1697)

rustc's time on a function grows faster than its length, so a table must not reach it as one
statement per element.  Two places emitted exactly that: `emit_const_vectors` wrote four lines
per element of every pre-built constant into `init`, and a literal-bodied table function
pushed its literal one element per line.  A uniform constant table is now one `static` array
per field and one loop (`write_const_columns`, the same `record_new` / set / `record_finish`
per element), and a literal-bodied function copies its constant twin in one append
(`@FR-R-Const`'s body clause, formal/rewrites.md).  On the crawler's `rivers_test` the
emitted Rust fell from 61.7 MB to 4.9 MB and the cold build from over 20 minutes to 16 s.
A new constant shape should keep this property: look at the emitted size of a 10 000-element
case before shipping it.

### Native→interpreter fallback, and `LOFT_REQUIRE_NATIVE` (efficiency-work aid)

A default `loft <file>` run prefers native but **degrades to the interpreter** rather
than failing when native is genuinely unavailable.  This keeps loft turnkey on a box
without a working toolchain, but it can silently mask a performance regression — a
program you *think* runs native is quietly interpreting.  There are exactly two places
native can degrade (both in `src/main.rs::main`):

| Where | Trigger | Default behaviour |
|---|---|---|
| **Auto-native library loop** | a `use`d `compile = "native"` library whose cdylib won't build (`Err`) or is being edited (`Ok(None)` dev-interpret) | the library interprets; `Err` warns, dev-interpret is silent |
| **Main-program `'native` block** | `rustc` absent / mismatched / a stale-rlib toolchain failure on a cache miss | warns, then runs the program on the interpreter |

Set **`LOFT_REQUIRE_NATIVE=1`** (the inverse of `LOFT_NO_NATIVE_LIBS`) to turn **every**
one of those fallbacks into a **hard error that names the reason**, so a performance run
can never silently interpret.  Off by default — the warn-and-interpret behaviour above
is unchanged.  Enforced at one chokepoint per location: the library loop, and a single
post-`'native`-block check (each fallback records *why* in `native_fallback_reason`; the
chokepoint reports it, and a catch-all `unwrap_or` still errors loudly if a future
fallback forgets to record one).  `--check` runs are exempt (they report parse status,
not execution).  Guards: `tests/n3_use_native.rs::require_native_*`.

**`--interpret` is NOT the escape hatch for a library that will not build**, and the
refusal says so.  `--interpret` chooses the interpreter for **your program**; a `use`d
library still builds its cdylib, so a broken library keeps failing under it.  The switch
that makes every library interpret is **`LOFT_NO_NATIVE_LIBS=1`**.  The refusal used to
advise `--interpret`, which sent a blocked reader nowhere — the command that hit it
already was `--interpret` (loft#815).  When you change that message, check the cure by
running it: `LOFT_FORCE_NATIVE_BUILD_FAIL=1` reproduces the refusal on any program.

### Architecture

The generated code uses these loft library types (already public):
- `loft::database::Stores` — runtime data store
- `loft::keys::{DbRef, Str, Key, Content}` — reference and string types
- `loft::ops` — pure scalar operations (arithmetic, conversions)
- `loft::vector` — vector operations

Each generated file contains:
1. An `init(cell: &UnsafeCell<Stores>)` function that registers all type schemas
2. Rust functions for each loft function, receiving `cell: &UnsafeCell<Stores>` as first arg
3. A `#[test]` wrapper that calls `init()` then the test function

Every one of those functions opens with `let stores: &mut Stores = unsafe { &mut *cell.get() };`.
The shared cell is what lets a callee reach the store table while its caller still holds a
reference into it, which ordinary `&mut` aliasing rules forbid and which the runtime needs:
a call can free, claim and move records.  The cost is that `rustc` marks nothing `noalias`,
so no store value survives a call in a register — see
[PERFORMANCE-history.md § Native vs Rust 3d](PERFORMANCE-history.md) for what LLVM could be told instead
and which stores can carry which claim.

#### The type-id correspondence (and how it is checked)

`init()` **replays** the parse-time registration order; it does not read ids
from the compiler. Every type id the generated ops carry — `OpReadFile`'s
`db_tp`, `OpDatabase`'s type, every keyed-collection id — is a plain integer
baked in at compile time. So the emission order is load-bearing: **the type
created Nth by `init()` must be the compiler's type N.** One type created a
position early or late renames every id after it.

`Type::name` is the schema KEY that order is keyed by — `typedef.rs` builds wrapper names
from it and `state/mod.rs` looks stores up by it — so re-spelling how a keyed type RENDERS
(`index<Rec,[("id", true)]>` to the source's `index<Rec[id]>`) re-identifies the type:
`init()` replays a different order and the emitted Rust references a temp no line binds
(rustc E0425, three `lazy_sql_source` failures, two steps from the cause; loft#923). A
user-facing rendering needs a function of its own; `name` is not it.

Nothing used to check this, and the failures it produced named the wrong thing.
A `f#read as u16` returned null because its `db_tp` had come to point at a
struct, while the other widths out of the same handle stayed right; a keyed
lookup aborted with `find called on non-collection type` naming whatever type
sat at the shifted id — a type in a library the program never called (loft#739).

`Stores::verify_schema_ids`, emitted at the end of every `init()`, now compares
the two tables and names a type the program placed at a different id than the
compiler did. It **reports and continues**: some drifts predate the check and
produce correct output today, so aborting would fail working programs. Set
`LOFT_STRICT_SCHEMA_IDS` to make it fatal — that is what you want while hunting
one. It deliberately stays quiet about a name the runtime table lacks entirely
(`db.sorted` registers `sorted<Rec[id]>` for a recorded `ordered<Rec[id]>`,
`db.vector` registers `vector<X>` for `array<X>` — different rendering, same
slot) and about a name the compiler's table holds twice (prelude shadowing),
since neither can witness a move.

Known drifts it currently reports, all producing correct output so far and none
yet run down: a nested narrow-int vector registers `vector<vector<integer>>`
where the compiler recorded `main_vector<vector<integer(-32768, 32767)>>`,
losing the narrow element type and minting an extra type; and `short<0,true>`
lands one late behind the same nested-vector shape.

**A `vector<τ?>` element used to be one of them (loft#923).** `Optional(τ)`
shares τ's storage exactly — the compiler's own table holds `vector<integer?>`
and `vector<integer>` as ONE type — but `emit_field_inner` tested the element
type without peeling the `Optional`, so a `vector<vector<integer>?>` field
missed the nested-vector arm and fell through to the generic path. There
`type_def_nr` resolves an `Optional` to the generic `vector` def and reads
whatever `known_type` was assigned last, so the emitted `db.vector(<that>)`
MINTED a type the program never named and moved every id after it. The peel now
happens once, at the top of the vector arm, beside the peel the FIELD's own type
already had. The lesson generalises: any test on an element type in this emitter
is a question about STORAGE, and `Optional` is not part of the answer.

A keyed collection in that position is a different story — `vector<hash<E[k]>>`
had no element type created at all, so `init()` named a binding no line made and
rustc refused the program. It is now refused where it is declared instead
(loft#923 leg A): nothing could fill one, so there was no working program to
keep. See [DESIGN_DECISIONS.md](DESIGN_DECISIONS.md).

`LOFT_TRACE_MINT=1` is the companion instrument: it narrates every
collection-type lookup as `hit=<nr>` or `MINT=<nr>` with caller frames, so
diffing a working run against a broken one shows the extra mint.

### Per-Op emitter dispatch (plan 09 phase 00)

Every `#rust` template substitution AND every user-fn / Op-stub call
flows through a single dispatch surface in `src/generation/ops/`:

```text
output_call_template(Output, w, def_fn, vals)   ← templates
output_call_user_fn(Output, w, def_fn, vals)    ← user fns / Op stubs
fn-ref dispatch in emit.rs:387                  ← runtime polymorphism
                            ↓
                     emit_op(ctx, name, args)
                            ↓
       custom emitter registered for `name`?
              yes ↓                ↓ no
       custom_emitter.emit()    DefaultEmitter::emit()
                                       ↓
              def_fn.rust.is_empty()?
                yes ↓              ↓ no
       user_fn_call_body()    substitute_template_body()
```

Custom emitters live in `src/generation/ops/<group>.rs` and
implement `OpEmitter::emit(&self, ctx, args)`.  Register them in
`src/generation/ops/mod.rs::build_registry`.

`EmitCtx<'a, 'b>` carries the writer, the Op definition, and a
back-reference to `Output<'b>` (the codegen state).  Custom
emitters call back into `Output` for helpers like
`generate_expr_buf`, `format_long`/`append_text`/…, the field-width /
signedness probes, and the template substitution itself.

**`dispatch.rs::output_call_inner` is now just two steps** — a
registry-first guard (`emit_op` when a custom emitter is registered
for the Op name) and a fallback (`output_call_user_fn` for a user fn,
else `output_call_template` for the `#rust` template).  The monolithic
special-case `match` that used to live between them was eliminated:
every Op-specific native emission is now either a registered
`OpEmitter` (`src/generation/ops/`: `parallel`, `key_ops`, `ref_ops`,
`coroutine`, `int_compare`, `text_ops`, `misc_ops`, …) or a `#rust`
template.  The `text_ops::TextDispatchEmitter` reproduces the @P283
refvar→`Stack` rewrite internally and is registered for the whole
text/format/buffer family.  A regression guard
(`tests/codegen_emitter.rs::dispatch_op_arm_budget_not_exceeded`,
ratchet at 0) fails if a `"Op…" =>` match arm is ever re-introduced.

### An argument may not borrow the store the call already borrowed

Rust evaluates a method call's **receiver place before its arguments**, so a
`#rust` template shaped `stores.method(@count)` holds `&mut *stores` for the
whole of `@count`.  An argument that itself calls a `&mut Stores` method is then
a second mutable borrow, and rustc rejects the entire generated function with
E0499 — two-phase borrows rescue a nested SHARED read and not this.  What
produces such an argument is ordinary loft: `/` and `%` expand to a
divide-by-zero guard, `v[i]` and `s[i]` to a bounds guard, each of which raises
through `&mut Stores`.

**The invariant:** an argument that can borrow the store is evaluated into a
local BEFORE the call takes its own borrow.  Two passes enforce it, split by
what they can see:

| | covers | where |
|---|---|---|
| `pre_eval.rs` | user-fn and Op-stub CALLS (`f(g(x))`, `f(c, h(c.field))`) | @P312, @P199 |
| `calls.rs::substitute_template_body` | `#rust` TEMPLATE arguments | loft#818 |

The template half is the one that kept being missed, because a template's
argument list is a string and no pass was reading it.  It was hand-patched into
individual templates three times — `OpGetVector`'s receiver (@P321d), then its
index (@P338), then `reserve` on a hash (loft#818) — before the third made the
shape legible.  Writing `{let __x = @arg; …}` into a `default/01_code.loft`
template still works and the two earlier ones are still there, but a NEW template
needs nothing: the emitter hoists for it.

Two properties of the hoist worth knowing before changing it:

- **It hoists a PREFIX, not one argument.** A hoisted argument keeps its position
  in evaluation order only if every argument before it is hoisted too; otherwise
  an earlier inline argument runs after a later hoisted one, and two arguments
  that both raise report the wrong error first.
- **A `text` argument blocks it.** Binding one to a local either MOVES a `String`
  out of the caller's frame or borrows a temporary that dies at the end of the
  `let`.  So if a text argument sits before the one that needs hoisting, nothing
  is hoisted and the call fails to compile exactly as it did before — loudly.
  `tests/scripts/818-store-borrow-in-argument.loft` is the matrix; `store_load_key(h,
  p(), n / 2)` is the shape that would show the residual.

The fn-ref dispatch (`emit.rs::output_fn_ref_dispatch`) hoists
arguments into `let _farg_N` Rust bindings before the runtime
match, then routes each candidate arm through `output_call_user_fn`
with synthetic `Value::RawExpr("_farg_N")` arguments.  This means a
custom emitter registered for any candidate target is honoured
even when called via fn-ref.  `Value::RawExpr` is a codegen-only
variant created on the codegen stack; the parser and bytecode
codegen never produce it.

#### How to register a custom emitter

```rust
// src/generation/ops/op_my_op.rs
use super::{EmitCtx, OpEmitter};
use crate::data::Value;
use std::io;

pub struct Emitter;

impl OpEmitter for Emitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        // ctx.w        — the writer (use `write!(ctx.w, …)` for raw text).
        // ctx.def_fn   — the resolved Op definition.
        // ctx.output   — back-reference to the codegen state (`Output`).
        // Prefer ctx.emit(value) over ctx.output.output_code_inner(…)
        // — it forwards to the same method but cuts the reborrow noise
        // (`&mut *ctx.w`).  Same for ctx.emit_i32_slot(value).
        write!(ctx.w, "ops::my_helper(cell, ")?;
        ctx.emit(&args[0])?;
        write!(ctx.w, ", ")?;
        ctx.emit_i32_slot(&args[1])?;
        write!(ctx.w, ")")
    }
}

// In src/generation/ops/mod.rs::build_registry:
//     r.insert("OpMyOp", Box::new(super::op_my_op::Emitter));
```

#### Forwarding-first recipe (verify before writing real emission)

When adding an emitter for an Op for the first time, register a
**forwarding emitter** first (delegate to `DefaultEmitter::emit`)
and verify byte-identical baseline.  Only then replace the body
with real emission logic.  This catches dispatch-path conflicts
before any real code is written.

**Pre-flight check** — does the Op have a special case in
`dispatch.rs::output_call_inner`?

```bash
grep -n '"OpYourOp" =>' src/generation/dispatch.rs
```

- **Empty result** → forwarding is safe.  Register a forwarding
  emitter first; the dispatch path is exercised end-to-end and
  the byte-identical baseline confirms no logic gets bypassed.
- **Hit** → forwarding will SKIP the special-case logic (e.g.
  OpFreeRef's debug-name string + store_nr reset, OpDatabase's
  `var_X = OpDatabase(...)` assignment shape).  Skip the
  forwarding step and write the real emitter directly,
  absorbing whatever the special-case arm does.

The forwarding emitter is written ad-hoc as a one-shot for the
verification pass:

```rust
pub struct Emitter;

impl OpEmitter for Emitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        DefaultEmitter.emit(ctx, args)
    }
}
```

(Plan-09 originally shipped a shared `forwarding_smoke.rs` file
covering 9 forwarded Op names as a registry-dispatch smoke test;
@PLN80 phase 02 retired it once 5 production custom emitters
proved the dispatch path was exercised end-to-end.  The recipe
above is the residual pattern — write a one-shot forwarding
emitter inline when adding a new Op, verify byte-identical
baseline, then swap in the real emission logic.)

Validation:
- `cargo test --release --test codegen_emitter` runs the byte-identical
  baseline guard + @P203 regression guard +
  `pre_eval_walkers_unspan` structural guard (see Walker convention
  below).
- `scripts/p09_fast_gate.sh` is the ~4-second human-driven gate.

#### Walker convention — always `.unspan()` before matching `Value::*`

When implementing a walker that pattern-matches against `Value::*`
variants (`matches!(op, Value::Set(...))`, `match &operators[i]
{ Value::Call(...) => ... }`), call `.unspan()` on the operator
first.  Skipping unspan is **"code that compiled but never
executed"** — the parser commonly wraps operators in
`Value::Span(box (pos, inner))` for source-position tracking, and
a raw match falls through to the `_ =>` arm even when the
unspanned value matches.

Plan-11 closed @P204 by fixing one such walker
(`detect_ref_tail_capture`); @PLN80 phase 01 generalised the
audit and patched 16 walker sites across 3 files (`pre_eval.rs`,
`emit.rs`, `coroutine.rs`).  Findings: all latent — no in-tree
miscompile reproducer, byte-identical baseline preserved — but
applied as insurance.

The structural guard
`tests/codegen_emitter.rs::pre_eval_walkers_unspan` slices
`patch_hoisted_returns` + `value_mentions_var` and asserts every
`matches!(op, Value::*)` site is paired with `.unspan()`.  Add new
walker sites to that slice or extend the guard if you introduce a
new pattern site outside `pre_eval.rs`.

---

The N1–N10 fix steps, N20 (`fill.rs` auto-generation), N21 (one-walk pre-eval), the N10 dependency graph and the "Path to Default" plan are the record in [NATIVE-history.md](NATIVE-history.md).  The identity model that keeps a `#native` package's link and cache coherent is [NATIVE_ARTIFACT_IDENTITY.md](NATIVE_ARTIFACT_IDENTITY.md).

---

## Critical Files

| File | Role |
|------|------|
| `default/01_code.loft` | All `#rust` templates (N1, N5) |
| `src/generation/` | Code emitter (N3–N5) |
| `tests/testing.rs:220–242` | Where generated files are written (N2) |
| `src/fill.rs` | Reference implementations for all 234 opcodes |
| `src/state/io.rs` | Reference for `OpDatabase`, `OpNewRecord`, etc. |
| `src/ops.rs` | Pure operations — already imported by generated code |
| `src/codegen_runtime.rs` | New runtime module (N3) |

---

## Verification

**A rewrite both backends run lands with its switch A/B green.** The interpreter is the
oracle only for a GENERATOR rewrite; one the parser or scope pass applies (the ownership /
buffer family) gives both backends the same answer, right or wrong, so its falsifier is the
switch itself: the corpus and the consumer suites run with it off and on, and any difference
is a defect (`.github/workflows/switch-ab.yml`; the rule is `(R-Switch)`'s both-backend
clause in [formal/rewrites.md](formal/rewrites.md)).

After each step:
1. `cargo test` — existing tests must still pass (bytecode interpreter unaffected)
2. Count remaining compilation errors:
   ```bash
   for f in tests/generated/*.rs; do
     rustc --edition 2024 --crate-type lib "$f" \
       -L target/debug/deps --extern loft=target/debug/libloft.rlib 2>&1
   done | grep "^error\[" | wc -l
   ```
3. After N6: CI gate prevents regressions

---

## N8a — Tuple Native Codegen — **Shipped**

`rust_type(Type::Tuple)` now emits the per-element type list (e.g.
`(i64, f64)` for `(integer, float)`), `Value::TuplePut` writes
`var_{var}.{idx} = <rhs>`, and tuple-returning functions land with the
correct signature.  `SCRIPTS_NATIVE_SKIP` in `tests/native.rs` is
empty — `50-tuples.loft` and `46-caveats.loft` both pass under
`--native`.

---

## N8b — Coroutine Native Codegen

### Current state (N8b.1 + N8b.2 implemented)

Generator functions (`fn foo() -> iterator<T>`) are fully supported in the `--native`
backend for integer/float/boolean/text-param yields.  Each generator is compiled into
a Rust state-machine struct implementing `LoftCoroutine`.  Text-local serialisation at
yield (`CO1.3d`) is not yet implemented; the M8-b `debug_assert!` in `coroutine_yield`
fires if text locals exist at a yield point.

**Key files:**
- `src/generation/coroutine.rs` — state-machine emitter (`output_coroutine`)
- `src/codegen_runtime.rs` — `LoftCoroutine` trait, `NATIVE_COROUTINES` thread-local,
  `alloc_coroutine`, `coroutine_next_i64`, `coroutine_is_exhausted`
- `src/generation/dispatch.rs` — `OpCoroutineNext`, `OpCoroutineExhausted` arms
- `src/generation/mod.rs` — routes generator functions to `output_coroutine`;
  `collect_calls` walks `Value::Yield` nodes; `rust_type(Type::Iterator) = "DbRef"`

### Implemented design: integer state-machine struct

Each coroutine function is transformed into:
1. A Rust `enum` with one variant per yield point plus `Exhausted`.
2. A Rust `struct` wrapping the enum (the opaque generator handle).
3. A `new` associated function (replacing `OpCoroutineCreate` at call sites).
4. A `next` method returning the yield type or a sentinel (replacing `OpCoroutineNext`).

The handle is allocated in a `codegen_runtime` coroutine table and referenced via a `DbRef`
with `store_nr == COROUTINE_STORE` — exactly mirroring the interpreter's convention so that
the same `OpCoroutineNext` call sites work unchanged.

---

### N8b.1 — State-machine transform design + infrastructure (✓ implemented)

The actual implementation uses a simpler `state: u32` integer rather than a Rust `enum`
with variant fields.  All function parameters are stored as struct fields; the state integer
selects the match arm on each `next_i64` call.

For `fn count() -> iterator<integer>` (3 yields of 10, 20, 30):

```rust
struct NCountGen {
    state: u32,
}

impl loft::codegen_runtime::LoftCoroutine for NCountGen {
    fn next_i64(&mut self, stores: &mut Stores) -> i64 {
        match self.state {
            0 => { self.state = 1; return (10_i32) as i64; }
            1 => { self.state = 2; return (20_i32) as i64; }
            2 => { self.state = 3; return (30_i32) as i64; }
            _ => loft::codegen_runtime::COROUTINE_EXHAUSTED,
        }
    }
}

fn n_count(stores: &mut Stores) -> Box<dyn loft::codegen_runtime::LoftCoroutine> {
    let _ = stores;
    Box::new(NCountGen { state: 0 })
}
```

`COROUTINE_EXHAUSTED = i32::MIN as i64` — when cast to `i32` this equals `i32::MIN`,
which is loft's null sentinel for integers.  `op_conv_bool_from_int(v) = (v != i32::MIN)`,
so the for-loop condition becomes false and the loop exits.

Thread-local storage avoids modifying `Stores`:

```rust
std::thread_local! {
    static NATIVE_COROUTINES: std::cell::RefCell<Vec<Option<Box<dyn LoftCoroutine>>>> = …;
}

pub fn alloc_coroutine(coro: Box<dyn LoftCoroutine>) -> DbRef { … }
pub fn coroutine_next_i64(gen_ref: DbRef, stores: &mut Stores) -> i64 { … }
pub fn coroutine_is_exhausted(gen_ref: DbRef) -> bool { … }
```

The returned `DbRef` has `store_nr = NATIVE_COROUTINE_STORE = 0xFFFD`, `rec = vec_index`.

---

### N8b.2 — Basic coroutine emission (integer/float/bool yields, no text) (✓ implemented)

Detection: `if matches!(def.returned, Type::Iterator(_, _))` in `output_function()` routes
to `output_coroutine(w, def_nr)` instead of the normal function emitter.

Call sites: `output_call_user_fn` detects `is_generator` and wraps with
`loft::codegen_runtime::alloc_coroutine(foo(stores, args))`.

`OpCoroutineNext` in `dispatch.rs` emits
`loft::codegen_runtime::coroutine_next_i64(gen_code, stores)` with a cast to `i32` for
integer generators.

`collect_calls` in `mod.rs` now walks `Value::Yield(inner)` nodes so helper functions
called from yield expressions are reachable.

**Test:** `tests/scripts/51-coroutines.loft` passes fully in `native_scripts`.

---

### N8b.3 — `yield from` delegation

`yield from inner()` desugars in the interpreter to: loop, call `next(inner)`, if not exhausted
`yield` the result, else break.  In the state machine, this introduces a sub-generator field.

**Generated pattern:**

```rust
NCountState::YieldFrom_1 { sub_gen, outer_locals... } => {
    // sub_gen implements LoftCoroutine
    let val = sub_gen.advance_i64();
    if val == i64::MIN {
        // sub-generator exhausted — transition to post-yield-from state
        self.state = NCountState::S2 { outer_locals... };
        continue; // loop to process S2 immediately
    }
    // sub-generator still live — stay in YieldFrom_1 with updated sub_gen
    self.state = NCountState::YieldFrom_1 { sub_gen, outer_locals... };
    return val;
}
```

The sub-generator type is `Box<dyn LoftCoroutine>` (to handle heterogeneous inner generators).

**Steps:**
1. Detect `Value::YieldFrom` in `scan_yield_points`; record it as a `YieldFromPoint`.
2. In the state enum, emit `YieldFrom_N { sub_gen: Box<dyn LoftCoroutine>, live_vars... }`.
3. In `next()`, emit the arm as shown above.
4. At the `yield from` call site, emit `alloc_coroutine(...)` for the inner generator and
   store it in the `YieldFrom_N` variant.

**Tests after N8b.3:**
- Remove `"51-coroutines.loft"` from `SCRIPTS_NATIVE_SKIP`.
- Full coroutine test suite passes in `--native` (text-yield tests may still be guarded
  pending S25).

---

## N8c — Generic Function Instantiation

### Current state

Generic functions (`fn f<T>`) are **monomorphized at the bytecode IR phase** in
`src/parser/mod.rs::try_generic_instantiation()`.  Each distinct binding produces a
`DefType::Function` keyed `i_<len><types>_<template key>` (`D-Key`, @PLN165 — e.g.
`i_7integer_n_identity`, `i_4text_n_identity`); the native emitter flattens a key's
non-identifier characters and refuses two keys that flatten to one identifier.
`LOFT_NO_INSTANCE_KEY=1` restores the older method-shaped `t_<len><type>_<name>` key.

By the time native codegen runs, all generic functions have been replaced by concrete
functions.  Native codegen does not need to implement polymorphism — it only needs to
correctly emit the monomorphized instantiations.

The skip reason in `tests/native.rs` — "P5: native codegen does not handle generic function
instantiation" — means that **some monomorphized instantiations produce compile errors**, not
that generics themselves are unsupported at the codegen level.

### N8c.1 — Audit: which instantiations fail and why

**Test file:** `tests/scripts/48-generics.loft`

Instantiations created by the test:

| Call | Monomorphized name | Return type | Expected issue |
|---|---|---|---|
| `identity(42)` | `i_7integer_n_identity` | `integer` | Likely OK |
| `identity(3.14)` | `i_5float_n_identity` | `float` | Likely OK |
| `identity("hello")` | `i_4text_n_identity` | `text` | **Likely fails** — text-return wrapping |
| `identity(true)` | `i_7boolean_n_identity` | `boolean` | Likely OK |
| `pick_second(1, 99)` | `i_7integer_n_pick_second` | `integer` | Likely OK |
| `pick_second("a", "b")` | `i_4text_n_pick_second` | `text` | **Likely fails** — same |

**Audit procedure:**
1. Temporarily remove `"48-generics.loft"` from `SCRIPTS_NATIVE_SKIP`.
2. Run `cargo test --test native 2>&1 | head -80` to capture compile errors.
3. Open the generated `.rs` file for the failing test and inspect the emitted bodies of
   `i_4text_n_identity` and `i_4text_n_pick_second`.
4. Compare with a hand-written native text-returning function to identify the difference.

**Expected finding:** Text-returning monomorphized functions lack the `Str::new(...)` return
wrapping that `output_function()` applies only when `def.returned == Type::Text(_)`.  The
wrapping logic reads the `returned` field of the definition; for monomorphized functions this
field should hold `Type::Text(...)` after substitution, so the wrapping should apply.  The
actual failure may instead be in how text *parameters* are passed (the `Str` vs `String`
boundary) or in how the substituted function body calls `output_code_inner`.

Record the exact error message and line in `NATIVE.md § N8c.1 findings` before writing N8c.2.

### N8c.2 — Fix

Based on N8c.1 audit findings (to be filled in after the audit):

**If the issue is text-return wrapping:** Ensure `output_function()` checks `def.returned` for
`Type::Text` on all functions, including `t_*`-named monomorphized ones.  The check should
already be generic (not name-specific), so this may point to a type-substitution bug in the
parser's `try_generic_instantiation` where `returned` is not correctly updated.

**If the issue is text-parameter handling:** In `rust_type(Type::Text(_), Context::Argument)`,
verify that `Str` (borrowed reference) is emitted for text parameters of monomorphized
functions, matching the convention used for hand-written functions.

**If the issue is call-site argument type:** Ensure the call-site emission for
`t_4text_identity(stores, arg)` passes `arg` as `&*var_arg` (a `Str` borrow) rather than
a `String` move.

**Cleanup after fix:**
- Remove `"48-generics.loft"` from `SCRIPTS_NATIVE_SKIP`.
- `cargo test --test native` confirms the generic tests pass.

---

## N9 — native-library shared-store dispatch (C71)

**Shipped + live end-to-end.**  An interpreted script that `use`s a library
auto-compiles the library's native-compilable subgraph to a cdylib and dispatches
to it over the **shared store** (`*mut Stores` by pointer, the `LibArg` uniform
slot), interpreting the rest — byte-identical to the all-interpreted run.  The
decision is automatic and invisible (`use <lib>` is native; `LOFT_NO_NATIVE_LIBS`
opts out), with a dev-interpret-on-edit fallback so an actively-edited library never
pays `rustc` per save.  Full build record + design:
[@PLN11 Arc N](plans/11-data-as-store/README.md#arc-n--native-library-execution-model-c71-build-out).

Shipped pieces: `generate_shared_cdylib_lib_rs` + `shared_bridge_wrapper` (the
per-export `LibArg` bridge); `wire_shared_native_fns` (post-load dlsym wiring);
`native_gate::native_compilable` (the maximal native subgraph — transitive,
exhaustive, denylist = concurrency constructs only); `mark_library_native` +
`build_shared_cdylib` (the `use`-flow build); the source-form `generate_interface`
(a consumer dispatches without redefinition); per-artifact fingerprint cache.
Soundness: the mixed interp+native boundary is parity-checked (`tests/n3_parity.rs`,
interp≡mixed≡native) and arms the Goal-E store guard; the one open soundness leg
(ASan on the cdylib) is tracked in the sanitizer plan, not here.

**macOS `dlopen`-cache trap (loft#777).** macOS dyld caches a loaded image BY PATH
for the process lifetime, and its `dlclose` is a no-op — so a second `dlopen` of the
same path returns the FIRST image even after the file was rebuilt underneath it.
`cached_or_build_shared_cdylib` therefore must never `dlopen` an auto-native artifact
for INSPECTION at a path a rebuild-and-load can follow at: the settling run would
execute the stale pre-edit copy while writing the fresh one for next time (a `base`
edit reaching the interpret run but not the native run — the dependent kept serving
its inlined pre-edit copy). Linux keys `dlopen` on (dev,inode) and loads the new
file, so only macOS is affected.

The layout-adoption probe (`artifact_matches_layout`, which opens the artifact to
read its `LAYOUT_FP_SYMBOL`) therefore asks its question through a RELOCATED
artifact — hard-linked, or copied when linking fails, into a throwaway
`.layout-probe-<pid>-<seq>/` — and never at the artifact's own path.  There is one
entry point, because which variant a call site picked used to be the difference
between a correct answer and a silently stale one.

**A degraded probe must not degrade to the answer a fix removed (loft#999).** The
relocation used to fall back to probing IN PLACE when the copy could not be made,
reasoning that a failed temp file beats answering "declares no layout" — which means
ADOPT.  Both of those are wrong; the third answer is *cannot adopt*, which rebuilds.
In place, one unlucky `copy` — a full disk, an exhausted descriptor limit, a
concurrent prune — silently restored the pre-#777 behaviour, and no later run cleared
it.  That is why the #777 guard failed on `macos-latest` twice in a fortnight and
passed the other eleven times: how a probe answers must not depend on whether a temp
file could be written.  Two rules came out of it:

- **A fallback must never be the behaviour a fix removed.**  Degrade toward the
  expensive-but-correct answer (rebuild), never toward the cheap one that was the bug.
  A silent fallback is the same defect twice: the wrong answer, and no way to see it.
  A relocation that fails now says so once per process and names the cure.
- **Relocation that costs nothing cannot fail for lack of room.**  The probe
  hard-links first (O(1), no space, no bytes read), so the conditions that made a
  copy fail no longer decide whether an artifact is adopted; the byte copy is the
  fallback for a filesystem without links.  A link shares the artifact's inode, which
  is safe under both keying schemes — the rebuild publishes a NEW inode by rename, and
  where the caller adopts instead, the bytes are identical anyway.

Guards: `tests/n3_parity.rs::a_dependency_edit_invalidates_its_dependents_cdylib` is
the end-to-end #777 shape; `a_dependency_edit_reaches_a_dependent_when_the_layout_probe_cannot_relocate`
and `an_unrelocatable_layout_probe_rebuilds_instead_of_adopting` set
`LOFT_FORCE_PROBE_RELOCATE_FAIL=1`, which makes both relocation legs fail and so turns
a macOS-only coin-flip into a deterministic test on every platform.

**Probe before marking (loft#831).**  Marking a function for cdylib dispatch is a
commitment: `byte_code` emits `OpStaticCall` to its bridge symbol, and if nothing
wires that symbol the call reaches the `compile.rs` panic stub and takes the
program down.  The commitment used to be made on the strength of the **build**
succeeding, which is a proxy — a cdylib can build cleanly and still be one this
process cannot dispatch through: linked against a different `libloft.rlib`,
missing a system library at load, or replaced by a concurrent `loft` between the
freshness check and the load.  Every freshness test in
`cached_or_build_shared_cdylib` (exists, newer than sources, declares this type
layout) can be true of such an artifact; an artifact that declares no layout at
all is adopted outright, since that is what a hand-written cdylib looks like.

`probe_and_mark_exports` therefore asks the question whose answer actually
matters, while the answer is still actionable: `dlopen` the artifact, `dlsym`
each bridge, and mark only what resolves.  A symbol that does not resolve leaves
its function **interpreting** — which was always the documented fallback for a
library that cannot compile native, now reached by the path that needed it most.
The marking can come out partial, and that is correct: the exports that resolve
dispatch native in the same run as the ones that do not.

Loading there is also what makes the decision STAY true.  The handle is kept for
the process, so `prune_artifacts` deleting the file or another process rebuilding
it afterwards cannot change what this one dispatches to — the earlier
open-and-close layout probe left a window in which exactly that could happen.
This is why crawler's 88-test suite lost a *different* test on each parallel run
while passing serially: independent `loft` processes share
`<pkg>/native-auto/`, and the loser of the race got the panic stub instead of the
interpreter.  A fallback is reported once per library with a count, not once per
function — the program's results are unchanged, only slower.  Under
`LOFT_REQUIRE_NATIVE=1` it is a hard error naming itself, like every other
native→interpreter degrade.  Guards:
`tests/n3_use_native.rs::an_unwirable_cdylib_interprets_instead_of_panicking`
and `::a_partially_exporting_cdylib_marks_only_what_resolves`.

**The artifact sweep owns one family, not the directory (loft#831, residual).**
`prune_artifacts` bounds `native-auto/` at `KEEP_ARTIFACTS` by mtime, and it used
to consider every `.so` there.  The directory is not exclusively the auto-native
builder's: a package with a `[c] shim` builds `<pkg>_shim_<key>.so` into it, and
that library is content-keyed and built exactly ONCE — so it is permanently the
oldest file, and an age-ordered sweep takes it first.  Pruning an auto-native
artifact costs a rebuild; pruning the shim removes the only definition of the
package's `#c` symbols, and the next run dies with *"`#c` symbol 'x' not found …
or check the spelling"*, naming neither the library nor the sweep.  There is no
fallback available for that one — a `#c` binding IS the implementation, so
nothing can interpret in its place.  The sweep now takes only the
`loft_auto_<pkg>_` family it built.  Reproduced deterministically against
`tests/fixtures/sqldb/sqlite` (saturate the directory, run once, shim gone, exit
101) and guarded by
`tests/n3_use_native.rs::a_foreign_library_in_native_auto_survives_pruning`.
Parallel runs are what saturate the directory, since each distinct type-layout
context mints an artifact of its own — this is the other half of why a suite
loses a *different* test on each parallel run while passing serially.

**Seeing and collecting the caches — `loft cache` (loft#861).**  The sweep above
runs only after a successful build *in that directory*, so a package that stopped
being rebuilt keeps whatever tail it had, and nothing ever looked at
`~/.loft/build-cache` at all.  Ten GB accumulated on one box with no command to
show it and no command to reduce it; the documented remedy was a hand-typed
`rm -rf`, which also takes the LIVE generation (545 s of rustc CPU to rebuild, on
one measured project gate).

* `loft cache status` — the footprint per area, and what is reclaimable.  Read-only,
  so it is also the dry run for `prune`.
* `loft cache prune` — drop it.  `--all` takes the live generation too.

The two areas are decided differently, and the report says which is which:

| area | test | certainty |
|---|---|---|
| `build-cache/<pkg>-<ver>/` | `release/.loft-build-fp` ≠ this loft's `native_artifact_cache_key()` | **exact** — the same comparison the cache lookup makes, so the tree can never be selected again |
| `registry/*/native-auto/` | beyond `KEEP_ARTIFACTS` newest per family | **conservative** — the artifact name folds the consumer's `layout_fp`, of which there is an open set, so reachability is not decidable from the name |

An **age bound was tried and removed**: "older than the running loft binary" reads
sound, and is how the issue measured the problem, but the reference point is the
binary you invoked — a freshly built one dates *everything* and the tool reports
100 % reclaimable, which is the `rm -rf` it exists to replace wearing a
measurement's clothes.

For the same reason `prune` **refuses to run from a binary that is not the installed
`loft`** (`--force` overrides).  A development build has its own `BUILD_ID`, so it
correctly judges the installed loft's live generation unusable *by itself* — and
deleting it costs every project on the machine a cold rebuild.  Measured while
building this: a dev binary called 49 of 50 build trees reclaimable, the installed
one 0.

### Open completeness items

All are *enhancements* on a complete, graceful core: a construct the dispatch can't
yet cross silently **interprets** (correct, just not native-accelerated), so none is
a correctness gap.

- **Closures across the boundary** (`__closure` param).  A native-library fn taking
  or returning a closure interprets today (the `CallRef`/closure path is
  conservatively excluded from `native_compilable`).  Crossing it native needs the
  `__closure` record to marshal over the `LibArg` bridge.
- **`generate_interface` aggregate type-name rendering** (`sorted<Item[k]>`).  The
  source-form interface renders scalar/struct/enum/vector type names but not keyed
  aggregate type names; a consumer of a library exposing `sorted<…>` in a public
  signature falls back to redefinition.
- **D2a — binary schema interface.**  The robust successor to the source-form
  `generate_interface`: a binary type-schema blob so type ids agree without
  re-parsing the library's loft source (the source form re-parses).  Most valuable
  once libraries are distributed (ties to the registry / package format).
- **`hash` / `index` / `spatial` cross.**  Same verified `DbRef`-over-shared-store
  path as vectors/structs, **untested** — add coverage; no new mechanism expected.
- **Gate-driven dispatch (N4 tail).**  Select the subgraph from `native_compilable`
  directly (currently the simpler per-fn `CallRef`/`parallel` split).  Widens which
  fns go native; user-facing behaviour unchanged.  Making concurrency constructs
  themselves native is the only later optional item and is tiny.
- **Background build (N3 polish).**  The first run after editing settles does a
  foreground `rustc` rebuild; move it to the background so even the settling run
  never blocks.

## Android target

**@PLN106 — SHIPPED 2026-07-15.** `loft --native-android [out.apk|out.so] prog.loft` cross-compiles the **same** target-agnostic
generated core to Android — a build target is a **descriptor over one core**, not a codegen
fork. All Android knowledge lives in `src/android.rs` (the `AndroidTarget` descriptor +
`AndroidSdk`): it wraps the unchanged `output_native_reachable` emit in a generated cargo crate
(program as crate root + a fixed `android_main` via the `android-activity` NativeActivity
feature), cross-builds a lean `loft` rlib (fingerprint-keyed, isolated `target/loft/android/`),
then packages a signed APK (aapt2 → `jar` → zipalign → apksigner, per-tree debug keystore) or
emits the bare `.so`. Env: `ANDROID_NDK_HOME`, `ANDROID_HOME`, `JAVA_HOME`;
`LOFT_ANDROID_TARGET` (default `aarch64-linux-android`; use `x86_64-linux-android` for the KVM
emulator), `LOFT_ANDROID_API` (default 24). Needs a `fn main`.

⚠ **That backend is FIXTURE-ONLY.** Graphics/input/audio run through the cfg-gated Android
backend at `tests/fixtures/libs/graphics/native/src/android_gl.rs`, which exists on **no branch**
of the canonical `loft-libs-graphics` — so an **installed** `graphics` cannot target Android,
and @PLN106's goldens prove the fixture rather than the published package. Flow-back filed as
[loft-libs-graphics#32](https://github.com/loft-lang/loft-libs-graphics/issues/32); the fixture
is pinned at `graphics-v0.1.1` against a `main` at v0.5.5, so it needs a rebase forward, not a
copy. What that backend does
(`native/src/android_gl.rs`): raw EGL/GLES-3.0 on `app.native_window()` (GLES 3.0 = WebGL2, so
website GL programs run unchanged) + android-activity's event pump. An UNCHANGED loft program
gets: rendering (clear/shapes/shaders/text), **touch** (feeds `gl_mouse_*` from `MotionEvent`s),
**keyboard/IME** (`gl_show_keyboard()` + `gl_key_pressed` from `KeyEvent`s), and **audio**
(oboe/AAudio via `audio_play_raw`). Two Android-specific runtime seams: `__run` runs INLINE on
the `android_main` ALooper thread (the graphics poll must own it — see `generation/mod.rs`), and
a `.init_array` constructor sets `RUST_MIN_STACK` to `NATIVE_MAIN_STACK` (512 MiB) so the
call-depth guard's stack assumption holds (that thread is a default-stack `std::thread::spawn`).
Vector-arg native fns (`gl_set_mat4`, `audio_play_raw`, …) export their store-aware `n_*` under
the raw C-ABI symbol on Android (the `--native` C-ABI marshals `(LoftStore, LoftRef)`, not
`(ptr, count)`). Full history + on-device goldens: [plans/106-android-build-target/](plans/106-android-build-target/README.md).

## See also
- [PERFORMANCE.md](PERFORMANCE.md) — Benchmark results and detailed designs for O4 (direct collection emit), O5 (pure function `stores` omission), O6 (`long` sentinel removal) — the native-codegen performance items
- [COMPILER.md](COMPILER.md) — Compiler pipeline: lexer, parser, IR, bytecode
- [INTERMEDIATE.md](INTERMEDIATE.md) — IR Value tree structure
- [DESIGN.md](DESIGN.md) — Algorithm analysis for major subsystems
- [DATABASE.md](DATABASE.md) — Runtime data store and type schema
- [COROUTINE.md](COROUTINE.md) — Interpreter coroutine design; CO1.3d text serialisation (S25)
- [THREADING_SAFETY_COROUTINES.md](THREADING_SAFETY_COROUTINES.md) — Safety analysis for coroutine text handling (P2-R1/R2/R3)

---

## Open work

`--native` is production (CI-gated, all 108/108 native tests pass).
The items below are remaining follow-ups that don't affect the
shipped state.  Each row links to its design content above.

| Item | Section | Status |
|---|---|---|
| **Text branch delivery — tail + bind** (was an accept/reject divergence) | this row | **✅ FIXED 2026-07-10, both sites.**  A branch producing `text` — `if`, `match`, or a `??` value-block — must deliver PER ARM into a destination; used as an expression, each arm emits `&*(callee(…))`, a borrow of the `Str` temporary the callee returned, and that temporary dies at the arm's `}`.  The interpreter, which keeps the value on its stack, was correct throughout.  **Trigger** is a callee needing a caller-provided destination buffer (a COMPUTED text body); a literal-bodied callee returns an owned `String` and never diverged, with or without params.  **Two masks, one cause:** scalar-subject `match` → `E0716`; `if` → `E0308`.  A TEXT-subject `match` passed only *by accident* — freeing the subject copy emits an `OpFreeText` after the value, incidentally tripping `has_trailing_void` in `generation::emit`, which materialised the block; the subject's type has nothing to do with the arm temporary's lifetime.  **Fixes:** TAIL — `if_tail_yields_text` now sees through the `scalar_match` block (the `ncc`/`ncr` see-through pattern).  BIND — `Parser::try_branch_text_bind`, read from both `operators::assign_text` and `expressions::append_to_text`; the leading `Set(q, "")` is load-bearing (without it the per-arm sets live inside the branch, `q` is never introduced, and the interpreter silently reads an EMPTY text — a wrong ANSWER, worse than the reject).  **Do NOT fix this emit-side:** widening `has_trailing_void` to any branch-valued text block was tried and reverted — it materialises an inner block that is itself an if-ARM, so that arm yields `String` while its sibling yields `&str` (`tests/docs/29-match.loft`, E0308).  Guards: `tests/scripts/536-text-match-tail-buffer-callee.loft`; oracle cells `27-native-tailcall-return-heap.loft` (tail) + `30-text-branch-bind-delivery.loft` (bind). |
| **`-> text?` branch tail → native `E0308`** (interp correct) | this row | **FIXED (loft#741).**  `fn g(k) -> text? { match k { 0 => { c0() }, _ => { "z{n}" } } }` — and the `if` and `null`-arm twins — compile-rejected on native while the interpreter was correct.  The per-arm accumulator (`do_if_acc`) read the TAIL's nullability, which conflated two different stores and excluded both: `-> text?` with a nullable tail is a nullable value reaching a nullable destination (fine), while `-> text` with one is a nullable-into-non-null `(N-Store)`.  Excluding the first cost every `-> text?` branch tail its per-arm delivery, so the `match` compiled as ONE Rust expression whose arms must unify — a buffered call yields `Str`, a formatted string `&String`.  The condition now reads the DECLARED RETURN, and the accumulator carries that nullability rather than claiming non-null while holding a null.  An earlier note here concluded per-arm delivery *could not* represent a null arm; it can — loft carries a null text as a content sentinel, which survives the buffer write.  The `-> text` half keeps reporting its N-Store (`tests/runtime_warnings.rs`), which is the other half of the fix.  Cells: `tests/scripts/741-nullable-text-branch-tail.loft`. |
| **@P321c** — `imaging` native ABI gap | [PROBLEMS.md @P321c](PROBLEMS.md) | Open (diagnosed, needs design, M+).  Native direct-call ABI cannot pass a `LoftStore` to a store-mutating `#native` fn (`load_png` decodes + allocates into the Image struct).  `output_native_direct_call` (`src/generation/mod.rs:2181`) has no struct-ref marshalling.  Recommended fix: route through `codegen_runtime + Abi::Cell` (crypto pattern).  16/17 library packages native-green; only `imaging` remains in `LIB_PKGS_NATIVE_SKIP`. |
| **@PLN26 ph.1** — same-symbol cross-package `#native` collision (**full fix DEFERRED — idea parked here; [#388](https://github.com/loft-lang/loft/issues/388) closed not-planned**) | [@PLN26](https://github.com/loft-lang/plans/issues/26) (closed) + [§ Resolution](NATIVE_ARTIFACT_IDENTITY.md#resolution-separate-the-api-id-from-the-rust-part-link-the-cdylib-by-c-abi) | **MVP shipped (`4004424b`+`027d187b`); full fix deferred — this row IS the parked idea (reopen #388 when the trigger below fires).**  Two `[native] crate` packages exporting the SAME `#native` symbol can't be disambiguated across the flat C-ABI namespace (link = first-`.so`-wins), nor the interpreter's symbol-keyed `BRIDGE_REGISTRY` (last-loaded-wins) — a **pre-existing both-backend** hazard, not a C-ABI regression.  MVP: native codegen rejects a **reachable** call to such a symbol with a "rename one" `compile_error!` (`Data::native_symbol_collisions` → `Output.native_collisions`, reachability-scoped so two packages sharing an *unused* symbol still build); `--interpret` keeps its existing silent behavior (guard is native-only by design).  **Deferred full fix:** per-package symbol prefix so they coexist — must change the cdylib export (loft-ffi-macros / `loft generate`), the interpreter registry + dispatch, and codegen *in lockstep*; only needed to CALL a symbol two **un-renameable** packages both export.  Repro/guards: `tests/lib/collide_{a,b,main,unused}` + `native_symbol_collision_across_packages_detected`. |
| **@PLN26 ph.2** — library-cdylib + native package (**✅ DONE → Part 2 of [#389](https://github.com/loft-lang/loft/issues/389)**) | [@PLN26](https://github.com/loft-lang/plans/issues/26) (closed) + [#389](https://github.com/loft-lang/loft/issues/389) | **Implemented + verified.**  A shared-store library cdylib that uses a `[native] crate` package now links it by C-ABI, exactly as the exec path does: `emit_program` sets `native_cabi` (the cdylib emits the package's fns as `extern "C"` `#[link_name]` decls — NOT `extern crate`), and `build_shared_cdylib` adds the `--extern loft_ffi` rlib + each package's resolved `.so` (`-L native`/`-l dylib`/RPATH via `extensions`-resolved `native_pkg_cabi_link_args`).  The sealed `.so` lifts the duplicate-`loft_register_v1` 2-package limit.  `LOFT_NATIVE_CABI=0` still refuses the combo loudly (the legacy rlib link can't take two `loft_ffi` rlibs into one cdylib).  Verified end-to-end: hex_grid's cdylib builds with graphics in the program; a library calling `graphics::save_png` links + runs the native (PNG written), `__cabi_loft_save_png` resolving via the cdylib's RUNPATH.  Regression: `shared_cdylib_with_native_package_emits_cabi_extern` (`tests/n2_cdylib.rs`).  Separately, the `viewer_markdown` collision is the cdylib's OWN raw `*mut Stores` → the `LoftStore`-handle decoupling (**Part 1 of #389 — now tracked as [STABILITY_HOTSPOTS.md § H9](STABILITY_HOTSPOTS.md#h9--raw-mut-stores-across-the-shared-store-cdylibhost-bridge); #389 closed**), NOT native-package linking. |
| **@PLN26 ph.3** — native package → wasm (**✅ DONE → [#438](https://github.com/loft-lang/loft/issues/438)**) | [@PLN26](https://github.com/loft-lang/plans/issues/26) | **Implemented + verified.**  A program that uses a `[native] crate` package now compiles to wasm: `extensions::auto_build_native_target` CROSS-BUILDS the package's native crate to the wasm target on demand (`cargo build --release --target <t>`, clean flags — a `#native` crate links the source-stable loft-ffi C-ABI, not loft's rlib, so no host-SVH flag-matching), into the IN-TREE `native/target/<t>/release/lib<stem>.rlib` the linker reads.  wasm links the **rlib** statically (no C-ABI `.so`), so `add_native_extern_flags` (`native_utils.rs`) also adds the package's HOST proc-macro deps (`native/target/release/deps`, where cargo builds e.g. `loft-ffi-macros` even under `--target`) — without it `extern crate <pkg>` fails `E0463` on the proc-macro.  Best-effort: a missing toolchain/target or a non-wasm-clean crate falls back to the clear "no wasm build" notice (no bare `E0463`).  Verified end-to-end: a program calling `native_scalar_pkg::native_answer()` → `loft --native-wasm` → cross-build → `wasmtime` prints `42` (`pln26_phase3_native_package_runs_on_wasm`, `tests/html_wasm.rs`); the `--html` leg (`wasm32-unknown-unknown`) cross-builds the same way; a shipped `prebuilt/<t>/` rlib is still honoured first.  **SVH note (the original ph.3 concern):** the StableCrateId collision is now *realizable* (packages have wasm rlibs), but a COLLISION still needs TWO packages with colliding wasm rlibs on a shared dep — isolate via `-Cmetadata` / rlib-identity when that first fires (NOT C-ABI; wasm links statically). |
| **@PLN26 ph.4** — Windows C-ABI link path (**✅ FLIPPED — C-ABI is the default on every host**) | [@PLN26](https://github.com/loft-lang/plans/issues/26) | **Done + CI-verified.**  Windows links a DLL through its import library; `-l dylib=<stem>` makes MSVC link.exe open `<stem>.lib`, but a Rust cdylib's import lib is named `<stem>.dll.lib`, so the arm copies `<stem>.dll.lib` → `<stem>.lib` beside it.  **No RPATH** (the MSVC linker rejects `-Wl,-rpath`; the loader finds the DLL beside the `.exe` / on `PATH`), so the DLL is **staged beside the binary** (`native_utils::stage_native_dlls`) — the Windows form of the `$ORIGIN` rpath.  `native_cabi_enabled()` now returns `true` everywhere; **`LOFT_NATIVE_CABI=0`** is the escape hatch back to the legacy rlib link.  Verified green on `windows-latest` (`win-cdylib.yml` job `win-cdylib-cabi`, `native_crate_package_links_and_runs_via_cabi` PASS, 36/36) before the flip.  Two Windows-only gaps the dependency-free fixture exposed were fixed en route: (1) the `loft --native test` path didn't propagate loft's own build-script `OUT_DIR`s (windows-targets `windows.X.lib`) → `LNK1181`; (2) the import-lib naming above.  **Coverage:** the C-ABI native_crate EXEC path had NO automated test (phase 0 was a manual probe), so the first focused-CI green was vacuous — its subset never linked a `[native] crate` package.  Closed by `native_crate_package_links_and_runs_via_cabi` (`tests/native.rs`) + the cheap `tests/lib/native_scalar_pkg` fixture (one scalar `#native` symbol, no loft-ffi): it rides BOTH the normal PR/CI suite and `win-cdylib-cabi`, asserting a `42` oracle with no LNK1181 env-skip so a broken link fails loudly. |
| **@PLN26 ph.5** — lazy host rlib (**deferred, LOW priority → [#390](https://github.com/loft-lang/loft/issues/390)**) | [@PLN26](https://github.com/loft-lang/plans/issues/26) (closed) | **Deferred.**  On the default C-ABI path a native package's host rlib is built but never linked (`auto_build_native` runs a plain `cargo build`; `crate-type = ["cdylib","rlib"]` emits both).  Only `LOFT_NATIVE_CABI=0` links the rlib (wasm uses a separate cross-built rlib).  Making it lazy (`cargo rustc --crate-type cdylib`, build the rlib on demand) saves only the rlib-emit — rustc compiles the crate + deps ONCE and emits both from that single compilation, so the rlib is a near-free byproduct, not a second compile.  Worth doing only if the emit shows measurable overhead. |
| **N8b.3** — `yield from` delegation | [§ N8b](#n8b--coroutine-native-codegen) (line ~944, marked CO1.3d) | Open — design drafted, not implemented.  Native coroutines support `yield value` (N8b.1 + N8b.2 shipped) but NOT `yield from <inner_iterator>` delegation. |
| **N8c.1** — Audit generic text-return | [§ N8c](#n8c--generic-function-instantiation) | **Probably overlaps shipped work.**  Plan-17 closure landed @P237 / @P238 / @P242 (`Value::Tuple` recursion in `substitute_type_in_value`; `tuple_text_to_string` flag).  Action: un-skip `tests/scripts/48-generics.loft`; if green, mark closed. |
| **N8c.2** — Fix generic text-return | [§ N8c](#n8c--generic-function-instantiation) | Same overlap.  N8c.1 audit determines whether N8c.2 is needed. |
| **`as_op_call` accessor** — fold the unspan+call-shape probe | [§ Walker convention](#walker-convention--always-unspan-before-matching-value) | Open (S; corpus-gated).  The `x.unspan()` + `Value::Call(d,_)` + `def(d).name()=="Op…"` idiom is open-coded ~dozens of times across `generation/` (81 `unspan()` sites; 21 in `dispatch.rs`) with no shared accessor.  That exact shape — a probe that forgot to unspan, so `Span(Call(..))` slipped through — was the routing 451 bug (`tests/scripts/451-text-tailcall-nwb-callee.loft`).  Fold it into one `Value::as_op_call(&self, data) -> Option<(&Def, &[Value])>` that always unspans → DRYs the sites and **structurally** kills the "forgot to unspan" class (the constructive form of the `pre_eval_walkers_unspan` guard).  Verifiable by `tests/oracle/27`+`28` + the 451/500 guards.  **Sequence AFTER @PLN98's `generation/` edits settle** — a pervasive same-file refactor collides with its opt-in-flag codegen work. |
| **N20a** — Add `ops` import to generated `fill.rs` | [§ N20](NATIVE-history.md#n20--repair-fillrs-auto-generation) | Open — trivial single-line add in `src/create.rs::generate_code()`. |
| **N20b** — Run `cargo fmt` on generated `fill.rs` | [§ N20](NATIVE-history.md#n20--repair-fillrs-auto-generation) | Open — runs `rustfmt` on the generated file so formatting matches the hand-maintained version. |
| **N9 (C71)** — native-dispatch completeness | [§ N9](#n9--native-library-shared-store-dispatch-c71) | Open *enhancements* on a complete, graceful core (a construct that can't cross **interprets** — not a bug): closures (`__closure`) · `generate_interface` aggregate names (`sorted<Item[k]>`) · D2a binary schema interface (no source re-parse; ties to the registry) · `hash`/`index`/`spatial` coverage · gate-driven dispatch (N4 tail) · background build (N3 polish).  Detail in § N9.  Routed here from @PLN11 Arc N (2026-06-05). |
| **N10 prune** | [§ N10 below](NATIVE-history.md#current-state-2026-04-07) | **Stale.**  Says "6 fail, 34 skip of 85 files"; current state is 108/108 pass.  Sub-steps are diagnostic recipes for failures that no longer exist.  Action: prune § N10 + N20 to historical pointers when N8b.3 + N8c.x close. |
| **one-walk pre-eval** (#272 class) | [§ N21 below](NATIVE-history.md#n21--one-walk-pre-eval-unlink-collect-from-emit--shipped-2026-06-06) | **Shipped (2026-06-06).**  Pre-eval identity is now intrinsic (IR node address → `_pre_N`, in `PreEvalSet`); `output_code_inner` substitutes a hoisted node by address, so the operand is emitted once and never re-generated.  The regenerate-and-string-match machinery (`output_code_with_subst` / `output_if_with_subst` / `try_subst_pre_eval`) is **deleted**.  Fixes #272 + the counter-coupling class.  See [COMPILER.md § Synthesised-identity stability](COMPILER_PARSER.md#synthesised-identity-stability--the-counter-coupling-hazard). |

Suggested order: N8c.1 audit (fastest) → N20a + N20b (trivial pair)
→ N8b.3 (actual feature work; touches `src/generation/coroutine.rs`)
→ § N10 + § N20 cleanup.

---
