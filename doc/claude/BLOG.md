<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Blog subjects — what a post about loft and its games can claim, and where the evidence is

The owner writes a blog about loft and the game suite built on it. This doc answers one
question: **which subjects are worth a post, and where does each claim's evidence live?**

It carries no numbers. Every figure a post quotes is re-measured with the command or source a
subject names, on the day the post is written: counts in a doc age against a tree that moves
(`BUS_FACTOR.md`'s own line and file counts are the worked example), and a published number
cannot be corrected by the next commit.

## The rules for every post

- **It is the owner's blog.** He gives each post's subject, its reasoning and its word use. An
  agent helps — language corrections, ordering, suggested content — and nothing is published
  until he has approved it and made it something he would say himself. The subjects and
  outlines below are suggestions to draw on, not posts. The working rules are in the blog
  repository's `CLAUDE.md`.
- **The blog is about the project** — loft, its games and its libraries. AI is a big part of how
  they are built and runs through the posts, but general AI commentary is out of scope. Two of
  the owner's convictions run under the series: using AI is a constant process, not a prompt
  and a miracle; and a project's quality follows the person behind the keyboard. A post shows
  them through what happened in the project. The blog's `CLAUDE.md` holds them in his words.

- **Re-measure, then quote.** A figure carries the commit or date it was measured at.
- **Name the ancestors.** A subject below that has precedent in another system says which; a
  post that claims "first" where it is not loses the reader on the one point they can check.
- **Publish the weak numbers with the strong ones.** loft's own docs pair every strength with
  its cost ([STRONG_POINTS.md](STRONG_POINTS.md), [CONTROL.md](CONTROL.md)); a post does too.
- **A limitation is a claim to re-measure.** Run the guard or issue repro a doc cites before
  repeating it — the OCaml bar's tier A read as missing after @PLN165 had built it.
- **A consumer repo is read, never written.** Cite its files; quote its measurements with
  their decision tag (`@M094`), which is how the source stays findable.

## The series

The posts themselves live in [loft-lang/blog](https://github.com/loft-lang/blog), published at
`loft-lang.org/blog`; this doc holds their subjects and sources, never their text.

The series opens with **why the project exists and how it is made**, not with a running log of
development. Posts about what is happening in a given month come later.

| # | post | draws on |
|---|---|---|
| 1 | *A team of eager juniors who google a lot* — building the language with AI agents | subjects 1–3 |
| 2 | How the games, the libraries and the language features connect | subjects 8–12 |
| 3 | What loft is for — its goals | [GOALS.md](GOALS.md), [COMPATIBILITY.md](COMPATIBILITY.md) |
| later | Stability — of loft, and of the libraries and projects built on it | see below |
| later | Games that feel made by a person, not by an AI | see below |

**Stability.** Sources for the language: [STABILITY_ROADMAP.md](STABILITY_ROADMAP.md) (the
tracking view), the `silent-wrong` freeze axis in [.github/LABELS.md](../../.github/LABELS.md),
and the promise in [COMPATIBILITY.md](COMPATIBILITY.md). For the libraries and projects:
[REVALIDATE_LIBS.md](REVALIDATE_LIBS.md) (does a loft change break a shipped library) and the
`lib-main-health` / `consumer-main-health` workflows in `.github/workflows/`.

**Games that feel made by a person.** Candidate sources, for the owner to choose from: dryopea's
design test that *the player cannot lean back* and its design questions settled by measured play
(`dryopea/docs/DESIGN.md`); crawler's rule that content keeps its full design and the engine is
built to realise it (`crawler/CLAUDE.md` § Conventions); moros as the hand-authored pole beside
crawler's generated one; and the `draw` skill's iterative craft ([DRAWING.md](DRAWING.md)) against
one-shot generation.

**Post 2 — games, libraries and language features.** The thread is the dogfood loop (CLAUDE.md
§ Dogfood loop): a real consumer, the lessons it yields, the language fixed, the result shipped.
Told through its cycles:

- game → library: a library that began as one game's code and now serves several — `fixstep`
  from dryopea's timers, `text2d` from dryopea's text, `drawing` from crawler's sprite renderer,
  the `hex_*` family from hexbody.
- game → language: each consumer's queue of defects it hit (`dryopea/QUESTIONS_FOR_LOFT.md`,
  `crawler/LOFT-HANDOFF.md`, `routing/docs/loft-feedback.md`); the `hit-by:<project>` labels
  on loft-lang/loft say who found what, and the silent wrong answers are the stories.
- the two-agent split: the language's agent builds and fixes, each consumer's agent uses and
  breaks, and neither edits the other's tree.

**Post 3 — loft's goals.** [GOALS.md](GOALS.md) is the source: the purpose, goals A–G, and *"the
destination is BORING"* — a tool noticed only when it is missing. [COMPATIBILITY.md](COMPATIBILITY.md)
carries the promise behind it: after the freeze, a working program keeps working.

## The subjects, by how unusual they are

### 1. A language its builder cannot remember building

The language is built almost entirely by AI agents that start every session without memory,
steered by one owner; the repo is the memory.

- Evidence: [AGENT_ACCOUNT.md](AGENT_ACCOUNT.md) — the agent's own account, written after
  reading the owner's side of the earlier sessions; [BUS_FACTOR.md](BUS_FACTOR.md); the skills
  in `.claude/skills/`.
- Measure: the co-author share is `git log --format=%b | grep -c 'Co-Authored-By: Claude'`
  against `git rev-list --count HEAD`, on a FULL clone (a shallow one undercounts).
- Caveat: the claim is about where knowledge lives, not that the owner did little — the
  one-way-door rulings are the owner's (DESIGN_DECISIONS.md).

### 2. Tests that have to prove they can fail

A guard records whether it fails on the build it was written to catch; a bug that answers
wrong silently outranks a crash; each fix says whether the rules already knew the answer.

- Evidence: [GUARDS.md](GUARDS.md) (`make falsify`, the `@falsified-at:` receipt);
  [.github/LABELS.md § silent-wrong](../../.github/LABELS.md); [BUG_REVIEW.md](BUG_REVIEW.md)
  (the `Contract: settled|strained` trailer, the keystone it routes to).
- Measure: `grep -l '@falsified-at:' tests/scripts/*.loft | wc -l`; `make bug-review`.
- Precedent: mutation testing asks the same question of a whole suite; the difference is the
  per-guard receipt against the defect's own control commit.

### 3. The rules lead, the code follows

A formal rule register, cited from every enforcing code site, with each chapter's open
deviations driven to zero — *"the rules do not change to match the code"*.

- Evidence: [formal/README.md](formal/README.md); one chapter read with its `-history.md`.
- Measure: `python3 scripts/rule_tags.py coverage` and `registers`.
- The best story is the counter-example the docs keep: *"OPEN: 0 is a claim to re-measure"*
  (CLAUDE.md § Debugging policy).

### 4. The heap is a typed database

Memory lives in stores whose internal links are record ids, so one store can be mapped from
disk, read over HTTP Range from a static host, or moved to another process without
translating a pointer. Keyed collections are part of the type, and subscripts are queries.

- Evidence: [DATABASE.md](DATABASE.md), [REMOTE_STORES.md](REMOTE_STORES.md),
  [LAZY_STORES.md](LAZY_STORES.md), [PLACEMENT.md](PLACEMENT.md), LOFT.md § Composite types.
- Ancestors to name: MUMPS globals, Smalltalk images, the AS/400 single-level store
  (COMPATIBILITY.md borrows its promise).
- Caveat: the durable tiers shipped and the ones planned are in DATABASE.md; a stored layout
  must match the reader's exactly.

### 5. A fault is a null, not an exception

Overflow, division by zero and an out-of-bounds read give null and the program continues; a
fallible conversion must say what failure becomes.

- Evidence: LOFT.md § Null representation; [formal/types.md](formal/types.md) (N-Index, N-Cast).
- Caveat: [CONTROL.md](CONTROL.md) names null erasing its origin as the largest loss of
  control; [formal/operational.md](formal/operational.md) lists where the runtime still traps.

### 6. Speed measured against a Rust twin

Every library routine is timed against a same-output reference in Rust; a routine over the bar
is work for the compiler, never a hand-written native export. Each Rust stand-in in the engine
is registered with its removal trigger.

- Evidence: [LIBRARY_AUTHORING.md § The library contract](LIBRARY_AUTHORING.md),
  [PERF_PORTAL.md](PERF_PORTAL.md), [KERNELS.md](KERNELS.md).
- Measure: `make perf-portal`; the interpreter against CPython is
  [PERFORMANCE-history.md § Interpreter vs Python](PERFORMANCE-history.md).
- Caveat, and it belongs in the post: the interpreter is slower than CPython. The record shows
  an earlier "beats CPython" claim that had measured the native backend, and was withdrawn.

### 7. Holding a language against OCaml and Lua

A bar turns another language's idioms into probes that either run on both backends or are
refused by name; re-measuring one finds defects, and finds the docs' own stale limitations.

- Evidence: [bars/README.md](bars/README.md), [bars/OCAML_BAR.md](bars/OCAML_BAR.md) § Evaluation,
  [bars/LUA_BAR.md](bars/LUA_BAR.md).
- Measure: run the probes as OCAML_BAR § Working rules says; quote the score with its commit.

### 8. Designing a game by measuring it — dryopea

Design questions are answered by replaying scripted `.keys` scenarios, often before any code:
what a wall in front of a tower is worth (`@M094`), whether the landing site is a real choice
or the dice decide (`@M091`), whether a base's layout matters only where something shoots
(`@M093`).

- Evidence: `jjstwerff/dryopea` — `docs/DECISIONS.md` (each `@M`/`@X` row), `docs/STATUS.md`,
  `docs/EXPLORATION.md`.
- Measure: `scripts/validate.sh` in that repo plays every scenario and gates its readings.
- Caveat: `@X355` reads `@M094` as a BARRIER result — no tested base has used walls as a
  FUNNEL — so the wall post states which arrangement was measured.

### 9. Exact hex geometry, at the fourth attempt — hexbody

Hex shapes are drawn onto an exact integer field and rebuilt from it with no epsilon; houses
and walls become bodies whose collision proxies are derived from their geometry. Earlier
Python, C++ and Rust versions came first.

- Evidence: `loft-lang/hexbody` — `ROUNDTRIP.md` (the constraints with their trust tiers;
  `X70` is *an opening is never "no wall"*), `SPEC.md`; `loft-libs-world/hex_field/README.md`.
- Measure: `make test` in hexbody — every gate carries a control that must fire.

### 10. Fifteen keys — crawler

A roguelike whose simulation is derived as deep as it likes while the player's interface stays
inside a fixed budget of keys: *depth in the derivation, shallow at the interface*.

- Evidence: `jjstwerff/crawler` — `DESIGN.md` § 3a (pillar 0, the count command beside it).
- Measure: that command, on the day; the budget and the count differ by design and on a
  schedule, which the post says.

### 11. A route planner with no server — routing

A map-matching kernel compiled to the browser, reading binary stores by ranged GET from static
hosting — subject 4 in a shipped app, including the path back from a server-first version.

- Evidence: `jjstwerff/routing` — `PLAN-PERF.md`, `browser/`, `docs/loft-feedback.md`.

### 12. Consumers that break the language on purpose

Each game has its own agent that uses loft adversarially and files what breaks; the language's
agent never edits a consumer's tree.

- Evidence: CLAUDE.md § Dogfood loop; the consumers' queues — `dryopea/QUESTIONS_FOR_LOFT.md`,
  `crawler/LOFT-HANDOFF.md`, `routing/docs/loft-feedback.md`.
- Measure: the `hit-by:<project>` labels on loft-lang/loft issues give who found what.
- The vivid cases are the silent ones: pick from the `silent-wrong` issues a consumer filed.

## Supporting subjects

The three backends from one source (NATIVE.md), freeing at scope end with no collector and no
user-facing borrow checker ([OWNERSHIP_MODEL.md](OWNERSHIP_MODEL.md)), `par` on a `for` loop
([THREADING.md](THREADING.md)), the brand layering of loft and lavition
([LAVITION.md](LAVITION.md)). Each has close precedent, so each is a section of a post rather
than a post of its own.
