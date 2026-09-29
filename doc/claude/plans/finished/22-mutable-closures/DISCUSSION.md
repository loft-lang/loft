<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# MUTABLE_CLOSURES_DISCUSSION — open issues, alternatives, analysis sketch, history

**Status:** companion to [README.md](README.md).
This file holds:

- The implementation-analysis sketch the spec depends on (the
  algorithm-level walkthrough of how the compiler should
  detect, classify, and report on mutating closures).
- Alternatives considered, with the trade-offs that led to the
  locked-in design.
- Open questions that don't change the spec but need resolution
  before or during implementation.
- Design-history notes — including refinements made through the
  conversation that produced the spec.

The locked-in design lives in `README.md`.  This
document is its counterpart for design conversation; settled
items move from here into the spec.

---

## Why this work — the novice cliff

The
[novice-readiness evaluation in EVENT_LOOP_DISCUSSION.md](../../32-event-loop/DISCUSSION.md#novice-readiness-evaluation-2026-05-05--pivot-trigger)
identified closure capture by value (loft's
[C38](../../../DESIGN_DECISIONS_OWNERSHIP.md#c38--closure-capture-is-copy-at-definition))
as the dominant blocker for novice game programmers.  Every
novice's first instinct, on every game framework they've ever
used:

```loft
let state = GameState { score: 0, ... };
el::on(loop, fn(e: ClickEvent) {
    state.score += 1;        // ← will not work today
});
```

In JavaScript, Python, Lua, and C# closures, `state` is captured
by reference (or via a hidden environment record), so the
mutation persists.  In loft today, `state` is captured by value
at the moment the closure is defined; mutations stay inside the
closure and do not affect the outer `state`.

Every other novice pain point with the EventLoop (no `connect()`
helper, no examples, undefined disconnect path) can be solved
with documentation or thin loft wrappers.  The closure-capture
cliff is structural: it appears in the very first handler the
novice writes, and no amount of library polish hides it.

Hence the pivot: revisit the closure model *before* shipping
EventLoop.

---

## Alternatives considered

The locked-in design (case classification A/B/C/D, implicit by
body, lowerings via `Reference<T>` and hidden cells) was reached
by surveying six options.  Recording them so they don't get
re-litigated.

Each option is graded on:
- **Novice fit** — does the JS/Python/Lua intuition Just Work?
- **Implementation cost** — small / medium / large.
- **Expressiveness** — covers mutable scalars, mutable struct
  fields, vectors, references that escape scope?
- **Risk to existing code** — does this break or reshape the
  shipped stdlib + game examples?

### A. Status quo — `Mutable<T>` stdlib helper as the only path

Novice writes:

```loft
let score = Mutable::new(0);
el::on(loop, fn(e: ClickEvent) {
    score.set(score.get() + 1);
});
```

- Novice fit: poor.  Every mutable scalar needs explicit wrapping.
- Implementation: small (~30 lines stdlib + parser diagnostic
  pointing at it).
- Expressiveness: full.
- Risk: zero.

**Outcome:** retained as a fallback for case D (explicit shared
ownership) but rejected as the *primary* path.  The novice cliff
is reduced — no compiler error — but the ergonomic shape is still
unfamiliar.

### B. Auto-Reference for captured user types

Closures capturing a `struct` or `enum` value automatically
capture a `Reference<T>` instead of snapshotting.

- Novice fit: excellent for the common "game state struct" case.
- Implementation: medium.
- Expressiveness: covers user types; mutable scalars need
  separate machinery.
- Risk: medium (changes capture semantics for ALL user-type
  captures, not just mutating ones).

**Outcome:** kept as the *lowering* for mutating user-type
captures, gated by the case classifier so it kicks in only when
needed.

### C. `let mut` auto-wraps to a hidden cell

Mutable scalar bindings (`let mut x = ...`) become an implicit
1-field cell; closures capturing them capture a Reference to
the cell.

- Novice fit: excellent.
- Implementation: medium.
- Risk: medium if always applied; small if scoped to
  closure-captured mutables.
- Side note: loft does not currently have `let mut` as syntax
  (all locals are reassignable; no mutability marker).  The
  auto-cell is therefore conceptual — applied to any captured
  scalar that is mutated.

**Outcome:** kept as the *lowering* for mutating scalar captures,
gated by the case classifier.  No `let mut` syntax introduced;
the analysis triggers on body mutation regardless.

### D. Explicit `cell<T>` — Rust's `Cell` / Python's `[x]` trick

```loft
let count: cell<integer> = cell::new(0);
el::on(loop, fn() { count.set(count.get() + 1) });
```

Equivalent to A under a different name.  Novice still typed
`cell::new()` and saw the wrapper.  Eliminated.

### E. Capture-mode syntax — Rust's `move` keyword family

```loft
fn(e) { ... }       // current value-snapshot
ref fn(e) { ... }   // capture-by-reference
mut fn(e) { ... }   // capture-by-mutable-reference
```

- Novice fit: poor — vocabulary cliff, novice doesn't know which
  mode to pick.
- Implementation: small to medium.

**Outcome:** eliminated as standalone.  Considered briefly as a
power-user override on top of an implicit-by-body default, then
dropped after the user clarified: *"I am unwilling to add a new
keyword here; that would not be a 'loft' thing to do."*

### F. Full Rust-style closure model

`&T` / `&mut T` capture, `FnOnce` / `FnMut` / `Fn` capability
hierarchy, lifetime annotations on captured references.

- Novice fit: variable.  When it works, novice-friendly.  When
  it doesn't, borrow-checker errors are catastrophically
  novice-hostile.
- Implementation: huge.  Touches every function signature, type,
  IR pass, and FFI.
- Risk: high.

**Outcome:** eliminated as a near-term option.  Stays as the
recorded long-term direction in
[C38](../../../DESIGN_DECISIONS_OWNERSHIP.md#c38--closure-capture-is-copy-at-definition).

### G. Co-scoped mutable capture — adopted and refined

The user's insight (2026-05-05): mutable capture is unsafe only
when the closure can outlive the data it points at.  If the
compiler can prove the closure cannot escape the capture's
lifetime, mutation is safe by construction — no GC, no borrow
checker, no runtime check.

Subsequent refinement (also user, 2026-05-05): there are
**two** distinct safe cases, not one.

> *"There are in fact two distinct cases where mutability is not
> a problem: (1) when everything stays in scope, (2) where the
> closure escapes the scope but the local variables do not need
> to read back the data.  There are in-between problematic
> cases."*

Case (1) became case **B** (co-scoped mutating).  Case (2) became
case **C** (moved mutating, gated by liveness).  The "in-between
problematic cases" became case **D** (aliased mutating, rejected).

This refinement is what produced the four-case classification in
the spec.  The two safe cases plus the rejection class together
cover the design space.

### Combined direction

The locked-in spec uses **G as the framing rule** with **B and C
as the lowerings**, all gated by **implicit-by-body** detection
(no keyword, body declares intent), with case classification by
**escape + liveness analysis**.

This combined direction:
- Addresses the novice cliff (the JS/Python pattern works without
  ceremony).
- Preserves backward compatibility (existing code unchanged;
  closures that don't mutate captures keep today's semantics).
- Keeps the heap model loft already has (no GC, no borrow
  checker).
- Introduces zero new vocabulary.
- Uses existing dep-tracking + variable-liveness infrastructure
  for the safety check.

---

## Trade-off comparison

| Option | Novice fit | Impl cost | Expressiveness | Risk to existing code |
|---|---|---|---|---|
| A — `Mutable<T>` stdlib | Poor | Small | Full | None |
| B — Auto-Reference for user-type captures | Excellent (structs) | Medium | Hot path | Medium if blanket |
| C — `let mut` auto-wraps to cell | Excellent (scalars) | Medium | Hot path | Medium if blanket |
| D — Explicit `cell<T>` | Poor (= A) | Small | Full | None |
| E — Capture-mode syntax | Poor (vocabulary cliff) | Small-medium | Full | Low |
| **G — Co-scoped + moved + aliased classification, implicit by body (B+C lowerings under classifier)** | **Excellent** | **Medium** | **Cases A/B/C; D rejected with clear diagnostic** | **Zero (no keyword; only mutating bodies are affected, and those error today)** |
| F — Full Rust borrow checker | Mixed | Huge | Full | High |

---

## Analysis sketch — escape detection and case classification

The spec depends on a compiler analysis the doc previously left
under-specified.  This section walks through the analysis at
algorithm level: how the compiler detects mutating closures,
classifies them into cases A/B/C/D, and emits the case-D
diagnostic.

### Where the analysis lives

A single forward pass, run after closure parsing finishes for a
given function body and before scope analysis exits the
enclosing fn.  For each closure, it produces:

- A **mutated-captures set** (the subset of `captured_names`
  the closure body writes to).
- A **case classification** (A / B / C / D).
- A **diagnostic** if classification is D.

### Phase 1 — Mutated-captures collection

For each closure body, walk the IR looking for writes whose
root target is a captured binding.

After lambda parsing synthesises the closure into a
`__closure_<n>` struct (`src/parser/vectors.rs:760-783`),
captured-binding reads and writes are encoded as `OpGetX` /
`OpSetX` opcodes on the closure struct's fields.  The pass
walks the body and collects:

- `Call(OpSetInt | OpSetByte | OpSetShort | OpSetFloat |
  OpSetSingle | OpSetEnum | OpSetText, [closure_param_var,
  field_id, value])` — direct field writes through the closure
  param.
- `Call(OpAppendVector | OpAppendText | OpAppendCharacter,
  [closure_param_var, field_id, value])` — compound assigns
  desugared by `compute_op_code()` (`src/parser/operators.rs:246-262`).
- `Call(OpClearVector | OpClearText, [closure_param_var,
  field_id])` — pre-write clears.
- `Call(OpInsertVector | OpRemoveVector, ...)` — collection
  mutations.
- `Set(slot, ...)` where `slot` corresponds to a captured-binding
  variable rebinding (whole-binding reassignment).
- Function/method calls of `Impure(ParentWrite)` callees where
  the first argument's root is a captured binding (existing
  purity machinery in `src/data.rs:1327-1364, 1580`).

For uncertain cases (method/function calls with `Unknown`
purity), the pass marks the capture as "potentially mutated"
and proceeds with the conservative assumption.

**Output:** for each closure, a side-table entry
`{ closure_id, mutated_captures: Vec<(name, defining_scope_id)> }`.

### Phase 2 — Case classification

For each closure with non-empty `mutated_captures`:

```
1. Determine destination_scope_id from how the closure value flows:
     - assigned to a let-binding: scope of that let.
     - stored in a struct field: scope of the struct's allocator.
     - passed to a function: depends on the function's storage
       intent (annotation; see open questions).
     - returned from the enclosing fn: caller's scope (>
       enclosing fn's scope).

2. If destination_scope ⊆ each mutated_capture.defining_scope:
     case = B (co-scoped).  Lower captures as Reference<T>.
     Done.

3. Else (destination_scope > some mutated_capture's scope):
     For each mutated_capture, run liveness check:
       Is mutated_capture.name read or written in the outer scope
       AFTER the closure's construction site, within the
       defining-scope's live range?
     If NO for all mutated captures:
       case = C (moved).  Lower as B but bind the cell's lifetime
       to the closure's lifetime (closure carries the cell).
       Done.
     If YES for any mutated capture:
       case = D (aliased).  Emit diagnostic.  Halt compilation.
```

### Phase 3 — Diagnostic emission

For case D, collect four positions:

1. The closure's body site (where mutation was written).
2. The captured binding's defining site.
3. The post-construction outer use that triggered the case-D
   classification.
4. The closure's destination site (where it escapes).

Emit either the rustc-style multi-caret diagnostic (if
`DiagEntry` is extended for secondary positions) or the
inline-position fallback that matches existing @P213 / @P215
shape.

### Paper-trace against three snippets

**Snippet 1 — Read-only closure (case A).**

```loft
fn main() {
    let state = GameState { score: 0, ... };
    el::on(loop, fn(e) {
        log_info(format!("score = {}", state.score));
    });
}
```

Phase 1 finds no `OpSetX` writes on the closure struct.  Empty
`mutated_captures`.  Classification halts at A.  No further
analysis; today's value-snapshot semantics apply.

**Snippet 2 — Co-scoped mutating closure (case B).**

```loft
fn main() {
    let state = GameState { score: 0, ... };       // S_main
    let loop  = el::new(16667);                    // S_main
    el::on(loop, fn(e) { state.score += 1 });
}
```

Phase 1 finds `OpSetInt` (or `OpAppendInt` via compound desugar)
on the closure's `state` field.  `mutated_captures = [(state,
S_main)]`.  Phase 2: closure passed to `el::on`, stored in
`loop.handlers[]`; `loop`'s scope = `S_main`.  Destination
scope ⊆ capture scope.  Case B.  Lower `state` as
`Reference<GameState>`.  Accept.

**Snippet 3 — Factory-function escape, no outer use (case C).**

```loft
fn make_counter() -> fn(integer) -> integer {
    let count = 0;          // S_inner
    fn(delta: integer) -> integer {
        count += delta;
        count
    }
}
```

Phase 1: compound assign on `count`.  `mutated_captures =
[(count, S_inner)]`.  Phase 2: closure is the return value;
destination scope is the caller's, > `S_inner`.  Liveness check
on `count` after the closure expression: no further reads or
writes in `make_counter`.  Case C.  Lower as B; bind cell
lifetime to closure lifetime.  Accept.

**Snippet 4 — Aliased escape (case D, rejected).**

```loft
fn problematic() -> fn(integer) {
    let count = 0;
    let closure = fn(delta) { count += delta };
    log_info(format!("count = {}", count));
    closure
}
```

Phase 1: same as Snippet 3 — `mutated_captures = [(count, S_inner)]`.
Phase 2: destination = caller's scope > `S_inner`.  Liveness
check on `count` past closure construction: `log_info` reads
`count`.  Case D.  Emit diagnostic naming the four positions
(closure body, count's defining site, the `log_info` read,
the `closure` return).

### Reuses vs new infrastructure

| Piece | Status |
|---|---|
| Closure synthesis (`__closure_<n>`) | Existing (`src/parser/vectors.rs:760-783`) |
| `captured_names: Vec<(String, Type)>` | Existing (`src/parser/mod.rs:147`) — extend with mutation flag |
| `OpSetX` / compound-assign desugar | Existing (`src/parser/operators.rs:246-327`) |
| Function purity (`Impure(ParentWrite)`) | Existing (`src/data.rs:1327-1364, 1580`) |
| `Type.dep` for lifetime tracking | Existing (`src/data.rs:649,722`) — extend with capture-scope union |
| Variable live intervals | Existing (visible via `LOFT_LOG=variables`) — needs to be consulted by parser |
| Multi-position diagnostics | New (~75 LOC additive); inline fallback matches existing @P213/P215 |
| Cross-fn closure-storage annotation | New annotation system (small) |

---

## Open questions

### Q1 — Cross-fn closure-storage flow

When a closure is passed to `el::on(loop, closure)`, the
analysis must know that `el::on` stores the closure in
`loop.handlers[]` so the closure's lifetime is bounded by
`loop`'s lifetime.  Two paths:

- (a) Annotate library functions with closure-storage info
  (e.g., `#stores_argument(0, in_field=handlers)` or simpler
  `#stores_argument(0)`).
