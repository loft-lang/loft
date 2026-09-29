<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# PLANNING — stability and compiler-correctness items

Split from [PLANNING.md](PLANNING.md) by subject: the S-tier stability hardening items and the slot, closure, name-clash and match-arm items (P70, C43, C47, C48, C52, C53).  Shipped items are in [PLANNING-history.md](PLANNING-history.md).

---

## S — Stability Hardening

Items found in a systematic stability audit (2026-03-20).  Each addresses a panic,
silent failure, or missing bound in the interpreter and database engine.  All target 0.8.2.

---

### S6  Fix remaining "recursive call sees stale attribute count" cases
**Sources:** @P271 (the surviving "Too few parameters" observation — seen once, not
reproducible).  The original PROBLEMS.md #84 (the merge-sort use-after-free
manifestation) was fixed in 0.8.2 and is no longer a separate row.
**Severity:** Medium — the merge-sort use-after-free (the primary manifestation) was fixed in 0.8.2.  Complex mutual-recursion patterns that trigger `ref_return` on a function after its recursive call sites were already compiled may still produce wrong attribute counts.
**Description:** `ref_return` adds work-ref attributes to a function's IR while the body is still being parsed.  When the function is recursive, call sites parsed before `ref_return` runs see the old (smaller) attribute count.  The merge-sort case was fixed by guarding `vector_needs_db` with `!is_argument` and injecting the return-ref in `parse_return`.  A general fix would scan the IR tree after the second parse pass and patch under-argument recursive calls via `add_defaults`.
**Fix path:** Post-parse IR scan and call-site patching in `parse_function`.
**Effort:** Medium
**Target:** 1.1+

---

### S19  Fix #85: struct-enum locals not freed in debug mode
**Sources:** PROBLEMS.md #85, CAVEATS.md C16
**Severity:** Low in production (no assertion), critical in debug builds (SIGABRT).
**Description:** `scopes.rs::free_vars()` emits `OpFreeRef` for plain struct local variables but not for struct-enum locals.  In debug builds, the store's allocation assert fires at scope exit because the record is still live.
**Fix path:**
1. In `get_free_vars` (or equivalent), add a branch for `Type::Named(_, _, _)` that is a struct-enum variant — emit `OpFreeRef` exactly as is done for plain structs.
2. Regression test: declare a local struct-enum variable inside a `for` or `if` body; verify no assertion fire in debug, value correct in release.
**Effort:** Small
**Target:** 0.9.0

---

### S20  Fix #91: init(expr) circular dependency silently accepted
**Sources:** PROBLEMS.md #91, CAVEATS.md C18
**Severity:** Medium — silent undefined behaviour at runtime when two store fields form a mutual initialisation cycle.
**Description:** The `init(expr)` attribute on struct fields is evaluated at record creation time.  If field A's init expr reads field B and field B's init expr reads field A, the interpreter reads uninitialised memory.  No cycle check is performed.
**Fix path:**
1. After all struct field defs are parsed, build a dependency graph: edge A→B if field A's init expr contains a read of field B.
2. DFS cycle detection over the graph; emit a compile error naming the cycle.
3. Test: two mutually-referencing `init(...)` fields produce a clear error; acyclic chains are unaffected.
**Effort:** Small
**Target:** 0.9.0

---

### S21  Fix #92: stack_trace() silent empty in parallel workers
**Sources:** PROBLEMS.md #92, CAVEATS.md C17
**Severity:** Medium — debugging parallel code is significantly harder without stack traces.
**Description:** `stack_trace()` reads `state.data_ptr` to walk the call stack.  In parallel workers spawned by `par(...)`, `execute_at` (and `execute_at_ref`) entry points do not set `data_ptr` before dispatch, so the pointer is null and `stack_trace()` returns an empty vec.
**Fix path:**
1. In `execute_at` and `execute_at_ref` in `src/state/mod.rs`, set `self.data_ptr = data as *const Data;` (or equivalent) immediately before the dispatch call, mirroring what the single-threaded `execute` path does.
2. Regression test: call `stack_trace()` inside a `par(...)` worker body; assert the returned vec is non-empty and contains the worker function name.
**Effort:** Small
**Target:** 0.9.0

