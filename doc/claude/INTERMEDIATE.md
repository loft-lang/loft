
# loft Intermediate Language (IR) Reference

## Overview

The compiler pipeline is:
```
.loft source -> Parser (Value tree IR) -> State.byte_code() -> bytecode Vec<u8> -> fill::OPERATORS[opcode](&mut State) at runtime
```

The intermediate representation is the `Value` enum tree defined in `src/data.rs`.
Bytecode generation is done in `src/state/codegen.rs` via `State`.
The 233 operator functions are in `src/fill.rs`.
Variable/scope tracking during parsing is in `src/variables/` via `Function`.

---

## Contents
- [Value Enum (IR Nodes) — `src/data.rs`](#value-enum-ir-nodes--srcdatars)
- [Type Enum — `src/data.rs`](#type-enum--srcdatars)
- [AST-Level Operators (Call node op_nr)](#ast-level-operators-call-node-op_nr)
- [Bytecode State — `src/state/`](#bytecode-state--srcstate)
- [Variable Tracking — `src/variables/`](#variable-tracking--srcvariablesrs)
- [233 Bytecode Operators — `src/fill.rs`](#233-bytecode-operators--srcfillrs)
- [DbRef](#dbref)
- [Key Patterns](#key-patterns)
- [Debug Tools (`src/state/debug.rs`, debug builds only)](#debug-tools-srcstatedebugrs-debug-builds-only)

---

## Value Enum (IR Nodes) — `src/data.rs`

```rust
pub enum Value {
    Null,
    Line(u32),                         // Source line annotation
    Int(i32),                          // Integer literal
    Long(i64),                         // 64-bit integer literal
    Single(f32),                       // 32-bit float literal
    Float(f64),                        // 64-bit float literal
    Boolean(bool),                     // Boolean literal
    Enum(u8, u16),                     // Enum variant index + database type id
    Text(String),                      // Text literal
    Call(u32, Vec<Value>),             // Operator/function call: (op_nr, args)
    CallRef(u16, Vec<Value>),          // Call via fn-ref variable: (var_nr, args)
    Block(Box<Block>),                 // Scoped statement sequence
    Insert(Vec<Value>),                // Inline statements (no new scope)
    Var(u16),                          // Read variable at stack position n
    Set(u16, Box<Value>),             // Write variable at stack position n
    Return(Box<Value>),                // Return from function
    Break(u16),                        // Break out of n-th enclosing loop
    Continue(u16),                     // Continue n-th enclosing loop
    If(Box<Value>, Box<Value>, Box<Value>), // cond, then-branch, else-branch
    Loop(Box<Block>),                  // Infinite loop (exit via Break)
    Drop(Box<Value>),                  // Evaluate and discard return value
    Iter(u16, Box<Value>, Box<Value>), // for-loop: (var_nr, init, step)
    Keys(Vec<Key>),                    // Key descriptor for sorted/hash/index
}
```

### Special Var(0)

`Var(0)` is used as a **placeholder** in struct field default expressions to mean
"the current record being initialized." It is replaced with the actual record reference
at object initialization time by `Parser::replace_record_ref()` in `src/parser/expressions.rs`.
In `$` expressions inside struct field defaults, `$` maps to `Value::Var(0)`.

### Block

```rust
pub struct Block {
    pub name: &'static str,   // Debug label
    pub operators: Vec<Value>, // Ordered IR nodes
    pub result: Type,          // Return type
    pub scope: u16,            // Scope nesting level
}
```

---

## Type Enum — `src/data.rs`

```rust
pub enum Type {
    Unknown(u32),         // Forward reference placeholder (linked type id or 0)
    Null,                 // No type / null literal
    Void,                 // Function return: no value
    Never,                // Divergent expression (return/break/continue) — compatible with any type
    Integer(IntegerSpec), // Range-constrained integer (min, max, forced_size, nullable)
    Boolean,
    Float,                // f64
    Single,               // f32
    Character,            // Single unicode codepoint
    Text(Vec<u16>),       // Text + dependency variable list (for lifetime tracking)
    Keys,                 // Key spec for collection types
    Enum(u32, bool, Vec<u16>),               // def_nr, is_ref, deps
    Reference(u32, Vec<u16>),                // struct def_nr + deps (nullable)
    RefVar(Box<Type>),                       // Mutable reference argument (&T)
    Vector(Box<Type>, Vec<u16>),             // Dynamic array + deps
    Sorted(u32, Vec<(String, bool)>, Vec<u16>), // Ordered set: def_nr, [(field, asc)]
    Index(u32, Vec<(String, bool)>, Vec<u16>),  // Index: def_nr, [(field, asc)]
    Spatial(u32, Vec<String>, Vec<u16>),        // Spatial index: def_nr, [fields]
    Hash(u32, Vec<String>, Vec<u16>),           // Hash table: def_nr, [fields]
    Routine(u32),                            // Dynamic routine reference
    Iterator(Box<Type>, Box<Type>),          // (yield_type, internal_state_type)
    Function(Vec<Type>, Box<Type>),          // First-class fn: arg types + return type
                                             // Stored as i32 d_nr at runtime (same as integer)
                                             // Variables of this type are callable: f(args)
    Rewritten(Box<Type>),                    // After append rewrite (Text/structs)
}
```

**Plan-01 (2026-04-21) removed `Type::Long`.**  All integer-family
values now flow through `Type::Integer(IntegerSpec)` with i64
arithmetic on the stack and per-field storage width via
`IntegerSpec.forced_size` (see § Integer Storage Size below).
Runtime IR literals still use `Value::Long(i64)` (the literal
carrier), but `Type::Long` no longer exists as a distinct type.

### Integer Storage Size

`Integer(IntegerSpec { min, max, forced_size, .. })` is stored
compactly based on range AND on whether the value is in a struct
field versus a vector element.

**Struct fields** (regular `Parts::*`):

| Condition | Storage | Variant |
|---|---|---|
| range < 256 | 1 byte | `Parts::Byte` (encodes `raw = val - min + 1`; raw 0 is null sentinel) |
| range < 65536 | 2 bytes | `Parts::Short` (encodes `raw = val - min + 1`; raw 0 is null sentinel) |
| `forced_size == 4` (e.g. `i32` alias) | 4 bytes | `Parts::Int` (encodes `raw = val`; `i32::MIN` is null sentinel) |
| otherwise | 8 bytes | `Parts::Long` (post-2c: always i64 at rest; `i64::MIN` sentinel) |

Nullable integers with range exactly 256 or 65536 also use 1 or 2
bytes respectively.

**Vector elements** (post-@PLAN02 narrowing): vectors of narrow
integer aliases honour `forced_size` via a divergent rule because
`vector_add` raw-byte-copies element bytes — the `+1` offset of
`Parts::Short` would cause read/write mismatch.

| Width | Vector storage | Variant |
|---|---|---|
| 1 byte | direct `raw = val - min` (no +1 offset) | `Parts::Byte` (the encoding agrees with raw-byte copies) |
| 2 bytes | direct `raw = val - min` | `Parts::ShortRaw` (parallel to `Parts::Short` but without the +1 sentinel offset; introduced specifically for `vector<u16>` / `vector<i16>` elements) |
| 4 bytes | direct `raw = val` | `Parts::Int` (already direct-encoded) |
| 8 bytes | direct `raw = val` | `Parts::Long` (default fallback) |

The selection logic lives in
`src/data.rs::IntegerSpec::vector_narrow_width()` (returns
`Option<u8>` — `Some(1)` / `Some(2)` / `Some(4)` / `None` for the
8-byte fallback).  The helper `Data::narrow_vector_content()`
applies it at every parser call site that registers a vector
content type — see [DATABASE.md § Narrow vector elements](DATABASE.md#narrow-vector-elements)
for the storage-side details and the per-call-site registration
pattern.

Important gotcha for compiler contributors: `typedef.rs::fill_database`
runs ONLY on struct definitions.  Local-variable / parameter /
return-type vector registration happens at every
`database.vector(c_tp)` call site in `src/parser/`.  Both paths
must call `narrow_vector_content()` on their content type before
registering, or narrowing only takes effect for struct fields.

### Content-type indexing — three numbering schemes

Integer storage intersects three *different* index spaces that bit us
twice during the Phase 2c migration.  Knowing which is which saves
hours:

| Scheme | Where | Numbering | Example |
|---|---|---|---|
| `field.content` (on a `Field` / `Attribute`) | `src/database/types.rs::Field::content` | **0-based** primitive-type index | `integer=0`, `long=1`, `single=2`, `float=3`, `boolean=4`, `text=5`, `character=6` |
| `Key.type_nr` (on a `Key` in `keys_definition`) | `src/keys.rs::Key::type_nr` | **1-based** key-type discriminator | `integer=1`, `long=2`, `single=3`, `float=4`, `boolean=5`, `text=6`, `character=7` |
| `Content::X` (runtime-dispatched key value) | `src/keys.rs::Content` | variant name | `Content::Long(i64)`, `Content::Single(f32)`, `Content::Float(f64)`, `Content::Str(Str)` — no separate Int variant, since post-2c all integer-family values dispatch through `Long(i64)` |

When reading field values through the key-lookup path
(`state/io.rs::io_push_keys`), the `Key.type_nr` → `Content::X`
mapping is one hop; when reading via the attribute path
(`OpGetInt` / `OpGetLong` / `OpGetInt4` etc.), `field.content` picks
the opcode directly, so the two indices never meet at runtime.
Mixing them up in a codegen fix *does* meet at runtime — with
silent data corruption.

### Dependency Lists (`Vec<u16>`)

Types carrying `Vec<u16>` track ownership for scope-based freeing.
See `src/data.rs` (Type enum doc) and `src/scopes.rs` (module doc) for
the full semantics.  `Type::RefVar(Box<Type>)` means "stack reference"
— a DbRef into another variable's stack slot; `depend()` delegates to
the inner type.

---

## AST-Level Operators (Call node op_nr)

These are the operator names used in `Call(op_nr, args)` at the AST level,
as listed in `data.rs`:

```
OpAdd   OpMin   OpMul   OpDiv   OpRem   OpPow
OpNot   OpLand  OpLor   OpEor   OpSLeft OpSRight
OpEq    OpNe    OpLt    OpLe    OpGt    OpGe
OpAppend  OpConv  OpCast
```

The actual numeric `op_nr` values are resolved via the operator registry in
`src/parser/mod.rs` during parsing.

---

## Bytecode State — `src/state/`

```rust
pub struct State {
    bytecode: Arc<Vec<u8>>,     // Main bytecode stream
    stack_cur: DbRef,           // Current stack frame (a DB record in store 1000)
    pub stack_pos: u32,         // Current stack pointer
    pub code_pos: u32,          // Current position in bytecode
    pub database: Stores,       // All data stores
    pub arguments: u16,         // Stack size of function arguments
    pub stack: HashMap<u32, u16>, // code_pos -> stack level (for scoping)
    pub vars: HashMap<u32, u16>,  // code_pos -> variable stack position
    pub calls: HashMap<u32, Vec<u32>>, // code_pos -> called def_nrs
    pub types: HashMap<u32, u16>,      // code_pos -> type id
    pub library: Arc<Vec<Call>>,       // Extern Rust function table
    pub library_names: HashMap<String, u16>,
    text_positions: BTreeSet<u32>,   // debug: set of absolute positions of live Strings
    line_numbers: BTreeMap<u32, u32>,
    fn_positions: Vec<u32>,          // d_nr → bytecode entry point (for OpCallRef)
    pub const_refs: Vec<DbRef>,      // d_nr → CONST_STORE DbRef for vector constants
}

pub type Call = fn(&mut Stores, &mut DbRef);
```

**`const_refs`** (@PLN82 Phase A, 2026): one entry per definition;
zero for non-constant defs, populated for vector constants whose
records were pre-built into `CONST_STORE` (store 1) during
`byte_code()`.  `OpConstRef(d_nr)` indexes into this vector.
The previous `text_code: Vec<u8>` string constant pool was retired
when long strings (>= 256 bytes) moved into `CONST_STORE` via
`OpConstStoreText`; short strings stay embedded in the bytecode
stream via `OpConstText`.

The `State` struct in production carries additional fields for
source-position lookup, coroutine frames, parallel arms, the
shadow call-frame vector, and `data_ptr` — see `src/state/mod.rs`
for the canonical definition.  See also
[DATABASE.md § Constant store (`CONST_STORE`)](DATABASE.md#constant-store-const_store)
for the storage-side view of `OpConstRef` and the lifetime + safety
properties.

The stack is stored as a database record in store 1000 (index 0 in `Stores::allocations`).
`stack_pos` starts at 4 (offset past the 4-byte record header slot reserved for the return address
of `main`). `execute()` immediately pushes `u32::MAX` as the sentinel return address, so on entry
to `main` the effective `stack_pos = 8`.

### Stack Frame Layout

For a function `fn foo(a: T1, b: T2) -> R { ... }`:

```
absolute offset from stack_cur.pos:
  [caller's stack ...]
  [a]        ← size_of(T1) bytes, compile-time position 0
  [b]        ← size_of(T2) bytes, compile-time position size(T1)
  [ret-addr] ← 4 bytes (u32 code_pos), compile-time position = args_size = size(T1)+size(T2)
  [locals]   ← compile-time positions start at args_size+4
```

`State::arguments` records `args_size` after scanning a function's parameter list.

### Variable Position Encoding (`var[N]` in bytecode dumps)

Each variable is accessed via `get_var(encoded_pos)` / `put_var(encoded_pos, val)`.

- **Compile-time alloc position `N`**: the value of `Stack::position` at the moment the variable
  was first pushed/allocated onto the compile-time stack. For a function argument the first bytes
  are pushed at N=0; for locals, N=args_size+4 or higher.
- **`var[N]` in bytecode dumps**: the dump shows `var[N]` where `N = stack_at_instruction - encoded_pos`. `N` is the compile-time allocation position of the variable.
- **Actual encoded value**: `encoded_pos = stack_at_instruction - N`. Stored as a u16 in the bytecode stream after the opcode byte.

Runtime formula for `get_var(encoded_pos)`:
```
absolute_address = stack_cur.pos + runtime_stack_pos - encoded_pos
                 = stack_cur.pos + (Z + compile_stack_pos) - (compile_stack_pos - N)
                 = stack_cur.pos + Z + N
```
where `Z` = absolute stack position at the start of the function's arguments (runtime `stack_pos`
before any args were pushed).

So `var[N]` always resolves to `stack_cur.pos + Z + N`, the variable's fixed absolute address for
the current call frame — regardless of how much has been pushed onto the stack since.

`put_var(encoded_pos, val)` writes to `stack_cur.pos + runtime_stack_pos + size_of::<T>() - encoded_pos`. This differs from `get_var` by `size_of::<T>()`, so callers must account for the size offset.

### `OpDatabase` vs `OpConvRefFromNull`

- `OpConvRefFromNull` → `Stores::null()` → allocates a fresh store with `rec=0`. This store is
  used as the "home" for the struct allocation in `OpDatabase`.
- `OpDatabase(var, db_tp)` → reads the existing DbRef from `var`, calls `clear`+`claim`+`set_default_value` on it, then writes `{store_nr, rec=1, pos=8}` back to `var`. The store it operates on is the one allocated by `ConvRefFromNull`.

### Store LIFO Invariant

`Stores::database()` allocates store `max` and increments `max`. `Stores::free()` just decrements `max` by 1 without verifying `al == max-1`. This means stores **must be freed in exact LIFO (reverse-allocation) order**; out-of-order frees corrupt `max` and will cause subsequent allocations to overwrite a still-live store.

### `text_positions` (debug-only `String` liveness tracker)

Text variables on the stack are stored as Rust `String` objects (`size_of::<String>() = 24 bytes`
on 64-bit). In debug builds, `State` tracks which stack positions hold live `String`s:

- `OpText` (`state.text()`) → inserts `stack_cur.pos + stack_pos` into `text_positions` before advancing `stack_pos`.
- `OpFreeText(pos)` (`state.free_text()`) → computes `stack_cur.pos + stack_pos - pos`, calls `shrink_to(0)` on the `String`, and removes the absolute position from `text_positions`. Asserts it was present (double-free detection).
- `OpFreeStack(value, discard)` (`state.free_stack()`) → after decrementing `stack_pos` by `discard`, asserts that `text_positions` has **no entries** in the discarded range. Violation = "Not freed texts" panic.

**Consequence**: every `String` allocated by `OpText` in a block must be freed by `OpFreeText` before the block's `OpFreeStack` runs — except the block's *return* variable, which must be allocated **outside** the block's stack range (i.e. at an enclosing scope) so its position falls below the discard range.

### `Store::addr` layout

```rust
pub fn addr<T>(&self, rec: u32, fld: u32) -> &T {
    debug_assert!(rec * 8 + fld + size_of::<T>() <= store.size * 8, "out of bounds");
    unsafe { self.ptr.offset(rec as isize * 8 + fld as isize).cast::<T>() … }
}
```

Address = `ptr + rec * 8 + fld`. The factor-of-8 comes from the 8-byte record header
(the size of a single-word `u64` slot reserved per record). For the stack store,
`stack_cur.rec` is whatever `claim(1000)` returned, and `stack_cur.pos = 8` (fixed
by `Stores::database()`). Therefore:

```
absolute byte offset into store's backing buffer
  = stack_cur.rec * 8 + stack_cur.pos + runtime_stack_pos
  = stack_cur.rec * 8 + 8 + runtime_stack_pos
```

In debug builds `addr` and `addr_mut` assert `rec * 8 + fld + size_of::<T>() <= store.size * 8`.  Without this check, a garbage `DbRef` (e.g. from `format_stack_float`'s off-by-4 bug) silently reads invalid memory and causes SIGSEGV rather than a panic with a useful location.

A `DbRef` created by `OpCreateStack` has `{store_nr=stack_cur.store_nr,
rec=stack_cur.rec, pos=stack_cur.pos + var.absolute_position}`. When
`addr::<String>(r.rec, r.pos)` is called on it, the actual byte offset is
`r.rec * 8 + r.pos = stack_cur.rec * 8 + stack_cur.pos + var.absolute_position`.
This is the same formula as `get_var`, so both paths address the same memory.

### Stack text operators: `string_mut` vs `string_ref_mut` vs `GetStackText`

Three closely related helpers deal with text buffers on the stack:

| Helper | Finds the `String` via… | Used by |
|---|---|---|
| `string_mut(pos)` | `stack_cur.pos + stack_pos - pos` (direct — variable IS a `String`) | `OpText`, `OpAppendText`, `OpFormatFloat`, `OpFreeText` |
| `string_ref_mut(pos)` | reads a `DbRef` at `stack_cur.pos + stack_pos - pos`, then follows `(r.rec, r.pos)` | `OpAppendStackText`, `OpFormatStackFloat`, `OpFormatStackInt`, … |
| `GetStackText` | pops a `DbRef` `r` from stack, calls `database.store(&r).addr::<String>(r.rec, r.pos)` | `OpGetStackText` (return of a `RefVar(Text)` function) |

`string_ref_mut` uses `store_mut(&self.stack_cur)` regardless of the DbRef's
`store_nr`, because the DbRef was always created via `OpCreateStack` from the same
stack frame.  `GetStackText` uses `database.store(&r)` which selects the store via
`r.store_nr` — correct only if `r.store_nr == stack_cur.store_nr`, which holds when
the DbRef was created by `OpCreateStack` in the same thread.

**The "Stack" format ops** (`OpAppendStackText`, `OpFormatStackFloat`, etc.) differ
from their plain counterparts (`OpAppendText`, `OpFormatFloat`) in that they expect the
target text buffer to be a `RefVar(Text)` — i.e. a pointer to a `String` stored
elsewhere on the stack — rather than directly being the `String`.  This indirection is
what allows a text-returning function to write into a caller-supplied buffer.

### `generate_var` — reading stack variables by type

`generate_var` (state.rs) emits different opcode sequences depending on the variable's type:

| Type | Opcodes emitted | Result on stack |
|---|---|---|
| `Integer` | `OpVarInt(pos)` | the `i32` value |
| `Long` | `OpVarLong(pos)` | the `i64` value |
| `Float` | `OpVarFloat(pos)` | the `f64` value |
| `Text` (owned) | `OpVarText(pos)` or `OpArgText(pos)` | a `Str` (ptr+len) |
| `Vector` | `OpVarVector(pos)` | the 12-byte `DbRef` pointing to the container record |
| `Reference` / `Enum(true)` | `OpVarRef(pos)` | the 12-byte `DbRef` pointing to the struct record |
| `RefVar(Integer)` | `OpVarRef(pos)` → `OpGetInt(0)` | the referenced `i32` |
| `RefVar(Text)` | `OpVarRef(pos)` → `OpGetStackText` | dereferences the `DbRef` to a `String *` |
| `RefVar(Vector)` | `OpVarRef(pos)` → `OpGetStackRef(0)` | dereferences the `DbRef` to another `DbRef` |

**`RefVar(Vector)` note:** `OpVarRef(pos)` pushes the `DbRef` of the `OpCreateStack`
temp record. `OpGetStackRef(0)` then reads the 12-byte `DbRef` stored at offset 0 in
that temp record, which is the **caller's vector `DbRef`** — the `OpCreateStack`
instruction stores it there when the call is set up. This is the correct result for
`v += extra` (Issue 4 fix): `assign_refvar_vector` in `parser.rs` emits
`OpAppendVector(Var(v_nr), rhs, rec_tp)`; `generate_var` for `Var(v_nr)` uses the
`OpVarRef + OpGetStackRef(0)` path to supply the caller's vector container `DbRef`
to `vector_append`, which modifies the caller's vector in place.

---

## Variable Tracking — `src/variables/`

```rust
pub struct Function {
    pub name: String,
    pub file: String,
    steps: Vec<u8>,          // Byte steps for each variable on stack
    unique: u16,             // Unique name counter
    current_loop: u16,       // Current loop nesting (MAX = top-level)
    loops: Vec<Iterator>,    // Active for-loop contexts
    variables: Vec<Variable>,
    work_text: u16,          // Work variable for text operations
    work_ref: u16,           // Work variable for ref operations
    work_texts: BTreeSet<u16>,
    work_refs: BTreeSet<u16>,
    names: HashMap<String, u16>,  // name -> last variable index
    pub done: bool,
    pub logging: bool,
}

pub struct Variable {
    name: String,
    type_def: Type,
    source: (u32, u32),   // (line, col) of declaration
    scope: u16,           // 0 = function arguments
    stack_pos: u16,       // Position on stack frame (u16::MAX = unassigned)
    first_def: u32,       // Bytecode-sequence position of first assignment (u32::MAX = never)
    last_use: u32,        // Bytecode-sequence position of last read (0 = never)
    uses: u16,            // Reference count
    argument: bool,
    defined: bool,
}
```

Variables are referenced in IR by their `stack_pos` (`u16`).
Scope 0 is always function arguments.
The same variable name may have multiple `Variable` instances across scopes.

### Live Intervals (`first_def` / `last_use`)

`compute_intervals(function, ir)` in `variables/` walks the entire IR tree in sequential
order (assigning each node a monotonically-increasing sequence number `seq`) and fills:

- `first_def` — the sequence number of the `Value::Set(v, …)` node that first defines `v`.
  Critically, the *value expression* inside `Set` is visited **before** `first_def` is
  recorded, so a block-return temporary that lives only inside the RHS expression does not
  create a false overlap with `v`.
- `last_use` — the maximum sequence number of any `Value::Var(v)` reference.

`validate_slots(function)` uses these intervals to detect conflicting stack-slot assignments:
two variables with the same `stack_pos` overlap if their live intervals intersect
(`left.first_def <= right.last_use && right.first_def <= left.last_use`).
Same-name + same-slot pairs are exempt (they represent the sequential-block reuse pattern).
Only owning types (Text, Reference, Vector, owned Enum) are checked; primitive scalars can
safely share slots.

Called (in debug builds) from `state.rs` after `byte_code()` completes, immediately before
`state.execute()`.

---

## Bytecode Operators — `src/fill.rs`

Operators are indexed by their position in the `OPERATORS` array. The array is generated by `src/create.rs::generate_code` from the `#rust "..."` annotations on `Op`-prefixed definitions in the default library. Operator names in the array follow the convention `Op<CamelCase>` → `op_<snake_case>` (e.g. `OpAddInt` → `add_int`). The exception is `OpReturn` → `op_return` (to avoid the Rust keyword).

The index of each operator equals its `op_code` field in `Data::Definition` (a `u16`), and is the index into `fill::OPERATORS` at runtime. It reaches the bytecode stream through `emit_op` — one byte for ops below 255, two for the rest (see the budget below).

### Opcode budget — and why the count is never written down

`op_code` is a **`u16`**, and the stream encodes it in one byte OR two
(`src/state/mod.rs::emit_op`): ops `0..=254` are a single byte, and byte
`255` is an **escape prefix** — the interpreter reads a second byte `ext`
and dispatches `OPERATORS[255 + ext]`. `ext` is a `u8`, so the addressable
range is `0..=510`: **511 opcodes**, not 256.

The table is **not** near saturation, and work parked on the belief that it
is should be re-read: `PLANNING.md` § O1 and `ROADMAP.md` still defer
superinstruction merging as *"opcode table full (254/256)"*, which describes
a one-byte-only encoding that no longer exists. `PERFORMANCE.md` § P1
corrected this in 2026-06 and O1 can proceed — a superinstruction lands in
the escape range as a two-byte opcode.

**No number is quoted here on purpose.** The count lived in prose in five
documents, was hand-copied between them, and every copy drifted: the "254 of
256" above stood while the table had long passed 255, and the corrected
"269/511" in `PERFORMANCE.md` was already stale when written down. The count
is derived — ask for it:

```bash
make ops-census        # declared, one-byte + escape-range, ceiling, free
```

That same report answers the question the table's growth actually raises —
which operators anything still **emits**. An operator is cheap to add and
invisible to retire; `scripts/op_census.py` classifies each as live,
unexercised (a site emits it, no program in the tree does), or orphan
(nothing emits it and nothing in `src/` names it). It is a report, never a
gate, and RELEASE.md § The operator census makes it a per-release read.

Plan-01 phase 5 (2026-04-21) reclaimed 34 duplicate `Op*Long` slots when the
integer family collapsed to i64.

Categories:

### Control Flow (0–6)
`goto`, `goto_word`, `goto_false`, `goto_false_word`, `call`, `op_return`, `free_stack`

### Boolean (7–12)
`const_true`, `const_false`, `cast_text_from_bool`, `var_bool`, `put_bool`, `not`

### Integer (13–56)
- Constants: `const_int` (4-byte), `const_short` (2-byte), `const_tiny` (1-byte)
- Var/Put: `var_int`, `var_character`, `put_int`, `put_character`
- Conversions: `conv_int_from_null`, `conv_character_from_null`, `cast_int_from_text`, `cast_long_from_text`, `cast_single_from_text`, `cast_float_from_text`, `conv_long_from_int`, `conv_float_from_int`, `conv_single_from_int`, `conv_bool_from_int`
- Math: `abs_int`, `min_single_int`
- Arithmetic: `add_int`, `min_int`, `mul_int`, `div_int`, `rem_int`
- Bitwise: `land_int`, `lor_int`, `eor_int`, `s_left_int`, `s_right_int`
- Comparison: `eq_int`, `ne_int`, `lt_int`, `le_int`

### Long (57–81)
Similar structure to Integer but for `i64`.
Extra: `format_long`, `format_stack_long`

### Single / Float (82–131)
Similar arithmetic. Math functions are merged: `math_func_single` / `math_func_float` dispatch 10 unary ops (cos/sin/tan/acos/asin/atan/ceil/floor/round/sqrt) via a 1-byte fn_id; `math_func2_single` / `math_func2_float` dispatch 2 binary ops (atan2/log). Separate entries for pow, pi, e.
Extra: `format_single`, `format_stack_single`, `format_float`, `format_stack_float`

### Text (152–175)
`var_text`, `arg_text`, `const_text`, `conv_text_from_null`, `length_text`, `length_character`, `conv_bool_from_text`, `text`, `append_text`, `get_text_sub`, `text_character`, `conv_bool_from_character`, `clear_text`, `free_text`, `eq_text`, `ne_text`, `lt_text`, `le_text`, `format_text`, `format_stack_text`, `append_character`, `text_compare`, `cast_character_from_int`, `conv_int_from_character`

### Enum (176–184)
`var_enum`, `const_enum`, `put_enum`, `conv_bool_from_enum`, `cast_text_from_enum`, `cast_enum_from_text`, `conv_int_from_enum`, `cast_enum_from_int`, `conv_enum_from_null`

### Database / Struct (185–215)
- Record ops: `database` (allocate), `format_database`, `format_stack_database`
- Ref: `conv_bool_from_ref`, `conv_ref_from_null`, `free_ref`, `var_ref`, `put_ref`, `eq_ref`, `ne_ref`, `get_ref`, `set_ref`
- Field access: `get_field`, `get_int`, `get_character`, `get_long`, `get_single`, `get_float`, `get_byte`, `get_enum`, `set_enum`, `get_short`, `get_text`, `set_int`, `set_character`, `set_long`, `set_single`, `set_float`, `set_byte`, `set_short`, `set_text`

### Vector (216–228)
`var_vector`, `length_vector`, `clear_vector`, `get_vector`, `vector_ref`, `cast_vector_from_text`, `remove_vector`, `insert_vector`, `new_record`, `finish_record`, `append_vector`, `get_record`, `validate`

### Collections (229–241)
`hash_add`, `hash_find`, `hash_remove`, `eq_bool`, `ne_bool`, `panic`, `print`, `iterate`, `step`, `remove`, `clear`, `append_copy`, `copy_record`

### Static / Stack (242–247)
`static_call`, `create_stack`, `get_stack_text`, `get_stack_ref`, `set_stack_ref`, `append_stack_text`, `append_stack_character`, `clear_stack_text`

### Callable fn-refs (op_code 232)
`call_ref` — `OpCallRef(fn_var_dist: u16, arg_size: u16)`. Reads the `d_nr` stored at the fn-ref variable's stack position (via `get_var(fn_var_dist)`), looks it up in `State::fn_positions`, and dispatches via `fn_call`. Declared in `default/02_files.loft` to avoid renumbering the file-I/O operators in `01_code.loft`.

`fn_positions: Vec<u32>` on `State` — maps each definition index to its bytecode entry point. Populated at the start of each `execute_argv` / `execute_log` call from `data.definitions`.

---

## DbRef

Universal pointer `(store_nr: u16, rec: u32, pos: u32)` — see [DATABASE.md](DATABASE.md) for the full definition and key/compare API. Used for stack frames (store 1000), struct instances, and vector elements.

---

## Key Patterns

### Object Initialization Order

`object_init()` in `src/parser/expressions.rs` fills unspecified struct fields in **definition order**.
Fields provided in the object literal are set first; then for each missing field,
the stored default expression is emitted, with `Var(0)` replaced by the actual record ref.

### Struct Default Expressions

Stored in `Definition.attributes[n].value` as a `Value` tree.
The `$` token in field defaults maps to `Value::Var(0)` (the current record placeholder).
`Parser::replace_record_ref()` substitutes `Var(0)` → actual record `Value` recursively
over `Call`, `If`, `Block`, and leaf nodes.

### Iterator Protocol

`Iter(var_nr, init, step)`:
- `init` evaluates to the iterator state and stores it in `var_nr`
- `step` advances the iterator and yields the next value (or signals done)
- The outer `Loop(Block)` with `Break` exits when done

---

## Debug Tools (`src/state/debug.rs`, debug builds only)

Two free functions in `src/state/codegen.rs` are compiled only in debug builds
(`#[cfg(debug_assertions)]`).

### `ir_contains_var(value, v) -> bool`

Recursively checks whether a `Value` tree contains any `Var(v)` node. Handles all
`Value` variants: `Call` args, `Set`/`Return`/`Drop` inner, `If` branches,
`Block`/`Loop` operators, `Insert` items, and `Iter` create/next/extra nodes.

Used in `generate_set` at the top of the first-assignment path (`pos == u16::MAX`) to
detect self-reference bugs — a variable appearing in its own first-assignment expression
always indicates a parser bug (storage not yet allocated), and panics with a clear message
naming the function and the broken IR.

### `print_ir(value, data, vars, depth)`

Pretty-prints a `Value` IR tree to stderr in loft-like syntax. Handles all `Value`
variants with appropriate indentation. Called from `def_code` when the `LOFT_IR`
environment variable is set.

**Usage:**
```bash
LOFT_IR=n_test    cargo test my_test -- --nocapture  # one function by name substring
LOFT_IR=*         cargo test my_test -- --nocapture  # all user functions
LOFT_IR=          cargo test my_test -- --nocapture  # same as *
```

Output format:
```
=== IR: n_test ===
{  // block
  d = t_5Color_double(c)
  ...
}
===
```

The `LOFT_IR` gate additionally checks the `logging` flag on the function definition
(true for non-default-library functions) to suppress output for built-in operators.

---

## See also
- [COMPILER.md](COMPILER.md) — Lexer, parser, two-pass design, IR, type system, scope analysis, bytecode
- [DATABASE.md](DATABASE.md) — Store allocator, Stores schema, DbRef, vector/tree/hash implementations
- [INTERNALS.md](INTERNALS.md) — calc.rs, stack.rs, create.rs, native.rs, ops.rs, parallel.rs
- [DESIGN.md](DESIGN.md) — Algorithm catalog including bytecode dispatch, store layout, and collection complexity
