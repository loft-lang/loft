# No `copy(from, to)` builtin
axis: clarity
met-by: fuzz/seed_program_source (strand 3, the first port)
status: open
fix: `path.copy_to(dst) -> FileResult` in the on-demand `script` library (README § Strand 8) — expressible over `read_bytes`/`write_bytes`, so never the stdlib (owner, 2026-09-30)
ref: `src.copy_to(dst)` in `script` 0.1.0 (loft-libs-core #42, release script-v0.1.0, registry submission loft-lang/registry#32) — struck when the port that met it re-measures green over the library, which waits on the registry fold
probe: 004.probe.loft
expect: refused
checked: 6a26d8ca9
holds: yes

no `copy(from, to)`: a file is copied as `write_bytes(dst, read_bytes(src) ?? [])`, two calls and a null discharge for what `cp` says in one word
