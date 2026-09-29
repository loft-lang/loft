# WASM file I/O — FS-A…F implementation steps (archived)

**Status: shipped (W1.16).** Step-by-step plan for WASM file I/O, kept for
reference. The runtime/reference docs live in [WASM.md](../../../WASM.md).

---

## Overview

W1.16 (WASM file I/O) is large enough to cause CI failures if attempted as a
single change.  This document breaks it into six self-contained steps
(FS-A through FS-F), each verifiable in isolation before the next begins.

The invariant held throughout: **native tests never regress**.
Every step compiles cleanly on both `--features wasm` and the default feature set.

---

## Current state

`src/state/io.rs` and `src/database/io.rs` contain file operations guarded by
`#[cfg(not(feature = "wasm"))]`.  Under the `wasm` feature those code paths
simply return early (null, false, empty), so loft programs that use `File` are
silently no-ops inside WASM.

`tests/wasm/host.mjs` (`createHost`) already exposes a complete VirtFS-backed
`loftHost` object with every `fs_*` function the bridge will need.
`tests/wasm/file-io.test.mjs` exercises that host object directly — these
tests already pass (no Rust changes required).

The JS side is complete.  The remaining work is on the Rust side: wire
`src/wasm.rs` host stubs to call `globalThis.loftHost.*` via `js_sys`, then
route each file operation in `io.rs` / `database/io.rs` to call those stubs
instead of returning early.

---

## Step FS-A — Expose host bridge stubs via `js_sys`

**Goal:** The stub functions in `src/wasm.rs` (`host_fs_exists`, `host_fs_read_text`,
… `host_fs_write_binary`, etc.) make real JS calls via `js_sys::Reflect` when
compiled under the `wasm` feature.  No call sites in `io.rs` change yet.

**Why this step first:** separates the FFI wiring from the loft logic.  If the
JS ↔ Rust boundary breaks, the failure is isolated here.

### Changes

`src/wasm.rs` — replace each stub body with a `js_sys` call:

```rust
// Pattern for every host bridge function:
//   js_sys::Reflect::get(&globalThis.loftHost, &"fs_exists".into())
//   → JsValue function → .call1(&loft_host(), &path.into()) → bool

#[cfg(feature = "wasm")]
fn loft_host() -> js_sys::Object {
    js_sys::Reflect::get(&js_sys::global(), &"loftHost".into())
        .expect("globalThis.loftHost not set")
        .dyn_into()
        .expect("loftHost is not an object")
}

pub fn host_fs_exists(path: &str) -> bool {
    #[cfg(feature = "wasm")]
    {
        let f = js_sys::Reflect::get(&loft_host(), &"fs_exists".into())
            .expect("fs_exists missing");
        let r = js_sys::Function::from(f)
            .call1(&wasm_bindgen::JsValue::NULL, &path.into())
            .expect("fs_exists call failed");
        r.as_bool().unwrap_or(false)
    }
    #[cfg(not(feature = "wasm"))]
    { false }
}

// … repeat for each host_fs_* / host_time_* / host_random_* stub
```

The `js_sys` + `wasm_bindgen` crates are already in `Cargo.toml` under
`features.wasm`; no new dependencies.

### Verification

```bash
# Compile-only check — no WASM binary needed:
cargo check --features wasm --no-default-features

# Native still compiles:
cargo check
```

No runtime tests yet — IO paths in `io.rs` still return early.

---

## Step FS-B — Wire `get_file_text` (text read)

**Goal:** `get_file_text` in `src/state/io.rs` calls `host_fs_read_text` under
the `wasm` feature instead of returning early.

**Scope:** one function, one new `#[cfg(feature = "wasm")]` branch.

### Current code (io.rs line ~27)

```rust
#[cfg(feature = "wasm")]
return; // no filesystem access under WASM
#[cfg(not(feature = "wasm"))]
{ let file_path = ...; if let Ok(mut f) = File::open(...) { ... } }
```

### Replacement

```rust
#[cfg(feature = "wasm")]
{
    let store = self.database.store(&file);
    let file_path = store
        .get_str(store.get_int(file.rec, file.pos + 24) as u32)
        .to_owned();
    let buf = self.database.store_mut(&r).addr_mut::<String>(r.rec, r.pos);
    if let Some(text) = crate::wasm::host_fs_read_text(&file_path) {
        *buf = text;
    }
}
#[cfg(not(feature = "wasm"))]
{ ... /* unchanged */ }
```

### Verification

```bash
# WASM compile check:
cargo check --features wasm --no-default-features

# Native text-read tests still pass:
cargo test -- scripts::files
cargo test -- docs::file
```

Integration test in `tests/wasm/bridge.test.mjs` (run after full WASM build):
```js
test('read text file', () => {
  const { host, fs } = createHost({ '/': {} });
  globalThis.loftHost = host;
  fs.writeText('/data.txt', 'hello from wasm');
  const r = runCode(`fn main() {
    f = file("/data.txt")
    println(f.content())
  }`);
  assert(r.output.trim() === 'hello from wasm');
});
```

