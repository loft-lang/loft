<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# loft-dap — design (@PLN63 LSP.3 DAP debug adapter)

> **Identity:** a design sub-doc of `@PLN63` (loft-lsp), the `loft-dap` row of the
> distribution table. Slug `loft-dap`. Companion to
> [EXTRACT.md](EXTRACT.md) (the LSP.2 refactor design).
> **Status:** BUILT — the D0–D6 spine is implemented in `src/bin/loft-dap.rs`
> (a separate binary linking the `loft` rlib, the loft-lsp shape), gated by the
> `tests/dap_transport.rs` DAP-framed harness (D0; 7 tests, D1–D6). The one
> engine-side change was `loft::rpc::DebugDriver` (`src/rpc.rs`) — a `pub` wrapper
> over the `pub(crate) handle` chokepoint that arms the in-process session exactly
> as `run_rpc` does; `run_rpc` was refactored onto it (behaviour-identical, the 11
> `tests/rpc.rs` still green). The debug ENGINE and its wire protocol it translates
> were already LANDED (@PLN16 A–F/M1–M5, `loft debug --rpc`,
> [16.P PROTOCOL.md](../../plans/16-debugger/PROTOCOL.md)); this doc is the DAP-side
> translation over them. The Neovim plugin auto-registers the adapter the moment the
> binary is on `$PATH` (`editors/nvim/lua/loft.lua:101`), so it now lights up.
>
> **One deviation from the capability list below:** `supportsHitConditionalBreakpoints`
> is NOT advertised — the engine has no hit-count, so advertising it would let a client
> set a hit condition the adapter silently ignores (a wrong picture). Plain
> `supportsConditionalBreakpoints` IS advertised (conditions pass straight through).

## Goal

Interactive interpreter-mode debugging of a `.loft` program in any DAP-aware editor
(VS Code, Neovim `nvim-dap`, JetBrains via LSP4IJ): set a source-line breakpoint,
launch, hit it, inspect locals, step in/over/out, continue, evaluate an expression,
edit a value — over the standard **Debug Adapter Protocol** (`Content-Length`-framed
JSON-RPC on stdio).

The substance is NOT new debug logic. Every capability already exists as a
`ReplSession` method behind one dispatch chokepoint (`rpc::handle`), serialised by
the @PLN16 NDJSON wire protocol that was *designed as the DAP shape on purpose*
(request/response + async `stopped`/`output`/`terminated` events). loft-dap is a
**pure translator**: DAP-JSON in → an RPC request line → `rpc::handle` → RPC
response/event lines → DAP-JSON out. It adds framing, the handshake, the DAP
envelope, and the `threads→stackTrace→scopes→variables` drill-down synthesis — and
no engine semantics whatsoever (the protocol invariant: no UI-only, no adapter-only
behaviour).

## Decision 1 — translate `rpc::handle`, don't rebuild the engine

loft-dap holds a `ReplSession` **in-process**, exactly as `run_rpc`
(`src/rpc.rs:69`) does, and drives it through the one dispatch chokepoint:

```rust
// src/rpc.rs:105
handle(session: &mut ReplSession, line: &str) -> (Vec<String>, bool)
//                            NDJSON request ─┘        │           └ disconnect?
//                                    NDJSON responses+events ─┘
```

**No child interpreter process and no port** (correcting the `launch` sketch in the
README surface table, which predates this decision): the debuggee runs *inside the
adapter*, the way `loft debug --rpc` already runs it. This removes an entire class of
failure (spawn, port binding, connect-back races) and makes the whole adapter a
synchronous string-to-string function that the D0 harness can drive deterministically.

The one code change on the engine side is visibility: `handle` is `pub(crate)` today,
and `src/bin/loft-dap.rs` is a *separate crate* linking the `loft` rlib (the loft-lsp
shape), so D1 widens the entry point to `pub` — either `handle` directly or a thin
`pub fn drive(session, line) -> (Vec<String>, bool)` wrapper. `ReplSession::new_with_libs`,
`run_rpc`, and `print_or_capture` are already `pub`; nothing else moves.

**Why this is safe to build on:** `tests/rpc.rs` already proves the full engine path
end-to-end (`launch → setBreakpoints → run → stopped → eval → continue → output →
terminated`) through this exact chokepoint. loft-dap inherits that coverage for free
— its own tests only need to prove the *translation*, not the debugger.

## Decision 2 — the DAP ⇆ RPC map is nearly 1:1

