# Start-up costs ~15 ms before the first statement
axis: performance
met-by: scripts/script_census (strand 1)
status: open
fix: where the interpreter's ~15 ms goes before the first statement — `LOFT_TIMING=1` on the installed binary: `parse_default` 5–10 ms for 748 stdlib defs on a WARM run, scopes 3, lints 2, codegen 3 — against the 7 ms a whole `cat \
ref: 

start-up of a `hello`, INSTALLED binary, idle box, 2026-09-30, 5 runs: `--interpret` 15–22 ms (Python 15–19, bash 5) — at the bar; the DEFAULT path (native, warm cache) 32–37 ms — 2× Python.  The from-source binary's 78–84 ms was rule 4 (program cache off), not the language
