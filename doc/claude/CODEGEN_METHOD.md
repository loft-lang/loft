<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The codegen method — types drive obvious bytecode, built bottom-up by scale

> This method is now the self-contained **`loft-codegen` skill**
> (`.claude/skills/loft-codegen/`) — the EMIT-mode sibling of the
> `engineering-rigor` (diagnosis) and `design-protocol` (design) skills. The skill
> is what *triggers* the method when you start compiler work and enforces its one
> gate (prove the working bytecode on both backends before editing the compiler);
> this doc is the full reference it routes to.

This is how loft's compiler work should be done, everywhere. It exists because the
opposite — patching the generator with on-the-spot heuristics and seeing what
breaks — produces unreliable code and endless regressions (see @PLN85's 9 reverted
attempts before this method was adopted). The whole of loft is to be moved onto
this method, one plan at a time.

## The principle

> **Code-gen is the wrong place for the complexity of RE-DERIVING facts. Codegen
> reads the parse-tree together with the types — and those two, in combination,
> should INDICATE what to emit. Complex *deduction* in codegen (re-computing a
> non-local fact the types should already carry) is a DIAGNOSTIC of a type-system
> flaw; fix the type, not the codegen.**

Codegen still has logic — it is *allowed* to. It walks the parse-tree, combines
its shape with the type facts, and emits ops. That local translation work is
fine and expected. What is NOT fine is codegen *re-deriving a fact* — recomputing,
per call site, something global like ownership, nullability, or layout that the
type system should have stated once.

The diagnosis:

