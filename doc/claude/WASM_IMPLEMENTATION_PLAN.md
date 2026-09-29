# WASM implementation plan — the step list and the W1.15 / W1.17 work items

Split out of [WASM.md](WASM.md): the numbered implementation steps, plus the `CallRef` (W1.15) and store-lock (W1.17) work items.

## Implementation Plan

### Principles

- Each step produces a testable result before moving to the next.
- Steps are ordered so that earlier steps unblock later ones.
- Each step works with `cargo test` (native) and/or `wasm-pack build` + Node.js.
- No step requires the Web IDE — all testing is CLI/Node.js until the final
  integration step.

---

### Step 1 — Cargo feature scaffolding

**Goal:** `cargo build --features wasm --no-default-features` compiles (with stubs).

**Changes:**
- `Cargo.toml`: add `wasm`, `wasm-threads`, `threading` features and optional deps
  (`wasm-bindgen`, `serde`, `serde-wasm-bindgen`, `js-sys`, `web-sys`,
  `wasm-bindgen-rayon`)
- `src/lib.rs` (new): `#[cfg(feature = "wasm")] mod wasm;` — expose crate as a
  library target alongside the binary
- `src/wasm.rs` (new): empty module with `// TODO` placeholders
- Add `[lib]` section to `Cargo.toml`: `crate-type = ["cdylib", "rlib"]`

**Test:**
```sh
cargo check --features wasm --no-default-features
cargo check                                          # default features still work
cargo test                                           # existing tests unchanged
```

**Verification:** both check commands succeed with zero warnings from new code.

---

### Step 2 — Output capture (`print` → buffer)

**Goal:** `println()` / `print()` output goes to a thread-local buffer under `wasm`.

**Changes:**
- `src/wasm.rs`: add `output_push()`, `output_take()` with thread-local `String`
- `src/fill.rs` line 1725: wrap `print!()` in `#[cfg(not(feature = "wasm"))]`,
  add `#[cfg(feature = "wasm")]` branch calling `crate::wasm::output_push()`

**Test:**
```sh
# Native — no change
cargo test

# WASM — write a Rust unit test in src/wasm.rs:
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_capture() {
        output_push("hello ");
        output_push("world");
        assert_eq!(output_take(), "hello world");
        assert_eq!(output_take(), "");  // cleared after take
    }
}

cargo test --features wasm --no-default-features -- wasm::tests
```

**Verification:** unit test passes; native `cargo test` still passes.

---

### Step 3 — Sequential `par()` fallback

**Goal:** `par()` loops run sequentially when `threading` feature is off.

**Changes:**
- `src/parallel.rs`: wrap each `run_parallel_*` function body in
  `#[cfg(feature = "threading")]`; add `#[cfg(not(feature = "threading"))]`
  sequential versions
- `src/native.rs`: the call sites (`run_parallel_int(...)` etc.) stay unchanged —
  the function signatures are identical

**Test:**
```sh
# Native with threading — existing tests pass as before
cargo test

# Without threading — 22-threading.loft produces correct results sequentially
cargo test --no-default-features --features png,random -- scripts::threading
cargo test --no-default-features --features png,random -- docs
```

**Verification:** `22-threading.loft` output matches expected values.
`tests/threading.rs` is expected to fail (or be `#[cfg]`-gated) without the
`threading` feature — add `#![cfg(feature = "threading")]` at the top of that file.

---

### Step 4 — Logging to console stub

**Goal:** logger compiles under `wasm` without file I/O.

**Changes:**
- `src/logger.rs`: gate file I/O functions (`ensure_log_dir`, file write, rotate)
  behind `#[cfg(not(feature = "wasm"))]`
- Add `#[cfg(feature = "wasm")]` versions that call `crate::wasm::host_log_write()`
- `src/wasm.rs`: add `host_log_write()` — for now, a no-op or `web_sys::console::log_1`
  behind `#[cfg(feature = "wasm")]`

