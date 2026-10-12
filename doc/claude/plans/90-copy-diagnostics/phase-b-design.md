<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN90 phase B — DESIGN: the last-use move-elision (grounded in captured IR)

Scope + reframe: [phase-b-scope.md](phase-b-scope.md). This is the concrete, IR-grounded design
that scope's slice 1 (the loft-codegen gate) produces. Captured bytecode:
[bytecode-comparisons/phaseB-captures.txt](bytecode-comparisons/phaseB-captures.txt) (+ the three
`phaseB-*.loft` probes). Every claim below is read off that capture, not guessed.

## The one invariant (the hypothesis to enforce)

> A construction (`S { f: src }`) or record (`v[i] = e`, `o.f = src`) copy whose **source is a
> named var, provably dead after the copy** is lowered as a **store transfer** — the source's
> existing store *becomes* the field/element (no fresh field store, no element copy, no separate
> source-free). It is **value-identical** to the copy (never observable — @C86: an elision, not a
> semantic), **leak- and double-free-free on both backends**, and a **surviving** source keeps its
> copy untouched.

This is @C86's own last-use rule (`ElidePlan` is exactly this analysis) applied to two shapes it
does not yet cover. The *decision* already exists — the phase-A survival split classifies these as
`move: source consumed`. Phase B adds the **lowering**, not new analysis.

## Grounded: current → target (from the capture)

**Case A — construction, dead source** (`base=[10,20,30]; a=Bag{items:base}; a.items[1]`):