Most DAP requests are a same-named or trivially-renamed RPC request; the RPC events
are already the DAP events. The full map (RPC verbs from `rpc::handle`, `src/rpc.rs:113`;
DAP names from the spec):

| DAP request | RPC line fed to `handle` | Engine method reached | DAP reply / event |
|---|---|---|---|
| `initialize` | *(adapter-local, no RPC)* | — | capabilities response, **then** `initialized` event |
| `launch {program, stopOnEntry}` | `{"req":"launch","file":program}` (defer `run`) | `ReplSession::load_program` (`repl.rs:1451`) | response; run deferred to `configurationDone` |
| `setBreakpoints {source, breakpoints}` | `{"req":"setBreakpoints","file":path,"breakpoints":[{line,condition}]}` | `add_file_breakpoint_rich` (`repl.rs:1858`) via `set_breakpoints` (`rpc.rs:336`) | `{breakpoints:[{verified,line}]}` |
| `configurationDone` | `{"req":"run","entry":"main"}` (the deferred launch) | `eval_observe` (`repl.rs:1876`) | resume; a hit → `stopped`, else `terminated` |
| `threads` | *(adapter-local — synthesize)* | — | `{threads:[{id:1,name:"main"}]}` |
| `stackTrace {threadId}` | `{"req":"stackTrace"}` | `paused_frame`/`paused_line` (`repl.rs:1937`,`1944`) via `frame_field` (`rpc.rs:408`) | `{stackFrames:[{id,name,line,source}], totalFrames}` |
| `scopes {frameId}` | *(adapter-local — synthesize)* | — | `{scopes:[{name:"Locals",variablesReference:N}]}` |
| `variables {variablesReference}` | *(cached from the last `stopped`/`stackTrace` frame)* | — | `{variables:[{name,value,type}]}` |
| `continue {threadId}` | `{"req":"continue"}` | `debug_continue` (`repl.rs:2235`) | `{allThreadsContinued:true}`; then `stopped`/`terminated` |
| `next` / `stepIn` / `stepOut` | `{"req":"stepOver"\|"stepIn"\|"stepOut"}` | `debug_step(StepMode::…)` (`repl.rs:2158`, `debugger.rs:22`) | response; then `stopped{reason:"step"}` |
| `evaluate {expression, frameId, context}` | `{"req":"eval","expr":expression}` | `debug_eval_json` (`repl.rs:2261`) | `{result, type, variablesReference:0}` |
| `setVariable` / `setExpression` | `{"req":"setValue","target":name,"value":val}` | `debug_set` (`repl.rs:1957`) | `{value}` (from the refreshed frame) |
| `disconnect` / `terminate` | `{"req":"disconnect"}` | — (returns `disconnect=true`) | `terminated`; loop ends |
| `pause` | *(no RPC — the adapter raises `debugger::INTERRUPT` from its request-reader thread)* | the running op loop suspends at its next stop check | `pause` response, then `stopped{reason:"pause"}` |

The RPC → DAP **event** map is direct (`report`, `src/rpc.rs:365`): RPC `stopped`
(reason `breakpoint`/`step`/`watch`/`entry`) → DAP `stopped`; RPC `output`
(`category:"stdout"|"trace"`) → DAP `output`; RPC `terminated` → DAP `terminated`
(+ a synthesized `exited`). The debuggee's `print` is already captured off the
protocol channel (`print_or_capture`, `src/rpc.rs:36`) and streamed as `output`
events — it never corrupts the DAP stdio stream.

## Decision 3 — synthesize the DAP drill-down from a flat frame

DAP inspection is a four-level walk: `threads → stackTrace → scopes → variables`.
The RPC gives a single **flat** frame: `frame_field` (`src/rpc.rs:408`) emits
`{"function":…, "locals":[{"name","value"}], "line":…}` from `BreakHit`
(`src/debugger.rs:36`: `function: String`, `locals: Vec<(String,String)>`). loft-dap
manufactures the DAP tree from it, holding a small per-stop table:

