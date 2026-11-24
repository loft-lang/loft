<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# LOGGER-history.md — the record of [LOGGER.md](LOGGER.md)

## Production mode on the compiled backend (moved on 2026-11-24)

Production mode was interpreter-only until 2026-09-01: `--native`, the default backend, went
straight to `report_and_exit()`, so a `panic()` under `production = true` aborted and wrote
nothing — the outcome the feature exists to prevent (loft#1263).  The fix shared one decision,
`runtime_error::logged_in_production`, between the interpreter's builtins and the `n_panic` /
`n_assert` bodies the rustc writer wrote (`src/generation/mod.rs`, hand-written since they had
no `#rust` template).  The rustc writer retired with `@C170`; the C runtime asks its own
production fault logger.
