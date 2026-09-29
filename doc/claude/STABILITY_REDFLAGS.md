<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# STABILITY_REDFLAGS.md — the re-derived facts a stable future must compute once

> **This is a forward-stability map, not a fix-now list.** It names the
> implementations that will keep manufacturing bugs, grouped by the **missing
> fact** each one re-derives — so the path to a stable future is "land the fact
> once; N forests collapse together," not "patch N sites." Companion to
> [STABILITY_HOTSPOTS.md](STABILITY_HOTSPOTS.md) (the H-register) and
> `OWNERSHIP_MODEL.md` (the ownership holes table); this doc is the cross-cutting
> *red-flag* view those two are read through. This map is scoped to
> runtime/memory/codegen; the FRONT-END rough spots it structurally misses — the
> typing/conversion relation (where #432/#433 live) and grammar precedence — are in
> [FORMALIZATION.md](FORMALIZATION.md) (the formal-definition-as-lens companion).
>
> **Two referenced docs — `OWNERSHIP_MODEL.md` and `CODEGEN_METHOD.md` — are the
> ownership-migration docs from the active store-lifetime plan (`@PLN85`) and are
> not yet merged into this tree; they are referenced by name (no local link) until
> they land here.**

## The one thesis

Per `CODEGEN_METHOD.md`: **codegen re-deriving a non-local fact
(ownership, transfer/borrow, null-encoding, container layout) per call-site is a
diagnostic of a missing type-system fact — fix the fact, not the generator.** A
red flag here is therefore not "ugly code"; it is a *fact computed in the wrong
place / N times*. The test for every entry below is the method's own:

> **Would this code collapse to a simple read if the type carried one more fact?**
> If yes → the fact belongs on the type, and every re-derivation site is a sibling
> of one bug class.

**The goal this doc serves is generalization, not tidiness.** Every duplicated case
analysis is a place a situation can be missed, and the misses are not distributed
evenly: they land on the case the newest duplicate forgot. So the unit of work is
never "patch the arm that was wrong" — it is "make this the only place the cases are
enumerated, and make the enumeration exhaustive so the compiler names the next
missing case instead of a user finding it." Two shapes recur:

- **one fact, N spellings** — the same question answered by several predicates at
  different depths (Cluster F), or the same per-kind table re-encoded per consumer
  (Clusters C, D);
- **a total walk written as a partial one** — a traversal that must reach every
  node, spelled with a wildcard, so the nodes it forgets fail silently
  (Cluster F's second half, loft#815).

The counter-test matters as much as the rule: a walker that is *deliberately*
partial is correct and must stay. What is wrong is that deliberate and accidental
are spelled the same way, so nothing can tell them apart.

**The deliverable is the collapsed structure, not a list of the bugs it was
hiding.** A repro in this doc is evidence that a duplicate is load-bearing — it is
not a ticket, and enumerating the remaining symptoms is not progress. This follows
the [STABILITY_ROADMAP](STABILITY_ROADMAP.md) standing rule (*in stability work,
bugs get FIXED, not filed*) and states its reason: the bugs that matter most here
are the ones nobody has hit yet, and those have no ticket to file. Collapsing the
duplicate retires the discovered and the undiscovered cases together, which is
precisely what patching the arm that was wrong does not do. So the measure of
progress is **how many places can still miss a case**, not how many issues are
open.

The rigor failure modes ([engineering-rigor skill](../../.claude/skills/engineering-rigor/SKILL.md)):
**under-reach** (N mechanisms for one family — a spray), **over-reach** (one
mechanism forced over N families — a false invariant), **wrong-signal** (a decision
keyed on a type where the authority is a runtime delta), and
**defensive-at-manifestation** (guard the symptom, leave the root).

**Survey provenance:** four parallel read-only rigor-audits, 2026-06-21, against the
`../loft` `plan-85-store-lifetime-retirement` tree; line numbers below are grounded
in **this** tree (loft2, `tuxedo-work2`) where confirmed, else cited by function.
The *patterns* are durable; re-confirm exact lines before acting.

**Re-survey 2026-08-20** (loft2, `tuxedo-post-973`): clusters A–E re-measured by
counting their re-derivation sites, and one new cluster added (**F**). What each
count is and how to re-run it is in [§ Re-survey](#re-survey-2026-08-20--what-moved)
below.

---

## The map — six clusters, ~4 missing facts

| Cluster | Missing fact | Re-derivation sites | Failure mode | Tracking |
|---|---|---|---|---|
| **A** Return/bind ownership | return-dep: owned/transferred vs borrows-{source} | ~10 | under-reach + wrong-signal + divergence | OWNERSHIP_MODEL 99/102/103/104 (mostly OPEN) |
| **B** Stack-balance signal | net stack delta (not the last-expr type) | 2 (+1 fixed) | wrong-signal | NEW (gen_if `null_else` already fixed; siblings open) |
| **C** Container taxonomy | a `for_each_owned_child` traversal keystone + per-kind descriptor | ~27 + ~20 | under-reach (per-kind spray) | **NEW** (H7-sibling on the heap-cascade side) |
| **D** Null-sentinel codec | the `sentinel(tp)` encode/decode table (H6) | ~3 sites × 3 | under-reach (one fact, drifting copies) | H6 (consumers unconverged) |
| **E** Manifestation guards | — (downstream of A) | 4 | defensive-at-manifestation | #405 / @P290 / @P317 / @P377 |
| **F** Type-variable fact | *does this type still mention a type variable?* — one recursive question over `Type` | 3 spellings / 14 sites | under-reach + divergence | **NEW** (the loft#1014/#1016/#1020/#1023/#1025/#1028 family) |

Clusters **B/E are mostly *consequences*** of A and C: a missing ownership fact is
why the stack and the free-path get guarded at the symptom. Landing **A** (return
ownership) and **C** (the traversal keystone) is what retires the bulk — D and E
largely dissolve behind them.

### Re-survey 2026-08-20 — what moved

Each row is a count anyone can re-run, not a judgement. The commands are the ones
that produced the numbers.

| Cluster | 2026-06-21 | 2026-08-20 | Evidence |
|---|---|---|---|
| **A** Return/bind ownership | ~10 sites; `has_ref_params` at 11 | **largely landed** — `has_ref_params` at **2** | `grep -rc has_ref_params src` |
| **B** Stack-balance signal | 2 open siblings | **unchanged — deliberately deferred**, no RED probe fires | `state/codegen.rs` arm-join + `stack.rs` `size_code` (below) |
| **C** Container taxonomy | ~27 + ~20, no keystone | **keystone landed**, 24 references; one holdout | `grep -rn for_each_owned_child src` |
| **D** Null-sentinel codec | 3 consumers × 3 widths | **largely landed** — `is_null_field` gone, width fact now `IntegerSpec::range_to_width` | `grep -rn range_to_width src/data.rs` |
| **E** Manifestation guards | 4 | not re-measured (gated on A) | — |
| **F** Type-variable fact | — | **new, 3 spellings / 14 sites** | § Cluster F |

Two clusters therefore came off the critical path, and the landing order below is
re-cut around that. The instrument that found **F** — group every `match` block by
the enum it dispatches on, then rank enums by how many independent blocks re-match
their arm set — is worth re-running on any enum that grows a variant.

---

## The evidence — what the tracker says (2026-08-20)

Everything above this section is argued from reading code. This section is the
independent check: the same question asked of the 334 closed `bug` issues, which
know nothing about the clusters. It is here because a map of "brittle places" is a
claim about where bugs COME FROM, and that claim is measurable.

### Method (re-runnable)

```bash
gh issue list --state all --limit 1200 --json number,title,labels,state,closedAt
```

Keep the `bug`-labelled issues, classify each by keyword against its TITLE (these
titles state mechanisms, which is what makes this work), and bucket by ISSUE NUMBER,
not close date. The tracker opened around #134 in June 2026 and 282 of 513 closes
land in August, so calendar recency cannot separate "still firing" from "closed
during the August push" — issue number can.

**Bounds on the claim, so it is not over-read:** the classes are keyword-matched and
OVERLAP (one issue can count in several); issue number is a proxy for time; and a
rising share can reflect where dogfooding went rather than where the code is weak.
That last confound matters and is picked up under *Exposure* below.

### Result 1 — the classes that got a single-fact home are the classes that stopped

Share of bugs in each issue-number band:

| class | #100–400 | #400–700 | #700–900 | #900–1030 | |
|---|---|---|---|---|---|
| **narrow-int / width** | 8.7 % | 7.7 % | 7.4 % | **2.0 %** | ↓ after `IntegerSpec::range_to_width` (Cluster D) — a real before-population, so this one is measurable |
| **keyed collections** | 0 % | 4.6 % | 14.0 % | **7.8 %** | appeared only AFTER `for_each_owned_child`; no before-population, so no payoff can be claimed |
| **tuple** | 0 % | 0 % | 9.9 % | **9.8 %** | ↑ no keystone |
| **generic / monomorph** | 0 % | 0 % | 1.7 % | **4.9 %** | ↑ no keystone |
| **null / sentinel** | 13.0 % | 6.2 % | 5.0 % | **17.6 %** | ↑ |
| ownership / free | 8.7 % | 12.3 % | 14.9 % | 13.7 % | flat (Cluster A substrate, still open) |

**One of these is a measured payoff and one is not, and the difference matters.**
`narrow-int/width` had a real bug population BEFORE `IntegerSpec::range_to_width`
landed (9.6 % across the earlier bands) and 2.0 % after — a fall with something to
fall from. `keyed collections` shows 0 % before `for_each_owned_child` and ~10 %
after, so the class only came into existence once those collections were exercised:
the fold cannot be credited with a reduction there, and `scripts/bug-review.py`
deliberately ABSTAINS on it rather than printing a verdict the data does not carry.
An earlier draft of this section claimed both as payoffs; that was reading a
within-period dip as causation.

So the honest form of the claim is narrower and still worth having: **where a class
had a bug population and then got a single-fact home, the population fell by roughly
two thirds.** One instance is not a law, and the protocol in
[BUG_REVIEW.md](BUG_REVIEW.md) exists to accumulate more of them.

The classes rising instead are tuple, generic and null, which is
[Cluster F](#cluster-f--is-this-type-still-a-type-variable-new-2026-08-20) arrived at
from the other direction. Nothing here was tuned to produce that agreement: the
clusters came from reading `match` blocks, the shares from reading issue titles.

This is the argument for the whole approach, stated as a measurement rather than a
principle: **collapsing a duplicated case analysis moves the bug rate, and patching
arms does not.**

### Result 2 — a hypothesis this data KILLED

The obvious reading of 22 tuple bugs is *"`Tuple` is the variant enumerations
forget."* It is wrong, and being wrong is the useful part. Omission rate of each
child-bearing variant across the 115 partial (wildcard, non-delegating) recursive
`Value` walkers:

```
Block   9.6 %   Insert 14.8 %   Call 23.5 %   If 26.1 %   Return 27.8 %
──────────────────────────── cliff ────────────────────────────
Iter   65.2 %   CallRef 67.8 %   BreakWith 68.7 %   Yield  71.3 %
Tuple  72.2 %   Parallel 74.8 %  TuplePut  79.1 %   ParFor 87.0 %
```

`Tuple` is not special. It is ORDINARY for the tail. Coverage falls off a cliff
after the five shapes an author holds in mind while writing a walker, and everything
past that cliff is omitted at roughly the same rate. So the defect is not a variant
anyone dislikes — it is that a hand-written enumeration reproduces its author's
attention, and attention has a short tail.

### Result 3 — exposure, and the prediction it makes

If omission rate were the whole story, `ParFor` (87 % omitted) would carry more bugs
than `Tuple` (72 %). It carries almost none:

| variant | omitted | bugs filed |
|---|---|---|
| `Tuple` | 72 % | **22** |
| `Yield` (coroutines) | 71 % | 9 |
| `Parallel` / `ParFor` | 75–87 % | 6 |
| `TuplePut` | 79 % | 4 |

**Exposure = omission rate × usage.** The tail is not safe; it is unexercised.
`par` appears in 12 `tests/scripts/` files against tuple's 51, and the tuple bug
share was itself 0 % before issue #700 — tuples did not become fragile, they became
USED. This is also where the dogfooding confound from the Method note lands, and it
does not weaken the reading: exposure IS the mechanism.

The falsifiable prediction: **`Parallel`, `ParFor`, `Yield` and `TuplePut` are the
next `Tuple`.** When a consumer leans on `par` or coroutines the way one leaned on
tuples, expect a bug wave of the same shape and roughly the same size. If that wave
does not arrive after real `par` dogfooding, this model is wrong and should be
rewritten.

It also settles how to respond. Adding a `Tuple` arm to N walkers treats one point
of a distribution and leaves the rest of the tail loaded. The cure is the one the
tree already has: a walker that must be total delegates recursion to the keystone,
so the tail is never enumerated by hand at all.

### Result 4 — the fifteen fixes on `tuxedo-post-973`, read against this map

The map above was written on 2026-08-20 from `main`. The branch that closes every open
issue landed the same week, so it is the first chance to ask the map's own question of a
batch of fixes: **did they fold facts, or patch arms?** Counts are `git show origin/main:…`
against the branch.

| fact | sites on `main` | sites on the branch |
|---|---|---|
| "is this slot text?" — coroutine emitter | 15 hand-rolled `Type::Text(_)` | **1 predicate**, 15 callers (`is_text_slot`) |
| "is this tuple element text?" | 3 hand-rolled, two of them disagreeing | **1 pair** (`tuple_elem_type` / `_is_text`), 6 callers |
| "is this narrow slot null?" — renderer | 8 hand-rolled sentinel tests | **1 home** (`Stores::is_null`), 9 callers |
| "what offset does this narrow slot use?" | declared `min` at 4 registration sites | **1 home** (`IntegerSpec::part_min`) |
| "how wide is this integer?" | 2 homes (`byte_width` / `forced_size`) | **1 home**; `vector_narrow_width` now derives from it |

Every one is the *one fact, N spellings* shape — and in four of the five the spellings had
already drifted far enough to produce the bug that was filed. The fifth (#1040) is the
thesis's other half, a fact computed in the wrong PLACE: the par route was picked in the
template, where the types it is picked from do not exist yet, so it now waits for the
monomorph. None of the five was patched arm-by-arm.

**The prediction in Result 3 is not yet tested, and this batch is not the test.** It says
`Parallel`, `ParFor`, `Yield` and `TuplePut` are the next `Tuple` *when a consumer leans on
them*. Three of those four variants appear in these fixes (#1040 ParFor, #1035 Yield, #1038
TuplePut) — but every one carries `hit-by:loft`: they were surfaced by loft's own generics
work, not by a consumer. So they are consistent with the exposure model without confirming
it: the same mechanism (a shape gets exercised in new combinations, and the tail's omissions
surface) driven from inside rather than from a dogfood repo. The prediction stands, still
waiting on real `par` / coroutine dogfooding.

What the batch DOES move is the exposure itself, which is the model's own remedy — the tail
is less unexercised than it was:

| variant | test files on `main` | on the branch |
|---|---|---|
| `par(…)` | 18 | **20** |
| `yield` | 19 | **21** |
| tuple declarations | 51 | **53** |

Fifteen new `tests/scripts/` guards, each run on both backends. That is a 10 % lift on the
two thinnest surfaces, which is small — but it is the direction the model says matters more
than adding arms to walkers.

**One prediction this batch does support.** Result 1 credits `IntegerSpec::range_to_width`
with the only measured payoff (narrow-int/width fell by roughly two thirds). `make
bug-review` re-run today still reads `PAID OFF` (9.5 % → 3.3 %) — and the four narrow-int
bugs in this batch (#1030, #1031, #1036, #1037) are all *residual second homes of that same
keystone*, not new mechanisms: a compound assignment, a field-vs-local disagreement, a
vector element's stride, and a bound the spec could not carry. A keystone that pays off
does not finish the class; it converts the class into a finite list of sites that still ask
the question somewhere else. That is a useful refinement of the payoff claim, and it is
falsifiable: if the next narrow-int bug is NOT a second home of `byte_width`, this reading
is wrong.

---

## Cluster A — the return/bind transfer-vs-borrow fact (cluster-II root)

**The fact:** every heap value's binding should read one carried answer — *does this
value OWN its store (move it), or BORROW another's (copy on bind/escape)?* Today
that answer is re-derived from accreting heuristics (`has_ref_params`, `is_argument`,
`vector_bound`, `dep.is_empty()`, runtime store-nr witnesses) at **~10 independent
sites**, which is why the same class produced #405/#406/#409/#410 this cycle and why
the `a = x.v` field-read aliasing was still open after the `return v` fix.

Re-derivation sites (loft2 tree):

| Site | loft2 location | Smell | Mode |
|---|---|---|---|
| BlockTail/MidReturn/native-forwarder return thicket | `src/parser/control.rs` (`block_result`, `ref_return`) | per-callee-shape adopt/copy/borrow arms + a `RetSite` fork | under-reach |
| `returned_var` single-`u16` walk | `src/scopes.rs:2285` | differing match/if arms collapse to "no return var" → arm buffers freed | **over-reach** (false invariant: ≤1 return source) |
| return-save-to-temp forest (B5-L3) | `src/scopes.rs` (`free_vars`) | ~5 type-keyed save-temp shapes each deciding "hoist before frees?" | under-reach |
| `has_ref_params` adopt-vs-copy | `src/` — **11 sites** (`grep has_ref_params`), incl. `state/codegen.rs` ×2 + `scopes.rs` | a heuristic standing in for "transfer or borrow?"; admits "cannot resolve statically" → runtime `OpFreeRefIfDistinct` | wrong-signal |
| `is_borrowed_view` — computed **twice, divergently** | `src/state/codegen.rs:1716,2441` (interp) **vs** `src/generation/dispatch.rs:178` (native) | the same `0x8000` source-free bit, two different structural derivations → drift on the OOB/hidden-only edge | **divergence** (H4) |
| bind-site owned-vs-borrow | `src/parser/expressions.rs` (`assign`) | per-RHS-shape copy decision (bare Var / borrowing Var / field-read / non-Var) | under-reach |
| reassign free-strategy forest | `src/state/codegen.rs` (`gen_set_first_at_tos` region) | `is_hidden_buf_arg && owned_ref && rhs_reads_v && …` — the canonical `has_ref_params && …` forest | under-reach |
| scan_set paired-witness | `src/scopes.rs` (`scan_set`) | emits a **runtime** store-nr comparison because the static fact is absent | re-derived → runtime |

**`dep.is_empty()` also HIDES the class from one backend** (measured, loft#882). Empty
deps mean OWNED to `--native`'s assignment lowering, so a read whose dep is *missing*
(not absent-because-owned) gets a defensive `OpDatabase` + `OpCopyRecord` and the program
comes out right; the interpreter aliases and reads freed bytes. The keyed-element borrow
scored `--interpret` 6/17 boundary cells against `--native` 14/17 **on the same IR**. Two
consequences for anyone working this cluster:

- a store-lifetime matrix that is lopsided between backends is evidence of a missing dep,
  not of a codegen bug — read the emitted Rust and look for an `OpCopyRecord` beside a
  plain element read;
- a `--native` PASS is not evidence the dep is present, so every ownership probe has to
  run `--interpret` under `LOFT_POISON=1` as well. The defensive copy is also why this
  class survives a green suite: it is a silent perf cost on one backend and a
  use-after-free on the other.

**Twins drift exactly on the input nothing tests.** `State::append_copy` and
`codegen_runtime::OpAppendCopy` disagreed on a NEGATIVE count (`--native` clamped, the
interpreter cast `as u32` and walked off the store until glibc aborted) and on whether to
re-read the backing record after a resize. A twin gets hardened where its bug was
*observed*, and an observation happens on one backend. When touching one twin, diff it
against the other line by line, and land every new boundary row in the shared `.loft`
guard so both backends run it — a "keep the two in step" comment is not a gate.

**Two owners of one store, and only one of them knows.** A `0x8000` source-free bit means
"nobody else owns this store" — a fact about the *source expression*, decided in the parser.
`scan_args` then lifts an inline call result into a `__lift_N` the scope sweep frees, which
makes the bit false without telling anyone. `free_named` is a no-op only while the slot is
still free, so the second free is silent until the allocator hands that slot to somebody
else — and a record return allocates its buffer in exactly that window (loft#890). Two rules
follow:

- **the FREE hand-off and the DROP hand-off are separate facts** and must be recorded
  separately. @PLN139 stage C recorded only the drop, reasoning that "the free is left to
  the ordinary sweep, which is null-tolerant either way"; a recycled slot is what makes that
  untrue;
- **`skip_free` is not a free-only switch.** Both backends read it at ALLOCATION time too
  (`is_inline_ref || is_skip_free` in `state/codegen.rs`), so stamping it to suppress a free
  made the lift BORROW instead of own, and the append wrote into its own source — 3 of 54
  fuzz cells SIGSEGV'd. A "somebody else already freed this" note belongs in the pass that
  emits the free, not on the variable.

**A verdict that MINTS must be pass-stable, and skipping pass 1 is not "no verdict".** It is
the opposite verdict: `ref_return` reads a binding with no dep as OWNED and renames it onto
the return buffer, permanently. Pass 2 then sees the borrow and materialises — into the
buffer the binding now IS — so `materialize_return_into` emits `OpDatabase(e);
OpCopyRecord(e, e)` and the function answers the record it just re-minted (loft#889;
`e = make()[k] ?? d; e` had it from loft#882 onward). A parse-time site that names an owner
has to name it on BOTH passes; the way to keep the numbering stable is a separate counter
(`__ref_p2_N`), not a pass guard.

**Would-one-fact-collapse-it?** Yes — all of the above are the OWNERSHIP_MODEL
remedy verbatim: *return-dep empty ⇒ adopt; `{Attr(src)}` ⇒ copy* (row 102), plus
the *return-source SET over arms* (row 99) and *one funnelled return path* (row
104). Landing the carried return-ownership dep collapses these ~10 sites and
deletes the runtime witnesses. **This is the single highest-leverage migration in
the tree** (most-reused decision; the documented cluster-II root).

---

## Cluster B — stack-balance keyed on type, not runtime delta

**The fact:** "did this branch leave a value on the eval stack, and how many bytes?"
is a **runtime stack-position delta**, never the last-expression *type*
(`generate_block` reports the last expr's type, not the net push — a value-typed
but stack-neutral tail op like `OpAppendVector` pushes nothing).

- ✅ **Fixed precedent:** `gen_if`'s `null_else` gate now reads `true_stack !=
  stack_pos` (the #405 fix). It is the template for the rest.
- 🔴 **`gen_if` B5 rebalance** — `src/state/codegen.rs:1072` (re-confirmed
  2026-08-20, unchanged): uses
  `size(def(self).returned())` (the *function's* return type) as the bytes-to-
  preserve, not `size(tp)` (the if-**expression's** result type). Wrong whenever a
  non-tail value-`if`/`match` with eval-stack-divergent arms has result type ≠ the
  function return. **Masked** today because its only natural trigger is recursive
  functions where `tp == returned()`; latent, not dead.
- 🟡 **`size_code` If/divergent arm** — `src/stack.rs:113` (`size_code`,
  re-confirmed 2026-08-20, unchanged — the arm reads `self.size_code(node.if_then())`
  and the function ends `_ => 0`): an `if`'s
  drop-size is read from the then-arm's static type; a divergent then-arm falls to
  `0` while the else-arm pushed a value. Same wrong-signal family; uncommon shape.

**Cleared (right signal, not red flags):** fn-ref `step(20)`/`step(16)` and callee
`size(op.returned())` advances — there the type *selects the opcode that pushes
exactly those bytes*, so the type IS the authority.

---

## Cluster C — the per-Type-kind container taxonomy (the heap-cascade keystone)

**The fact:** "to traverse / construct / free a collection's owned nested heap —
which element type, at which stride, with which container walk" is ONE per-`Parts`
descriptor. Today it is hand-re-encoded across the family below — the **densest bug
cluster in the tree** (the @P29x/@P3xx history). <!--noindex-->

| Family | loft2 location | Count | Named bugs from the drift |
|---|---|---|---|
| `copy_claims` / `remove_claims` / `validate_claims` triad | `src/database/allocation.rs:1374 / 1682 / 984` (+ `copy_claims_{seq_vector,array,hash,index}_body` at `:1119/1163/1233/1303`) | 3 dispatchers × ~9 `Parts::` arms ≈ **27**, already drifting (copy has 4 helpers; remove/validate don't) | @P290 (SIGSEGV, `room*2` vs `(room-1)*2`), @P306/@P318 (hash slot-drift), @P309 (missing length header) |
| `record_new` / `record_finish` / `insert_record` construction | `src/database/structures.rs` | 3 fns × ~8 arms; **3 independent encodings** of "element-record word count" | @P309 class |
| `gen_set_first_*_null` codegen family | `src/state/codegen.rs:1091/1143/1303/1378/1390` + the multi-arm `gen_set_first_at_tos` ladder | 5 fns + ladder + 3× copy-pasted `sentinel \| owned-init \| borrowed-view` tri-state | #260/#330 class (wrong null-init → leak / use-before-init) |
| keyed `Type::{Sorted,Hash,Index,Spacial} → database.{kind}` re-dispatch | `src/state/codegen.rs` + `src/parser/vectors.rs` + `src/generation/dispatch.rs` (+more) | same 4-arm block in **≥4 files**, interp/native already shaped differently | interp/native drift (H4) |
| `Stores::{vector,hash,sorted,index,spacial,child_rec}` constructors | `src/database/types.rs` | 6–7 near-identical intern-or-push bodies + **3× key-resolution drift** (`hash` resolves vs `key_owner`, `sorted`/`spacial` vs raw content) | latent @PLN25 nullable-key bug |

**Would-one-fact-collapse-it?** Yes — a single `for_each_owned_child(tp, rec) ->
Iterator<(child, child_tp)>` keystone (the per-`Parts` walk as a carried fact), with
copy/free/validate/construct as thin visitors over it. This is the `for_each_child`
keystone **H7** already names for *codecs* — here on the heap-*cascade* side, and
**not yet a tracked H-row** (H3's pass-2 explicitly scoped itself to `scopes.rs`
free-*placement*, away from this traversal cascade). **Highest-leverage NEW finding.**

**LANDED (re-survey 2026-08-20).** `Stores::for_each_owned_child`
(`src/database/allocation.rs:254`) exists and carries the walk, with 24 references
across `allocation.rs` and `database/spans.rs`. `copy_claims` and its per-kind
bodies, `remove_claims_mode`, and the span walk are visitors over it. Its own
history already shows the keystone earning its keep: `allocation.rs:6364` records a
real fault from it having had no `Trie` arm — one arm, one place, instead of the
same omission repeated per consumer.

**The holdout — `validate_claims`.** `allocation.rs:2116` still hand-rolls its own
ladder: six `Parts` arms (`Base` at `tp == 5`, `Struct`/`EnumValue`, `Vector`/
`Sorted`, `ChildRec`, `Enum`, `Hash`) and then `_ => {}`. `copy_claims` knows
thirteen kinds, so the validator silently walks past **`Array`, `Index`, `Ordered`,
`Radix`, `Trie`**.

Its *divergence* from the keystone is deliberate, documented at `allocation.rs:96`,
and already SETTLED: a 2026-06-22 design probe falsified the wider fold and the
decision is recorded in
[STABILITY_ROADMAP § Red-flag remediation](STABILITY_ROADMAP.md) row C —
`validate_claims` runs on suspected-corrupt heaps and must bounds-check before
following a pointer, where the keystone trusts it. That reasoning holds, it is not
the red flag, and this section does not re-open it. The `_ => {}` is: a validator that answers "no
problems" for a container kind it never looked at cannot tell a clean heap from an
unwalked one, and `store_verify` (`allocation.rs:4640`) is one of its callers. The
fix is not to make it a visitor — it is to make the arm set exhaustive, so a new
`Parts` kind forces a decision here the way it already does in the keystone. An arm
that deliberately does nothing is fine when it says so.

---

## Cluster D — the null-sentinel codec (H6 consumers, unconverged)

**The fact:** the per-width null encoding (`u8`→255, `u16`→65535, `*Raw`→`i32::MIN`,
…) is **one** `sentinel(tp)` table. It is still hand-inlined at read AND write sites
beyond the two H6 already unified:

- `is_null_field` — `src/database/types.rs` (3 inline narrow arms).
- `set_default_value` — `src/database/structures.rs` (the write twin).
- `walk_parsed_into` (JSON path) — `src/database/structures.rs` (a third copy).

This is H6's thesis exactly ("one width-fact, N drifting copies" — already cost the
`389-h6` nullable-narrow bug). Tracked under **H6**; these specific consumers are the
named-but-unconverted ones. Lands behind the staged `NullEnc`/`sentinel(tp)` table.

**Landed — the heap-ref + character facet (Cluster D D.1/D.2).** The four named
typed-null encoders (`write_typed_null` native, `emit_typed_null` interp,
`STRING_NULL`, `init_ref_sentinel`) are converged:

- **Heap-ref null = one source, `DbRef::NULL` (`keys.rs`).** Every heap-ref null
  encoder (native `write_typed_null` / `default_native_value` / `dispatch` / `calls`
  / `coroutine`, the `codegen_runtime`/`parallel`/`structures` runtime writers, and
  the interp `init_ref_sentinel` / `null_ref_sentinel`) read the single
  `DbRef::NULL` const instead of re-spelling `DbRef { store_nr: u16::MAX, … }`. The
  drift was a mixed `pos: 0` (interp/canonical) vs `pos: 8` (native) literal —
  semantically inert (`is_null()` keys off `store_nr`, ignores `pos`) but real byte
  drift, now gone. Round-trip matrix byte-identical on both backends; regression
  `tests/scripts/407-cluster-d-null-sentinel-roundtrip.loft`.
- **Character null H4 cell, found + closed.** `Parser::null` folded `character`
  into `OpConvIntFromNull` (the i64 integer sentinel), so `-> character { return
  null; }` emitted `return i64::MIN` into an `i32` return slot — native rustc E0308,
  interpreter tolerated. Now routes to `OpConvCharacterFromNull` (char-domain null
  `'\0'`), correct on both backends.

**Deliberately LEFT (distinct representations, not the same fact).** `STRING_NULL`
(a text `Str` sentinel `"\0"`, already a single `const` read by all text-null sites)
and the interp `Reference` path's `database.null()` (which allocates a real null
*store*, a different runtime mechanism than the `DbRef::NULL` sentinel) — forcing
either onto `DbRef::NULL` would be a false merge.

**✅ The narrow-width facet is CLOSED (2026-08-22).** The three named consumers are
converged, and the convergence paid for itself immediately:

- `walk_parsed_into`'s four open-coded narrow arms → `Stores::write_narrow_value`, the
  one place the four encodings live. (Same pass gave the absent-field decision its own
  home, `Stores::write_absent_value`; see QUALITY-history.md § Q1/P54.)
- `is_null`'s three inline arms → `Stores::narrow_is_null`, its read twin.
- `set_default_value_nullable`'s narrow arms stay where they are — they write through the
  ENCODING-aware setters (`set_byte`/`set_short`/`set_i16_raw`/`set_i32_raw`), which is a
  different mechanism from a raw read, so folding them onto the read walk would be the
  same false merge `validate_claims` was ruled out for in Cluster C.

**What the fold found, which is the argument for doing it at all:** `Parts::Int` — the
4-byte width — had **no arm in `is_null`**. So the 2026-08-20 render pass fixed the three
widths that had one and could not fix the one that did not, and nothing said so. An absent
`i32?` was therefore not omitted from a render and went onto the wire as the NUMBER
`-2147483648`, on both backends, while reading the same slot answered null — the exact
symptom that pass was fixing, surviving one width over. A missing arm is invisible to a
per-width audit and obvious the moment the widths share a home.

Verified across both `min` axes (`Byte` at 0 and 10, `Short` at 0 and 1000, `ShortRaw`
elements at 1000) — a sentinel test that compares a DECODED value only holds at `min == 0`,
which is what made the original `limit(10,255)?` cell read `265 == 255`. Guarded by the
extended `tests/scripts/narrow-null-render.loft` on both backends, falsified by removing
the new `Int` arm.

---

## Cluster E — defensive-at-manifestation guards (downstream of A)

These guard the *symptom* of the missing ownership fact rather than preventing the
bad value. They are correct stopgaps; the stable future **retires** them when A lands
(and a `debug_assert` that documents a contract is the GOOD form — keep those).

| Guard | loft2 location | What it hides | Tracking |
|---|---|---|---|
| `free_named` OOB-refuse | `src/database/allocation.rs:191` | a wrong/stale free with an out-of-range `store_nr` — refused (release) instead of not-produced | #405 (cluster II) |
| `free_protected` call-bracket | `src/database/allocation.rs:680` + `lock_store` | "safety net for `is_borrowed_view` mis-detection" (its own @P290 words) — runtime locking to stop the wrong free landing | @P290 (retire when dep inference is complete) |
| `["??"]` one-buffer return marker | `src/generation/mod.rs:714`, `src/generation/dispatch.rs:173` | a placeholder dep standing in for the unresolved return-ownership fact | OWNERSHIP_MODEL `"??"` row |
| `n_protect_store_frees` `rec != 0` guard | `src/native.rs` (per @P377) | a half-state `free_protected` leak across `unlock`/`init` | @P377 |

---

## Cluster F — "is this type still a type variable?" (NEW, 2026-08-20)

**The fact:** *does this type still mention a type variable?* is ONE recursive
question over `Type`. Three predicates answer it today, each seeing a different
depth, and none of them recurses through the keystone that already exists —
`Type::for_each_child` (`src/data.rs:1783`), whose own doc calls it *"the ONE place
that knows which `Type` variants carry child types … exhaustive on purpose — a new
variant forces a decision here and every walker inherits it."*

| Spelling | loft2 location | Depth it sees | Verdict when measured |
|---|---|---|---|
| `is_type_var_placeholder(d_nr)` | `src/data.rs:6309` | none — takes a def-nr, walks no type | the ATOM the other two are built from; correct |
| `is_type_var_operand(tp)` | `src/parser/operators.rs:511` | `tp.base()` — peels `Optional`, stops | **correct by design, not a drift** |
| `type_mentions_tv(t, tv_nr)` | `src/parser/mod.rs:5240` | recursive, ended `_ => false` | **the real duplicate — COLLAPSED** |

⚠ **This table first called all three drifted spellings of one question. Two of those
calls were wrong, and measuring is what showed it.** The correction is kept visible
rather than quietly rewritten, because the mistake is the instructive part: *three
predicates that look alike are not automatically three copies of one fact.* Deciding
by reading them was not enough; each had to be run.

- `type_mentions_tv` WAS a duplicate. `Type::contains_def` already answers the same
  question through `any_node` over the `Type::for_each_child` keystone, and sat two
  hundred lines from a call to it. The hand-rolled version knew Vector, Optional and
  Tuple and answered `_ => false` for `Iterator`, `Function`, `RefVar`, `Rewritten`
  and the keyed collections. Now folded onto `contains_def`, with an eight-shape
  generic-return corpus byte-identical before and after.
- `is_type_var_operand` is **not** one. It asks whether the OPERAND ITSELF is a type
  variable, and a container of `T` is a container whatever `T` turns out to be —
  measured on both backends, `vector<T> == null`, `vector<T>? == null` and
  `T? == null` all answer correctly inside a template today. Deepening it would defer
  decisions that are already decidable. The `Optional` peel is exactly the right depth.
- `is_type_var_placeholder` is the atom the other two are built from, not a walk.

**Why this manufactures bugs.** A template's `T` is an attribute-less placeholder
STRUCT. Any parse-time decision keyed on `τ` — which null sentinel to write, which
null test to emit, which default to construct — picks the REFERENCE answer for it
and the monomorph keeps that already-chosen op after substitution rewrites only the
type. loft#1014, #1016, #1020, #1023, #1025 and #1028 are all that one shape.

The cure applied so far is per-site and correct as far as it goes: the template
stamps a marker (`TV_DEFAULT_BLOCK`, `TV_NULLTEST_EQ`/`_NE`, `TV_NULL_BLOCK`) and
`rewrite_generic_type_defaults` re-asks once `T` is concrete, so there stays exactly
one spelling of *"what is `τ`'s null?"*. But it is one marker per DECISION. The
markers arrived one per issue (`TV_DEFAULT_BLOCK` with #1016, the null-test pair
with #1020, `TV_NULL_BLOCK` with #1028), which is the signature of a fact being
retrofitted rather than carried.

**Reproducing, both backends, refused identically:**

```
fn f<T>(x: T) -> (T?, integer) { (x, 1) }

error: type layout: __nullable<T>::Some: field 'payload' has no position (u16::MAX)
```

Boundary matrix, both backends agreeing on every cell. Two axes are swept: what
HOLDS the `T?` (rows 1–5), and then, once the tuple was implicated, everything about
the tuple that could plausibly matter (rows 6–11).

| shape | `--interpret` / `--native` |
|---|---|
| `-> T?` | ok |
| `-> vector<T?>` | ok |
| `-> (T, integer)` — no `?` | ok |
| `-> (text?, integer)` — concrete nullable, still from a generic | ok |
| `-> (T?, integer)` | **refused** |
| `fn f<T>(x: (T?, integer))` — parameter position | **refused** |
| `-> (integer, T?)` — element position 1 | **refused** |
| `-> (T?, T?)` — two mentions | **refused** |
| `-> (integer, integer, T?)` — arity 3 | **refused** |
| `T` ∈ {text, integer, float, character, boolean, struct, vector} | **refused, all 7** |

**Boundary: `Tuple` containing `Optional(<type variable>)`** — any element position,
any arity, any `T`. Both halves are load-bearing, and rows 3 and 4 are the controls
that prove it: drop the `?` and it passes, make the nullable CONCRETE and it passes.
The type axis is inert because the refusal happens at LAYOUT time, before `T` is
substituted at all.

*What this matrix still holds FIXED* (per CLAUDE.md § Debugging policy — count the
axes you pinned, not the ones you swept; `tuples.md`'s `OPEN: 0` and
`ownership.md`'s D-own-6 are both worked instances of a corpus reading clean because
of an axis nobody moved): the generic has exactly ONE type parameter, the tuple is
never nested inside another tuple, and the first sweep of rows 1–5 used `T = text`
in every cell — rows 10 fixed that particular hole afterwards, and it turned out
inert, but it was a hole when the first four rows were written.

`substitute_type` has both a `Tuple` arm and an `Optional` arm (each retrofitted
after its own bug — read their comments), so the gap is the COMPOSITION, not either
arm.

⚠ **This doc previously said the predicate collapse would fix this. It does not** —
measured identical before and after the collapse. The cause was one layer down and is
now **FIXED**: nothing stopped the synthetic `__nullable<S>` enum being minted for a
template's `T`, which is a `DefType::Struct` from user source with no attributes and so
satisfied every eligibility condition. A bare `-> T?` never reached that path at all —
it stays an `Optional` and substitution answers it per monomorph — so the tuple went
through a different door.

True to the cluster's shape, the eligibility question had **two spellings**:
`nullable_vector_elem` calls itself "the ONE home" for it, while the field-rewrite
sweep asked its own narrower version. Both now read one `synth_nullable_target`.
Verified on both backends across integer, float, struct and vector, each tuple
against its BARE twin — `struct` being the cell that must still get a real synth enum,
so a fix that merely stopped minting would fail it.

**The emitter conflation is now FIXED too, and it was the same red flag twice more.**
`tuple_text_to_string` meant "this slot is an owned `String`", read at four places; three
were *"produce something ownable HERE"* and the literal was the odd one, converting
wherever it sat — including inside a sub-expression merely passing through the element.
The split is positional rather than another flag: *don't borrow here* stays a flag, while
*convert to owned* becomes the job of whoever knows the slot (the element loop, and
`TuplePut`, which had no loop to inherit one). Six references stayed byte-identical.

One site along sat a third instance: `tuple_has_text_leaf` matched `Type::Text` without
peeling `Optional`, so `-> (text?, integer)` was invisible to it — and the return path had
its own inline `any(Text)` copy besides. **Not a generic problem at all**: a plain
`fn ret() -> (text?, integer)` would not compile on `--native`.

**Still open, and it moved sides:** the discharged `a?` in a text tuple now answers
correctly on `--native` and *wrongly on the interpreter*, which gives the one-character
text null sentinel where the bare twin gives the empty text. Both render as nothing, so
printing cannot tell them apart — it took `len()` and `== ""` to see it, and an earlier
read of this matrix looked clean because of that. The backends disagree, which by
[CODEGEN_METHOD](CODEGEN_METHOD.md)'s rule is itself the bug.

### The residual — CLOSED 2026-08-20, from the other side

It is fixed, and how says something about the method rather than the bug.

The symptom was a discharged `a?` in a TEXT tuple answering the one-character text null
sentinel on the interpreter where the bare twin answers the empty text — invisible to
printing, since both render as nothing, and only separable with `len()` / `== ""`.

Four readings were falsified by measurement before stopping: the tuple element aliasing
its source, the `?` discharge itself, `ref_return` promotion (`LOFT_TRACE_RETPROMO`
showed neither function reaching the classifier), and the `text["a"]` element dep. What
survived a three-way IR comparison was *"no block-result dep + a `__ref_1` retbuf + an
element that is not a place"*, with the note that `__ref_1` comes from the `__retbuf`
machinery — Cluster A's territory.

**That was the right machinery.** loft#1026 closed it from the return-lowering side, and
its account matches the surviving reading almost word for word: *"`parse_block` has two
mutually exclusive text-return promotions … the monomorph promoter was replicating half
of it."* Verified here afterwards on all four carriers (`t.0`, element 1, `TuplePut`,
and the bare twin) on both backends.

Two things worth keeping from it:

- **Stopping without a fix still moved the work forward.** The investigation did not
  produce a patch; it produced a boundary and four dead ends, and the fix that landed
  came from a session working the same machinery from the opposite direction. Recording
  a falsified hypothesis is cheaper than re-falsifying it.
- **A residual is a claim to re-measure, like an `OPEN: 0`.** This one was closed by
  somebody else's commit while the doc still called it open, and only re-running the
  repro before filing an issue caught that.

## Landing order (leverage-first — the stable-future roadmap)

> **Site-level steps + per-step verification gates for each cluster below:**
> [STABILITY_REDFLAG_REMEDIATION.md](STABILITY_REDFLAG_REMEDIATION.md) — the
> actionable *how* to this map's *what*.

Re-cut 2026-08-20, after A and D largely landed and C's keystone shipped.

1. **Cluster F — `Type::mentions_type_var` on the `Type::for_each_child`
   keystone**, with all three spellings collapsed onto it. Now the most-reused
   under-generalized decision in the tree, and the only cluster with a live
   both-backend repro and a six-issue history behind it. Small: one predicate plus
   its call sites. Step 2 (the `rewrite_generic_type_defaults` extraction/recursion
   split) is independent and equally small.
2. **Cluster A — carried return/bind ownership dep** (OWNERSHIP_MODEL 99/102/104).
   Still the deepest fact, and `has_ref_params` is down from 11 sites to 2, so the
   remaining work is the carried dep itself rather than the forest around it.
   **Dissolves Cluster E** and the `is_borrowed_view` divergence.
   *Prereq already laid:* typed `Deps` (H2 / DEPS_INVENTORY.md).
3. **Cluster C's holdout — make `validate_claims`'s arm set exhaustive.** XS, and
   it closes a diagnostic that currently reports clean on five container kinds it
   never walks. Keep its deliberate divergence from the keystone; delete only the
   silent `_ => {}`.
4. **Cluster B — apply the `true_stack`-delta template to the `gen_if` arm-join +
   `size_code`.** Small, bounded, unchanged since the first survey; the #405 fix is
   the worked precedent. Both sites are re-confirmed present, but the ROADMAP row
   deferred this cluster deliberately ("unverifiable, no RED probe fires; latent —
   pick up only on a real trigger"), and the re-survey found no such trigger. Listed
   here so the sites stay findable, not to re-open the decision.
5. **Cluster D — converge the remaining per-width consumers** onto
   `IntegerSpec::range_to_width`, now that the width fact has a home. S-sized; any
   gap.

Each is a *single fact computed once*, validated on **both backends** per
CODEGEN_METHOD — not a patch. The win is structural: a new collection kind / return
shape / narrow width then arrives *with its fact*, not as the next special case.

### Result 5 — a rising class the mechanism buckets cannot see (2026-08-20)

`make bug-review` classifies by SUBSYSTEM — `generic/monomorph`, `null/sentinel`, `tuple`,
`keyed collections`. That is the right cut for finding which machinery keeps producing
bugs, and Results 1–4 lean on it. But it cannot see a class defined by the INVARIANT it
violates, because each instance files under whichever subsystem it landed in, and no
single bucket rises far enough to flag.

One such class is measurable and rising: **declaration order changes what a program
means.** LOFT_DECLARATIONS.md § File structure states the contract — *"A loft file may contain (in any
order)"* its declarations — and the two-pass parser exists to make it true.

| tracker band | order-dependent bugs | share |
|---|---|---|
| #246–445 | 0 | 0.0 % |
| #445–644 | 0 | 0.0 % |
| #644–843 | 4 | 3.3 % |
| #843–1046 | 8 | 5.3 % |

Monotonic, and the same profile as `generic/monomorph`, the steepest riser `bug-review`
names. The members are spread across SIX subsystems — enum (#803), tuple (#944, #960),
struct (#986), par (#988), closures (#686), packages (#788), generics (#1023, #1024) —
which is exactly why no bucket flags it. Prior instances were closed one at a time as
subsystem bugs (#374, #803), and the class kept producing.

**The remedy the model asks for is an instrument, not an arm.** The backend axis has one
(`tests/differential_oracle.rs`: same program, two backends, same answer). The order axis
had nothing, so an **order-permutation oracle** was written to the same shape: permute a
file's top-level declarations and the program must behave identically. Run against the
`tests/scripts/` corpus it immediately produced three live defects on `main`:

| defect | status |
|---|---|
| `match` on an enum declared BELOW its use returned `Void` on pass 1, so a bound local locked to void and pass 2 was refused | **fixed** — `parse_match`'s `!valid_enum` exit now defers |
| a struct-enum VARIANT literal above its enum — the pass-1 stub was never adopted by the variant registration | **fixed** (loft#1046) — variant registration adopts the stub, as `parse_struct` already did |
| a ONE-CHARACTER type name (`enum D`) — the forward-reference path guesses "is this a type?" from the SPELLING, and a name with no lowercase letter kept a placeholder VARIABLE that shadowed the later declaration | **fixed** (loft#1047) — a qualifier (`D.N`) settles it without the spelling test |

All three are the same fault under the thesis: **a fact decided on pass 1 from a guess,
where pass 2 holds the answer.** That is the "one fact, N homes" thesis on its other axis
— not one fact spelled in several places, but one fact decided at the wrong TIME. Cluster
F (*"is this type still a type variable?"*) is the same question asked about a different
fact, which is why loft#1047's cure lands next door to it.

**How the class actually closed (2026-08-21, same day).** The three defects above were not
the end of it. The `all` → `any` widening merged to `main` as #1050 and **retro-broke the
published `markdown` 0.2.0** — `line[start..ln]` where `start = hlevel() + 1` now correctly
defers on pass 1, and `parse_text_index` refused an `unknown` index outright. Two more
sites, both in that one function, four lines apart: the START bound and the range END. Same
cure, the same first-pass escape. Four known sites in total (`call_op`, `parse_match`'s
`!valid_enum` exit, and those two).

Three things from that are worth more than the fix:

- **A green `make ci` does not clear a language-semantics change.** Nothing local compiles
  the published registry libraries. The gate that caught it is
  `.github/workflows/revalidate-libs.yml`, which runs on `pull_request`, on `push` to
  `main`, and nightly — so `main` sat observably red for hours, with the failing job named
  `markdown 0.2.0`. After landing an inference/coercion/refusal change, read
  `gh run list --workflow=revalidate-libs.yml`; do not read a green local gate as clearance.
- **The first regression guard for it was INERT and passed.** Every callee in the file sat
  ABOVE `main`, so nothing deferred anywhere: it asserted in six places that it tested
  forward declarations and tested none. It was caught only because the two deferrals were
  disabled SEPARATELY — start-only, then end-only — and each was required to fail the guard
  on its own. Aggregate green would have hidden it, which is the same lesson as
  [absent warning is not a pass], reached from the test side.
- **A behavioural sweep for a fifth site came back clean — and was wrong.** 29 probes, one
  per operation kind, re-run across return types after rounds 1–2 had pinned them to
  `integer`. No additional sites found. It was careful work and it still missed one,
  because a probe sweep can only test shapes someone thinks to write.

- **The enumeration found the fifth site the sweep could not.** Instead of writing probes,
  list every refusal in `src/parser/` phrased as a type REQUIREMENT and check which are
  ungated on pass 1: 82 diagnostics render a type (via `.name(…)` and — the ones a first
  regex misses — via `.show(…)`), plus 84 phrased as a requirement whether they render one
  or not. 61 are already `!first_pass`-gated, 6 test `is_unknown`, and the residue is
  small enough to read. Every candidate was then confirmed BEHAVIOURALLY with a negative
  control proving the diagnostic fires at all, which is what separates "clean" from "never
  reached": the `filter` check needed a named fn rather than a lambda before its path was
  even entered.

  The survivor was **`xs[(0, 0)..: lim()]`** — a spatial slice's count limit, refused as
  "spatial slice limit must be an integer" when `lim` is declared lower. Fixed here, same
  cure, guarded on both backends by `tests/scripts/forward-spatial-slice-limit.loft`.
  Nobody writes a spatial slice when guessing at shapes, which is exactly why the sweep
  missed it and the list did not.

**Method, generalised:** for a class defined by "an invariant is violated at site X", a
probe sweep samples the shapes you can imagine, and an enumeration of the CODE that can
violate it does not. Run the enumeration; use probes to confirm each candidate rather than
to find them.

**What is still open:** the 61 diagnostics classified as already-gated were checked by a
45-line context window, not read individually — a diagnostic gated by a guard further away
is misclassified safely (more candidates to review), but one whose `first_pass` mention is
unrelated would be missed. Reading those 61 is the remaining completeness task.

**Falsifiable.** If the next order-dependence bug is NOT a pass-1 decision that pass 2
could have made, this reading is wrong. And the oracle itself is not yet a gate: its
top-level splitter is line-based, so it skips what it cannot split safely and its diffs
still need triage by hand. Promoting it to CI needs a real parser-backed split — worth
doing, since the three defects above came from its first two hundred files.

The residual first left open here — `enum T`, which collides with the type-variable
placeholder the stdlib registers for `min_of<T>` — was **fixed too** (loft#1049), and the
way it closed is the more useful record. It was filed as a design call, "reserve the name"
versus "let a user declaration shadow it". `formal/interfaces.md` had already settled it:
a type variable is *"a name bound by a generic header"*, so the binding is per-header and
reserving the spelling globally was never admissible. The rules do not change to match the
code — and here they turned a two-way choice into a one-way fix before any deliberation.

Its own lesson is about *counting the homes*. Three separate lookups had to agree (the
known-name branch, the forward-reference stub registration, and `parse_constant_value`),
and fixing two of them changed NOTHING observable — the symptom stayed byte-identical,
which reads exactly like "wrong hypothesis" and is really "right hypothesis, one home
short". The third was the one producing the misleading message, by consuming the `.` of
`T.N` while chasing a variant that does not exist.

A second lesson, this one about instruments: the `code!` harness parses its snippet AS
source 0, so a fix gated on the current source number was **inert under the tests** while
working through the CLI. Gating on `!self.default` fixed that. When a guard must
distinguish "the stdlib" from "user code", the source number is not the thing to ask.

---

## What is NEW vs already-tracked

- **NEW (file a forward home / H-row when picked up):** **Cluster F** — the
  type-variable fact and the partial marker-rewrite walk (2026-08-20 re-survey),
  carrying the one reproducing defect in this doc (`-> (T?, integer)`); and the
  Cluster-B arm-join / `size_code` wrong-signal siblings, still open.
- **LANDED since the first survey:** the Cluster-C `for_each_owned_child` keystone
  (shipped, 24 references — `validate_claims` is the one holdout), Cluster A's
  `has_ref_params` forest (11 sites → 2), and Cluster D's width fact
  (`IntegerSpec::range_to_width`; `is_null_field` gone).
- **Already tracked (this doc is the cross-cut, not a new filing):** Cluster A →
  OWNERSHIP_MODEL holes 99/102/103/104 + H3; the `is_borrowed_view` divergence → H4
  + DEPS_INVENTORY H2; Cluster D → H6; Cluster E → #405/@P290/@P317/@P377.

No bugs filed: open items map to existing OWNERSHIP_MODEL/H rows; the genuinely-new
ones are structural-debt forward risks (this doc), not `main`-reproducing defects.
