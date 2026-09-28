<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# QUALITY — open quality work, its ratchets and its tools

The quality work that is still open, the derived rows the gates hold, and the instruments that
answer the duplication questions.  Elsewhere:
- the live bug queue is GitHub Issues — `make work` ([ISSUE_TRACKING.md](ISSUE_TRACKING.md));
- the stability tracking view is [STABILITY_ROADMAP.md](STABILITY_ROADMAP.md);
- the method is [STABILITY_METHOD.md § The rule-led walk](STABILITY_METHOD.md);
- declined work is [DESIGN_DECISIONS.md](DESIGN_DECISIONS.md).

The history of this work — every rule walk (B3 through B8r), the JSON sprint, the struct-enum
blockers, the enhancement tiers — is [QUALITY-history.md](QUALITY-history.md).  A section
cited as `QUALITY.md § B6m` or similar lives there.

## Contents
- [Open programmer-biting issues](#open-programmer-biting-issues)
- [Ratchets — the derived rows the gates hold](#ratchets--the-derived-rows-the-gates-hold)
- [The duplication thread — what remains](#the-duplication-thread--what-remains)
- [Instruments](#instruments)
- [Store-lifetime residuals](#store-lifetime-residuals)

---

## Open programmer-biting issues

Programmer-facing defects are GitHub issues.  This table holds only what is open and is not a
defect: a behaviour a program can depend on, waiting for a decision.

| # | Issue | Severity | Status |
|---|-------|----------|--------|
| `#errors` | `record#errors` returns one newline-separated text, and reading it clears it; `for e in r#errors` therefore iterates nothing, and bound to a variable first it iterates CHARACTERS. The docs show the shape that works (read it into a variable, then test it) | Medium | **OPEN — a compatibility decision** ([COMPATIBILITY.md](COMPATIBILITY.md)): keep the text, or make the accessor a `vector<text>` with one entry per error. `tests/scripts/197-struct-json-parse-errors-dest.loft` depends on the text shape and on clear-on-read. Measured: [QUALITY-history.md § JSON cluster](QUALITY-history.md#json-cluster) |

---

## Ratchets — the derived rows the gates hold

Each row below is a count a tool re-derives and a `tests/doc_hygiene.rs` gate compares with
the row.  Read a row as the goal's current bound, never as a figure to quote: the tool is the
answer.  After a join, re-run the tool on the merged tree; never pick a side
([JOINING.md § Resolving a conflict](JOINING.md#resolving-a-conflict)).

### `optional` — who resolves a shape without peeling `τ?`

A function that asks a type's shape without peeling `Optional` answers wrong for a nullable
value (`@FR-N-Shape`).  The ratchet is the count of such functions, and it must not grow.

| opaque to a wrapped shape — must not grow |
|---:|
| **304** |

`python3 scripts/ir_walker_audit.py optional` reports it (the fourth figure);
`make optional-ratchet` fails when it grows; `quality_optional_table_matches_the_audit` holds
this row.  A site that is opaque ON PURPOSE says why at the site.  Origin:
[QUALITY-history.md § B6p](QUALITY-history.md).

### `unspan` — who matches a shape a `Span` can hide

`Value::unspan`'s contract: every second-pass site that pattern-matches a specific `Value`
variant calls `code.unspan()` first, or a per-site `Span` wrap silently disables the
optimisation that relied on the unwrapped shape.

| sites a `Span` hides the shape from — must not grow |
|---:|
| **22** |

`python3 scripts/ir_walker_audit.py unspan` reports it; `quality_unspan_table_matches_the_audit`
holds this row.  Origin: [QUALITY-history.md § B4f](QUALITY-history.md).

### `spellings` — who sees both spellings of a projection

A projection has two IR spellings: the call (`OpGet…`) and `Value::TupleGet`.  A function that
resolves it by op name sees only the first.

| functions ALSO handling the `TupleGet` spelling — must not shrink |
|---:|
| **17** |

`python3 scripts/ir_walker_audit.py spellings` prints three figures — functions resolving a
projection by op name, those also handling `TupleGet`, those seeing only the call spelling.
Quote all three together: a filtered pair has been read as a total.
`quality_spellings_table_matches_the_audit` holds this row.  Origin:
[QUALITY-history.md § B6g](QUALITY-history.md).

---

## The duplication thread — what remains

The premise is an owner diagnosis: most of loft's code came from fixing bugs, and that fixing
wrote duplications without a design — one structure, with several implementations of one
question, often in several files.  The response anchors duplication on the RULES: a formal
rule is what two implementations both claim to implement, so `@FR-` tags, cited at the code
and resolved by `scripts/rule_tags.py`, are where "is this the same question?" is asked.  The
tags, the eight-family checklist in [formal/IMPLEMENTATIONS.md](formal/IMPLEMENTATIONS.md) and
the instruments below are in place; the rule-led walk
([STABILITY_METHOD.md](STABILITY_METHOD.md)) is the standing practice.  `make rule-coverage`
reports the position against the contract-1 floors.  What remains open:

### Families still open

| # | family | next action |
|---|---|---|
| 1 | **scalar** — the bare sites | adopting `is_scalar` adds value enums at each site: a behaviour change per site, one probe each, not a sweep |
| 9 | **what a slice bound means** (`vector` + `text`) | `ops::sub_text` normalises both bounds and is the documented home, but nothing calls it; the two live halves (`State::get_text_sub`, `codegen_runtime::OpGetTextSub`) are fixed and cited.  Delete the orphan, or make the live halves derive from it |

### Rules gaps (spec decisions, not code)

| gap | state |
|---|---|
| **no rule names the keyed family** as a category — `Col-Hash`/`-Sorted`/`-Index`/`-Spatial`/`-Trie` define one kind each, yet sites test the category | `vectors::is_keyed` cites all five as a stand-in; minting a family rule is a spec decision |
| **no rule says a narrow value in a VARIABLE slot is a raw `i64`** — `L-Narrow` states the stored width, `L-Null` the field encoding | the code comments the distinction; the rules cannot express it |

### The owner's calls

| decision | evidence |
|---|---|
| **`Parallel` is the least-exercised construct at IR level** | a corpus census (`ir_walker_audit.py producers`): a coverage gap in the suite, not a defect |

### Process

| item | state |
|---|---|
| `skill-creator`'s description-optimisation loop against `design-protocol` | ☐ not run — whether a trigger fires is the thing being fixed, so it is the part worth measuring |
| the negative-control gate's LEAK channel | ⚠ `falsify.sh` reads "stores not freed" off stderr, which only a `main`-ful `--interpret` run prints, so a `main`-less leak guard scores INERT on both trees.  The warning is in the tool's header; the cure — a leak check on `--tests` — is a decision about every library's `loft test` |

### Carried

[STABILITY_ROADMAP.md](STABILITY_ROADMAP.md) owns these: Plan-53 cluster 2 S4, @PLN130's
uncovered copy sites, gate 4 durability (@PLN43), and H6 `i32::MIN`.

---

## Instruments

Each answers one duplication question and is a REPORT unless named as a gate.  Each was scored
against answers found by hand before it shipped.

| tool | question |
|---|---|
| `scripts/rule_tags.py` — `check` · `sites <tag>` · `dups` · `registers` | does every `@FR-` citation resolve, which sites enforce a rule, is a rule defined twice, does each chapter's `OPEN: n` match its entries — `check` is gated by `every_rule_citation_resolves` |
| `scripts/ir_walker_audit.py walkers` | who hand-rolls `Value`'s tree shape instead of deriving from `Value::for_each_child` |
| `… producers` / `… dead` | which `Value` variants nothing in the corpus can build |
| `… reach` | which catch-all walkers production actually runs |
| `… unspan` · `… spellings` · `… optional` | the three ratchets above |
| `… former <name>` | who can see through a given `Type` wrapper |
| `scripts/matrix_axes.py file <guard>` · `cross <A> <B>` | which composition axes a guard's cells reach, and which value pairs no corpus file crosses |
| `make doc-probes` (`scripts/doc_probe_sweep.sh`) | the hard faults among the executable files under `doc/`; crash channels only, since those files carry no expected values |
| `make falsify GUARD=… REF=…` | does a guard fail on the build it was written to catch — blind on the leak channel as above |

---

## Store-lifetime residuals

| item | home | state |
|---|---|---|
| **@PLN85 cluster I** — FFI struct-return read gap | [@PLN85](plans/85-store-lifetime-retirement/README.md) | latent: a `#native` fn returning a non-vector struct has no `alloc_struct` helper, so the read path cannot be exercised.  Re-probe when the helper lands |
| **@PLN130 Q6** — which uncovered copy families to accept rather than eliminate | [COPY_DIAGNOSTICS.md § What remains open](COPY_DIAGNOSTICS.md) | design question, framing settled: an accept is written at the SITE, never as a blanket exemption.  Which families qualify is not chosen; an unaccepted copy is reported, which is the state the model requires |
| **@PLN130** — the uncovered copy set | [COPY_DIAGNOSTICS.md § What remains open](COPY_DIAGNOSTICS.md) | ranked, with the `Unknown` bucket empty (every emitted copy is attributed to a named emitter); cost not established — a record copy is cheap beside a syscall.  L effort |

---

## See also
- [QUALITY-history.md](QUALITY-history.md) — the record of this work
- [STABILITY_METHOD.md](STABILITY_METHOD.md) — the rule-led walk
- [formal/IMPLEMENTATIONS.md](formal/IMPLEMENTATIONS.md) — the eight-family duplication checklist
- [COPY_DIAGNOSTICS.md](COPY_DIAGNOSTICS.md) — the copy census
