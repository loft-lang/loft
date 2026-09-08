<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# OWNERSHIP_MODEL.md — loft's ownership/borrow system: the north star

This is the beacon the language steers by. The FORMAL finish line — the five
`D-own-*` deviations in [formal/ownership.md](formal/ownership.md) — is CLOSED on the
shipped path (`OPEN: 0`, 2026-07-04), validated by the @PLN89 differential oracle + the
`program_ownership` fuzzer (validation, not a machine-checked proof). What remains is
not a correctness hole but the *substrate*: the ownership fact is still computed by a
flow-INSENSITIVE classifier (a `Join` over-approximation), which
[@PLN94](plans/94-cfg-ownership-dataflow/) would replace with a flow-sensitive dataflow
fixpoint. This doc gives every store-lifetime / codegen decision one place to point at;
formal/ownership.md is the authoritative per-rule closure register.

## The beacon (one sentence)

> **`deps` should become a sound, complete, statically-computed ownership/borrow
> system — loft's borrow checker — from which every store-lifetime codegen decision
> (free placement, adopt-vs-copy, move-vs-clone, drop) derives MECHANICALLY.**

Corollary (from [CODEGEN_METHOD.md](CODEGEN_METHOD.md)): a store-lifetime bug is
then never a *codegen* bug — it is a **hole in the ownership computation**. Fix the
fact, not the generator.

## External validation — a consumer named this from its own pain (2026-06-24)

The **zero-trust** dogfood consumer — building crypto/signature systems on loft —
independently identified this exact beacon as loft's **single most important weakness and
#1 priority**, reasoning only from its own bugs: *"stop patching store-lifetime
symptom-by-symptom, pin the single invariant the heap model must uphold, enforce it at the
chokepoint — turn this class from recurring-and-silent into impossible-by-construction."*
A real consumer landing on the project's own north star is the priority confirmed from outside.

**The stake it sharpened: trust.** For crypto/signatures, **silent corruption is the one
fatal failure** — a wrong byte is an invalid signature with no error raised. So this class is
not one weakness among many; it is THE gate on loft being trustworthy with real data.
Cross-target parity tests catch the divergences today (the right defensive move), but the
cure is the complete `deps` analysis, not the test net.

## ACTIVE — the simplification exploration (next days; exploratory + revertable)

> **NOW: spend the next days collapsing the per-site ownership thicket toward the beacon —
> `deps` as the ONE fact every store-lifetime site reads.** This is an exploration of *how
> far we can go*: land the fact once and watch N forests collapse. A branch that doesn't pay
> off gets reverted — no harm; the goal is to find the ceiling of the simplification.

