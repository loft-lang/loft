---
name: loft-codegen
description: >-
  The discipline for ANY loft compiler/codegen work — fixing a store-lifetime or
  codegen bug, adding/altering bytecode, parser IR, the interpreter, or the native
  generator, or implementing a language feature that emits ops — OR restructuring
  any of those without meaning to change what they emit (extracting a selector,
  folding a special case, deleting a redundant condition). USE THIS the moment you
  are about to edit src/parser/, src/state/codegen.rs, src/fill.rs,
  src/generation/, src/compile.rs, or scopes.rs to change OR reshape what code is
  emitted — ESPECIALLY when a fix looks obvious and you're tempted to "just patch the
  generator and run the suite." The one load-bearing instrument is the same in both
  modes — `loft introspect` on BOTH backends BEFORE you touch the compiler — but the
  gate flips: a BUG FIX proves the WORKING bytecode standalone beside the broken one;
  a behaviour-PRESERVING REFACTOR proves the emitted IR + native Rust BYTE-IDENTICAL
  before/after. Skipping it is how codegen work flails and regresses the suite (see
  the probe-04 anti-example). The DESIGN/diagnosis counterparts are the
  engineering-rigor and design-protocol skills; this is the EMIT-mode sibling. Routes
  to CODEGEN_METHOD.md (the method) and OWNERSHIP_MODEL.md (the ownership beacon).
user-invocable: true
---

# loft-codegen — Prove the Working Bytecode First

## The one gate (do not pass it)

> **Do NOT edit the compiler until you have the WORKING bytecode/IR for THIS
> situation, captured beside the current/broken bytecode, and proven correct on
> BOTH backends (`--interpret` AND `--native`).**

This is the step that, skipped, wrecks codegen work. The proof is cheap (a `/tmp`
source shape + `loft introspect` + a run on each backend); skipping it is what costs
hours. A patch to `src/generation/` or `gen_set_*` written *before* you can point at
the exact ops you intend to emit is a guess — and guesses in codegen regress the
suite, because every new shape needs another condition and the conditions interact.

**The anti-example (internalize it):** @PLN85 probe 05 was fixed correctly — working
bytecode proven standalone (`bytecode-comparisons/` rungs, the hand-correct
`a:vec=[]; a+=enc()` form) → types → codegen → clean on both backends. Probe 04 was
NOT — a parser change went in with no working IR proven first; it regressed
`loft_suite` and never closed the case. Same method, opposite outcomes. The
difference was this gate.

## Zeroth: has the spec already TRIED your fix?

`doc/claude/formal/` is not only the rule book. For the store-lifetime and return-buffer
machinery it is also the **record of attempts** — which fixes were made, which were reverted,
and WHY — so it answers three questions before you write a line, and each one costs a session
if you learn it by hand instead:

1. **Has this exact change been tried and reverted?** Reverted attempts are written up beside
   the rule they failed, with the measurement that killed them.
2. **Where does the spec say the unsound step IS?** It usually names a site, and it is usually
   not the site where the symptom appears.
3. **What guards it now?** Each closure names its `tests/scripts/` cell, so you get a working
   control for free.

Grep the mechanism, not the symptom — the op or the pass (`OpFreeRefIfDistinct`, `work-ref`,
`classify_ret_promotion`), across `formal/*.md`. `IMPLEMENTATIONS.md` is the index of what is
already merged; `ownership.md` holds the rules and `ownership-history.md` the
store-lifetime narrative (what was tried, measured, reverted).

⚠ **The anti-example is loft#1096, and it is recent.** A callee freeing the caller's work-ref
buffer on a null return was traced to `OpFreeRefIfDistinct`, and the obvious repair — skip the
free when the witness is null — was written and measured: use-after-free gone on both backends,
values correct. Then the poison sweep exhausted the store table. `formal/ownership-history.md` had
already recorded that same move: *"removed the wrong answer and left both leaks — a trade, not
a closure"*, **reverted as inert**, because *"a guard that cannot fail proves nothing"*. Two
lines further it names where the fix belongs — *"Closed at the promotion, which is where the
unsound step is"* — and the `tests/scripts/` cell that guards it. Reading it first would have
skipped the whole attempt. This is CLAUDE.md's *"READ THE FORMAL SPEC FIRST when the fix has a
choice in it"* with a price tag on it.

