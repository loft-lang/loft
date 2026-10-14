# No recursive directory listing
axis: clarity
met-by: fuzz/seed_program_source (strand 3)
status: open
fix: `dir.walk(suffix) -> vector<text>` in the on-demand `script` library (README § Strand 8), symlink-aware, replacing the five copies — expressible over `files()` and `is_symlink`, so never the stdlib
ref: `dir.walk(suffix)` in `script` 0.1.0 (loft-libs-core #42, release script-v0.1.0, registry submission loft-lang/registry#32) — struck when the port that met it re-measures green over the library, which waits on the registry fold
probe: 005.probe.loft
expect: refused
checked: 647c14b83
holds: yes

no recursive listing: `files()` answers one level, so `find … -name '*.loft'` is a hand-written walk — the tree already carries one in `tools/indexer/src/scan.loft` (`walk_plan_dirs`), one in each of the two gallery scripts, one in the census and one in this port, none symlink-aware before `is_symlink` existed
