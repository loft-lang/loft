# WASM frame yield — browser game loop via interpreter suspension

Split out of [WASM.md](WASM.md): the frame-yield design for the browser game loop.

## Frame Yield — Browser Game Loop via Interpreter Suspension

### Problem

The loft interpreter runs synchronously to completion (`execute_argv` is a tight
`while code_pos < len` loop).  In the browser, `requestAnimationFrame` is the only
way to render frames and receive input.  A loft game loop like:

```loft
for _ in 0..3600 {
  if !gl_poll_events() { break }
  gl_clear(bg);
  gl_draw(vao, count);
  gl_swap_buffers();
}
```

executes all 3 600 iterations in a single synchronous WASM call, blocking the
browser's main thread for the entire duration.  Only the last frame is visible.
No input is processed.

### Design: yield at `gl_swap_buffers`

The interpreter pauses mid-execution at frame boundaries and returns control to
JavaScript.  JavaScript calls `requestAnimationFrame` and resumes the interpreter
on the next frame.  Loft game code is **identical** on native and browser — no
API changes, no callback pattern, no user-visible generators.

> **Two asyncify suspend imports (@PLN84 ZT-C).**  The `--html` `wasm-opt
> --asyncify --pass-arg` now names **two** import functions (comma-separated):
> `loft_gl.loft_gl_swap_buffers` (the GL frame-yield) and `loft_web.ws_yield`
> (the WebSocket frame-yield).  The `web` library's `frame_yield()` lowers to
> `loft_web.ws_yield`, so a synchronous WS poll loop returns to the JS event loop
> between iterations — without it `WebSocket.onmessage` never fires (the @P337
> deadlock applied to sockets).  `frame_yield()` does NOT call swap_buffers, so it
> needs its own suspend point — a non-GL WS client has no GL window.  The headless
> verify driver `tools/wasm_ws_repro.mjs` drives this suspend/resume loop on a
> `setImmediate` (the Node rAF equivalent); see `plans/84-zt-c-web-ws-bridge.md`.

### Alternatives considered and rejected

| Approach | Why rejected |
|---|---|
| **JS-driven frame callback** (`fn frame(dt)` pattern) | Forces a different programming model; user must manage state between calls; native loop code doesn't work in browser |
| **Web Worker + SharedArrayBuffer + Atomics.wait** | Requires COOP/COEP headers; WebGL context can't be used from a Worker (all GL calls must proxy to main thread); Safari support unreliable; massive complexity |
| **Coroutine-based game loop** | Requires user to write `for frame in game_loop() { yield }` instead of a natural loop; awkward API that doesn't match native expectations |

### Architecture

```
                         ┌─── Browser main thread ───┐
                         │                           │
  compile_and_run()      │   requestAnimationFrame   │
         │               │          │                │
         ▼               │          ▼                │
  ┌─ execute_argv ─┐     │   ┌─ resume_frame ─┐     │
  │  ...opcodes... │     │   │  clear yield    │     │
  │  gl_clear()    │     │   │  ...opcodes...  │     │
  │  gl_draw()     │     │   │  gl_clear()     │     │
  │  gl_swap()     │     │   │  gl_draw()      │     │
  │  ► YIELD ◄     │─────│──►│  gl_swap()      │     │
  │  (return)      │     │   │  ► YIELD ◄      │─────│──► next rAF
  └────────────────┘     │   └─────────────────┘     │
                         └───────────────────────────┘
```

### Implementation steps

#### Step 1: State persistence across yields

`State` (bytecode stream, stack, code_pos, call frames) must survive between
calls.  It lives in a `Game` the page holds (`src/wasm_entry.rs`), a wasm-bindgen class: the
page passes the handle to every call, so nothing is kept per thread and two games on one page
never meet (`@FR-C-Boundary`, @PLN226).  The `State` is boxed, because `execute_argv` keeps
pointers into it.

```rust
#[wasm_bindgen]
pub struct Game {
    session: Option<GameSession>, // the started program: a boxed `State`
    stage: Option<String>,        // a world snapshot staged for the next start
}
```

