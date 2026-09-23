<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# STABILITY_ROADMAP.md — every open stability item, in finishing order

> **STANDING RULE — in stability work, bugs get FIXED, not filed.**
> This queue is this agent's stream (feature work — gaming/engine —
> belongs to a parallel agent) and it is **work-limited, not
> time-limited**: done when the queue is finished, not at a date.  A
> surfaced bug gets fixed in the same working session; the bug-filing
> escape hatches (blocks-the-task, too-big-now) do not apply here.
> This is the same standing rule long documented for investigation
> plans (findings live in the plan's catalog and get fixed, never
> double-filed), generalized to all stability work: fixing IS the
> work, so there is no "later" to file for.  Filing re-pays the
> scope/repro/mechanism derivation later and grows a backlog instead
> of shrinking the bug count; with diagnostics warm, the fix is the
> cheapest it will ever be.  An issue is acceptable only as the RECORD
> of a fix in flight (fixed-pending-merge), never as a deferral.

The ONE tracking view over the open stability work that is otherwise spread
across [STABILITY_SWEEP.md](STABILITY_SWEEP.md) (the pass-1 catalog),
[STABILITY_HOTSPOTS.md](STABILITY_HOTSPOTS.md) (the H register),
[STABILITY_PASS2.md](STABILITY_PASS2.md) (relocations),
[DEPS_INVENTORY.md](DEPS_INVENTORY.md) (H2), and `plans/`.  Detail stays in
those canonical homes — this file holds only the ORDER, the size, and the
live status.  When an item lands: flip its row to ✅ with the closing
commit, and update the canonical home as usual.  When new stability work
surfaces: insert a row at its priority, don't append.

Sizes as in STABILITY_HOTSPOTS § Reading the sizes (`S` under a day, `M`
days, `L` a plan with phases).  Every M+ design round runs
[DESIGN_PROTOCOL](DESIGN_PROTOCOL.md) (the `design-protocol` skill); every
fix runs matrix-first (CLAUDE.md § Debugging policy).

## The owner's directive (2026-09-23) — the horizon, and the meters that say "stable"

