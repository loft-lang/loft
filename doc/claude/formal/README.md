<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/ — loft's strict formal definition (rules + tracked deviations)

This directory is the **strict** formal definition of loft. Each document covers one
area and has exactly two parts:

1. **Rules** — the formal definition we *want*: the judgments / relations / grammar
   loft is meant to satisfy, written tightly enough to be checked against. This is the
   **target**, not a description of today's code.
2. **Deviations** — every place the *current implementation* breaks a rule above,
   numbered (`D1`, `D2`, …), each with: the rule it violates, where it lives, the
   user-visible effect, and a status. **The deviation list is meant to shrink to
   zero over time** — closing a deviation means making the implementation obey the
   rule (often a bug fix or a refactor).

> **Two files per area: the rules, and their timeline.**  `<area>.md` is the CONTRACT and
> carries the current state — a `## Deviations` section giving the open count and one line per
> open entry.  `<area>-history.md` beside it is the REGISTER: every entry, open and closed,
> with its dates, measurements and closure record.  They are apart because a contract a reader
> has to skim past its own history stops being one they can skim — `ownership.md` was 1905
> lines of which 1748 were register.  Both files live in this directory, so `rule_tags.py`
> (which globs `formal/*.md`) resolves a citation to either.  The general form of the rule, and
> the report that ranks the docs still carrying their own history, is
> [RELEASE.md § 5b](../RELEASE.md).

> **The rules do not change to match the code. The code changes to match the rules.**
> A new edge that the rules can't express is a signal the *rule* is wrong (fix the
> rule); a place the code disobeys a sound rule is a *deviation* (fix the code).

## When to reach for this doc

**Before** the work, not after — these are the moments the rules decide something you would
otherwise deliberate about:

- **An issue calls something "a design call"**, or offers "two ways to close it". Check whether
  the rule is already written; if it is, the choice is not open. loft#1002 was filed as *"the
  choice is a design call, which is why this is filed rather than fixed"* — while
  [collections.md](collections.md) already carried `(Slice-Open) xs[(x,y)..]  open outward walk
  from a point`. The rule was written and the shipped tail was the deviation, so only one of the
  two "ways" was ever admissible.
- **You are about to ship a REFUSAL** (*"X is not supported"*). A rule may say it must work, which
  makes the refusal a deviation to record rather than a decision to make. loft#1006's
  `&(text, …)` refusal reads against [binding.md](binding.md)'s `B-Ref-Alias` (`&` makes ANY
  binding — *scalar OR heap* — a live link) and `B-Ref-Uniform` (no operation is special-cased).
- **You are about to change the observable semantics of a shipped surface.** The rule tells you
  which direction is the fix and which is the regression.
- **The two backends disagree.** The differential oracles named in each doc's Deviations section
  are the enforcement layer; a divergence is usually a deviation with a name already.

And the doctrine cuts the other way too, per the rule above: an edge the rules **cannot express**
means the *rule* is wrong and wants extending — that is not a licence to leave the code as-is.