The fresh motivation is **[#448](https://github.com/loft-lang/loft/issues/448)**: fixing
ONE NRVO multi-return leak required adding THREE more per-site conditions
(`returned_uses_buffer` + `body_has_buffer_return` + `tail_terminal_fresh_local_vec`) and a
special case to `block_result` (`src/parser/control.rs`), which already carries dozens of
tail-shape cases. That is the thicket the beacon exists to delete: each leak closed by
another condition rather than by completing the fact. ([[evolve-data-structures-when-burdened]]
— the condition-count IS the signal the structure, not the logic, is the burden.)

**The substrate is already done — the work is the collapse.** Typed `Deps` (D-own-3 /
[DEPS_INVENTORY.md](DEPS_INVENTORY.md) H2) is COMPLETE (steps 1–5, 2026-06-12): the newtype,
named constructors, space-checked queries (`frame_vars` / `as_attr_indices`), and the
`CALLEE_FRAME_BIT` value tag all landed, debug+release suites green. So the fact is now
*typed and readable*; what remains is **D-own-1 — every store-lifetime decision should READ
that fact, not re-derive it per site.**

**Entry point: the `block_result` return-delivery thicket** (`src/parser/control.rs`, the
heart of D-own-1). Measured: **459 lines, 45 special-case helper calls, 15 distinct
tail-shape decision helpers** — each re-deriving "which store does this return deliver / who
frees `__retbuf`" from the parse-tree SHAPE rather than reading one deps fact. #448 was a
fresh deposit into it (three more helpers). The first collapse is scoped in
[plans/85-store-lifetime-retirement/D-own-1-return-delivery-collapse.md](plans/85-store-lifetime-retirement/D-own-1-return-delivery-collapse.md).
Then `parse_return` and the [STABILITY_REDFLAGS.md](STABILITY_REDFLAGS.md) clusters follow.
The [formal/ownership.md](formal/ownership.md) `D-own-*` deviations are the finish line: OPEN
→ 0 means the class is closed by construction. The differential oracle (`tests/oracle/`,
@PLN89) is the safety net — every collapse validated leak/value-identical across both backends
before it lands.

**The invariant as a violation set.** The whole class is ONE invariant — *each heap store has
exactly one owner at every point; all mutation flows through that owner; a non-owning alias is
read-only and never outlives its owner* — breached four ways: **leak** (owner dropped without a
free) · **double-free** (owner duplicated) · **use-after-free** (alias outlives owner) ·
**silent corruption** (two owners mutate one store). The #437 NRVO regression is the corruption
face: two coexisting returned vectors handed the same `__retbuf`. The concrete entry that
consumer forced open is **@PLN85 cluster V**
([cluster-V-nrvo-adopt-ownership.md](plans/85-store-lifetime-retirement/cluster-V-nrvo-adopt-ownership.md),
formerly the standalone plan-90 #437 investigation) — pin this slice of the invariant (a vector
local's `dep` = the store it owns), enforce at the chokepoint.

## Why this shape — the control story (maker's call)

> **loft exists to give the programmer MORE control over memory than Rust does — control
> over *allocation topology*: group data that belongs together into ONE allocation, split
> data that does not into ANOTHER.** One store is one allocation with one lifetime, laid
> out together and freed as a unit; separate stores hold data with independent lifetimes
> and no false coupling. That grouping choice is the programmer's, made directly. It is the
> engine author's core tool — arenas, pools, per-frame allocators, entity storage — and
> loft hands it over instead of hiding it.

**The pitfall it kills — interleaved on-the-fly allocation.** The worst memory problem in a
managed language is both *invisible* and *unactionable*. An object allocated on the fly lands
wherever the allocator puts it, interleaved with unrelated data, so a logically-cohesive set of
objects ends up scattered across the heap. It costs nothing at small data (everything fits in
cache) and surfaces only under production load — and even the programmer who *diagnoses* it has
no lever: Java cannot dictate placement (`ArrayList<Foo>` is a contiguous array of *references*
to scattered objects; GC compaction reorders by generation, never by your logical grouping). The
store is that missing lever: put related records in one store and they are packed together, not
interleaved — you write down the co-location the managed language gave you no syntax for. This is
data-oriented design as a first-class control, which is exactly why it doubles as the engine
author's core tool.

**The split that makes this work — the machine keeps the bookkeeping, the programmer keeps
the layout.** Two different things get called "control"; loft assigns them to different
owners:

- the **layout** decision — *what is co-allocated with what* — is the **programmer's**, explicit;
- the **bookkeeping** — *when a store is freed, who owns it, copy-vs-move, borrow tracking* —
  is the **machine's**, automated. That automation is the whole `deps` beacon above.

loft gives you the first and takes the second off your hands. This is why "internal and
invisible" (next section) does not fight "full control": the *bookkeeping* is invisible; the
*layout* is yours.

**The line between the two is drawn by performance-criticality — and what is automated stays
deterministic.** A performance-critical decision is never abstracted away: allocation topology,
the highest-leverage one, is exposed and controlled by the programmer. What loft automates —
free placement, copy-vs-move — it derives from the layout you chose and runs *predictably*:
freeing happens at owner death (no tracing collector, no pauses), and a whole-value bind copies
unless the source is provably dead (§ The law), so the cost is legible from the source, not
hidden. loft draws the line at *hidden, nondeterministic* machinery — a tracing GC, a surprise
reallocation, a silent deep copy — and rejects it. A language that hides its performance-critical
decisions cannot be trusted with them; loft keeps every such decision either in your hands or
predictable.

**Why this is MORE control than Rust, not less.** Rust's ownership is single-owner and
tree-shaped by construction, which pushes you toward one-allocation-per-object. The moment
your data is a graph — which real engine data always is — controlling co-allocation means
reaching for arena crates and re-encoding pointers as indices into a `Vec`, which drops you
*out* of the borrow checker: you get layout control **or** the safety discipline, not both.
loft makes the arena the native unit — a store IS the arena, `DbRef` is the index-pointer
(`src/keys.rs`) — and the `deps` discipline is built to hold *across* grouped, graph-shaped
data. So you author the allocation topology **and** keep the lifetime guarantees. That
combination — hand-authored layout under an enforced discipline that survives it — is the
control loft is built to give and Rust withholds.

## Internal and invisible — never a user-facing borrow checker (decided)

> **Ownership in loft is INTERNAL.** It never surfaces an ownership error to the
> programmer. The user writes naively — `a = makeThing(); b = a; b.x = 9` — and the
> compiler always finds a correct lowering, **copying when it cannot prove an alias is
> safe**. No "cannot borrow", no lifetime annotations, no move-vs-borrow puzzles. The goal
> is the *most natural solution for the programmer*: write it the obvious way, it works.
> (This was always the plan.)

This invisibility is about the **bookkeeping** — ownership errors, free placement,
copy-vs-move — not about layout. Layout (which store data lives in) stays the programmer's
explicit control; see the control story above.

This makes the system's load-bearing property **completeness, not just soundness**: the
analysis must be **total** — produce a valid free/copy/move for *every* binding on *every*
path, never get stuck, never reject. An incomplete fact is **not** a compile error the user
fixes (that is the user-facing model loft does *not* have) — it is a miscompile or a leak.
So [ownership.md](formal/ownership.md)'s `O-Complete` is the invariant under the most pressure.

**"Rust as the reference model" means SOUNDNESS, not UX.** loft borrows Rust's *internal
discipline* (one owner, move on return, borrow tracking) to get codegen right — it does
**not** import Rust's user-facing borrow checker. The one deliberate user-facing ownership
concept is `&` (§ The law): an opt-in for *shared mutation*, when the programmer explicitly
wants a live reference instead of the default copy/alias. That is the entire surface; a
user-facing borrow checker is **declined** ([DESIGN_DECISIONS.md](DESIGN_DECISIONS.md)) — it
would fight loft's *fun-on-pickup* goal ([GOALS.md](GOALS.md)).

**The one carve-out: loft DECLINES what it cannot implement safely** (C79, revisited
2026-08-05). "Never rejects" is about programs loft *can* compile correctly — and that is
nearly all of them, which is why the surface stays clean. It is not a promise to accept a
program whose meaning loft cannot deliver. Writing `&` is an ownership decision by the author,
not a hint: it asks for a live link. Disturb the place that link names while it is still in use
and loft has no honest lowering left, so it reports a compile error instead of quietly handing
back a copy — `formal/binding.md` **B-Ref-Reshape**. The distinction that keeps this from being
a borrow checker by the back door: loft refuses only where *no* correct lowering exists, never
because a correct one is hard to prove. Naive code is unaffected, and a plain bind never meets
it at all — it gets the copy described below.

## The law — whole-value binds COPY, projections view; `&` binds a live REFERENCE

> **CORRECTED (2026-07-03, C86 — maker's call).** The earlier text ("a binding to a heap
> value aliases; it does not copy") was empirically FALSE on both backends for whole-value
> binds and never matched the ecosystem's behaviour. The 2026-06-23 correction (the `&`
> reading) stands. See [DESIGN_DECISIONS C86](DESIGN_DECISIONS.md#c86--whole-value-heap-binds-copy-aliasing-is-a-last-use-elision-the-rustc-rule)
> and [plans/87-reference-default-binding.md](plans/87-reference-default-binding.md).

**Whole-value heap binds COPY** — `p = o` (struct), `b = x` (vector), `af = bx.v` (a
field read bound to a local) all give the new binding its OWN store: value semantics,
verified on both backends. The compiler may **ELIDE the copy to an alias only when the
source is provably dead afterwards** (the rustc last-use rule — `use_analysis::ElidePlan`
is exactly this analysis): an *optimization*, never an observable semantic.
**Projection reads are VIEWS** — `a = vv[0]; a[i] = z` writes through (the #426 decided
feature), and in-place mutation through a *path* (`o.field = x`, `o.v[i] = y`) reaches
the source. A **scalar** binding is a by-value **copy**.

### A view lasts as long as the thing it names — and loft says when it does not

A projection view is only sound while the place it names still exists, so loft **materialises**
the binding — gives it its own copy, taken at the bind — whenever it can prove the place will
not survive the binding. Three things end a view's place, and all three are decided at COMPILE
time (@PLN130):

| what happens to the container | why the view cannot stay an alias | the copy is reported as |
|---|---|---|
| **RESHAPED** — `a.remove(i)` | removal renumbers the positions, so the view's pinned position starts naming a DIFFERENT element | *"`c` was copied out of `a` because `a` is modified while `c` is in use"* |
| **RE-KEYED** — `c.key = 5` on a keyed collection | the element would be reachable by no key at all, and for a `sorted` the record MOVES, invalidating the very view doing the writing | *"… because `key` is one of `s`'s keys"* |
| **REASSIGNED** — `bx = T{…}` | the view names a place inside `bx`, and giving `bx` a new value leaves nothing for it to point at | *"… because `bx` is reassigned while `c` is in use"* |

After materialising, writes through the binding **no longer reach the container** — which is
why every one of them is reported rather than done silently. The advice names the function, the
binding and the container.

**All three require the view to be LIVE ACROSS the event — the advice's "while `c` is in use"
is a condition, not a figure of speech.** A view whose last use comes *before* the disturbance
keeps its alias and writes through, and so does one bound *after* it: nothing was pointing into
the container when it changed. This is the rustc rule — a borrow that has ended is no conflict
— and it is not a refinement, it is required. Materialising a dead view LOSES a write the
program used to land, which is breakage, and it makes the advice state something false about a
binding that is not in use. Both are on @PLN130's closure bar. Pinned by
`tests/scripts/148-view-liveness-across-reshape.loft`, which asserts each edge in both
directions on both backends.

**A `&` binding is never materialised — it is a live link and always writes through, because
the shape that could break that does not compile.** `c = &v[0]` opts into aliasing explicitly,
so no reshape may quietly turn it into a copy ([formal/binding.md](formal/binding.md)
B-Ref-Alias). The three rows above are the PLAIN-bind rule: that binding copies, so losing
write-through is consistent with what it already meant. For a `&` there is nothing consistent
to do, so loft **refuses the removal instead** — B-Ref-Reshape, the rustc bargain in loft's
spelling. Two rows, and they are disjoint:

| binding | container reshaped while it is live | outcome |
|---|---|---|
| plain `c = v[0]` | yes | materialise + advice (the table above) |
| `c = &v[0]` | yes | **compile-time error** |
| `c = &v[0]` | no | live link, writes through |

**Across a frame the `&` stops being the discriminator, and that is measured.** A struct
PARAMETER aliases the caller's element whether or not it is spelled `&` — `fn w(t: Box) { t.n
= 99 }` called as `w(v[2])` writes 99 into the caller's `v`, which is exactly what the
redundant-`&` advice tells authors. So `f(v[i], v)` where `f` removes from its container
parameter is refused for both spellings; there is no bind site inside `f` to materialise at,
and refusing only `&` would mean taking loft's own advice trades a compile error for a silent
lost write. Closed deviation **D-bind-8**;
[loft#779](https://github.com/loft-lang/loft/issues/779).

**Mutating the place is not ending it.** `d.mid = Mid{…}` writes the new value INTO the place
`d.mid` already occupies, so a view of `d.mid.inner` sees the update and keeps writing through:
that is the in-place path mutation above, not an invalidation. A view survives its place being
overwritten and cannot survive its place being destroyed.

**A vector local never needed this.** `a = [...]` allocates through a hidden compiler-owned
backing record, so re-binding `a` mints a NEW one and the original keeps its identity — a view
of it stays valid with no copy. A struct local IS its own owner, which is why reassigning one
is the case that had to be handled. Contract-wise both answer the value bound at the view;
they differ only in whether a copy was needed to get there.

The reassignment case also reaches across a call: handing a container to a function that
reassigns its `&` parameter frees the caller's store (@PLN87 P2.2), so the caller's views of it
materialise too. A **plain** (non-`&`) parameter reassignment is local to the callee (P2.1) and
leaves the caller's views alone.

Pinned by `tests/scripts/774-view-outlives-reassigned-container.loft` (the reassignment
boundary, with the write-through cells as controls), `tests/scripts/145-view-materialised-on-reshape.loft`
and `tests/scripts/201-bind-copies-projection-views.loft` (the C86 copy/view boundary).

**`&` binds a live REFERENCE** to its source — a variable, struct field, or vector
element. Every operation goes *through* the reference to the source: a read sees the
source's current value, a write writes the source, a field/element mutate mutates the
source.

> | operation | plain (no `&`) | `&` reference |
> |---|---|---|
> | heap in-place mutate (`o.field = x`, `o.v[i] = y`) | writes through (heap is shared) | same |
> | scalar read / write (`n`, `n = 5`) | local **copy** | **reads / writes the SOURCE** |
> | whole-binding reassign (`o = X`) | local rebind — source untouched | writes the source through the reference |

- **`&` is NOT a general operator** — it appears only in a reference-*binding* position
  (`a = &b`, or the declared type `a: &T = b`), and its operand must be **addressable**
  (a variable / struct field / vector element, never a temporary). `&` in a general
  expression — `f(&x)`, `&x + 1`, `[&a]` — is an error; a `&` *parameter* is called
  WITHOUT `&` (`f(x)`), since the reference comes from the parameter's type. `&a = 4`
  (`&` on the assignment target) is an error.
- **No lifetime annotations.** A reference is safe when the source outlives it; the
  analysis infers that from scope (rejecting `a = &mk()`, a reference to a temporary),
  exactly as it already does for `&` *parameters* (the caller outlives the call). C38
  declined reference *types* with annotations; this is a binding *notation* the analysis
  resolves itself.
- **Aliasing is bounded, not a footgun.** The single owner still frees once; a reference
  is a tracked borrow in `deps` (the borrow system above). The copy-vs-view question
  (#426 A/C) dissolves: the view is the documented default for heap, the by-value copy
  for scalars, and `&` makes either a live reference.
- **The payoff — the store-lifetime decisions collapse.** Once "is this binding a borrow
  of that store?" is one carried `deps` fact, the N places that re-derive ownership
  (#415's struct-field branch, the `a = x.v[i]` path, `has_ref_params`, the return-source
  set, the `block_result`/`ref_return` funnel, `reclaim_safe`) just read it. The
  store-lifetime bug class (Cluster A, #415, #426, the free-suppress + return-buffer
  leaks) is the *symptom* of computing it N times; landing the fact once closes the class.
- **Staging.** Heap aliasing is already loft's behaviour (`a = vv[0]` is a view — #426
  A/C correct, no value-semantics migration). The reference work (@PLN87) makes `&` a
  true live reference, scalars first: `a = &b` is live read- and write-through on both
  backends (L1/L2). Remaining: the addressable-operand check, the ban on `&` as a general
  operator (with its `f(&x)` → `f(x)` ecosystem migration), and the `a: &T = b` typed
  form — see the @PLN87 plan.

## Why — the bug class is the symptom of an incomplete ownership system

The store-lifetime class (#405/#406/#409/#410 this cycle, cluster II of @PLN85) is
not N unrelated bugs. Each is a place where ownership was **re-derived by a codegen
heuristic** instead of **computed once as a fact**:

- `has_ref_params` at the call site, standing in for "does the return transfer
  ownership or borrow an arg?"
- `returned_var`'s single-`u16` structural walk, collapsing a `match`/`if` return
  to "no return var" → the returned arm buffers get freed.
- the return type left with an **empty / `"??"` dep** for `return v`, so a
  borrow-of-param return is indistinguishable from an owned return → the caller
  aliases the arg.
- **three separate return-handling paths** (`BlockTail` / `MidReturn` /
  native-forwarder) each re-deriving the same answer differently.

Every one is a missing or incomplete ownership fact. The class closes when the fact
is complete; it cannot be closed by adding more codegen conditions (that is what the
9 reverted @PLN85 attempts proved).

### The unification — `ownership_of` as the ONE carried fact (@PLN85, DEFAULT-ON)

`src/use_analysis.rs` computes the ownership oracle once — `ownership_of(data, d_nr,
value) -> Own { Owned | Borrowed{base} | Join{base} }`, carrying the interprocedural
borrow `base` (the witness a runtime guard needs) — and the over-free chokepoints READ
it instead of re-deriving. **FLIPPED DEFAULT-ON (2026-07-02, `keys.rs::join_own_enabled`;
`LOFT_NO_JOIN_OWN` opts out).** Evidence at the flip: the 54-cell over-free map 6/54
opt-out → **0/54 default** (value + divergence + leak + poison, both backends); full
suite green both legs; `tests/use_analysis.rs` pins both legs (the oracle ground-truth
dumps read the RAW shapes via the opt-out; the synthesis's observable effect has its own
two-leg discriminator). Each site validated on BOTH backends:

- **`local_source`** — ✅ both backends (scope-pass dep-strip of the displaced-owned slot).
  Since loft#1336 the same shape is ALSO closed by `(O-Witness)` (`LOFT_NO_OWNER_WITNESS`
  opts out), so the join-own A/B holds both switches off on its control leg.
- **`elem_accumulate`** — ✅ both backends (interp `OpBindOrCopy` + native inline guard, both
  reading the oracle's `base` as the runtime witness).
- **`match_return`** — ⏳ NATIVE done; INTERP pending. `ref_return` materialises a borrowed-view
  return candidate (guarded by `skip_free(v)` + `base ∈ deps`, so only match-field bindings, not
  stdlib `result` buffers). The interp gap is now PROBE-PINNED to the parser layer: the proven
  `copy_borrow_tail_into_retbuf` path survives churn, so the defect is that the hand-rolled append
  OMITS that path's return **registration** (`returned = Vector(elm, Deps::attrs([buf_attr]))` +
  the typed `borrow_tail_copy` block). Fix = route through / replicate the proven path, not a bare
  `OpAppendVector`. (Not the tp, not interp execution — both ruled out by probe.)

### The RETURN join is decided per run, not by a bit (loft#981 / loft#982)

A heap return carries ONE static answer to *may the caller free this?*, read off the
return deps: a dep naming a visible parameter means BORROW (never free), an empty one
means OWNED (the caller adopts and frees). A return that is a view of a parameter on one
path and a freshly minted store on the other has **no correct static answer**, and the one
it got — borrow — orphaned the minted store: one leaked store per call, unbounded in a
loop, silent on both backends with every value correct.

Two filed shapes, one defect:

| shape | why the dep says borrow |
|---|---|
| loft#981 — the arms genuinely split (`b.items[k] ?? Item { … }`) | the hit arm really is a view; which arm ran is a run-time fact |
| loft#982 — the arms do NOT split (`if !fresh { return o; }` over a by-value struct param) | the dep is STALE: the return hoist lowers `return o` to `__ret_1 = o`, which DEEP-COPIES, so both paths deliver a fresh store — measured, and why BOTH arms leaked |

**The cure reuses @P290 rather than adding a rung.** The call bracket already marks a
caller's arguments *"do not free mine"* for the duration of the call, and both backends'
`OpCopyRecord` already refuse the `0x8000` source-free on a marked store. So the bit is
SET at a bracketed call site and the run decides: a returned store belonging to an
argument is refused the free, a callee-minted one is not marked and is freed. That is the
`Own::Join` witness test, spelled with machinery that already existed — no new opcode, and
it scales to several witnesses where a single `OpFreeRefIfDistinct` could not.

Three emitters read one fact (`use_analysis::call_return_frees_source`): the interpreter's
first-bind and reassign paths, and native `generation/dispatch.rs`, which had no bracket
and now emits the same one.

**Two bounds, both load-bearing:**

- The witness set must span every argument the return COULD borrow, not every argument the
  bracket happens to accept. The bracket takes `Reference`/`Vector`/`Enum`; a **keyed
  collection** (`hash`/`sorted`/`index`/`radix`/`trie`) is a borrow source it does not
  cover. Reading the narrower list as complete freed a hash parameter's element out from
  under the caller — `tests/scripts/882-…` went red under poison, which is how it was
  found. Coverage is asked with `heap_dep()`, through `base()` so an `Optional` wrapper
  cannot hide the storage under it.
- An argument the bracket cannot name (not a bare `Var`, or a keyed collection) leaves the
  witness set incomplete, so that call site keeps the old conservative *never free*. The
  leak stays for that shape rather than risking a free of a store the caller still reaches
  — a leak is the lesser wrong. In practice the parser lifts a non-`Var` ref argument into
  a temporary first, so the fallback is rarer than it reads.

`Store::free_protect_depth` is a DEPTH, not a flag, for the same reason: one call bracket
inside another may protect the SAME store, and a boolean let the inner release drop the
outer bracket's protection — after which the outer copy's source-free would free the
caller's own argument. No probe in the suite distinguishes the two today; the depth closes
the hazard by construction rather than by luck.

Guard: `tests/scripts/981-split-ownership-return.loft` + `tests/split_ownership_return.rs`
(leak as the deterministic oracle, poison/strict for the opposite direction, and the
keyed-collection control). Against a pre-fix binary it orphans 161 + 41 records on both
backends while every value cell still passes — which is the whole shape of the defect.

### A var holds a value two ways — `Set` and FILL (loft#704)

`ownership_of` resolves a var by joining its DEFINITIONS, and for a long time a
definition meant a `Set`. It is only one of the two ways a var comes to hold something:
the other is an in-place **fill** — `OpClearVector(v)` / `OpAppendVector(v, …)` — which
defines nothing a `Set`-scan can see. So

```loft
match e { Filled { items } => { items }, _ => { [] } }
```

read as a plain `Borrowed(base=e)`: the borrowed arm re-binds the return buffer and is
visible, the owned `[]` arm FILLS it and is not — losing the exact split `Join` exists
to name. The fill is an **Owned** alternative and unconditionally so, because
`OpAppendVector` deep-copies: the filled branch delivers the buffer's own contents
whatever the source was.

**It is an alternative, never the whole answer**, and that restriction is the sound half.
A buffer that is only ever filled and never re-bound is the retbuf-param shape the model
deliberately calls `Borrowed` (§ the approximations in `use_analysis.rs`): its value
really is owned, but the buffer belongs to the CALLER, and a callee told `Owned` may free
it. So the fill joins with a var's other defs and does nothing when there are none.

The general rule: **when an analysis reads "the definitions of X", enumerate every way X
can be established before trusting the join.** A conservative approximation guarding one
direction (here: a bare param reads `Borrowed`) must be EXTENDED with the missing
alternative, not replaced by it — replacing flips the very direction it was protecting.

### The slot-level twin — `Function::owns_store` (loft#664)

`ownership_of` answers "does this VALUE own its store"; the codegen and parser sites
that decide whether to ALLOCATE into a slot ask the narrower slot-level question, and
they used to answer it three ways at once: an empty dep list, the `_elm` NAME prefix,
and the `inline_ref` / `skip_free` markers. `Function::owns_store(v)` is now the one
predicate — `!inline_ref && !skip_free && is_independent` — and `is_element_alias` is
deleted.

**Deps alone cannot carry it.** A dep names the borrow SOURCE, so a borrow with no
source VARIABLE — a vector inside an enum payload is addressed by a field DbRef —
produced an empty list, and empty means OWNED. That is a wrong answer, not an unknown
one, and it is why loft#660 had to fall back on a name. So the fact is now STATED where
the non-owning slot is MINTED (`unique_elm_var` marks the element `inline_ref`), and the
dep is still recorded where a container variable exists — the two carry different facts,
which is what falsified the attractive "add a marker, delete the deps" version.

The general rule this instance teaches: **when a fact cannot be represented for some
inputs, the encoding is the bug — not the consumer that read it wrong.** An empty dep
list conflates "owns" with "cannot say", and every consumer inherits the conflation.

### A placement rule may not delete a fact (loft#677)

The same empty dep also arrives a third way: the fact is computed correctly and then
**thrown away by a rule that was answering a different question**. `ref_return`'s ladder
decides where a returned LOCAL should live — rename it onto `__retbuf`, bind it, promote
it to a parameter — and one rung skips a local that looks reassigned. That rung sat
*above* the rung that records which attribute the return borrows, so a PARAMETER reaching
it lost its borrow: `fn add(o: Outer, …) -> Outer { o.tags += […]; o.items += […]; o }`
returned with an empty dep, callers read the returned borrow as an owned store, and the
second such call freed the caller's store while it was still being filled.

Two rules come out of it:

- **Placement never outranks provenance.** "Where does this value live" and "what does
  this value borrow" are different questions; a var that is already an attribute has no
  placement left to choose, so a promotion verdict must not pre-empt its dep merge.
- **Name a predicate for what it measures, not for what you concluded from it.** The rung
  asked `reassign_count(body, o) >= 2`, but the count walks `Set(_elm_k, OpNewRecord(v,
  …))` — records allocated as CHILDREN of `v`, i.e. APPENDS. A rebind lowers to
  `OpDatabase` and is invisible to it, so the predicate never saw the thing it was named
  after, while two ordinary appends tripped it every time. It is `child_allocs` now. An
  earlier carve-out had already exempted vector returns from the same false positive —
  a per-type exemption is the tell that the predicate, not the type, is wrong.

### A promoted local is not a parameter (loft#688)

The same ladder's `Rename` rung `become_argument`s a returned local so it IS the caller's
return buffer. That moves it into scope 0 — and `scopes::variables` stops at scope 0 under
the comment *"never return function arguments"*, because a true parameter belongs to the
caller and the callee must not free it.

An NRVO buffer does not satisfy that contract. It is a **promoted local**: this function
mints its store with `OpDatabase`, and a sibling return site can deliver a different one.
On that path the minted store was returned by nobody and freed by nobody — one orphan per
call. `fn parse(line) -> Way { none = Way{…}; if bad { return none; } Way{…} }` in a loop
therefore exhausted the 65,535-entry store table, and *nothing* was visible below 65k
iterations, so it passed the whole suite and first appeared on a country-sized input.

- **Scope class is a claim about ownership, and moving a variable into one must earn it.**
  Scope 0 encodes "the caller owns this". The rename put a callee-minted store there, so a
  fact that was true of every other member of that class was silently false for this one.
  The repair states the remaining obligation instead of asserting the class: the buffer is
  routed through the same hoist + `OpFreeRefIfDistinct` leg the null-arm join uses, so each
  return frees it *unless* it is the store that path transfers.
- **Refusing the promotion is the wrong altitude.** It was measured: an explicit
  `return out;` parses as a `MidReturn` even when it is a function's ONLY exit, where the
  rename is both sound and load-bearing — the sandbox admission walk reads it to know a
  write escapes, and suppressing it regressed plan86's laundering guard. The leak is an
  ownership fact, so it belongs in the ownership sweep, not the placement ladder.
- **Two axes the report named were not the trigger.** The abandoned struct need not own a
  collection (a plain `text`+`integer` struct leaks identically), and building the local
  without the branch-return is clean. Guard:
  `tests/scripts/688-abandoned-return-candidate-leak.loft` (seven shapes plus three
  controls, each verified leaking against a pre-fix binary).

**The methodology that made this work** (vs the 9 thrash attempts): build the fact INERT and
validate it against a ground-truth corpus FIRST (the accumulated per-fix regression tests ARE the
spec), then collapse one chokepoint per commit reading that fact — and when a proven sibling path
already exists for a delivery, MATCH it rather than re-derive it (diverging from the proven path is
itself the defect). See `plans/85-store-lifetime-retirement/ownership-analysis-gaps.md`.

## Rust as the reference model

Rust is this beacon already realized — which is why it is the design reference:

- **Ownership and lifetimes are *type* facts**, computed once by the borrow
  checker. Drop insertion, move-vs-copy, and "may this value be returned" all
  *fall out* of those facts; codegen re-derives nothing.
- **Move by default; `Clone` is explicit.** A return value is *moved* to the
  caller, who becomes its sole owner. loft's `a = id(x)` aliasing bug is exactly
  the case Rust makes unrepresentable: returning `v` moves it out (or borrows it
  with a tracked lifetime); you cannot silently end with two owners.
- **Completeness is the whole game.** Rust computes ownership for *every* binding
  on *every* path — no "we didn't enumerate the `match` shape", no `"??"`. loft's
  bugs are precisely its incompletenesses.
- **It took years.** The borrow checker was a multi-release effort. loft's
  equivalent is a multi-cycle migration, not a patch.

The lesson, concretely: **do not bolt ownership onto codegen; make `deps` a
first-class, complete ownership analysis and let codegen read it.** The "thicket of
return paths" is what you get when ownership is re-derived per-shape; Rust avoids it
by having ONE place own the answer.

## loft today — a nascent borrow system

The pieces exist; they are incomplete:

- **Types carry `deps`** (typed `Deps`: `DepEntry::Attr(a)` | `DepEntry::CalleeFrame(w)`
  — see [DEPS_INVENTORY.md](DEPS_INVENTORY.md)). Semantics: empty dep ⇒ **owned**;
  `{Attr(a)}` ⇒ **borrows attribute a**.
- **The allocator has per-store liveness** (`free_bits` / `find_free_slot` in
  `src/database/allocation.rs`): an owned, live store is not recycled — the
  substrate a sound ownership system needs.
- **But the computation is partial** and supplemented by heuristics (above). The
  store-lifetime bug class is the catalogue of those gaps.

**How partial, measured.** `make licence-census` (@PLN155 phase 0) classifies every free the
compiler EMITS by the fact that licensed it. Over the 1412-file corpus, 80 325 emitted frees:
56.3 % rest on an owner fact the oracle derived from the binding's own definitions, 21.9 % on
an `OpDatabase` mint, 11.6 % were emitted where the oracle answers `Borrowed`/`Join`, and
**1.8 % — 1464 frees — rest on the empty-`deps` PROXY alone**, with the oracle having nothing
to read at all. It read 8.0 % until the oracle was told about DELIVERY BUFFERS: 6.2 % of frees
are a `__vdb`/`__ref`/`__retbuf` whose store was minted to back that var, a positive owner fact
`use_analysis::is_synth_buffer` already stated and the oracle had never consulted (@PLN155
phase 3a).

That is the number this doc's "partial" means, and it is a report rather than a defect count:
it says what each licence RESTS on, not which frees are wrong. `make licence-census
ARGS=--control` proves the buckets can move in both directions before any of it is believed.

## The invariants the system must enforce (sound AND complete)

1. **Single owner.** Every heap store has exactly one owner at any moment.
2. **Move on return.** A returned heap value's ownership transfers to the caller's
   binding; the callee never frees what it transfers. If the return *borrows* a
   parameter, that is recorded on the return type (`{Attr(param)}`), and the caller
   copies to obtain its own store.
3. **Borrow tracking.** A value aliasing another (a param, field, or element)
   carries that source in its `deps`; the borrower is skip-free; the single owner
   frees once.
4. **Free placement is derived, not decided.** Free a local iff it owns its store
   and does not transfer it out — once, at scope exit. No per-site heuristic.
5. **Per binding, per path, complete.** Including every `match`/`if` arm — a set +
   reconcile, not a single-var structural walk.

When these hold, both backends translate the *same* facts, so interp and native
cannot diverge.

## The migration backlog (provenance record — live status is formal/ownership.md)

**Status:** the formal register in [formal/ownership.md](formal/ownership.md) is now at
`OPEN: 0` — all five `D-own-*` deviations closed on the shipped path. The rows below are
the migration's provenance; the few still framed as open (a general "return-source set",
complete `"??"` deps) are subsumed by the CLOSED D-own-1/D-own-2 — the `Join` fact is
total, resolved by a runtime witness rather than a further static dep. The remaining
*static-precision* gap (flow-insensitive classification) is tracked by
[@PLN94](plans/94-cfg-ownership-dataflow/), not here.

| Hole | Symptom | The fact to complete |
|---|---|---|
| `returned_var` collapses `match`/`if` | returned arm buffers freed (cluster II / #405) | a return-**source set** (union of arms), not one var |
| ~~return dep empty for `return v`~~ ✅ CLOSED | `a = id(x)` aliased `x` | RESOLVED @PLN85: when the returned value is backed by a PARAMETER, the callee copies it into the return buffer (clear+append+return buffer, gated on `is_argument`) so the caller gets an owned copy — `control.rs` return promotion. Guard: `tests/scripts/85-store-lifetime-param-return-copy.loft`. (A copy-on-return, vs the originally-envisioned borrow-tag-then-caller-copies; same value-semantics outcome.) |
| ~~`has_ref_params` at the call site~~ ✅ CLOSED | adopt-vs-copy re-derived; vector returns alias (the `a = getv(x)` case of [probe 06](plans/85-store-lifetime-retirement/probes/06-field-read-adopt-vs-copy.loft)) | RESOLVED @PLN85 (A.3): the 11 `has_ref_params` decision sites (4 live — `scopes.rs`, `state/codegen.rs` ×3; the REASSIGNMENT-path gate joined 2026-07-04, #497 — its visible-Ref/Enum proxy missed a vector-param borrow and the owned pre-Set free killed the lender's store) now read ONE carried fact, `Definition::return_adopts_fresh_store()`: **return dep empty OR the `["??"]` one-buffer marker ⇒ adopt; any real attr index ⇒ copy**. This is STRICTLY BROADER than the A.4 `returns_borrowed_view()` (which only checks VISIBLE attrs): the adopt-vs-copy decision must ALSO copy a HIDDEN `ref_return` work-ref return (dep `["cv"]`, the caller-reused `__ref_N` buffer — over-unifying onto `returns_borrowed_view` regressed `143-plan51-cluster3` with a cross-iteration alias). The coarse `has_ref_params` over-approximated copy; the refinement flips copy→adopt ONLY for the genuinely-fresh case (`fn mk_from(seed) -> Box { Box{…} }`). `returns_borrowed_view()` stays as the *source-free-bit* fact (A.4, 3 sites). Guard: `tests/scripts/85-store-lifetime-return-ownership-adopt.loft` (+ `143` native leak-clean both backends). **loft#810 — unifying the ANSWER left the GATE duplicated, and it drifted.** Each site still spelled out for itself WHICH callees the question is asked about, and the spellings disagreed: `scopes.rs` accepted `n_` and `t_` alike (and on that basis stripped the binding's deps, which is what makes its scope-exit `OpFreeRef` fire) while both `state/codegen.rs` sites accepted `n_` only. A METHOD returning through the caller's `__ref_N` buffer therefore fell past both copy arms to plain adopt AND was freed as an owner — the buffer's store recycled while the caller still named it, one record with two owners. The gate is now `Definition::is_loft_defined()`, living beside the two facts it guards. Guard: `tests/scripts/810-method-return-buffer.loft` (three assignment shapes — first-Set, reassignment, nested block — because the two codegen sites split them, plus adopts-fresh / returned-param / vector-retbuf controls). |
| ~~field read binds without an owner~~ ✅ CLOSED | `a = x.v` / `a = getv(x)` aliased the field's store | RESOLVED @PLN85 (#415): a STRUCT vector-field read now COPIES on bind (the @P292/@P394 bind-site branch admits a struct-field OpGetField) and on implicit-tail return (block_result copies a struct-field-of-argument return into `__retbuf`). Both narrowed to struct-field reads — a vector INDEX read (`vv[i]`) keeps its existing nested-stride path. Guard: `tests/scripts/85-store-lifetime-field-read-copy.loft`. (Copy-on-bind/return, the row-102 adopt-vs-copy fact instantiated for struct fields; a general dep-driven caller copy for arbitrary borrowing returns remains row 102's broader work.) |
| ~~value-`if`-return promotes ONE arm's buffer~~ ✅ CLOSED (a7) | `fn f(c) -> vector { if c { [..] } else { [..] } }` lost the true arm (interp read the sibling, native read empty — backends DIVERGED); struct/ref 3-arm `if` read the LAST arm on native | RESOLVED @PLN85 (a7): two facets of the row-100 `match`/`if` hole, for the `if` tail. (i) The function-return PROMOTION (`ref_return`/`text_return` renaming `__retbuf` to a body-tail local) is now gated to the GENUINE return context (`context == "return from block"`) — an `if`/`match` ARM is not the function tail, so no arm's `__vdb_N` is promoted; the fn-body tail delivers every arm into `__retbuf` (the `match` path). (ii) `unify_if_branches_work_refs` now collects EVERY arm's terminal work-ref across the whole if-tree and unifies them to one slot, so an `else if` chain (3+ arms) shares the return buffer like the 2-arm case. Guard: `tests/scripts/85-store-lifetime-if-return-owned-arms.loft`. (The `match` tail was already correct; this brings the `if` tail to parity. The row-100 "return-source set" remains the general framing.) |
| 3 return paths (`BlockTail`/`MidReturn`/native-forwarder) | each re-derives; fixes miss paths | funnel to ONE return-ownership computation. **A.2 partial:** the borrow-tail copy (a value backed by a visible param/field returned implicitly) now funnels to ONE helper `copy_borrow_tail_into_retbuf` shared by the struct-field tail (#415) and the whole-arg tail (`fn idv(v) -> vector { v }`), gated to `context == "return from block"` — both copy into `__retbuf` instead of `ref_return` recording a borrow dep. Guard `tests/scripts/85-store-lifetime-implicit-param-return-copy.loft`. The `if`-return buffer-model case (matrix a7) is **CLOSED (a7)** — see the row above. |
| ~~inline/discarded owned heap return leaks on native~~ ✅ CLOSED (#490/#491) | a heap value produced for inline use — a native-constructor temp (`jt(json_parse(x), n)`), a discarded statement (`json_parse(x);`), or a hidden-`__ref_N` enum return used un-bound (`relen(mk().x)`) — leaked its store on native (and some on interp) | RESOLVED: three converging holes in the same "who frees a temporary the caller never named" class. (i) `inline_struct_return` now matches through `Span` (native-constructor result used as call arg / method receiver) so its lift fires. (ii) `scan`'s `Drop` arm lifts a discarded owned result to a `__lift_N` temp (was a bare stack-pop that freed nothing — once per loop iteration). (iii) the enum-arm lift guard widened from `dep.is_empty()` to `!returns_borrowed_view()`, so a hidden-work-ref enum return (`returns_borrowed_view()==false`, the caller-reallocated `__ref_N` the by-value native ABI never delivers back) lifts and is freed via the bound-case copy path, while a genuine borrowed view of a VISIBLE param (`fn field_of_arg(d) -> H { d.value }`) stays unlifted — no UAF. Underpinning both: `is_stack_store()` replaced every bare `store_nr == 0` source-free / leak-check guard — the native runtime has NO eval-stack store at slot 0, so those guards were hiding + refusing-to-free the first native heap store. Guards: `tests/leak_cases/clean/i490_*`, `i491_file_ctor_receiver.loft` (both backends). |
| `"??"` deps | unresolved ownership | compute the dep completely, no placeholder |

(The typed-`Deps` newtype work in [DEPS_INVENTORY.md](DEPS_INVENTORY.md) is the
groundwork already laid for this.)

## The migration discipline

Per [CODEGEN_METHOD.md](CODEGEN_METHOD.md): **one fact at a time, bottom-up,
working-vs-broken bytecode, study how Rust does it, validated on both backends.**
Replace heuristics with facts and *consolidate* the duplicated paths as you go —
each consolidation is a down payment on the beacon (one path, one fact). Expect
multiple cycles. Order by leverage: return-ownership first (it is the cluster-II
root and the most-reused decision), then nullability/layout/capture as their bug
shapes surface.

### Small steps are VITAL — not just tidy

Each fact is a **small** migration, and it must STAY small: a single, narrowly
scoped change that touches one decision and leaves the rest alone. This is the
load-bearing constraint, for two reasons:

- **It keeps the migrations small.** A small step is reviewable, testable on both
  backends in one rung, and revertible without collateral. A big-bang "rewrite the
  ownership system" is exactly the multi-hundred-line thrash the 9 reverted @PLN85
  attempts were — un-bisectable and unsafe.
- **It keeps the parser clean.** The danger when moving a fact INTO the
  type/parser layer is over-correcting — dumping a pile of new analysis into the
  parser and merely relocating the complexity (the caution in CODEGEN_METHOD § The
  balance). Small steps prevent that: each adds one well-defined fact, the
  heuristic it replaces is *deleted* in the same step, so the parser's net
  complexity stays flat or drops. The parser should get *cleaner* with each
  migration, never heavier.

**If a step is getting large, you have bundled facts — split it.** A migration that
can't be a small step is a sign the fact isn't isolated yet; find the one decision,
do that, and let the next step take the next decision. Net code should trend DOWN
(a fact replacing several heuristic branches), not up.

### Build on a mostly-working base — never break it to fix it

The migration takes time, and that time is the accepted cost of **never regressing
a mostly-working language**. Every small step LANDS green: both backends pass, the
suite is clean, the language works at least as well after as before (plus the one
fixed fact). We never take the language down for a multi-step rewrite; we never
leave a backend broken "to be fixed in the next step" (an interp fix that breaks
native compile is NOT landable — both green, or it doesn't land); we never trade a
working base for a half-built better one. **Time is fine; a broken intermediate is
not.** The base is always shippable, and each migration only ever adds correctness
on top of it. This is why the steps are small *and* sequential: a working language
at every commit is the substrate the whole effort stands on.

A fact is "done" when: it is computed once, completely (all shapes/paths); the
heuristic it replaces is deleted; codegen reads it in one place; the parser is no
heavier than before; and the rung's probe is green on both backends with no leak.

## Connections

- [CODEGEN_METHOD.md](CODEGEN_METHOD.md) — the *how* (the diagnostic + the rung discipline)
- [DEPS_INVENTORY.md](DEPS_INVENTORY.md) — the typed `Deps` substrate this builds on
- [LIFETIME.md](LIFETIME.md) — the current dep/scope model
- [STABILITY_HOTSPOTS.md](STABILITY_HOTSPOTS.md) — the forward risk register (ownership-by-shape-analysis is H-tier)
- [plans/85-store-lifetime-retirement/](plans/85-store-lifetime-retirement/) — the first application, incl. `type-ownership-design.md` (the return-ownership fact) and the bytecode rungs
