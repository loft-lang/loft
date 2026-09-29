# Database — the Stores schema and multi-store manager

The type schema and the multi-store manager (`src/database/`).  Part of [DATABASE.md](DATABASE.md), which holds the overview, the raw `Store` heap, `DbRef` and vectors.

## Stores — Type Schema + Multi-Store Manager (`src/database/`)

### Stores struct

```rust
pub struct Stores {
    pub types: Vec<Type>,           // all registered types
    names: HashMap<String, u16>,    // type name → index
    allocations: Vec<Store>,        // one Store per allocation context
    pub max: u16,                   // number of registered types
}
```

`Stores` owns the complete type schema and all live stores. The `types` vector is append-only at runtime; type indices (`u16`) are stable.

### Fixed Base Type IDs

The following type indices are permanently fixed:

| ID | Type |
|---|---|
| 0 | `integer` (32-bit signed) |
| 1 | `long` (64-bit signed) |
| 2 | `single` (32-bit float) |
| 3 | `float` (64-bit float) |
| 4 | `boolean` |
| 5 | `text` (string) |
| 6 | `character` |

Types 0–6 are registered at construction time and never relocated.

### Type struct

```rust
pub struct Type {
    pub name: String,
    pub parts: Parts,
    pub keys: Vec<Key>,      // key fields for sorted/hash/index
    pub size: u32,           // byte size of one record
    pub align: u32,          // alignment requirement
    pub linked: bool,        // has back-reference (tree backward links)
    pub complex: bool,       // contains non-trivial types (strings, refs)
}
```

Two DERIVED cells ride on a `Type` row beside those fields and take no part in equality or
in the stored form: `facts` (`TypeFacts` — the heap and all-zero facts `heap_facts` derives)
and `prefill` (`PrefillImage`, `@FR-R-Prefill`).  The prefill image is the bytes the
field-by-field default walk writes for `Absent::Prefill` — the sentinels, the zeros, the
variant tag, an inline record's own defaults — captured ONCE from the walk's first run over
a zeroed span and written as one block (`Store::write_image`) on every later mint of the
type, in place of the walk's store resolution per field.  It is read back from what the
walk wrote, never computed a second way, and `LOFT_PREFILL_VERIFY=1` re-runs the walk after
every image write and panics naming the type where the two disagree.  Three cases keep the
walk: a type with a field not laid out yet (its image would freeze the field's zero where
the walk, once the field is laid out, writes its sentinel), `Absent::Final` (the `text as`
fill — declared defaults and interned text are not a fixed byte pattern), and an image
whose length no longer matches the layout; a table rollback forgets the image with the
facts.  `LOFT_NO_PREFILL_IMAGE=1` restores the walk on both backends; `LOFT_TRACE_PREFILL=1`
names each capture and each use.  The all-zero fast path (`zero_range`) stays in front of it.

### Parts enum

`Parts` describes the runtime layout and category of a type:

| Variant | Description |
|---|---|
| `Base` | Primitive (integer, long, float, boolean, text, character) |
| `Struct(Vec<Field>)` | Named fields with offsets |
| `Enum(Vec<(u16, String)>)` | Discriminated union; entries are (discriminant, name) |
| `EnumValue(u8, Vec<Field>)` | One variant of an enum (discriminant + fields) |
| `Byte(i32, bool)` | Byte-sized integer; `bool` = signed |
| `Short(i32, bool)` | 16-bit integer; `bool` = signed |
| `Vector(u16)` | Dynamic by-value array of element type `u16` |
| `Array(u16)` | Dynamic by-reference array of element type `u16` |
| `Sorted(u16, Vec<(u16,bool)>)` | Red-black tree ordered by key fields; `bool` = ascending |
| `Ordered(u16, Vec<(u16,bool)>)` | Ordered array (binary search) by key fields |
| `Hash(u16, Vec<u16>)` | Open-addressing hash table; field indices as hash keys |
| `Index(u16, Vec<(u16,bool)>, u16)` | Combo: sorted tree + hash table for a single collection |
| `Radix(u16, Vec<u16>)` | Spatial index for `spatial<T[x,y]>` / `spatial<T[x,y,z]>` — Morton/Z-order radix tree, 1–3 coordinate axes (renamed from `Spatial`) |
| `Trie(u16, u16)` | Text index for `trie<T[k]>` — the SAME radix tree over ONE text key; content type nr + the key field index |

