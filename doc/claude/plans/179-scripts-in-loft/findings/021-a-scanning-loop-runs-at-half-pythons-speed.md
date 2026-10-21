# A line-scanning port's own loop runs at half Python's speed, with no single hot line
axis: performance
met-by: scripts/opl_points (1.9× its original over 200k lines), tests/dump_ignored_tests (2.2×, 0.68 s against 0.31 s)
status: open
fix: interpreter mechanism work on the operations such a loop is made of — a split per line, a slice and a character per field, a comparison chain — measured first (loft-optimize skill), not a kernel for this shape; the call into a library native already went from ~270 ns to ~170 ns less dispatch (756e8a99f)
ref: —
probe: 021.probe.loft
expect: over:250
checked: 9c2c9306d
holds: yes

The probe is `opl_points`' inner loop with nothing else in it: 200k OPL-like lines split on spaces, each field told apart by its first character.  490 ms interpreted against 245 ms for the same loop in Python.  The port's profile has no line that dominates — half the time is the per-field loop (`split`, `field[0]`, the `kind ==` chain), a third the hand-written `%hex%` unescape, and rewriting that unescape to copy whole slices bought 4 % — so this is the interpreter's cost per operation, not a missing primitive.  `dump_ignored_tests` shows the same once its walk is fixed: 0.36 s of its loop is the scan itself, the rest two library calls per `matches` (finding 013 cured `lines()`; this is what remained).
