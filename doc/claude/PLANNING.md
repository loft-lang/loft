
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
- [L — Language Quality](#l--language-quality)
  - [T1 — Tuple types](#t1--tuple-types) *(1.1+)*
  - [CO1 — Coroutines](#co1--coroutines) *(1.1+)*
- [I — Interfaces](#i--interfaces) *(completed — I1–I8 + I9 stdlib; P136 loop bug open)*
- [A — Architecture](#a--architecture)
  - [A12 — Lazy work-variable initialization](#a12--lazy-work-variable-initialization) *(deferred to 1.1+)*
- [S, P70, C47–C53](PLANNING_STABILITY.md) · [N, O, AOT](PLANNING_NATIVE.md) · [H, R, W, E](PLANNING_WEB.md) — split by subject
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


---

The ticket bodies below are split by subject.  Shipped, withdrawn and superseded items, and the old milestone reasoning, are in [PLANNING-history.md](PLANNING-history.md).

- [PLANNING_STABILITY.md](PLANNING_STABILITY.md) — S-tier stability hardening; slot reuse (P70), closures (C47, C48), name clash (C52), match arms (C53).
- [PLANNING_NATIVE.md](PLANNING_NATIVE.md) — Tier N native codegen, O-tier performance, AOT libraries.
- [PLANNING_WEB.md](PLANNING_WEB.md) — H HTTP / JSON, R repository, W web IDE, E library ergonomics (C57); its first-run presentation design is [FIRST_CONTACT.md](FIRST_CONTACT.md).

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
pub interface Ordered   { operator compare(self: Self, other: Self) -> Ordering }
pub interface Equatable { }    // every type: `==` is structural (C134)
pub interface Addable   { operator plus(self: Self, other: Self) -> Self }
pub interface Printable { operator to_text(self: Self) -> text }
```

The full list and what meets each: [INTERFACES.md § Standard library interfaces](INTERFACES.md#standard-library-interfaces).

**Example:**

```loft
fn best_of<T: Ordered>(v: vector<T>) -> T {
    result = v[0];
    for item in v { if result < item { result = item; } }
    result
}

struct Score { value: integer }
operator compare(self: Score, other: Score) -> Ordering { self.value.compare(other.value) }

// Score satisfies Ordered automatically — no explicit declaration needed.
best = best_of([Score{value: 3}, Score{value: 7}, Score{value: 1}]);
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
- [FORMATTER.md](FORMATTER.md) — `loft fmt`, the parser-driven formatter
- [NATIVE.md](NATIVE.md) — Native Rust code generation: root cause analysis, step details, verification (Tier N detail)
- [PERFORMANCE.md](PERFORMANCE.md) — Benchmark results and implementation designs for O1–O7 (interpreter and native performance improvements)
- [WEB_IDE.md](lib_plans/62-web-ide/README.md) — Web IDE full design: architecture, JS API contract, per-milestone deliverables and tests, export ZIP layout (Tier W detail)
- [RELEASE.md](RELEASE.md) — 1.0 gate items, project structure changes, release artifacts checklist, post-1.0 versioning policy