- (b) Run the analysis recursively across function bodies to
  discover closure flow.

(a) is more tractable but requires every fn that takes a closure
to declare its storage intent.  (b) is more general but
expensive.  **Recommended: ship (a); it's the loft idiom (cf.
existing `#impure(parent_write)` annotations).**

### Q2 — User fns with `Unknown` purity

The default for user-defined fns without an annotation is
`Purity::Unknown`.  The mutation classifier conservatively treats
calls of `Unknown` purity on captured bindings as mutating (false
positive over false negative).  Consequences:

- Reading-from-capture-via-user-fn forces the closure into B
  semantics even though no write happens.  Cost: 12B Reference
  instead of inline value.
- Programmers can avoid the false positive by annotating their
  fns `#pure` when applicable.
- Implementation should provide a tooling hint: when the
  classifier downgrades to conservative, the diagnostic suggests
  adding `#pure` if appropriate.

### Q3 — Liveness check granularity

Two integration paths for the liveness check (case C
discriminator):

- (a) Run live-interval analysis BEFORE the escape pass and
  look up each capture's last-use position vs the closure's
  construction position.  Cleanest; one extra pass over the
  fn's code per captured binding.
- (b) Maintain the live set incrementally during parse and
  snapshot it at the closure-construction site.  Smaller
  one-time cost but tangles two analyses.

