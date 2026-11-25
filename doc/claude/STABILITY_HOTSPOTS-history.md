<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->

# STABILITY_HOTSPOTS history

Records moved out of [STABILITY_HOTSPOTS.md](STABILITY_HOTSPOTS.md).

## H6 — the inline byte-field read on the rustc writer (closed; the writer left with @PLN225, `@C170`)

**Follow-up DONE (`f6f660f8`): the inline `Struct{..}.x` byte-field read on
native.**  The pre-eval/hoisting path (`generation/dispatch.rs`) hoisted a
store-mutating call arg into a `_harg_*` temp and emitted the CALL form
`OpGetByteNullable(cell, _harg, …)` for ANY `Value::Call` — including inline
`#rust` ops (byte/short field reads), which have no callable fn, so native
compilation failed with "not found in scope".  It predated this work (broke plain
`OpGetByte` too) and was NOT narrow-int-specific.  Fixed by gating that path on
`def_fn.rust().is_empty()`: only true user-fn / `codegen_runtime` Op-stub calls
take the call-form hoist; inline `#rust` ops fall through to the normal emit, which
inlines the template (its own `let db = @v1; …` sequences the mutating arg before
the read borrow).  Regression `loft-suite/attic/scripts/inline-construct-narrow-read.loft`
(both backends, all four narrow widths × nullable/not-null/null).
