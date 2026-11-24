<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# WASM implementation plan — history

The record behind [WASM_IMPLEMENTATION_PLAN.md](WASM_IMPLEMENTATION_PLAN.md): two work items
written against the rustc writer's wasm build (`src/generation/`, `codegen_runtime.rs`),
retired with the writer (@PLN225).  `tests/wrap.rs` no longer has a `WASM_SKIP` list naming
either script.

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
