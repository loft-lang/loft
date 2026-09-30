# No recursive directory listing
axis: clarity
met-by: fuzz/seed_program_source (strand 3)
status: open
fix: a `files(dir, recursive)` or a `find(dir, suffix)` in the stdlib, symlink-aware, replacing the five copies
ref: 

no recursive listing: `files()` answers one level, so `find … -name '*.loft'` is a hand-written walk — the tree already carries one in `tools/indexer/src/scan.loft` (`walk_plan_dirs`), one in each of the two gallery scripts, one in the census and one in this port, none symlink-aware before `is_symlink` existed