- **Symptom:** codegen that accumulates conditions to *deduce* a non-local fact
  on the spot (e.g. "callee has ref params AND result is X AND not a borrowed view
  AND …" to figure out *ownership*). The branchiness is the recomputation, not the
  translation.
- **Diagnosis:** the type system is missing that fact, so codegen re-derives it
  per site (and gets it wrong on the shapes it didn't enumerate).
- **Remedy:** compute the fact ONCE, carry it on the type, and let codegen *read*
  it. The generator's logic stays — it just consults a fact instead of
  reconstructing one.

### Corollary: when the fact is WRONG (not just missing), fix its PRODUCER

The same principle has a sharper form. A carried fact can be present but **lie** — a
dep/type that says one thing while the runtime store says another. The lie surfaces
downstream at the **consumer** (a free → use-after-free; a guard → a noisy refusal),
and that is the tempting place to start. Don't. Every per-site condition there
*compensates* for an upstream lie and stacks more conditions. **Go to where the fact
is PRODUCED and make it true** — then the consumer-side compensation is *deletable*,
not patched. Worked example: #457's vector adopt — the return dep said "`out` borrows
`__vdb_1`" while `out` held the adopted `__ref_N`. A free-side thicket (pairing /
strip / explicit-free) grew for a whole session and never closed; the fix was to
**deliver `out` into the buffer at the return** so the dep is true by construction,
which deleted the thicket (`scopes.rs` back to baseline). Two early signals you took
the wrong start: complexity is *growing* per shape, and you can already *name* the
fact as the root ("X lies; patching here can't reconcile it") — that sentence is the
instruction to leave the consumer. Often the tangle is N callers working around one
**unsafe shared primitive** (#457: `clear; append` assuming non-aliasing); making the
primitive safe collapses it.

### The balance (do NOT over-correct)

Pushing *everything* into the type system is the opposite error. The type must NOT
encode a one-to-one map to codegen output — that overburdens the type system and
merely relocates all the complexity into it. The split is:

- **Types carry the non-local FACTS** — properties that need global reasoning or
  recur across many sites (ownership/transfer, nullability, layout, capture).
  Computed once, verifiable.
- **Codegen carries the LOCAL TRANSLATION** — given the parse-tree node plus those
  facts, choose and emit the ops. Logic here is fine; it is reading facts, not
  rebuilding them.

The test for "is this complexity in the right place?": *would this code be a simple
read if the type carried one more fact?* If yes → the fact belongs in the type. If
the code is genuinely just translating a clear (tree + facts) signal into ops →
it belongs in codegen, leave it there.

Why this is the reliability lever: re-derivation is where bugs hide and regressions
breed — every new shape needs another condition, and the conditions interact
combinatorially. Facts-in-types + translation-in-codegen is verifiable (the type
carries the intent), stable (a new shape arrives with the right fact, not a new
deduction branch), and the same on both backends (they translate the same facts).

## The companion principle — build the instrument before the fix

The principle above says where a fact should *live*. This one says how you *obtain*
the information in the first place, before you know what to write.

> **Most of engineering is information. Before writing a fix, build the instrument
> that tells you WHERE the problem is — one more precise than an oracle — then use
> it to find the real cases and code paths. Only then do you know what to write.**

An **oracle** gives one bit about a case you already built: pass or fail. Its reach
stops at what you thought to write down. An **instrument** reports *where*, per site,
across code nobody wrote a case for.

This matters most in codegen, because the generators are exactly where an oracle goes
blind: a check that reads the IR cannot see a fact the emitters invent later. Writing
more test cases against such a check never finds the gap — it is blind to a whole
class, not to particular inputs.

### When to reach for one — and when not to

**Building an instrument is not a flex.** It is not evidence of rigor and not a way to
show the work is being taken seriously. It is a purchase: someone pays to build it,
calibrate it, extend it to every path, and maintain it afterwards. Reach for one
deliberately, on a trigger — never as a default move or a display.

**Two triggers earn it:**

1. **An oracle failed.** Not "a test went red" — that is the oracle working. This is the
   check itself returning a confident answer that was WRONG, or one that cannot see the
   class by construction. When the thing you would normally trust is the thing that
   misled you, more cases against it buy nothing.
2. **A case you assumed was solved turns up broken.** The belief is the finding. One
   such case means your *information* about the area is wrong — not that one input is
   wrong — and every other conclusion resting on that belief is now unsupported. This is
   the stronger of the two signals and the easier one to explain away.

A weaker third: you cannot yet write the fix because the scope is genuinely unpinned,
and the class is broad, recurring, or hides where an oracle cannot look.

**Do not reach for one when:**

- **The scope and root cause are already pinned.** Then there is nothing to find. Fix it,
  with a regression test.
- **It is one shape, one input, already understood.** That is a test case. An instrument
  built for it is overhead with no payer.
- **You want to demonstrate care.** An unjustified instrument is a permanent tax charged
  to every later reader, in exchange for a feeling.

Five rules, in order:

1. **Put the instrument where the fact is CREATED, not where it is consumed.** If
   copies are minted during emission, record them in the branch that emits them —
   that is the one place that cannot be wrong about whether a copy exists.
2. **Calibrate on cases whose answer you already know, in BOTH directions.** An
   instrument that has never been made to fire, *and* to stay quiet, on understood
   cases is an unread dial. Mis-installation is normal and code-reading does not
   catch it: a dead code path can read exactly like the live one.
3. **Survey the whole corpus — turn a bug into a distribution.** One repro tells you
   a shape exists. A sweep tells you how often, and where the cost actually is,
   which is rarely where the first report landed.
4. **Write the probe that could refute you, and keep it when it does.** A probe that
   agrees teaches almost nothing. A probe that measures the opposite of your
   hypothesis is what moves the cause to the right place.
5. **Let the instrument name the GATE on the fix, not just the fix.** Knowing the
   cause is not permission to change it. An instrument that only ever argues *for*
   your change is a worse instrument.
6. **State the instrument's own coverage — a partial instrument that reports zero is
   the oracle again.** Honing one to every case is real, ongoing work, and until it
   is done "found nothing" means *"nothing on the paths I watch"*, never *"nothing"*.
   Write down which paths those are, next to the readings.

**The cost is real — this is an investment, not a free move.** Building an instrument,
calibrating it, and extending it to every path takes effort that a one-line fix does
not, and that effort is only repaid when the class is broad, keeps recurring, or hides
where an oracle cannot look. For a bug whose scope and root cause are already pinned,
skip all of this and fix it. Reach for an instrument when you cannot yet write the fix
because you do not know what you are dealing with.

Worked example, with the full evidence for each rule:
[`plans/130-element-view-invalidation/`](plans/130-element-view-invalidation/README.md)
§ Method — where an oracle reported *"none — every structure copy is a move, a literal,
or already borrowed"* on a program that provably deep-copies, and the instrument that
replaced it found the real distribution (plus two mis-installations of itself).

**Both triggers fired there**, which is why it earned the cost. The oracle failed — the
copy report did not merely stay quiet, it asserted a clean bill of health. And the area
was *assumed solved*: the copy diagnostics had shipped, the buckets were classified, the
Avoidable set was being managed — while `exists()` copied a whole record on every call,
in every compile, unreported. The second signal is the one that would have been easiest
to wave away as a one-off.

## The method — bytecode → types → code, per scale

For each feature/fix, work the **smallest** case first, in three layers, then grow:

1. **Bytecode first (the target, proven).**
   - Write the minimal case. Capture the **WORKING** bytecode *beside* the
     **current/broken** generated bytecode for the *same situation*. The diff is
     the exact spec — pure observation, no compiler change yet.
   - Get the working bytecode from a hand-correct source shape (or hand-written),
     so the target is a real, runnable artifact, never a guess. **Prototype it
     standalone and prove it correct on BOTH backends before touching the
     compiler.**
2. **Types next (make the target OBVIOUS).**
   - Define the type-level signal that makes the working bytecode the obvious
     output: what must the type carry (ownership, transfer/borrow, nullability,
     layout, …) so codegen reads one clear thing and emits the target mechanically?
   - If the current types already carry it → no type change, codegen just reads it.
     If they don't → **that gap is the type change**, and it lands before the
     codegen change. (A signal that's only derivable by a pile of conditions at the
     call site is the symptom that it should be a type, computed once.)
3. **Code last (translation — read facts, don't rebuild them).**
   - Implement the codegen that reads the type facts together with the parse-tree
     node and emits the working bytecode. Translation logic is fine; the red flag
     is codegen *re-deriving* a non-local fact — that fact belongs back in step 2.

Then **validate at this scale on both backends** (correct result · clean exit · no
leak · suite) and **grow**: minimal → bigger examples → full functions, comparing
working-vs-broken bytecode at each rung. A rung is closed only when its bytecode
matches the working form and the gates pass on both backends.

## The refactor mode — behaviour-preserving (flip the gate to before/after)

The three layers above are the BUG-FIX mode: the reference is the *working*
bytecode, and the diff against the *broken* output is the spec. A large share of
codegen work, though, is the opposite intent — RESHAPING emitting code that should
emit exactly the same ops: extract a selector, fold a special case into a cell of a
general path, delete a redundant condition, route several sites through one
dispatch. (The whole @PLN85 ownership *simplification* is a string of these.) Here
the instrument is unchanged — `loft introspect` on both backends — but the
reference flips: **the gate is a byte-identical before/after diff, and an empty diff
IS the proof you changed nothing emitted.**

1. **Build a one-function-per-path corpus first.** One `.loft` file, one small
   function per code path the change touches (each emit/delivery branch), plus a
   `main` that runs them all so the file is also a clean end-to-end run. The corpus
   is what makes the diff *exercise* the branches you restructured — a path the
   corpus misses cannot be caught if the refactor breaks it. Keep it under the
   plan's [bytecode-comparisons/](plans/85-store-lifetime-retirement/bytecode-comparisons/).
2. **Capture BEFORE.** `loft introspect corpus.loft > before.txt`. One capture
   covers both backends — the dump carries the IR + bytecode AND the generated
   native Rust. Confirm the corpus runs clean on `--interpret` and `--native`
   (`LOFT_STORES=warn` / `LOFT_NATIVE_LEAK_CHECK`) so the baseline is leak-free.

   ⚠ **First, prove the capture is deterministic** — run it TWICE on the same
   binary and diff. A file that differs from ITSELF makes this whole gate read
   "everything changed", and it fails in the safe-looking direction only by luck.
   Compilation is not yet fully reproducible (loft#750): 23 of 599 scripts differed
   from themselves before the emitter half was fixed, 3 still do. Discard
   self-differing files from the corpus and SAY how many you discarded — a gate that
   silently skips is a gate that passed for the wrong reason.

   The other blind-instrument tell here is the **stdlib path**: a `before` binary
   run from a worktree with `--path <worktree>` embeds that path in every
   `// loft:…` line, so the diff is 1700 lines of nothing. Point both binaries at
   the same `default/` (confirm with `diff -r`) before reading the diff.
3. **Refactor, then prove AFTER == BEFORE.** `loft introspect corpus.loft >
   after.txt; diff before.txt after.txt`. **Empty = you changed nothing emitted, on
   both backends** — exactly the claim of a behaviour-preserving refactor. Re-run
   after `cargo fmt` (it rewrites the file). One commit per sub-step; a non-empty
   diff you did not intend is the bug — bisect by sub-step, do not push through.

This is why a delicate function can be reshaped quickly and safely: "did I change
what's emitted?" becomes a one-line `diff`, not a guess that the suite happens to
cover. It is the dual of the bug-fix gate — *working-vs-broken* becomes
*before-vs-after*.

### When a refactor surfaces a real behaviour change

A latent bug often shows up mid-collapse (a leak the old structure hid, a case two
branches handled inconsistently). Then the two gates run **together**: the
byte-identical corpus proves *"I changed nothing on the paths I did not mean to,"*
and a boundary matrix ([engineering-rigor](../../.claude/skills/engineering-rigor/SKILL.md)
/ CLAUDE.md § matrix-first) proves *"the path I did mean to change is now correct."*
Assert the matrix on **value + length + leak**, both backends — not leak alone: a
delivery that doubles a vector's contents is leak-free yet wrong, and only a
length/value check catches it. Keep the corpus byte-identical for the untouched
paths even as the matrix cells change. The @PLN85 D-own-1 `block_result` collapse is
the worked example — the corpus stayed byte-identical step by step, and the two
leaks it surfaced (#448 c5 and its mirror) were closed under the matrix.

## Diff against the proven sibling — when your new path misbehaves

> This is the codegen instance of a **general** design method — *when the answer
> already exists, capture its artifact and diff* — see the
> [design-protocol skill § the other half](../../.claude/skills/design-protocol/SKILL.md).
> For codegen the artifact is the IR; the principle is identical for serialized
> bytes, store state, or protocol frames.

The bug-fix gate hand-writes the *working* form; the refactor gate diffs *before* vs
*after*. There is a third, frequently the strongest, instrument: **when a new code
path misbehaves and a PROVEN sibling path already does the same operation
correctly, capture both IRs and diff — the residual divergence IS the bug.** You do
not have to invent the correct output or theorise about the cause; a working
artifact already exists, so make your IR equal to it.

The discipline — capture THREE IRs into files (not grep snippets), then two diffs:

1. **BEFORE** — your change OFF (old behaviour). **NOW** — your change ON. **PROVEN**
   — the sibling function that already delivers this operation cleanly (often a
   simpler shape of the same thing: `fn f(b: Box) -> vector { b.rows }` for a
   borrowed vector return). Save all three under the plan's
   [bytecode-comparisons/](plans/85-store-lifetime-retirement/bytecode-comparisons/);
   one `loft introspect` capture carries IR + interp bytecode + native Rust.
2. **BEFORE → NOW** answers *"did my change do what I intended, and only that?"*
3. **NOW → PROVEN** answers *"what is still different = the bug."* Read the divergence
   structurally (buffer var identity, block dep type, delivery shape), not just op-by-op.
4. **The captured diff is ground truth; your mental model is not.** Theories you hold
   without the capture will be wrong, repeatedly — chase the diff, not the theory.

**Worked example — @PLN85 `match_return` interp crash** (captures + finding in
[bytecode-comparisons/match_return-emit/](plans/85-store-lifetime-retirement/bytecode-comparisons/match_return-emit/README.md)).
A new materialise emitted IR that ran clean on native and clean on interp *without*
churn, but corrupted under allocation pressure. BEFORE→NOW confirmed the change was
semantically right (alias → owned copy, return dep dropped `"e"`). NOW→PROVEN
(`field_return` via `copy_borrow_tail_into_retbuf`, which survives churn) exposed the
real divergence: the proven path delivers into a separate canonical `__retbuf` typed
`["__retbuf"]` (the buffer *attr*), while the new path reused the match-field binding
`_mv_items_1` *as* the buffer, typed `["_mv_items_1"]` (a *var* dep the store analysis
does not track as the owned return — so churn reuses its slot). That single diff
retired three theories held WITHOUT a capture — the append tp (both correct for their
own retbuf), a `__vdb`/`OpReplaceVector` wrapper (the proven path has neither), and
`skip_free`. The fix the diff *specifies*: make the delivery `["__retbuf"]`-typed
(route through the proven machinery), not re-derive a near-miss.

## Which PASS owns a copy — the scopes pass cannot make one

A `Value::Set(dest, value)` synthesised in `src/scopes.rs` **binds what it is given**.  The
copy a bind owes is emitted by the PARSER and is already decided by the time the scopes pass
runs: a vector bind mints a backing store and emits `OpReplaceVector`; a record bind emits the
`__disp_N` snapshot, `OpCopyRecord` and the `__hoff_<src>` hand-off flags that suppress the
source's release.

So a cure for an ALIASING bug that writes a new `Set` in the scopes pass aliases.  Measured
twice while closing loft#1612, in both directions:

- writing a `??` chain out per arm (`Set(dest, Var(x))` inside each arm) lost the vector copy —
  `bv = xv ?? yv ?? vmk(9)` then answered `2 2` where the rule gives `1 2`, because `bv`'s type
  already carried a dep on `xv` and nothing copied it;
- sinking a chain BLOCK into an `if` arm lost the record copy — `x: H = if k > 0 { s.h ?? b }
  else { mk(3) }` aliased `b` (`R9|B9`, where the same program on the previous build reads
  `R2|B9`).

**The rule this gives.** Fix an aliasing or copy defect in the parser, or at whatever decides
which SHAPE the parser builds — loft#1612's cure is one line of associativity, after which
every operand is an arm and the parser's own per-arm bind is the copy.  Where a scopes-level
rewrite is unavoidable, keep the parser's `Set` and rewrite only the VALUE inside it, so the
copy machinery around that statement still applies.

**The instrument.** `cargo test --release --test ownership_drop_gate` — its copy census names
the copy each cell owes and is what caught the second case.  A value cell cannot see a release
at all, and `every_cell_releases_each_resource_once` cannot see WHICH record was released
(loft#1617 lives in exactly that blind spot).

## What this rules out

- Heuristic forests at the generation site (the `has_ref_params && … && …` shape).
- "Fix it and see what the suite says" — the working bytecode is proven *first*.
- Per-shape branches that accrete: a new shape should arrive with the right type,
  not a new special case.
- Interp/native divergence shipped as a partial win: a rung is closed only when
  BOTH backends emit the proven bytecode.

## Applying it across plans

Each plan picks a scale/domain and runs the three layers to completion, validated,
before the next scale. Plans reference this doc as their method. The first
canonical application is **@PLN85** (store-lifetime / heap-return ownership): see
[plans/85-store-lifetime-retirement/stage-c-move-convention-design.md](plans/85-store-lifetime-retirement/stage-c-move-convention-design.md)
for the worked example — minimal case, working-vs-broken bytecode
([bytecode-comparisons/](plans/85-store-lifetime-retirement/bytecode-comparisons/)),
the ownership type signal, and the rung ladder up to full functions.

## The shape of a "type signal" (heap-return ownership, as the worked example)

The recurring need is to make ownership a **type fact**, not a codegen guess. For
heap returns (@PLN85): a function's return type should state whether the result is
**owned-and-transferred** (caller adopts) or **borrows** a buffer/arg (caller must
copy / the callee must not free it). With that one bit on the return type, the
caller's codegen is obvious — adopt vs copy — and the callee's is obvious — free
the return or not. Today that bit is re-derived from `has_ref_params` and friends
at each site (the anti-pattern this doc replaces); the fix is to carry it on the
type. The same shape recurs for nullability, vector/keyed layout, and capture
ownership — each is a type fact that should drive codegen mechanically.

---

## See also

- [OWNERSHIP_MODEL.md](OWNERSHIP_MODEL.md) — the north star this method serves: `deps` as loft's borrow checker (Rust as the reference model); the store-lifetime bug class as holes to close
- [COMPILER.md](COMPILER.md) — lexer/parser/two-pass/IR/types/bytecode
- [INTERMEDIATE.md](INTERMEDIATE.md) — Value/Type enums, bytecode ops, State layout
- [SLOTS.md](SLOTS.md) — stack-slot assignment
- [LIFETIME.md](LIFETIME.md) / [DEPS_INVENTORY.md](DEPS_INVENTORY.md) — the dep/ownership model the type signals build on
- [DEBUG.md § Introspection CLI](DEBUG.md) — `loft introspect` to capture bytecode for the working-vs-broken comparison
- First application: [plans/85-store-lifetime-retirement/](plans/85-store-lifetime-retirement/)
- Instrument-before-fix, worked: [plans/130-element-view-invalidation/](plans/130-element-view-invalidation/README.md) § Method — the worked example (CLOSED; the rules above are what it produced)