- **threads** — one synthetic thread `{id:1, name:"main"}` (multi-worker `par`
  extends this to one per worker — [§ Multi-worker](README.md#multi-worker-support), a follow-up).
- **stackTrace** — the RPC returns the *current* frame only (`stackTrace` handler,
  `src/rpc.rs:320`, is `frame_field`), so v1 emits a **single** `StackFrame`
  `{id:1000, name: function, line, source:{path}}`. (True multi-frame — TR1.3's
  `vector<StackFrame>` built in `src/native.rs:2362` — is not on the RPC wire yet;
  surfacing it is the one engine-side follow-up, see Risks.)
- **scopes** — a fixed `Locals` scope `{name:"Locals", variablesReference: frameId+1}`.
- **variables** — the frame's `locals` array, each `{name, value, type:""}`. The RPC
  frame carries **name + value only** (no per-local type — `frame_field` omits it),
  and each `value` is a pre-rendered own-format string, so every variable is a **leaf**
  (`variablesReference: 0`). Expanding a struct/vector *child* needs an RPC that walks
  a value's fields; that is a follow-up (the flat frame can't page into a value today).

`variablesReference` handles are minted by loft-dap and invalidated on every resume
(`continue`/`step`/`terminated`) — a stale reference from a prior stop returns an
empty `variables` list, never a wrong frame.

## The chokepoints (code points loft-dap reads / writes)

| Concern | Code point | Gives |
|---|---|---|
| Engine dispatch (the one seam) | `rpc::handle` (`src/rpc.rs:105`), widened to `pub` in D1 | one RPC request line → response + event lines |
| In-process session | `ReplSession::new_with_libs` + `debug_stepping(true)` (`repl.rs:1924`) — mirror `run_rpc` (`rpc.rs:69`) | the live debuggee, no child process |
| DAP framing | reuse loft-lsp's `read_message` (`loft-lsp.rs:1127`) + `send` (`:1148`) — `Content-Length` byte-framed JSON | DAP messages in/out |
| RPC envelope in / out | `resp_ok`/`resp_err` (`rpc.rs:491`/`498`), `event` (`rpc.rs:501`) | the `{id,ok,…}` / `{event,…}` lines to parse |
| Breakpoint install + verify | `set_breakpoints` (`rpc.rs:336`) → `add_file_breakpoint_rich` (`repl.rs:1858`); `breakable_lines_in_file` (`repl.rs:1885`) gives `verified` | DAP `Breakpoint.verified` per line |
| Stop + frame render | `report` (`rpc.rs:365`) + `frame_field` (`rpc.rs:408`); `paused_frame`/`paused_line` | the `stopped` event + the flat frame to fan out |
| Program output | `print_or_capture` (`rpc.rs:36`, pub) + the capture drain in `report` | `output` events, off the protocol channel |
| Step / continue / eval / edit | `debug_step` (`repl.rs:2158`), `debug_continue` (`:2235`), `debug_eval_json` (`:2261`), `debug_set` (`:1957`) | the DAP step/continue/evaluate/setVariable bodies |
| JSON parse/serialise | `crate::json::parse` + `esc` (`rpc.rs:471`) — loft's own JSON | one JSON implementation across LSP + DAP + RPC |

## Envelope mechanics (the DAP-specific bookkeeping)

The RPC envelope is `{id, req, …}` / `{id, ok, …}` / `{event, …}`. DAP wraps every
message in `{seq, type, …}`. The translation:

```
DAP request   {seq:S, type:"request",  command:C, arguments:A}
   →  RPC      {"id":S, "req":<map(C)>, <map(A)>}                     (use the DAP seq as the RPC id)
RPC response  {"id":S, "ok":B, <body>}
   →  DAP      {seq:K, type:"response", request_seq:S, success:B, command:C, body:<body>}
RPC event     {"event":E, <body>}
   →  DAP      {seq:K, type:"event",    event:<map(E)>, body:<body>}
```

- **`seq` vs `request_seq`.** DAP requires every message (both directions) to carry a
  monotonic `seq`; a response echoes the request's `seq` as `request_seq`. loft-dap
  keeps **two counters**: it forwards the client's request `seq` as the RPC `id` (so
  the RPC response's `id` maps straight back to `request_seq`), and mints its own
  outgoing `seq` from an adapter-local counter (responses + spontaneous events share
  it). An event has no `request_seq`.
- **Handshake ordering (strict).** On `initialize`: send the capabilities *response*
  FIRST, THEN the `initialized` *event* — the client waits for `initialized` before
  sending `setBreakpoints`, and reversing them hangs the session. Capabilities
  (as built): `supportsConfigurationDoneRequest`, `supportsConditionalBreakpoints`,
  `supportsEvaluateForHovers`, `supportsTerminateRequest`, `supportsSetVariable`;
  **NOT** `supportsHitConditionalBreakpoints` (no engine hit-count — see the Status
  note) and **NOT** `supportsStepBack` (undo/redo exist in the engine but
  reverse-*execution* stepping is a follow-up, § Non-goals).
- **Deferred launch.** DAP's launch sequence is `initialize → (initialized) → launch →
  setBreakpoints… → configurationDone`. loft-dap answers `launch` by loading the
  program only (RPC `launch`, no `run`) and defers the actual start to
  `configurationDone` (RPC `run`) — so breakpoints set between the two are installed
  before the program moves. `stopOnEntry:true` turns the first resume into an
  immediate `stopped{reason:"entry"}` (the RPC `run`'s first pause) instead of running
  through.
- **`variablesReference` lifetime.** Minted per stop, invalidated on the next resume.
  A single flat `Locals` scope in v1; a `0` reference means "leaf, not expandable".

## Worked translations

**Set a conditional breakpoint** (DAP → RPC → DAP):

```jsonc
// DAP request (framed with Content-Length)
{ "seq": 7, "type": "request", "command": "setBreakpoints",
  "arguments": { "source": { "path": "/p/game.loft" },
                 "breakpoints": [ { "line": 42, "condition": "hp < 0" } ] } }
// → RPC line into handle()
{ "id": 7, "req": "setBreakpoints", "file": "/p/game.loft",
  "breakpoints": [ { "line": 42, "condition": "hp < 0" } ] }
// ← RPC response (set_breakpoints → breakable_lines_in_file verifies line 42 carries code)
{ "id": 7, "ok": true, "breakpoints": [ { "line": 42, "verified": true } ] }
// → DAP response
{ "seq": 31, "type": "response", "request_seq": 7, "success": true,
  "command": "setBreakpoints",
  "body": { "breakpoints": [ { "verified": true, "line": 42 } ] } }
```

**Hit the breakpoint** (spontaneous RPC event → DAP event; the flat frame is cached
for the drill-down):

```jsonc
// RPC event emitted by report() after continue/run
{ "event": "stopped", "reason": "breakpoint",
  "frame": { "function": "tick", "locals": [ { "name": "hp", "value": "-3" } ], "line": 42 } }
// → DAP event (frame cached under threadId 1 / frameId 1000)
{ "seq": 44, "type": "event", "event": "stopped",
  "body": { "reason": "breakpoint", "threadId": 1, "allThreadsStopped": true } }
```

**Walk the locals panel** (all four DAP levels; only `stackTrace` touches the RPC):

```jsonc
threads   → { "threads": [ { "id": 1, "name": "main" } ] }              // adapter-local
stackTrace→ RPC {"req":"stackTrace"} → frame_field →
            { "stackFrames": [ { "id": 1000, "name": "tick", "line": 42,
                                 "source": { "path": "/p/game.loft" } } ], "totalFrames": 1 }
scopes    → { "scopes": [ { "name": "Locals", "variablesReference": 1001,
                            "expensive": false } ] }                    // adapter-local
variables → { "variables": [ { "name": "hp", "value": "-3", "type": "" } ] } // from cache; leaf (ref 0)
```

**Evaluate + edit at the stop:**

```jsonc
{ "command": "evaluate", "arguments": { "expression": "hp + 10" } }
   → RPC {"req":"eval","expr":"hp + 10"} → {"id":_,"ok":true,"value":7}
   → DAP body { "result": "7", "variablesReference": 0 }
{ "command": "setVariable", "arguments": { "name": "hp", "value": "100" } }
   → RPC {"req":"setValue","target":"hp","value":"100"} → refreshed frame_field
   → DAP body { "value": "100" }
```

## Small, safe steps

Each step is independently landable behind a **scripted harness** gate (a DAP-framed
driver, never a live editor), mirroring the LSP.1 spine. Nothing forwards a request
until its translation is proven against `rpc::handle` in a test.

- **D0 — DAP protocol harness (the instrument, first).** A test driver that writes
  `Content-Length`-framed DAP JSON to `loft-dap`'s stdin and reads/asserts its framed
  stdout, spawning the real `CARGO_BIN_EXE_loft-dap` (mirror `tests/lsp_transport.rs`'s
  `Session`). *Gate:* it drives an `initialize` handshake **and can fail** — feed a
  wrong expected reply and watch the assert fire; a harness that can't fail proves
  nothing (the @PLN16 "prove the harness can fail" rule).
- **D1 — transport skeleton + handshake, no engine.** Widen `rpc::handle` (or add
  `pub fn drive`) to `pub`. `src/bin/loft-dap.rs`: reuse loft-lsp's `read_message`/`send`
  framing; the `seq`/`request_seq` two-counter bookkeeping and the DAP envelope
  (`type`, `command`, `request_seq`, `success`) live here. `initialize` → capabilities
  → `initialized` event → `configurationDone` → `disconnect`. *Gate:* the harness
  completes the handshake and a clean exit; a request/response round-trips with correct
  `request_seq`. Framing and envelope bugs die here, before any feature rides them.
- **D2 — launch + run + terminated + output.** `launch {program, stopOnEntry?}` → RPC
  `launch` (load only), then `configurationDone` → RPC `run`. Run-to-end → `terminated`
  (+ synthesized `exited`); `stopOnEntry` → `stopped{reason:"entry"}`. Drain the RPC
  `output` events (`report`) → DAP `output` events. *Gate:* launch a trivial program →
  `terminated`; with `stopOnEntry` → `stopped`; a `print` shows up as an `output`
  event. **First end-to-end slice — a real program under the adapter.**
- **D3 — breakpoints + stopped.** `setBreakpoints {source, breakpoints:[{line,
  condition?}]}` → RPC `setBreakpoints` → `{breakpoints:[{verified,line}]}`; a hit →
  DAP `stopped{reason:"breakpoint", threadId:1}`. Conditions pass straight through
  (the engine's driver-side resolve loop already evaluates them). *Gate:* set a line
  breakpoint, run, assert the stop fires at the right line; an unbreakable line comes
  back `verified:false`. **Breakpoint-stop is real value in every DAP editor — the
  shippable milestone.**
- **D4 — inspection drill-down (threads / stackTrace / scopes / variables).** Synthesize
  per Decision 3: one thread; `stackTrace` → the RPC single frame → one `StackFrame`;
  a fixed `Locals` scope; `variables` from the cached frame locals (leaf values).
  Mint/invalidate `variablesReference`s across stops. *Gate:* at a breakpoint, walk
  threads→stackTrace→scopes→variables and assert the locals-panel content
  (name+value), and that a stale reference after a resume returns empty.
- **D5 — stepping + continue.** `next`/`stepIn`/`stepOut` → RPC
  `stepOver`/`stepIn`/`stepOut` → `stopped{reason:"step"}`; `continue` → RPC `continue`
  (`{allThreadsContinued:true}` then the next stop / `terminated`). `pause` was
  unsupported in v1 (no async interrupt); it is BUILT since 2026-09-25 (§ pause). *Gate:* step over/in/out and assert each lands on the
  expected line; continue runs to the next breakpoint or to `terminated`.
- **D6 — evaluate + setVariable.** `evaluate {expression, frameId, context}` → RPC
  `eval` → `{result, type}` (identifier / field-access / call, per the RPC's evaluator);
  `setVariable`/`setExpression` → RPC `setValue` → the refreshed frame value.
  *Gate:* evaluate an expression at a breakpoint; set a variable and assert the change
  is reflected in the next `variables` read.

D0–D6 complete the loft-dap MVP (LSP.3). The Neovim launch config already present
(`editors/nvim/lua/loft.lua:101`, `dap.adapters.loft = { command = 'loft-dap' }`)
lights it up with zero further adapter code. Every step ships behind the three LSP.1
checks: a unit test (the RPC driver, already covered by `tests/rpc.rs`), a harness
test (the DAP side, D0), and a real-editor smoke.

## Refusals + v1 boundaries (never a wrong picture)

Each boundary below is now designed as a small-step spine in
[DAP_ADVANCED.md](DAP_ADVANCED.md) (grounded in probes on the `--rpc` path); until built,
each stays an honest capability bit or clean error.

- **`pause`** (async interrupt) — **BUILT 2026-09-25.**  Requests are read on their own thread,
  because while `continue` runs the main loop is inside the engine and reads nothing.  The
  reader raises `loft::debugger::INTERRUPT` and answers the `pause` at once (DAP's order: the
  response, then the `stopped` event); both debug loops (`State::debug_check` for the first run,
  `State::debug_step` for `continue`/steps) take it at their next stop check and suspend there,
  and the report names the stop `pause`.  The main loop clears an interrupt nothing took — a
  `pause` while already stopped, or after the end — so it never fires on the next run.  A
  stepping run installs the debugger even with no breakpoint, or a run with none could not be
  paused.  Gate: `dap_transport::pause_stops_a_running_program_and_continue_resumes_it`.
- **Reverse-execution stepping** (`stepBack`/`reverseContinue`) — **BUILT**: a bounded
  snapshot ring checkpoints each forward step, so `stepBack` / `reverseContinue` restore the
  prior state (heap + registers) byte-identically
  ([DAP_ADVANCED.md § RX](DAP_ADVANCED.md#rx--reverse-execution-the-large-one-engine-checkpointing--rpc--dap)).
  `supportsStepBack` is advertised. Interpreter-only; I/O is not reversed (heap-only restore);
  depth is `LOFT_REVERSE_DEPTH` (default 200).
- **Structured variable expansion** — **BUILT** (no longer a leaf-only refusal): a struct
  or nested value expands into its children via `debug_eval_json`, a node is a leaf iff its
  JSON value is a scalar ([DAP_ADVANCED.md § VE](DAP_ADVANCED.md#ve--structured-variable-expansion-adapter-only--built)).
  Remaining leaf-only edge: a **bare top-level heap-vector** local shows under its `__vdb`
  backing (a `frame_field` fidelity limit, not VE).
- **Multi-frame stack** — **BUILT** (no longer single-frame): `stackTrace` returns the full
  runtime call stack, innermost first, each frame at its parked / call-site line, and
  `scopes {frameId}` reads any frame's locals
  ([DAP_ADVANCED.md § SF](DAP_ADVANCED.md#sf--multi-frame-stack-trace-engine--rpc--dap--built)).
  Caller-frame locals are leaves (eval is top-frame-scoped).
- **Data breakpoints** (break on a variable's change) — **BUILT**: `supportsDataBreakpoints`
  is advertised; a scalar local (or a nested struct-field / vector-element) can be watched and
  the run stops on its change
  ([DAP_ADVANCED.md § DB](DAP_ADVANCED.md#db--data-breakpoints-via-watchpoints-engine--rpc--dap--built)).
  Set at a stop only; caller-frame locals aren't watchable.

Each boundary is an honest capability bit or a clean error — never a silent, wrong
response.

## Risks + open questions

| Risk / question | Handling |
|---|---|
| `rpc::handle` is `pub(crate)`; `loft-dap` is a separate crate | D1 widens it to `pub` (or a `pub fn drive` wrapper); the only engine-side change |
| RPC `stackTrace` returns a single frame, not the call stack | v1 shows one `StackFrame`; surfacing TR1.3's `vector<StackFrame>` (`native.rs:2362`) on the RPC wire is the one engine follow-up for real multi-frame |
| RPC frame locals carry no per-local `type` | DAP `variables.type` is `""` in v1 (optional in DAP); add a `type` to `frame_field` if editors want it |
| No async `pause` in the engine | CLOSED 2026-09-25 — `debugger::INTERRUPT`, taken at the stop checks (see § pause) |
| DAP handshake ordering (`initialize` response before `initialized` event) | Fixed in D1's envelope layer; a harness assertion pins the order |
| `seq`/`request_seq` bookkeeping | Two counters: forward the request `seq` as the RPC `id`; mint outgoing `seq` locally |
| Multi-worker (`par`) threads | One synthetic thread in v1; one-per-worker is a follow-up over the same translation (§ Multi-worker) |
| Value expansion / paging | Leaf-only v1; a value-walking RPC + `variablesReference` tree is a follow-up |

## See also

- [README.md § LSP.3](README.md#lsp3--loft-dap-debug-adapter-090) — the DAP surface
  table + the D0–D6 spine this doc expands; also the IDE-plugin wiring.
- [EXTRACT.md](EXTRACT.md) — the sibling LSP.2 design (same design-first discipline).
- [16.P PROTOCOL.md](../../plans/16-debugger/PROTOCOL.md) — the RPC request/event
  contract loft-dap translates (the single serialisation of the debug engine).
- `src/rpc.rs` — `handle` (the chokepoint), `run_rpc` (the in-process driver to
  mirror), `report`/`frame_field` (the stop + frame rendering).
- [@PLN16](../../plans/16-debugger/README.md) — the debug engine (A–F, M1–M5) whose
  capabilities loft-dap forwards; TR1.3 `stack_trace` is the multi-frame follow-up.
- `editors/nvim/lua/loft.lua` — the adapter registration already waiting on the binary.
