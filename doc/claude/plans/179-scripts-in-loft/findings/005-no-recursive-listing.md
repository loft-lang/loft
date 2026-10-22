# No recursive directory listing
axis: clarity
met-by: fuzz/seed_program_source (strand 3)
status: fixed
fix: `dir.walk(suffix) -> vector<text>` in the on-demand `script` library (README § Strand 8), symlink-aware, replacing the five copies — expressible over `files()` and `is_symlink`, so never the stdlib
ref: `dir.walk(suffix)` in `script` 0.1.1 (registry, 2026-10-13), symlink-aware; fuzz/seed_program_source and tests/dump_ignored_tests re-twinned over it 2026-10-14 (SCOREBOARD.md) — their two hand-written walks are gone
probe: 005.probe.loft
expect: refused
checked: 9c2c9306d
holds: no

no recursive listing: `files()` answers one level, so `find … -name '*.loft'` is a hand-written walk — the tree already carries one in `tools/indexer/src/scan.loft` (`walk_plan_dirs`), one in each of the two gallery scripts, one in the census and one in this port, none symlink-aware before `is_symlink` existed