**Recommended: (a) — clean separation, modest cost.**

### Q4 — Backward compatibility verification

Pure-functional closures (no body mutations) are unaffected by
the new analysis.  But the conservative-on-Unknown-purity rule
could turn previously-trivial closures into
reference-tracked closures if their body calls a user fn with
unannotated purity.

Action: grep `tests/scripts/`, `default/`, `lib/` for closures
with body method/function calls; check that the existing fns
have or can gain `#pure` annotations.

### Q5 — `--native` codegen interaction

`--native` lowers loft to Rust.  Auto-Reference closures that
capture user types become Rust struct-with-pointer captures.
Auto-cell scalars become `Cell<T>` or similar.  Multi-element
`Type.dep` lists need their lowering verified.

Action: when implementation begins, confirm
`src/generation/coroutine.rs` and related files handle the
extended closure shape.

### Q6 — `Mutable<T>` API ships regardless

Even with the case classifier, programmers who want shared
ownership across mismatched lifetimes (case D by design) need
`Mutable<T>`.  Ship it independently; it's ~30 lines and useful
in its own right.  The case-D diagnostic recommends it as one
of the three fixes.

### Q7 — Closures stored in static / module-level state

Static state has program-scope lifetime.  A mutating closure
captured into static state can never satisfy case B or C unless
its captures are also program-scope (e.g., `Mutable<T>` cells
allocated at module load).  The case-D diagnostic should
recognise this pattern and recommend `Mutable<T>` directly.

