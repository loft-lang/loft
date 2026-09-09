<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# STABILITY_METHOD.md — find dual invariants, move algorithms to their data, then de-duplicate

The working method for turning a grown codebase into a stable one.  It is
[GOALS.md](GOALS.md) Goal E ("one home per fact", robustness by subtraction)
expressed as a **procedure with three separated passes**.  The separation is
the point: each pass produces a complete artifact before the next starts, so
the survey is never invalidated by the repairs, and the repairs are never
improvised without the survey.

The method exists because of a measured failure class.  Six bugs fixed on
2026-06-10/11 (#313, #314, #316, #318, #322, #323, #328) shared one anatomy:
**a single fact was implemented in two or more places, and the places
disagreed** — a flag and a layout answering "is this field split?"
differently per parse order (#313); five sites each deciding "who frees this
store" (#316/#323); a cache manifest claiming to cover inputs the parser
loaded behind its back (#322); the parse erasing pointer-ness that the docs
and the layout still asserted (#328).  None of these were typos or logic
slips — each was a *structural* defect: the invariant had no single home, so
the copies drifted.

## The trigger — a condition-thicket means the structure, not the logic, is wrong

Dual-home drift is one shape.  The other arrives over time on a structure that
was *fine when it was designed*.  A data structure is right for the features it
was built to carry — not in the absolute.  As later features and fixes pile on,
the implementations behind it accrete tests and branches: *"four tests on an `if`
to get into a branch with another `if`."*  That condition-thicket is the signal —
not a logic slip, not a coding-skill gap — that the structure has crossed an
inflection point and is now the burden.

This is the field's oldest principle, not a loft one.  Brooks: *"show me your
tables, and I won't usually need your flowcharts; they'll be obvious."*  Pike's
Rule 5: *"data dominates — if you've chosen the right data structures, the
algorithms will almost always be self-evident."*  Torvalds names the smell exactly:
good code *"rewrites it so that a special case goes away and becomes the normal
case"* — the condition-thicket IS his special-case-that-should-not-exist.  What
those quotes omit, and what this section adds, is the corrective: even their authors
refactor their core structures *constantly* — you do not get the structure right
once and keep it.  So the working skill is not "pick the perfect structure" (you
can't, see below); it is recognizing *when* the one you have has become the burden.

The mechanism is the same drift in a different dress: the algorithm needs a fact
the structure does not carry, so every site **re-derives** it — order-dependently,
with special cases — and the re-derivations diverge exactly like dual homes do.
loft's vector element-nullability is the worked case: *"is this element nullable?"*
lives nowhere — it is emergent from whether the element resolved to
`Enum(__nullable<S>)` or `Reference(S)`, decided at **parse time** (so it depends
on definition order), and the `not null` opt-out is *lost* once it collapses to the
dense form.  The fact is then re-derived across ~188 sites; a forward-referenced
element (`enum E { V { f: vector<S> } }` with `struct S` defined after `E`)
silently misses the rewrite and corrupts discriminant reads on both backends.  No
single edit was wrong; the accumulation is.

The recognition is itself a skill — separate from logic or fluency, and learnable.
Its most useful form is *felt*: when you are **stuck or thrashing** on a complex
topic — the change keeps wanting more tests, it feels harder or longer than it
should for what it does — that difficulty is itself the signal.  Like the rest of
this method it is a **sight discipline, not a willpower one** (the engineering-rigor
framing): being stuck means *suspect the structure*, not redouble the effort or add
care.  Concretely: **when a fix wants to add one more condition / branch /
special-case to re-derive a fact, especially a fact already re-derived elsewhere —
stop.**  That edit is condition #189; the correct move is to evolve the structure so it *carries* the
fact (here: an explicit element-nullable bit, with `__nullable<S>` synthesized by a
single lowering pass once all types are known), and let every site just read it.
Structures decay into burdens over a feature's life — budget for the evolution
instead of paying interest on the thicket.  Then run the three passes below to move
the now-named fact into its home.

And this is **not avoidable** — that is the point, not a caveat.  You cannot design
the perfect structure upfront: *which* facts it must carry is fixed by use cases and
bugs that do not exist until the code is built and exercised.  A structure decaying
into a burden is therefore the expected trajectory of a *living* one — not a logic
fault, not a foresight gap, not a sign someone designed it badly.  So the
professional move is neither chasing a perfect upfront design (impossible) nor
silently patching the thicket forever (paying interest) — it is to **understand it
is happening and signal it**: the moment a fix wants condition #189, record the
thicket as a pass-1 finding ([STABILITY_SWEEP.md](STABILITY_SWEEP.md)) naming the
fact the structure fails to carry, so the evolution is decided *deliberately* — with
the use-case-and-bug knowledge that exists now and did not before — instead of
quietly becoming condition #190.  The signal is the deliverable; the refactor
follows from it.

## Pass 1 — the sweep (find and document; do not fix)

Walk the whole body of code with one question: **which facts are asserted in
more than one way?**  The tell-tale shapes:

- a **flag and a derived structure** that answer the same question (a
  `*_d_nr` marker vs the registered layout);
- a **parse-time decision re-derived at codegen time** (or at cache-load
  time, or on the native backend);
- **two encodings for one value** (a null sentinel and a zero default);
- **one field carrying several meanings** (a deps list that is liveness here,
  ownership there, a type marker elsewhere);
- a **document asserting semantics the code does not implement** (the spec is
  a home too).

For every find, write a catalog entry (the live catalog:
[STABILITY_SWEEP.md](STABILITY_SWEEP.md)) containing four things:

1. **The invariant, in one sentence** — the fact itself, stated so a reader
   can check any site against it ("a struct field's layout is whatever
   `fill_database` registered — nothing else may answer layout questions").
2. **Every home** — each place the code (or a doc) asserts, caches, or
   re-derives the fact today, with `file:line`.
3. **The natural home** — which *data structure* the invariant belongs to.
   This names where the algorithm will eventually live (pass 2), and is the
   one judgment call in the entry: the home is the structure whose lifetime
   and mutation already match the fact's (layout facts live with the layout;
   ownership facts live with the store allocator; encoding facts live with
   the type that is encoded).
4. **The probe and its verdict** — a minimal program that makes the homes
   disagree, run on both backends.  A probe that breaks becomes a GitHub
   issue plus an `#[ignore = "stability-sweep: #NNN"]` test; a probe that
   holds is recorded as "probed, held" — coverage is a result too.

**No fixing during the sweep.**  A mid-sweep fix re-shuffles the ground being
surveyed: it moves homes, invalidates recorded line numbers, and — worse —
spends the fresh diagnostic context on one instance instead of the class.
The discipline mirrors the matrix-first debugging rule: the urge to fix is
the signal the survey is not finished.

## Between the passes — fix the known bugs first

**Fix as many open bugs as possible BEFORE the pass-2 rewrite (user,
2026-06-11).**  Each fix sharpens the contract between the routines the
relocation will touch: a routine whose edge cases are correct documents its
own obligations, while a buggy one leaves the mover guessing which
behaviours are contract and which are accident.  The sweep's findings list
is therefore also the fixing queue — work it down (ordinary bug-fix rigor,
one issue at a time) until what remains is exactly the structural moves
pass 2 exists for.

## Pass 2 — move each algorithm to its data structure

For each catalog entry, relocate the deciding logic INTO the natural home
named by the entry: the data structure whose state the invariant describes.
After the move, every former site *asks* the home instead of *re-deriving*
the answer.

This is the structural fix, and it is different from "deduplicate the code":
two textually different sites cannot be merged while each owns part of the
decision, but both collapse trivially once a method on the right structure
answers the question.  Worked precedents:

- `Parser::fn_ref_field_is_split` (#313) — read/write shape stopped
  consulting a parse-order-mutable flag and started asking the registered
  layout: the layout is the structure whose state IS the answer.
- `free_named`'s cascade (#323) — capture lifetime moved into the store
  allocator's free path; the scope analysis now only decides *not to emit*
  a free, never *who owns*.

A move is complete when the old sites contain **no remaining copy of the
decision** — only calls.  Each move is an ordinary change with the ordinary
gates (probes from pass 1 re-run as its verification matrix; both backends).

**When to run pass 2 (user, 2026-06-11): in a quiet stretch — when there is
not much rewrite activity in flight.**  Relocations cut across the same
files feature branches touch, so running them concurrently multiplies merge
conflicts and re-introduces drift while homes are mid-move.  The catalog is
deliberately durable for this: it waits, fully specified, until a low-churn
window (typically right after a release ships, before the next dogfood wave
starts), and each entry names everything needed to execute the move cold.

## Pass 3 — remove the duplications

Only now delete: the flags nobody reads, the re-derivations that became
calls, the second encodings, the dead guards that compensated for drift
between homes.  Deletion is last because pass 2 made it *safe* — each
removal is of something demonstrably unused (the usage-sentinel test from
the engineering-rigor skill applies: route the suspect through a loud
chokepoint, run the suite, delete on silence after a positive control).

The pass-3 deliverable is negative diff with the pass-1 probes still green.
Goal E's check applies verbatim: the robust version is the shorter one.

## Why three passes and not one

Fixing during the hunt optimises locally: each fix is reasonable, but the
catalog ends up describing a tree that no longer exists, the natural homes
get chosen one-at-a-time without seeing the whole family, and duplications
get "fixed" by patching both copies (which *preserves* the dual home).  The
separation forces the three different judgments to each happen with full
information: *what exists* (pass 1), *where it belongs* (pass 2), *what can
go* (pass 3).

## The precondition — the consolidation is only as safe as the test corpus it answers to

Pass 2 replaces many sites that each re-derive a fact with ONE analysis the sites
read.  That replacement is safe **only when a body of working examples already
exists that a *wrong* version of the new analysis would visibly fail against** —
otherwise the unified analysis is as brittle as the scar tissue it replaces, just
with fewer lines and more confidence (and confident-and-wrong hides longer, so it
is worse).  The go/no-go before generalizing is therefore not "do I understand the
fact?" — a clean story is always available — but *"is there a corpus where a wrong
me fails?"*  If the corpus cannot falsify a wrong version, the consolidation is
premature, regardless of how clear the invariant looks.

The corpus is not separate work — it is what the instance-era *produced*.  Each
symptom-fix in the bug history (#316/#323's five free-sites; the over-free class's
~16) shipped with a regression test, and **those tests are the durable
specification, decoupled from the implementation that motivated them**: a test pins
a *behaviour* (leak-free + value-correct + both backends), not a code path.  So the
unified analysis is correct precisely because it must satisfy every accumulated pin
at once.  Two consequences for pass 3 (remove the duplications):

- **Delete the fix, keep its test.**  The scattered code is disposable; its test is
  the constraint.  Removing a test with the code it guarded silently reopens the hole.
- **The generalization is only as sound as the corpus's ground truth.**  A pin
  validated by a weak test — agreement-between-two-binaries, or leak-only that misses
  value corruption (cluster V: interp-clean + leak-free still hid native corruption) —
  is a blind spot a brittle analysis passes while feeling safe.  Audit inherited pins
  for value AND length AND leak on BOTH backends before trusting them as spec.

This is why the order cannot be reversed: the heap model's specification had to be
*discovered* one failing case at a time, and the tests are where the discovery got
recorded.  @PLN85's join-aware ownership analysis (`src/use_analysis.rs` — built
inert, validated against an 87-cell boundary matrix *plus* the accumulated per-fix
regression suite, then landed gated off-by-default) is the worked example; see
[plans/85-store-lifetime-retirement/ownership-analysis-gaps.md](plans/85-store-lifetime-retirement/ownership-analysis-gaps.md).

## When the fix WIDENS what flows, sweeping the class is a precondition

The consolidation above replaces many sites with one.  The narrower cousin — one shared
helper is wrong, and the sites that call it inherit the defect — has a trap the precondition
section does not cover: **a fix that turns a rare input into a common one arms every sibling
you did not fix.**

Worked example (2026-08-25, `Function::depend`).  Nine sites looped over a dependency list
calling a setter that REPLACES rather than appends, so every list silently collapsed to its
last element.  Six were the filed defect.  Fixing those six is precisely what makes lists of
length > 1 reachable — and the other three sat downstream, ready to collapse the newly
multi-element lists the fix had just made possible.  Sweeping the class was therefore a
precondition for the six-site fix being safe, not a tidy-up afterwards.  *"I will get the
others later"* is a regression scheduled for later.

Two habits make that sweep reliable:

* **Screen by the SETTER, not by the iterated expression.**  Grepping the obvious shape
  (`for … in ….depend()`) found the six known sites and read as complete.  Screening instead
  for *the replacing setter called anywhere inside a loop* found nine — including a
  save/restore pair whose SAVE side looped correctly over the whole list while its RESTORE
  side collapsed, an asymmetry that had been visible within its own six lines for as long as
  it had existed.
* **Prove the screen is not vacuous before believing its zero.**  Run it against
  `git archive HEAD` and check it reproduces the count you already know by hand — here 9 on
  the pre-fix tree, 0 after.  A screen that cannot find the bug you have already found is not
  evidence that no others exist.  Same discipline as `make profile-corpus`.

**A second instance, and it inverts the intuition** (2026-08-25, the `*Nullable` swap).  The
same family had TWO duplicated lists: a swap table, duplicated *deliberately* with a comment
saying why (*"kept inline… so the dispatch table stays grep-discoverable from both swap
sites"*), and a wrapper-getter list that nothing defended.  **The defended duplicate stayed in
sync across all three copies; the undefended one drifted** — one copy was extended past the
integer wrappers and another kept the four it was born with.  So the lesson is not "never
duplicate": it is that a duplicate kept on purpose still needs something other than memory
keeping it honest.  The copy is now one function, and the remaining pair has a gate
(`doc_hygiene::the_nullable_swap_tables_do_not_drift`) that pays for the discoverability trade.

**And it drifted on a channel nobody was checking.**  Every cell of that matrix answers the
same VALUE either way — the defended read yields `null` whether or not the swap applied — so a
value-based probe of the exact 2×2 comes back clean and reads as a pass.  The difference is on
the REPORT channel: a defended site that still logs.  Before building a matrix, name the
channel each cell is scored on, and check the one the mechanism actually acts on; see
[DEBUG.md](DEBUG.md) on captured-but-uncompared channels.

And **bound the blast radius with a property, not with confidence**.  The fix above changes
*which* deps a value carries but never *whether* it carries any, so every decision reading
`depend().is_empty()` — at least three of them, each measured load-bearing — provably cannot
move.  A one-line invariant of that kind is worth more to a reviewer than a green suite,
because it says what the change *cannot* do rather than what happened not to break.

## The rule-led walk — the standing practice, measured in years

The three passes above start from a **condition thicket**: you notice a structure has decayed
and you go clean it. That works, and it needs someone to notice. The rule-led walk starts from
the **formal rules** instead, which makes it a queue rather than an observation — and a queue
long enough to work from for years.

**Which is it — a walk or a campaign?** `make campaign-review` answers that on four measured
gates rather than by feel (@PLN155 arc A, and [BUG_REVIEW.md](BUG_REVIEW.md) § The pass): a
class passing all four earns a plan, one to three earn a walk, and gate 2 alone earns a queue
entry. Two readings are worth knowing before you reach for it. A class whose keystone has
landed but has not been scored yet reads `MEASURING` and is held back whatever else it
passes. And a class whose walks landed days ago cannot be scored either — the report says
`unjudged (thin window)` instead of a verdict, because a two-bug difference in a hundred-bug
window separates nothing.

**Why the rules are the right index.** Code moves; a rule does not. `formal/`'s rules are the
thing two implementations are both claiming to implement, so they are the only stable place to
ask *"is this the same question?"*. That is what `@FR-` tags are for, and what makes the
duplication question askable at all rather than a matter of taste.

**The size of the queue, so nobody plans it as a sprint.** `scripts/rule_tags.py` reports the
position:

```
257 defined rules · 78 cited · 203 citation sites
```

**179 rules (70 %) have no code representation** — for those, *"where is this enforced?"* has
no answer. And of the 78 that do, **23 are cited from two or more files**; the most scattered
is the most instructive. One rule is comfortably an afternoon. The queue is therefore measured
in years, and the practice has to survive being picked up and put down.

### The loop

1. **Pick a rule, not a site.** Rank by `rule_tags.py dups` (most scattered first) or by which
   rule sits under a class the bug review says is rising. Both were true of `@FR-L-Null`: 13
   sites across 8 files, and 14 of that cycle's 27 bugs named null.
2. **Split the rule into the QUESTIONS its sites actually ask.** A rule with a dozen citations
   is rarely one question. `@FR-L-Null`'s thirteen were two — *"is this the same storage?"*
   (the peel) and *"what value means absent in it?"* (the sentinel). Merging those would have
   been the early-abstraction failure; the split is the first product of the walk, and
   recording it is what stops the next reader re-deriving it.
   ⚠ **The split can run INSIDE a shared helper, not only across sites.** A predicate every
   site calls may itself be answering two questions, and then widening it is the wrong move —
   it breaks the callers it currently serves. `data::is_dbref` answers *"does this occupy a
   DbRef slot?"* (LAYOUT) for seventeen of its eighteen callers, where a `Type::Tuple`
   correctly answers **no**; the eighteenth asked *"can this binding reach a store?"*
   (BORROW), where a tuple answers **yes** through its elements. The cure was a sibling
   predicate, not a ninth variant on the list. So before adding to a list, read what each
   caller DOES with the answer.

3. **Per question, find the ROOT — the one home.** Usually it already exists and the callers do
   not ask it. `vectors::is_collection` was already the declared home for *"which collections
   are store-backed?"*, and the broken site spelled its own `matches!(should, Type::Vector(…))`
   instead.
4. **Verify the RELATED cases with the root.** This is the step that yields. Once the root is
   known, its siblings are cells you can enumerate rather than guess: the other collection
   kinds, the other positions (local / field / argument / return), the other spellings of one
   notion, the READ twin of a write. A root you cannot enumerate siblings for is a root you have
   not found yet.

   **Order them shallowest first** — a cell a programmer reaches without knowing the language
   has edges, before one reached by composing four features nobody would naturally combine. A
   shallow cell that fails is the finding; a deep cell that fails is a note. ⚠ That is an
   ORDERING, not a licence to refuse: whether a shape must work is settled by the rule you are
   walking, and **a rule that gives a clear picture of what to implement is implemented right
   away, at any depth** — refusing there would be a deviation, not a decision.

   The ranking is not a new judgement to make: it is what the **`wa:` labels already measure**,
   because a contrived cell has the simple thing to fall back on by construction (`wa:clean`)
   while a casual user who hit a wall on an obvious shape has nowhere simpler to go
   (`wa:none`, *"blocks whoever hits it"*). Read `wa:none` as decisive; `wa:clean` only runs one
   way and is weak evidence. That is also why a VERIFIED workaround belongs in every issue —
   it is the ranking datum, not a courtesy.
   [GOALS.md § Not every unwalked cell is worth the same](GOALS.md).
5. **The defects live in the disagreements.** Where two sites answer the same question
   differently, one of them is wrong, and the wrong one has usually been wrong quietly.
6. **Guard on the channel the defect actually moved.** It is frequently not the value channel —
   a nullable keyed local produced correct answers and a bogus `OpFreeRef`, visible only as
   `BUG (#306)` on stderr. Name the channel in the guard's `@falsified-at` line so the next
   reader knows what would fail.
7. **Cite last.** The citation is the receipt for work done, never the work.

### What makes it hold up over years

- **A negative result is a product.** `@FR-L-Null`'s sentinel half turned out to be genuinely
  consolidated — two tables keyed differently (`Type` variant vs content-type number) whose
  doc *claimed* they agree. Tested: 9 types × 3 routes, 27 cells, all agreeing. That claim is
  now measured, and no one has to re-derive it. A walk that finds nothing has still converted a
  claim into a fact.
- **Do NOT file the de-duplication itself.** The fix a walk exists to make — one home adopted,
  a hand-spelled list retired, and whatever that list's disagreement was causing — is the WORK,
  not a report about the work. Filing it floods the tracker with items whose only reader is the
  person already fixing them, and buries the issues that need someone else. The two streams
  split on exactly this line: this one walks rules and de-duplicates, the sibling checkout keeps
  the issue list short and lands fixes as PRs, so an issue is a HANDOFF and costs someone's
  attention. If nobody but you will act on it, it is a commit message, not an issue.

  ⚠ **But the boundary runs the other way too, and getting it wrong starves them.** A walk
  surfaces far more than it fixes, and everything it surfaces that needs a DIFFERENT pair of
  hands is that stream's supply. File those generously and file them well — a walk typically
  produces several per defect it cures. The line is not *"did I find it?"* but:

  > **Would fixing this be part of the same commit as the de-duplication?**
  > Yes → it is the work; it goes in the commit message. No → it is an issue.

  Three shapes come out of that, and today's walk produced one of each: the **de-duplication
  itself** (a `Vector`-only list retired onto `vectors::is_collection`, plus the bogus free its
  disagreement was causing) is the commit; a **separate root the walk merely revealed** (a
  nullable `index<T[k]>?` failing layout, loft#1125 — A/B'd as pre-existing, its own
  investigation, its own fix) is an issue; and a **de-duplication blocked on another defect**
  (the `⇐` channel's return position, loft#1122, which could not land until the `__retbuf`
  divergence behind it did) is an issue too, saying plainly what blocks it.

  An unmerged branch is not itself a reason to file: the other checkout cherry-picks from here
  when it needs a fix, so work in flight is reachable without a merge.
- **Findings that are not this defect get FILED, not folded in.** The `@FR-L-Null` walk turned
  up a nullable `index<T[k]>?` that fails layout outright (loft#1125) and a latent unpeeled arm
  in the generic `FromNull` loop. Neither belongs in the fix; both belong on the record. Folding
  them in is how a one-afternoon walk becomes a three-day rewrite that nothing can review.
- **A/B every causal claim against a reverted build.** *"My change caused this"* and *"this was
  already broken"* look identical from one run. loft#1125 was called pre-existing only after the
  hunk was reverted and the errors came back byte-identical.
- **A rule's own CHECKER can have a classifier hole, and it reports that as compliance.**
  `scripts/o_proxy_check.py` gates @FR-O-Proxy's obligation and reported the set clean while
  a site freed on the proxy with no override — because it classified by whether the test was
  NEGATED, and `if !…is_empty() { continue; }` puts the conclusion on the fall-through. A
  clean gate is a claim about the classifier as much as about the code, so verify BOTH
  directions on each syntactic form it claims to cover: remove a known-good guard and confirm
  it fires. Fixing the classifier also needs its own control — the first widening ("the rest
  of the function") produced a false positive on a loop that only pushes to a list, and the
  right region is what the keyword actually exits.

- **Do not optimise the citation count.** `76 → 255` over unchanged duplication would read as
  progress while nothing had changed. The count is a position marker, not a target; what moves
  is the number of questions with one home. `doc_hygiene::every_rule_citation_resolves` keeps
  the marker honest by failing on a citation that names no rule, but it cannot tell an earned
  citation from a sprinkled one — only the reviewer can.

### What a walk owes at its end — the signal, not the verdict

A walk that finds four defects in one area has NOT established that the area is rotten. Two
situations produce the identical count and look the same from inside the walk:

- a **sore spot** — machinery so fragile it keeps manufacturing bugs, where the answer may be to
  cut the shape rather than repair it (§ Stability trumps features);
- a **maturing asset** — a feature being exercised properly for the first time, converging, and
  about to become one of the language's strengths.

Telling those apart needs the project's history and its direction, which is a judgement from
outside the walk; it is why the PR stream is owner-controlled ([CLAUDE.md](../../CLAUDE.md)
§ Branch policy). **So report the evidence and do not editorialise the verdict** — a walk that
concludes *"this subsystem is a mess"* has spent its credibility on the one call it is least
equipped to make.

What IS the walk's to report, because it is measurable from inside:

- **Convergence or divergence.** Did each fix close a class — the remaining siblings verified
  clean — or did each one reveal two more? A walk whose route table shrinks as it goes is
  converging; one whose findings branch is not. State which, with the counts.
- **Whether the defects share a root.** Four faults from one unpeeled `Optional` is a different
  fact from four independent faults that happen to be adjacent, and only the first is evidence
  about the machinery.
- **Whether the rules covered the cells.** A shape the rules settle and the code got wrong is a
  deviation being closed. A shape the rules cannot express is a gap in the definition, and that
  is a design question rather than a quality one.

### When a filed issue names which route is broken, re-derive it from the layout

An issue reporting *"route A is wrong, routes B and C are correct"* has already done a
comparison, and a comparison over routes elects the MAJORITY, not the truth. Two routes agreeing
is exactly as weak as two backends agreeing ([[agreement-is-not-correctness]]) whenever both
share the mistake — and a write and a read that make the SAME offset error always do, because
the write's error is what the read's error is compensating for.

The oracle is the declared layout, not the programs. In loft that is one command:

```
LOFT_DUMP_TYPES=1 loft --interpret p.loft      # every type, its size, and every field's offset
```

loft#1134 was filed as *"one field high through a `for` loop; correct as a local and correct by
index"*. The dump said `__tuple<integer,S?>` holds `_1:__nullable<S>[8]` and
`__nullable<S>::Some` holds `enum:byte[0], payload:S[8]` — so the payload is at 16, the loop was
the ONLY reader that went there, and the write and the indexed read were a matched pair of
mistakes. Fixing what the report named would have broken the one correct route.

Reading the layout first also **re-scopes** the defect, which is the larger half. Once the dump
says a discriminant sits at offset 8, the question stops being *"which field does it answer
with?"* and becomes *"what happens to the byte the tag should be in?"* — and the cell that asks
it is a PRESENT value whose first field is zero. That cell was not in the filed matrix, it is
not suggested by any route comparison, and it is the one that showed presence had become a data
byte: `S { a: 0, … }` read absent, and a `float` first member read absent for every value whose
low byte was zero. Same defect, two orders of magnitude more program surface.

Corollary for the fix: when the write is corrected first, **expect passing cells to fail**. A
cell that was right by cancellation goes wrong the moment one half is repaired, and that is the
second site announcing itself rather than a regression to back out. Fix the write, re-run the
whole route table, and treat every newly-red cell as a reader to visit.

### Why this is not the same as the screens

`ir_walker_audit.py`, `matrix_axes.py` and the rest rank SITES: they answer *"who might have
forgotten a variant?"* over the whole tree. They are worth running and they found real defects,
but their yield per unit of effort is low, and BUG_REVIEW.md's `2026-08` (3rd) cycle measured
how low against a two-checkout control. The rule-led walk is bounded by a rule instead of by the
tree, comes with its own oracle (the rule states what must be true), and ends with something to
enforce. Reach for a screen when a rule walk has named a class and you want its full extent;
do not reach for one as the way in.

## Relation to the rest of the method stack

- [GOALS.md](GOALS.md) Goal E — the destination this method walks toward;
  § "Stability trumps features" governs what to do when a sweep finding is
  better closed by rejection than by consolidation (→ C74/C75 precedents).
- The **engineering-rigor skill** — supplies the per-finding instruments
  (boundary matrix, usage sentinel, falsification probes).
- [DESIGN_PROTOCOL.md](DESIGN_PROTOCOL.md) — pass 2's moves are designs;
  load-bearing ones get the protocol (name the invariant, count re-assertion
  sites — the catalog already did both — then probe to falsify).
- [STABILITY_SWEEP.md](STABILITY_SWEEP.md) — the live pass-1 catalog and
  work list.
- [CODEGEN_METHOD.md](CODEGEN_METHOD.md) / [OWNERSHIP_MODEL.md](OWNERSHIP_MODEL.md)
  — the same "facts live in the structure, not re-derived per-site" principle
  for the compiler.  The *trigger* section above is its temporal form: a
  structure that was fine decays into a burden as features pile re-derivation
  onto it — recognizing that inflection point (the condition-thicket) and
  paying for the evolution is a net win, because it retires the bug *class*,
  not one instance.

---

## Where the method points next

[STABILITY_HOTSPOTS.md](STABILITY_HOTSPOTS.md) (2026-06-11) applies this
method's lens to *designs* instead of routines: the eight structures the
bug history says will keep manufacturing bugs (H1 analysis-dependent
arity is the headline), each with sized mitigation work and a landing
order.  Treat it as the input queue for the next pass-2-style quiet
window.
