# No `copy(from, to)` builtin
axis: clarity
met-by: fuzz/seed_program_source (strand 3, the first port)
status: open
fix: `path.copy_to(dst) -> FileResult` in the on-demand `script` library (README § Strand 8) — expressible over `read_bytes`/`write_bytes`, so never the stdlib (owner, 2026-09-30)
ref: 
probe: 004.probe.loft
expect: refused
checked: 02cc37e78
holds: yes

no `copy(from, to)`: a file is copied as `write_bytes(dst, read_bytes(src) ?? [])`, two calls and a null discharge for what `cp` says in one word