**Test:**
```sh
# Compile check
cargo check --features wasm --no-default-features

# Native logging unchanged
cargo test -- log_config
```

**Verification:** compiles cleanly; `log_config.rs` tests still pass on native.

---

### Step 5 — Random bridge stub

**Goal:** `rand()` / `rand_seed()` compile under `wasm` without the `rand_pcg` crate.

**Changes:**
- `src/ops.rs`: gate the `thread_local! { RNG }` and PCG usage behind
  `#[cfg(feature = "random")]`
- Add `#[cfg(all(feature = "wasm", not(feature = "random")))]` versions that call
  `crate::wasm::host_random_int()` and `crate::wasm::host_random_seed()`
- `src/wasm.rs`: declare `#[wasm_bindgen]` extern functions for `random_int`,
  `random_seed`; for Rust-side testing, add `#[cfg(test)]` mock implementations

**Test:**
```sh
# Native — rand tests still pass
cargo test -- scripts::random
cargo test -- docs::random

# WASM compile check
cargo check --features wasm --no-default-features
```

**Verification:** native random tests pass; WASM target compiles.

---

### Step 6 — Time and environment bridge stubs

**Goal:** `now()`, `ticks()`, `env_variable()`, `arguments()`, `directory()`,
`user_directory()`, `program_directory()` compile under `wasm`.

**Changes:**
- `src/native.rs` / `src/database/format.rs`: gate `SystemTime`, `Instant`,
  `std::env::*`, `dirs::home_dir()` behind `#[cfg(not(feature = "wasm"))]`
- Add `#[cfg(feature = "wasm")]` versions calling host bridge functions
- `src/wasm.rs`: declare extern functions for `time_now`, `time_ticks`,
  `env_variable`, `arguments`, `fs_cwd`, `fs_user_dir`, `fs_program_dir`

**Test:**
```sh
# Native — time/env tests still pass
cargo test -- scripts::time
cargo test -- scripts::files

# WASM compile check
cargo check --features wasm --no-default-features
```

**Verification:** native tests pass; WASM target compiles cleanly.

---

### Step 7 — File I/O bridge stubs

**Goal:** all file operations compile under `wasm`, calling host bridge functions
instead of `std::fs`.

**Changes:**
- `src/state/io.rs` and `src/database/io.rs`: gate every `std::fs` call behind
  `#[cfg(not(feature = "wasm"))]`; add `#[cfg(feature = "wasm")]` versions calling
  host functions (`fs_exists`, `fs_read_text`, `fs_write_text`, `fs_read_binary`,
  `fs_write_binary`, `fs_delete`, `fs_move`, `fs_mkdir`, `fs_mkdir_all`,
  `fs_list_dir`, `fs_is_dir`, `fs_is_file`, `fs_file_size`, `fs_seek`,
  `fs_read_bytes`, `fs_write_bytes`, `fs_get_cursor`)
- `src/wasm.rs`: declare all `#[wasm_bindgen]` extern functions

**Test:**
```sh
# Native — file tests still pass
cargo test -- scripts::files
cargo test -- scripts::file_result
cargo test -- docs::file

# WASM compile check
cargo check --features wasm --no-default-features
```

**Verification:** native file I/O tests pass; WASM target compiles. This is the
largest single step — review the diff carefully for missed `std::fs` calls.

---

### Step 8 — PNG buffer-based decoding

**Goal:** `file("img.png").png()` works under `wasm` by reading bytes from the host
instead of `std::fs::File`.

**Changes:**
- `src/png_store.rs`: extract shared logic into `decode_into_store<R: Read>()`; add
  `#[cfg(feature = "wasm")]` version that reads via `crate::wasm::host_read_binary()`
  and wraps in `std::io::Cursor`

**Test:**
```sh
# Native — any PNG-using tests still pass
cargo test

# WASM compile check (png feature is ON for wasm)
cargo check --features wasm --no-default-features
```