---

### S22  Fix parallel worker auto-lock in release builds
**Sources:** SAFE.md § P1-R1, CAVEATS.md C22
**Severity:** Medium — release builds silently return wrong results when a worker writes to a `const` argument.
**Description:** The auto-lock insertion (`n_set_store_lock`) for `const` worker arguments is guarded by `#[cfg(debug_assertions)]` in `parser/expressions.rs`.  Release builds never lock the input stores, so a buggy worker that accidentally mutates a `const` argument silently discards the write into a 256-byte dummy buffer and continues with stale data.
**Fix path:**
1. Remove the `#[cfg(debug_assertions)]` guards from the two auto-lock insertion sites in `parse_code` and `expression` that emit `n_set_store_lock` for `const` parameters and local const variables.
2. In `addr_mut` (`store.rs`), change the release-build dummy-buffer path to `panic!("write to locked store")` — no legitimate code path should hit it once auto-lock is unconditional.
3. Add an integration test that runs a `par()` loop whose worker attempts to push to its `const` input in release mode; assert the panic fires with a clear message.
**Effort:** Small
**Target:** 0.8.3

---

### S23  Compiler + runtime: reject `yield` inside `par()` body
**Sources:** SAFE.md § P2-R6, CAVEATS.md C25, COROUTINE.md § SC-CO-4
**Severity:** Medium — `yield` or generator calls inside `par(...)` produce out-of-bounds panics or silent wrong results depending on frame-index collision.
**Description:** No compiler check prevents `yield` or calls to `iterator<T>`-returning functions inside `par(...)` bodies.  Worker `State` instances hold only a null-sentinel `coroutines` table; a DbRef produced by the main thread indexes into it incorrectly.
**Fix path:**
1. In `src/parser/collections.rs` (parallel-for desugaring) and wherever `par(...)` body parsing begins, add an `inside_par_body: bool` flag to the parser context.
2. In `parse_yield` and any site that resolves a function call returning `iterator<T>`, emit a compile error when `inside_par_body` is true.
3. In `coroutine_next` (`state/mod.rs`), add a bounds check: `if idx >= self.coroutines.len() { panic!("iterator<T> DbRef out of range in worker") }`.  This defence-in-depth guard catches the case where the compiler check is missing.
4. Test: a loft program that calls a generator inside `par(...)` produces a compile error; one that bypasses the check triggers the runtime guard in debug.
**Effort:** Small
**Target:** 0.8.3

---

### S24  Compiler + runtime: reject `e#remove` on generator iterator
**Sources:** SAFE.md § P2-R9, CAVEATS.md C26, COROUTINE.md § SC-CO-11
**Severity:** Medium — release builds silently corrupt a real store record; debug builds panic with an uninformative out-of-bounds message.
**Description:** `e#remove` on a generator-typed loop variable passes a DbRef with `store_nr == u16::MAX` (the coroutine sentinel) to `database::remove`.  In debug `u16::MAX` overflows `allocations`; in release `u16::MAX % len` selects a real store and the `rec` (frame index ≈ 1–2) deletes a real record.
**Fix path:**
1. In `src/parser/fields.rs` (or wherever `e#remove` is resolved), check whether the loop's collection type is `iterator<T>` (backed by `OpCoroutineCreate`).  If so, emit: `error: e#remove is not valid on a generator iterator`.
2. In `database::remove` (or the calling opcode), add: `if db.store_nr == COROUTINE_STORE { debug_assert!(false, "remove on coroutine DbRef"); return; }`.  The `return` prevents release-build corruption even if the compiler check is missing.
3. Test: `e#remove` on a generator iterator is a compile error; a debug-only test verifies the runtime guard fires if the check is bypassed.
**Effort:** Extra Small
**Target:** 0.8.3

---

