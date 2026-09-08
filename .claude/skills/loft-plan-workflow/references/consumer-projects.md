# Consumer (dogfood) projects

*Read this when working in a project built ON the engine — a game, a tool, a library
that dogfoods the language. It answers where a finding goes when two repos are
involved. In the engine repo itself, none of it applies.*

A **consumer** is a project built ON the engine to find out what the engine is
missing — a game, a tool, a library that dogfoods the language.  It runs the same
plan model as the engine repo, deliberately: one convention, so a plan reads the
same wherever you open it, and an agent moving between trees does not re-learn it.
What differs is only what a plan is allowed to CONTAIN.

**The one rule that makes the split work: a plan describes work THIS repo will do.**
Everything else follows from it.

| What you found | Where it goes |
|---|---|
| An engine defect, reproducible with the engine alone | An **issue in the engine repo**, minimal both-backend repro.  Never a plan here — a plan here would track work this repo cannot do. |
| An engine gap that blocks a phase | The engine issue, **plus** a line in the blocked phase naming it.  The phase stays `Blocked on <issue>`; it does not become an engine plan wearing a consumer's directory. |
| "The engine is awkward here" with no repro yet | A row in the light TODO doc until it has a repro.  A gap you cannot demonstrate is not yet a report. |
| Work this repo does — content, tools, a subsystem | The normal ladder above. |

**Why the discipline is worth it.** The two streams are adversarial on purpose: the
engine stream builds and fixes, the consumer stream uses and tries to break, and the
report is the product.  A consumer plan that absorbs engine work hides the finding in
a directory the engine's agent never reads — the gap is then discovered twice and
fixed neither time.  The engine's tracker is the shared channel; a consumer plan is
private by comparison.

**Read the other tree, write only your own.**  A consumer may read the engine's
source, docs, git log and handoff freely, and must not write to it — the symmetric
half of the engine's own "edit only this repo" rule.  Both trees are often worked
concurrently, so a staged file or a checkout lands in someone else's uncommitted
work.  Verify a cross-repo bug from a **scratchpad** package that points at the other
tree by path, never by editing inside it.

**What stays identical across repos** — this is what makes one convention cheaper
than two:

- **Identity is the issue number** in *that repo's own* tracker, claimed before the
  directory exists.  Numbers are per-repo and never shared; a plan id is only
  meaningful next to its repo.
- **The same value letters** (`S/R/G/F/U/C/Q/N`) and the same effort letters
  (`XS…VH (plus `L`, large multi-arc — ROADMAP's legend)`), so "a `G` plan at `MH`" reads the same everywhere.  Only the *examples*
  are repo-specific.
- **The same closing procedure** — move reference content out, rewrite incoming
  links, leave the closure record, set the lifecycle label on the issue.
- **The same phase-cutting rule** — both bounds, and a `Verify` cell naming the
  comparison ([§ Cutting a phase](../SKILL.md#cutting-a-phase--two-bounds-not-one)).

**Cross-repo coordination, when a phase genuinely spans both.**  Say in the plan
which repo owns which half, and what *done* means on each side.  An engine change
that consumers depend on is done when **every** named consumer is green — so name
them; "consumers are updated" with no list is a claim nobody can check.  Expect the
consumer's phase to sit `Blocked on <engine issue>` for as long as the engine's
release clock takes, and prefer a phase cut so the consumer half can land first
behind the old behaviour.

**Efficiency, concretely.** Most consumer work is not a plan: a row in the light
TODO doc beats a plan directory that only points back at a reference doc, and the
cap of 2–3 active plans is what keeps the tracker readable.  A plan whose only
content is "wait for the engine" is not a plan — it is one blocked row and an
upstream issue.

## In one line

A plan describes work THIS repo will do. An upstream defect is an issue in the
upstream repo — a plan that absorbs it files the finding where the engine's agent
never opens it, so the gap gets discovered twice and fixed neither time. A plan whose
only content is "wait for upstream" is one blocked row plus an issue.