**Verification:** compiles; native PNG tests unchanged.

---

### Step 9 — `compile_and_run()` WASM entry point

**Goal:** the full interpreter is callable from JS via a single function.

**Changes:**
- `src/wasm.rs`: implement `compile_and_run(files_js: JsValue) -> JsValue` as
  described in [WEB_IDE.md](lib_plans/62-web-ide/README.md) § src/wasm.rs
- Wire up virtual FS population, parser, scope check, bytecode gen, execution,
  output collection, diagnostic collection
- Return `{ output, diagnostics, success }`

**Test:**
```sh
# WASM build (first time producing a .wasm file)
wasm-pack build --target nodejs --out-dir tests/wasm/pkg -- --features wasm --no-default-features

# Smoke test from Node.js
node -e "
  const loft = require('./tests/wasm/pkg/loft_wasm.js');
  const r = loft.compile_and_run([{name:'main.loft', content:'fn main(){println(\"hi\")}'}]);
  console.log(r);
  process.exit(r.success ? 0 : 1);
"
```

**Verification:** prints `{ output: 'hi\n', diagnostics: [], success: true }` and
exits 0. This is the first time loft actually runs in WASM.

---

### Step 10 — VirtFS implementation (JavaScript)

**Goal:** the `VirtFS` class passes all unit tests in Node.js.

**Changes:**
- `tests/wasm/virt-fs.mjs`: implement the `VirtFS` class
- `tests/wasm/harness.mjs`: minimal test runner (`test()`, `assert()`,
  `assert.deepEqual()`, `assert.throws()`)
- `tests/wasm/virt-fs.test.mjs`: all unit tests from the [VirtFS unit tests]
  section of this document

**Test:**
```sh
node tests/wasm/virt-fs.test.mjs
```

**Verification:** all VirtFS tests pass (exists, read, write, delete, move, mkdir,
binary, cursor, snapshot/restore, toJSON/fromJSON, path resolution).

---

### Step 11 — Host factory and WASM bridge tests

**Goal:** loft programs that do file I/O, random, and time work end-to-end in Node.js
via WASM.

**Changes:**
- `tests/wasm/host.mjs`: implement `createHost()` wiring VirtFS to `loftHost`
- `tests/wasm/bridge.test.mjs`: integration tests from the [WASM bridge integration
  tests] section (file write/read, exists/delete, directory listing, rand
  determinism, mkdir_all, binary I/O)
- `tests/wasm/file-io.test.mjs`: edge case tests
- `tests/wasm/random.test.mjs`: rand/seed determinism tests

**Test:**
```sh
node --experimental-vm-modules tests/wasm/bridge.test.mjs
node --experimental-vm-modules tests/wasm/random.test.mjs
node --experimental-vm-modules tests/wasm/file-io.test.mjs
```

**Verification:** all bridge tests pass — loft programs produce correct output,
VirtFS reflects writes made by loft code, rand sequences are reproducible.

---

### Step 12 — LayeredFS and base tree

**Goal:** the overlay filesystem works; base tree is generated from project files.

**Changes:**
- `tests/wasm/layered-fs.mjs`: implement `LayeredFS` extending `VirtFS`
- `tests/wasm/layered-fs.test.mjs`: tests from the [Node.js testing with layers]
  section (shadow, delete tracking, coexistence, delta serialise/reload)
- `ide/scripts/build-base-fs.js`: build script that generates `base-fs.json` from
  `tests/docs/*.loft`, `doc/*.html`, `default/*.loft`

**Test:**
```sh
node tests/wasm/layered-fs.test.mjs
node ide/scripts/build-base-fs.js && ls -la ide/assets/base-fs.json
```

**Verification:** LayeredFS tests pass; `base-fs.json` is generated and contains
expected entries.

---

### Step 13 — Run existing loft test suite through WASM

