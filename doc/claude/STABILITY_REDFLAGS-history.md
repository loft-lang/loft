<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->

# STABILITY_REDFLAGS history

Records moved out of [STABILITY_REDFLAGS.md](STABILITY_REDFLAGS.md).  The files they name under
`src/generation/` and `src/codegen_runtime.rs` left the tree with the rustc writer (@PLN225, `@C170`).

## Cluster A — the re-derivation-site row of the rustc writer

| Site | loft2 location | Smell | Mode |
|---|---|---|---|
| `is_borrowed_view` — computed **twice, divergently** | `src/state/codegen.rs:1716,2441` (interp) **vs** `src/generation/dispatch.rs:178` (native) | the same `0x8000` source-free bit, two different structural derivations → drift on the OOB/hidden-only edge | **divergence** (H4) |

Closed by @PLN225: `src/generation/dispatch.rs` left the tree with the rustc writer (`@C170`).

## Cluster D — the heap-ref + character facet, landed

**Landed — the heap-ref + character facet (Cluster D D.1/D.2).** The four named
typed-null encoders (`write_typed_null` native, `emit_typed_null` interp,
`STRING_NULL`, `init_ref_sentinel`) are converged:

- **Heap-ref null = one source, `DbRef::NULL` (`keys.rs`).** Every heap-ref null
  encoder (native `write_typed_null` / `default_native_value` / `dispatch` / `calls`
  / `coroutine`, the `codegen_runtime`/`parallel`/`structures` runtime writers, and
  the interp `init_ref_sentinel` / `null_ref_sentinel`) read the single
  `DbRef::NULL` const instead of re-spelling `DbRef { store_nr: u16::MAX, … }`. The
  drift was a mixed `pos: 0` (interp/canonical) vs `pos: 8` (native) literal —
  semantically inert (`is_null()` keys off `store_nr`, ignores `pos`) but real byte
  drift, now gone. Round-trip matrix byte-identical on both backends; regression
  `tests/scripts/407-cluster-d-null-sentinel-roundtrip.loft`.
- **Character null H4 cell, found + closed.** `Parser::null` folded `character`
  into `OpConvIntFromNull` (the i64 integer sentinel), so `-> character { return
  null; }` emitted `return i64::MIN` into an `i32` return slot — native rustc E0308,
  interpreter tolerated. Now routes to `OpConvCharacterFromNull` (char-domain null
  `'\0'`), correct on both backends.
