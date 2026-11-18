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
- **The test, not the cause.**  A refusal names the TEST of the rule's gate that failed.  It
  does not describe why that test failed at that site: which statement, which variable,
  which callee.
- **A closed vocabulary.**  A rule's labels are the fixed reasons its gate gives, one per
  test.  A label carries no name, type or expression of the program.
- **Counted, never listed.**  The program-wide output is one line per rule and test with a
  count.  Per-site lines exist only for one function, on request.

## The three levels

| Question | Answered by | State |
|---|---|---|
| Where does each rewrite fire? | `LOFT_REWRITE_CENSUS`, and per function `LOFT_REWRITE_CENSUS_FN` ([PROFILING.md](PROFILING.md)); `make rewrite-census` | built |
| How often does each test refuse, program-wide? | `LOFT_DECLINE_CENSUS`; `make decline-census` | built; few rules report |
| In one function, which rules stopped and at which test? | a per-function count of refusals by rule and test | to build |
| Why did that test fail at this site? | a small loft probe of the case, read with `loft introspect` or the rule's own `LOFT_TRACE_*` switch ([DEBUG_PROBES.md](DEBUG_PROBES.md)) | the way of working |

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

- **Validation only.**  The label is the validation test that rejected the match.  A rule
  with no validation phase has nothing to report (`@no-decline`).
- **Telling the phases apart in the code.**  An admission function often runs its pattern
  tests and its safety tests as one list of early exits.  An exit belongs to the match when
  it asks whether this is the construct the rule is about, and to validation when it asks
  about anything around it — other uses, aliases, effects, the enclosing function.
- **One label per test.**  The label is the fixed reason the gate gives for the test that
  failed ("a local still views the destination after the call"), which is finer than the
  clauses of the rule's statement and says more about the probe to write.  No table maps a
  test to a clause; a reason that reads badly is reworded in the gate.
- **A gate that validates before it knows it matched undercounts.**  Some gates walk the
  code once, testing what it may not hold before they know the pattern is there
  (`(R-FaultFlag)`: a `for` statement is refused at a forbidden node ahead of its first fault
  form).  Such a gate counts a refusal only once the pattern was seen, so a non-match is
  never logged and a match refused early is missed.  The gate is not reordered for the log;
  the rule carries a `@decline-owed` marker naming what is missed.
- **One count per site.**  A site counts once, under the first test that refused it.
- **A fixed label.**  `declined` takes `&'static str`, so a label cannot be built from a
  name, a type or a number of the program.  A gate that also has something to say about the
  site answers `rewrite_census::Refusal { clause, about }`: the census counts `clause`, and a
  trace of one function prints both.
- **The vocabulary is the gate's.**  `make decline-census` prints how many labels each rule
  uses over the bench programs.  Nothing caps the number: a gate with many tests has many
  reasons, and `&'static str` already keeps a program's names out of them.
- **No cost when off.**  The call returns at once unless the census is switched on.

An admission logs no reason.  The reason a rewrite fired is the rule, and the clause it
fired under is in the name it reports (`R-Header/kept`, `R-Apart/records`).

## The state of every rule

`make reason-census` (`scripts/reason_census.py`, a scan of `src/`, no build) gives each rule
one of six states:

| State | Meaning | Recorded by |
|---|---|---|
| reports | it calls `declined` for its refusals | the call |
| reports in part | some of its gates call `declined`, others say nothing | the call, and `// @decline-owed: <the gates that say nothing>` above a `fired` call |
| nothing to report | its shape is its whole condition | `// @no-decline: <why>` above its `fired` call |
| owes | it refuses on conditions it does not report yet | `// @decline-owed: <the conditions>` above its `fired` call |
| silent | it fires and nobody has read it | nothing; `--missing` lists these |
| not counted | no census counts it | the phase table in formal/rewrites.md (`not counted — <kind>: <why>`) |

`--owed` lists the rules that owe, in whole or in part: the worklist of this page.  `--check`
reads `tests/reason_census.baseline` and fails when a rule falls back to silent, when a marker
has no text or stands above no `fired` call, when a rule marked `@no-decline` also reports,
and when a rule no census counts has no row in the phase table.

A rule with several gates is wired one gate at a time.  While a gate says nothing, the rule
keeps a `@decline-owed` marker naming it, and reads `reports in part`; the marker goes with
the last gate.  The script cannot see a second gate by itself: whoever wires one gate of a
rule writes the marker for the rest.

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
   function, rule and test with a count, as `LOFT_REWRITE_CENSUS_FN` does for admissions,
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
   shows how many matches a rule rewrote and how many each test rejected, and not which.

A rule moves from `owes` through `reports in part` to `reports`: its first call is in, then
its marker is removed with the last gate.

**OPEN: 0**

## What is not built

- **A listing of refused sites inside expressions.**  See step 3: the compiler counts them
  and does not try to name them.
- **A full dump of a decision.**  It would have every gate describe the state it looked at,
  written once per gate and kept in step with it.  The per-rule trace switches give that
  depth where it was needed.
- **A table from rule to trace switch.**  The label says where to look, and a probe of the
  case — a few lines of loft that hold the refused site and nothing else — shows the rest
  faster than a lookup would.
- **A reason for an admission.**
- **A run-time check of the compiler's facts**, under any switch.
