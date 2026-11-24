<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->

# DEBUG_INTROSPECT-history.md — what DEBUG_INTROSPECT.md no longer describes

The record behind [DEBUG_INTROSPECT.md](DEBUG_INTROSPECT.md).

## `--show-rust` and the Rust source map — retired with the rustc writer (@PLN225, 2026-11-24)

The Sections table carried this row until the writer was retired (`@C170`):

| Flag | Output | When to use |
|------|--------|-------------|
| `--show-rust` | Generated Rust (`--native-emit` shape) | Native-codegen bugs, rustc errors |

### The Rust source map, as it stood

The `--show-rust` (and any `--native` compilation) emits
`// loft:<file>:<line>` comments above each function header and
each statement.  `rustc` errors on `/tmp/loft_native.rs:1450` map
back to a .loft line by reading the nearest preceding comment.

```rust
// loft:/tmp/myprog.loft:7
fn n_first(stores: &mut Stores, mut var_a: DbRef) -> Str {
  ...
  // loft:/tmp/myprog.loft:8
  return Str::new(...)
}
```

When rustc reports a borrow-check error or type mismatch, scroll
upward in the generated file from the error line to the nearest
`// loft:` comment — that's the source line under suspicion.
