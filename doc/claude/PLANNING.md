<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Enhancement Planning

## Goals

Loft aims to be:

1. **Correct** — programs produce the right answer or a clear error, never silent wrong results.
2. **Prototype-friendly** — a new developer should be able to express an idea in loft with minimal
   ceremony: imports that don't require prefixing every name, functions that can be passed and
   called like values, concise pattern matching, and a runtime that reports errors clearly and
   exits with a meaningful code.
3. **Performant at scale** — allocation, collection lookups, and parallel execution should stay
   efficient as data grows.
4. **Architecturally clean** — the compiler and interpreter internals should be free of technical
   debt that makes future features hard to add.
5. **Developed in small, verified steps** — each feature is complete and tested before the next
   begins.  No half-implementations are shipped.  No feature is added "just in case".  Every
   release must be smaller and better than its estimate, never larger.  This is the primary
   defence against regressions and against the codebase growing beyond one person's ability to
   understand it fully.

## What this doc holds

The backlog of SKETCHES: an item lives here while it is an idea with a fix path and no plan
yet.  When it is activated it becomes a `loft-lang/plans` issue (the canonical plan, with its
phases and status — [ISSUE_TRACKING.md § Plans](ISSUE_TRACKING.md)) and leaves this page;
[ROADMAP.md](ROADMAP.md) groups the open plans by the kind of value each buys.  Items are
ordered by tier: things that break programs come first, then language-quality and
prototype-friction items, then architectural work.  [RELEASE.md](RELEASE.md) has the release
gate and its checklist.

**Completed, superseded and withdrawn items are removed entirely** — they live in
[PLANNING-history.md](PLANNING-history.md), in git and in `CHANGELOG.md`.

**Before proposing a new item here, check [DESIGN_DECISIONS.md](DESIGN_DECISIONS.md)** —
that file holds the closed-by-decision register (features evaluated and explicitly
declined).  If the idea is already there, surface new evidence on the existing entry
instead of re-proposing it.

The ticket bodies are split by subject:

- this page — L (language quality), A (architecture), the ecosystem sketches;
- [PLANNING_STABILITY.md](PLANNING_STABILITY.md) — S-tier stability hardening; slot reuse (P70), closures (C47, C48), name clash (C52), match arms (C53);
- [PLANNING_NATIVE.md](PLANNING_NATIVE.md) — Tier N native codegen, O-tier performance, AOT libraries;
- [PLANNING_WEB.md](PLANNING_WEB.md) — H HTTP / JSON, R repository, W web IDE, E library ergonomics (C57); its first-run presentation design is [FIRST_CONTACT.md](FIRST_CONTACT.md).

---

