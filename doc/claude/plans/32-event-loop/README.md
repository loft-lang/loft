<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN32 — EVENT_LOOP — Prioritised event-loop abstraction (client + server)

**Status:** Design spec.  Not yet implemented.  Depends on
[@P213 v4](../../PROBLEMS.md#213-typefunction-storage-layout-limit--full-design-for-the-proper-fix)
(capturing closures in struct fields).

This document is the concrete design.  Open questions, alternatives
considered, and design history live in
[EVENT_LOOP_DISCUSSION.md](DISCUSSION.md).

---

## Canonical status

**EventLoop is the canonical client/server dispatch model for
loft going forward.**  Earlier paper designs that proposed a
parallel mechanism are superseded:

| Earlier design | Status | Replaced by |
|---|---|---|
| `Dispatcher` struct + `dispatch(env, &Dispatcher)` ([GAME_CLIENT_LIB.md § Dispatcher](../../lib_plans/64-game-client/README.md#dispatcher-in-loft--superseded)) | Superseded paper, never implemented | EventLoop bidirectional handlers |
| `run_game_loop(GameLoop, tick_fn)` ([WEB_SERVER_LIB.md § Server-side game loop](../../lib_plans/future/08-server/README.md)) | Superseded paper, never implemented | `el::run` with a programmer-supplied `poll_sources` callback (kernel-multiplexed source polling is recorded as future work in [EVENT_LOOP_DISCUSSION.md](DISCUSSION.md), not as a separate API) |
| `GameEnvelope { sender, recipient, sequence, timestamp, message: WsMessage }` + `MsgType` enum (`lib/game_protocol/src/game_protocol.loft`, 104 lines, used only by its own tests) | Superseded shipped paper — the structs compile but nothing depends on them | EventLoop wire frame: `[handler_id][priority][seq][flags][length][payload]` |

The shipped `lib/game_protocol` will be reshaped (or replaced) to
fit the EventLoop wire format when EventLoop implementation
begins.  Until then, programmers should treat EVENT_LOOP.md as
the forward-looking spec; nothing currently depends on the
`MsgType`/`GameEnvelope` types in production code.

The shipped foundations that EventLoop *does* build on (and that
remain canonical) are:

- `lib/server` — `tcp_listen / tcp_accept / ws_upgrade / ws_send / ws_recv`
- `OpFormatDatabase` — struct → JSON / loft-text serialisation
- `database.parse(text, tp, result_dbref)` — text → struct deserialisation
- `lib/web` — HTTP client (orthogonal to EventLoop)

---

## Design principle — transport transparency

**The implementation of a game must not be bothered by the way
its data is transported.**

A handler is a typed reaction to a typed message.  The programmer
writes:

```loft
let h = el::on(loop, fn(input: PlayerInput) {
    state.apply(input);
});

el::send(loop, h, PlayerInput::Move(5, 3));
```

That code is identical for:

- a single-player game where `submit` and `recv` happen in the
  same process;
- a two-player networked game where `send` crosses a WebSocket;
- a 100-player server where the same handler runs on every
  connection;
- a hot-reload session where the same handler now also receives
  events from a file watcher.

The library — not the programmer — decides:

- **Stable handler name.**  Derived from the type name of `R`
  (e.g. `PlayerInput`).  Programmers do not pick string names
  unless they want to override.
- **Encoding.**  Defaults to JSON for plain structs and enums
  via `OpFormatDatabase` + `database.parse`.  Auto-detects
  `Raw` for types whose payload is a `bytes` field.  Programmer
  may opt into `Encoding::Raw` or `Encoding::Binary` for
  performance reasons; the API works without them.
- **Wire framing.**  Header bytes (handler-id, priority, seq,
  flags, length) are library-internal.  The programmer never
  reads or writes them.
- **Multi-frame reassembly.**  The handler always receives a
  fully-decoded typed value; partial frames are invisible.
- **Local vs networked dispatch.**  `submit(loop, h, msg)` for
  in-process delivery and `send(loop, h, msg)` for the same
  message over the wire are interchangeable from the handler's
  perspective.  A handler that only ever runs locally never
  encodes or decodes anything.

What the programmer DOES choose, deliberately:

- The shape of the typed message (struct or enum).
- The handler's behaviour (the closure body).
- (Separately, in the tuning phase:) the priority lane.

Anything else — names, encodings, framing, headers — is
transport, and transport is the library's job.

This principle is the test for every API choice in this document:
**if a parameter forces the programmer to think about how the
bytes get from A to B, it is wrong by default and must justify
itself.**  The current API treats `Encoding::Raw` and explicit
handler names as advanced overrides, not part of the basic flow.

---

## Design principle — interpreter as baseline

The EventLoop is **not** a "WASM construct" or a piece that comes
alive only after a build step.  It runs on the loft interpreter
from day one, with no WASM compilation required, no toolchain
present at runtime, and no server contact required for the
client to function.  Native WASM is an optimisation layered on
top; the interpreter is the floor.

This was the original motivation for building loft as a
tree-walking interpreter and a native-code generator
simultaneously: a loft client always has running code.  No build
step, no compile wait, no "loading native module…" moment, no
required server.  When the server ships compiled WASM, the
client runs faster.  When the server doesn't — cold start,
network blip, dev environment without a toolchain, hot-edit
mid-frame, or simply no internet connection — the client keeps
running on the interpreter.

For the EventLoop specifically, this principle has three
direct architectural consequences:

### 1. Frame-boundary hand-off uses the existing pump cycle

The EventLoop's `step()` (or the `pump()` shape used in
`lib/web`'s WebSocket handler) sits between frames — at the
moment no opcode is mid-execution and no I/O is in-flight.
This is precisely the consistency point that a runtime swap
(interpreter → WASM) requires.  The V4 hot-swap described in
[TIC_TAC_TOE.md § Tic-tac-toe v4](../39-tic-tac-toe/README.md#tic-tac-toe-v4--client-uploaded-scripts-server-side-compile-hot-wasm-swap)
does not need to invent a synchronisation point — the EventLoop
already has one.  At a frame boundary the runtime rebinds which
code reads the heap; the next pump call dispatches under the
new module.

### 2. Handler captures survive the swap

Handlers register at program start (`el::on(loop, fn(...) { ... })`)
and persist for the life of the EventLoop.  Their captured
state — the `Reference<T>` pointers into Store-allocated game
state — lives at addresses whose layout is **byte-identical
between the interpreter and `--native-wasm` output**.  Same
`Stores` heap, same `DbRef` shape, same struct / vector / hash
internals.  The swap rebinds the runtime; it does not migrate
state, re-register handlers, or convert any value.

This is a **load-bearing invariant**: if any future codegen
optimisation introduces divergence between interpreter and
WASM data layouts, the hand-off breaks.  The shared layout is
not a coincidence — it is an architectural asset that any
future change to either codegen path must preserve.

### 3. Connection autonomy

When the server is unreachable, `lib/web`'s auto-reconnect
backoff (capped at 10 s) spins in the Rust transport layer;
`send()` returns false; received-message events stop arriving.
The EventLoop **keeps pumping**: OS-source events (keyboard,
mouse, file watch, timers) still fire, and any locally-injected
events (`submit(...)` calls from the loft program) still flow
through priority lanes to their handlers.  The dispatch fabric
is live on the interpreter regardless of network state.  When
the server returns, `send()` succeeds, the inbound pipe
re-fills, and gameplay resumes from wherever it was paused —
no special handling on the loft side.

This is what makes the auto-reconnect design *invisible* to
the EventLoop's user.  Without the interpreter floor, a server
outage would either freeze the client (waiting for I/O) or
require explicit reconnection logic in the loft program.  With
the floor, neither: the program keeps stepping.

### 4. Handler registration: stable names with swap-aware mode

The "handler captures survive the swap" property above only
holds if the *handler-id-to-handler mapping itself* survives
intact.  That requires registration to key on **stable names**
(derived from the recv-type's canonical name plus an optional
instance suffix) rather than on call order, AND to distinguish
two operating modes:

- **Normal mode** (program startup, steady-state):  duplicate
  registration of the same name is a hard programmer error.
- **Swap mode** (the runtime invoking the new module's
  registration code during a hot-swap): re-registering an
  existing name replaces the closure body in place and returns
  the same id.

The mode flag is **library-internal**.  The programmer's source
is identical in both modes — the same `el::on(loop, fn(e: T) {
body })` call means "create or fail" during normal startup and
"create or replace" during a swap.  The runtime sets the flag
for the duration of the new module's registration phase, then
clears it.

#### Three-tier registration

Three API entry points trade ceremony for control.  Each tier
opts in to one more level of explicit naming; the programmer
pays only what their case requires.

| Tier | Function | Handler name in table | When to use |
|---|---|---|---|
| 1 | `on(loop, recv)` | fully-qualified type name (`lib_world::WorldChunk`) | The default — one handler per type, name auto-derived. |
| 2 | `on_at(loop, instance, recv)` | type name + `/` + instance (`lib_timer::Timer/game`) | Two or more handlers for the same recv-type; the instance string disambiguates. |
| 3 | `on_with(loop, opts, recv)` | fully explicit (`HandlerOptions { name }`) | Override the auto-derived name entirely (legacy compatibility, non-loft peer protocols, etc.). |

Tier 2 (`on_at`) is the middle ground that makes the
*"library used twice for slightly different roles"* case cheap
to express — register a Timer for game ticks at instance
`"game"`, and another for animation at instance `"animation"`,
without inventing distinct types or going to a full name
override.  See § Why three tiers below.

#### Normal-mode rule: one handler per name, duplicates rejected

A second registration of the same handler name during normal
operation produces a fatal diagnostic naming both sites and
suggesting the right fix:

```
error: handler for type 'lib_timer::Timer' is already registered
   --> game/main.loft:42:15
    |
 42 |     el::on(loop, fn(t: Timer) { animation_tick(t) });
    |               ^^ duplicate registration
   --> game/main.loft:18:15
    |
 18 |     el::on(loop, fn(t: Timer) { game_tick(t) });
    |               ^^ first registered here
   hint: if these are two distinct roles for the same type,
         use `el::on_at(loop, "<role>", fn(...) { ... })` to
         give each handler its own instance suffix:
             el::on_at(loop, "game",      fn(t: Timer) { game_tick(t) });
             el::on_at(loop, "animation", fn(t: Timer) { animation_tick(t) });
   hint: if you only meant one Timer handler, merge the
         bodies into a single registration.
```

The two hints map to the two real situations a junior
programmer can hit: copy-paste mistake (merge), or genuinely
two roles for one type (use `on_at`).  The diagnostic steers
toward the right fix without forcing them to read the spec.

This catches copy-paste mistakes, refactor leftovers, and
accidental double-imports at the source site rather than as
silent runtime confusion.

#### Swap-mode rule: re-registration replaces body in place

When the runtime is applying a hot-swap, the EventLoop's swap
flag is set.  In this mode:

- Re-registration of an existing name **does not error**.  It
  replaces the existing handler's `recv` closure with the new
  one and returns the **same** `HandlerId`.  The numeric id,
  the wire-level routing, and any in-flight events on priority
  queues all keep working untouched.
- A new name (not in the table before) is allocated a fresh id
  exactly as in normal mode.  The new handler ships into the
  table without disturbing existing ids.
- A handler from the previous code that is **not re-registered**
  in the new code keeps its old body.  Its id remains valid;
  events of that type still dispatch to the old closure (whose
  captures are still valid because the shared-layout invariant
  preserves them).  V4 ships this "stale handlers stay"
  semantics; V5+ may tighten to "remove on swap" once
  orphaned-event handling is designed.

#### Why three tiers

The two-handler-per-type case is real and common — timers,
loggers, telemetry sinks, anything that's fundamentally one type
used in multiple roles.  Three options for handling it, and the
spec picks the third:

- **Force distinct types per role** (define `GameTimer`,
  `AnimationTimer` as wrappers): pure, but costs a type
  declaration per role.  Often the right fix at the design
  level, but heavy ceremony for transient development needs.
- **Force `on_with(name, ...)` with full explicit naming**:
  works, but the programmer now manages the whole name string,
  including the type-prefix conventions, for what should be a
  small disambiguation.
- **Provide `on_at(instance, ...)`** as the middle tier: keep
  the auto-derived FQ-type prefix, add a programmer-supplied
  instance suffix.  One extra string, only when needed; the
  default tier-1 path stays untouched.

The instance string is the programmer's only added concept; the
type system stays out of it.  Both ends of an on-the-wire
handler can use `on_at` with matching instance strings to keep
client/server symmetry; for purely local (`submit`-only)
handlers, the instance is just a name-table disambiguator.

#### Why this scheme

- **The programmer writes no swap-aware code.**  No `if swapping
  { ... }`, no `el::on_replace`, no flag to remember to set.
  The same source runs in both modes.
- **Duplicate detection at the source site catches real bugs.**
  Two `on(...)` calls for the same recv-type during startup is
  almost always a mistake; the diagnostic catches it
  immediately and steers to `on_at` if the duplication was
  deliberate.
- **Wire format and server state stay coherent across swaps.**
  Stable name-to-id mapping means in-flight events, wire-level
  frames, and the server's handshake-established id map all
  keep working without renegotiation.  A same-shape edit
  (changed body, same handlers) needs zero server-side
  coordination.
- **Stale handlers are safe to leave.**  Captured
  `Reference<T>` survives because the heap layout is invariant
  across runtimes; events of stale types only arrive if the
  wire is still labelled with their id, in which case dispatch
  to the old body remains correct.

### Invariant to protect

The byte-identical layout between interpreter and `--native-wasm`
output deserves CI protection: a fixed test program, allocated
under both code paths, with a heap-image comparison that fails
on any divergence.  This belongs in the loft test suite once
the V4 hot-swap implementation begins; recording it here so the
dependency is visible from the EventLoop spec, since the
EventLoop is the consumer that depends on it most directly.

---

## Design principle — four-target async portability

The same loft client code must run unchanged across every loft
deployment target.  The EventLoop is the abstraction that hides
the per-target differences in *how the program waits for an
event* — without it, every game / multiplayer / TTT-style client
has to be ported four ways.

### The four targets

| Target | Entry point | Blocking semantics | Yield mechanism |
|---|---|---|---|
| **Interpreter** (`loft program.loft`) | `cargo run --bin loft -- program.loft` | OS thread blocks on syscalls (`thread::sleep`, `recv`); other threads run | OS scheduler |
| **Native** (`loft --native program.loft`) | compiled-to-Rust binary | same as interpreter | OS scheduler |
| **WASM-direct** (`loft --html program.loft`, @PLAN31) | per-program WASM with embedded interpreter-free generated code | nothing blocks; asyncify yields at instrumented points | `wasm-opt --asyncify --pass-arg=asyncify-imports@<fn>` instruments named host imports; the JS shell's `requestAnimationFrame` resumes |
| **WASM-interpreter** (`compile_and_run` + browser, TTT v3.5) | `wasm-pack` build of the loft interpreter; runs loft source delivered by HTTP | nothing blocks; the only yield is *return-from-WASM-call* | `compile_and_start` + `resume_frame` re-enter from JS event loop |

The differences cluster around one question: **what does the
loft program do between events?**  In targets 1+2 it can simply
block.  In targets 3+4 it must explicitly hand control back to
the JS event loop so messages, paints, and timers can be
delivered.

### The unifying primitive — `yield_to_host()`

A single new builtin, declared in `default/`:

```loft
// Surrender the current execution slice back to the host's event
// loop.  In interp / native this is a hint to the OS scheduler
// (returns immediately).  In WASM targets this returns from the
// current WASM call so the JS event loop can dispatch pending
// I/O / DOM / timer events; the host re-enters via
// `resume_frame()` (WASM-interp) or the asyncify resume path
// (WASM-direct).
pub fn yield_to_host();
#impure(host_io)
```

Per-target implementation:

| Target | Impl |
|---|---|
| Interpreter | `n_yield_to_host` in `src/native.rs` calls `std::thread::yield_now()` (no-op semantically; gives other threads a turn) |
| Native | same (generated code emits `std::thread::yield_now()`) |
| WASM-direct | `loft_yield_to_host` is asyncify-instrumented (`--pass-arg=asyncify-imports@loft_yield_to_host`); the JS shell's resume loop calls `resume()` from `requestAnimationFrame` |
| WASM-interpreter | `n_yield_to_host` (wasm-feature gate) sets a thread-local "yield requested" flag; the interpreter's main loop checks the flag at the next safepoint and returns from `compile_and_start` / `resume_frame`; JS calls `resume_frame` again on the next `requestAnimationFrame` |

This is the **only** new primitive needed.  Every higher-level
abstraction (`pump`, `for ev in events`, `await frame`) is built
on top.

### The portable API — events as a coroutine stream

Loft already has stackful coroutines (shipped 0.8.3, lowered to
a state machine on native).  The EventLoop's user-facing API is
an `iterator<Event>`:

```loft
fn handler.events(self: Handler) -> iterator<Event> {
  while !self.closed {
    while !ws_recv_native(self.id) {
      yield_to_host();
    }
    yield decode_event(ws_message_native());
  }
}

// The portable client loop — same code in all 4 targets:
fn play(h: Handler) {
  for ev in h.events() {
    match ev {
      Place { mark, r, c } => render_cell(r, c, mark),
      GameOver { winner }  => { show(winner); break; }
    }
  }
}
```

The `for ev in h.events()` loop reads identically on every
target.  In interp/native it busy-loops with cooperative
`yield_to_host`s (which are no-ops); in WASM targets each
`yield_to_host` returns control to JS, the event loop dispatches
the WS open / message events, and `resume_frame` re-enters
the iterator from where it yielded.

### Library surface — `lib/web` migrates to the abstraction

Today's `pub fn pump(self: WsHandler, on_message: fn(text)) ->
integer` becomes a thin wrapper over the iterator:

```loft
pub fn pump(self: WsHandler, on_message: fn(text)) -> integer {
  count = 0;
  for msg in self.messages() {     // messages() is an iterator<text>
    on_message(msg);
    count += 1;
  }
  count
}
```

`messages()` is the coroutine.  `pump` callers see no change —
existing v2/v3/v5 clients keep compiling.  The internal switch
from "while-poll-busy-loop" to "for-iterator-with-yields" is
invisible to consumers.

`web::sleep_ms` becomes a portable wrapper too:

```loft
pub fn sleep_ms(ms: integer) {
  // interp/native: native impl blocks the OS thread (today's behaviour).
  // WASM:          spin a yield-loop until the host's clock advances by ms.
  end = now() + ms;
  while now() < end { yield_to_host(); }
}
```

The current native `n_sleep_ms` becomes the interp/native impl;
the WASM impl is the spin-yield form.  Same loft surface in
all four.

### Sequencing (within @PLN32)

Phase boundaries — each is independent and can ship piecemeal:

| Phase | Step | Effort |
|---|---|---|
| YIELD.1 | Add `yield_to_host()` to `default/02_images.loft` (or a new `default/06_async.loft`); register `n_yield_to_host` in `src/native.rs` for interp + native (`std::thread::yield_now`) | XS |
| YIELD.2 | WASM-interpreter impl: thread-local `YIELD_REQUESTED` flag; interpreter loop checks it; `compile_and_start` returns when set; `resume_frame` clears + re-enters | M |
| YIELD.3 | WASM-direct impl: add `loft_yield_to_host` to the `loft --html` exports + the `wasm-opt --pass-arg=asyncify-imports@…` list | S |
| YIELD.4 | Migrate `lib/web::sleep_ms` to the portable spin-yield form (keeping the native blocking impl as the interp/native arm); migrate `pump` over the new iterator | S |
| YIELD.5 | EventLoop's `handler.events()` iterator + the demo loop showcased in this section's example | M |
| YIELD.T | Cross-target test matrix — same loft client runs through all 4 targets and asserts the same observable trajectory | M |

YIELD.1-3 ship the primitive; YIELD.4-5 build the abstraction on
top.  Each YIELD.* phase can land independently; the v5/v3/v2
tests guard against regressions on interp/native through every
step.

### Cross-target test strategy

A single loft client (`tests/scripts/cross_target_async.loft`)
plus four runners that drive it through each entry point:

| Runner | What it does |
|---|---|
| `tests/multiplayer_async_interp.rs` | Spawns the script under `loft --interpret`; asserts trajectory |
| `tests/multiplayer_async_native.rs` | Same script under `loft --native`; same assertion |
| `tests/multiplayer_async_wasm_html.rs` | `loft --html` produces standalone HTML; headless harness asserts trajectory via stdout-equivalent |
| `tests/multiplayer_async_wasm_interp.rs` | `compile_and_run` (or `compile_and_start` once YIELD.2 ships) under Node + the host shim; asserts trajectory |

All four use the **same loft source** — the test pin is
"this client runs identically through every target."  Adding a
new target later (e.g., a server-WASM rewrite) gets a fifth
runner and the same assertion.

### Why this lives in @PLN32

The EventLoop already declares "interpreter as baseline" as a
design principle — the cross-target async story is the natural
generalisation: every target sees the same loft surface, the
runtime hides the differences.  The `yield_to_host` primitive is
EventLoop's lowest-level building block; the iterator-of-events
abstraction is its user-facing API.  Splitting these into a
separate plan would require duplicating most of @PLN32's
"transport transparency" + "interpreter as baseline" sections.

This section informs the YIELD.* sub-arc; the rest of @PLN32
(handler registry, message envelope, transport-agnostic protocol)
stays as designed.

---

## Context

Both the loft game client and the multiplayer server want a main
loop that reads events from heterogeneous sources and processes
them in priority order:

- **Client (game):** keyboard / mouse / controller input must
  respond every frame.  World tick (physics, collisions) must
  finish before render.  Ambient effects (particles, distant AI,
  decorative animation) can drop frames if budget is tight.
- **Server (multiplayer):** player network messages must be
  drained and acked promptly.  World tick (rules step, state
  broadcast) runs at a fixed rate.  Background work (logging,
  analytics, NPC pathfinding for off-screen entities, long-running
  construction simulation) can run when there's spare budget.

The user (concrete consumer) wants a single abstraction whose shape
serves both — game and server share the "high-priority real player
actions vs lower-priority ambient work" pattern.  This document
specifies that abstraction.

@P213 (capturing closures in struct fields) is the underlying
language change that makes this abstraction comfortable to express;
it's a prerequisite, not part of this plan.

---

## Existing infrastructure

A second-pass survey (2026-05-05) confirmed substantial loft
infrastructure already ships for this design.  The EventLoop
**evolves** what's there; it doesn't propose a parallel system.

### What's shipped today

- **Server / WebSocket layer** (`lib/server/src/server.loft`):
  `Server { handle: integer }`, `WebSocket { ws_id: integer }`,
  `tcp_listen / tcp_accept / tcp_respond / tcp_close`,
  `ws_upgrade / next / send / close`.  Blocking poll model;
  text-only payloads.  The opaque-integer handle pattern is
  exactly the "library-assigned id" idiom.

- **Game protocol envelope** (`lib/game_protocol/src/game_protocol.loft`):
  `MsgType` enum (StateFullSync / StateDelta / PlayerInput /
  Ping / Pong / Chat / Lobby* / Match* / Error), `WsMessage
  { msg_type, payload: text }`, `GameEnvelope { sender, recipient,
  sequence, timestamp, message }`.  The dispatcher's per-msg_type
  routing is exactly what this design wants; adding priority is
  "a per-msg_type priority table."

- **Server-side game-loop design** (`../08-server/README.md` § Gap 4):
  `run_game_loop(cfg, tick_fn)` fixed-timestep loop, `Dispatcher`
  struct + `dispatch(env, &Dispatcher)`.  Designed; not yet
  implemented.

- **Built-in struct ↔ text serialisation:**
  `OpFormatDatabase(pos, val, db_tp, format_byte)` for struct →
  JSON or struct → loft-native text; `database.parse(text, tp,
  result_dbref)` for text → struct.  This IS loft's "default
  serialising and deserialising of objects."  Round-trip JSON for
  any struct is one op-call away.

- **HTTP client** (`lib/web/src/web.loft`): GET/POST/PUT/DELETE.

- **Polled input** (`lib/graphics/src/graphics.loft`):
  `gl_key_pressed`, `gl_mouse_*`, `gl_poll_events`.

- **Frame timing:** `ticks()` (microseconds), `gl_swap_buffers`
  (vsync).  `sleep_until_us` is in the WEB_SERVER_LIB.md design,
  not yet shipped.

- **Parallelism**: `par(...)` for data-parallel workloads, coroutines
  (`iterator<T>` + `yield`).  Useful inside handlers but not for
  event multiplexing.

- **Game client design** (`GAME_CLIENT_LIB.md`): client-side
  fixed-timestep loop with interpolated render.  Designed.

### What's NOT shipped (real gaps the EventLoop fills)

| Gap | Impact |
|---|---|
| Async / kernel multiplexer (mio / epoll / kqueue) | Server can't scale past a few connections without it.  **Deferred** — blocking server scales to first-multiplayer-game scope. |
| File watching | Asset hot-reload requires polling.  **Deferred.** |
| Priority queues / lanes | The thing this design adds. |
| Per-direction priority on the wire | New byte in `GameEnvelope`. |
| Library-assigned numeric handler-ids tied to the wire | Existing pattern uses `MsgType` enum keys; design adds wire-level handler-id. |
| Typed-binary serialisation of structs | Workaround: bytes-in-struct via JSON envelope.  **Deferred** until benchmarks show JSON cost dominates. |

---

## Architecture

### Design principle — separate "what" from "how urgent"

The programmer registers handlers without specifying priority.
Their concern is "this event means: do X."  Priorities are a
runtime tuning concern, assigned per handler in a separate
configuration phase.

```
                  PROGRAMMER PHASE              TUNING PHASE
                  ─────────────────              ────────────
                  on(loop, name, encoding,       set_priority(loop, h, HIGH)
                     marker, recv)
                       ↓                              ↓
                  ┌─────────────────────────────────────────┐
                  │ EventLoop                               │
                  │  handlers: vector<Handler>              │
                  │  priorities: hash<id, Priority>         │
                  └─────────────────────────────────────────┘
                                   ↑
                              send(loop, h, msg) (over wire)
                              submit(loop, h, msg) (locally)
```

The game programmer writes "this is what happens when the player
clicks" without thinking about scheduling.  Later, when the game
runs and you observe that fluff particles cost too much frame
time, you change ONE config value — without touching the particle
handler.  Tuning lives next to performance data, not next to game
logic.

### One typed channel per handler — bidirectional by composition

Each handler carries one typed message.  The handler-id is shared
across the wire: when both sides register a handler for the same
type, traffic of that type can flow in either direction; the
library routes by type.

A request-response conversation is two handlers — one per
direction — composed by the programmer:

```
CLIENT SIDE                                    SERVER SIDE
───────────                                    ───────────
let h_chunk = el::on(loop,                     let h_request = el::on(loop,
    fn(chunk: WorldChunk) {                        fn(req: WorldRequest) {
        state.apply(chunk);                            let chunk = compute(req);
    });                                                el::send(loop, h_chunk, chunk);
                                                   });

el::send(loop, h_request,                      // h_chunk and h_request are
    WorldRequest::Chunk(123));                 // declared on both sides; the
                                               // library matches by type name.

  ───── wire frame ─────→
  [handler_id: WorldRequest] [priority=HIGH] [seq] [flags] [length] [JSON payload]

  ←───── wire frame ─────
  [handler_id: WorldChunk]   [priority=LOW]  [seq] [flags] [length] [JSON payload]
```

The programmer never writes `"world_update"`, never writes
`Encoding::Json`, never writes a send-marker.  The handler's
recv-type tells the library everything it needs to know.  See
**Design principle — transport transparency** above.

**Client-authoritative protocol enumeration.**  The design is
anchored from the client developer's perspective.  The client
program declares handlers; the library reflects on the recv-types
to derive names; the connection handshake communicates the (name
→ id) map to the server.  The server's library expects matching
handlers (registered by the server program under the same types);
the handshake fails fast if a name is missing on either side.
Server-side restrictions (auth, rate-limit, validation) are
middleware that wraps handlers — out of scope for v1, layered
when needed.

**Per-connection id-space.**  Each connected client may register
different handlers; the server tracks the (name → id) map
per-connection.  Frames from connection A use A's id-space.

### Closure ↔ event interaction — events stay small, state lives in closures

The event payload carries trigger data only.  The "big" state
(game world, accumulator tables, sound queue, network connections)
lives in the handler closure's captured environment.

**Captures are by value (loft's C38 design).**  Captured *values*
are frozen at capture time.  But a captured `Reference<T>` holds
a 12B DbRef pointer that's snapshotted; the record it points at
remains mutable through the pointer.

So:
- ✅ `state: Reference<GameState>` — handler can mutate
  `state.score`; the record is alive in the host scope.
- ❌ `count: integer` — snapshot inside the closure; mutations
  don't persist outside.
- ✅ Workaround for mutable scalars: stdlib `Mutable<T>` helper
  (allocates a 1-field struct in a Store; capture by Reference).

The EventLoop relies on this pattern.  **Anything mutable lives
in a Store and is captured by Reference**; anything captured is
just a pointer to live data.

**Single-threaded mutation safety.**  The event loop is
single-threaded at loft level.  Multiple handlers can all hold
`Reference<GameState>` and all mutate it without conflicting —
at any moment exactly one handler is running.  Sequential dispatch
makes the design safe without borrow-checking.

### Wire format and encoding — see EVENT_PROTOCOL.md

The bytes that flow on the WebSocket between two endpoints —
binary header layout, name→id MAP handshake, priority byte,
encoding modes (JSON / Raw / Binary), multi-frame streaming
reassembly, reserved prefixes, version negotiation — live in the
companion document
[EVENT_PROTOCOL.md](PROTOCOL.md).  This document focuses
on the application-layer concerns (registration, dispatch, captures,
priority lanes as a runtime concept); EVENT_PROTOCOL is what
implementers and wire-level debuggers read.

What the EventLoop spec relies on from the protocol layer:

- **A server-arbited handshake.**  The server sends a name→id MAP
  before any other frames; the client conforms.  The integer ids
  are stable for the connection's lifetime and survive hot-swaps
  (see § 4. Handler registration).
- **An integer handler id on every frame.**  Routing inside the
  EventLoop is by id (array index into the priority queues); the
  protocol layer guarantees the id makes it from sender to
  receiver.
- **Per-direction priority labelling.**  The sender labels each
  frame with its priority class; the receiver routes that frame
  to the matching priority lane by reading the wire byte.  No
  receiver-side lookup of "what priority does this handler
  default to" — the wire carries it.
- **Library-assembled streaming.**  Multi-frame messages
  reassemble below the EventLoop's surface; handlers always see a
  fully-decoded typed value.  The "wait" for chunks IS the async
  property — a LOW handler may wait arbitrarily long while
  HIGH-priority frames stream past it; it doesn't fire until its
  message is whole.

The EventLoop's API does not expose any of this.  `el::send` and
`el::submit` accept typed messages; handlers receive typed
messages; nothing in the API surface concerns frame bytes.

---

## API

### Core types

```loft
// Three priority lanes; numeric (0-255) under the hood for finer
// tuning.  HIGH/NORMAL/LOW are convenience constants.
enum Priority { HIGH, NORMAL, LOW }   // = 0, 128, 255

// Encoding mode declared per-handler.
enum Encoding {
    Json,     // round-trip via OpFormatDatabase + database.parse
    Binary,   // deferred for v1
    Raw,      // bytes-in-struct, library passes through
}

// HandlerId carries the type of messages on this handler.
// A handler is a single typed channel: messages of type R flow
// in EITHER direction (sent by either side, received on the
// other).  Bidirectional request-response conversations are
// modelled as two handlers (e.g. HandlerId<WorldRequest> for
// client → server, HandlerId<WorldChunk> for server → client).
struct HandlerId<R> { id: integer }

// Library-internal handler record (one per registered type).
// The `name` is derived from R's type name at registration;
// `encoding` is derived from R's shape (Json default; Raw if R
// is a bytes-payload wrapper); `priority` is set later in the
// tuning phase.
struct Handler {
    id:       integer,         // library-assigned, returned to user as HandlerId<R>
    name:     text,            // derived from R's canonical name by default
    encoding: Encoding,        // derived from R's shape by default
    priority: Priority,        // set by tuning phase; defaults to NORMAL
    recv:     fn(bytes) -> void,   // type-erased; wraps user's typed closure
}

// EventLoop holds all registered handlers, the priority routing
// table, and the priority-lane queues.  `name_table` and
// `in_swap` are library-internal — they support the registration
// rules in § Handler registration: stable names with swap-aware
// mode.  Programmers do not read or write them.
struct EventLoop {
    handlers:        vector<Handler>,
    name_table:      hash<text, integer>,   // recv-type name → handler id
    in_swap:         boolean,               // set by runtime during a hot-swap
    queue_high:      vector<EventEntry>,
    queue_normal:    vector<EventEntry>,
    queue_low:       vector<EventEntry>,
    frame_budget_us: integer,
    running:         boolean,
}

struct EventEntry { handler_id: integer, payload: bytes }
```

### Public API

The basic API takes the recv closure and nothing else.  The
library reflects on the closure's parameter type to derive the
handler name (the type's canonical user-visible name as the
compiler resolved it, e.g. `lib_world::WorldChunk` for a
type defined in `lib_world` — the actual API is whatever
accessor the loft compiler exposes for `Definition.name` when
this work lands) and the encoding (JSON for plain structs /
enums; Raw for bytes-payload wrappers).  For two handlers of the same recv-type
in different roles, `on_at` adds an instance suffix.  For full
overrides (matching legacy peer protocols, etc.), `on_with`
takes a `HandlerOptions` struct.  See § Handler registration:
stable names with swap-aware mode for the registration rules and
the duplicate-detection diagnostic.

```loft
// Build a loop with the given frame budget (microseconds).
pub fn new(frame_budget_us: integer) -> EventLoop;

// Bidirectional handler registration — tier 1, the default.
// Name = fully-qualified recv-type name; encoding derived from R.
// Normal mode: registering the same recv-type twice is a fatal
// error.  Swap mode: the second call replaces the closure body
// in place and returns the existing HandlerId.  See § Handler
// registration: stable names with swap-aware mode.
pub fn on<R>(loop: Reference<EventLoop>,
             recv: fn(R) -> void) -> HandlerId<R>;

// Bidirectional handler registration — tier 2, instance-suffixed.
// Name = fully-qualified recv-type name + "/" + instance.  Use
// when two or more handlers share a recv-type for distinct roles
// (e.g. two timers, two loggers).  Each (type, instance) pair is
// a distinct handler with its own id; duplicate-detection rules
// apply per-pair.
pub fn on_at<R>(loop:     Reference<EventLoop>,
                instance: text,
                recv:     fn(R) -> void) -> HandlerId<R>;

// Bidirectional handler registration — tier 3, full override.
// Use when even the auto-derived FQ-type prefix is wrong (legacy
// compatibility, non-loft peer protocol with fixed names, etc.).
pub fn on_with<R>(loop: Reference<EventLoop>,
                  opts: HandlerOptions,
                  recv: fn(R) -> void) -> HandlerId<R>;

struct HandlerOptions {
    name:     text,         // override the auto-derived name entirely
    encoding: Encoding,     // override the type-derived encoding
}

// Receive-only local handler (no remote peer; OS event sources
// like keyboard, file watch, timers).  Same shape as on().
pub fn on_local<R>(loop: Reference<EventLoop>,
                   recv: fn(R) -> void) -> HandlerId<R>;

// Send to the peer via this handler's channel.  Library encodes
// `msg`, tags with handler-id and priority, enqueues on outbound
// priority queue.  In single-player or local-only programs this
// becomes a no-op alias for submit().
pub fn send<R>(loop: Reference<EventLoop>,
               h:    HandlerId<R>,
               msg:  R);

// Locally inject a message (used by pollers — keyboard input →
// submit to h_input; testing harness; intra-program signalling).
// Identical to send() except the message never crosses the wire.
pub fn submit<R>(loop: Reference<EventLoop>,
                 h:    HandlerId<R>,
                 msg:  R);

// Tuning — assign priority for a handler's outbound traffic and
// inbound lane.  Can be called at startup or live.
pub fn set_priority<R>(loop:     Reference<EventLoop>,
                       h:        HandlerId<R>,
                       priority: Priority);

// Bulk load priorities from a config map (variant-name → priority).
pub fn load_priorities(loop:   Reference<EventLoop>,
                       config: hash<text, Priority>);

// Run one full frame: drain priority queues HIGH → NORMAL → LOW
// within frame_budget_us.
pub fn step(loop: Reference<EventLoop>);

// Convenience: run frame-driven (game client) until stop_loop()
// or platform exit.
pub fn run(loop:         Reference<EventLoop>,
           tick_rate:    integer,
           poll_sources: fn(Reference<EventLoop>) -> boolean);

pub fn stop_loop(loop: Reference<EventLoop>);
```

### Drain semantics

Each frame:

1. Caller's `poll_sources` drains platform raw sources and calls
   `submit(...)` / `send(...)` for each event.  Each submit reads
   the event's handler-id, looks up the priority, queues in the
   matching lane.
2. `step` walks lanes in priority order:
   - **HIGH lane:** all events fire, no budget cap.  Input and
     player network messages always process this frame.
   - **NORMAL lane:** events fire until `normal_share *
     remaining_budget` is exhausted.  Unprocessed events stay
     queued for next frame.
   - **LOW lane:** events fire until total frame budget is
     exhausted.  Unprocessed events stay queued.
3. Each event-fire decodes the bytes payload via the handler's
   `Encoding` and calls the handler's typed recv closure.
4. Multi-frame packets are reassembled in the library's
   per-(handler_id) accumulator before dispatching; the handler
   never sees partial payloads.
5. Next frame starts.  No event is lost; deferred lower-lane
   events accumulate during heavy frames, catch up on lighter ones.

---

## Implementation foundations

This abstraction depends on:

1. **@P213 v4 (capturing closures in struct fields)** — the
   `Handler.recv` field holds a fn-ref that captures the user's
   typed closure.  Today's diagnostic blocks this.  @P213 v4
   unblocks it.  See [PROBLEMS.md § 213](../../PROBLEMS.md#213-typefunction-storage-layout-limit--full-design-for-the-proper-fix).

2. **`OpFormatDatabase` + `database.parse`** — round-trip JSON.
   Already shipped.

3. **WebSocket primitives** — `ws_upgrade`, `ws_send`, `ws_recv`.
   Already shipped (text-only payloads today; needs binary-frame
   extension for `Encoding::Raw`).

4. **`GameEnvelope` evolution** — add a priority byte; either
   extend `MsgType` for new handlers or introduce a generic
   handler-id discriminator.

5. **Frame timing** — `ticks()` shipped; `sleep_until_us` from
   WEB_SERVER_LIB.md design needs to land.

6. **`Mutable<T>` stdlib helper** — wraps a `Reference<T>` for
   the mutable-scalar capture pattern.  ~30 lines of stdlib.

7. **Compiler hint for value-type-mutated-in-closure** — small
   parser diagnostic that points users at `Mutable<T>` or the
   wrap-in-struct workaround.  ~20 lines.

---

## Critical files to modify (when implementation starts)

| File | Change |
|---|---|
| `lib/event_loop/src/event_loop.loft` (new) | Core EventLoop library — types, on / send / submit / set_priority / step / run |
| `lib/event_loop/examples/01-keyboard-state.loft` (new) | Client example: capture state, handle input |
| `lib/event_loop/examples/02-server-tick.loft` (new) | Server example: run_game_loop wrapped in EventLoop |
| `lib/game_protocol/src/game_protocol.loft` (extend) | Add priority field to envelope; add handler-id discriminator |
| `lib/server/src/server.loft` (extend) | Native: accept binary WebSocket frames + handshake handler-id exchange |
| `lib/mutable/src/mutable.loft` (new) | `Mutable<T>` stdlib helper + `cell / get / set / modify` |
| `default/01_code.loft` (extend) | If new ops needed: `OpClaimChild` (per @P213 v4 design) |
| `src/native.rs` (extend) | Native side of `tcp_*` / `ws_*` for binary frames |
| `doc/claude/EVENT_LOOP.md` (this file) | Update as implementation reveals real friction |

---

## Verification

End-to-end checks once implemented:

1. **@P213 v4 regression tests** — see PROBLEMS.md § 213.  Must all
   pass before EventLoop work begins.

2. **EventLoop unit tests:**
   - Priority drain order: HIGH events fire before NORMAL before LOW.
   - Budget cap: NORMAL/LOW events deferred when frame budget exhausted.
   - Static handler registration: handler-id stable across frames.
   - JSON encode/decode round-trip via `Encoding::Json`.
   - Raw bytes pass-through via `Encoding::Raw`.

3. **Streaming tests:**
   - Multi-frame packet reassembly: handler sees only the
     complete message.
   - HIGH-priority events fire during in-progress LOW assembly.
   - Stream timeout: 30-second default fires
     `StreamCancelled` event when END never arrives.

4. **Bidirectional handler tests:**
   - `send` from client; matching `recv` fires on server (and
     vice versa).
   - Priority on the wire honoured: HIGH outbound from one side,
     LOW outbound from the other, both work simultaneously.

5. **Handshake tests:**
   - Client and server agree on handler-id mapping.
   - Mismatched handler-name handled gracefully (rejected /
     warned, not silently corrupting).

6. **End-to-end demo:**
   - Single-player game using EventLoop with capturing-closure
     handlers.
   - Two-player networked game using bidirectional handlers.

7. **CI gate:**
   - `cargo fmt --check`
   - `cargo clippy --release --all-targets -- -D warnings`
   - `cargo build --release --no-default-features`

---

## Sequencing

The dependency stack is long.  Phase 1-2 are the gate to a running
game; phases 3-5 are multiplayer infrastructure layered on top.

| Phase | Ships | Cost | Game capability |
|---|---|---|---|
| **1** | @P213 v4 + minimal closure-using single-player game | 3-4 sessions | First playable single-player demo |
| **2** | EventLoop core (priority lanes, handler-ids, local OS sources, Mutable<T>) | 2-3 sessions | Single-player game with structured event flow |
| **3** | Wire protocol + bidirectional handlers + handshake + JSON encoding | 2-3 sessions | Two-player networked game |
| **4** | AsyncPoller + mio + server scaling | 2 sessions | Server handles many clients |
| **5** | Streaming wire format (`Raw` + multi-frame reassembly) for binary blobs | 1-2 sessions | Large world / texture streaming |

### YIELD.* — four-target async portability (parallel sub-arc)

The "interpreter as baseline" + "transport transparency" design
principles generalise to **same loft code runs on every loft
deployment target** — see § Design principle — four-target async
portability above.  Independent of Phases 1-5; can ship in
parallel as soon as the v3.5 spike (`compile_and_start`-driven
WASM-interpreter clients) creates an immediate consumer.

| Phase | Ships | Cost | Unlocks |
|---|---|---|---|
| **YIELD.1** | `yield_to_host()` builtin + interp/native impl (`std::thread::yield_now`) | XS | The primitive — no behaviour change yet |
| **YIELD.2** | WASM-interpreter impl (thread-local YIELD_REQUESTED flag + `compile_and_start`/`resume_frame` integration) | M | TTT v3.5 browser client |
| **YIELD.3** | WASM-direct impl (asyncify-instrument `loft_yield_to_host` in `loft --html`) | S | Plan-31 graphics programs that also do WS / async I/O |
| **YIELD.4** | Migrate `lib/web::sleep_ms` + `pump` to the portable form | S | Existing v2/v3/v5 clients become 4-target portable for free |
| **YIELD.5** | EventLoop's `handler.events()` iterator + the demo loop | M | The user-facing portable API — single `for ev in h.events()` works everywhere |
| **YIELD.T** | Cross-target test matrix (one loft client × four cargo runners) | M | Regression guard: a future change can't silently break one target |

**Recommended order:** YIELD.1 → YIELD.2 → YIELD.4 → YIELD.5 →
YIELD.3 → YIELD.T.  YIELD.2 unblocks v3.5 (the immediate
consumer); YIELD.4 makes existing clients work everywhere
without source changes; YIELD.5 ships the iterator API; YIELD.3
generalises to graphics programs; YIELD.T locks the contract.

**Independence from Phases 1-5:** YIELD.* doesn't depend on
EventLoop's wire protocol, handler registry, or AsyncPoller.
The reverse is true: once Phases 2+ ship, they can adopt the
YIELD.* abstractions internally for cross-target portability.

### Priority lanes ≠ threads

Worth flagging since the question keeps coming up: **priority
lanes (Phase 2) don't need `parallel{}` workers** — they're a
scheduling concept, not a parallelism one.  Single-threaded loop,
non-blocking polls per lane, drain in priority order:

```loft
while running {
  while let ev = next_input()     { dispatch_high(ev); }   // HIGH
  if   now() >= next_tick         { do_tick(); next_tick += period; }   // MED
  if   let ev = next_low_event()  { dispatch_low(ev); }    // LOW
  yield_to_host();   // no-op on interp/native; cooperative on WASM
}
```

The bones are already in lib/server (`ws_accept_nonblocking`,
`ws_next_event_native`, `ws_idle_sleep_ms_native`) + stdlib
(`now()`, `ticks()`).  Phase 2 can ship on the interpreter
without YIELD.* — the loop just spins a few % CPU until YIELD.*
makes it cooperative.

`parallel{}` workers are for **CPU-bound batches** (physics over
10k entities, image decoding, world generation) — not for getting
multiple priority lanes.  A 30-client × 10 Hz audience-demo
server runs on one thread.

### Beyond YIELD.* — Python-parity gaps (second wave)

Three additions complete async/await-shape parity with
Python's `asyncio`, with the bonus that loft keeps its
no-function-coloring property throughout.  Each builds on
YIELD.1-5; cancellation is the keystone (the other two are
combinators on top).

| Phase | Adds | Python equivalent | Effort |
|---|---|---|---|
| **CANCEL.1** | `cancel(coro)` builtin → flips a per-coroutine flag the state-machine driver checks at every yield point; loft's existing scope-exit + dep-tracking handles cleanup automatically | `task.cancel()` + `CancelledError` propagation | S-M |
| **CONCUR.1** | `concurrent { a(); b(); c(); }` block — single-threaded interleaving of coroutines on the EventLoop; sibling cancel on first panic / first done | `async with asyncio.TaskGroup() as tg: …` | M |
| **TIMEOUT.1** | `select(a, b)` combinator + `timer(ms)` one-shot iterator + `timeout(ms, coro)` desugar; pure-loft on top of CANCEL + iterators | `await asyncio.wait_for(coro, 5)` | S |

Order: CANCEL.1 → CONCUR.1 → TIMEOUT.1.  TIMEOUT.1 is pure-loft
once CANCEL.1 lands.  No new runtime primitives beyond YIELD.*
— everything else is codegen + stdlib.

**Recommendation:** ship Phase 1 ASAP; design beyond Phase 2 only
as Phase 1's real friction reveals what's actually needed.  The
abstract design in this document and EVENT_LOOP_DISCUSSION.md is
a target; concrete implementation may diverge as friction
surfaces.

---

## Stack-overflow recovery via the async substrate (deferred)

**Status:** Design intent only.  No urgency.  Tracked here
because the user's framing (chat 2026-05-11) ties recovery to
the async/event-loop substrate rather than to the runtime-error
@PLN28 phase 4f.  Ship when the engine arrives — not before.

### The user constraint

> "I would only like to do something in combination with our
> full async structure.  It doesn't matter that much if it is
> harder to write because it will probably be library driven
> and thus not directly the user.  But there is no real hurry,
> just something I would like to have when we have a full-blown
> game engine with custom user scripts."

Two implications:

1. **Library-driven, not user-facing.**  The recovery boundary
   is set by the engine library, not by the user script.  User
   scripts can recurse however they want; if they overflow, the
   engine library catches and continues.  This reduces UX
   pressure (no `try`/`catch` syntax bikeshed) and lets the
   recovery mechanic look as ugly as it needs to under the hood.

2. **Async-substrate-aware.**  Recovery must not fight the
   `yield_to_host()` pump cycle, the per-handler coroutine
   driver, or the cancellation machinery (CANCEL.1).  The
   recovery primitive lives at the EventLoop layer where the
   pump already controls scheduling — not at `State::raise`'s
   layer where the dispatch loop has no notion of "frames" vs
   "scheduled coroutines."

### Why @PLN28's stack-overflow typed error (4f.12) is the prerequisite, not the solution

Plan-07 phase 4f slice 2 (commit `ad468876`) converted
`State::fn_call`'s recursion-depth panic into a typed
`RuntimeError::StackOverflow`.  The dispatch loop's
`runtime_error.is_some()` check halts execution and `main.rs`
renders.  This is the right shape for CLI tools and tests; it
is NOT a recovery mechanism.

The typed-error shape is a prerequisite for recovery because:

- Recovery code needs to inspect the kind (was it
  StackOverflow, OOB, divide-by-zero?) to decide what to log
  and whether to retry.
- The position + call chain (4g.1) lets the recovery handler
  log "the failed handler was X at Y:Z called from W:V."
- Production-mode log+continue already exists at the
  `State::raise` level — the EventLoop layer just needs to
  intercept BEFORE the runtime_error.is_some() check halts
  the dispatch loop.

### Concrete design (slice 1)

When EventLoop dispatches a handler coroutine, it wraps the
dispatch call in an `el::run_handler_with_recovery` boundary
which:

1. Calls the handler via the YIELD.1 frame-yield primitive.
2. After each yield-to-host, checks
   `state.database.runtime_error.is_some()` AND the kind is in
   `RECOVERABLE_KINDS` (StackOverflow, IndexOutOfBounds,
   DivideByZero, NullDereference — every kind except
   AssertionFailed and UserPanic which are intentional
   terminal calls).
3. If recoverable: takes the runtime_error, logs the rendered
   diagnostic via `Logger::log_runtime_kind` (production-mode
   shape per C66), pops the call_stack down to the
   handler-coroutine boundary, sets the handler's coroutine to
   `Exhausted`, decrements `had_fatal` if appropriate, returns
   to the EventLoop pump.
4. The pump's next tick spins up new event handlers — including
   a fresh coroutine for the same handler if its source still
   has events.  Each fresh coroutine starts with a fresh
   `call_depth = 0`.

The "boundary" is the coroutine-frame's bytecode pc.  The
dispatch loop unwinds to it by setting `code_pos` to the
coroutine's parked pc (the last `yield_to_host` site) and
restoring its stack snapshot.

### Concrete design — fault-loop circuit breaker

A handler that overflows on every event would spam logs
forever.  The EventLoop tracks `(handler_id,
recent_fault_count)` and:

- On Nth (default N=5) consecutive recoverable fault for the
  same handler: stop dispatching that handler entirely and log
  `Level::Fatal "handler X disabled after N consecutive
  faults"`.
- Surface a query API (`el.disabled_handlers() ->
  vector<text>`) so user code can introspect the breaker
  state.
- Reset the count when the handler completes a full event
  cycle without faulting.

### What this avoids (and why)

- **No `try { } catch (e) { }` user syntax.**  Per the user
  constraint, recovery is library-driven.  Adding new syntax
  for a library-only mechanism would over-commit the language.
- **No per-frame `#[recovery_point]` attribute.**  The
  EventLoop is the single recovery boundary; no per-fn opt-in
  needed.  If a future use case actually wants user-controlled
  recovery points, design then — `#[recovery_point]` could be
  layered on top of this slice without breaking it.
- **No implicit `n_main` recovery.**  CLI scripts and tests
  retain today's halt-on-fault behaviour (developer-friendly).
  Only EventLoop-driven programs (engine + game scripts) get
  recovery — explicit opt-in via the library entry point.

### Sequencing — when this design is ready to ship

Prerequisites already shipped (@PLN28 phase 4):
- ✅ Typed `RuntimeError` with kind + position + call_chain.
- ✅ Production-mode log+continue at `State::raise`.
- ✅ `--dev-soft-halt` (which proves the unwind shape works).

Prerequisites in flight (this plan):
- 🔜 YIELD.1-5 (frame-yield primitive + coroutine driver).
- 🔜 EventLoop dispatch loop with handler coroutines.
- 🔜 CANCEL.1 (recovery uses similar primitives — coroutine
  state transition, dep-tracking cleanup at unwind).

When all three @PLN32 prerequisites land, this slice becomes
straightforward: ~300 lines in `lib/web/src/event_loop.loft`
plus ~50 lines of new primitive
(`el_recover_to_handler_boundary`) in `src/state/mod.rs`
mirroring the `frame_yield` shape.

No code in this commit; this section is the design anchor.

---

## Cross-references

- **EVENT_LOOP_DISCUSSION.md** — open questions, alternatives
  considered, design history.
- **PROBLEMS.md § 213** — @P213 v4 design (closure-in-struct-fields,
  the load-bearing prerequisite).
- **DESIGN_DECISIONS.md § C38** — closures are copy-at-definition;
  forward direction toward Rust-style references.
- **WEB_SERVER_LIB.md** — server library (existing + planned),
  including `Dispatcher` and `run_game_loop` designs that the
  EventLoop evolves.
- **GAME_CLIENT_LIB.md** — client-side game loop design,
  fixed-timestep updates with interpolated render.
- **THREADING.md** — `par(...)` parallelism.
- **COROUTINE.md** — `iterator<T>` + `yield`.
- **@PLN28 phase 4f slice 2** (`doc/claude/plans/28-error-messages/04-runtime-error-kinds.md`) — typed `StackOverflow` RuntimeError; the
  prerequisite for the recovery design above.
