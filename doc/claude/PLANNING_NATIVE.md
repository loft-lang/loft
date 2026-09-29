<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# PLANNING — native codegen and performance items

Split from [PLANNING.md](PLANNING.md) by subject: Tier N native codegen, the O-tier performance items and AOT-compiled libraries.  Shipped items are in [PLANNING-history.md](PLANNING-history.md).

---

## N — Native Codegen

All N-tier items (N1–N9) are completed.  Native test parity achieved 2026-03-23:
all `.loft` tests pass in both interpreter and native mode.
Full design in [NATIVE.md](NATIVE.md).

---

### N8  Native codegen: extend to tuples, coroutines, and generics
**Sources:** CAVEATS.md C19, NATIVE.md, TUPLES.md, COROUTINE.md
**Severity:** Medium — programs using tuples, coroutines, or `maybe<T>` cannot be compiled with `--native`.
**Description:** The native (`--native`) code generator currently falls back to the interpreter for three feature areas (see CAVEATS.md C19): tuples, coroutines, and generic/maybe types.  Each area is split into independently shippable sub-items below.

---

#### N8a.1 — Native: `Type::Tuple` dispatch in code generator
**Effort:** Small · **Depends:** T1
Add `Type::Tuple` to all `output_type`, `output_init`, `output_set`, and variable-declaration paths in `src/generation/`.  Until N8a.2 is done, functions that use tuples should be gracefully skipped (added to `SCRIPTS_NATIVE_SKIP`).
**Tests:** compile without errors for files that don’t use tuple operations; skip gate for `50-tuples.loft`.

#### N8a.2 — Native: tuple construction and element access
**Effort:** Small · **Depends:** N8a.1
Emit a tuple literal as consecutive scalar assignments onto the Rust stack frame.  Emit element reads (`.0`, `.1`, …) as direct field reads from the emitted Rust struct/tuple.  Emit `OpPutInt`/`OpPutText` analogs for element writes.
**Tests:** `tests/scripts/50-tuples.loft` passes in `--native` mode for construction and read sections; element assignment and deconstruction covered by sub-tests.

#### N8a.3 — Native: tuple function return (multi-value Rust struct)
**Effort:** Medium · **Depends:** N8a.2
Tuple-returning functions emit a generated Rust struct (e.g. `struct Ret_foo { f0: i64, f1: String }`) as the return type.  Caller deconstructs the struct into local slots.  LHS deconstruction (`(a, b) = foo()`) handled in the call site template.
**Tests:** `50-tuples.loft` fully passes in `--native` mode (no `SCRIPTS_NATIVE_SKIP` entry).

---

#### N8b.1 — Native: coroutine state-machine transform design
**Effort:** High · **Depends:** CO1
Design and document the Rust enum state machine that represents a suspended coroutine.  Each `yield` point becomes a variant that stores all live locals.  Write the state-machine emitter skeleton in `src/generation/`; no working coroutines yet, but the infrastructure compiles.  Document the design in NATIVE.md § N8b.
**Note:** Using `genawaiter` or `async-std` generators is an alternative; evaluate before committing to the hand-written state machine approach.

#### N8b.2 — Native: basic coroutine emission (yield/resume cycle)
**Effort:** High · **Depends:** N8b.1
Emit `OpCoroutineCreate`, `OpCoroutineNext`, `OpYield`, and `OpCoroutineReturn` using the state machine from N8b.1.  Cover coroutines with integer/float/boolean yields and no text locals (text serialisation adds complexity, tackled as a follow-on).
**Tests:** `tests/scripts/51-coroutines.loft` basic sections pass in `--native`; text-yield sections remain skipped.

#### N8b.3 — Native: `yield from` delegation in native coroutine
**Effort:** Medium · **Depends:** N8b.2
Extend the state machine emitter to handle `yield from inner()` — the sub-generator loop is inlined into the outer state machine as an additional state range.  Requires careful handling of the sub-generator’s exhaustion sentinel.
**Tests:** `51-coroutines.loft` fully passes in `--native` mode (yield-from sections un-skipped).

---

