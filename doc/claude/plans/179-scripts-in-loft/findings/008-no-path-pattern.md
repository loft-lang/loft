# No path pattern: a glob is four lines
axis: clarity
met-by: scripts/wasm_bundle_stamp (strand 3)
status: open
fix: `pattern.glob() -> vector<text>` in the on-demand `script` library (README § Strand 8) — a pattern match over `list_dir`, so never the stdlib
ref: `pattern.glob()` in `script` 0.1.0 (loft-libs-core #42, release script-v0.1.0, registry submission loft-lang/registry#32) — struck when the port that met it re-measures green over the library, which waits on the registry fold
probe: 008.probe.loft
expect: refused
checked: 6a26d8ca9
holds: yes

no path pattern: bash's `default/*.loft` is a `list_dir` + an `ends_with` filter + a path join, four lines for one word
