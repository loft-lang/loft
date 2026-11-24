<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->

# COMPILER-history.md — what COMPILER.md no longer describes

The record behind [COMPILER.md](COMPILER.md).

## Rust code generation (`src/generation/`) — retired with the rustc writer (@PLN225, 2026-11-24)

The section as it stood in COMPILER.md until `src/generation/` was removed (`@C170`):


`src/generation/` provides the `Output` struct and `rust_type` function used to transpile compiled loft programs to Rust source files. This is used only during development to regenerate `src/fill.rs` and `src/native.rs` from the `#rust "..."` annotations in the default library. It is not involved in the normal interpreter execution path.

#### `Output<'a>`

```rust
pub struct Output<'a> {
    pub data: &'a Data,         // read-only view of all definitions
    pub stores: &'a Stores,     // runtime type schema
    pub counter: u32,           // unique label counter for generated identifiers
    pub def_nr: u32,            // definition number currently being emitted
    pub indent: u32,            // current indentation level
    pub declared: HashSet<u16>, // variable slots already declared in this function
}
```

Bundles the read-only compile-time data with the mutable emission state so that individual emit functions receive a single context argument.

#### `rust_type(tp, context) -> String`

Maps a loft `Type` to the corresponding Rust type string. The `context` parameter controls the form:

| Context | Effect |
|---|---|
| `Context::Argument` | Stack/argument passing type (e.g. `Str` for text, `i32` for integer) |
| `Context::Variable` | Local variable type (e.g. `String` for text — owned heap allocation) |
| `Context::Reference` | Prefixes the argument type with `&` |

Integer types are mapped to `u8`/`u16`/`i8`/`i16`/`i32` based on the `Integer(min, max)` range. Reference, vector, and collection types all map to `DbRef`.