| current (copy-then-free) | target (move) |
|---|---|
| `OpDatabase(a)` — a's store, items **empty** | `OpDatabase(a)` — a's store, items **empty** |
| `OpAppendVector(a.items, base)` — **copy** store#2→store#3 | *(dropped)* |
| `OpFreeRef(a)` — frees a + a.items(store#3) | `OpFreeRef(a)` — frees a + a.items(**= store#2**) |
| `OpFreeRef(__vdb_1)` — frees base(store#2) | *(dropped — store#2 moved into a)* |
| — | **new:** `a.items := base` (transfer store#2's DbRef into the field) |

Net: **−1 store alloc, −1 element-copy, −1 free**; base's store-free *folds into* a's scope-exit
free. **Record case B (`a.items = base`) is worse today — TWO copies** (`base→__p154_rhs→a.items`);
the same transfer removes both. The efficiency target is the literal `Bag{items:[…]}` (one store,
field filled in place, no copy) — the move must match it.

## The chokepoint: one decision, read by three emit sites

`ElidePlan` (`use_analysis.rs`) already carries "this copy is elidable" for the var-buffer idiom,
consumed by `scopes::elide_borrows`/`elide_rewrite`. Phase B extends it: a **`MovePlan`** (or a new
`ElidePlan` variant) for a construction/record copy whose source is a dead named var. The plan is
computed **once** (from the survival `move` fact) and the three lowering sites **read** it — none
re-derives it (the design-protocol re-assertion count = **3**, all reading one fact):

1. **Suppress the field copy** — drop `OpAppendVector(field, src)` / the record-copy `OpCopyRecord`
   (+ the `__p154_rhs` temp + its append/clear for the record case).
2. **Emit the transfer** — `field := src`'s DbRef (the field's slot now holds the source's store
   handle). The exact op: a whole-vector/record **field-set-by-handle** (mirror how the local-var
   slot receives a moved store in the var-buffer elision — `elide_rewrite` re-points reads today;
   here we re-point the *field slot* to the source store). **Crux (the capture exposes it):**
   `OpDatabase(a)` already pre-allocates an **empty** `a.items` store (store#3 in the trace) so the
   current `OpAppendVector` has a target. The transfer must **not orphan store#3** — either alloc
   `a` with the field left NULL and set it to the source store, or **free the empty placeholder
   before adopting** (the exact pattern the #506 adopt-arm used: free the real distinct `_dst`
   before `var = _src`; `generation/dispatch.rs` + `displaced_owned_slots`). Skipping this is the
   F2 leak.
3. **Suppress the source-free** — drop `OpFreeRef(src)` at scope exit; the source store is now owned
   by the field and freed transitively when the container is freed.

Sites (1)+(3) are *removals* keyed on the plan; (2) is the one *addition*. This is the same
ownership-transfer class as the shipped var-buffer `ElidePlan`, so the free-side bookkeeping
(`scopes.rs` scope-exit frees, `displaced_owned_slots`) is the code to teach, not to invent.

## Both generators (the both-backends rule)

Interp (`src/state/codegen.rs` + the `scopes.rs` elide pass) and native (`src/generation/`) are
separate generators reading the same IR. The `MovePlan` is produced once in the post-parse pass
(`scopes::check`, where `elide_borrows` already runs); **both** backends then lower the transferred
field-set. A rung is closed only when BOTH emit the move and pass value+leak+poison. Capture the
native target Rust in slice 1b (the interp capture above is the spec for the shared IR; native's
`generation/` must emit the equivalent store-handle set + the dropped free).

## Boundary matrix (assert value + length + **leak + poison**, both backends)

`{construction S{f:src}, record v[i]=e, record o.f=src} × {source DEAD (must MOVE), source SURVIVES
(must COPY — @C86), source MUTATED after (must COPY), source is a param (never dead — must COPY),
nested field s.a.b = src, source in a loop} × {interp, native}`.

- **DEAD** cells: 0 runtime copies (`LOFT_COPY_DUMP`), value identical, no leak, no double-free.
- **SURVIVES / MUTATED / param** cells: unchanged from today (still copy) — proves phase B touches
  only what @C86 sanctions. These are the falsification cells: if a survivor elides, @C86 is broken.
- **leak** must be checked *with value+length* (a move that drops the field reads leak-free but
  wrong; only value+length catches it).

## Failure modes → the guard that kills each (design-protocol)

| # | failure | why | guard |
|---|---|---|---|
| F1 | **double-free** — source freed by both the transfer-owner and the old `OpFreeRef(src)` | forgot to suppress site (3) | POISON + native leak-check on every DEAD cell; the plan MUST drop the source-free |
| F2 | **leak** — source store transferred but the field-free doesn't reach it | the field slot didn't actually adopt the store | `LOFT_STORES=warn` / `LOFT_NATIVE_LEAK_CHECK`; assert store count returns to base |
| F3 | **survivor elided** — a live source becomes the field's store, later read/mutation now aliases | the dead-source fact was wrong / too wide | the SURVIVES + MUTATED matrix cells (must still copy); this is the @C86 boundary |
| F4 | **one backend moves, the other copies** | the plan lowered in only one generator | both-backends matrix; not landable until both move |
| F5 | **nested / aliased field** (`s.rows[i] = src`) transfers into an aliased slot | the field slot is itself a view | reject the un-transferable case cleanly (fall back to copy — loud, never silent) |

## B1.2 findings (built + verified — the detection, `LOFT_MOVE_ELIDE`)

`MovePlan` + `move_elidable_source` + `dump_move_plans` land in `use_analysis.rs` (gated,
byte-identical off; `analyze_fn` now returns a 3rd `Vec<MovePlan>`, callers unchanged via `.0`/`.1`).
Detection = the survival **move** fact + elision preconditions (not a param; owns a store —
`def_vdb`/`database_vars`). Verified on the probes: fires `make_dead` (Construct, `base`) and
`rec_move` (Record, `e` — a clean `OpCopyRecord(e, v[0])`, item-3 swap confirmed), **silent on both
survivors**. Corpus reach: **37 Construct + 65 Record** move-elidable sites. Two findings the
capture surfaced, to fold into B1.3:

- **Record shape splits.** A clean `v[i] = e` element-set IS one `OpCopyRecord` (source `e`, caught).
  But a **vector-field whole-replacement** `a.items = base` lowers to a **chained copy through a
  temp** — `__p154_rhs += base` (a var-buffer copyfill) then `a.items += __p154_rhs` — so the field
  append's source is `__p154_rhs`, not `base`; B1.2 does not catch it. It needs a **chained-source**
  widening (follow `field += tmp` where `tmp` was copyfilled from a dead owned `base`).
- **Ordering vs `elide_borrows`.** `dump_move_plans` runs in `scopes::check` **before**
  `elide_borrows`. The var-buffer `ElidePlan` inlines `__p154_rhs`→`base`; if move-detection ran
  *after* that inlining, the chained `a.items = base` would collapse to `a.items += base` and be
  caught by the plain Construct rule. So B1.3's lowering ordering (move-elide relative to
  var-buffer elide) is a real design knob — likely: run var-buffer elision first, then move-elide
  on the collapsed IR.

## B1.3 mechanism (investigated — the transfer primitive; the loft-codegen gate for the lowering)

Captured the ops the move rewrite must emit/drop, and found the one missing primitive:

- **The move IS an `adopt`** — write the source's `DbRef` into the destination slot; the source
  stops owning the store. The runtime already does this for **var slots**: `OpPutRef`
  (`fill.rs::put_ref` = `put_var(pos, dbref)`) and `bind_or_copy`'s OWNED arm (`mut_var = src`,
  io.rs:1624). This is the mechanism the var-buffer `ElidePlan` transfer uses.
- **But every field-write today COPIES** — `a.items = base` → `OpClearVector` + `OpAppendVector`
  into the field's *existing* store; `s.i = fresh` → `OpCopyRecord` into the existing field store.
  **No op writes a DbRef into a struct FIELD slot** (`OpPutRef` targets a var *position*, not a
  field offset). So the field cannot adopt a source store today.
- **`OpDatabase(a)` leaves `a.items` an (empty) vector store** that `OpAppendVector` fills — the
  placeholder. The move must **free-before-adopt** it (the #506 adopt-arm pattern), or construct
  `a` with the field left null and adopt into it.

**⇒ B1.3 needs a new `OpAdoptField(container, field_off, src, tp)` op** (interp `fill.rs` +
`state/io.rs`, native `generation/ops/`): free the container's current field store if distinct
from `src`, then write `src`'s `DbRef` into the field slot; `src` is no longer freed separately.
The rewrite then, for each `MovePlan`: (1) drop the field copy (`OpAppendVector(field, src)` /
`OpCopyRecord`); (2) emit `OpAdoptField(container, off, src, tp)`; (3) drop `OpFreeRef(src)` (the
store is now the field's, freed when the container is). The Record `v[i]=e` case adopts into an
*element* slot (`OpGetVector` target) rather than a field — likely a sibling `OpAdoptElem` or a
unified `OpAdopt(dest_ref, src)` taking any destination `DbRef`. **Cleanest: one `OpAdopt(dest, src)`
that takes the destination as a `DbRef` (from `OpGetField`/`OpGetVector`) — mirrors how the copy
ops already take their destination — so construction and record share one op.** Prove that op on a
hand-constructed `make_dead`/`rec_move` (value+leak+poison, both backends) BEFORE wiring the
rewrite (the loft-codegen gate).

## B1.3 CORRECTION (the gate caught it) — the representation splits the lowering in two

Attempting to build one `OpAdopt(dest, src)` surfaced a heap-safety fact that **reframes B1.3** —
recorded here because it is exactly the corruption the loft-codegen gate exists to prevent:

**`DATABASE.md`: `Vector(T)` is a BY-VALUE array — elements live INLINE in the vector's element
block** (`Array(T)` is the by-reference one). Consequences for the two `MovePlan` kinds:

- **Record `v[i] = e` (`vector<E>`, E a struct) — the element is INLINE.** `do_copy_record` copies
  `e`'s record *into* the inline slot (`store(&to).copy_block`). There is **no separate element
  store to adopt** — a DbRef-swap here would point the inline slot at foreign memory = corruption.
  The right lowering is **INLINE CONSTRUCTION**: build `e` DIRECTLY into `v[i]`'s inline slot (skip
  `e`'s own `OpDatabase` + the copy + the free), i.e. retarget `e`'s construction ops to the element
  slot — the var-buffer `elide_rewrite` pattern, no new op.
- **Construction `S { f: base }` where `f` is a `vector`/collection FIELD — the field holds a
  HANDLE** to a separate element block. THIS one adopts: swap `a.items`'s handle to `base`'s block,
  free the empty placeholder block, drop `base`'s free. An `OpAdopt`-style handle-write fits here.
- **`S { f: base }` where the source is BUILT IN PLACE** (a literal / append-built `base`) can
  ALSO be inlined (retarget `base`'s fill to `a.items`), often simpler than an adopt; **only a
  source whose store comes from a CALL** (`base = mk()`) genuinely needs the handle-adopt (its store
  can't be retargeted).

**⇒ Revised B1.3 shape:** the move-elision is **retarget-the-source-construction** (inline) as the
primary mechanism — it covers both the inline-element record case and the in-place-built
construction case, reuses the `elide_rewrite` machinery, and needs no new heap op. The
**handle-adopt** (`OpAdopt` on a collection-field handle) is the SECOND mechanism, needed only for a
**call-sourced** handle field. The `MovePlan` should therefore carry the source's **origin**
(in-place-built vs call-result) and the destination's **representation** (inline slot vs handle) so
B1.3 picks the correct lowering — a detection widening for B1.2b, before any lowering lands.

This is the gate working: the naive single-`OpAdopt` would have corrupted every `v[i] = e`
(inline-element) move. Do NOT implement a heap op until the inline-vs-handle case is proven per
destination (value + length + **leak + poison**, both backends) on a hand-constructed cell.

## B1.3 LANDED — the RECORD-shape inline-retarget rewrite (`move_elide`, both backends)

Built as a pure **IR rewrite** in `src/scopes.rs::move_elide` (mirrors `elide_borrows`), run in
`check` after borrow-elision, gated on `LOFT_MOVE_ELIDE`; **no new op** — so both backends lower
the retargeted `OpSet*` via existing support. Covers the **Record** `MovePlan` kind (`v[i] = e`,
`o.f = src`); the **Construct** kind is filtered out (deferred — needs a build-order reorder, see
below). Per dead-after source `s`:

```text
  OpDatabase(s)                          ── dropped
  OpSetInt (s, off, v)  ── retarget ──▶  OpSetInt (dest, off, v)   (dest = the OpCopyRecord target)
  OpSetText(s, off, v)  ── retarget ──▶  OpSetText(dest, off, v)
  OpCopyRecord(s, dest)                  ── dropped (the deep copy is gone)
  set_skip_free(s)                       ── the later variables() pass emits no scope-exit free
```

Two findings that the empirical matrix (value + poison + leak, both backends) settled — each had
been a theorised blocker:

- **The old-content-free is handled by `OpSet*` itself.** Retargeting `OpSetText(dest, …)` onto a
  slot that already holds a heap text (`v[0].name` = a 320-byte store) does **not** leak the old
  text — `OpSetText` frees the field's prior content when it overwrites. So the inline retarget
  needs no explicit `remove_claims(dest)`. (Proven: the 320-byte heap-text cell is leak-clean.)
- **The source free is inserted by a LATER pass**, not present when `move_elide` runs — so dropping
  `OpDatabase(s)` left a `variables()`-emitted `OpFreeRef(s)` on the now-null slot (a harmless
  free-of-null, but fragile). `set_skip_free(s)` suppresses it at the source of the fact.

**Validated (both backends, `LOFT_POISON=1` + `LOFT_NATIVE_LEAK_CHECK=1`):** element set,
field replacement (`o.inner = src`), two-moves-in-one-fn, nested-record source — all value-identical
+ leak-clean; the survivor (`e` read after the set) still COPIES; the Construct shape stays a copy.
Guards: `tests/use_analysis.rs::move_elide_record_*` (behavioural both-backend + IR off-vs-on copy
count). OFF is a no-op (early return) → suite byte-identical.

**Deferred to B1.3b — the Construct shape (`S { f: src }`).** Its source is built BEFORE the
container exists, so the retarget needs a build-order reorder (or the handle-adopt for a
call-sourced collection field). Filtered out of `move_elide` for now; detection still fires (the
`MOVE-PLAN … kind=Construct` dump), so B1.3b has its worklist.

### B1.3b design (grounded — `make_dead` capture, `a = Bag { items: base }`)

Current IR: `base`'s backing `__vdb_1` is `OpDatabase`'d + filled (`OpPreAllocVector(base)` +
element `OpNewRecord/OpFinishRecord`), THEN `a` is `OpDatabase`'d, THEN
`OpAppendVector(a.items, base)` **copies** base's block into `a.items`, then `OpFreeRef(__vdb_1)`.
Two things differ from the Record case:

1. **Reorder.** `base` is built before `a` exists. To retarget base's fill into `a.items`, `a`'s
   `OpDatabase` + items-init must move BEFORE base's construction.
2. **The retarget is over the vector-BUILD ops** (`OpPreAllocVector` / `OpNewRecord` /
   `OpFinishRecord` — the element append machinery), not the scalar `OpSet*` the Record case had.

**Two candidate lowerings, ranked:**
- **(A, recommended) reorder-inline retarget** — hoist `a`'s alloc above base's fill, retarget
  `OpPreAllocVector(base)`+element-ops onto `a.items`, drop `__vdb_1` + its free + the
  `OpAppendVector` copy. **Reuses the retarget idiom (extended to the vector-build ops), no new heap
  op, no handle-representation risk** — same safety envelope as the landed Record rewrite. Cost: the
  reorder mutation (move a's alloc-block earlier) and matching a wider op set.
- **(B) handle-adopt** — swap `a.items`'s handle to `__vdb_1`'s element block, free a's empty items
  placeholder, drop the `__vdb_1` free. More surgical but needs a **handle-write primitive** AND a
  proven model of how a `vector<T>` FIELD references its block (inline-length vs separate handle) —
  the representation the gate insists on proving on a hand-built poison cell before any heap op.

**Plan:** do (A) — extend `move_rewrite` to (i) recognise the Construct `MovePlan`, (ii) hoist the
container's alloc block, (iii) retarget the source's `OpPreAllocVector`/element-build ops onto the
field, (iv) drop the wrapper alloc/copy/free + `set_skip_free`. Matrix the same grid (value + poison
+ leak, both backends) on `make_dead`/`keep_alive`. Reserve (B) only for a **call-sourced** field
(`items: mk()`) where there is no in-place fill to retarget — and prove the field representation
first. Also fold the `a.items = base` whole-replacement (chained through `__p154_rhs`) here.

### B1.3b LANDED — the REORDER-FREE field-append (`x.field += src`, both backends)

Built as `src/scopes.rs::construct_move_rewrite` (a sibling of the Record rewrite). It splits the
Construct kind by the one axis that makes (A) safe WITHOUT a reorder: **does the destination
container already exist when `src` is built?**

- **Reorder-free (`x.field += src`, container is a param OR built before `src`)** — the case
  handled. `src` is a view over its backing wrapper `vdb` (`src = OpGetField(vdb, …)`); retarget
  `src`'s element builds (`OpNewRecord`/`OpFinishRecord`) directly onto `x.field` (= the
  `OpAppendVector` destination), and drop `vdb`'s alloc/len-init/free, `src`'s view-def, `src`'s
  capacity `OpPreAllocVector` (omitted — `x.field` already exists; retargeting it would reset the
  length), and the `OpAppendVector` copy. `set_skip_free(vdb)` (the wrapper is the owned store;
  `src` is its borrow). **No new op** — the retargeted `OpNewRecord` grows `x.field` exactly as the
  copy's `vector_add` did (proven: value + poison + leak clean, both backends).
- **Fresh construction (`a = Bag { items: base }`, container built AFTER the source)** — still a
  COPY. **The reorder guard** (`construct_move_rewrite`: the container's `OpDatabase` must precede
  the source-backing's, tracked by encounter order; a param has none → always before) SKIPS it, so
  it never fires unsafely. This is the residual for B1.3c (needs the build-order reorder, whose
  hazards — a's alloc and the copy share one construction Insert; other fields may depend on the
  source — are why it is split off).

**Validated (both backends, `LOFT_POISON=1` + `LOFT_NATIVE_LEAK_CHECK=1`):** local-container append,
param-container append, two-appends-in-one-fn — all value-identical + leak-clean; the survivor still
copies; fresh construction stays a copy (guard fires). Guards:
`tests/use_analysis.rs::move_elide_construct_*`. OFF byte-identical (early return).

#### The guard the list was missing — how OFTEN the append runs (loft#1243)

Every guard above asks a DATA question: does the source escape, is the destination disturbed or
cleared, is the container allocated first, is the destination unique. The prescan walks the whole
body, so it finds the `OpAppendVector` at any depth — and none of the guards asked whether that
append is reached the same number of times as the build it is being folded into. It is not, in two
directions, and the answers were wrong on both backends with nothing said:

```loft
for _i in 0..3 { d.c += s; }     // grew d.c by ONE copy, not three — at every iteration count
if <false> { d.c += s; }         // appended anyway
```

The fold leaves the element builds where the source is BUILT and drops the append, so the program
it produces matches the one written only where the two run together. The invariant is
[`formal/ownership.md`](../../formal/ownership.md) **O-Latest** — ownership is a property of the
latest assignment *at the loop depth at which that assignment was taken*, and no type-level fact
carries that depth, so the site has to measure it.

`construct_prescan` therefore carries a **control-flow path**: the chain of enclosing regions that
run zero, one or many times — a loop body, an `if` arm, a parallel arm, an iterator's step — each
with its own id. It records the path for the source's `OpDatabase` and for the append, and the fold
is admitted only when the two are **equal**. Equality of the whole path, not of its depth: two arms
of one `if` are equally deep and never run together.

Declining here is the same answer the design already gives the reorder cases, so it costs the
optimisation and never correctness. Guard:
`tests/scripts/an-appended-source-is-built-as-often-as-the-append-runs.loft`, whose two control
cells (a straight-line append, and a source declared INSIDE the loop) fail any fix that simply
switches the fold off.

#### A retargeted source is erased, and its borrowers must say so (loft#1241)

The fold removes the wrapper alloc, the view-def and the append, so after it the IR holds no `Set`
for `src` at all. What still named it was a stale `deps`: the element work-ref whose builds had
just been re-pointed onto `x.field` went on saying its store belonged to `src`. A dep is the
statement "my store belongs to that variable" (**O-Deps**), so leaving it wrong made the ownership
derivation name a variable that owns nothing — and made the scope pass declare `src` so a borrower
could name it, which handed the erased local a stack slot no instruction writes. That slot is what
@PLN120 A's store-span check reported.

`construct_move_rewrite` now reports which container each source was retargeted into, and every var
that borrowed the source is re-pointed there. The work-ref reads `deps=[container]` exactly as its
siblings do, `src` is named by nothing, and no declaration, interval or slot follows. Guard:
`tests/frame_vars.rs::a_retargeted_append_source_is_not_a_runtime_local` — the slot in an ordinary
build, since the check itself exists only under `-C debug-assertions=on`.

**Why the reorder-free split (not the full reorder) first:** the reorder-free case is a pure
retarget (the Record machinery, extended to the vector-build ops) — representation-safe, no code
motion. It captures the field-append subset of Construct now; the fresh-construction reorder (B1.3c)
is a distinct, higher-risk slice deferred behind the guard, exactly per the "prove the safe path,
gate the rest" discipline that caught the `OpAdopt` corruption.

### B1.3c LANDED — the FRESH-construction reorder (`a = Bag { items: base }`, both backends)

Built as `src/scopes.rs::construct_fresh_rewrite`, run AFTER B1.3b on the copies it left standing.
The structure dump settled the mechanism: **make_dead's function body is a FLAT top-level block** (a's
alloc and base's build are siblings in one operator vec), and **a's construction is a contiguous run
of statements immediately before the copy**. So the reorder is a flat-vec rebuild, not a nested-Insert
split:

```text
  [base build → __vdb_1] … [a alloc+init] [OpAppendVector(a.items, base)]   (before)
  ─────────────────────────────────────────────────────────────────────
  [a alloc+init] [base build → RETARGETED onto a.items, __vdb_1 dropped] …  (after)
```

Retarget `base`'s build ops (`OpPreAllocVector`/`OpNewRecord`/`OpFinishRecord`) onto `a.items`;
here `OpPreAllocVector` IS retargeted (the fresh field is empty, so pre-claiming its capacity is
correct — the field-append case dropped it because the field was non-empty). Drop the wrapper
(`__vdb_1`) alloc/len-init + `base`'s view-def + the copy; `set_skip_free(__vdb_1)`.

**Conservative guards (fire only when ALL hold — else stays a copy, safe under flip):** flat
top-level block; exactly ONE still-copying construct source in the function; `a`'s construction is a
contiguous run right before the copy that references **only `a`** (dependency-safe to hoist past
`base`'s build — a param/computed field value like `Bag { id: n, items: base }` references another
var → SKIP); the run contains `a`'s `OpDatabase`; and `a` is allocated AFTER the source backing (a
real reorder). Validated (both backends, poison + leak): `make_dead`, single-literal-field fresh
construct → elide; survivor, param-field-value, and two-fresh-constructs-in-one-fn → correctly stay
copies. Guards: `tests/use_analysis.rs::move_elide_fresh_*` +
`move_elide_construct_elides_field_append_and_fresh_construction`.

**Residual widenings (future, all SAFE-skipped today):** param/computed field values (allow run vars
that are parameters — they exist at entry, so hoisting is safe); >1 fresh construct per function;
nested-Insert (non-flat) bodies. Each is a guard relaxation, not a new mechanism.

### B1.4 LANDED — param field values in the hoisted run (the interprocedural soundness line)

The B1.3c run guard was "references ONLY `a`" — which skipped the common `Bag { id: n, items: base }`
(a param field value). B1.4 relaxes it to **`a` or a never-written parameter**: a param's value is
constant, so hoisting `a`'s construction past `base`'s build reads the same value.

**The soundness line — and the trap the review caught.** "Never written" MUST be interprocedural.
loft's `&`-param passes a param by reference (`add(100, n)` where `add(x, y: &integer)` mutates the
caller's `n`), and the param can sit in ANY argument position — so a naive "Set-target or Call
first-arg" write-scan MISSES an arg1 `&`-mutation and would hoist a stale read (a wrong VALUE, not
just a missed optimisation). The fix reuses `parser::find_written_vars` (the existing
interprocedural mutation analysis, aware of callee `&`-param signatures) computed in `move_elide`
and threaded into `construct_fresh_rewrite`; a run var is allowed iff `is_argument(v) &&
!written.contains(v)`.

**Validated (both backends, poison + leak):** `Bag { id: n, items: base }` with `n` never written →
elides; `n` reassigned (`n = n*10`), and `n` mutated via a callee's arg1 `&`-param (`add(100, n)`) →
correctly stay copies (a wrong hoist would have surfaced as a stale value, e.g. `6` instead of
`106`). Guards: `tests/use_analysis.rs::move_elide_param_field_widening_is_sound`.

### B1.3d LANDED — `a.field = base` whole-vector replacement (the double-copy idiom)

`a.field = base` is the most wasteful copy shape — the compiler lowers it (via the `__p154_rhs`
temp) as a DOUBLE copy, with a clear between:

```text
  <build base into __vdb>
  __p154_rhs = null; OpAppendVector(__p154_rhs, base);   copy 1: base → temp
  OpClearVector(a.field);                                clear a.field's old contents
  OpAppendVector(a.field, __p154_rhs);                   copy 2: temp → a.field
```

`base`'s copy target is a TEMP, so `base` is not a `MovePlan` (the survey never saw it — it reports
`0` here). `construct_replace_rewrite` therefore detects the idiom **structurally**: an
`OpAppendVector(a.field, Var(rhs))` (copy 2) immediately preceded by `OpClearVector(a.field)` (same
field) and `OpAppendVector(Var(rhs), Var(base))` (copy 1). Because it is not plan-driven, it runs
UNCONDITIONALLY (the empty-plan early-continue in `move_elide` was removed).

When `base` is a **dead-after local** — it owns a backing wrapper (`base = OpGetField(vdb, …)`), is
NOT referenced after copy 2 (else the moved store would dangle), and is NOT a param — the rewrite
moves the `OpClearVector` ahead of `base`'s build, retargets `base`'s build ops onto `a.field`, and
drops both copies + the temp's alloc + the wrapper (temp + wrapper `skip`-freed). This eliminates
**two** copies and a wrapper allocation.

**Validated (both backends, poison + leak):** an integer-vector replacement and a **heap-text**
replacement (`d.lines = fresh` where `d.lines` held `["old1","old2"]`) — both elide; the old
heap-text contents are freed by the moved-up `OpClearVector` (no leak — the old-content-free stress
that the record case also passed). A surviving source (`base` read after) and a param source stay
copies. Multiple replacements per fn handled (loop, like the fresh case). Guard:
`tests/use_analysis.rs::move_elide_whole_vector_replacement_eliminates_double_copy`.

## Verifiable slices (each: matrix on BOTH backends, gated behind a flag, suite byte-identical off)

- **B1.1 — gate (DONE for interp).** The capture above is the spec; add the native target Rust
  beside it. Prove the WORKING move by hand-writing/patching one case and running it value+leak
  clean on both backends before touching the generator.
- **B1.2 — the plan.** `MovePlan { field-target, source-var, container }` produced in the post-parse
  pass from the survival `move` classification (source = a dead named var; NOT a param; NOT
  mutated-after; NOT in a loop where the source outlives the body). Gate on `LOFT_MOVE_ELIDE`
  (default off). No lowering yet → suite byte-identical; dump the plan to prove detection matches
  the survey's `move` rows exactly (positive control: fires on `make_dead`, silent on `keep_alive`).
- **B1.3 — RECORD-shape lowering (DONE, both backends).** `move_elide` retargets the source's
  construction ops onto the copy destination + drops the alloc/copy/free (`set_skip_free`). Landed
  as ONE IR pass (no new op) → interp AND native from the same rewrite; the B1.4 native split is
  therefore unnecessary for this shape. Matrix-validated (value + poison + leak). See "B1.3 LANDED".
- **B1.3b — CONSTRUCT field-append (DONE, both backends).** `x.field += src` where the container
  already exists: `construct_move_rewrite` retargets `src`'s element builds onto `x.field` + drops
  the wrapper/copy/prealloc/free, guarded reorder-free (container's `OpDatabase` precedes the
  source-backing's, or the container is a param). No new op. See "B1.3b LANDED".
- **B1.3c — CONSTRUCT fresh construction (DONE, both backends).** `a = Bag { items: base }`,
  container built after the source. `construct_fresh_rewrite` does the build-order reorder on a flat
  block (a's construction is a contiguous run right before the copy — hoist it, retarget base's build
  onto a.field, drop the wrapper/copy). Conservatively guarded (flat block, one construct, run refs
  only `a`, real reorder); everything else stays a copy. No new op. See "B1.3c LANDED".
- **B1.4 — param field values + multiple constructs (DONE).** (a) Allow a never-written parameter
  as a hoisted run value, soundness on the interprocedural `find_written_vars` (catches `&`-param
  mutation in any arg position). (b) Lift the one-construct-per-fn cap: `construct_fresh_rewrite`
  loops, rewriting each safe construct by earliest remaining copy (re-scan after each; a failed
  source is recorded for termination) — a cross-construct dependency is a non-`a`/non-param run var,
  so it self-SKIPs. See "B1.4 LANDED".
- **B1.3d — `a.field = base` whole-vector replacement (DONE).** The `__p154_rhs` DOUBLE copy
  (`base → __p154_rhs → a.field`, with an `OpClearVector`). `base`'s copy target is a temp, so it is
  NOT a MovePlan — `construct_replace_rewrite` detects the idiom STRUCTURALLY (copy2 = append temp
  into field; preceded by clear-of-field; preceded by copy1 = append base into temp). When `base` is
  a dead-after local (its own `__vdb` backing; not referenced after copy2; not a param), it moves the
  `OpClearVector` ahead of `base`'s build, retargets `base`'s build onto `a.field`, and drops both
  copies + the temp + the wrapper (temp + wrapper `skip`-freed). Runs UNCONDITIONALLY (not gated on
  a MovePlan). See "B1.3d LANDED".
- **Nested bodies (DONE).** Both reorder-based rewrites (`construct_fresh_rewrite`,
  `construct_replace_rewrite`) now WALK every block, applying the per-block reorder to nested `if`/
  loop-body blocks too — a construct's `a`-alloc + `base`-build + copy are flat within whatever block
  they live in. Validated: fresh + replace inside an if-branch, and a fresh construct in a loop body
  (per-iteration correct); an outer-local / loop-var field value stays a copy (the run-guard only
  admits `a` or a never-written param). Guard: `move_elide_handles_constructs_in_nested_blocks`.
  The move-elision now covers all four copy shapes in flat AND nested positions.
- **B1.5 — FLIPPED (default-on).** The move-elision REWRITE is now DEFAULT ON; `LOFT_NO_MOVE_ELIDE`
  restores the copy. The `MOVE-PLAN` dump was split onto its own opt-in gate (`LOFT_MOVE_ELIDE`,
  `move_elide_dump_enabled`) so the default-on rewrite doesn't spew plan lines. Reaching the flip
  took a **flag-ON exposure sweep** — running the whole corpus with the rewrite on, which the ~30
  hand-probes had never done. It found ~12 corpus bugs (all the same class: retargeting a source's
  build into a destination that isn't a stable, pre-existing, single-def, user-owned slot, OR that
  isn't dead between build and copy) — all fixed via the guard layer (`bad_containers` = element ∪
  reassigned; `def_order` container-before-source; `source_escapes` dead-between-build-and-copy;
  self-read; replace-vs-append; compiler-temp / bare-Var dest). The behavioural corpus is green
  default-on (issues 748, leak 49, native, wrap, native_scripts, loft_suite); the remaining flag-ON
  sweep reds were dump-pollution (fixed by the gate split) + wasm/browser env. **Lesson: a flag-ON
  full-corpus sweep is MANDATORY before flipping any heap rewrite default-on — narrow probes
  massively under-cover.**

## Falsification probes (cheapest thing that could break each load-bearing claim)

1. *Claim: the source-free folds into the container's free (no leak, no double-free).* Probe: the
   DEAD construction cell under `LOFT_POISON=1` + `LOFT_STORES=warn` — must be clean. If it
   double-frees, site (3) suppression is wrong; if it leaks, the field didn't adopt the store.
2. *Claim: a survivor is untouched.* Probe: `keep_alive` (base read after) must STILL emit the
   copy (`LOFT_COPY_DUMP` shows the append) — value-identical to today. A silent 0-copy here = @C86
   broken.
3. *Claim: the decision is the phase-A `move` fact, not a new re-derivation.* Probe: the B1.2 plan
   dump must be exactly the survival split's `bucket=implicit [move: source consumed]` rows — no
   more, no fewer. A divergence means phase B re-derived (the wrong chokepoint).
4. *Claim: value-identity across backends.* Probe: the differential oracle (`tests/oracle/`) over
   the matrix — interp and native must agree on stdout/exit/leak for every cell.

## Do-not-ship (revert, don't push through)

Any DEAD cell double-frees/leaks (F1/F2); a SURVIVES/MUTATED cell elides (F3, @C86 broken); one
backend moves and the other copies (F4); the plan diverges from the survival `move` rows (re-derivation).

## Why this is safe to build on (per the "do the complexity now" rule)

The move-elision is the store-lifetime engine's **positive** direction (transfer a store INTO a
container), the mirror of @PLN85 P4's borrow-return (borrow a field OUT —
[borrow-return/DESIGN.md](borrow-return/DESIGN.md)). Landing it makes the survival `move` fact
*load-bearing* (not just diagnostic), which is the foundation the rest of phase B / the @C86 elision
completeness builds on — and it retires a real, measured cost (every dead-source construction /
record set currently copies-then-frees).
