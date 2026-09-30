# `File.lines()` and `text.split()` walk text character by character in loft — 16 µs a line interpreted
axis: performance
met-by: tests/dump_ignored_tests (strand 3) — 584k lines of Rust read through `lines()`; scripts/opl_points — 200k lines of OPL through `split('\n')` and `split(' ')`
status: open
fix: `lines()` as a runtime primitive (a `#rust` body over `content()`, the CR/LF rule kept) — reading a file as lines is an I/O service the runtime should answer, the one kind the stdlib keeps; `split` on a character is the same shape
ref: 
probe: 013.probe.loft
expect: over:150
checked: 23155096
holds: yes

`LOFT_PROFILE=1` on the port: 90.9 % of 10.7 s in `t_4File_lines`, three lines of `02_files.loft` (the `for ch in c` walk, the slice into `result`, the `\n` test); the regex matching the port exists for is 2 %. The original, Python's `f.read().splitlines()` plus its regexes, runs the whole job in 298 ms; the interpreted port takes 5.9 s and the native one about 0.75 s. Python's split is C; loft's is a loft loop the interpreter executes per character, so the port loses by 20× on the one stdlib call every text-processing script makes first.

The same loop under another name: `scripts/opl_points` splits its whole input on `\n` and each line on `' '`, and `LOFT_PROFILE=1` puts 73 % of 4.15 s (50k lines) in `t_4text_split`, the `for c in self` walk of `02_files.loft`. The Python original runs 200k lines in 850 ms; the interpreted port takes 9.1 s, and every other cost in it (the `%hex%` unescape, the rounding) is a rounding error beside the split. One native body under `split` and `lines` moves both ports from **slower** to a contest.