#### Step 2: Yield flag on Stores

Add `pub frame_yield: bool` to `Stores` (or `State`).  Default `false`.

The WASM `wgl_swap_buffers` implementation sets `stores.frame_yield = true`
after calling the JavaScript `gl_swap_buffers`.

#### Step 3: Yield check in `execute_argv`

In the main interpreter loop (`src/state/mod.rs`, `execute_argv`), check the
flag after each opcode dispatch:

```rust
while self.code_pos < bytecode_len {
    let op = *self.code::<u8>();
    OPERATORS[op as usize](self);

    // Frame yield: return to JavaScript for requestAnimationFrame
    if self.database.frame_yield {
        self.database.frame_yield = false;
        return;  // code_pos, stack, call frames all preserved in self
    }
}
```

When `execute_argv` returns early, the loft program is **suspended** — its
stack, call frames, local variables, and code position are all intact inside
`State`.  The next call to `execute_argv` (or a thin `resume` wrapper) picks
up from the exact instruction after `gl_swap_buffers`.

#### Step 4: Resume export

`Game::resume_frame`, which JavaScript calls on each `requestAnimationFrame`, resumes the
game's `State` and answers JSON: `{"running":true}` (with `"output"` when the frame printed)
while the game runs, `{"running":false,"output":"…"}` when the loft program completes (loop
finished, window closed, etc.), `{"running":false,"error":"…"}` when it crashed.

#### Step 5: JavaScript frame loop

```javascript
async function runGame(source) {
  const files = JSON.stringify([{ name: 'main.loft', content: source }]);
  const game = new Game();
  let r = JSON.parse(game.start(files));  // parses, compiles, starts execute_argv
  function frame() {
    r = JSON.parse(game.resume_frame());
    if (r.running) requestAnimationFrame(frame);
  }
  if (r.running) requestAnimationFrame(frame);
}
```

#### Step 6: Input integration

`gl_poll_events` reads the current input state from JavaScript (key map,
mouse position) on each call.  Since the interpreter yields between frames,
JavaScript has time to process DOM events between `resume_frame` calls.
The existing `loftHost.gl_key_pressed` / `gl_mouse_x` / `gl_mouse_y`
functions already return the current state — they just need DOM event
listeners to keep that state updated (see GL6.6 in GAME_INFRA.md).

### Scope of changes

| File | Change |
|---|---|
| `src/database/mod.rs` | Add `pub frame_yield: bool` to `Stores` |
| `src/state/mod.rs` | Check `frame_yield` in `execute_argv` loop; add `execute_continue` |
| `src/wasm_entry.rs` | The `Game` class: `start`, `resume_frame`, `stop`, the swap entries |
| `src/wasm_gl.rs` | `wgl_swap_buffers` sets `stores.frame_yield = true` |
| `doc/gallery-run.html` | Use a `requestAnimationFrame` loop with `game.resume_frame()` |

### Native behaviour