### S26  `OpFreeCoroutine` at for-loop exit
**Sources:** SAFE.md § P2-R7, COROUTINE.md § Phase 1
**Severity:** Low — memory growth; `State::coroutines` accumulates one `Box<CoroutineFrame>` per generator invocation forever.
**Description:** `coroutine_return` marks the frame `Exhausted` but never sets the slot to `None`.  The `free_coroutine(idx)` helper is designed but never called.  Programs that create many generators in a loop grow `State::coroutines` without bound.
**Fix path:**
1. In the `for … in gen { }` desugaring codegen, emit `OpFreeCoroutine(gen_slot)` at loop exit (both exhaustion and `break`).
2. Implement `OpFreeCoroutine` in `fill.rs`: call `free_coroutine(idx)` which sets `coroutines[idx] = None`.
3. Optionally, lazily free in `coroutine_exhausted` when it first observes `Exhausted` status (covers the `explicit-advance` API path).
**Effort:** Medium
**Target:** 0.8.3

---

### S27  Coroutine `text_positions` save/restore across yield/resume
**Sources:** SAFE.md § P2-R4
**Severity:** Medium (debug-only) — `text_positions` BTreeSet becomes inconsistent across yield/resume, causing false double-free misses and masking missing `OpFreeText` for unrelated code.
**Description:** `coroutine_yield` rewinds `stack_pos` but does not remove text-local entries from `State::text_positions`.  The orphaned entries interfere with the debug detector for unrelated text frees at the same stack positions.
**Fix path:**
1. In `coroutine_yield` (debug path): collect `text_positions` entries in `[base, locals_end)`, remove them, store in `frame.saved_text_positions: BTreeSet<u32>`.
2. In `coroutine_next` (debug path): re-insert `frame.saved_text_positions` and clear it.
3. In `coroutine_return` (debug path): clear `frame.saved_text_positions` without reinserting.
**Effort:** Small (debug-only path)
**Target:** 0.8.3

---

### S28  Debug generation-counter for stale DbRef detection in coroutines
**Sources:** SAFE.md § P2-R8, COROUTINE.md § SC-CO-2
**Severity:** Medium — a generator resuming after its backing record or store was freed silently reads/writes wrong data with no diagnostic.
**Description:** A `DbRef` live in a generator local at a `yield` point can refer to memory freed or resized by the consumer between iterations.  Worse than ordinary functions: the suspension window spans many `next()` calls.
**Fix path:**
1. Add `generation: u32` to `Store`; increment on every `claim`, `delete`, and `resize`.
2. When `coroutine_create` / `coroutine_yield` saves a `DbRef` to `stack_bytes`, also record `(store_nr, generation_at_save)` in a new `frame.store_generations: Vec<(u16, u32)>`.
3. At `coroutine_next`, verify each saved store's current generation matches; emit a runtime diagnostic on mismatch.
**Effort:** Medium
**Target:** 0.8.3

---

### S29  Parallel store hardening: `thread::scope` + LIFO assert + skip claims
**Sources:** SAFE.md § P1-R2/P1-R3/P1-R4
**Severity:** Low/Medium — three independent low-effort fixes for parallel store infrastructure.
**Description:**
- **P1-R2:** `run_parallel_direct` uses a raw `*mut u8` with a lifetime invariant enforced only by convention; `thread::spawn` + manual join does not give compile-time guarantees.
- **P1-R3:** `clone_locked` copies `self.claims` (all live record offsets) into worker clones that never call `validate()` — wasted O(records) allocation per worker.
- **P1-R4:** `free_named` relies on LIFO store freeing order; out-of-order frees stall `max` and may cause subsequent `database()` to reuse a live slot.
**Fix path:**
1. Replace `thread::spawn` + manual join in `run_parallel_direct` with `std::thread::scope` (Rust 1.63+) to give compile-time lifetime enforcement over `out_ptr`.
2. Add `clone_locked_for_worker` on `Store` that omits `claims: HashSet::new()`; use it in `Stores::clone_for_worker`.
3. Add `debug_assert!(store_nr == self.max - 1, "free() must be called in LIFO order")` in `free_named`.
**Effort:** Small (three independent one-function changes)
**Target:** 0.8.3

---

