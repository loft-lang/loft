<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN102 — `par` captured state is read-only (fix scope)

> **Status: IMPLEMENTED for `par(worker, n)` writes (2026-07-14).** Owner ruling: the parent
> state a `par` worker captures is read-only; a write to it is a compile error — *no runtime
> error, ever* (@C80): a data race can't be resolved to a null, so it is DISALLOWED, not defined.
> Decision record: [DESIGN_DECISIONS.md @C93](../../DESIGN_DECISIONS.md).
>
> **Done:** a raw `OpSet*` write to a captured (non-local param) reference/struct/vector inside a
> `par(worker, n)` worker (direct or transitive) is now a **clean compile error** at the
> par-safety check (`scopes.rs::worker_calls_parent_write_deep` + `raw_write_to_captured`),
> replacing the `codegen.rs:3235` slot-panic. Reads / worker-locals / `HostIo`/`Prng` still
> compile. Test `tests/scripts/pln102-c93-par-write.loft`.
>
> **Two par constructs, handled separately:** `parallel { }` (the arm form) already cleanly
> rejects enclosing-scope writes ("cannot write or mutate enclosing-scope variable" —
> `tests/scripts/170-parallel-capture-soundness.loft`); this change covers the `for … par(worker,
> n)` FOLD form, whose worker-fn param-writes used to crash.
>
> **Read residual — RESOLVED (2026-07-15) by disallowing captured references.** A par worker runs
> on an ISOLATED store clone, so a captured *reference* arg (a heap DbRef) never crossed: it read
> empty (silent-wrong) when element-first, or slot-panicked (`codegen.rs:3235`) when mis-ordered.
> Per *no runtime errors, ever* — and the strict-`par` philosophy — this is DISALLOWED, not made
> to work (cross-clone deep-copy would be a capability add; the owner's line is "current par
> covers my needs, new capability via new constructs"). `parse_parallel_worker_fn`
> (`builtins.rs`) now emits a clean compile error: (A) the element param (param 0) must match the
> loop element's reference-vs-scalar kind — catches the element passed as a non-first arg; (B) a
> captured (context) param that is a reference (`vector`/`struct`/`hash`/…/`text`) is rejected —
> "pass a scalar, or read the value into a scalar before the loop (only the loop element may be a
> reference)". Captured SCALARS still cross by value; the loop ELEMENT may be a reference (the
> dispatcher copies each element — 22-threading's `const Score` element still works). Test
> `tests/scripts/pln102-c93-par-captured-ref.loft`.

## The current behaviour (verified 2026-07-14, interpret)

An impure `par` worker — one that writes captured parent state — is neither cleanly rejected nor
cleanly raced. It **crashes codegen**:

```loft
struct Shared { n: integer }
fn bump(s: Shared, x: integer) -> integer { s.n = s.n + x; x * 2 }
s = Shared { n: 0 };
for x in [1,2,3,4,5,6,7,8] par(r = bump(s, x), 8) { total += r; }
// thread 'main' panicked at src/state/codegen.rs:3235: Incorrect var x[65535] versus 176
```

(A shared `vector` written by `acc[0] = acc[0] + x` panics the same way.) A runtime crash is
exactly what the platform never does — this is the bug the ruling closes.

## The machinery that already exists (and the gap)

- **`Purity`** (`src/data.rs:2457`) per definition: `Unknown` (default) / `Pure` / `Impure(cat)`,
  set by `#pure` / `#impure(category)` (`parser/definitions.rs:1409`). **`Unknown` is conservatively
  treated as `Impure(ParentWrite)`** by the analyser.
- **`ImpureCategory::ParentWrite`** (`data.rs:2489`) — "writes a parent-side store via its first
  argument … compile error in `par` workers when the first arg is non-local." `HostIo`/`Prng`/`Io`
  stay allowed (the host serialises).
- **`is_par_safe(data, d_nr)`** (`scopes.rs:5336`) + the **phase-5b deep check at ERROR level**
  (`collections.rs:2762`): a worker calling `Impure(ParentWrite|ParCall)` is rejected.