#### N8c.1 — Native: audit which generic instantiations fail and why
**Effort:** Small · **Depends:** none
Generic functions are monomorphized at parse time (`try_generic_instantiation` in
`src/parser/mod.rs`); each call site produces a concrete `DefType::Function` named
`t_<len><type>_<name>` (e.g. `t_4text_identity`).  Native codegen sees only concrete
functions.  The P5 skip is because some monomorphized instantiations produce compile
errors, not because generics are unsupported at codegen level.

Audit procedure:
1. Temporarily remove `"48-generics.loft"` from `SCRIPTS_NATIVE_SKIP`.
2. Run `cargo test --test native 2>&1` and capture the exact compile errors.
3. Inspect the generated `.rs` file for the failing `t_4text_*` functions.
4. Record findings in NATIVE.md § N8c.1 before writing N8c.2.

Expected: text-returning instantiations lack the `Str::new()` return wrapping or have a text-parameter type mismatch.  Full design in NATIVE.md § N8c.
**Output:** Exact error message + root-cause note in NATIVE.md § N8c.1.

#### N8c.2 — Native: fix failing monomorphised instantiations
**Effort:** Small · **Depends:** N8c.1
Apply the fix identified in N8c.1.  If the issue is text-return wrapping: verify
`output_function()` applies the `Str::new()` path for all `Type::Text` return types
including `t_*` functions.  If parameter type: fix the call-site argument emission for
text arguments in monomorphized calls.  Remove `"48-generics.loft"` from
`SCRIPTS_NATIVE_SKIP`; confirm `cargo test --test native` passes.
**Tests:** `48-generics.loft` passes in `--native` mode; all four identity instantiations
(integer, float, text, boolean) and both pick_second instantiations produce correct output.

---

**Overall effort:** N8a Small+Small+Medium; N8b High+High+Medium; N8c Small+Small
**Depends:** T1 (N8a), CO1 (N8b)
**Target:** 0.8.3

---

### S31  Native harness: pass `--extern` for optional feature deps
**Sources:** CAVEATS.md C27
**Severity:** Medium — `rand`, `rand_seed`, `rand_indices` and any future optional-dep functions are silently untested in native mode.
**Description:** The native test harness in `tests/native.rs` compiles generated `.rs` files by invoking `rustc` directly with `--extern loft=libloft.rlib`.  Optional feature dependencies (`rand_core`, `rand_pcg`) are not passed as `--extern` flags, so any generated code that uses the `random` feature fails to compile with `E0433: use of undeclared crate or module 'rand_core'`.  `15-random.loft` and `21-random.loft` are therefore in `SCRIPTS_NATIVE_SKIP` / `NATIVE_SKIP`.

**Fix path:**
1. In `find_loft_rlib()` (`tests/native.rs`), after locating the `deps/` directory, scan it for `.rlib` files matching the optional deps listed in `Cargo.toml` (`rand_core`, `rand_pcg`, `png`, etc.).
2. Build a `Vec<(String, PathBuf)>` of `(crate_name, rlib_path)` pairs.
3. Pass each as an additional `--extern <crate_name>=<path>` argument in the `rustc` invocations inside `run_native_test`.
4. Remove `"15-random.loft"` from `SCRIPTS_NATIVE_SKIP` and `"21-random.loft"` from `NATIVE_SKIP`.
5. Confirm `cargo test --test native` passes for both random files.

**Tests:** `15-random.loft` and `21-random.loft` pass in native mode.
**Effort:** Small
**Target:** 0.8.3

---

### S35  Native: Insert-return pattern emits malformed Rust
**Sources:** `tests/native.rs` `SCRIPTS_NATIVE_SKIP`, `tests/scripts/20-binary.loft`
**Severity:** Medium — the native codegen path for `20-binary.loft` has been excluded
since S34's interpreter fix exposed it.
**Description:** The native code generator (`src/generation/`) emits malformed Rust for
the IR pattern `Set(rv, Insert([Set(_read_34, Null), Block]))`.  This is a block-return
pattern where the return value `rv` is assigned the result of an `Insert` that contains
a nested `Set`.  The emitted Rust looks like:

```rust
let mut var_rv: DbRef =   let mut var__read_34: DbRef = DbRef::null();
```

The inner `Set(_read_34, Null)` is being emitted inline as a declaration rather than
as a separate statement before the `Insert` call, producing a declaration in the middle
of an expression context.

