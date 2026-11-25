
# Native-side API

How a native library talks to the interpreter: auto-marshalling, `loft-ffi`, store allocation.  Split out of [PACKAGES.md](PACKAGES.md).

---

## Auto-marshalling dispatch (interpreter, legacy path)

The `#[loft_native]` bridge above (§ Function binding model) is the modern
zero-boilerplate path.  Libraries not yet migrated to it fall back to the
generic auto-marshaller in `src/extensions.rs`, which bridges loft stack
values to C-ABI calls without per-function glue code.

### How it works

1. **`compute_sig()`** reads the `#native` definition's types and produces a
   compact `NativeSig { params: Vec<ArgT>, ret: Option<ArgT> }`.

2. **`wire_native_fns()`** iterates all `#native` definitions, resolves symbols
   via dlsym, and replaces panic-stubs with the generic `native_auto_dispatch`.

3. **`native_auto_dispatch()`** pops arguments from the loft stack in reverse
   order, builds typed `ArgVal` values, and calls `dispatch_call()`.

4. **`dispatch_call()`** pattern-matches on the signature and calls the native
   function pointer with the correct C-ABI cast.

### Type mapping

| Loft type | ArgT | C-ABI type |
|-----------|------|-----------|
| `integer` (plain, no `size(N)`) | `I64` | `i64` |
| `character` / narrow int (`i8`/`u8`/`i16`/`u16`/`i32`) | `I32` | `i32` |
| `float` | `F64` | `f64` |
| `single` | `F32` | `f32` |
| `boolean` | `Bool` | `bool` |
| `text` | `Text` | `*const u8, usize` |
| struct / vector / collection | `Ref` | `LoftRef` (with `LoftStore` prepended) |

When any parameter or return type is `Ref`, a `LoftStore` handle is prepended
as the first C-ABI argument, giving the native function access to store memory.

For functions returning `Ref` with no `Ref` parameters (e.g. `rand_indices`),
the dispatcher allocates a fresh store for the result automatically.

### How a callback finds the stores

The `LoftStore` handle carries the interpreter's `Stores` itself: `LoftStoreCtx`'s opaque word
holds the store number in its low 16 bits and the `Stores` pointer above them, so the
allocation callbacks (`claim()`, `resize()`, `reload()`) reach back into the stores of the call
they serve — a `par` worker's call into its worker's own.  A native library reads only the low
16 bits (`loft_ffi_store_nr`).  This needs 64-bit pointers; a 32-bit host runs one program per
instance and publishes its `Stores` once.

---

## loft-ffi helper crate

The `loft-ffi` crate (`/loft-ffi/`) provides safe building blocks for native
extension authors. No dependencies.

### Core types

**`LoftRef`** — Opaque reference to a store object (struct, vector, collection):
```rust
#[repr(C)]
pub struct LoftRef {
    pub store_nr: u16,
    pub rec: u32,
    pub pos: u32,
}
```

**`LoftStore`** — Direct memory access to a store buffer, with allocation callbacks:
```rust
#[repr(C)]
pub struct LoftStore {
    pub ptr: *mut u8,                    // base pointer (may move on alloc)
    pub size: u32,                       // capacity in 8-byte words
    pub ctx: LoftStoreCtx,              // opaque context for callbacks
    pub claim_fn: ...,                   // allocate words → rec
    pub reload_fn: ...,                  // refresh ptr/size after alloc
    pub resize_fn: ...,                  // resize record → new rec
    pub foreign_fn: ...,                 // adopt the cdylib's own bytes, no copy (@PLN174 F5)
}
```

**`LoftStr`** — `#[repr(C)]` text return type (borrowed pointer, valid until
next `ret()` call on the same thread).

### Text helpers

```rust
// Convert C-ABI text parameter to &str
let name = unsafe { loft_ffi::text(name_ptr, name_len) };

// Return a String as LoftStr (stored in thread-local buffer)
loft_ffi::ret(format!("Hello, {name}!"))

// Return a borrowed &str without copying
loft_ffi::ret_ref(some_str)
```

### Field access

`LoftStore` provides direct read/write methods for store memory:
- `get_int()` / `set_int()` — `i32` fields
- `get_long()` / `set_long()` — `i64` fields
- `get_float()` / `set_float()` — `f64` fields
- `get_byte()` / `set_byte()` — `u8` fields (boolean, simple enum)
- `get_text()` — read text field as `(*const u8, usize)`
- `get_ref()` — read sub-reference field as `LoftRef`

