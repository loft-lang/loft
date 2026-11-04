<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# C′4 — the view plan as a front fact

Arc C′'s last item ([README.md § Arc C′](README.md#arc-c--harmonising-the-moves-made)):
`(R-ElemFirst)`'s one form the IR does not take — a temp a function also returns inside its
record — and with it D-phase-4 (`formal/rewrites.md`).  The owner chose this design over
deleting the writers' clause (2026-11-04): make the decision the form needs a front fact, so
the rule lives in one place without losing the form.

## The form

```loft
fn g2(sc: Sc, n: integer) -> Mark {
  g2_p: vector<Pt> = [Pt { px: n, py: 1 }, Pt { px: n + 1, py: 2 }];
  sc.ops += [Op { opts: g2_p, ow: 7, … }];        // built in the element …
  Mark { matched: true, bad: false, mpts: g2_p }  // … and returned: the exit copies it
}
```

`element_first` pairs `g2_p` with its append only when the exit's copy of `g2_p` is no use:
when `g2` is an admitted value record whose `mpts` is a VIEW LEAF (`(O-ViewField)`) — the
returned tuple carries a reference to the place `g2_p` names instead of a copy.  That answer
is the view plan (`ValueRecords::views`).  The cells: `tests/scripts/164-element-place.loft`
g1, g2, g4, g10, g16 (the `parse_circle` shape), g20.

## What was measured

- **The plan is computed three times**, each on the IR it sees: `value_record::rewrite_program`
  in `close_program` (the IR's own value records), rustc's `lower::plan::plan_program`, and the
  C's `FnWriter::value_records` (`c/mod.rs`).  The two writers compute it on the final IR.
- **Its view leaves are keyed by node address** — `ViewPlan::leaves` by `(exit Object block
  address, field offset)`, `FreshQuery` by the exit block and its copy — valid for the IR they
  were read from.  Address keys are the codebase's general idiom (109 uses outside the store
  code): an analysis is read where it was computed.
- **The view leaf is a WRITER clause of `(R-ValueRecord)`.**  The IR pass declines a function
  whose result position holds a view (`value_record.rs`, `body_shape`: "the tuple form has no
  spelling for it here"); the writers alone deliver the reference.  So the view plan is not a
  fact the IR lacks a copy of — it is a second rule left in the writers, the same deviation as
  D-phase-4 one layer down.

## S0 — the premise, measured

A census name counts the writers' view-leaf functions: `R-ValueRecord/view` (both writers,
`lower::plan` and the C's `value_records`).  On the benches: **3 functions** — drawing's
`parse_lock` and `parse_poly` (the parser behind drawing's `parse` routine, over 2× its twin
on both lanes: C ship 2.14×, rustc 2.97×) and two in the moros/dryopea consumer.  No bench
function has an element-first pair that needs the plan: rustc's element-first clause applies
nowhere on the benches.  So the clause carries weight on its own — the value form of a parser's
result — and the element-first form rides on it; S1 comes next, before `(R-LoopRecord)`.

## The invariant

**One value-record decision per function, made in the IR phase and spelled in the IR**, so that
every consumer reads the rewritten IR rather than recomputing the plan: the view leaf is a
tuple element the IR names, and the writers spell it.  With it, `(R-ElemFirst)`'s view form
needs no plan at all: the exit is `Tuple(…, g2_p, …)`, and the element-first rename that makes
`g2_p` the element's field makes that element the view — on every backend.

## The steps

| # | step | done when |
|---|---|---|
| S0 | **Probe the premise** (design-protocol: the cheapest check that could falsify it).  Count, on the benches and the corpus, the functions the writers admit ONLY through a view leaf, and for each pair of `164-element-place.loft` whether its function is one.  If no bench function needs a view leaf, the clause's weight is the cells and the libraries (`make libcatalogue`'s consumers), and that decides S1's priority | **done** — § S0 below the measurements |
| S1 | **A tuple element that is a reference.**  The IR's value-record tuple gains a reference part for a view leaf: the interpreter carries it as a `DbRef` slot, the writers as their reference (`loft_ref` / `DbRef`), never freed by the tuple (a view is never freed).  `body_shape` admits a result position that is a view of a place `(O-ViewField)` admits | the tuple form spells a view on all three backends; the value-record cells pass; a plant (the view freed with the tuple) caught |
| S2 | **The writers read it.**  The view-leaf delivery is the IR's tuple; `lower::plan` and the C's `value_records` drop their view leaves for a function the IR already rewrote; `ViewPlan` then serves only what remains a writer clause, if anything | rustc's and the C's emission byte-identical before and after for every function the IR did not rewrite (`loft-codegen`: a refactor proves emission unchanged) |
| S3 | **`(R-ElemFirst)` takes the form.**  The pass pairs a temp whose exit is a tuple element naming it; the rename makes it the element's field.  Both writers' element-first clauses are deleted | the 164 cells' pairs taken by the IR (`tests/element_first.rs` counts the trace); `rule_tags.py sites @FR-R-ElemFirst` shows no writer clause; D-phase-4 closed |

Each step goes red alone; the checks per step and per chunk are the move protocol's
([README.md § Arc C](README.md#arc-c--moving-them)).

## Risks and open questions

- **A reference in a tuple is new to the interpreter's tuple layout.**  S1 has to show that a
  frame holding such a tuple never frees the view and that a copy OUT of it copies the place —
  the two things a view owes ([formal/binding.md](../../formal/binding.md) `(B-View)`).
- **The writers' other value-record clauses** (the nullable clause, `(R-FnRefValue)`) stay
  writer clauses here.  S2 measures whether `ViewPlan` has any remaining reader; if it does not,
  it goes.
- **Size.**  This is `(R-ValueRecord)`'s view clause moved into the IR, an M-sized arc rather
  than the S-sized item C′4 was listed as.  S0 decides whether it comes before `(R-LoopRecord)`
  and `(R-CompleteWrite)` or after them.
