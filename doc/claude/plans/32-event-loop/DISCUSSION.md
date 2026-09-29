<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# EVENT_LOOP_DISCUSSION — open issues, alternatives considered, design history

**Status:** companion to [EVENT_LOOP.md](README.md).  This file
holds the parts of the design that are still being discussed,
superseded design iterations kept for context, and resolved
questions recorded so they don't get re-litigated.

The concrete spec lives in `EVENT_LOOP.md`; this document is its
counterpart for design conversation.  When a question is settled it
moves from here into the spec.

---

## Novice-readiness evaluation (2026-05-05) — pivot trigger

**Question.** Does the EventLoop library, as currently specified,
help a novice game programmer (someone who knows how to write
games but is not a senior systems programmer) use loft's
multiplayer/networking infrastructure?

This evaluation exists for one reason: **if closure-capture is
the dominant blocker, the right next move may be to fix that
before EventLoop ships, not after.**  The user has flagged that
they want to pivot focus to closures if this analysis confirms
they're the breaking point.

### What helps a novice

1. **The mental model matches Unity / Godot / Phaser.**
   "Register a callback for an event type" is the shape every
   game framework teaches.  `el::on(loop, fn(e: PlayerInput)
   { ... })` reads exactly that way.  No new vocabulary.

2. **Transport transparency is novice-shaped almost by accident.**
   Novices don't know what JSON is in detail, can't tell you
   what a header byte does.  They write a struct, register a
   handler, the message arrives.  The transport-transparency
   principle is the right concession to inexperience.

3. **Single-player → multiplayer path is gentle.**
   `submit` works locally; `send` works on the wire; both have
   identical handler signatures.  A novice can build single-player
   first, then add a server, and their handlers don't change.
   This is dramatically better than Unity's "rewrite for Mirror
   or Photon" story.

4. **Priorities default to NORMAL.**
   Novices skip the tuning phase entirely.  That's correct —
   the library doesn't punish them for not understanding priority
   lanes.  Real games ship at NORMAL-everywhere and tune only
   when they observe a problem.

5. **Single namespace.**
   `el::on / send / submit / step / run` is discoverable.
   Compare to today's "figure out `ws_send`, write your own
   JSON, write your own dispatcher" — dramatically smaller
   surface.

### Where a novice falls off

#### 1. Closure capture by value will be THE cliff

The novice writes:

```loft
let state = GameState { score: 0, ... };
el::on(loop, fn(e: ClickEvent) { state.score += 1 });   // doesn't work
```

…and gets a parser error about `Reference<T>` or `Mutable<T>`.

In every language the novice has used (JS, Python, Lua, C#
closures), this Just Works.  The error message will be Greek.
This is **not** something the EventLoop spec can fix — it's
loft's [C38](../../DESIGN_DECISIONS_OWNERSHIP.md#c38--closure-capture-is-copy-at-definition)
design choice — but it dominates the novice's first-day
experience.

This is the dominant blocker.  Every other novice issue listed
below can be solved with documentation or thin wrapper layers;
this one is a language-level decision that bleeds through every
handler the novice writes.

#### 2. No `connect()` / `host()` helpers

The spec assumes the programmer wires up `ws_upgrade`,
`tcp_accept`, the per-connection poll loop, and feeds bytes into
the EventLoop.  A novice expects:

```loft
let loop = el::host(7878);          // I'm a server
let loop = el::connect("ws://...");  // I'm a client
```

Without these wrappers the library *technically* solves the
dispatch problem but leaves the novice 100 lines from a working
multiplayer skeleton.  These wrappers are ~30 lines of loft on
top of `lib/server`, but they need to ship in v1 or the design
hasn't crossed the novice threshold.

#### 3. Handshake-by-type-name will fail invisibly

Novice has `PlayerInput` on client and `Player_input` on server
(typo in different file).  Either:
- The handshake fails (good — fails fast with a clear message), or
- The handshake succeeds for matching names and one handler
  silently never fires (bad — novice debugs for hours).

Implementation must do a registered-name diff and print it.
Today the spec says "fails fast if a name is missing on either
side" — adequate as words, but the implementation must actually
deliver that.

#### 4. State ownership is on the user

A novice writes the same handler on both sides.  Client mutates
`state.score` locally; server also computes `state.score`.  They
desync.  Novice doesn't know about authoritative servers,
prediction, lag compensation.

The library can't solve this for them, but **the docs must
include a "who owns what" diagram** for the canonical
multiplayer-game shape, or the novice learns by hitting a wall.

