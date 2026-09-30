# A small pure-loft library under `~/.loft/lib` corrupts the store on its first call
axis: behaviour
met-by: the `script` library (strand 8), the first time `tests/dump_ignored_tests` loaded it
status: open
fix: loft#1776 — a load-time layout or table sized from the library's definitions; until it closes, the ports that would use `script` carry a marked local copy of the helper and fold it back
ref: loft#1776
probe: 
expect: 
checked: 
holds: unprobed

A one-method package (`pub fn literals(self: text) -> vector<text>`, a three-line body) placed under `~/.loft/lib/script/` crashes the interpreter on its first call — `Store access out of bounds … the reference is corrupt`, or `realloc(): invalid next size` — deterministically, while the same method with a longer body, or with an unrelated second function beside it, runs; the shape of the whole file decides, not the function called. `regex` and `arguments` under the same directory are fine. `--native` fails earlier with `cannot find function cr_call_site` in the generated Rust. A probe needs a library on disk, so this one is re-measured by hand against the issue.