### S30  `WorkerStores` newtype for type-level non-aliasing
**Sources:** SAFE.md § P1-R5
**Severity:** Low — no current bug; guards against future extensions to the parallel dispatch that could silently allow workers to hold main-thread `DbRef` values.
**Description:** The architecture relies on convention (workers receive cloned stores and may not hold main-thread `DbRef`s) rather than Rust types.  A future refactor extending worker dispatch could silently break the invariant.
**Fix path:**
1. Introduce `WorkerStores(Stores)` newtype, constructible only by `clone_for_worker` (private inner field).
2. Worker closures receive `WorkerStores`; the type is `Send` but not `Sync`, preventing cross-thread sharing.
3. Long-term: add `origin: StoreOrigin` tag to `DbRef` and a debug assert in `copy_from_worker` that all result DbRefs have worker origin, not main-thread origin.
**Effort:** Medium
**Depends:** S29 (clean parallel store state first)
**Target:** 0.8.3

---

## P70 — Text in `generate_set` TOS-override causes SIGSEGV

**Status:** Workaround in place — `Type::Text` excluded from the large-type
TOS-override in `generate_set` (`codegen.rs:~689`).  Stable; no regressions.

**Real fix (only if C43 text slot reuse needs it):** switch `OpFreeText` to
take a variable number instead of a pre-resolved stack offset, so codegen
resolves the final position at emit time — same pattern as `OpFreeRef`.
Touches `scopes.rs` (emit `Value::Var(v)`) and `codegen.rs` (resolve at
emit).  Safe but affects every text-variable scope exit.

**Effort:** Medium.  **Target:** deferred to C43.

---

## C43 — Text slot reuse: zone-2 dead-slot tracking

**Problem:** Text variables (24 bytes each) cannot reuse dead slots, wasting
stack space when many short-lived text variables are used sequentially.

**Root cause:** Text variables are placed by zone 2 (`place_large_and_recurse`
in `slots.rs`), which assigns slots sequentially at TOS without dead-slot
reuse.  Zone 1 has dead-slot reuse but only handles variables ≤ 8 bytes.

**Failed attempt:** A naive same-type reuse check caused slot conflicts
because it only compared against one dead variable, not ALL assigned
variables.  `nums` at [40,52) was still live when `_map_result_5` reused
slot 44.  Full conflict scan (like zone-1) is required.

**Files:** `src/variables/slots.rs`

P70 (text TOS-override) is NOT blocking: text-to-text same-size reuse
places the variable at the dead slot's existing position — no movement
occurs, so the `generate_set` TOS-override path is never triggered.

---

### C43.1 — Zone-2 dead-slot finder with full conflict scan

**Goal:** A standalone `find_reusable_zone2_slot` function that returns a
safe reuse slot or `None`.

**File:** `src/variables/slots.rs`

**Implementation:**
```rust
/// Find a dead zone-2 variable whose slot can be reused by variable `v`.
/// Returns `Some(slot)` if a conflict-free candidate exists, `None` otherwise.
/// Guards: same size, same type discriminant, dead (last_use < v.first_def),
/// no spatial+temporal overlap with any other assigned variable.
fn find_reusable_zone2_slot(
    function: &Function,
    v: usize,
    scope: u16,
) -> Option<u16> {
    let v_size = size(&function.variables[v].type_def, &Context::Variable);
    let v_first = function.variables[v].first_def;
    let v_last = function.variables[v].last_use;
    let v_disc = std::mem::discriminant(&function.variables[v].type_def);
    for (j, jv) in function.variables.iter().enumerate() {
        if j == v || jv.stack_pos == u16::MAX || jv.scope != scope {
            continue;
        }
        let j_size = size(&jv.type_def, &Context::Variable);
        // Same size + same type family (e.g., text-to-text only).
        if j_size != v_size || std::mem::discriminant(&jv.type_def) != v_disc {
            continue;
        }
        // Dead: candidate's last use is before our first definition.
        if jv.last_use >= v_first {
            continue;
        }
        // Full conflict scan: verify no other variable overlaps both
        // spatially (byte range) and temporally (live interval).
        let slot = jv.stack_pos;
        let conflict = function.variables.iter().enumerate().any(|(k, kv)| {
            if k == v || k == j || kv.stack_pos == u16::MAX {
                return false;
            }
            let ks = kv.stack_pos;
            let ke = ks + size(&kv.type_def, &Context::Variable);
            // Spatial overlap: [slot, slot+v_size) ∩ [ks, ke) ≠ ∅
            let spatial = slot < ke && ks < slot + v_size;
            // Temporal overlap: [v_first, v_last] ∩ [k_first, k_last] ≠ ∅
            let temporal = v_first <= kv.last_use && v_last >= kv.first_def;
            spatial && temporal
        });
        if !conflict {
            return Some(slot);
        }
    }
    None
}
```

