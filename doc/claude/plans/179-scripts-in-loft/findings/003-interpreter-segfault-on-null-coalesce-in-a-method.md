# The interpreter segfaults on `f(self.v[n - 1] ?? "")` in a mutating method
axis: behaviour
met-by: scripts/script_census (strand 1)
status: fixed
fix: [loft#1773](https://github.com/loft-lang/loft/issues/1773) — the census binds the element to a local first, marked to fold back
ref: loft#1773 — fixed in c9e70e2ac, guarded by tests/scripts/1773-a-coalesce-inside-a-conditional-arm-is-freed-on-every-path.loft (seven cells, the census rhythm among them).  Still owed: fold the census's workaround back (`scripts/script_census`, the bound `last` before `argv_call`) and run the census interpreted — that run is this finding's probe, the heap-state-dependent shape it was met in
probe: 
expect: 
checked: 
holds: unprobed

the interpreter segfaults at `OpFreeText` on `argv_call(self.result[n - 1] ?? "")` inside a struct method that also appends to `self.result`, once enough heap is behind it; `--native` runs it; four other spellings run; no extraction reproduces it yet
