<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLAN22 — MUTABLE_CLOSURES — design: making closures novice-fit

**Status:** **Shipped 2026-05-13**.  Moved to `plans/finished/22-mutable-closures/`.
Drivers (see [§ Drivers](#drivers)): TTT v6 server retrofit
([@PLN39 § v6](../../39-tic-tac-toe/README.md#tic-tac-toe-v6--ergonomic-retrofit-using-writable-closures))
+ @PLN6 audience-demo server (loft code projected to the
audience as part of the "loft snippet highlights" beats —
visible code structure is part of the spectacle).  Companion
discussion (options surveyed, alternatives considered,
implementation analysis sketch, design history) lives in
[DISCUSSION.md](DISCUSSION.md).

This spec evolves
[C38](../../../DESIGN_DECISIONS_OWNERSHIP.md#c38--closure-capture-is-copy-at-definition)
(closures are copy-at-definition).  Default closure semantics
remain unchanged; the spec adds opt-in *behaviour by body* for
closures whose bodies mutate captures.

## Drivers

This was always on the will-do path (lived in `plans/future/`,
not `plans/deferred/`); promotion to current is driven by two
concrete consumers with a soft deadline:

- **TTT v5 server** (just shipped phase 0; full v5 ~weeks away)
  uses `Reference<T>` to mutate captured server state
  (`world`, `next_session_id`, `replay_cache`, `last_active_player`,
  `tick_counter`).  Writable closures drop the `.inner`
  ceremony at every access — ~10% fewer characters and one
  fewer mental indirection.
- **Plan-36 audience-generative-art demo** ([plans/6-audience-generative-art/](../../6-audience-generative-art))
  is the more time-pressured driver.  The talk frames itself as
  an "art show with loft footnotes" — small loft snippets
  projected to the audience.  Server code with `state.inner.X`
  every line reads worse on stage than `state.X` does.  Cleaner
  code earns its keep at the spectacle, not only in the
  codebase.

**Non-blocking constraint**: TTT v6 + @PLN6 must remain
shippable without this plan's implementation.  If @PLAN22 lands
before the talk, @PLN6 server uses writable closures.  If it
doesn't, @PLN6 server uses `Reference<T>` exactly like v5.
Either way the demo functions; only the on-screen code-snippet
elegance differs.

---

## Specification — the design at a glance

Loft closures default to value-snapshot capture (unchanged from
today's C38).  When a closure body mutates a captured binding,
the compiler classifies the closure into one of four cases by
escape and aliasing:

- **A — Read-only**: body does not mutate any capture.  Today's
  semantics, no analysis change.
- **B — Co-scoped mutating**: body mutates captures, but the
  closure's destination scope is included within each capture's
  defining scope.  Lowered to `Reference<T>` capture for user
  types and a hidden cell for `let mut`-style scalars; the
  mutation is visible to the outer scope through the live record.
- **C — Moved mutating**: body mutates captures, the closure
  escapes the capture's scope, AND the outer scope does not
  read or write the captured bindings after the closure's
  construction site.  The closure takes ownership of the
  captured data; safe by liveness.
- **D — Aliased mutating**: body mutates captures, the closure
  escapes, AND the outer scope still reads or writes the
  captured bindings after construction.  Genuine aliased state
  across mismatched lifetimes.  **Rejected** with a clear
  diagnostic naming the post-construction outer use and the
  smallest fix.

The classification is **implicit by body**: there is no new
keyword, no annotation, no capture list.  The mutation in the
body declares the intent; the absence of mutation declares
read-only.  Closures whose bodies don't mutate captures keep
today's semantics exactly.

The implementation requires three pieces of compiler analysis,
each extending existing infrastructure rather than introducing
new subsystems: mutation detection on closure bodies, lifetime +
liveness analysis on captured bindings, and multi-position
diagnostics for the rejection case.

---

## Design principle

> *"We try to do what the programmer asks for all the time, and
> we indicate the places where we can sadly not do what they
> want."* — user, 2026-05-05

When the programmer writes `state.score += 1` inside a closure,
the compiler's job is to make that work — not to reject it on
grounds the programmer wasn't asked to care about.  When loft
genuinely cannot make it work (the closure escapes the capture's
scope AND the outer scope still uses the binding), the
diagnostic explains the constraint and points at the smallest
fix.

This is the same internal-switch pattern loft already uses for
`par(...)`: one syntax the programmer writes, multiple
implementations the compiler picks based on context the
programmer wasn't asked to specify.

| Construct | Programmer writes | Compiler/runtime picks based on | Modes |
|---|---|---|---|
| `par(...)` | one form | build target | native multithreaded; WASM sequential ([C3](../../../DESIGN_DECISIONS_PLATFORM.md#c3--wasm-par-runs-sequentially)) |
| `fn(...)` closure | one form | body's use of each capture, escape, and outer liveness | A read-only; B co-scoped (Reference); C moved (ownership); D rejected |

---

## The four cases — full specification

The classification is determined by three boolean properties of
each closure: **does the body mutate any capture**, **does the
closure escape its captures' scope**, and **does the outer scope
still use the captured bindings after the closure's construction
site**.

| Case | Mutates? | Escapes? | Outer reads after construction? | Verdict | Lowering |
|---|---|---|---|---|---|
| A | No | (any) | (any) | Safe | Today's value-snapshot, no change |
| B | Yes | No | (any) | Safe | `Reference<T>` capture for user types; hidden cell for scalars |
| C | Yes | Yes | No | Safe | Moved capture — closure owns the data |
| D | Yes | Yes | Yes | **Rejected** | Diagnostic, no compilation |

### Case A — read-only

```loft
fn main() {
    let state = GameState { score: 0, ... };
    el::on(loop, fn(e) {
        log_info(format!("score = {}", state.score));
    });
}
```

The body reads `state.score` but never writes it.  Today's
value-snapshot semantics apply.  No analysis change, no new
machinery, no overhead.

### Case B — co-scoped mutating

```loft
fn main() {
    let state = GameState { score: 0, ... };       // scope S_main
    let loop  = el::new(16667);                    // scope S_main
    el::on(loop, fn(e) { state.score += 1; });
    el::run(loop, 60, poll_input);
}   // ← state, loop, and the closure stored in loop.handlers[]
    //   all die simultaneously.
```

The closure's destination is `loop.handlers[]`; `loop` lives in
`S_main`; `state` lives in `S_main`.  The closure cannot outlive
its capture.  The compiler lowers the `state` capture to
`Reference<GameState>` and accepts the mutation; the live record
is mutated through the pointer.

This is the EventLoop's typical pattern.

### Case C — moved mutating

```loft
fn make_counter() -> fn(integer) -> integer {
    let count = 0;
    fn(delta: integer) -> integer {
        count += delta;       // closure mutates captured `count`
        count                 // and returns the new total
    }
    // After the closure expression, `count` is dead in this scope.
    // The closure carries the only live `count`.
}
```

The closure escapes (it's returned), but `make_counter` does not
read or write `count` after constructing the closure.  The
closure has effectively taken ownership.  Safe — accepted with
the same lowering as case B internally, but the lifetime of the
captured cell is bound to the closure's lifetime, not the
defining scope's.

### Case D — aliased mutating (rejected)

```loft
fn problematic() -> fn(integer) {
    let count = 0;
    let closure = fn(delta) { count += delta };
    log_info(format!("count = {}", count));   // ← outer reads
    closure                                    // ← closure escapes
}
```

The outer scope reads `count` AFTER the closure is constructed,
AND the closure escapes upward.  Two writers / readers of `count`
exist with mismatched lifetimes.  Rejected with a diagnostic that
names the post-construction outer use (line of the `log_info`)
and offers three fixes (remove the outer read → falls into case
C; restructure to keep the closure in scope → case B; use
`Mutable<integer>` for explicit shared ownership).

---

## Implicit by body — the rule

The classifier sees the closure's body and the surrounding scope.
No syntax distinguishes mutating from non-mutating closures; the
body itself is the declaration of intent.

```loft
fn(e: ClickEvent) {
    let n = state.score;
    log_info(format!("score={}", n));
}
// → case A: pure read, today's value-snapshot semantics.
```

```loft
fn(e: ClickEvent) {
    state.score += 1;
}
// → case B/C/D depending on the closure's destination and the
//   outer scope's use of `state` after the closure's
//   construction site.
```

The classifier treats these uses as mutation:

- Direct field write (`b.x = v`) and compound assign (`b.x += v`).
- Whole-binding reassign (`b = v`).
- Index assign (`b[i] = v`).
- Struct chain (`b.inner.deeper.x = v`).
- Method call where the method's purity is `Impure(ParentWrite)`
  (existing loft purity annotation; see Implementation
  foundations).
- Function call where the callee's purity is
  `Impure(ParentWrite)` and the captured binding is the first
  argument.
- Local alias `let s = b` followed by a mutation through `s`,
  iff `b`'s static type is a `Reference<T>` (alias-tracking;
  value-type aliases are copies and don't propagate mutation).
- Mutation inside a nested closure that captures the same
  binding (transitive).

The classifier is **conservative on uncertain cases**: if a
method or function call has `Unknown` purity (the default for
user fns without an annotation), the captured binding is
classified as potentially mutated.  False positives turn
read-only closures into reference-tracked ones at modest cost
(12B Reference instead of inline value); false negatives are not
acceptable because they would silently drop the user's mutation.

### Suppressing the lowering

Programmers who explicitly want value-snapshot semantics for a
captured user type — even though the body would otherwise look
like it mutates — bind a fresh local copy before the mutation:

```loft
fn(e) {
    let snapshot = state;        // local copy, value-typed
    snapshot.score += 1;         // mutates the local, not `state`
}
// → case A: `state` is read once (to copy); the mutation is on
//   the local `snapshot`.  Value-snapshot of `state` preserved.
```

---

## Lowerings

### B / C — user-type captures: `Reference<T>`

When the classifier identifies a captured binding `b` of user
type `T` as mutated (and the closure passes the safety check
for case B or C), the closure record stores `b` as
`Reference<T>` — a 12B DbRef pointer to the live record.  Field
reads and writes inside the closure body operate on the pointed
record through the existing `OpGetX` / `OpSetX` machinery.

The lowering reuses today's closure synthesis path
(`__closure_<n>` struct in `src/parser/vectors.rs:760-783`)
with the field type changed from inline `T` to `Reference<T>`.

### B / C — scalar captures: hidden cell

When the classifier identifies a captured *scalar* binding as
mutated, the binding is auto-wrapped in a hidden 1-field record
allocated in the binding's defining scope; the closure captures
a `Reference` to that record.  Reads and writes inside the
closure body desugar to the cell's get/set ops.

This lowering kicks in **only when the binding is captured by a
mutating closure** — pure-local mutables stay on the stack with
no indirection.  Loft's existing capture-set tracking
(`Parser.captured_names: Vec<(String, Type)>` at
`src/parser/mod.rs:147`) provides the trigger.

### D — explicit `Mutable<T>` for shared ownership

When the programmer needs aliased mutating state (case D) by
design — multiple closures all updating shared state across
mismatched lifetimes — the spec ships `Mutable<T>` as the
explicit opt-in:

```loft
let count = Mutable::new(0);
let closure = fn(delta) { count.set(count.get() + delta) };
log_info(format!("count = {}", count.get()));
closure
```

`Mutable<T>` allocates `T` in a longer-lived store; closures
capture a `Reference<Mutable<T>>` (no escape constraint because
the cell's lifetime is explicit); the programmer accepts the
explicit shared-ownership semantics.  ~30 lines of stdlib in
`lib/mutable/`.

---

## Diagnostic shape

Case D rejections name **four** things:

1. **Which capture is at fault** — the binding name and its
   defining scope.
2. **Why the closure's destination outlives the capture** — the
   destination expression and its containing scope.
3. **Where the outer scope still uses the capture** (the case-D
   discriminator that distinguishes from case C) — the
   post-construction read or write site.
4. **The smallest fix** — one of:
   - Remove the post-construction outer use → falls into case
     C, accepted.
   - Move the binding to the destination's scope → case B,
     accepted.
   - Wrap in `Mutable<T>` → explicit shared ownership, accepted.

### Template

```
error: closure mutates captured `count` and escapes, but the
       outer scope still uses `count` after the closure is
       constructed
       --> game/main.loft:42:18
        |
     42 |     let closure = fn(delta) { count += delta };
        |                               ^^^^^^^^^^^^^^^^ mutates `count` here
       --> game/main.loft:39:9
        |
     39 |     let count = 0;
        |         ^^^^^ `count` is bound in this scope
       --> game/main.loft:43:31
        |
     43 |     log_info(format!("count = {}", count));
        |                                    ^^^^^ outer scope reads `count` here
       --> game/main.loft:44:5
        |
     44 |     closure
        |     ^^^^^^^ closure escapes via return
       hint: remove the outer read at line 43 → closure takes ownership of `count` (case C)
       hint: OR allocate `count` in the caller's scope and pass a Reference (case B)
       hint: OR wrap with Mutable<integer> for explicit shared ownership
```

If multi-caret pretty-printing is too much work for v1, the
fall-back inline format matches the existing @P213 / @P215 shape:

```
closure mutates captured 'count' and escapes (returned at game/main.loft:44:5),
but outer scope reads 'count' at game/main.loft:43:31 after the closure is
constructed at game/main.loft:42:18; binding declared at game/main.loft:39:9;
hint: remove the outer read (case C), OR move the binding outward (case B),
OR use Mutable<integer> for explicit shared ownership.
```

---

## Implementation foundations

The design extends existing loft infrastructure; no new
subsystems are required.  Pieces involved:

- **Closure synthesis** (`src/parser/vectors.rs:760-783`) —
  closures already become a synthesized `__closure_<n>` struct
  with captured fields.  Lowerings change field types from
  inline `T` to `Reference<T>` (or hidden cell for scalars) but
  reuse the synthesis path.
- **Captured-name table** (`Parser.captured_names: Vec<(String,
  Type)>` at `src/parser/mod.rs:147`) — already collected
  during lambda parsing; extended with a per-name "mutated"
  flag from the new analysis.
- **Mutation detection** — walks the closure body's IR for
  `OpSetX` / `OpAppend*` / `OpClear*` / `OpInsertVector` /
  `OpRemoveVector` opcodes whose target is a captured field, plus
  function/method calls with `Impure(ParentWrite)` purity
  (`Definition.purity` at `src/data.rs:1580`,
  `src/data.rs:1327-1364`).
- **Lifetime inclusion (case B)** — extends the existing
  `Type.dep: Vec<u16>` field (`src/data.rs:649,722`) so
  mutating-closure types carry the union of capture-defining
  scope ids.  Loft's existing dep-exit machinery rejects
  assignments where dep > destination scope.
- **Liveness on captures (case C discriminator)** — extends
  loft's existing per-variable live-interval tracking (visible
  via `LOFT_LOG=variables`) with a check at each closure
  construction site: are any captured bindings used in the outer
  scope past this point?
- **Multi-position diagnostics** — extends `DiagEntry` in
  `src/diagnostics.rs` with optional secondary positions; ~75
  LOC of additive change.  Inline-position fallback matches
  existing @P213 / @P215 shape and requires no infrastructure
  change.

The detailed analysis sketch (algorithm pseudocode, paper-trace
against three snippets, gaps to verify in implementation) lives
in
[DISCUSSION.md § Analysis sketch](DISCUSSION.md).

---

## Critical files (when implementation starts)

| File | Change |
|---|---|
| `src/parser/vectors.rs` | Adjust closure synthesis to honour mutation flags on captured names |
| `src/parser/mod.rs` | Extend `captured_names` with per-binding mutation/liveness flags |
| `src/parser/operators.rs`, `src/parser/control.rs` | Walk closure bodies for mutation detection |
| `src/scopes.rs` / `src/variables/` | Liveness check on captured bindings past the closure's construction site |
| `src/data.rs` | Augment `Type.dep` propagation for closure types; document the multi-element-dep case |
| `src/diagnostics.rs` | Optional: secondary-position support; or fall back to inline format |
| `lib/mutable/` (new) | `Mutable<T>` stdlib helper for explicit case-D shared ownership |
| `default/01_code.loft` | Audit user fns for purity annotations; tighten where false-positive cost matters |

---

## Verification

End-to-end checks once implemented:

1. **Case A regression tests** — confirm existing read-only
   closures (in `tests/scripts/`, `default/`, `lib/`) compile
   unchanged with no analysis activity.
2. **Case B acceptance** — the EventLoop pattern (closure
   captures `state`, registered with `el::on`, both bound in
   `main()`) compiles and the mutation is visible to the outer
   scope.
3. **Case C acceptance** — `make_counter()` factory pattern
   compiles; counter behaves as a state machine across calls.
4. **Case D rejection** — `problematic()` from § The four cases
   produces the predicted diagnostic with all four parts present.
5. **Conservative-default behaviour** — a closure that calls a
   user fn with `Unknown` purity classifies the capture as
   mutated; verify no silent drops.
6. **Suppression** — local-copy pattern (`let snapshot = state`)
   correctly preserves value-snapshot semantics.
7. **`Mutable<T>` interaction** — captures of `Mutable<T>`
   bypass the escape check (the cell's lifetime is explicit).
8. **CI gate**: `cargo fmt --check`, `cargo clippy --release
   --all-targets -- -D warnings`, `cargo build --release
   --no-default-features`.

---

## Sequencing

### Plan-22 internal phasing (added 2026-05-12)

Each phase has its own design doc under `plans/finished/22-mutable-closures/`:

| Phase | Ships | Dependencies |
|---|---|---|
| [00 — matrix freeze + harness wiring](00-matrix.md) | `tests/mut_closure_matrix.rs` + Case A baseline cells | None |
| [01 — mutated-captures detection](01-mutation-detection.md) | Walker that marks captures as `mutated: bool`; no behavior change | Phase 00 harness |
| [02 — Case B (co-scoped)](02-case-b.md) | Reference / hidden-cell lowering for mutating co-scoped closures | Phase 01 |
| [03 — Case C (moved)](03-case-c.md) | Liveness check + ownership transfer for factory pattern | Phase 02 |
| ~~[04 — Case D (rejection)](04-case-d.md)~~ | DECOMMISSIONED 2026-05-13 — cell + auto-Reference from phases 02-03 already give Case D correct shared-state semantics; no rejection needed | Phase 03 |
| ~~[05 — `Mutable<T>` helper](05-mutable-helper.md)~~ | DEFER (cell + auto-Reference subsumes; revisit only if a concrete use case surfaces) | Phase 04 |
| [06 — closeout](06-closeout.md) | Doc closeout — CHANGELOG_TECHNICAL, DESIGN_DECISIONS, CAVEATS, ROADMAP, move to finished/ | Phases 02-03 |

**Acceptance for the whole plan**: every Case A regression cell stays green; phase 02-05 cells run cross-mode under `tests/mut_closure_matrix.rs`; phase 04 case-D rejections pinned in `tests/parse_errors.rs`; phase 06 retrofit ships TTT v6 + @PLN6 servers using writable closures.

@P257's parse-time-rejection pattern (closed 2026-05-12) is the
template phase 04 uses for case-D diagnostics.  Plan-15
(closure validation, finished 2026-05-12) provides the
regression net — its 22 cells in `tests/closure_matrix.rs`
+ 5 leak guards in `tests/leak.rs` confirm Case A semantics
stay correct as @PLAN22 evolves the synthesis path.

### External dependency stack (first-game ship)

This spec sits on the dependency stack for first-game ship:

| Phase | Ships | Dependency |
|---|---|---|
| 1 | [@P213 v4](../../../PROBLEMS.md#213-typefunction-storage-layout-limit--full-design-for-the-proper-fix) — closures-in-struct-fields layout | None (separate plan) |
| 2 | This spec — implicit-by-body classifier with cases A/B/C/D | @P213 v4 |
| 3 | EventLoop core ([EVENT_LOOP.md](../../32-event-loop/README.md)) | This spec |
| 4 | First playable single-player game | Phase 2 |
| 5 | First multiplayer game | Phase 3 |

@P213 v4 already shipped (closed 2026-05-04 via `Parts::ChildRec`
layout-widening); @PLAN22 phase 2 dependency is met.

---

## Cross-references

- [DISCUSSION.md](DISCUSSION.md)
  — alternatives considered, implementation-analysis sketch,
  open questions, design history.
- [DESIGN_DECISIONS.md § C38](../../../DESIGN_DECISIONS_OWNERSHIP.md#c38--closure-capture-is-copy-at-definition)
  — the closed-by-decision entry this spec evolves; long-term
  direction note recorded 2026-05-04.
- [EVENT_LOOP.md](../../32-event-loop/README.md) — the spec waiting on
  novice-fit closures.
- [EVENT_LOOP_DISCUSSION.md § Novice-readiness](../../32-event-loop/DISCUSSION.md#novice-readiness-evaluation-2026-05-05--pivot-trigger)
  — the evaluation that prompted this work.
- [LIFETIME.md](../../../LIFETIME.md) — dep tracking, scope-based
  freeing, Reference<T> semantics.
- [PROBLEMS.md § 213](../../../PROBLEMS.md#213-typefunction-storage-layout-limit--full-design-for-the-proper-fix)
  — @P213 v4 layout for closures-in-struct-fields.
- [CAVEATS.md](../../../CAVEATS.md) — current closure capture caveat.
