<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# LOCKS-history.md — the record of [LOCKS.md](LOCKS.md)

## Locks removed with the rustc writer (2026-11-24)

The rustc writer retired with `@C170`, and its runtime (`src/codegen_runtime.rs`) with it.  The
row it carried:

| `LAZY_FETCH`, `LAZY_FETCH_REFUSAL` (`codegen_runtime.rs`) | lazy drivers of the rustc writer's programs | set by `init` before any thread | **not needed** | `OnceLock`, or gone with the writer (@PLN225) | none |
