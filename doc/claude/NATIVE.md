
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
are in [PERFORMANCE.md § Design: P2](PERFORMANCE.md) → *Shipped: the NATIVE half*.

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
1. An `init(db: &mut Stores)` function that registers all type schemas
2. Rust functions for each loft function, receiving `stores: &mut Stores` as first arg
3. A `#[test]` wrapper that calls `init()` then the test function

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

## Steps

### N1 — Fix `#rust` templates for generated code

Three search-and-replace fixes in `default/01_code.loft`, batchable into a single commit:

**1a. `external::` → `ops::`**
`#rust` templates use `external::op_add_int(...)` etc. The `external` module doesn't exist
in generated code — only `ops` is imported. Two renames needed:
- `external::op_min_single_int(@v1)` → `ops::op_negate_int(@v1)`
- `external::op_min_single_long(@v1)` → `ops::op_negate_long(@v1)`

All other `external::op_*` names match `ops::op_*` exactly.

**1b. `u32::from(@fld)` → `((@fld) as u32)`**
Const parameters are emitted as `i32` literals but templates wrap in `u32::from()`.
Rust has no `u32: From<i32>` impl. Field offsets are always non-negative, so `as u32` is safe.

**1c. `s.database.*` → `stores.*` in generation/ (NOT in templates)**
Some templates reference `s.database.allocations`, `s.database.enum_val()`, etc.
In generated code there is no `s` — only `stores: &mut Stores`.  However, these
patterns **must stay unchanged in `default/01_code.loft`** because `create.rs` needs
them for fill.rs (the bytecode interpreter).  The fix goes in `src/generation/`:
add `res = res.replace("s.database.", "stores.");` in the template substitution path
(the inverse of what create.rs does with `stores.` → `s.database.`).

**Files:** `src/generation/` (template substitution), possibly `src/database/mod.rs` (make methods pub)
**Verify:** `grep -c 'external::\|u32::from\|s\.database' tests/generated/*.rs` returns 0
**Eliminates:** ~1019 errors
**Interpreter safety:** Templates unchanged; fill.rs unaffected

---

### N2 — Include stdlib in each generated test file

**Problem:** `tests/testing.rs` calls `output_native(w, start, def_nr)` for test files,
where `start` skips all default-library definitions. Standard library functions like
`n_assert` are in `[0, start)` and only written to `tests/generated/default.rs`. Individual
test files cannot find them.

**Fix:** Change `tests/testing.rs` to pass `(0, def_nr)` instead of `(start, def_nr)` to
`output_native()` for test files. Each file becomes self-contained.

**Files:** `tests/testing.rs` lines 232–235
**Verify:** `grep -c 'fn n_assert' tests/generated/expressions_*.rs` finds definitions
**Eliminates:** ~92 errors; ~41 simple files compile after N1+N2

---

### N3 — Add `codegen_runtime` module for database operations

**Problem:** Database operations are bytecode opcodes with no `#rust` template. The code
generator emits them as function calls (`OpNewRecord(...)`, `OpDatabase(...)`) but no such
functions exist in generated code. These can't be simple templates because they involve
complex multi-step interactions with `Stores`.

**Fix:** Create `src/codegen_runtime.rs` with wrapper functions that replicate what the
bytecode interpreter does for each operation:

| Function | Reference in | Purpose |
|----------|-------------|---------|
| `op_database(stores, tp) -> DbRef` | `src/state/io.rs` | Allocate database root record |
| `op_new_record(stores, parent, tp, fld) -> DbRef` | `src/state/io.rs` | Create struct element |
| `op_finish_record(stores, parent, rec, tp, fld)` | `src/state/io.rs` | Finalize record (insert into collection) |
| `op_free_ref(stores, v)` | `src/fill.rs` | Free a reference |
| `op_get_record(stores, db, tp, keys) -> DbRef` | `src/state/io.rs` | Look up record in collection |
| `op_format_database(stores, ...) -> String` | `src/state/debug.rs` | Format record for display |
| `op_conv_text_from_null() -> Str` | `src/fill.rs` | Null text constant |

Register the module: add `pub mod codegen_runtime;` to `src/lib.rs`.
Add `use loft::codegen_runtime::*;` to the generated preamble in `src/generation/`.
Update `output_call()` in `generation/` to emit these function names for the
corresponding `Op*` definitions.

**Files:** new `src/codegen_runtime.rs`, `src/lib.rs`, `src/generation/`
**Eliminates:** ~260 errors

---

### N4 — Handle `Value::Iter` and `Value::Keys` in code generation

**Problem:** `output_code_inner()` has no match arms for `Value::Iter` and `Value::Keys`.
They fall through to the `_ => write!(w, "{code:?}")` debug fallback.

**Fix:** Add match arms:
- `Value::Iter(var, create, next, extra_init)` — emit a Rust loop calling
  `codegen_runtime::op_iterate()` / `codegen_runtime::op_step()`
- `Value::Keys(keys)` — emit a key array literal `vec![Key { ... }, ...]`

Also add `op_iterate()` and `op_step()` to `codegen_runtime.rs`.

**Files:** `src/generation/`, `src/codegen_runtime.rs`
**Depends on:** N3
**Eliminates:** ~11 errors

---

### N5 — Skip or fix empty native function bodies

**Problem:** Functions like `OpConvTextFromNull`, `OpLengthCharacter`, operator functions
with a return type but no `#rust` body are emitted as `fn name() -> T {}` — missing the
return expression.

**Fix:** In `output_function()`, skip emitting functions that are:
- Operators with a `#rust` template (these are inlined at call sites, not called directly)
- Native functions with no IR body (registered via `FUNCTIONS` table in `native.rs`)

For operators that genuinely need a `#rust` template but don't have one, add the template
to `default/01_code.loft`.

**Files:** `src/generation/`, `default/01_code.loft`
**Eliminates:** remaining ~50 errors; all files compile

---

### N6 — Add compilation gate test

**Problem:** No CI protection against regressions in generated code quality.

**Fix:** Add a test that runs `rustc` on a representative generated file and asserts
it compiles without errors. This prevents future changes from breaking the code generator.

**Files:** new test in `tests/` or addition to `tests/testing.rs`

---

### N7 — Add `--native` CLI flag

**Problem:** No user-facing way to generate and run native code.

**Fix:** Add `--native <file.loft>` to `src/main.rs`:
1. Parse and compile the loft program (same as normal)
2. Generate a Rust source file via `Output::output_native()`
3. Compile with `rustc` (linking against the loft crate)
4. Run the resulting binary

**Files:** `src/main.rs`
**Depends on:** N1–N6

---

### N10 — Fix remaining native codegen failures

**Current state** (after N1–N7): 51 compile, 45 pass, 6 fail, 34 skip of 85 files.

The 6 runtime failures and 34 compile failures have distinct root causes.  Each
sub-step below fixes one root cause and is independently testable.

---

#### N10a — Fix `output_init` to register ALL intermediate types

**Problem:** `output_init` (generation/:273–318) skips intermediate type
registrations.  The compile-time type IDs are sequential across ALL definitions
with `known_type != u16::MAX`, but `output_init` only emits types matching:
`DefType::Struct || DefType::Enum || DefType::Vector || (EnumValue with attrs)`.

This skips:
- Plain `EnumValue` variants without attributes (like `Start`, `Ongoing`)
- `DefType::Type` entries (byte/short field types created by `db.byte()`)
- Anonymous vector types created as struct fields

**Symptoms:**
- `enums_types`: "index out of bounds: the len is 20 but the index is 20"
- `enums_enum_field`: "Unknown record 1150964204" (garbage from wrong type layout)

**Root cause detail:** The compile-time `fill_database` (`src/typedef.rs:135–232`)
assigns `known_type` via `database.structure()`, `database.enumerate()`, etc. to
every definition in order.  The runtime must register types in exactly the same
order.  When `output_init` skips a type, all subsequent type IDs shift down.

**Fix (generation/ `output_init`):**
1. Collect ALL definitions with `known_type != u16::MAX` into `type_defs` — remove
   the `def_type` filter at line 281–285.