#### 5. No examples means no library

For novice users, **examples ARE the API**.  A novice opens the
docs, scans for "Pong, but multiplayer", and copies the code.
The spec currently lists `examples/01-keyboard-state.loft` and
`02-server-tick.loft` as files-to-create.  Until those exist
(plus `03-two-player-pong.loft`, `04-shared-world.loft`), the
design is library-shaped but not novice-usable.

#### 6. Disconnect / reconnect path is undefined

What happens to in-flight handlers when the WebSocket drops?
Does `send` return an error or silently drop?  Does the EventLoop
emit a `Disconnected` event the novice can handle?  A novice
expects the framework to surface this.  The spec doesn't say.

#### 7. "Tuning phase" is the wrong name

Novices have no performance data to tune against.  They read
"tuning phase" and skip it ("I'll do that when my game is
slow") — which is correct behaviour but the *name* makes them
feel they're skipping something important.  Better:
"Optional: priority overrides."

### Bottom line

**Is the library novice-fit as it stands?  No, but it's two thin
layers away.**

The EventLoop core is the right foundation — transport-transparent,
single-API, type-driven.  But "infrastructure a novice can use to
build multiplayer games" requires:

1. A `lib/game/` wrapper with `el::host(port)` / `el::connect(url)`
   / `el::run_client(loop)` / `el::run_server(loop)` — the 30
   lines of loft on top that hide WebSocket setup.
2. 3-4 worked examples progressing single-player → local two-player
   → networked Pong → shared world.
3. A "first hour with multiplayer" tutorial leading with the
   closure-capture footgun, then walking through one example.
4. A clear `Disconnected` / `Reconnected` event the novice can
   register a handler for.

Without those, the EventLoop is "a library a programmer can use
to build a multiplayer game", not "a library a novice game
programmer can use."

### The pivot question

Items 2-4 above are docs and ~30 lines of loft.  They're cheap
and can ship with v1 of EventLoop.

**Item 1 (closure capture) is structural** and eats every
handler the novice writes.  No amount of EventLoop polish hides
it.  The design currently sits on `Reference<T>` + the planned
`Mutable<T>` stdlib helper, and tells novices "wrap your mutable
state in these."  That's a workaround, not a fix.

The pivot the user has flagged:

> If closure-capture is the breaking point, reconsider C38
> *before* EventLoop ships.  An EventLoop sitting on a
> closure model the novice can't use is a library the novice
> can't use.

Options if the pivot is taken:

1. **Move closer to Rust's closure model** (recorded as the
   future direction in [DESIGN_DECISIONS.md § C38](../../DESIGN_DECISIONS_OWNERSHIP.md#c38--closure-capture-is-copy-at-definition)).
   `&T` / `&mut T` capture, FnOnce / FnMut / Fn capability
   hierarchy.  Heaviest lift; most novice-friendly because
   `state.score += 1` Just Works.
2. **Capture-by-reference-of-Reference, automatically.**  When
   the closure mentions a captured `Reference<T>`, the parser
   auto-dereferences mutating field accesses.  Light lift;
   covers the "I have a state struct" case that novices write
   first; doesn't help mutable scalars.
3. **`Mutable<T>` stdlib helper as planned.**  Novice wraps
   every mutable field manually.  Cheapest; novice still
   stumbles, just less.

Concrete pivot decision rule (to be made by the user, not
pre-empted here): if the closure-capture issue is the thing
that costs novices most of their first-day experience — and
the evaluation above suggests it is — then 1 or 2 above
deserves to be the priority work, with EventLoop layered on top
once the closure model carries the load a novice expects.

If the user takes the pivot, the EventLoop spec stays in this
document set as the target architecture; implementation is
deferred until the closure model is novice-fit.

---

## Verification of open questions against shipped code (2026-05-05)

A second-pass survey corrected several false assumptions in the
first design draft.  Each open question re-checked against actual
loft code:

| # | Question | First-draft status | Re-verified status |
|---|---|---|---|
| Encoding (JSON) | "Need a JSON encoder/decoder" | Assumed missing | **Already shipped.** `OpFormatDatabase` (struct → JSON), `database.parse(text, tp, ref)` (JSON → struct), `n_json_parse` (JSON → JsonValue), lenient parser for loft-native data literals.  No new encoder needed; design uses existing ops. |
| Encoding (binary typed) | "Binary mode for typed structs" | Assumed designable | **Genuine gap.**  No `OpFormatDatabaseBinary` exists.  Workaround: keep raw-bytes-in-struct via JSON envelope (texture bytes inside a `bytes` field); typed-binary packing deferred. |
| Encoding (raw bytes) | "Encoding::Raw — pass bytes through" | Listed in design | **Works today via WebSocket binary frames** (extension to native side needed); the `bytes`-in-struct pattern works through JSON. |
| Wire envelope | "[handler_id][priority][seq][flags][length][payload]" | Designed from scratch | **Existing `GameEnvelope` covers most fields.**  Has `sender`, `recipient`, `sequence`, `timestamp`, `message: WsMessage { msg_type: MsgType, payload: text }`.  Missing: priority byte; per-handler routing currently uses `msg_type` enum.  Evolution: add priority field; either extend `MsgType` or add a generic discriminator. |
| Handler-id pattern | "Library auto-assigns integer handler-id" | Proposed as new | **Already idiomatic.**  `Server.handle: integer`, `WebSocket.ws_id: integer`, GL texture/VAO/shader IDs all use opaque integer handles.  Fits the established convention. |
| Dispatcher | "EventLoop with handler list + match" | Proposed as new | **Already designed in WEB_SERVER_LIB.md** as `Dispatcher` struct + `dispatch(env, &Dispatcher)`.  EventLoop adds priority lanes on top. |
| Game loop driver | "run_async / run_frame" | Proposed as new | **Already designed in WEB_SERVER_LIB.md** (`run_game_loop`) and `GAME_CLIENT_LIB.md`.  EventLoop wraps these; doesn't replace. |
| Server-side platform | "blocking accept loop or async" | Open | **Blocking today.**  Async via mio is a real gap; not blocking for first multiplayer game (a small connection count scales fine on blocking model). |
| File watching | "notify crate" | Listed as gap | **Genuine gap.**  Asset hot-reload requires polling today. |
| Priority lanes | "HIGH/NORMAL/LOW" | Proposed | **Genuine gap.**  Existing dispatcher dispatches in declaration order; no priority concept. |
| Per-direction priority on the wire | "1B priority byte tagged by sender" | Proposed | **Genuine gap.**  `GameEnvelope` has no priority field; adding it is one struct field + native serialisation step. |
| Generic over user payload | "EventLoop<E> or Event<P> with Custom(P)" | Proposed | **Sidestepped by GameEnvelope's design.**  `MsgType` is an enum; payload is text; programmer extends `MsgType` and adds a constructor.  No generics needed.  Less type-safe than `Custom(P)` but matches what's there. |

---

## Resolved questions (recorded so they don't get re-litigated)

Decisions that have been made and locked into the spec:

- **Handlers register without priority; tuning is a separate phase.**  The programmer says "this event does X"; priorities are configured separately and tweakable at runtime, including from a config file or live-tuned during gameplay.

- **Bidirectional handlers, client-authoritative.**  Each handler is a typed Send/Recv channel.  Library auto-assigns the handler-id at registration; the connection handshake exchanges (name → id) maps; server mirrors the client's handler API.  Server-side restrictions (auth, rate-limit, validation) are middleware, deferred.

- **Wire format: binary header + JSON or binary payload, per-handler at registration.**  Three encoding modes: `Json` for structured messages, `Binary` for typed-struct packing (deferred for v1), `Raw` for bytes-in-struct (textures, meshes, world chunks).

- **Per-direction priority on the wire.**  Each side independently tags outbound frames with a priority byte (0 = highest, 255 = lowest).  Receiver routes by what's on the wire, not by lookup.  Asymmetric channels (HIGH outbound request → LOW outbound response) work naturally.

- **Streaming = library-assembled, handler sees complete messages.**  v1 default is Model C: the user's recv closure only fires once a full payload has been assembled.  The patient wait IS the async property — higher-priority traffic preempts the assembly without the handler caring.  Models A (chunk events) and B (coroutine iterator) are deferred or rejected (see "Design history" below).

- **`on_each` future opt-in for vector recv-types.**  Refinement to streaming: when the handler subscribes to a vector-typed payload, an opt-in `on_each(loop, name, encoding, marker, fn(item) { … })` form delivers each parsed element as it arrives on the wire.  Doesn't ship in v1; doesn't change the default `on()` semantics.

- **Closure capture is by value (C38); state is captured via `Reference<T>`; `Mutable<T>` stdlib helper for scalars.**  See [DESIGN_DECISIONS.md § C38](../../DESIGN_DECISIONS_OWNERSHIP.md#c38--closure-capture-is-copy-at-definition).

- **EventLoop is a library, not a language construct.**  Lanes-and-budgets are pure runtime semantics; no parser change.

- **3 priority lanes (HIGH/NORMAL/LOW) initially, numeric priorities (0-9 or 0-255) under the hood.**  Three lanes is the user-visible default; numeric grants finer tuning when needed.  Each handler picks one slot.

- **Defer-on-overflow for budget enforcement.**  When NORMAL or LOW lane budget is exhausted mid-frame, unprocessed events stay queued for next frame.  Telemetry warning if backlog grows past a threshold.

- **Default priority for unconfigured kinds = NORMAL** with a one-time warning the first frame an unconfigured kind submits.

- **Static handler registration in v1.**  Handlers added at startup, never removed.  Dynamic add/remove deferred to a follow-on if real friction surfaces.

- **User-supplied poll callback in v1.**  Library doesn't know about specific sources; consumer writes the bridge from raw sources to `submit(...)` calls.  Convenience pollers (graphics, websocket) added later as additive helpers.

- **Tick as event.**  Treat physics tick as a NORMAL-priority event submitted by a small helper each frame.  Uniform mental model.

- **No per-instance priority override in v1.**  Define a separate variant per priority class instead.

---

## Concerns and open issues — critical pass

After the design accumulated several layers, an honest critical pass
surfaced these problems.  Each carries a status: resolved /
deferred / accepted limitation / still-open.

### Hard problems

**P0. Binary payloads must be first-class.**
*Status:* **Resolved by per-handler encoding declaration.**  Handler
registration takes an explicit `Encoding` (Json / Binary / Raw);
library encodes/decodes accordingly.  Raw mode passes bytes
through with no library introspection; streaming machinery
handles multi-frame transport identically across all three
encodings.  Three concrete cases (world chunks, avatar textures,
unique meshes) all use Raw mode.

**P1. Schema drift between client and server.**
*Status:* **Softened by JSON wire format + the existing
`database.parse` machinery.**  Missing fields default; extra fields
ignored; per-frame decode errors are local (not protocol-fatal).
Not perfect — silent semantic drift won't be caught — but
acceptable for v1.  Future: stdlib `Schema<T>` helper that
exchanges schemas in the handshake.

**P2. Per-handler priority too coarse for asymmetric channels.**
*Status:* **Resolved by per-direction priority on the wire.**  Each
side independently tags outbound frames; receiver routes by
wire-tag.  `set_priority(h, p)` means "tag MY outbound for h as
p"; the peer's choice is its own.

**P3. Streaming under bidirectional handlers.**
*Status:* **Resolved.** Library-assembled (Model C) — handler
sees the fully decoded message.  Patient wait property is the
core async feature.  See § Streaming consumers in EVENT_LOOP.md.

**P4. Send-side backpressure is unspecified.**
*Status:* **OPEN — needs decision.**  What does
`el::send(loop, h, msg)` do when the socket buffer is full?
Recommended approach: buffer in user-space outbound priority
queues (the dual of inbound priority lanes); per-handler queue
depth limit; on overflow, drop OLDEST LOW-priority pending or
return Result::Err for HIGH-priority overflow.  Telemetry
counter on overflow.  Not a hard problem to solve; needs a
concrete spec.

### Medium problems (workable; document as known limitations)

**P5. Plan-17 partial generics force ugly registration API.**
*Status:* **Accepted as v1 friction.**  `HandlerId<R>` + send-type
marker is what we get under partial generics.  Migrate to
`HandlerId<S, R>` additively when @PLAN17 ships multi-param.

**P6. Handler-name strings are fragile coordination.**
*Status:* **Accepted as v1 friction.**  Workaround: a shared
`handler_names.loft` module both client and server import,
defining `let HANDLER_WORLD: text = "world_update";` etc.

**P7. Reconnection and id-stability.**
*Status:* **Punt to v2.**  v1 spec: reconnection always performs a
fresh handshake; in-flight messages from before the disconnect
are lost.

**P8. Library-internal events (Network/File/Timer) integration.**
*Status:* **OPEN — needs design.**  Where do "incoming connection
from a peer that hasn't handshaken yet" events go?  Recommended:
a library-reserved `pre_handshake` handler-id that the user
registers a recv closure on; closure decides whether to accept
or reject.

**P9. Cross-handler local communication.**
*Status:* **Resolved as design guideline.**  Pin in docs: prefer
`send → server → server forwards → other handler's recv` over
local handler-to-handler `submit`.  The former runs through the
priority/handshake/wire machinery uniformly; the latter is an
internal short-circuit.

**P10. Closures are copy-by-value.**
*Status:* **Resolved by Mutable<T> stdlib helper + teaching
strategy.**  See § Teachability in EVENT_LOOP.md.

### Meta problem — phasing the project

**P11. The dependency stack is long.**
*Status:* **Resolved as phased delivery.**  See § Sequencing in
EVENT_LOOP.md.  Phases 1-2 (@P213 v4 + minimal closure-using
single-player game + EventLoop core) are the gate to a running
game; phases 3-5 (wire protocol, async multiplexer, streaming
optimisations) are multiplayer infrastructure layered on top.

---

## Still-open questions

After resolving the questions above, two remain that need a decision
before implementation:

### Q-A. Send-side backpressure (P4)

When `el::send(loop, h, msg)` is called and the socket buffer is
full, what's the contract?

**Options:**
- **(a) Block** until space is available.  Simple but blocks the
  EventLoop's frame.
- **(b) Buffer** in user-space outbound priority queues; drain
  when the socket is writable.  Per-handler queue depth limit; on
  overflow, decide.
- **(c) Drop oldest** LOW-priority pending message.  Simple; risks
  losing messages.
- **(d) Return Result/error** and let user retry.  Most explicit;
  most user code to write.

**My recommended path:** (b) with these defaults — per-handler queue
depth = 1024 messages; on overflow, return Result::Err to the
caller for HIGH-priority sends (so the user can decide to retry,
back off, or skip); for NORMAL/LOW priority sends, drop the OLDEST
pending message and emit a `BackpressureDrop(handler_id)` telemetry
event.  Keeps the EventLoop frame non-blocking while giving users
the option to handle critical-path send failure.

### Q-B. Pre-handshake events (P8)

When a TCP/WebSocket connection is accepted but the handshake
hasn't completed yet, where do incoming bytes go?  No handler-id
exists yet for that connection.

**Options:**
- **(a) Library handles handshake transparently.**  User registers
  handlers; library accepts, performs handshake, then dispatches
  handler events.  Simple but no policy point.
- **(b) Reserved `pre_handshake` handler the user registers.**
  User's closure decides whether to accept the connection (then
  triggering full handshake) or reject.  Adds a policy point.
- **(c) Hybrid.**  Library handshakes by default; user can override
  by registering a `pre_handshake` handler.  Most flexible.

**My recommended path:** (c).  Default to (a)-like transparent
handshake; if the user wants to filter / authenticate / rate-limit
before letting a connection through, they register a handler on
the `pre_handshake` reserved id.

### Q-C. Typed-binary serialisation

`Encoding::Binary` (typed-struct packing) is in the design but
loft has no `OpFormatDatabaseBinary` op today.  v1 ships only
`Encoding::Json` and `Encoding::Raw`.

**Decision pending:** when does typed binary become worth the
implementation cost?  Not v1.  Not v1.x unless real benchmarks
show JSON encoding is the bottleneck.  Recorded for future when
needed.

---

## Design history — superseded iterations

Earlier iterations of the design that were superseded but kept here
as context.  Don't re-propose these without new evidence.

### Iteration 1 — `Event<P>` enum with `Custom(P)` for user payload

Initial design proposed a library-defined `Event<P>` hierarchy
where the library shipped variants for Network/File/Timer/Stream
events and the user added their own via a `Custom(P)` wrapper
variant.

```loft
enum Event<P> {
    Network(Token, NetReadyKind),
    File(text, FileChangeKind),
    Timer(integer),
    BeginStream(integer, Priority, bytes),
    ChunkStream(integer, bytes),
    EndStream(integer, bytes),
    Tick,
    Custom(P),
}
```

Programmer would write `match e { Custom(KeyDown(k)) => …,
Tick => … }` style handlers — one per priority lane, with all
events flowing through the same handler.

**Why superseded:** the user's clarification ("a handler should
not be tied to only input; it has two-way communication with the
server") shifted the model from broadcast-handlers to bidirectional
typed channels.  Each handler now has its own typed Send/Recv
pair, registered via `on(loop, name, encoding, send_marker, fn(R))`.
The match-style broadcast handler doesn't fit the bidirectional
shape.

The Custom(P) approach also imposed `match e { Custom(KeyDown(k)) =>
… }` — a wrapper at every match arm — which read awkwardly.

### Iteration 2 — handler registration carries priority

Earlier registration form:
```loft
el::on(loop, HIGH, fn(e) { … });
el::on(loop, LOW, fn(e) { … });
```

**Why superseded:** the user pointed out that the programmer's
job is "what does this event do", not "where does it sit in
the priority stack."  Priorities should be assigned in a
separate tuning phase.  Replaced with priority-less registration
+ separate `set_priority(loop, h, p)`.

### Iteration 3 — Streaming Model A (begin/chunk/end events to user)

```loft
el::on(loop, fn(e) { match e {
    BeginStream(id, hdr) => state.start_chunk(id),
    ChunkStream(id, body) => state.append(id, body),
    EndStream(id, body) => state.finalize(id),
}});
```

Handler is a state machine that accumulates chunks across calls.

**Why superseded:** Model C (library-assembled, handler sees only
complete messages) is cleaner — matches the bidirectional handler
signature `fn(R: RecvType)` directly, no per-handler accumulator
state machine.  The user's "use default serialization" direction
explicitly favours hiding chunks behind the library.

**Could return as opt-in advanced API** if a use case really
requires partial-data handling (progress bars, streaming
decompression) — but `on_each` (per-element vector streaming)
covers most of that need.

### Iteration 4 — Streaming Model B (coroutine-style iterator)

```loft
el::on_streaming(loop, "world", fn(stream: iterator<bytes>) {
    let buf = vector<u8>();
    for chunk in stream { buf.append(chunk); }
    let world = parse(buf);
    state.apply(world);
});
```

Handler reads from a coroutine-style iterator that pauses when no
chunks are available.

**Why superseded:** needs a new language primitive — a
"consumer-controlled coroutine" / channel — that loft doesn't
have.  Loft's `iterator<T>` + `yield` is producer-driven; this
requires the inverse.  Model C is simpler and ships with current
loft.  Defer Model B until real use cases demand it.

### Iteration 5 — `OpDatabase` retarget for closure record co-location

Considered design for @P213 v4 (capturing closures in struct
fields): preset the closure work-var `w` to the host's DbRef
before `OpDatabase` fires, so the new closure record is claimed
in the host's Store rather than a fresh one.

**Why rejected:** `OpDatabase` calls `database.clear(&db)`
unconditionally.  Setting `w` to `host_ref` would cause the host's
Store to be *cleared* before the new record is allocated —
destroying the host record.  Killed by critical pass before any
code was written.

**What replaced it:** `OpAppendVector` / `OpClaimChild` path —
existing op that claims a new record without calling clear.
See PROBLEMS.md § 213.

### Iteration 6 — Layout widening of `element_size(Type::Function)`

Initial @P213 v4 attempt widened `element_size(Type::Function)`
from 4 to 16 (or 20) bytes so closure records could be inlined.

**Why reverted:** the database layer routes `Type::Function`
through `def_nr("i32")` for storage (4 bytes); widening the
declared size didn't reach the actual record allocator, leading
to silent corruption.  Multiple cascading effects on tuple,
vector, hash storage — too much blast radius.

**What replaced it:** the co-located closure record approach,
where the closure data lives in the same Store as the host as a
child record (vector-style claim).  See PROBLEMS.md § 213.

### Iteration 7 — Detailed mio-based async multiplexer in v1

Earlier design fully spec'd a mio-based AsyncPoller with file
watching, listener registration, kernel-level multiplexing,
hybrid game-host-as-server.

**Why deferred:** the existing blocking server (`lib/server/`)
scales adequately for first-multiplayer-game.  mio integration
is a real chunk of native work (new dep, FFI surface,
cross-platform file watching shim).  Layered AFTER first
multiplayer game ships — when real connection-count pressure
demonstrates the need.  Recorded design exists; just not v1.

---

## Cross-references

- **EVENT_LOOP.md** — concrete spec.
- **PROBLEMS.md § 213** — @P213 v4 design (closure-in-struct-fields,
  the load-bearing prerequisite).
- **DESIGN_DECISIONS.md § C38** — closures are copy-at-definition;
  forward direction toward Rust-style references.
- **WEB_SERVER_LIB.md** — server library (existing + planned),
  including `Dispatcher` and `run_game_loop` designs that the
  EventLoop evolves.
- **GAME_CLIENT_LIB.md** — client-side game loop design,
  fixed-timestep updates with interpolated render.
- **THREADING.md** — `par(...)` parallelism (used inside handlers
  but not for event multiplexing).
- **COROUTINE.md** — `iterator<T>` + `yield` (producer-driven; not
  the same shape as Model B's consumer-controlled iterator).
