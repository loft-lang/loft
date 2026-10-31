<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# plans/README — history

What the working [README.md](README.md) no longer carries.

## The bug-filing ground rule, as it read until 2026-10-31

Phases filed every bug they surfaced as a P-issue row in PROBLEMS.md before the phase closed.

The May 2026 @P211 hunt is the canonical example: the original
P-issue was native `yield text`, but the diagnostic probes
surfaced @P217 (text accumulator), @P218 (format-with-param in
generator body), @P219 (vector-for-yield), @P220 (`""` in
`vector<text>`) and @P221 (server-side BufReader).  All five were
filed in the same commit window; none were lost.  The @P217
follow-up hunt then surfaced @P222 / @P223 (narrower self-concat
shapes) — same rule applied.
