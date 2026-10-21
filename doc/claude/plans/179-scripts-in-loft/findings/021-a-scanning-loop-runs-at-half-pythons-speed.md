# A line-scanning port's own loop runs at half Python's speed, with no single hot line
axis: performance
met-by: scripts/opl_points (1.9× its original over 200k lines), tests/dump_ignored_tests (2.2×, 0.68 s against 0.31 s)
status: open
fix: measured 2026-10-21 (probe 426 ms interpreted, Python 249): `split` ~125 ms against Python's ~58, the per-field loop ~285 against ~178.  The loop's largest cost is the loop variable — every field copied into an owned `String` and freed (`B-Copy`), ~100 ms: read in place the probe is −28 %.  The native generator already elides that copy (`@FR-R-TextBorrow`); bringing its admission to the interpreter is @PLN198.  `split`'s allocator cost is partly gone: a claim on an empty free tree bumps the wilderness (ae79c6e1b, split −10 %).  Left after @PLN198: the character tests (each `==` converts both sides to integer) and `split`'s per-part vector append
ref: —
probe: 021.probe.loft
expect: over:250
checked: 9c2c9306d
holds: yes

The probe is `opl_points`' inner loop with nothing else in it: 200k OPL-like lines split on spaces, each field told apart by its first character.  490 ms interpreted against 245 ms for the same loop in Python.  The port's profile has no line that dominates — half the time is the per-field loop (`split`, `field[0]`, the `kind ==` chain), a third the hand-written `%hex%` unescape, and rewriting that unescape to copy whole slices bought 4 % — so this is the interpreter's cost per operation, not a missing primitive.  `dump_ignored_tests` shows the same once its walk is fixed: 0.36 s of its loop is the scan itself, the rest two library calls per `matches` (finding 013 cured `lines()`; this is what remained).
