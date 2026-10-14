# Start-up costs ~15 ms before the first statement
axis: performance
met-by: scripts/script_census (strand 1)
status: open
fix: a smaller stdlib — strand 8's direction OUT (fewer of the 748 definitions parsed before the first statement) — and where the interpreter's ~15 ms goes before the first statement — `LOFT_TIMING=1` on the installed binary: `parse_default` 5–10 ms for 748 stdlib defs on a WARM run, scopes 3, lints 2, codegen 3 — against the 7 ms a whole `cat \
ref: 
probe: 001.probe.loft
expect: startup-over:12
checked: 647c14b83
holds: yes

start-up of a `hello`, INSTALLED binary, idle box, 2026-09-30, 5 runs: `--interpret` 15–22 ms (Python 15–19, bash 5) — at the bar; the DEFAULT path (native, warm cache) 32–37 ms — 2× Python.  The from-source binary's 78–84 ms was rule 4 (program cache off), not the language

Two bars: against Python's `hello` (15–19 ms) the installed binary is AT the bar — the recheck measured 13 ms; against the bash original the stamp port was met on (a 12 ms `cat | sha256sum` pipeline) it is not, by a millisecond, which is why the probe's bar is 12.  The from-source `target/release/loft` reads 60–80 ms because its program cache is off (PERFORMANCE.md rule 4) and must never be the number quoted here.