**The gap:** a *user* worker that writes an **aliased reference/vector parameter** (`bump`'s `s`, an
alias of the parent's `s`) slips past this check and reaches codegen, where it slot-panics. So the
rejection either does not fire for this shape or fires too late (after codegen has begun). Pinning
*which* is scope step 1.

## The invariant (data-centric — the ruling)

> Every variable a `par` worker captures from its enclosing scope is **read-only inside the
> worker**. A write to it — directly, or via a call binding it to a writing parameter — is a
> compile error reported **at the write**. The worker may READ captured state, READ the element,
> use mutable locals, and RETURN a value (folded sequentially). Only captured *parent* state is
> frozen.

This makes the race **unexpressible**, not merely detected — the same shape as host/param
read-only-by-default (the sandbox model), applied to `par` capture.

## Two facets of the fix

1. **Close the crash (must land regardless).** The impure-worker rejection must fire at
   **parse/type time, before codegen** — never the `codegen.rs:3235` panic. Determine why the
   phase-5b deep check misses the aliased-ref-param user-worker (Step 1) and make it catch that
   shape.
2. **The read-only-capture diagnostic (the ruling's expression).** Mark each captured parent var
   **read-only for the duration of the worker body**, so a write to it is caught by the *existing*
   write-to-read-only machinery — the same path host-data/`const` writes already error through —
   and the message points at the write (`s.n = …` → "cannot write captured `s` inside a `par`
   worker; it is read-only here"), not at an abstract purity verdict. This subsumes facet 1: a
   read-only capture cannot reach the crashing codegen shape.

## Implementation steps (each with its verification)

**Step 0 — instruments.** A corpus of `par` workers: pure (baseline, must still run); direct
capture-write (`cap[i] = …`, `cap.f = …`); indirect via a call (`writes(cap, x)`); the benign
allowed impurities (a worker that `print`s = `HostIo`; a `random_*` = `Prng`); read-only reads of
capture (`cap[i]` read, `cap.f` read — must stay legal). *Verify:* record current behaviour — pure
runs, the two writes CRASH, reads work, HostIo/Prng work.

**Step 1 — locate the miss.** Instrument the phase-5b deep check (`collections.rs:2762` /
`scopes.rs::is_par_safe`) for the crashing worker: does it run for the `for … par(worker, n)` fold
form? Does it classify the user `bump` (`Unknown` → `ParentWrite`)? *Verify:* an env-gated print
shows whether the check fires and its verdict for `bump`.

**Step 2 — mark captured vars read-only in the worker body.** At `par`-worker parse
(`parser/builtins.rs::parse_parallel_worker*`), flag every captured parent var read-only for the
worker's scope (reuse the `const`/host-read-only carrier — see how host params are made read-only).
*Verify:* a direct `cap.f = v` in the worker reports "cannot write captured `cap` … read-only in a
`par` worker" AT THE WRITE, on both backends; a `cap.f` READ still compiles.

**Step 3 — the indirect (aliased-param) write.** Ensure a worker calling a fn that writes its
reference/vector param with a captured argument is rejected — either because the callee is
`Impure(ParentWrite)` (the existing rule, once Step 1's gap is closed) or because passing a
read-only capture where the callee writes it is itself the read-only violation. *Verify:*
`writes(cap, x)` in a worker → clean compile error, both backends; NO codegen panic.

**Step 4 — benign impurity stays allowed.** *Verify:* a worker that `print`s (`HostIo`) or calls
`random_int` (`Prng`) still compiles + runs (the host serialises) — the split is preserved.

**Step 5 — corpus + suite.** *Verify:* the impure cells are clean compile errors (no crash), the
pure/read/HostIo/Prng cells run, both backends; full suite green (the existing `par` scripts —
`160-*`, the @PLN90 W6 par regressions — must stay green). Graduate to
`tests/scripts/pln102-par-capture-readonly.loft` (with `@EXPECT_ERROR` cells for the writes).

## Scope / effort

**S–M.** Mostly *closing a gap + wiring a read-only flag* on machinery that already exists
(`Purity`/`ParentWrite`/`is_par_safe`/the host-read-only carrier); no new runtime op, no runtime
behaviour — the whole change is a compile-time rejection that replaces a codegen crash. The
uncertainty is Step 1 (why the deep check misses the aliased-param user-worker), which the
instrument settles before any fix.

## See also

- [DESIGN_DECISIONS.md @C93](../../DESIGN_DECISIONS.md) — the decision.
- [THREADING.md](../../THREADING.md) — `par` semantics.
- [COMPATIBILITY.md](../../COMPATIBILITY.md) § the error surface — an error-ADD only lands pre-freeze.
- The platform rule: *no runtime errors, ever* — disallow, or degrade to null; a race can only be
  disallowed.
