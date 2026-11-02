<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# F1 — the facts routine, and moving the analyses onto it

The design for phase F1 of [@PLN210](README.md): the routine of
[`formal/facts.md`](../../formal/facts.md) built as code, and the analyses of its § The analyses
today moved onto it one at a time.  Every step is a behaviour-preserving refactor: the gate is
an EMPTY emission diff (loft-codegen skill, mode B), never a passing suite.

## The gate, per step

1. `scripts/introspect_diff.sh BEFORE AFTER` — the interpreter's bytecode and native's Rust
   over the whole corpus: `IDENTICAL n/n`.
2. `scripts/introspect_diff.sh BEFORE AFTER --c` — the C over the same corpus, with and
   without `LOFT_RELEASE_PASS_PROBE=1`: `IDENTICAL n/n`.
3. The same `--c` diff over the library benches (`--extra` with every
   `loft-bench-libs/*/*/bench/bench.loft`).
4. `make rewrite-census` — no row moved.

`BEFORE` is the binary of the step's parent commit, kept under `target-exp/bin/`.  A non-empty
diff is the bug of that step: the step is reverted or fixed, never pushed through.

## What each kind must keep exactly

The routine reproduces today's answers first; every difference between the analyses (their
cycle answers, their memo policies) stays a declared property of the kind, and unifying them
is a later, measured change (§ After the move).

| kind | answer | cycle scope | on a cycle | memo policy | context |
|---|---|---|---|---|---|
| `stores_written` (`call_writes_store`) | bool, may | the writes family | yes (writes) | a no always; a yes only with context | `HoistOwned::for_callee` |
| `in_place_only` (`in_place_only_writer`) | bool, must | the writes family | no | a yes always; a no only with context | `for_callee` |
| `retbuf_only` (`retbuf_only_writer`) | bool, must | — (asks `stores_written`) | — | none | `for_callee` |
| `writes` (`callee_writes`) | `WriteSet`, may | the writes family | none (unknown) | always, the cycle's none included | `for_callee` |
| `buffer_only` (`callee_writes_only_its_buffer`) | bool, must, per call site | its own open set | yes | none | the site's minted set |
| `inputs` (`callee_inputs`) | `CalleeInputs`, may | its own (memo pre-seeded) | none | always | — |
| `effect_free` (`pure_reuse`) | set, whole program | — | greatest fixpoint | — | — |
| `total` (`shared_call`) | set, whole program | — | least fixpoint | — | — |
| `entry` non-sentinel (`non_sentinel`) | per parameter, down | — | greatest fixpoint | — | — |

The writes family shares ONE active set today (`call_writes_store`, `in_place_only_writer` and
`callee_writes` all insert into the `active` they are handed), so a question about `f` asked
while another member evaluates `f` gets the cycle answer.  The routine keeps that: a kind
declares its cycle SCOPE, and kinds of one scope share the guard.

## The code

```rust
// src/opt/facts/mod.rs
pub struct Facts<'a> {                 // one per compilation and writer
    pub data: &'a Data,
    pub stores: &'a Stores,
    guard: HashMap<Scope, HashSet<u32>>,   // the functions under evaluation, per cycle scope
    memo: HashMap<(KindId, u32), Box<dyn Any>>,
}

pub trait Kind {
    type Fact: Clone + 'static;
    type Ctx;                           // `()` where the kind takes none
    const ID: KindId;
    const SCOPE: Scope;
    fn on_cycle(&self) -> Self::Fact;   // the answer for a function already under evaluation
    fn native(&self, def: &Definition) -> Self::Fact;   // NATIVES `declared`
    fn keep(&self, fact: &Self::Fact, ctx: Option<&Self::Ctx>) -> bool;  // the memo policy
    fn body(&self, f: &mut Facts, d: u32, ctx: Option<&Self::Ctx>) -> Self::Fact;
}

impl Facts<'_> {
    pub fn ask<K: Kind>(&mut self, k: &K, d: u32, ctx: Option<&K::Ctx>) -> K::Fact;
}
```

`ask` is the one place that reads the memo, answers a native, an out-of-range number and a
function under evaluation, and runs the body; a kind's `body` is today's walk with every
recursive question turned into an `ask`.  A whole-program kind (`effect_free`, `total`,
`entry`) is `solve::<K>()`: the fixpoint loop, its direction a property of the kind.

## The order

| step | moves | why this order |
|---|---|---|
| S1 | `Facts` holds the writes family's `cache` and `active`; every signature `(…, cache, active, …)` becomes `(…, facts, …)`; the bodies unchanged | the state first: a pure signature change, the diff empty by construction |
| S2 | `writes` (`callee_writes`) and its `WriteCache` into `Facts` | the same, one more map |
| S3 | `inputs` (`callee_inputs`) and its `InputCache` | its own cycle scope |
| S4 | `ask` and `Kind`; `stores_written`, `in_place_only`, `writes`, `inputs` as kinds | the memo and cycle code leaves the analyses |
| S5 | `buffer_only` as a kind (its thread-local open set gone) | its per-site context |
| S6 | `effect_free`, `total`, `entry` as `solve` kinds | the fixpoint loops leave the analyses |
| S7 | the own-body facts (`callee_reinits_buffer`, view returns, destinations, `written_params`, `callee_allowance`) read through `ask` with DEPTH `own` | one entry point for every callee question |

Each step is its own commit, gated as above.

**State.**  S1–S6 built: the writes family, `inputs` and `buffer_only` are kinds asked through
`Facts::ask`; `effect_free`, `total` and the non-sentinel parameters are `solve` fixpoints.
Gated from the binary before S1: the interpreter and native over 2,938 programs and the C
over 2,976 (the corpus and the 38 library benches, checked and ship units), byte-identical;
the rewrite census identical, rises included.  S7 is not taken as written: a writer builds a
handle per question over the memos it keeps, so a kind whose memo lives on the writer
(`view_return::callee`'s `vr_memo`) has nowhere to keep it; it waits for one long-lived handle
per writer (D-fact-1).

## Found while reading, for after the move

These are the disagreements the move makes visible.  Each is a separate change, measured by
the rewrite census and the emission diff, never folded into a move.

1. **A memo that ignores its context.**  `call_writes_store` and `in_place_only_writer` memoise
   by function number, while their answer depends on `HoistOwned::for_callee`; the policy
   (keep only the conclusive answer, or any answer when a context was given) limits the harm
   but does not remove it: a first ask with one caller's context fixes the answer for the
   next caller.
2. **Order-dependent cycle answers.**  The writes family memoises the answer it gave a
   function on a cycle (`callee_writes`: "a `None` reached on a recursive edge is memoised
   too"), so a function in a cycle answers differently depending on which member the walk
   entered first.  `Fact-Reach`'s CYCLES `solve` removes it.
3. **Four cycle rules for one question.**  The writes family cuts; `buffer_only` and
   `effect_free` assume the property; `total` refuses recursion.  After S6 each is a declared
   choice in one table, and unifying them is a census-measured change.