> **An "OPEN: 0" line is a claim to re-measure, not a fact.** It is only as strong as the
> conformance corpus underneath it. [tuples.md](tuples.md) read `OPEN: 0` while loft#1004 (a
> tuple's `text` element written one index too high) and loft#1005 (a tuple `text` parameter that
> would not compile on `--native`) were both live tuple deviations — because the oracle it leans
> on, `tests/oracle/17-tuples-recursion.loft`, is all-`(integer, integer)` and carries no `text`
> at all. Before trusting a zero, read what its oracle actually covers.
>
> [ownership.md](ownership.md) read `OPEN: 0` for six weeks with loft#1029 live, for the same
> reason from a different angle: its Join corpus
> (`tests/scripts/1019-join-owned-arm-owner.loft`) moves four axes — which side owns, arm
> count, `if` vs `match`, free function vs method — and holds the ARGUMENT SPELLING fixed at a
> variable in every cell. The runtime witness that closes the Join only covered an argument the
> call site could NAME, so a literal argument leaked and no cell asked. Moving that one axis
> turned up **six more leaking spellings** — a field, a nested field, a vector element, a
> vector-typed field, `??`, and an `if` in argument position — none of them in the issue.
> **A corpus that varies the subject exhaustively still proves nothing about the axis it never
> varies** — count what is held fixed, not what is swept. (Closed 2026-08-20; the axis is now
> swept by `tests/scripts/1029-inline-argument-borrow-source.loft`.)
>
> [closures.md](closures.md) read `OPEN: 0` with a crash live, and the held-fixed axis there
> was not the subject at all but the **moment**. Its `L-Escape` corpus checks three
> destinations — a local, a struct field, a return — and every one of them writes into a
> place being INITIALISED, so the axis it never varies is *first-Set vs re-Set*. Assigning a
> fn-ref into a place that already held one kept the eight-byte form against a twenty-byte
> slot: `g = inc` panicked on `--interpret` while `--native` ran the same program, so it was
> a backend SPLIT that neither backend alone could witness. Sweeping the CONTAINER — vector
> element, keyed value, struct-enum payload, nested struct — came back entirely clean; the
> find was on the axis nobody had thought of as an axis. (D-clo-3, opened and CLOSED
> 2026-08-22 — the re-Set half is now swept at every destination by
> `tests/scripts/fn-ref-assigned-into-a-field.loft`.)
>
> [iteration.md](iteration.md) read `OPEN: 0` while every vector combinator was broken over
> a TUPLE element — `map` answering a packed DbRef as `343597383710`, `filter` segfaulting,
> `--native` refusing to compile. Its conformance corpus runs map/filter/reduce entirely on
> `vector<integer>`; the text cell is a different SOURCE kind, not a vector of text, so the
> ELEMENT TYPE is never varied at all. That is now the THIRD doc whose zero rested on an
> all-scalar corpus, after interfaces.md (scalar instantiation) and tuples.md (no `text`) —
> which makes "what does this corpus instantiate at?" the first question to ask of any
> conformance list, not a per-doc accident. (D-iter-1, loft#1074, opened and CLOSED
> 2026-08-22 — the zero is back, and now rests on a corpus that varies the element type.)
>
> **And a zero can fail a third way, with no oracle or corpus involved.**
> [tuples.md](tuples.md) read `OPEN: 0` again on the day it closed D-tup-1 by collapsing three
> disagreeing element lists into one — while D-tup-2 was live, because the surviving list is
> only consulted at one of the two sites that construct a `&(…)`. Single-sourcing a rule makes
> the answers agree; it does not make anyone ASK.
>
> So a zero rests on three things, and each has to be checked separately: what the corpus
> COVERS, which axes it holds FIXED, and whether every site that makes the decision actually
> reaches the rule.

## Reading guide

These docs are dense (they are a spec), but every rule is meant to be readable. How to
get through them:

- **Read the prose first.** Each substantial block of formal rules is paired with an
  **"In words"** reading in plain English. The formal rule is the precise version; the
  prose is the one to read first. If the two ever disagree, the prose is the mistake
  (fix it).
- **The notation, explained once:**
  - `Γ ⊢ e ⇒ τ` — "expression `e` *has* type `τ`" (the parser works the type out itself).
  - `Γ ⊢ e ⇐ τ` — "`e` is *checked against* an expected type `τ`" (the `τ` is pushed in
    from the surrounding code).
  - `τ ⤳ σ` — "a value of type `τ` is *accepted where* `σ` is expected", with no cast.
  - `⊔` — the *join*: the smallest type that contains both (for integers, the wider range).
  - `(Name)` in front of a rule is just its label, so a deviation can cite it.
- **The examples are the anchor.** Each area ends with *falsifying programs* — tiny
  snippets where obeying the rule and obeying today's code disagree. Read those to see
  what a rule actually buys.
- **A deviation (`Dn`) is a known gap, not a bug report** — "the code breaks this rule,
  here" — tracked to be removed, then deleted.

## Relationship to the working docs

These formal docs are **new and separate**; the existing planning/analysis docs are
unchanged and stay where they are:

| doc | role |
|---|---|
| [FORMALIZATION.md](../FORMALIZATION.md) | the **lens** — why formalizing is worth it, per-layer readiness, ranked rough spots |
| [TYPING_RELATION.md](../TYPING_RELATION.md) | the **analysis** of the type/conversion area — rough spots R1–R3, recommendations |
| [STABILITY_REDFLAGS.md](../STABILITY_REDFLAGS.md), [OWNERSHIP_MODEL.md](../OWNERSHIP_MODEL.md), … | the runtime/ownership working records |
| **`formal/*` (here)** | the **strict spec** — the rules, and the deviation list to drive to zero |

The lens docs answer *"is this worth doing and where does it hurt?"*; these answer
*"what exactly is the rule, and which lines break it?"* They cite each other but do
not duplicate: a deviation entry links to the lens analysis instead of re-explaining it.

## Areas

**Nine deviations are open, in five chapters:** heap.md 3, operational.md 2, binding.md 2,
coroutines.md 1 and performance.md 1 (`rule_tags.py registers`, re-measured on the joined tree
2026-09-22, after heap.md's `D-heap-26` closed and `D-heap-28` to `D-heap-32`, tuples.md's
`D-tup-15` and coroutines.md's `D-cor-4` each opened and closed that day, coroutines.md's
`D-cor-3` and collections.md's `D-col-5` closed, and coroutines.md's `D-cor-5` opened with
`(G-Hold)`). Every other chapter is at 0, and each zero is a claim to re-measure against the
oracle line its chapter names. The two in operational.md are the **meta** entry,
`D-op-1`/`D-op-2` — there being no shared operational semantics, the interpreter is the spec and
a backend divergence is test-caught rather than definition-caught (@PLN89's differential
oracle, an open-ended instrument and not a one-shot close); every operational chapter below
inherits them. No open row is a rule that
needs changing. What closed, when, and what it cost is each chapter's `<area>-history.md`;
this paragraph and the table below are derived from the chapters and say nothing the
chapters do not.

> ⚠ **This paragraph and the table below are a CLAIM to re-measure, exactly like an `OPEN: 0`.**
> They read `0` for closures.md and ownership.md while each carried three live entries, and `1`
> for tuples.md two weeks after `D-tup-1` closed — an index summarising eighteen chapters drifts
> from every one of them at once, and it is the first thing a reader trusts. The chapters are
> authoritative; this view is derived. Re-measure it with:
>
> ```bash
> t=0; for f in doc/claude/formal/*.md; do
>   case "$f" in *-history.md|*README.md|*ROADMAP.md) continue;; esac
>   n=$(awk '/^## ([0-9]+\. *)?Deviations/{d=1} d && /OPEN:/{print; exit}' "$f" | grep -oE '[0-9]+' | head -1)
>   n=${n:-0}; if [ "$n" != 0 ]; then echo "$(basename "$f") $n"; t=$((t + n)); fi
> done; echo "TOTAL $t"
> ```
>
> A chapter's own `## Deviations` line is the source of truth; this table restates it, and a
> restated predicate is one that can disagree.

