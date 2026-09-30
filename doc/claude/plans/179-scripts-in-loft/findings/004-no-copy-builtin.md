# No `copy(from, to)` builtin
axis: clarity
met-by: fuzz/seed_program_source (strand 3, the first port)
status: open
fix: a `copy` builtin beside `move`, answering `FileResult`
ref: 

no `copy(from, to)`: a file is copied as `write_bytes(dst, read_bytes(src) ?? [])`, two calls and a null discharge for what `cp` says in one word
