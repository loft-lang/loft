# Investigation-style plans

*Read this when the deliverable is **mechanism understanding before fix design** —
several failure clusters to catalogue, where source-reading alone will not converge
and the fix cannot be designed without the catalogue. For a feature ship or a fix
landing, the standard shape in SKILL.md is the right one.*

## Opening one

1. Identity + flat directory as above, README from the **investigation** template;
   add a `probes/` subdirectory.
2. **Stage A first — write probes before reading source.**  Probes are the
   executable spec for what "understood" means: a hypothesis is confirmed when the
   probe-pair diff confirms it.  Be **liberal** — missing a crucial shape is the
   worst failure; redundant variants cost nothing because they get attic-curated at
   the *end* of Stage A, not during.  Extract at least one probe from a *real
   consumer*, not only synthetic cases — real extraction catches classes synthetic
   probes miss.
3. **Run every probe on every execution mode** the result can diverge across
   (e.g. interpreter vs compiler).  Record the full results matrix.
4. Keep a **flat probe table** (one row per probe: file, shape, cluster, status).
   Curate into groups only when the suite is large *and* multiple people read it
   cold.
5. For each failure mode, write a cluster doc with a **verified-vs-hypothesized
   accountability table** — every mechanism statement is either VERIFIED (cited
   trace/code-line) or HYPOTHESIZED (marked).  Without this column, hypotheses
   drift into the prose as if they were facts.
6. **Track severity as two separate fields** — corruption/panic/hang *and* leak —
   so "FIXED" can't conflate them (closing corruption while leaks persist is a
   false-fix trap).
7. **Tools as needed, not upfront.**  Don't build a debugging framework first; add
   the *one* tool blocking progress, revert nice-to-haves, and list what you added
   in a `Tool gaps` section — tools added during the plan are part of its output.
8. Add a Status + next-session roadmap (per-cluster action items with effort
   estimates).

**Probe → regression migration.**  Probes stay in `probes/` during the
investigation and graduate to the regression suite **per cluster, as each cluster's
fix lands** — not all at once.  The plan stays open during phased implementation
and closes when the last cluster's regression is in the suite.  A probe is
graduation-ready only when it passes **all** of: assertions pass · clean process
exit (no crash at teardown — "PASSED prints" is not enough, check the exit code) ·
no leak warning · bounded runtime (seconds, not a hang).  A probe that passes
assertions but fails any other gate stays in `probes/` with a status note;
graduate a representative sibling from the same cluster instead.

## Why this shape, in five lines

- **Its own template and reading order** (Status → Probes → Cluster docs → Roadmap).
  Forcing investigation work into the feature-ship template either bloats the README
  or loses the catalogue.
- **Probe-first beats source-first.** Source-reading with no probe to ground it
  explores code paths without converging; the probe suite is the executable spec for
  "understood".
- **Liberal probes, attic-curate after** — curate at the end of Stage A, not during.
  Real-consumer extraction is non-negotiable: it surfaces classes synthetic probes
  never imagine.
- **Verified-vs-hypothesized accountability prevents drift.** Mark every mechanism
  claim; the table answers "do we actually know?" honestly.
- **Tools as needed, not upfront.** Add the one tool blocking progress, list it in
  `Tool gaps`, revert the nice-to-haves.