**Root cause (confirmed):** `output_set` in `src/generation/dispatch.rs` handles
`Value::Set(var, to)` by writing `let mut var_{name}: type = ` and then calling
`output_code_inner(w, to)` for the RHS.  When `to` is `Value::Insert(ops)`, the
`Value::Insert` arm in `output_code_inner` (emit.rs:52–63) iterates over `ops` and
emits each one indented with a trailing semicolon — treating them as statements.
This is correct at the top level but wrong inside an expression context.  The result
is a Rust declaration nested inside another Rust expression, which is a syntax error.

**Fix path (concrete — `src/generation/dispatch.rs`, `output_set`):**

Add a branch for `to = Value::Insert(ops)` before the general `output_code_inner`
call, handling it by hoisting all-but-last ops as statements then assigning the
last op's result:

```rust
// S35: Set(var, Insert([stmt1, ..., last_expr])) — hoist all-but-last ops
// as statements before the declaration, then assign from the final expression.
if let Value::Insert(ops) = to {
    // Emit prefix statements (all except the last op).
    for op in &ops[..ops.len() - 1] {
        self.indent(w)?;
        self.output_code_inner(w, op)?;
        writeln!(w, ";")?;
    }
    self.indent(w)?;
    // Now emit the declaration/assignment with only the last op as the value.
    if self.declared.contains(&var) {
        write!(w, "var_{name} = ")?;
    } else {
        self.declared.insert(var);
        let tp_str = rust_type(variables.tp(var), &Context::Variable);
        write!(w, "let mut var_{name}: {tp_str} = ")?;
    }
    self.output_code_inner(w, &ops[ops.len() - 1])?;
    return Ok(());
}
```

This branch is added after the `Value::Block` pre-declaration handling (line ~73) and
before the general `declared.contains` check (line ~85).

**Tests:** Remove `"20-binary.loft"` from `SCRIPTS_NATIVE_SKIP` in `tests/native.rs`
once fixed.
**Effort:** Medium
**Target:** 0.8.3

---

### S-lexer  Fix 15-lexer.loft / 16-parser.loft "Unknown record" crash
**Severity:** Medium — blocks `tests/docs/16-parser.loft` (the parser library test)
**Description:** Running `16-parser.loft` panics with `Unknown record 2147483648` at
`store.rs:897`.  The crash is on `main` too (not a regression).

**Root cause:** The crash path is `io.rs:611` in the `on==2` (sorted) branch of `iterate()`:
```
io.rs:607  sorted_rec = get_int(data.rec, data.pos) as u32   → i32::MIN cast to u32
io.rs:608  sorted_rec == 0?  No — 0x80000000 ≠ 0
io.rs:611  get_int(sorted_rec=2147483648, 4)                  → panics: "Unknown record"
```
When a struct field has an unresolved or unknown type (type 0 or 6), `set_default_value()`
in `database/structures.rs` writes `i32::MIN` instead of `0`.  When cast `as u32`, this
becomes 2147483648 — a poison value that passes the `== 0` null check but is not a valid
record number.  The `Parser` struct in `16-parser.loft` has hash/sorted collection fields;
if any field's type isn't fully resolved, iteration over it hits this crash.

**Fix path:**
1. **Immediate guard (io.rs):** In the `on==2` sorted branch, check `sorted_rec_raw <= 0`
   (not just `== 0`) before using it as a record number.  This catches both the `0`
   (empty collection) and `i32::MIN` (unresolved type) sentinels:
   ```rust
   let sorted_rec_raw = all[data.store_nr as usize].get_int(data.rec, data.pos);
   let sorted_rec = if sorted_rec_raw <= 0 { 0 } else { sorted_rec_raw as u32 };
   ```
   Apply the same guard to `on==1` (index) and `on>=4` (hash) branches.

2. **Debug guard (io.rs):** Add a `debug_assert!(sorted_rec_raw >= 0, ...)` before the
   cast so debug builds catch the root cause (unresolved type in set_default_value) rather
   than silently treating it as empty.

3. **Root-cause investigation:** Add a temporary `eprintln!` in `set_default_value()` when
   type is 0 or 6 to identify which Parser field has the unresolved type.  Fix the type
   resolution so the field gets a proper `0` default instead of `i32::MIN`.

