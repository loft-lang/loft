# No path pattern: a glob is four lines
axis: clarity
met-by: scripts/wasm_bundle_stamp (strand 3)
status: open
fix: `pattern.glob() -> vector<text>` in the on-demand `script` library (README § Strand 8) — a pattern match over `list_dir`, so never the stdlib
ref: 
probe: 008.probe.loft
expect: refused
checked: 02cc37e78
holds: yes

no path pattern: bash's `default/*.loft` is a `list_dir` + an `ends_with` filter + a path join, four lines for one word
