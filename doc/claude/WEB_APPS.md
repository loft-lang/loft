<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Building web applications with loft

A task-oriented guide: how to take a `.loft` program and run it in a web
browser. Read this first — it tells you which build target to pick, exactly what
a browser program can and cannot do, and gives one recipe per kind of web app,
so you do not have to reverse-engineer the host surface yourself.

For the pipeline internals see [HTML_EXPORT.md](HTML_EXPORT.md); for the design
of how loft and JavaScript exchange data see [BROWSER_INTEROP.md](BROWSER_INTEROP.md).

---

## 1. Which build target? (the one that trips people up)

loft has two different WebAssembly builds. They are **not** interchangeable — pick
by where the program runs:

| You want to run… | Use | Output | Size (typical) |
|---|---|---|---|
| **in a browser** | `--html` | one self-contained `.html` (base64 WASM + JS bridge) | ~1.1 MB / ~330 KB gzipped |
| **headless / on a server** (via `wasmtime`) | `--native-wasm` | a `wasm32-wasip2` `.wasm` module | ~5.4 MB / ~1.5 MB gzipped |
| on the desktop | `--native` | a native binary | — |
| quick check, no compile | `--interpret` | runs in the tree-walking VM | — |

**For the browser, use `--html`.** It is the small, no-std engine built for the
web. `--native-wasm` is ~4× larger, links full `std` + WASI, needs an external
runtime (`wasmtime`) to run, and **only compiles** — it does not run the program
for you. Do not reach for it to target a browser.

## 2. Your first browser program

```loft
// hello.loft
fn main() {
  println("Hello from loft in the browser!");
}
```

```bash
loft --html hello.loft        # writes hello.html next to the source
```

Open `hello.html` in any modern browser (2020+). The text appears in a `<pre>`
box on the page. The file is fully self-contained — no server, no other files.

**The engine is opt-in.** A program that uses no graphics/audio compiles to a
**minimal engine-less page**: a small wasm plus a ~10-line inline shim — no
WebGL2 context, no asyncify, no canvas, none of the `loft-gl-wasm.js` engine.
`loft --html` picks this automatically (it reports `· minimal engine-less
shell`) and only emits the full engine page when the program actually uses the
canvas/audio/frame-loop. So "a small standalone web module" is the default, and
"a game engine in the page" is what you opt into by using it.

## 3. What a `--html` program can and cannot do (the host surface)

A `--html` program talks to the browser through a small, fixed set of **host
imports** — JavaScript functions the generated page provides. This surface is
**closed**: anything not on it is not available in the browser, even if it works
on native.

**Available today:**

- **Text output** — `print` / `println` go to a `<pre>` box (or the JS console).
- **Text input** — `host_input()` reads the bytes JavaScript hands in (§5). The
  mirror of `print`; the input channel for a headless compute module.
