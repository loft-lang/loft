# Start-up costs ~15 ms before the first statement
axis: performance
met-by: scripts/script_census (strand 1)
status: open
fix: a smaller stdlib — strand 8's direction OUT (fewer of the 748 definitions parsed before the first statement) — and where the interpreter's ~15 ms goes before the first statement — `LOFT_TIMING=1` on the installed binary: `parse_default` 5–10 ms for 748 stdlib defs on a WARM run, scopes 3, lints 2, codegen 3 — against the 7 ms a whole `cat \
ref: re-measured 2026-10-15 (@PLN192's session) with `LOFT_TIMING=1` and `strace -tt` — `LOFT_PROFILE` samples loft operations and sees nothing of a start (0 samples over a 26 ms hello).  A warm `hello` on this box, best of 30: 20.8 ms before, 17.9 ms after, Python 15.0, bash 2.2.  The program bundle IS memory-mapped (`open_bundle_into`), and before it two things ran every time: the stdlib key read and hashed every `default/*.loft` (2.9 ms), and the compiled-stdlib dispatch check hashed them again (1.6 ms) — 240 kB through two hashes per run to ask whether seven files had moved.  `cache::stdlib_fingerprint` now keeps both answers in a stamp of the files' sizes and nanosecond mtimes: 0.1 ms and 0.2 ms.  What a warm start still pays, by `LOFT_TIMING`'s `warm_load` line: materialising the 7 MB bundle into the definition table (`bundle` 4.8 ms — the map is copied out, not used in place; `LOFT_CODEGEN_STORE`'s skeleton load keeps bodies in the store for codegen only and moves the wall clock by nothing here), the scope pass 1.5 ms and codegen 1.5 ms (neither is cached: the bundle holds the parse, not the bytecode), and the process itself (~4 ms to the first file opened, ~2 ms to exit, dropping that table).  The next step is a bundle the interpreter runs FROM — bytecode cached beside the parse, or a store-backed definition table — which is @PLN11's M6 line, not a script's
probe: 001.probe.loft
expect: startup-over:12
checked: 4b1553fc
holds: yes

start-up of a `hello`, INSTALLED binary, idle box, 2026-09-30, 5 runs: `--interpret` 15–22 ms (Python 15–19, bash 5) — at the bar; the DEFAULT path (native, warm cache) 32–37 ms — 2× Python.  The from-source binary's 78–84 ms was rule 4 (program cache off), not the language

Two bars: against Python's `hello` (15–19 ms) the installed binary is AT the bar — the recheck measured 13 ms; against the bash original the stamp port was met on (a 12 ms `cat | sha256sum` pipeline) it is not, by a millisecond, which is why the probe's bar is 12.  The program cache is on for every binary since @C133 (`LOFT_NO_CACHE=1` is the cold reference, 87 ms here), so a from-source `target/release/loft` measures the same path as an installed one after its first run.

