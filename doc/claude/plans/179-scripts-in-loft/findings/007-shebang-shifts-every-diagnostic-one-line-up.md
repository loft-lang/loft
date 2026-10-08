# A `#!` first line shifts every diagnostic one line up
axis: behaviour
met-by: fuzz/seed_program_source (strand 3)
status: fixed
fix: the lexer skips the `#!` line without counting it — count it, so a bare script's diagnostics name the line the editor shows
ref: 03b3e836
probe: 007.probe.loft
expect: refused-line:6
checked: 231a813a9
holds: no

Met as "the refusal of `{n:0>5}` points at the line above the format string" in the seeder port; re-measured with two probe shapes: without a `#!` line the diagnostic points at the right line, with one it points one line up — every diagnostic in every bare script, not the format width's. A type error under a `#!` line pointed two lines up (at the `fn` header), which is a second shape to look at with the fix.