- **Graphics + input** — a WebGL2 canvas surface (`lib/graphics`): draw calls,
  shaders, textures, plus keyboard, mouse and wheel polling and the canvas size.
  This is what games use. It is a **subset** of the native surface, because a
  canvas cannot do everything a desktop window can — the event queue
  (`gl_next_event` and friends), `gl_create_fullscreen_window` and `gl_screenshot`
  have no browser handler. Calling one is a **build error** naming the function,
  not a broken page: `loft --html` compares the program's imports against the
  shim before writing any HTML (loft#668).

  A library's own `[wasm.bridge] host_js` counts as part of that shim, in all three
  ways JS defines a handler — `name(…) {`, `name:` and `name = function` (loft#681;
  the assignment form used to be invisible, so a program was refused for a handler
  its library had already written).

  If you **drive the emitted wasm from your own JS host** rather than loft's page —
  extracting the module and supplying the imports yourself — the check's premise
  does not hold, because the page it inspects is discarded. Pass **`--host-provided`**
  (alias `--no-host-check`) and a missing import becomes a warning instead of a
  refusal. The wasm is unchanged either way: a name your host does not define still
  fails at instantiate, so this relaxes the diagnostic, never the requirement.

**Ask before you design: `loft targets`.** It lists the stdlib builtins that do NOT
exist on a target, so a plan can be checked in seconds instead of discovering the
hole when its first executable step fails to build (loft#680 — a coverage plan was
written on the assumption that the working-set store loaders worked in the browser,
and only the first `--html` build said otherwise). The answer is **derived, never
hand-written**: `scripts/gen_target_surface.py` asks rustc which runtime methods each
builtin's `#rust` body can reach in loft's Rust runtime built for the browser, and
`make ci` fails if the committed table goes stale.  A page is built from the C, which
that table does not read: a builtin the C cannot serve on a page shows at the
`--html` build. As of
this writing every stdlib builtin is available on the browser target.
- **Audio** — raw PCM playback via the Web Audio API.
- **WebSocket** — a live network socket, through the `web` library (see §4b).

**Also available:**

- **A filesystem** (loft#851) — the whole file surface, answering what
  `--interpret` and `--native` answer for the same program. It is the PAGE's
  filesystem, not the user's disk: an immutable base tree the page supplies
  (`globalThis.loftBaseFS`) plus every write, kept in `localStorage` so a tab
  close does not lose the user's work. A path the page was never given reads as
  absent. See [WASM.md § The page filesystem](WASM.md).

**NOT available in `--html`** (these silently do nothing or return empty):
- **No program arguments** — `arguments()` returns an empty list. (Use
  `host_input()` for JavaScript-supplied input instead — §5.)
- **No HTTP client** — `web`'s `http_get` / `http_post` / … are **native-only**;
  they do not work in the browser. WebSocket is the only browser network
  transport today. (Let JavaScript do `fetch` and hand loft the bytes via
  `host_input()` — §5.)

> This closed surface is the thing the first browser consumer had to discover by
> dumping the WASM imports. That is exactly what this section is for — you should
> never have to do that.

(There is a *second*, unrelated browser build — the IDE's `make wasm`
[wasm-bindgen] build — which exposes an args bridge `--html` still lacks. Its
bridges belong to that build, **not** to `--html`, which reaches its own
filesystem over raw `loft_io` imports instead. See [WASM.md](WASM.md) § Host Bridge
API, and do not assume it applies here.)

## 4. The three shapes of a loft web app

### 4a. A canvas / GL app (games, visualisations) — shipped, the main path

loft owns the frame loop; the browser renders each frame. Your source is
**identical** to the native version — the `gl_swap_buffers()` call yields to the
browser automatically (the "frame yield", see [HTML_EXPORT.md](HTML_EXPORT.md)).

```loft
use graphics::(create_window, clear, swap_buffers, poll_events, key_pressed);
fn main() {
  create_window(800, 600, "demo", true);
  while poll_events() {
    clear(0x101820);
    // draw your frame here
    swap_buffers();          // yields to the browser on --html, blocks ~16ms native
  }
}
```

```bash
loft --html game.loft
```

Worked example: `make game` builds `doc/brick-buster.html`. Input comes from
`key_pressed` / `mouse_x` / `mouse_y` / `mouse_button`.

### 4b. A WebSocket client (live data from a server) — via the `web` library

```bash
loft install web            # from the registry
```

```loft
use web::(ws_handler, try_recv, send, frame_yield);
fn main() {
  h = ws_handler("wss://example.com/socket");
  loop {
    msg = try_recv(h);       // "" when nothing is waiting
    if msg != "" { println("got: {msg}"); }
    send(h, "ping");
    frame_yield();           // hand the frame back so the socket can deliver — REQUIRED
  }
}
```

`frame_yield()` is load-bearing in the browser: a loop that never yields freezes
the page and never receives. On native it is a no-op. The `web` library ships the
browser bridge for this (its `[wasm.bridge]` + `wasm/host.js`); you do not write
any JavaScript.

### 4c. A headless compute kernel (JavaScript drives the UI, loft computes)

This is the "loft as a browser compute service" shape: JavaScript owns the page
and the network, calls a pure-loft kernel with some input, and shows the result.
This is a standalone module (§2) — the minimal engine-less page, no canvas, no
frame loop.

```loft
// kernel.loft — reads the input, computes, prints the result.
fn main() {
  spec = host_input();               // the bytes JS handed in
  // ... parse spec, compute ...
  println("{result}");               // JS reads this off the page / print hook
}
```

- **Output** — the kernel `print`s its result; the page's `<pre>` (or your own
  `loft_host_print` hook) receives it.
- **Input** — `host_input()` returns the bytes JavaScript set on
  `globalThis.loftInput` before the program ran. `loft_start` builds fresh state
  each call, so a Web Worker can instantiate the wasm once and call it per
  request:

```js
globalThis.loftInput = "52.0,5.0;52.009,5.0";   // set input, then run
const { instance } = await WebAssembly.instantiate(wasmBytes, {
  loft_io: {
    loft_host_print: (p, l) => { out += dec.decode(new Uint8Array(mem.buffer, p, l)); },
    loft_host_input_len: () => inputBytes.length,
    loft_host_input_copy: (ptr) => new Uint8Array(mem.buffer, ptr, inputBytes.length).set(inputBytes),
  }
});
mem = instance.exports.memory;
instance.exports.loft_start();                   // out now holds the result
```

The kernel stays pure loft and runs unchanged on every target: develop and test
it on `--interpret` / `--native` / `--native-wasm` by feeding the same bytes on
**stdin** (`host_input()` reads stdin off the browser), then ship the browser leg
— identical output (§7).

## 5. Getting data in and out — the byte channel

The model is simple and one-directional in each direction: **the browser host
moves only opaque bytes** between loft and JavaScript; neither side reads the
other's data structures. Your loft library decides the meaning.

- **loft → JavaScript (out):** `println` / `print` (the `loft_io.loft_host_print`
  host import); the page appends the text. A library can serialize any value to
  JSON or bytes and push it the same way.
- **JavaScript → loft (in):** `host_input()` (the `loft_io.loft_host_input_len` +
  `loft_host_input_copy` host imports — loft sizes the buffer, the host fills it).
  The mirror of `print`; set `globalThis.loftInput` before the program runs (§4c).
- **Networking:** let JavaScript do it. JS `fetch(...)` is trivial; hand the
  response bytes to loft via `host_input()`. loft stays pure compute and needs no
  HTTP in the browser at all.

## 6. Build, size, and serve

- `loft --html app.loft` → `app.html` (override with `loft --html app.loft out.html`).
- The `.html` is one self-contained file — serve it as a static asset, or open it
  directly. No bundler, no ES modules, no server runtime.
- `LOFT_HTML_TITLE="My App"` sets the page `<title>`.
- Install `wasm-opt` on your `PATH` for a smaller binary (the pipeline runs `-O1`
  automatically when present; it warns and skips if absent).
- **Cache gotcha:** if you switch a program between `--interpret` and `--html` and
  see stale behaviour, clear `~/.cache/loft` (`rm -rf ~/.cache/loft`). See
  [WASM.md](WASM.md).

## 7. Run the same source on every target (parity)

A pure-loft program (no `#native`, no I/O beyond the channel) compiles to the
interpreter, `--native`, `--native-wasm`, and `--html` from **one source**. Prove
they agree before shipping the browser leg — build and test the kernel on the
non-browser targets, where iteration is fast, and assert byte-identical output:

```bash
loft --interpret   kernel.loft < input.txt
loft --native-wasm kernel.wasm ; wasmtime run kernel.wasm < input.txt   # WASI stdin
```

A worked parity harness (interpreter vs `--native-wasm`, byte-for-byte) lives in
the `routing` consumer at `tools/kernel_headless_test.sh` — copy its shape.

## See also

- [HTML_EXPORT.md](HTML_EXPORT.md) — the `--html` pipeline internals: cdylib
  codegen, the WebGL2 bridge, the asyncify resume loop.
- [BROWSER_INTEROP.md](BROWSER_INTEROP.md) — the design of the loft↔JavaScript
  byte channel, the four tiers, and the `host_input` input primitive.
- [WASM.md](WASM.md) — the broader WASM runtime (the separate `make wasm`
  wasm-bindgen build, VFS, threading).
- The `web` library — HTTP (native) + WebSocket (native + browser) client.
