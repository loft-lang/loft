<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# PLANNING — native codegen and performance items

Split from [PLANNING.md](PLANNING.md) by subject: native and performance items, and AOT-compiled libraries.  Shipped items are in [PLANNING-history.md](PLANNING-history.md).

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

## AOT — a library as its own wasm module

A `use`d library already compiles through C to a shared library the interpreter loads (the
auto-native library; `LOFT_NO_NATIVE_LIBS=1` keeps it interpreted).  What stays open is the
wasm half: each library its own `.wasm` module, built through C (clang, wasm32), sharing one
`WebAssembly.Memory` with the main module so a `DbRef` crosses the module boundary without
copying (`SharedArrayBuffer`, so COOP/COEP headers); without shared memory the library stays
interpreted in the main module.  The design as it read for the rustc route:
[PLANNING-history.md § Closed by the rustc writer's retirement](PLANNING-history.md#closed-by-the-rustc-writers-retirement).

**Effort:** High
**Target:** 1.0+