⚠ **And the second half of that lesson, which is the harder one: loft#1096's fix was NOT at
the promotion.** *"Closed at the promotion"* is loft#1081's sentence about loft#1081. Refusing
the rename for loft#1096 was built and measured too, and it over-fires (a `match` with no
catch-all lowers its fallthrough to the same null sentinel, so an ordinary two-variant `match`
loses its NRVO) and needs a second half or the null arm delivers an empty vector instead of the
sentinel. The unsound step was one site over, in `scopes::free_vars`. Read the register for
what was TRIED and what the measurement was; a closure names where ITS unsound step was, and
the next defect in the same machinery has to be measured, not inherited (`ownership-history.md`
D-own-9).

⚠ **Grep the BELIEF, not the op.** loft#1096 and loft#1097 are one wrong sentence — *a
collection slot holds the null sentinel on a path that did not write it* — at two sites, found
a day apart, costing a use-after-free and a silently wrong value. Both sites had written the
belief down in their own comment. When a register entry names a false premise, the next search
is for other places that hold it.

**And a citation gap is a finding, not a dead end.** If the site you are about to edit enforces
a rule and cites none — `src/fill.rs` carries zero `@FR-` tags, so no `rule_tags.py sites` query
reaches the free it performs — say so in the fix. A rule the enforcing site does not name is a
rule the next reader cannot find from the code. (`fill.rs` is `@generated` from the `#rust`
templates in `default/*.loft`, so the citation, like the fix, belongs in the template.)

## The method — bytecode → types → code, smallest scale first

1. **Bytecode first (the target, proven).** Write the minimal case. Capture the
   **WORKING** bytecode/IR *beside* the **current/broken** one for the same
   situation — `loft introspect prog.loft`, or `LOFT_LOG=static`. The diff IS the
   spec. Get the working form from a hand-correct SOURCE shape (or hand-write it),
   so it's a real runnable artifact, never a guess. **Prove it on BOTH backends
   before any compiler edit.** Save the pair (e.g. under a plan's
   `bytecode-comparisons/`).
   - Static dumps don't show runtime identity (stack-ref vs owned heap). When the
     two IRs look identical but behaviour differs, the fault is value-delivery —
     reach for `LOFT_LOG=ref_debug` / `LOFT_TRACE_*`, not another static read.
2. **Types next (make the target OBVIOUS).** What must the type carry (ownership /
   transfer-vs-borrow, nullability, layout) so codegen reads ONE clear fact and
   emits the working bytecode mechanically? If the types already carry it → no type
   change. If not → **that gap is the type change, and it lands before the codegen
   change.**
3. **Code last (translation, not re-derivation).** Codegen reads the type fact +
   the parse-tree node and emits the ops. Local translation logic is fine; the red
   flag is codegen *re-deriving a non-local fact* (the `has_ref_params && … && …`
   shape) — that fact belongs back in step 2.

Then **validate at this scale on both backends** (correct result · clean exit · no
leak via `LOFT_STORES=warn` / `LOFT_NATIVE_LEAK_CHECK` · suite) and **grow**:
minimal → bigger → full functions, comparing working-vs-broken at each rung.

## Mode B — behaviour-preserving refactor (flip the gate to before/after)

Steps 1–3 above are the BUG-FIX mode: the reference is the *working* bytecode, the
diff against *broken* is the spec. When instead you RESHAPE emitting code that
should emit exactly the same ops — extract a selector, fold a special case into a
cell, delete a redundant condition, route two sites through one dispatch — the same
instrument (`loft introspect`) stays, but the reference flips: **the gate is a
byte-identical before/after diff, and an EMPTY diff is the proof.**

1. **Build a one-fn-per-path corpus FIRST.** Write one `.loft` file with one small
   function per code path the change touches (each delivery/emit branch), plus a
   `main` that runs them all so the file is also a clean end-to-end check. This is
   what makes the diff *exercise* the branches you restructured — a corpus that
   misses a path can't catch a regression in it. Save it under the plan's
   `bytecode-comparisons/`.
2. **Capture BEFORE.** `loft introspect corpus.loft > before.txt` (it carries the IR
   + bytecode AND the native Rust, so one capture covers both backends). Confirm the
   corpus runs clean on `--interpret` and `--native` (`LOFT_STORES=warn` /
   `LOFT_NATIVE_LEAK_CHECK`).
3. **Refactor, then prove AFTER == BEFORE.** `loft introspect corpus.loft >
   after.txt; diff before.txt after.txt` — **empty means you changed nothing
   emitted**, on both backends, which is the whole claim of a behaviour-preserving
   refactor. Re-run after `cargo fmt` (it touches the file). One commit per
   sub-step; if the diff is non-empty and you didn't intend it, that line is the
   bug — bisect by sub-step, don't push through. Your corpus proves the branches
   you touched; **`scripts/introspect_diff.sh` is the corpus-WIDE gate** (every
   tests/scripts + tests/docs + examples file, per-file verdict — DEBUG.md
   § Introspection CLI: "verified by this and by nothing weaker"). Run it before
   calling the refactor done.

