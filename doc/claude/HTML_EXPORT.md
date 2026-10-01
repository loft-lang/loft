<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# HTML export pipeline — `loft --html`

Reference for the shipped single-file HTML export pipeline.
`loft --html game.loft` produces a self-contained `.html` file
that runs a fully compiled loft program in the browser at
native speed.

**Non-goal:** embedding the interpreter.  The program compiles
to optimised WASM via the native codegen pipeline; the HTML
file is just a base64-embedded WASM blob + inline JS bridge.

## Pipeline architecture

```
loft --html game.loft
  │
  ├─ Parse + compile (existing)
  ├─ Generate Rust source (existing output_native_reachable)
  ├─ rustc --target wasm32-unknown-unknown --crate-type cdylib
  ├─ wasm-opt -O1 (optional)
  └─ Assemble HTML (base64 WASM + inline JS bridge)
```

**Size:** ~200–400 KB raw WASM, ~70–130 KB gzipped (typical).

The CLI flag is parsed in `src/main.rs` (alongside `--native-emit`).
Output defaults to `game.html` next to the source; override with
`--html out.html`.

## Codegen — cdylib entry point

In normal `--native` mode, the generated Rust has `fn main()`.
For `--html`, codegen instead emits an exported entry point:

```rust
use std::cell::RefCell;

thread_local! {
    static STORES: RefCell<Option<Stores>> = RefCell::new(None);
}

#[no_mangle]
pub extern "C" fn loft_start() -> i32 {
    let mut stores = Stores::new();
    init(&mut stores);
    n_main(&mut stores);
    0  // 0 = finished, 1 = yielded (frame loop)
}
```

The `cdylib` crate type produces a `.wasm` module rather than a
binary.  The exported `loft_start` is what the JS bridge calls.

## JS bridge — host imports

