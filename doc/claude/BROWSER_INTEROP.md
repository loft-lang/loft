<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Browser interaction model — the engine as an agnostic service provider

How a loft `--html` program and its libraries interact with the browser and
with JavaScript.

> **STATUS.**  *Shipped today:* the asyncify yield, the host-import bridge
> (see [HTML_EXPORT.md](HTML_EXPORT.md)), the `web` WebSocket
> `[wasm.bridge]`, `loft_host_print` (output), **`host_input()`** — since
> #476, and QUEUE-shaped since 2026-07-03: JS seeds it with
> `globalThis.loftInput` and pushes live messages any time via
> `globalThis.loftPush(msg)`; each call pops one message — and
> **`host_output(msg)`**, the outbound mirror: a structured message to
> `globalThis.loftOutput(msg)` (stderr on native/WASI).  Together they ARE
> the push/poll byte channel this doc designs: loft `host_output`s a request,
> the JS shell acts on it (`fetch()` etc.) and `loftPush`es the completion
> for `host_input()` to pop — JS owns the network, loft stays pure compute.
> Round-trip proven headlessly:
> `tests/html_wasm.rs::host_output_input_roundtrip_queue`.
> *Not shipped:* NAMED multi-channel routing (one queue today — frame your
> own message ids, as the engine's kind-3 events do) and any browser HTTP in
> wasm (by design — the network stays in JS).  Where a claim is proven, this
> doc says so and points at the running code.

**Motivating consumer.** The zero-trust browser client (a `--html` page that
runs the same platform-blind loft core as the native binary, behind a clickable
file list + editor) is the dogfood that drives this. But nothing here is
document- or game-specific by design — that is the whole point.

---

## The one invariant

> **The browser host — the "engine" — moves only opaque bytes (and scalars)
> between a loft routine and JavaScript. It never knows the shape or meaning of
> what crosses. All knowledge of structure — loft types on one side, JavaScript
> structures on the other — lives in loft *libraries* that consume the engine,
> never in the engine itself. JavaScript-specific knowledge is confined further
> still, to libraries that declare themselves browser-only.**

Everything else in this doc is a consequence of that one rule. The engine is a
service provider with no opinion about its consumer; a game and a document
editor are both just routines that compose its services.

### Why this rule, and how it stays true (the re-assertion sites)

The brittleness of any "let loft talk to the browser" design lives in **how
many places encode knowledge of the other side**. The cure is to push that
count to near zero at the layer that must stay generic:

- The engine's host-import surface is **small and closed** — render, input
  (incl. text/IME/clipboard), the frame loop/scheduler, and one generic byte
  channel. Each entry takes/returns only bytes or scalars.
- A loft *type* (a struct, a vector) or a JavaScript *shape* (a JSON key, a
  typed-array layout) appearing in an engine host-import signature **is the
  leak** — the single smell to watch for in review or a lint. Because the
  engine surface is closed and tiny, this is a visible, catchable violation, not
  a silent one.

So `N` (sites that must restate the invariant) equals the number of engine
services — small, fixed, additive-only — and omission is **loud** (a non-byte
type in that surface is reviewable). That is the prospective tell that this
design is not a spray.

---

## The four tiers

| Tier | Knows about | Runs where | Example |
|---|---|---|---|
| **Engine** (host bridge) | nothing domain- or language-specific; moves bytes/scalars | the browser host (JS in the generated page; native libs natively) | render, input, loop, the byte channel |
| **Platform-blind loft libs** | loft types + a domain; **not** the platform | every target identically | the doc model, merge, crypto, CBOR |
| **Browser-target loft libs** | loft types **and** JavaScript shapes; declared browser-only | wasm/browser only | a JS-facing SDK; a DOM/typed-array adapter |
| **Consumer routine** | composes libraries into an application | wherever it is built | the zero-trust client; a game |

The document-editor behaviour I earlier mistook for "the engine lacks it"
(text layout, selection, a caret, undo) is **not** an engine concern and not a
gap in an agnostic surface — it is a **widget library** in the platform-blind
tier, a client of the engine's input + render services, reusable by a game's
in-world text field and a settings dialog alike. The engine gains general
*services* (text/IME/clipboard input); the *behaviour* stays in a library.

---

## What is shipped vs what is new

**Shipped** (today, on the installed loft):

- The `--html` AOT path: loft → generated Rust → `wasm32-unknown-unknown`
  cdylib, embedded base64 in a self-contained `.html`. One export today:
  `loft_start` (= `n_main`), plus `memory` and the asyncify control exports.
- The **host-import bridge**: codegen emits one `env`-namespace import per
  native a reachable program uses — this is how `loft_println`, the `gl_*`
  functions, and `web`'s `ws_*` become JS-implemented bridge functions. **A
  loft library already declares its own host imports this way.**
- The **asyncify yield**: a suspending import (`loft_gl.loft_gl_swap_buffers`,
  `loft_web.ws_yield`) returns control to the JS event loop and resumes intact.
  Issue #450 fixed the resume pump for hidden/headless pages (an unthrottled
  `MessageChannel` while hidden, `requestAnimationFrame` while visible, with a
  fallback when a visible page is granted no frames).