**Goal:** the bulk of `tests/scripts/*.loft` and `tests/docs/*.loft` produce correct
output when executed via the WASM module in Node.js.

**Changes:**
- `tests/wasm/suite.mjs`: test runner that:
  1. Reads each `.loft` file
  2. Creates a VirtFS with the file and any supporting fixtures
  3. Calls `compile_and_run()` via the WASM module
  4. Compares output against the expected output (from the native test framework's
     result files, or by running native first and capturing)
- Skip list: `22-threading.loft` runs but is not compared for timing output;
  file-I/O tests need fixture trees

**Test:**
```sh
# Generate reference output from native
cargo test 2>&1 | tee tests/wasm/reference-output.txt

# Run through WASM
node --experimental-vm-modules tests/wasm/suite.mjs
```

**Verification:** WASM output matches native output for all non-skipped tests.
This is the major confidence gate — if this passes, the WASM port is functionally
correct.

---

### Step 14 — Tier 2: Web Worker threading (optional)

**Goal:** `par()` loops use real Web Workers when SharedArrayBuffer is available.

**Changes:**
- `Cargo.toml`: `wasm-threads` feature adds the Web Worker pool (@PLN117: loft's
  own, `wasm-native-threads`; this plan predates it and said `wasm-bindgen-rayon`)
- `src/parallel.rs`: route the `par` dispatch over the page's global pool
- `ide/src/loft-worker.js`: worker script
- `ide/src/wasm-bridge.js`: `initWasmThreaded()` with shared memory and worker pool
- `ide/serve.mjs`: dev server with COOP/COEP headers

**Test:**
```sh
# Build threaded WASM
RUSTFLAGS='-C target-feature=+atomics,+bulk-memory,+mutable-globals' \
  wasm-pack build --target web --out-dir ide/pkg/mt -- --features wasm-threads --no-default-features

# Test with the dev server
node ide/serve.mjs &
# Open http://localhost:8080, run a par() program, verify output + thread badge
```

**Verification:** `22-threading.loft` produces correct output; browser console shows
worker pool initialisation; IDE badge shows thread count.

---

### Step summary

| Step | What | Test method | Depends on |
|---|---|---|---|
| 1 | Cargo features | `cargo check` | — |
| 2 | Output capture | Rust unit test | 1 |
| 3 | Sequential `par()` | `cargo test` (no threading) | 1 |
| 4 | Logging stub | `cargo check` | 1, 2 |
| 5 | Random bridge | `cargo check` + native tests | 1 |
| 6 | Time/env bridge | `cargo check` + native tests | 1 |
| 7 | File I/O bridge | `cargo check` + native tests | 1 |
| 8 | PNG buffer decode | `cargo check` + native tests | 7 |
| 9 | `compile_and_run` | `wasm-pack` + Node.js smoke | 2, 4, 5, 6, 7 |
| 10 | VirtFS (JS) | Node.js unit tests | — |
| 11 | Host + bridge tests | Node.js + WASM | 9, 10 |
| 12 | LayeredFS + base tree | Node.js unit tests | 10 |
| 13 | Full test suite via WASM | Node.js suite runner | 11 |
| 14 | Tier 2 threading | Browser manual test | 9, 3 |

```
Steps 1-8: Rust-side — all testable with cargo
Step 9:    First WASM build — smoke test in Node.js
Steps 10-12: JavaScript-side — testable with Node.js alone
Step 13:   Full validation — WASM matches native
Step 14:   Threading upgrade — browser-only, optional
```

Steps 1-8 can be done in rough parallel (they touch different files), but the
dependency arrows above show the minimum ordering. Steps 10-12 have no Rust
dependency and can be developed in parallel with steps 1-8.

---

## Function References (`CallRef`) in WASM — W1.15

**Skip list entry:** `06-function.loft` — `#77: CallRef not implemented`

### Current status