**Attention goes to the engine; loft's job is to stop needing it.** No hurry: the end of
the year is the horizon for a stable loft. What loft must do meanwhile is minimise owner
interrupts — so a question the written rules already answer is answered by the agent that
meets it (the standing ruling in [DEVELOPMENT.md § Where a rule's letter and the code
disagree](DEVELOPMENT.md#where-a-rules-letter-and-the-code-disagree)), and only three kinds
reach the owner: a rule that does not exist yet, a change to a shipped surface, or two rules
in conflict. The months, in order: **October** closes the language walk (`heap.md` is the
least-walked core chapter); **November** is the library pass the freeze doc already owes
([COMPATIBILITY.md § The road to contract 1](COMPATIBILITY.md)); **December** is the read.
The perf stream continues — optimisation is the usability measure game developers judge by —
pointed WIDE, not deep ([PERFORMANCE.md § Wide before deep](PERFORMANCE.md#wide-before-deep)).

**The meters, read weekly — together they are the definition of "stable" the owner uses.**
No figure is written here: each is a query, and a figure in prose is stale the day after.

| meter | stable when | read it with |
|---|---|---|
| `contract:strained` share of the week's judged fixes | **0 for two consecutive weeks** | `make bug-review` § 5 (weekly window) |
| owner rulings asked for | **0** — a ruling the rules already gave is a process defect | the week's `Contract: strained — letter applied` trailers, read in one sitting |
| the consumer gate | **green with no cure needed, two weeks running** | the `consumer-main-health` nightly |
| `silent-wrong` open **on main** | **0** — `fixed-pending-merge` does not count; the freeze binds what main ships | `gh issue list -R loft-lang/loft --state open --label silent-wrong` minus `fixed-pending-merge` |

**The raw issue count is not a meter.** It is effort-driven: a walk reaching a chapter nobody
had read files dozens in days, and a quiet month can mean nobody ran anything. The measured
case (2026-09-23): consumer-found issues fell from ~120 in August to 5 in September — because
the consumers made no commits after 2026-08-31, not because loft stabilised. Run against
main from scratch copies, their suites went red on three causes, two of them loft moving under
code nobody was running; the gate that sees that is the consumer nightly, not the tracker.

## The wide-release bar — what must be true before loft goes to many people

> This roadmap exists to clear **one** thing: the GOALS.md promise of **"a floor that
> does not betray you"** ([GOALS.md § The deeper aim](GOALS.md)). The H-register and the
> live store-lifetime stream below both serve it. This section is the explicit gate —
> the bar loft must clear before it is handed to a lot of people (and to AI writing
> loft). In priority order; **gate 1 is the definition of "stabilized," not one item
> among five.**

### The overriding gate — proven by working, not by a green checklist (soak target: early September)

**Clearing the list below is the beginning, not the end.** A stable declaration is not
"the gates are green." loft has to **work** — real, varied programs built and run in it,
and *staying* correct as it evolves — not be a well-specified language admired from afar. A
green board means "no *known* problem"; stable means "proven across the ways people actually
use it." Those are different claims, and only the second earns the name.

**Stable / contract 1 is a one-way door.** At contract 1, compatibility becomes *absolute*
([COMPATIBILITY.md](COMPATIBILITY.md)) — no functioning program ever breaks again, forever.
You walk through that door once. The only thing that earns the confidence to open it is
**breadth of real use** exercising the contract surface — because a contract that has only
been *tested*, not *lived in*, hides exactly the defects that surface the moment someone uses
it a way you didn't imagine.

**One consumer is not breadth.** **crawler** (a ~29k-line hex roguelike, a separate repo)
proved loft can build a real game — and in doing so surfaced defects nothing internal had
(a compiler `SIGSEGV` on a self-referential `??`, a silent interpreter struct-corruption),
precisely because it *used* loft differently. But games/graphics/procedural-world/3D/
store-lifetime is **one slice** of the contract. A different domain stresses a different
edge, and finds the class of defect only that edge produces.

**So the stable declaration waits on a diverse consumer soak.** Beyond crawler, **four more
projects — deliberately unlike crawler and unlike each other — are now exercising other
angles** of the language and stdlib:

| Consumer | Angle it stresses (that crawler does not) |
|---|---|
| **crawler** | games · graphics · procedural world-gen · 3D math · struct/vector store-lifetime |
| **routing** | graph / pathfinding data structures · large-collection throughput · algorithmic core |
| **zero-trust** | security · crypto · protocol / wire encoding · wasm bridges · exactness under adversarial input |
| *(two more)* | further distinct angles — each finds the contract stress its own domain uniquely produces |

Each of these will do for its domain what crawler did for games: surface the real defects
that turn "no known bugs" into "proven." **Target: hold the stable declaration until ~early
September**, giving the four additional consumers a genuine soak against the live toolchain.

This does **not** contradict the standing "work-limited, not time-limited" rule above: the
gates get cleared when the work is done, and stable *additionally* waits until diverse real
use has exercised the contract — which is why the useful marker is a **soak window
(~September), a floor on the declaration, not a ship date to rush toward**. If a consumer
surfaces a contract-level defect, the freeze waits until it is fixed and re-verified; if the
breadth stays clean through the soak, the door opens. Consumer coverage is tracked as
[GOALS.md Goal C](GOALS.md#goal-c--capability-via-dogfood)'s build matrix.

### Reading the soak — measure `wa:`, not the ticket rate

**The ticket rate does not measure stability.** It rises and falls with how much
dogfood is flowing: a new consumer coming online produces a burst whatever the code
is like, and a quiet week can just mean nobody was looking. Counting tickets measures
throughput.

**The `wa:` distribution does measure it, because it is normalised by usage.** It asks
how BAD each defect was for whoever hit it, not how many there were. As the language
settles, the remaining defects should be increasingly *routable* — a consumer meets one
and keeps working. So the share of `wa:clean` should RISE and `wa:partial` should FALL,
and `wa:none` — nothing works, the consumer is blocked — is the number that decides the
gate. `LABELS.md` already weights it that way: *"the most urgent triage axis, often
above `sev:`"*.

Two cuts matter more than the totals:

- **`wa:none` by AREA, as a rate** (`wa:none` ÷ that area's bugs), not a count. A count
  just tracks where the bugs are; the rate tracks where the *blocking* is.
- **Core vs fringe, over time.** A language-level bug usually has a workaround, because
  you can write the code differently. A packaging / CLI / install bug usually does not,
  because the program never starts — there is no "write it differently" when the file
  will not run. So blockers migrating OUT of codegen/parser/store-lifetime and INTO the
  toolchain surface is the shape of a language settling, even while the raw count holds.
- **WHO hit it, over time** (`hit-by:`) — the cut that made the August picture legible, and
  the sharpest demonstration of this section's own point. Measured over four weeks:

  | week of | hit by a consumer | hit by loft's own audits | consumer share |
  |---|---:|---:|---:|
  | 2026-08-03 | 35 | 4 | 89.7 % |
  | 2026-08-10 | 46 | 30 | 60.5 % |
  | 2026-08-17 | 22 | 69 | 24.2 % |
  | 2026-08-24 | 5 | 33 | 13.2 % |

  Consumers went from 35 defects a week to 5 while the TOTAL rose, because our own
  instruments took over the finding. Read as a ticket rate that month looks like a
  regression; read this way it is the `@F` / formal-tag rewrites working — the defects
  moved from being met by a consumer to being found by an audit before one arrived. **A
  discovery shift is invisible to every other cut**, which is why it belongs beside the
  two above: a rising count with a falling consumer share is a language settling, and the
  same rising count with a flat consumer share would be the opposite.

Reproduce all of it from the tracker (needs `gh` + `jq`):

```sh
# wa: share per month — the headline trend.  Exclude the un-labelled from the
# shares, or better hygiene reads as better stability.
gh issue list --state all --limit 500 --label bug --json createdAt,labels \
  --jq '.[] | (.createdAt[0:7]) as $m
        | ([.labels[].name | select(startswith("wa:"))]
           | if length==0 then "wa:MISSING" else .[0] end) as $w
        | "\($m) \($w)"' | sort | uniq -c

# blocking RATE per area — for each area, its bug count and its wa:none count.
for a in packages runtime store-lifetime codegen parser wasm closures stdlib native; do
  t=$(gh issue list --state all --limit 500 --label bug --label area:$a --json number --jq 'length')
  n=$(gh issue list --state all --limit 500 --label bug --label area:$a --label wa:none --json number --jq 'length')
  echo "area:$a bugs=$t wa:none=$n"
done

# every blocker with its date and area — the core-vs-fringe read is a JUDGEMENT
# over these titles, not a label query.  Read them; do not trust the area label
# alone (a Windows LSP hang carries `area:runtime` and is not the runtime).
gh issue list --state all --limit 500 --label bug --label wa:none \
  --json number,title,createdAt,labels \
  --jq '.[] | "\(.createdAt[0:10])  #\(.number)  [\([.labels[].name
        | select(startswith("area:"))] | join(","))]  \(.title[0:72])"' | sort
```

**Baseline — 2026-07-30** (107 bug issues; every one carries a `wa:`):

| | June (n=54) | July (n=53) |
|---|---|---|
| `wa:clean` | 43% | **62%** |
| `wa:partial` | 43% | **25%** |
| `wa:none` | 15% | **13%** |

Blocking rate by area, all-time — packages 6/18 and wasm 3/11 dominate, the core is far
lower, and three areas have never produced a blocker:

| area | bugs | `wa:none` | rate |
|---|---|---|---|
| packages | 18 | 6 | 33% |
| wasm | 11 | 3 | 27% |
| runtime | 16 | 3 | 19% |
| store-lifetime | 27 | 4 | 15% |
| parser | 13 | 1 | 8% |
| codegen | 20 | 1 | 5% |
| closures / stdlib / native | 7 / 3 / 12 | 0 | 0% |

**Label hygiene is not cosmetic — it changed this reading.** Nine bugs carried no `wa:`
when the metric was first run, and they skewed June-heavy. Excluding them made the
blocking tail look FLAT-to-worsening (10%→12%); labelling them showed it actually
SHRINKING (15%→13%), because three of the un-labelled June bugs were blockers
(`#407`, `#408`, `#457`). An incomplete denominator does not just add noise, it can
invert the sign of the trend. Keep `wa:MISSING` at zero.

**The core-vs-fringe read at this baseline.** Of June's 8 blockers, 5 were core
(nested-vector compound-assign `#246`, store-pressure `#306`, error-cascade `#376`,
corrupt enum discriminant `#406`, `vector<text>` arg corruption `#457`). Of July's 7,
ONE was core (`#497`, a `len`-on-freed-vector SIGSEGV); the other six were fringe — the
wasm bridge (`#623`), the registry cache (`#634`), Windows LSP transport (`#639`),
`--html` import validation (`#681`), the binary↔rlib install mismatch (`#693`), and the
issue tracker's own label guard (`#626`, not the language at all). Core blockers went
**5-of-8 → 1-of-7**: the unroutable tail did not merely shrink in share, it MOVED off
the core.

**What would falsify that read**, and is therefore the thing to watch: a NEW core
blocker — `area:codegen`/`parser`/`store-lifetime` carrying `wa:none` — filed by one of
the consumers still to come online. Store-lifetime is where to expect it if it comes: it
produced two of June's four core blockers and July's only one.

**Re-read — 2026-08-20** (345 bug issues; the same two queries, re-run). August is the
third data point the baseline said it needed, and it is the first one taken while a
consumer wave was flowing — 224 bugs filed in the month against July's 67:

| | June (n=54) | July (n=67) | August (n=219) |
|---|---|---|---|
| `wa:clean` | 43% | 63% | **67%** |
| `wa:partial` | 43% | 27% | **27%** |
| `wa:none` | 15% | 10% | **6%** |

The July column differs from the 2026-07-30 baseline above (53 → 67) because issues
created in July were still being filed and labelled after that snapshot; the shares moved
by a point, not in direction. **The trend the baseline predicted held through a 3× rise in
ticket rate**, which is the reading that matters: the rate rose because dogfooding rose,
while the share that BLOCKS its finder fell by more than half.

Blocking rate by area, all-time — the core stayed low while the count tripled, and the
fringe now carries the tail almost alone:

| area | bugs | `wa:none` | rate | vs 2026-07-30 |
|---|---|---|---|---|
| wasm | 16 | 7 | 43% | 27% → **43%** |
| packages | 46 | 8 | 17% | 33% → **17%** |
| runtime | 53 | 5 | 9% | 19% → **9%** |
| store-lifetime | 107 | 8 | 7% | 15% → **7%** |
| native | 51 | 4 | 7% | — |
| codegen | 60 | 1 | 1% | 5% → **1%** |
| parser | 103 | 2 | 1% | 8% → **1%** |
| closures / stdlib | 10 / 7 | 0 | 0% | unchanged |

**The core-vs-fringe read for August.** Thirteen blockers: two store-lifetime (`#727`
bound-store growth, `#833` the ASan gate), one parser+store-lifetime (`#906`), one
runtime (`#961`), and the remaining nine fringe — four wasm/`--html` (`#737`, `#950`,
`#964`, `#967`), three native/toolchain (`#742`, `#815`, `#834`), one macOS cache
invalidation (`#999`), one sanitizer gate (`#920`). Core blockers ran 5-of-8 (June) →
1-of-7 (July) → **4-of-13 (August)**: the share ticked up from July's low but stayed
under half, and two of the four are GATE failures rather than programs that will not run.
`wasm` is now the one area whose rate is rising, and it is the youngest surface.

**What this says about the last fixes.** None of #1035–#1040 were blockers — every one
carried `wa:clean` or `wa:partial`, and each had a written workaround that a consumer
could take. That is the shape the gate wants: the defects still being found in the core
are increasingly ones you can route around while they are fixed.

**Label hygiene has slipped and it matters here.** Five August bugs carry no `wa:`
(`#757`, `#784`, `#785`, `#787`, `#970`) — the first time the metric has had a
`wa:MISSING` population since the baseline said to keep it at zero. They are excluded from
the shares above, which is the same incomplete-denominator hazard that inverted the sign of
the trend once already. Label them at fix time.

**Three limits on the metric, so it is not over-read.** The `wa:` labels are applied by
whoever fixes the bug and the policy says *verified*, so there is an optimism bias no
query can audit. Two months is two data points; a third turns this from a reading into a
trend. And the nine back-filled labels above were judged from each report's own text —
a closed bug cannot be re-tested for its workaround, since the bug is gone — so they are
weaker evidence than a label applied while the bug was live. Label at fix time.

### The other half — how much STEERING the fixing took

`wa:` measures how bad the bugs were for whoever hit them. It says nothing about the
second question, which is at least as good a stability signal: **how hard did the owner
have to push to get them fixed, and fixed thoroughly?** A language settles when defects
get routable AND when fixing one stops needing supervision.

That effort is not in the repository at all — it happens in conversation. But it leaves
a timestamped trace in Claude Code's transcripts, and
**[`scripts/steering_rate.py`](../../scripts/steering_rate.py)** extracts it. The signal
is the owner interrupting a running turn; the discriminator is TIMING, because not every
interruption is steering:

- a **short** gap after the owner's own previous message means they never waited for the
  turn to process — their own flow, adding a fact they forgot;
- a **long** gap means they had been watching the work and stopped it.

A correction of the agent is uncorrelated with the owner's typing rhythm; an amendment to
themselves follows it within seconds. Reading the extremes confirms the split — under 20s
gives *"it is merged"*, over 5 minutes gives *"The fix is not committed"* and *"Are you
introducing runtime errors? Remove that immediately"*.

**Baseline — 2026-07-30** (`scripts/steering_rate.py`, de-duplicated, `>=60s` = steering):

| week | msgs | interrupts | int/100 | steering | steer/100 |
|---|---|---|---|---|---|
| W27 | 379 | 24 | 6.3 | 15 | 4.0 |
| W28 | 610 | 55 | 9.0 | 27 | 4.4 |
| W29 | 406 | 38 | 9.4 | 26 | 6.4 |
| W30 | 482 | 42 | 8.7 | 27 | 5.6 |
| W31 | 188 | 7 | 3.7 | 2 | 1.1 |

**The reading: FLAT at 4–6 per 100 across W27–W30, no step change.** The transcripts start
`2026-06-30`, and the owner reports the real improvement in steering came earlier than
that — so this instrument, like the tracker, was built after the transition it would most
want to show. It records a LEVEL to measure the next months against, not a turn.

**Four things that keep this honest.** *W31 is n=2* — the per-bug figure computes to 0.06
against W30's 1.69, which is arithmetic, not evidence; a partial week containing one long
productive session. *The threshold is arbitrary*: the gap distribution is unimodal with a
long tail (`--histogram`), so the populations overlap and the LEVEL moves with the cut —
the flatness does not (at 120s: 3.2, 2.5, 4.7, 4.4). *Per-bug only means anything in
bug-heavy weeks* — W27–W29 closed 2, 2 and 0 bugs, because they were plan and feature
work. *The transcripts are outside the repo* and nothing here preserves them; if they are
pruned this baseline cannot be recomputed.

**The precise complement is the [`steered`](../../.github/LABELS.md) label** (live since
2026-07-30), which the owner applies when they had to intervene on a specific issue. The
transcript rate is continuous and needs no behaviour change but cannot attribute to a bug;
the label attributes exactly but only where it is worth a click. Neither depends on the
agent noticing it needed help — which self-reporting would, and which is the one thing an
agent that needed steering is least likely to do. **Agents must never apply or remove it.**

Read it against the same denominator as `wa:` — as a SHARE of the bugs fixed in a window,
not a count, or it just tracks how many bugs there were:

```sh
# steered share of bugs closed in a window
gh issue list --state closed --limit 500 --label bug --json number,labels,closedAt \
  --jq '[.[] | select(.closedAt > "2026-08-01")] as $all
        | ($all | length) as $n
        | ($all | map(select([.labels[].name] | index("steered"))) | length) as $s
        | "steered \($s) of \($n) closed bugs"'
```

Two readings to keep apart once it has data. A FALLING steered share with a steady bug
count is the thing worth wanting: the fixes are landing right the first time. A falling
share with a falling bug count says nothing on its own — it can just mean less was
attempted. Pair it with the `wa:` table above, which is usage-normalised, before drawing
either conclusion.

1. **Seal the memory model — the non-negotiable gate.** The store-lifetime /
   return-bind-ownership class (loft's stated #1 weakness, REOPENED 2026-06-21) must be
   **closed, not merely quiet**. At one dogfooding agent a residual UAF/over-free
   surfaces occasionally; at many users + AI hitting every composition it surfaces
   constantly — and a substrate that *sometimes* corrupts invalidates the whole pitch
   (the language carries correctness so the maker never does — DESIGN_DECISIONS C79/C80).
   "Closed" = pin the ONE ownership invariant at the `deps` chokepoint (Cluster C / H10,
   not symptom-by-symptom), THEN prove the class gone *by construction* — graduate the
   boundary matrices into the fuzz/sanitizer corpora (@PLN53/@PLN54, queue step 9) so the
   silence is earned, not anecdotal. Live work:
   [§ Red-flag remediation](#red-flag-remediation--the-live-store-lifetime-stream-2026-06-21-)
   + @PLN85. **This gate's fuzz-proof IS the definition of stabilized.**
   **Instrument status (2026-07):** the fuzz-proof instrument is BUILT and live —
   the standing `tests/ownership_fuzz_gate.rs` job, the in-process libfuzzer target
   (caught + closed 2 real bugs in its first five minutes), `LOFT_POISON=1 cargo test`
   fully green (24 latent memory bugs fixed across the poison campaign), and the
   debug-assertions calibration run
   ([DEBUG.md](DEBUG.md#the-debug-assertions-calibration-run-target-da)).  The residue
   is enumerated, not anecdotal: the open DA cells + unfuzzed axes in
   [plans/85 fuzz-proof-gate.md](plans/85-store-lifetime-retirement/fuzz-proof-gate.md).
   **Build-order dependency — RESOLVED.** Gate 1 was blocked by gate 2: the ownership invariant
   could not be *defined* until the value/null model settled, because ownership flows through the
   `deps` facts and what a vector/value *is* (dense vs nullable, how it copies vs borrows) is
   exactly what @PLN25 decided. @PLN25 CLOSED 2026-07-02, so the foundation is in place.
   (This is why earlier @PLN85 attempts flailed — there wasn't enough of @PLN25 settled to know
   what to build.)
   **Gate 1 — the last item landed 2026-07-10.** Every *tracked* store-lifetime bug is closed; the
   **fuzz-proof half** is done (@PLN53 harness #542, @PLN54 sanitizer stack + S4 LSan, both CLOSED via
   #547, only S9's toolchain-blocked cdylib ASan spun out); and the **Cluster C / H10** `copy_claims`
   keystone fold — the last structural item — is **✅ done** (branch `tuxedo-cluster-c`, see the C row
   below), so the three divergent source re-encodings that produced the densest historical bug cluster
   are gone by construction. The standing verification is now WIDER, too: the nightly debug-assertions
   gate and the per-PR `stack_align_guard` sweep were both widened to the whole in-process interpreter
   corpus (2026-07-10, see the coverage-gaps section) — so the invariant is checked over `wrap`/`strings`/
   `frame_vars`/`expressions`/… not just `issues`. What remains before the gate is *sealed* is only that
   these standing corpora keep running over the folded code and that **`tuxedo-cluster-c` merges to
   `main` green**. The invariant is enforced and now broadly gated; this is "keep it enforced," not
   "define it" — gate 1 is **sealed pending that merge**.
   **⚠ RE-OPENED (2026-07-19): the widened debug-assertions gate did its job and surfaced a NEW
   store-lifetime UAF the older corpus never hit** — `tests/scripts/35m-mid-slice-repetition.loft`
   (`get_vector: use-after-free`) + `35c-rest-capture.loft` (POISON null), one root: **reading an
   element of a match-captured repetition group** (`(x)*`/`(x)+`) frees the group's backing store
   before the arm body dereferences it (the group is a `frame1(__vdb)` view whose dep-liveness
   scopes.rs doesn't extend past the collection). Benign on release, caught only under
   debug-assertions/POISON. **CLOSED 2026-07-19 — both scripts, both backends** (35m was
   `vector_match`'s `__acc` materialisation, 35c the skip-trailing-frees in
   `collect_return_sources`), plus a static overlay gate so the pair cannot regress silently →
   [plans/captured-group-elem-uaf.md](plans/captured-group-elem-uaf.md). Gate 1's "every *tracked*
   store-lifetime bug is closed" holds again.

2. **One coherent null model — and the substrate gate 1 is built on. MODEL LANDED 2026-07-02 (#480);
   the gate is NOT yet cleared.** @PLN25 (nullable sequences / dense-default) is closed as a *plan*:
   vectors, scalars and DN1–DN6 all landed default-on across both backends, `formal/types.md` is at
   0 open deviations, and `not null` is now a **deprecated no-op** — it still parses, with a warning,
   so not-yet-republished libraries keep loading, and the hard "retired" error stays blocked on the
   registry republish (#546). **Load-bearing half realised:** the value shapes and `deps` ownership
   facts the memory-model fix reads are settled, which is what makes **gate 1's invariant
   *knowable*** — so gate 1 is unblocked, and that was gate 2's job for it.
   **What keeps the gate open** is now only close-out — **all three null-model SOUNDNESS edges are
   CLOSED (2026-07-16)**: the **`?? null` unsoundness** (`??` keeps the result `τ?` when the fallback
   can be null — `qq_null_typing_enabled`, `tests/qq_null_typing.rs`); the **`u8?`-return native
   codegen failure** (no longer reproduces, re-probed); and the **call-arg N-Store hole** (the
   `n_store_violation` check now runs at the param-binding chokepoint — `callarg_nstore_enabled`,
   `tests/callarg_nstore.rs`). No known "null reaches a non-null slot" path remains. What is LEFT for
   gate 2 is **not a soundness edge**: the registry-gated `not null` hard-reject (blocked on the
   library republish) and the F6 bookkeeping close-out (the `Closes @PLN25` PR + CHANGELOG +
   deviation-register confirmation). So the gate is now a **close-out**, not an open soundness hole.
   *Provenance:* re-probed on both backends 2026-07-10 by the @PLN25 stream — the authoritative list
   is [RESUME.md § VERIFIED-OPEN RESIDUALS](plans/25-nullable-sequences/RESUME.md#verified-open-residuals-re-probed-both-backends-2026-07-10).
   These are **not** independently re-verified here.

3. **First-contact developer experience. ✅ CLEARED 2026-07-07.** GOALS' acceptance test is
   *"done = picking it up is fun,"* and first contact is dominated by what happens when the user
   is **wrong**. Error messages (@PLN28) and developer experience (@PLN36) are **both CLOSED**
   (`status:finished`): `file:line:col` + caret across parser/type/runtime, did-you-mean
   suggestions, concrete type-mismatch + match-pattern checks. Residual is two non-blocking
   polish slices (finer format-null tokens, the `= note:` renderer) — not a gate.

4. **Durability.** The "trust and forget your data" half — opt-in mmap so a crash or edit
   never loses the store (@PLN43). Skippable for throwaway prototypes, load-bearing for
   real projects.

5. **A stability contract for scale. ▶ TRIGGER FIRED 2026-07-10 — plan OPENED as
   [@PLN102](https://github.com/loft-lang/plans/issues/102)** (`status:next`,
   [plans/102-stability-contract/](plans/102-stability-contract/README.md)). A stated
   semver / compatibility promise, a **public** bug-intake path (the fix-not-file discipline is
   internal-only and doesn't reach strangers), and a 1.0 line — what is frozen vs still moving.
   The opening condition was "open one when gate 1 is in sight"; gate 1 is now **sealed pending merge**,
   so gate 5 is the active next gate. **Design refined 2026-07-10** (plan README § Phase ordering): arc
   B splits into a *mechanical* half — a real constraint parser that binds upper bounds/ranges/pins and
   loudly rejects the unparseable, spec-ready and independent of policy — and a *semantic* half gated on
   the **language-versioning decision** (the pivot: a bound only means "compatibility" once the language
   has a version axis that increments on breaking changes, which calver does not).
   **The mechanical half now ships** (verified 2026-08-09): `manifest::check_version` parses
   comma-separated predicates AND-ed into a range, `parse_predicate` binds `>=`, `<=`, `>`, `<`,
   `=` and a bare version (lower bound, for backward compatibility), and anything else returns
   `VersionCheck::Malformed` with a message naming the supported forms. The loud-rejection cases
   are pinned by `tests/compat_floor.rs` (`"garbage"`, `""`, `"1.2.3.4"`, `"v1.0"`, `"latest"`,
   `"0.x"`) and `tests/package_layout.rs`. What remains is the *semantic* half: under calendar
   versioning the `>=0.8` that published libraries carry is still vacuous, so a library can now
   *express* incompatibility but the version axis does not yet make a bound mean it.
   **The failure mode it prevents was live.** `hex_terrain 0.1.0` failed its own registry
   test with `0 land cells`: it used the plain-bind write-through idiom (`th = t.tr_h; th[i] = v`),
   and loft now **copies on plain bind** (C86 H-Copy), so the heights landed in throwaway copies.
   `graphics` hit the identical class and was migrated to `&self.data`; `hex_terrain` has since
   been fixed too and the package validates green. Both pinned `loft = ">=0.8"`, so nothing
   guarded them, and the library did not crash — it computed a plausible-looking wrong answer.
   That is precisely what
   [GOALS.md](GOALS.md) forbids of the platform: *"the platform never broke its users; the cost of
   change was paid by the maker, not the customer."* A compat promise with a deprecation channel
   is the mechanism that would have caught it.

**Sequence — gate 3 is CLEARED; gate 2 delivered what gate 1 needed but is not itself closed; gate 1
is the live one.** Gate 2 (@PLN25) settled the value model and the `deps` ownership facts that gate 1's
invariant is defined against, exactly as the build order required — so gate 1 is **unblocked** even
though gate 2 still carries verified-open soundness edges of its own (see gate 2 above; they do not
block gate 1, because the *model* is what gate 1 reads). Gate 3 (@PLN28 + @PLN36) is closed. So the
order it ran in was: finish gate 1, drain gate 2's edge cases, then gate 5 — all now done or at
close-out (see § Readiness today). Gate 4 (@PLN43) is still parked. Performance (the copy-vs-borrow
elision, an @PLN25 sub-thread) is "good enough for prototyping" — fold in opportunistically, not a
blocker.

**Readiness today (2026-07-28) — the gate list is nearly exhausted; the soak is what is left.**
Every numbered gate below is now closed or down to close-out, so the binding constraint has moved to
the overriding gate above: breadth of real use, not a green board.

| gate | state |
|---|---|
| 1 — memory model | **sealed.** Tracked store-lifetime bugs closed; the 2026-07-19 re-opened captured-group UAF is fixed both backends; the fuzz/sanitizer corpora (@PLN53/@PLN54) stand and the DA + `stack_align_guard` gates span the in-process corpus. |
| 2 — null model | **close-out only.** @PLN25 CLOSED; all three soundness edges closed 2026-07-16. What remains is not a soundness hole: the `not null` hard-reject is still a deprecation warning, blocked on the library republish, plus F6 bookkeeping. |
| 3 — first contact | **cleared** (@PLN28 + @PLN36 closed). |
| 4 — durability | **parked** (@PLN43, `status:parked`). Needs an explicit in-or-out-of-contract-1 decision rather than continued drift. |
| 5 — stability contract | **CLOSED.** @PLN102 is `status:finished`. The `CONTRACT_VERSION` 0→1 flip remains, but that is the freeze act itself, not plan work. |

`registry-validation` is **green** again (since 2026-07-26, after four red days) — `hex_terrain` was
its last red and is no longer failing.

With the language surface effectively frozen in practice (no breaking changes planned), "time since
the last defect" stops being evidence: a static surface produces no new reports whether it is sound or
merely unchanging. Only a composition nobody has tried can still surface a contract-level defect,
which is why the diverse-consumer soak — not this list — is the remaining gate.

**Why the tracker is empty — and what to read instead.** This stream's standing rule at the top of
this file is *fix, don't file*, and the cycle runs under a warm feature freeze
([ROADMAP § Feature freeze](ROADMAP.md)): **a known defect cannot be parked** — it is fixed in the
session that surfaces it, with a regression test, and new feature work stops until what we can see
works. So "zero open bug issues" is not bookkeeping; it is the *consequence* of refusing to tolerate a
defect, and it is why nothing accumulates. What the number is **not** is the ledger. The known
remainder is **recorded, scoped and owned** in each open plan's residual list — plus this queue — and
it is not all comfortable: gate 2's `?? null` unsoundness above is a real soundness edge, named rather
than parked. **Read those lists, not the issue count.**

The discipline earns its keep because *the person who finds a bug is the person who fixes it*: repro
warm, paths loaded, no scope/mechanism re-derivation to re-pay later. It does not survive contact with
anyone who **cannot** fix — filing is a stranger's only available move — which is why a public intake
path is its own arc of gate 5 ([@PLN102](https://github.com/loft-lang/plans/issues/102)) rather than an
afterthought. The policy is right; its boundary is scale, not size.

Two standing gates were RED; **both are now resolved except one external library**:
- **`main` on the differential oracle — ✅ GREEN (2026-07-10).**
  `tests/oracle/27-native-tailcall-return-heap.loft` was the `a7_match_arm_tail` divergence
  (a `-> text` fn whose tail `match` arm calls a caller-buffer callee → rustc E0599); it was
  fixed in `b1426f9e` (#548) on `main` (`if_tail_yields_text` now sees through the `scalar_match`
  block) and pinned by `tests/scripts/536-text-match-tail-buffer-callee.loft`.
  `oracle_corpus_agrees_across_backends` passes both backends. The corpus cell added by @PLN97 did
  its job.
- **`registry-validation` — graphics leg FIXED (2026-07-10); one library still red.** `graphics`
  failed at native-crate build (`alsa-sys` needs `libasound2-dev`; the workflow installed only
  `mold`) — a provisioning gap, now closed by mirroring the main CI Test job's Linux install
  (`libasound2-dev xvfb libgl1-mesa-dri`) into `registry-validation.yml`. The remaining red leg is
  **`hex_terrain 0.1.0`**, a real published-library bug (the C86 plain-bind write-through idiom
  lands its heights in throwaway copies — see gate 5): it needs a **library republish in
  loft-libs-game**, out of this repo's scope, and is the motivating case for the @PLN102 compat
  promise. Not a network flake — the other ~20 pass.

**Coverage gaps against the GOALS.md Checks — both CLOSED 2026-07-10** (the Checks are the bar and
stay as written; these were *results*):
- Goal A (`stack_align_guard` fires zero across every test binary): **✅ widened.** The guard fires
  only IN-PROCESS — a `cross_mode!` matrix cell / mixed-boundary suite shells out to a spawned
  `--native`/`--wasm` binary the sweep can't observe (`tests/n3_parity.rs` states this), so the
  reachable corpus IS the in-process interpreter binaries. The `guard` sweep now runs all of them:
  `issues/wrap/strings/frame_vars` **plus** `expressions`, `expressions_auto_convert`, `slots`,
  `slot_v2_baseline`, `value_struct_alloc`, `dispatch_reentry`, `format` (each verified zero-fires
  under the feature). `library_suite` stays excluded (native cdylibs + GL/ALSA the lean job omits;
  guard-blind anyway).
- Goal E (`LOFT_STORE_GUARD=1` silent across the corpus, promoted to a `cfg(debug_assertions)`
  assertion): **✅ wired + widened.** The enforced twin — the `reclaim_guard`
  `reclaim_unfreed_eligible == 0` `assert_eq!` — now hard-gates across the interpreter corpus
  because the **nightly debug-assertions gate was widened** from `--lib --test issues` to
  `--test wrap --test strings --test frame_vars` (`library_suite` excluded), the plan-85
  DA-inventory chain having been cleared (below). `LOFT_STORE_GUARD=1` is now set on that gate too
  (closing "set in no workflow"), additionally running the block-confinement `store_lifetime_guard`
  detector; both verified silent corpus-wide, positive-controlled by
  `watermark.rs::phase4_goal_e_guard_is_falsifiable`.

### The `wrap` loft_suite DA-gate residuals — the widen-the-gate worklist (✅ CLEARED 2026-07-10)

**DONE — the nightly DA gate now spans `--lib --test issues --test wrap --test strings
--test frame_vars` (`library_suite` excluded), matching the per-PR `stack_align_guard`
sweep scope.** Widening was blocked by a **chain of debug-assert tripwires**: under
`RUSTFLAGS='-C debug-assertions=on' … cargo test --release --test wrap`, the
`loft_suite` test (one test that runs every `tests/scripts/*.loft`) aborts at the
FIRST script that trips an assert — so each fix unmasked the next.  Cleared on
`tuxedo-cluster-c` (UNMERGED).  **Most were FALSE ALARMS** — over-eager
sentinels firing on correct-but-flagged cases (the H2 sentinel's OWN advice, "re-add
the read", would have *leaked*; the relocate one tempted a wide-blast-radius "complete
the traversal" that wasn't needed); the read-surface one (86) was the same shape.  Two
were real latent bugs.  Lesson: before obeying a sentinel's "this shouldn't happen"
premise, verify the flagged behaviour (value + leak + `LOFT_POISON` + the DA store-free
asserts, BOTH backends) — a debug tripwire is a hypothesis, not a verdict.

Cleared (each verified: the fixed case correct on both backends; a non-vacuous
`tests/issues.rs` guard):

| Script(s) | Assert | Commit | Was it real? |
|---|---|---|---|
| `156-plan52-chained-coalesce` | `text.rs:334` double free | `afacd148` | **REAL** — a chained `??` double-freed an owned `__ncc` coalesce temp (interp; `collect_consumed_ncc_text` double-collected a nested-`??` temp). |
| `387-text-fn-ref`, `85-ncc-container-text-return`, `85-poison-return-tail-uaf` | `parser/mod.rs:1195` (H5 two-pass contract) | `cd9c1f94` | **REAL, latent** — a pass-2-only `__tret` hidden `&text` signature buffer → forward-ref caller "Too few parameters" crash (BOTH backends, release, not just DA). Gate pass-2 tret promotion on pass-1. |
| `450-struct-field-vector-return`, `508-empty-arm-real-empty-vector`, `repro_p365`, 4× `85-store-lifetime-*` | `scopes.rs` (H2 step-5 `tp_alone` sentinel) | `e1d594cb` | **FALSE ALARM** — retired positional block-result read; a field/enum-arm vector return copies its source into the retbuf, so freeing the local source is correct (re-adding the read would leak). Sentinel removed. |
| `501-map-filter-literal-receiver`, `85-short-lambda-capture` | `scopes.rs` (`relocate_null_init`) | `097879bb` | **FALSE ALARM** — the best-effort Plan-57 null-init relocation can't reach a confined block off the control-flow spine (a `map`/`filter`/lambda body); the body-0 fallback is correct. Assert softened to fire only on genuine scope-absence. |
| `86-writeread-struct` | `scopes.rs` (`check_ref_leaks`) | `d5b6212a` | **FALSE ALARM** — `_read_1`, the `#reading file` surface temp behind `q = f#read as S`, is a MOVE source: the block allocs one record and PutRef-adopts it into `q`, which IS freed. `get_free_vars` already elides the temp's free; `check_ref_leaks` didn't model the adoption. Both backends clean (values + empty-allowlist leak gate + POISON). Fixed by crediting the adopted block-tail temp (`collect_adopted_block_results`) — narrow: a plain-bind COPY has a bare-`Var` RHS and its source is freed separately, and the credit requires `lhs ∈ freed`, so it can't mask a real leak. |

Remaining — **NONE for the interpreter gate.** The two that were listed are outside the
gate's scope by construction (the gate covers the in-process interpreter corpus,
`library_suite`/native excluded — exactly like the alignment sweep):

- **`75-native-stub`** — INFRA (needs a rebuilt native cdylib); native-only, so it is
  never hit by the interpreter DA gate (which runs `--interpret`).  `find_problems.sh`
  (runs `make rebuild-native-cdylibs`) passes it.  Only a hypothetical *native* DA sweep
  would need this; not built.
- **`audience_crystal/03-crystal-incr`** — `mod.rs:4014` op-count watchdog.  Lives in
  `library_suite` (excluded).  NOT a runaway: under normal CI it runs NATIVE (cdylib
  built) with no op watchdog; a DA `library_suite` attempt forces interpreter fallback
  (the cdylib isn't rebuilt against the DA `libloft`), and the compute-heavy crystal-incr
  legitimately exceeds the *interpreter's* op limit — an artifact of interpreter fallback,
  not a bug.  Out of scope unless `library_suite` is ever run under DA.

Order of the chain (each masked the next): `156 → H5 → 3787(H2) → 280(relocate) → 86`.
All cleared — the `wrap` (+ `strings`/`frame_vars`) suites joined the nightly DA gate in
`d5b6212a`. `75` (native infra) and `audience_crystal` (library_suite op-limit) sit in the
excluded native/library domain, out of the interpreter gate by construction (above).

## The queue

The bug-level stability work — the F-family sweep, the armed-corpus residuals,
the store-lifetime UAFs *as of that pass*, and the **Pass-2 arity-growth cascade**
(Reference + Vector/struct-Enum, #355/#356) — is **complete** (see Done below). The
store-lifetime class REOPENED 2026-06-21, but as of 2026-07-09 **all its tracked bugs are CLOSED**
(see [§ Red-flag remediation](#red-flag-remediation--the-live-store-lifetime-stream-2026-06-21-)
below); the one remaining item there is the **Cluster C** `copy_claims` keystone fold — forward-risk
hardening, not an active failure. Nothing in the
H-register queue below is an active failure either. What remains *there* is **forward-risk hardening** (the
H-register): asserts and tripwires that lock in finished work, then structural
refactors that retire whole future-bug *classes*. In finishing order:

> **▶ QUEUE CLEARED 2026-06-17.** Every H-register item is fixed or resolved:
> step 1 (H5), 3 (H6 — real bug), 4 (H7-short), 5 (**H7-long** — silent-gap risk
> CLOSED: exhaustive both-codec round-trip guards over all 34 Value variants + the
> `len()==34` count guard; the build-time schema macro is now an optional
> stronger-enforcement upgrade, not a gap), 6 (H3 — resolved by design-protocol:
> premise over-stated, facts already carried), 7 (**H8** — the load-bearing
> worker-slot swap-back encapsulated into one home; full field-privacy is
> design-protocol over-reach for the benign reads, optional), 8 (H4-medium GET↔SET),
> and 10 (de-dup tail — triaged clean) are ✅. Remaining are NOT bugs/hardening-gaps:
> **step 9** is the fuzzing/sanitizer instrument pair (@PLN53/@PLN54) — **✅ BOTH CLOSED 2026-07-10**
> (#547); it was gate 1's fuzz-proof and it now stands (only S9's toolchain-blocked cdylib-boundary
> ASan spun out); **step 2** is parked WIP on a separate branch.  The live open queue is
> **step 11** (CI docs-only matrix-skip — risky CI surgery, validate via a docs-only PR).
> **Step 8** (H4-medium) is now ✅ (design-protocol — free-op already single-homed, null-init
> are different facts; premise over-stated like H3) and **step 12** (i32 reclaim) is deferred
> (matrix-revised to M + low-value). The optional residuals
> (the F9 build-time macro, the full `allocations` field-privacy conversion) are
> stronger-enforcement / mechanical cleanups whose RISK is already covered — each
> opens with the design-protocol if/when its trigger fires.

| # | Item | Size | Status | Detail + entry point |
|---|---|---|---|---|
| 1 | **H5 leftover asserts — COMPLETE.** The two-pass contract asserted where it's cheap. **Attr-COUNT-per-def-equal-across-passes LANDED** (961e6c27, `assert_pass2_def_attr_stable`; post-arity-cascade an invariant, enforced end-to-end not just at the `ref_return` growth site; silent across the 270-script debug corpus). The **work-ref-counter half was resolved by re-evaluation, NOT a second assert** (the spec's item-3 "re-evaluate after H1" call): post-H1 `work_refs()` fires 0× in the corpus, a stored-table work-ref assert is permanently vacuous (`append` resets it to 0 at store time), and its only load-bearing failure (a cross-pass `__ref_N` shift → spurious `ref_return` attr) is already a count divergence the attr-count assert catches. Lambda naming is the sole remaining live name-stability consumer. | S | ✅ done (attr-count assert + work-ref re-eval) | [STABILITY_HOTSPOTS § H5](STABILITY_HOTSPOTS.md) |
| 2 | **Plan-53 cluster 2, S4 half (eval-TOS / frame-base alignment)** — parked WIP on branch `plan-53-sanitizer-ci-lever` (HEAD 8abfb8e1): cluster 1 fixed, aligned-V2 allocator half done and validating clean; the eval-TOS rounding half remains. Finishing it makes the sanitizer CI lever fully green = the second standing instrument beside the armed channel. | M | ⬜ parked WIP | plan closed by the @PLAN53 wrap-up; the session handoff is preserved at [plans/finished/53-sanitizer-ci-lever/cluster-2-fix-design.md § SESSION HANDOFF](plans/finished/53-sanitizer-ci-lever/cluster-2-fix-design.md) + `cluster-2-S4-progress.md` |
| 3 | **H6 — null-sentinel width-fact — LOAD-BEARING PART DONE (2026-06-17).** The matrix-first settle (the gate) overturned the design note: the `get_byte`/`set_byte` "asymmetry" was a misread — the nullable consumer pairs round-trip null symmetrically for every `min`, both backends. The REAL latent bug was on the **range-fullness** axis: a nullable FULL-range narrow field (`max-min == 255`/`65535`) under-allocated to 1 byte and read its null back as `max-1`, because the storage/WRITE width (`Type::size`) disagreed with the READ width (`byte_width`). Fixed at the chokepoint — one `IntegerSpec::range_to_width` home both derive from. Regression `tests/scripts/389-h6-nullable-full-range-narrow.loft`; full suite green. The `NullEnc` encode/decode TABLE is downgraded to OPTIONAL lower-risk cleanup (the per-width pairs already agree) → folds into step 10 Pass-3 de-dup, not a load-bearing fix. **NEW design follow-up (2026-06-17): the ALIAS path** — `u8`/`i8`/`u16`/`i16` are MEMORY-allocation types (fixed byte width is the invariant; nullability reserves a sentinel by SHRINKING the range, never widening). Current `IntegerSpec::u8()`=`0..=255` / `i8()`=`-128..=127` don't make their usable bounds `not_null`-aware, so a nullable `u8`'s `255` collides with the `ByteNullable` null sentinel. **ALIAS path DONE 2026-06-17 (`4a632251`), full suite green both backends.** `IntegerSpec::usable_min`/`usable_max` (one home) wired at the read op, write op, and `int_value_fits` (gained a `narrow_field` flag — a field STORE reserves the sentinel; a param/cast is full-width, so `f(65535)` to a `u16` param stays legal). Nullable is SYMMETRIC for signed (`-127..=127`, `-32767..=32767`), top-trimmed for unsigned (`0..=254`, `0..=65534`); the all-ones byte is the uniform sentinel, only `min` shifts. The SEPARATE 2-byte not-null-max bug was fixed in the same pass via `NarrowIntKind::ShortFull` / `OpGetShortFull` (direct read+min, no sentinel — the 2-byte twin of `Byte`). Field nullability now stamped onto the stored `IntegerSpec.not_null`; `lib/code.loft` `cur_arg` → `u8 not null`; `fill.rs` regenerated. Regression `tests/scripts/389-narrow-alias-ranges.loft`. Open separate/pre-existing follow-up: inline `Struct{..}.x` byte read fails on native (pre-eval gap, breaks plain `OpGetByte` too). | M | ✅ heuristic-path + alias path + 2-byte not-null all done | [STABILITY_HOTSPOTS § H6](STABILITY_HOTSPOTS.md); F3 row in [STABILITY_SWEEP](STABILITY_SWEEP.md) |
| 4 | **H7 short half — IR codec round-trip property test — LANDED (7187d5c6)**. `tests_scripts_round_trip` round-trips every `tests/scripts/` def's Type/Value/Attribute through the IR JSON codec (270 scripts seeded on the cached stdlib). It **earned its keep on day one**: caught `Value::Long(2^53+1)` decoding as `2^53` — the codec wrote i64 as a JSON number, which the parser stores as f64, silently truncating beyond 2^53. Fixed (i64 → quoted string; `as_i64` accepts both forms, legacy snapshots still decode). Now a standing tripwire: the next `Value`/`Type` variant with a silent codec gap, or any i64-precision regression, fails here loudly. | S | ✅ landed (found+fixed a Long codec bug) | [STABILITY_HOTSPOTS § H7](STABILITY_HOTSPOTS.md) |
| 5 | **H7 long half — derive the IR codecs from one schema declaration (F9)** — encoder, decoder, and the exhaustive walker all derive from one macro/table; a new variant then breaks the build until all three know it. Own design slot (codecs encode FIELDS, `for_each_child` can't drive them). **Runtime coverage of the STORE codec LANDED 2026-06-17** (`7b01e2a9`): the `corpus_store_codec_round_trips` guard widens the store-codec round-trip from one hand-written program to the whole corpus, and it caught a real reproducibility bug — `snapshot_names` iterated a `HashMap`, so the cached `Data` (variable-name list) was non-reproducible; fixed by a `(var_nr, name)` sort at that one chokepoint. **The silent-gap RISK is now CLOSED 2026-06-17** (`3e45d465`): a new `Value` variant breaks `write_into`'s exhaustive match (build error) AND fails the new `materialize_all_variants_round_trip` guard — all 34 variants through `materialize_node`→`read_value`, with a `len()==34` count guard forcing inclusion — so a variant or dropped field can no longer reach the cache silently. Both codecs now have all-34 exhaustive coverage (JSON `type_/value_*_round_trip`, STORE `materialize_all_variants` + the corpus guard). The build-time **schema MACRO** (deriving the arms from `ir.loft`, which already drives `ir_schema_gen`) is now a stronger-enforcement UPGRADE (build-time vs test-time), not an open risk. | M | ✅ silent-gap risk closed (exhaustive both-codec guards); F9 macro = optional upgrade | [STABILITY_HOTSPOTS § H7](STABILITY_HOTSPOTS.md); deferred row in [STABILITY_PASS2 § Work list](STABILITY_PASS2.md) |
| 6 | **H3 — ownership as carried data** — UNLOCKED (H1 + H2 both shipped). **Design-protocol (2026-06-17, `97b5d6f0` + pass 2) reframed this: NOT an open L carry-conversion.** The per-var ownership facts (`captured`, `caller_hidden_buf`, owned/borrowed via `Type::is_heap_owned` + `Deps`) are ALREADY carried, and the two core free-placement analyses (`reclaim_safe`, `store_confinement`) READ them — neither re-derives ownership inline. The "re-asserts what construction knew" premise is over-stated; what remains is the INHERENT shape-locality of free-placement (escape/retention/confinement-span), managed by the cross-check corpus, not removable by carrying. **Verification done: 4 analyses confirmed read-only** (`reclaim_safe`, `store_confinement`, `get_free_vars`, `store_lifetime_guard` — all READ `tp`/`Deps`/`is_captured`/`is_skip_free`/`is_argument`, none re-derive ownership inline). H3's core worry (analyses re-asserting carried state) is therefore **already addressed**. Residual is small + separate: the INHERENT shape-locality of free-placement (not removable) + possibly-scattered flag SETTERS (the #316/#323 "five homes" — a de-dup, not a carry-conversion). The sweep pass-2 notes (`value_reads_var`/`base_var_of`) fold into the de-dup tail. | L→S | ✅ resolved by design-protocol (premise over-stated; facts carried + read) | [STABILITY_HOTSPOTS § H3](STABILITY_HOTSPOTS.md) |
| 7 | **H8 — the `Stores.allocations` privacy pass** — the load-bearing target (per design-protocol) was the **worker-slot swap-back** — the par store-isolation "swap dance" (memory-safety), inline in `parallel.rs` with its no-cross-thread-aliasing invariant living only in a comment. **DONE 2026-06-17** (`69b6eb15`): moved to `Stores::grow_allocations_to` + `Stores::swap_in_worker_slots` — ONE named, documented home for the invariant (threading 47/47 green, behavior-preserving). The remaining ~498 raw `allocations[nr]` touches are **benign bounded reads carrying no invariant beyond bounds**; rewriting them all to make the field `private` would be over-broad (blast radius ≫ defect — design-protocol over-reach) and adds no invariant enforcement — so full field-privacy stays an OPTIONAL mechanical cleanup, gated on a future `par` API that genuinely needs the whole accessor surface. The STABILITY_PASS2 accessor rows (`types[].parts`, Definition reads) fold into that optional pass. | M–L | ✅ load-bearing swap encapsulated; full field-privacy = optional/deferred | [STABILITY_HOTSPOTS § H8](STABILITY_HOTSPOTS.md); deferred rows in [STABILITY_PASS2 § Work list](STABILITY_PASS2.md) |
| 8 | **H4 medium half — extend the `#rust`-template idea upward** — the free-op family, null-init emission, and the GET→SET table become one declaration each that both backends derive, the way fill.rs derives ops. Includes the F4 op-coverage sentinel (enumerate ops lacking `cross_mode` cells) as its completeness check. The L half (one shared lowering IR) stays 1.1+ and explicitly NOT before step 6 (H3). **GET↔SET table DONE 2026-06-17** (`NarrowIntKind::of` — one width→op home for `get_val`/`set_field_check`, `9153e132`); the other two halves RESOLVED by design-protocol (2026-06-17, "start Row 8"): the **free-op family** is already single-homed (`scopes.rs` selection + de-duped `pre_eval::free_op_var` recognizer — no per-backend table); the **null-init** pair are DIFFERENT facts (`emit_typed_null` live NULL sentinel vs `default_native_value` default-INIT placeholder — probed identical live-null round-trip on both backends; `floatvar=null` type-rejected; merging would be the H6-`NullEnc`-phantom), with the lone residual (`default_native_value`'s conflated contract) fixed by a clarifying doc comment. Premise over-stated, like H3. | M | ✅ done — GET↔SET shipped; free-op + null-init resolved by design-protocol | [STABILITY_HOTSPOTS § H4](STABILITY_HOTSPOTS.md); F4 row in [STABILITY_SWEEP](STABILITY_SWEEP.md) |
| 9 | **The instrument plans — fuzzing + sanitizer expansion** — **✅ BOTH CLOSED 2026-07-10.** @PLN53 program-level fuzzing (harness shipped #542; continuous-run/OSS-Fuzz decision-deferred) + @PLN54 sanitizer coverage expansion (S1/S2/S3/S5/S6/S7 green; S4 LSan `detect_leaks=1` unblocked by @PLN85 + green; S8 MSan deferred; only S9 mixed-boundary cdylib ASan spun out, toolchain-blocked on curve25519 `E0463`). The sweep's "store-level fuzz harness" instrument + the remaining pass-1 DEFERRED cells (F1 diagnostics-altering-flow, F5 odd-size adjacency, F6 P191 late-mutation, F8 crafted attr/var collision, F9 lib-path axes, F10 par text buffers, match-unification, dispenser stress) fold into the shipped fuzz corpora (or re-open with S9) rather than being probed by hand. | L | ✅ both plans closed | [plans/53](plans/53-program-level-fuzzing/README.md), [plans/54](plans/54-sanitizer-coverage-expansion/README.md); DEFERRED markers throughout [STABILITY_SWEEP](STABILITY_SWEEP.md) |
| 10 | **Pass-3 de-dup tail** — **TRIAGED CLEAN 2026-06-17.** Every named candidate is already resolved: `generation/ops` post-plan-57 rc remnants — **gone** (no rc code left); `value_reads_var` — already centralised (`data.rs::reads_var`, replaced `scopes::value_reads_var` + two more); `base_var_of` — already unified (`data.rs::base_var`); the variables size-table — DECIDED a non-dup vs `byte_width` (PASS2 wave 5, different facts); `towards_set` dual discriminators + the codegen_runtime mirrors — INTENTIONAL (interp-vs-native, #328). The H3 flag-setter "five homes" (#316/#323) is the one residual de-dup, opportunistic. Nothing actionable stands open. | S each | ✅ triaged clean | module rows in [STABILITY_SWEEP § Module work list](STABILITY_SWEEP.md) + [STABILITY_PASS2 § Work list](STABILITY_PASS2.md) |
| 11 | **CI — docs-only PRs block on the skipped Test matrix** (surfaced 2026-06-17, PR #400). A pure docs-only diff skips the Test matrix at the JOB level, so the required `Test (<os>)` contexts never appeared → branch protection stayed `BLOCKED`. **✅ DONE** — the companion-job fix (a `test-skip` matrix job, `if: needs.changes.outputs.code == 'false'`, that posts `Test (ubuntu-latest)`/`Test (macos-latest)` green on a docs-only diff) was landed in `b79c3798` (PR #400 itself) and is on `main`; the row was just never flipped. Verified present on `origin/main`. | S | ✅ done (`b79c3798`) | `.github/workflows/ci.yml` (`test-skip` job) |
| 12 | **H6 follow-up — 4-byte `i32` reclaims `i32::MIN` (not-null full range).** A not-null `i32` cannot hold `i32::MIN`: `OpGetInt4` decodes a stored `i32::MIN → i64::MIN` (null) — the sentinel-decode the 1-byte `Byte` and 2-byte `ShortFull` reads avoid. Fix = the 4-byte twin: a no-sentinel read (`get_i32_raw` direct → `i64::from`), a `NarrowIntKind` arm splitting not-null vs nullable at width 4, `reserves_narrow_sentinel` + `usable_min/max` coverage for `Some(4)`; the compile + runtime sentinel communication then falls out of the shared `usable_min/max` home. LOW value — reclaims one extreme value. **Matrix-first (2026-06-17) revised this to M, not S, and KEPT IT DEFERRED:** the `i32::MIN` *literal* is itself narrowing-rejected — `x: i32 = -2147483648` → "cannot implicitly narrow integer to i32", because the unary-minus isn't const-folded before `int_value_fits`, which sees the positive `2147483648` (> `i32::MAX`). So a real fix needs negative-literal const-folding PLUS the not-null no-sentinel read PLUS the width-4 not_null distinction — M effort for a value (`i32::MIN`) that is virtually never real data. **`i64`/`integer` likewise DEFERRED**: its null IS the universal stack/register sentinel (`i64::MIN`, 60+ sites in `ops.rs` alone), so reclaiming it is a null-model rearchitecture. | M (was S) | ⏸ deferred — low value | [STABILITY_HOTSPOTS § H6](STABILITY_HOTSPOTS.md) |

Standing discipline (not queue items): every lowering-semantics change lands
with a `cross_mode!` cell or `tests/scripts/` file (H4's S half — add as a
CODE.md checklist line with the next CODE.md touch); every M+ design through
the design protocol; verify-armed before trusting the armed channel's silence
([reference: STABILITY_SWEEP § armed-channel restoration](STABILITY_SWEEP.md)).

### Routed from the `2026-09` bug review — the runtime-join TRIGGER, asked six ways (M)

`Scopes::free_vars` decides which return sources are routed through the runtime
`OpFreeRefIfDistinct` decision instead of a plain scope-exit free, and it decides it in
**six** places. The 2026-09 bug review collapsed the three that restate one OWNERSHIP
predicate (onto `Scopes::owns_freeable_store`, IR byte-identical across the corpus). The
remaining three are not copies — each is a different TRIGGER carrying its own ownership
test: one takes every `Reference`/`Enum` source once a null arm is reachable and tests
ownership not at all; one walks the *deps of* a collection source rather than the source;
one is the inverse, selecting arguments by a hidden attribute.

The predicate underneath all six is **"is this return a runtime join?"** — a question with
no home, which is why each new shape arrived as a seventh leg (@PLN85 P4-records, loft#936,
@PLN85 F2, loft#688, loft#1022, then the keyed leg). Twenty-two keyed-collection bugs
landed in one window and eleven were `silent-wrong`; the join legs are where several of
them live.

M, not XS: folding the triggers changes which sources are routed, and the corpus cannot
score that the way it scored the ownership collapse — the IR moves by design. Needs the
boundary matrix first (null arm × multi-source × hidden-attr buffer × keyed × nullable),
hand-computed, before any fold. See [BUG_REVIEW.md](BUG_REVIEW.md) § `2026-09`.

## Red-flag remediation — the live store-lifetime stream (2026-06-21 →)

> **The H-register above is the forward-risk hardening (cleared 2026-06-17). This stream
> tracked the store-lifetime / return-bind-ownership class that reopened 2026-06-21**
> ([STABILITY_REDFLAGS.md](STABILITY_REDFLAGS.md)). **As of 2026-07-09 every tracked bug in it
> is CLOSED** — the Cluster A residuals (#426, #429), the #462 leak, the native mixed-mode
> boundary (#460, #461), and the A1b temporary-subject UAF (@PLN90 #516). What remains is **one
> untracked refactor, not an active bug**: Cluster C, folding the **`copy_claims` source
> enumeration** onto the keystone (`validate_claims` and construction were probed and ruled OUT
> of scope — see the C row) — this retires the densest historical bug cluster *by construction*.
>
> **Re-opened 2026-08-20 by a re-survey** ([STABILITY_REDFLAGS § Re-survey](STABILITY_REDFLAGS.md#re-survey-2026-08-20--what-moved)).
> Cluster C's keystone shipped and Cluster A's forest is down from 11 sites to 2, so the
> store-lifetime half is indeed finished. What the re-survey added is a NEW cluster of the
> same *shape* in a different subsystem — the generic/monomorphisation path — plus one
> holdout arm-set. Both are generalizations, not bugs: the work is to collapse the
> duplicated case analysis so the undiscovered cases go with the discovered one, and per the
> standing rule at the top of this file nothing here gets filed.
> Finishing order:

| # | Item | Size | Status |
|---|---|---|---|
| **F** | **Cluster F — DONE.** Five collapses landed: the deferred-marker walk onto `Value::for_each_child_mut` (it enumerated 10 of 17 child-bearing variants, so a marker in a `Tuple` shipped as the placeholder — silent at `integer`, SIGSEGV at `text`, E0308 on native); `type_mentions_tv` onto `Type::contains_def`; the synthetic `__nullable<S>` no longer minted for a template's `T`; the tuple emitter's owned-text decision split from the literal passing through it; and `tuple_has_text_leaf` peeling `Optional`, which fixed a PLAIN `-> (text?, integer)` that never compiled on `--native`. Each proven under both gates — byte-identical corpora for untouched paths, twin-compared matrices for the changed ones. **The residual is CLOSED too** (loft#1026, cherry-picked from ../loft): its account matches the machinery this cluster's investigation had isolated from the other side, and all four carriers are correct on both backends. Doc claims corrected by measuring along the way: `is_type_var_operand`'s shallow peel is correct by design. | 5 collapses + residual | ✅ done (41befa0f, 4733f5e6, ebb32a31, ad961977, and loft#1026) |
| **C-holdout** | **`validate_claims` arm set made exhaustive — DONE 2026-08-21.** The keystone fold was correctly ruled out for it (row C — defensive walk, bounds-checks before dereferencing); what remained was the arm set. It ended `_ => {}`, so it walked past `Array`, `Ordered`, `Radix`, `Trie` and `DbRef` and answered "no problems" for a container it never looked at. Now: **`Array`/`Ordered`** bounds-check the claim record, its length, and each element rec before following it; **`Radix`/`Trie`** check the tree spine is in range and stop, the same split `for_each_owned_child` makes (the leaves belong to the primary and are validated through it); **`DbRef`** checks the store number it names. **`Index` and the scalar kinds now say they do nothing and why** — an index BORROWS its rows, so it has no claim of its own and walking into the primary would double-report; the narrow-integer kinds and a non-text `Base` hold values, not claims, so there is no record number to be out of range. The `_ => {}` is GONE, which is the load-bearing half: adding a `Parts` variant now fails the build at this match (proven — a probe variant produced `error[E0004]: non-exhaustive patterns` at `allocation.rs:2157`) instead of silently joining the unchecked set. Reachability measured under `LOFT_TRACE_CR` (which validates every copy): the `Radix`/`Trie` arm and the documented no-op arm both fire on a struct holding `spatial` / `index` fields — 2 and 36 hits — where both were previously swallowed. `Array`/`Ordered` and `DbRef` are written from the keystone's own treatment of those kinds and were NOT reached by any program I could construct, so they are unexercised rather than verified. | XS | ✅ done |
| **A** | **Cluster A — return/bind ownership** (collapse the per-site ownership re-derivation onto one carried `deps` fact). A.4 / A.3 / A.2-a7 + the native-FFI fixes merged (#423); A.1 part i (free-suppress, return-source SET) + the parser-counter substrate / #426B / #425-sibling / native-leak fixes on `tuxedo-substrate-followup`. **Residuals #426 + #429 both CLOSED** (2026-06-22). #429 (borrowed-view return over-free) landed the borrow-classify in `ref_return` + the nullable-enum copy-bind path in `gen_set_first_at_tos`; regression `tests/scripts/85-store-lifetime-enum-match-borrowed-view-overfree.loft` passes both backends. Cluster A's tracked bugs are done; the remaining ownership-substrate work is Cluster C. | — | ✅ done (residuals closed) |
| **C** | **Cluster C — fold `copy_claims` onto the keystone** (was: "per-`Parts` container taxonomy"). `remove_claims` already collapsed onto `for_each_owned_child` (C.0–C.3, merged) — that is the model thin-visitor and the proof the fold works. **The remaining scope is `copy_claims` ALONE.** A 2026-06-22 design probe *falsified* the wider framing: `validate_claims` does **NOT** fold (a defensive walk over suspected-corrupt heaps — it bounds-checks before following a pointer, where the keystone trusts it), and `record_new`/`record_finish` is a WRITE path, so forcing it onto a read-walk is over-reach. Retires the densest HISTORICAL bug cluster (@P290 SIGSEGV, @P306/@P318 hash slot-drift, @P309, #260/#330) **by construction**. Now H10. **This was the last item of gate 1.** A work item under the light flow, not a plan — the design was settled and the phases were three mechanical helper folds, so *this row is its lifecycle*. **✅ DONE 2026-07-10** (branch `tuxedo-cluster-c`): `for_each_owned_child` is now the single source enumeration for `remove_claims` and all four `copy_claims` kinds (`hash_body` already read it; phases 1–3 folded `index_body` → `array_body` → `seq_vector`). Each phase verified on both backends against the keystone guard + the phase's named regressions + the leak gate; a per-fold count `debug_assert` (proven non-vacuous) closes the length-vs-count gap `LOFT_COPY_CHECK` leaves open (phase 0 calibration). Destination build stays per-kind. | **S per copy helper** (was mis-sized M–L against the falsified wider scope) | ✅ done — [STABILITY_REDFLAG_REMEDIATION § Cluster C / H10](STABILITY_REDFLAG_REMEDIATION.md#cluster-c--h10--fold-copy_claims-source-enumeration-onto-the-keystone) |
| **@PLN87** | **Reference-default `&`-binding semantics — DONE.** `&` binds a LIVE REFERENCE to an addressable source (variable / field / element): reads see the source, writes and field/element mutation write through, uniform across scalars and heap. Shipped via PR #436 (the L1–L7 ladder, both backends) + #506 (`&`-write-back to a computed lvalue) + the W4 redundant-`&` lint (#510, on by default). The corrected live-reference model supersedes the original write-back framing; realizes the OWNERSHIP_MODEL binding rule. | M | ✅ done ([@PLN87](https://github.com/loft-lang/plans/issues/87)) |
| **B** | **Cluster B — stack-delta wrong-signal.** Deferred — unverifiable, no RED probe fires; latent. Pick up only on a real trigger. | — | ⏸ deferred |
| **462-leak** | **#462 residual store leak — CLOSED (2026-06-26).** The native-only `MonsterDef` record leak (the `mon_*` borrowed-view shape) is fixed and landed; issue #462 closed. | S–M | ✅ done |
| **N** | **Native mixed-mode boundary — CLOSED (2026-06-27).** #460 (`--interpret` aborts when a main-program fn is marked for cdylib dispatch with no cdylib built) and #461 (interpret→native shared-store call corrupts a complex nested struct arg) are both fixed and landed. | M | ✅ done |
| **A1b** | **Temporary-subject borrow UAF — FIXED (2026-07-06, default ON, @PLN90 #516).** A borrowed return whose subject is freed before the result's last use is now materialised by the caller so the subject out-lives the result (the `deps` decision), on both backends. | M | ✅ done |

D (typed-null encoders) merged; E (manifestation guards) dissolves behind A. Full
detail: [STABILITY_REDFLAG_REMEDIATION.md](STABILITY_REDFLAG_REMEDIATION.md),
[STABILITY_REDFLAGS.md](STABILITY_REDFLAGS.md), @PLN85.

## Done (this cycle — closing commits in the canonical homes)

- **#462 adopt-and-re-return vector leak — FIXED** 2026-06-26 (commit `cafe98a0`). A vector-returning fn that adopts a call result and re-returns it (`t = base(); t`), and the `t = base(); t += …; t` merge shape (`game_items()`/`game_monsters()`), leaked one store per call: the NRVO collapse redirected the inner call's hidden `__ref_N` buffer onto the retbuf but left its eager allocation orphaned. Fix in `parser/control.rs` (`nrvo_collapse_tail_set` + new `nrvo_collapse_defining_call`); vector-only. Crawler interp 531→0, native 752→216 (the 216 is the 462-leak row above). Regression `tests/leak_cases/clean/p462_adopt_rereturn_vector.loft`, both backends. #462 stays open for the record-leak residual.
- **Cluster A residuals #426 + #429 — both CLOSED** 2026-06-22 (see the A row above).

- **Pass-2 arity-growth CASCADE — COMPLETE.** Reference arm 2026-06-12 (one-buffer design: arity = signature+1 fixed at declaration, every return path delivers in THE `__retbuf`; 10-cell matrix green both backends). Vector/struct-Enum arm then closed via **#355** (multi-return-site vector behind a forward caller silently returned the WRONG element) and **#356** (mid-body `return f(g(x))` returned the null sentinel on native; fn-refs to struct-returning fns couldn't be CALLED — the latter also fixed by **#383**, merged in #393 this session). Both #355/#356 CLOSED. Live guards `tests/issues.rs::pass2_arity_growth_*` + `tests/scripts/387-text-fn-ref.loft`. The @PLN55 growth assert (the H2 residual) fell out — cleared.
- **Armed-corpus residuals ×4** — all resolved 2026-06-14: `132` silent interpreter UAF → `OpFreeRefIfDistinct` (regression `tests/scripts/372`); `collections.loft` two bugs → `parse_object_field` accepts `{}` (regression `373`) + `dedup_keyed` `secondary` flag for `other_indexes` (regression `374`); `166` verified over-strict; `75-native-stub` clean on the rebuilt armed binary. The armed channel's silence is now trustworthy.
- **H1 analysis-dependent arity** — RETIRED 2026-06-11 (@PLN55 phases 0–2; signature-time `__retbuf`, uniform return ABI, retro-patch deleted).
- **H2 typed deps** — steps 1–5 DONE 2026-06-12 (`Deps` newtype, space-asserting accessors, the positional contract retired via `CALLEE_FRAME_BIT`).  Residual (the @PLN55 growth assert on two lib fns) → cleared with the arity cascade above.
- **F11 error-path state** — swept, all four breaks FIXED 2026-06-12.
- **The armed-channel restoration** — four stale duals fixed 2026-06-12; the channel is the standing instrument.
- **`store.rs:1640` armed row (the "keyed armed UAF", 7 files)** — RESOLVED 2026-06-12 (4cba84c5): three mechanisms (header-as-`room` accessor → `Store::record_words`; parallel s_pos array header stomp; OpDatabase bytes-vs-words under-claim = a real release OOB write).  Armed corpus 12 → 5 files.
- **Plan-57 vector store-lifetime watermark** — CLOSED (@PLN2; rc removal complete).
- **Plan-53 cluster 1 + the aligned-V2 allocator half of cluster 2** — fixed/validating; the S4 half is queue #4.