4. **Extend LOFT_ITERATE_TRACE:** Add trace output for the sorted branch (currently only
   the index branch is traced).

**Tests:** `last` in `tests/wrap.rs` runs unconditionally.
**Effort:** Small (guard) + Medium (root-cause fix in type resolution)
**Target:** 0.8.3

---

### A7.2-par  Fix `load_one` heap corruption under parallel test execution
**Severity:** Low — only affects test parallelism, not production
**Description:** `load_one_registers_native_functions` in `tests/native_loader.rs` passes
with `--test-threads=1` but crashes with "corrupted size vs. prev_size" or
"munmap_chunk(): invalid pointer" when run in parallel with other tests.

**Root cause:** `extensions::load_one()` calls `Library::new(path)` (wrapping `dlopen`)
without synchronisation.  When multiple test threads call `dlopen` on the same `.so`
simultaneously, the shared library's initialisation code and the `trampoline_register`
callback allocate heap memory concurrently, causing corruption.  The `std::mem::forget(lib)`
at the end prevents cleanup, compounding the issue.

**Fix path:**
1. **Mutex in `load_one`** (`src/extensions.rs`):
   ```rust
   static LOAD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
   pub fn load_one(state: &mut State, path: &str) {
       let _guard = LOAD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
       // ... existing code ...
   }
   ```
   This serialises `dlopen` calls process-wide.  The lock only contends during library
   loading (startup, not runtime).

2. **Double-load prevention:** Track loaded library paths in a `HashSet<String>` behind
   the same mutex.  If a library has already been loaded, skip `Library::new()` entirely.

3. **Debugging:** Log at `load_one` entry/exit gated by `LOFT_LOG`:
   `eprintln!("[extensions] loading {path}")`.

**Tests:** `load_one_registers_native_functions` runs unconditionally.
**Effort:** Small
**Target:** 0.8.3

---

### O1  Superinstruction merging
**Status: no longer blocked — the prerequisite it waited on exists**
**Sources:** PERFORMANCE-history.md § P1
**Description:** Peephole pass in `src/compile.rs` merges common 4-opcode sequences (var/var/op/put) into single opcodes.  This was deferred on "no slots remain without a redesign of the opcode space (e.g. a two-byte opcode escape)" — **that escape exists**: byte 255 escapes to `OPERATORS[255 + ext]`, so the space is 511 opcodes and a superinstruction lands in the escape range as a two-byte opcode (INTERMEDIATE.md § Opcode budget; `make ops-census` for the current occupancy).  The extra byte-fetch is negligible against replacing ~4 one-byte ops.
**Expected gain:** 2–4× on tight integer loops.
**Effort:** Medium — the peephole pass itself; the opcode-space work it was waiting for is done.
**Target:** 1.1+

---

### O2  Stack raw pointer cache
**Sources:** PERFORMANCE-history.md § P2
**Description:** Every `get_stack`/`put_stack` call resolves `database.store(&stack_cur)` then computes a raw pointer from `rec + pos`. Adding `stack_base: *mut u8` to `State` that is refreshed once per function call/return eliminates this lookup on every arithmetic push/pop, reducing the hot path to a single pointer add.
**Expected gain:** 20–50% across all interpreter benchmarks.

**Fix path:**

*Step 1 — Add `stack_base: *mut u8` and `stack_dirty: bool` to `State`.*

*Step 2 — Add `refresh_stack_ptr()`:*
```rust
fn refresh_stack_ptr(&mut self) {
    self.stack_base = self.database
        .store_mut(&self.stack_cur)
        .record_ptr_mut(self.stack_cur.rec, self.stack_cur.pos);
}
```
Call after `fn_call`, `op_return`, and any op that sets `stack_dirty = true`.

*Step 3 — Rewrite `get_stack` / `put_stack` as pointer arithmetic:*
```rust
pub fn get_stack<T: Copy>(&mut self) -> T {
    self.stack_pos -= size_of::<T>() as u32;
    unsafe { *(self.stack_base.add(self.stack_pos as usize) as *const T) }
}
pub fn put_stack<T>(&mut self, val: T) {
    unsafe { *(self.stack_base.add(self.stack_pos as usize) as *mut T) = val; }
    self.stack_pos += size_of::<T>() as u32;
}
```