| doc | area | status |
|---|---|---|
| [types.md](types.md) | type system + conversion relation (incl. integer width) | **0 open** — the value/null model (DN1–DN6), null-flow (`N-Prop`/`N-Domain`/`N-Cast`/`N-Store`, DN3-Float) and the narrowing rules; register in [types-history.md](types-history.md) |
| [binding.md](binding.md) | reference types & `&` (the bind-site link law) + the `const` immutability axis | **2 open** (D-bind-38 and D-bind-39, both `(B-Ref-Lvalue)`: a link to a TEXT place is refused, and so is one to an integer place stored in fewer than 8 bytes — the rule says both link) — `&` is a type annotation (`B-Ref-*`), the bind-site link law, `B-Ref-Reshape` (disturbing a container under a live `&` is refused), the two-level `const` model; register in [binding-history.md](binding-history.md) |
| [grammar.md](grammar.md) | concrete grammar + operator precedence | **0 open** — the 12-level precedence ladder; the prefix-`&`/infix-`&` overload and the non-CFG surface are decided edges (C81/C82) |
| [operational.md](operational.md) | small-step semantics — the scalar core | **2 open** — the META pair `D-op-1`/`D-op-2` (conformance is differential, not definitional), inherited by every operational chapter below; the rules are complete for the scalar core; register in [operational-history.md](operational-history.md) |
| [heap.md](heap.md) | store steps — alloc / read / write / **copy** / free / **drop** | **3 open** (D-heap-8 and D-heap-9 — the copy-lease rules `H-Copy-Refuse` / `H-Copy-Lease`, written 2026-09-15 ahead of their implementation, @PLN163; D-heap-15, a value the rules MOVE still copied) — the `DbRef`/`Store` model, the whole-value COPY (C86), `H-Materialise`, the LIFO free discipline whose soundness is ownership.md, the drop hook's one-release-per-resource rule (`H-Drop`: owner's scope end, reassignment, container cascade; a copy moves the responsibility); conformance via the oracle (D-op-1) |
| [layout.md](layout.md) | the store BYTE layout — `layout(τ)` (widths, offsets, packing, the reference encoding) | **0 open** — `D-layout-1` closed 2026-09-21 (loft#1562): every path that reads an existing image — `store_load`, the bind of an existing file, the whole-image URL loaders and the paged loaders — refuses a store whose `.dschema` records another layout (`L-Sound`). One format (RAM = disk); nullability is a sentinel, not a layout (`L-Null`); register in [layout-history.md](layout-history.md) |
| [iteration.md](iteration.md) | `for`, ranges, text iteration, the map/filter/reduce/comprehension combinators | **0 own** — index-cursor `for`, deterministic combinator order, fresh result vector; conformance via the oracle; register in [iteration-history.md](iteration-history.md) |
| [coroutines.md](coroutines.md) | generators — `yield` / `next`, stackful suspension, a generator held by a record or a collection (`G-Hold`) | **1 open** — `D-cor-5` (loft#1601: a generator held by a record in a keyed collection is never released); lazy one-value-per-advance; a loop body with a SECOND statement is eager on native (a decided edge, loft#836); conformance via the oracle; register in [coroutines-history.md](coroutines-history.md) |
| [concurrency.md](concurrency.md) | `par` — the one parallel construct | **0 own** — a parallel map consumed in source order; determinism CONDITIONAL on a pure worker; conformance via the oracle |
| [calls.md](calls.md) | function call & return — args, parameter binding, the frame | **0 open** — args left-to-right; scalar params by value, heap params share (`F-ParamHeap`), `&` writes back; returns independent; a void tail is dropped (`F-Drop`) and a block's value is its tail's (`F-Block`); register in [calls-history.md](calls-history.md) |
| [matching.md](matching.md) | `match` — enum-variant dispatch + payload binding | **0 own** — an expression; struct-payload patterns bind by name; `_` is the final catch-all; compile-time exhaustiveness |
| [tuples.md](tuples.md) | tuples — construct / project / destructure | **0 open** — D-tup-9 (a type-variable member of a tuple literal, loft#1365) opened and closed 2026-09-05 — positional products (n≥2), `.i` a compile-time index, destructuring, tuple returns, the reference tuple (`T-Ref-Rep`); ⚠ its differential oracle is all-`(integer, integer)`; register in [tuples-history.md](tuples-history.md) |
| [closures.md](closures.md) | lambdas / closures / fn-refs — capture + apply | **0 open** — both lambda forms capture identically; scalar-by-value / heap-shared capture; first-class into every container (`L-Escape`); `D-clo-18`/`D-clo-20` are decided refusals (C115); register in [closures-history.md](closures-history.md) |
| [formatting.md](formatting.md) | text formatting — `"{x}"` interpolation + value→text rendering | **0 own** — arbitrary-expression interpolation, per-type render, the width/align/pad/precision/radix specs, fault-safe interpolation, one rendering sink, `F-Target` (a template builds a VALUE against a type defining `lit`/`hole_*`); register in [formatting-history.md](formatting-history.md) |
| [interfaces.md](interfaces.md) | interfaces (traits) + generics — bounds, satisfaction, monomorphization | **0 open** — STRUCTURAL satisfaction (no `impl`), bounded generics, parser-side monomorphization, static satisfaction check; compile-time only (decided edges); register in [interfaces-history.md](interfaces-history.md) |
| [collections.md](collections.md) | collection kinds (`vector`/`hash`/`sorted`/`index`/`spatial`/`trie`), indexing & slicing | **0 open** (`D-col-5`, loft#1576, closed 2026-09-22: a displaced record now leaves every keyed member of its linked group); the six kinds, indexing, slicing (`Slice-Open`/`Slice-Cap` hold), linked groups; still a SCOPE doc graduating to rules; register in [collections-history.md](collections-history.md) |
| [ownership.md](ownership.md) | the `deps` / borrow **checker** (lifetimes) — distinct from binding.md's surface | **0 open** (D-own-43 closed 2026-09-17) — every store-lifetime decision reads the one total `deps` fact (`O-Deps`), per binding, per path, complete (`O-Complete`); the soundness proof heap.md's free rules rest on; register in [ownership-history.md](ownership-history.md) |
| [capabilities.md](capabilities.md) | sandbox **admission** — what a restricted caller may do (call / parameter / field / mutation rights) | **0 open** — the 6-rule judgment `P;ctx ⊢ e ✓` fully enforced, each closed entry with a RED/GREEN adversarial pair; register in [capabilities-history.md](capabilities-history.md) |
| [rewrites.md](rewrites.md) | the NATIVE emitter's cheaper forms — a header derived once, a scalar read once, a view's header at its binding, a stdlib wrapper as its op, a callee handed its invariant inputs, a push through its own header | **0 open** — the HOIST STATE the rewrites compose through (`R-State` one holder per path, `R-Refresh` the changing op refreshes at its own site, `R-Alias` ownership decides who may name one vector) and the rewrites themselves (`R-Switch` … `R-LeafChain`): every rewrite has a generation-time switch and a falsifier (`LOFT_HOIST_VERIFY=1`), the interpreter applies none and is the oracle; the emission audit (@PLN157 § V-r) is the emission-time check; D-rw-1 closed 2026-09-09; [rewrites-history.md](rewrites-history.md) records where each invariant is held |
| [performance.md](performance.md) | routines pull their weight — the DISTRIBUTION's speed contract | **1 open** — `Perf-Like` (hash-equal lanes before any comparison), `Perf-Weight` (per-release measurement against an industry-language twin — drift satisfies nothing), `Perf-Twin` (a twin is written where a hit is expected, never waived), `Perf-Cure` (the twin MEASURES, never ships — the cure goes to the engine, keeping libraries readable loft); D-perf-1 = loft#1570 |

## Roadmap

[ROADMAP.md](ROADMAP.md) is the single ordered view of every open deviation across the
areas — sequenced into the order to resolve them, each flagged **code→spec** (the default)
or **spec-may-adjust** (where the rule itself is the decision). Start there to see the path
to a spec-conformant implementation; the per-area docs hold the detail.

[VERIFICATION.md](VERIFICATION.md) is the companion worklist for the operational rules
written 2026-07-04 (heap / iteration / coroutines / concurrency / calls / matching / tuples /
closures): per-rule, the single falsifiable claim, its both-backends status, and the standing
guard that pins it. Where ROADMAP tracks *deviations to close*, VERIFICATION tracks *rules to
pin* — the concrete plan to drive the differential oracle down from every area to every rule.

## Rule tags — `@Name`, and the code cites them

A rule is only an anchor for the code if it can be found **exactly**. Each rule carries an
`@FR-` tag (`@FR-B-Copy`, `@FR-T-Ref`, `@FR-Col-Order`) — the family shape CLAUDE.md § Tracker
tags reserves, *"`@`-prefixed so regex is unambiguous"*, with its own namespace because a bare
`@Name` is not unambiguous: `@` already carries `@P259` / `@PLN3` / `@F7` / `@AAA-###` and the
corpus annotations `@ARGS` / `@NAME` / `@IGNORE` / `@EXPECT_ERROR`. Measured: a bare-`@` reading
of `src/` returned 4142 hits, not one of them a rule. A site that enforces a rule names it in a comment, at the one moment the fact
is reliably known: while making that site obey it.

What that buys, and none of it is available from a rule NAME alone:

- **the duplication question becomes a lookup** — *does this rule already have an
  implementation?* is `scripts/rule_tags.py sites B-Copy`, not a guess at which code shape someone chose;
- **the re-assertion count is a query** — a rule's sites ARE its citations, so it stays right
  as code moves, instead of being re-derived per investigation;
- **merge-or-not is arbitrated** — two sites citing ONE rule are candidates to merge; two
  citing different rules stay apart however alike they look today.

**Two constraints the measurement forced** (numbers in [IMPLEMENTATIONS.md](IMPLEMENTATIONS.md)):

1. **Match with an explicit boundary.** A substantial share of the defined rules are a prefix of
   another — `@FR-B-View` / `@FR-B-View-Base`, `@FR-T-Ref` / `@FR-T-Ref-El` — and
   `rule_tags.py check` reports how many. A plain `grep @FR-B-View` sweeps in its own sub-rules,
   and `\b` does not help because `-` is already a word boundary. So a citation matches `@Name`
   only when the next character is not `[-A-Za-z0-9]`. **Renaming them is deliberately NOT the
   fix** — the sub-rule names are meaningful, and a matcher that is right by construction beats
   the renames plus the churn. ⚠ This paragraph used to carry the count, and managed to say
   **21** in its first sentence and **23** in its last — disagreeing with itself before either
   figure also went stale. The number belongs in the tool, where it is recomputed rather than
   remembered.
2. **Only a DEFINED rule is a citation target.** `B-Ref`, `D-op`, `D-own`, `D-cap` and
   `D-op-null` read like rules and are family PREFIXES used in prose — no definition line
   exists for them. A citation naming one is an error, which is exactly what the resolve check
   catches.

**The measurement that argues for the mechanism rather than for the register.** A deviation
entry can name its own generator, in as many words, and still not prevent the next instance —
because the entry is reachable from the RULES doc and the defect is met in the CODE.
`D-bind-17` closed on 2026-09-06 saying what had been missing was *"one spelling of the SLOT
behind a link, at the nine sites that each asked the link's inner type bare"*. On 2026-09-07,
loft#1443 and loft#1454 found **six more sites doing exactly that**, for a different τ — a
fn-typed link — across the parser, both backends' emitters and the native reachability marker.
The entry did not fail to describe the class. It failed to be reachable from the sites that
needed it: a reader at `variables.tp(var)` has nothing to grep, because the thing they are
about to get wrong is an absence. That is the case for the citation direction — **a site
enforcing a rule names it, so "which sites ask this?" is a grep instead of a memory** — and it
is why the rules with no code representation are the backlog rather than a statistic
(`make rule-coverage` counts them; the contract-1 floors are 70 % annotated, 40 % guarded).
Better prose in the register could not have closed any of the six.

**Why this is the quality lever, and not just tidiness.** Fixing a bug has no intrinsic test
for *did we now cover every similar case?* — a fix is shaped by the subset of the language
that happened to get stressed, and nothing in it asks about the rest. A citation converts that
question into one a tool answers: given `@FR-X`, `rule_tags.py sites @FR-X` IS the coverage
check. So the tag is what ties the code to the LANGUAGE rather than to our incident history,
and generality — code organised around the constructions the language has — is the thing being
bought.  The bug that prompted a site is not what the comment should say (see
[DOC_QUALITY.md](../DOC_QUALITY.md)); the rule it obeys is.

**Adopt honestly rather than completely.** A citation naming a rule that does not exist is
worth failing on from the first day; *every rule has at least one citation* tightens as coverage
grows. Any rule→site index is **generated** from the citations, never maintained beside
them — a second copy of where the rules live is the defect this convention exists to remove.

**Coverage: `make rule-coverage`.** The contract-1 floors are **70 % of rules carrying a code
annotation and 40 % an active guard** — minimums for the freeze rather than targets to stop at —
and the command says how many rules short of each the tree is. No share
is written here on purpose — this line read `76 of 255 cited, across 163 sites` for long enough
to go stale, in company with three other homes that carried the same figure and rotted together.
A stale share does not read as stale; it reads as a measured position. ⚠ **An
uncited rule is not merely undocumented — it is one where the coverage question cannot be
ASKED**, because the query returns nothing and the absence looks identical to "no sites needed".
By area the gap tracks where the bugs still are: the chapters whose classes are RISING in
`make bug-review` hold most of the uncited rules, while `ownership.md` is fully cited and
`layout.md` all but one, their classes falling or paid off. ⚠ Read that correlation the right
way round: `ownership.md` is fully cited BECAUSE it was hammered, so coverage is a lagging
record of attention, not a leading indicator of safety. What it does say is where the next
coverage question cannot yet be asked at all.

The generic form of this argument, for any project: the `design-protocol` skill, § *As the
system grows, anchor the question on the RULE, not on the code*.

## Deviation entry format

```
### Dn — OPEN (<date>, loft#NNNN): <one-line name>
- **Violates:** <rule id(s)>
- **Where:** <file:symbol> (the site(s) that break it)
- **Effect:** <user-visible symptom / issue refs>
- **Status:** OPEN | IN PROGRESS (<branch/PR>) | CLOSED (<commit> — then delete)
- **Removal:** <the change that makes the code obey the rule>
```

**Every OPEN deviation a release can close has a tracking issue, named in its head as
`loft#NNNN`** — in the heading, or in the first line of a register bullet, which is where
`rule_tags.py` reads it.  A register records a deviation; it does not make anyone fix it, and a
deviation only this directory knows about is deferred work the tracker cannot count, rank or
schedule.  The owner's standing is that no deviation a release can resolve ships in one, so
each is filed the day it is opened — as a bug, with a both-backend repro, a verified workaround
and the labels the bug-filing policy asks for (`status:planned` when a plan owns the fix) — and
its entry and its issue close together.

The one exception is an entry no release can close, which says so in its head — the words `not
resolvable in a release` — and gives the reason in the entry.  `D-op-1` and `D-op-2` are the
case: the interpreter and the native generator are two implementations with no executable
semantics linking them outside the tests that run both, and @PLN89's differential oracle
narrows that gap without closing it.  The exception is for that shape, not for work that is
merely large.

`python3 scripts/rule_tags.py registers` names every open entry that has neither, and
`registers --issues` names an open entry whose issue the tracker has since closed — which is a
pair to re-measure, never a closure: `D-perf-1`'s issue closed at a bar the rules had since
tightened, and `D-layout-1`'s residual had moved to a path its text no longer named.

A CLOSED deviation leaves the rules doc; its entry — dates, measurements, what closed it —
is kept in `<area>-history.md`. The count of OPEN deviations per doc is the area's distance
from formal.

⚠ **A deviation NUMBER is a property of the repository, not of your checkout.** Several
agents hold sibling checkouts of this repo at once, each with a different subset of the
others' branches, and a register is cherry-picked in pieces like anything else — so the
next free `D-<area>-N` has to be grepped across **every** checkout, not across the file
you are editing. Measured 2026-09-08: `D-bind-28` was taken on a sibling branch and
reused here, because `grep '^> \*\*D-bind-'` over the local `binding-history.md` said it
was free — and it was free *in that file*, while `binding.md` in the same working tree
carried its header. Renumbered to `D-bind-29`.

The same partial-pick shape explains a count that looks wrong rather than old: a rules
doc can carry a deviation's HEADER from before the commit that closed it while the same
tree already contains the fix, so `binding.md` reads `OPEN: 1` beside a working
behaviour. **Do not reconcile a disagreement like that by editing a number** — establish
which side is older, and note it. `scripts/rule_tags.py` cannot help here either: it
takes a tag's status from the first 120 characters after its FIRST definition, so a
deviation written in parts reports one status for all of them (loft#1452).