2. Sort by `known_type` (already done at line 290).
3. For each type, dispatch on `def_type`:
   - `Struct` → `db.structure(name, 0)` + fields (existing code)
   - `EnumValue` with attrs → `db.structure(name, enum_value)` + fields (existing)
   - `EnumValue` without attrs → skip (no runtime registration needed — the parent
     Enum's `db.value()` already created the slot)
   - `Enum` → `db.enumerate(name)` + `db.value()` per variant (existing)
   - `Vector` → `db.vector(content_type)` (existing)
   - `Type` → check if it's a byte/short type; emit `db.byte(min, nullable)` or
     `db.short(min, nullable)` or skip (field-types are registered implicitly by
     their parent struct's `db.field()` call)

The key insight: `DefType::Type` entries with `known_type != u16::MAX` represent
standalone byte/short types (like the text type = 5).  They must be registered
with `db.byte()` or `db.short()` so their type ID is consumed.  Compare with
`typedef.rs:173–195` which handles `Parts::Byte` and `Parts::Short`.

**Files:** `src/generation/` (`output_init`, lines 273–318)
**Test:** `enums_types` and `enums_enum_field` pass
**Verify:** `grep -c 'db\.' tests/generated/enums_types.rs` registration count
matches compile-time types: `cargo test --test expressions -- enums_types` then
count db.structure + db.enumerate + db.vector + db.byte calls in the generated file

---

#### N10b — Fix `output_set` for DbRef deep copy

**Problem:** `Set(var_b, Var(var_a))` where both are `Type::Reference` emits
`var_b = var_a` — a pointer copy.  Both variables then share the same database
record.  Modifying one modifies the other.

**Symptom:** `objects_independent_strings`: "hello world" instead of "hello" —
modifying `b.name` also changes `a.name` because they share the same record.

**Root cause detail:** The bytecode codegen (`src/state/codegen.rs:405–423`)
detects same-type reference assignment in `generate_set` and synthesises a
`Value::Call(OpCopyRecord, [Var(src), Var(dst), Int(tp_nr)])`.  The `generation/`
`output_set` does not perform this synthesis — it emits a plain `var_b = var_a`.

**Fix (generation/ `output_set`, after line 997):**
After emitting the assignment, check if:
1. Variable type is `Type::Reference(d_nr, _)`
2. RHS is `Value::Var(src_var)` where src_var has the same reference type
3. RHS is NOT `Value::Null`

If all three hold, emit an `OpCopyRecord` call:
```rust
// In output_set, after the regular assignment emission:
if let Type::Reference(d_nr, _) = variables.tp(var) {
    if let Value::Var(src) = to {
        if let Type::Reference(_, _) = variables.tp(*src) {
            let tp_nr = self.data.def(*d_nr).known_type;
            writeln!(w)?;
            self.indent(w)?;
            write!(w, "OpCopyRecord(stores, var_{src_name}, var_{name}, {tp_nr}_i32)")?;
        }
    }
}
```

The `tp_nr` comes from `data.def(d_nr).known_type` where `d_nr` is the struct
definition number from the `Type::Reference(d_nr, _)`.

**Files:** `src/generation/` (`output_set`, lines 967–1014)
**Test:** `objects_independent_strings` passes

---

#### N10c — Fix `OpFormatDatabase` for struct-enum variants

**Problem:** `OpFormatDatabase` outputs only the enum type name (e.g. "Call")
instead of the full struct representation ("Call {function:\"foo\",parameters:2}").

**Symptom:** `enums_define_enum`: 'Call != "Call {function:\"foo\",parameters:2}"'

**Root cause detail:** `ShowDb::write` (`src/database/format.rs:295–349`) handles
struct-enum variants by reading the discriminator byte from the record to determine
the variant, then dispatching to `write_struct()` for the variant's fields.  This
works correctly — the issue is in how `output_call` passes the type to
`OpFormatDatabase`.

The bytecode interpreter's `format_db` (`src/state/io.rs:301–317`) reads `db_tp`
from bytecode and passes it as `known_type` to `ShowDb`.  The `known_type` must
be the PARENT enum type (e.g. the `Val` enum containing `A` and `B` variants),
not a specific variant.  `ShowDb` then reads the discriminator to pick the variant.

Check what the generated code passes — if `output_call`'s `OpFormatDatabase`
handler passes the variant type instead of the parent enum type, the format will
only show the variant name without struct fields.

**Fix (src/generation/ or src/codegen_runtime.rs):**
1. In `output_call`'s `OpFormatDatabase` handler, verify the `tp_val` argument
   is the parent enum's `known_type`, not a variant's.
2. If the IR passes the wrong type, fix the `output_call` handler to look up
   the parent enum type from the definition.
3. If the IR passes the correct type but `ShowDb` doesn't recurse into variant
   fields, the bug is in `ShowDb::write` — check `Parts::Enum` handling at
   format.rs:328–349.

**Debug approach:** Compare the `db_tp` value passed by the bytecode interpreter
vs the generated code by adding a `eprintln!("OpFormatDatabase db_tp={db_tp}")` in
both `codegen_runtime::OpFormatDatabase` and `State::format_db`.

**Files:** `src/codegen_runtime.rs` and/or `src/generation/`
**Test:** `enums_define_enum` and `enums_general_json` pass

---

#### N10d — Fix null DbRef handling in vector operations

**Problem:** `vectors_fill_result` panics with "Unknown record 2147483648" (`u32::MAX`).

**Symptom:** `vectors_fill_result`: "Unknown record 2147483648"

**Root cause detail:** `stores.null()` (`src/database/allocation.rs:103–105`) calls
`self.database(u32::MAX)` which allocates a store but returns `DbRef { store_nr, rec: 0, pos: 0 }`.
The `store_nr` is a real store index (not 0).  The null DbRef is passed to
`n_fill(stores, var_result)` by value.  Inside `n_fill`:
1. `vector::clear_vector(&var_result, &mut stores.allocations)` is called
2. `var_result.rec == 0` but `store_nr` points to a real store
3. `clear_vector` tries to access the store and hits an invalid record

The bytecode interpreter avoids this because the variable sits on the stack and
`OpDatabase` modifies it in-place before `clear_vector` runs.  In generated code,
`OpDatabase` returns a new DbRef (assigned to `var_result`), but `clear_vector`
runs BEFORE `OpDatabase` in the generated sequence.

**Fix (src/codegen_runtime.rs and/or src/generation/):**

Option A — Guard `clear_vector` calls:
In generated code, add a null check before `clear_vector`:
```rust
if var_result.rec != 0 { vector::clear_vector(&var_result, &mut stores.allocations); }
```
This requires detecting `OpClearVector` in `output_call` and wrapping it.

Option B — Fix `stores.null()` return value:
Return `DbRef { store_nr: u16::MAX, rec: 0, pos: 0 }` as the sentinel.
The `u16::MAX` store_nr is already used by `OpNullRefSentinel` and guards in
`Stores::free/valid` already check for it.  However, this changes `Stores::null()`
behaviour which could affect the interpreter.

Option C — Reorder in generated code:
Ensure `OpDatabase` runs before `clear_vector`.  Check the IR ordering and whether
`output_code_inner` preserves statement order correctly.

**Recommended:** Option A — minimal, codegen-only change, no interpreter impact.

**Files:** `src/generation/` (`output_call` for `OpClearVector`)
**Test:** `vectors_fill_result` passes

---

#### N10e — Fix remaining 34 compile failures

After N10a–N10d fix the 6 runtime failures, the 34 compile failures remain.

| Category | Count | Sub-step |
|----------|-------|----------|
| Mismatched types (`()` for missing else) | 16 | N10e-1 |
| `if`/`else` incompatible types | 4 | N10e-1 |
| `OpIterate` / `OpStep` / `Keys` not found | 3 | N10e-2 |
| `OpFormatFloat` / `OpFormatStackLong` | 2 | N10e-3 |
| Empty pre-eval (`let _pre = ;`) | 2 | N10e-5 |
| `crate::state::STRING_NULL` reference | 2 | N10e-4 |
| Double borrow of `stores` | 1 | N10e-5 |
| Wrong argument count for `OpGetRecord` | 1 | N10e-5 |
| `prefix _pre14 is unknown` | 1 | N10e-5 |

---

**N10e-1: Fix `output_if` for missing else branches (fixes ~20 files)**

**Location:** `src/generation/` `output_if` (lines 828–862) and
`output_code_inner` (line 747: `Value::Null => write!(w, "()")`)

**Problem:** When `false_v` is `Value::Null`, the if-expression emits `()` for the
else branch.  If the true branch produces a value (e.g. `i32`, `&str`), Rust
reports "mismatched types: expected i32, found ()".

**Current code path:** `output_if` at line 856 calls `output_code_inner(w, false_v)`
which hits `Value::Null => write!(w, "()")` at line 747.

**Fix approach:** `output_if` does not receive type information.  The type must be
inferred from the true branch.  Two options:

Option A (simpler): Add a helper `fn infer_if_type(&self, true_v: &Value) -> Option<Type>`
that inspects the true branch to determine its result type.  Then in `output_if`,
when `false_v` is `Value::Null` and `infer_if_type` returns a non-void type, emit
a typed null instead of `()`:

```rust
// In output_if, when false_v is Value::Null and true branch returns a value:
match inferred_type {
    Type::Integer(_, _) => write!(w, "{{ i32::MIN }}")?,
    Type::Long => write!(w, "{{ i64::MIN }}")?,
    Type::Float => write!(w, "{{ f64::NAN }}")?,
    Type::Single => write!(w, "{{ f32::NAN }}")?,
    Type::Boolean => write!(w, "{{ false }}")?,
    Type::Text(_) => write!(w, "{{ \"\" }}")?,
    Type::Reference(_, _) => write!(w, "{{ stores.null() }}")?,
    Type::Enum(_, false, _) => write!(w, "{{ 255_u8 }}")?,
    _ => write!(w, "{{ () }}")?,
}
```

Option B: Track the expected result type through the `output_code_inner` recursion
by adding a `result_type: Option<&Type>` parameter.  More invasive but cleaner.

**Recommended:** Option A — `infer_if_type` can inspect:
- `Value::Call(d, _)` → `data.def(d).returned`
- `Value::Var(v)` → `variables.tp(v)`
- `Value::Int(_)` → `Type::Integer(...)`
- `Value::Block(bl)` → `bl.result`

**Files:** `src/generation/`
**Test:** 20 files that currently fail with "mismatched types" or "if/else incompatible"

---

**N10e-2: Add `OpIterate`/`OpStep` + `Value::Iter` handler (fixes 3 files)**

**Problem:** Iterator operations are complex bytecode sequences.  The generated
code currently falls through to debug output for `Value::Iter`.

**Reference implementation:**
- `iterate()`: `src/state/io.rs:373–446` — reads `on: u8` (flags), `arg: u16`
  (field ref), `keys: Vec<Key>`, `from_key`/`till_key`, stack values `from`/`till`,
  then dispatches on collection type (1=index/tree, 2=sorted/vector, 3=ordered)
  to compute `(start, finish)` position markers.
- `step()`: `src/state/io.rs:473–570` — reads current position from state variable,
  advances to next element via `tree::next()`/`vector::vector_step()`, signals
  loop end with `u32::MAX` sentinel.

**Codegen_runtime signatures:**
```rust
/// Returns (start_pos, finish_pos) for the iteration range.
pub fn OpIterate(
    stores: &Stores,
    data: DbRef,       // collection reference
    on: u8,            // flags: bits 0-5=type, bit 6=reverse, bit 7=exclusive
    arg: u16,          // field type reference
    keys: &[Key],      // sort/index key definitions
    from: &[Content],  // start key values
    till: &[Content],  // end key values
) -> (u32, u32)

/// Advances iterator; returns next element DbRef or None if done.
pub fn OpStep(
    stores: &Stores,
    cur: &mut u32,     // current position (mutated in-place)
    finish: u32,       // end sentinel from OpIterate
    data: DbRef,       // collection reference
    on: u8,            // same flags as OpIterate
    arg: u16,          // field type reference
) -> DbRef             // next element (rec=0 when done)
```

**Value::Iter handler in `output_code_inner`:**
`Value::Iter(var_nr, create, step, extra_init)` should emit:
```rust
{
    <extra_init>;
    let (mut _iter_pos, _iter_end) = { <create> };
    loop {
        let var_<name> = { <step> };
        if var_<name>.rec == 0 { break; }
        // loop body follows in the enclosing Block
    }
}
```

The `create` sub-expression is a `Value::Call(OpIterate, ...)`.
The `step` sub-expression is a `Value::Call(OpStep, ...)`.
The loop body is NOT inside the Iter — it follows in the parent Block.

**Files:** `src/generation/` (`output_code_inner`), `src/codegen_runtime.rs`
**Test:** 3 files with iterator operations compile and pass

---

**N10e-3: Add `OpFormatFloat`/`OpFormatStackLong` handlers (fixes 2 files)**

**Problem:** Format operations for float and long values are not handled in
`output_call`, so they're emitted as function calls to non-existent functions.

**Reference implementation:** `src/ops.rs:518–586`
```rust
pub fn format_long(s: &mut String, val: i64, radix: u8, width: i32, token: u8, plus: bool, note: bool)
pub fn format_float(s: &mut String, val: f64, width: i32, precision: i32)
pub fn format_single(s: &mut String, val: f32, width: i32, precision: i32)
```

These are already public in `loft::ops`.  The bytecode versions
(`src/state/text.rs:351–391`) read parameters from bytecode + stack and call
these `ops` functions.

**Fix:** Add special-case handlers in `output_call` that emit direct calls to
`ops::format_long` / `ops::format_float`:

```rust
"OpFormatLong" | "OpFormatStackLong" => {
    // Already handled by self.format_long(w, vals) — verify it works
}
"OpFormatFloat" | "OpFormatStackFloat" => {
    if let [ref work_var, ref val, ref width, ref precision] = vals[..] {
        write!(w, "ops::format_float(&mut ")?;
        // emit work_var as mutable String ref
        // emit val, width, precision
        write!(w, ")")?;
    }
    return Ok(());
}
```

Check whether `OpFormatLong` is already handled (line 1028: `"OpFormatLong" => return self.format_long(w, vals)`).  If so, only `OpFormatFloat` /
`OpFormatStackFloat` need new handlers.

**Files:** `src/generation/` (`output_call`)
**Test:** 2 files with float/long formatting compile

---

**N10e-4: Fix `crate::state::STRING_NULL` reference (fixes 2 files)**

**Problem:** The `#rust` template for `OpConvBoolFromText` contains:
```
@v1 != crate::state::STRING_NULL
```
In the bytecode interpreter (`fill.rs`), this resolves because `crate` = the
`loft` crate.  In generated standalone `.rs` files, `crate` refers to the
generated file itself — not the `loft` crate.

**`STRING_NULL` definition:** `src/state/mod.rs:24`:
```rust
pub const STRING_NULL: &str = "\0";
```

**Fix:** In `output_call_template` (generation/, after the `s.database.` → `stores.`
substitution at line 1102), add:
```rust
res = res.replace("crate::state::", "loft::state::");
```

This handles any `crate::` reference in templates that should point to the `loft`
crate in generated code.

**Files:** `src/generation/` (`output_call_template`, ~line 1103)
**Test:** 2 files with `crate::state::` references compile

---

**N10e-5: Fix empty pre-eval, prefix, and argument count issues (fixes 3 files)**

**Problem 1 — Empty pre-eval:** `collect_pre_evals` (`src/generation/:601–655`)
can produce a pre-eval binding where the expression buffer is empty:
`let _pre19 = ;` — a syntax error.

**Root cause:** `rewrite_code` (line 659) calls `generate_expr_buf(arg)` which
for certain `Value::Null` or void expressions returns an empty string.

**Fix:** In `output_code_with_subst` or `rewrite_code`, skip emitting a pre-eval
binding when the expression is empty or when `generate_expr_buf` returns `"()"`.

**Problem 2 — Prefix `_pre14`:** Rust edition 2021+ treats `_pre14` as a prefix
token (like `b"..."` or `r"..."`), causing parse errors in some contexts.

**Fix:** Change the pre-eval naming from `_pre{counter}` to `_pre_{counter}`
(underscore separator).  In `collect_pre_evals_inner` at lines 615 and 640:
```rust
let name = format!("_pre_{}", self.counter);
```

**Problem 3 — Wrong argument count for `OpGetRecord`:** The generated code
passes inline key values as separate arguments, but the `codegen_runtime`
function expects a `&[Content]` slice.

**Fix:** In `output_call`, add a handler for `OpGetRecord` that collects
the key arguments into a `vec![...]` literal before calling the runtime function.

**Files:** `src/generation/`
**Test:** 3 remaining files compile

---

## N20 — Repair fill.rs Auto-Generation

### Problem

`src/fill.rs` (the bytecode operator dispatch table) is hand-maintained.
`src/create.rs::generate_code()` produces `tests/generated/fill.rs` on every
debug test run, but it cannot replace `src/fill.rs` because:

1. **Missing `use crate::ops;`** — the generated file omits the `ops` import
2. **Formatting** — inline braces (`if x {y}`) vs expanded (`if x {\n    y\n}`)
3. **Math functions inlined vs delegated** — the hand-maintained version inlines
   match arms for `math_func_single` etc.; the generated version calls
   `s.math_func_single()` which delegates to the same State method

The OPERATORS array order and function bodies are otherwise identical.  The
generated file compiles inside the crate.

### Impact

When a new opcode is added to `default/01_code.loft` or `default/02_files.loft`,
the developer must manually add the operator to `src/fill.rs` — find the right
position in the OPERATORS array, write the function body, and update the array
size constant.  This is error-prone (the T2-7 `mkdir` issue showed this).

### Fix Path

#### N20a — Add `ops` import to generated fill.rs

In `create.rs::generate_code()`, add `use crate::ops;` to the generated header.

**File:** `src/create.rs` (line 125)
**Effort:** Trivial

---

#### N20b — Run `cargo fmt` on generated fill.rs

After `generate_code()` writes `tests/generated/fill.rs`, run `rustfmt` on it
(or call `std::process::Command::new("rustfmt")` from the test).  This fixes
all formatting differences.

Alternatively, emit properly formatted code in `generate_code()` by adding
newlines after `{` and before `}` in the template expansion.

**File:** `src/create.rs` or `tests/testing.rs`
**Effort:** Small

---

#### N20c — Replace src/fill.rs with generated version

Once N20a+N20b produce a generated fill.rs that is byte-for-byte equivalent to
the hand-maintained one (after formatting), add a CI step that:

1. Runs `generate_code()` (happens automatically in debug tests)
2. Compares `tests/generated/fill.rs` with `src/fill.rs`
3. Fails if they differ — forces the developer to copy the generated version

This eliminates manual maintenance.  New opcodes added to `default/*.loft` with
`#rust` templates are automatically included.  Operators without templates
(those that delegate to State methods) need a `#rust` template added, or a
new `#state_call "method_name"` annotation.

**File:** `tests/testing.rs` or CI script
**Effort:** Medium

---

#### N20d — Add `#state_call` annotation for delegation operators

Currently, 52 operators have no `#rust` template because they delegate to
a State method.  Their function bodies are `s.method_name()`.

Add a new annotation in `default/*.loft`:
```loft
fn OpIterate(...);
#state_call"iterate"
```

`create.rs::generate_code()` recognises `#state_call` and emits:
```rust
fn iterate(s: &mut State) {
    s.iterate();
}
```

This covers all 52 delegation operators and eliminates the last hand-written
functions from fill.rs.

**Files:** `default/01_code.loft`, `default/02_files.loft`, `src/create.rs`,
`src/parser/definitions.rs` (parse the new annotation)
**Effort:** Medium

---

## N21 — one-walk pre-eval (unlink collect from emit) — **Shipped (2026-06-06)**

Fixes the #272 counter-coupling class. Full root analysis:
[COMPILER.md § Synthesised-identity stability](COMPILER.md#synthesised-identity-stability--the-counter-coupling-hazard).

### Problem

Native statement lowering hoists store-borrowing / side-effecting / duplicated
sub-expressions into `let _pre_N = …;` bindings so an outer expression can borrow
them safely. Today this is **two traversals of the same IR**:

1. `collect_pre_evals(v)` walks `v`, decides what to hoist, and records
   `PreEvalEntry = (name, pcode, prefix, pre_counter, replace_all)` — where `pcode` is
   *the Rust text the node generates with `self.counter` at `pre_counter`*.
2. `output_code_with_subst(v, pre_evals)` walks `v` again and must re-recognise each
   hoisted node to emit its `_pre_N` name instead of the inline code. It does this by
   `try_subst_pre_eval`: rewind `self.counter = pre_counter`, **re-run codegen, and
   string-compare** to `pcode`; on match emit the name, else fall through to a
   whole-expression `.replacen(pcode, name)`.

A node's identity is therefore "the exact text it emits at counter K" — a property of
the walk, not the node. Any node kind that bypasses the structural recogniser (`Op*`
comparisons go through `output_call`'s template, not the per-arg `try_subst_pre_eval`
path) or whose inner `_pre_N` names drift between the two walks fails to match. #272:
`"{x}" != literal` with a stateful producer hoists the format block into a dead `_pre_4`
**and** re-inlines it in the `!=`; the producer (`#errors`, cleared on read) is consumed
by the dead copy, so the live copy reads empty. Silent on the interpreter, wrong on native.

### Fix as shipped: intrinsic identity, read from one place

`collect_pre_evals` still runs as a pass, but it now records each hoist keyed on the
**IR node's address** in a `PreEvalSet` (`by_node: address → entry`). `output_block`
emits the `let _pre_N = …;` bindings, then installs the set's address→name map as
`Output::active_pre_eval` (saved/restored per statement so nested blocks nest cleanly).
`output_code_inner` — the one primitive every sub-expression emission funnels through,
including `#rust` template operands via `generate_expr_buf` — checks `active_pre_eval`
at the top: a hoisted node emits its `_pre_N` name and returns. The operand is therefore
emitted **once**, recognised by identity, never re-generated.

With identity intrinsic, the second traversal's recogniser is dead and **deleted**:
`output_code_with_subst`, `output_if_with_subst`, `try_subst_pre_eval`, the
`pre_counter`-rewind regeneration, and the `.replacen` string substitution are gone.
`Op*` stops being special because nothing re-recognises anything — the `output_code_inner`
check fires for every node regardless of kind. The `PreEvalEntry`'s `match_code` /
`pre_counter` fields survive only for collect-time inner-binding assembly in
`rewrite_code` (not for emit-time matching).

### How the risks were handled

- **Borrow / evaluation order** — unchanged: bindings emit before the statement body in
  collection order (inner hoists before outer, since `rewrite_code` collects inner first).
- **Nested hoists** — the bindings list is already inner-first; address keys are unique
  across the live IR tree, so no collisions.
- **Nested blocks** — `active_pre_eval` is saved/restored around each statement, so an
  inner block installs its own map and restores the outer on return.
- **`patch_hoisted_returns` / ref-tail capture** — unaffected; verified by the full suite.

### Verification (done)

Matrix on **both** backends — pure operand, side-effecting operand (#272), nested user-fn
args (the double-borrow case), narrow-int — all identical interp/native. Regression test
`tests/scripts/repro_p272_inline_format_compare.loft` (runs both backends). Full suite
green (only the pre-existing `viewer_markdown` timing flake red).

**Files:** `src/generation/pre_eval.rs` (`PreEvalSet`, deletions), `src/generation/emit.rs`
(`output_code_inner` push-down, `output_block` install/restore), `src/generation/mod.rs`
(`Output::active_pre_eval`).

---

## Dependency Graph

```
N1–N7 (done) ── 51 compile, 45 pass

N10a (output_init types) ──── fixes enums_types, enums_enum_field
N10b (DbRef deep copy) ────── fixes objects_independent_strings
N10c (FormatDatabase enum) ── fixes enums_define_enum, enums_general_json
N10d (null DbRef guard) ───── fixes vectors_fill_result
N10e-1 (output_if typed null) ── fixes 20 compile failures
N10e-2 (OpIterate/OpStep) ───── fixes 3 compile failures
N10e-3 (OpFormatFloat/Long) ─── fixes 2 compile failures
N10e-4 (crate::state:: fix) ─── fixes 2 compile failures
N10e-5 (pre-eval/prefix) ────── fixes 3 compile failures
                                ── all 85 files compile and pass
```

N10a–N10d fix the 6 runtime failures (independent of each other).
N10e-1 is the highest-impact compile fix (20 files).
N10e-2–N10e-5 fix the remaining 10 compile failures.

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
`src/parser/mod.rs::try_generic_instantiation()`.  Each call site with a concrete type
produces a `DefType::Function` named `t_<len><type>_<name>`
(e.g. `t_7integer_identity`, `t_4text_identity`).

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
| `identity(42)` | `t_7integer_identity` | `integer` | Likely OK |
| `identity(3.14)` | `t_5float_identity` | `float` | Likely OK |
| `identity("hello")` | `t_4text_identity` | `text` | **Likely fails** — text-return wrapping |
| `identity(true)` | `t_7boolean_identity` | `boolean` | Likely OK |
| `pick_second(1, 99)` | `t_7integer_pick_second` | `integer` | Likely OK |
| `pick_second("a", "b")` | `t_4text_pick_second` | `text` | **Likely fails** — same |

**Audit procedure:**
1. Temporarily remove `"48-generics.loft"` from `SCRIPTS_NATIVE_SKIP`.
2. Run `cargo test --test native 2>&1 | head -80` to capture compile errors.
3. Open the generated `.rs` file for the failing test and inspect the emitted bodies of
   `t_4text_identity` and `t_4text_pick_second`.
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
- [THREADING.md](THREADING.md) — Safety analysis for coroutine text handling (P2-R1/R2/R3)

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
| **@PLN26 ph.1** — same-symbol cross-package `#native` collision (**full fix DEFERRED — idea parked here; [#388](https://github.com/loft-lang/loft/issues/388) closed not-planned**) | [@PLN26](https://github.com/loft-lang/plans/issues/26) (closed) + [§ Resolution](#resolution-separate-the-api-id-from-the-rust-part-link-the-cdylib-by-c-abi) | **MVP shipped (`4004424b`+`027d187b`); full fix deferred — this row IS the parked idea (reopen #388 when the trigger below fires).**  Two `[native] crate` packages exporting the SAME `#native` symbol can't be disambiguated across the flat C-ABI namespace (link = first-`.so`-wins), nor the interpreter's symbol-keyed `BRIDGE_REGISTRY` (last-loaded-wins) — a **pre-existing both-backend** hazard, not a C-ABI regression.  MVP: native codegen rejects a **reachable** call to such a symbol with a "rename one" `compile_error!` (`Data::native_symbol_collisions` → `Output.native_collisions`, reachability-scoped so two packages sharing an *unused* symbol still build); `--interpret` keeps its existing silent behavior (guard is native-only by design).  **Deferred full fix:** per-package symbol prefix so they coexist — must change the cdylib export (loft-ffi-macros / `loft generate`), the interpreter registry + dispatch, and codegen *in lockstep*; only needed to CALL a symbol two **un-renameable** packages both export.  Repro/guards: `tests/lib/collide_{a,b,main,unused}` + `native_symbol_collision_across_packages_detected`. |
| **@PLN26 ph.2** — library-cdylib + native package (**✅ DONE → Part 2 of [#389](https://github.com/loft-lang/loft/issues/389)**) | [@PLN26](https://github.com/loft-lang/plans/issues/26) (closed) + [#389](https://github.com/loft-lang/loft/issues/389) | **Implemented + verified.**  A shared-store library cdylib that uses a `[native] crate` package now links it by C-ABI, exactly as the exec path does: `emit_program` sets `native_cabi` (the cdylib emits the package's fns as `extern "C"` `#[link_name]` decls — NOT `extern crate`), and `build_shared_cdylib` adds the `--extern loft_ffi` rlib + each package's resolved `.so` (`-L native`/`-l dylib`/RPATH via `extensions`-resolved `native_pkg_cabi_link_args`).  The sealed `.so` lifts the duplicate-`loft_register_v1` 2-package limit.  `LOFT_NATIVE_CABI=0` still refuses the combo loudly (the legacy rlib link can't take two `loft_ffi` rlibs into one cdylib).  Verified end-to-end: hex_grid's cdylib builds with graphics in the program; a library calling `graphics::save_png` links + runs the native (PNG written), `__cabi_loft_save_png` resolving via the cdylib's RUNPATH.  Regression: `shared_cdylib_with_native_package_emits_cabi_extern` (`tests/n2_cdylib.rs`).  Separately, the `viewer_markdown` collision is the cdylib's OWN raw `*mut Stores` → the `LoftStore`-handle decoupling (**Part 1 of #389 — now tracked as [STABILITY_HOTSPOTS.md § H9](STABILITY_HOTSPOTS.md#h9--raw-mut-stores-across-the-shared-store-cdylibhost-bridge); #389 closed**), NOT native-package linking. |
| **@PLN26 ph.3** — native package → wasm (**✅ DONE → [#438](https://github.com/loft-lang/loft/issues/438)**) | [@PLN26](https://github.com/loft-lang/plans/issues/26) | **Implemented + verified.**  A program that uses a `[native] crate` package now compiles to wasm: `extensions::auto_build_native_target` CROSS-BUILDS the package's native crate to the wasm target on demand (`cargo build --release --target <t>`, clean flags — a `#native` crate links the source-stable loft-ffi C-ABI, not loft's rlib, so no host-SVH flag-matching), into the IN-TREE `native/target/<t>/release/lib<stem>.rlib` the linker reads.  wasm links the **rlib** statically (no C-ABI `.so`), so `add_native_extern_flags` (`native_utils.rs`) also adds the package's HOST proc-macro deps (`native/target/release/deps`, where cargo builds e.g. `loft-ffi-macros` even under `--target`) — without it `extern crate <pkg>` fails `E0463` on the proc-macro.  Best-effort: a missing toolchain/target or a non-wasm-clean crate falls back to the clear "no wasm build" notice (no bare `E0463`).  Verified end-to-end: a program calling `native_scalar_pkg::native_answer()` → `loft --native-wasm` → cross-build → `wasmtime` prints `42` (`pln26_phase3_native_package_runs_on_wasm`, `tests/html_wasm.rs`); the `--html` leg (`wasm32-unknown-unknown`) cross-builds the same way; a shipped `prebuilt/<t>/` rlib is still honoured first.  **SVH note (the original ph.3 concern):** the StableCrateId collision is now *realizable* (packages have wasm rlibs), but a COLLISION still needs TWO packages with colliding wasm rlibs on a shared dep — isolate via `-Cmetadata` / rlib-identity when that first fires (NOT C-ABI; wasm links statically). |
| **@PLN26 ph.4** — Windows C-ABI link path (**✅ FLIPPED — C-ABI is the default on every host**) | [@PLN26](https://github.com/loft-lang/plans/issues/26) | **Done + CI-verified.**  Windows links a DLL through its import library; `-l dylib=<stem>` makes MSVC link.exe open `<stem>.lib`, but a Rust cdylib's import lib is named `<stem>.dll.lib`, so the arm copies `<stem>.dll.lib` → `<stem>.lib` beside it.  **No RPATH** (the MSVC linker rejects `-Wl,-rpath`; the loader finds the DLL beside the `.exe` / on `PATH`), so the DLL is **staged beside the binary** (`native_utils::stage_native_dlls`) — the Windows form of the `$ORIGIN` rpath.  `native_cabi_enabled()` now returns `true` everywhere; **`LOFT_NATIVE_CABI=0`** is the escape hatch back to the legacy rlib link.  Verified green on `windows-latest` (`win-cdylib.yml` job `win-cdylib-cabi`, `native_crate_package_links_and_runs_via_cabi` PASS, 36/36) before the flip.  Two Windows-only gaps the dependency-free fixture exposed were fixed en route: (1) the `loft --native test` path didn't propagate loft's own build-script `OUT_DIR`s (windows-targets `windows.X.lib`) → `LNK1181`; (2) the import-lib naming above.  **Coverage:** the C-ABI native_crate EXEC path had NO automated test (phase 0 was a manual probe), so the first focused-CI green was vacuous — its subset never linked a `[native] crate` package.  Closed by `native_crate_package_links_and_runs_via_cabi` (`tests/native.rs`) + the cheap `tests/lib/native_scalar_pkg` fixture (one scalar `#native` symbol, no loft-ffi): it rides BOTH the normal PR/CI suite and `win-cdylib-cabi`, asserting a `42` oracle with no LNK1181 env-skip so a broken link fails loudly. |
| **@PLN26 ph.5** — lazy host rlib (**deferred, LOW priority → [#390](https://github.com/loft-lang/loft/issues/390)**) | [@PLN26](https://github.com/loft-lang/plans/issues/26) (closed) | **Deferred.**  On the default C-ABI path a native package's host rlib is built but never linked (`auto_build_native` runs a plain `cargo build`; `crate-type = ["cdylib","rlib"]` emits both).  Only `LOFT_NATIVE_CABI=0` links the rlib (wasm uses a separate cross-built rlib).  Making it lazy (`cargo rustc --crate-type cdylib`, build the rlib on demand) saves only the rlib-emit — rustc compiles the crate + deps ONCE and emits both from that single compilation, so the rlib is a near-free byproduct, not a second compile.  Worth doing only if the emit shows measurable overhead. |
| **N8b.3** — `yield from` delegation | [§ N8b](#n8b--coroutine-native-codegen) (line ~944, marked CO1.3d) | Open — design drafted, not implemented.  Native coroutines support `yield value` (N8b.1 + N8b.2 shipped) but NOT `yield from <inner_iterator>` delegation. |
| **N8c.1** — Audit generic text-return | [§ N8c](#n8c--generic-function-instantiation) | **Probably overlaps shipped work.**  Plan-17 closure landed @P237 / @P238 / @P242 (`Value::Tuple` recursion in `substitute_type_in_value`; `tuple_text_to_string` flag).  Action: un-skip `tests/scripts/48-generics.loft`; if green, mark closed. |
| **N8c.2** — Fix generic text-return | [§ N8c](#n8c--generic-function-instantiation) | Same overlap.  N8c.1 audit determines whether N8c.2 is needed. |
| **`as_op_call` accessor** — fold the unspan+call-shape probe | [§ Walker convention](#walker-convention--always-unspan-before-matching-value) | Open (S; corpus-gated).  The `x.unspan()` + `Value::Call(d,_)` + `def(d).name()=="Op…"` idiom is open-coded ~dozens of times across `generation/` (81 `unspan()` sites; 21 in `dispatch.rs`) with no shared accessor.  That exact shape — a probe that forgot to unspan, so `Span(Call(..))` slipped through — was the routing 451 bug (`tests/scripts/451-text-tailcall-nwb-callee.loft`).  Fold it into one `Value::as_op_call(&self, data) -> Option<(&Def, &[Value])>` that always unspans → DRYs the sites and **structurally** kills the "forgot to unspan" class (the constructive form of the `pre_eval_walkers_unspan` guard).  Verifiable by `tests/oracle/27`+`28` + the 451/500 guards.  **Sequence AFTER @PLN98's `generation/` edits settle** — a pervasive same-file refactor collides with its opt-in-flag codegen work. |
| **N20a** — Add `ops` import to generated `fill.rs` | [§ N20](#n20--repair-fillrs-auto-generation) | Open — trivial single-line add in `src/create.rs::generate_code()`. |
| **N20b** — Run `cargo fmt` on generated `fill.rs` | [§ N20](#n20--repair-fillrs-auto-generation) | Open — runs `rustfmt` on the generated file so formatting matches the hand-maintained version. |
| **N9 (C71)** — native-dispatch completeness | [§ N9](#n9--native-library-shared-store-dispatch-c71) | Open *enhancements* on a complete, graceful core (a construct that can't cross **interprets** — not a bug): closures (`__closure`) · `generate_interface` aggregate names (`sorted<Item[k]>`) · D2a binary schema interface (no source re-parse; ties to the registry) · `hash`/`index`/`spatial` coverage · gate-driven dispatch (N4 tail) · background build (N3 polish).  Detail in § N9.  Routed here from @PLN11 Arc N (2026-06-05). |
| **N10 prune** | [§ N10 below](#current-state-2026-04-07) | **Stale.**  Says "6 fail, 34 skip of 85 files"; current state is 108/108 pass.  Sub-steps are diagnostic recipes for failures that no longer exist.  Action: prune § N10 + N20 to historical pointers when N8b.3 + N8c.x close. |
| **one-walk pre-eval** (#272 class) | [§ N21 below](#n21--one-walk-pre-eval-unlink-collect-from-emit) | **Shipped (2026-06-06).**  Pre-eval identity is now intrinsic (IR node address → `_pre_N`, in `PreEvalSet`); `output_code_inner` substitutes a hoisted node by address, so the operand is emitted once and never re-generated.  The regenerate-and-string-match machinery (`output_code_with_subst` / `output_if_with_subst` / `try_subst_pre_eval`) is **deleted**.  Fixes #272 + the counter-coupling class.  See [COMPILER.md § Synthesised-identity stability](COMPILER.md#synthesised-identity-stability--the-counter-coupling-hazard). |

Suggested order: N8c.1 audit (fastest) → N20a + N20b (trivial pair)
→ N8b.3 (actual feature work; touches `src/generation/coroutine.rs`)
→ § N10 + § N20 cleanup.

---


# Native Code Generation: Path to Default

## Goal

Make `--native` the default execution mode for loft. Games will run
as compiled native binaries, not interpreted bytecode. The interpreter
remains available via `--interpret` for debugging and WASM builds.

---

## Current State (2026-04-07)

### What works

- **108/108 native tests pass** (29 docs + 79 scripts, 0 failures)
- **All language features**: structs, enums, match, closures, coroutines,
  tuples, generics, threading, file I/O
- **Binary caching**: FNV-1a hash, <200ms recompile on change
- **Codegen infrastructure for #native calls**: `output_native_direct_call`
  and `output_native_api_call` are implemented
- **Package rlibs exist**: `lib/graphics/native/target/release/` etc.
- **Linking flags**: `--extern` and `-L dependency` already wired
- **Benchmarks exist**: `bench/run_bench.sh` with 10 test cases

### Architecture

Both modes share the same pipeline up to bytecode compilation:

```
Parse → Scopes → Bytecode compile → Extensions loaded
                                     ↓
                        ┌────────────┴────────────┐
                        ↓                         ↓
              Native codegen (1645)      Interpreter (1912)
              Output::output_native()    state.execute_argv()
              → Rust source → rustc     → Dispatch loop
              → Binary → Execute
```

Divergence: `main.rs:1645` checks `native_mode`.

---

## Step 1: Fix package path resolution

### Problem

`loft --lib lib --native /tmp/test.loft` with `use random` fails:
"Unknown function rand". The `make test-packages` target works because
it uses `loft test` (auto-detects `loft.toml` and adds `src/` to
lib_dirs).

### Root cause

The `--lib lib` flag pushes the RELATIVE path `"lib"` to `lib_dirs`
(main.rs:1153). The parser's `lib_path()` (mod.rs:2052-2170) searches
`lib_dirs` for `<dir>/<id>.loft` and `<dir>/<id>/src/<id>.loft`. But
relative paths break when the parser's working directory differs from
the CLI's.

### Design

**Option A: Resolve `--lib` paths to absolute** (recommended)

In `main.rs` after flag parsing (before line 1510), canonicalize all
`lib_dirs` entries:

```rust
let lib_dirs: Vec<String> = lib_dirs
    .into_iter()
    .map(|d| std::fs::canonicalize(&d)
        .unwrap_or_else(|_| std::path::PathBuf::from(&d))
        .to_string_lossy()
        .into_owned())
    .collect();
```

**Option B: Auto-add project lib/ to search path**

When the source file is inside a project directory (has `loft.toml`
or a `lib/` sibling), automatically add `lib/` to `lib_dirs`. The
`test` subcommand already does this (main.rs:1249-1261).

**Recommendation: Do both.** Option A fixes the immediate bug. Option B
makes `use` work without explicit `--lib` flags.

### Files

- `src/main.rs:1153-1155` (--lib parsing)
- `src/main.rs:1450-1510` (lib_dirs setup before parser)
- `src/parser/mod.rs:2052-2170` (lib_path search)

### Verification

```bash
cargo run --bin loft -- --lib lib /tmp/test.loft           # interpreter
cargo run --bin loft -- --lib lib --native /tmp/test.loft   # native
```

Both must resolve `use random` and run successfully.

---

## Step 2: Wire `--native` as default

### Design

**main.rs changes (lines 1100-1210):**

1. Initialize `native_mode = true` (was `false`)
2. Add `--interpret` flag:
   ```rust
   } else if a == "--interpret" || a == "--bytecode" {
       native_mode = false;
   }
   ```
3. Keep `--native` as no-op (already default)

**Rustc fallback (before line 1645):**

Check for rustc before attempting native compilation. If missing,
fall back to interpreter:

```rust
if native_mode {
    // Check rustc availability before committing to native path
    match std::process::Command::new("rustc").arg("--version").output() {
        Ok(_) => {} // proceed with native
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("Warning: rustc not found, falling back to interpreter");
            native_mode = false;
        }
        Err(e) => {
            eprintln!("Warning: rustc check failed ({e}), falling back to interpreter");
            native_mode = false;
        }
    }
}
```

This goes BEFORE the native codegen block (line 1645) but AFTER
bytecode compilation (line 1526), so the interpreter path is ready.

**Help text update:**

```
loft [options] <file>
  Native compilation is the default. Use --interpret for bytecode mode.
  
  --interpret          run in interpreter/bytecode mode instead of native
  --native-release     native compilation with optimizations
  --native-emit <file> generate Rust source without compiling
```

### Files

- `src/main.rs:1100-1210` (flag handling)
- `src/main.rs:1645` (native mode check)
- `src/main.rs:1870-1936` (help text)

### Verification

```bash
cargo run --bin loft -- program.loft          # runs native (default)
cargo run --bin loft -- --interpret prog.loft  # runs interpreter
# On a system without rustc:
cargo run --bin loft -- program.loft          # falls back to interpreter
```

---

## Step 3: Validate packages in native mode

### Design

**New Makefile target:**

```makefile
test-packages-native:
	@pass=0; fail=0; total=0; \
	for pkg in lib/*/; do \
	  for f in $$pkg/src/*.loft $$pkg/tests/*.loft; do \
	    [ -f "$$f" ] || continue; \
	    total=$$((total + 1)); \
	    if $(LOFT) --native "$$f" 2>&1 | grep -q "^Error\|panicked"; then \
	      echo "  FAIL $$f"; fail=$$((fail + 1)); \
	    else \
	      echo "  ok $$f"; pass=$$((pass + 1)); \
	    fi \
	  done \
	done; \
	echo "$$total package tests, $$fail failed"
```

**Expected issues and fixes:**

| Package | #native funcs | Status | Action |
|---------|--------------|--------|--------|
| random | 3 | Built-in (`n_rand` etc.) | Should work |
| graphics | 45 | Has rlib + `[native] crate` | Test linking |
| server | 12 | Has `#native` | Needs `[native] crate` in loft.toml |
| crypto | 6 | Has `#native` | Needs `[native] crate` in loft.toml |
| imaging | 2 | Has `#native` | Needs `[native] crate` in loft.toml |
| web | 2 | Has `#native` | Needs `[native] crate` in loft.toml |
| shapes | 0 | Pure loft | Should work |
| arguments | 0 | Pure loft | Should work |

Packages missing `[native] crate = "..."` in loft.toml will get
`todo!("native function ...")` stubs. Add the crate field for each.

### Files

- `Makefile` (new target)
- `lib/*/loft.toml` (add `[native] crate` where missing)

### Verification

```bash
make test-packages          # interpreter: 16/16
make test-packages-native   # native: 16/16
```

---

## Step 4: Game validation

### Design

Test the Brick Buster game in native mode:

```bash
cargo run --bin loft -- --native lib/graphics/examples/25-brick-buster.loft
```

### Requirements

The graphics package has 45 `#native` functions and a compiled rlib.
The `loft.toml` already has `[native] crate = "loft-graphics-native"`.
The codegen should emit `loft_graphics_native::symbol()` calls via
`output_native_direct_call`.

### Expected issues

1. **OpenGL context**: Native binary needs the same GL context setup
   as the interpreter. The `gl_create_window` native function must
   link correctly.
2. **Frame yield**: The interpreter's `frame_yield` mechanism pauses
   at `gl_swap_buffers()`. Native code needs equivalent — probably
   a loop calling the swap function directly.
3. **Asset paths**: Texture/shader paths must resolve relative to the
   script, not the binary.

### Files

- `lib/graphics/native/src/lib.rs` (native GL bindings)
- `lib/graphics/src/graphics.loft` (45 #native declarations)
- `src/generation/dispatch.rs` (native call dispatch)

---

## Step 5: Performance baseline

### Design

Use the existing benchmark suite at `bench/run_bench.sh`:

```bash
cd bench && ./run_bench.sh
```

This runs 10 benchmarks comparing Python, loft interpreter, loft
native, and Rust reference implementations.

**Key metrics to validate:**

| Benchmark | Expected native/interpreter ratio |
|-----------|----------------------------------|
| Fibonacci | 10-50x faster |
| Sum loop | 20-100x faster |
| Sieve | 10-50x faster |
| String build | 2-10x faster |
| Matrix mul | 10-50x faster |

**If native is slower than expected:**

Profile it: `make profile ARGS="--native prog.loft"`, and
`PROFILE_FLAGS="--annotate"` for the hot source lines
([PERFORMANCE.md § Profiling a run](PERFORMANCE.md)). The profiler builds the
binary once unprofiled and records with `--native-debug`, so the report names
your `n_<fn>` symbols rather than rustc and LLVM. Common issues:
unnecessary store allocation, bounds checks in tight loops, string
allocation overhead.

### Files

- `bench/run_bench.sh`
- `doc/claude/PERFORMANCE.md` (update with results)

---

## Step 6: Documentation cleanup

### Changes

| File | Update |
|------|--------|
| `CLAUDE.md` | Key commands: remove `--native` from examples (it's default) |
| `doc/claude/DEVELOPMENT.md` | Native-first workflow |
| `doc/claude/PROBLEMS.md` | Mark P61 fixed, update P79 status |
| `doc/claude/NATIVE.md` | Update architecture for default mode |
| `CHANGELOG.md` | Native-as-default entry |
| `--help` output | "Native compilation is the default" |

---

## Risk assessment

| Risk | Mitigation |
|------|------------|
| rustc not installed | Auto-fallback to interpreter with warning |
| Compilation slow for large programs | Binary caching (already works) |
| Native binary larger than needed | `--native-release` strips + optimizes |
| Edge case fails only in native | Run both native + interpreter in CI |
| External crate version mismatch | Pin in loft.toml, validate at parse |
| WASM builds can't use native | WASM path is separate (`--native-wasm`) |

---

## Success criteria

1. `loft program.loft` compiles and runs natively by default
2. `loft --interpret program.loft` runs the interpreter
3. All 108 native tests pass
4. All 16 package tests pass in native mode
5. Brick Buster game runs natively with OpenGL
6. Graceful fallback when rustc is missing
7. No performance regression vs interpreter


---

# Native-artifact identity & cache coherence

> Why a freshly-built `#native` package can still abort a native consumer's link
> with `found crates (libloading and libloading) with colliding StableCrateId
> values`, and the identity model that prevents it. (Investigated 2026-06-15 on
> the `crawler`/graphics consumer; `libloading` was the concrete collider.)
>
> **Resolution (see the end of this section): link the package's cdylib by C-ABI so
> its Rust deps stay private — the StableCrateId class is eliminated by construction.
> The build-identity exploration below is recorded but SUPERSEDED.**

## The shape of the problem

A native consumer (`loft --native` / `--check`) generates a Rust program and
compiles it, linking against `loft`, each `#native` package's prebuilt rlib
(`loft_graphics_native`, …), and those packages' transitive deps. Some deps are
**shared** with loft's own — sharply `libloading` (loft dlopens cdylibs with it;
a graphics stack pulls it via `glutin` for GL). rustc requires that any crate
appearing twice in one link have ONE identity: same name+version => same
`StableCrateId` => the copies must be byte-identical (same SVH). Two `libloading
0.8.9` rlibs built under **different RUSTFLAGS** (loft's `.cargo/config` `mold`
link-arg vs a package built `-g`) share a `StableCrateId` but differ in SVH ->
**collision, link aborts**. Same mechanic, rustc-version trigger -> `E0514 ...
incompatible version of rustc`.

## Two identities, two jobs - keep them separate

An artifact carries two orthogonal properties. Collapsing them into one staleness
number is the root of every recurrence; the fix is to give each its own job.

| | **Build identity** - *how* it was built | **Codegen hash** - *what* loft generates |
|---|---|---|
| Captures | rustc version + effective RUSTFLAGS | loft-ffi source / the generated-ABI surface (`LOFT_FFI_FINGERPRINT`) |
| Its job | **pick the right rlib for the job, fast** - the one built for *my* toolchain | **the deeper test** - has loft's *code generation* changed since this was built? |
| How checked | a path lookup (`.../<build-id>/` present?) - no hashing | a content-hash compare |
| When | **always**, first - you can never link the wrong-toolchain rlib | only to validate an already-selected rlib |
| Resolved by | building *that* identity if its slot is empty | rebuilding when loft's codegen moved |

The two answer different questions: build identity decides **which** artifact can
even link with me; the codegen hash decides **whether** that artifact is current.
We **always want the right rlib for the job, quickly** (a path-keyed lookup), and
the **hash is the deeper test - it fires when our code generation changes** (a new
loft-ffi ABI / generated-code contract), not on every run.

Today both are collapsed into one number: the staleness key is
`loft_ffi_fingerprint` (the loft-ffi *source* hash, deliberately invariant across
debug/release so a CI job reuses one `~/.loft/build-cache`). It carries the codegen
identity but is **blind to rustc + flags** - so a `-g` loft and a `mold` loft are
one identity, a package built by one is judged fresh by the other, its shared deps
keep the wrong flags -> collision. Confirmed: graphics' `.loft-build-fp` stamp
(@23:23) and a fresh stamp from this session's loft are the **same**
`1813746251070023740`, though one's `libloading` is `-g` (246 KB) and the other
`mold` (168 KB).

## When do we load what (today)

1. **Storage** - one rlib per package per profile, at
   `native_target_root(pkg_dir)/release/lib<crate>.rlib` (registry installs
   redirect the root to `~/.loft/build-cache/<pkg>-<ver>/`; monorepo
   `lib/<pkg>/native/` keeps in-tree `target/`). A rebuild **clobbers** the one slot.
2. **Staleness on use** (`add_native_extern_flags`, `src/native_utils.rs`) - rebuild
   iff the rlib is missing OR `!native_artifact_fingerprint_matches(dir,
   loft_build_fingerprint())`.
3. **Build gate** (`auto_build_native`, `src/extensions.rs`) - rebuild iff the
   `.loft-build-fp` sidecar != `loft_ffi_fingerprint()`; on build, re-stamp and pass
   loft's `LOFT_BUILD_RUSTFLAGS` to the package cargo (#274).
4. **Link** - `--extern` the package rlib, `-L dependency=` its deps, **pin shared
   crates to loft's copy**. The pin fixes the *name*; it cannot override the SVH the
   package rlib's metadata demands (compiled against its own `libloading`, found via
   `-L`) -> collision.
5. **Interpreter path is separate** - the dlopen'd cdylib (`pending_native_libs`)
   never enters the rlib link, which is why the interpreter runs graphics while
   `--native` aborts.

## Failure paths

| # | Failure | Trigger | Why today's gate misses it |
|---|---|---|---|
| F1 | `colliding StableCrateId` | shared dep built under different RUSTFLAGS (package vs consumer-loft) | key is flag-blind |
| F2 | `E0514 incompatible rustc` | shared dep / `loft` built by a different rustc | key is rustc-blind |
| F3 | `E0463 can't find crate` | no rlib for this target/identity was ever built (e.g. the wasm rlibs) | not every artifact kind is auto-built |
| F4 | silent stale reuse | codegen hash matches, build identity differs (F1/F2's mechanism) | no build-identity comparison |
| F5 | `fp == 0` "match anything" | loft can't hash its own rlib -> matcher returns `true` | a fallback that reuses *any* artifact |
| F6 | two-check inconsistency | `add_native_extern_flags` checks `loft_build_fingerprint`; the stamp uses `loft_ffi_fingerprint` | masked because `auto_build_native` is the real gate |

## Can we have multiple rlibs beside each other?

- **In one link - NO.** rustc forbids two crates sharing a `StableCrateId`; a
  consumer compile may contain exactly one `libloading`. This is why "namespace
  incidental deps to coexist" cannot be applied uniformly - RUSTFLAGS-level
  namespacing would also split `loft`/`loft-ffi`, whose `Stores`/`DbRef` types
  **must** unify across loft and every package (the *contract* crates are
  non-negotiably shared, one identity).
- **In the cache - YES, and that is the fix.** Key storage by **build identity**:
  `~/.loft/build-cache/<pkg>-<ver>/<build-id>/release/lib<crate>.rlib`, where
  `<build-id>` is a short token of (rustc-version + effective RUSTFLAGS). N
  toolchain/flag-sets then coexist as N cached artifacts; the consumer loads the
  one matching **its** identity, building it on demand. Switching toolchains stops
  clobbering (no churn) **and** stops colliding (the loaded artifact's shared deps
  match the consumer by construction).

## The design - build identity selects (fast, always); codegen hash validates (deep)

> **SUPERSEDED** by the Resolution at the end.  Explored and partly built (it fixes
> flag-axis collisions like `libloading`) but it is whack-a-mole: `log` collides on the
> *profile* axis next.  Kept as the design record so the build-id approach is not
> re-attempted.

Two separate, legible axes - never folded into one number:

- **Build identity = the fast selector.** The consumer computes its `<build-id>`
  from stamps already present (`LOFT_BUILD_RUSTC`, `LOFT_BUILD_RUSTFLAGS`) and
  resolves the package rlib under that directory - a path lookup, no hashing,
  **always**: *the right rlib for the job, quickly.* An empty slot => build *this*
  identity (leave the others alone). A mismatch is diagnosable: "graphics here is
  rustc-1.95/`-g`; you are rustc-1.96/`mold` - building that variant."
- **Codegen hash = the deeper test.** Within the selected identity, the existing
  `loft_ffi_fingerprint` (source hash) answers *"has loft's code generation changed
  since this rlib was built?"* - the case where even the right-toolchain rlib is
  stale (new loft-ffi ABI / generated-code contract). The expensive, occasional
  check; it fires **when our codegen changes**, not on every run.

So **build identity decides *which* artifact (fast, always); the codegen hash
decides *whether it is current* (deep, on change).** The collision becomes
structurally impossible - a consumer can only ever load an artifact built under its
own toolchain+flags - without forcing one global build (a moving target under the
floating-stable toolchain) and without one-slot clobber-churn.

### Cache lifecycle - discard lazily, after convergence

The identity-keyed slots are a **multi-producer store**, not one mutable slot. Two
producers on different toolchains - this laptop on rustc-1.96/`mold`, a Mac laptop on
1.95 with its own flags - each build and use the slot that is *totally right for
them*; both are live and correct at the same time, and the cache can even be shared
across machines, since each producer's build identity selects its own.

Discarding is therefore **lazy and deferred, never eager**: a slot is not dead just
because it is not *my* current identity - another producer may still be on it. The
old variants become collectable only once the producers **fold to a common rustc**
(the heterogeneity that justified them is gone) and the superseded slots stop being
touched. So prune by **disuse** - a last-access timestamp bumped on every load,
reaping slots untouched past a threshold (or keep-N-recent) - **not** by
"not-the-current-one." A still-diverged producer is never robbed of its working
artifact, and cleanup happens on its own after convergence.

### What it closes
F1/F2/F4 structurally (an incompatible artifact is a *different cache slot*, never
loaded). F5 - the build-identity path is derived from stamps, so "match anything"
stops gating shared artifacts. F3/F6 - route every artifact kind (incl. the wasm
rlibs) through the same identity-keyed resolution and use one key on both sides.

### Implementation touch points (not yet built)
- `src/cache.rs`: add `build_identity()` -> short token (rustc + flags); keep
  `loft_ffi_fingerprint` as the codegen freshness hash - **do not fold them.**
- `src/extensions.rs` (`auto_build_native`, `native_target_root`): key the package
  target dir by `build_identity()`; build + stamp per identity.
- `src/native_utils.rs` (`add_native_extern_flags`): resolve under the consumer's
  `build_identity()`; align its freshness key with the stamp (closes F6).
- Wasm rlib paths (Makefile + `tests/html_wasm.rs`) join the scheme so F3 stops
  being a manual rebuild.
- GC by **disuse** (a last-access reaper, e.g. `loft cache gc` / age-out), never an
  eager clobber - heterogeneous producers coexist and a superseded build identity is
  reaped only once a common-rustc convergence stops touching it.
- **Discarded alternative:** folding rustc+flags into `loft_ffi_fingerprint` (one
  number). It detects the mismatch but forces a clobber-rebuild on every toolchain
  switch (churn) and conflates "which artifact" with "is it fresh" - the opposite of
  the fast-select / deep-validate split.

---

## Resolution: separate the API id from the Rust part (link the cdylib by C-ABI)

The build-identity design above (rebuild the package so its shared deps MATCH loft's) was
implemented and partly works — but it is whack-a-mole, and that is structural.  Verified
end-to-end on the crawler:

- keying rebuilt graphics into its slot with loft's flags (#274) and **`libloading`
  vanished** — it was a *flags* mismatch (`-g` vs `mold`).
- A NEW collision surfaced at once: **`log`** — same version/features/rustflags/rustc, but
  a different **profile** hash.  The next would be build-script reproducibility.  Matching
  *every* SVH-affecting input is the fragile path.

**The reframe:** the collision exists ONLY because native-compile links the package as a
Rust *rlib* (`extern crate loft_graphics_native`), pulling its whole crate graph into the
consumer's rustc link where it overlaps loft's.  Link the package's **cdylib by C-ABI**
instead and that graph is sealed in the `.so` — it never enters the link, so the class is
gone BY CONSTRUCTION, for any toolchain / flags / profile / reproducibility.

| | **API id** — crosses the boundary | **Rust part** — stays private |
|---|---|---|
| What | loft-ffi C-ABI: `gl_*` as `extern "C"` symbols + loft-ffi types as opaque pointers | the package's crate graph (`libloading`, `log`, `glutin`, ...) + its rustc/flags/profile |
| Must match | only the **loft-ffi version** (already gated by the codegen hash) | **nobody** — sealed in the `.so` |

Proven by the rebuilt artifact: `gl_*` are exported as plain C symbols
(`#[no_mangle] pub unsafe extern "C"`, via loft-ffi-macros), and `nm -D ...so | grep -c
'U .*(libloading|log)' == 0` — the deps are statically bundled inside the `.so`, not
leaked.  Linking it brings ZERO Rust crates into the consumer.  It is also what the
**interpreter already does** (dlopen the `.so`, call the C symbols), so this UNIFIES the
two native-package paths; the codegen even has a non-rlib call mode already (the
wasm-browser path emits `#[link(...)]` imports, not `krate::sym`).

### Consequence: the build-identity axis disappears for native packages
With the Rust part private, the package's rustc/flags/profile are **irrelevant** — so the
build-identity keying, the multi-producer cache lifecycle, and the lazy GC above are all
**unnecessary**.  F1/F2/F4 close *structurally* (no shared rlib to collide), not by
matching.  The only remaining staleness axis is the **codegen hash** (loft-ffi ABI),
which already gates the cdylib.

### The codegen change — SHIPPED (host-native executable backend)
The native executable backend (`output_native` / `output_native_reachable`) links a `[native]
crate` package's cdylib `.so` by C-ABI; it no longer pulls the package's rlib into the
consumer's rustc link, so the StableCrateId collision class is gone by construction.  Gated on
`Output::native_cabi` (set by `native_utils::native_cabi_enabled()` — on everywhere except
Windows, which stays on the rlib path until C-ABI dylib import-library linking is built).  The
codegen and the linker flags read the *same* helper so they never disagree.

- **`emit_file_header`** (`native_cabi` arm): emits `unsafe extern "C" { … }` declaring each
  body-less `#native` fn (`code() == Null`) that belongs to a `[native] crate` package (has a
  `native_symbol_crates` entry — a stem/dlopen native, `[library] native = "…"`, has none and
  stays on its existing route, not declared/linked here).  The signature is derived from the
  loft types to match exactly what `output_native_direct_call` marshals: a `loft_ffi::LoftStore`
  first param when the fn writes the store (a Reference arg, or a Vector/Reference return);
  text → `*const u8, usize`; vector → `*const ELEM, u32`; Reference → `loft_ffi::LoftRef`; text
  return → `loft_ffi::LoftStr`; Vector/Reference return → `loft_ffi::LoftRef`; scalar widths per
  `is_wide`.  **No reachability filter** — the fn emitter emits a wrapper (a native call) for
  *every* body-less native in range, so each needs a decl (a genuinely-uncalled one is a
  harmless dead extern).  **`#[link_name = "<sym>"]` + a `__cabi_<sym>` local alias** because a
  bare `#native` defaults the symbol to the fn's own `n_<name>`, which would otherwise shadow
  the generated `n_<name>` wrapper (E0428).
- **call site**: `native_cabi` emits the unqualified `__cabi_<sym>` alias (resolved by the
  link), not `krate::sym`.  `output_native_direct_call`'s body is unchanged — its
  `transmute_copy`s are now harmless identities (the consumer has only loft's `loft_ffi`, named
  via `--extern loft_ffi`, since the cdylib's copy is sealed in the `.so`).
- **which symbol** (loft#907): `#[link_name]` names the fn the LIBRARY says implements the
  binding, which is not always the `#native` string.  See § Which symbol a `#native` binding
  links, below.
- **link** (`native_utils::add_native_extern_flags`, `target.is_none() && native_cabi_enabled()`):
  `-L native=<so dir> -l dylib=<stem> -Clink-arg=-Wl,-rpath,<so dir>`, not `--extern
  <ident>=<rlib>`.  Keyed on **`loft_ffi_fingerprint()`** (the ABI hash, matching what
  `auto_build_native` stamps) — NOT `loft_build_fingerprint`: the `.so` is ABI-sealed and stays
  valid across loft rebuilds.
- The rlib is **still built** (`auto_build_native` builds `.so` + rlib) and still *linked* by
  wasm32-wasip2 (cross-compiled rlib), the library-cdylib path, and Windows.  Dropping the rlib
  build awaits converting those remaining paths; the win here is removing it from the native
  executable consumer's link, the one place the collision arose.
- The trade: the `.so` is a runtime/dynamic dependency (RPATH / shipped beside the binary)
  rather than statically baked in — the same model the interpreter already needs.

**Verified** (2026-06-15): a store-mutating round-trip — imaging `save_png` + `load_png` (both
Reference-arg/`LoftStore`) — native-compiles, links `libloft_imaging.so` by C-ABI, runs the
store mutation through the handle, and prints output identical to `--interpret`
(`saved=true reloaded=2x1`).  Full native test suite green.

**Remaining paths + residual gaps** — the library-cdylib path (the viewer collision),
wasm32-wasip2, and Windows still link the rlib; plus same-symbol cross-package
disambiguation, `make install` `.so` packaging (`$ORIGIN` RPATH + copy), the
boolean→`u8` ABI, and the `prebuilt/` + `fp == 0` resolution edges — are tracked in
**@PLN26** ([loft-lang/plans#26](https://github.com/loft-lang/plans/issues/26)).

### Which symbol a `#native` binding links (loft#907)

**`#native "sym"` is an API id, not the name of the Rust fn behind it.**  A native library
registers its implementations by loft symbol —
`loft_register_bridges! { "sym" => other__loft_bridge }` — and nothing requires `other == sym`.
So a `#native` string names a *binding*; only the library's own table names the *function*.

The two backends used to answer that differently, and the disagreement was silent:

| | how it resolved `#native "sym"` |
|---|---|
| `--interpret` | the loaded cdylib's `loft_register_bridges_v1` table — the library's own answer |
| `--native` (before) | `#[link_name = "sym"]` — whatever the cdylib exports under that name |

A C-ABI link matches on **name alone**, so where the two differed the call was marshalled into a
different function with no error, no warning and no failed link.  In the published `graphics`
that was ten functions: each store-aware one has loft's `(LoftStore, LoftRef)` entry point at
`n_<x>` and an older raw `(ptr, count)` fn under the `#native` name, so arguments arrived shifted
by a register — `save_png` answered `false` and wrote nothing, the WebGL upload calls uploaded
nothing.

**Both backends now read one source.**  `extensions::resolve_native_impl_symbols` runs after
`load_all` and before any codegen, asks the loaded cdylibs which fn implements each symbol
(`dladdr` on the registered bridge pointer names it — `#[loft_native]` emits `X__loft_bridge`
beside `X`, so stripping the suffix gives the implementation), and records **only the differing
entries** in `Data::native_impl_symbols`.  `Data::link_symbol` is the one accessor codegen emits
through, on both the C-ABI `#[link_name]` and the rlib `krate::sym` path.

- A **clean binding** — the loft symbol equals the implementing fn — maps to itself and nothing
  changes.  That is the only shape `loft-ffi-build`'s generator can produce, so a library that
  derives its registration from its `#native` annotations is clean by construction; a remap only
  ever comes from a hand-written `loft_register_bridges!` list.
- A cdylib that is **absent, un-loadable, or predates the bridge registry** cannot be resolved and
  keeps the literal name.  Not a silent wrong answer: the interpreter reports the missing
  registration at load (loft#886) and a call panics rather than answering.
- `dladdr` is required to hit the bridge **exactly** (`dli_saddr == ptr`); it otherwise reports the
  nearest preceding symbol, which would hand codegen a neighbouring function's name.  A near miss
  leaves the symbol alone.

Guards: `tests/lib/native_remap_pkg` is a `[native] crate` fixture in this shape which exports a
DECOY under each `#native` name, so a regression *answers* (-1000 / -2000) instead of failing to
link — the way this reached users.  `native_scalar_pkg` is the clean-binding control.