*Step 4 — Mark allocation ops as dirty.*
In `fill.rs`, ops that allocate new records (`OpDatabase`, `OpNewRecord`, `OpInsertVector`, `OpAppendCopy`) set `self.stack_dirty = true`. The dispatch loop checks `stack_dirty` once per iteration and calls `refresh_stack_ptr()`.

*Step 5 — Benchmark and verify.* Run `bench/run_bench.sh` before/after. Target: ≥20% gain on benchmark 01.

**Safety invariant:** `stack_base` is valid only while no allocation modifies `stack_cur`'s backing store. Collection ops use separate stores, so the invariant holds between `refresh_stack_ptr` calls as long as `stack_dirty` is set by any store-mutating op.

**Effort:** High (`src/state/mod.rs`, `src/fill.rs`)
**Target:** 1.1+

---

**Target:** 0.8.2

---

### O4  Native: direct-emit local collections
**Sources:** PERFORMANCE-history.md § N1
**Description:** All vector/hash access in generated Rust currently goes through `codegen_runtime` helpers that take `stores: &mut Stores` and decode `DbRef` pointers. For a local `vector<integer>` used only within one function, the correct Rust type is `Vec<i32>` — no stores, no DbRef, no bounds-check overhead.
**Expected gain:** 5–15× on data-structure benchmarks (word frequency 16×, dot product 12×, insertion sort 7×).

**Fix path:**

*Step 1 — Escape analysis pass (`src/generation/escape.rs`, new).*
Before native codegen runs per function, classify each local variable:
- `Local` — declared in this function, never passed by `&ref` to another function, never assigned to a struct field.
- `Escaping` — passed by reference, stored in a field, or returned.
Conservative: any uncertain case is `Escaping`.

*Step 2 — Direct-emit type mapping.*
For `Local` variables of collection type, emit Rust native types:
`vector<integer>` → `Vec<i32>`, `vector<float>` → `Vec<f64>`, `index<text, T>` → `HashMap<String, T>`.
Declaration site: `let mut var_counts: Vec<i32> = Vec::new();` instead of `let mut var_counts: DbRef = stores.null();`.

*Step 3 — Direct-emit operation mapping.*
In `output_code_inner`, when the target variable is `Local`, bypass `codegen_runtime`:
`v[i]` → `v[i as usize]`, `v.length` → `v.len() as i32`, `v.append(x)` → `v.push(x)`, `v.sort()` → `v.sort()`.
For `Escaping` variables, the existing `codegen_runtime` path is unchanged.

*Step 4 — Drop is automatic.*
`Local` `Vec`/`HashMap` values drop at end of scope via RAII — no `OpFreeRef` emission needed.

*Step 5 — Verify.*
All 10 native benchmarks pass; `native_dir` and `native_scripts` test suites pass. New assertion: generated Rust for a known `Local` vector contains `Vec<` not `DbRef`.

**Effort:** High (`src/generation/escape.rs` new, `src/generation/emit.rs`, `src/generation/mod.rs`)
**Target:** 1.1+

---

### O5  Native: omit `stores` param from pure functions
**Sources:** PERFORMANCE-history.md § N2
**Description:** Every generated function currently receives `stores: &mut Stores` even when it never touches a store. For recursive functions like Fibonacci, `rustc -O` cannot eliminate this parameter across recursive calls, adding a register save/restore pair per call (measured: 1.84× slower than hand-written Rust). Purity analysis emits a `_pure` variant without `stores`; the wrapper delegates to it.
**Expected gain:** 10–30% on recursive compute benchmarks.
**Depends:** O4

**Fix path:**

*Step 1 — Purity analysis (`src/generation/purity.rs`, new).*
Recursively scan `def.code: Value`. A function is **pure** if its IR contains none of:
`Value::Ref`, `Value::Store`, `Value::Format`, `Value::Call` to any op with `stores` in its `#rust` body.
Memoize per `def_nr` to avoid exponential recursion on call graphs.

