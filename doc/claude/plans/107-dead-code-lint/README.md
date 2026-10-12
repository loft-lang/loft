<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN107 — Dead-code lint: never-read local (unused variable + dead store)

**Status — ✅ DONE (2026-07-15). Phase 1 (S0–S5) shipped** on `tuxedo-work`: the dead-store
lint is **default ON** (`LOFT_NO_DEAD_STORES` opts out), verified both backends against a
corpus swept clean (stdlib + all 454 scripts + fixture libs + `tests/lib` + examples). See
[the step plan](#implementation--small-verifiable-steps). Phase 2: **P2a (dead-final-store)
SHELVED — the sketched counter-flush is UNSOUND** (see Phase 2 below); P2b
(method-on-dead-receiver) remains FUTURE; the **value-struct-field-copy completeness gap is
now CLOSED** (see § S4a) — value-struct copies warn like the vector copy.
**Key finding:** loft
**already ships** `unused_variables` (`Function::test_used`, `src/variables/mod.rs:1666`)
— it correctly flags W-scalar/W-accumulator/N-effectful *today*; the real gap is that its
`uses` counter treats a write-**target** as a read, so it misses **W-copy** (the graphics
footgun) and **W-reassign**. The fix is a **surgical extension** of that shipped lint
(split value-observing reads from write-targets), not a new analysis. The motivating
library bug is **already fixed** (see [The payoff](#the-payoff--already-banked)).
Issue: [loft-lang/plans#107](https://github.com/loft-lang/plans/issues/107).

## Goal — the one sentence

> **Warn when code writes something that *looks* like it does useful work but actually
> does not** — a store whose written value is never observed before the binding drops.
> That is dead code, and it is exactly the class of mistake the programmer cannot see by
> reading the source (it reads as productive).

This is loft's analogue of Rust's `unused_variables` + `unused_assignments`, unified
under one liveness rule. It is **not** a copy/borrow lint (that is @PLN90, shipped and
closed) — it fires on **plain scalars too**. The struct-copy footgun is one instance,
not the definition.

The two shapes that must warn, from the user's own examples:

```loft
{ a = 3; a += 1 }                 // W-scalar: a created + mutated, never read → BOTH statements dead
{ d = self.data; d[i] = x }       // W-copy:   d is a C86 field-read COPY; d[i]=x is a lost write
```

`d[i] = x` *reads* as "paint pixel i" but paints a throwaway copy — precisely
"seems useful, isn't." `&` (borrow the field instead of copying) makes the copy case
*safer to write*, but the lint is the **safety net that catches it whether or not the
author reached for `&`**, and it catches the scalar case that `&` has nothing to do with.

## The rule — fixpoint backward liveness

A local variable `v` is **dead** iff its value never flows to an **observable use**.
Compute the observable-use frontier, then propagate liveness backward to fixpoint:

1. **Seed (observable sinks — a use that escapes analysis or has an effect):**
   - passed as a **reader-position argument** to any call (user or native),
   - **returned** from the function,
   - a **branch/loop condition** (`if v`, `while v`, `match v`),
   - read into a value that is itself stored into an **escaping** structure
     (a returned/observable record, a field of a param, an appended element),
   - the **base of a projection that is itself read** (`v[i]`/`v.f` in a read context).
2. **Propagate:** if `w = <expr>` and `w` is live, every variable read in `<expr>`
   becomes live. Iterate until no change.
3. **Report:** any local never marked live is **dead** → warn at its definition site,
   and (nice-to-have) list its dead mutation sites.

The fixpoint is what makes `{ a = 3; a += 1 }` fire: the only read of `a` is the RHS of
`a += 1`, but that write is itself dead, so its read does **not** rescue `a`. A read that
only feeds another dead store is not an observable use.

**loft already approximates this soundly.** The shipped `test_used` counter (see below)
is a conservative one-step version — `reads == 0 ⇒ definitely dead` — and it *already*
handles the `a += 1` self-read correctly (W-scalar fires today). Full fixpoint would catch
strictly more (transitively-dead chains like `a = 3; b = a` where `b` is also dead); that
is a later refinement, not needed for the motivating cases. The concrete Phase-1 mechanism
is the counter split below, not a from-scratch dataflow engine.

## What warns / what stays silent (the spec matrix)

Corpus: [`spec.loft`](spec.loft) — every row hand-verified on **both** backends.

| # | Shape | Verdict | Why |
|---|---|---|---|
| **W-scalar** | `a = 3; a += 1` (a unread) | **WARN** | created + mutated, never observed |
| **W-copy** | `d = self.data; d[i] = x` (d unread) | **WARN** | mutates a copy — lost write |
| **W-accumulator** | `total = 0; for i in xs { total += i }` (total unread after) | **WARN** | leftover accumulator — classic dead loop work |
| **W-reassign** | `a = 3; print(a); a = 5` (a unread after) | **WARN** *(per-store, Phase 2)* | the `a = 5` store is dead |
| **W-method-mut** | `d = Box{…}; d.push(9)` (d unread, `push` only mutates self) | **WARN** *(interprocedural, Phase 2)* | the mutation is unobservable — needs an effect summary to prove it (see below) |
| **N-read** | `d = s.f; d[i] = x; use(d)` | silent | d is observed → the write is real work |
| **N-write-through** | `s.f[i] = x` (no copy) | silent | mutates the live field directly |
| **N-fresh-used** | `e = [1,2,3]; e[i] = x; use(e)` | silent | e escapes into `use` |
| **N-copy-read** | `d = s.f; use(d)` | silent | a copy that is *read* is not dead (it may be a wasteful copy — that is @PLN90, not this lint) |
| **N-method-effect** | `d = Box{…}; d.log_push(9)` (`log_push` prints) | silent | the call does real external work; d being dead is irrelevant |
| **N-method-return** | `x = d.pop(); use(x)` (d unread after) | silent | the call produced an observed value — real work |
| **N-method-escape** | `sink.take(d)` | silent | d escapes into the callee → live |
| **N-builder** | `b = B{}; b.set_x(1); r = b.build(); use(r)` | silent | b is read by `build()` → the setter chain is live |
| **N-effectful** | `x = launch()` (x unread, launch has effects) | **WARN on the binding, not the call** | the *binding* is dead; the RHS effect stays. Message must not imply deleting the computation. |

The **N-copy-read** row is the load-bearing guard: the whole point is to *not* warn when
the programmer legitimately copied and then used the copy. "Seems useful **and is**" ⇒ silent.

### Method calls on a dead receiver — the interprocedural frontier

A **direct** write (`d.f = x`, `d[i] = x`, `d += 1`) has its *entire* effect on `d`, so if
`d` is dead the write is provably dead by **intraprocedural** liveness alone (Phase 1). A
**method call** on a dead receiver (`d.push(9)`) is dead **only if** the call's *sole*
observable effect is mutating `self` **and** its return value is not observed — otherwise
the statement does real work (I/O, a global mutation, an observed result) even though `d`
itself dies. Deciding that needs a **per-function effect summary**:

> A function is **self-mutation-only** iff its only observable effects are writes through
> its `&`/mut arguments, it performs no I/O or global mutation, and its return is `()` (or
> unobserved at the call site). A call to such a function, on a receiver that is dead after
> the call, is a dead store.

Until that summary exists, method-mediated deadness (**W-method-mut**) stays **silent** —
the zero-false-positive bar wins over catching this shape. It is Phase 2 work, gated on the
effect summary, and it is *why* Phase 1 restricts itself to direct writes: those are the
shapes local liveness can prove dead without lying. **N-method-effect / N-method-return /
N-method-escape** are the false positives that summary must avoid.

## What loft already has (verified 2026-07-14)

Running [`spec.loft`](spec.loft) shows loft **already ships two** dead-code lints:

1. **`unused_variables`** — `Function::test_used` (`src/variables/mod.rs:1666`) warns
   "`Variable X is never read`" when `var.uses == 0`. Fires correctly today on **W-scalar**
   (`a = 3; a += 1` — note `a += 1` does *not* bump `uses`, so the self-read doesn't rescue
   it), **W-accumulator**, and **N-effectful** (warns on the binding — exactly right).
2. **`unused_assignments`** — `Function::track_write` (`src/variables/mod.rs:987`, called from
   the whole-var `var = expr` path at `expressions.rs:1384`) warns "`Dead assignment — X is
   overwritten before being read`" when a **whole-variable** reassignment overwrites a prior
   write with no read between (`uses == uses_at_write`, tracked via `write_source` /
   `uses_at_write`, with branch save/restore).

So this is **not greenfield**. The two gaps that motivate @PLN107 are precise:

- **W-copy** (`d = b.data; d[0] = 9`, d unread) — the motivating footgun. `d.uses > 0`
  because the write-target base `d` in `d[0]=9` is `in_use`'d (element writes read the base
  to locate the slot), so `test_used` thinks d is used; and `d[0]=9` is **not** a whole-var
  reassignment, so `track_write` never sees it. Falls through **both** lints.
- **W-reassign-final** (`a = 3; print(a); a = 5`, a unread after) — `track_write` only checks
  at the *next* write, and there is none after `a = 5`; `test_used` sees `uses > 0` (the
  print). The dead *final* store falls through both. (`a = 3; a = 5` with no read between **is**
  caught by `track_write` today.)

The single root cause of the W-copy gap: **`uses` conflates a value-observing read with a
write-target base reference.** `uses` also drives codegen (last-use elision; `uses == 1`
checks in `state/codegen.rs`, `parser/operators.rs`, `parser/collections.rs`), so it **must
not change** — the fix ADDS a counter and derives a read count for the lint only.

## Mechanism — split `reads` from write-targets in the existing lint

The whole fix, in one sentence: **a variable whose only uses are write-targets counts as
never-read.**

- Add a `reads` sub-count to `Variable` (`src/variables/mod.rs`) alongside `uses` — bumped
  only for **value-observing** references, *not* for a `Var` appearing as the target base of
  a `Set`/`OpSet*`. `test_used` then also flags a non-escaping local with `reads == 0 &&
  writes > 0` (the W-copy dead store), distinct from the existing `uses == 0` (never bound
  usefully) case.
- The read-vs-write classifier **already exists**: `src/variables/intervals.rs`
  `compute_intervals` walks the IR and *already distinguishes* `Value::Var(v)` (a read) from
  `Value::Set(v, …)` (a write) — its own comments (`intervals.rs:74`) note "variables that
  are only ever WRITTEN (never read after…)". The subtlety it must handle: `d[i]=x` lowers to
  `Call(OpSetInt, [Var(d), idx, rhs])`, so the *target* `Var(d)` (arg 0 of an `OpSet*`) must
  be classified as a write-target, not a read — the same `first_arg_write_ops` set
  `use_analysis.rs` already defines identifies exactly those ops.
- Keep the existing exclusions (`_`-prefixed, `#`-synthetic, `captured`, global-shadowing).
  Escape stays live: any read that feeds a call arg / return / escaping store is a real read.

This is a **surgical extension of a shipped lint**, not a new analysis engine — far cheaper
and lower-risk than the from-scratch `use_analysis.rs` pass first sketched. The @PLN90
`use_analysis.rs` classification (`first_arg_write_ops`, `Ctx::ReaderArg`) is the reference
for *which op positions are writes*; the emission stays in `test_used`.

## Implementation — small verifiable steps

Phase 1 closes the **W-copy** gap (the motivating footgun). Each step is independently
landable (compiles + suite green), and each has a concrete pass/fail check against the
**S0 oracle**. The order front-loads the risk: make the read/write classifier *observable*
and prove *zero behaviour change* before any warning depends on it; sweep for false
positives before flipping default-on.

- **S0 — Lock the oracle (test-only, no product change).** Add `tests/dead_code_lint.rs`:
  compile [`spec.loft`](spec.loft) on **both** backends, collect emitted warnings, assert the
  *current* set (W-scalar/W-accumulator/N-effectful warn; W-copy + W-reassign-final silent).
  **Verify:** `cargo test --test dead_code_lint` green; adding a read of `d` to the W-copy row
  flips it → proves the harness can fail. *Risk: none. Payoff: the regression net for every
  later step.*
- **S1 — Observable read count, NO warning (the subtle part, isolated). ✅ DONE 2026-07-14.**
  Implemented as a **decoupled post-parse IR walk** (`use_analysis::dead_store_accesses`),
  returning per-var `(reads, write_targets)` — NOT the originally-sketched
  `reads = uses − write_target_uses`. **Why the sketch was wrong:** the dump showed `uses` and
  true value-reads diverge — `a += 1`'s self-read bumps the walk's `reads` but not `uses`
  (uses=0), so subtraction would mis-count. The walk classifies a `Var` as a write-target only
  at arg 0 of an `OpSet*` (descending projection chains); every other `Var` is a read; `uses`
  is never touched. Exposed via a gated dump (`LOFT_DUMP_READS`) — no warning. **The dump did
  its job:** it caught a bug — the copy-fill `d = b.data` lowers to `OpAppendVector(Var(d),…)`,
  whose arg-0 `d` was falsely counted as a read (W-copy showed `reads=1`); fixed by treating
  arg 0 of the whole `first_arg_write_ops` set as a write-DESTINATION (never a read), while
  counting `write_targets` only for the `OpSet*` subset. **Verified** (`tests/dead_code_lint.rs
  ::s1_classifier_isolates_w_copy_dead_store`): `d`@W-copy → `(reads=0, wt=1)` (the signal);
  N-read/N-fresh `reads ≥ 1`; N-copy-read `wt=0`; W-scalar `a` / N-effectful `x` present no
  signal (so S2 won't double-warn what `test_used` owns). **S0 oracle byte-identical** on both
  backends ⇒ `uses`/codegen untouched; the walk is inert unless `LOFT_DUMP_READS` is set.
- **S2 — Emit the warning behind an off-by-default flag. ✅ DONE 2026-07-15.**
  `Function::test_dead_stores` (sibling of `test_used`, `variables/mod.rs`) warns on `uses > 0
  && reads == 0 && write_targets > 0` (plus the `test_used` exclusions: `_`/`#`, argument,
  captured, global-shadow), gated on `LOFT_DEAD_STORES` (default OFF). Message (refined to the arc-C
  steer, @PLN102 alias-where-correct step 6): *"'d' is mutated but its value is never read — the
  write is LOST. A whole-value bind (`d = …`) COPIES the heap value (@C86), so the mutation lands in
  the copy, not the source. For write-through, bind a live reference with `&` (`d = &…`). If a copy
  was intended, read `d` after the mutation."* — points straight at the explicit fix (`&`) and offers
  the read-back alternative; the write-through is always the programmer's `&`, never inferred.
  **Findings:** (a) the
  `uses > 0` clause is provably REDUNDANT (`reads==0 && write_targets>0` ⇒ `uses>0`) but kept as
  an explicit belt-and-braces guarantee that S2 and `test_used` (`uses==0`) never both fire — it
  costs nothing and hardens the S4 sweep against unseen IR shapes; (b) the feared construction
  double-flag is a NON-issue — `z = Box{…}`'s element fills READ `z` (`reads>0`), so `reads==0`
  excludes constructed values automatically (locked by the `n_construct_unread` corpus row).
  **Verified** (`s2_flag_on_warns_w_copy_*`, both backends): flag-on emits exactly ONE dead-store
  warning (W-copy `d`), the three never-read warnings are untouched, and `d` is not also
  never-read-flagged (no double warning); flag-off is byte-identical to S1 (the S0 oracle, which
  never sets the env, stays green).
- **S3 — Escape + branch hardening. ✅ DONE 2026-07-15 (no code change needed).** Added corpus
  rows: escape-to-call (pass whole `d` to a callee), escape-via-return, conditional read
  (`if c { use(d) }`), loop cross-iteration read (`buf[i] = buf[i-1]+1`). The lint was already
  robust — escaping *is* a read in the IR (a call arg / return / any-path read → `reads>0`), so
  every row stays silent with **zero** false positives. **Verified** (both backends): flag-on
  stays at exactly ONE warning (W-copy `d`); the S1 test asserts every *other* `d` keeps
  `reads≥1` and W-copy `d`=`(0,1)`, which together with the count==1 assertion pins that the one
  warning is W-copy's `d`. **Finding:** `n_escape_return`'s `d` is move-elided (returned
  directly) → no user-var, so the return path is guarded by the count, not a classifier cell.
- **S4 — Suite-wide false-positive sweep (the cry-wolf gate). ✅ DONE 2026-07-15.**
  Swept the stdlib + all 454 `tests/scripts/*.loft` with `LOFT_NO_CACHE=1 LOFT_DEAD_STORES=1`.
  **Methodology finding:** loft's **program cache** (`~/.cache/loft/program-*.store`) skips the
  re-parse on a warm run, so ALL parse-time diagnostics (this lint AND the shipped `test_used`)
  vanish on cache hits — the sweep MUST use `LOFT_NO_CACHE=1` (the cache is auto-off under Cargo,
  which is why the tests are stable). **Result:** stdlib **clean** (0); **13 warnings in 10
  copy/borrow-semantics test scripts**, triaged into two classes:
  - **5 genuine FALSE POSITIVES — aliases** (the write propagates, so it is NOT dead): `178`/`434`
    `&`-refs (`RefVar`), `516` reference-struct field alias, `25`/`85-borrow` element views
    (`Optional(Reference)`). Each test asserts the *original changed*.
  - **8 TRUE dead stores in intentional copy tests** (`516` `m`/`c`, `517`, `519` value structs;
    `503` `copy`/`cc`, `85-tier1` vectors; `294` var-to-var): the write to the copy genuinely is
    never read; the test proves independence by asserting the *original unchanged*.

  **✅ RESOLVED 2026-07-15 (S4a + S4b) — the tree is now clean under `LOFT_DEAD_STORES`.**
  - **S4a (`080bbef3`) — the alias false-positive fix.** `ownership_of` is only reliable AFTER
    `scopes::check` materialises the copies, so the lint MOVED from a parse-time `Function` method
    to a post-scopes whole-program pass **`use_analysis::warn_dead_stores`** (`main.rs`, after
    `warn_copies`, emitting via `diags.add_at`). It warns only when the mutated var OWNS its
    store: `Own::Owned`, OR `Borrowed/Join{base}` where `base` is the var itself or a **synth
    buffer** (`is_synth_buffer` — the copy idiom builds a vector copy as `OpGetField(__vdb,…)`, so
    a genuinely-owned copy reads as `Borrowed{__vdb}`). `RefVar` (`&`) is excluded outright. Two
    findings: `uses` is not maintained post-scopes (the S2 `uses>0` guard was dropped as redundant),
    and the sweep "non-determinism" was the program cache (tests hardened with `LOFT_NO_CACHE=1`).
    All 5 alias FPs silent; spec.loft still warns; no new FPs.
  - **Value-struct completeness — ✅ CLOSED 2026-07-15.** S4a left value-struct field/element
    copies as a safe false NEGATIVE: `ownership_of` reports `Borrowed{source-view}` for them, so
    the `owns` gate stayed shut. But a `value struct` read out of a field/element COPIES (the
    `value_struct_copy` pass materialises a fresh `OpCopyRecord`; @PLN101), so `m = w.field;
    m.x = 9` (m unread) is a lost write — the SAME footgun. Fix: `is_value_struct_local` — a
    local whose type is a `Type::Reference` marked in `Data.value_structs` ALWAYS owns its store,
    OR'd into the `owns` gate. The load-bearing subtlety: a reference-struct alias (`al =
    rv.cells[0]; al.n = 42`) has the IDENTICAL `(reads=0, write_targets=1)` classifier signal as a
    value-struct copy — the two are separated ONLY by ownership (value struct = owned copy → warn;
    reference struct = aliases, the write propagates → silent). Corpus rows `w_vstruct_field`/
    `w_vstruct_elem` (WARN) + `n_vstruct_read`/`n_ref_alias` (silent) lock it; scripts 516/517
    strengthened (S4b pattern: read the copy → prove copy changed AND source didn't). Full suite
    green both backends (2913 passed; the 3 fails are the known s7/wasm-relay/viewer env flakes).
  - **S4b (`01172384`) — the 8→4 true positives.** The remaining copy-mutate dead stores lived in
    intentional copy-semantics tests (503/294/519/85-tier1). Each was strengthened to also assert
    the COPY was mutated (reads the copy → silences the lint AND proves both "copy changed" and
    "original didn't"). All pass both backends.
  - **Gate MET:** zero dead-store warnings across the stdlib + all 454 `tests/scripts`. S5
    (default-on) is unblocked.
- **S5 — Flip default-on + graduate. ✅ DONE 2026-07-15.** `dead_stores_enabled()` is now
  **default ON**; `LOFT_NO_DEAD_STORES` opts out (mirrors the @PLN28 toggles; documented in
  `CLAUDE.md`). Pre-flip the corpus was swept comprehensively cold — stdlib + all 454
  `tests/scripts` + all fixture libs + `tests/lib` + examples — **zero warnings**, so no real
  hits remained. The oracle (`tests/dead_code_lint.rs`) was rewritten for default-on: the corpus
  warns exactly on the W-copy `d` by default, `LOFT_NO_DEAD_STORES` silences it (but not
  `test_used`), and a dedicated `s4a_aliases_stay_silent` test locks that real alias scripts
  (`178`/`434`/`25`/`85-borrow`) emit no dead-store warning. `spec.loft` stays the oracle corpus
  in this plan dir (the Rust oracle test IS its regression — moving it into `tests/scripts/`
  would only add its intentional warning to the script runner). **Verified both backends**;
  6 oracle tests green.

**Phase 2 (later, separate steps).**

*P2a — dead FINAL store* (W-reassign-final) — **SHELVED 2026-07-15. The sketched approach
is UNSOUND; do not re-attempt it.** The sketch was "extend `track_write` with a scope-end
flush that re-checks the last write's `uses == uses_at_write`." Instrumentation (gated
`LOFT_DBG_P2A` probe, since reverted) proved the `uses`/`uses_at_write` counters are
**perturbed across the two parser passes** and cannot express scope-end liveness: `uses`
accumulates across BOTH passes while `in_use(v,false)` (`variables/mod.rs:985`) decrements
it, so at the flush point `uses_at_write` can even EXCEED `uses`. Measured at scope-end:
  - motivating `a = 3; print(a); a = 5` (a dead) → `uses=1, uses_at_write=2` ⇒ flush MISSES it;
  - live `c = 3; print(c); c = 5; return c` → `2 == 2` ⇒ **false positive** (c is returned);
  - loop-carried `prev_cr = false; for … { if prev_cr {…}; prev_cr = … }` → `1 == 1` ⇒ **false
    positive** (the init IS read in the loop).

  `uses == uses_at_write` is coherent ONLY inside `track_write`'s own mid-pass moment (which is
  why the existing "overwritten before read" warning is correct); it does not survive to
  scope-end. Two guardrails caught this instantly: the stdlib loader requires a warning-clean
  parse (the two stdlib false positives hard-failed the load). A *sound* P2a needs
  **position-aware IR backward-liveness** ("does a read of `v` follow `v`'s last write on any
  path, incl. loop back-edges?" — the fixpoint sketched in [The rule](#the-rule--fixpoint-backward-liveness),
  deliberately deferred), living in `use_analysis.rs` beside `dead_store_accesses` — a real
  dataflow pass, NOT a cheap flush. Shelved because the cost/payoff inverted: a dataflow pass
  for a shape that catches no known loft bug (it is `unused_assignments`-completeness, not the
  W-copy footgun). (The value-struct-field-copy completeness gap that was its sibling is now
  CLOSED — § S4a.)

*P2b — method-on-dead-receiver* (W-method-mut) — **INVESTIGATED + SHELVED 2026-07-15. Needs
substantial new infrastructure; not a lint extension.** Findings (probes since reverted):
  - The shape is real: `self: T` (non-`const`) methods write through `self` and the mutation
    PROPAGATES to the caller (measured: `d.add(9); d.total()` → 9), so a mutated-then-unread
    receiver genuinely IS dead work.
  - **It cannot reuse the shipped Phase-1 signal.** Phase-1 fires on `reads == 0`, but a
    CONSTRUCTED receiver always has `reads ≥ 1` — a construction `d = Bag{…}` fills its fields
    by reading the target (the deliberate S2 behaviour that keeps `z = Box{}` silent; measured:
    `g = Bag{…}` unused → `reads=1, write_targets=2`). Reclassifying the method receiver as a
    write-target does NOT help — `d` still carries the construction reads. P2b needs a NEW,
    refined-observability liveness (construction-fills and self-mutation receivers are NOT
    observable reads), i.e. the same deferred backward-liveness fixpoint P2a needs.
  - **The "self-mutation-only" effect summary does not exist in loft.** `Purity::Impure(ParentWrite)`
    ("writes a parent store via its first arg") is conceptually the right predicate, BUT purity
    is annotation-driven for the stdlib only — USER functions default to `Purity::Unknown`. The
    lone effect analyser, `scopes::is_par_safe` (phase-5b), is `#[allow(dead_code)]` with "no
    production caller", uses an optimistic cycle short-circuit (not a real fixpoint), and
    EXPLICITLY lacks the "is first arg local?" check — the exact arg-write-vs-global-write
    distinction P2b's summary hinges on. So the effect summary must be built (interprocedural
    fixpoint), not read.

  Net: P2b = (a) build a per-fn self-mutation-only effect summary + (b) a refined-observability
  backward-liveness pass + (c) call-site return-observed checks + (d) a high-risk FP sweep (the
  graphics libs are full of construct → method-mutate → render). A multi-session analysis, not
  the "extend the lint" the plan implied. Both Phase-2 items are gated on the same missing
  backward-liveness foundation (P2b additionally on a nonexistent effect summary). The other
  candidate — the value-struct-field-copy completeness gap — was the cheaper win (extends the
  SHIPPED lint via the ownership analysis, no new dataflow) and is now **CLOSED** (§ S4a).

## False-positive guards (the entire risk)

The lint is worthless if it cries wolf on legitimate code. Non-negotiable:

- **Copy-then-read is silent** (N-copy-read). A read anywhere on a live path rescues.
- **Effectful RHS**: an unread binding whose RHS calls an impure fn still warns (the
  binding is dead) but the diagnostic must say "unused binding" not "dead statement" —
  never imply removing a side-effecting call.
- **Escape = live.** Anything passed to a call arg, returned, or stored into an escaping
  structure is live (conservative — an unknown callee may observe it).
- **Loops.** Back-edges break position order; a var written in an iteration and read in a
  *later* iteration is live. Liveness (not position comparison) handles this correctly —
  but the pass must treat a loop body's reads as reachable from its writes.
- **Params are never "unused-variable" dead** here (that is a separate lint); this pass is
  locals only. A param passed straight through is observable by contract.
- **Compiler-generated temporaries** (`_`-prefixed, `is_compiler_generated`) are excluded
  from the user-facing report (same exclusion @PLN90 already applies).

## The payoff — already banked

The motivating case is **fixed** on `tuxedo-work` (commit pending): `lib/graphics`
`set_pixel` / `fill_rect` / `hline` / `vline` used `X_d = self.data; X_d[i] = c` — a @C86
copy-then-mutate, so the writes were **silent no-ops on both backends** (verified: the
copy leaves `data[0]=0`, direct write-through gives `data[1]=222`). Rewritten to direct
`self.data[i] = c` write-through (matching `blend_pixel`/`clear`), so the software canvas
now actually paints — which is what made on-device text render (@PLN106 B3). This lint
would have flagged all four at authoring time; it is the general guard against the class.

## See also

- @PLN90 [COPY_DIAGNOSTICS.md](../../COPY_DIAGNOSTICS.md) + [90-copy-diagnostics/](../90-copy-diagnostics/) —
  the copy/borrow lint and the `use_analysis.rs` classification this reuses.
- [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) (@C86: whole-value heap binds COPY; #415) —
  *why* `d = self.data` copies, which is what makes the dead-store shape possible.
- @PLN106 [106-android-build-target/](../106-android-build-target/README.md) — the consumer
  that surfaced the copy-mutate footgun in `lib/graphics`.
