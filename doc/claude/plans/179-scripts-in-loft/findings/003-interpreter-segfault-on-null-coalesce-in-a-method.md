# The interpreter segfaults on `f(self.v[n - 1] ?? "")` in a mutating method
axis: behaviour
met-by: scripts/script_census (strand 1)
status: open
fix: [loft#1773](https://github.com/loft-lang/loft/issues/1773) — the census binds the element to a local first, marked to fold back
ref: loft#1773

the interpreter segfaults at `OpFreeText` on `argv_call(self.result[n - 1] ?? "")` inside a struct method that also appends to `self.result`, once enough heap is behind it; `--native` runs it; four other spellings run; no extraction reproduces it yet