---

## Step FS-C — Wire `write_file` (text + binary write)

**Goal:** `write_file` in `src/state/io.rs` calls `host_fs_write_text` /
`host_fs_write_binary` under `wasm` instead of being a no-op.

**Why this is its own step:** `write_file` is the largest function in `io.rs`
(~80 lines of native code).  The WASM branch must assemble the same byte
payload, then hand it to the host instead of writing to a `std::fs::File`.

### Changes

Inside the existing `#[cfg(not(feature = "wasm"))]` block a `std::fs::File`
handle is opened and `data: Vec<u8>` is assembled.  The WASM branch needs to
assemble the same `data` vec (shared logic) and then call the host.

Extract a helper:

```rust
/// Assemble the bytes to write for `val` of type `db_tp`.
/// Used by both the native and WASM write paths.
fn assemble_write_data(&self, val: DbRef, db_tp: u16, little_endian: bool) -> Vec<u8> {
    // … identical to the existing data-assembly code …
}
```

Then the WASM branch:

```rust
#[cfg(feature = "wasm")]
{
    let file_path = { ... }; // read path from store
    let format = ...;
    let little_endian = format == 2;
    let data = self.assemble_write_data(val, db_tp, little_endian);
    if self.database.is_text_type(db_tp) {
        let text = String::from_utf8_lossy(&data).into_owned();
        crate::wasm::host_fs_write_text(&file_path, &text);
    } else {
        crate::wasm::host_fs_write_binary(&file_path, &data);
    }
    // Update #next position field as native path does.
    let written = data.len();
    self.database.store_mut(&file).set_long(
        file.rec, file.pos + 16, next_pos + written as i64,
    );
}
```

### Verification

```bash
cargo check --features wasm --no-default-features
cargo test -- scripts::files
```

WASM integration test:
```js
test('write and read back text file', () => {
  const { host, fs } = createHost({ '/': { 'project': {} } });
  globalThis.loftHost = host;
  const r = runCode(`fn main() {
    f = file("/project/out.txt")
    f.write("hello world")
  }`);
  assert(r.success);
  assert(fs.readText('/project/out.txt') === 'hello world');
});
```

---

## Step FS-D — Wire metadata ops (`size_file`, `truncate_file`, `seek_file`)

**Goal:** `size_file`, `truncate_file`, and `seek_file` in `src/state/io.rs`
call host bridge functions instead of returning early/false.

These are smaller and purely mechanical; grouping them avoids three trivial PRs.

### Changes

`size_file`:
```rust
#[cfg(feature = "wasm")]
{
    let store = self.database.store(&file);
    let file_path = store.get_str(store.get_int(file.rec, file.pos + 24) as u32);
    self.put_stack(crate::wasm::host_fs_file_size(file_path));
    return;
}
```

`truncate_file`:
```rust
#[cfg(feature = "wasm")]
{
    let path = { /* read path from store */ };
    // No native handle management needed — delegate entirely to host.
    let ok = crate::wasm::host_fs_write_binary(&path, &vec![/* zeroed to size */])
        == 0;
    // Simpler alternative: expose host_fs_truncate or use write_binary with resize.
    // For now mark success if host acknowledges the path.
    self.put_stack(ok);
    return;
}
```

`seek_file` under WASM manages position in the store's `#next` field exactly as
it does for unopened native files (no real handle — call `host_fs_seek`):

```rust
#[cfg(feature = "wasm")]
{
    crate::wasm::host_fs_seek(&file_path, pos as u32);
    self.database.store_mut(&file).set_long(file.rec, file.pos + 16, pos);
    return;
}
```

### Verification

```bash
cargo check --features wasm --no-default-features
cargo test -- scripts::file_result
```

WASM integration tests:
```js
test('file size matches written content', () => { ... });
test('seek and read from offset', () => { ... });
```

---

## Step FS-E — Wire `database/io.rs` (directory + list ops)

**Goal:** `list_dir`, `file_exists`, `is_dir`, `delete_file`, `move_file`,
`make_dir`, `make_dir_all` in `src/database/io.rs` call host bridge functions
instead of being no-ops under `wasm`.

### Changes

`database/io.rs` already has `#[cfg(not(feature = "wasm"))]` guards.
Add symmetric `#[cfg(feature = "wasm")]` branches:

```rust
pub fn list_dir(path: &str) -> Vec<String> {
    #[cfg(feature = "wasm")]
    { return crate::wasm::host_fs_list_dir(path); }
    #[cfg(not(feature = "wasm"))]
    {
        if let Ok(iter) = std::fs::read_dir(path) {
            ...
        }
        vec![]
    }
}

pub fn file_exists(path: &str) -> bool {
    #[cfg(feature = "wasm")]
    { return crate::wasm::host_fs_exists(path); }
    #[cfg(not(feature = "wasm"))]
    { std::path::Path::new(path).exists() }
}

// … etc for is_dir, delete_file, move_file, make_dir, make_dir_all
```

