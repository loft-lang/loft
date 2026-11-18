<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Reading a rewrite's decision from outside

How to see what a rewrite did at a site and at which condition it refused one, without a
checker inside the compiler.  The rules themselves are
[formal/rewrites.md](formal/rewrites.md); the ruling this page follows is `(Fact-Oracle)` in
[formal/facts.md](formal/facts.md).

## Contents
- [What is decided](#what-is-decided)
- [The three levels](#the-three-levels)
- [What a refusal logs](#what-a-refusal-logs)
- [The state of every rule](#the-state-of-every-rule)
- [To implement](#to-implement)
- [What is not built](#what-is-not-built)

## What is decided

- **No self-check.**  The compiler has no mode in which it verifies its own analyses while a
  program runs.  A layer that validates the analyses adds complexity to check complexity.
- **Logging is the assurance.**  A decision is trusted because it can be read from outside:
  what fired, where, and at which condition a site was refused.
- **The clause, not the cause.**  A refusal names the CLAUSE of the rule that failed.  It
  does not describe why that clause failed at that site: which statement, which variable,
  which callee.
- **A closed vocabulary.**  A rule's labels are fixed strings, one per clause of its
  statement.  A label carries no name, type or expression of the program.
- **Counted, never listed.**  The program-wide output is one line per rule and clause with a
  count.  Per-site lines exist only for one function, on request.

## The three levels

| Question | Answered by | State |
|---|---|---|
| Where does each rewrite fire? | `LOFT_REWRITE_CENSUS`, and per function `LOFT_REWRITE_CENSUS_FN` ([PROFILING.md](PROFILING.md)); `make rewrite-census` | built |
| How often does each clause refuse, program-wide? | `LOFT_DECLINE_CENSUS`; `make decline-census` | built; few rules report |
| In one function, which rules stopped and at which clause? | a per-function count of refusals by rule and clause | to build |
| Why did that clause fail at this site? | a small loft probe of the case, read with `loft introspect` or the rule's own `LOFT_TRACE_*` switch ([DEBUG_PROBES.md](DEBUG_PROBES.md)) | the way of working |

The effect of a rewrite is also readable by its `LOFT_NO_*` switch
([BOTH_BACKEND_SWITCHES.md](BOTH_BACKEND_SWITCHES.md), [NATIVE_SWITCHES.md](NATIVE_SWITCHES.md)):
the program with the rewrite off is the control.

## What a refusal logs

A rule works in two phases:

1. **Match** — it looks for its pattern in the code.  Code that is not the pattern is not a
   refusal, and nothing is logged for it: a function without a loop is no refused unrolling,
   a function that makes a call is no refused leaf.
2. **Validate** — for a match, it tests whether something around the site makes the rewrite
   unsafe or pointless there: the vector is also written, the local escapes, the function
   yields.

A refusal is a MATCH that validation rejected, and
`rewrite_census::declined(rule, site, label)` (`src/rewrite_census.rs`) is called there:

- **Validation only.**  The label is the validation clause that rejected the match.  A rule
  with no validation phase has nothing to report (`@no-decline`).
- **Telling the phases apart in the code.**  An admission function often runs its pattern
  tests and its safety tests as one list of early exits.  An exit belongs to the match when
  it asks whether this is the construct the rule is about, and to validation when it asks
  about anything around it — other uses, aliases, effects, the enclosing function.
- **One label per clause.**  Where the rule's statement lists its conditions (`when ALL of:
  (1) … (2) … (3) …`), the labels are those clauses.  A gate with many exits behind one
  clause reports that one clause.
- **One count per site.**  A site counts once, under the first clause that refused it.
- **A fixed label.**  `declined` takes `&'static str`, so a label cannot be built from a
  name, a type or a number of the program.  A gate that also has something to say about the
  site answers `rewrite_census::Refusal { clause, about }`: the census counts `clause`, and a
  trace of one function prints both.
- **A bounded vocabulary.**  `make decline-census` prints how many labels each rule uses
  over the bench programs, and `ARGS=--check-labels` fails a rule that passes `LABEL_CAP`
  (`scripts/decline_census.py`).  Labels are counted where they are reported, because a gate
  hands its clause on as a value.
- **No cost when off.**  The call returns at once unless the census is switched on.

An admission logs no reason.  The reason a rewrite fired is the rule, and the clause it
fired under is in the name it reports (`R-Header/kept`, `R-Apart/records`).

## The state of every rule

`make reason-census` (`scripts/reason_census.py`, a scan of `src/`, no build) gives each rule
one of five states:

| State | Meaning | Recorded by |
|---|---|---|
| reports | it calls `declined` for its refusals | the call |
| nothing to report | its shape is its whole condition | `// @no-decline: <why>` above its `fired` call |
| owes | it refuses on conditions it does not report yet | `// @decline-owed: <the conditions>` above its `fired` call |
| silent | it fires and nobody has read it | nothing; `--missing` lists these |
| not counted | no census counts it | the phase table in formal/rewrites.md (`not counted — <kind>: <why>`) |

`--owed` lists the rules that owe: the worklist of this page.  `--check` reads
`tests/reason_census.baseline` and fails when a rule falls back to silent, when a marker has
no text or stands above no `fired` call, when a rule carries a marker and also reports, and
when a rule no census counts has no row in the phase table.

An `owes` marker names the main conditions the rule's admission code tests.  It is a summary
for a reader choosing what to wire; the complete list is the deciding function.

## To implement

In this order.  Each step keeps every rewrite's behaviour: the rewrite census shows the same
admissions before and after, and the code emitted for the bench programs is byte-identical.

1. **Wire the rules whose gate already names its failing clause.**  Many admission functions
   return a reason string or call a local decline helper for their trace.  Each needs one
   `declined` call where that reason is consumed, and its `@decline-owed` marker removed.
2. **Wire the rules whose gate answers `None` or `false`.**  The gate returns the clause
   that failed, and its one caller reports it.  Taken by yield: first the rules whose
   refusals sit in routines over their performance bar ([PERFORMANCE.md](PERFORMANCE.md)).
3. **The per-function listing.**  `LOFT_DECLINE_CENSUS_FN=<file>` writes one line per
   function, rule and clause with a count, as `LOFT_REWRITE_CENSUS_FN` does for admissions,
   limited to one function when a name is given.  It names no site: a loop or a local has a
   name, a decision inside an expression has none, and tracking positions through a tree the
   compiler is rewriting is a tracer that would have to be exactly right.  The site key stays
   what it is, a means to count a site once.
   The function on a line is the one the RULE names for that decision.  A rule that works
   across functions — one that inlines a callee into its caller, one that writes a twin —
   may name the caller, the callee or the twin, so one refusal can stand under different
   functions in different rules.  A rule about a call may add the CALLEE, and a rule that
   has the source line at hand may add it: two optional columns beside the label, there to
   give enough context to write the right probe.  None is required to find either, and the
   label itself stays a fixed string.
   The listing holds COUNTS, never a verdict for a function: one function can match a rule
   at one place and be refused at another, and its inlined copies can go either way in
   different callers.  Read beside the admissions of `LOFT_REWRITE_CENSUS_FN`, a function
   shows how many matches a rule rewrote and how many each clause rejected, and not which.

A rule moves from `owes` to `reports` when its marker is removed and its call is in;
`--check` refuses both at once, so the `owes` count only falls.

**OPEN: 3** — each decides how the remaining rules are wired.

- **Is a label the rule's clause or the gate's test?**  A wired rule reports the fixed
  reason its gate gives, one per TEST ("the local is rebound before its
  delivery"), which is finer than the clauses of the rule's statement.  Over the bench
  programs a rule uses 1 to 6 of them, but a gate can hold many more (the place rule's has
  59), so `LABEL_CAP` bounds nothing for such a rule.  Either the test is the label and the
  cap goes, or each gate's tests are grouped under the statement's clauses, one mapping per
  rule.
- **A rule that reports some of its clauses.**  A rule is `reports` as soon as one of its
  gates calls `declined`, and its `@decline-owed` marker must go then, so the clauses that
  do not report are off the worklist (R-HostStore's write-back clause, R-Apart's
  value and vector instances, R-CtorLiteral's callee side).  Either a rule may carry a marker
  for what it still owes beside its calls, or it stays `owes` until every clause reports.
- **A gate that validates before it knows it matched.**  R-FaultFlag walks a `for` statement
  for what it may not hold before it knows the statement holds a fault form at all; it counts
  a refusal only where a fault form was seen first, which misses a statement refused ahead of
  its first one.  The same order is in other gates still to wire.

## What is not built

- **A listing of refused sites inside expressions.**  See step 3: the compiler counts them
  and does not try to name them.
- **A full dump of a decision.**  It would have every gate describe the state it looked at,
  written once per gate and kept in step with it.  The per-rule trace switches give that
  depth where it was needed.
- **A table from rule to trace switch.**  The clause says where to look, and a probe of the
  case — a few lines of loft that hold the refused site and nothing else — shows the rest
  faster than a lookup would.
- **A reason for an admission.**
- **A run-time check of the compiler's facts**, under any switch.