- The **target-conditional library + JS bridge** mechanism: a `[wasm.bridge]`
  in `loft.toml` plus a `wasm/host.js`. The `web` library already ships exactly
  this (its `host.js` is where the browser WebSocket lives).

**New** (the proposed additions this design names):

1. A generic **byte channel** as an engine service — push bytes out to JS, poll
   bytes in from JS — declared as host imports that carry only `(ptr, len)`. The
   input half has a concrete, code-anchored design below (*The input half as a
   shipped engine primitive*).
2. The **input-service surface** the engine is missing for any text UI: typed
   text + IME compose state + clipboard read/write, backed JS-side by a hidden
   editable element. General I/O services, not document features.
3. Convention (not a language feature) for **browser-target libs to emit
   JavaScript-shaped data** — see the cost gradient below.

No new core-language feature is required for any of this — no wasm-export ABI,
no wasm-bindgen. Everything reduces to the shipped yield + host-import
primitives, composed in libraries. (The wasm-export / JS-calls-loft ABI was
considered and set aside: it solves synchronous typed JS→loft calls, which this
model does not need — see *The data boundary* and *Rejected alternatives*.)

**Verified stronger (scoping pass):** the three items above are not even engine-
*code* additions — they are realizable as a browser-target **library** on
machinery `--html` already ships. A library's `wasm/host.js` bridge is inlined
into the generated page generically (`src/parser/mod.rs:6567`,
`src/main.rs:5716`); any reachable body-less `#native` is emitted as a host
import with no allowlist (`src/generation/mod.rs:1152`); and the `(ptr,len)`
push + len/copy poll primitives are proven by `web`'s browser WebSocket bridge.
So loft's `src/` needs **zero change** — the lone conditional touch (a *blocking*
suspend import → the asyncify `--pass-arg` list in `src/main.rs`) is avoided by
keeping every service poll-based. The *Verifiable build steps* below are the
proof: each runs against the stock installed loft.

---

## The input half as a shipped engine primitive — `host_input`

The byte channel's **output** half already ships as an engine primitive:
`loft_host_print` is hard-coded into the `--html` host-import set under the
`loft_io` module (`src/generation/mod.rs:1152`), and the stdlib reaches it through
the per-target `#rust` body of `OpPrint` (`default/01_code.loft:1125`). Its
**input** half is missing: under `--html`, `arguments()` and `file()` return
empty — both return heap values, so codegen emits them as graceful stubs
(`src/generation/mod.rs:1187`), which the first browser consumer confirmed by
dumping the WASM imports. So a headless `--html` program can emit a result but
**cannot receive its input**. This is now **shipped** as `host_input()`, closing
that gap for the **headless one-shot** shape: JavaScript owns the loop, and the
input is complete before compute starts (a request/response kernel).