## Contents
- [L — Language Quality](#l--language-quality)
  - [CO1 — Coroutines: three generator cells](#co1--coroutines-three-generator-cells-still-unwritten)
- [A — Architecture](#a--architecture)
  - [A2 — Logger: hot-reload, run-mode helpers, release + debug flags](#a2--logger-hot-reload-run-mode-helpers-release--debug-flags)
  - [A12 — Lazy work-variable initialization](#a12--lazy-work-variable-initialization) *(blocked)*
- [Ecosystem — `game_client`](#ecosystem--game_client)
- [See also](#see-also)

---

## L — Language Quality

### CO1  Coroutines: three generator cells still unwritten
**Sources:** [COROUTINE.md](COROUTINE.md)
**Description:** Stackful `yield`, `iterator<T>` and `yield from` are shipped
(COROUTINE.md; the phase record is in [PLANNING-history.md](PLANNING-history.md)).  Three
of the cells the design asked for are not written, each a suspension point whose iterator
state must survive the yield/resume cycle in `stack_bytes`:

| Test (`tests/expressions.rs`) | Shape | What it checks |
|---|---|---|
| `coroutine_yield_from_store_text_loop` | `for c in it.name { yield c; }` on a struct's `text` field | the field read's string and the `DbRef` to the record survive the resume |
| `coroutine_yield_from_whole_store` | `for n in Node { yield n.value; }` | the `DbRef` cursor survives serialisation; a mutation between `next()` calls is the generation guard's to catch |
| `coroutine_two_text_params` | `fn join_chars(a: text, b: text) -> iterator<character>` | `serialise_text_args` serialises every `text` parameter, not only the first |

The neighbouring cells to copy the shape from: `coroutine_yield_from_text_loop`,
`coroutine_yield_from_vector_loop`, `coroutine_text_local_after_yield`.  No new opcode is
expected: `coroutine_yield` / `coroutine_next` serialise the whole `[stack_base .. stack_pos)`
range, so a failing cell names a gap in that window, not a missing mechanism.

**Effort:** Small (three tests, a targeted fix if one fails)

---

## A — Architecture

### A2  Logger: hot-reload, run-mode helpers, release + debug flags
**Sources:** [LOGGER.md](LOGGER.md) § Remaining Work
**Description:** Four independent additions to the logging system.  The core framework
(production mode, source-location injection, log file rotation, rate limiting) is shipped;
these are the remaining pieces.
**Fix path:**

**A2.1 — Wire hot-reload** (`src/native.rs`):
Call `lg.check_reload()` at the top of each `n_log_*`, `n_panic`, and `n_assert` body so
the config file is re-read at most every 5 s.  `check_reload()` is already implemented.
*Tests:* write a config file; change the level mid-run; verify subsequent calls respect the new level.

**A2.2 — `is_production()` and `is_debug()` helpers** (`src/native.rs`, `default/01_code.loft`):
Two new loft natives read `stores.run_mode`.  The `RunMode` enum replaces the current
`production: bool` flag on `RuntimeLogConfig` so all runtime checks share one source of truth.
*Tests:* a loft program calling `is_production()` returns `true` under `--production`/`--release`
and `false` otherwise; `is_debug()` returns `true` only under `--debug`.

**A2.3 — `--release` flag with zero-overhead assert elision** (`src/parser/control.rs`, `src/main.rs`):
`--release` implies `--production` AND strips `assert()` and `debug_assert()` from bytecode
at parse time (replaced by `Value::Null`).  Adds `debug_assert(test, message)` as a
companion to `assert()` that is also elided in release mode.
*Tests:* a `--release` run skips assert; `--release` + failed assert does not log or panic.

**A2.4 — `--debug` flag with per-type runtime safety logging** (`src/fill.rs`, `src/native.rs`):
When `stores.run_mode == Debug`, emit `warn` log entries for silent-null conditions:
integer overflow, shift out-of-range, null field dereference, vector OOB.
*Tests:* a deliberate overflow under `--debug` produces a `WARN` entry at the correct file:line.

**Effort:** Medium (logger.rs, native.rs, fill.rs; see LOGGER.md for full design)

---

### A12  Lazy work-variable initialization
**Picked up only once its two blockers fall** (below): with them standing the change is
too disruptive for the stability work.  The record of the earlier attempt is in
[PLANNING-history.md](PLANNING-history.md).
**Description:** Work text variables (`__work_N`) are initialized at function start via
`Set(wt, Text(""))` inserted at index 0 of the body block.  This forces `first_def = 0` for
every work text variable, making its live interval span the entire function: two
sequential, non-overlapping text operations each hold a 24-byte slot for the full lifetime
of the call frame.  The same applies to non-inline work ref variables (`__ref_N`), which
also get function-start null-inits.

Inline-ref temporaries already use lazy insertion: their null-init is placed immediately
before the statement that first assigns them, giving accurate intervals.  This item extends
that approach to all work variables.

**Fix path:**

*Step 1 — Generalize `inline_ref_set_in`* (`src/parser/expressions.rs`): rename it to
`first_set_in` (or add that as a general helper).  No logic changes — the function already
recurses into the relevant `Value` variants and works for both text and ref work variables.

*Step 2 — Extend the insertion loop in `parse_code` to work texts*: replace the eager-insert
loop for work texts with a lazy insert using `first_set_in`; non-inline work references stay
eagerly inserted at position 0 while the first blocker stands.

```rust
// BEFORE: for wt in work_texts() { ls.insert(0, v_set(wt, Text(""))) }
// AFTER: find the first top-level statement containing a Set to wt, insert before it.
let mut insertions: Vec<(usize, u16, Value)> = Vec::new();
for wt in self.vars.work_texts() {
    let pos = ls.iter().position(|stmt| first_set_in(stmt, wt, 0)).unwrap_or(fallback);
    insertions.push((pos, wt, Value::Text(String::new())));
}
for r in self.vars.work_references() {          // still position 0: the first blocker
    if !is_argument && depend.is_empty() && !is_inline_ref {
        insertions.push((0, r, Value::Null));
    }
}
for r in self.vars.inline_ref_references() { ... lazy as before ... }
insertions.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
for (pos, r, init) in insertions { ls.insert(pos, v_set(r, init)); }
```

**Blockers:**

- `first_set_in` does not descend into `Block` / `Loop` nodes, so a work reference used only
  inside a nested block is not found and the fallback position lands *after* the block,
  giving `first_def > last_use`.  Fix: `Block` and `Loop` arms in `first_set_in`.
- Text slot sharing needs `OpFreeText` placed after each variable's last use rather than at
  function end; until then sequential work texts keep overlapping live intervals, and the
  `pos < TOS` bump-to-TOS override in `generate_set` cannot be extended to `Type::Text`
  (it faults in `append_fn` when a text slot is reused).

`assign_slots_sequential_text_reuse` in `src/variables/` runs unconditionally: the slot
allocator's side of text reuse holds; the two blockers above are the parser's.

**Effort:** Medium (two inter-related blockers)

---

## Ecosystem — `game_client`

The published libraries have their own repositories and plans
([LIBRARY_AUTHORING.md](LIBRARY_AUTHORING.md), the catalogue from `make libcatalogue`).  One
sketched library is not built: the client-side companion to `server`.

**`game_client` — multi-player game client** ([GAME_CLIENT_LIB.md](lib_plans/64-game-client/README.md)):
WebSocket connectivity, a typed game message protocol (envelope + dispatcher), lobby
management, a fixed-timestep game loop, client-side prediction with server reconciliation,
and dynamic WASM script loading.  WASM scripts are loft programs compiled with
`--native-wasm` and loaded at runtime by both client and server — guaranteeing identical
physics and rules without sending full state every tick.  Depends on `game_protocol`
(published, `loft-lang/loft-libs-net`).  Phases:

- **Phase 1** — WebSocket client + protocol: `WsClient`, `GameEnvelope`, `GameMessage`
  enum, `Dispatcher`.  Requires `server` and `game_protocol`.
- **Phase 2** — Lobby + fixed-timestep game loop.
- **Phase 3** — Client-side prediction + reconciliation + state delta sync + ping.
- **Phase 4** — WASM script loading: `WasmModule`, `wasm_load/call/verify`, Ed25519
  signature check.
- **Phase 5** — Shared game logic: document the `n_script_*` export interface; build
  an end-to-end example (Tic-Tac-Toe server + browser client + shared `rules.wasm`).

---

## See also
- [ROADMAP.md](ROADMAP.md) — the open plans, grouped by the value each buys
- [PLANNING-history.md](PLANNING-history.md) — the shipped, superseded and withdrawn entries
- [../../CHANGELOG.md](../../CHANGELOG.md) — completed work (all fixed bugs and shipped features)
- [PROBLEMS.md](PROBLEMS.md) — the closed-bug archive; open bugs are GitHub issues (`make work`)
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — language design asymmetries and surprises
- [SLOTS.md](SLOTS.md) — stack slot assignment (A12's ground)
- [LOGGER.md](LOGGER.md) — logger design (A2 detail)
- [COROUTINE.md](COROUTINE.md) — coroutine design (CO1 detail)
- [PACKAGES.md](PACKAGES.md) — external library packaging
- [../DEVELOPERS.md](../DEVELOPERS.md) — feature proposal process, quality gates, scope rules, and backwards compatibility
- [RELEASE.md](RELEASE.md) — the release process and its checklist