**Debug guard:** When `function.logging` is true, emit:
```
[assign_slots]   zone2-reuse '{}' reuses dead '{}' at slot={}
```

**Verification:**
1. Add a unit test `zone2_reuse_conflict_free` that creates three 24-byte
   variables: v1 (live 0–10), v2 (live 5–15, overlaps v1), v3 (live 11–20,
   does not overlap v1).  Assert v3 reuses v1's slot but v2 does not.
2. `cargo test --lib assign_slots` — all slot tests pass.

---

### C43.2 — Wire zone-2 reuse into `place_large_and_recurse`

**Goal:** Call `find_reusable_zone2_slot` before advancing `*tos`.

**File:** `src/variables/slots.rs`, function `place_large_and_recurse`

**Change:** In the `if v_size > 8` block (line ~183), before `let v_slot = *tos`:
```rust
let v_slot = if let Some(slot) = find_reusable_zone2_slot(function, v, scope) {
    slot
} else {
    let s = *tos;
    *tos += v_size;
    s
};
```

Remove the existing `*tos += v_size` after `pre_assigned_pos = v_slot`.

**Debug guard:** `function.logging` message distinguishes "zone2" (new slot)
from "zone2-reuse" (reused slot).

**Verification:**
1. `cargo test --lib assign_slots` — all unit tests pass including the new
   `zone2_reuse_conflict_free` from C43.1.
2. `cargo test warning_only_program` — the `46-caveats.loft` script that
   triggered the original failure must pass (the full conflict scan prevents
   the `nums` / `_map_result_5` partial overlap).

---

### C43.3 — Enable `assign_slots_sequential_text_reuse` test — **Done**

`#[ignore]` removed; test runs unconditionally in `src/variables/slots.rs`.

---

### C43.4 — Integration test: text-heavy script with slot validation

**Goal:** Verify text slot reuse works end-to-end in a loft program.

**File:** `tests/expressions.rs`

**Test:**
```rust
#[test]
fn text_slot_reuse_sequential() {
    // Two sequential text variables with non-overlapping lifetimes
    // should not cause stack corruption.
    code!(
        "fn check() -> text {
             a = \"hello\";
             b = a + \" world\";
             c = \"goodbye\";
             d = c + \" world\";
             d
         }"
    )
    .expr("check()")
    .result(Value::str("goodbye world"));
}
```

**Verification:** `make ci` — zero failures across all test suites.

---

## C47 — Native codegen: cross-scope closure CallRef dispatch

**Status:** Interpreter cross-scope closures work.  Native codegen has two
remaining issues.

**Sub-issues fixed:**
1. *(Fixed)* `Value::FnRef` emits `(d_nr, var___clos_N)` when closure exists.
2. *(Fixed)* CallRef dispatch passes `var_f.1` as `__closure` for cross-scope.
3. *(Fixed)* Scope analysis bounds check: deps from callee variable space
   (d >= function.count()) are skipped in Set registration and check_ref_leaks.

**Sub-issues remaining:**

**C47.3a — Broad CallRef match dispatch** — `output_call_ref` includes ALL
functions with matching parameter/return types, not just lambdas that could
actually be stored in the fn-ref.  For `fn(integer) -> integer`, the match
includes `abs()`, `len()`, and every other `integer -> integer` function.
Non-closure candidates have no `__closure` parameter, so the emitted
`var_f.1` argument causes "cannot find value `var_`" compile errors.