### Field struct

```rust
pub struct Field {
    pub name: String,
    pub type_nr: u16,    // index into Stores::types
    pub offset: u32,     // byte offset within the record
}
```

### Stores API

| Method | Description |
|---|---|
| `new() -> Stores` | Create empty stores; registers base types 0–6 |
| `structure(name) -> u16` | Register a new struct type; returns its index |
| `field(type_nr, name, field_type, offset)` | Add a field to an existing struct type |
| `enumerate(name) -> u16` | Register a new enum type |
| `value(enum_nr, discriminant, name)` | Add a variant to an enum type |
| `finish()` | Seal schema registration (calculates sizes, alignment) |
| `allocate() -> u16` | Create a new `Store`; returns its index |
| `store(nr) -> &Store` | Borrow store by index |
| `mut_store(nr) -> &mut Store` | Mutably borrow store by index |
| `byte(min: i32, nullable: bool) -> u16` | Register or get a byte integer type; name = `"byte"` for (0,false) or `"byte<min,nullable>"` |
| `short(min: i32, nullable: bool) -> u16` | Register or get a 16-bit integer type; name = `"short<min,nullable>"` |
| `database(size: u32) -> DbRef` | Allocate a new top-level store slot; `size=u32::MAX` means no record claim |
| `free(db: &DbRef)` | Release a top-level store slot (LIFO order required) |
| `null() -> DbRef` | Allocate an empty store slot (calls `database(u32::MAX)`) |
| `read_data(r, tp, little_endian, data)` | Serialize a stored value to raw bytes (for writing to binary file) |
| `write_data(r, tp, little_endian, data)` | Deserialize raw bytes into a stored value (from reading a binary file) |
| `lock_store(r: &DbRef)` | Lock the store that owns `r` (no-op for null refs) |
| `unlock_store(r: &DbRef)` | Unlock the store that owns `r` |
| `is_store_locked(r: &DbRef) -> bool` | Return whether the store that owns `r` is locked |
| `adopt_store(store) -> u16` | Install an externally-built `Store`; clears the slot's free bit |
| `take_store(slot) -> Store` | Move a `Store` out, leaving a freed sentinel — **does NOT release the slot** |
| `release_slot(slot)` | Give a slot borrowed by `adopt_store` back to the pool |

**`adopt_store` / `take_store` are not symmetric about the slot, on purpose.**
`take_store` is written for a store handed out to OUTLIVE the table — the REPL's
session store, adopted for a run and taken back afterwards — where the slot
should stay reserved. So it leaves the free bit CLEAR, and `find_free_slot` only
ever returns a slot whose bit is SET. A caller borrowing a slot as **scratch**
must therefore call `release_slot`, or the slot number is burned for the life of
the process.

That leak is invisible from two places you would look: `store_memory()` counts
only LIVE stores and a freed sentinel is not one, and `LOFT_STORES=log` does not
trace this allocation path. It was found by reading the pair rather than by any
probe (@PLN123 B2, where compaction borrows a scratch slot on every load), and
`slot_recycling_tests` in `src/database/mod.rs` pins both halves so the
asymmetry stays recorded.

### Storing a scalar: `Store::write`, never a reference into the slot