**When a refactor SURFACES a real behaviour change** (a latent bug shows up mid-
collapse — e.g. a leak the old structure hid): the two gates run TOGETHER. The
byte-identical corpus proves *"I changed nothing on the paths I didn't mean to,"*
and a boundary matrix (the `engineering-rigor` skill) proves *"the path I did mean
to change is now correct"* — value + length + leak on both backends, not leak
alone (a delivery that doubles a vector reads as leak-free; only a length/value
check catches it). Keep the corpus byte-identical for the untouched paths even as
the matrix cells change.

Worked example: the @PLN85 D-own-1 `block_result` collapse —
`doc/claude/plans/85-store-lifetime-retirement/bytecode-comparisons/D-own-1-corpus.loft`
is the corpus; the collapse stayed byte-identical step by step, and the two leaks it
surfaced (c5, h4) were closed under the matrix.

## The diagnostic

Complex *deduction* in codegen (accumulating conditions to recompute ownership /
nullability / layout per site) is a SYMPTOM of a missing type fact — fix the type,
not the generator. But don't over-correct into a 1:1 type→codegen map: **facts in
types, translation in codegen.** Test: *would this be a simple read if the type
carried one more fact?* Yes → the fact belongs in the type.

## The both-backends rule

Interp (`src/state/codegen.rs` → bytecode) and native (`src/generation/` → Rust) are
**separate generators reading the same IR**. A rung is closed ONLY when BOTH emit
the proven bytecode and pass (correct + clean + no leak). An interp fix that breaks
native compile is NOT landable. When they diverge on the same IR (one clean, one
wrong/E0425), that divergence is the bug — neither "the suite is green" nor "interp
works" closes it.

## Before you add the arm: does this predicate already exist?

Emitters accrete duplicates faster than anything else in the tree, because a fix arrives at
ONE site and the sibling that asks the same question is in another file. Measured here:
"is this a keyed collection" was spelled **sixteen** times — a `pub(crate)` helper, a second
private helper with the same variants reordered, and fourteen inline `matches!` copies — so
adding a keyed kind meant finding sixteen places.

So before adding a match arm or a type list, ask where that question already lives:

- `python3 scripts/rule_predicate_audit.py` — the same variant set at 2+ sites
  (`--near` for lists differing by exactly ONE variant, which is drift already present);
- `python3 scripts/rule_tags.py sites <Rule>` / `./scripts/idx tag:@FR-<Rule>` — every site
  that CITES a formal rule, which finds duplicates that share no code;
- `doc/claude/formal/IMPLEMENTATIONS.md` — the checklist of predicates already merged, and
  the pairs deliberately kept apart.

⚠ **Equal today is not the same rule.** The narrow-width family had five sites spelling one
list and only three were the same question — the other two write a RAW slot where the three
write an encoded field, and their own comments said so. Merging on the list alone would have
folded a raw-slot writer onto an encoded-field writer. Read what each site asks; cite the
rule it enforces; leave a note where two look alike and must stay apart.

## And the dual: does this notion have a SECOND spelling?

The section above asks whether your predicate already exists. This asks whether the thing it
matches reaches the IR more than one way — because a matcher keyed on one spelling is blind to
the other, silently, and the blindness cannot be grepped for from the symptom. Searching for the
spelling you DO match returns every site that gets it right; the sites that get it wrong contain
nothing to search for.

Three instances in one week, in three subsystems: a PROJECTION is `Call(OpGetField|OpGetVector,…)`
**and** `Value::TupleGet(base, i)`, which is a variant carrying its base as a var NUMBER and is
not a call at all; a NULL AT A JOIN is a literal lowering to `OpConv*FromNull` **and** a
nullable-TYPED value that carries no null-shaped node; a BORROW is a value with a dep list **and**
one with no dep at all. Each cost a wrong answer that no test could see.

So before writing *"is this an X?"* over the IR, ask whether X has a second spelling — a `Value`
VARIANT beside an op call, a TYPE fact beside a node shape, an absence beside a presence. Match
the notion, not the spelling, and put both in ONE predicate. `python3
scripts/ir_walker_audit.py spellings` asks it for the projection notion (18 functions, 2 handle
both); the mode is ~30 lines and the shape generalises to any notion whose two spellings can be
named. `… optional` asks it one TYPE FORMER over — who resolves a shape by naming `Type`
variants and so cannot see the same shape wrapped in `τ?`, plus which callers peel with
`.base()` first. Re-run either rather than reading a count off this page. Full treatment, with what each instance cost:
`doc/claude/formal/IMPLEMENTATIONS.md` § *One notion, how many SPELLINGS?*