`Value::CallRef` is handled by `output_call_ref` in `src/generation/emit.rs`.  The
implementation enumerates all reachable definitions with a matching `Type::Function`
signature and emits a `match` dispatch on the runtime `u32` definition number:

```rust
match var_fn_ref {
    3 => n_double(stores, arg0),
    7 => n_triple(stores, arg0),
    _ => panic!("unknown fn-ref {}", var_fn_ref),
}
```

This covers `fn <name>` expressions (stored as a definition number).  Lambda
expressions compile to anonymous function definitions with the same mechanism.

### Investigation step

Before implementing anything, verify whether `06-function.loft` actually fails under
the current WASM backend by removing it from `WASM_SKIP` in `tests/wrap.rs` and
running `cargo test --test wrap wasm_docs`.  If it passes, the skip was stale and
should simply be removed.

### If still failing: likely root causes

1. **Lambda with closure capture** — `output_call_ref` only handles `fn <name>` and
   uncapturing lambdas.  A lambda that captures variables (A5.6) may produce a
   `Value::CallRef` variant whose closure record is not emitted correctly in the
   `--native-wasm` backend.  Fix: confirm captured-variable closures are excluded from
   CallRef dispatch and remain interpreter-only until A5.6 lands.

2. **Higher-order stdlib functions** — `map`, `filter`, `reduce` call through
   `Value::CallRef` at the call site.  If `output_call_ref` does not collect `map`'s
   lambda as a reachable definition, the `match` arm is missing and the panic fires.
   Fix: ensure `start_fn` / `reachable` tracking follows `fn <name>` constants through
   `Value::FnRef` assignments.

### Fix path

1. Remove `"06-function.loft"` from `WASM_SKIP` and run WASM tests.
2. If tests pass — remove the entry and close issue #77.
3. If tests fail — capture the panic message to identify which specific case fails (closure capture vs. reachability), then apply the targeted fix above.
4. Add a `tests/wasm/call-ref.test.mjs` that exercises `fn <name>`, lambdas, `map`, `filter`, and `reduce` via the host bridge.

**Effort:** S (investigation + targeted fix)
**Source:** `src/generation/emit.rs:output_call_ref`, `tests/docs/06-function.loft`, issue #77

---

## Store Locks in WASM — W1.17

**Skip list entry:** `18-locks.loft` — `todo!()`

### Current status

`n_get_store_lock` and `n_set_store_lock` are listed in `CODEGEN_RUNTIME_FNS` in
`src/generation/mod.rs`.  Functions in this list are **not** emitted as `todo!()` stubs
— they are silently skipped and resolved at link time from `loft::codegen_runtime`.
`codegen_runtime.rs` implements both functions using the standard `Store::locked` flag,
which is pure Rust with no OS dependency.  No host bridge is needed.

### Investigation step

Remove `"18-locks.loft"` from `WASM_SKIP` in `tests/wrap.rs` and run `cargo test
--test wrap wasm_docs`.  Because `n_get_store_lock` / `n_set_store_lock` are
feature-agnostic (no `#[cfg(feature = "wasm")]` needed), they should work without
modification.

### If still failing

The `todo!()` comment in the WASM skip list may refer to a different function in
`18-locks.loft` — inspect the panic message.  If `set_store_lock` panics, check that
the `Store::locked` flag is correctly maintained across the `clone_for_worker` path
used by WASM worker spawning (W1.18).

### Fix path

1. Remove `"18-locks.loft"` from `WASM_SKIP` and run WASM tests.
2. If tests pass — remove the entry; the skip was stale.
3. If tests fail — capture the panic, identify the specific failing function, and apply the targeted fix.
4. Add a lock assertion to `tests/wasm/bridge.test.mjs`.

**Effort:** XS (investigation; likely a stale skip)
**Source:** `src/generation/mod.rs:CODEGEN_RUNTIME_FNS`, `tests/docs/18-locks.loft`
