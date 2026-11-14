<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 98 — Live/debug tier: one primitive (the interpreter over the shared live store)

**Status — DONE / SHIPPED 2026-07-08.** P1 P2 P1b P3.1 P3.2 P3.3 P3.4 ✅ ALL LANDED, all probes
confirmed. **Closed by [loft PR #529](https://github.com/loft-lang/loft/pull/529)** — its body carries
`Closes @PLN98`, so on merge to `main` the `close-plans` workflow sets
[`@PLN98`](https://github.com/loft-lang/plans/issues/98) `status:finished` and closes it. This file is
now the **closed historical record** (design rationale + what shipped + how it was verified); the
living reference is the code + guards it points at below.

The full browser debug tier is proven end-to-end and headless: an agent debugs a `--html --debug` wasm
client THROUGH an engine_host server over WebSockets — breakpoint → pause → FULL-expression eval →
resume — verified by `wasm_debug_relay::agent_debugs_a_browser_wasm_client_through_the_server_relay`;
the one-shared-heap invariant (compiled ↔ interpreted agree) is confirmed on wasm (Probe 2).

> **Where it lives now (the living reference).** The primitive: `src/live_dispatch.rs`. The browser
> client: `src/wasm_debug.rs`. The server relay: `src/engine_host.rs`. The frame-eval primitive:
> `State::eval_frame_reenter` (`src/state/mod.rs`). Guards: `tests/wasm_debug_relay.rs`,
> `tests/html_wasm.rs`, `tests/rpc.rs`, `tests/debugger.rs`. Non-plan follow-ups (multi-file embedded
> source; the tier-wide default-lean release flip) are tracked separately, not here.

`subject:loft` · design-doc-first (Design Protocol 1). Consumers: the `@PLN16` debugger, the game /
`engine_host` loop, and `routing`'s offline `--html` build (its `loft-feedback.md` 2026-07-07).

Prior art this COMPLETES (not greenfield): `@PLN16` (debugger, `status:finished`) and `@PLN18`/`@I78`
(engine-host live tier — phase 08 `08-live-build-swap.md` shipped for native). The primitive is proven;
this plan closes the last eval increment, adds the browser tier, and makes the whole thing opt-in.

---

## The invariant (one sentence)

> **The live/interpreted path — whether RUNNING an edited function or EVALUATING a debug expression —
> operates on the ONE live `Stores` the compiled program uses, by direct reference: never a copy, never
> a re-serialisation of its state.**

Two facets of one principle (one live heap, no copies):
- **Execution** — a flipped/edited function re-enters the interpreter over the program's own `Stores`
  (a swap into the parked `State`, not an alias/copy). **Shipped** (`live_dispatch`).
- **Eval** — reading a value out of a paused frame reads the **live store record**, not a reconstructed
  text literal of it. **This is where the design is violated today** — see F1.

Every capability anyone asked for — breakpoint `eval`, `setValue`, hot-reload, a game's on-the-fly
script edit, a browser live-edit — is a use of that one principle. The plan is **make the principle hold
everywhere it currently doesn't**, not build three features.

## Why this is a design, not a feature list (the tell)

Three separate-looking asks — an agent debugger (`@PLN16`), the game changing scripts without a restart,
`routing`'s offline browser build — all reduce to the same sentence: "run/evaluate against the live heap
without stopping the program." One mechanism (`live_dispatch`) already realises it for native. The
design's job is to keep `N` (re-assertion sites) at ~1 and repair the one place the invariant leaks.

## The primitive that exists (grounding, with refs)

`live_dispatch` (`src/live_dispatch.rs`, `@PLN18`/`@I78`):

- A `--native` binary under `LOFT_LIVE_FLIP=1` (`live_dispatch.rs:101`) bootstraps a **parked
  interpreter** `State` from the same sources (`bootstrap`, `:112-205`; reads `LOFT_LIVE_SRC/_STDLIB/
  _LIBS` off disk, `:113-126`).
- **The program's `Stores` IS the bootstrap's world** — swapped into `state.database` around each
  dispatched call (`State::reenter`/`reenter_ret`), not aliased or copied.
- Every generated user fn opens with a one-atomic-load `live_flipped(idx)` check
  (`Emitter::live_entry_check`, `src/generation/mod.rs:1201-1241`, emitted at `:3252`); cold cost off is
  a single relaxed load (`live_dispatch.rs:73-85`).
- **Flip** routes a fn's calls into the interpreter (`live_call_void/_i64/_ref/…`) over those stores.
- **Tier-0 live reload** (`src/live_reload.rs`): `poll` runs inside the execute loop every ~32k ops
  (`state/mod.rs:3632-3651`, `RELOAD_POLL_OPS`), diffs `fn`-blocks, and `reload_fn` repoints
  `state.fn_positions[d_nr]` + the recorded call sites (`live_reload.rs:386-398`) — the flipped fn then
  runs the **new** body over the **live** state. Also an explicit `reload` control command.
- A **breakpoint** auto-flips the target fn (a compiled body cannot pause — `debug_apply_bp` →
  `set_flip_in`, `live_dispatch.rs:399-413`) and runs it under the pause loop (`debug_pause_loop`,
  `:444-508`).
- **Failure posture:** every bootstrap problem WARNS and falls back to compiled. Single-threaded by
  construction.

**Status honesty:** native execution/flip/reload is **shipped** (`@PLN18` 08, `tests/engine_host_reload.rs`);
the native debugger is **shipped** (`@PLN16`, incl. D2 "live-frame eval, first cut"). So the game's
"change scripts on-the-fly" **already works on native today** (flip + `LOFT_LIVE_RELOAD=1` or the
`reload` control command). The remaining work is three specific gaps.

## The three consumers, one mechanism

| consumer | a "live action" is… | status |
|---|---|---|
| **Debugger** (`@PLN16`, `--rpc`) | breakpoint = flip + pause; `eval` = evaluate against the paused frame; `setValue` = write a frame var | breakpoints/step/frame-read **shipped**; **`eval`/`setValue` broken on any frame with a heap (vector/keyed) local** — F1 |
| **Game on-the-fly scripts** | edit a `.loft` fn → `fn_position` repoints → next call runs the new body over the live world | **shipped (native)**; note the game control-channel `eval` is a live `hit.locals` lookup (`live_dispatch.rs:464-471`), NOT the reconstruct path, so it does NOT hit F1 |
| **Offline / browser** | the same parked interpreter compiled into the `--html` module, sources embedded, cooperative pause | **not wired** — `loft_start` (`generation/mod.rs:1710`) bypasses the tier entirely; hard blockers in F2 |

## Failure paths (the design's core — write how it breaks first)

### F1 — `eval` re-serialises the frame through a lossy text round-trip (the invariant violation)

`debug_eval` has two paths (`src/repl.rs:2277-2314`):
- **(A) live read** — a bare identifier that names a heap local is read straight from the paused store
  via `State::eval_frame_heap` (`state/mod.rs:2294-2327`). **Honours the invariant; works.**
- **(B) reconstruct** — ANY non-bare expression (`2 + 2`, `len(v)`, `v[0]`, `s.f`) falls through
  (`repl.rs:2298-2313`): it binds **every** captured local into a text `prefix` and compiles/runs
  `fn replmain() -> τ { <prefix> <rhs> }` on a throwaway `State` (`capture_typed`, `:2697-2730`).

The captured literals come from `render_frame_local` (`state/mod.rs:3187-3296`), whose heap arm
(`:3263-3282`) renders with **`show_loft_bounded(…, depth 2, 8 elems)`** — a **bounded** render that
emits non-reparseable truncation tokens: `,...` past 8 elems (`database/format.rs:1258`), `[...]` past
depth (`:1231`), `{...}` for a struct past depth (`:1080`); keyed collections hit the catch-all
`other => "<…>"` (`mod.rs:3294`). Any realistic heap local → a literal like `[1,…,8,...]` that **fails
to parse** → `Capture::Skip` → `value_of_fmt` returns `None` → `eval` replies `value:null`. Because the
poison sits in the **shared prefix**, it kills eval of **every** `rhs`, including `2 + 2`. `debug_set`
calls `debug_eval` first (`repl.rs:1923-1925`), so `setValue` fails the same way — even for a scalar.

**This is not a random bug: it is the one place the design's invariant is violated.** Path B copies the
frame OUT to text and back IN through a lossy channel, instead of reading the live store. The true
trigger is "any frame local whose captured literal doesn't round-trip through the parser" — not vectors
specifically. `@PLN16` README:280 already names it: *"a heap field path still routes through the
reconstruct path → null for a vector field… Extending the live read down a field/index path is the next
increment."* So F1 = **finish extending path A to seed heap locals for path B.**

*Minimal repro (routing, verified):* delete `v` and `eval "2 + 2"` → `4`; keep it → `null`.
```loft
fn main() { v: vector<integer> = [1, 2, 3]; x = 2 + 2; println("{x} {len(v)}"); }
```

### F2 — the browser bypasses the tier + four hard blockers

`--html` runs **fully compiled**: `loft_start` (`generation/mod.rs:1710`) does `Stores::new(); init; n_main`
— it never calls `boot_stores`, so there is no parked interpreter, no flip, no breakpoints. The
interpreter `State` itself *computes* on wasm (cfg stubs exist, `live_dispatch.rs:437-438`), but four
things are `--native`-only and must be replaced for the browser:
1. **Source delivery** — bootstrap reads `LOFT_LIVE_SRC` + `parse_dir` off disk; the browser has no fs.
   Must bootstrap from **bytes embedded in the wasm bundle**, delivered via the shipped `host_input`
   channel (loft#476). (`--html` host imports are print + GL + `host_input` only — `HTML_EXPORT.md:59-92`.)
2. **The pause loop blocks a thread** — `thread::sleep(2ms)` (`live_dispatch.rs:506`); a browser is a
   single-threaded event loop. Must become a **cooperative yield**.
3. **The control transport is loopback TCP** (`engine_host.rs:1908-1961`, `#[cfg(not(wasm32))]`). Browser
   needs a **`postMessage`/WebSocket** control channel.
4. **`std::env`/`std::fs`/child-process/rustc** (native rebuild+swap S4/S5) are unavailable — out of
   scope for the browser by design (`03-wasm-tier.md`).

None require wasm to *compute* something it can't; they require a different bootstrap + transport + a
non-blocking pause. The interpreter is the reusable piece.

### F3 — the entry check has no opt-out (packaging)

`live_entry_check` is emitted for **every** user fn unconditionally (`generation/mod.rs:3252`, gated only
by fn *shape*, never a build flag), and the `LOFT_LIVE_FNS` table + `boot_stores` call are always emitted
(`mod.rs:1807-1818`). `LOFT_LIVE_FLIP` is a **runtime** env, not compile-time. There is **no** cfg to
strip the guards/table for a lean release. Opt-in means adding one codegen gate around those emission
sites (and, for wasm, the interpreter's inclusion).

## Re-assertion sites — `N` is small (keep it there)

- **World-swap** funnels through `live_call_*` — one chokepoint, `N ≈ 1`. Good; unchanged.
- **The eval seam** is the single reconstruct path (`repl.rs:2298-2313`). F1 is that this one seam copies
  through text. The fix belongs at that one seam — route heap locals through the live read — **not** a
  per-value-kind literal-widening spray. If the fix wants to teach `render_frame_local` to emit
  unbounded round-trippable literals for *every* kind, that is the alarm: it re-serialises (still a copy)
  instead of reading the live store, and it will re-break on the next non-round-trippable kind. Prefer
  the live read; the unbounded-literal seed is the fallback only where a live read is genuinely
  unavailable.

## The design, per axis

### A1 — fix `eval`/`setValue` (F1): seed only referenced locals, heap ones live+unbounded ✅ LANDED (first cut)

**Done (2026-07-07)** in `debug_eval_fmt` (`src/repl.rs`). Two changes, one seam:
1. **Seed only the locals the expression NAMES** (`expr_idents`) — not every frame local. This was the
   real root: seeding *all* locals let one whose literal isn't loft source poison the whole parse. The
   actual poison was the vector's compiler backing `__vdb_N` (renders `main_vector<…>{…}`), so even
   `2 + 2` (no local) returned null. Now an unrelated heap/keyed local can never break an expression.
2. **Render a referenced heap local live + UNBOUNDED** via `eval_frame_heap` (path A's render) instead
   of the bounded display literal, so a >8-element user vector round-trips too. Scalars fall back to
   the captured literal.

- **Verified:** in a vector-local frame `eval "2+2"`→4, `len(v)`→3, `v[0]`→1, `v[1]+x`→25; a 10-element
  vector `v[9]`→100; struct `s.a+s.b`→16; `setValue x=42` accepted → `continue` prints the edited run.
  Regression guard `tests/rpc.rs::rpc_eval_and_set_in_a_vector_local_frame`; 38+10 debugger tests green;
  backend-independent (the REPL debugger always interprets).
### A1b — P1b: the true live-frame eval closes the keyed-collection residual ✅ LANDED

**Done (2026-07-07)** — a referenced keyed collection (`hash`/`sorted`/`index`) no longer degrades to
`null`; it is read **live, where it lives**. The seam is `eval_frame_expr` (`src/repl.rs`) →
`State::eval_frame_reenter` (`src/state/mod.rs`), reached from `debug_eval_fmt` before the text-seed
path. Mechanism (the invariant-honouring form the plan called for):
1. **Bind each referenced keyed local as a typed ARG** of a synthetic `fn __eval(k1: K1, …) -> RT { … }`
   — `keyed_type_source` renders the parseable `hash<Ent[k]>` (not `Type::name`'s Debug `["k"]`). The
   local's **live `DbRef`** is pushed straight into the call, so the collection is read in place, never
   text-reconstructed. Other referenced locals stay in the seed prefix (scalars/vectors/structs
   reparse fine).
2. **Append-only compile onto the PAUSED state** (like `live_reload`): `def_code` at
   `bytecode.len()`, `fn_positions` extended — the paused frame's PC + stack slots are untouched, so
   `continue` resumes correctly. `reenter_ret` allocates the eval frame above the high-water mark and
   pushes the args; the call stack + watermark are snapshotted/restored (the parked-vs-paused
   difference — the eval callee's return pops a frame `reenter_ret` doesn't push back).
3. **Return path:** scalars ride the frame base and are read straight back; a **heap** result
   (struct/vector) is destination-passed (never lands at the base), so it is serialised in-fn via
   `.to_json()` — a call-returned-owned text safe past teardown (@P293) — and the raw JSON is returned.
- **Verified** (`tests/rpc.rs::rpc_eval_in_a_keyed_collection_frame`): in a `hash<HRec[name]>` frame
  `h["a"].v`→7, `h["b"].v + x`→14, `len(h)`→2, `h["a"]`→`{"name":"a","v":7}`; `2 + 2`→4 (the text path
  is untouched); `continue` prints correctly (the paused frame survives the eval). Full suite green
  (2628/2630; the 2 fails are the browser/wasm env gate). The P1 vector matrix still passes.
- **Residual (tiny):** a *bare* keyed-frame result whose type has no `.to_json()` (an odd non-struct
  heap, or a genuine borrowed `text` field) still falls back to `null` — the @P293-safe boundary; the
  common struct/scalar cases are covered.

### A2 — the browser live/debug tier (F2) — DECOMPOSITION (P3)

**The reframing (grounded 2026-07-07):** the four "hard blockers" are largely **already solved by
existing machinery** — the browser tier is mostly *composition*, not new hard mechanisms:

- **The make-or-break blocker (the cooperative pause) already exists.** `src/wasm.rs:886-897`: a
  `--html` GL app sets `frame_yield`, RETURNS control to the JS event loop, and the session is stored
  for `resume_frame` — a full yield-and-resume across the browser event loop. The coroutine machinery
  (`CoroutineFrame`, `Suspended`, serialised stack — `state/mod.rs:72-105`) shows mid-call suspend/
  resume is supported. A debug pause is the SAME model: yield at a breakpoint instead of a frame
  boundary. So the pause is not "fundamentally impossible" — it reuses `frame_yield`/`resume_frame`.
- **The control transport already exists.** `host_input` (loft#476, shipped): `loft_host_input_len`/
  `loft_host_input_copy` + the JS input queue `inQ` (`src/main.rs:5907-5908`). JS→wasm control frames
  ride it; wasm→JS results ride `loft_host_print` (or a tagged debug channel).
- **Source delivery = embed bytes.** Emit the program + stdlib as static blobs and `bootstrap_from_bytes`
  (no env/fs) — a small addition beside `bootstrap` (`live_dispatch.rs:112`).
- **`loft_start`** (`generation/mod.rs:1717`, `Stores::new(); init; n_main`) gains a live variant that
  bootstraps the parked interpreter from the embedded bytes.

Native rebuild/swap stays out; `03-wasm-tier` (per-fn wasm module promotion) is parked/superseded — this
is the *interpreter* tier, not module relink.

**Sub-phases (dependency order):**

- **P3.1 — wasm bootstrap from embedded source.** Under the live build, emit `static LOFT_SRC` +
  `LOFT_STDLIB` byte blobs; add `bootstrap_from_bytes(stdlib, src)` beside `bootstrap` (share the parse
  path, drop the `LOFT_LIVE_SRC`/`parse_dir` fs reads); a `loft_start_live` that calls it + parks the
  `State`. **Native-testable, no browser.** *Probe:* a native `bootstrap_from_bytes` round-trip parks a
  world byte-identical to `bootstrap`'s (compare defs + fn_positions).
- **P3.2 — cooperative-yield pause (reuse `frame_yield`).** At a breakpoint in a wasm live build, set a
  debug-yield (mirror `frame_yield`), return through the existing yield path, store the session for
  `resume_frame`; a control command (`resume`/`eval`) drives `resume_frame`. *Probe (the load-bearing
  one) — ✅ CONFIRMED FEASIBLE by code-read (2026-07-07):* loft's interpreter keeps its CALL STACK in
  the `State`, not the Rust stack, so `execute_argv` unwinds on `frame_yield` with the loft stack
  preserved and `resume_frame` re-enters exactly where it left off (`wasm.rs:884-897`) — a genuine
  MID-call suspend/resume, already proven every frame by `--html` GL apps. The debug pause is the same
  yield, triggered at a breakpoint. So P3 is not blocked on an "impossible" pause.
- **P3.3 — control over `host_input`.** JS pushes `flip`/`bp`/`eval`/`resume`/`reload` frames into `inQ`;
  the wasm debug pump reads them via `host_input` and applies them through the SAME
  `debug_cmd_dispatch` frame parser the TCP channel feeds today (`engine_host.rs:1980-2037`); results/
  frames out via `loft_host_print` or a tagged debug output. *Probe:* debug frames vs program input on
  one queue need a tag/second channel — confirm they don't collide.
- **P3.4 — the JS debug driver + opt-in + acceptance.** A JS driver (extend `doc/loft-gl-wasm.js` or a
  debug variant) wiring the control channel ↔ the DAP-ish `--rpc` surface; `loft_start` opts into the
  tier under the live flag. *Acceptance:* `routing`'s headless-Chromium harness — in a `--html` live
  build, breakpoint + `eval` a vector expr (needs P1) → the value; edit a fn → the live world picks up
  the new body; a compiled write then an interpreted read of the same var agree (one heap, not two).

**Recommended order:** P3.1 (foundation, native-testable) → P3.3 (control, reuses shipped channel) →
P3.2 (the pause — mid-call resume already confirmed feasible above) → P3.4 (driver + headless
acceptance).

### The implementation ladder — every step natively verified

Two facts make this cheap: (1) the *interpreter side* of the browser tier — bootstrap, pause, control —
is all **native-testable**; ONLY the JS integration (P3.4) needs headless-Chromium. (2) the
**live-frame eval** primitive is **shared** — it closes P1b AND powers the browser `eval`, so build +
verify it once, natively. Each step is one commit with its verification green before the next.

**P1b / shared — the live-frame eval primitive ✅ LANDED (see § A1b above).**

**P3.1 — embedded-source bootstrap ✅ LANDED (2026-07-07):**
1. ✅ Refactored `bootstrap` → shared `bootstrap_core`; added
   `bootstrap_from_bytes(fn_names, program_src)` (the stdlib is the LINKED
   `stdlib_sources::STDLIB_SOURCES` — no need to pass it) + `Parser::parse_source` (the
   fs-free, all-targets twin of `parse`). **Verified**
   (`live_dispatch::tests::bootstrap_from_bytes_parses_a_fs_identical_world`): embedded parse parks the
   SAME def-count as the fs path, `n_main` resolves; `bootstrap_from_bytes_ok` runs the full
   parse+byte-code+park.
2. ✅ Codegen emits `static LOFT_SRC: Option<&str>` under the live build (`Output::program_src`, set
   from the program file in `main.rs`); the boot call is `boot_stores(LOFT_LIVE_FNS, LOFT_SRC)`.
   **Verified**: `loft --native-emit x.rs prog.loft` → the blob + program text are present and `rustc`
   compiles it (`loft --native prog.loft` → correct output).
3. ✅ `boot_stores` prefers fs when `LOFT_LIVE_SRC` is set (arms tier-0 reload) and the EMBEDDED bytes
   otherwise. **Verified** end-to-end: the cached `--native` binary run with `LOFT_LIVE_SRC` UNSET,
   `LOFT_LIVE_FLIP=1 LOFT_FLIP_FNS=addup` → `live-flip: n_addup -> interp`, `live-dispatch: n_addup #1`,
   correct result — the embedded-parked interpreter dispatches a flipped fn with no filesystem source.
   *(The design differs from the plan's original "emit a `LOFT_STDLIB` blob too": the stdlib is baked
   into `libloft` via `STDLIB_SOURCES`, which the generated program links — so only `LOFT_SRC` needs
   emitting. Multi-file/lib programs embed only the main file today; libs stay the fs follow-up.)*

<details><summary>original P3.1 plan (superseded by the above)</summary>

3. The bootstrap entry prefers the embedded blobs (falls back to fs). **Verify** (native): a `--native`
   live build runs correctly via the embedded bootstrap (`LOFT_LIVE_SRC` unset); def-count byte-identical
   to the fs path.

</details>

**P3.2 — cooperative pause ✅ LANDED (2026-07-07):**
The mechanism ALREADY EXISTS and needs no new `debug_yield` flag. `execute_argv` at a breakpoint (in
stepping mode) calls `debug_check`, which stashes the frame in `debug.paused` and RETURNS from the
execute loop (`state/mod.rs:3849`) — a cooperative yield: control goes back to the caller with the
State-held stack preserved, NOT the blocking `debug_pause_loop`. `debug_step(Continue)` re-enters the
same loop and runs to completion. The signal is `is_paused()` (`= debug.paused.is_some()`), which the
wasm session checks alongside `frame_yield`. So the browser reuses the REPL's model verbatim — a flag
mirroring `frame_yield` would be redundant.
- **Verified** (`tests/debugger.rs::cooperative_pause_yields_control_then_resumes_to_completion`, State
  level — exactly what a wasm session drives): `execute_argv` yields at the breakpoint (`is_paused`,
  frame with `n==40` capturable); `debug_step(Into)` resumes one step and computes `m==42` correctly;
  `debug_step(Continue)` runs to completion cleanly. The eval-at-pause half (`eval "2+2"` → resume →
  correct captured output) is `tests/rpc.rs::rpc_launch_break_eval_continue`, which drives the same
  `eval_observe`→`debug_eval_json`→`debug_continue` API the browser driver wraps.

**P3.3 — control over `host_input` ✅ ROUTING CORE LANDED (2026-07-07); wasm wiring → P3.4:**
The load-bearing invariant — debug control + program input share ONE queue without colliding — is the
`D!:` tag. `engine_host::route_input_frame(cid, msg)` classifies one message off the shared channel: a
`D!:`-tagged frame is stripped and dispatched through the SAME `debug_cmd_dispatch` core the TCP channel
feeds (`InputFrame::Debug(reply)`); anything else is `InputFrame::Program(msg)` for the program's
`host_input()`.
- **Verified** (`engine_host::p98_p33_tests::debug_frames_route_off_the_shared_input_channel`): program
  input passes straight through; a `D!:bp some_fn` frame reaches the `bp` arm (accepted → queued on the
  debug mailbox, `D:ok`); an unknown `D!:` command still reaches the dispatcher (`D:err unknown`) — so
  strip+dispatch, not a bare prefix check. Each consumer gets exactly its own frames.
- **Remaining (P3.4 scope):** `debug_cmd_dispatch` + the mailbox/`debug_set_bp` machinery are
  `#[cfg(not(wasm32))]` today (they assume threads / a TCP peer / process-spawn for rebuild). Wiring
  `route_input_frame` into the wasm `host_input_native` needs that core un-gated (or a wasm subset) for
  `wasm32` — that lands with the browser driver, since it can only be exercised there.

**P3.4 — opt-in + driver + acceptance:**

> **Transport decision (2026-07-07, user):** the debug driver is NOT an in-page JS harness — it is the
> **server relaying over the WebSocket the client already holds.** An agent is close to the *server*
> (it already speaks the `--rpc` / `D!:` surface); the server forwards `D!:` frames to a target client's
> socket and relays the client's debug output back. The client applies them locally via
> `route_input_frame` (P3.3) → its own `debug_cmd_dispatch` → the parked interpreter. This reuses TWO
> existing channels, needs no headless-Chromium to *drive* (only a connected browser), and is why
> P3.2's cooperative pause is load-bearing: the client must yield to its JS event loop so the socket
> can deliver `D!:resume`. Debug is **opt-in per client** (a production client ships none), and the
> opt-in carries a **debug name** the server uses to address that client.

1. **The opt-in ✅ LANDED (2026-07-07):** `--debug[=name]` includes the browser debug tier; a production
   `--html` client is debug-OFF by default (flipped from the old default-live). `Output.emit_live =
   debug_name.is_some() && !lean` for `--html`; `emit_wasm_start` emits either a plain `Stores::new()`
   `loft_start` (production) or, for a debug client, one that bootstraps the parked interpreter from the
   EMBEDDED `LOFT_SRC` (`bootstrap_from_bytes`, P3.1 — no fs in a browser) and bakes
   `static LOFT_DEBUG_NAME` for server addressing. **Verified**:
   `generation::p98_p34_tests::wasm_start_gates_the_debug_tier_on_the_opt_in` (production = plain boot,
   no tier/name; `--debug=alice` = embedded bootstrap + `LOFT_DEBUG_NAME="alice"` + the source blob);
   and the real builds — `loft --html` → 170 KB *engine-less* shell, `loft --html --debug=alice` → 3977
   KB *full-engine* shell that **compiles to wasm** (so the debug `loft_start` + `bootstrap_from_bytes`
   are valid on `wasm32`).
**The tier CORE is now verified ON wasm (2026-07-08).** The `--html --debug` client runs headlessly
under Node (`tools/wasm_repro.mjs` / `wasm_debug_selftest.mjs`) — the wasm32 toolchain works here, and
Chromium works unsandboxed (`html_asyncify` passes). Two tests prove the interpreter tier runs on wasm:
- `html_wasm::html_debug_tier_runs_on_wasm` — a compiled call DISPATCHES into
  the interpreter over the shared store in wasm (`flip_all_dispatch_debug`; the one primitive).
- `html_wasm::html_debug_tier_runs_on_wasm` — the P3.2 breakpoint pause + frame read +
  resume run identically on wasm32 (`PAUSE n=40 STEP m=42 DONE=true`, via `wasm_debug_selftest` + the
  `loft_debug_selftest` export). So the make-or-break cooperative pause is CONFIRMED on wasm, not just
  native.
- Also fixed en route: `emit_native_main` spawned a large-stack thread unconditionally → every wasip2
  binary aborted (`Os code 58`); now inline on `wasm32`, thread only on native (commit d73d2df0).

2. **Client debug CONTROL wiring ✅ LANDED (2026-07-08).** `src/wasm_debug.rs` is the browser client: the
   program runs INTERPRETED over a parked `State`, breakpoints pause it cooperatively (P3.2 — no block),
   `D!:` control frames arrive over `host_input` and `D:` replies go over host output. It uses the
   `State`'s own debug methods directly (NOT the native `debug_cmd_dispatch` / TCP machinery), so the
   whole path is wasm-safe. `apply(session, frame)` (bp / run|resume|step / eval) is native-unit-tested;
   the `--html --debug` build exports `loft_debug_start` + `loft_debug_pump`. **Verified on wasm**
   (`html_wasm::html_debug_tier_runs_on_wasm`): `D!:bp compute`→`D:ok`;
   `D!:run`→`D:hit compute n=40`; `D!:eval n`→`D:eval n=40`; `D!:resume`→`D:terminated`.
3. **Server relay ✅ LANDED (2026-07-08).** The kernel's `D!:` handler gained: `D!:iam <name>` (client
   registers its debug name), `D!:@<name>:<cmd>` (a loopback agent forwards to that client's socket +
   records the reply route), `D!:reply <msg>` (client → the requesting agent). Local server-authoritative
   debug still routes to `debug_cmd_dispatch`. **Verified**
   (`engine_host_kernel::server_relays_debug_frames_to_a_named_client`).
4. **Full acceptance ✅ LANDED (2026-07-08), headless.** `wasm_debug_relay::agent_debugs_a_browser_wasm_
   client_through_the_server_relay` runs ALL real components — a `loft --html --debug` wasm client in
   Node (JS driver `tools/wasm_debug_e2e.mjs` bridging its WebSocket ↔ host I/O), a real `engine_host`
   server with the relay, and an agent addressing the client by name: `D!:@alice:bp compute`→
   `D:ok bp compute`; `run`→`D:hit compute n=40` (the CLIENT paused); `eval n`→`D:eval n=40`;
   `resume`→`D:terminated`. The complete chain agent → server → browser wasm client → back, proven
   headlessly.
5. **Full-expression eval on the client ✅ LANDED (2026-07-08).** The client's `eval` evaluates ARBITRARY
   expressions over the paused frame (`n + 2`, `n * n`, `h["a"].v`, `len(v)`, `s.field`), not just a
   frame-local read: it binds every referenced live local as a typed arg of a synthetic fn and reenters
   over the paused frame (`wasm_debug::eval_via_reenter`), reusing the P1b `eval_frame_reenter` primitive
   — now GENERALIZED to push any arg type (heap `DbRef` + integer/float/bool/char/enum), not just keyed
   `DbRef`s. `State::frame_local_arg_type` renders any local's parseable type (`None` for `text` — a
   borrowed `Str` arg is @P293-unsafe → graceful fallback). No `repl`/`rpc` dependency, wasm-safe.
   **Verified**: `wasm_debug::tests` (native — `n + 2`=42, `n * n`=1600, `2 + 3`=5) and the wasm +
   full-relay tests both now eval `n + 2`→42 end-to-end (agent → server → browser client).

### A3 — packaging: `--lean` opt-OUT, default LIVE (F3) ✅ LANDED (P2)

**Shipped as an opt-OUT, not opt-in — the default stays LIVE so nothing existing changes.** One
codegen flag on `Output` (`emit_live`, default `true`) gates two emission sites: the per-fn
`live_entry_check` (`generation/mod.rs:3252` — when off, the check is skipped AND `live_fns` stays
empty because its sole producer never runs) and the `LOFT_LIVE_FNS`/`boot_stores`/`live_enabled`
machinery in `emit_native_main` (`mod.rs:1800`, two template branches selected by `emit_live`). The
CLI flag `--lean` (`main.rs`) flips `emit_live = false` at all three build `Output::new` sites —
`--native`, `--native-wasm`, `--html`. A lean `main` bootstraps a plain `Stores::new()`, runs `init`
+ the leak check UNCONDITIONALLY, and references no `live_dispatch` symbol at all. `LOFT_LIVE_FLIP`
stays the runtime activation within a live-capable (default) build.

- **Why opt-out, not opt-in:** flipping the shipped default to LIVE-on keeps every consumer working
  untouched; `--lean` is the deliberate "smallest release binary, no live-flip/breakpoints" choice.
  **Making lean the DEFAULT is a release-policy follow-up**, gated on the live-tier consumers (the
  game's on-the-fly scripts, `serve`, and the `@PLN16` debugger) explicitly opting the tier back IN
  with a flag — until then default-live is the non-breaking posture.
- **Acceptance (met — probe 3):** a lean `--native` build's emitted Rust has **zero**
  `live_flipped|LOFT_LIVE_FNS|boot_stores|live_enabled|live_dispatch` (grep = 0); the default build
  has them (`live_flipped` count > 0) and the tier works; `--native --lean` compiles via rustc and
  runs. Still TODO: record the size/perf delta (esp. the `--html` wasm module, ~1.1 MB today; the
  bundled interpreter is the cost the opt-out removes) and wire `emit_live=false`'s effect on the
  browser interpreter inclusion (A2).

## Probes (falsify before building)

1. **"F1 is the eval seam copying through text, fixable at one place."** ✅ largely confirmed by the map
   (path B vs path A; `@PLN16` README:280). Remaining probe: read `eval_frame_heap` and confirm a
   field/index live-read composes without a text round-trip.
2. **"Sharing is one heap on wasm too."** ✅ CONFIRMED (2026-07-08). A compiled write and an interpreted
   read of the same var AGREE, both directions, on wasm — the swap carries the world. In a `--html
   --debug` build (`flip_all` flips `reader`+`writer`, `main` stays compiled): main writes `w.a=777`
   (compiled) → `reader` reads it (interpreted) → `r=777`; `writer` writes `w.b=999` (interpreted) → main
   reads it (compiled) → `b=999` — identical to the all-compiled baseline. Guard:
   `html_wasm::html_debug_tier_runs_on_wasm` (+ native
   `LOFT_FLIP_FNS=reader,writer`).
3. **"Opt-in is zero-cost-absent."** Grep the lean build's emitted Rust for `live_flipped` → must be 0.

## Out of scope (declared)

- A language-level debug API/syntax (host/runtime capability only).
- Changing interpreter eval *semantics* (it evaluates as `--interpret` does).
- Multi-threaded live flip; native rebuild/swap in the browser; per-fn wasmtime tier (`03`, parked).

## Phase order

**P1 (A1) — fix the evaluator.** ✅ LANDED (first cut) — `eval`/`setValue` work in heap-local frames;
the invariant violation for round-trippable values is closed; residual P1b (referenced keyed
collections → the true live-frame eval). **P2 (A3) — the `--lean` opt-OUT flag.** ✅ LANDED —
`--lean` strips the tier from `--native`/`--native-wasm`/`--html`; default stays LIVE (non-breaking).
The boundary every other piece plugs into. **P3 (A2) — the browser tier.** DECOMPOSED + DE-RISKED
(2026-07-07): mostly *composing* existing machinery — the cooperative pause reuses `frame_yield`/
`resume_frame`, control reuses the shipped `host_input` channel — not four hard mechanisms. Sub-phases
P3.1 (embedded-source bootstrap, native-testable) → P3.3 (control over `host_input`) → P3.2 (the pause;
prototype the mid-call `resume_frame` probe FIRST — it gates feasibility) → P3.4 (JS driver + headless
acceptance). Each has its own probe/acceptance in § A2.