On native, `frame_yield` is never set (the native `gl_swap_buffers` calls
glutin's swap which blocks until vsync).  `execute_argv` runs to completion
as before.  No change to native execution.

### Performance

The yield check is one `if` per opcode — a predictable branch that is almost
always not-taken.  Cost: ~1 ns per opcode on modern CPUs, invisible compared
to the opcode dispatch itself.

### Safety analysis and mitigations

#### M1. Data ownership — raw pointer to borrowed Data (HIGH)

**Risk:** `execute_argv` stores `data_ptr = std::ptr::from_ref::<Data>(data)` where
`data` is a borrow from the caller.  If the caller drops `Data` between yield and
resume, the pointer dangles.

**Mitigation:** `GameSession` **owns** `Data` (moved in, not borrowed).  The current
`run_pipeline` already owns `p.data` — `Game::start` moves it into the session.
`execute_argv` is replaced by a wrapper that borrows `&session.data` for each resume,
keeping the borrow live only while the interpreter runs.

```rust
struct GameSession {
    state: State,
    data: Data,         // owned, not borrowed
}
```

#### M2. Cleanup after natural completion (MEDIUM)

**Risk:** `execute_argv` lines 1333–1335 pop the synthetic call frame and clear
`parallel_ctx` after the main loop exits.  With frame-yield, the loop returns early
on each yield, so cleanup runs only when the program finishes its last frame.

**Mitigation:** Split the loop body into `execute_loop` (the `while` loop only) and
move cleanup into `finalize()`.  `resume_frame` calls `execute_loop`; when it returns
without the yield flag set, the program is done — call `finalize` and drop the session.

```rust
// In resume_frame:
session.state.execute_loop();
if session.state.is_finished() {
    session.state.finalize();
    *session_slot = None;  // drop the session
    return false;
}
true
```

#### M3. Re-run disposes old session (MEDIUM)

**Risk:** User clicks "Run" while a game session is still alive.  The old State
holds GL resource handles (VAOs, shaders, FBOs) that leak if not freed.

**Mitigation:** `Game::start` disposes the game's running session first: it calls
`loftHost.gl_destroy_window()` (which frees all GL resources on the JS side) and drops the old
`State` before creating the new one.  `Game::stop` does the same without starting another.

#### M4. Panic during resume (MEDIUM)

**Risk:** An opcode panics inside `resume_frame`.  Without cleanup, the session
remains in the game in an inconsistent state; a subsequent resume would access corrupt
State.

**Mitigation:** `Game::resume_frame` runs the frame under `std::panic::catch_unwind`.  On a
panic it drops the session, closes the window (`loftHost.gl_destroy_window()`) and answers
`{"running":false,"error":"internal panic"}`.

#### M5. Window close / tab navigation (LOW)

**Risk:** `gl_poll_events` always returns `true` in the browser.  The loft program
loops forever if it uses `if !gl_poll_events() { break }` as the exit condition.
Navigating away orphans the session.

**Mitigation:** Add a `should_close` flag on the JavaScript side, set by a "Stop"
button or `beforeunload` event.  `loftHost.gl_poll_events()` returns `false` when
the flag is set, causing the loft loop to break naturally.

```javascript
let shouldClose = false;
document.getElementById('stop-btn').onclick = () => { shouldClose = true; };
window.addEventListener('beforeunload', () => { shouldClose = true; });

loftHost.gl_poll_events = () => !shouldClose;
```

#### M6. Debug step counter overflow (LOW)

**Risk:** The `#[cfg(debug_assertions)]` step counter in `execute_argv` accumulates
across all frames.  At 60fps with ~1000 ops/frame, the 100M limit triggers after
~28 minutes of gameplay, killing the program.

**Mitigation:** Reset `step = 0` at the start of each `execute_loop` entry (i.e.,
each `resume_frame` call).  This counts ops-per-frame rather than ops-total, which
is the useful diagnostic anyway.

### Invariants preserved across yield

These are safe by construction and require no mitigation:

| Component | Why safe |
|---|---|
| **Interpreter stack** | Owned by State, heap-allocated, persists between calls |
| **Call frames** | `call_stack: Vec<CallFrame>` owned by State |
| **Store/heap records** | `Stores.allocations` owned by State.database |
| **Text buffers** | `Str` pointers reference store records; no reallocation during yield |
| **Coroutine frames** | `State.coroutines: Vec<Option<CoroutineFrame>>` persists |
| **Scope-based freeing** | Yield is inside a scope; `OpFreeRef`/`OpFreeText` emit at scope exit, which hasn't happened |
| **WebGL context** | GL state persists across JS event loop iterations |
| **For-loop state** | Counter, limit, jump target are bytecodes — pausing between iterations is safe |

### Limitations

- **One running program per `Game`.** A page that runs two games holds two handles.
- **No input between opcodes.** Input is sampled at `gl_poll_events`, not
  continuously.  This is identical to native behaviour and standard for games.

### Effort

**S–M** — The interpreter change is ~20 lines.  The WASM exports are ~60 lines
(including catch_unwind and session lifecycle).  The gallery JavaScript is ~20 lines.
Mitigations M1–M4 are structural (ownership, error handling) and add minimal code.