⚠ The normal appearance of this defect is a matcher that is RIGHT about every site it can see.
So the evidence is never a failing site — it is the other spelling, built by hand, arriving where
the matcher is not looking.

## Say why the FALLBACK is right, not just what the function computes

A walker that recurses over `Value` and ends in `_ => false` / `_ => None` is answering a
question ABOUT A SUBTREE. Its fallback is a claim — *"none of the shapes I did not name can
carry this property"* — and a caller that guards on the answer stops guarding when the claim is
wrong.

**Write that claim down in the doc block, beside what the function computes.** Measured over
the walkers audited so far, every one whose doc gives a reason for the fallback was clean, and
the one whose doc explained only the QUESTION carried two shipped bugs — a compound assign that
ran its container call twice, and a hoist that wrote the wrong struct. That one was not
undocumented; it had a careful comment about what a user call is and why a place reaching one
must be bound once, and nothing about `_ => false`.

Good fallback sentences already in the tree, as models:

- *"a cyclic chain has no single borrow base, and every caller handles `None` conservatively"*
- *"an extra marked store can only REFUSE a free, never license one"*
- *"a USER function is not a conflict — it is called with `cell`, not with a live `&mut Stores`"*

If you cannot write the sentence, that is the signal to probe the omitted shapes rather than
ship the arm. `python3 scripts/ir_walker_audit.py reach` lists these walkers, marks the ones
whose fallback answers no, and ranks by production reachability.

## The projection case in full — match the NODE, not the op

The dual above in its measured form: `b.items` and `vv[0]` lower to
`Call(OpGetField|OpGetVector, [base, …])`; `t.0` lowers to `TupleGet(base, i)`, which carries
its base as a var NUMBER and is not a `Call` at all. The two return gates that decide whether a
returned projection must be COPIED into the caller's buffer both matched the call spelling only,
so a tuple-element tail renamed the TUPLE local onto a vector-shaped `__retbuf` — the prologue
cleared a stack tuple slot as a vector, the tail became a discarded statement, the function
returned null, and `--native` would not compile the result. Even the canonical helper cannot
express the other spelling: `use_analysis::is_projection_op(data, d_nr)` takes a def number, and
a projection that is not a call has none.

The variants that carry a var number outside a `Value::Var` node are the ones to check against
any "does this mention / project from variable X?" walker: `TupleGet`, `TuplePut`, `CallRef`,
`FnRef`, `FnRefDnr`, `Set`, `Iter`. `scopes::dominance_walk` names three of them and is the
model; the two Plan-57 gates beside it name none and are clean only because the corpus never
puts a holder there (651 113 arrivals, 2 hits, both a write to the target).

So when adding or auditing such a matcher, ask: **is there a second spelling of this notion?**
If the answer is yes, match the node kind, and put both spellings in one predicate rather than
adding the missing arm at the site that happened to break.

## Stop conditions (revert, don't push through)

- You're editing the compiler but cannot point at the working bytecode you intend to
  emit → STOP, go do step 1.
- A change makes one backend pass and the other regress/hang/crash → revert; it is
  not landable.
- The patch is growing per-shape conditions → the fact belongs in the type (step 2).

## Anchors

- **Keep `git diff main` a usable codegen compass** — ONE branch held close to main, rebased on `origin/main` often (the `engineering-rigor` skill § "Keep `git diff main` usable"); a diverged branch loses the working-vs-broken comparison this method depends on.

- [`doc/claude/formal/`](../../../doc/claude/formal/) — the rules AND the record of attempts:
  `ownership.md` for the store-lifetime rules and `ownership-history.md` for the record
  (which fixes were reverted and why, and where each closure put the unsound step),
  `IMPLEMENTATIONS.md` for what is already merged. Read BEFORE writing a
  store-lifetime or return-buffer fix, not after it fails.
- [CODEGEN_METHOD.md](../../../doc/claude/CODEGEN_METHOD.md) — the full method
- [OWNERSHIP_MODEL.md](../../../doc/claude/OWNERSHIP_MODEL.md) — `deps` as loft's borrow checker (the north star for store-lifetime work)
- Worked example + rungs: `doc/claude/plans/85-store-lifetime-retirement/` (`bytecode-comparisons/`, `type-ownership-design.md`); probe 05 = method followed (clean), probe 04 = method skipped (regressed) — read both
- [DEBUG.md § Introspection CLI](../../../doc/claude/DEBUG.md) — `loft introspect` / `LOFT_LOG`
- DESIGN/diagnosis siblings: the `engineering-rigor` and `design-protocol` skills
