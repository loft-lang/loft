# No `copy(from, to)` builtin
axis: clarity
met-by: fuzz/seed_program_source (strand 3, the first port)
status: fixed
fix: `path.copy_to(dst) -> FileResult` in the on-demand `script` library (README § Strand 8) — expressible over `read_bytes`/`write_bytes`, so never the stdlib (owner, 2026-09-30)
ref: `src.copy_to(dst) -> boolean` in `script` 0.1.1 (registry, 2026-10-13); fuzz/seed_program_source re-twinned over it 2026-10-14 (SCOREBOARD.md) — the two calls and the null discharge are one call
probe: 004.probe.loft
expect: refused
checked: f570a06e
holds: no

no `copy(from, to)`: a file is copied as `write_bytes(dst, read_bytes(src) ?? [])`, two calls and a null discharge for what `cp` says in one word
