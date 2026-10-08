# No membership test on a vector — "is this path in the changed set" is a four-line loop
axis: clarity
met-by: scripts/check_bundle_fresh (strand 4d) — `is_changed`, the original's one-line `grep -qxF` over the changed-file list
status: open
fix: `contains(self: vector<T>, value: T) -> boolean` (and the `in` spelling if the language wants one) in the stdlib or the on-demand `script` library — equality on the element type, the one question `index_of` and `filter` both answer at more cost
ref: 
probe: 017.probe.loft
expect: refused
checked: 1eddc9d57
holds: yes

The original asks `printf '%s\n' "$changed" | grep -qxF "$1"` — one line, exact-line match — three times.  loft has `contains` on `text` and on nothing else: a `vector<text>` has `filter`, `map` and `reduce`, so the port writes a four-line `is_changed(self: vector<text>, path: text)` with a `for` and an early `return`, and every later script that holds a list of names will write it again.  Eleven of the port's 38 non-comment lines against the original's 27 are the helper and the `Bundle` struct that stands in for bash's tab-separated `printf` row; the struct is a gain in names, the helper is not.
