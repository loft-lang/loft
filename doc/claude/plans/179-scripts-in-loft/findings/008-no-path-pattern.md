# No path pattern: a glob is four lines
axis: clarity
met-by: scripts/wasm_bundle_stamp (strand 3)
status: fixed
fix: `pattern.glob() -> vector<text>` in the on-demand `script` library (README § Strand 8) — a pattern match over `list_dir`, so never the stdlib
ref: `pattern.glob()` in `script` 0.1.1 (registry, 2026-10-13), name order, dot-names skipped as the shell's; scripts/wasm_bundle_stamp re-twinned over it 2026-10-14 (SCOREBOARD.md) — `"../default/*.loft".glob()` for the four lines
probe: 008.probe.loft
expect: refused
checked: 4b1553fc
holds: no

no path pattern: bash's `default/*.loft` is a `list_dir` + an `ends_with` filter + a path join, four lines for one word
