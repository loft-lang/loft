# `LOFT_PROFILE` charges a long library call to a LATER line, not the line that made it
axis: performance
met-by: tests/dump_ignored_tests — profiled while re-measuring its gap to Python
status: open
fix: charge a call into a `use`d library's compiled code to the calling line, as PROFILING.md already says it does — the sampler takes its samples by operation count, so a long native call that runs no operations ends the interval it sits in, and its wall time lands on wherever the next sample falls
ref: —

On `dump_ignored_tests`, `LOFT_PROFILE=1` reported 58 % of the run on `regex.loft:86` — the `pattern_error` call in `report` — and 1.1 s for `report` in all.  Re-run with `LOFT_NO_NATIVE_LIBS=1` (the libraries interpreted, so the sampler sees inside them), `report` is 0.5 s and the largest single cost is `script`'s `walk` at 0.75 s, which the first profile had spread over other lines.  A one-call probe shows the mechanism: a 650 ms `walk` over `tests/` is charged 100 % to `02_files.loft:1291` (inside `text.dir`, the next loft operation after the call returns), not to the line that called `walk`.  PROFILING.md says a library's time "lands on the loft line that called them"; it lands on the next sampled line.  The cost is a profile that names the wrong hot spot: the first fix this port's profile suggested would have been to the wrong function.
