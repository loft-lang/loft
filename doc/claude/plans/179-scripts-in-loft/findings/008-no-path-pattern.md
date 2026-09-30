# No path pattern: a glob is four lines
axis: clarity
met-by: scripts/wasm_bundle_stamp (strand 3)
status: open
fix: a glob, or a `files(dir, pattern)` in the stdlib
ref: 

no path pattern: bash's `default/*.loft` is a `list_dir` + an `ends_with` filter + a path join, four lines for one word
