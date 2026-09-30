# No recursive directory listing
axis: clarity
met-by: fuzz/seed_program_source (strand 3)
status: open
fix: `dir.walk(suffix) -> vector<text>` in the on-demand `script` library (README § Strand 8), symlink-aware, replacing the five copies — expressible over `files()` and `is_symlink`, so never the stdlib
ref: 
probe: 005.probe.loft
expect: refused
checked: 23155096
holds: yes

no recursive listing: `files()` answers one level, so `find … -name '*.loft'` is a hand-written walk — the tree already carries one in `tools/indexer/src/scan.loft` (`walk_plan_dirs`), one in each of the two gallery scripts, one in the census and one in this port, none symlink-aware before `is_symlink` existed
