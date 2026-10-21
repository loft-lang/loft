# A directory walk costs 6× Python's: every file call re-checks its whole path, and `files()` stats every entry
axis: performance
met-by: tests/dump_ignored_tests, fuzz/seed_program_source, scripts/rule_predicate_audit — every port over `script`'s `walk`
status: open
fix: a listing that answers each entry's kind from the directory read (what Python's `os.scandir` gives without a `stat`), so a walk never asks again about a path it was just handed; and the `@FR-Path-Case` check paid once per directory a walk enters rather than once per call — both are stdlib/runtime surface, the owner's call
ref: —
probe: 020.probe.loft
expect: over:60
checked: 8d7364cd2
holds: yes

The probe walks `tests/` (18,257 files in 2,639 directories) over the stdlib's own primitives, as `script`'s `walk` does: 390 ms interpreted on the owner's macOS box, where Python's `os.scandir` walk of the same tree takes 60 ms.  Sampling the walk (macOS `sample`, the loft profiler sees none of it) puts the time in the kernel, from two runtime calls: `resolve_path` → `case_clash` → `canonical` (a full `realpath`, one `getattrlist` per path component) on EVERY file operation — `file(dir)`, `files()`, and each `is_symlink` — and `fill_file`'s `stat` per listed entry, for a `size` a walk never reads.  An `is_symlink` costs about 15 µs here against about 1 µs for the `lstat` it stands for; `script` 0.4's `walk` asks it only of directories, which halved the library's walk (0.68 s → 0.36 s), and what is left is these two.  On Linux the case check is one `stat`, so the gap there is smaller and not yet measured.  A name read from a listing is already the on-disk spelling, which is why a listing that hands out kinds can skip the check without weakening the rule.