**Root cause:** The dispatch is a static match on d_nr.  Without a closed
set of possible values, it must conservatively include all type-compatible
functions.  Same-scope fn-refs work because `closure_var_of` returns a valid
closure variable name for the specific `has_closure` arms.

**Fix approach:** When emitting the `__closure` argument in the cross-scope
path, only emit `var_{fn_ref_name}.1` for candidates that `has_closure`.
For non-closure candidates in the same match, omit the closure argument
(they don't need one).  This is already handled — each arm checks
`*has_closure` independently.

The real problem: the `has_closure` arm IS emitting for `abs()` etc., but
`abs` doesn't have `has_closure = true`, so it emits without the closure
arg.  That's correct.  The compile error `var_??` suggests the fn-ref
variable's name is empty — a separate bug in how temporary fn-ref variables
are named.

**Investigation:** trace which CallRef variable has an empty name and fix
the temporary naming.

**Files:** `src/generation/emit.rs`, `src/generation/mod.rs`

### Step 1 — Fix temporary fn-ref variable naming

Find where temporary fn-ref variables (from chained calls like
`make_adder(5)(10)`) get created without a name.  Ensure all fn-ref
variables have a valid sanitized name.

### Step 2 — Test with named variables only

Test `add5 = make_adder(5); add5(10)` in native mode (avoids the
temporary naming issue).

### Step 3 — Enable cross-scope doc test

Add `make_adder` example back to `26-closures.loft`.  Run `make ci`.

**Effort:** Small–Medium
**Target:** 0.8.3

---

## C48 — Capturing closures with map/filter/reduce

**Problem:** The interpreter rejects capturing lambdas passed to `map`, `filter`,
`reduce` with "function reference must be a compile-time constant".

**Root cause:** `map`/`filter`/`reduce` are parsed by `parse_for_each_call` in
`src/parser/collections.rs`.  The callback argument is resolved as a static
`fn <name>` reference (d_nr known at parse time).  Lambda expressions produce
a `Type::Function` value via `emit_lambda_code`, but the collections parser
doesn't accept fn-ref variables or lambda expressions in the callback position.

**Fix approach:**

The interpreter's `map`/`filter`/`reduce` implementation (`src/parser/collections.rs`)
unrolls the callback into a for-loop internally.  For a fn-ref variable or lambda,
the unrolled loop should use `CallRef` instead of `Call`:

```loft
// map(v, |x| { x * factor }) desugars to:
result = vector<T>{};
for x in v {
    result += [CallRef(fn_ref_var, x)]
}
```

This requires:
1. Parser: detect when the callback argument is a fn-ref variable or lambda
   (not a static `fn <name>`)
2. Collections: emit `Value::CallRef` in the unrolled loop body instead of
   `Value::Call`
3. The fn-ref variable must be in scope during loop execution

**Alternative (simpler):** reject the error only when the callback is a
*non-function* type.  If the callback is a `Type::Function` variable, accept
it and emit CallRef.  This doesn't require changing the collections desugaring
— just the argument validation.

**Depends on:** C47 (for native codegen parity)
**Effort:** Medium
**Target:** 0.8.3

---

## C52 — Stdlib name clash: inconsistent behavior

**Problem:** User-defined names that collide with stdlib names behave
inconsistently:

| Collision | Current behavior |
|-----------|-----------------|
| `fn len(text)` | Silently ignored — stdlib wins, user fn is dead code |
| `fn println(text)` | Hard error: "Cannot redefine Function" |
| `struct File` | Hard error: "Redefined struct" |

The inconsistency arises because some stdlib functions are registered via
`#rust` annotations (native ops — hard error on redefine) while others use
method dispatch on specific types (type-specific overload resolution —
stdlib variant wins by being first in the lookup chain).

**Design — emit a warning, never silently shadow:**

1. **All collisions produce a warning** — never silently ignore the user's
   definition.  The message should be:
   `Warning: 'len' shadows a standard library function`

2. **User definition wins** — local definitions shadow stdlib, matching
   the convention of most languages (Python, JavaScript, Rust).  The user
   explicitly chose to define this name.

3. **Stdlib accessible via `std::name`** — add a virtual `std` source for
   the default library, so the user can write `std::len("hello")` to access
   the original.  This reuses the existing `source::name` import mechanism.

4. **No names are forbidden** — the user can redefine anything, including
   `assert`, `println`, `len`.  The warning is informational.

**Implementation steps:**

### Step 1 — Emit warning on stdlib name collision

In `src/parser/definitions.rs`, when `add_fn` or `add_def` encounters a name
that already exists in source 0 (the default stdlib source), emit:
```
Warning: 'name' shadows a standard library function/type
```
Instead of the hard error "Cannot redefine".

### Step 2 — Make user definition win

Change name resolution order: when a name exists in both the current source
and source 0, prefer the current source.  This is already the behavior for
type-dispatched methods; extend it to global functions.

### Step 3 — Register stdlib as `std` source

In `src/parser/mod.rs`, after loading `default/*.loft`, register source 0
with the name `std`.  Then `std::len`, `std::println`, `std::File` work
via the existing `source::name` resolution path.

### Step 4 — Tests

- `fn len(t: text) -> integer { 42 }` → warning + user fn called
- `std::len("hello")` → returns 5 (stdlib version)
- `struct File { x: integer }` → warning + user struct used
- `std::File` → accesses stdlib File

**Effort:** Medium
**Target:** 0.9.0 (not blocking 0.8.3/0.8.4)

---

## C53 — Match arms: library enums and bare variant names

**Problem:** Match arms cannot use library enum variants at all — neither
prefixed (`testlib::Ok`) nor bare (`Ok`).  The match arm parser at
`control.rs:396` reads one identifier then expects `{`, `=>`, or `|`.
It does not handle the `::` namespace separator.

**Investigation findings:**

1. `has_identifier()` at line 396 reads `testlib`, not `testlib::Ok`.
   The `::` is then unexpected → parse error.

2. Even if `::` were consumed, the discriminant lookup at line 497 uses
   `pattern_name` (which would be `testlib`, not `Ok`) in `attr_names`.

3. Bare variant names (`Ok` without prefix) fail during first pass because
   `def_nr("Ok")` returns `u32::MAX` (library variant not in local scope)
   and `children_of(e_nr)` may not find it if the enum children aren't
   indexed by name.

4. Same-file enums already work because their variants ARE in global scope.

**Three fixes needed:**

### Fix 1 — Handle `::` in match arm identifier (line 396)

After `has_identifier()`, check for `::`.  If present, read the second
identifier.  Use `data.source_nr(source, variant_name)` to resolve the
variant.  Track the resolved variant name separately from `pattern_name`
for the discriminant lookup at line 497.

```rust
let (resolved_name, variant_def_nr) = if self.lexer.has_token("::") {
    let source = self.data.get_source(&pattern_name);
    if let Some(vname) = self.lexer.has_identifier() {
        (vname.clone(), self.data.source_nr(source, &vname))
    } else { (pattern_name.clone(), u32::MAX) }
} else {
    (pattern_name.clone(), self.data.def_nr(&pattern_name))
};
```

### Fix 2 — Use `resolved_name` for discriminant lookup (line 497)

Replace `pattern_name` with `resolved_name` in
`self.data.def(e_nr).attr_names.get(&resolved_name)`.

### Fix 3 — Bare variant fallback via `children_of` (line 419)

When `def_nr` fails and `e_nr` is known, search the enum's children:
```rust
if variant_def_nr == u32::MAX && e_nr != u32::MAX {
    variant_def_nr = self.data.children_of(e_nr)
        .find(|&c| self.data.def(c).name == resolved_name)
        .unwrap_or(u32::MAX);
}
```

### Fix 4 — Update or-pattern `|` to also handle `::` and bare names

The `while self.lexer.has_token("|")` loop at line 511 reads additional
variant names.  It needs the same `::` and `children_of` resolution.

**Effort:** Medium (4 changes in `parse_match`, all in `control.rs`)
**Target:** 0.9.0

---

