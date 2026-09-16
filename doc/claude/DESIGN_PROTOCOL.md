<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Design Protocol 1 — A Design Is a Testable Hypothesis

> **Moved to a skill.** The protocol now lives as the **`design-protocol` skill**
> (`.claude/skills/design-protocol/SKILL.md`) — a self-contained, tree-agnostic
> sibling of the `engineering-rigor` skill, loaded on demand rather than carried in
> every context. It is the DESIGN-mode counterpart engineering-rigor routes to.
> Run `/design-protocol`, or it loads automatically when you are about to commit to
> a load-bearing design. This page is kept as a stable anchor for the doc-graph, and
> holds the fuller treatment of one part the skill only summarises:
> [§ Deciding what a construct means](#deciding-what-a-construct-means--from-its-use-cases).

The first protocol **graduated** from the [Design Verification List](DESIGN_VERIFICATION.md)
(concern **C1 — brittleness over bugs**). In one line: *a design is a testable
**hypothesis** about an invariant, not a plan you execute* — name the one invariant,
count its re-assertion sites, build the cheapest probe that could **falsify** each
load-bearing claim, then build and validate against the written prediction; and for
exact-invariant domains where you cannot even form the invariant, flip to the
constructive instrument (plot a concrete instance of the *answer* and read the
invariant off it). The full method, evidence, and worked examples are in the skill.

## Deciding what a construct means — from its use cases

### Why the protocol's two instruments do not reach it

The skill has two instruments, and both assume a correct answer exists. A falsifying probe
needs a candidate design to break. The constructive instrument needs a construction to
recover — an exact target such as a round-trip state or a geometry. What a user-facing
construct *means* has neither. What a bind, a copy, a drop or a return does is a choice, and
the programs people write with the construct pull that choice in different directions.

This is the kind of decision that goes wrong quietly, because it rarely looks like a design
question. It usually arrives as a defect ("this releases twice") whose fix seems to follow from
the rule already written down.

### The procedure

1. **List the programs a user writes with the construct.** Vary what the construct is sensitive
   to: lifetimes that nest and lifetimes that cross; resources that can be shared and ones that
   cannot; the move-like uses (a parameter, a `return`, a last use); a value bound back into its
   own variable.
2. **Run each one today**, as a separate process, on every backend. One process per program,
   because a wrong result in one cell can corrupt the next when they share a run.
3. **Table every candidate rule's result per program**, beside today's result.
4. **Decide in the rows where the candidates disagree.** Rows where every candidate agrees carry
   no information about the choice.
5. **Write the decision where the rule lives** (for loft, `doc/claude/formal/`), with the table
   as its justification.

Do not draw the candidate rules from the rule text or from the defect list. Both encode the
model in force, so every option derived from them inherits it. A defect list in particular
cannot question its own premise: it records what the current rule calls wrong.

### Worked example — copy leases (`@PLN163`)

The question began as a family of "released twice" defects for types with an `OpDrop` hook, and
the first designs were fixes under the existing rule, `(H-Drop)`: *a copy takes over the release,
the source stops dropping.* Each model below was replaced by one program:

| program | model on the table | what the program showed |
|---|---|---|
| `b = same(a)`, where `same` returns its parameter | a drop releases a RESOURCE once, so two drops are a defect | the defect list framed the options as "which copy should stop releasing" |
| `a = mk(); b = a; b.id = 4` | same | two structures exist — two drops are right; today's one drop leaves `a`'s structure never dropped |
| `b = a` inside a block, `a` read after the block | a drop ends a STRUCTURE | `b`'s drop releases a handle `a` still uses — no drop rule fixes it; the copy needs its own lease |
| an outward network connection, a writable file | a copy hook makes a second lease | `dup` shares one stream and one position — a second lease cannot exist, so refusing the copy must be the default |
| `a = same(a)` | — | a value copied back into its own variable is one structure: one drop |

The design that came out — `OpCopy` makes a second lease, a type without it refuses the copy at
compile time, and sharing a store stays an optimisation — was not among the options the rule text
offered. See [`plans/163-copy-leases.md`](plans/163-copy-leases.md).

### When the patches point the other way

Short-term designs often point *opposite* to the design that is needed. A local fix answers *"how do
I make this case right under the current rule?"*, and a correct answer to that question
strengthens the rule — even when the rule is what is wrong.

The signs, which tend to appear together:

- **The fix infers an intent the language gives no way to state.** Flags, marks and witnesses
  guess on the programmer's behalf what a copy or a release should do.
- **One rule keeps collecting exceptions.** *"A copy off a parameter moves nothing"* became
  *"… unless it is a local holding the caller's record"*, then *"… unless that local is written
  through"*.
- **The defect list re-cuts instead of shrinking.** Closing one shape reveals its neighbour;
  `formal/heap.md` D-heap-1 records its list being re-cut four times as it was measured.
- **Each option grows its own machinery.** The next proposal needs a runtime check, and the one
  after it a hidden value in the calling convention.

Evidence from the copy-lease case: in the drop-release walk before the decision, about ten
closures — per-path flags, the caller-record mark, the carrier and branch families, a warning for a
member copied into a container — each made the compiler better at guessing where a release had
moved. The needed design removes that guessing, and its plan's P5 phase deletes most of it.

**At the third exception to one rule, stop patching and walk the use cases.** The patches were not
wasted: the matrices, gates and censuses they built are what make the walk measurable in minutes
rather than a matter of opinion.

### Relation to reading the formal spec first

`CLAUDE.md` asks for the formal spec to be read before a fix that has a choice in it, and that stays
right for its question — *is this behaviour a deviation from the rule?* A rule does not settle the
different question — *is the rule itself right?* Many rules were written during validation passes
rather than decided from use cases, so when a fix keeps fighting a rule, walk the use cases before
trusting the rule's premise.

## See also

- **`engineering-rigor` skill** — the synthesis + router; this protocol is its DESIGN-mode depth.
- [DESIGN_VERIFICATION.md § C1](DESIGN_VERIFICATION.md) — the incubator this graduated from.
- [GOALS.md](GOALS.md) Goal E — robustness by subtraction, the deep reason the short version is usually the robust one.
- [DESIGN_DECISIONS.md](DESIGN_DECISIONS.md) — where a decided meaning is recorded (C86 is a bind-semantics example).
