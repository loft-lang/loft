
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

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

The items below are ordered by tier: things that break programs come first, then language-quality
and prototype-friction items, then architectural work.  See [RELEASE.md](RELEASE.md) for the full
release gate criteria, project structure changes, and release artifact checklist.

**Completed items are removed entirely** — history lives in git and `CHANGELOG.md`.
Cross-document links are at the end; this doc is for future work.

**Before proposing a new item here, check [DESIGN_DECISIONS.md](DESIGN_DECISIONS.md)** —
that file holds the closed-by-decision register (features evaluated and explicitly
declined).  If the idea is already there, surface new evidence on the existing entry
instead of re-proposing it.

---

## Contents
- [Version Milestones](#version-milestones)
  - [Milestone Reevaluation](#milestone-reevaluation)
  - [Recommended Implementation Order](#recommended-implementation-order)
- [S — Stability Hardening](#s--stability-hardening)
  - [S4 — Binary I/O type coverage (Issue 59, 63)](#s4--binary-io-type-coverage)
  - [S6 — `for` loop in recursive function](#s6--fix-for-loop-in-recursive-function----too-few-parameters-panic) *(1.1+)*
- [I — Interfaces](#i--interfaces) *(completed — I1–I8 + I9 stdlib; P136 loop bug open)*
- [P — Prototype Features](#p--prototype-features)
  - [T1 — Tuple types](#t1--tuple-types) *(1.1+)*
  - [CO1 — Coroutines](#co1--coroutines) *(1.1+)*
- [A — Architecture](#a--architecture)
  - [A1 — Parallel workers: extra args + value-struct + text/ref returns](#a1--parallel-workers-extra-arguments-value-struct-returns-and-textreference-returns) *(completed 0.8.3)*
  - [A12 — Lazy work-variable initialization](#a12--lazy-work-variable-initialization) *(deferred to 1.1+)*
  - [A13 — Complete two-zone slot assignment](#a13--complete-two-zone-slot-assignment-steps-8-and-10) *(completed 0.8.3)*
  - [A14 — `par_light`: lightweight parallel loop with pre-allocated stores](#a14--par_light-lightweight-parallel-loop-with-pre-allocated-stores)
  - [TR1 — Stack trace introspection](#tr1--stack-trace-introspection) *(completed 0.8.3)*
- [E — Library Ergonomics](#e--library-ergonomics)
  - [C57 — Route decorator syntax (`@get`, `@post`, `@ws`)](#c57--route-decorator-syntax) *(1.1+)*
- [N — Native Codegen](#n--native-codegen)
- [O — Performance Optimisations](#o--performance-optimisations)
  - [O1–O7 — Interpreter and native performance](#o1--superinstruction-merging) *(O1 deferred indefinitely — opcode table full; O2–O7 deferred to 1.1+)*
- [H — HTTP / Web Services](#h--http--web-services)
- [R — Repository](#r--repository)
- [W — Web IDE](#w--web-ide)
- [Quick Reference](#quick-reference) → [ROADMAP.md](ROADMAP.md)

---

## Version Milestones

Authoritative milestone definitions live in [ROADMAP.md](ROADMAP.md) and
[RELEASE.md](RELEASE.md).  High-level shape:

| Version | Goal                                       | Status      |
|---------|--------------------------------------------|-------------|
| 0.8.0–0.8.3 | Stability, native codegen, slot correctness, lambdas, parallel, stack trace, sprite sheet API | **Shipped** |
| 0.8.4   | **Awesome Brick Buster** — a game worth sharing on itch.io | In progress |
| 0.8.5   | **Working Moros editor** — paint hex scenes in the browser | Planned     |
| 0.9.0   | **Fully working loft language** — feature-complete + verified | Planned     |
| 1.0.0   | **Everything works** — IDE + multiplayer + stability contract | Planned     |

When updating priorities, edit ROADMAP.md / RELEASE.md first; this document
catches up later.

---

### Per-version ticket bodies

Per-version scoping (which tickets land in 0.8.4 / 0.8.5 / 0.8.6 / 0.9.0 / 1.0.0)
is authoritative in [ROADMAP.md](ROADMAP.md).  The full ticket text (L/I/A/S/N/H/R/W
tier entries below) is the source this document owns; ROADMAP.md references it by ID.

### Version 1.x — Minor releases (additive)

New language features that are strictly backward-compatible.  Candidates: A5 (closures),
A7 (native extensions), Tier N (native codegen), C57 (route decorator syntax).

When A5 (closures) lands, `server` middleware can be written as factory functions
instead of enum variants:

```loft
// Post-A5: middleware becomes a function returning a handler:
app.use_middleware(rate_limit(100));
app.use_middleware(require_roles(["admin"]));
```

---

### Version 2.0 — Breaking changes only

Reserved for language-level breaking changes (sentinel redesign, syntax removal).
Not expected in the near term.

---

### Ecosystem libraries (independent of interpreter version)

These are separate repositories installed via `loft install`.  They are not
gated to a specific interpreter milestone — they evolve alongside the interpreter
and publish their own version numbers.  Full designs live in their own documents.

**`server` — HTTP server library** ([WEB_SERVER_LIB.md](lib_plans/future/08-server/README.md)):
A fully featured HTTP server written mostly in loft with a thin native Rust layer
for TCP, TLS, WebSockets, ACME, and cryptographic primitives.  Phases:

- **Phase 1** — Plain HTTP: routing, middleware pipeline, request/response structs.
  Requires: interpreter 0.8.3 (lambdas for handler fn-refs), PKG Phase 2 (native
  extension loading).
- **Phase 2** — HTTPS with static PEM certificates.
- **Phase 3** — WebSocket support.
- **Phase 4** — Authentication: JWT, session, API key, HTTP Basic.
- **Phase 5** — ACME / Let's Encrypt automatic certificate provisioning and renewal.
- **Phase 6** — Advanced middleware: CORS, rate limiting, decompression, static files.

**`graphics` → `loft-lang/loft-libs-graphics`** (LIB.2, 0.9.0):
2D canvas, mesh, scene, and GLB export.  Migrated from `lib/graphics/` in the
main repo.

**`shapes` → `loft-lang/loft-libs-graphics`** (LIB.3, 0.9.0):
Shape primitives built on the graphics library.  Migrated from `lib/shapes/`.

**`web` — HTTP client** (H4, 0.8.4):
Blocking HTTP client and JSON response handling.  Lives in `loft-lang/loft-libs-net`.

**`game_protocol` — shared multiplayer protocol** (`loft-lang/loft-libs-net`):
Lightweight shared package depended on by both `server` (when used as a game server)
and `game_client`.  Contains the canonical `WsMessage` enum, `GameEnvelope` struct,
`MSG_*` constants, and all `Msg*` request/response structs.  Extracting these into a
separate package prevents the two libraries from diverging in their protocol definitions.
No native layer required — pure loft.  Phases: just one (all types defined at once).

**`game_client` — multi-player game client** ([GAME_CLIENT_LIB.md](lib_plans/64-game-client/README.md)):
Client-side companion to `server`.  Provides WebSocket connectivity, a typed game
message protocol (envelope + dispatcher), lobby management, fixed-timestep game loop,
client-side prediction with server reconciliation, and dynamic WASM script loading.
WASM scripts are loft programs compiled with `--native-wasm` and loaded at runtime by
both client and server — guaranteeing identical physics and rules without sending full
state every tick.  Depends on `game_protocol`.  Phases:

- **Phase 1** — WebSocket client + protocol: `WsClient`, `GameEnvelope`, `GameMessage`
  enum, `Dispatcher`.  Requires: interpreter 0.8.3, `server` Phase 1, `game_protocol`.
- **Phase 2** — Lobby + fixed-timestep game loop.
- **Phase 3** — Client-side prediction + reconciliation + state delta sync + ping.
- **Phase 4** — WASM script loading: `WasmModule`, `wasm_load/call/verify`, Ed25519
  signature check.  Requires: `--native-wasm` codegen (0.8.4 PKG.5).
- **Phase 5** — Shared game logic: document the `n_script_*` export interface; build
  an end-to-end example (Tic-Tac-Toe server + browser client + shared `rules.wasm`).

---

### Milestone Reevaluation

The previous plan had 1.0 as a language-stability contract for the interpreter alone,
with the Web IDE deferred indefinitely to "post-1.0".  This reevaluation changes both
milestones and adds the small-steps goal.  The reasoning:

**Why introduce 0.9.0?**
The old plan reached the current state (0.8.1) and declared "L1 is the last blocker
before 1.0", but that understated what "fully featured" actually requires.  Several items
(P1 lambdas, A9 vector CoW, A6 slot pre-pass, A8 string efficiency, A1
parallel completeness) are not optional polish — they close correctness and usability
gaps that a production-ready interpreter must not have.  A 0.9.0 milestone gives these
items a home without inflating the 1.0 scope.

**Why include the IDE in 1.0.0?**
A standalone interpreter 1.0 that is later extended with a breaking IDE integration
produces two separate stability contracts to maintain.  The Web IDE (W1–W6) is already
concretely designed in [WEB_IDE.md](lib_plans/62-web-ide/README.md) and is bounded, testable work.  Deferring
it to "post-1.0" without a milestone risks it never shipping.  In 2026, "fully featured"
for a scripting language includes browser-accessible tooling; shipping a 1.0 without it
would require walking back that claim at 1.1.

**Why include native codegen (Tier N) in 0.8.2?**
`src/generation/` already translates the loft IR to Rust source; the code exists but
does not compile.  The N items are incremental bug fixes — each is Small or Medium effort,
independent of each other and of the other 0.8.2 items — they can be interleaved freely.
Fixing them in 0.8.2 means 0.9.0 ships a binary where `--native` actually works, at no
extra milestone cost.  Deferring them would mean shipping a 0.9.0 that silently generates
uncompilable output.

**Why include REPL (P2) in 0.9.0?**
The Web IDE covers the browser-based interactive use case, but a terminal REPL is
independently useful for development workflows where a browser is not available or
convenient.  P2 is self-contained (new `src/repl.rs`, small changes to `main.rs`)
and depends on L1 (error recovery) which is already in 0.9.0.  Including it rounds
out the "prototype-friendly" goal without affecting the IDE track.

**Why split syntax into 0.8.3?**
Lambda expressions, nested patterns, and field iteration all touch the parser and type
system simultaneously.  Grouping them in a dedicated milestone means syntax decisions can
be reviewed and refined in isolation, before runtime infrastructure work in 0.9.0 begins.
It also keeps each milestone small enough to be fully understood in a single pass.

**The small-steps principle in practice:**
Each milestone is a strict subset of the next.  0.8.2 hardens correctness; 0.8.3 adds new
syntax; 0.8.4 adds HTTP and JSON on top of lambdas; 0.9.0 completes runtime infrastructure
and tooling; 0.8.3 adds R1 + W1 (WASM runtime); 1.0.0 adds W2–W6 (IDE) on top of a complete 0.9.0.  No item moves
forward until the test suite for the previous item is green.  This prevents the "everything
at once" failure mode where half-finished features interact and regressions are hard to pin.

---

### Recommended Implementation Order

Ordered by unblocking impact and the small-steps principle (each item leaves the codebase
in a better state than it found it, with passing tests).

**Released as 0.8.2 (2026-03-24).**

**For 0.8.3 (after 0.8.2 is tagged):**
1. **P3** + **L2** — aggregates and nested patterns; P3 depends on P1 (done in 0.8.2); batch together
2. **P5** — generic functions; independent of P3/L2; land after data.rs changes settle
3. **A10** — field iteration; independent, medium; can land in parallel with P3

**For 0.8.4 (after 0.8.3 is tagged):**
1. **REG.1** — Registry file parser + download/extract; Small, independent of H-items; adds `ureq` + `zip` under `registry` feature
2. **REG.2** — `loft install <name>` CLI extension; Small, depends on REG.1; no parser changes
3. **REG.3** — `loft registry sync`; Small, reuses `ureq` from REG.1; adds `source:` header parsing
4. **REG.4** — `loft registry check` + `list`; Small, pure filesystem + registry parsing; no new deps
5. **H1** — `#json` + `to_json`; Small, no new Rust deps; validates annotation parsing
4. **H2** — JSON primitive stdlib; Small–Medium, new `src/database/json.rs` (~80 lines, no new dep); test each extractor in isolation
5. **H3** — `from_json` scalar codegen; Medium, depends on H1 + H2; verify `Type.from_json` as fn-ref
8. **H4** — HTTP client + `HttpResponse`; Medium, `ureq` already present from REG.1; test against httpbin.org or mock
7. **H5** — nested/array/enum `from_json` + integration tests; Med–High, depends on H3 + H4

**For the 0.8.5 → 0.8.6 → 0.9.0 advertising-readiness sequence (after 0.8.4 is tagged):**

Ordered by (immediate leverage) × (low scope risk) ×
(dependencies-on / unblocking), now split across three releases so
each ship is a standalone tag with its own CHANGELOG entry.  The
split was introduced after today's "can we advertise loft?"
assessment; see
[ROADMAP.md § 0.8.5](ROADMAP.md#085--loft-is-learnable),
[§ 0.8.6](ROADMAP.md#086--loft-is-extensible), and
[§ 0.9.0](ROADMAP.md#090--fully-working-loft-language) for per-release
scope and ship criteria.

**Release 0.8.5 — "loft is learnable" (~2 weeks)**

1. **DX.4** — native CI parity.  Promote `tests/native.rs` into the
   fast `cargo nextest run --profile ci` gate.  One CI config
   change + timing-budget check.  Start here — smallest item,
   biggest safety return.  Catches P143/P144/P157/P171/P180-class
   regressions pre-commit instead of mid-release.
2. **SH.1 + SH.2** — TextMate grammar + VS Code extension.  Land
   together since SH.2 consumes SH.1.  Gives newcomers syntax
   highlighting + a "Run loft file" task button.  Needed before
   DX.3 so the tutorial can screenshot real VS Code.
3. **DX.1** — quick-start `examples/` directory at repo root.
   XS effort.  Gathers scattered examples
   (`lib/graphics/examples/*.loft`, brick-buster, moros-editor)
   under a discoverable path with one-paragraph READMEs.  Feeds
   DX.3.
4. **DX.3** — "Learn loft in 30 minutes" walkthrough.  Writing
   work, not coding.  Start from the house-scene canvas demo
   (already working, gold-tested) and narrate forward.  Single
   GitHub Pages page.  Depends on SH.1 / SH.2 / DX.1 for concrete
   screenshots and referential examples.

Ship criterion: one external programmer can install SH.2 from VS
Code Marketplace, open an example, read DX.3 top-to-bottom, and
run the demo within 30 minutes from zero prior exposure.

**Release 0.8.6 — "loft is extensible" (~3 weeks)**

5. **FFI.1 → FFI.2 → FFI.3 → FFI.4** in that order.  Generic
   type marshaller, generic cdylib loader, per-function glue
   elimination, docs.  Each is S to MH.  Landing all four
   together shrinks the boilerplate bar for extracted libraries —
   `lib/graphics/native/` has ~15 hand-written type-punning
   functions today that FFI.1–3 subsume.  Prerequisite for
   0.9.0's PKG.EXTRACT.
6. **PKG.7** — lock file (`loft.lock`).  Cheap, precedes PKG.REG.
7. **PKG.REG** — registry MVP.  Design is complete in
   [PACKAGES.md](PACKAGES.md); implementation is the
   `loft install <name>` fetcher + a `registry.txt` on GitHub
   seeded with 3–5 first-party libraries that stay in-repo
   for 0.8.6 (extraction is 0.9.0's PKG.EXTRACT).

Ship criterion: `loft install <name>` resolves and installs from
the public registry for at least 3 libraries; a third-party
library outside the `loft` repo proves the registry is genuinely
federated.

**Release 0.9.0 — "fully working loft language" (~6 weeks)**

8. **L1** — error recovery after token failures.  Standalone UX
   improvement; RELEASE.md H blocker for 0.9.0.  Also unblocks
   P2.4.
9. **A2** — logger remaining work.  Independent, small-medium;
   can land any time a hand's free.
10. **P2** — REPL.  High effort; RELEASE.md H blocker.  Land
    after L1 (needed for P2.4 error recovery).
11. **W-warn** — developer warnings (Clippy-inspired).  RELEASE.md
    M blocker.
12. **AOT** — auto-compile libraries to native shared libs.
    Medium; design in PLANNING.md.
13. **C52** — stdlib name clash warning + `std::` prefix.
    RELEASE.md M blocker.
14. **C53** — match arms for library enums + bare variant names.
    Medium.
15. **CS.B / CS.C1 / CS.C2 / CS.C3** — compilation cache finish.
    CS.C1 is the biggest (MH — ~2K lines of recursive-enum binary
    serialisation in `data.rs`); budget a focused run.
16. **P117 / P120 / P121 / P124** — verification passes (RELEASE.md
    M blockers).  Each is a hands-on re-run of a fix that landed
    earlier; not reopening the bug.
17. **PKG.EXTRACT** — **last 0.9.0 item**.  Move every `lib/*/`
    into separate GitHub projects (logical bundling allowed — see
    [ROADMAP.md § 0.9.0 Library extraction](ROADMAP.md#library-extraction)).
    Depends on PKG.REG + DX.4 + FFI.1–4 (all shipped in 0.8.5 /
    0.8.6).  Starting earlier duplicates work; starting later is
    fine.  L effort per bundle; moves happen one family at a time
    ("extract loft-moros, land, extract loft-net, land, …") so a
    failed extract doesn't strand the others.

**Explicitly excluded from the 0.8.5 → 0.9.0 window** to avoid scope creep:
- LSP — stays in 1.0.0 per roadmap.  Months-long on its own.
- HTTP stdlib / `server` / `game_client` libraries — 1.1+ (`WEB_SERVER_LIB.md`, `GAME_CLIENT_LIB.md`).
- Moros hex RPG editor (web version) — [independent lifecycle](ROADMAP.md#demo-applications--independent-lifecycles); does not gate any language tag.

**For 1.0.0 (after 0.9.0 is tagged):**
7. **R1** — workspace split; small change, unblocks all Tier W
8. **W1** — WASM foundation; highest risk in the IDE track; do first
9. **W2** + **W4** — editor shell + multi-file projects; can develop in parallel after W1
10. **W3** + **W5** — symbol navigation + docs browser; can follow independently
11. **W6** — export/import + PWA; closes the loop

---

## L — Language Quality

### L1  Error recovery after token failures — **Partially done**
**Sources:** [DEVELOPERS.md](../DEVELOPERS.md) § "Diagnostic message quality" Step 5
**Severity:** Medium — a single missing `)` or `}` produces a flood of cascading errors

`Lexer::recover_to(targets)` landed in `src/lexer.rs`: linear scan forward
over the token stream, balances `{`/`(`/`[` and their closers so a target
inside a nested group does not falsely terminate recovery.  The target is
NOT consumed — the caller decides.

Applied at the statement-boundary site in `src/parser/control.rs::parse_block`
(after a failed `token(";")`) with targets `[";", "}"]`.  A missing `;`
inside a function body now produces a single diagnostic instead of the
previous 4-error cascade.
**Tests:** `tests/parse_errors.rs::l1_missing_semicolon_single_diagnostic`,
`l1_missing_semicolon_in_body_single_diagnostic`.

**Remaining work:** apply `recover_to` at other cascading-prone call sites
— `parse_arguments::token(")")`, `parse_block::token("}")` at block end,
match-arm `token("=>")`, struct-literal `token(",")` after a missing field
value.  Each is small but needs per-construct target lists.
**Effort:** Small per site.
**Target:** 0.9.0

---


**Severity:** Low–Medium — a REPL dramatically reduces iteration time when exploring data
or testing small snippets
**Description:** Running `loft` with no arguments (or `loft --repl`) enters an
interactive session where each line or block is parsed, compiled, and executed immediately.
State accumulates across lines (variables and type definitions persist).
```
$ loft
> x = 42
> "{x * 2}"
84
> struct Point { x: float, y: float }
> p = Point { x: 1.0, y: 2.0 }
> p.x + p.y
3.0
```
**Fix path:**

**Phase 1 — Input completeness detection** (`src/repl.rs`, new):
A pure function `is_complete(input: &str) -> bool` that tracks brace/paren depth to decide
whether to prompt for more input.  No parsing or execution involved.
*Tests:* single-line expressions return `true`; `fn foo() {` returns `false`;
`fn foo() {\n}` returns `true`; unclosed string literal returns `false`.

**Phase 2 — Single-statement execution** (`src/repl.rs`, `src/main.rs`):
Read one complete input, parse and execute it in a persistent `State` and `Stores`; no
output yet.  New type definitions and variable bindings accumulate across iterations.
*Tests:* `x = 42` persists; a subsequent `x + 1` evaluates to `43` in the same session.

**Phase 3 — Value output**:
Non-void expression results are printed automatically after execution; void statements
(assignments, `for` loops) produce no output.
*Tests:* entering `42` prints `42`; `x = 1` prints nothing; `"hello"` prints `hello`.

**Phase 4 — Error recovery**:
A parse or runtime error prints diagnostics and the session continues; the `State` is
left at the last successful checkpoint.
*Tests:* entering `x =` (syntax error) prints one diagnostic and re-prompts;
`x = 1` then succeeds and `x` holds `1`.

**Effort:** High (main.rs, parser.rs, new repl.rs)
**Target:** 0.9.0

---


### T1  Tuple types
**Sources:** TUPLES.md
**Description:** Multi-value returns and stack-allocated `(A, B, C)` compound values. Enables functions to return more than one value without heap allocation. Seven implementation phases; full design in [TUPLES.md](TUPLES.md).

- **T1.1** — Type system *(completed 0.8.3)*: `Type::Tuple(Vec<Type>)` variant, `element_size`, `element_offsets`, `owned_elements` helpers in `data.rs`.
- **T1.2** — Parser *(completed 0.8.3)*: type notation `(A, B)`, literal syntax `(expr, expr)`, element access `t.0`, LHS destructuring `(a, b) = expr`.  `Value::Tuple` IR variant added.
- **T1.3** — Scope analysis *(completed 0.8.3)*: tuple variable intervals, owned-element cleanup tracking in `scopes.rs`.
- **T1.4** — Bytecode codegen *(completed 0.8.3)*: `Value::TupleGet` IR, element read via `OpVar*` at offset, tuple set via per-element `OpPut*`, tuple parameters.  6 tests passing; function-return convention, text elements, destructuring, and element assignment remain for follow-up.
- **T1.5** — *(completed 0.8.3)* SC-4: `RefVar(Tuple)` element read/write via `OpVarRef` + element offset; `parse_ref_tuple_elem` helper in operators.rs.
- **T1.6** — *(completed 0.8.3)* SC-8: `check_ref_mutations` emits WARNING (not error) for `RefVar(Tuple)` params never written; `find_written_vars` recognises `TuplePut`.
- **T1.7** — *(completed 0.8.3)* SC-7: `Type::Integer` gains a `not_null: bool` third field; `parse_type` accepts `not null` suffix; null assigned to a `not null` tuple element is a compile error.

- **T1.8** — Tuple function return convention + struct-ref element lifetime tracking.
  Three sub-issues remain after T1.1–T1.7:

  **T1.8a — Function return convention** *(closed 2026-05-04 by @PLAN14 phase 01 follow-up)*: actual-error survey showed the original design (new `ReturnTuple` IR variant + `OpReturnTuple(size)` opcode + caller-pre-allocated slot) was over-engineered.  Basic `-> (integer, integer)` returns already worked end-to-end on both backends; only **tuple-of-text returns under `--native`** failed, with three coupled type mismatches (signature `-> (Str, Str)` vs body `("alpha", "beta")` vs caller `(String, String)`).  Fix: `rust_type` for `Type::Tuple` in `Context::Result` now recurses with `Context::Variable` for elements (so signature is `(String, String)`); `Value::Return` sets the existing `tuple_text_to_string` flag when returning a tuple-of-text literal; `output_set` adds a `tuple_text_elem_clone` arm for destructure-from-tuple-text-element so `var_t.0.clone()` is emitted instead of `&var_t.0.to_string()` (which yields `&String`).  Pinned by `e1_d2_return_int_int` and `e2_d2_return_text_text` un-ignored cells in `tests/tuple_matrix.rs`.  Plan-06 phases 9b–9e remain in @PLAN06's scope; the par-specific dispatch (worker tuple returns, fused-for destructure) is independent of this fix.

  **T1.8b — Text elements:** `Type::Text` inside a `Type::Tuple` needs lifetime tracking and `OpFreeRef`-style cleanup for the text slot on scope exit.  `owned_elements` in `data.rs` must enumerate text positions within a tuple so `get_free_vars` can emit the right cleanup sequence.

  **T1.8c — Struct-ref (DbRef) tuple elements: move vs copy semantics.**
  *(closed 2026-05-11 by Plan-14 phase 04)* — **MOVE semantics**
  selected and validated against the cross-mode harness.  Already
  implemented by `src/scopes.rs:1000-1009`'s tuple scope-exit
  `continue` stub (no per-element `OpFreeRef` on the source tuple)
  + `parser/expressions.rs:1252-1278`'s destructure path which types
  each destination variable as the source element's `Type::Reference`
  (via `change_var_type(v_nr, &rhs_elems[i])`), so each `q1`/`q2`
  gets ordinary scope-exit cleanup.  The "copy + null" alternative
  was rejected (it adds an opcode — not the scarce resource this
  once assumed, INTERMEDIATE.md § Opcode budget — and is not
  observably different from move semantics).  6 cross-
  mode E5 cells lock the canonical shapes (swap, arg, return,
  mixed Ref+int, mixed Ref+text); the loop-iteration aliasing
  shape is parked behind @P250 as a separate dep-tracking bug.

- **T1.9** *(completed 0.8.3)* — Tuple destructuring in `match`.  See [TUPLES.md](TUPLES.md).

  `Type::Tuple` dispatch added to `parse_match`; new `parse_tuple_match` handles wildcard
  (`_`), binding, and literal patterns. AND conditions use `v_if(a,b,false)` (no OpAnd).
  Tests: `tuple_match_wildcard`, `tuple_match_literal`, `tuple_match_binding`.

- **T1.10** *(completed 0.8.3)* — Same-element-type tuple coverage across data sources:

  T1.1–T1.8b verified tuples with *mixed* element types (`(integer, text)`,
  `(integer, float)`, etc.) but left same-element-type (homogeneous) tuples
  undertested, especially when the elements come from sources other than simple
  literals.  This item adds tests for four practically important categories,
  mirroring the CO1.7 iterator-source matrix.

  **1 — Text elements (homogeneous text tuple)**
  ```
  fn make_greeting(first: text, last: text) -> (text, text) {
      ("Hello " ++ first, last)
  }
  (g, s) = make_greeting("World", "!");
  assert(g == "Hello World" && s == "!");
  ```
  Both elements are `text`.  Verifies that `T1.8b` lifetime tracking and
  `OpPutText` work correctly when *all* tuple positions are text slots, not just
  one mixed into scalars.  The `owned_elements` cleanup must emit `OpFreeRef`
  for both positions at scope exit.

  **2 — Store-backed text (text from a struct field)**
  ```
  struct Label { name: text }
  fn label_pair(a: Label, b: Label) -> (text, text) {
      (a.name, b.name)
  }
  la = Label { name: "alpha" };
  lb = Label { name: "beta" };
  (n1, n2) = label_pair(la, lb);
  assert(n1 == "alpha" && n2 == "beta");
  ```
  Elements are texts read from struct record fields (heap-allocated strings).
  Verifies that reading a `text` field and storing it into a tuple element does
  not produce a dangling reference: the field read returns a `Str` backed by the
  store, but the tuple element must be a self-contained owned value.

  **3 — Struct record references (whole-store elements)**
  ```
  struct Point { x: integer, y: integer }
  fn two_points(a: Point, b: Point) -> (Point, Point) {
      (b, a)            // swap
  }
  p1 = Point { x: 1, y: 2 };
  p2 = Point { x: 3, y: 4 };
  (q1, q2) = two_points(p1, p2);
  assert(q1.x == 3 && q2.x == 1);
  ```
  Both elements are `Type::Reference` (12-byte `DbRef`).  Verifies that two
  adjacent DbRef slots in a tuple are laid out correctly and that element access
  (`q1.x`) produces the right field read after destructuring.

  **4 — Elements sourced from a vector**
  ```
  fn first_two(v: vector<integer>) -> (integer, integer) {
      (v[0], v[1])
  }
  nums = [10, 20, 30];
  (a, b) = first_two(nums);
  assert(a == 10 && b == 20);
  ```
  Both elements come from indexed vector reads.  Verifies that the vector-element
  `OpVarInt` / index-add path produces the correct values in consecutive tuple
  slots and that destructuring (`(a, b) = ...`) correctly assigns each slot.

  **Tests to add** (`tests/expressions.rs`, T1.10 section, or extend `tests/scripts/50-tuples.loft`):

  | Test name | Element type | Checks |
  |-----------|-------------|--------|
  | `tuple_homogeneous_text` | `(text, text)` | both text slots live/freed correctly |
  | `tuple_store_text_fields` | `(text, text)` from struct fields | field-text into tuple element |
  | `tuple_struct_refs` | `(Point, Point)` | two DbRef slots, field access after destruct |
  | `tuple_from_vector_elements` | `(integer, integer)` from vector | index read into tuple slots |

  All 4 tests now active.  `tuple_struct_refs` un-ignored 2026-05-11
  by Plan-14 phase 04 with the **MOVE semantics** decision (T1.8c
  closed); the cross-mode harness's e5_d1_struct_ref_swap cell carries
  the same shape on both backends.

- **T1.11** *(completed 0.8.3, T1.11a status updated 2026-05-11)* — Tuple type constraints:

  T1.11a (struct field rejection) was REVERSED by Plan-06 phase 4d
  and validated by Plan-14 phase 05: tuples are now accepted as
  struct field types, lay out their elements inline using the
  synthetic `__tuple<…>` struct's positions, and round-trip through
  `parser/mod.rs::set_field_check`'s Tuple arm and `get_val`'s Tuple
  arm.  6/7 D3 cells green on both backends; the seventh
  (e4_d3_field_closure_local — closure-element tuple as struct
  field) is parked behind @P251.

  T1.11b: `parse_assign` in `expressions.rs` returns early (both passes) when a compound
  operator follows a tuple LHS; consumes the operator and RHS to keep parser state clean.
  Tests: `tuple_compound_assign_rejected` (T1.11a's negative test
  `tuple_in_struct_field_rejected` was removed when the lift shipped).

**Effort:** Very High
**Target:** 1.1+

---

### CO1  Coroutines
**Sources:** COROUTINE.md
**Description:** Stackful `yield`, `iterator<T>` return type, and `yield from` delegation. Enables lazy sequences and producer/consumer patterns without explicit state machines. Six implementation phases; full design in [COROUTINE.md](COROUTINE.md).

- **CO1.1** — *(completed 0.8.3)* `CoroutineStatus` enum in `default/05_coroutine.loft`; `CoroutineFrame` struct, coroutine storage, and helpers on State.
- **CO1.2** — *(completed 0.8.3)* `OpCoroutineCreate` + `OpCoroutineNext` opcodes: frame construction (argument copy, COROUTINE_STORE DbRef push) and advance (stack restore, call-frame restore, state machine).
- **CO1.3** — `OpYield` + `OpCoroutineReturn` + parser `yield` keyword.  Split into five independently testable sub-steps:

  **CO1.3a — `OpCoroutineReturn` opcode** *(completed 0.8.3)*:
  `coroutine_return(value_size)` on State: clears text_owned/stack_bytes, truncates
  call_stack, marks Exhausted, pops active_coroutines, pushes null, returns to consumer.
  Fixes #96.

  **CO1.3b — `OpCoroutineYield` opcode (integer-only)** *(completed 0.8.3)*:
  `coroutine_yield(value_size)` on State: serialises stack[stack_base..stack_pos] into
  stack_bytes, saves call frames, suspends, slides yielded value to stack_base, returns
  to consumer.  Text serialisation deferred to CO1.3d.  Fixes #95.

  **CO1.3c — Parser: `yield` keyword + codegen emit** *(completed 0.8.3)*:
  `yield` lexer keyword added.  `yield expr` parsed as `Value::Yield(Box<Value>)`.
  `iterator<T>` single-parameter syntax accepted.  Codegen: OpCoroutineCreate for
  generator calls, OpCoroutineYield for yield, OpCoroutineReturn for generator return.
  Remaining: generator body return-type check suppression and `next()` wiring.

  **CO1.3d — Text serialisation** *(completed 0.8.3)* (`src/state/codegen.rs`, `src/state/mod.rs`):
  Two root causes for SIGSEGV in generators with `text` parameters: (1) `coroutine_create`
  now appends a 4-byte return-address slot to `stack_bytes` so `get_var` offsets match the
  codegen-time layout on every resume; (2) `Value::Yield` codegen decrements `stack.position`
  by the yielded value size after emitting `OpCoroutineYield`, so subsequent variable accesses
  use correct offsets on second and later resumes.  Fixes #94.

  **CO1.3e — Nested yield** *(completed 0.8.3)*:
  Call-stack save/restore in `OpCoroutineYield` / `OpCoroutineNext` verified for nested
  helper calls between yields.

- **CO1.4** — *(completed 0.8.3)* `yield from sub_gen` parsed and desugared to
  advance-loop + yield forwarding.

  **CO1.4-fix** — *(completed)* The slot-assignment regression (C21) was resolved
  by the two-zone slot redesign (S17/S18): the `__yf_sub` coroutine handle and
  inner loop temporaries no longer overlap.  Test `coroutine_yield_from` passes
  without `#[ignore]`.
- **CO1.5** — *(completed 0.8.3)* `for item in generator` integration + `e#remove` rejection.
- **CO1.3e** — *(completed 0.8.3)* Nested yield verified — helper call between yields.

- **CO1.6** — *(completed 0.8.3)* `next()` / `exhausted()` stdlib, stack tracking fix,
  null sentinel on exhaustion.  `OpCoroutineNext` and `OpCoroutineExhausted` bypass the
  operator codegen path; stack.position manually adjusted.  `push_null_value` writes
  `i32::MIN` / `i64::MIN` for typed null returns.

- **CO1.7 — Yield from inside for-loops over multiple collection types** (0.8.3):

  Existing tests only yield from simple sequential `yield expr;` statements.  This item
  verifies that the coroutine save/restore machinery is correct when a `yield` occurs
  *inside* a `for` loop body — a structurally different suspension point where the
  iterator state (index variable, text byte offset, DbRef) must survive the yield/resume
  cycle in `stack_bytes`.

  Four collection types are tested, each combined with at least one plain `yield` outside
  the loop so that both suspension-from-loop and suspension-from-statement are exercised
  in the same generator:

  **1 — Text (character iteration)**
  ```
  fn yield_chars(s: text) -> iterator<character> {
      yield ' ';                         // plain yield before loop
      for c in s { yield c; }           // yield inside text loop
  }
  // consumer: collect chars from yield_chars("ab") → [' ', 'a', 'b']
  ```
  The text-loop iterator state is two `i32` slots (`{id}#next` byte offset and
  `{id}#index`).  Both must be serialised to `stack_bytes` at yield and restored on
  resume; the text parameter/local itself must also survive (CO1.3d already handles this,
  but the combination is not yet tested).

  **2 — Store-backed string (text field of a struct record)**
  ```
  struct Item { name: text }
  fn yield_name_chars(it: Item) -> iterator<character> {
      yield ' ';
      for c in it.name { yield c; }
  }
  ```
  `it.name` is a `text` field on a heap-allocated struct record.  The field read
  returns a live `String` reference; the text-loop position variables for `c` index
  into that string.  Verifies that field-text iteration inside a generator does not
  corrupt the DbRef to the struct record across yield/resume.

  **3 — Whole store (all records of a struct type)**
  ```
  struct Node { value: integer }
  fn yield_all_values() -> iterator<integer> {
      yield 0;                           // sentinel before loop
      for n in Node { yield n.value; }  // iterate every Node record
  }
  ```
  Store iteration uses a `DbRef`-based index variable; the `DbRef` cursor must survive
  serialisation.  Any structural mutation of the Node store between `next()` calls is
  already caught by S28's generation-counter guard in debug builds.

  **4 — Vector elements**
  ```
  fn yield_vec_items(v: vector<integer>) -> iterator<integer> {
      yield -1;                          // sentinel before loop
      for e in v { yield e; }
      yield -2;                          // sentinel after loop
  }
  ```
  Vector iteration uses an integer index variable.  The `vector<integer>` argument is
  copied to a temp at loop entry (`vec_var`); the temp DbRef and the index must both
  survive yield/resume.

  **Implementation notes:**

  No new opcodes are needed.  The existing `coroutine_yield` / `coroutine_next` path
  serialises the full `[stack_base .. stack_pos)` range to `stack_bytes`, which covers
  all iterator state variables regardless of loop kind.  If any test fails it will
  indicate a specific gap in the serialisation (e.g. text-loop position variables not
  being included in the saved slice, or a DbRef cursor being relative to a stack pointer
  that shifts after resume).

  **Tests to add** (`tests/expressions.rs`, CO1.7 section, or extend `tests/scripts/51-coroutines.loft`):

  | Test name | Collection type | Checks |
  |-----------|----------------|--------|
  | `coroutine_yield_from_text_loop` | `text` literal | char sequence, plain yield before loop |
  | `coroutine_yield_from_store_text_loop` | text field of struct | field-text chars, DbRef survives |
  | `coroutine_yield_from_whole_store` | whole struct store | all records yielded |
  | `coroutine_yield_from_vector_loop` | `vector<integer>` | pre/post sentinels + all elements |

**Effort:** Very High
**Depends:** TR1
**Target:** 0.8.3 (CO1.1–CO1.6 completed; CO1.7 in progress)

---

**CO1.8 — Coroutine generator: multi-text and nested-block safety** (0.8.3, depends on CO1.3d ✓):

CO1.3d fixed text serialisation for the common single-text-parameter case.  Three
related gaps are not yet tested and may still corrupt memory:

**CO1.8a — Multiple text parameters:**

A generator with two or more `text` parameters must serialise all of them on
`coroutine_create`, not only the first.  `serialise_text_args` iterates attribute
definitions by index; the test only covers a single text param.

```loft
fn join_chars(a: text, b: text) -> iterator<character> {
    for c in a { yield c; }
    for c in b { yield c; }
}
// consumer: collect all → chars of "hello" ++ chars of "world"
```

If only `a` is serialised and `b` is not, the second `for c in b` loop yields
garbage after the first resume.

**CO1.8b — Text locals created after first yield:**

A text local that is assigned inside the generator body (after a `yield`) is
allocated as a Zone-2 slot.  `parse_code` inserts `v_set(wv, Text(""))` for it,
so the slot is initialised on entry.  On resume, `coroutine_next` restores
`stack_bytes` but does NOT re-run the initialisations — the slot gets its
value from `stack_bytes`.  If the serialisation window does not include the
zone-2 slot (e.g. if `stack_base` was snapshotted before the slot was pushed),
the text local is zeroed on resume.

```loft
fn lazy_labels() -> iterator<text> {
    yield "first";
    let label = "second";   // text local created after first yield
    yield label;
}
```

If `label`'s slot is outside `[stack_base .. stack_pos)` at the first yield,
it will be zero on resume and `yield label` outputs garbage.

**CO1.8c — Text locals in deeply nested blocks:**

`drop_text_locals_in_bytes` (S25.3) frees text locals that are alive in
`stack_bytes` when a coroutine is freed.  It handles the simple case (text
locals in the generator body at top scope).  Deeper nesting — text locals
inside a `for` loop that is inside an `if` branch that is inside the generator
— may produce additional text slots that `drop_text_locals_in_bytes` does not
walk.  Result: memory leak on generator exhaustion or early `break`.

```loft
fn conditional_labels(v: vector<text>) -> iterator<text> {
    if v.size > 0 {
        for item in v {
            let upper = item.upper();   // text local in nested block
            yield upper;
        }
    }
}
```

**Concrete source locations and fix paths:**

**CO1.8a — `src/state/mod.rs`, `serialise_text_args` (line 474)**

The loop already iterates ALL `def.attributes` and increments `byte_offset` per
attribute size — it does not stop at the first text parameter.  The existing
implementation is likely correct; the fix is to write the test and confirm.  If the
test fails, check the `break` condition at line 494:
```rust
if byte_offset >= args_size as usize { break; }
```
If any text attribute is laid out past the `args_size` boundary this guard would
prematurely exit.  Fix: compute `args_size` from the full attribute list rather than
from `stack_pos - args_base`; or remove the guard and let the offset check at
`off + size_of::<Str>() <= stack_bytes.len()` handle bounds.

**CO1.8b — `src/state/mod.rs`, `coroutine_yield` and `generator_zone2_size` (lines 350–395)**

At first resume `coroutine_next` zeros the Zone-2 region
(`generator_zone2_size` bytes past the arg region).  `parse_code` inserts
`v_set(wv, Text(""))` for every Zone-2 text variable, so the slot is
initialised to an empty `Str` on entry.  At the first yield, `coroutine_yield`
snapshots `[stack_base..stack_pos)` — `stack_base` is set to the bottom of the
current call frame, which includes both the arg region and the Zone-2 region.
This means the empty-`Str` value for `label` IS captured in `stack_bytes` at the
first yield; on resume the slot is restored with the empty `Str`; `label = "second"`
then overwrites it correctly.

If `coroutine_text_local_after_yield` fails, verify that `stack_base` at yield time
equals the start of the generator's call frame (not the start of the arg region
only).  The relevant line in `coroutine_yield` is the snapshot:
```rust
let snap = &self.database.store(&self.stack_cur)
    .as_bytes()[stack_base as usize .. self.stack_pos as usize];
frame.stack_bytes = snap.to_vec();
```
If `stack_base` was advanced past Zone-2 init, extend it back to
`args_base - zone2_size`.

**CO1.8c — `src/state/mod.rs`, `drop_text_locals_in_bytes` (line 398)**

The function already walks ALL variables in `def.variables` (not a fixed window)
and uses an offset-bounds check:
```rust
if off + std::mem::size_of::<String>() > bytes.len() { continue; }
```
Variables in nested blocks have their own stack slots that are part of the same
function frame; as long as their slot offset is within `bytes.len()`, they are freed.

If `coroutine_text_local_nested_block` leaks, the failure mode is: the text local's
slot was allocated AFTER the yield snapshot was taken (i.e., the nested block was
never entered before the yield), so `off >= bytes.len()`.  In this case the slot is
correctly skipped (the `String` is zeroed at first resume and never set, so there is
nothing to free).  A real leak would require the block to have been entered, the
`String` set, then the generator yielded without the slot being in the snapshot.
That should not happen with the current `stack_base` pointing to the full frame.

**Tests to add** (`tests/expressions.rs`):

| Test name | File | Checks |
|-----------|------|--------|
| `coroutine_two_text_params` | expressions.rs | both param chars correct on each resume |
| `coroutine_text_local_after_yield` | expressions.rs | correct value on second resume |
| `coroutine_text_local_nested_block` | expressions.rs | no panic; run under Valgrind or `LOFT_LOG=ref_debug` for leak check |

**Effort:** Small (tests + targeted fixes if they fail)
**Target:** 0.8.3

---

**CO1.9** *(completed 0.8.3)* — Store iteration safety: generation guard promoted to always-on.

All `#[cfg(debug_assertions)]` gates removed from `Store.generation` field, struct
constructors (`new`, `open`, `clone_locked`, `clone_locked_for_worker`), and increment
sites (`claim`, `resize`, `delete`) in `src/store.rs`.  `CoroutineFrame.saved_store_generations`
field and the yield snapshot in `coroutine_yield` also ungated.  `debug_assert!` in
`coroutine_next` replaced with `assert!` so the guard panics in release builds too.
Test: `coroutine_stale_store_guard_all_builds` (no `#[cfg]` gate).

---

## I — Interfaces

### I1–I10 — Structural interfaces and bounded generics

**Motivation:** loft's single-`<T>` generics are opaque — no method calls,
operators, or comparisons are allowed on a generic `T`. Every generic algorithm
that needs ordering or addition must be reimplemented per type or written in
native Rust. Structural interfaces fix this by adding compile-time constraints
on `T`, enabling bounded generics (`<T: Ordered>`) without vtables or runtime cost.

Full design: [INTERFACES.md](INTERFACES.md).

**Design principles:**
- **Implicit satisfaction (structural):** a type satisfies an interface by having
  the required methods — no explicit `impl` declaration needed, matching loft's
  existing dispatch model.
- **Static dispatch only:** interfaces are generic constraints, not types.
  `x: Ordered` as a variable type is a compile error; there are no vtables.
- **`Self` keyword:** refers to the concrete satisfying type inside interface bodies.
- **Single bound per type parameter:** consistent with the existing single `<T>`.

**Standard library interfaces** (declared in `default/01_code.loft`):

```loft
pub interface Ordered   { fn OpLt(self: Self, other: Self) -> boolean
                          fn OpGt(self: Self, other: Self) -> boolean }
pub interface Equatable { fn OpEq(self: Self, other: Self) -> boolean
                          fn OpNe(self: Self, other: Self) -> boolean }
pub interface Addable   { fn OpAdd(self: Self, other: Self) -> Self }
pub interface Printable { fn to_text(self: Self) -> text }
```

**Example:**

```loft
interface Ordered {
    fn OpLt(self: Self, other: Self) -> boolean
}

fn max_of<T: Ordered>(v: vector<T>) -> T {
    result = v[0];
    for item in v { if result < item { result = item; } }
    result
}

struct Score { value: integer }
fn OpLt(self: Score, other: Score) -> boolean { self.value < other.value }

// Score satisfies Ordered automatically — no explicit declaration needed.
best = max_of([Score{value: 3}, Score{value: 7}, Score{value: 1}]);
```

**Steps:**

| ID  | Title | E | Source |
|-----|-------|---|--------|

**Dependency order:** I1 → I3 → I4 → I6 → I7 → I8 → I9.
I2 is parallel with I1. I5 depends on I3. I10 depends on I6.

**Native codegen impact:** none. Interfaces produce no bytecode and no Rust output.
Specialised copies of bounded generic functions are identical to ordinary concrete
functions from the codegen perspective.

**Target:** 0.8.3

---

## A — Architecture

---

### A2  Logger: hot-reload, run-mode helpers, release + debug flags
**Sources:** [LOGGER.md](LOGGER.md) § Remaining Work
**Description:** Four independent improvements to the logging system.  The core framework
(production mode, source-location injection, log file rotation, rate limiting) was shipped
in 0.8.0.  These are the remaining pieces.
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
**Target:** 0.9.0

---


### A8  Slicing & comprehension on `sorted` / `index`
**Sources:** [SORTED_SLICE.md](plans/38-sorted-slice/README.md)
**Description:** Extend `sorted<T>` and `index<T>` with key-range slicing, open-ended
bounds, partial-key match iteration, and vector comprehensions over key ranges.

**Features:**
- `col[lo..]`, `col[..hi]`, `col[..]` — open-ended range iterators (A8.1)
- `sorted[lo..hi]` — range slicing on sorted (A8.2; index already works)
- `col[k1]` on multi-key index — partial-key match iterator (A8.3)
- `[for v in col[lo..hi] { v.f }]` — comprehensions on key ranges (A8.4)
- `rev(col[lo..hi])` — reverse range iteration (A8.5)
- `match col[key] { null → ..., elm → ... }` — documented + tested (A8.6)

**Fix path:** See [SORTED_SLICE.md](plans/38-sorted-slice/README.md) — 6-step plan, all work in
`src/parser/fields.rs` and `src/codegen_runtime.rs`. No new opcodes.

**Effort:** M
**Target:** 0.8.3

---

### A4  Spatial index operations (full implementation)
**Sources:** PROBLEMS #22
**Description:** `spatial<T>` collection type: insert, lookup, and iteration operations
are not implemented.  The pre-gate (compile error) was added 2026-03-15.
**Fix path:**

**Phase 1 — Insert and exact lookup** (`src/database/`, `src/fill.rs`):
Implement `spatial.insert(elem)` and `spatial[key]` for point queries.  Remove the
compile-error pre-gate for these two operations only; all other `spatial` ops remain gated.
*Tests:* insert 3 points, retrieve each by exact key; null returned for missing key.

**Phase 2 — Bounding-box range query** (`src/database/`, `src/parser/collections.rs`):
Implement `for e in spatial[x1..x2, y1..y2]` returning all elements within a bounding box.
*Tests:* 10 points; query a sub-region; verify count and identity of results.

**Phase 3 — Removal** (`src/database/`):
Implement `spatial[key] = null` and `remove` inside an active iterator.
*Tests:* insert 5, remove 2, verify 3 remain and removed points are never returned.

**Phase 4 — Full iteration** (`src/database/`, `src/state/io.rs`):
Implement `for e in spatial` visiting all elements; compatible with the existing iterator
protocol (sorted/index/vector).  Remove the remaining pre-gate.
*Tests:* insert N points, iterate all, count matches N; reverse iteration produces correct order.

**Effort:** High (new index type in database.rs and vector.rs)
**Target:** 1.1+

---

### A5  Closure capture for lambda expressions
**Sources:** Depends on P1
**Description:** P1 defines anonymous functions without variable capture.  Full closures
require the compiler to identify captured variables, allocate a closure record, and pass
it as a hidden argument to the lambda body.  This is a significant IR and bytecode change.
**Fix path:**

**Phase 1 — Capture analysis** *(completed 0.8.3)*:
Parser detects variables from enclosing scopes referenced inside lambdas.  Emits a clear
error ("lambda captures variable 'name' — closure capture is not yet supported") and
creates a placeholder variable so parsing continues without cascading errors.  Capture
context saved/restored in both parse_lambda and parse_lambda_short.

**Phase 2 — Closure record layout** *(completed 0.8.3)*:
For each capturing lambda, the parser synthesizes `__closure_N` with fields matching
the captured variables.  The record def_nr is stored on Definition.closure_record.
Diagnostic emitted with field count/names/types for test verification.

**Phase 3 — Capture at call site** *(completed 0.8.3)*:
Capture diagnostic updated from generic "not yet supported" to specific "closure body
reads not yet implemented (A5.4)".  Closure record struct (A5.2) is still synthesized.
Actual closure record allocation IR and codegen deferred to A5.4.

**Phase 4 — Closure body reads** *(completed 0.8.3)*:
Hidden `__closure` parameter added on second pass.  Captured variable reads redirect
to `get_field` on the closure record.  Read-only captures work; mutable captures
(`count += x`) pending — codegen panics on self-reference for write targets.

**Phase 5 — Lifetime and cleanup** *(completed 0.8.3)*:
Closure record work variable (Type::Reference with empty deps) is already freed by
the existing OpFreeRef scope-exit logic in get_free_vars.  No new code needed.
Per-field text/reference cleanup inside the record is pending — only matters when
text captures become testable.

**Phase 6 — Mutable capture + text capture** (C1 remaining, tracked as A5.6):
Two remaining restrictions after A5.1–A5.5:

**A5.6a — Mutable capture** *(completed 0.8.3)*:
`capture_detected` passes without source changes.  The mutable-capture path
(`count += x`) routes through `call_to_set_op` → `OpSetInt`, which never hits the
`generate_set` self-reference guard.  The earlier plan for a `SetClosureField` IR
variant was not needed.  Test: `tests/parse_errors.rs::capture_detected`.

**A5.6b.1 — Text capture: garbage DbRef in `CallRef` stack frame** (✓ implemented in `safe` branch):
Text-capturing, text-returning lambdas (e.g. `fn(name: text) -> text { "{prefix} {name}" }`)
produce a garbage `__closure` DbRef at runtime, causing panics such as "Unknown record
49745" or "Store write out of bounds".  Integer-only captures work correctly.

**Root cause — `text_return()` adds captured text variables as spurious work-buffer attributes:**

When the lambda body `"{prefix} {name}"` is compiled, the format-string processor calls
`text_return(ls)` (control.rs:1550) where `ls` contains the text variables referenced in
the format string — including the captured variable `prefix`.

`text_return` iterates over `ls` and for each text variable that is NOT already an
attribute of the lambda, it adds it as a `RefVar(Text)` attribute (a hidden work-buffer
argument) and calls `self.vars.become_argument(v)`.  The guard that skips already-registered
attributes (line 1557: `attr_names.get(n)`) does NOT catch captured variables — at the point
`text_return` runs, `prefix` is not yet registered as an attribute (the hidden `__closure`
parameter is added later in `parse_lambda`).

Result: `prefix` is added as a `RefVar(Text)` attribute of the lambda, giving the lambda
an **extra 12-byte argument slot** that the caller knows nothing about.

**Broken argument layout (with the bug):**

The lambda’s `def_code` processes attributes in order:
1. `name: text` → slot 0, 16 bytes (`size_of::<&str>()`)
2. `prefix: RefVar(Text)` → slot 16, 12 bytes ← spurious, added by `text_return`
3. `__closure: Reference` → slot 28, 12 bytes

Total argument area = 40 bytes; `+4` for return addr → TOS at 44.
Reading `__closure`: `var_pos = 44 - 28 = 16`.  At runtime `stack_pos - 16 = args_base + 16`.

But the caller only pushes 28 bytes (`name` 16 + `__closure` DbRef 12):
- `args_base + 0..16`: `name` ✓
- `args_base + 16..28`: closure DbRef ← callee reads this as `prefix` slot
- `args_base + 28..40`: **nothing** ← callee reads this as `__closure` slot → garbage

**Fix (concrete — `src/parser/control.rs`, `text_return`):**

Add a captured-variable guard immediately after the existing `attr_names` check:

```rust
pub(crate) fn text_return(&mut self, ls: &[u16]) {
    if let Type::Text(cur) = &self.data.definitions[self.context as usize].returned {
        let mut dep = cur.clone();
        for v in ls {
            let n = self.vars.name(*v);
            let tp = self.vars.tp(*v);
            // skip related variables that are already attributes
            if let Some(a) = self.data.def(self.context).attr_names.get(n) {
                if !dep.contains(&(*a as u16)) {
                    dep.push(*a as u16);
                }
                continue;
            }
            // A5.6b.1: skip captured variables — they are read from the closure
            // record at runtime, not passed as hidden work-buffer arguments.
            // Adding them as RefVar(Text) attributes shifts __closure to the wrong
            // argument slot, giving the lambda a garbage DbRef.
            if self.captured_names.iter().any(|(name, _)| name == n) {
                continue;
            }
            if matches!(tp, Type::Text(_)) {
                // ... rest unchanged
```

After this fix, the lambda’s argument layout becomes:
1. `name: text` → slot 0, 16 bytes
2. `__closure: Reference` → slot 16, 12 bytes
Total = 28 bytes, matching what the caller pushes. ✓

**Why `name` is still handled correctly:**

`name` IS already an attribute (it’s the declared parameter), so the `attr_names.get(n)`
check catches it and just adds its attribute index to `dep`.  The format string’s text
dependency tracking for `name` still works — only the spurious insertion of `prefix` as
a work-buffer attribute is suppressed.

**Scope of fix:** Only affects lambdas that (a) return text AND (b) capture text variables
from an outer scope.  No other code path is changed.

**Test scope note:** The existing `closure_capture_text` test (`make_greeter("Hello")("world")`)
crosses function scope — the closure is returned from `make_greeter` and called from
outside.  The `last_closure_alloc` block references variable slots in `make_greeter`'s
frame; calling the returned fn-ref from a different scope would access those stale slots.
This pattern requires A5.6 (0.8.3) — returning a closure alongside its DbRef.

After this fix, add a **same-scope** test that exercises A5.6b.1 directly:
```
prefix = "Hello";
f = fn(name: text) -> text { "{prefix} {name}" };
f("world")  // expected: "Hello world"
```
Same-scope calls use `last_closure_alloc` correctly (consumed at the call site within
the same definition) and do not require the returning-closure architecture.  The
existing `closure_capture_text` test should remain `#[ignore]` until A5.6 (0.8.3).

**A5.6b.2 — `generate_call_ref`: text work buffers not pre-allocated** (✓ implemented):
Text-returning lambdas called via `CallRef` now correctly push the hidden `__work_N`
work-buffer DbRef argument that the callee expects.

**Fix** (`src/parser/control.rs`, `try_fn_ref_call` and zero-param closure path):
- Both passes call `work_text()` for each dep in the return type’s deps list.
- `work_text()` adds each variable to `work_texts`; `parse_code` (expressions.rs:79)
  inserts `v_set(wv, Text(""))` so the Zone 2 slot allocator fires.
- In pass 2, a `v_block([OpCreateStack(Var(wv))], Type::Reference(...))` is injected
  between the visible args and the closure arg — producing the required 12-byte DbRef.
- `generate_call_ref` simplified to a single `for arg in args { generate(...) }` loop;
  the blocks produce the correct sizes automatically.

**Verified:** `closure_capture_text_return` passes; all other closure tests unaffected.

**A5.6c — Mutable capture write-back: void-return lambdas** (✓ implemented in `safe` branch):
A void-return capturing lambda (`fn(x: integer) { count += x; }`) updates the
`count` field inside the closure record, but the outer `count` variable (in the
caller’s stack frame) is never updated.  After `f(10); f(32)`, the outer `count`
remains 0.

The lambda’s IR correctly modifies the closure record field (A5.6a is done — the
`capture_detected` test proves mutable field writes work inside the lambda body).
The missing step is the write-back from closure record to outer variable after each
`CallRef` returns.

**Fix path (concrete — parser `control.rs`, call site generation):**

At the call site where `Value::CallRef(v_nr, args)` is built (control.rs:2000),
after constructing `converted`, emit write-back IR for each mutable captured variable:

```rust
// A5.6c: after CallRef to a closure, write captured mutable fields back to
// the outer variables so the caller sees the updated values.
if let Some(&closure_w) = self.closure_vars.get(&v_nr) {
    // closure_vars maps fn-ref var → closure work var (the __clos DbRef in scope).
    let closure_rec = self.data.def(d_nr).closure_record;
    if closure_rec != u32::MAX {
        for aid in 0..self.data.attributes(closure_rec) {
            let cap_name = self.data.attr_name(closure_rec, aid);
            let outer_v = self.vars.var(&cap_name);
            if outer_v != u16::MAX {
                // Emit: outer_var = get_field(__clos, aid)
                write_back_ops.push(self.get_field(closure_rec, aid, 0,
                    Value::Var(closure_w)));
                write_back_ops.push(Value::Set(outer_v, /* get_field result */));
            }
        }
    }
}
```

The exact IR construction follows the existing `set_field_no_check` / `get_field`
helpers.  The write-back IR is emitted as statements immediately after the
`Value::CallRef(...)` expression in the enclosing block.

**Prerequisite:** `closure_vars` must be populated for the fn-ref variable `v_nr`.
Currently `closure_vars.insert` fires only when `last_closure_work_var != u16::MAX`,
but `last_closure_work_var` is never set.  Fix: in `emit_lambda_code` (vectors.rs),
after creating `w` (the `__clos` work var), set `self.last_closure_work_var = w`.
Then in `parse_assign` (expressions.rs:710–712), `closure_vars.insert(var_nr, w)`
fires correctly.

**Test:** Remove `#[ignore]` from `tests/issues.rs::p1_1_lambda_void_body` and
update the ignore reason from the old "A5, 1.1+" text to "A5.6c" once the fix is
implemented.

**Effort:** A5.6b.1 Medium · A5.6b.2 Small · A5.6c Medium
**Target:** 0.8.3 (A5.6b.1, A5.6b.2, A5.6c, A5.6d, A5.6e, A5.6f completed; full cross-scope A5.6 also 0.8.3)

---

**A5.6 — Full closure semantics: 16-byte fn-ref + chained-call parser** *(completed 0.8.3)*:
After A5.6b.1, A5.6b.2, and A5.6c are implemented, the last open item for
`closure_capture_text` is the **cross-scope** pattern: a capturing lambda returned
from a function and then called from outside.  Two distinct problems remain:

---

#### The opcode problem: `Type::Function` is 4 bytes — no room for closure DbRef

`size(Type::Function, _)` returns 4 (same arm as `Type::Integer` in
`src/variables/mod.rs:995`).  `fn_call_ref` in `state/mod.rs:221` reads exactly 4
bytes: `*get_var::<i32>(fn_var)` = the d_nr.

A closure DbRef is 12 bytes (store_nr + rec + pos — same layout as every other
`DbRef`).  When `make_greeter` returns the inner lambda as its return value, only
the 4-byte d_nr lands on the caller's stack; the 12-byte DbRef for the closure
record has nowhere to go and is lost.  The closure record itself stays alive in the
store (it was heap-allocated via `OpDatabase`), but no pointer to it survives the
return — so the lambda body's `__closure` parameter can never be populated.

**Fix — 16-byte fn-ref slot:**

```
offset 0..4:  d_nr (i32)        — function definition index
offset 4..8:  store_nr (i32) ─┐
offset 8..12: rec (i32)        ├─ closure DbRef (12 bytes; all-zero = no closure)
offset 12..16: pos (i32)      ─┘
```

`size(Type::Function, _)` → 16 (move `Type::Function` out of the `4`-byte arm in
`src/variables/mod.rs:995`; add a new arm `Type::Function(_, _) => 4 + size_of::<DbRef>() as u16`).

**Emitting the fn-ref value (vectors.rs `emit_lambda_code`):**

Non-capturing lambdas: `*code = Value::Int(d_nr as i32)` unchanged — `OpPutInt`
writes d_nr to bytes 0..4; bytes 4..16 stay zero (zeroed by `OpReserveFrame`).

Capturing lambdas: emit a `v_block` that:
1. Runs the existing `alloc_steps` to allocate and fill the closure record into work
   var `w` (type `Type::Reference`).
2. Emits `v_set(fn_ref_var, Value::Int(d_nr as i32))` — writes d_nr to bytes 0..4
   of the new 16-byte work var `fn_ref_var` (type `Type::Function`).
3. Emits `cl("OpStoreClosure", [Var(fn_ref_var), Var(w)])` — a new opcode that
   copies the 12-byte DbRef from `w`'s stack slot into `fn_ref_var`'s bytes 4..16.
4. Yields `Value::Var(fn_ref_var)`.

Then **drop** `self.last_closure_alloc` — the closure is now embedded in the fn-ref
value and no longer needs to be injected separately at call sites.

**New opcode: `OpStoreClosure(fn_ref_var: u16, closure_var: u16)`** (fill.rs):
Reads the absolute stack position of `fn_ref_var` and `closure_var`; copies 12 bytes
from `closure_var`'s slot to `fn_ref_var`'s slot at byte offset 4.  No stack push/pop.

**Calling through the 16-byte fn-ref (state/mod.rs `fn_call_ref`):**

```rust
pub fn fn_call_ref(&mut self, fn_var: u16, arg_size: u16) {
    let d_nr = *self.get_var::<i32>(fn_var) as usize;
    // Read closure DbRef from bytes 4..16 of the 16-byte fn-ref slot.
    // The slot start is at (stack_pos - fn_var); byte 4 is one i32 further.
    let store_nr = *self.get_var::<i32>(fn_var - 4);   // fn_var_abs + 4
    let has_closure = store_nr != -1;  // -1 is the null sentinel for store_nr
    let total = arg_size + if has_closure { size_of::<DbRef>() as u16 } else { 0 };
    if has_closure {
        let rec = *self.get_var::<i32>(fn_var - 8);
        let pos = *self.get_var::<i32>(fn_var - 12);
        // Push DbRef (12 bytes) onto the stack as __closure argument
        self.push_stack(store_nr);
        self.push_stack(rec);
        self.push_stack(pos);
    }
    let code_pos = self.fn_positions[d_nr] as i32;
    self.fn_call(d_nr as u32, total, code_pos);
}
```

Note: the fn-ref variable's absolute position is `stack_pos - fn_var`.  Because the
stack grows upward, `fn_var_abs + 4` is referenced as `stack_pos - (fn_var - 4)`.
Verify the offset arithmetic matches `get_var`'s addressing in the implementation.

**Call-site codegen (parser/control.rs `try_fn_ref_call`, zero-param path):**

Remove the `last_closure_alloc.take()` and `closure_vars.get(&v_nr)` injection.
The closure is now pushed by `fn_call_ref` at runtime from the embedded DbRef —
no parser-level injection needed.  `generate_call_ref` is unchanged (already
simplified by A5.6b.2): all args in `converted` are visible params and work bufs.

**`generate_var` for `Type::Function` (codegen.rs line 1210):**

Change from `OpVarInt` (4 bytes) to a new `OpVarFnRef` (16 bytes).  This is the
read side of the 16-byte push: push all 16 bytes of the fn-ref slot onto the stack
so fn-ref values can be passed, returned, and assigned.

`OpVarFnRef` implementation (fill.rs): read `pos: u16` from bytecode; push 16 bytes
starting at `stack_pos - pos` onto the stack (similar to `OpVarRef` which pushes 12
bytes, but 4 bytes larger).

**`OpPutInt` for `Type::Function` (codegen.rs lines 1521, 1210):**

Assignment `v_set(fn_ref_var, Value::Int(d_nr))` still uses `OpPutInt` — it writes
4 bytes to the variable's slot at offset 0 (the d_nr).  Bytes 4..16 are untouched
(already zeroed by `OpReserveFrame` or set by a preceding `OpStoreClosure`).
So `OpPutInt` at call sites for fn-ref assignment is **correct as-is** when the
RHS is `Value::Int(d_nr)`.

For the case where a fn-ref is copied variable-to-variable (`f = g` where both are
`Type::Function`), use `OpVarFnRef` to push 16 bytes then `OpPutFnRef` (new) to
store them — OR reuse `OpPutRef`-style logic for 16 bytes.

---

#### The parser problem: `expr(args)` chained calls not handled

`parse_part` (operators.rs:277) loops on `.` and `[` only.  After
`make_greeter("Hello")` returns `Type::Function`, the `("world")` token is not
consumed as a chained call — it is parsed as a separate parenthesised expression.

**Fix (operators.rs `parse_part`):**

Extend the loop to handle `(` when `t` is `Type::Function`:

```rust
while self.lexer.peek_token(".")
    || self.lexer.peek_token("[")
    || (self.lexer.peek_token("(") && matches!(t, Type::Function(_, _)))
{
    if self.lexer.has_token("(") {
        if let Type::Function(param_types, ret_type) = t.clone() {
            // Store fn-ref expression in a work var so CallRef can name it.
            let fn_work = self.create_unique("__fnref_tmp", &t);
            if !self.first_pass {
                let orig = std::mem::replace(code, Value::Var(fn_work));
                // emit: fn_work = <fn_ref_expression>
                // (parse_code will insert the assignment via inline-ref logic)
                // Actually: wrap in a block: { fn_work = orig; fn_work }
                // ... see implementation note below
            }
            t = self.call_fn_work_var(fn_work, param_types, *ret_type);
        }
    } else { /* existing . and [ handlers */ }
}
```

`call_fn_work_var(work_var, param_types, ret_type)`: parse argument list, emit
`Value::CallRef(work_var, args)`, return `ret_type`.  Because the closure DbRef is
embedded in the 16-byte fn-ref slot of `work_var`, `fn_call_ref` pushes it at
runtime — no explicit closure injection needed.

**Implementation note:** Storing `orig` into `fn_work` before the call requires
either:
(a) Wrapping in a `v_block([v_set(fn_work, orig), Value::CallRef(fn_work, args)], ret_type)`, or
(b) Using the inline-ref temp pattern from `parse_part`'s existing chained-ref logic
    (lines 342–361) — mark `fn_work` as an inline-ref temp; `parse_code` inserts the
    null-init.

Option (a) is simpler for the first implementation.

---

#### Remaining deferred sub-items (post-0.8.3)

After the 16-byte fn-ref lands, these edge cases remain deferred:

1. **Lambda re-definition:** if `f = fn(x) { ... }` is followed by `f = fn(x) { ... }`,
   the old closure record (bytes 4..16 of the old fn-ref) must be freed before overwriting.
   `get_free_vars` must emit `OpFreeRef` reading from the fn-ref slot before the
   `OpPutInt`/`OpStoreClosure` of the new lambda.

2. **Lambdas in collections / struct fields:** `closure_vars` is irrelevant with 16-byte
   fn-refs; the closure DbRef travels with the fn-ref value.  But for collections,
   `OpVarFnRef` / store operations need to work correctly for the 16-byte size.

3. **Concurrent sharing:** two parallel workers calling the same closure simultaneously
   share the closure record.  Requires per-call copy or locking — deferred to the
   parallel safety audit.

---

**Implementation steps (independently testable):**

**A5.6-1 — Widen `Type::Function` to 16 bytes**

- `src/variables/mod.rs`: change the `Type::Function(_, _)` arm in `size()` from `4` to
  `4 + size_of::<DbRef>() as u16` (= 16).
- `src/state/codegen.rs` (`generate_var`): change the `Type::Function` arm from emitting
  `OpVarInt` (4 bytes) to a new `OpVarFnRef` (16 bytes).
- `src/fill.rs`: add `op_var_fn_ref` — reads `pos: u16` from bytecode; pushes 16 bytes
  starting at `stack_pos - pos` onto the stack (same as `op_var_ref` but 4 bytes larger).

**Pass:** all existing non-capturing lambda tests pass; fn-ref variable occupies 16 bytes.

---

**A5.6-2 — `OpStoreClosure` + embed closure DbRef in fn-ref**

- `src/fill.rs`: add `op_store_closure` — reads `fn_ref_pos: u16` and `closure_pos: u16`
  from bytecode; copies 12 bytes from `stack_pos - closure_pos` to
  `(stack_pos - fn_ref_pos) + 4`. No stack push/pop.
- `src/parser/vectors.rs` (`emit_lambda_code`): for capturing lambdas, after the existing
  `alloc_steps` (which produce the closure record in work var `w`), emit:
  1. `v_set(fn_ref_var, Value::Int(d_nr as i32))` — writes d_nr into bytes 0..4.
  2. `cl("OpStoreClosure", &[Value::Var(fn_ref_var), Value::Var(w)])` — embeds the
     12-byte DbRef from `w` into fn-ref bytes 4..16.
  Store result in `fn_ref_var` (a new Zone-1 work variable of type `Type::Function`).
  **Drop** `self.last_closure_alloc` — the closure is now embedded in the fn-ref value and
  no longer injected at call sites.

**Pass:** a capturing lambda assigned to a local variable carries its closure DbRef in the
fn-ref slot; `LOFT_LOG=ref_debug` shows the DbRef bytes 4..16 non-zero.

---

**A5.6-3 — `fn_call_ref` reads closure from fn-ref bytes 4..16**

- `src/state/mod.rs` (`fn_call_ref`): after reading `d_nr` from `*get_var::<i32>(fn_var)`,
  read `store_nr` from `*get_var::<i32>(fn_var - 4)`.  If `store_nr != -1` (non-null),
  read `rec` and `pos` and push the 12-byte DbRef onto the stack as the `__closure`
  argument.  Adjust `total_arg_size` accordingly.
  ```rust
  let store_nr = *self.get_var::<i32>(fn_var - 4);
  let has_closure = store_nr != -1;
  if has_closure {
      let rec = *self.get_var::<i32>(fn_var - 8);
      let pos = *self.get_var::<i32>(fn_var - 12);
      self.push_stack(store_nr);
      self.push_stack(rec);
      self.push_stack(pos);
  }
  ```
  (Offset arithmetic: fn-ref occupies bytes `[fn_var_abs .. fn_var_abs+16]`; d_nr is at
  offset 0, store_nr at +4, rec at +8, pos at +12.  `get_var::<i32>(fn_var)` reads from
  `stack_pos - fn_var` = `fn_var_abs`; `fn_var - 4` reads `fn_var_abs + 4`, etc.)
- `src/parser/control.rs` (`try_fn_ref_call`, both paths): remove
  `last_closure_alloc.take()` injection and `closure_vars` lookup — the closure is now
  pushed by `fn_call_ref` at runtime from the embedded DbRef.

**Pass:** `closure_capture_text_return` and `closure_capture_text_integer_return` pass
without the closure being injected at the call site.

---

**A5.6-4 — `parse_part`: chained `(...)` call on `Type::Function`**

- `src/parser/operators.rs` (`parse_part`): extend the postfix loop:
  ```rust
  while self.lexer.peek_token(".")
      || self.lexer.peek_token("[")
      || (self.lexer.peek_token("(") && matches!(t, Type::Function(_, _)))
  {
      if self.lexer.has_token("(") {
          if let Type::Function(param_types, ret_type) = t.clone() {
              // Store fn-ref in work var so CallRef can name it.
              let fn_work = self.create_unique("__fnref_tmp", &t);
              if !self.first_pass {
                  let orig = std::mem::replace(code, Value::Var(fn_work));
                  *code = Value::Block(Box::new(Block {
                      ops: vec![Value::Set(fn_work, Box::new(orig))],
                      result: Box::new(Value::Var(fn_work)),
                      ..Default::default()
                  }));
              }
              t = self.call_fn_work_var(fn_work, param_types, *ret_type);
          }
      } else { /* existing . and [ handlers */ }
  }
  ```
  `call_fn_work_var`: parse argument list inside `(...)`, emit
  `Value::CallRef(fn_work, args)`, return `ret_type`.

**Pass:** `make_greeter("Hello")("world")` parses and produces "Hello world".

---

**A5.6-5 — Un-ignore `closure_capture_text`; full test pass**

- `tests/expressions.rs`: remove `#[ignore]` from `closure_capture_text`.
- `tests/wrap.rs` (WASM_SKIP): keep `19-threading.loft` skipped (that is W1.18, not A5.6).

**Pass:** `cargo test --test expressions closure_capture_text` succeeds; full `make test`
green.

---

**Files changed:**

| File | Change |
|------|--------|
| `src/variables/mod.rs` | `size(Type::Function)` → 16 |
| `src/fill.rs` | Add `op_store_closure`, `op_var_fn_ref` |
| `src/state/mod.rs` | `fn_call_ref`: read closure from bytes 4..16, push if present |
| `src/state/codegen.rs` | `generate_var`: `OpVarFnRef` for `Type::Function` |
| `src/parser/vectors.rs` | `emit_lambda_code`: emit `OpStoreClosure`; drop `last_closure_alloc` |
| `src/parser/control.rs` | Remove closure injection from `try_fn_ref_call` (both paths) |
| `src/parser/operators.rs` | `parse_part`: handle chained `(...)` on `Type::Function` |
| `tests/expressions.rs` | Remove `#[ignore]` from `closure_capture_text` |

**Guards and debugging:**
- `fn_call_ref`: `debug_assert!(d_nr < self.fn_positions.len())` before indexing.
- `OpStoreClosure`: `debug_assert!` that the fn_ref_var slot has 16 bytes allocated.
- Strip internal text deps from the public fn-type: in `emit_lambda_code` (vectors.rs:667),
  replace `Text(deps)` with `Text(vec![])` in the return type of the constructed
  `Type::Function`.  Internal dependency tracking is for the lambda body, not the interface.
- Add `LOFT_LOG=closure` mode that prints fn-ref slot contents (d_nr + DbRef) at call sites
  in `fn_call_ref` — catches misaligned reads immediately.

**Effort:** High (8 files, 2 new opcodes, 5 independently testable steps)
**Depends on:** A5.6b.1 ✓, A5.6b.2 ✓, A5.6c ✓
**Target:** 0.8.3

---


### L9  Format specifier / type mismatch — escalate to compile error
**Status: completed**
Changed `Level::Warning` → `Level::Error` in `append_data()` for radix specifiers on
text/boolean and zero-padding on text.  Tests updated in `38-parse-warnings.loft`.
CAVEATS.md C14 closed.

---

### L10  `while` loop syntax sugar
**Status: completed**
Added `while` keyword to the lexer and `parse_while()` in `expressions.rs`.
Desugars to `v_loop([if !cond { break }, body])`.  Tests in `46-caveats.loft`.
CAVEATS.md C11 closed.

---

### L11  Digit-group separators (`_`) in number literals + thousands-position lint
**Status: completed** — `src/lexer.rs::get_number` accepts `_` between digits (stripped before
parse) across all bases + float/exponent parts; misplaced `_` (trailing / doubled / next to a
prefix·`.`·`e`) is one `Level::Error`; decimal groups that aren't thousands (leftmost 1-3, rest
exactly 3) emit a `Level::Warning` but still accept. Tests: `01-integers.loft` (accept),
`38-parse-warnings.loft` (warn), `102-expected-errors.loft` (errors). Both backends (shared lexer).
**Sources:** user request 2026-06-25 (verified absent — the lexer rejected `1_000_000`).
**Description:** Let `_` separate digit groups in numeric literals so big numbers read
easily — `1_000_000`, `0xFF_FF`, `1_000.000_5` — and **warn** (accept, do not reject) when
the separators are *not* on standard thousands boundaries (`1_00_000`, `10_0`). The warning
nudges toward conventional 3-digit grouping without forcing it.

Today `_` is rejected: `get_number()` (`src/lexer.rs:977`) accepts only digits / `b` / `o`
/ hex chars, and `_` is an identifier character (`src/lexer.rs:966`), so `1_000_000` lexes
as `1` followed by the identifier `_000_000`, and the parser reports "Expect token `;`".

**Design (XS–S):**
1. **Lexer (`get_number`)** — accept `_` *only between two digits*; strip it from the
   collected string before `parse`. Reject (Error) a leading `_` (`_1`, which is an
   identifier), a trailing `_` (`1_`), a doubled `__`, or a `_` adjacent to the radix
   prefix / `.` / `e` (`0x_F`, `1._5`, `1e_3`). The same function scans the integer,
   fraction, and exponent parts and every base (dec/hex/bin/oct), so one change covers all.
2. **Thousands-position lint** — while scanning the integer part, record the digit count
   between separators. If any *interior* group is not exactly 3 digits (the most-significant
   group may be 1–3), emit a `Level::Warning` ("digit separators not on thousands
   boundaries") and still accept the value. Decimal only at first — hex/bin group by 4/8,
   not 3, so either skip the lint there or add nibble/byte grouping as a follow-up.
3. **Tests** (`tests/` parse-warnings suite): `1_000_000` parses to `1000000`; warn on
   `1_00_000` / `10_0`; error on `_1` / `1_` / `1__0` / `0x_F`. The lexer is shared, so
   interp and native agree by construction (no separate codegen).

**Files:** `src/lexer.rs` (`get_number` + the lint), a regression test, and a one-line note
in `LOFT.md` § number literals once shipped. No parser/codegen change — the value is already
a plain integer by the time it leaves the lexer.

---

### A12  Lazy work-variable initialization
**Status: deferred to 1.1+ — too complex and disruptive for stability; also blocked by Issues 68–70 (see PROBLEMS.md)**
**Sources:** Stack efficiency evaluation 2026-03-20
**Description:** Work text variables (`__work_N`) are currently initialized at function
start via `Set(wt, Text(""))` inserted at index 0 of the body block.  This forces
`first_def = 0` for every work text variable, making its live interval span the entire
function.  Two sequential, non-overlapping text operations each hold a 24-byte slot for
the full lifetime of the call frame.  The same applies to non-inline work ref variables
(`__ref_N`), which also get function-start null-inits.

Inline-ref temporaries already use lazy insertion (per A6.3a work): their null-init is
placed immediately before the statement that first assigns them, giving accurate intervals.
This item extends that approach to all work variables.

**Fix path:**

*Step 1 — Rename and generalize `inline_ref_set_in`* (`src/parser/expressions.rs`):

Rename `inline_ref_set_in` to `first_set_in` (or add it as a general helper).  No logic
changes — the function already recurses into all relevant `Value` variants and works
correctly for both text and ref work variables.

*Step 2 — Extend insertion loop in `parse_code` to work texts*:

Replace the eager-insert loop for work texts with a lazy-insert using `first_set_in`.
Non-inline work references remain eagerly inserted at position 0 (see blocker below).
Inline-ref variables continue to use the same lazy path as before.

```rust
// BEFORE: for wt in work_texts() { ls.insert(0, v_set(wt, Text(""))) }
// AFTER: find the first top-level statement containing a Set to wt, insert before it.
let mut insertions: Vec<(usize, u16, Value)> = Vec::new();
for wt in self.vars.work_texts() {
    let pos = ls.iter().position(|stmt| first_set_in(stmt, wt, 0)).unwrap_or(fallback);
    insertions.push((pos, wt, Value::Text(String::new())));
}
// work_references: still position 0 (blocker: Issue 68)
for r in self.vars.work_references() {
    if !is_argument && depend.is_empty() && !is_inline_ref {
        insertions.push((0, r, Value::Null));
    }
}
for r in self.vars.inline_ref_references() { ... lazy as before ... }
insertions.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
for (pos, r, init) in insertions { ls.insert(pos, v_set(r, init)); }
```

**Known blockers (found during 2026-03-20 implementation):**

- **Issue 68** — `first_set_in` does not descend into `Block`/`Loop` nodes.  Work
  references used only inside a nested block cannot be found; the fallback position lands
  *after* the block, giving `first_def > last_use`.  Fix: add `Block` and `Loop` arms to
  `first_set_in`.  Until then, non-inline work references stay at position 0.

- **Issue 69** — Extending `can_reuse` in `assign_slots` to `Type::Text` causes slot
  conflicts: two smaller variables can independently claim the first bytes of the same
  dead 24-byte text slot.  The `assign_slots_sequential_text_reuse` unit test passes in
  isolation (with explicit non-overlapping intervals) but the integration suite fails.
  Full text slot sharing also requires OpFreeText to be placed after each variable's last
  use (not at function end), otherwise sequential work texts still have overlapping live
  intervals.  Both issues must be resolved before `can_reuse` is extended.

- **Issue 70** — Adding `Type::Text` to the `pos < TOS` bump-to-TOS override in
  `generate_set` causes SIGSEGV in `append_fn`.  This override was added to handle
  "uninitialized memory if lazy init places a text var below current TOS", but that
  scenario only arises when text slots are reused (Issue 69), which is disabled.  The
  override must be reverted until text slot reuse is safe.

*Interval effect (partial):* `first_def` for work texts is now accurate.  Slot sharing
requires resolving Issues 69 and 70 and moving OpFreeText to after each variable's last
use.

**Tests:** `assign_slots_sequential_text_reuse` in `src/variables/` runs
unconditionally (Issue 69 fix landed).
**Effort:** Medium (three inter-related blockers; Issues 68–70)
**Target:** 0.8.2

---


### TR1  Stack trace introspection
**Sources:** STACKTRACE.md
**Description:** `stack_trace()` stdlib function returning `vector<StackFrame>`, where each frame exposes function name, source file, and line number. Full design in [STACKTRACE.md](STACKTRACE.md). Prerequisite for CO1 (coroutines use the frame vector for yield/resume).

- **TR1.1** — Shadow call-frame vector *(completed 0.8.3)*: CallFrame struct and call_stack on State; OpCall encodes d_nr and args_size; fn_call pushes, fn_return pops.
- **TR1.2** — Type declarations *(completed 0.8.3)*: ArgValue, ArgInfo, VarInfo, StackFrame in `default/04_stacktrace.loft`.
- **TR1.3** — Materialisation *(completed 0.8.3)*: `stack_trace()` native function builds `vector<StackFrame>` from snapshot. Tests blocked by Problem #85.
- **TR1.4** — Call-site line numbers *(completed 0.8.3)*: CallFrame stores source line directly; resolved in fn_call. Tests blocked by Problem #85.

**Effort:** Medium
**Completed:** 0.8.3 (phases 1–4; phases 5–6 deferred to 1.1+)

---


## S — Stability Hardening

Items found in a systematic stability audit (2026-03-20).  Each addresses a panic,
silent failure, or missing bound in the interpreter and database engine.  All target 0.8.2.

---

### S6  Fix remaining "recursive call sees stale attribute count" cases
**Sources:** @P271 (the surviving "Too few parameters" observation — seen once, not
reproducible).  The original PROBLEMS.md #84 (the merge-sort use-after-free
manifestation) was fixed in 0.8.2 and is no longer a separate row.
**Severity:** Medium — the merge-sort use-after-free (the primary manifestation) was fixed in 0.8.2.  Complex mutual-recursion patterns that trigger `ref_return` on a function after its recursive call sites were already compiled may still produce wrong attribute counts.
**Description:** `ref_return` adds work-ref attributes to a function's IR while the body is still being parsed.  When the function is recursive, call sites parsed before `ref_return` runs see the old (smaller) attribute count.  The merge-sort case was fixed by guarding `vector_needs_db` with `!is_argument` and injecting the return-ref in `parse_return`.  A general fix would scan the IR tree after the second parse pass and patch under-argument recursive calls via `add_defaults`.
**Fix path:** Post-parse IR scan and call-site patching in `parse_function`.
**Effort:** Medium
**Target:** 1.1+

---

### S19  Fix #85: struct-enum locals not freed in debug mode
**Sources:** PROBLEMS.md #85, CAVEATS.md C16
**Severity:** Low in production (no assertion), critical in debug builds (SIGABRT).
**Description:** `scopes.rs::free_vars()` emits `OpFreeRef` for plain struct local variables but not for struct-enum locals.  In debug builds, the store's allocation assert fires at scope exit because the record is still live.
**Fix path:**
1. In `get_free_vars` (or equivalent), add a branch for `Type::Named(_, _, _)` that is a struct-enum variant — emit `OpFreeRef` exactly as is done for plain structs.
2. Regression test: declare a local struct-enum variable inside a `for` or `if` body; verify no assertion fire in debug, value correct in release.
**Effort:** Small
**Target:** 0.9.0

---

### S20  Fix #91: init(expr) circular dependency silently accepted
**Sources:** PROBLEMS.md #91, CAVEATS.md C18
**Severity:** Medium — silent undefined behaviour at runtime when two store fields form a mutual initialisation cycle.
**Description:** The `init(expr)` attribute on struct fields is evaluated at record creation time.  If field A's init expr reads field B and field B's init expr reads field A, the interpreter reads uninitialised memory.  No cycle check is performed.
**Fix path:**
1. After all struct field defs are parsed, build a dependency graph: edge A→B if field A's init expr contains a read of field B.
2. DFS cycle detection over the graph; emit a compile error naming the cycle.
3. Test: two mutually-referencing `init(...)` fields produce a clear error; acyclic chains are unaffected.
**Effort:** Small
**Target:** 0.9.0

---

### S21  Fix #92: stack_trace() silent empty in parallel workers
**Sources:** PROBLEMS.md #92, CAVEATS.md C17
**Severity:** Medium — debugging parallel code is significantly harder without stack traces.
**Description:** `stack_trace()` reads `state.data_ptr` to walk the call stack.  In parallel workers spawned by `par(...)`, `execute_at` (and `execute_at_ref`) entry points do not set `data_ptr` before dispatch, so the pointer is null and `stack_trace()` returns an empty vec.
**Fix path:**
1. In `execute_at` and `execute_at_ref` in `src/state/mod.rs`, set `self.data_ptr = data as *const Data;` (or equivalent) immediately before the dispatch call, mirroring what the single-threaded `execute` path does.
2. Regression test: call `stack_trace()` inside a `par(...)` worker body; assert the returned vec is non-empty and contains the worker function name.
**Effort:** Small
**Target:** 0.9.0

---

### S22  Fix parallel worker auto-lock in release builds
**Sources:** SAFE.md § P1-R1, CAVEATS.md C22
**Severity:** Medium — release builds silently return wrong results when a worker writes to a `const` argument.
**Description:** The auto-lock insertion (`n_set_store_lock`) for `const` worker arguments is guarded by `#[cfg(debug_assertions)]` in `parser/expressions.rs`.  Release builds never lock the input stores, so a buggy worker that accidentally mutates a `const` argument silently discards the write into a 256-byte dummy buffer and continues with stale data.
**Fix path:**
1. Remove the `#[cfg(debug_assertions)]` guards from the two auto-lock insertion sites in `parse_code` and `expression` that emit `n_set_store_lock` for `const` parameters and local const variables.
2. In `addr_mut` (`store.rs`), change the release-build dummy-buffer path to `panic!("write to locked store")` — no legitimate code path should hit it once auto-lock is unconditional.
3. Add an integration test that runs a `par()` loop whose worker attempts to push to its `const` input in release mode; assert the panic fires with a clear message.
**Effort:** Small
**Target:** 0.8.3

---

### S23  Compiler + runtime: reject `yield` inside `par()` body
**Sources:** SAFE.md § P2-R6, CAVEATS.md C25, COROUTINE.md § SC-CO-4
**Severity:** Medium — `yield` or generator calls inside `par(...)` produce out-of-bounds panics or silent wrong results depending on frame-index collision.
**Description:** No compiler check prevents `yield` or calls to `iterator<T>`-returning functions inside `par(...)` bodies.  Worker `State` instances hold only a null-sentinel `coroutines` table; a DbRef produced by the main thread indexes into it incorrectly.
**Fix path:**
1. In `src/parser/collections.rs` (parallel-for desugaring) and wherever `par(...)` body parsing begins, add an `inside_par_body: bool` flag to the parser context.
2. In `parse_yield` and any site that resolves a function call returning `iterator<T>`, emit a compile error when `inside_par_body` is true.
3. In `coroutine_next` (`state/mod.rs`), add a bounds check: `if idx >= self.coroutines.len() { panic!("iterator<T> DbRef out of range in worker") }`.  This defence-in-depth guard catches the case where the compiler check is missing.
4. Test: a loft program that calls a generator inside `par(...)` produces a compile error; one that bypasses the check triggers the runtime guard in debug.
**Effort:** Small
**Target:** 0.8.3

---

### S24  Compiler + runtime: reject `e#remove` on generator iterator
**Sources:** SAFE.md § P2-R9, CAVEATS.md C26, COROUTINE.md § SC-CO-11
**Severity:** Medium — release builds silently corrupt a real store record; debug builds panic with an uninformative out-of-bounds message.
**Description:** `e#remove` on a generator-typed loop variable passes a DbRef with `store_nr == u16::MAX` (the coroutine sentinel) to `database::remove`.  In debug `u16::MAX` overflows `allocations`; in release `u16::MAX % len` selects a real store and the `rec` (frame index ≈ 1–2) deletes a real record.
**Fix path:**
1. In `src/parser/fields.rs` (or wherever `e#remove` is resolved), check whether the loop's collection type is `iterator<T>` (backed by `OpCoroutineCreate`).  If so, emit: `error: e#remove is not valid on a generator iterator`.
2. In `database::remove` (or the calling opcode), add: `if db.store_nr == COROUTINE_STORE { debug_assert!(false, "remove on coroutine DbRef"); return; }`.  The `return` prevents release-build corruption even if the compiler check is missing.
3. Test: `e#remove` on a generator iterator is a compile error; a debug-only test verifies the runtime guard fires if the check is bypassed.
**Effort:** Extra Small
**Target:** 0.8.3

---

### S25  CO1.3d — coroutine text serialisation
**Sources:** SAFE.md § P2-R1/R2/R3, CAVEATS.md C23/C24, COROUTINE.md § CO1.3d/SC-CO-1/SC-CO-8/SC-CO-10

#### S25.1 — Text arg serialisation at coroutine create *(completed 0.8.3)*

`serialise_text_args` in `State` walks each attribute slot in `stack_bytes`
(only arg-sized `Str` slots, 16 bytes each), clones dynamic strings into
owned `String` objects stored in `text_owned`, and patches the `Str` pointer
in `stack_bytes` to point to the owned buffer.  Called from `coroutine_create`.
This fixed C23 (use-after-free on first resume for generators with `text` args).

#### S25.2 — Pointer-patch on resume + String drain on exhaustion *(completed 0.8.3)*

`coroutine_next` re-patches text-arg `Str` pointers from `text_owned` into the
cloned `bytes` before copying them to the live stack (M6-b).
`coroutine_return` calls `frame.text_owned.clear()` before `stack_bytes.clear()`,
which drops the owned String objects via RAII (M7-a).

#### S25.3 — Text local leak on early `break` from a generator loop *(completed 0.8.3 — C24)*

**Severity:** High — memory leak affects every generator with at least one text
local variable that is consumed via `break` (not iterated to exhaustion).

**Precise diagnosis (2026-03-29):**

Text local variables (e.g. `word = "hello"` inside a generator body) are `String`
objects (24 bytes: ptr+len+cap) held on the generator's live stack.  At
`coroutine_yield`, the raw bytes `[base..value_start]` are bitwise-copied to
`frame.stack_bytes`.  The copy is safe across yield/resume cycles because:

- String heap buffers are not freed while the generator is suspended (no Rust
  destructor runs on the abandoned live-stack copy).
- On resume, `coroutine_next` raw-copies `frame.stack_bytes` back to the live
  stack — the same heap pointer is restored and remains valid.
- At exhaustion via `coroutine_return`, `OpFreeText` has already been emitted
  before `OpCoroutineReturn` by `scopes::check`.  The live-stack String is freed
  by `OpFreeText`; `frame.stack_bytes` then contains stale bytes pointing to an
  already-freed allocation, which `frame.stack_bytes.clear()` discards safely.

**The single remaining leak path** — generator is `Suspended` (has yielded), then
the consumer breaks from the for-loop before exhaustion:

1. `OpFreeCoroutine` fires → `free_coroutine(idx)`
2. `free_coroutine` sets `self.coroutines[idx] = None`
3. This drops `Box<CoroutineFrame>`, which drops `stack_bytes: Vec<u8>`
4. `Vec<u8>::drop` frees the raw byte buffer but does NOT call `String::drop` on
   embedded String structs — their heap allocations (`"hello"`, etc.) are leaked.

**Complication — uninitialized text local slots:**

Zone 2 variables (including text locals) are pre-claimed at function entry via
`OpReserveFrame` (which only bumps `stack_pos`, does not zero memory).  If a
text local is assigned AFTER the yield point, its slot in `frame.stack_bytes`
contains garbage bytes from the store.  Calling `drop_in_place::<String>` on
garbage bytes is undefined behaviour.

**Fix design (S25.3):**

Step 1 — **Zero Zone 2 at generator startup** (in `coroutine_next`, `Created` status only).
After copying `frame.stack_bytes` (args+return-slot only) to the live stack,
compute the Zone 2 region extent from `def.variables` and zero those store bytes:

```rust
// After: std::ptr::copy_nonoverlapping(bytes, dst, bytes.len())
// New:   zero the Zone-2 region so uninitialised text locals start with null ptr.
let zone2_abs = self.stack_cur.pos + stack_base + bytes.len() as u32;
let zone2_size = Self::generator_zone2_size(d_nr, self.data_ptr);
if zone2_size > 0 {
    let store = self.database.store_mut(&self.stack_cur);
    let ptr = store.addr_mut::<u8>(self.stack_cur.rec, zone2_abs);
    unsafe { std::ptr::write_bytes(ptr, 0, zone2_size); }
}
```

```rust
/// Compute the total Zone-2 variable extent for generator function `d_nr`.
/// Returns bytes above the args+return-slot region (= `args_size + 4`).
fn generator_zone2_size(d_nr: u32, data_ptr: *const Data) -> usize {
    if data_ptr.is_null() { return 0; }
    let data = unsafe { &*data_ptr };
    let def = data.definitions.get(d_nr as usize)?;
    let vars = &def.variables;
    let mut top: u16 = 0;
    for v in 0..vars.count() {
        if vars.is_argument(v) { continue; }
        let slot = vars.stack(v);
        if slot == u16::MAX { continue; }
        let sz = vars.size(v, &Context::Variable);
        top = top.max(slot.saturating_add(sz));
    }
    top as usize
}
```

Step 2 — **Drop text locals in `free_coroutine`** before setting the slot to `None`:

```rust
pub fn free_coroutine(&mut self, idx: usize) {
    if idx > 0 && idx < self.coroutines.len() {
        // C24 / S25.3: drop text-local String objects from a suspended frame.
        if let Some(frame) = self.coroutines[idx].as_mut() {
            if frame.status == CoroutineStatus::Suspended {
                let d_nr = frame.d_nr;
                let data_ptr = self.data_ptr; // raw ptr — no borrow conflict
                Self::drop_text_locals_in_bytes(d_nr, &mut frame.stack_bytes, data_ptr);
            }
        }
        self.coroutines[idx] = None;
    }
}

/// Drop String objects embedded at text-local slots in `bytes`.
/// Guards against uninitialized slots via null-ptr check (Step 1 zeroed them).
fn drop_text_locals_in_bytes(d_nr: u32, bytes: &mut Vec<u8>, data_ptr: *const Data) {
    if data_ptr.is_null() { return; }
    let data = unsafe { &*data_ptr };
    let Some(def) = data.definitions.get(d_nr as usize) else { return };
    let vars = &def.variables;
    for v in 0..vars.count() {
        if vars.is_argument(v) { continue; }
        if !matches!(vars.tp(v), Type::Text(_)) { continue; }
        let slot = vars.stack(v);
        if slot == u16::MAX { continue; }
        let off = slot as usize;
        if off + std::mem::size_of::<String>() > bytes.len() { continue; }
        // Check the String's ptr field (first word on 64-bit).
        // Null means uninitialized (zeroed in Step 1); skip.
        let ptr_val: usize = unsafe {
            std::ptr::read_unaligned(bytes.as_ptr().add(off).cast::<usize>())
        };
        if ptr_val == 0 { continue; }
        // Drop in place and zero to prevent any future double-drop.
        unsafe { std::ptr::drop_in_place(bytes.as_mut_ptr().add(off).cast::<String>()); }
        unsafe { std::ptr::write_bytes(bytes.as_mut_ptr().add(off), 0, std::mem::size_of::<String>()); }
    }
}
```

Step 3 — **Fix misleading comment** in `coroutine_yield` (line ~723):
Remove the sentence "CO1.3d is now implemented — text locals are serialised to
frame.text_owned above".  Replace with accurate text: "The raw-bytes copy of text
locals in `stack_bytes` is safe across yield/resume cycles because no external code
frees the String heap buffers while suspended.  The early-break leak is fixed by
`free_coroutine` (S25.3)."

**Files changed:** `src/state/mod.rs` (3 locations: `free_coroutine`, `coroutine_next`,
new `generator_zone2_size` + `drop_text_locals_in_bytes` helpers)

**Tests to add** (`tests/expressions.rs`):
- `coroutine_text_local_early_break` — generator has text local, loop breaks after
  first yield.  Run under Miri to verify no leak.
- `coroutine_text_local_declared_after_first_yield` — text local declared after
  the first yield; no panic at break.  Verifies the null-ptr guard.

**Atomicity:** Steps 1 and 2 must land in the same commit.  If Step 1 lands without
Step 2, Zone 2 is zeroed but Strings are still leaked.  If Step 2 lands without
Step 1, `drop_in_place` may fire on garbage bytes (UB).

**Effort:** Small (1–2 hours)
**Target:** 0.8.3

---

### S26  `OpFreeCoroutine` at for-loop exit
**Sources:** SAFE.md § P2-R7, COROUTINE.md § Phase 1
**Severity:** Low — memory growth; `State::coroutines` accumulates one `Box<CoroutineFrame>` per generator invocation forever.
**Description:** `coroutine_return` marks the frame `Exhausted` but never sets the slot to `None`.  The `free_coroutine(idx)` helper is designed but never called.  Programs that create many generators in a loop grow `State::coroutines` without bound.
**Fix path:**
1. In the `for … in gen { }` desugaring codegen, emit `OpFreeCoroutine(gen_slot)` at loop exit (both exhaustion and `break`).
2. Implement `OpFreeCoroutine` in `fill.rs`: call `free_coroutine(idx)` which sets `coroutines[idx] = None`.
3. Optionally, lazily free in `coroutine_exhausted` when it first observes `Exhausted` status (covers the `explicit-advance` API path).
**Effort:** Medium
**Target:** 0.8.3

---

### S27  Coroutine `text_positions` save/restore across yield/resume
**Sources:** SAFE.md § P2-R4
**Severity:** Medium (debug-only) — `text_positions` BTreeSet becomes inconsistent across yield/resume, causing false double-free misses and masking missing `OpFreeText` for unrelated code.
**Description:** `coroutine_yield` rewinds `stack_pos` but does not remove text-local entries from `State::text_positions`.  The orphaned entries interfere with the debug detector for unrelated text frees at the same stack positions.
**Fix path:**
1. In `coroutine_yield` (debug path): collect `text_positions` entries in `[base, locals_end)`, remove them, store in `frame.saved_text_positions: BTreeSet<u32>`.
2. In `coroutine_next` (debug path): re-insert `frame.saved_text_positions` and clear it.
3. In `coroutine_return` (debug path): clear `frame.saved_text_positions` without reinserting.
**Effort:** Small (debug-only path)
**Target:** 0.8.3

---

### S28  Debug generation-counter for stale DbRef detection in coroutines
**Sources:** SAFE.md § P2-R8, COROUTINE.md § SC-CO-2
**Severity:** Medium — a generator resuming after its backing record or store was freed silently reads/writes wrong data with no diagnostic.
**Description:** A `DbRef` live in a generator local at a `yield` point can refer to memory freed or resized by the consumer between iterations.  Worse than ordinary functions: the suspension window spans many `next()` calls.
**Fix path:**
1. Add `generation: u32` to `Store`; increment on every `claim`, `delete`, and `resize`.
2. When `coroutine_create` / `coroutine_yield` saves a `DbRef` to `stack_bytes`, also record `(store_nr, generation_at_save)` in a new `frame.store_generations: Vec<(u16, u32)>`.
3. At `coroutine_next`, verify each saved store's current generation matches; emit a runtime diagnostic on mismatch.
**Effort:** Medium
**Target:** 0.8.3

---

### S29  Parallel store hardening: `thread::scope` + LIFO assert + skip claims
**Sources:** SAFE.md § P1-R2/P1-R3/P1-R4
**Severity:** Low/Medium — three independent low-effort fixes for parallel store infrastructure.
**Description:**
- **P1-R2:** `run_parallel_direct` uses a raw `*mut u8` with a lifetime invariant enforced only by convention; `thread::spawn` + manual join does not give compile-time guarantees.
- **P1-R3:** `clone_locked` copies `self.claims` (all live record offsets) into worker clones that never call `validate()` — wasted O(records) allocation per worker.
- **P1-R4:** `free_named` relies on LIFO store freeing order; out-of-order frees stall `max` and may cause subsequent `database()` to reuse a live slot.
**Fix path:**
1. Replace `thread::spawn` + manual join in `run_parallel_direct` with `std::thread::scope` (Rust 1.63+) to give compile-time lifetime enforcement over `out_ptr`.
2. Add `clone_locked_for_worker` on `Store` that omits `claims: HashSet::new()`; use it in `Stores::clone_for_worker`.
3. Add `debug_assert!(store_nr == self.max - 1, "free() must be called in LIFO order")` in `free_named`.
**Effort:** Small (three independent one-function changes)
**Target:** 0.8.3

---

### S30  `WorkerStores` newtype for type-level non-aliasing
**Sources:** SAFE.md § P1-R5
**Severity:** Low — no current bug; guards against future extensions to the parallel dispatch that could silently allow workers to hold main-thread `DbRef` values.
**Description:** The architecture relies on convention (workers receive cloned stores and may not hold main-thread `DbRef`s) rather than Rust types.  A future refactor extending worker dispatch could silently break the invariant.
**Fix path:**
1. Introduce `WorkerStores(Stores)` newtype, constructible only by `clone_for_worker` (private inner field).
2. Worker closures receive `WorkerStores`; the type is `Send` but not `Sync`, preventing cross-thread sharing.
3. Long-term: add `origin: StoreOrigin` tag to `DbRef` and a debug assert in `copy_from_worker` that all result DbRefs have worker origin, not main-thread origin.
**Effort:** Medium
**Depends:** S29 (clean parallel store state first)
**Target:** 0.8.3

---

## N — Native Codegen

All N-tier items (N1–N9) are completed.  Native test parity achieved 2026-03-23:
all `.loft` tests pass in both interpreter and native mode.
Full design in [NATIVE.md](NATIVE.md).

---

### N8  Native codegen: extend to tuples, coroutines, and generics
**Sources:** CAVEATS.md C19, NATIVE.md, TUPLES.md, COROUTINE.md
**Severity:** Medium — programs using tuples, coroutines, or `maybe<T>` cannot be compiled with `--native`.
**Description:** The native (`--native`) code generator currently falls back to the interpreter for three feature areas (see CAVEATS.md C19): tuples, coroutines, and generic/maybe types.  Each area is split into independently shippable sub-items below.

---

#### N8a.1 — Native: `Type::Tuple` dispatch in code generator
**Effort:** Small · **Depends:** T1
Add `Type::Tuple` to all `output_type`, `output_init`, `output_set`, and variable-declaration paths in `src/generation/`.  Until N8a.2 is done, functions that use tuples should be gracefully skipped (added to `SCRIPTS_NATIVE_SKIP`).
**Tests:** compile without errors for files that don’t use tuple operations; skip gate for `50-tuples.loft`.

#### N8a.2 — Native: tuple construction and element access
**Effort:** Small · **Depends:** N8a.1
Emit a tuple literal as consecutive scalar assignments onto the Rust stack frame.  Emit element reads (`.0`, `.1`, …) as direct field reads from the emitted Rust struct/tuple.  Emit `OpPutInt`/`OpPutText` analogs for element writes.
**Tests:** `tests/scripts/50-tuples.loft` passes in `--native` mode for construction and read sections; element assignment and deconstruction covered by sub-tests.

#### N8a.3 — Native: tuple function return (multi-value Rust struct)
**Effort:** Medium · **Depends:** N8a.2
Tuple-returning functions emit a generated Rust struct (e.g. `struct Ret_foo { f0: i64, f1: String }`) as the return type.  Caller deconstructs the struct into local slots.  LHS deconstruction (`(a, b) = foo()`) handled in the call site template.
**Tests:** `50-tuples.loft` fully passes in `--native` mode (no `SCRIPTS_NATIVE_SKIP` entry).

---

#### N8b.1 — Native: coroutine state-machine transform design
**Effort:** High · **Depends:** CO1
Design and document the Rust enum state machine that represents a suspended coroutine.  Each `yield` point becomes a variant that stores all live locals.  Write the state-machine emitter skeleton in `src/generation/`; no working coroutines yet, but the infrastructure compiles.  Document the design in NATIVE.md § N8b.
**Note:** Using `genawaiter` or `async-std` generators is an alternative; evaluate before committing to the hand-written state machine approach.

#### N8b.2 — Native: basic coroutine emission (yield/resume cycle)
**Effort:** High · **Depends:** N8b.1
Emit `OpCoroutineCreate`, `OpCoroutineNext`, `OpYield`, and `OpCoroutineReturn` using the state machine from N8b.1.  Cover coroutines with integer/float/boolean yields and no text locals (text serialisation adds complexity, tackled as a follow-on).
**Tests:** `tests/scripts/51-coroutines.loft` basic sections pass in `--native`; text-yield sections remain skipped.

#### N8b.3 — Native: `yield from` delegation in native coroutine
**Effort:** Medium · **Depends:** N8b.2
Extend the state machine emitter to handle `yield from inner()` — the sub-generator loop is inlined into the outer state machine as an additional state range.  Requires careful handling of the sub-generator’s exhaustion sentinel.
**Tests:** `51-coroutines.loft` fully passes in `--native` mode (yield-from sections un-skipped).

---

#### N8c.1 — Native: audit which generic instantiations fail and why
**Effort:** Small · **Depends:** none
Generic functions are monomorphized at parse time (`try_generic_instantiation` in
`src/parser/mod.rs`); each call site produces a concrete `DefType::Function` named
`t_<len><type>_<name>` (e.g. `t_4text_identity`).  Native codegen sees only concrete
functions.  The P5 skip is because some monomorphized instantiations produce compile
errors, not because generics are unsupported at codegen level.

Audit procedure:
1. Temporarily remove `"48-generics.loft"` from `SCRIPTS_NATIVE_SKIP`.
2. Run `cargo test --test native 2>&1` and capture the exact compile errors.
3. Inspect the generated `.rs` file for the failing `t_4text_*` functions.
4. Record findings in NATIVE.md § N8c.1 before writing N8c.2.

Expected: text-returning instantiations lack the `Str::new()` return wrapping or have a text-parameter type mismatch.  Full design in NATIVE.md § N8c.
**Output:** Exact error message + root-cause note in NATIVE.md § N8c.1.

#### N8c.2 — Native: fix failing monomorphised instantiations
**Effort:** Small · **Depends:** N8c.1
Apply the fix identified in N8c.1.  If the issue is text-return wrapping: verify
`output_function()` applies the `Str::new()` path for all `Type::Text` return types
including `t_*` functions.  If parameter type: fix the call-site argument emission for
text arguments in monomorphized calls.  Remove `"48-generics.loft"` from
`SCRIPTS_NATIVE_SKIP`; confirm `cargo test --test native` passes.
**Tests:** `48-generics.loft` passes in `--native` mode; all four identity instantiations
(integer, float, text, boolean) and both pick_second instantiations produce correct output.

---

**Overall effort:** N8a Small+Small+Medium; N8b High+High+Medium; N8c Small+Small
**Depends:** T1 (N8a), CO1 (N8b)
**Target:** 0.8.3

---

### S31  Native harness: pass `--extern` for optional feature deps
**Sources:** CAVEATS.md C27
**Severity:** Medium — `rand`, `rand_seed`, `rand_indices` and any future optional-dep functions are silently untested in native mode.
**Description:** The native test harness in `tests/native.rs` compiles generated `.rs` files by invoking `rustc` directly with `--extern loft=libloft.rlib`.  Optional feature dependencies (`rand_core`, `rand_pcg`) are not passed as `--extern` flags, so any generated code that uses the `random` feature fails to compile with `E0433: use of undeclared crate or module 'rand_core'`.  `15-random.loft` and `21-random.loft` are therefore in `SCRIPTS_NATIVE_SKIP` / `NATIVE_SKIP`.

**Fix path:**
1. In `find_loft_rlib()` (`tests/native.rs`), after locating the `deps/` directory, scan it for `.rlib` files matching the optional deps listed in `Cargo.toml` (`rand_core`, `rand_pcg`, `png`, etc.).
2. Build a `Vec<(String, PathBuf)>` of `(crate_name, rlib_path)` pairs.
3. Pass each as an additional `--extern <crate_name>=<path>` argument in the `rustc` invocations inside `run_native_test`.
4. Remove `"15-random.loft"` from `SCRIPTS_NATIVE_SKIP` and `"21-random.loft"` from `NATIVE_SKIP`.
5. Confirm `cargo test --test native` passes for both random files.

**Tests:** `15-random.loft` and `21-random.loft` pass in native mode.
**Effort:** Small
**Target:** 0.8.3

---

### S32  Fix slot conflict in `20-binary.loft` (`rv` / `_read_34`) — **Done**
**Sources:** CAVEATS.md C28
**Status:** Fixed — `20-binary.loft` runs unconditionally in `tests/wrap.rs::binary` and `tests/native.rs`; no longer in `ignored_scripts()` or `SCRIPTS_NATIVE_SKIP`, no `#[ignore]`.

**Tests:** `binary` and `loft_suite` (wrap) pass; `20-binary.loft` passes in native mode.

---

### S34  Interpreter: `20-binary.loft` `pos >= TOS` assertion at codegen.rs:751 — **Done**
**Sources:** `tests/scripts/20-binary.loft`, `src/state/codegen.rs:751`
**Status:** Fixed (0.8.3) — `skip_free` mechanism in `src/state/codegen.rs` and
`src/variables/validate.rs` aliases the inner `_read_*` variable to its TOS slot,
suppresses the double-free, and skips the conflict check.  `wrap::binary` passes
unconditionally; `20-binary.loft` removed from `ignored_scripts()`.
Side effect: exposed a pre-existing native codegen bug (S35) for the same pattern.

---

### S35  Native: Insert-return pattern emits malformed Rust
**Sources:** `tests/native.rs` `SCRIPTS_NATIVE_SKIP`, `tests/scripts/20-binary.loft`
**Severity:** Medium — the native codegen path for `20-binary.loft` has been excluded
since S34's interpreter fix exposed it.
**Description:** The native code generator (`src/generation/`) emits malformed Rust for
the IR pattern `Set(rv, Insert([Set(_read_34, Null), Block]))`.  This is a block-return
pattern where the return value `rv` is assigned the result of an `Insert` that contains
a nested `Set`.  The emitted Rust looks like:

```rust
let mut var_rv: DbRef =   let mut var__read_34: DbRef = DbRef::null();
```

The inner `Set(_read_34, Null)` is being emitted inline as a declaration rather than
as a separate statement before the `Insert` call, producing a declaration in the middle
of an expression context.

**Root cause (confirmed):** `output_set` in `src/generation/dispatch.rs` handles
`Value::Set(var, to)` by writing `let mut var_{name}: type = ` and then calling
`output_code_inner(w, to)` for the RHS.  When `to` is `Value::Insert(ops)`, the
`Value::Insert` arm in `output_code_inner` (emit.rs:52–63) iterates over `ops` and
emits each one indented with a trailing semicolon — treating them as statements.
This is correct at the top level but wrong inside an expression context.  The result
is a Rust declaration nested inside another Rust expression, which is a syntax error.

**Fix path (concrete — `src/generation/dispatch.rs`, `output_set`):**

Add a branch for `to = Value::Insert(ops)` before the general `output_code_inner`
call, handling it by hoisting all-but-last ops as statements then assigning the
last op's result:

```rust
// S35: Set(var, Insert([stmt1, ..., last_expr])) — hoist all-but-last ops
// as statements before the declaration, then assign from the final expression.
if let Value::Insert(ops) = to {
    // Emit prefix statements (all except the last op).
    for op in &ops[..ops.len() - 1] {
        self.indent(w)?;
        self.output_code_inner(w, op)?;
        writeln!(w, ";")?;
    }
    self.indent(w)?;
    // Now emit the declaration/assignment with only the last op as the value.
    if self.declared.contains(&var) {
        write!(w, "var_{name} = ")?;
    } else {
        self.declared.insert(var);
        let tp_str = rust_type(variables.tp(var), &Context::Variable);
        write!(w, "let mut var_{name}: {tp_str} = ")?;
    }
    self.output_code_inner(w, &ops[ops.len() - 1])?;
    return Ok(());
}
```

This branch is added after the `Value::Block` pre-declaration handling (line ~73) and
before the general `declared.contains` check (line ~85).

**Tests:** Remove `"20-binary.loft"` from `SCRIPTS_NATIVE_SKIP` in `tests/native.rs`
once fixed.
**Effort:** Medium
**Target:** 0.8.3

---

### S-lexer  Fix 15-lexer.loft / 16-parser.loft "Unknown record" crash
**Severity:** Medium — blocks `tests/docs/16-parser.loft` (the parser library test)
**Description:** Running `16-parser.loft` panics with `Unknown record 2147483648` at
`store.rs:897`.  The crash is on `main` too (not a regression).

**Root cause:** The crash path is `io.rs:611` in the `on==2` (sorted) branch of `iterate()`:
```
io.rs:607  sorted_rec = get_int(data.rec, data.pos) as u32   → i32::MIN cast to u32
io.rs:608  sorted_rec == 0?  No — 0x80000000 ≠ 0
io.rs:611  get_int(sorted_rec=2147483648, 4)                  → panics: "Unknown record"
```
When a struct field has an unresolved or unknown type (type 0 or 6), `set_default_value()`
in `database/structures.rs` writes `i32::MIN` instead of `0`.  When cast `as u32`, this
becomes 2147483648 — a poison value that passes the `== 0` null check but is not a valid
record number.  The `Parser` struct in `16-parser.loft` has hash/sorted collection fields;
if any field's type isn't fully resolved, iteration over it hits this crash.

**Fix path:**
1. **Immediate guard (io.rs):** In the `on==2` sorted branch, check `sorted_rec_raw <= 0`
   (not just `== 0`) before using it as a record number.  This catches both the `0`
   (empty collection) and `i32::MIN` (unresolved type) sentinels:
   ```rust
   let sorted_rec_raw = all[data.store_nr as usize].get_int(data.rec, data.pos);
   let sorted_rec = if sorted_rec_raw <= 0 { 0 } else { sorted_rec_raw as u32 };
   ```
   Apply the same guard to `on==1` (index) and `on>=4` (hash) branches.

2. **Debug guard (io.rs):** Add a `debug_assert!(sorted_rec_raw >= 0, ...)` before the
   cast so debug builds catch the root cause (unresolved type in set_default_value) rather
   than silently treating it as empty.

3. **Root-cause investigation:** Add a temporary `eprintln!` in `set_default_value()` when
   type is 0 or 6 to identify which Parser field has the unresolved type.  Fix the type
   resolution so the field gets a proper `0` default instead of `i32::MIN`.

4. **Extend LOFT_ITERATE_TRACE:** Add trace output for the sorted branch (currently only
   the index branch is traced).

**Tests:** `last` in `tests/wrap.rs` runs unconditionally.
**Effort:** Small (guard) + Medium (root-cause fix in type resolution)
**Target:** 0.8.3

---

### A7.2-par  Fix `load_one` heap corruption under parallel test execution
**Severity:** Low — only affects test parallelism, not production
**Description:** `load_one_registers_native_functions` in `tests/native_loader.rs` passes
with `--test-threads=1` but crashes with "corrupted size vs. prev_size" or
"munmap_chunk(): invalid pointer" when run in parallel with other tests.

**Root cause:** `extensions::load_one()` calls `Library::new(path)` (wrapping `dlopen`)
without synchronisation.  When multiple test threads call `dlopen` on the same `.so`
simultaneously, the shared library's initialisation code and the `trampoline_register`
callback allocate heap memory concurrently, causing corruption.  The `std::mem::forget(lib)`
at the end prevents cleanup, compounding the issue.

**Fix path:**
1. **Mutex in `load_one`** (`src/extensions.rs`):
   ```rust
   static LOAD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
   pub fn load_one(state: &mut State, path: &str) {
       let _guard = LOAD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
       // ... existing code ...
   }
   ```
   This serialises `dlopen` calls process-wide.  The lock only contends during library
   loading (startup, not runtime).

2. **Double-load prevention:** Track loaded library paths in a `HashSet<String>` behind
   the same mutex.  If a library has already been loaded, skip `Library::new()` entirely.

3. **Debugging:** Log at `load_one` entry/exit gated by `LOFT_LOG`:
   `eprintln!("[extensions] loading {path}")`.

**Tests:** `load_one_registers_native_functions` runs unconditionally.
**Effort:** Small
**Target:** 0.8.3

---

### O1  Superinstruction merging
**Status: no longer blocked — the prerequisite it waited on exists**
**Sources:** PERFORMANCE.md § P1
**Description:** Peephole pass in `src/compile.rs` merges common 4-opcode sequences (var/var/op/put) into single opcodes.  This was deferred on "no slots remain without a redesign of the opcode space (e.g. a two-byte opcode escape)" — **that escape exists**: byte 255 escapes to `OPERATORS[255 + ext]`, so the space is 511 opcodes and a superinstruction lands in the escape range as a two-byte opcode (INTERMEDIATE.md § Opcode budget; `make ops-census` for the current occupancy).  The extra byte-fetch is negligible against replacing ~4 one-byte ops.
**Expected gain:** 2–4× on tight integer loops.
**Effort:** Medium — the peephole pass itself; the opcode-space work it was waiting for is done.
**Target:** 1.1+

---

### O2  Stack raw pointer cache
**Sources:** PERFORMANCE.md § P2
**Description:** Every `get_stack`/`put_stack` call resolves `database.store(&stack_cur)` then computes a raw pointer from `rec + pos`. Adding `stack_base: *mut u8` to `State` that is refreshed once per function call/return eliminates this lookup on every arithmetic push/pop, reducing the hot path to a single pointer add.
**Expected gain:** 20–50% across all interpreter benchmarks.

**Fix path:**

*Step 1 — Add `stack_base: *mut u8` and `stack_dirty: bool` to `State`.*

*Step 2 — Add `refresh_stack_ptr()`:*
```rust
fn refresh_stack_ptr(&mut self) {
    self.stack_base = self.database
        .store_mut(&self.stack_cur)
        .record_ptr_mut(self.stack_cur.rec, self.stack_cur.pos);
}
```
Call after `fn_call`, `op_return`, and any op that sets `stack_dirty = true`.

*Step 3 — Rewrite `get_stack` / `put_stack` as pointer arithmetic:*
```rust
pub fn get_stack<T: Copy>(&mut self) -> T {
    self.stack_pos -= size_of::<T>() as u32;
    unsafe { *(self.stack_base.add(self.stack_pos as usize) as *const T) }
}
pub fn put_stack<T>(&mut self, val: T) {
    unsafe { *(self.stack_base.add(self.stack_pos as usize) as *mut T) = val; }
    self.stack_pos += size_of::<T>() as u32;
}
```

*Step 4 — Mark allocation ops as dirty.*
In `fill.rs`, ops that allocate new records (`OpDatabase`, `OpNewRecord`, `OpInsertVector`, `OpAppendCopy`) set `self.stack_dirty = true`. The dispatch loop checks `stack_dirty` once per iteration and calls `refresh_stack_ptr()`.

*Step 5 — Benchmark and verify.* Run `bench/run_bench.sh` before/after. Target: ≥20% gain on benchmark 01.

**Safety invariant:** `stack_base` is valid only while no allocation modifies `stack_cur`'s backing store. Collection ops use separate stores, so the invariant holds between `refresh_stack_ptr` calls as long as `stack_dirty` is set by any store-mutating op.

**Effort:** High (`src/state/mod.rs`, `src/fill.rs`)
**Target:** 1.1+

---

**Target:** 0.8.2

---

### O4  Native: direct-emit local collections
**Sources:** PERFORMANCE.md § N1
**Description:** All vector/hash access in generated Rust currently goes through `codegen_runtime` helpers that take `stores: &mut Stores` and decode `DbRef` pointers. For a local `vector<integer>` used only within one function, the correct Rust type is `Vec<i32>` — no stores, no DbRef, no bounds-check overhead.
**Expected gain:** 5–15× on data-structure benchmarks (word frequency 16×, dot product 12×, insertion sort 7×).

**Fix path:**

*Step 1 — Escape analysis pass (`src/generation/escape.rs`, new).*
Before native codegen runs per function, classify each local variable:
- `Local` — declared in this function, never passed by `&ref` to another function, never assigned to a struct field.
- `Escaping` — passed by reference, stored in a field, or returned.
Conservative: any uncertain case is `Escaping`.

*Step 2 — Direct-emit type mapping.*
For `Local` variables of collection type, emit Rust native types:
`vector<integer>` → `Vec<i32>`, `vector<float>` → `Vec<f64>`, `index<text, T>` → `HashMap<String, T>`.
Declaration site: `let mut var_counts: Vec<i32> = Vec::new();` instead of `let mut var_counts: DbRef = stores.null();`.

*Step 3 — Direct-emit operation mapping.*
In `output_code_inner`, when the target variable is `Local`, bypass `codegen_runtime`:
`v[i]` → `v[i as usize]`, `v.length` → `v.len() as i32`, `v.append(x)` → `v.push(x)`, `v.sort()` → `v.sort()`.
For `Escaping` variables, the existing `codegen_runtime` path is unchanged.

*Step 4 — Drop is automatic.*
`Local` `Vec`/`HashMap` values drop at end of scope via RAII — no `OpFreeRef` emission needed.

*Step 5 — Verify.*
All 10 native benchmarks pass; `native_dir` and `native_scripts` test suites pass. New assertion: generated Rust for a known `Local` vector contains `Vec<` not `DbRef`.

**Effort:** High (`src/generation/escape.rs` new, `src/generation/emit.rs`, `src/generation/mod.rs`)
**Target:** 1.1+

---

### O5  Native: omit `stores` param from pure functions
**Sources:** PERFORMANCE.md § N2
**Description:** Every generated function currently receives `stores: &mut Stores` even when it never touches a store. For recursive functions like Fibonacci, `rustc -O` cannot eliminate this parameter across recursive calls, adding a register save/restore pair per call (measured: 1.84× slower than hand-written Rust). Purity analysis emits a `_pure` variant without `stores`; the wrapper delegates to it.
**Expected gain:** 10–30% on recursive compute benchmarks.
**Depends:** O4

**Fix path:**

*Step 1 — Purity analysis (`src/generation/purity.rs`, new).*
Recursively scan `def.code: Value`. A function is **pure** if its IR contains none of:
`Value::Ref`, `Value::Store`, `Value::Format`, `Value::Call` to any op with `stores` in its `#rust` body.
Memoize per `def_nr` to avoid exponential recursion on call graphs.

*Step 2 — Emit `_pure` variant.*
For each pure function, emit two Rust functions:
```rust
fn n_fibonacci_pure(n: i32) -> i32 {   // no stores parameter
    if n <= 1 { return n; }
    n_fibonacci_pure(n - 1) + n_fibonacci_pure(n - 2)
}
fn n_fibonacci(stores: &mut Stores, n: i32) -> i32 {  // wrapper for uniform call interface
    n_fibonacci_pure(n)
}
```

*Step 3 — Call-site dispatch.*
In `output_call`, when emitting a call from a pure context to a pure callee, emit `n_foo_pure(…)` directly, omitting `stores`. This allows `rustc` to inline and tail-call-optimise freely.

*Step 4 — Verify.*
`n_fibonacci_pure` appears in generated Rust for any recursive integer function. All native benchmarks pass.

**Effort:** High (`src/generation/purity.rs` new, `src/generation/emit.rs`, `src/generation/mod.rs`)
**Target:** 1.1+

---


## H — HTTP / Web Services

Full design rationale and approach comparison: [WEB_SERVICES.md](lib_plans/06-web-services/HTTP_CLIENT.md).

The `#json` annotation is the key enabler: it synthesises `to_json` and `from_json` for a
struct, making `Type.from_json` a first-class callable fn-ref that composes with `map` and
`filter`.  The HTTP client is a thin blocking wrapper (via `ureq`) returning a plain
`HttpResponse` struct — no thread-local state, parallel-safe.  All web functionality is
gated behind an `http` Cargo feature.

---

### H1  `#json` annotation — parser and `to_json` synthesis
**Sources:** [WEB_SERVICES.md](lib_plans/06-web-services/HTTP_CLIENT.md) § Approach B, Phase 1
**Description:** Extend the annotation parser to accept `#json` (no value) before a struct
declaration.  For every annotated struct, the compiler synthesises a `to_json` method that
reuses the existing `:j` JSON format flag.  No new Rust dependencies are needed.
**Fix path:**

**Step 1 — Parser** (`src/parser/parser.rs` or `src/parser/expressions.rs`):
Extend the annotation-parsing path that currently handles `#rust "..."` to also accept
bare `#json`.  Store a `json: bool` flag on the struct definition node (parallel to how
`#rust` stores its string).  Emit a clear parse error if `#json` is placed on anything
other than a struct.
*Test:* `#json` before a struct compiles without error; `#json` before a `fn` produces a
single clear diagnostic.

**Step 2 — Synthesis** (`src/state/typedef.rs`):
During type registration, for each struct with `json: true`, synthesise an implicit `pub fn`
definition equivalent to:
```loft
pub fn to_json(self: T) -> text { "{self:j}" }
```
The synthesised def shares the struct's source location for error messages.
*Test:* `"{user:j}"` and `user.to_json()` produce identical output for a `#json` struct.

**Step 3 — Error for missing annotation** (`src/state/typedef.rs`):
If `to_json` is called on a struct without `#json`, emit a compile error:
`"to_json requires #json annotation on struct T"`.
*Test:* Unannotated struct calling `.to_json()` produces a single clear diagnostic.

**Effort:** Small (parser annotation extension + typedef synthesiser)
**Target:** 0.8.4
**Depends on:** —

---

### ~~H2  JSON primitive extraction stdlib~~ — WITHDRAWN

**Status:** Withdrawn 2026-04 — superseded by [P54 § JsonValue
enum](QUALITY.md#active-sprint--p54-jsonvalue-enum).  The
text-based `json_text/int/long/float/bool/items/nested` surface
this section designed has been replaced wholesale by the typed
`JsonValue` tree (`json_parse(text) -> JsonValue` plus six
variants and dedicated read/write helpers — see
[STDLIB.md § JSON](STDLIB.md)).  The original design is preserved
below as a historical record; do not implement.

**Sources:** [WEB_SERVICES.md](lib_plans/06-web-services/HTTP_CLIENT.md) § Approach B; CODE.md § Dependencies
**Description:** Add a new stdlib module `default/06_web.loft` with JSON field-extraction
functions.  Functions extract a single typed value from a JSON object body supplied as
a `text` string.  No `serde_json` dependency — the existing parsing primitives in
`src/database/structures.rs` are sufficient; a new `src/database/json.rs` module adds
schema-free navigation on top.
**Fix path:**

**Step 1 — Cargo dependency** (`Cargo.toml`):
Add only `ureq` (used in H4) under a new `http` optional feature.  No `serde_json`.
```toml
[features]
http = ["ureq"]

[dependencies]
ureq = { version = "2", optional = true }
```

**Step 2 — `src/database/json.rs`** (new file, ~80 lines, no new dependency):
Add as a submodule of `src/database/`.  Provides three `pub(crate)` building blocks:

```rust
// Find `key` in a top-level JSON object; return raw value slice (unallocated).
pub(crate) fn json_get_raw<'a>(text: &'a str, key: &str) -> Option<&'a str>

// Return raw JSON text for each element of a top-level JSON array.
pub(crate) fn json_array_items(text: &str) -> Vec<String>

// Parse a raw value slice into a Rust primitive (loft null sentinels on failure):
pub(crate) fn as_text(raw: &str) -> String   // strips quotes + handles \n \t \\
pub(crate) fn as_int(raw: &str) -> i32       // i32::MIN on failure
pub(crate) fn as_long(raw: &str) -> i64      // i64::MIN on failure
pub(crate) fn as_float(raw: &str) -> f64     // f64::NAN on failure
pub(crate) fn as_bool(raw: &str) -> bool     // false on failure
```

Internally `json.rs` uses its own `skip_ws`, `skip_value`, and `extract_string` helpers
(~50 lines combined).  These mirror the primitives in `structures.rs` but operate
schema-free: no `Stores`, no `DbRef`, no type lookup.  The byte-scanning logic is
identical in style to the existing `match_text` / `skip_float` functions.

*Design note:* The primitives in `structures.rs` (`match_text`, `match_integer`, etc.)
are `fn` (module-private) because they are only called by `parsing()` within the same
module.  Rather than widening their visibility, `json.rs` keeps its own small copies
to preserve the clean boundary between schema-driven and schema-free parsing.

**Step 3 — Loft declarations** (`default/06_web.loft`):
```loft
// Extract primitive values from a JSON object body.
// Returns zero/empty/null-sentinel if the key is absent or type does not match.
pub fn json_text(body: text, key: text) -> text;
pub fn json_int(body: text, key: text) -> integer;
pub fn json_long(body: text, key: text) -> long;
pub fn json_float(body: text, key: text) -> float;
pub fn json_bool(body: text, key: text) -> boolean;

// Split a JSON array body into element bodies (each element as raw JSON text).
pub fn json_items(array_body: text) -> vector<text>;

// Extract a named field as raw JSON text (object, array, or primitive).
// Use for nested structs and array fields: json_nested(body, "field").
pub fn json_nested(body: text, key: text) -> text;
```

**Step 4 — Rust implementation** (new `src/native_http.rs`, registered in `src/native.rs`):
Each native function calls `json::json_get_raw` then the appropriate `as_*` converter.
All functions return the loft null sentinel (or empty string) on any error — never panic.
- `json_text`: `json_get_raw(body, key).map(as_text).unwrap_or_default()`
- `json_int`: `json_get_raw(body, key).map(as_int).unwrap_or(i32::MIN)`
- `json_long`: `json_get_raw(body, key).map(as_long).unwrap_or(i64::MIN)`
- `json_float`: `json_get_raw(body, key).map(as_float).unwrap_or(f64::NAN)`
- `json_bool`: `json_get_raw(body, key).map(as_bool).unwrap_or(false)`
- `json_items`: `json_array_items(body)` → build a `vector<text>` via `stores.text_vector`
- `json_nested`: `json_get_raw(body, key).unwrap_or_default().to_string()`

**Step 5 — Feature gate** (`src/native.rs` or `src/main.rs`):
Register the H2 natives only when compiled with `--features http`.  Without the feature,
calling any `json_*` function raises a compile-time error:
`"json_text requires the 'http' Cargo feature"`.

*Tests:*
- Valid JSON object: each extractor returns the correct value.
- Missing key: returns zero/empty/null-sentinel without panic.
- Invalid JSON body: returns zero/empty/null-sentinel without panic.
- Nested object value: `json_nested` returns a string parseable by `json_int` etc.
- `json_items` on a 3-element array returns a `vector<text>` of length 3.
- Unicode and `\"` escapes inside string values are handled correctly.

**Effort:** Small–Medium (new `json.rs` ~80 lines + 7 native functions; no new dependency)
**Target:** 0.8.4
**Depends on:** H1 (for the `http` feature gate pattern)

---

### H3  `from_json` codegen — scalar struct fields
**Sources:** [WEB_SERVICES.md](lib_plans/06-web-services/HTTP_CLIENT.md) § Approach B, Phase 2
**Description:** For each `#json`-annotated struct whose fields are all primitive types
(`text`, `integer`, `float`, `single`, `boolean`, `character`), the compiler
synthesises a `from_json(body: text) -> T` function.  The result is a normal callable
fn-ref: `User.from_json` can be passed to `map` without any special syntax.
**Fix path:**

**Step 1 — Synthesis** (`src/state/typedef.rs`):
After H2 is in place, extend the `#json` synthesis pass (H1 Step 2) to also emit
`from_json`.  For each field, select the extractor by type:

| Loft type | Extractor call |
|-----------|---------------|
| `text` | `json_text(body, "field_name")` |
| `integer` | `json_int(body, "field_name")` |
| `float` / `single` | `json_float(body, "field_name")` |
| `boolean` | `json_bool(body, "field_name")` |
| `character` | first char of `json_text(body, "field_name")` |

The synthesised `from_json` body is a struct-literal expression using the above calls.
Fields not in the table (nested structs, enums, vectors) are silently skipped in this
phase (H5 adds them).

**Step 2 — fn-ref validation** (`src/state/compile.rs` or `src/state/codegen.rs`):
Verify that `Type.from_json` resolves as a callable fn-ref with type
`fn(text) -> Type`, so it can be passed directly to `json_items(...).map(...)` and
`json_items(...).filter(...)`.

*Tests:*
- `User.from_json(body)` returns a struct with all fields set from JSON.
- `json_items(resp.body).map(User.from_json)` returns a `vector<User>`.
- Absent JSON key sets the field to its zero value (0, "", false).
- Struct with a nested `#json` struct field compiles without error (nested field gets zero value until H5).

**Effort:** Medium (typedef synthesiser + fn-ref type check)
**Target:** 0.8.4
**Depends on:** H1, H2

---

### H4  HTTP client stdlib and `HttpResponse`
**Sources:** [WEB_SERVICES.md](lib_plans/06-web-services/HTTP_CLIENT.md) § Approach B, stdlib additions; PROBLEMS #55
**Description:** Add blocking HTTP functions to `default/06_web.loft` backed by `ureq`.
All functions return `HttpResponse` — a plain struct — so there is no thread-local status
state and the API is parallel-safe (see PROBLEMS #55).
**Fix path:**

**Step 1 — `HttpResponse` struct** (`default/06_web.loft`):
```loft
pub struct HttpResponse {
    status: integer
    body:   text
}

pub fn ok(self: HttpResponse) -> boolean {
    self.status >= 200 and self.status < 300
}
// Mirror the File read interface so HTTP sources are interchangeable with
// file sources in any function that processes text.
pub fn content(self: HttpResponse) -> text {
    self.body
}
pub fn lines(self: HttpResponse) -> vector<text> {
    self.body.split('\n')  // strips \r so CRLF bodies match LF bodies
}
```
No `#rust` needed; all three methods are plain loft.  `lines()` uses the same
CRLF-stripping logic as `File.lines()` — HTTP/1.1 bodies frequently use CRLF.

**Optical similarity with `File`:** the shared method names let processing
functions accept either source without modification:
```loft
fn process(rows: vector<text>) { ... }
process(file("local/data.txt").lines());
process(http_get("https://example.com/data").lines());
```

**Step 2 — HTTP functions declaration** (`default/06_web.loft`):
```loft
// Body-less requests
pub fn http_get(url: text) -> HttpResponse;
pub fn http_delete(url: text) -> HttpResponse;

// Body requests (body is a text string, typically to_json() output)
pub fn http_post(url: text, body: text) -> HttpResponse;
pub fn http_put(url: text, body: text) -> HttpResponse;
pub fn http_patch(url: text, body: text) -> HttpResponse;

// With explicit headers (each entry: "Name: Value")
pub fn http_get_h(url: text, headers: vector<text>) -> HttpResponse;
pub fn http_post_h(url: text, body: text, headers: vector<text>) -> HttpResponse;
pub fn http_put_h(url: text, body: text, headers: vector<text>) -> HttpResponse;
```

**Step 3 — Rust implementation** (`src/native_http.rs`):
Use `ureq::get(url).call()` / `.send_string(body)`.  Parse each `"Name: Value"` header
entry by splitting at the first `:`.  On network error, connection refused, or timeout,
return `HttpResponse { status: 0, body: "" }` — never panic.  Set a default timeout of
30 seconds.
```rust
fn http_get(url: &str) -> HttpResponse {
    match ureq::get(url).call() {
        Ok(resp) => HttpResponse {
            status: resp.status() as i32,
            body:   resp.into_string().unwrap_or_default(),
        },
        Err(_) => HttpResponse { status: 0, body: String::new() },
    }
}
```

**Step 4 — Content-Type default**:
`http_post` and `http_put` set `Content-Type: application/json` automatically when the
body is non-empty (the common case).  Callers who need a different content type use the
`_h` variants to supply their own `Content-Type` header.

*Tests (run with a local mock server or httpbin.org):*
- `http_get("https://httpbin.org/get").ok()` is `true`.
- `http_get("https://httpbin.org/status/404").status` is `404`.
- `http_post` with a JSON body returns the echoed body from `/post`.
- Network failure (bad URL) returns `HttpResponse { status: 0, body: "" }`.
- Header variants set the supplied headers (verify via httpbin.org `/headers`).

**Effort:** Medium (`ureq` integration + 8 native functions)
**Target:** 0.8.4
**Depends on:** H2 (for the `http` Cargo feature; `ureq` added there)

---

### H5  Nested/array/enum `from_json` and integration tests
**Sources:** [WEB_SERVICES.md](lib_plans/06-web-services/HTTP_CLIENT.md) § Approach B, Phases 3–4
**Description:** Extend the H3 `from_json` synthesiser to handle nested `#json` structs,
`vector<T>` array fields, and plain enum fields.  Add an integration test suite that calls
real HTTP endpoints and verifies the full round-trip.
**Fix path:**

**Step 1 — Nested `#json` struct fields** (`src/state/typedef.rs`):
For a field `addr: Address` where `Address` is `#json`-annotated, emit:
```loft
addr: Address.from_json(json_nested(body, "addr"))
```
The compiler must verify that `Address` is `#json` at the point of synthesis; if not,
emit: `"field 'addr' has type Address which is not annotated with #json"`.

**Step 2 — `vector<T>` array fields** (`src/state/typedef.rs`):
For a field `items: vector<Item>` where `Item` is `#json`, emit:
```loft
items: json_items(json_nested(body, "items")).map(Item.from_json)
```
This relies on `map` with fn-refs, which already works.  If `Item` is not `#json`, emit
a compile error.

**Step 3 — Plain enum fields** (`src/state/typedef.rs`):
For a field `status: Status` where `Status` is a plain (non-struct) enum, emit a `match`
on the string value:
```loft
status: match json_text(body, "status") {
    "Active"   => Status::Active,
    "Inactive" => Status::Inactive,
    _          => Status::Active,   // first variant as default
}
```
The default fallback uses the first variant; a compile-time warning notes it.
Struct-enum variants in JSON (e.g. `{"type": "Paid", "amount": 42}`) are not supported
in this phase — a compile error is emitted if a struct-enum field appears in a `#json` struct.

**Step 4 — `not null` field validation** (`src/state/typedef.rs`):
Fields declared `not null` whose JSON key is absent should emit a runtime warning (via the
logger) and keep the zero value rather than panicking.  This matches loft's general approach
of never crashing on bad data.

**Step 5 — Integration test suite** (`tests/web/`):
Write loft programs that call public stable APIs and assert on the response.  Tests should
be skipped if the `http` feature is not compiled in or if the network is unavailable:
- `GET https://httpbin.org/json` → parse known struct, assert fields.
- `POST https://httpbin.org/post` with JSON body → assert echoed body round-trips.
- `GET https://httpbin.org/status/500` → `resp.ok()` is `false`, `resp.status` is `500`.
- Nested struct: `GET https://httpbin.org/json` contains a nested `slideshow` object.
- Array field: `GET https://httpbin.org/json` contains a `slides` array.

**Effort:** Medium–High (3 codegen extensions + integration test infrastructure)
**Target:** 0.8.4
**Depends on:** H3, H4

---

## R — Repository

Standalone `loft` repository created (2026-03-16).  The remaining R item is the
workspace split needed before starting the Web IDE.

**Presentation / first-contact work** — the first-run experience, README
truthfulness, doc-site discoverability and repo surface — has its own design in
[FIRST_CONTACT.md](FIRST_CONTACT.md), from an external review carrying the
visitor's perspective (2026-07-24).  Batched B1–B5 there; B1 (first-run bugs)
is the sequencing head.

---

### R1  Add `cdylib` + `rlib` crate types for WASM compilation
**Sources:** WASM.md § Step 1, W1.1
**Description:** The loft interpreter must be compiled as a `cdylib` (dynamic library) to produce a `.wasm` file via `wasm-bindgen`, and as an `rlib` so the existing native tests and `cargo test` continue to work against the library API.  No workspace split is required for 0.8.3 — the binary targets (`[[bin]] loft`, `[[bin]] gendoc`) are separate compilation units and will not be included in the `cdylib` output.

**Fix path:**

**Step 1 — Add `[lib]` section to `Cargo.toml`:**
```toml
[lib]
name = "loft"
crate-type = ["cdylib", "rlib"]
```
If a `[lib]` section already exists, just add the `crate-type` line.

**Step 2 — Add `src/lib.rs` if not present:**
`src/lib.rs` should already exist and re-export the public API (`pub mod parser`, `pub mod compile`, `pub mod state`, etc.).  Verify it compiles cleanly as a library target with `cargo build --lib`.

**Step 3 — Verify no `main.rs` symbols leak into the `cdylib`:**
`cargo check --target wasm32-unknown-unknown --features wasm --no-default-features` must pass.  Any use of `std::process::exit`, `std::env::args`, or `dirs::home_dir` in `src/lib.rs`-reachable modules must be feature-gated (done in W1.3–W1.6).

**Step 4 — Deferred workspace split (post-1.0):**
A full workspace split into `loft-core / loft-cli / loft-gendoc` reduces incremental build times and isolates CLI from the library API.  This is deferred until the Web IDE (W2+) makes it necessary.  The current single-crate layout is sufficient for 0.8.3.

**Verify:** `cargo check` ✔  `cargo test` ✔  `cargo check --target wasm32-unknown-unknown --features wasm --no-default-features` ✔

**Effort:** Small (one `Cargo.toml` change; no logic changes)
**Depends on:** repo creation (done)
**Target:** 0.8.3

---

## W — Web IDE

A fully serverless, single-origin HTML application that lets users write, run, and
explore Loft programs in a browser without installing anything.  The existing Rust
interpreter is compiled to WebAssembly via a new `wasm` Cargo feature; the IDE shell
is plain ES-module JavaScript with no required build step after the WASM is compiled
once.  Full design in [WEB_IDE.md](lib_plans/62-web-ide/README.md).

---


### W2  Editor Shell
**Sources:** [WEB_IDE.md](lib_plans/62-web-ide/README.md) — M2
**Severity/Value:** High — the visible IDE; needed by all later W items
**Description:** A single `index.html` users can open directly (no bundler).
- `ide/src/loft-language.js` — CodeMirror 6 `StreamLanguage` tokenizer: keywords, types, string interpolation `{...}`, line/block comments, numbers
- `ide/src/editor.js` — CodeMirror 6 instance with line numbers, bracket matching, `setDiagnostics()` for gutter icons and underlines
- Layout: toolbar (project switcher + Run button), editor left, Console + Problems panels bottom

JS tests (5): keyword token, string interpolation span, line comment, type names, number literal.
**Effort:** Medium (CodeMirror 6 setup + Loft grammar)
**Depends on:** W1
**Target:** 1.0.0

---

### W3  Symbol Navigation
**Sources:** [WEB_IDE.md](lib_plans/62-web-ide/README.md) — M3
**Severity/Value:** Medium — go-to-definition and find-usages; significant IDE quality uplift
**Description:**
- `src/wasm.rs`: implement `get_symbols()` — walk `parser.data.def_names` and variable tables; return `[{name, kind, file, line, col, usages:[{file,line,col}]}]`
- `ide/src/symbols.js`: `buildIndex()`, `findAtPosition()`, `formatUsageList()`
- Editor: Ctrl+click → jump to definition; hover tooltip showing kind + file
- Outline panel (sidebar): lists all functions, structs, enums; clicking navigates

JS tests (3): find function definition, format usage list, no-match returns null.
**Effort:** Medium (Rust symbol walk + JS index)
**Depends on:** W1, W2
**Target:** 1.0.0

---

### W4  Multi-File Projects
**Sources:** [WEB_IDE.md](lib_plans/62-web-ide/README.md) — M4
**Severity/Value:** High — essential for any real program; single-file is a toy
**Description:** All projects persist in IndexedDB.  Project schema: `{id, name, modified, files:[{name,content}]}`.
- `ide/src/projects.js` — `listProjects()`, `loadProject(id)`, `saveProject(project)`, `deleteProject(id)`; auto-save on edit (debounced 2 s)
- UI: project-switcher dropdown, "New project" dialog, file-tree panel, tab bar, `use` filename auto-complete

JS tests (4): save/load roundtrip, list all projects, delete removes entry, auto-save updates timestamp.
**Effort:** Medium (IndexedDB wrapper + UI wiring)
**Depends on:** W2
**Target:** 1.0.0

---

### W5  Documentation & Examples Browser
**Sources:** [WEB_IDE.md](lib_plans/62-web-ide/README.md) — M5
**Severity/Value:** Medium — documentation access without leaving the IDE; example projects lower barrier to entry
**Description:**
- Build-time script `ide/scripts/bundle-docs.js`: parse `doc/*.html` → `assets/docs-bundle.json` (headings + prose + code blocks)
- `ide/src/docs.js` — renders bundle with substring search
- `ide/src/examples.js` — registers `tests/docs/*.loft` as one-click example projects ("Open as project")
- Right-sidebar tabs: **Docs** | **Examples** | **Outline**

Run the bundler automatically from `build.sh` after `cargo run --bin gendoc`.
**Effort:** Small–Medium (bundler script + panel UI)
**Depends on:** W2
**Target:** 1.0.0

---

### W6  Export, Import & PWA
**Sources:** [WEB_IDE.md](lib_plans/62-web-ide/README.md) — M6
**Severity/Value:** Medium — closes the loop between browser and local development
**Description:**
- `ide/src/export.js`: `exportZip(project)` → `Blob` (JSZip); `importZip(blob)` → project object; drag-and-drop import
- Export ZIP layout: `<name>/src/*.loft`, `<name>/lib/*.loft` (if any), `README.md`, `run.sh`, `run.bat` — matches `loft`'s `use` resolution path so unzip + run works locally
- `ide/sw.js` — service worker pre-caches all IDE assets; offline after first load
- `ide/manifest.json` — PWA manifest
- URL sharing: single-file programs encoded as `#code=<base64>` in URL

JS tests (4): ZIP contains `src/main.loft`, `run.sh` invokes `loft`, import roundtrip preserves content, URL encode/decode.
**Effort:** Small–Medium (JSZip + service worker)
**Depends on:** W4
**Target:** 1.0.0

---
## P70 — Text in `generate_set` TOS-override causes SIGSEGV

**Status:** Workaround in place — `Type::Text` excluded from the large-type
TOS-override in `generate_set` (`codegen.rs:~689`).  Stable; no regressions.

**Real fix (only if C43 text slot reuse needs it):** switch `OpFreeText` to
take a variable number instead of a pre-resolved stack offset, so codegen
resolves the final position at emit time — same pattern as `OpFreeRef`.
Touches `scopes.rs` (emit `Value::Var(v)`) and `codegen.rs` (resolve at
emit).  Safe but affects every text-variable scope exit.

**Effort:** Medium.  **Target:** deferred to C43.

---

## C43 — Text slot reuse: zone-2 dead-slot tracking

**Problem:** Text variables (24 bytes each) cannot reuse dead slots, wasting
stack space when many short-lived text variables are used sequentially.

**Root cause:** Text variables are placed by zone 2 (`place_large_and_recurse`
in `slots.rs`), which assigns slots sequentially at TOS without dead-slot
reuse.  Zone 1 has dead-slot reuse but only handles variables ≤ 8 bytes.

**Failed attempt:** A naive same-type reuse check caused slot conflicts
because it only compared against one dead variable, not ALL assigned
variables.  `nums` at [40,52) was still live when `_map_result_5` reused
slot 44.  Full conflict scan (like zone-1) is required.

**Files:** `src/variables/slots.rs`

P70 (text TOS-override) is NOT blocking: text-to-text same-size reuse
places the variable at the dead slot's existing position — no movement
occurs, so the `generate_set` TOS-override path is never triggered.

---

### C43.1 — Zone-2 dead-slot finder with full conflict scan

**Goal:** A standalone `find_reusable_zone2_slot` function that returns a
safe reuse slot or `None`.

**File:** `src/variables/slots.rs`

**Implementation:**
```rust
/// Find a dead zone-2 variable whose slot can be reused by variable `v`.
/// Returns `Some(slot)` if a conflict-free candidate exists, `None` otherwise.
/// Guards: same size, same type discriminant, dead (last_use < v.first_def),
/// no spatial+temporal overlap with any other assigned variable.
fn find_reusable_zone2_slot(
    function: &Function,
    v: usize,
    scope: u16,
) -> Option<u16> {
    let v_size = size(&function.variables[v].type_def, &Context::Variable);
    let v_first = function.variables[v].first_def;
    let v_last = function.variables[v].last_use;
    let v_disc = std::mem::discriminant(&function.variables[v].type_def);
    for (j, jv) in function.variables.iter().enumerate() {
        if j == v || jv.stack_pos == u16::MAX || jv.scope != scope {
            continue;
        }
        let j_size = size(&jv.type_def, &Context::Variable);
        // Same size + same type family (e.g., text-to-text only).
        if j_size != v_size || std::mem::discriminant(&jv.type_def) != v_disc {
            continue;
        }
        // Dead: candidate's last use is before our first definition.
        if jv.last_use >= v_first {
            continue;
        }
        // Full conflict scan: verify no other variable overlaps both
        // spatially (byte range) and temporally (live interval).
        let slot = jv.stack_pos;
        let conflict = function.variables.iter().enumerate().any(|(k, kv)| {
            if k == v || k == j || kv.stack_pos == u16::MAX {
                return false;
            }
            let ks = kv.stack_pos;
            let ke = ks + size(&kv.type_def, &Context::Variable);
            // Spatial overlap: [slot, slot+v_size) ∩ [ks, ke) ≠ ∅
            let spatial = slot < ke && ks < slot + v_size;
            // Temporal overlap: [v_first, v_last] ∩ [k_first, k_last] ≠ ∅
            let temporal = v_first <= kv.last_use && v_last >= kv.first_def;
            spatial && temporal
        });
        if !conflict {
            return Some(slot);
        }
    }
    None
}
```

**Debug guard:** When `function.logging` is true, emit:
```
[assign_slots]   zone2-reuse '{}' reuses dead '{}' at slot={}
```

**Verification:**
1. Add a unit test `zone2_reuse_conflict_free` that creates three 24-byte
   variables: v1 (live 0–10), v2 (live 5–15, overlaps v1), v3 (live 11–20,
   does not overlap v1).  Assert v3 reuses v1's slot but v2 does not.
2. `cargo test --lib assign_slots` — all slot tests pass.

---

### C43.2 — Wire zone-2 reuse into `place_large_and_recurse`

**Goal:** Call `find_reusable_zone2_slot` before advancing `*tos`.

**File:** `src/variables/slots.rs`, function `place_large_and_recurse`

**Change:** In the `if v_size > 8` block (line ~183), before `let v_slot = *tos`:
```rust
let v_slot = if let Some(slot) = find_reusable_zone2_slot(function, v, scope) {
    slot
} else {
    let s = *tos;
    *tos += v_size;
    s
};
```

Remove the existing `*tos += v_size` after `pre_assigned_pos = v_slot`.

**Debug guard:** `function.logging` message distinguishes "zone2" (new slot)
from "zone2-reuse" (reused slot).

**Verification:**
1. `cargo test --lib assign_slots` — all unit tests pass including the new
   `zone2_reuse_conflict_free` from C43.1.
2. `cargo test warning_only_program` — the `46-caveats.loft` script that
   triggered the original failure must pass (the full conflict scan prevents
   the `nums` / `_map_result_5` partial overlap).

---

### C43.3 — Enable `assign_slots_sequential_text_reuse` test — **Done**

`#[ignore]` removed; test runs unconditionally in `src/variables/slots.rs`.

---

### C43.4 — Integration test: text-heavy script with slot validation

**Goal:** Verify text slot reuse works end-to-end in a loft program.

**File:** `tests/expressions.rs`

**Test:**
```rust
#[test]
fn text_slot_reuse_sequential() {
    // Two sequential text variables with non-overlapping lifetimes
    // should not cause stack corruption.
    code!(
        "fn check() -> text {
             a = \"hello\";
             b = a + \" world\";
             c = \"goodbye\";
             d = c + \" world\";
             d
         }"
    )
    .expr("check()")
    .result(Value::str("goodbye world"));
}
```

**Verification:** `make ci` — zero failures across all test suites.

---

## C47 — Native codegen: cross-scope closure CallRef dispatch

**Status:** Interpreter cross-scope closures work.  Native codegen has two
remaining issues.

**Sub-issues fixed:**
1. *(Fixed)* `Value::FnRef` emits `(d_nr, var___clos_N)` when closure exists.
2. *(Fixed)* CallRef dispatch passes `var_f.1` as `__closure` for cross-scope.
3. *(Fixed)* Scope analysis bounds check: deps from callee variable space
   (d >= function.count()) are skipped in Set registration and check_ref_leaks.

**Sub-issues remaining:**

**C47.3a — Broad CallRef match dispatch** — `output_call_ref` includes ALL
functions with matching parameter/return types, not just lambdas that could
actually be stored in the fn-ref.  For `fn(integer) -> integer`, the match
includes `abs()`, `len()`, and every other `integer -> integer` function.
Non-closure candidates have no `__closure` parameter, so the emitted
`var_f.1` argument causes "cannot find value `var_`" compile errors.

**Root cause:** The dispatch is a static match on d_nr.  Without a closed
set of possible values, it must conservatively include all type-compatible
functions.  Same-scope fn-refs work because `closure_var_of` returns a valid
closure variable name for the specific `has_closure` arms.

**Fix approach:** When emitting the `__closure` argument in the cross-scope
path, only emit `var_{fn_ref_name}.1` for candidates that `has_closure`.
For non-closure candidates in the same match, omit the closure argument
(they don't need one).  This is already handled — each arm checks
`*has_closure` independently.

The real problem: the `has_closure` arm IS emitting for `abs()` etc., but
`abs` doesn't have `has_closure = true`, so it emits without the closure
arg.  That's correct.  The compile error `var_??` suggests the fn-ref
variable's name is empty — a separate bug in how temporary fn-ref variables
are named.

**Investigation:** trace which CallRef variable has an empty name and fix
the temporary naming.

**Files:** `src/generation/emit.rs`, `src/generation/mod.rs`

### Step 1 — Fix temporary fn-ref variable naming

Find where temporary fn-ref variables (from chained calls like
`make_adder(5)(10)`) get created without a name.  Ensure all fn-ref
variables have a valid sanitized name.

### Step 2 — Test with named variables only

Test `add5 = make_adder(5); add5(10)` in native mode (avoids the
temporary naming issue).

### Step 3 — Enable cross-scope doc test

Add `make_adder` example back to `26-closures.loft`.  Run `make ci`.

**Effort:** Small–Medium
**Target:** 0.8.3

---

## C48 — Capturing closures with map/filter/reduce

**Problem:** The interpreter rejects capturing lambdas passed to `map`, `filter`,
`reduce` with "function reference must be a compile-time constant".

**Root cause:** `map`/`filter`/`reduce` are parsed by `parse_for_each_call` in
`src/parser/collections.rs`.  The callback argument is resolved as a static
`fn <name>` reference (d_nr known at parse time).  Lambda expressions produce
a `Type::Function` value via `emit_lambda_code`, but the collections parser
doesn't accept fn-ref variables or lambda expressions in the callback position.

**Fix approach:**

The interpreter's `map`/`filter`/`reduce` implementation (`src/parser/collections.rs`)
unrolls the callback into a for-loop internally.  For a fn-ref variable or lambda,
the unrolled loop should use `CallRef` instead of `Call`:

```loft
// map(v, |x| { x * factor }) desugars to:
result = vector<T>{};
for x in v {
    result += [CallRef(fn_ref_var, x)]
}
```

This requires:
1. Parser: detect when the callback argument is a fn-ref variable or lambda
   (not a static `fn <name>`)
2. Collections: emit `Value::CallRef` in the unrolled loop body instead of
   `Value::Call`
3. The fn-ref variable must be in scope during loop execution

**Alternative (simpler):** reject the error only when the callback is a
*non-function* type.  If the callback is a `Type::Function` variable, accept
it and emit CallRef.  This doesn't require changing the collections desugaring
— just the argument validation.

**Depends on:** C47 (for native codegen parity)
**Effort:** Medium
**Target:** 0.8.3

---

## C52 — Stdlib name clash: inconsistent behavior

**Problem:** User-defined names that collide with stdlib names behave
inconsistently:

| Collision | Current behavior |
|-----------|-----------------|
| `fn len(text)` | Silently ignored — stdlib wins, user fn is dead code |
| `fn println(text)` | Hard error: "Cannot redefine Function" |
| `struct File` | Hard error: "Redefined struct" |

The inconsistency arises because some stdlib functions are registered via
`#rust` annotations (native ops — hard error on redefine) while others use
method dispatch on specific types (type-specific overload resolution —
stdlib variant wins by being first in the lookup chain).

**Design — emit a warning, never silently shadow:**

1. **All collisions produce a warning** — never silently ignore the user's
   definition.  The message should be:
   `Warning: 'len' shadows a standard library function`

2. **User definition wins** — local definitions shadow stdlib, matching
   the convention of most languages (Python, JavaScript, Rust).  The user
   explicitly chose to define this name.

3. **Stdlib accessible via `std::name`** — add a virtual `std` source for
   the default library, so the user can write `std::len("hello")` to access
   the original.  This reuses the existing `source::name` import mechanism.

4. **No names are forbidden** — the user can redefine anything, including
   `assert`, `println`, `len`.  The warning is informational.

**Implementation steps:**

### Step 1 — Emit warning on stdlib name collision

In `src/parser/definitions.rs`, when `add_fn` or `add_def` encounters a name
that already exists in source 0 (the default stdlib source), emit:
```
Warning: 'name' shadows a standard library function/type
```
Instead of the hard error "Cannot redefine".

### Step 2 — Make user definition win

Change name resolution order: when a name exists in both the current source
and source 0, prefer the current source.  This is already the behavior for
type-dispatched methods; extend it to global functions.

### Step 3 — Register stdlib as `std` source

In `src/parser/mod.rs`, after loading `default/*.loft`, register source 0
with the name `std`.  Then `std::len`, `std::println`, `std::File` work
via the existing `source::name` resolution path.

### Step 4 — Tests

- `fn len(t: text) -> integer { 42 }` → warning + user fn called
- `std::len("hello")` → returns 5 (stdlib version)
- `struct File { x: integer }` → warning + user struct used
- `std::File` → accesses stdlib File

**Effort:** Medium
**Target:** 0.9.0 (not blocking 0.8.3/0.8.4)

---

## C53 — Match arms: library enums and bare variant names

**Problem:** Match arms cannot use library enum variants at all — neither
prefixed (`testlib::Ok`) nor bare (`Ok`).  The match arm parser at
`control.rs:396` reads one identifier then expects `{`, `=>`, or `|`.
It does not handle the `::` namespace separator.

**Investigation findings:**

1. `has_identifier()` at line 396 reads `testlib`, not `testlib::Ok`.
   The `::` is then unexpected → parse error.

2. Even if `::` were consumed, the discriminant lookup at line 497 uses
   `pattern_name` (which would be `testlib`, not `Ok`) in `attr_names`.

3. Bare variant names (`Ok` without prefix) fail during first pass because
   `def_nr("Ok")` returns `u32::MAX` (library variant not in local scope)
   and `children_of(e_nr)` may not find it if the enum children aren't
   indexed by name.

4. Same-file enums already work because their variants ARE in global scope.

**Three fixes needed:**

### Fix 1 — Handle `::` in match arm identifier (line 396)

After `has_identifier()`, check for `::`.  If present, read the second
identifier.  Use `data.source_nr(source, variant_name)` to resolve the
variant.  Track the resolved variant name separately from `pattern_name`
for the discriminant lookup at line 497.

```rust
let (resolved_name, variant_def_nr) = if self.lexer.has_token("::") {
    let source = self.data.get_source(&pattern_name);
    if let Some(vname) = self.lexer.has_identifier() {
        (vname.clone(), self.data.source_nr(source, &vname))
    } else { (pattern_name.clone(), u32::MAX) }
} else {
    (pattern_name.clone(), self.data.def_nr(&pattern_name))
};
```

### Fix 2 — Use `resolved_name` for discriminant lookup (line 497)

Replace `pattern_name` with `resolved_name` in
`self.data.def(e_nr).attr_names.get(&resolved_name)`.

### Fix 3 — Bare variant fallback via `children_of` (line 419)

When `def_nr` fails and `e_nr` is known, search the enum's children:
```rust
if variant_def_nr == u32::MAX && e_nr != u32::MAX {
    variant_def_nr = self.data.children_of(e_nr)
        .find(|&c| self.data.def(c).name == resolved_name)
        .unwrap_or(u32::MAX);
}
```

### Fix 4 — Update or-pattern `|` to also handle `::` and bare names

The `while self.lexer.has_token("|")` loop at line 511 reads additional
variant names.  It needs the same `::` and `children_of` resolution.

**Effort:** Medium (4 changes in `parse_match`, all in `control.rs`)
**Target:** 0.9.0

---

## AOT — Ahead-of-time compiled libraries called from interpreter

**Problem:** Library functions like `blend_pixel`, `wu_line`, `scanline_fill`
are compute-intensive.  The interpreter runs them ~10–50x slower than native.
Users want library code at native speed while the main script runs in the
interpreter (for rapid iteration and REPL use).

**Design: auto-compile library to shared library, load via dlopen:**

When the interpreter loads a library via `use graphics;`:
1. Check cache: `lib/graphics/.loft/graphics.so` exists and source hash matches
2. If stale: emit Rust via `output_native`, compile with `rustc --crate-type cdylib -O`
3. Load shared library via `extensions::load_one`
4. Library functions dispatch through native code, not bytecode

The interpreter still parses `.loft` source for types and scope analysis.
Only bytecode execution is replaced by the native version.

```
User script: interpreted bytecode
    ↓ calls blend_pixel(canvas, x, y, color)
Library fn:  native compiled (loaded via dlopen)
    ↓ returns
User script: continues interpreting
```

**Cache:** `lib/<name>/.loft/` stores `.so` + source hash + generated `.rs`.
Recompile only when hash changes (~1–3s rustc cost on first run).

**Steps (desktop — dlopen):**
1. `output_native_library(lib_source)` — emit only library functions as cdylib
2. Compile with `rustc --crate-type cdylib -O --extern loft=...`
3. Load via `extensions::load_one` — registers functions via C-ABI
4. Cache with source hash in `.loft/` directory

**WASM — shared-memory cross-module calls (Approach B):**

WASM cannot dlopen, but can achieve the same result: compile each library
to its own `.wasm` module, share `WebAssembly.Memory` between modules, and
call library functions directly — no data serialization needed.

```
main.wasm ──shared memory──► graphics.wasm
    │                             │
    │  blend_pixel(dbref, x, y)   │
    ├────► JS import bridge ─────►│  runs native WASM blend
    │◄──── JS import bridge ◄────┤
    │                             │
    Stores heap: shared           │
```

How it works:
1. Compile each library to a separate `.wasm` via `rustc --target wasm32`
2. All modules share one `WebAssembly.Memory` instance (requires COOP/COEP
   headers for `SharedArrayBuffer`)
3. The `Stores` heap lives in shared memory — both modules read/write the
   same byte array.  `DbRef` values (store_nr + rec + pos) work across
   module boundaries without copying.
4. JS glue auto-generated from `.loft` type info: scalar args pass directly;
   `DbRef` args pass as three integers; text args pass as `(ptr, len)` into
   shared memory.
5. Cache: `lib/<name>/.loft/<name>.wasm` + source hash, same as desktop.

**Why shared memory works for loft:** the `Stores` allocator is a flat byte
array addressed by `(store_nr, rec, pos)`.  When two WASM modules share the
same memory, a `DbRef` allocated by the main module is directly readable by
the library module — same bytes, same offsets.  No marshalling needed for
struct or vector arguments.

**Fallback:** if `SharedArrayBuffer` is unavailable (no COOP/COEP headers),
library functions stay interpreted in the main module (Approach A — single
WASM, works today).

**Cargo feature:** `native-libs` (includes `native-extensions` + rustc)
**Effort:** High
**Target:** 0.9.0 (desktop dlopen), 1.0+ (WASM shared memory)

---

## E — Library Ergonomics

Features motivated by the `server` and `game_client` library designs.
Full design rationale and before/after code examples: [SERVER_FEATURES.md](plans/37-server-features/README.md).

---

### C57  Route decorator syntax

**Problem:** route registration is separated from the handler by potentially
hundreds of lines.  With 30+ routes, finding which URL maps to which handler
requires scrolling between the handler and the `main()` registration block.
Keeping them in sync is error-prone.

```loft
// Today — URL and handler are far apart; easy to get out of sync:
fn handle_health(req: Request) -> Response { ... }
// ... 400 lines ...
fn main() {
    get(app, "/health", fn handle_health);   // must match spelling above
}
```

**Fix:** `@annotation` syntax before `fn` declarations synthesises registration
calls at compile time.

```loft
@get("/health")
fn handle_health(req: Request) -> Response { ... }

@post("/login")
fn handle_login(req: Request) -> Response { ... }

@ws("/ws/chat")
fn handle_chat(req: Request, ws: &WebSocket) { ... }

fn main() {
    app = new_app(srv);
    register_routes(app);   // generated: registers each annotated handler
    serve(app);
}
```

The `server` library defines `@get`, `@post`, `@ws`, etc. as annotations.  At
compile time a synthetic `register_routes(app: &App)` function is generated from
them.  The `app` reference is **not** captured at annotation time — it is passed
at call time, avoiding implicit global state.

**What annotations are NOT:** not runtime metadata, not general macros, not
Turing-complete.  They are fixed templates that apply only to `fn` definitions.

**Implementation:** new token `Token::At`; parser reads `@name(args)` before `fn`;
annotation declarations (`annotation get(pattern: text) expands ...`); synthesis
pass generates `register_routes`; two-pass compiler already supports this pattern.

**Effort:** H — new token, new definition form, annotation registry, synthesis pass.
**Target:** 1.1+

---

## Quick Reference

See [ROADMAP.md](ROADMAP.md) — items in implementation order, grouped by milestone.

---

## See also
- [ROADMAP.md](ROADMAP.md) — All items in implementation order, grouped by milestone
- [../../CHANGELOG.md](../../CHANGELOG.md) — Completed work history (all fixed bugs and shipped features)
- [PROBLEMS.md](PROBLEMS.md) — Known bugs and workarounds
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — Language design asymmetries and surprises
- [SLOTS.md](SLOTS.md) — Stack slot assignment (A6 detail)
- [PACKAGES.md](PACKAGES.md) — External library packaging design (A7 Phase 2)
- [../DEVELOPERS.md](../DEVELOPERS.md) — Feature proposal process, quality gates, scope rules, and backwards compatibility
- [THREADING.md](THREADING.md) — Parallel for-loop design (A1 detail)
- [LOGGER.md](LOGGER.md) — Logger design (A2 detail)
- [FORMATTER.md](FORMATTER.md) — Code formatter design (backlog item)
- [NATIVE.md](NATIVE.md) — Native Rust code generation: root cause analysis, step details, verification (Tier N detail)
- [PERFORMANCE.md](PERFORMANCE.md) — Benchmark results and implementation designs for O1–O7 (interpreter and native performance improvements)
- [WEB_IDE.md](lib_plans/62-web-ide/README.md) — Web IDE full design: architecture, JS API contract, per-milestone deliverables and tests, export ZIP layout (Tier W detail)
- [RELEASE.md](RELEASE.md) — 1.0 gate items, project structure changes, release artifacts checklist, post-1.0 versioning policy