**Why an engine primitive, not a library, for this shape.** The invariant above
lists "one generic byte channel" as an *engine* service, and output already
honours it. Input should be symmetric. The library route (a `#native` +
`wasm/host.js`, zero core change — see *Verified stronger*) stays right for the
**streaming, loft-owns-the-loop** shape, where the channel is polled each frame
across the asyncify yield. But for a headless one-shot read a library is both
wider (every consumer installs it and registers a `host.js`) and mis-tiered (input
in a library while its sibling output lives in the engine). The two shapes differ
— input complete at start vs arriving per frame — so one mechanism should not
cover both: `host_input` is the one-shot read; a future `host_poll` is the
streaming sibling.

**The one invariant it rests on.** A pure-loft program that reads `host_input()`
and emits a transform of it produces byte-identical output on `--interpret`,
`--native`, `--native-wasm`, and `--html` for the same input — because on each
target `host_input()` resolves to that target's own input source, and the engine
never reads the bytes.

**The named brittleness: four backings, each silent if wrong.** The primitive is
small, but its correctness lives in one body per target (the shape `print`
carries), each a silent empty string if wrong:

| Target | `host_input()` reads | If the backing is missing |
|---|---|---|
| `--interpret` | stdin (or an injected capture buffer) | returns empty — **silent** |
| `--native` | stdin to EOF | returns empty — **silent** |
| `--native-wasm` (WASI) | WASI stdin | returns empty — **silent** |
| `--html` | the JavaScript-supplied blob, via the `loft_io` host imports | returns empty — **silent** |