### Q8 — Inter-thread sharing

The escape analysis assumes single-threaded execution of the
event-dispatch level.  `par(...)` runs work in parallel but with
store isolation (existing design); closure-captured Reference
across threads is already constrained by the par execution
model.  The new analysis does not change this.  Document it
explicitly so users don't expect the case classifier to make
multi-threaded closure capture safe.

---

## Design history — refinements through the conversation

Recorded so future readers see how the spec converged.

### First proposal — `mut fn` keyword

Initial sketch (2026-05-05) had a `mut fn(...)` keyword to mark
mutating closures.  User rejection: *"I am unwilling to add a
new keyword here; that would not be a 'loft' thing to do."*

Replaced by **implicit by body** — the closure's body declares
intent without a keyword.  This is the same internal-switch
pattern loft already uses for `par(...)` (one form, multiple
implementations chosen by context).

### Second proposal — blanket auto-Reference

Earlier draft (Option B as the primary path) had auto-Reference
applied to every captured user type.  User correction: *"I do
not want all closures to be mutable, just the ones that I care
about."*

Replaced by **classifier-gated lowerings** — auto-Reference and
auto-cell apply only to captures that the body actually
mutates, not blanket.  Pure-read closures keep value-snapshot
semantics.

### Third proposal — single safe case (B only)