All take `(rec, pos, offset)` and compute byte address as `rec * 8 + pos + offset`.

### Null sentinels

```rust
pub const NULL_INT: i32 = i32::MIN;
pub const NULL_LONG: i64 = i64::MIN;
```

---

## Store allocation from native code

Native extensions can allocate records and build vectors directly in the store
via `LoftStore` methods. Each mutating operation automatically reloads the
store pointer, since allocation may trigger reallocation.

### Low-level allocation

```rust
// Allocate raw words (auto-reloads ptr)
let rec = unsafe { store.claim(words) };

// Resize a record (may relocate; auto-reloads ptr)
let new_rec = unsafe { store.resize(rec, new_words) };

// Manually refresh ptr/size
unsafe { store.reload() };
```

### Record allocation

```rust
// Allocate a struct record (store_nr derived from the LoftStore handle)
let r = unsafe { store.alloc_record(words) };
// r.rec = record number, r.pos = 8 (data start)
```

### Vector operations

```rust
// Create an empty vector with pre-allocated capacity
let mut v = unsafe { store.alloc_vector(elem_size, capacity) };

// Append elements (handles resize automatically)
unsafe { store.vector_push_int(&mut v, 42) };
unsafe { store.vector_push_long(&mut v, 123i64) };
unsafe { store.vector_push_float(&mut v, 3.14) };

// Read current length
let len = unsafe { store.vector_len(&v) };
```

The `vector_push_*` methods update `v.rec` in place if the vector record
moves during resize. The minimum allocation is 11 elements (matching the
interpreter's convention). The `store_nr` is derived automatically from
the `LoftStore` handle.

### A read-only answer with no copy (@PLN174 F5)

```rust
// Hand loft the cdylib's own Vec<u8> as a `vector<u8>`: no copy, released with the last
// loft handle over it (in this crate's allocator).  Reads answer exactly what
// `alloc_vector_from_bytes` answers; a WRITE into it halts the program with the advice
// to copy first (`w = v`).  Copies on a host without `foreign_fn`.
let decoded: Vec<u8> = decode(input);
unsafe { store.foreign_vector_from_owned(decoded) }
```

Use it where the library's contract already says "read the result" — a decoded payload,
a file's bytes — never for a buffer the caller is meant to grow.  A library that calls it
declares the loft floor that carries loft-ffi 0.1.2 in `loft.toml` (`loft = ">=…"`): on an
older host the field is past the handle it was given.  DATABASE.md § Foreign stores has
the runtime side.

### Callback architecture

The allocation callbacks bridge native code back into the interpreter:

```
Native extension                    Interpreter (the Stores in the handle)
─────────────────                   ──────────────────────────────
store.claim(words)
  → claim_fn(ctx, words)    ──→    Store::claim(words) → rec
  → reload_fn(ctx, &ptr, &size) →  read store.base_ptr(), capacity
  ← updated ptr, size
  ← rec
```

`LoftStoreCtx` encodes the `store_nr` and, above it, the `Stores` the handle was built from.

### Safety guarantees

The callback infrastructure provides two safety mechanisms:

1. **Panic containment**: All three callbacks (`ffi_claim`, `ffi_resize`,
   `ffi_reload`) wrap their bodies in `std::panic::catch_unwind` to prevent
   panics from propagating across the C-ABI boundary. On panic, `claim`
   returns 0, `resize` returns the original record unchanged, and `reload`
   is a no-op.

2. **No state outlives the call**: the handle is the only route back to the
   stores, and it is built for one call, so nothing has to be reset after a
   native function or a callback panics.

---

## Code generation: `loft generate`

`loft generate` is refused: it wrote Rust stubs for a package's `#native` declarations, and a
package's native part is C now (@PLN225).  `loft new --native <name>` scaffolds the C part —
`c/<name>.c` under `[c] shim`, bound by a `#c` declaration ([PACKAGES_C_BINDING.md](PACKAGES_C_BINDING.md)).

---

## Key source files

| File | Role |
|------|------|
| `src/extensions.rs` | cdylib loader, auto-marshalling dispatcher, allocation callbacks |
| `src/native.rs` | Built-in function registry (`FUNCTIONS` table, `init()`) |
| `src/manifest.rs` | `loft.toml` reader and version checker |
| `loft-ffi/src/lib.rs` | `LoftRef`, `LoftStore`, `LoftStr`, allocation helpers |