**The bound: `host_input(wait_ms)` (loft#891).** Reading to EOF can only be
answered by a host that hangs up, so the bare form cannot ask an OPTIONAL
question — and "is anyone there?" is exactly the question a page-or-native client
needs answered at boot. The defaulted `wait_ms` bounds the wait: `-1` (the
default) keeps the read-the-whole-stream behaviour, `0` takes what has already
arrived, and a positive value waits that long for the first byte. So
`host_output("MODE?"); host_input(200)` is a request/response exchange on native
and interpret, the way it already was on `--html`, and an empty answer means
nobody is listening rather than a hang.

On native this rests on a background drain of stdin (`HostInputPump` in
`src/database/format.rs`), started on the first read so `loft repl` and
`loft debug` keep their own stdin. It hands over only COMPLETE characters —
a timed read that lands mid-character holds the tail back — because bytes split
across two reads would otherwise become replacement characters, and a truncated
character left in the buffer would end every later wait instantly and report an
empty read while the stream is live. `--html` ignores `wait_ms` (a page cannot
wait for a message its own thread must deliver, so every read there is already
the `0` poll); `--native-wasm` has no threads, so a bounded read there falls back
to the blocking drain rather than claim "nothing pending" it cannot verify.

Every omission is silent (an empty string, not a compile error) — that is the real
risk, and the parity gate below is its cure: it turns a missing backing into a
loud test failure.

**The surface — three layers, as shipped.**

- *Engine host imports.* Declared in the loft library's `--html` extern block
  (`src/lib.rs`, beside `loft_host_print`): `loft_host_input_len() -> usize` and
  `loft_host_input_copy(ptr: *mut u8)`. Two calls, because input has a sizing step
  output does not: loft learns the length, allocates a buffer, then has the host
  fill it — the `web` library already proves this host-owns-buffer → loft-`text`
  pattern (`ws_msg_len`/`ws_msg_copy`).
- *Stdlib + runtime.* `pub fn host_input() -> text` (`default/02_files.loft`,
  beside `env_variable`), a **native builtin** (`n_host_input`), **not** an
  operator. It backs onto `Stores::host_input_native` (`src/database/format.rs`),
  whose per-target `cfg` arms are the backings above: stdin on native/WASI, the
  `loft_io` host imports on `--html`, empty on the IDE `wasm` build. The
  interpreter routes it through the dest-passing native registry
  (`n_host_input_dest` in `src/native.rs`, listed in `is_text_dest_native`) — and
  that is *why* it is a native builtin, not an operator: interpreter text values
  on the stack are borrowed `&str`, so a freshly-read `String` can't cross
  `put_stack`; the dest-passing path (`env_variable`'s exact shape) allocates it
  into the store instead. Native codegen inlines the `#rust` body
  `stores.host_input_native()`, whose `--html` `cfg` arm calls the host imports.
- *JavaScript bridge.* `loft_host_input_len` / `loft_host_input_copy` in both the
  minimal engine-less shell (`src/main.rs`) and `buildLoftImports`
  (`doc/loft-gl-wasm.js`) read the bytes JS set on `globalThis.loftInput` before
  `loft_start`. A headless Web-Worker consumer supplies the same few lines in its
  own imports object — no bridge crate. `loft_start` builds fresh `Stores` each
  call (`src/generation/mod.rs:1567`), so one instance serves many requests (set
  the input, call, read the output, repeat).

**The acceptance gate (the cure for the four silent backings).** `tests/host_input.rs`
guards the interpreter == native byte-parity (plus the empty and UTF-8 cases) by
feeding the same bytes on stdin. Three further tests hold stdin OPEN — the state
the plain harness cannot reach, since writing stdin and dropping it ends even a
blocking drain — and check that a bounded read answers with no host, still reads
a host that stays connected, and never splits a character across two reads. The `--html` leg — JS sets `globalThis.loftInput`,
the wasm prints the same result — is proven with the Node harness recorded in
[WEB_APPS.md](WEB_APPS.md); the WASI leg shares the native stdin backing. Extending
the Rust gate to drive `--html` under headless Chromium (the `tests/html_wasm.rs`
shape) would close the last automated corner.

---

## The data boundary — push, poll, gather-until-enough

loft owns the loop (the engine's scheduler service); JavaScript is event-driven.
They never call each other's functions. They meet at the engine's byte channel:

- **Push (loft → JS).** A library calls a host import (`fn js_emit(channel:
  text, payload: text);`) with serialized bytes; the bridge routes it to a JS
  callback. Identical in shape to how `println` pushes a string the page
  appends. Available today.
- **Poll (JS → loft).** JavaScript enqueues a request on the host side; the loft
  routine polls it each frame and pushes a response back — the same channel run
  the other way. Polling each frame is already how loft reads input
  (`gl_poll_events`).

Because the **library serializes to bytes before it pushes**, the boundary never
carries a loft heap object. JavaScript reads `(ptr, len)` out of `memory.buffer`
exactly as it does for `println`; it never walks loft's store. The "a struct is
a store-addressed heap object JS can't read" problem simply does not arise.

### "Pretend to be synchronous" — the gather-until-enough contract

The loft side does **not** pay for the event-driven boundary in its own code.
The asyncify yield lets a library present a **synchronous-looking** API over the
frame-yielding engine: a function gathers inbound bytes across however many
frames it takes, decides it has *enough* (a complete request, a finished
computation), and returns the finished unit. The yielding is invisible to the
loft caller.

This is **already proven** — the zero-trust `ztclient` transport does exactly
it. `poll_for` loops `try_recv()` and calls `frame_yield()` each pass,
correlating frames by request-id until the match arrives, then returns it. The
loft consumer writes `let reply = poll_for(h, mid);` — straight-line — while
asyncify suspends to the event loop and resumes between polls.

**The single load-bearing rule:** *"blocking" must mean yield-and-accumulate,
never a hard spin.* The gather loop has to give the frame back each pass
(`frame_yield()`), so the engine keeps rendering and taking input while the
library waits. A wait that does **not** yield starves the event loop — the page
never receives, the UI freezes. (That is the exact failure issue #450's repro
showed: a synchronous loop that never returns to the event loop.) The yield
contract enforces this; `poll_for` obeys it.

### The cost gradient — how a browser-target lib shapes data for JS

A browser-only library is free to emit JavaScript-native structures. Pick the
cheapest rung that fits the payload; default to the top:

1. **JSON text, one push → `JSON.parse`.** JS gets a native object/array.
   Cheapest, the default; right for structured metadata (a file list, a record).
2. **A typed-array view over wasm memory (zero-copy).** The real reason to go
   past JSON: bulk binary (pixels, audio, file bytes) handed to JS as a
   `Uint8Array`/`Float32Array` with no copy.
3. **A host-import object-builder API** (`js_obj_new` / `set` / `push`).
   Richest, but **chatty** — many bridge calls per object, and the rung where
   you have hand-rolled a slice of wasm-bindgen. Reserve it for a genuinely
   needed live JS object graph; never the default.

---

## The binary bridge — `deliver` / `expose` (zero-copy, layout-driven)

Rung 2 above, made general (@PLN105, shipped). `deliver`/`expose` hand JS a whole loft value —
record, vector, enum, nested to any depth — **in wasm linear memory, with no serialization and no
copy**, driven only by a self-describing layout descriptor loft emits alongside the value. This is
the primitive routing's base-map `view` wanted (its ~230k-feature text serialize + JS `parseFloat`
of millions of coord strings was the front-end bottleneck) and the games per-frame vertex/index
path. Additive to the `loft_io` shim; nothing existing changes.

**The two calls** (`default/02_files.loft`; lowered in `parser/control.rs` like a builtin):

```loft
deliver(tag: integer, value: T)   // SYNCHRONOUS one-shot: the borrow ends when deliver returns
expose(tag: integer, value: T)    // LONG-LIVED: pins the value's store; read it each frame
release(tag: integer, value: T)   // unpins the exposed store
```

**Mechanism — a descriptor + a reader that is the twin of `read_data`.** loft emits a compact
descriptor (`LayoutDesc::to_json` → `{nodes, names, sizes}`, memoized per type; keyed collections
add a `flat` redirect map) INLINE with the handle — `loft_host_deliver(tag, storeBase, rec, pos,
typeId, descPtr, descLen)`. JS reconstructs via `readLoftValue` (`doc/loft-deliver.js`), the exact
twin of Rust's `read_via_descriptor` (`src/database/descriptor.rs`) — **keep the two in lockstep**;
`tests/deliver_wasm.rs` is the byte-parity gate (interpret == native == `--html`). `storeBase` (the
store's `ptr`) IS a wasm linear-memory address; `addr(rec,pos) = storeBase + rec*8 + pos`.

**Encodings a modifier must respect** (the reader mirrors `read_data`'s exact byte facts, but
reconstructs the true *value* where the Rust twin emits packed parity bytes):
- **Scalar-vector fast lane — the zero-copy win.** `vector<scalar>` → a `Float32Array`/`Int32Array`/
  `BigInt64Array` **view straight over `mem.buffer`** (aliases wasm memory; hand to `gl.bufferData`
  with no repack). This is what makes `deliver` **O(1)** in element count for the inline path (proven
  structurally: the delivered node is a plain `vector`, the JS value's `.buffer === mem.buffer`).
- **Text** is interned inline: the field holds a string id; bytes at `id*8+8`, len at `id*8+4`;
  `STRING_NULL` (`id==0 || id>0x7fffffff`) → `null`.
- **Narrow ints** are PACKED: `Byte`/`ShortRaw` store `value - start` (add `start` back; read shorts
  UNSIGNED); nullable `Short` stores `value - start + 1` with `0` = null; `Int` is a direct i32
  (`start` is the null sentinel, not an offset). `start` (the range min) rides in the descriptor.
- **Value enum** discriminant is stored **1-based** (`variants[disc - 1]`; `0`/`255` → null).
- **DbRef** null is `rec==0 || store_nr==u16::MAX`.

**Keyed collections are PRE-FLATTENED, not cursor-walked** (the design flipped from the original
sketch). A hash/index/sorted/radix field has no byte layout a reader can walk, so at deliver time
loft materialises it to a scratch array (key-ordered, == the interpreter's `for x in coll`) and the
descriptor node becomes `FlatArray` (or in-place `Vector` for `sorted`, already inline-ordered). A
per-value `flat` map keyed by `(rec,pos)` lets one type-shared node serve every instance (a
`vector<Bag>` where each element owns a hash). JS reads them via the ordinary array path, blind to
the tree/hash/spatial structure. **Cost note:** pre-flatten is **O(n)** by intent — only the inline
fast lane is O(1); a keyed collection copies once at delivery.

**The borrow contract (the load-bearing safety invariant).**
- `deliver` is synchronous: read (or copy out) **within** the `loft_host_deliver` call; the borrow
  ends on return. loft must not grow/realloc/free the value during it.
- JS **re-derives `mem.buffer` on every read** — `memory.grow()` DETACHES the old `ArrayBuffer`, so
  a retained `DataView`/typed-array view is a use-after-detach. `readLoftValue` re-derives per call;
  a page must not cache a view past the borrow. (`tests/deliver_wasm.rs` forces a grow mid-read.)
- `expose` additionally PINS the value's store (`lock_store` — read-only, no free/resize/move) so its
  addresses stay valid across frames; the page holds a re-reader closure (`globalThis.loftExposed.
  get(tag)()`) for a fresh value each frame; `release` unpins. Cross-frame reads survive an asyncify
  yield (`tests/deliver_wasm.rs` reads an exposed value after a `store_load_url_trusted` unwind).

This is loft's own borrow model (`OWNERSHIP_MODEL.md`): the handle is a **View**, not an owned
transfer; the descriptor + linear memory are the backing store.

**Backends.** `--html` is the true zero-copy path (both page shells embed `readLoftValue` from
`doc/loft-deliver.js`); interpret/native run a `deliver_loopback` that prints the reconstructed bytes
as hex — a cross-backend parity **oracle** (interpret == native), not a value consumer, so it stays
lossy-but-stable for packed narrow ints. Consumer migration (routing's `view`/`match`, the games GPU
path) is @PLN105 Phase 4, owned by those consumers.

Code map: `src/ffi_deliver.rs` (deliver/expose/reconstruct) · `src/database/descriptor.rs`
(`LayoutDesc`/`read_via_descriptor`) · `doc/loft-deliver.js` (`readLoftValue`) · `default/02_files.loft`
(`deliver`/`expose`/`release`) · `tests/deliver_wasm.rs` + `tools/deliver_*.mjs` (the parity, grow,
cross-frame, and zero-copy-perf harnesses).

---

## Failure paths (what breaks, and what holds the line)

Enumerated because this is where the invariant earns its keep — each row is a
way the design degrades and the thing that prevents it.

- **Event-loop starvation.** A gather/wait loop that does not yield freezes the
  UI and never receives (the #450 class). *Held by:* the yield-and-accumulate
  rule; every blocking-looking library primitive must `frame_yield()` on its
  wait path. Worth a lint/test that a suspend-using `--html` program reaches its
  final line both visible and hidden (`tests/html_asyncify.rs` already does this
  for the engine).
- **Agnosticism leak into the engine.** A convenience host import grows a loft
  struct arg or a JSON-shaped parameter, and domain knowledge seeps into the
  generic layer. *Held by:* the engine surface stays closed and bytes/scalars
  only; a non-byte type in an engine host-import signature is the reviewable
  smell.
- **JS-coupling leak into a platform-blind lib.** A lib that is supposed to run
  on every target starts speaking JSON keys or DOM shapes, and silently stops
  being platform-blind. *Held by:* JS-fluency is allowed **only** in libraries
  that declare themselves browser-only; the platform-blind tier carries byte
  payloads (e.g. CBOR) and nothing JS-specific.
- **Bridge chattiness.** Reaching for the object-builder rung by default turns
  one logical result into dozens of boundary crossings. *Held by:* default to
  JSON-text; escalate only for zero-copy binary or a genuinely required live
  object.
- **Partial / reordered inbound data.** A request arrives split or batched
  across frames. *Held by:* the library owns reassembly/framing — exactly as
  `ztclient` correlates frames by id rather than taking the first off the wire.
- **Reentrancy.** Asyncify is one suspendable stack: the gather is cooperative
  and single-threaded, so a library cannot be mid-wait on request A and process
  request B on the same stack. *Held by:* this is the correct shape for a UI
  (one event at a time); the per-frame poll-and-accumulate loop interleaves
  naturally with render. Name it so no one expects threads.

---

## Verifiable build steps

Dependency-ordered; each step is independently shippable and pinned to **one
invariant + one runnable check**. The engine-side guarantee is structural:
**every step builds and passes against the stock installed loft with no `src/`
diff** — the suite going green *is* the proof that the model needs no engine
change. All the build work is the browser-target **library** (its `#native`
declarations + a `wasm/host.js` registered via `LOFT_WASM_EXTENSIONS`, modelled
on `web`); none of it edits loft core.

Verification reuses the shipped headless-Chromium gate pattern
(`tests/html_wasm.rs` loads + runs an `--html` fixture; `tests/html_asyncify.rs`
asserts a multi-suspend program reaches its final line **both visible and
hidden**; `tests/html_render.rs` + the CDP driver `tools/html_render_check.mjs`
drive input and inspect the page). Graduate each step's probe into a `tests/`
gate so it stays a regression.

**Step 1 — Push (loft → JS).**
- *Deliverable:* a browser-target lib with `#native fn emit(channel: text, payload: text);` + a `wasm/host.js` that routes `emit` to a JS sink.
- *Invariant:* a loft routine pushes arbitrary bytes to JS through a **library-declared** host import — no core change.
- *Check:* a headless `--html` page calls `emit` with a distinctive payload; the CDP driver reads the sink and asserts the bytes are identical.

**Step 2 — Poll (JS → loft), full round-trip.**
- *Deliverable:* `poll_len(channel: text) -> integer` + `poll_copy(channel: text, buf: text)` (the len + copy-into-`(ptr,len)` pattern) backed by a host.js inbound queue fed by a JS `send(channel, bytes)`.
- *Invariant:* loft pulls JS-enqueued bytes via len/copy; the write-into-wasm-memory direction works for an arbitrary library.
- *Check:* CDP enqueues a payload; loft polls + copies + echoes it back via `emit`; assert echo == sent (a JS→loft→JS round-trip).

**Step 3 — Gather-until-enough (the synchronous-pretend contract).**
- *Deliverable:* a lib `receive(channel: text) -> text` that loops `poll_len`/`poll_copy` + `frame_yield()` until a complete unit arrives, returning it synchronously (mirrors `ztclient`'s `poll_for`).
- *Invariant:* "blocking" = yield-and-accumulate; the gather never starves the event loop (no new suspend import — it reuses the existing `frame_yield`).
- *Check:* CDP sends a payload N frames after load; assert loft returns the complete unit **and** the page reaches its final line both visible and hidden (the `tests/html_asyncify.rs` gate shape). This is the critical starvation guard.

**Step 4 — Input services (poll-based: text / IME / clipboard).**
- *Deliverable:* `typed_text() -> text`, `ime_active() -> integer`, `clipboard_read() -> text`, `clipboard_write(text)` + a host.js backing (a hidden editable element + clipboard wiring). Poll-based, so the asyncify `--pass-arg` list is untouched.
- *Invariant:* the input surface covers text + clipboard for an arbitrary consumer, poll-based — and proves no engine `src/` change is needed for input.
- *Check:* CDP synthesizes keystrokes (`Input.dispatchKeyEvent`) into the hidden element; loft polls `typed_text` + emits it; assert == synthesized. Clipboard: CDP seeds the clipboard, loft reads + emits, assert. (IME *compose* is a visible-browser manual check; the scalar `ime_active` poll is verified headless.)

**Step 5 — JS-shaped data (the cost gradient).**
- *Deliverable:* lib routines that (a) serialize a loft structure to JSON text and `emit` it, and (b) `emit` a `(ptr,len)` the host.js wraps as a zero-copy `Uint8Array` view.
- *Invariant:* a browser-target lib emits JS-native structures with the JS-coupling confined to it — JSON the default, typed-array for binary.
- *Check:* CDP does `JSON.parse` on the pushed object and asserts fields; for binary, emit a known byte pattern and assert the `Uint8Array` view matches with no copy.

**Step 6 — Integration probe → permanent gate: clickable file-list + editable box.**
- *Deliverable:* a `--html` demo composing Steps 1–5 — a file list + an editable text box wired through the input service + byte channel, agnostic of any document model.
- *Invariant:* the full model works end-to-end for an arbitrary consumer; the engine's input surface actually covers text/clipboard in composition.
- *Check:* CDP synthesizes a click + typing; assert the page reflects the edit (a canvas-content gate à la `tests/html_render.rs`, or a pushed-state assertion). This is the afternoon spike, graduated to a regression gate — and it answers the one question the clean architecture cannot answer from the desk: *is text editing a deeper hole than text/IME/clipboard?* — **before** any real UI is built on top.

**Engine-side guarantee (the answer to "what's on the loft side").** Steps 1–6
build and pass against an **unmodified** loft binary; that is the verification.
The single conditional core touch the plan deliberately avoids: a *blocking*
(suspending) input or channel import would need its name added to the asyncify
`--pass-arg` list in the `--html` `wasm-opt` invocation (`src/main.rs`). Because
every service above is poll-based and reuses the shipped `frame_yield` suspend,
that list is never touched — verified by Steps 3 and 4 passing on stock loft. If
a future need forces a blocking primitive, that one-line list addition is the
*entire* engine change.

---

## Rejected alternatives (and when they would flip)

- **Export loft `pub fn`s as wasm exports + a JS→loft call ABI** (the
  "JS renders, loft computes" compute-core model). Solves *synchronous typed*
  JS→loft calls. Set aside because this model does not need them: loft owns the
  loop and the synchronicity lives inside the library via the yield. *Flips* if
  a future consumer genuinely needs synchronous typed JS→loft calls with loft
  *not* driving the loop.
- **wasm-bindgen / web-sys for the JS glue.** It automates the easy half (the JS
  marshalling helper, alloc/free, ptr/len packing) while leaving the actual work
  (loft-heap ↔ boundary bytes, which it cannot see), and it costs the
  self-contained `.html`, a pinned `wasm-bindgen-cli` toolchain, and a separate
  ES-module output. *Flips* only under the loft-drives-a-rich-DOM model (model B
  below), where wide bidirectional typed interop is the whole job.
- **loft drives the DOM directly** (a large DOM host-import surface). A much
  wider, typed, bidirectional bridge to maintain. Out of scope; the canvas +
  byte-channel model covers the consumer without it.

---

## See also

- [WEB_APPS.md](WEB_APPS.md) — the task-oriented on-ramp: which target to pick,
  the exact `--html` host surface, and one recipe per kind of web app. Start here
  if you are building a web app rather than designing the interop.
- [HTML_EXPORT.md](HTML_EXPORT.md) — the shipped `--html` pipeline: cdylib
  codegen, the WebGL2 bridge, the asyncify resume loop, the frame-yield contract
  this design builds on.
- [WASM.md](WASM.md) — the broader WASM runtime (VFS, host bridges, threading).
- [LAVITION.md](LAVITION.md) — why an agnostic engine-as-service-provider is the
  product thesis, not just a convenience.
- [plans/105-ffi-deliver-layout-bridge.md](plans/105-ffi-deliver-layout-bridge.md) —
  the closure record for the binary bridge (`deliver`/`expose`): phase log,
  decisions, and the falsifier suite behind § *The binary bridge* above.