*Step 2 — Emit `_pure` variant.*
For each pure function, emit two Rust functions:
```rust
fn n_fibonacci_pure(n: i32) -> i32 {   // no stores parameter
    if n <= 1 { return n; }
    n_fibonacci_pure(n - 1) + n_fibonacci_pure(n - 2)
}
fn n_fibonacci(stores: &mut Stores, n: i32) -> i32 {  // wrapper for uniform call interface
    n_fibonacci_pure(n)
}
```

*Step 3 — Call-site dispatch.*
In `output_call`, when emitting a call from a pure context to a pure callee, emit `n_foo_pure(…)` directly, omitting `stores`. This allows `rustc` to inline and tail-call-optimise freely.

*Step 4 — Verify.*
`n_fibonacci_pure` appears in generated Rust for any recursive integer function. All native benchmarks pass.

**Effort:** High (`src/generation/purity.rs` new, `src/generation/emit.rs`, `src/generation/mod.rs`)
**Target:** 1.1+

---


## AOT — Ahead-of-time compiled libraries called from interpreter

**Problem:** Library functions like `blend_pixel`, `wu_line`, `scanline_fill`
are compute-intensive.  The interpreter runs them ~10–50x slower than native.
Users want library code at native speed while the main script runs in the
interpreter (for rapid iteration and REPL use).

**Design: auto-compile library to shared library, load via dlopen:**

When the interpreter loads a library via `use graphics;`:
1. Check cache: `lib/graphics/.loft/graphics.so` exists and source hash matches
2. If stale: emit Rust via `output_native`, compile with `rustc --crate-type cdylib -O`
3. Load shared library via `extensions::load_one`
4. Library functions dispatch through native code, not bytecode

The interpreter still parses `.loft` source for types and scope analysis.
Only bytecode execution is replaced by the native version.

```
User script: interpreted bytecode
    ↓ calls blend_pixel(canvas, x, y, color)
Library fn:  native compiled (loaded via dlopen)
    ↓ returns
User script: continues interpreting
```

**Cache:** `lib/<name>/.loft/` stores `.so` + source hash + generated `.rs`.
Recompile only when hash changes (~1–3s rustc cost on first run).

**Steps (desktop — dlopen):**
1. `output_native_library(lib_source)` — emit only library functions as cdylib
2. Compile with `rustc --crate-type cdylib -O --extern loft=...`
3. Load via `extensions::load_one` — registers functions via C-ABI
4. Cache with source hash in `.loft/` directory

**WASM — shared-memory cross-module calls (Approach B):**

WASM cannot dlopen, but can achieve the same result: compile each library
to its own `.wasm` module, share `WebAssembly.Memory` between modules, and
call library functions directly — no data serialization needed.

```
main.wasm ──shared memory──► graphics.wasm
    │                             │
    │  blend_pixel(dbref, x, y)   │
    ├────► JS import bridge ─────►│  runs native WASM blend
    │◄──── JS import bridge ◄────┤
    │                             │
    Stores heap: shared           │
```

How it works:
1. Compile each library to a separate `.wasm` via `rustc --target wasm32`
2. All modules share one `WebAssembly.Memory` instance (requires COOP/COEP
   headers for `SharedArrayBuffer`)
3. The `Stores` heap lives in shared memory — both modules read/write the
   same byte array.  `DbRef` values (store_nr + rec + pos) work across
   module boundaries without copying.
4. JS glue auto-generated from `.loft` type info: scalar args pass directly;
   `DbRef` args pass as three integers; text args pass as `(ptr, len)` into
   shared memory.
5. Cache: `lib/<name>/.loft/<name>.wasm` + source hash, same as desktop.

**Why shared memory works for loft:** the `Stores` allocator is a flat byte
array addressed by `(store_nr, rec, pos)`.  When two WASM modules share the
same memory, a `DbRef` allocated by the main module is directly readable by
the library module — same bytes, same offsets.  No marshalling needed for
struct or vector arguments.

**Fallback:** if `SharedArrayBuffer` is unavailable (no COOP/COEP headers),
library functions stay interpreted in the main module (Approach A — single
WASM, works today).

**Cargo feature:** `native-libs` (includes `native-extensions` + rustc)
**Effort:** High
**Target:** 0.9.0 (desktop dlopen), 1.0+ (WASM shared memory)

---