An element's offset is `index * stride + fld`, so any stride the scalar's alignment does not
divide puts that slot at a misaligned address — `Lay`'s element 1 puts an `i64` at 34, and
`34 % 8 != 0`. A **reference** to a misaligned address is undefined behaviour even if it is never
read through, which is why storing a scalar goes through `Store::write` (a `write_unaligned`) and
not through `&mut *ptr.cast::<T>()` (loft#1481). `Store::addr_mut` exists for the values `write`
cannot store by value — a `String` or a `Str` whose heap buffer the caller mutates in place — and
it asserts the alignment rather than assuming it, with a plain `assert!` under no `cfg`, so a
wrong call panics in a release build too.

⚠ **If you are resolving a MERGE or a REBASE and this line is the conflict, the `addr_mut` side is
the unsound one.** The note is here because every other note about this rule — the comment at
`src/database/mod.rs`'s push site, `Store::write`'s own doc, and the assert's message — lives on
the *surviving* side, so a merge that quietly takes the older `*store.addr_mut::<T>(rec, len *
stride) = val` form shows its reader nothing at the moment the choice is made. Twice now the
`@PLN157` native-hoist branch and loft#1481's fix have met in exactly this conflict.

### The value stack lives in ONE record, and that record has to grow with it

Store index `0` is the interpreter's value stack. It is a single claimed
record — `PRIMARY`, record 1 — and every frame slot is addressed as
`(0, 1, pos)`. Frame writes go straight through `addr_mut`; they never call
`claim`, so the normal growth path never runs and `State::ensure_stack` is what
extends the buffer when a program nests deeply enough.

**Growing the buffer is only half of it.** `Store::grow_words` extends the
allocation; record 1's header still claims the size it was born with (1000 words
= 8000 bytes), so every stack byte above that mark sits outside the record that
owns it. `ensure_stack` therefore calls `Store::extend_primary_to_store_end`
after every growth — the store's only record spans the whole store, which is the
invariant `State::new` established and nothing since maintained (loft#935).

`Store::resize` is the wrong tool here and must stay unused on this store: its
fallback is claim-copy-delete, which RELOCATES the record. Record 1 IS the
running stack, so moving it moves every live frame out from under the
interpreter.

The failure this caused is worth remembering for its shape rather than its
cause: a consumer added a `vector<Struct>` local to a ~700-line dispatcher and
got `realloc(): invalid next size` — a glibc heap abort — in a *different* test
file, one that never called the edited code. The enclosing function's size was
never the defect; it was what made the frame big enough to cross the initial
claim. That is also why the shape resisted every attempt to shrink it: the axis
it needed was stack DEPTH, and a matrix that varies the expression while holding
the function fixed cannot reach it. `tests/scripts/935-stack-store-growth.loft`
is the depth axis in fifteen lines.

### Constant store (`CONST_STORE`)

Store index `1` is reserved for compile-time constant data:

```rust
// src/database/mod.rs
pub const CONST_STORE: u16 = 1;
```

Allocated by `State::new()` immediately after the stack store (index 0)
and before any runtime store.  Populated during `byte_code()` and
**locked** before `execute()` runs.

| Index | Purpose | Allocated in |
|---|---|---|
| 0 | Stack store (evaluation stack, record in store 1000 historical alias) | `State::new()` |
| 1 | **Constant store** (read-only data) | `State::new()` |
| 2+ | Runtime stores (structs, vectors) | `OpDatabase` at runtime |

**What lives in `CONST_STORE`** (@PLN82 Phase A, 2026):

- **Vector constants** — file-scope `QUAD = [1, 2, 3];` is built as
  a vector record in `CONST_STORE` during `byte_code()`.  Each
  constant's `DbRef` is recorded in `Definition.const_ref` and
  cached in `State.const_refs[d_nr]`.  Closes P127 (Var-collision
  on inlined vector-literal IR).
- **Long string constants** (>= 256 bytes) — `Store::set_str()`
  copies bytes into `CONST_STORE`; `OpConstStoreText` reads the
  `Str` pointer at runtime.  Replaced the ad-hoc `text_code:
  Arc<Vec<u8>>` buffer that previously lived on `State`.
- Short strings (< 256 bytes) stay embedded inline in the bytecode
  via `OpConstText` — record-header overhead exceeds the inline
  format's 1-byte-prefix cost at small sizes.

**Reference-site codegen** for vector constants:

```text
__cv = null
OpDatabase(__cv, vec_tp)             # allocate fresh runtime store
OpConstRef(d_nr)                     # push the constant's DbRef
OpCopyRecord(const, __cv, tp)        # deep-copy into __cv's store
return __cv                          # caller owns __cv (mutable)
```

Each reference site allocates a fresh runtime store and deep-copies
the constant record in.  Mutations to the copy never affect the
original; the copy participates in normal `OpFreeRef` lifetime.

**Lifetime + safety**:

- `CONST_STORE` is **never freed** — persists for the program's lifetime.
- **Locked** after construction (`store.locked = true`) — writes panic
  in debug, are no-ops in release.
- No `OpFreeRef` for `CONST_STORE` — it has no runtime refcount.
- Parallel workers may read the locked store directly without cloning
  (read-only = thread-safe).
- Excluded from the debug-mode "Database N not correctly freed" exit
  check — expected to remain allocated.

See [INTERMEDIATE.md § Bytecode State](INTERMEDIATE.md#bytecode-state--srcstate)
for `State.const_refs`'s role in `OpConstRef` dispatch.

For deferred follow-ups (mmap-backed cache file; WASM
pre-compiled stdlib including `CONST_STORE` as static bytes via
`include_bytes!`) see
[`plans/82-const-store/`](plans/82-const-store) §
Memory-mapped + WASM fast startup.

### Store Locking via `Stores`

`Stores` exposes three methods that wrap the per-`Store` lock flag:

```rust
pub fn lock_store(&mut self, r: &DbRef)       // enable write-protection
pub fn unlock_store(&mut self, r: &DbRef)     // remove write-protection
pub fn is_store_locked(&self, r: &DbRef) -> bool
```

All three methods silently ignore null refs (`r.rec == 0`) and out-of-range store indices so they are safe to call unconditionally from generated code.

These methods are surfaced to loft code via two native functions registered in `src/native.rs`:

| Native function | Loft declaration (`default/01_code.loft`) |
|---|---|
| `n_get_store_lock` | `fn get_store_lock(r: reference) -> boolean` |
| `n_set_store_lock` | `fn set_store_lock(r: reference, locked: boolean)` |

The `reference` parameter type accepts any concrete `Reference` type at call sites thanks to the type-compatibility check in the parser.

### `d#lock` Syntax

Loft code interacts with store locks through the `#lock` pseudo-field syntax:

```loft
c#lock        // read: boolean — true if the store is locked
c#lock = true // write: lock the store
```

**Parser routing** (`src/parser/collections.rs` and `src/parser/expressions.rs`):
- `iter_op` detects the `lock` keyword and emits `n_get_store_lock(c)` for reads.
- `towards_set` converts a `n_get_store_lock` call into `n_set_store_lock` for the left-hand side of an assignment.
- `parse_assign` validates the assignment: only a literal `true` or `false` is accepted (not an expression); assigning `false` to a `const` variable or argument is a compile-time error.

**Constraints enforced by the compiler**:
1. `d#lock` is only valid on `Reference` or `Vector` typed variables; any other type is a diagnostic error.
2. The right-hand side must be a constant boolean (`true` or `false`).
3. `d#lock = false` on a `const` variable is a compile-time error.

### `const` Variables and Arguments

The `const` keyword can be applied to local variable declarations and function arguments:

```loft
const d = Counter { value: 42 }   // local const variable
fn read_value(self: const Counter) // const argument
```

**Semantics**:
- The compiler marks the variable with `const_param`, preventing reassignment via `OpSet` in generated bytecode.
- In **debug builds only** (`#[cfg(debug_assertions)]`): the store is automatically locked immediately after initialisation (local `const`) or at the start of the function body (const arguments). This turns any accidental write into a runtime panic.
- In **release builds**: the lock is _not_ set automatically; only explicit `d#lock = true` in loft code locks the store.
- Reading `c#lock` on a const variable emits a runtime `n_get_store_lock` call. In a debug build this always returns `true` because the store was auto-locked; in release it returns whatever the current flag is.

**Implementation locations**:
- Auto-lock for local `const`: `expression()` in `src/parser/expressions.rs` — after the initialising assignment is compiled, inserts a `n_set_store_lock` call under `#[cfg(debug_assertions)]`.
- Auto-lock for const arguments: `parse_code()` in `src/parser/expressions.rs` — inserts lock calls at the start of the function body for every argument that is both an argument and const.

### Foreign stores — bytes the runtime does not own (@PLN174)

A `Store` can serve bytes that belong to somebody else — a file's mapping today
(`file_map`), a library's buffer or a host frame later — through the SAME contract every
vector read goes through, with no copy into a store.  **The one invariant:** a foreign
store is read-only, its bytes are described by the header contract every vector op reads
(a size word at `+0`, the length at `+4`, elements from `+8`), and they outlive every
`DbRef` into the store.  Everything below re-asserts that sentence.

* **One synthetic record.**  A foreign store is an ordinary minted store (`Stores::database(4)`)
  whose collection slot at `(rec, 8)` names `FOREIGN_REC` (`0x7FFF_FFF0`, past any record an
  owned store can hold).  `Store::make_foreign` writes the slot, keeps `(base, len,
  elem_size)` plus an 8-byte synthetic header and the OWNER (`ForeignOwner`: a `Vec<u8>` or a
  read-only `memmap::Mmap`; a host's token joins when F7 serves one), then locks the store.  The handle a program
  holds is `DbRef { store_nr, rec, pos: 8 }` — the shape every minted vector has, so
  `vec_header`, `length_vector`, `get_vector`, `get_elem_at`, `vec_base` and the push
  header all serve it unchanged.
* **Four accessors answer the id.**  `Store::read`, `Store::addr`, `Store::valid` and
  `Store::elem_base` answer `FOREIGN_REC` from the synthetic header (`fld < 8`) or the
  foreign base (`fld - 8`, bounded by `len * elem_size` exactly as `offset_in_bounds`
  bounds an owned record); every other accessor is built on those.  `Store::bytes_of` is
  the read-only twin of `Store::buffer` for a reader (`text_from_bytes` uses it); `buffer`
  itself refuses the id, since there is no `&mut [u8]` to give.
* **A write is the author's fault, not a corrupt reference.**  The store is `read_only`
  with `Store::FOREIGN_ORIGIN` as its lock origin; `begin_write_inner` refuses the lock
  BEFORE computing the address (asked after, the bounds test would call the reference
  corrupt), and `refuse_locked_write` routes a foreign store beside the author's `#lock`
  to `write_to_locked_store`: a development run halts with the advice for this origin (copy
  first), a production run logs it and discards the write.  Nothing can grow or move the
  block, which is exactly what every hoist needs — a production claim or resize answers a
  fresh record in the store's own buffer and never touches the mapped bytes.
* **Lifetime is the handle's.**  `Stores::foreign_vector` mints it; `Stores::free_named`
  drops the owner (`Store::release_foreign`) before the slot is recycled, so the mapping
  is released with the handle and the slot reinitialises as any other.  The bytes are the
  owner's, not the store-heap ceiling's (`store_budget` counts the tiny owned block only).
* **A slice of it is a VIEW (F4b).**  `s = m[lo..hi]` bound to a local that owns its
  backing store (a `__vdb_N`) makes THAT store foreign over the span: `OpSliceView` →
  `Stores::vector_slice_view`, where `Store::foreign_span` clamps the bounds and answers a
  `ForeignSpan` whose owner is SHARED (`Arc<ForeignOwner>`) and `Store::make_foreign`
  serves it.  The local reads the mapping's bytes in place under the same contract, and
  the bytes live until the last store serving them is released — the handle or a view,
  in any order, with no count kept.  A view is read-only (its write meets the same
  refusal); the clear before a rebind or a literal (`clear_vector_release`,
  `clear_vector`) drops the view first, so the store is ordinary, writable and empty
  again; `make_foreign` returns a record the slot held to the free tree; a view of a view
  adds its offset.  An append, an argument, a field or a return copies (`vector_slice`):
  their store holds more than the one vector, and locking it would lock the rest.
  `LOFT_NO_FOREIGN_VIEW=1` copies everywhere.
* **A library's bytes (F5).**  A cdylib bridge answers a `vector<u8>` with no copy through
  `loft_ffi::LoftStore::foreign_vector_from_owned(Vec<u8>)`: the host adopts the block as a
  foreign store (`ForeignOwner::Extern` — the `Vec`'s parts and the cdylib's own release,
  called once with the last store serving the bytes, in the cdylib's allocator since the
  host's may differ; a loaded library is never unloaded) and answers the bare `FOREIGN_REC`,
  which the two consumers of a bridge's ref return (`extensions::bridge_push_ref`,
  `codegen_runtime::from_loft_ref`) turn into the store's own handle
  (`Store::foreign_handle`) instead of claiming a header in a store that is now locked.
  The callback is `LoftStore::foreign_fn`, the LAST field of the handle (an older cdylib
  reads a prefix; every host-to-cdylib crossing is the one fixed `LoftBridgeFn` shape, so a
  longer struct shifts no other argument), and the helper copies where the host has none or
  declined.  When the dispatcher minted an empty store for the answer, the context word says
  so (`extensions::CTX_RETURN_STORE`, bit 16 above the store number) and the bytes are
  adopted INTO it (`Stores::foreign_vector_in`), never beside an orphan.  At the BIND, the
  local's own store views the answer whole — `OpAdoptVector` → `Stores::vector_adopt`, the
  op the #410 direct bind and the #409 wrapper delivery now emit where they copied every
  element; an owned answer is still copied, so its in-place `+=` holds — and the loop-buffer
  reset (`vector_buffer_reset`) and a store re-init (`Stores::clear`) drop a view rather
  than write its length.  A foreign vector handed to a bridge as an ARGUMENT is copied into
  a record of the store the bridge is given (`Stores::bridge_args`, one set-up shared by the
  interpreter's dispatcher and the generated `--native` call): a cdylib reads a vector by
  pointer arithmetic on that one store, which cannot see foreign bytes (a mapped file handed
  to any library bridge read sixteen gigabytes past the store before F5).  A foreign vector
  never pins the store — the next argument does, or one is minted for the call — and after
  the call the copies are deleted, or the minted store freed; a bridge that WROTE through a
  copy meets the same refusal a direct write does.
* **A copy OUT reads where the bytes are.**  `copy_block_between`, `Stores::copy_block` and
  the `par` workers' row readers take the source address from `Store::block_src`, which
  answers the foreign bytes for `FOREIGN_REC` (a bind `w = m` and a `par` over `m` both read
  sixteen gigabytes past the store before F4b), and `write_bytes` reads the payload through
  `bytes_of`.  A worker's borrow (`borrow_locked_for_light_worker`), a locked clone and a
  checkpoint copy carry the span, since the owner is shared.
* **Not (yet):** a record VIEW (`rec_ptr`) is not served — a foreign store holds scalar
  vectors until F5.

The unit test `store::tests::a_foreign_store_answers_a_vector_read_through_the_same_accessors`
reads a foreign store and a copied one through the same accessors byte for byte, and
`a_view_of_a_foreign_store_reads_beside_the_copied_slice` does the same for a view;
`tests/foreign_store.rs` does the same at the program level on both backends and asserts the
write refusal; the cells are `tests/scripts/174-foreign-file.loft`.

### Binary File I/O: `read_data` and `write_data`

`read_data` reads from a `DbRef` into a `Vec<u8>` (for writing to a binary file). `write_data` reads from a `&[u8]` into a `DbRef` (for reading from a binary file).

**Critical design constraint**: temp variables used for file I/O (created by `write_to_file` / `read_from_file` in `parser.rs`) are **always stored as full i32 on the stack** (`Context::Variable` always allocates 4 bytes for all integer types). This means `read_data`/`write_data` for `Parts::Byte` and `Parts::Short` must use `get_int`/`set_int`, NOT `get_byte`/`get_short`.

The reason: `get_short(rec, pos, min)` reads the null-sentinel-encoded storage (`stored_u16 = value − min + 1`) and returns the actual value. But a temp var's slot holds a raw i32 (no encoding offset). Using `get_short` on an i32 temp var returns `raw_u16 − 1`, which is off by one.

| Part type | `read_data` (store → bytes) | `write_data` (bytes → store) |
|---|---|---|
| `Base(0)` / `Base(6)` (integer/char) | `get_int` → 4 bytes | `set_int` from 4 bytes |
| `Base(1)` (long) | `get_long` → 8 bytes | `set_long` from 8 bytes |
| `Base(2)` (single) | `get_single` → 4 bytes | `set_single` from 4 bytes |
| `Base(3)` (float) | `get_float` → 8 bytes | `set_float` from 8 bytes |
| `Base(4)` (boolean) | `get_byte(_, _, 0) as u8` → 1 byte | `set_byte(_, _, 0, data[0])` |
| `Base(5)` (text) | `get_str` → UTF-8 bytes | `set_str` from UTF-8 bytes |
| `Parts::Byte(_, _)` | `get_int` → truncate to u8 → 1 byte | `set_int(i32::from(data[0]))` |
| `Parts::Short(_, _)` | `get_int` → truncate to i16 → 2 bytes | `set_int(i32::from(i16::from_le/be_bytes))` |
| `Parts::Struct(fields)` | recurse for each field | recurse for each field |
| `Parts::Enum(_)` | `get_byte` → 1 byte | `set_int(i32::from(data[0]))` |
| `Parts::Vector(elem_tp)` | iterate elements, recurse per element | `vector_append` + `write_data` per element + `vector_finish` |

**Note**: `Parts::Byte`/`Parts::Short` in `read_data`/`write_data` are designed for temp variable contexts (i32 layout). Using these with actual 1/2-byte struct fields would produce incorrect results. Struct serialization via `Parts::Struct` recursion is not yet fully tested.