Mid-stage design (2026-05-05) had only co-scoped mutating
closures (case B) as safe; all other mutating closures rejected.
User refinement: *"There are in fact two distinct cases where
mutability is not a problem: (1) when everything stays in scope,
(2) where the closure escapes the scope but the local variables
do not need to read back the data.  There are in-between
problematic cases."*

Replaced by the **four-case classification** — A read-only,
B co-scoped, C moved, D aliased rejected.  Liveness analysis
discriminates C from D.

### Fourth proposal — analysis sketch missing

Spec was first published with the principle and rule but without
the algorithm-level analysis sketch.  User flagged this as the
top-level concern: *"the design's claims rest on a compiler
analysis that has not been sketched, only assumed."*

Replaced by **this discussion's analysis-sketch section** —
algorithm pseudocode, paper-trace against four snippets,
explicit reuse vs new infrastructure table, gaps to verify in
implementation.

### Fifth proposal — sprawling discussion doc

The combined discussion + spec became ~1000 lines of mixed
content.  User direction: *"Create a locked-in mutable closure
design and a discussion part like for the event loop."*

Resulting structure:
- `README.md` — locked-in spec, design at a glance,
  four cases, lowerings, diagnostic shape, foundations,
  verification, sequencing, cross-references.
- `DISCUSSION.md` (this file) — alternatives,
  analysis sketch, open questions, design history.

---

## What this discussion is NOT deciding

- Whether to take the closure pivot (sequence closures before
  EventLoop).  That's the implementation prioritisation
  decision; this design exists so the decision can be made on
  evidence.
- Concrete syntax for the `Mutable<T>` API (deferred to
  `lib/mutable/` design when implementation begins).
- The exact Phase 2 implementation timeline (unsized at the
  time of writing; the open questions above name the unknowns
  that must be resolved before estimation).
- The ordering of @P213 v4 and this spec's implementation (the
  spec assumes @P213 v4 as a prerequisite, but the actual order
  depends on which is buildable first).

---

## Cross-references

- [README.md](README.md) — the locked-in
  spec.
- [DESIGN_DECISIONS.md § C38](../../../DESIGN_DECISIONS_OWNERSHIP.md#c38--closure-capture-is-copy-at-definition)
  — closed-by-decision entry; long-term direction note recorded
  2026-05-04.
- [DESIGN_DECISIONS.md § C3](../../../DESIGN_DECISIONS_PLATFORM.md#c3--wasm-par-runs-sequentially)
  — `par(...)` internal-switch precedent.
- [EVENT_LOOP.md](../../32-event-loop/README.md) and
  [EVENT_LOOP_DISCUSSION.md](../../32-event-loop/DISCUSSION.md) — the
  spec waiting on novice-fit closures.
- [LIFETIME.md](../../../LIFETIME.md) — dep tracking, scope-based
  freeing, Reference<T> semantics.
- [PROBLEMS.md § 213](../../../PROBLEMS.md#213-typefunction-storage-layout-limit--full-design-for-the-proper-fix)
  — @P213 v4 layout for closures-in-struct-fields.
- [CAVEATS.md](../../../CAVEATS.md) — current closure capture caveat.