> **This list is COMPLETE.**  An `--html` bundle imports print, `host_input`
> (JS→loft input QUEUE, #476 — seed `globalThis.loftInput`, push live
> messages with `globalThis.loftPush(msg)`, one pop per call),
> `host_output` (loft→JS structured messages → `globalThis.loftOutput(msg)`),
> the `loft_io.loft_host_fs_*` FILESYSTEM (loft#851 — `doc/loft-fs.js` is the
> host half; see [WASM.md § The page filesystem](WASM.md)), the GL set, the
> frame yield — plus whatever `[wasm.bridge]` routes a used library ships (e.g.
> `web`'s WebSocket).  **There is still no `arguments()` and no env**: those
> stdlib calls compile to in-wasm stubs returning empty.  (The
> `globalThis.loftHost` bridges in [WASM.md](WASM.md) belong to the
> wasm-bindgen IDE build, not to `--html`, and `--html` cannot reuse them —
> they need wasm-bindgen.)

The HTML loader passes a JS `imports` object into the WASM
instance.  The shape:

```javascript
const imports = {
  env: {
    // println goes through this host import
    loft_println: (ptr, len) => { /* read from memory.buffer, decode UTF-8, append */ },

    // GL functions are imports — see "GL bridge" below
    gl_clear: (r, g, b, a) => { /* WebGL2 call */ },
    gl_draw: (vao, count) => { /* WebGL2 call */ },
    gl_swap_buffers: () => { /* request next animation frame */ },
    gl_poll_events: () => { /* return 0 if user closed tab, 1 otherwise */ },
    // … one entry per GL function the program uses

    // Frame yield — see "Frame yield contract" below
    loft_yield: () => { /* save state, request animation frame, exit loft_start */ },
  }
};

WebAssembly.instantiate(wasmBytes, imports).then(r => {
  instance = r.instance;
  instance.exports.loft_start();
});
```

Strings cross the boundary as `(ptr, len)` pairs into the WASM
linear memory.  The JS side reads via
`new Uint8Array(instance.exports.memory.buffer)` and decodes with
`TextDecoder('utf-8')`.

### A call the browser cannot serve

The shim implements a SUBSET of the native surface — a canvas cannot do
everything a desktop window can, and some calls have no browser meaning at all
(`gl_screenshot` writes to a `path`; a canvas capture is asynchronous and the
wasm target has a virtual FS).

**Naming such a call is not an error.**  Whether it can be served is a fact
about this run on this target, not about whether the program is well-formed, so
the build reports it and the page answers at RUNTIME: each unserviceable
`loft_*` import gets a stub returning its declared zero (`false` / `0`), and the
first call logs `loft: <name> is not available in the browser` to the console.
Callers already have to check — a screenshot fails for a dozen reasons besides
the target — so one source runs on both renderers, and the browser build simply
reports "no picture here".

This was a build refusal until loft#709.  Refusing forced the source to fork
into two entry points differing only in which calls they may *name*, which
destroys the property two renderers exist to provide: each is the other's
control, so a difference between two pictures is the renderer and not the
harness.  (The refusal was itself an improvement over loft#668, where the same
shape reached the browser as a `LinkError` naming an import index.  The
diagnosis was right; the disposition was not.)

`host_import_stub_js` (`src/native_utils.rs`) emits the stubs, applied after the
page shim and every `[wasm.bridge]` host_js, so a real handler always wins.
Gate: `tests/wasm/html-unavailable-builtin.sh` builds a program naming an absent
builtin, loads the page in headless chromium, and checks that the program's own
`else` branch is what ran.

## GL bridge — WebGL2 imports

Every GL function the loft program uses becomes an import.
Codegen scans the reachable native call graph and emits one
import per `gl_*` function found.  The JS bridge implements
each as a thin wrapper over the WebGL2 context obtained from
the `<canvas>`:

```javascript
const gl = canvas.getContext('webgl2', { antialias: true });

// Each gl_* loft function maps to one or more WebGL2 calls
const imports = {
  env: {
    gl_clear: (r, g, b, a) => {
      gl.clearColor(r/255, g/255, b/255, a/255);
      gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    },
    gl_draw: (vao, count) => {
      gl.bindVertexArray(vaoTable[vao]);
      gl.drawArrays(gl.TRIANGLES, 0, count);
    },
    // VAO / shader / buffer / texture handles are integer ids stored
    // in JS-side tables, indexed by the values loft passes around.
    // …
  }
};
```

VAOs, shaders, buffers, and textures are GL handles in JS
tables; loft programs pass around the integer ids returned by
`gl_create_*` functions.

### Text and authored pixels (loft#737, loft#738)

The browser already HAS a rasteriser, so the text bridge is a 2D canvas:
`measureText` for the metrics, `fillText` for the coverage bitmap.  The alpha
channel of white-on-transparent IS the coverage, which is the same 8-bit bitmap
fontdue gives the desktop backend — bitmap height = line height, baseline at
`ascent` from the top — so `gl_measure_text` / `gl_text_height` /
`gl_font_ascent` / `rasterize_text_into` / `gl_text_texture` all agree with it.

A font PATH resolves to a **CSS family**, not a file: nothing here can load font
bytes synchronously, and an async load would change the metrics *between* the
measure and the rasterise of one string.  `familyFor` answers the requested
family followed by a generic guessed from the name (`"Foo", monospace`), so a
family the page carries wins and one it does not have falls to the generic
rather than to a wrong one.

A page brings its own font by declaring `[[font]]` in its `loft.toml`: `--html`
emits the `@font-face` (or the provider `<link>`) into the `<head>` and awaits
`document.fonts.load` for every declared family ahead of `loft_start`
(@PLN146 F5/F6, `src/html_fonts.rs`).  A library's declarations travel to the
consumer's page the same way `[wasm.bridge] host_js` does.  Carrying the CSS by
hand in `host_js` still works — the declaration makes the common case
declarative so a game writes no JS and the ordering is automatic rather than
remembered.

Three sources, one shape:

| Source | Declared | Browser | Native / `--native-wasm` |
|---|---|---|---|
| a family the browser already has | `family` (+ `native`) | nothing shipped, nothing fetched | the TTF beside the game |
| our own file server | `url = "…"` | `@font-face { src: url(…) }`, or the WOFF2 packed in the asset store and range-read like every other asset | the same store |
| Google Fonts, or any CDN | `stylesheet = "…"` | the provider's stylesheet `<link>`; zero bytes of ours | the TTF beside the game |

```toml
[[font]]
family = "PressStart2P"
native = "fonts/PressStart2P.ttf"
url    = "fonts/PressStart2P.woff2"
```

Two mechanics decide whether it works.  The declared `family` must **equal** the
base name the program passes to `gl_load_font` — that string is the key
`familyFor` builds, and drift is a silent fallback, never an error; `native` is
that path, so the two are checked against each other at BUILD time and a
mismatch is refused before the wasm compile.  And the await has to come before
`loft_start`: a webfont is still in flight while the first frame draws, and
`familyFor`'s answer is cached per handle, so an early `gl_load_font` paints in
the fallback with nothing on stderr.

A remote font is a third-party dependency: offline, or with the CDN blocked, the
chain degrades to the generic rather than failing — which is right, and is why
the native source stays declared beside the browser one.  What was wrong was
doing it quietly.

⚠ `document.fonts.check` cannot be used to decide whether the page HAS a family:
it is **true** for a family nothing declares and **false** for an `@font-face`
that is still loading.  To ask, measure the family against two generics — one
the browser has overrides both, one it does not follows each.  `gl_load_font`
does exactly that and `console.warn`s when the answer is no; every resolution is
recorded on `globalThis.loftFonts`.

`gl_upload_alpha_texture` takes a coverage buffer the PROGRAM computed — already
in wasm memory, so no fetch and no asset pipeline.  WebGL2 has no
`TEXTURE_SWIZZLE`, which is how the desktop backend makes a RED texture sample as
`(1,1,1,r)`, so the bridge expands to RGBA: same sampling result, and the shaders
stay identical across targets.  `gl_load_texture` serves a **bundled** asset —
`--html` embeds every `.png` sibling of the entry file and decodes it before
`loft_start`, so the lookup is synchronous; a runtime URL genuinely needs async
and reports failure (`0`, never a valid handle — `hold` is 1-based).

Gates: `tests/html_fonts.rs` drives the emitted font block in headless chromium
— three sources resolving to the family asked for, and a throttled source with
the await removed as the control.  `tests/gl_text_bridge.rs` drives
`tests/data/gl_text_probe.html` in the same way.  It asserts what the bridge PRODUCES — ink in the coverage,
metrics that scale with size, handles that are handles — because the failure this
replaces was stubs that returned plausible numbers and no pixels, which every
import-shape check passed.

### Shader translation: desktop GLSL → GLSL ES 3.00

`lib/graphics/src/*.loft` writes shaders for desktop OpenGL
(`#version 330 core`).  WebGL2 requires GLSL ES 3.00 —
the directive `'core' : invalid version directive` is a
hard error; explicit precision qualifiers are also mandatory
in fragment shaders.

Both JS bridges (`doc/loft-gl-wasm.js` for `--html`,
`doc/loft-gl.js` for the gallery) rewrite the version header
transparently in `gl_create_shader`:

```javascript
function translateShader(src, isFragment) {
  const re = /^\s*#version\s+\d+(\s+\w+)?\s*\n?/;
  const head = isFragment
    ? '#version 300 es\nprecision highp float;\nprecision highp int;\n'
    : '#version 300 es\n';
  return re.test(src) ? src.replace(re, head) : head + src;
}
```

The GLSL subset our shaders use (`in`/`out`/`layout(location)`/
`texture()`/`discard`/`gl_Position`/`uniform`) is shared between
the desktop and ES profiles, so the version + precision header is
the only change needed.  Loft sources stay portable — same .loft
runs on desktop OpenGL via lib/graphics native cdylib and on
WebGL2 via the browser bridge.

### Browser-side render gate — `tests/html_render.rs`

The recurring failure mode for browser deployment is a JS-side
console error after a bridge or shader change: WASM `loft_start`
returns cleanly (so `tests/html_wasm.rs` passes), the page loads
without exceptions, but `compileShader` fails on every frame and
the canvas stays blank.

`tests/html_render.rs` closes the loop with two layers:

**Layer 1 — JS console gate.**  Spawns headless Chrome with
SwiftShader WebGL2, navigates to `doc/brick-buster.html`, asserts
zero `Runtime.consoleAPICalled type=error` or
`Runtime.exceptionThrown` events across a 6-second startup
window.  Catches shader compile errors, missing WebGL2 contexts,
init exceptions.

**Layer 2 — canvas content gate.**  After Layer 1 passes, clip a
screenshot to the canvas element's bounding rect, decode the PNG
inline (via `node:zlib` — no extra deps), count distinct RGB
triples.  Fail if below 20 distinct colors.  A blank canvas (only
clearColor) has 1-2 colors; a working Brick Buster frame has 128.
Catches "compiles clean, blank canvas" — a successful shader
compile that never gets used to draw, a draw call that no-ops, a
state-tracking regression that silently skips geometry.

Wired into `cargo test --release` (so `make ship` / `make ci`
pick it up); skips cleanly when prerequisites (google-chrome /
node / target/release/loft / `doc/brick-buster.html`) are missing
OR when `doc/brick-buster.html` is older than its source loft
program.  ~6 s end-to-end if the HTML is fresh.

**Refresh the HTML before testing.**  The test does NOT auto-build
via `make game` — invoking `cargo build` mid-`cargo test` races
the parallel rustc invocations in `tests/native.rs` over
`target/release/deps/`.  Run `make game` manually (or
`make test-html-render` which does the build + test in one step)
when touching `--html` / `lib/graphics`.

The CDP driver lives in `tools/html_render_check.mjs` — a
single-file Node script with no extra deps (uses Chrome
DevTools Protocol via a built-in WebSocket implementation, and
a minimal RGBA8/RGB8 PNG decoder for Layer 2).  Reusable for
any future browser-deployed example:

```
node tools/html_render_check.mjs <url> \
  --wait-ms N --screenshot path \
  --canvas SELECTOR --canvas-min-colors N
```

Reports JSON to stdout on success; JSON to stderr on failure.
`make test-html-render` runs just this gate for ad-hoc
invocation.

## Frame yield contract — browser game loop

The interpreter / native code runs synchronously.  In the
browser, that would block the main thread for an entire frame
sequence — only the last frame would render, no input would
process.

The pipeline solves this by **yielding at `gl_swap_buffers`**:

1. The loft `gl_swap_buffers()` call is mapped to an import
   `loft_yield`.
2. `loft_yield` saves the loft program's state into the
   `STORES` thread-local, requests `requestAnimationFrame`,
   then returns control to the JS bridge.
3. JS receives control; on the next frame, it resumes loft
   from where it left off.

User loft code is **identical** on native and browser:

```loft
for _ in 0..3600 {
  if !gl_poll_events() { break }
  gl_clear(bg);
  gl_draw(vao, count);
  gl_swap_buffers();   // ← yields on browser; blocks 16ms on native
}
```

No callback pattern, no API change, no user-visible
generators.  This is the design that "Step 7: Frame yield"
shipped with.

The native target uses `wasm-opt --asyncify` if available, OR
a manual state-machine yield implemented in codegen
(measure-and-decide; the current implementation favours
asyncify for binary size).

### The resume loop — `AsyncifyCtrl` + scheduler

Any suspending import — `loft_gl.loft_gl_swap_buffers` (games) and
`loft_web.ws_yield` (the `web` library's WebSocket `frame_yield`) — drives the
same `AsyncifyCtrl` in `doc/loft-gl-wasm.js`.  Two correctness rules are
load-bearing for the suspend/rewind ABI; getting either wrong leaves the
program stuck on its FIRST suspend (the symptom of issue #450 — the page
prints only the first line, then nothing):

1. **Rewind from the SAVED TOP, not the buffer base.**  Asyncify writes the
   stack upward from the buffer base during an unwind (leaving `current` at the
   top) and reads it back during a rewind.  `resume()` must set the data
   struct's `current` to the saved top captured after the matching unwind;
   resetting it to the base discards every saved frame, so the program rewinds
   into an empty stack and "returns" without resuming.
2. **The suspend shim is state-aware.**  During a rewind the suspend import is
   re-invoked when execution replays up to the yield point.  In that case it
   must `asyncify_stop_rewind()` and RETURN so execution continues PAST the
   yield.  Re-starting an unwind there spins forever on one iteration.

The resume loop is then driven by a scheduler that survives a **hidden** page.
Chromium pauses/throttles `requestAnimationFrame` for hidden pages (a headless
capture page and a backgrounded tab are both hidden), so an rAF-only loop
stalls.  The generated page (`src/main.rs`) pumps via an unthrottled
`MessageChannel` while `document.hidden` is true and via `requestAnimationFrame`
while visible (so a GL render loop stays vsync-aligned), re-checking visibility
each tick.  A visible page can be denied frames too (a throttled or occluded
renderer that never sets `document.hidden`), so each visible resume also arms a
100 ms fallback; once it fires, the loop pumps as a hidden page does until a frame
arrives again.  The asyncify-aware Node driver `tools/wasm_ws_repro.mjs` carries the
same corrected `AsyncifyCtrl` and pumps on `setImmediate`; the browser gate is
`tests/html_asyncify.rs` (asserts a multi-suspend program reaches its final line
visible, hidden, and visible with no frames granted).

## Reading a store in the page (@PLN146 F4)

`wasm32-unknown-unknown` has no filesystem, so `Store::load`'s `std::fs::read` cannot
answer there — `store_load` used to return `false` for every path in a browser, politely
and with nothing to act on.  The loader now falls back to the `loft_host_fs_*` bridge
(`store::image_bytes` / `image_at_least`), which is what `doc/loft-fs.js` serves
`globalThis.loftBaseFS` through.  A page that CARRIES a store therefore reads it with the
same `store_load` call the desktop makes:

```html
<script>globalThis.loftBaseFS = {"/game.meta.store": <Uint8Array>};</script>
```

Native is unchanged — one `metadata` call, then `std::fs::read`; the host arm never runs
there.  On wasm the existence probe costs a read and the load costs a second, which is
the price of one code path and is paid against bytes the page already holds.

⚠ A page with no such file still answers `false`, and `tests/html_page_store.rs` gates
both halves: a loader that reported success for a file nobody supplied would be worse
than the refusal it replaced.

### Putting a file there — `[[embed]]`

A page carries a file by declaring it in `loft.toml`:

```toml
[[embed]]
path   = "assets/game.pack"   # what the PROGRAM passes; also the key in the page's FS
source = "build/game.pack"    # optional — where the bytes are on the build box.
                              # Defaults to `path`.
```

Both are resolved against **the program's own directory**, because loft resolves a path
a program passes relative to the program file: `assets/game.pack` in `src/game.loft`
means `src/assets/game.pack`, on the desktop and here alike.  So omitting `source` means
*the file the desktop run opens* — which is what makes the two agree.  (A library's
declarations resolve against the library instead; its file is the library's to locate.)

`--html` reads the file and seeds `globalThis.loftBaseFS` with it, so
`store_load(q, "assets/game.pack")` is the same call on the desktop and in the page.
The key is `/` + `path`, which is what `loft-fs.js` `resolve()` makes of a relative
path under the default cwd `/`.  The seed ADDS to whatever the page already had, so a
hand-seeded tree and a library's `host_js` both survive it.  A library's declarations
reach a consumer's page by the same route as `[wasm.bridge] host_js`, with `source`
resolved against the LIBRARY.

⚠ `LOFT_PATHS=cwd` moves the desktop's resolution to the working directory while the
page keeps resolving against `/`.  The parity statement above is about the default.

`path` must be **relative and in normal form** — `--html` refuses `/abs/game.pack`,
`./assets/game.pack` and `a/../b` before the wasm compile, along with a `source` that
is not there and one name declared from two files.  Each of those would otherwise be
carried faithfully under a key the program never asks for: `store_load` answers
`false`, the game draws no art, and nothing says why.

This is the exception, not the pipeline.  Assets travel as a store on a dumb file
server read by HTTP range; embedding is for the bytes a page needs before its first
fetch, and for a page that has to be a single self-contained file
([REMOTE_STORES.md](REMOTE_STORES.md) § An asset pack is a store, not a bundle).  The
bytes go in base64, so the page grows by about 4/3 of the file — the size line
`--html` prints is what to watch.

⚠ A `store_persist_copy` writes a `.dschema` sidecar beside the store.  A load does
not need it, so one `[[embed]]` per store is enough; declare the sidecar too if the
page should carry it.

## HTML assembly — what ships in the output

The output `.html` is a single self-contained file:

```html
<!DOCTYPE html>
<html><head>
  <meta charset="utf-8">
  <title>Loft Program</title>
  <style>/* canvas full-bleed; pre console for non-GL programs */</style>
  <!-- @PLN146 F5: one @font-face / <link> per declared [[font]] source -->
</head><body>
  <canvas id="c" tabindex="0" style="display:none"></canvas>
  <pre id="out"></pre>
  <script>
    const wasmB64 = "<base64-encoded WASM>";
    const wasmBytes = Uint8Array.from(atob(wasmB64), c => c.charCodeAt(0));
    /* host imports as described above */
    /* WebAssembly.instantiate + loft_start() */
  </script>
</body></html>
```

The `<canvas>` is hidden by default; GL functions show it on
first call.  The `<pre id="out">` collects `println` output.

## wasm-opt integration

If `wasm-opt` is on `PATH`, the pipeline runs `-O1` post-
compile to shrink the binary.  `-Oz` was tried but was too
slow for the iterative-build use case (CLI feedback latency
matters more than a few KB).  See CHANGELOG_TECHNICAL.md
"loft --html switched to wasm-opt -O1" for the rationale.

If `wasm-opt` is not on `PATH`, the pipeline skips the
optimisation step with a warning.

`--strip-debug` is part of that `-O1` line, and it drops the wasm `name`
section along with the DWARF — which is what makes a trap's frames
unresolvable.  `--names` swaps it for `--strip-dwarf -g`, keeping the names
while still dropping the bulk (1.5 MB of DWARF against a 100-byte name
section, measured on a toy module).

## Customisation hooks

Limited surface today; expand as need arises:

- `--html out.html` — explicit output path
- `--names` — keep the wasm name section so a trap's frames resolve to loft
  function names (see *When a page faults* below)
- `LOFT_HTML_TITLE=…` (env var) — override the `<title>`
- WASM imports use the `env` namespace — replace any import
  via a custom JS bridge if you wrap the output HTML

## When a page faults (loft#950)

A browser page reports a fault to the page, not to stderr — on
`wasm32-unknown-unknown` stderr is a sink, so anything written there is
simply lost.

That matters because of how a fault ENDS on this target: a panic aborts,
and abort compiles to the `unreachable` instruction.  So the console's
whole symptom is:

```
RuntimeError: unreachable
```

No message, no location, no stack.  loft#950 was reported as an
undebuggable trap for exactly that reason.  Both fault paths now route
through the same host import `println` uses:

| fault | what the page gets |
|---|---|
| a Rust panic (incl. a failed `assert`) | `loft: panicked at …: <message>` + the loft frames under it |
| a loft `panic(…)` / any `RuntimeError` | the ordinary rendered diagnostic, with `--> file:line:col` |

The loft frames come from the shadow call stack `cr_call_push` already
maintains, and they are the half a reader acts on: the Rust location names
a spot in generated code or in loft's own runtime, which does not say what
the *program* was doing.

```
loft: panicked at prog.rs:502:18:
game.loft:2 assertion failed
  in inner() (game.loft:1)
  in middle() (game.loft:5)
  in main() (game.loft:6)
```

### A trap is not a panic (loft#1059)

The table above is about a PANIC.  A **trap** — most often the stack running out
under deep recursion — does not run the panic hook at all, so none of that
rendering fires.  What a trap does do is throw into the JS that called the
module, and both page shells now catch it: the exception, the browser's own wasm
backtrace, and — when the message names an exhausted stack — a line saying that
bound belongs to the page's wasm engine rather than to loft, since the same
program halts with loft's diagnostic on `--interpret` and `--native`
([WASM.md § How deep a program can recurse](WASM.md)).

The report does not call back into the module for loft's own frames, and cannot:
a trap leaves the shadow-stack pointer wherever it died, because the epilogues
that would restore it never ran, so a second entry runs off the end of the stack
it just exhausted.  The browser's backtrace is the evidence instead, and
`--names` is what makes its frame numbers resolve.

The catch covers the boot call and the asyncify RESUME pump both — a trap during
a frame loop used to stop the pump silently, which reads as a page that simply
stopped.

**What is still silent:** an allocation failure.  `handle_alloc_error`
aborts without running the panic hook, so a page that OOMs still traps
bare.  That is worth knowing rather than worth fixing — after this, a trap
with no message before it has told you it was not a panic.

The hook is installed by the generated `loft_start`, ahead of `init`, so a
panic during startup is covered.  It is a no-op on every other target,
where stderr works and the native crash reporter owns this job.

### `--names`: reading the browser's own backtrace (loft#954)

Chrome hands a trap a complete wasm backtrace, and it is the best evidence
there is — ten frames naming the failing function and its whole call chain:

```
[exception] RuntimeError: unreachable
    at wasm://wasm/0168beca:wasm-function[1073]:0x56a035
    at wasm://wasm/0168beca:wasm-function[1054]:0x567983
```

A default page cannot resolve one of those numbers, so the only route left
is bisecting the loft source by hand — rebuild, redrive the browser, move
one `println`.  **`loft --html --names`** makes them resolve.  It costs
roughly 10–15 % page size, so it is opt-in: the people who need it are
debugging and will take the bytes.

It has to do two things, and either one alone is useless:

| half | what it does | if it were missing |
|---|---|---|
| `wasm-opt --strip-dwarf -g` instead of `--strip-debug` | keeps the `name` custom section | no names at all — the default page |
| `#[inline(never)]` on each generated loft function | leaves a frame to name | measured on a four-function program, LLVM folded all of them into `loft_start`; the section then named 616 std/alloc internals and not one loft function |

The names are the generated Rust symbols, which carry the loft name
verbatim: `_RNvCs3DwF3yqkNJQ_4prog17n_part_thumb_wire` is `part_thumb_wire`.
Only loft's own functions are pinned out of line — std, alloc and loft's
runtime still inline freely, so the cost stays on the code the backtrace is
about.

Two things to know before leaning on it:

- **It is a different build.** Pinning functions out of line can move an
  optimiser-sensitive fault.  If a trap reproduces without `--names` and not
  with it, that difference is itself the finding.
- **A build that produced no section says so.**  `--names` is asked for
  exactly when a page must be debugged from its backtrace, so a silently
  nameless page — a binaryen that does not honour `-g` — is reported rather
  than shipped looking identical.

Guarded by `html_names_flag_makes_loft_functions_resolvable_in_a_backtrace`
in `tests/html_wasm.rs`, which asserts both halves and pins the default page
as still stripped, so a green there cannot come from names that were always
present.

## Build prerequisites

For users running `loft --html`:

- Rust toolchain with `wasm32-unknown-unknown` target
  installed (`rustup target add wasm32-unknown-unknown`).
- `libloft.rlib` + transitive deps for that target
  (installed by `make install` to
  `/usr/local/share/loft/wasm32-unknown-unknown/`).
- `wasm-opt` on `PATH` (optional but recommended for size).
- A modern browser with WebGL2 + WebAssembly + ES module
  support (any 2020+ browser).

## Where to find the code

| Concern | Location |
|---|---|
| `--html` CLI flag parsing | `src/main.rs` |
| Codegen entry-point switch (cdylib vs main) | `src/generation/mod.rs` (output_native_reachable, with `wasm_cdylib: bool`) |
| WASM-import emission for GL / println / yield | `src/generation/` (ops dispatch + native registry) |
| HTML assembly + base64 embedding | `src/main.rs` (HTML template + `wasm-opt` invocation) |
| End-to-end test | `tests/html_wasm.rs` (runs `--html` against fixture loft programs, verifies the output HTML loads + executes via headless chromium) |
| Makefile targets | `make game` (brick-buster.html), `make wasm-html-test` (E2E gate) |

## Subsequent W1.x work (not part of W1.1)

W1.18 added WASM Worker Thread infrastructure for parallel
`par(...)` in the browser.  W1.19 / W1.20 added the random /
time host bridges.  See [WASM.md](WASM.md) for the broader
WASM runtime + CHANGELOG_TECHNICAL.md for the W1.x history.

The HTML export pipeline itself (W1.1) was complete in 0.8.4;
later W1.x evolutions enhance the runtime but don't change
the export model documented above.

## See also

- [WEB_APPS.md](WEB_APPS.md) — **start here to build a web app**: the
  task-oriented guide (which target, the exact `--html` host surface, a recipe per
  app kind). This doc is the pipeline internals underneath it.
- [BROWSER_INTEROP.md](BROWSER_INTEROP.md) — **design**: how a
  `--html` program and its libraries talk to the browser/JS —
  the engine as an agnostic byte-mover, the four tiers, and the
  gather-until-enough library contract over the frame yield.
- [WASM.md](WASM.md) — WASM runtime architecture (VFS, host
  bridges, threading, frame yield mechanics — the layer this
  pipeline produces output for)
- [`lib_plans/58-graphics/README.md`](lib_plans/58-graphics/README.md) — graphics library design
  (the GL functions imported by the HTML bridge)
- [`lib_plans/61-game-infra/README.md`](lib_plans/61-game-infra/README.md) — game infrastructure
  (sprites, tilemap, etc., consumed by HTML-exported games)
- `plans/finished/31-html-export/README.md` — closure record
  (commits, evidence, historical 10-step build sequence
  preserved as archaeology)