### Verification

```bash
cargo check --features wasm --no-default-features
cargo test -- scripts::files
cargo test -- scripts::file_result
```

WASM integration tests:
```js
test('list dir returns entries', () => { ... });
test('exists and delete', () => { ... });
test('mkdir', () => { ... });
```

---

## Step FS-F — Enable `13-file.loft` test under WASM

**Goal:** `tests/scripts/13-file.loft` passes when run through the WASM entry
point.  This is the end-to-end acceptance test for W1.16.

### Prerequisite

Steps FS-A through FS-E completed; `wasm-pack` build succeeds.

### Changes

1. `tests/wasm/bridge.test.mjs` — add a test that reads `13-file.loft` from
   `tests/scripts/` and runs it via `compileAndRun`:

```js
import { readFileSync } from 'node:fs';

test('13-file.loft full script', () => {
  const content = readFileSync('tests/scripts/13-file.loft', 'utf-8');
  const { host } = createHost({ '/': {} });
  globalThis.loftHost = host;
  const r = JSON.parse(compileAndRun(JSON.stringify([
    { name: 'main.loft', content }
  ])));
  assert(r.success, `Failed: ${JSON.stringify(r.diagnostics)}`);
  assert(r.output.trim() !== '');
});
```

2. `doc/claude/ROADMAP.md` — remove the `13-file.loft` skip entry from the
   "Tests skipped by design" table, or update its note from "no WASM filesystem"
   to "passes — W1.16 complete".

3. `CHANGELOG.md` — add W1.16 entry.

### Verification

```bash
# Full native suite must still pass:
make ci

# WASM suite:
node tests/wasm/bridge.test.mjs
node tests/wasm/file-io.test.mjs
```

---

## Testing at each step

| Step | Native test | WASM check | JS test |
|------|-------------|-----------|---------|
| FS-A | `cargo check` | `cargo check --features wasm` | — |
| FS-B | `scripts::files`, `docs::file` | compile check | bridge: read |
| FS-C | `scripts::files` | compile check | bridge: write + read |
| FS-D | `scripts::file_result` | compile check | bridge: size, seek |
| FS-E | `scripts::files`, `scripts::file_result` | compile check | bridge: dir ops |
| FS-F | full `make ci` | `node tests/wasm/bridge.test.mjs` | 13-file.loft end-to-end |

Each step's `cargo check --features wasm --no-default-features` must pass before
moving to the next.  The native test suite must never regress.

---

## File map

| File | FS-A | FS-B | FS-C | FS-D | FS-E | FS-F |
|------|:----:|:----:|:----:|:----:|:----:|:----:|
| `src/wasm.rs` | ✎ | — | — | — | — | — |
| `src/state/io.rs` | — | ✎ | ✎ | ✎ | — | — |
| `src/database/io.rs` | — | — | — | — | ✎ | — |
| `tests/wasm/bridge.test.mjs` | — | ✎ | ✎ | ✎ | ✎ | ✎ |
| `doc/claude/ROADMAP.md` | — | — | — | — | — | ✎ |
| `CHANGELOG.md` | — | — | — | — | — | ✎ |

---

## Decisions deferred to implementation

- **`truncate_file` under WASM**: the host bridge has no explicit truncate call.
  Options: (a) add `host_fs_truncate(path, size)` to the host API contract in
  `host.mjs`; (b) implement truncate on the JS side in `VirtFS.truncate(path, size)`.
  Preferred: option (b) — `VirtFS` owns the canonical bytes so it can truncate in JS
  without a new bridge call.

- **`read_file` (binary read)** is not listed above because it uses the same
  `File` handle that `write_file` opens.  Under WASM it should call `host_fs_read_bytes`
  with the cursor position stored in `file.pos + 16`.  This can be added to FS-C
  or treated as FS-B.5 — binary reads.  Decide at implementation time based on whether
  the script `13-file.loft` exercises binary reads.

- **`free_ref` file handle close** (`src/state/io.rs`): the native path closes
  the OS file handle when the `File` record is freed.  Under WASM there is no
  handle; the WASM branch should be a no-op (already is, since the `#[cfg(not(feature = "wasm"))]`
  guard covers it).  No change needed.

---

## See also

- `tests/wasm/host.mjs` — `createHost()`: the JS side is already complete.
- `tests/wasm/file-io.test.mjs` — host-level edge-case tests (already pass).
- `tests/wasm/bridge.test.mjs` — integration tests that require the WASM binary.
- `doc/claude/WASM_IMPLEMENTATION_PLAN.md` § "Step 7 — File I/O bridge stubs" — original high-level design.
- `src/wasm.rs` — current stubs (all return empty/false/`Vec::new()`).
